use serde::{Deserialize, Serialize};

/// The fields a memory record holds, read from a row of `memory`.
///
/// `disputed_against` returns the raw JSON string exactly as the column holds
/// it; `disputed_against_ids` parses it into `Vec<i64>`. Both exist: the raw
/// form spares callers that only need "is this disputed at all" a JSON parse,
/// and keeps a malformed value inspectable instead of collapsing it to None.
pub trait MemoryRecord {
    fn id(&self) -> i64;
    fn scope(&self) -> &str;
    /// Returns `None` when `scope` is `'project'` or `'global'`.
    fn agent_id(&self) -> Option<&str>;
    fn project_id(&self) -> Option<&str>;
    fn created_at(&self) -> i64;
    fn last_accessed_at(&self) -> i64;
    fn importance(&self) -> f64;
    /// Tier level: 1 (highest priority) to 3 (lowest priority).
    fn tier(&self) -> u8;
    fn status(&self) -> &str;
    fn dispute_count(&self) -> u32;
    /// Raw JSON array of disputed-against record IDs, or `None` if not disputed.
    /// For the parsed form use `disputed_against_ids`, whose default implementation
    /// is directly below.
    fn disputed_against(&self) -> Option<&str>;
    /// Typed accessor — parses raw `disputed_against` JSON into `Vec<i64>`.
    /// Keeps the raw `disputed_against()` for back-compat.
    /// Returns `None` for NULL or malformed JSON without panicking.
    fn disputed_against_ids(&self) -> Option<Vec<i64>> {
        self.disputed_against()
            .and_then(|raw| serde_json::from_str::<Vec<i64>>(raw).ok())
    }
}

// ---------------------------------------------------------------------------
// Row structs
// ---------------------------------------------------------------------------

// MemoryRow keeps bare i64 timestamps on purpose. Nothing outside this file
// builds it: it is defined here, re-exported by the memory module, and
// constructed only by the tests below. Giving its timestamps the millisecond
// newtype would put the compiler on guard over a path that never runs. If it
// ever gets a production caller, type them in that same change; until then you
// would be paying for a guarantee nobody collects.

/// A row from the `memory` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: i64,
    pub scope: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub content: String,
    pub memory_type: String,
    pub source: String,
    pub created_at: i64,
    pub created_by_type: String,
    pub created_by_ref: String,
    pub source_session_id: Option<String>,
    pub last_modified_at: i64,
    pub last_modified_by: Option<String>,
    pub update_count: i64,
    pub last_accessed_at: i64,
    pub access_count: i64,
    pub importance: f64,
    pub tier: i64,
    pub superseded_by: Option<i64>,
    pub status: String,
    pub dispute_count: i64,
    pub disputed_against: Option<String>,
    pub last_disputed_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

impl MemoryRecord for MemoryRow {
    fn id(&self) -> i64 { self.id }
    fn scope(&self) -> &str { &self.scope }
    fn agent_id(&self) -> Option<&str> { self.agent_id.as_deref() }
    fn project_id(&self) -> Option<&str> { self.project_id.as_deref() }
    fn created_at(&self) -> i64 { self.created_at }
    fn last_accessed_at(&self) -> i64 { self.last_accessed_at }
    fn importance(&self) -> f64 { self.importance }
    fn tier(&self) -> u8 { self.tier as u8 }
    fn status(&self) -> &str { &self.status }
    fn dispute_count(&self) -> u32 { self.dispute_count as u32 }
    fn disputed_against(&self) -> Option<&str> { self.disputed_against.as_deref() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    ChatMessage,
    ToolError,
    CodeBlock,
    SessionSummary,
    UserPreference,
    ArchitecturalDecision,
    SemanticFact,
}

impl MemoryType {
    /// Every type, in the order an error message lists them.
    pub const ALL: [MemoryType; 7] = [
        Self::SemanticFact,
        Self::UserPreference,
        Self::ArchitecturalDecision,
        Self::SessionSummary,
        Self::CodeBlock,
        Self::ChatMessage,
        Self::ToolError,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatMessage => "chat_message",
            Self::ToolError => "tool_error",
            Self::CodeBlock => "code_block",
            Self::SessionSummary => "session_summary",
            Self::UserPreference => "user_preference",
            Self::ArchitecturalDecision => "architectural_decision",
            Self::SemanticFact => "semantic_fact",
        }
    }

