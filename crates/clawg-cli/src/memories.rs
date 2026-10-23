//! `clawg memories`: a read-only view of what the daemon has stored.
//!
//! Everything that decides how something looks is a pure function from data and
//! a timestamp to a `String`, so the whole output is testable without a daemon
//! or a clock. The only I/O is in [`run`], which asks the daemon for rows and
//! hands them to those functions.

use std::fmt::Write as _;

use clawg_core::paths::Profile;
use clawg_core::preferences::Lang;

use crate::cli::{Filters, ListOpts, MemoriesAction, Scope, SearchOpts};
use crate::client::{ClientError, Count, DaemonClient, ListArgs, MemoryRow, MemoryStats};
use crate::mcp::tools::truncate_content;

/// Characters of content a table row shows before it is cut. `show` prints the
/// rest.
pub const CONTENT_WIDTH: usize = 60;

const COLUMNS: [&str; 10] = [
    "KIND", "ID", "AGE", "T", "IMP", "TYPE", "AGENT", "PROJECT", "STATUS", "CONTENT",
];
/// Numbers read better flush right; everything else flush left.
const RIGHT_ALIGNED: [bool; 10] = [
    false, true, true, true, true, false, false, false, false, false,
];

// ── Time ────────────────────────────────────────────────────────────────────

