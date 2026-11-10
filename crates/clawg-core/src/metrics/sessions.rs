//! The log of client sessions: one row per session of an agent client, with
//! when it began, when it was last heard of, how it ended and what it did
//! meanwhile.
//!
//! Kept in memory and handed to the writer of `metrics.db` with each minute.
//! Nothing here reads a clock or a file: every call is given its time, so the
//! rules are tested with times as data.
//!
//! A row holds three ids (the client's session, the agent, the project) and,
//! besides them, only values of closed sets written here or by the caller:
//! never a text, a path or anything about the user.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// How long a session goes without an event before it is taken as ended, at
/// its last event. Whatever comes later under its id is another row.
pub const IDLE_CLOSE_MS: i64 = 6 * 60 * 60 * 1000;

/// Why a session nobody ended is closed.
pub const IDLE: &str = "idle";

/// What a value outside its closed set is kept as.
pub const OTHER: &str = "other";

/// What a start without a source, or a save without a type, is kept as.
pub const UNSPECIFIED: &str = "unspecified";

/// What starts a session, as Claude Code names it.
pub const SOURCES: [&str; 5] = ["startup", "resume", "clear", "compact", "fork"];

/// Why a client says a session ended: Claude Code's reasons, then Cursor's.
pub const END_REASONS: [&str; 9] =
    ["clear", "resume", "logout", "prompt_input_exit", "completed", "aborted", "error", "window_close", "user_close"];

pub const CLIENTS: [&str; 2] = ["claude-code", "cursor"];

/// `raw` as `set` writes it, or [`OTHER`]: what is kept is never the text
/// that came in.
pub fn label(set: &[&'static str], raw: &str) -> &'static str {
    set.iter().copied().find(|known| *known == raw).unwrap_or(OTHER)
}

/// Whether a row last heard of at `last_seen_at_ms` has ended by `at_ms`.
pub fn idle_closed(last_seen_at_ms: i64, at_ms: i64) -> bool {
    at_ms.saturating_sub(last_seen_at_ms) >= IDLE_CLOSE_MS
}

/// What a named count of a session counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CountKind {
    /// Starts, by source.
    Start,
    /// Tool calls, by tool.
    Call,
    /// The tool calls that failed, by tool.
    FailedCall,
    /// Accepted saves, by memory type.
    Save,
}

impl CountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CountKind::Start => "start",
            CountKind::Call => "call",
            CountKind::FailedCall => "failed_call",
            CountKind::Save => "save",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [CountKind::Start, CountKind::Call, CountKind::FailedCall, CountKind::Save].into_iter().find(|k| k.as_str() == raw)
    }
}

/// One row of the log. A session has one, or one more each time it went on
/// after it was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub session_id: String,
    pub started_at_ms: i64,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub client: String,
    /// Local time minus UTC when the row began, in seconds.
    pub utc_offset_s: i64,
    pub last_seen_at_ms: i64,
    pub last_save_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<String>,
    pub prompts: u64,
    pub empty_searches: u64,
    pub counts: BTreeMap<(CountKind, String), u64>,
}

impl SessionRow {
    pub fn count(&self, kind: CountKind, name: &str) -> u64 {
        self.counts.get(&(kind, name.to_string())).copied().unwrap_or(0)
    }

    fn add(&mut self, kind: CountKind, name: &'static str) {
        *self.counts.entry((kind, name.to_string())).or_insert(0) += 1;
    }

    fn close(&mut self, at_ms: i64, reason: &'static str) {
        self.ended_at_ms = Some(at_ms.max(self.last_seen_at_ms));
        self.end_reason = Some(reason.to_string());
    }
}

/// Who a hook says a session is.
#[derive(Debug, Clone, Copy)]
pub struct Who<'a> {
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub project_id: Option<&'a str>,
    /// One of [`CLIENTS`], or [`OTHER`].
    pub client: &'static str,
    pub utc_offset_s: i64,
}

/// What a hook reports of a session, with its exact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// The session started or came back, from one of [`SOURCES`].
    Start(&'static str),
    Prompt,
    /// The session is there, and nothing else is said.
    Seen,
}

/// What reaches the daemon without a session id and is counted for the
/// session its agent and project are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Call { tool: &'static str, failed: bool },
    /// An accepted save, by its memory type.
    Save(&'static str),
    EmptySearch,
}

/// What happened to a project whose sessions the stored rows must follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectChange {
    Merged { from: String, into: String },
    Deleted(String),
}

