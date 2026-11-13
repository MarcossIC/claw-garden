//! What a deliberate save must be before it is accepted.
//!
//! A save is parsed, not validated: [`NewSave::parse`] turns what a client
//! sent into a value that can only hold a save its format allows, with its
//! `memory_type` already a [`MemoryType`], or says which rule it breaks.
//! Every rule is checked before the save is queued: once queued, a refusal
//! could only reach the caller long after it was told the save was accepted.

use crate::memory::domain::events::{MemoryEvent, MemorySource};
use crate::memory::domain::record::{MemoryType, UnknownMemoryType};
use crate::memory::revision::topic::{validate_topic_key, TopicKeyError};
use crate::memory::store::error::MemoryError;
use crate::models::chat::Role;

/// Most bytes a save's `content` may hold. Every piece of a long save
/// rewrites its outbox row with the progress so far, so the bytes a save
/// writes grow with the square of its length; this keeps the worst case near
/// 230 pieces and under 10 MB, and still takes the longest note an imported
/// history was measured to hold (45 372 characters).
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// Longest `operation_id` a caller may send.
pub const MAX_OPERATION_ID_LEN: usize = 128;

/// Most sources one save may name. A conclusion rests on a handful of
/// memories; a list longer than this is a client passing a listing through,
/// and every id costs a lookup before the save is queued.
pub const MAX_SOURCES: usize = 50;

/// A deliberate save as a client sends it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveInput {
    pub content: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub memory_type: Option<String>,
    pub importance: Option<f64>,
    /// Ids of the memories this one is a conclusion drawn from.
    pub derived_from: Vec<i64>,
    /// The content is what the user said, not something the agent inferred.
    pub user_stated: bool,
    /// The caller's id for this save; absent, one is made up when it is accepted.
    pub operation_id: Option<String>,
    /// The topic this save is a version of.
    pub topic_key: Option<String>,
}

/// A save whose format holds, made only by [`NewSave::parse`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewSave {
    pub(crate) content: String,
    pub(crate) agent_id: String,
    pub(crate) project_id: Option<String>,
    pub(crate) memory_type: Option<MemoryType>,
    pub(crate) importance: Option<f64>,
    /// Each id once, ascending.
    pub(crate) derived_from: Vec<i64>,
    pub(crate) user_stated: bool,
    pub(crate) operation_id: Option<String>,
    pub(crate) topic_key: Option<String>,
}

