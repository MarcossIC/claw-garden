/// Shared SSE line parser used by all streaming providers.
///
/// Accumulates raw bytes from an HTTP stream and yields complete SSE data
/// payloads one at a time. Handles `data:` prefix stripping, comment lines,
/// blank separators, and the `[DONE]` sentinel.

pub struct SseLineParser {
    buffer: String,
}

impl SseLineParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Push a raw byte chunk from the HTTP stream into the internal buffer.
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
    }

    /// Yield the next complete SSE data payload, or `None` if the buffer
    /// doesn't contain a full line yet.
    ///
    /// Returns `SseLine::Data(payload)` for data lines,
    /// `SseLine::Done` for the `[DONE]` sentinel,
    /// or `None` to keep waiting.
    pub fn next_line(&mut self) -> Option<SseLine> {
        loop {
            let newline_pos = self.buffer.find('\n')?;
            let line = self.buffer[..newline_pos].trim_end().to_string();
            self.buffer = self.buffer[newline_pos + 1..].to_string();

            // Skip blank lines and SSE comments
            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            // SSE data line — handle both "data: value" and "data:value" (both valid per spec)
            let data = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"));
            if let Some(data) = data {
                if data == "[DONE]" {
                    return Some(SseLine::Done);
                }
                return Some(SseLine::Data(data.to_string()));
            }

            // JSONL (no "data:" prefix) — used by Ollama
            // Only return if it looks like JSON to avoid garbage
            if line.starts_with('{') {
                return Some(SseLine::Data(line));
            }
        }
    }
}

pub enum SseLine {
    /// A complete data payload (JSON string).
    Data(String),
    /// The `[DONE]` sentinel — stream is finished.
    Done,
}