/// What the log holds that `metrics.db` does not have yet: the project
/// changes first, then every row that changed, whole.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pending {
    pub changes: Vec<ProjectChange>,
    pub rows: Vec<SessionRow>,
}

#[derive(Debug)]
struct Open {
    row: SessionRow,
    dirty: bool,
}

/// What is kept of a closed row, so that activity under its id opens the
/// next one as the same session.
#[derive(Debug, Clone)]
struct Identity {
    agent_id: String,
    project_id: Option<String>,
    client: String,
    utc_offset_s: i64,
    started_at_ms: i64,
}

#[derive(Debug, Default)]
struct State {
    open: HashMap<String, Open>,
    closed: Vec<SessionRow>,
    gone: HashMap<String, Identity>,
    changes: Vec<ProjectChange>,
}

impl State {
    fn close(&mut self, session_id: &str, at_ms: i64, reason: &'static str) {
        let Some(mut open) = self.open.remove(session_id) else { return };
        open.row.close(at_ms, reason);
        let row = open.row;
        self.gone.insert(
            row.session_id.clone(),
            Identity {
                agent_id: row.agent_id.clone(),
                project_id: row.project_id.clone(),
                client: row.client.clone(),
                utc_offset_s: row.utc_offset_s,
                started_at_ms: row.started_at_ms,
            },
        );
        self.closed.push(row);
    }

    /// Closes the open row of `session_id` when it went [`IDLE_CLOSE_MS`]
    /// without an event before `at_ms`.
    fn settle(&mut self, session_id: &str, at_ms: i64) {
        let idle_since = self.open.get(session_id).map(|open| open.row.last_seen_at_ms).filter(|seen| idle_closed(*seen, at_ms));
        if let Some(last_seen) = idle_since {
            self.close(session_id, last_seen, IDLE);
        }
    }

    /// The open row of `session_id`, opened from `identity` when it has none.
    /// A row never begins at the time of the one before it: the two would be
    /// one key.
    fn row(&mut self, session_id: &str, at_ms: i64, identity: impl FnOnce() -> Option<Identity>) -> Option<&mut SessionRow> {
        if !self.open.contains_key(session_id) {
            let identity = identity()?;
            let started_at_ms = match self.gone.remove(session_id) {
                Some(before) => at_ms.max(before.started_at_ms + 1),
                None => at_ms,
            };
            let row = SessionRow {
                session_id: session_id.to_string(),
                started_at_ms,
                agent_id: identity.agent_id,
                project_id: identity.project_id,
                client: identity.client,
                utc_offset_s: identity.utc_offset_s,
                last_seen_at_ms: started_at_ms,
                last_save_at_ms: None,
                ended_at_ms: None,
                end_reason: None,
                prompts: 0,
                empty_searches: 0,
                counts: BTreeMap::new(),
            };
            self.open.insert(session_id.to_string(), Open { row, dirty: true });
        }
        let open = self.open.get_mut(session_id)?;
        open.dirty = true;
        open.row.last_seen_at_ms = open.row.last_seen_at_ms.max(at_ms);
        Some(&mut open.row)
    }
}

/// The sessions of this run of the daemon, and what of them is not written
/// yet. Off until the writer says the metrics are kept: with nowhere to
/// write, nothing is held.
#[derive(Debug, Default)]
pub struct SessionLog {
    state: Mutex<State>,
    enabled: AtomicBool,
    changed: tokio::sync::Notify,
}

impl SessionLog {
    /// Turns the log on or off; off drops what it holds.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            *self.lock() = State::default();
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Takes up the open rows an earlier run left, as they were stored.
    pub fn restore(&self, rows: Vec<SessionRow>) {
        if !self.enabled() {
            return;
        }
        let mut state = self.lock();
        for row in rows.into_iter().filter(|row| row.ended_at_ms.is_none()) {
            state.open.entry(row.session_id.clone()).or_insert(Open { row, dirty: false });
        }
    }

    /// Counts `event` of the session `who` names at `at_ms`, opening its row
    /// when it has none open.
    pub fn hook(&self, who: Who<'_>, at_ms: i64, event: HookEvent) {
        if !self.enabled() {
            return;
        }
        let mut state = self.lock();
        state.settle(who.session_id, at_ms);
        let identity = || {
            Some(Identity {
                agent_id: who.agent_id.to_string(),
                project_id: who.project_id.map(str::to_string),
                client: who.client.to_string(),
                utc_offset_s: who.utc_offset_s,
                started_at_ms: at_ms,
            })
        };
        let Some(row) = state.row(who.session_id, at_ms, identity) else { return };
        match event {
            HookEvent::Start(source) => row.add(CountKind::Start, source),
            HookEvent::Prompt => row.prompts += 1,
            HookEvent::Seen => {}
        }
    }

