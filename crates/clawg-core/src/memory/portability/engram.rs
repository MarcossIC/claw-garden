//! The adapter for engram's own export (`engram export`): its JSON file, in
//! versions 0.1.0 and 0.2.0, read into the notes of [`super::foreign`].

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

use super::foreign::{parse_utc, ForeignError, ForeignExport, ForeignNote, ForeignRelation, NoteKind, When};

const SOURCE: &str = "engram";
const VERSIONS: &[&str] = &["0.1.0", "0.2.0"];

/// Top-level keys that describe the file, not what it holds.
const FILE_KEYS: [&str; 2] = ["version", "exported_at"];
/// A relation's own id: the file refers to relations by it, nothing here does.
const RELATION_ID: &str = "sync_id";

/// Reads an engram export. Lists the file leaves out or writes as `null` are
/// empty; every field the adapter does not read is counted in `unplaced` by
/// name instead of being dropped unseen.
pub fn read(text: &str) -> Result<ForeignExport, ForeignError> {
    let value: Value = serde_json::from_str(text).map_err(|e| ForeignError::NotJson {
        format: SOURCE,
        cause: e.to_string(),
    })?;
    let Value::Object(object) = &value else {
        return Err(ForeignError::NotAnObject { format: SOURCE });
    };
    let version = match object.get("version") {
        Some(Value::String(version)) => version.clone(),
        _ => return Err(ForeignError::NoVersion { format: SOURCE }),
    };
    if !VERSIONS.contains(&version.as_str()) {
        return Err(ForeignError::UnknownVersion {
            format: SOURCE,
            found: version,
            known: VERSIONS,
        });
    }
    let file: File = serde_json::from_value(value).map_err(|e| ForeignError::Malformed {
        format: SOURCE,
        version: version.clone(),
        cause: e.to_string(),
    })?;
    Ok(file.into_export(version))
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    sessions: Option<Vec<Session>>,
    #[serde(default)]
    observations: Option<Vec<Observation>>,
    #[serde(default)]
    prompts: Option<Vec<Prompt>>,
    #[serde(default)]
    relations: Option<Vec<Relation>>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Session {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Observation {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    sync_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    topic_key: Option<String>,
    #[serde(default)]
    revision_count: i64,
    #[serde(default)]
    duplicate_count: i64,
    #[serde(default)]
    last_seen_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    deleted_at: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Prompt {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    sync_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Relation {
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    relation: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    judgment_status: Option<String>,
    #[serde(default)]
    marked_by_kind: Option<String>,
    #[serde(default)]
    superseded_at: Option<String>,
    #[serde(default)]
    superseded_by_relation_sync_id: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl File {
    fn into_export(self, version: String) -> ForeignExport {
        let sessions = self.sessions.unwrap_or_default();
        let session_project: HashMap<&str, &str> = sessions
            .iter()
            .filter_map(|s| Some((s.id.as_deref()?, filled(&s.project)?)))
            .collect();
        let project_of = |own: &Option<String>, session: &Option<String>| -> Option<String> {
            filled(own)
                .or_else(|| session_project.get(filled(session)?).copied())
                .map(str::to_owned)
        };

        let mut unplaced = BTreeMap::new();
        let mut notes = Vec::new();
        let mut rewritten = 0;

        for o in self.observations.unwrap_or_default() {
            if o.revision_count > 1 {
                rewritten += 1;
            }
            count_extra(&mut unplaced, "observation", &o.extra);
            let origin = match filled(&o.sync_id) {
                Some(sync_id) => format!("engram:{sync_id}"),
                None => format!("engram:observation:{}", o.id),
            };
            notes.push(ForeignNote {
                origin,
                kind: NoteKind::Observation,
                source_type: o.kind.clone(),
                title: o.title.clone().unwrap_or_default(),
                content: o.content.clone().unwrap_or_default(),
                project: project_of(&o.project, &o.session_id),
                personal: o.scope.as_deref() == Some("personal"),
                topic_key: filled(&o.topic_key).map(str::to_owned),
                session: filled(&o.session_id).map(str::to_owned),
                written: when(filled(&o.updated_at).or(filled(&o.created_at))),
                seen: u64::try_from(o.duplicate_count).unwrap_or(0),
                last_seen: filled(&o.last_seen_at).map(|t| when(Some(t))),
                deleted: filled(&o.deleted_at).map(|t| when(Some(t))),
                pinned: o.pinned,
            });
        }

        for p in self.prompts.unwrap_or_default() {
            count_extra(&mut unplaced, "prompt", &p.extra);
            let origin = match filled(&p.sync_id) {
                Some(sync_id) => format!("engram:{sync_id}"),
                None => format!("engram:prompt:{}", p.id),
            };
            notes.push(ForeignNote {
                origin,
                kind: NoteKind::Prompt,
                source_type: None,
                title: String::new(),
                content: p.content.clone().unwrap_or_default(),
                project: project_of(&p.project, &p.session_id),
                personal: false,
                topic_key: None,
                session: filled(&p.session_id).map(str::to_owned),
                written: when(filled(&p.created_at)),
                seen: 0,
                last_seen: None,
                deleted: None,
                pinned: false,
            });
        }

        let mut discarded_sessions = 0;
        for s in &sessions {
            count_extra(&mut unplaced, "session", &s.extra);
            let (Some(id), Some(summary)) = (filled(&s.id), s.summary.as_deref().filter(|t| !t.trim().is_empty()))
            else {
                discarded_sessions += 1;
                continue;
            };
            notes.push(ForeignNote {
                origin: format!("engram:session:{id}"),
                kind: NoteKind::SessionSummary,
                source_type: None,
                title: String::new(),
                content: summary.to_owned(),
                project: filled(&s.project).map(str::to_owned),
                personal: false,
                topic_key: None,
                session: Some(id.to_owned()),
                written: when(filled(&s.ended_at).or(filled(&s.started_at))),
                seen: 0,
                last_seen: None,
                deleted: None,
                pinned: false,
            });
        }

        let mut relations = Vec::new();
        for r in self.relations.unwrap_or_default() {
            count_extra(&mut unplaced, "relation", &r.extra);
            relations.push(ForeignRelation {
                source: format!("engram:{}", r.source_id.unwrap_or_default()),
                target: format!("engram:{}", r.target_id.unwrap_or_default()),
                verb: r.relation.unwrap_or_default(),
                judgment: r.judgment_status.unwrap_or_default(),
                by_user: r.marked_by_kind.as_deref() == Some("human"),
                reason: filled(&r.reason).map(str::to_owned),
                evidence: filled(&r.evidence).map(str::to_owned),
                created: when(filled(&r.created_at)),
                updated: when(filled(&r.updated_at)),
                superseded: filled(&r.superseded_at).is_some() || filled(&r.superseded_by_relation_sync_id).is_some(),
            });
        }

        for (key, value) in &self.extra {
            if FILE_KEYS.contains(&key.as_str()) || is_empty(value) {
                continue;
            }
            let entries = match value {
                Value::Array(list) => list.len() as u64,
                _ => 1,
            };
            *unplaced.entry(key.clone()).or_insert(0) += entries;
        }

        ForeignExport {
            source: SOURCE,
            version,
            notes,
            relations,
            unplaced,
            rewritten,
            discarded_sessions,
        }
    }
}

/// The text, unless it is missing or blank.
fn filled(text: &Option<String>) -> Option<&str> {
    text.as_deref().filter(|t| !t.trim().is_empty())
}

fn when(text: Option<&str>) -> When {
    text.and_then(parse_utc).map_or(When::Unreadable, When::At)
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(list) => list.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn count_extra(unplaced: &mut BTreeMap<String, u64>, entity: &str, extra: &BTreeMap<String, Value>) {
    for (field, value) in extra {
        if is_empty(value) || (entity == "relation" && field == RELATION_ID) {
            continue;
        }
        *unplaced.entry(format!("{entity}.{field}")).or_insert(0) += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::foreign::{ForeignNote, NoteKind, When};
    use super::*;

    fn note<'a>(export: &'a ForeignExport, origin: &str) -> &'a ForeignNote {
        export
            .notes
            .iter()
            .find(|n| n.origin == origin)
            .unwrap_or_else(|| panic!("no note {origin}"))
    }

    const V1: &str = r#"{
        "version": "0.1.0",
        "exported_at": "2025-03-01 10:00:00",
        "sessions": [
            {"id": "s1", "project": "alpha", "directory": "", "started_at": "2025-01-01 09:00:00", "summary": "Wired the importer"},
            {"id": "s2", "project": "beta", "started_at": "2025-01-01 09:30:00"}
        ],
        "observations": [
            {"id": 1, "sync_id": "obs-1", "session_id": "s1", "type": "decision", "title": "Use SQLite",
             "content": "WAL on", "project": "alpha", "scope": "project", "topic_key": "storage/engine",
             "revision_count": 1, "duplicate_count": 2, "last_seen_at": "2025-01-02 10:00:00",
             "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 11:00:00"},
            {"id": 2, "sync_id": "", "session_id": "s1", "type": "manual", "title": "Mine", "content": "private",
             "project": null, "scope": "personal", "topic_key": "", "revision_count": 1, "duplicate_count": 1,
             "last_seen_at": null, "created_at": "2025-01-03 10:00:00", "updated_at": ""}
        ],
        "prompts": [
            {"id": 7, "sync_id": "pr-7", "session_id": "s1", "content": "how does it work?", "project": "alpha",
             "created_at": "2025-01-04 10:00:00"},
            {"id": 8, "sync_id": "", "session_id": "s2", "content": "", "project": "", "created_at": "2025-01-05 10:00:00"}
        ]
    }"#;

    #[test]
    fn an_unknown_or_missing_version_is_refused() {
        let error = read(r#"{"version": "0.3.0", "observations": []}"#).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("0.3.0") && message.contains("0.1.0") && message.contains("0.2.0"), "{message}");
        assert!(matches!(read(r#"{"observations": []}"#), Err(ForeignError::NoVersion { .. })));
        assert!(matches!(read(r#"{"version": 2}"#), Err(ForeignError::NoVersion { .. })));
        assert!(matches!(read("not json"), Err(ForeignError::NotJson { .. })));
        assert!(matches!(read("[]"), Err(ForeignError::NotAnObject { .. })));
        assert!(matches!(
            read(r#"{"version": "0.2.0", "observations": {"id": 1}}"#),
            Err(ForeignError::Malformed { .. })
        ));
    }

    #[test]
    fn a_0_1_0_export_reads_its_notes_prompts_and_session_summaries() {
        let export = read(V1).unwrap();
        assert_eq!((export.source, export.version.as_str()), ("engram", "0.1.0"));
        let origins: Vec<_> = export.notes.iter().map(|n| n.origin.as_str()).collect();
        assert_eq!(
            origins,
            ["engram:obs-1", "engram:observation:2", "engram:pr-7", "engram:prompt:8", "engram:session:s1"]
        );

        let first = note(&export, "engram:obs-1");
        assert_eq!(first.kind, NoteKind::Observation);
        assert_eq!(first.source_type.as_deref(), Some("decision"));
        assert_eq!((first.title.as_str(), first.content.as_str()), ("Use SQLite", "WAL on"));
        assert_eq!(first.project.as_deref(), Some("alpha"));
        assert!(!first.personal && !first.pinned);
        assert_eq!(first.topic_key.as_deref(), Some("storage/engine"));
        assert_eq!(first.session.as_deref(), Some("s1"));
        assert_eq!(first.seen, 2);
        assert_eq!(first.last_seen, Some(When::At(1_735_812_000_000)));
        assert_eq!(first.deleted, None);

        let second = note(&export, "engram:observation:2");
        assert!(second.personal);
        assert_eq!(second.topic_key, None);
        assert_eq!(second.last_seen, None);
        assert_eq!(second.written, When::At(1_735_898_400_000));

        let prompt = note(&export, "engram:pr-7");
        assert_eq!(prompt.kind, NoteKind::Prompt);
        assert_eq!((prompt.source_type.clone(), prompt.title.as_str()), (None, ""));
        assert_eq!(prompt.content, "how does it work?");
        assert!(!prompt.personal);
        let empty = note(&export, "engram:prompt:8");
        assert_eq!((empty.content.as_str(), empty.project.as_deref()), ("", Some("beta")));

        let summary = note(&export, "engram:session:s1");
        assert_eq!(summary.kind, NoteKind::SessionSummary);
        assert_eq!(summary.content, "Wired the importer");
        assert_eq!(summary.project.as_deref(), Some("alpha"));
        assert_eq!(summary.session.as_deref(), Some("s1"));
        assert_eq!(summary.written, When::At(1_735_722_000_000));

        assert_eq!(export.discarded_sessions, 1);
        assert!(export.relations.is_empty());
        assert!(export.unplaced.is_empty(), "{:?}", export.unplaced);
    }

    #[test]
    fn a_0_2_0_export_reads_deletions_pins_and_relations() {
        let export = read(
            r#"{
            "version": "0.2.0",
            "exported_at": "2025-03-01 10:00:00.123456789",
            "sessions": [{"id": "s1", "project": "alpha", "started_at": "2025-01-01 09:00:00",
                          "ended_at": "2025-01-01 12:00:00.5", "summary": "  "}],
            "observations": [
                {"id": 1, "sync_id": "a", "session_id": "s1", "type": "bugfix", "title": "t", "content": "c",
                 "project": "alpha", "scope": "project", "revision_count": 1, "duplicate_count": 1, "pinned": true,
                 "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
                 "deleted_at": "2025-02-01 08:00:00.999999999"},
                {"id": 2, "sync_id": "b", "session_id": "s1", "type": "bugfix", "title": "t2", "content": "c2",
                 "project": "alpha", "scope": "project", "revision_count": 1, "duplicate_count": 1, "pinned": false,
                 "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00"}
            ],
            "prompts": [],
            "relations": [
                {"sync_id": "r1", "source_id": "a", "target_id": "b", "relation": "supersedes",
                 "reason": "newer", "evidence": "e", "judgment_status": "judged", "marked_by_kind": "human",
                 "created_at": "2025-02-01 08:00:00.5", "updated_at": "2025-02-01T08:00:00Z"},
                {"sync_id": "r2", "source_id": "b", "target_id": "a", "relation": "related",
                 "judgment_status": "pending", "marked_by_kind": "system",
                 "superseded_at": "2025-02-02 08:00:00", "superseded_by_relation_sync_id": "r1",
                 "created_at": "2025-02-01 08:00:00", "updated_at": "2025-02-01 08:00:00"}
            ]
        }"#,
        )
        .unwrap();
        assert_eq!(export.version, "0.2.0");
        let deleted = note(&export, "engram:a");
        assert!(deleted.pinned);
        assert_eq!(deleted.deleted, Some(When::At(1_738_396_800_999)));
        assert!(!note(&export, "engram:b").pinned);
        assert_eq!(export.discarded_sessions, 1);
        assert_eq!(export.notes.len(), 2);

        let [human, system] = export.relations.as_slice() else { panic!("two relations") };
        assert_eq!((human.source.as_str(), human.target.as_str()), ("engram:a", "engram:b"));
        assert_eq!((human.verb.as_str(), human.judgment.as_str()), ("supersedes", "judged"));
        assert!(human.by_user && !human.superseded);
        assert_eq!((human.reason.as_deref(), human.evidence.as_deref()), (Some("newer"), Some("e")));
        assert_eq!(human.created, When::At(1_738_396_800_500));
        assert_eq!(human.updated, When::At(1_738_396_800_000));
        assert!(!system.by_user && system.superseded);
        assert_eq!((system.reason.clone(), system.evidence.clone()), (None, None));
        assert!(export.unplaced.is_empty(), "{:?}", export.unplaced);
    }

    #[test]
    fn null_lists_read_as_empty() {
        let export = read(r#"{"version": "0.2.0", "sessions": null, "observations": [], "prompts": null}"#).unwrap();
        assert!(export.notes.is_empty() && export.relations.is_empty());
        assert_eq!(export.discarded_sessions, 0);
        assert!(export.unplaced.is_empty());
        let bare = read(r#"{"version": "0.1.0"}"#).unwrap();
        assert!(bare.notes.is_empty());
    }

    #[test]
    fn fields_without_a_place_are_counted_by_name() {
        let export = read(
            r#"{
            "version": "0.2.0",
            "sessions": [{"id": "s1", "project": "p", "directory": "/work", "ownership_mode": "shared",
                          "started_at": "2025-01-01 09:00:00", "summary": null}],
            "observations": [
                {"id": 1, "sync_id": "a", "session_id": "s1", "type": "manual", "title": "t", "content": "c",
                 "project": "p", "scope": "project", "revision_count": 1, "duplicate_count": 1, "pinned": false,
                 "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00",
                 "tool_name": "mem_save", "review_after": "2026-01-01 00:00:00", "superseded_by_sync_id": "z",
                 "mood": "calm", "empty_text": "", "nothing": null, "no_list": [], "no_map": {}},
                {"id": 2, "sync_id": "b", "session_id": "s1", "type": "manual", "title": "t", "content": "c",
                 "project": "p", "scope": "project", "revision_count": 1, "duplicate_count": 1, "pinned": false,
                 "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00", "tool_name": "mem_update"}
            ],
            "prompts": [{"id": 3, "sync_id": "p", "session_id": "s1", "content": "q", "project": "p",
                         "created_at": "2025-01-01 10:00:00", "source_inbox_id": "inbox-1"}],
            "relations": [{"sync_id": "r", "source_id": "a", "target_id": "b", "relation": "related",
                           "judgment_status": "pending", "marked_by_kind": "system", "confidence": 0.8,
                           "marked_by_actor": "claude", "marked_by_model": "m", "session_id": "s1",
                           "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00"}],
            "prompt_tombstones": [{"sync_id": "x"}, {"sync_id": "y"}]
        }"#,
        )
        .unwrap();
        let counted: Vec<(&str, u64)> = export.unplaced.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(
            counted,
            [
                ("observation.mood", 1),
                ("observation.review_after", 1),
                ("observation.superseded_by_sync_id", 1),
                ("observation.tool_name", 2),
                ("prompt.source_inbox_id", 1),
                ("prompt_tombstones", 2),
                ("relation.confidence", 1),
                ("relation.marked_by_actor", 1),
                ("relation.marked_by_model", 1),
                ("relation.session_id", 1),
                ("session.directory", 1),
                ("session.ownership_mode", 1),
            ]
        );
        assert_eq!(export.notes.len(), 3);
        assert_eq!(note(&export, "engram:a").content, "c");
    }

    #[test]
    fn notes_rewritten_at_the_source_are_counted() {
        let export = read(
            r#"{"version": "0.2.0", "observations": [
                {"id": 1, "sync_id": "a", "type": "t", "title": "x", "content": "y", "revision_count": 1},
                {"id": 2, "sync_id": "b", "type": "t", "title": "x", "content": "y", "revision_count": 2},
                {"id": 3, "sync_id": "c", "type": "t", "title": "x", "content": "y", "revision_count": 5},
                {"id": 4, "sync_id": "d", "type": "t", "title": "x", "content": "y"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(export.rewritten, 2);
        assert_eq!(export.notes.len(), 4);
    }

    #[test]
    fn a_note_without_a_project_takes_its_sessions() {
        let export = read(
            r#"{"version": "0.2.0",
            "sessions": [{"id": "s1", "project": "alpha", "started_at": "2025-01-01 09:00:00"},
                         {"id": "s2", "project": "  ", "started_at": "2025-01-01 09:00:00"}],
            "observations": [
                {"id": 1, "sync_id": "a", "session_id": "s1", "type": "t", "title": "x", "content": "y", "project": null},
                {"id": 2, "sync_id": "b", "session_id": "s1", "type": "t", "title": "x", "content": "y", "project": ""},
                {"id": 3, "sync_id": "c", "session_id": "s1", "type": "t", "title": "x", "content": "y", "project": "own"},
                {"id": 4, "sync_id": "d", "session_id": "s2", "type": "t", "title": "x", "content": "y", "project": ""},
                {"id": 5, "sync_id": "e", "session_id": "gone", "type": "t", "title": "x", "content": "y"}
            ],
            "prompts": [{"id": 1, "sync_id": "p", "session_id": "s1", "content": "q", "project": "", "created_at": "2025-01-01 10:00:00"}]
        }"#,
        )
        .unwrap();
        let project = |origin: &str| note(&export, origin).project.clone();
        assert_eq!(project("engram:a").as_deref(), Some("alpha"));
        assert_eq!(project("engram:b").as_deref(), Some("alpha"));
        assert_eq!(project("engram:c").as_deref(), Some("own"));
        assert_eq!(project("engram:d"), None);
        assert_eq!(project("engram:e"), None);
        assert_eq!(project("engram:p").as_deref(), Some("alpha"));
    }

    #[test]
    fn dates_read_as_utc() {
        let export = read(
            r#"{"version": "0.2.0", "observations": [
                {"id": 1, "sync_id": "a", "type": "t", "title": "x", "content": "y",
                 "created_at": "2025-01-01 10:00:00", "updated_at": "2025-01-01 10:00:00.987654321"},
                {"id": 2, "sync_id": "b", "type": "t", "title": "x", "content": "y",
                 "created_at": "2025-01-01 10:00:00", "updated_at": ""},
                {"id": 3, "sync_id": "c", "type": "t", "title": "x", "content": "y",
                 "created_at": "2025-01-01T12:00:00+02:00", "updated_at": "2025-01-01T12:00:00Z"},
                {"id": 4, "sync_id": "d", "type": "t", "title": "x", "content": "y",
                 "created_at": "2025-01-01 10:00:00", "updated_at": "last tuesday", "last_seen_at": "soon"},
                {"id": 5, "sync_id": "e", "type": "t", "title": "x", "content": "y"}
            ]}"#,
        )
        .unwrap();
        let written = |origin: &str| note(&export, origin).written;
        assert_eq!(written("engram:a"), When::At(1_735_725_600_987));
        assert_eq!(written("engram:b"), When::At(1_735_725_600_000));
        assert_eq!(written("engram:c"), When::At(1_735_732_800_000));
        assert_eq!(written("engram:d"), When::Unreadable);
        assert_eq!(note(&export, "engram:d").last_seen, Some(When::Unreadable));
        assert_eq!(written("engram:e"), When::Unreadable);
    }
}
