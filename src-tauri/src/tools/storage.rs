use std::path::{Path, PathBuf};
use std::fs;

/// Result of persist_if_large. Either inline (small) or persisted (large).
pub enum ToolResultContent {
    /// Small enough to include directly in context.
    Inline(String),
    /// Too large — saved to disk. Model gets a preview and a path.
    Persisted {
        full_size_bytes: usize,
        preview: String,
        persisted_path: PathBuf,
    },
}

impl ToolResultContent {
    /// Convert to the string that gets placed in tool_result.content.
    /// For persisted results, returns a structured text that the model can read.
    pub fn to_content_string(&self) -> String {
        match self {
            Self::Inline(s) => s.clone(),
            Self::Persisted {
                full_size_bytes,
                preview,
                persisted_path,
            } => {
                format!(
                    "<truncated-output>\n\
                     Full output ({full_size_bytes} bytes) saved to: {}\n\n\
                     Use read_file with offset/limit to access specific sections.\n\n\
                     Preview (first ~2000 bytes):\n\
                     {preview}\n\
                     </truncated-output>",
                    persisted_path.display()
                )
            }
        }
    }

    /// True if this result is an inline (non-persisted) result.
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline(_))
    }
}

/// If the result exceeds the threshold, persist it to disk and return a preview.
/// Returns is_error: false — a large result is NOT a failure.
pub fn persist_if_large(
    result: &str,
    max_bytes: Option<usize>,
    session_dir: &Path,
    tool_use_id: &str,
) -> ToolResultContent {
    let Some(limit) = max_bytes else {
        return ToolResultContent::Inline(result.to_string());
    };

    if result.len() <= limit {
        return ToolResultContent::Inline(result.to_string());
    }

    // Persist to disk
    let tool_results_dir = session_dir.join("tool-results");
    if fs::create_dir_all(&tool_results_dir).is_err() {
        // Fallback: truncate inline if we can't write to disk
        let preview = safe_truncate(result, limit);
        return ToolResultContent::Inline(format!(
            "[output truncated to {limit} bytes]\n{preview}"
        ));
    }

    let path = tool_results_dir.join(format!("{tool_use_id}.txt"));
    if fs::write(&path, result).is_err() {
        let preview = safe_truncate(result, limit);
        return ToolResultContent::Inline(format!(
            "[output truncated to {limit} bytes]\n{preview}"
        ));
    }

    let preview = safe_truncate(result, 2000).to_string();

    ToolResultContent::Persisted {
        full_size_bytes: result.len(),
        preview,
        persisted_path: path,
    }
}

/// Truncate to the last char boundary at or before max_bytes.
/// Never panics on UTF-8 input — never cuts in the middle of a multi-byte char.
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the last valid char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello world", 5), "hello");
    }

    #[test]
    fn safe_truncate_does_not_cut_utf8_char() {
        // "é" is 2 bytes (0xC3 0xA9). Truncating at 1 must back up to 0.
        let s = "aéb";
        let truncated = safe_truncate(s, 2); // "é" starts at byte 1, ends at byte 3
        assert!(s.is_char_boundary(truncated.len()));
        assert_eq!(truncated, "a");
    }
}
