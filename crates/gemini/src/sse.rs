//! Server-sent events, decoded incrementally from raw bytes.
//!
//! Push bytes in as they arrive, pull `data:` payloads out as frames complete.
//! A frame ends at a blank line, and a blank line is `\n\n` **or** `\r\n\r\n`:
//! Gemini itself sends CRLF, and browser `fetch` surfaces CRLF too, so an
//! LF-only decoder silently never yields a frame. At end of stream the last
//! frame need not be terminated — `finish` flushes it.

#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Bytes before this index have been consumed as frames.
    head: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.head > 0 && self.head * 2 > self.buf.len() {
            self.buf.drain(..self.head);
            self.head = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame's `data:` payload, if one is buffered. Frames
    /// without data (comments, heartbeats, `event:` only) are skipped.
    pub fn next(&mut self) -> Option<String> {
        loop {
            let bytes = &self.buf[self.head..];
            let mut end = None;
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"\r\n\r\n") {
                    end = Some((i, 4));
                    break;
                }
                if bytes[i..].starts_with(b"\n\n") {
                    end = Some((i, 2));
                    break;
                }
                i += 1;
            }
            let (at, len) = end?;
            let payload = data_of(&bytes[..at]);
            self.head += at + len;
            if !payload.is_empty() {
                return Some(payload);
            }
        }
    }

    /// Flush whatever remains as one last frame. Call once at end of stream.
    pub fn finish(&mut self) -> Option<String> {
        let payload = data_of(&self.buf[self.head..]);
        self.head = self.buf.len();
        if payload.is_empty() {
            None
        } else {
            Some(payload)
        }
    }
}

/// Join every `data:` line of a frame with `\n`, per the WHATWG algorithm.
fn data_of(frame: &[u8]) -> String {
    let mut out = String::new();
    for line in frame.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(rest) = line.strip_prefix(b"data:") else {
            continue;
        };
        let rest = rest.strip_prefix(b" ").unwrap_or(rest);
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&String::from_utf8_lossy(rest));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_lf_frames_split_across_pushes() {
        let mut d = Decoder::new();
        d.push(b"data: {\"a\":1}\r\n\r\ndata: {\"b\"");
        assert_eq!(d.next().as_deref(), Some("{\"a\":1}"));
        assert_eq!(d.next(), None);
        d.push(b":2}\n\n: comment\n\nevent: ping\n\ndata: x\ndata: y\n\n");
        assert_eq!(d.next().as_deref(), Some("{\"b\":2}"));
        assert_eq!(d.next().as_deref(), Some("x\ny"));
        assert_eq!(d.next(), None);
        d.push(b"data:last");
        assert_eq!(d.next(), None);
        assert_eq!(d.finish().as_deref(), Some("last"));
        assert_eq!(d.finish(), None);
    }
}
