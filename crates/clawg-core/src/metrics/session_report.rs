//! What `clawg metrics sessions` prints: the stored sessions summed by
//! project, by agent or by local day, or one session row by row. Pure: it
//! is handed the rows and the time.

use std::collections::BTreeMap;

use serde::Serialize;

use super::sessions::{idle_closed, CountKind, SessionRow, IDLE, IDLE_CLOSE_MS};

/// The memory type of a save that closes a session with its summary.
const SUMMARY: &str = "session_summary";

/// What the sessions are grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsBy {
    Project,
    Agent,
    /// The local date the row began on.
    Day,
}

impl SessionsBy {
    fn as_str(self) -> &'static str {
        match self {
            SessionsBy::Project => "project",
            SessionsBy::Agent => "agent",
            SessionsBy::Day => "day",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CallCount {
    pub count: u64,
    pub failed: u64,
}

/// One row as the report shows it. A row the file still has open, six hours
/// or more after its last event, reads as closed for being idle then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowView {
    pub session_id: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub client: String,
    pub started_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub last_save_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<String>,
    /// From its start to its end, or to its last event while it is open.
    pub duration_ms: i64,
    pub prompts: u64,
    pub empty_searches: u64,
    pub starts: BTreeMap<String, u64>,
    pub calls: BTreeMap<String, CallCount>,
    pub saves: BTreeMap<String, u64>,
}

impl RowView {
    pub fn of(row: &SessionRow, now_ms: i64) -> Self {
        let (ended_at_ms, end_reason) = match row.ended_at_ms {
            Some(at) => (Some(at), row.end_reason.clone()),
            None if idle_closed(row.last_seen_at_ms, now_ms) => (Some(row.last_seen_at_ms), Some(IDLE.to_string())),
            None => (None, None),
        };
        let mut view = RowView {
            session_id: row.session_id.clone(),
            agent_id: row.agent_id.clone(),
            project_id: row.project_id.clone(),
            client: row.client.clone(),
            started_at_ms: row.started_at_ms,
            last_seen_at_ms: row.last_seen_at_ms,
            last_save_at_ms: row.last_save_at_ms,
            ended_at_ms,
            end_reason,
            duration_ms: ended_at_ms.unwrap_or(row.last_seen_at_ms).saturating_sub(row.started_at_ms).max(0),
            prompts: row.prompts,
            empty_searches: row.empty_searches,
            starts: BTreeMap::new(),
            calls: BTreeMap::new(),
            saves: BTreeMap::new(),
        };
        for ((kind, name), count) in &row.counts {
            match kind {
                CountKind::Start => *view.starts.entry(name.clone()).or_default() += count,
                CountKind::Call => view.calls.entry(name.clone()).or_default().count += count,
                CountKind::FailedCall => view.calls.entry(name.clone()).or_default().failed += count,
                CountKind::Save => *view.saves.entry(name.clone()).or_default() += count,
            }
        }
        view
    }

