//! The ingest spill file a previous version wrote: events an orderly
//! shutdown could not ingest, in `<data>/ingest-pending.jsonl`, one
//! `MemoryEvent` as JSON per line. Nothing writes it any more; on start its
//! events move to the outbox and the file goes.

use std::path::{Path, PathBuf};

use crate::memory::events::MemoryEvent;

/// Name of the spill file inside the data directory.
pub const SPILL_FILE: &str = "ingest-pending.jsonl";

/// Where spill lines that do not parse are kept. Nothing deletes it: those
/// lines are someone's saves, and only a person can tell what they were.
pub const REJECTED_FILE: &str = "ingest-pending.rejected.jsonl";

/// The events a spill file held.
pub(crate) struct Spilled {
    pub events: Vec<MemoryEvent>,
    /// The file to remove once the events are kept elsewhere; `None` when
    /// there is none, or when its unparseable lines could not be set aside and
    /// it must stay as their only copy.
    file: Option<PathBuf>,
}

impl Spilled {
    fn none() -> Self {
        Self { events: Vec::new(), file: None }
    }

    /// Whether there is anything to move or remove.
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty() && self.file.is_none()
    }

    /// Removes the spill file, once its events are kept elsewhere.
    pub(crate) async fn remove(self) {
        let Some(file) = self.file else { return };
        match tokio::fs::remove_file(&file).await {
            Ok(()) => log::info!("moved every spilled ingest event from {} to the outbox", file.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("could not remove {} after moving its events: {e}", file.display()),
        }
    }
}

/// Reads `<base_dir>/ingest-pending.jsonl`, moving lines that do not parse to
/// [`REJECTED_FILE`].
pub(crate) async fn load(base_dir: &Path) -> Spilled {
    let path = base_dir.join(SPILL_FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Spilled::none(),
        Err(e) => {
            log::error!("could not read {}, its events are not ingested: {e}", path.display());
            return Spilled::none();
        }
    };
    let (events, rejected) = parse_lines(&text);
    log::info!("moving {} spilled ingest events from {} to the outbox", events.len(), path.display());
    let mut file = Some(path);
    if !rejected.is_empty() {
        let rejected_path = base_dir.join(REJECTED_FILE);
        match append_lines(&rejected_path, &rejected) {
            Ok(()) => log::warn!(
                "{} spilled lines are not ingest events; kept in {}",
                rejected.len(),
                rejected_path.display()
            ),
            Err(e) => {
                log::error!(
                    "could not set aside {} unreadable spilled lines in {}: {e}; keeping the spill file",
                    rejected.len(),
                    rejected_path.display()
                );
                file = None;
            }
        }
    }
    Spilled { events, file }
}

/// Blocking on purpose: it runs once at startup, only when a spill had bad
/// lines, and writes a few lines.
fn append_lines(path: &Path, lines: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let mut text = lines.join("\n");
    text.push('\n');
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()
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
            derived_from: vec![3, 1],
            topic_key: None,
            user_stated: false,
        }
    }

    fn to_lines(events: &[MemoryEvent]) -> String {
        events.iter().map(|e| format!("{}\n", serde_json::to_string(e).unwrap())).collect()
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
        assert_eq!(events[0].derived_from, vec![3, 1]);
    }

    #[test]
    fn a_spill_without_sources_still_parses() {
        let mut previous = json(&event("written before sources existed"));
        previous.as_object_mut().unwrap().remove("derived_from");

        let (events, rejected) = parse_lines(&format!("{previous}\n"));

        assert!(rejected.is_empty());
        assert!(events[0].derived_from.is_empty());
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
}