    /// Only a type that asserts something can be contradicted. An explicit
    /// `match` rather than `matches!`: adding an 8th variant is then a
    /// compile error here, not a silent `false`.
    pub(crate) fn is_claim_bearing(self) -> bool {
        match self {
            Self::UserPreference | Self::ArchitecturalDecision | Self::SemanticFact => true,
            Self::ChatMessage | Self::ToolError | Self::CodeBlock | Self::SessionSummary => false,
        }
    }

    /// A preference or a decision is replaced by a verdict, or loses its type,
    /// only once the caller declares the user answered.
    pub(crate) fn is_protected(self) -> bool {
        match self {
            Self::UserPreference | Self::ArchitecturalDecision => true,
            Self::SemanticFact | Self::ChatMessage | Self::ToolError | Self::CodeBlock | Self::SessionSummary => false,
        }
    }

    /// A stored type the parser does not know is not protected.
    pub(crate) fn is_protected_name(s: &str) -> bool {
        Self::from_db_str(s).is_some_and(Self::is_protected)
    }

    /// Parses the raw DB string. Unknown or legacy values return `None` —
    /// treated as not eligible for the contradiction type gate rather than
    /// panicking on a value written by a future or foreign schema version.
    pub(crate) fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "chat_message" => Some(Self::ChatMessage),
            "tool_error" => Some(Self::ToolError),
            "code_block" => Some(Self::CodeBlock),
            "session_summary" => Some(Self::SessionSummary),
            "user_preference" => Some(Self::UserPreference),
            "architectural_decision" => Some(Self::ArchitecturalDecision),
            "semantic_fact" => Some(Self::SemanticFact),
            _ => None,
        }
    }
}

/// A `memory_type` a caller named that is not one of [`MemoryType::ALL`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown memory_type {:?}; expected one of {}", .0, MemoryType::ALL.map(MemoryType::as_str).join(", "))]
pub struct UnknownMemoryType(pub String);