/// How long before `now_ms` the moment `then_ms` was, as `12s`, `5m`, `3h` or
/// `2d`. A moment in the future (clock skew between writer and reader) is `0s`
/// rather than a negative age.
pub fn format_age(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms.saturating_sub(then_ms) / 1000).max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3_599 => format!("{}m", secs / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Epoch milliseconds as `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Computed here because the workspace has no date library and one conversion
/// is not worth adding one: a day count becomes a civil date with Howard
/// Hinnant's `civil_from_days`, which is exact across leap years and centuries.
pub fn format_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let in_day = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        in_day / 3_600,
        in_day % 3_600 / 60,
        in_day % 60
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian
/// calendar.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `2026-10-04 23:32:02 UTC (12s ago)`, or `-` for a missing moment.
fn stamp(ms: Option<i64>, now_ms: i64) -> String {
    match ms {
        Some(t) => format!("{} ({} ago)", format_utc(t), format_age(now_ms, t)),
        None => "-".to_string(),
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn or_dash(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "-".to_string())
}

fn or_dash_num(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| v.to_string())
}

/// The listing: a header, one line per row, and a footer. Columns are as wide
/// as their widest cell, measured in characters so accents and emoji do not
/// push the later columns out. The content is last and unpadded, so a cut-off
/// cell leaves no trailing blanks.
///
/// `limit` and `offset` are what was asked for: a page that came back full
/// means there may be more, and the footer says where to continue.
pub fn render_table(lang: Lang, rows: &[MemoryRow], now_ms: i64, limit: u32, offset: u32) -> String {
    if rows.is_empty() {
        return match lang {
            Lang::En => "no memories match",
            Lang::Es => "ninguna memoria coincide",
        }
        .to_string();
    }
    render_rows(lang, rows, now_ms, limit, offset, Listing::Live)
}

/// The listing of deleted rows: the age column says how long ago each was
/// deleted, and the footer how to bring one back.
pub fn render_deleted_table(lang: Lang, rows: &[MemoryRow], now_ms: i64, limit: u32, offset: u32) -> String {
    if rows.is_empty() {
        return match lang {
            Lang::En => "no deleted memories match",
            Lang::Es => "ninguna memoria borrada coincide",
        }
        .to_string();
    }
    render_rows(lang, rows, now_ms, limit, offset, Listing::Deleted)
}

#[derive(Clone, Copy)]
enum Listing {
    Live,
    Deleted,
}

fn render_rows(lang: Lang, rows: &[MemoryRow], now_ms: i64, limit: u32, offset: u32, listing: Listing) -> String {
    let mut header = COLUMNS.map(str::to_owned);
    if let Listing::Deleted = listing {
        header[2] = "DELETED".to_owned();
    }
    let mut cells: Vec<[String; 10]> = Vec::with_capacity(rows.len() + 1);
    cells.push(header);
    for r in rows {
        let moment = match listing {
            Listing::Live => r.created_at,
            Listing::Deleted => r.deleted_at,
        };
        cells.push([
            r.kind.clone(),
            r.id.to_string(),
            moment.map_or_else(|| "-".to_string(), |t| format_age(now_ms, t)),
            r.tier.to_string(),
            format!("{:.1}", r.importance),
            or_dash(&r.memory_type),
            or_dash(&r.agent_id),
            or_dash(&r.project_id),
            r.status.clone(),
            truncate_content(&r.summary, CONTENT_WIDTH),
        ]);
    }

    let mut widths = [0usize; 10];
    for line in &cells {
        for (w, cell) in widths.iter_mut().zip(line) {
            *w = (*w).max(cell.chars().count());
        }
    }

    let mut out = String::new();
    for line in &cells {
        let last = line.len() - 1;
        for (i, cell) in line.iter().enumerate() {
            if i == last {
                out.push_str(cell);
                continue;
            }
            let pad = " ".repeat(widths[i] - cell.chars().count());
            if RIGHT_ALIGNED[i] {
                out.push_str(&pad);
                out.push_str(cell);
            } else {
                out.push_str(cell);
                out.push_str(&pad);
            }
            out.push_str("  ");
        }
        out.push('\n');
    }

    let shown = rows.len() as u64;
    let (count, order) = match (listing, lang) {
        (Listing::Live, Lang::En) => (plural(shown, "row", "rows"), "newest first"),
        (Listing::Deleted, Lang::En) => (plural(shown, "row", "rows"), "most recently deleted first"),
        (Listing::Live, Lang::Es) => (plural(shown, "fila", "filas"), "las más nuevas primero"),
        (Listing::Deleted, Lang::Es) => (plural(shown, "fila", "filas"), "las borradas más recientemente primero"),
    };
    let _ = write!(out, "{count}, {order}.");
    if shown >= u64::from(limit) {
        let next = u64::from(offset) + shown;
        let _ = match lang {
            Lang::En => write!(out, " More: --offset {next}"),
            Lang::Es => write!(out, " Más: --offset {next}"),
        };
    }
    if matches!(listing, Listing::Deleted) && rows.iter().any(|r| r.kind == "memory") {
        out.push_str(match lang {
            Lang::En => " Undo a memory row with 'clawg undelete <ID>'.",
            Lang::Es => " Una fila memory vuelve con 'clawg undelete <ID>'.",
        });
    }
    out
}

/// One row in full: the whole content, then every piece of metadata.
pub fn render_detail(row: &MemoryRow, now_ms: i64) -> String {
    let state = if row.deleted_at.is_some() {
        "deleted"
    } else {
        row.status.as_str()
    };
    let mut out = format!("{} #{} [{state}]\ncontent:\n", row.kind, row.id);
    for line in row.summary.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "  {line}");
        }
    }

    let created_by = match (&row.created_by_type, row.created_by_ref.as_deref()) {
        (Some(kind), Some(who)) if !who.is_empty() => format!("{kind} ({who})"),
        (Some(kind), _) => kind.clone(),
        (None, _) => "-".to_string(),
    };
    let mut grid: Vec<(&str, String)> = vec![
        ("type", or_dash(&row.memory_type)),
        ("tier", row.tier.to_string()),
        ("importance", format!("{:.1}", row.importance)),
        ("agent", or_dash(&row.agent_id)),
        ("project", or_dash(&row.project_id)),
        ("scope", or_dash(&row.scope)),
        ("source", or_dash(&row.source)),
        ("created by", created_by),
        ("session", or_dash(&row.source_session_id)),
        ("created", stamp(row.created_at, now_ms)),
        ("last access", stamp(row.last_accessed_at, now_ms)),
        ("access count", row.access_count.to_string()),
        ("modified", stamp(row.last_modified_at, now_ms)),
        ("updates", or_dash_num(row.update_count)),
        ("disputes", or_dash_num(row.dispute_count)),
        ("against", or_dash(&row.disputed_against)),
        ("superseded by", or_dash_num(row.superseded_by)),
        ("deleted", stamp(row.deleted_at, now_ms)),
    ];
    // Only procedural rows have these; showing a dash for the others would be
    // noise on every memory.
    if let Some(trigger) = &row.trigger {
        grid.push(("trigger", trigger.clone()));
    }
    if let Some(n) = row.success_count {
        grid.push(("successes", n.to_string()));
    }

    let label_width = grid.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 2;
    for (label, value) in grid {
        let _ = writeln!(out, "{:<label_width$}{value}", format!("{label}:"));
    }
    out.truncate(out.trim_end().len());
    out
}

/// Totals first, then each grouping on one line as `key count | key count`.
pub fn render_stats(lang: Lang, stats: &MemoryStats, now_ms: i64) -> String {
    let t = &stats.totals;
    let live = t.memory + t.episodic + t.procedural;
    let mut out = match lang {
        Lang::En => format!(
            "{} (memory {}, episodic {}, procedural {}), {} deleted.",
            plural(live, "live row", "live rows"),
            t.memory,
            t.episodic,
            t.procedural,
            t.deleted
        ),
        Lang::Es => format!(
            "{} (memory {}, episodic {}, procedural {}), {} {}.",
            plural(live, "fila viva", "filas vivas"),
            t.memory,
            t.episodic,
            t.procedural,
            t.deleted,
            if t.deleted == 1 { "borrada" } else { "borradas" }
        ),
    };
    if let Some(newest) = stats.newest_created_at {
        let age = format_age(now_ms, newest);
        let _ = match lang {
            Lang::En => write!(out, " Newest {age} ago."),
            Lang::Es => write!(out, " La más nueva, de hace {age}."),
        };
    }
    out.push('\n');

    let labels = match lang {
        Lang::En => ["by type:", "by tier:", "by status:", "by agent:", "by project:"],
        Lang::Es => ["por tipo:", "por tier:", "por estado:", "por agente:", "por proyecto:"],
    };
    let none = match lang {
        Lang::En => "(none)",
        Lang::Es => "(ninguno)",
    };
    let sections: [(&str, &[Count]); 5] = [
        (labels[0], &stats.by_type),
        (labels[1], &stats.by_tier),
        (labels[2], &stats.by_status),
        (labels[3], &stats.by_agent),
        (labels[4], &stats.by_project),
    ];
    let width = sections.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0) + 1;
    for (label, counts) in sections {
        let body = if counts.is_empty() {
            "-".to_string()
        } else {
            counts
                .iter()
                .map(|c| format!("{} {}", c.key.as_deref().unwrap_or(none), c.count))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let _ = write!(out, "\n{label:<width$}{body}");
    }
    out
}

fn json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| format!("{{\"error\": \"could not serialize: {e}\"}}"))
}

/// The rows as a JSON array, under the API's own field names.
pub fn rows_json(rows: &[MemoryRow]) -> String {
    json(rows)
}

/// One row as a JSON object.
pub fn row_json(row: &MemoryRow) -> String {
    json(row)
}

/// The stats as a JSON object.
pub fn stats_json(stats: &MemoryStats) -> String {
    json(stats)
}

// ── Messages ────────────────────────────────────────────────────────────────

/// What to tell the user when a read could not be served.
///
/// A viewer never starts a daemon, so "not running" has to say how to start one.
pub fn explain_unavailable(lang: Lang, err: &ClientError, profile: Profile) -> String {
    let profile = profile.as_str();
    match (err, lang) {
        (ClientError::NotRunning, Lang::En) => {
            format!("no clawg daemon is running for profile {profile}; start it with 'clawg serve --profile {profile}'")
        }
        (ClientError::NotRunning, Lang::Es) => format!(
            "no hay ningún daemon de clawg corriendo para el perfil {profile}; arráncalo con 'clawg serve --profile {profile}'"
        ),
        (other, _) => other.to_string(),
    }
}

/// A warning when the daemon answering is not the version of this binary: the
/// viewer reads columns an older daemon does not send, so its output would be
/// thinner than it looks.
pub fn version_warning(lang: Lang, daemon_version: &str, cli_version: &str) -> Option<String> {
    (daemon_version != cli_version).then(|| match lang {
        Lang::En => format!(
            "warning: the running daemon is version {daemon_version} but this clawg is \
             {cli_version}; run 'clawg stop' so the next start uses this version"
        ),
        Lang::Es => format!(
            "aviso: el daemon que corre es la versión {daemon_version} pero este clawg es la \
             {cli_version}; ejecuta 'clawg stop' para que el próximo arranque use esta versión"
        ),
    })
}

// ── Requests ────────────────────────────────────────────────────────────────

fn filter_args(filters: &Filters, project: Option<String>) -> ListArgs {
    ListArgs {
        tier: filters.tier,
        memory_type: filters.memory_type.clone(),
        agent: filters.scope.agent.clone(),
        project,
        no_project: filters.no_project,
        status: filters.status.map(|s| s.as_str().to_owned()),
        ..ListArgs::default()
    }
}

/// `project` is the already-resolved project id: `--here` is turned into one by
/// the caller, which is the part that reads the working directory.
pub fn list_args(opts: &ListOpts, project: Option<String>) -> ListArgs {
    ListArgs {
        kind: opts.kind.map(|k| k.as_str().to_owned()),
        limit: Some(opts.limit),
        offset: Some(opts.offset),
        deleted: opts.deleted,
        origin_project: opts.origin_project.clone(),
        ..filter_args(&opts.filters, project)
    }
}

/// A search names no kind: the daemon restricts a text query to `memory` rows,
/// the only ones with a text index.
pub fn search_args(opts: &SearchOpts, project: Option<String>) -> ListArgs {
    ListArgs {
        q: Some(opts.query.clone()),
        limit: Some(opts.limit),
        offset: Some(opts.offset),
        ..filter_args(&opts.filters, project)
    }
}

fn resolve_project(scope: &Scope) -> Result<Option<String>, ClientError> {
    if !scope.here {
        return Ok(scope.project_id(None));
    }
    let cwd = std::env::current_dir().map_err(|e| ClientError::WorkingDir(e.to_string()))?;
    Ok(scope.project_id(Some(clawg_core::memory::ProjectIdentity::resolve(&cwd).id)))
}

// ── Running ─────────────────────────────────────────────────────────────────

/// Runs one `clawg memories` action against a live daemon and returns what to
/// print.
///
/// Warns on stderr, rather than failing, when the daemon is a different version
/// than this binary.
pub async fn run(
    lang: Lang,
    client: &DaemonClient,
    action: MemoriesAction,
    now_ms: i64,
) -> Result<String, ClientError> {
    let health = client.health().await?;
    if let Some(warning) = version_warning(lang, &health.version, clawg_core::build_info::version()) {
        eprintln!("{warning}");
    }

    match action {
        MemoriesAction::List(opts) => {
            let args = list_args(&opts, resolve_project(&opts.filters.scope)?);
            let rows = client.list(args).await?;
            Ok(if opts.json {
                rows_json(&rows)
            } else if opts.deleted {
                render_deleted_table(lang, &rows, now_ms, opts.limit, opts.offset)
            } else {
                render_table(lang, &rows, now_ms, opts.limit, opts.offset)
            })
        }
        MemoriesAction::Search(opts) => {
            let args = search_args(&opts, resolve_project(&opts.filters.scope)?);
            let rows = client.list(args).await?;
            Ok(if opts.json {
                rows_json(&rows)
            } else {
                render_table(lang, &rows, now_ms, opts.limit, opts.offset)
            })
        }
        MemoriesAction::Show(opts) => {
            let row = client.get(opts.kind.as_str(), opts.id).await?;
            Ok(if opts.json {
                row_json(&row)
            } else {
                render_detail(&row, now_ms)
            })
        }
        MemoriesAction::Stats(opts) => {
            let project = resolve_project(&opts.scope)?;
            let stats = client
                .stats(opts.scope.agent.as_deref(), project.as_deref())
                .await?;
            Ok(if opts.json {
                stats_json(&stats)
            } else {
                render_stats(lang, &stats, now_ms)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::{
        Filters, KindArg, ListOpts, ProfileArg, Scope, SearchOpts, ShowOpts, StatsOpts, StatusArg,
    };
    use crate::client::tests::{publish, save_and_wait, start_daemon};
    use crate::client::{Count, DaemonClient, MemoryStats, StatsTotals};
    use clawg_core::memory::store_lock::{StoreHolder, StoreOwner};
    use clawg_core::paths::Profile;

    /// 2026-10-04 23:32:14 UTC.
    const NOW: i64 = 1_791_156_734_441;

    fn row(id: i64, kind: &str, summary: &str) -> MemoryRow {
        MemoryRow {
            id,
            kind: kind.into(),
            memory_type: Some("semantic_fact".into()),
            summary: summary.into(),
            tier: 2,
            importance: 5.0,
            status: "active".into(),
            access_count: 3,
            scope: Some("agent".into()),
            agent_id: Some("agent-a".into()),
            project_id: Some("claw-garden-1a2b".into()),
            created_at: Some(NOW - 12_000),
            last_accessed_at: Some(NOW - 90_000),
            ..Default::default()
        }
    }

    /// The char column at which `needle` starts in `line`.
    fn char_col(line: &str, needle: &str) -> usize {
        let byte = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {line:?}"));
        line[..byte].chars().count()
    }

    // ── table ───────────────────────────────────────────────────────────────

    #[test]
    fn list_deleted_shows_when_each_row_was_deleted() {
        let recent = MemoryRow { deleted_at: Some(NOW - 120_000), ..row(7, "memory", "the deploy runs on fridays") };
        let older = MemoryRow { deleted_at: Some(NOW - 3 * 86_400_000), ..row(3, "memory", "tabs in go") };

        let out = render_deleted_table(Lang::En, &[recent, older], NOW, 20, 0);

        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("KIND") && lines[0].contains("DELETED"), "{out}");
        assert!(!lines[0].contains(" AGE "), "{out}");
        assert!(lines[1].contains(" 2m ") && lines[1].contains("fridays"), "{out}");
        assert!(lines[2].contains(" 3d ") && lines[2].contains("tabs in go"), "{out}");
        assert!(out.ends_with("2 rows, most recently deleted first. Undo a memory row with 'clawg undelete <ID>'."), "{out}");
        assert_eq!(render_deleted_table(Lang::En, &[], NOW, 20, 0), "no deleted memories match");
    }

    #[test]
    fn deleted_episodes_alone_are_not_offered_an_undelete() {
        let episode = MemoryRow { deleted_at: Some(NOW - 60_000), ..row(4, "episodic", "ran the migration") };

        let out = render_deleted_table(Lang::En, &[episode], NOW, 20, 0);

        assert!(out.ends_with("1 row, most recently deleted first."), "{out}");
    }

    #[test]
    fn table_one_line_per_row_plus_header() {
        let rows = [row(2, "memory", "second"), row(1, "memory", "first")];
        let out = render_table(Lang::En, &rows, NOW, 20, 0);
        let lines: Vec<&str> = out.lines().collect();

        assert_eq!(lines.len(), 4, "header, two rows, footer:\n{out}");
        let header: Vec<&str> = lines[0].split_whitespace().collect();
        assert_eq!(
            header,
            ["KIND", "ID", "AGE", "T", "IMP", "TYPE", "AGENT", "PROJECT", "STATUS", "CONTENT"]
        );
        assert!(lines[1].starts_with("memory"), "{}", lines[1]);
        assert!(lines[1].ends_with("second"), "{}", lines[1]);
        assert!(lines[1].contains("12s"), "{}", lines[1]);
        assert!(lines[1].contains("5.0"), "{}", lines[1]);
        assert_eq!(lines[3], "2 rows, newest first.");
    }

    #[test]
    fn columns_align_by_chars_not_bytes() {
        let mut a = row(1, "memory", "one");
        a.agent_id = Some("ñandú-é".into());
        a.memory_type = Some("preferencia".into());
        let mut b = row(22, "memory", "two");
        b.agent_id = Some("🚀🚀🚀".into());
        b.project_id = Some("proyecto-ó".into());
        let mut c = row(333, "episodic", "three");
        c.agent_id = Some("plain".into());
        c.memory_type = None;

        let out = render_table(Lang::En, &[a, b, c], NOW, 20, 0);
        let lines: Vec<&str> = out.lines().collect();

        let project = char_col(lines[0], "PROJECT");
        let status = char_col(lines[0], "STATUS");
        let content = char_col(lines[0], "CONTENT");
        assert_eq!(char_col(lines[1], "claw-garden-1a2b"), project, "{out}");
        assert_eq!(char_col(lines[2], "proyecto-ó"), project, "{out}");
        assert_eq!(char_col(lines[3], "claw-garden-1a2b"), project, "{out}");
        for (line, content_text) in [(lines[1], "one"), (lines[2], "two"), (lines[3], "three")] {
            assert_eq!(char_col(line, "active"), status, "{out}");
            assert_eq!(
                line.chars().count() - content_text.chars().count(),
                content,
                "{out}"
            );
        }
    }

    #[test]
    fn table_flattens_newlines() {
        let out = render_table(Lang::En, 
            &[row(1, "memory", "line one\nline two\r\nline three")],
            NOW,
            20,
            0,
        );
        assert_eq!(
            out.lines().count(),
            3,
            "a multi-line content must stay on one line:\n{out}"
        );
        assert!(out.contains("line one line two"), "{out}");
    }

    #[test]
    fn table_truncates_long_content_by_chars() {
        let long = "é".repeat(100);
        let out = render_table(Lang::En, &[row(1, "memory", &long)], NOW, 20, 0);
        let line = out.lines().nth(1).expect("row line");
        let shown = line.rsplit("  ").next().expect("content column");
        assert_eq!(
            shown.chars().count(),
            CONTENT_WIDTH + 1,
            "60 chars and the ellipsis: {shown}"
        );
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn dash_for_missing() {
        let bare = MemoryRow {
            id: 4,
            kind: "episodic".into(),
            summary: "x".into(),
            tier: 3,
            status: "active".into(),
            ..Default::default()
        };
        let out = render_table(Lang::En, &[bare], NOW, 20, 0);
        let cells: Vec<&str> = out
            .lines()
            .nth(1)
            .expect("row")
            .split_whitespace()
            .collect();
        // KIND ID AGE T IMP TYPE AGENT PROJECT STATUS CONTENT
        assert_eq!(cells[2], "-", "age of a row with no created_at: {cells:?}");
        assert_eq!(
            &cells[5..8],
            ["-", "-", "-"],
            "type, agent and project: {cells:?}"
        );
    }

    #[test]
    fn empty_says_so() {
        assert_eq!(render_table(Lang::En, &[], NOW, 20, 0), "no memories match");
    }

    #[test]
    fn a_full_page_points_at_the_next_one() {
        let rows = [row(5, "memory", "a"), row(4, "memory", "b")];
        let out = render_table(Lang::En, &rows, NOW, 2, 4);
        assert!(
            out.ends_with("2 rows, newest first. More: --offset 6"),
            "{out}"
        );

        let short = render_table(Lang::En, &rows, NOW, 3, 4);
        assert!(
            !short.contains("More"),
            "a short page is the last one: {short}"
        );

        let one = render_table(Lang::En, &rows[..1], NOW, 20, 0);
        assert!(one.ends_with("1 row, newest first."), "{one}");
    }

    // ── detail ──────────────────────────────────────────────────────────────

    fn full_row() -> MemoryRow {
        MemoryRow {
            id: 57,
            summary: "first line\n\n  indented line\nlast line".into(),
            source: Some("explicit".into()),
            source_session_id: Some("sess-9".into()),
            created_by_type: Some("agent".into()),
            created_by_ref: Some("agent-a".into()),
            last_modified_at: Some(NOW - 3_600_000),
            update_count: Some(2),
            dispute_count: Some(1),
            disputed_against: Some("[42,7]".into()),
            superseded_by: Some(60),
            ..row(57, "memory", "")
        }
    }

    #[test]
    fn detail_keeps_full_content_and_newlines() {
        let out = render_detail(&full_row(), NOW);
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("memory #57 [active]"));
        assert_eq!(lines.next(), Some("content:"));
        assert_eq!(lines.next(), Some("  first line"));
        assert_eq!(
            lines.next(),
            Some(""),
            "a blank line in the content stays blank"
        );
        assert_eq!(lines.next(), Some("    indented line"));
        assert_eq!(lines.next(), Some("  last line"));

        let long = "word ".repeat(200);
        let out = render_detail(&row(1, "memory", &long), NOW);
        assert!(out.contains(long.trim_end()), "the detail never truncates");
    }

    #[test]
    fn detail_has_every_label() {
        let out = render_detail(&full_row(), NOW);
        for label in [
            "type:",
            "tier:",
            "importance:",
            "agent:",
            "project:",
            "scope:",
            "source:",
            "created by:",
            "session:",
            "created:",
            "last access:",
            "access count:",
            "modified:",
            "updates:",
            "disputes:",
            "against:",
            "superseded by:",
            "deleted:",
        ] {
            assert!(out.contains(label), "missing {label}:\n{out}");
        }
        assert!(out.contains("semantic_fact"), "{out}");
        assert!(out.contains("agent-a"), "{out}");
        assert!(out.contains("[42,7]"), "{out}");
        assert!(out.contains("2026-10-04 23:32:02 UTC (12s ago)"), "{out}");
        assert!(out.contains("2026-10-04 22:32:14 UTC (1h ago)"), "{out}");

        let values: Vec<usize> = out
            .lines()
            .filter(|l| l.starts_with("tier:") || l.starts_with("superseded by:"))
            .map(|l| char_col(l, l.split_whitespace().last().expect("value")))
            .collect();
        assert_eq!(
            values[0], values[1],
            "the values line up in one column:\n{out}"
        );
    }

    #[test]
    fn detail_shows_a_dash_where_there_is_nothing() {
        let out = render_detail(
            &MemoryRow {
                id: 3,
                kind: "episodic".into(),
                ..Default::default()
            },
            NOW,
        );
        assert!(
            out.lines()
                .any(|l| l.starts_with("source:") && l.ends_with('-')),
            "{out}"
        );
        assert!(
            out.lines()
                .any(|l| l.starts_with("deleted:") && l.ends_with('-')),
            "{out}"
        );
    }

    #[test]
    fn detail_marks_a_deleted_row() {
        let mut gone = row(9, "memory", "bye");
        gone.deleted_at = Some(NOW - 5 * 86_400_000);
        let out = render_detail(&gone, NOW);
        assert_eq!(out.lines().next(), Some("memory #9 [deleted]"));
        assert!(out.contains("2026-09-29 23:32:14 UTC (5d ago)"), "{out}");
    }

    // ── time ────────────────────────────────────────────────────────────────

    #[test]
    fn age_buckets_s_m_h_d() {
        let at = |secs: i64| format_age(NOW, NOW - secs * 1000);
        assert_eq!(at(0), "0s");
        assert_eq!(at(59), "59s");
        assert_eq!(at(60), "1m");
        assert_eq!(at(3_599), "59m");
        assert_eq!(at(3_600), "1h");
        assert_eq!(at(86_399), "23h");
        assert_eq!(at(86_400), "1d");
        assert_eq!(at(86_400 * 40), "40d");
        assert_eq!(
            format_age(NOW, NOW + 5_000),
            "0s",
            "a clock skew is not a negative age"
        );
    }

    #[test]
    fn utc_formats_known_epochs() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(format_utc(1_704_067_199_000), "2023-12-31 23:59:59 UTC");
        assert_eq!(format_utc(1_704_067_200_000), "2024-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_709_208_000_000), "2024-02-29 12:00:00 UTC");
        assert_eq!(format_utc(NOW), "2026-10-04 23:32:14 UTC");
        assert_eq!(format_utc(-1_000), "1969-12-31 23:59:59 UTC");
    }

    // ── stats ───────────────────────────────────────────────────────────────

    fn count(key: Option<&str>, count: u64) -> Count {
        Count {
            key: key.map(str::to_owned),
            count,
        }
    }

    fn sample_stats() -> MemoryStats {
        MemoryStats {
            totals: StatsTotals {
                memory: 40,
                episodic: 2,
                procedural: 1,
                deleted: 3,
            },
            by_type: vec![
                count(Some("semantic_fact"), 30),
                count(Some("preference"), 10),
            ],
            by_tier: vec![count(Some("1"), 4), count(Some("3"), 36)],
            by_status: vec![count(Some("active"), 39), count(Some("disputed"), 1)],
            by_agent: vec![count(Some("agent-a"), 40)],
            by_project: vec![count(Some("claw-garden-1a2b"), 25), count(None, 15)],
            newest_created_at: Some(NOW - 12_000),
        }
    }

    #[test]
    fn the_viewer_speaks_spanish() {
        assert_eq!(render_table(Lang::Es, &[], NOW, 20, 0), "ninguna memoria coincide");
        assert_eq!(render_deleted_table(Lang::Es, &[], NOW, 20, 0), "ninguna memoria borrada coincide");
        let mut gone = row(9, "memory", "bye");
        gone.deleted_at = Some(NOW - 60_000);
        let deleted = render_deleted_table(Lang::Es, &[gone], NOW, 1, 0);
        assert!(deleted.starts_with("KIND"), "the columns are field names: {deleted}");
        assert!(
            deleted.ends_with(
                "1 fila, las borradas más recientemente primero. Más: --offset 1 Una fila memory vuelve con 'clawg undelete <ID>'."
            ),
            "{deleted}"
        );
        let stats = render_stats(Lang::Es, &sample_stats(), NOW);
        assert!(
            stats.starts_with("43 filas vivas (memory 40, episodic 2, procedural 1), 3 borradas. La más nueva, de hace 12s.\n"),
            "{stats}"
        );
        assert!(stats.contains("\npor proyecto: claw-garden-1a2b 25 | (ninguno) 15"), "{stats}");
        assert_eq!(
            explain_unavailable(Lang::Es, &ClientError::NotRunning, Profile::Dev),
            "no hay ningún daemon de clawg corriendo para el perfil dev; arráncalo con 'clawg serve --profile dev'"
        );
        assert_eq!(
            version_warning(Lang::Es, "0.1.0", "0.2.0").as_deref(),
            Some(
                "aviso: el daemon que corre es la versión 0.1.0 pero este clawg es la 0.2.0; ejecuta 'clawg stop' para \
                 que el próximo arranque use esta versión"
            )
        );
    }

    #[test]
    fn stats_renders_sections() {
        let out = render_stats(Lang::En, &sample_stats(), NOW);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "43 live rows (memory 40, episodic 2, procedural 1), 3 deleted. Newest 12s ago."
        );
        let section = |label: &str| -> &str {
            lines
                .iter()
                .find(|l| l.starts_with(label))
                .unwrap_or_else(|| panic!("{label}\n{out}"))
        };
        assert!(section("by type:").ends_with("semantic_fact 30 | preference 10"));
        assert!(section("by tier:").ends_with("1 4 | 3 36"));
        assert!(section("by status:").ends_with("active 39 | disputed 1"));
        assert!(section("by agent:").ends_with("agent-a 40"));
        assert!(section("by project:").ends_with("claw-garden-1a2b 25 | (none) 15"));

        let col = |l: &str| char_col(l, l.split_whitespace().nth(2).expect("first key"));
        assert_eq!(
            col(section("by type:")),
            col(section("by project:")),
            "{out}"
        );
    }

    #[test]
    fn stats_of_an_empty_store_says_so() {
        let out = render_stats(Lang::En, &MemoryStats::default(), NOW);
        assert!(
            out.starts_with("0 live rows (memory 0, episodic 0, procedural 0), 0 deleted."),
            "{out}"
        );
        assert!(!out.contains("Newest"), "{out}");
        assert!(
            out.lines()
                .any(|l| l.starts_with("by type:") && l.ends_with('-')),
            "{out}"
        );
    }

    // ── json ────────────────────────────────────────────────────────────────

    #[test]
    fn json_is_valid() {
        let rows = [row(2, "memory", "naïve\ncontent"), row(1, "episodic", "x")];
        let parsed: serde_json::Value = serde_json::from_str(&rows_json(&rows)).expect("array");
        let items = parsed.as_array().expect("an array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], 2);
        assert_eq!(
            items[0]["summary"], "naïve\ncontent",
            "the full content, not the table's cell"
        );
        assert_eq!(items[0]["agent_id"], "agent-a");
        assert!(
            items[0]["deleted_at"].is_null(),
            "absent values are null, not missing keys"
        );

        let one: serde_json::Value = serde_json::from_str(&row_json(&full_row())).expect("object");
        assert_eq!(one["id"], 57);
        assert_eq!(one["disputed_against"], "[42,7]");

        let stats: serde_json::Value =
            serde_json::from_str(&stats_json(&sample_stats())).expect("object");
        assert_eq!(stats["totals"]["memory"], 40);
        assert!(stats["by_project"][1]["key"].is_null());

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rows_json(&[])).expect("array"),
            serde_json::json!([])
        );
    }

    // ── messages ────────────────────────────────────────────────────────────

    #[test]
    fn explain_unavailable_names_profile_and_holder() {
        assert_eq!(
            explain_unavailable(Lang::En, &ClientError::NotRunning, Profile::Dev),
            "no clawg daemon is running for profile dev; start it with 'clawg serve --profile dev'"
        );

        let held = ClientError::StoreHeld(StoreOwner {
            holder: StoreHolder::Eval,
            pid: 4242,
            started_at_ms: 1,
        });
        let msg = explain_unavailable(Lang::En, &held, Profile::Dev);
        assert_eq!(msg, held.to_string(), "the existing wording is reused");
        assert!(
            msg.contains("evaluation run") && msg.contains("4242"),
            "{msg}"
        );

        let other = ClientError::Daemon {
            status: 404,
            message: "no Memory row with id 1".into(),
            details: Default::default(),
        };
        assert_eq!(explain_unavailable(Lang::En, &other, Profile::Dev), other.to_string());
    }

    #[test]
    fn version_mismatch_is_reported() {
        let warning = version_warning(Lang::En, "0.1.0", "0.2.0").expect("they differ");
        assert!(
            warning.contains("0.1.0") && warning.contains("0.2.0"),
            "{warning}"
        );
        assert!(warning.contains("clawg stop"), "{warning}");
        assert_eq!(version_warning(Lang::En, "0.2.0", "0.2.0"), None);
    }

    // ── request building ────────────────────────────────────────────────────

    fn list_opts() -> ListOpts {
        ListOpts {
            kind: None,
            filters: Filters::default(),
            limit: 20,
            offset: 0,
            json: false,
            deleted: false,
            origin_project: None,
            profile: ProfileArg { profile: None },
        }
    }

    #[test]
    fn list_args_ask_for_deleted_rows_only_with_the_flag() {
        assert!(!list_args(&list_opts(), None).deleted);
        assert!(list_args(&ListOpts { deleted: true, ..list_opts() }, None).deleted);
    }

    #[test]
    fn list_args_carry_every_flag() {
        let opts = ListOpts {
            kind: Some(KindArg::Procedural),
            filters: Filters {
                scope: Scope {
                    agent: Some("a1".into()),
                    project: Some("ignored".into()),
                    here: false,
                },
                memory_type: Some("preference".into()),
                tier: Some(1),
                status: Some(StatusArg::Superseded),
                no_project: true,
            },
            limit: 7,
            offset: 14,
            ..list_opts()
        };
        let args = list_args(&opts, Some("p1".into()));
        assert_eq!(args.kind.as_deref(), Some("procedural"));
        assert_eq!(args.agent.as_deref(), Some("a1"));
        assert_eq!(
            args.project.as_deref(),
            Some("p1"),
            "the resolved project wins"
        );
        assert_eq!(args.memory_type.as_deref(), Some("preference"));
        assert_eq!(args.tier, Some(1));
        assert_eq!(args.status.as_deref(), Some("superseded"));
        assert!(args.no_project);
        assert_eq!(args.q, None);
        assert_eq!((args.limit, args.offset), (Some(7), Some(14)));
    }

    #[test]
    fn search_args_set_the_query_and_never_a_kind() {
        let opts = SearchOpts {
            query: "deploy window".into(),
            filters: Filters::default(),
            limit: 20,
            offset: 0,
            json: false,
            profile: ProfileArg { profile: None },
        };
        let args = search_args(&opts, None);
        assert_eq!(args.q.as_deref(), Some("deploy window"));
        assert_eq!(
            args.kind, None,
            "the daemon restricts a text search to memory itself"
        );
    }

    // ── against an in-process daemon ────────────────────────────────────────

    async fn live_client() -> (DaemonClient, crate::client::tests::TestDaemon) {
        let d = start_daemon().await;
        publish(&d);
        let c = DaemonClient::from_data_dir(d.dir.path()).expect("client");
        (c, d)
    }

    #[tokio::test]
    async fn run_list_against_in_process_daemon_returns_table() {
        let (c, _d) = live_client().await;
        save_and_wait(
            &c,
            "the release train leaves on tuesday",
            "agent-run",
            Some("proj-r"),
            1,
        )
        .await;

        let out = run(Lang::En, &c, MemoriesAction::List(list_opts()), NOW)
            .await
            .expect("run");
        assert!(out.starts_with("KIND"), "{out}");
        assert!(out.contains("the release train leaves on tuesday"), "{out}");
        assert!(out.contains("agent-run") && out.contains("proj-r"), "{out}");
        assert!(out.ends_with("1 row, newest first."), "{out}");

        let json = run(
            Lang::En,
            &c,
            MemoriesAction::List(ListOpts {
                json: true,
                ..list_opts()
            }),
            NOW,
        )
        .await
        .expect("run");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(parsed.as_array().expect("array").len(), 1);
    }

    #[tokio::test]
    async fn run_search_finds_a_saved_memory() {
        let (c, _d) = live_client().await;
        save_and_wait(&c, "alpha note about caching layers", "agent-a", None, 1).await;
        save_and_wait(&c, "beta note about retry budgets", "agent-a", None, 2).await;

        let opts = SearchOpts {
            query: "retry".into(),
            filters: Filters::default(),
            limit: 20,
            offset: 0,
            json: false,
            profile: ProfileArg { profile: None },
        };
        let out = run(Lang::En, &c, MemoriesAction::Search(opts), NOW)
            .await
            .expect("run");
        assert!(out.contains("beta note about retry budgets"), "{out}");
        assert!(!out.contains("alpha"), "{out}");

        let none = SearchOpts {
            query: "zzzunmatched".into(),
            filters: Filters::default(),
            limit: 20,
            offset: 0,
            json: false,
            profile: ProfileArg { profile: None },
        };
        assert_eq!(
            run(Lang::En, &c, MemoriesAction::Search(none), NOW)
                .await
                .expect("run"),
            "no memories match"
        );
    }

    #[tokio::test]
    async fn run_show_prints_the_detail_and_a_missing_row_is_an_error() {
        let (c, _d) = live_client().await;
        save_and_wait(&c, "show me\nin full", "agent-a", None, 1).await;
        let id = c
            .list(crate::client::ListArgs::default())
            .await
            .expect("list")[0]
            .id;
        let show = |id, json| {
            MemoriesAction::Show(ShowOpts {
                id,
                kind: KindArg::Memory,
                json,
                profile: ProfileArg { profile: None },
            })
        };

        let out = run(Lang::En, &c, show(id, false), NOW).await.expect("run");
        assert!(out.starts_with(&format!("memory #{id} [active]")), "{out}");
        assert!(out.contains("  show me\n  in full"), "{out}");

        let json = run(Lang::En, &c, show(id, true), NOW).await.expect("run");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(parsed["id"], id);

        match run(Lang::En, &c, show(999_999, false), NOW)
            .await
            .expect_err("no such row")
        {
            ClientError::Daemon { status, .. } => assert_eq!(status, 404),
            other => panic!("expected the daemon's 404, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_show_missing_is_error() {
        let (c, _d) = live_client().await;
        let action = MemoriesAction::Show(ShowOpts {
            id: 12345,
            kind: KindArg::Episodic,
            json: false,
            profile: ProfileArg { profile: None },
        });
        let err = run(Lang::En, &c, action, NOW).await.expect_err("nothing is stored");
        assert!(
            matches!(err, ClientError::Daemon { status: 404, .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn run_stats_against_in_process_daemon() {
        let (c, _d) = live_client().await;
        save_and_wait(&c, "a note to be counted", "agent-s", None, 1).await;

        let action = |json| {
            MemoriesAction::Stats(StatsOpts {
                scope: Scope::default(),
                json,
                profile: ProfileArg { profile: None },
            })
        };
        let out = run(Lang::En, &c, action(false), NOW).await.expect("run");
        assert!(
            out.starts_with("1 live row (memory 1, episodic 0, procedural 0), 0 deleted."),
            "{out}"
        );
        assert!(out.contains("agent-s 1"), "{out}");

        let json = run(Lang::En, &c, action(true), NOW).await.expect("run");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(parsed["totals"]["memory"], 1);
    }
}
