//! The ingest spill file: events an orderly shutdown could not ingest,
//! written to `<data>/ingest-pending.jsonl` and replayed on the next start.
//!
//! One `MemoryEvent` as JSON per line. The line format is pure so it can be
//! tested without a filesystem; the file is replaced atomically so a shutdown
//! that dies mid-write leaves the previous spill whole.

use std::path::Path;

use crate::memory::events::MemoryEvent;

/// Name of the spill file inside the data directory.
pub const SPILL_FILE: &str = "ingest-pending.jsonl";

/// One JSON line per event. JSON escapes newlines inside strings, so an event
/// always fits on one line.
pub(crate) fn to_lines(events: &[MemoryEvent]) -> String {
    let mut out = String::new();
    for event in events {
        // A `MemoryEvent` is plain owned data with string keys; serializing it
        // cannot fail.
        out.push_str(&serde_json::to_string(event).expect("MemoryEvent serializes"));
        out.push('\n');
    }
    out
}

/// Parses spill text into the events it holds and the lines that are not
/// events, both in file order. Blank lines are neither.
pub(crate) fn parse_lines(text: &str) -> (Vec<MemoryEvent>, Vec<String>) {
    let mut events = Vec::new();
    let mut rejected = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(event) => events.push(event),
            Err(_) => rejected.push(line.to_string()),
        }
    }
    (events, rejected)
}

/// Replaces `<base_dir>/ingest-pending.jsonl` with `events`.
pub(crate) async fn write(base_dir: &Path, events: &[MemoryEvent]) -> std::io::Result<()> {
    crate::memory::workspace::write_atomically(&base_dir.join(SPILL_FILE), to_lines(events).as_bytes())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::events::MemorySource;
    use crate::models::chat::Role;

    fn event(content: &str) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-spill".into(),
            project_id: Some("proj".into()),
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 42,
            memory_type: Some("decision".into()),
            importance: Some(8.0),
        }
    }

    fn json(e: &MemoryEvent) -> serde_json::Value {
        serde_json::to_value(e).unwrap()
    }

    #[test]
    fn spill_lines_round_trip_events_with_newlines_and_overrides() {
        let original = event("first line\nsecond line\r\nthird");
        let text = to_lines(std::slice::from_ref(&original));
        assert_eq!(text.lines().count(), 1, "{text}");

        let (events, rejected) = parse_lines(&text);
        assert!(rejected.is_empty());
        assert_eq!(events.len(), 1);
        assert_eq!(json(&events[0]), json(&original));
        assert_eq!(events[0].memory_type.as_deref(), Some("decision"));
        assert_eq!(events[0].importance, Some(8.0));
    }

    #[test]
    fn invalid_spill_lines_are_set_apart_and_the_valid_ones_keep_their_order() {
        let a = event("a");
        let b = event("b");
        let mut text = to_lines(std::slice::from_ref(&a));
        text.push_str("{\"agent_id\": \"trunc\n\n");
        text.push_str(&to_lines(std::slice::from_ref(&b)));

        let (events, rejected) = parse_lines(&text);
        assert_eq!(
            events.iter().map(|e| e.content.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(rejected, vec!["{\"agent_id\": \"trunc".to_string()]);
    }

    #[tokio::test]
    async fn write_replaces_the_spill_file_with_exactly_the_given_events() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), &[event("old")]).await.unwrap();
        write(tmp.path(), &[event("new 1"), event("new 2")]).await.unwrap();

        let text = std::fs::read_to_string(tmp.path().join(SPILL_FILE)).unwrap();
        let (events, rejected) = parse_lines(&text);
        assert!(rejected.is_empty());
        assert_eq!(
            events.iter().map(|e| e.content.as_str()).collect::<Vec<_>>(),
            vec!["new 1", "new 2"]
        );
    }
}
