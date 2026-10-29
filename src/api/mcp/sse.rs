//! Server-Sent Events (SSE) session management and streaming adapter for MCP.

use crate::http::Request;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

const SSE_QUEUE_CAPACITY: usize = 64;

static SSE_SESSIONS: OnceLock<Mutex<HashMap<String, mpsc::SyncSender<String>>>> = OnceLock::new();

fn sessions() -> &'static Mutex<HashMap<String, mpsc::SyncSender<String>>> {
    SSE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers an active SSE session sender keyed by `session_id`.
pub fn register_session(session_id: String, sender: mpsc::SyncSender<String>) {
    if let Ok(mut lock) = sessions().lock() {
        lock.insert(session_id, sender);
    }
}

/// Removes an active SSE session sender.
pub fn remove_session(session_id: &str) {
    if let Ok(mut lock) = sessions().lock() {
        lock.remove(session_id);
    }
}

/// Sends a JSON-RPC message to an active SSE session stream.
pub fn send_to_session(session_id: &str, message: String) -> bool {
    let sender = sessions()
        .lock()
        .ok()
        .and_then(|lock| lock.get(session_id).cloned());
    let Some(sender) = sender else {
        return false;
    };

    match sender.try_send(message) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_)) | Err(mpsc::TrySendError::Disconnected(_)) => {
            remove_session(session_id);
            false
        }
    }
}

/// Checks whether an active SSE session exists for the given ID.
pub fn has_session(session_id: &str) -> bool {
    if let Ok(lock) = sessions().lock() {
        lock.contains_key(session_id)
    } else {
        false
    }
}

/// Extracts an MCP session ID from request query parameters or headers.
pub fn extract_session_id(request: &Request) -> Option<String> {
    if let Some((_, query)) = request.path.split_once('?') {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k.eq_ignore_ascii_case("sessionid") || k.eq_ignore_ascii_case("session_id") {
                    return Some(v.to_string());
                }
            }
        }
    }
    for (k, v) in &request.headers {
        if k.eq_ignore_ascii_case("mcp-session-id") {
            return Some(v.clone());
        }
    }
    None
}

/// Handles an inbound SSE connection, emitting initial endpoint handshake and streaming events.
pub fn handle_sse_stream(client: &mut TcpStream, cors_header: &str) -> io::Result<u16> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::sync_channel(SSE_QUEUE_CAPACITY);
    register_session(session_id.clone(), tx);

    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache, no-transform\r\nConnection: keep-alive\r\n{}Access-Control-Allow-Headers: Content-Type, Authorization, Mcp-Session-Id\r\n\r\n",
        cors_header
    );
    // Initial MCP SSE endpoint announcement event:
    let endpoint_event = format!("event: endpoint\r\ndata: /mcp?sessionId={session_id}\r\n\r\n");
    let initial_response = format!("{head}{endpoint_event}");
    if let Err(error) = client
        .write_all(initial_response.as_bytes())
        .and_then(|_| client.flush())
    {
        remove_session(&session_id);
        return Err(error);
    }

    loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(msg) => {
                let event = format!("event: message\r\ndata: {msg}\r\n\r\n");
                if client.write_all(event.as_bytes()).is_err() || client.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if client.write_all(b": keep-alive\r\n\r\n").is_err() || client.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    remove_session(&session_id);
    Ok(200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_session_registry_lifecycle() {
        let sid = "test-session-123".to_string();
        let (tx, rx) = mpsc::sync_channel(SSE_QUEUE_CAPACITY);
        register_session(sid.clone(), tx);

        assert!(has_session(&sid));
        assert!(send_to_session(&sid, "{\"jsonrpc\":\"2.0\"}".to_string()));

        let received = rx.recv_timeout(Duration::from_millis(50)).unwrap();
        assert_eq!(received, "{\"jsonrpc\":\"2.0\"}");

        remove_session(&sid);
        assert!(!has_session(&sid));
        assert!(!send_to_session(&sid, "test".to_string()));
    }

    #[test]
    fn full_session_queue_is_disconnected_without_blocking() {
        let sid = format!("full-session-{}", uuid::Uuid::new_v4());
        let (tx, _rx) = mpsc::sync_channel(1);
        register_session(sid.clone(), tx);

        assert!(send_to_session(&sid, "first".to_string()));
        assert!(!send_to_session(&sid, "second".to_string()));
        assert!(!has_session(&sid));
    }

    #[test]
    fn test_extract_session_id_query_and_headers() {
        let req1 = Request {
            method: "POST".to_string(),
            path: "/mcp?sessionId=session-abc-456".to_string(),
            headers: vec![],
            body: vec![],
        };
        assert_eq!(
            extract_session_id(&req1),
            Some("session-abc-456".to_string())
        );

        let req2 = Request {
            method: "POST".to_string(),
            path: "/mcp?other=1&session_id=session-def-789".to_string(),
            headers: vec![],
            body: vec![],
        };
        assert_eq!(
            extract_session_id(&req2),
            Some("session-def-789".to_string())
        );

        let req3 = Request {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers: vec![("Mcp-Session-Id".to_string(), "hdr-session-001".to_string())],
            body: vec![],
        };
        assert_eq!(
            extract_session_id(&req3),
            Some("hdr-session-001".to_string())
        );

        let req4 = Request {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers: vec![],
            body: vec![],
        };
        assert_eq!(extract_session_id(&req4), None);
    }
}