/// What a caller names: unlike [`MemoryType::from_db_str`], which reads values
/// already stored, an unknown name here is a mistake to report.
impl std::str::FromStr for MemoryType {
    type Err = UnknownMemoryType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_db_str(s).ok_or_else(|| UnknownMemoryType(s.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CreatedByType {
    Agent,
    User,
    System,
}

impl CreatedByType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MemoryRow with all required fields defaulted.
    fn make_memory_row() -> MemoryRow {
        MemoryRow {
            id: 42,
            scope: "agent".to_string(),
            agent_id: Some("agent-1".to_string()),
            project_id: None,
            content: "test content".to_string(),
            memory_type: "observation".to_string(),
            source: "agent".to_string(),
            created_at: 0,
            created_by_type: "agent".to_string(),
            created_by_ref: "".to_string(),
            source_session_id: None,
            last_modified_at: 0,
            last_modified_by: None,
            update_count: 0,
            last_accessed_at: 0,
            access_count: 0,
            importance: 5.0,
            tier: 3,
            superseded_by: None,
            status: "active".to_string(),
            dispute_count: 0,
            disputed_against: None,
            last_disputed_at: None,
            deleted_at: None,
        }
    }

    /// Compile-time assertion: the row implements MemoryRecord.
    #[test]
    fn trait_dispatch() {
        fn check<T: MemoryRecord>(_: &T) {}
        check(&make_memory_row());
    }

    /// agent_id returns None when field is None.
    #[test]
    fn agent_id_returns_option_none() {
        let mut row = make_memory_row();
        row.agent_id = None;
        assert_eq!(row.agent_id(), None);
    }

    /// MemoryRecord impl returns struct fields.
    #[test]
    fn memory_record_returns_fields() {
        let row = make_memory_row();
        assert_eq!(row.id(), 42);
        assert_eq!(row.scope(), "agent");
    }

    /// dyn MemoryRecord works.
    #[test]
    fn dyn_memory_record_works() {
        fn print_id(r: &dyn MemoryRecord) -> i64 { r.id() }
        assert_eq!(print_id(&make_memory_row()), 42);
    }

    // ── disputed_against_ids() default trait method ───────────────────────────

    /// Valid JSON array → Some(vec)
    #[test]
    fn disputed_against_ids_valid_json_returns_some() {
        let mut row = make_memory_row();
        row.disputed_against = Some("[1,2,3]".to_string());
        assert_eq!(row.disputed_against_ids(), Some(vec![1i64, 2, 3]));
    }

    /// Field is None → returns None without panic
    #[test]
    fn disputed_against_ids_null_returns_none() {
        let row = make_memory_row(); // disputed_against starts as None
        assert_eq!(row.disputed_against_ids(), None);
    }

    /// Malformed JSON → returns None without panic
    #[test]
    fn disputed_against_ids_malformed_json_returns_none() {
        let mut row = make_memory_row();
        row.disputed_against = Some("not-json".to_string());
        assert_eq!(row.disputed_against_ids(), None);
    }

    /// Empty JSON array → Some(empty vec)
    #[test]
    fn disputed_against_ids_empty_array_returns_some_empty() {
        let mut row = make_memory_row();
        row.disputed_against = Some("[]".to_string());
        assert_eq!(row.disputed_against_ids(), Some(vec![]));
    }

    #[test]
    fn memory_type_session_summary_as_str() {
        assert_eq!(MemoryType::SessionSummary.as_str(), "session_summary");
    }

    // ── New pivotal variants ──

    #[test]
    fn as_str_new_variants() {
        assert_eq!(MemoryType::UserPreference.as_str(), "user_preference");
        assert_eq!(MemoryType::ArchitecturalDecision.as_str(), "architectural_decision");
        assert_eq!(MemoryType::SemanticFact.as_str(), "semantic_fact");
    }

    // ── is_claim_bearing — truth table over all 7 variants ─────────────────

    #[test]
    fn is_claim_bearing_true_for_user_preference() {
        assert!(MemoryType::UserPreference.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_true_for_architectural_decision() {
        assert!(MemoryType::ArchitecturalDecision.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_true_for_semantic_fact() {
        assert!(MemoryType::SemanticFact.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_chat_message() {
        assert!(!MemoryType::ChatMessage.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_tool_error() {
        assert!(!MemoryType::ToolError.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_code_block() {
        assert!(!MemoryType::CodeBlock.is_claim_bearing());
    }

    #[test]
    fn is_claim_bearing_false_for_session_summary() {
        assert!(!MemoryType::SessionSummary.is_claim_bearing());
    }

    // ── from_db_str — round-trips as_str(), unknown → None ─────────────────

    #[test]
    fn from_db_str_round_trips_every_variant() {
        let variants = [
            MemoryType::ChatMessage,
            MemoryType::ToolError,
            MemoryType::CodeBlock,
            MemoryType::SessionSummary,
            MemoryType::UserPreference,
            MemoryType::ArchitecturalDecision,
            MemoryType::SemanticFact,
        ];
        for v in variants {
            assert_eq!(MemoryType::from_db_str(v.as_str()), Some(v), "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn every_listed_type_parses_back_to_itself() {
        // Exhaustive on purpose: an eighth variant fails to compile here until
        // it is placed in `ALL` too.
        let listed = |t: MemoryType| match t {
            MemoryType::ChatMessage
            | MemoryType::ToolError
            | MemoryType::CodeBlock
            | MemoryType::SessionSummary
            | MemoryType::UserPreference
            | MemoryType::ArchitecturalDecision
            | MemoryType::SemanticFact => MemoryType::ALL.contains(&t),
        };
        for t in MemoryType::ALL {
            assert!(listed(t));
            assert_eq!(t.as_str().parse::<MemoryType>(), Ok(t));
            assert_eq!(MemoryType::from_db_str(t.as_str()), Some(t));
        }
        let distinct: std::collections::BTreeSet<&str> = MemoryType::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(distinct.len(), 7);
    }

    #[test]
    fn an_unknown_type_names_the_accepted_ones() {
        let err = "preference".parse::<MemoryType>().unwrap_err();
        assert_eq!(err, UnknownMemoryType("preference".into()));
        assert_eq!(
            err.to_string(),
            "unknown memory_type \"preference\"; expected one of semantic_fact, user_preference, \
             architectural_decision, session_summary, code_block, chat_message, tool_error"
        );
        for raw in ["", "Semantic_Fact", "episode"] {
            assert!(raw.parse::<MemoryType>().is_err(), "{raw:?}");
        }
    }

    #[test]
    fn from_db_str_unknown_value_returns_none() {
        assert_eq!(MemoryType::from_db_str("episode"), None);
        assert_eq!(MemoryType::from_db_str("not_a_real_type"), None);
        assert_eq!(MemoryType::from_db_str(""), None);
    }
}
