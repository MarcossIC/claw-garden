//! Millisecond instants for the memory subsystem.
//!
//! This is a second, independent wall clock. `models::chat` has its own
//! millisecond reader, used by chat, sessions, compaction, agent CRUD and
//! provider config — code that has nothing to do with retrieval and whose
//! timestamps never reach a decay formula. Unifying them would drag every one
//! of those callers into this type's rules for a guarantee only retrieval
//! needs, so they stay separate. Both read the system clock and both fall back
//! to the epoch if it predates 1970; they agree on the unit, they just do not
//! share a type.
//!
//! The field is private on purpose. With a public field, `TimestampMs(secs)`
//! compiles — the type whose only reason to exist is stopping a second count
//! from passing as a millisecond count would accept a second count without a
//! word. Every value enters through a named constructor, and the constructor
//! name carries the unit.

use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::ToSql;

/// A wall-clock instant in milliseconds since the UNIX epoch.
///
/// `pub(crate)`, same as the `clock` module: only in-crate producers and
/// consumers (`search`, `store_insert`) ever name this type, so there is
/// nothing to gain from a wider visibility keyword. The inner field stays
/// private regardless — the single-constructor unit guarantee this type
/// exists for is a separate axis from how far its name travels.
/// `Serialize` is a transparent newtype over the raw `i64`, so a serialized
/// instant is the same millisecond number every other layer already speaks —
/// no date string, no object wrapper. It was added for `RankedHit`, which the
/// `clawg serve` daemon returns as JSON; keeping the wire shape numeric means
/// the daemon's clients read `created_at` exactly as SQLite stores it.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct TimestampMs(i64);

impl TimestampMs {
    /// Reads the system clock.
    ///
    /// A clock set before 1970 yields the epoch rather than an error: every
    /// call site this replaced already collapsed that case to zero, and a
    /// retrieval pass must not fail because a laptop's clock is wrong.
    pub fn now() -> Self {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Self::from_unix_millis_u64(u64::try_from(millis).unwrap_or(u64::MAX))
    }

    /// Wraps a value already known to be in milliseconds.
    pub(crate) fn from_unix_millis(ms: i64) -> Self {
        Self(ms)
    }

    /// Wraps an unsigned millisecond count, clamping instead of wrapping.
    ///
    /// The producers upstream count in `u64`; SQLite stores `i64`. A bare cast
    /// would turn an out-of-range value into a negative date silently — the
    /// exact failure mode this type exists to prevent. Clamping is safe to do
    /// without a fallible signature because the clamp point is roughly the
    /// year 292 million: any input that reaches it is already garbage, and
    /// forcing every call site to handle an impossible error would buy
    /// nothing.
    pub(crate) fn from_unix_millis_u64(ms: u64) -> Self {
        if ms > i64::MAX as u64 {
            log::warn!(
                "millisecond timestamp {ms} exceeds the signed range SQLite stores; \
                 clamping to {} instead of letting the cast wrap to a negative date",
                i64::MAX
            );
        }
        let clamped = i64::try_from(ms).unwrap_or(i64::MAX);
        debug_assert!(
            clamped >= 0,
            "clamping an unsigned millisecond count must never produce a negative instant"
        );
        Self(clamped)
    }

    /// The raw millisecond count.
    ///
    /// Reach for this only where the receiving code cannot take the type —
    /// a dynamically-built SQL value list, or arithmetic the type does not
    /// expose. Everywhere else, hand over the typed value.
    pub fn as_millis_i64(self) -> i64 {
        self.0
    }

    /// Days elapsed between `self` and `now`.
    ///
    /// Returns a NEGATIVE number when `self` is later than `now`, deliberately.
    /// Clamping to zero would make "this row is stamped in the future" read
    /// identically to "this row was written a moment ago", and a future stamp
    /// is precisely the corruption worth seeing — a database copied between
    /// machines with skewed clocks produces it. Consumers that treat age as a
    /// decay input already read a negative as "as fresh as possible", which is
    /// the same outcome clamping would give, minus the lost signal.
    #[allow(dead_code)] // Kept ahead of its caller: the demotion pass computes its
    // day count inside SQL today, and this is the Rust-side replacement for that
    // expression when the arithmetic moves out of the query. Removing it now means
    // re-deriving the epoch-to-days conversion the day the pass moves.
    pub(crate) fn age_days_from(self, now: TimestampMs) -> f64 {
        now.0.saturating_sub(self.0) as f64 / 86_400_000.0
    }
}