    /// Counts `activity` for `session_id` at `at_ms`. A session closed in
    /// this run goes on in a new row; one this run never had a row of is
    /// not counted, since only a hook says whose it is.
    pub fn activity(&self, session_id: &str, at_ms: i64, activity: Activity) {
        if !self.enabled() {
            return;
        }
        let mut state = self.lock();
        state.settle(session_id, at_ms);
        let before = state.gone.get(session_id).cloned();
        let Some(row) = state.row(session_id, at_ms, || before) else { return };
        match activity {
            Activity::Call { tool, failed } => {
                row.add(CountKind::Call, tool);
                if failed {
                    row.add(CountKind::FailedCall, tool);
                }
            }
            Activity::Save(memory_type) => {
                row.add(CountKind::Save, memory_type);
                row.last_save_at_ms = Some(row.last_seen_at_ms);
            }
            Activity::EmptySearch => row.empty_searches += 1,
        }
    }

    /// Closes the open row of `session_id` at `at_ms` for `reason`. One that
    /// already went [`IDLE_CLOSE_MS`] without an event stays closed as idle,
    /// and without an open row nothing happens.
    pub fn end(&self, session_id: &str, at_ms: i64, reason: &'static str) {
        if !self.enabled() {
            return;
        }
        let mut state = self.lock();
        state.settle(session_id, at_ms);
        state.close(session_id, at_ms, reason);
    }

    /// Closes as idle every open row that went [`IDLE_CLOSE_MS`] without an
    /// event before `now_ms`.
    pub fn close_idle(&self, now_ms: i64) {
        let mut state = self.lock();
        let idle: Vec<String> =
            state.open.iter().filter(|(_, open)| idle_closed(open.row.last_seen_at_ms, now_ms)).map(|(id, _)| id.clone()).collect();
        for session_id in idle {
            state.settle(&session_id, now_ms);
        }
    }

    /// The sessions of `from` are `into`'s from now on, stored ones included.
    pub fn merge_project(&self, from: &str, into: &str) {
        if !self.enabled() {
            return;
        }
        {
            let mut state = self.lock();
            let State { open, closed, gone, changes } = &mut *state;
            let projects = open
                .values_mut()
                .map(|o| &mut o.row.project_id)
                .chain(closed.iter_mut().map(|row| &mut row.project_id))
                .chain(gone.values_mut().map(|identity| &mut identity.project_id));
            for project in projects.filter(|p| p.as_deref() == Some(from)) {
                *project = Some(into.to_string());
            }
            changes.push(ProjectChange::Merged { from: from.to_string(), into: into.to_string() });
        }
        self.changed.notify_one();
    }

    /// Drops the sessions of `project_id`, stored ones included.
    pub fn delete_project(&self, project_id: &str) {
        if !self.enabled() {
            return;
        }
        {
            let mut state = self.lock();
            let of_project = |project: &Option<String>| project.as_deref() == Some(project_id);
            state.open.retain(|_, open| !of_project(&open.row.project_id));
            state.closed.retain(|row| !of_project(&row.project_id));
            state.gone.retain(|_, identity| !of_project(&identity.project_id));
            state.changes.push(ProjectChange::Deleted(project_id.to_string()));
        }
        self.changed.notify_one();
    }

    /// Waits for a change that must reach the file before the minute turns.
    pub async fn changed(&self) {
        self.changed.notified().await;
    }

