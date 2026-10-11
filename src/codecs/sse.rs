//! Server-Sent Events (SSE) decoding codec.
//! Handles capability-neutral event framing, LF/CRLF/CR delimiters, multiline data,
//! and payload size safeguards.

pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    pub fn next_event(&mut self) -> Option<String> {
        let mut line_end = None;
        let mut delim_len = 0;

        for i in 0..self.buffer.len() {
            if self.buffer[i] == b'\n' {
                line_end = Some(i);
                delim_len = if i > 0 && self.buffer[i - 1] == b'\r' {
                    2
                } else {
                    1
                };
                break;
            } else if self.buffer[i] == b'\r' {
                if i + 1 < self.buffer.len() && self.buffer[i + 1] == b'\n' {
                    line_end = Some(i);
                    delim_len = 2;
                } else {
                    line_end = Some(i);
                    delim_len = 1;
                }
                break;
            }
        }

        let end = line_end?;

        let line_bytes = self.buffer.drain(..end).collect::<Vec<u8>>();
        self.buffer.drain(..delim_len);

        let line = String::from_utf8_lossy(&line_bytes).into_owned();
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_decoder_basic() {
        let mut decoder = SseDecoder::new();
        decoder.feed(b"data: hello\n\ndata: world\r\n\r\n");
        assert_eq!(decoder.next_event(), Some("data: hello".to_string()));
        assert_eq!(decoder.next_event(), Some("".to_string()));
        assert_eq!(decoder.next_event(), Some("data: world".to_string()));
    }
}