    /// How long the session stayed open after its last save; only a closed
    /// row that saved has one.
    fn since_last_save_ms(&self) -> Option<i64> {
        Some(self.ended_at_ms?.saturating_sub(self.last_save_at_ms?).max(0))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Durations {
    pub total: i64,
    pub mean: f64,
    pub max: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct SinceLastSave {
    /// The closed sessions that saved something.
    pub sessions: u64,
    pub mean: f64,
    pub max: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Group {
    /// The project id, the agent id or the date; `null` for the sessions
    /// without a project.
    pub key: Option<String>,
    pub sessions: u64,
    pub open: u64,
    /// How many ended for each reason, `idle` among them.
    pub ended: BTreeMap<String, u64>,
    pub duration_ms: Durations,
    pub since_last_save_ms: SinceLastSave,
    pub prompts: u64,
    pub empty_searches: u64,
    pub starts: BTreeMap<String, u64>,
    pub calls: BTreeMap<String, CallCount>,
    pub saves: BTreeMap<String, u64>,
    /// The sessions that saved a `session_summary`.
    pub with_summary: u64,
    pub with_summary_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionsReport {
    pub by: &'static str,
    pub since_ms: i64,
    pub idle_close_ms: i64,
    pub groups: Vec<Group>,
}

/// One session, each of its rows whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OneSession {
    pub session_id: String,
    pub rows: Vec<RowView>,
}

fn local_date(at_ms: i64, utc_offset_s: i64) -> String {
    let local_ms = at_ms + utc_offset_s * 1000;
    let (y, m, d) = crate::memory::store::backup::civil_from_days(local_ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

fn mean(total: i64, n: u64) -> f64 {
    if n == 0 {
        0.0
    } else {
        total as f64 / n as f64
    }
}

/// `rows` summed by `by`, as they read at `now_ms`, the groups in the order
/// of their keys with the sessions without a project first.
pub fn report(rows: &[SessionRow], by: SessionsBy, since_ms: i64, now_ms: i64) -> SessionsReport {
    let mut groups: BTreeMap<Option<String>, (Group, i64)> = BTreeMap::new();
    for row in rows {
        let view = RowView::of(row, now_ms);
        let key = match by {
            SessionsBy::Project => row.project_id.clone(),
            SessionsBy::Agent => Some(row.agent_id.clone()),
            SessionsBy::Day => Some(local_date(row.started_at_ms, row.utc_offset_s)),
        };
        let (group, since_save_total) = groups.entry(key.clone()).or_insert_with(|| (Group { key, ..Group::default() }, 0));
        group.sessions += 1;
        match &view.end_reason {
            Some(reason) => *group.ended.entry(reason.clone()).or_default() += 1,
            None => group.open += 1,
        }
        group.duration_ms.total += view.duration_ms;
        group.duration_ms.max = group.duration_ms.max.max(view.duration_ms);
        if let Some(since_save) = view.since_last_save_ms() {
            group.since_last_save_ms.sessions += 1;
            group.since_last_save_ms.max = group.since_last_save_ms.max.max(since_save);
            *since_save_total += since_save;
        }
        group.prompts += view.prompts;
        group.empty_searches += view.empty_searches;
        for (source, n) in &view.starts {
            *group.starts.entry(source.clone()).or_default() += n;
        }
        for (tool, n) in &view.calls {
            let total = group.calls.entry(tool.clone()).or_default();
            total.count += n.count;
            total.failed += n.failed;
        }
        for (memory_type, n) in &view.saves {
            *group.saves.entry(memory_type.clone()).or_default() += n;
        }
        if view.saves.get(SUMMARY).is_some_and(|n| *n > 0) {
            group.with_summary += 1;
        }
    }
    let groups = groups
        .into_values()
        .map(|(mut group, since_save_total)| {
            group.duration_ms.mean = mean(group.duration_ms.total, group.sessions);
            group.since_last_save_ms.mean = mean(since_save_total, group.since_last_save_ms.sessions);
            group.with_summary_pct = 100.0 * mean(group.with_summary as i64, group.sessions);
            group
        })
        .collect();
    SessionsReport { by: by.as_str(), since_ms, idle_close_ms: IDLE_CLOSE_MS, groups }
}

/// The rows of `session_id` among `rows`, as they read at `now_ms`.
pub fn one_session(session_id: &str, rows: &[SessionRow], now_ms: i64) -> OneSession {
    OneSession { session_id: session_id.to_string(), rows: rows.iter().map(|row| RowView::of(row, now_ms)).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    fn row(session_id: &str, agent_id: &str, project_id: Option<&str>, started_at_ms: i64, last_seen_at_ms: i64) -> SessionRow {
        SessionRow {
            session_id: session_id.into(),
            started_at_ms,
            agent_id: agent_id.into(),
            project_id: project_id.map(String::from),
            client: "claude-code".into(),
            utc_offset_s: 0,
            last_seen_at_ms,
            last_save_at_ms: None,
            ended_at_ms: None,
            end_reason: None,
            prompts: 0,
            empty_searches: 0,
            counts: BTreeMap::new(),
        }
    }

    fn ended(mut row: SessionRow, at_ms: i64, reason: &str) -> SessionRow {
        row.ended_at_ms = Some(at_ms);
        row.end_reason = Some(reason.into());
        row
    }

    fn counting(mut row: SessionRow, counts: &[(CountKind, &str, u64)]) -> SessionRow {
        for (kind, name, n) in counts {
            row.counts.insert((*kind, name.to_string()), *n);
        }
        row
    }

    #[test]
    fn a_report_groups_sessions_by_project_agent_and_day() {
        let start = NOW - 2 * HOUR;
        let mut summed = ended(row("a", "claude-code", Some("p"), start, start + 50 * MINUTE), start + 60 * MINUTE, "clear");
        summed.prompts = 4;
        summed.empty_searches = 1;
        let summed = counting(
            summed,
            &[
                (CountKind::Start, "startup", 1),
                (CountKind::Start, "compact", 2),
                (CountKind::Call, "search_memory", 5),
                (CountKind::FailedCall, "search_memory", 1),
                (CountKind::Call, "save_memory", 2),
                (CountKind::Save, "session_summary", 1),
                (CountKind::Save, "semantic_fact", 1),
            ],
        );
        let mut open = row("b", "cursor", Some("p"), start + 30 * MINUTE, NOW - 10 * MINUTE);
        open.prompts = 1;
        let open = counting(open, &[(CountKind::Call, "search_memory", 1), (CountKind::Start, "startup", 1)]);
        let mut loose = ended(row("c", "claude-code", None, start - 34 * HOUR, start - 33 * HOUR), start - 33 * HOUR, "logout");
        loose.utc_offset_s = -3 * 3600;
        let rows = [summed, open, loose];

        let by_project = report(&rows, SessionsBy::Project, 7, NOW);
        assert_eq!((by_project.by, by_project.since_ms, by_project.idle_close_ms), ("project", 7, IDLE_CLOSE_MS));
        assert_eq!(by_project.groups.iter().map(|g| g.key.as_deref()).collect::<Vec<_>>(), [None, Some("p")]);
        let p = &by_project.groups[1];
        assert_eq!((p.sessions, p.open, p.prompts, p.empty_searches), (2, 1, 5, 1));
        assert_eq!(p.ended, BTreeMap::from([("clear".to_string(), 1)]));
        assert_eq!(p.duration_ms, Durations { total: 140 * MINUTE, mean: 70.0 * MINUTE as f64, max: 80 * MINUTE });
        assert_eq!(p.starts, BTreeMap::from([("compact".to_string(), 2), ("startup".to_string(), 2)]));
        assert_eq!(p.calls["search_memory"], CallCount { count: 6, failed: 1 });
        assert_eq!(p.calls["save_memory"], CallCount { count: 2, failed: 0 });
        assert_eq!(p.saves, BTreeMap::from([("semantic_fact".to_string(), 1), ("session_summary".to_string(), 1)]));
        assert_eq!((p.with_summary, p.with_summary_pct), (1, 50.0));
        assert_eq!(by_project.groups[0].ended, BTreeMap::from([("logout".to_string(), 1)]));

        let by_agent = report(&rows, SessionsBy::Agent, 0, NOW);
        let agents: Vec<(Option<&str>, u64)> = by_agent.groups.iter().map(|g| (g.key.as_deref(), g.sessions)).collect();
        assert_eq!(agents, [(Some("claude-code"), 2), (Some("cursor"), 1)]);

        let by_day = report(&rows, SessionsBy::Day, 0, NOW);
        let days: Vec<(Option<&str>, u64)> = by_day.groups.iter().map(|g| (g.key.as_deref(), g.sessions)).collect();
        assert_eq!(days, [(Some("2026-09-19"), 1), (Some("2026-09-21"), 2)], "the local date of each start, which three hours behind UTC is the day before");

        let json = serde_json::to_value(&by_project).unwrap();
        assert_eq!(json["groups"][0]["key"], serde_json::Value::Null);
        assert_eq!(json["groups"][1]["calls"]["search_memory"], serde_json::json!({ "count": 6, "failed": 1 }));
        assert_eq!(report(&[], SessionsBy::Project, 0, NOW).groups, []);
    }

    #[test]
    fn an_open_session_past_six_hours_reads_as_idle() {
        let stale = row("stale", "a", Some("p"), NOW - 9 * HOUR, NOW - IDLE_CLOSE_MS);
        let fresh = row("fresh", "a", Some("p"), NOW - 9 * HOUR, NOW - IDLE_CLOSE_MS + 1);
        let closed = ended(row("closed", "a", Some("p"), NOW - 9 * HOUR, NOW - 8 * HOUR), NOW - 8 * HOUR, "clear");

        let report = report(&[stale.clone(), fresh.clone(), closed], SessionsBy::Project, 0, NOW);
        let p = &report.groups[0];
        assert_eq!((p.sessions, p.open), (3, 1));
        assert_eq!(p.ended, BTreeMap::from([("clear".to_string(), 1), ("idle".to_string(), 1)]));
        assert_eq!(with_summary(p), (0, 0.0));

        let one = one_session("stale", &[stale], NOW);
        assert_eq!((one.rows[0].ended_at_ms, one.rows[0].end_reason.as_deref()), (Some(NOW - IDLE_CLOSE_MS), Some("idle")));
        assert_eq!(one.rows[0].duration_ms, 3 * HOUR, "it ends at its last event");
        let one = one_session("fresh", &[fresh], NOW);
        assert_eq!((one.rows[0].ended_at_ms, one.rows[0].end_reason.as_deref()), (None, None));
    }

    fn with_summary(group: &Group) -> (u64, f64) {
        (group.with_summary, group.with_summary_pct)
    }

    #[test]
    fn the_time_since_the_last_save_counts_closed_sessions_that_saved() {
        let start = NOW - 5 * HOUR;
        let mut long_after = ended(row("a", "a", Some("p"), start, start + 5 * HOUR), start + 5 * HOUR, "prompt_input_exit");
        long_after.last_save_at_ms = Some(start + HOUR);
        let mut right_after = ended(row("b", "a", Some("p"), start, start + HOUR), start + HOUR, "clear");
        right_after.last_save_at_ms = Some(start + HOUR - 2 * MINUTE);
        let never_saved = ended(row("c", "a", Some("p"), start, start + HOUR), start + HOUR, "clear");
        let mut still_open = row("d", "a", Some("p"), start, NOW - MINUTE);
        still_open.last_save_at_ms = Some(start + MINUTE);

        let report = report(&[long_after, right_after, never_saved, still_open], SessionsBy::Project, 0, NOW);
        let since = report.groups[0].since_last_save_ms;
        assert_eq!((since.sessions, since.max), (2, 4 * HOUR));
        assert_eq!(since.mean, (4 * HOUR + 2 * MINUTE) as f64 / 2.0);
    }
}