    /// What is not written yet, leaving nothing pending.
    pub fn take(&self) -> Pending {
        let mut state = self.lock();
        let mut rows = std::mem::take(&mut state.closed);
        for open in state.open.values_mut().filter(|open| open.dirty) {
            open.dirty = false;
            rows.push(open.row.clone());
        }
        rows.sort_by(|a, b| (a.started_at_ms, &a.session_id).cmp(&(b.started_at_ms, &b.session_id)));
        Pending { changes: std::mem::take(&mut state.changes), rows }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while the lock was held leaves counts, never a broken
        // invariant: go on with them.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_790_000_000_000;
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    fn log() -> SessionLog {
        let log = SessionLog::default();
        log.set_enabled(true);
        log
    }

    fn who<'a>(session_id: &'a str, project_id: Option<&'a str>) -> Who<'a> {
        Who { session_id, agent_id: "agent", project_id, client: "claude-code", utc_offset_s: -10_800 }
    }

    fn rows_of(log: &SessionLog, session_id: &str) -> Vec<SessionRow> {
        log.take().rows.into_iter().filter(|row| row.session_id == session_id).collect()
    }

    #[test]
    fn a_session_opens_on_its_first_event_and_counts_what_follows() {
        let log = log();
        log.hook(who("s", Some("p")), T, HookEvent::Start("startup"));
        log.hook(who("s", Some("q")), T + MINUTE, HookEvent::Prompt);
        log.activity("s", T + 2 * MINUTE, Activity::Call { tool: "search_memory", failed: false });
        log.activity("s", T + 3 * MINUTE, Activity::Call { tool: "search_memory", failed: true });
        log.activity("s", T + 4 * MINUTE, Activity::EmptySearch);
        log.activity("s", T + 5 * MINUTE, Activity::Save("session_summary"));
        log.hook(who("s", Some("p")), T + 6 * MINUTE, HookEvent::Start("compact"));
        log.hook(who("s", Some("p")), T + 7 * MINUTE, HookEvent::Seen);

        let rows = rows_of(&log, "s");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!((row.started_at_ms, row.last_seen_at_ms), (T, T + 7 * MINUTE));
        assert_eq!((row.agent_id.as_str(), row.client.as_str(), row.utc_offset_s), ("agent", "claude-code", -10_800));
        assert_eq!(row.project_id.as_deref(), Some("p"), "the project of the event that opened it");
        assert_eq!((row.prompts, row.empty_searches), (1, 1));
        assert_eq!((row.count(CountKind::Start, "startup"), row.count(CountKind::Start, "compact")), (1, 1));
        assert_eq!((row.count(CountKind::Call, "search_memory"), row.count(CountKind::FailedCall, "search_memory")), (2, 1));
        assert_eq!(row.count(CountKind::Save, "session_summary"), 1);
        assert_eq!(row.last_save_at_ms, Some(T + 5 * MINUTE));
        assert_eq!((row.ended_at_ms, row.end_reason.as_deref()), (None, None));
        assert_eq!(log.take(), Pending::default(), "what was taken is not pending any more");
    }

    #[test]
    fn activity_of_an_unknown_session_is_dropped() {
        let log = log();
        log.activity("nobody", T, Activity::EmptySearch);
        log.end("nobody", T, "clear");
        assert_eq!(log.take(), Pending::default());

        let off = SessionLog::default();
        off.hook(who("s", None), T, HookEvent::Prompt);
        assert_eq!(off.take(), Pending::default(), "a log that is off holds nothing");
    }

    #[test]
    fn a_name_outside_its_set_is_other() {
        assert_eq!(label(&SOURCES, "clear"), "clear");
        assert_eq!(label(&SOURCES, "a prompt someone typed"), OTHER);
        assert_eq!(label(&END_REASONS, "window_close"), "window_close");
        assert_eq!(label(&END_REASONS, "/home/someone"), OTHER);
        assert_eq!(label(&CLIENTS, "cursor"), "cursor");
        assert_eq!(label(&CLIENTS, ""), OTHER);
        for kind in [CountKind::Start, CountKind::Call, CountKind::FailedCall, CountKind::Save] {
            assert_eq!(CountKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(CountKind::parse("prompt"), None);
    }

    #[test]
    fn six_hours_without_activity_close_a_session_as_idle() {
        assert!(!idle_closed(T, T + 6 * HOUR - 1));
        assert!(idle_closed(T, T + 6 * HOUR));

        let log = log();
        log.hook(who("s", Some("p")), T, HookEvent::Start("startup"));
        let second = T + 5 * HOUR + 59 * MINUTE;
        log.hook(who("s", Some("p")), second, HookEvent::Prompt);
        let rows = rows_of(&log, "s");
        assert_eq!(rows.len(), 1, "five hours and 59 minutes later it is the same row");
        assert_eq!((rows[0].prompts, rows[0].ended_at_ms), (1, None));

        let third = second + 6 * HOUR;
        log.hook(who("s", Some("p")), third, HookEvent::Prompt);
        let rows = rows_of(&log, "s");
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].started_at_ms, rows[0].ended_at_ms, rows[0].end_reason.as_deref()), (T, Some(second), Some(IDLE)));
        assert_eq!((rows[1].started_at_ms, rows[1].prompts, rows[1].ended_at_ms), (third, 1, None));
    }

