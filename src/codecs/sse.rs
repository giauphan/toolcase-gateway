//! Server-Sent Events (SSE) decoding codec.
//! Handles LF/CRLF/CR delimiters, multiline data, arbitrary input chunking,
//! and bounded event payloads.

use std::fmt;

pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseDecodeError {
    EventTooLarge { limit: usize },
    InvalidUtf8,
}

impl fmt::Display for SseDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventTooLarge { limit } => {
                write!(f, "SSE event exceeds maximum size of {limit} bytes")
            }
            Self::InvalidUtf8 => f.write_str("SSE event contains invalid UTF-8"),
        }
    }
}

impl std::error::Error for SseDecodeError {}

pub struct SseDecoder {
    line: Vec<u8>,
    data: String,
    event_bytes: usize,
    saw_data: bool,
    pending_cr: bool,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event_bytes: 0,
            saw_data: false,
            pending_cr: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<String>, SseDecodeError> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    self.process_line(&mut events)?;
                    continue;
                }
                self.process_line(&mut events)?;
            }

            match byte {
                b'\n' => self.process_line(&mut events)?,
                b'\r' => self.pending_cr = true,
                byte => {
                    self.line.push(byte);
                    let pending_size = self.event_bytes.saturating_add(self.line.len());
                    self.check_size(pending_size)?;
                }
            }
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<String>, SseDecodeError> {
        let mut events = Vec::new();
        if self.pending_cr {
            self.pending_cr = false;
            self.process_line(&mut events)?;
        }
        self.line.clear();
        self.reset_event();
        Ok(events)
    }

    fn process_line(&mut self, events: &mut Vec<String>) -> Result<(), SseDecodeError> {
        let line = std::mem::take(&mut self.line);
        self.check_size(self.event_bytes.saturating_add(line.len() + 1))?;
        self.event_bytes += line.len() + 1;

        if line.is_empty() {
            if self.saw_data {
                self.data.pop();
                events.push(std::mem::take(&mut self.data));
            }
            self.reset_event();
            return Ok(());
        }

        let line = std::str::from_utf8(&line).map_err(|_| {
            self.reset_current();
            SseDecodeError::InvalidUtf8
        })?;
        if line.starts_with(':') {
            return Ok(());
        }

        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });
        if field == "data" {
            self.data.push_str(value);
            self.data.push('\n');
            self.saw_data = true;
        }
        Ok(())
    }

    fn check_size(&mut self, size: usize) -> Result<(), SseDecodeError> {
        if size > MAX_EVENT_BYTES {
            self.reset_current();
            return Err(SseDecodeError::EventTooLarge {
                limit: MAX_EVENT_BYTES,
            });
        }
        Ok(())
    }

    fn reset_event(&mut self) {
        self.event_bytes = 0;
        self.saw_data = false;
        self.data.clear();
    }

    fn reset_current(&mut self) {
        self.line.clear();
        self.pending_cr = false;
        self.reset_event();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_complete_events_and_multiline_data() {
        let mut decoder = SseDecoder::new();
        let events = decoder.feed(b"data: first\ndata: second\n\n").unwrap();
        assert_eq!(events, vec!["first\nsecond"]);
    }

    #[test]
    fn ignores_comments_and_non_data_fields() {
        let mut decoder = SseDecoder::new();
        let events = decoder
            .feed(b": heartbeat\nevent: message\ndata: hello\n\n")
            .unwrap();
        assert_eq!(events, vec!["hello"]);
    }

    #[test]
    fn supports_lf_crlf_cr_and_split_crlf_delimiters() {
        let mut decoder = SseDecoder::new();
        assert_eq!(decoder.feed(b"data: lf\n\n").unwrap(), vec!["lf"]);
        assert_eq!(decoder.feed(b"data: crlf\r\n\r\n").unwrap(), vec!["crlf"]);
        assert!(decoder.feed(b"data: cr\r\r").unwrap().is_empty());
        assert_eq!(decoder.finish().unwrap(), vec!["cr"]);
        assert!(decoder.feed(b"data: split\r").unwrap().is_empty());
        assert!(decoder.feed(b"\n\r").unwrap().is_empty());
        assert_eq!(decoder.finish().unwrap(), vec!["split"]);
    }

    #[test]
    fn preserves_utf8_split_across_feeds() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.feed(b"data: \xe2").unwrap().is_empty());
        assert!(decoder.feed(b"\x98").unwrap().is_empty());
        assert_eq!(decoder.feed(b"\x83\n\n").unwrap(), vec!["☃"]);
    }

    #[test]
    fn rejects_oversized_events() {
        let mut decoder = SseDecoder::new();
        let error = decoder.feed(&vec![b'x'; MAX_EVENT_BYTES + 1]).unwrap_err();
        assert_eq!(
            error,
            SseDecodeError::EventTooLarge {
                limit: MAX_EVENT_BYTES
            }
        );
    }
}
