//! The topic key a deliberate save is a version of.
//!
//! A save that names a key replaces the live rows its agent saved under the
//! same key in the same project; the replaced rows stay, `superseded` by the
//! new save, as its history.

/// Longest key accepted, in characters.
pub const MAX_TOPIC_KEY_LEN: usize = 80;

/// Why a `topic_key` is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopicKeyError {
    #[error("topic_key must be 1 to {MAX_TOPIC_KEY_LEN} characters")]
    Length,
    #[error("topic_key is segments of a-z, 0-9, '.', '_' or '-' joined by '/', with no empty segment")]
    Shape,
}

/// Checks a key: lowercase segments of `[a-z0-9._-]` joined by `/`, none
/// empty, 1 to [`MAX_TOPIC_KEY_LEN`] characters in all. Lowercase only, so
/// one topic written twice in another case is still one key.
pub fn validate_topic_key(key: &str) -> Result<(), TopicKeyError> {
    if key.is_empty() || key.chars().count() > MAX_TOPIC_KEY_LEN {
        return Err(TopicKeyError::Length);
    }
    let segment_ok = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    };
    if key.split('/').all(segment_ok) {
        Ok(())
    } else {
        Err(TopicKeyError::Shape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_key_cases() {
        let long = "a".repeat(MAX_TOPIC_KEY_LEN);
        let too_long = "a".repeat(MAX_TOPIC_KEY_LEN + 1);
        let cases: &[(&str, Result<(), TopicKeyError>)] = &[
            ("storage", Ok(())),
            ("storage/engine", Ok(())),
            ("clawg-core/ingest.outbox_v2", Ok(())),
            ("0.18/release", Ok(())),
            (&long, Ok(())),
            ("", Err(TopicKeyError::Length)),
            (&too_long, Err(TopicKeyError::Length)),
            ("Storage", Err(TopicKeyError::Shape)),
            ("storage engine", Err(TopicKeyError::Shape)),
            ("/storage", Err(TopicKeyError::Shape)),
            ("storage/", Err(TopicKeyError::Shape)),
            ("storage//engine", Err(TopicKeyError::Shape)),
            ("almacenamiento/motor-ñ", Err(TopicKeyError::Shape)),
            ("storage\\engine", Err(TopicKeyError::Shape)),
        ];
        for (key, expected) in cases {
            assert_eq!(&validate_topic_key(key), expected, "{key:?}");
        }
    }

    #[test]
    fn the_error_names_the_field() {
        for e in [TopicKeyError::Length, TopicKeyError::Shape] {
            assert!(e.to_string().starts_with("topic_key "), "{e}");
        }
    }
}