impl ToSql for TimestampMs {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0))
    }
}

impl FromSql for TimestampMs {
    /// Accepts any integer the column holds, negatives included.
    ///
    /// Range checking is not this impl's job. One corrupt row must not take
    /// down a whole retrieval pass, and a value that survives to the assertions
    /// is more diagnosable than one that turned into an error at the row
    /// boundary. The type guards the unit; the tests guard the range.
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self::from_unix_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_match_the_pinned_set() {
        fn _f<T: Copy + Clone + PartialEq + Eq + PartialOrd + Ord + std::fmt::Debug + std::hash::Hash>() {}
        _f::<TimestampMs>();
        fn _ser<T: serde::Serialize>() {}
        _ser::<TimestampMs>();
    }

    /// The `transparent` attribute is the contract, not an implementation
    /// detail: a wrapper object or an ISO string here would change the daemon's
    /// JSON shape for every `RankedHit.created_at` at once.
    #[test]
    fn serializes_as_a_bare_millisecond_number() {
        let ts = TimestampMs::from_unix_millis_u64(1_700_000_000_000);
        assert_eq!(
            serde_json::to_string(&ts).expect("TimestampMs must serialize"),
            "1700000000000"
        );
    }

    #[test]
    fn u64_max_clamps_to_i64_max_without_wrapping() {
        let ts = TimestampMs::from_unix_millis_u64(u64::MAX);
        assert_eq!(ts.as_millis_i64(), i64::MAX);
    }

    #[test]
    fn in_range_u64_round_trips_through_as_millis_i64() {
        let ts = TimestampMs::from_unix_millis_u64(1_700_000_000_000);
        assert_eq!(ts.as_millis_i64(), 1_700_000_000_000);
    }

    #[test]
    fn now_reads_a_plausible_present_instant() {
        let a = TimestampMs::now();
        let b = TimestampMs::now();
        let age = a.age_days_from(b);
        assert!(age >= 0.0 && age < 1e-6, "age was {age}");
    }

    #[test]
    fn ninety_days_apart_is_ninety_days() {
        let earlier = TimestampMs::from_unix_millis(0);
        let later = TimestampMs::from_unix_millis(90 * 86_400_000);
        assert_eq!(earlier.age_days_from(later), 90.0);
    }

    #[test]
    fn an_instant_after_now_reports_a_negative_age() {
        let earlier_now = TimestampMs::from_unix_millis(0);
        let future = TimestampMs::from_unix_millis(86_400_000);
        assert!(future.age_days_from(earlier_now) < 0.0);
    }

    #[test]
    fn from_sql_accepts_a_negative_integer() {
        let result = TimestampMs::column_result(rusqlite::types::ValueRef::Integer(-1));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn to_sql_binds_without_manual_extraction() {
        let store = crate::memory::db::MemoryStore::new_for_test().await.unwrap();
        let ts = TimestampMs::from_unix_millis(1_700_000_000_000);
        store
            .conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute_batch("CREATE TABLE scratch_ts (v INTEGER NOT NULL)")?;
                conn.execute("INSERT INTO scratch_ts (v) VALUES (?1)", rusqlite::params![ts])?;
                Ok(())
            })
            .await
            .unwrap();
        let read_back: TimestampMs = store
            .conn
            .call(|conn| -> rusqlite::Result<TimestampMs> {
                let v = conn.query_row("SELECT v FROM scratch_ts", [], |row| row.get(0))?;
                Ok(v)
            })
            .await
            .unwrap();
        assert_eq!(read_back, ts);
    }
}
