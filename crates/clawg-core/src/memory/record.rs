use serde::{Deserialize, Serialize};

/// Discriminates which backing table a memory record originates from.
///
/// Ordered (declaration order = Memory < Episodic < Procedural) so it can
/// serve as a tie-break component in a merged-hit comparator: `id` alone is
/// unique only within one table, so the pair `(kind, id)` is what is unique
/// across all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TableKind {
    Memory,
    Episodic,
    Procedural,
}

/// Common interface for all three memory record types.
///
/// Every implementor provides access to the 12 shared fields defined by the
/// memory schema. The `table_kind` method identifies the source table.
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
    fn table_kind(&self) -> TableKind;
}

// ---------------------------------------------------------------------------
// Row structs
// ---------------------------------------------------------------------------

// MemoryRow, EpisodicTrajectoryRow and ProceduralSkillRow keep bare i64
// timestamps on purpose. Nothing outside this file builds them — they are
// defined here, re-exported by the memory module, and constructed only by the
// tests below. Giving their timestamps the millisecond newtype would put the
// compiler on guard over a path that never runs, and the churn would land in a
// review whose actual subject is the live retrieval path. If they ever get a
// production caller, type them in that same change; until then you would be
// paying for a guarantee nobody collects.

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
    fn table_kind(&self) -> TableKind { TableKind::Memory }
}

// ---------------------------------------------------------------------------

/// A row from the `episodic_trajectory` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicTrajectoryRow {
    pub id: i64,
    pub scope: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub session_id: String,
    pub task: String,
    pub tool_calls: String,
    pub errors_encountered: String,
    pub resolution: Option<String>,
    pub success_score: f64,
    pub created_at: i64,
    pub created_by_type: String,
    pub created_by_ref: String,
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

impl MemoryRecord for EpisodicTrajectoryRow {
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
    fn table_kind(&self) -> TableKind { TableKind::Episodic }
}

// ---------------------------------------------------------------------------

/// A row from the `procedural_skill` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralSkillRow {
    pub id: i64,
    pub scope: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub skill_name: String,
    pub trigger: String,
    pub tool_sequence: String,
    pub success_count: i64,
    pub promoted_at: i64,
    pub source_trajectory_ids: String,
    pub created_at: i64,
    pub created_by_type: String,
    pub created_by_ref: String,
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

impl MemoryRecord for ProceduralSkillRow {
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
    fn table_kind(&self) -> TableKind { TableKind::Procedural }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    fn make_episodic_row() -> EpisodicTrajectoryRow {
        EpisodicTrajectoryRow {
            id: 1,
            scope: "agent".to_string(),
            agent_id: Some("agent-1".to_string()),
            project_id: None,
            session_id: "sess-1".to_string(),
            task: "do something".to_string(),
            tool_calls: "[]".to_string(),
            errors_encountered: "[]".to_string(),
            resolution: None,
            success_score: 1.0,
            created_at: 0,
            created_by_type: "agent".to_string(),
            created_by_ref: "".to_string(),
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

    fn make_procedural_row() -> ProceduralSkillRow {
        ProceduralSkillRow {
            id: 1,
            scope: "agent".to_string(),
            agent_id: Some("agent-1".to_string()),
            project_id: None,
            skill_name: "test-skill".to_string(),
            trigger: "when X".to_string(),
            tool_sequence: "[]".to_string(),
            success_count: 0,
            promoted_at: 0,
            source_trajectory_ids: "[]".to_string(),
            created_at: 0,
            created_by_type: "agent".to_string(),
            created_by_ref: "".to_string(),
            last_modified_at: 0,
            last_modified_by: None,
            update_count: 0,
            last_accessed_at: 0,
            access_count: 0,
            importance: 8.0,
            tier: 1,
            superseded_by: None,
            status: "active".to_string(),
            dispute_count: 0,
            disputed_against: None,
            last_disputed_at: None,
            deleted_at: None,
        }
    }

    /// `TableKind` needs a total order so a merged-hit comparator can use it
    /// as a tie-break: reflexive (equal to itself), antisymmetric (Memory <
    /// Episodic < Procedural in one direction only), and covering every pair.
    #[test]
    fn table_kind_has_a_total_order() {
        use std::cmp::Ordering;
        assert_eq!(TableKind::Memory.cmp(&TableKind::Memory), Ordering::Equal);
        assert_eq!(TableKind::Episodic.cmp(&TableKind::Episodic), Ordering::Equal);
        assert_eq!(TableKind::Procedural.cmp(&TableKind::Procedural), Ordering::Equal);

        assert_eq!(TableKind::Memory.cmp(&TableKind::Episodic), Ordering::Less);
        assert_eq!(TableKind::Episodic.cmp(&TableKind::Memory), Ordering::Greater);

        assert_eq!(TableKind::Episodic.cmp(&TableKind::Procedural), Ordering::Less);
        assert_eq!(TableKind::Procedural.cmp(&TableKind::Episodic), Ordering::Greater);

        assert_eq!(TableKind::Memory.cmp(&TableKind::Procedural), Ordering::Less);
        assert_eq!(TableKind::Procedural.cmp(&TableKind::Memory), Ordering::Greater);
    }

    /// Compile-time assertion: all three types implement MemoryRecord.
    #[test]
    fn trait_dispatch_all_three_types() {
        fn check<T: MemoryRecord>(_: &T) {}
        check(&make_memory_row());
        check(&make_episodic_row());
        check(&make_procedural_row());
    }

    /// agent_id returns None when field is None.
    #[test]
    fn agent_id_returns_option_none() {
        let mut row = make_memory_row();
        row.agent_id = None;
        assert_eq!(row.agent_id(), None);
    }

    /// Each row type returns its correct TableKind.
    #[test]
    fn table_kind_per_type() {
        assert_eq!(make_memory_row().table_kind(), TableKind::Memory);
        assert_eq!(make_episodic_row().table_kind(), TableKind::Episodic);
        assert_eq!(make_procedural_row().table_kind(), TableKind::Procedural);
    }

    /// MemoryRecord impl returns struct fields.
    #[test]
    fn memory_record_returns_fields() {
        let row = make_memory_row();
        assert_eq!(row.id(), 42);
        assert_eq!(row.scope(), "agent");
    }

    /// dyn MemoryRecord works for all three types.
    #[test]
    fn dyn_memory_record_works() {
        fn print_id(r: &dyn MemoryRecord) -> i64 { r.id() }
        assert_eq!(print_id(&make_memory_row()), 42);
        assert_eq!(print_id(&make_episodic_row()), 1);
        assert_eq!(print_id(&make_procedural_row()), 1);
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

    /// All three implementors inherit the default method correctly
    #[test]
    fn disputed_against_ids_inherited_by_all_three_types() {
        let mut mem = make_memory_row();
        mem.disputed_against = Some("[42]".to_string());
        assert_eq!(mem.disputed_against_ids(), Some(vec![42i64]));

        let mut epi = make_episodic_row();
        epi.disputed_against = Some("[7,8]".to_string());
        assert_eq!(epi.disputed_against_ids(), Some(vec![7i64, 8]));

        let mut pro = make_procedural_row();
        pro.disputed_against = None;
        assert_eq!(pro.disputed_against_ids(), None);
    }
}