/// Why a save is refused before it is accepted. The text of each is the
/// message the caller reads; the fields carry what it names.
#[derive(Debug, thiserror::Error)]
pub enum SaveRejected {
    #[error("content must not be empty")]
    EmptyContent,
    #[error("content is {bytes} bytes; at most {MAX_CONTENT_BYTES} are accepted: split it into several saves, one fact each")]
    ContentTooLong { bytes: usize },
    #[error(transparent)]
    UnknownType(#[from] UnknownMemoryType),
    #[error(transparent)]
    OperationId(#[from] OperationIdError),
    #[error(transparent)]
    TopicKey(#[from] TopicKeyError),
    #[error("derived_from names {count} memories; at most {MAX_SOURCES} are accepted")]
    TooManySources { count: usize },
    /// A conclusion is the agent's even when its sources are the user's
    /// words; storing it as the agent's after this declaration would hide
    /// the refusal.
    #[error("user_stated cannot go with derived_from: a conclusion is drawn by the agent, not stated by the user")]
    StatedConclusion,
    /// Sources not in the store, deleted, or out of the saver's reach, in
    /// the order the save names them. A hidden row reads as missing, so a
    /// refusal never confirms a row the policy hides.
    #[error("derived_from names memories that do not exist or were deleted: {}", ids_list(.0))]
    MissingSources(Vec<i64>),
    /// Sources that were replaced, each with the row that replaced it when known.
    #[error("derived_from names memories that were replaced: {}; draw the conclusion from the current ones", replaced_list(.0))]
    ReplacedSources(Vec<(i64, Option<i64>)>),
    /// The embedder will not load, so a queued save would wait until the
    /// process stops and then be lost. `retryable` when it may still load.
    #[error("the embedder failed: {error}")]
    EmbedderFailed { error: String, retryable: bool },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

fn ids_list(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
}

/// `5 (by 9), 7` for replaced sources, each with what replaced it when known.
fn replaced_list(replaced: &[(i64, Option<i64>)]) -> String {
    replaced
        .iter()
        .map(|(id, by)| match by {
            Some(by) => format!("{id} (by {by})"),
            None => id.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The requested sources that are not among the ones that exist, in order.
pub(crate) fn missing_sources(requested: &[i64], existing: &[i64]) -> Vec<i64> {
    requested.iter().copied().filter(|id| !existing.contains(id)).collect()
}

/// An `operation_id` that is not one to [`MAX_OPERATION_ID_LEN`] visible
/// ASCII characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation_id must be 1 to {MAX_OPERATION_ID_LEN} visible ASCII characters")]
pub struct OperationIdError;

/// A caller's operation id, or why it is refused: one to
/// [`MAX_OPERATION_ID_LEN`] visible ASCII characters, so it prints and
/// compares the same everywhere it is shown. It names a row of the outbox,
/// so the routes that act on one check it the same way.
pub fn check_operation_id(id: Option<String>) -> Result<Option<String>, OperationIdError> {
    match id {
        Some(id) if id.is_empty() || id.len() > MAX_OPERATION_ID_LEN || !id.bytes().all(|b| b.is_ascii_graphic()) => {
            Err(OperationIdError)
        }
        id => Ok(id),
    }
}

impl NewSave {
    /// The save `input` describes, or the first rule it breaks, in this
    /// order: the content, the type, the operation id, the topic key, the
    /// number of sources and a conclusion the user is said to have stated.
    pub fn parse(input: SaveInput) -> Result<Self, SaveRejected> {
        check_content(&input.content)?;
        let memory_type = input.memory_type.as_deref().map(str::parse::<MemoryType>).transpose()?;
        let operation_id = check_operation_id(input.operation_id)?;
        if let Some(key) = input.topic_key.as_deref() {
            validate_topic_key(key)?;
        }
        let derived_from = distinct_sources(&input.derived_from)?;
        if input.user_stated && !derived_from.is_empty() {
            return Err(SaveRejected::StatedConclusion);
        }
        Ok(Self {
            content: input.content,
            agent_id: input.agent_id,
            project_id: input.project_id,
            memory_type,
            importance: input.importance,
            derived_from,
            user_stated: input.user_stated,
            operation_id,
            topic_key: input.topic_key,
        })
    }
}

/// A save that passed every rule, its project followed to the live one,
/// made only by `MemoryRuntime::admit`. What is left before it is queued
/// belongs to the host: the session it came from and the moment it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct Admitted {
    pub(crate) save: NewSave,
    pub(crate) waits: bool,
}

impl Admitted {
    pub fn agent_id(&self) -> &str {
        &self.save.agent_id
    }

    /// The live project, after any merge.
    pub fn project_id(&self) -> Option<&str> {
        self.save.project_id.as_deref()
    }

    pub fn memory_type(&self) -> Option<MemoryType> {
        self.save.memory_type
    }

    pub fn topic_key(&self) -> Option<&str> {
        self.save.topic_key.as_deref()
    }

    /// Whether the worker can report on the save soon: the embedder is
    /// ready. While it loads, which can be a long download, nobody waits.
    pub fn waits(&self) -> bool {
        self.waits
    }

    /// The event to queue, stamped with the session the host read for it
    /// and the instant it was accepted, and the caller's operation id. The
    /// type travels by its name, the text the client sent: the event is
    /// stored in the outbox, where a row written before the type was checked
    /// may hold a name that is none of the seven.
    pub fn into_event(self, session_id: Option<String>, timestamp_ms: u64) -> (MemoryEvent, Option<String>) {
        let save = self.save;
        let event = MemoryEvent {
            agent_id: save.agent_id,
            project_id: save.project_id,
            // Not read for an explicit save: `provenance::seal` decides who
            // stated it.
            role: Role::User,
            content: save.content,
            // Only `Role::Tool` messages carry an error flag.
            is_error: None,
            // The one source that honours the caller's `memory_type`/`importance`.
            source: MemorySource::Explicit,
            timestamp_ms,
            memory_type: save.memory_type.map(|t| t.as_str().to_owned()),
            importance: save.importance,
            derived_from: save.derived_from,
            topic_key: save.topic_key,
            user_stated: save.user_stated,
            session_id,
        };
        (event, save.operation_id)
    }
}

/// Refused rather than cut when it is too long, since a cut save would store
/// a text the caller never wrote. Counted in bytes as received, before
/// anything is masked.
fn check_content(content: &str) -> Result<(), SaveRejected> {
    if content.trim().is_empty() {
        return Err(SaveRejected::EmptyContent);
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err(SaveRejected::ContentTooLong { bytes: content.len() });
    }
    Ok(())
}

/// The sources a save names, each once and ascending.
fn distinct_sources(ids: &[i64]) -> Result<Vec<i64>, SaveRejected> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() > MAX_SOURCES {
        return Err(SaveRejected::TooManySources { count: ids.len() });
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(content: &str) -> SaveInput {
        SaveInput { content: content.into(), agent_id: "a".into(), ..SaveInput::default() }
    }

    fn parsed(f: impl FnOnce(&mut SaveInput)) -> Result<NewSave, SaveRejected> {
        let mut save = input("a fact");
        f(&mut save);
        NewSave::parse(save)
    }

    #[test]
    fn content_is_checked_in_bytes() {
        assert!(NewSave::parse(input(&"x".repeat(MAX_CONTENT_BYTES))).is_ok());
        assert!(NewSave::parse(input(&"ñ".repeat(MAX_CONTENT_BYTES / 2))).is_ok());
        let over = NewSave::parse(input(&"ñ".repeat(MAX_CONTENT_BYTES / 2 + 1))).unwrap_err();
        assert!(matches!(over, SaveRejected::ContentTooLong { bytes } if bytes == MAX_CONTENT_BYTES + 2), "{over:?}");
        assert_eq!(
            over.to_string(),
            format!(
                "content is {} bytes; at most {MAX_CONTENT_BYTES} are accepted: split it into several saves, one fact each",
                MAX_CONTENT_BYTES + 2
            )
        );
        for empty in ["", "  \n "] {
            assert_eq!(NewSave::parse(input(empty)).unwrap_err().to_string(), "content must not be empty");
        }
    }

    #[test]
    fn operation_ids_are_checked() {
        assert_eq!(check_operation_id(None).unwrap(), None);
        assert_eq!(check_operation_id(Some("hook-42.a_b".into())).unwrap(), Some("hook-42.a_b".into()));
        let longest = "x".repeat(MAX_OPERATION_ID_LEN);
        assert_eq!(check_operation_id(Some(longest.clone())).unwrap(), Some(longest));
        for bad in ["".to_string(), "x".repeat(MAX_OPERATION_ID_LEN + 1), "has space".into(), "acción".into()] {
            assert_eq!(check_operation_id(Some(bad.clone())), Err(OperationIdError), "{bad:?}");
            let refused = parsed(|s| s.operation_id = Some(bad.clone())).unwrap_err();
            assert!(matches!(refused, SaveRejected::OperationId(OperationIdError)), "{bad:?}: {refused:?}");
        }
    }

    #[test]
    fn sources_are_counted_once_and_sorted() {
        assert_eq!(parsed(|s| s.derived_from = vec![9, 3, 9, 1]).unwrap().derived_from, vec![1, 3, 9]);
        assert_eq!(parsed(|_| {}).unwrap().derived_from, Vec::<i64>::new());
    }

    #[test]
    fn too_many_sources_are_refused() {
        let at_cap: Vec<i64> = (0..MAX_SOURCES as i64).collect();
        assert!(parsed(|s| s.derived_from = at_cap.clone()).is_ok());
        let repeated: Vec<i64> = at_cap.iter().chain(at_cap.iter()).copied().collect();
        assert!(parsed(|s| s.derived_from = repeated).is_ok(), "the cap counts distinct ids");
        let over: Vec<i64> = (0..=MAX_SOURCES as i64).collect();
        let refused = parsed(|s| s.derived_from = over).unwrap_err();
        assert!(matches!(refused, SaveRejected::TooManySources { count } if count == MAX_SOURCES + 1), "{refused:?}");
    }

    #[test]
    fn replaced_sources_name_what_replaced_them() {
        assert_eq!(
            SaveRejected::ReplacedSources(vec![(5, Some(9)), (7, None)]).to_string(),
            "derived_from names memories that were replaced: 5 (by 9), 7; draw the conclusion from the current ones"
        );
    }

    #[test]
    fn missing_sources_are_the_ones_that_were_not_found() {
        assert_eq!(missing_sources(&[1, 2, 3, 999], &[1, 3]), vec![2, 999]);
        assert!(missing_sources(&[1, 3], &[1, 3]).is_empty());
        assert_eq!(
            SaveRejected::MissingSources(vec![2, 999]).to_string(),
            "derived_from names memories that do not exist or were deleted: 2, 999"
        );
    }

    #[test]
    fn a_conclusion_cannot_be_stated_by_the_user() {
        let refused = parsed(|s| {
            s.derived_from = vec![1];
            s.user_stated = true;
        })
        .unwrap_err();
        assert!(matches!(refused, SaveRejected::StatedConclusion), "{refused:?}");
        assert!(parsed(|s| s.user_stated = true).is_ok(), "the user's own words are no conclusion");
        assert!(parsed(|s| s.derived_from = vec![1]).is_ok(), "the agent's conclusion");
    }

    #[test]
    fn each_format_refusal_keeps_its_text() {
        let types = "semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error";
        let cases: [(SaveRejected, &str); 7] = [
            (SaveRejected::EmptyContent, "content must not be empty"),
            (
                SaveRejected::ContentTooLong { bytes: 65_537 },
                "content is 65537 bytes; at most 65536 are accepted: split it into several saves, one fact each",
            ),
            (parsed(|s| s.memory_type = Some("preference".into())).unwrap_err(), &format!("unknown memory_type \"preference\"; expected one of {types}")),
            (SaveRejected::OperationId(OperationIdError), "operation_id must be 1 to 128 visible ASCII characters"),
            (
                parsed(|s| s.topic_key = Some("Storage Engine".into())).unwrap_err(),
                "topic_key is segments of a-z, 0-9, '.', '_' or '-' joined by '/', with no empty segment",
            ),
            (SaveRejected::TooManySources { count: 51 }, "derived_from names 51 memories; at most 50 are accepted"),
            (
                SaveRejected::StatedConclusion,
                "user_stated cannot go with derived_from: a conclusion is drawn by the agent, not stated by the user",
            ),
        ];
        for (refused, text) in cases {
            assert_eq!(refused.to_string(), text, "{refused:?}");
        }
        assert_eq!(OperationIdError.to_string(), "operation_id must be 1 to 128 visible ASCII characters");
        assert_eq!(parsed(|s| s.topic_key = Some(String::new())).unwrap_err().to_string(), "topic_key must be 1 to 80 characters");
    }

    #[test]
    fn the_format_is_checked_in_order() {
        let many: Vec<i64> = (0..=MAX_SOURCES as i64).collect();
        let every_rule_broken = SaveInput {
            content: String::new(),
            agent_id: "a".into(),
            memory_type: Some("preference".into()),
            operation_id: Some(String::new()),
            topic_key: Some(String::new()),
            derived_from: many,
            user_stated: true,
            ..SaveInput::default()
        };
        let mut save = every_rule_broken;
        let mut seen = Vec::new();
        loop {
            match NewSave::parse(save.clone()) {
                Ok(_) => break,
                Err(refused) => {
                    seen.push(std::mem::discriminant(&refused));
                    match refused {
                        SaveRejected::EmptyContent => save.content = "a fact".into(),
                        SaveRejected::UnknownType(_) => save.memory_type = None,
                        SaveRejected::OperationId(_) => save.operation_id = None,
                        SaveRejected::TopicKey(_) => save.topic_key = None,
                        SaveRejected::TooManySources { .. } => save.derived_from = vec![1],
                        SaveRejected::StatedConclusion => save.user_stated = false,
                        other => panic!("not a format refusal: {other:?}"),
                    }
                }
            }
        }
        let order = [
            SaveRejected::EmptyContent,
            SaveRejected::UnknownType(UnknownMemoryType(String::new())),
            SaveRejected::OperationId(OperationIdError),
            SaveRejected::TopicKey(TopicKeyError::Length),
            SaveRejected::TooManySources { count: 0 },
            SaveRejected::StatedConclusion,
        ];
        assert_eq!(seen, order.iter().map(std::mem::discriminant).collect::<Vec<_>>());

        let long = parsed(|s| {
            s.content = "x".repeat(MAX_CONTENT_BYTES + 1);
            s.memory_type = Some("preference".into());
        })
        .unwrap_err();
        assert!(matches!(long, SaveRejected::ContentTooLong { .. }), "the size before the type: {long:?}");
    }

    #[test]
    fn a_parsed_save_keeps_its_type() {
        assert_eq!(parsed(|s| s.memory_type = Some("user_preference".into())).unwrap().memory_type, Some(MemoryType::UserPreference));
        assert_eq!(parsed(|_| {}).unwrap().memory_type, None);
        for t in MemoryType::ALL {
            assert_eq!(parsed(|s| s.memory_type = Some(t.as_str().into())).unwrap().memory_type, Some(t));
        }
    }
}
