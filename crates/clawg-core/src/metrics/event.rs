//! The rare things a run does once in a while (a model load, a backup, an
//! import), kept one row each beside the minutes.
//!
//! A kind and every label of a detail are `&'static str` from the code, so no
//! text of a memory, a query, a path or an id can reach a row.

use std::collections::BTreeMap;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    ModelLoad,
    ModelUnload,
    EmbedderRecovery,
    ModelDownload,
    Maintenance,
    Backup,
    Import,
    OutboxFailed,
    Shutdown,
}

impl EventKind {
    pub const ALL: [EventKind; 9] = [
        EventKind::ModelLoad,
        EventKind::ModelUnload,
        EventKind::EmbedderRecovery,
        EventKind::ModelDownload,
        EventKind::Maintenance,
        EventKind::Backup,
        EventKind::Import,
        EventKind::OutboxFailed,
        EventKind::Shutdown,
    ];

    /// How the kind is written in `metrics.db` and in the API.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::ModelLoad => "model_load",
            EventKind::ModelUnload => "model_unload",
            EventKind::EmbedderRecovery => "embedder_recovery",
            EventKind::ModelDownload => "model_download",
            EventKind::Maintenance => "maintenance",
            EventKind::Backup => "backup",
            EventKind::Import => "import",
            EventKind::OutboxFailed => "outbox_failed",
            EventKind::Shutdown => "shutdown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == s)
    }
}

/// One value of a detail: a number, or a value of a set the code closes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetailValue {
    Number(f64),
    Label(&'static str),
}

impl From<u64> for DetailValue {
    fn from(n: u64) -> Self {
        DetailValue::Number(n as f64)
    }
}

impl From<usize> for DetailValue {
    fn from(n: usize) -> Self {
        DetailValue::Number(n as f64)
    }
}

impl From<i64> for DetailValue {
    fn from(n: i64) -> Self {
        DetailValue::Number(n as f64)
    }
}

impl From<f64> for DetailValue {
    fn from(n: f64) -> Self {
        DetailValue::Number(n)
    }
}

impl From<&'static str> for DetailValue {
    fn from(label: &'static str) -> Self {
        DetailValue::Label(label)
    }
}

impl From<bool> for DetailValue {
    fn from(yes: bool) -> Self {
        DetailValue::Label(if yes { "yes" } else { "no" })
    }
}

/// An event as the recorder holds it until the writer keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub at_ms: i64,
    pub kind: EventKind,
    pub detail: BTreeMap<&'static str, DetailValue>,
}

impl Event {
    /// The detail as the JSON object `metrics.db` keeps.
    pub fn detail_json(&self) -> String {
        let object: serde_json::Map<String, serde_json::Value> = self
            .detail
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    // A whole number stays one, as a count or milliseconds are.
                    DetailValue::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => serde_json::Value::from(*n as i64),
                    DetailValue::Number(n) => serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, Into::into),
                    DetailValue::Label(label) => serde_json::Value::from(*label),
                };
                (key.to_string(), value)
            })
            .collect();
        serde_json::Value::Object(object).to_string()
    }
}

/// An event read back from `metrics.db`, its detail as it was stored.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub run_id: i64,
    pub at_ms: i64,
    pub kind: String,
    pub detail: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_reads_back_from_its_name() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::parse("something_else"), None);
    }

    #[test]
    fn a_detail_is_a_json_object_of_numbers_and_labels() {
        let event = Event {
            at_ms: 5,
            kind: EventKind::Backup,
            detail: BTreeMap::from([("ms", 12u64.into()), ("reason", "manual".into()), ("done", true.into())]),
        };
        assert_eq!(event.detail_json(), r#"{"done":"yes","ms":12,"reason":"manual"}"#);
    }
}