    #[test]
    fn a_closed_session_is_not_reopened() {
        let log = log();
        log.hook(who("s", Some("p")), T, HookEvent::Start("startup"));
        log.end("s", T + MINUTE, "clear");
        log.end("s", T + 2 * MINUTE, "logout");
        log.activity("s", T + 3 * MINUTE, Activity::Call { tool: "memory_status", failed: false });

        let rows = rows_of(&log, "s");
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].ended_at_ms, rows[0].end_reason.as_deref()), (Some(T + MINUTE), Some("clear")));
        assert_eq!(rows[0].count(CountKind::Call, "memory_status"), 0);
        let again = &rows[1];
        assert_eq!((again.started_at_ms, again.ended_at_ms), (T + 3 * MINUTE, None));
        assert_eq!((again.agent_id.as_str(), again.project_id.as_deref(), again.client.as_str()), ("agent", Some("p"), "claude-code"));
        assert_eq!(again.count(CountKind::Call, "memory_status"), 1);

        log.end("s", T + 4 * MINUTE, "logout");
        log.hook(who("s", Some("p")), T + 4 * MINUTE, HookEvent::Prompt);
        let rows = rows_of(&log, "s");
        assert_eq!(rows.iter().map(|r| r.started_at_ms).collect::<Vec<_>>(), [T + 3 * MINUTE, T + 4 * MINUTE]);
        assert_eq!(rows[0].end_reason.as_deref(), Some("logout"));
    }

    #[test]
    fn a_row_opened_as_the_one_before_it_closed_begins_a_millisecond_later() {
        let log = log();
        log.hook(who("s", None), T, HookEvent::Prompt);
        log.end("s", T, "clear");
        log.hook(who("s", None), T, HookEvent::Prompt);
        assert_eq!(rows_of(&log, "s").iter().map(|r| r.started_at_ms).collect::<Vec<_>>(), [T, T + 1]);
    }

    #[test]
    fn an_end_after_six_idle_hours_leaves_the_row_idle() {
        let log = log();
        log.hook(who("s", None), T, HookEvent::Start("startup"));
        log.end("s", T + 8 * HOUR, "prompt_input_exit");
        let rows = rows_of(&log, "s");
        assert_eq!(rows.len(), 1, "an end opens nothing");
        assert_eq!((rows[0].ended_at_ms, rows[0].end_reason.as_deref()), (Some(T), Some(IDLE)));

        log.hook(who("t", None), T, HookEvent::Start("startup"));
        log.hook(who("u", None), T + 5 * HOUR, HookEvent::Start("startup"));
        log.take();
        log.close_idle(T + 6 * HOUR);
        let closed = log.take().rows;
        assert_eq!(closed.iter().map(|r| (r.session_id.as_str(), r.end_reason.as_deref())).collect::<Vec<_>>(), [("t", Some(IDLE))]);
    }

    #[test]
    fn merging_a_project_moves_its_sessions_and_deleting_it_drops_them() {
        let log = log();
        log.hook(who("open", Some("old")), T, HookEvent::Prompt);
        log.hook(who("closed", Some("old")), T, HookEvent::Prompt);
        log.end("closed", T + MINUTE, "clear");
        log.hook(who("apart", Some("kept")), T, HookEvent::Prompt);
        log.merge_project("old", "new");

        let pending = log.take();
        assert_eq!(pending.changes, [ProjectChange::Merged { from: "old".into(), into: "new".into() }]);
        let projects: BTreeMap<&str, Option<&str>> =
            pending.rows.iter().map(|r| (r.session_id.as_str(), r.project_id.as_deref())).collect();
        assert_eq!(projects, BTreeMap::from([("open", Some("new")), ("closed", Some("new")), ("apart", Some("kept"))]));
        log.activity("closed", T + 2 * MINUTE, Activity::EmptySearch);
        assert_eq!(log.take().rows[0].project_id.as_deref(), Some("new"), "what follows a closed row keeps the project it went to");

        log.delete_project("new");
        log.hook(who("apart", Some("kept")), T + 3 * MINUTE, HookEvent::Prompt);
        let pending = log.take();
        assert_eq!(pending.changes, [ProjectChange::Deleted("new".into())]);
        assert_eq!(pending.rows.iter().map(|r| r.session_id.as_str()).collect::<Vec<_>>(), ["apart"]);
        log.activity("open", T + 4 * MINUTE, Activity::EmptySearch);
        log.activity("closed", T + 4 * MINUTE, Activity::EmptySearch);
        assert_eq!(log.take(), Pending::default(), "nothing is left to count for a deleted project's sessions");
    }
}
