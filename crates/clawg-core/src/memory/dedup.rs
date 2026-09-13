//! Write-border content hashing shared by the V6 backfill UDF and the
//! runtime dedup lookup.
//!
//! `content_hash` is the ONLY normalization + digest implementation in this
//! module — the SQL scalar function registered below is a thin wrapper that
//! delegates to it. If the UDF and the runtime path ever compute the hash
//! differently, backfilled rows land in a different hash space than newly
//! written ones and dedup silently stops matching history.

use crate::memory::MemoryError;
use unicode_normalization::UnicodeNormalization;

/// Trim, then NFC-normalize, then SHA-256 — trim BEFORE compose, deliberately:
/// NFC can fold a leading combining mark into the preceding character, so
/// trimming after normalization would trim a different string than the one
/// that got hashed. Trim-then-compose is the order under which
/// `content_hash(content_hash_input(x)) == content_hash(x)` holds.
///
/// Hex digest computed inline rather than via `eval_api::sha256_hex`: that
/// module is `#[cfg(feature = "eval")]`-gated (an intentionally opaque door
/// for the offline evaluation binary, never compiled into the shipped app),
/// while this function runs on the production write path and in the V6
/// migration — both need to work in a default build. `model_download.rs`'s
/// private `sha256_hex` already duplicates the same three-line digest for
/// the identical reason; this is that same, unavoidable duplication, not a
/// new pattern.
pub(crate) fn content_hash(text: &str) -> String {
    let normalized: String = text.trim().nfc().collect();
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(normalized.as_bytes());
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Registers the `content_hash` scalar UDF on the connection.
///
/// Arity 1, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`. The closure body is exactly
/// `Ok(content_hash(&ctx.get::<String>(0)?))` and nothing more — any future
/// change to normalization goes inside `content_hash` itself, never here, so
/// the UDF and the runtime path can never diverge. Called from
/// `register_udfs` in `db.rs`, BEFORE `run_migrations` — the V6 backfill
/// calls this function by name.
pub(crate) async fn register_content_hash_udf(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError> {
    conn.call(|c| {
        use rusqlite::functions::FunctionFlags;
        let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;

        c.create_scalar_function("content_hash", 1, det, |ctx| {
            let text: String = ctx.get(0)?;
            Ok(content_hash(&text))
        })?;

        Ok(())
    })
    .await
    .map_err(|e| match e {
        tokio_rusqlite::Error::Error(rusqlite_err) => MemoryError::Sqlite(rusqlite_err),
        tokio_rusqlite::Error::ConnectionClosed => {
            MemoryError::Backend("connection closed during content_hash UDF registration".into())
        }
        other => MemoryError::Backend(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfd_and_nfc_input_hash_identically() {
        let nfd = "cafe\u{301}"; // "café" as e + combining acute accent
        let nfc = "café";
        assert_eq!(content_hash(nfd), content_hash(nfc));
    }

    #[test]
    fn leading_and_trailing_whitespace_is_trimmed() {
        let padded = "  hello world  \n";
        let trimmed = "hello world";
        assert_eq!(content_hash(padded), content_hash(trimmed));
    }

    #[test]
    fn distinct_content_hashes_differently() {
        assert_ne!(content_hash("hello world"), content_hash("goodbye world"));
    }

    #[test]
    fn hash_is_64_lowercase_hex_chars() {
        let h = content_hash("some content");
        assert_eq!(h.len(), 64, "sha256 hex digest must be 64 chars, got {h:?}");
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "digest must be lowercase hex, got {h:?}"
        );
    }

    // ── SQL UDF equals the Rust function, on the same input ────────────────

    async fn sql_content_hash(store: &crate::memory::db::MemoryStore, text: &str) -> String {
        let text = text.to_string();
        store
            .conn
            .call(move |c| c.query_row("SELECT content_hash(?1)", [text], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn sql_udf_matches_rust_content_hash() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let plain = "hello world";
        assert_eq!(sql_content_hash(&store, plain).await, content_hash(plain));
    }

    #[tokio::test]
    async fn sql_udf_matches_rust_content_hash_on_nfd_input() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let nfd = "cafe\u{301}";
        assert_eq!(sql_content_hash(&store, nfd).await, content_hash(nfd));
        // And it must equal the NFC form's hash too, transitively.
        assert_eq!(sql_content_hash(&store, nfd).await, content_hash("café"));
    }

    /// `leading_and_trailing_whitespace_is_trimmed` above only exercises the
    /// Rust function. Neither SQL-UDF equality test sends whitespace-padded
    /// input, so a UDF that stopped trimming (while the Rust function kept
    /// trimming) would pass both of them and go undetected — the UDF and the
    /// Rust path would silently hash into different spaces for any content
    /// with surrounding whitespace. This closes that hole by running a
    /// padded string through both.
    #[tokio::test]
    async fn sql_udf_matches_rust_content_hash_on_whitespace_padded_input() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let padded = "  hello world  \n";
        assert_eq!(sql_content_hash(&store, padded).await, content_hash(padded));
        // And it must equal the trimmed form's hash too, transitively.
        assert_eq!(sql_content_hash(&store, padded).await, content_hash("hello world"));
    }

    // ── Backfill: simulates a row written before V6, exercises the backfill
    // UPDATE statement directly (not migration ordering — the whole suite
    // failing loudly already proves that, see D-A) ──────────────────────────

    /// The exact `idx_memory_dedup`-shaped lookup: a row with `content_hash
    /// IS NULL` (simulated legacy state) must be invisible to it; running the
    /// V6 backfill UPDATE must make it visible.
    #[tokio::test]
    async fn backfilled_row_becomes_lookup_visible() {
        use rusqlite::OptionalExtension;
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();

        // `new_for_test()` already runs V6, so simulate legacy data: raw-insert
        // a row, then force content_hash back to NULL as if it predated V6.
        let id: i64 = store
            .conn
            .call(|c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', 'agent-backfill', 'legacy content', 'chat_message', 'chat', \
                      0, 'user', 'agent-backfill', 0, 0, 5.0, 3, 'active')",
                    [],
                )?;
                let id = c.last_insert_rowid();
                c.execute("UPDATE memory SET content_hash = NULL WHERE id = ?1", [id])?;
                Ok(id)
            })
            .await
            .unwrap();

        let lookup = |c: &rusqlite::Connection, agent: &str| -> rusqlite::Result<Option<i64>> {
            c.query_row(
                "SELECT id FROM memory \
                 WHERE agent_id = ?1 \
                   AND COALESCE(project_id, '') = COALESCE(?2, '') \
                   AND content_hash = ?3 \
                   AND content_hash IS NOT NULL \
                   AND deleted_at IS NULL \
                   AND status IN ('active', 'disputed') \
                 ORDER BY created_at ASC LIMIT 1",
                rusqlite::params![agent, Option::<String>::None, content_hash("legacy content")],
                |r| r.get(0),
            )
            .optional()
        };

        let agent = "agent-backfill".to_string();
        let before: Option<i64> = store
            .conn
            .call({
                let agent = agent.clone();
                move |c| lookup(c, &agent)
            })
            .await
            .unwrap();
        assert!(before.is_none(), "a NULL content_hash row must be invisible to lookup, got {before:?}");

        // Run V6's backfill UPDATE directly.
        store
            .conn
            .call(|c| -> rusqlite::Result<usize> {
                c.execute(
                    "UPDATE memory SET content_hash = content_hash(content) WHERE content IS NOT NULL",
                    [],
                )
            })
            .await
            .unwrap();

        let after: Option<i64> = store
            .conn
            .call(move |c| lookup(c, &agent))
            .await
            .unwrap();
        assert_eq!(after, Some(id), "backfilled row must become lookup-visible");
    }
}
