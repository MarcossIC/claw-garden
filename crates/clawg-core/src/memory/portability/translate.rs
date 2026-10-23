//! A [`ForeignExport`] as the lines of a `clawg-memories` file, which the
//! import then places as it places a clawg export's.
//!
//! Each note is masked whole and split as a save of its text: its rows are
//! the pieces a save would store, in order, the later ones embedded behind
//! the same head. Its project comes from the user's map, its key is made one
//! clawg takes, and the keys of two projects of the source that land in one
//! project here are told apart. Pure but for the chunker, which cuts with the
//! tokenizer it holds.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::memory::chunker::ChatChunker;
use crate::memory::dedup::content_hash;
use crate::memory::redact::redact;
use crate::memory::store_insert::MemoryType;
use crate::memory::tier::compute_tier;
use crate::memory::topic::{validate_topic_key, MAX_TOPIC_KEY_LEN};

use super::foreign::{ForeignExport, ForeignNote, ForeignRelation, NoteKind, When};
use super::format::{ConflictLine, MemoryLine, Parsed, PieceLine, ProjectLine};

/// The agent rows from another tool are stored under: the tool does not say
/// which agent wrote each note.
pub const MIGRATED_AGENT: &str = "migrated";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateOptions {
    /// The agent every row is stored under.
    pub agent: String,
    /// The first entry of a name counts.
    pub project_map: Vec<MappedProject>,
}

/// A project name of the source, the root it has here, and the project the
/// rows of that root go to, as registering the root gives it: two clones of
/// one remote are one project here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedProject {
    pub name: String,
    pub root: String,
    pub project_id: String,
}

/// An export as the lines to import.
#[derive(Debug, Clone)]
pub struct Translated {
    /// Rows and pieces with ids that only mean something here, numbered from
    /// 1 by the date of their note's text, each note's first row first. A
    /// memory line's number is its id.
    pub parsed: Parsed,
    /// Each note that gives rows, in the order of its rows.
    pub notes: Vec<NoteRows>,
    /// What the translation alone tells.
    pub report: ForeignReport,
}

/// The rows one note gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRows {
    pub origin: String,
    /// The project it goes to here.
    pub project_id: Option<String>,
    pub rows: Range<i64>,
    /// Its text held a secret, masked before it was split.
    pub masked: bool,
    /// Its text date was after the import or could not be read, so its rows
    /// carry the time of this import, which no later import gives again.
    pub undated: bool,
}

/// Notes of one type of the source, and the type they were stored as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeCount {
    /// `observation`, `prompt` or `session`.
    pub kind: String,
    /// The type the source gave an observation.
    pub source_type: Option<String>,
    pub memory_type: String,
    pub notes: usize,
}

/// Notes of one project of the source, and where they went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCount {
    /// As the source wrote it; `None` for the notes without one.
    pub project: Option<String>,
    pub personal: bool,
    /// The root the map gave it, and its project here; `None` for notes
    /// stored without a project.
    pub root: Option<String>,
    pub project_id: Option<String>,
    pub notes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeyChange {
    pub from: String,
    pub to: String,
}

/// A note stored superseded by a newer note of its origin and key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupersededNote {
    pub origin: String,
    pub by: String,
}

/// A note of the same text as an older one of its origin, stored in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsorbedNote {
    pub origin: String,
    pub into: String,
}

/// A row of a note stored as the live row of the same text that was here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedNote {
    pub origin: String,
    /// Its position among the note's rows, from 1.
    pub piece: i64,
    pub into: i64,
}

/// A note stored superseded by the live row that holds its key here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakenKey {
    pub origin: String,
    pub topic_key: String,
    pub by: i64,
}

/// The notes of a project of the source left out because a registration gave
/// the id planned for its root to another root while the import ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakenProject {
    pub project: String,
    pub root: String,
    pub belongs_to: String,
    pub notes: usize,
}

/// A note left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftOut {
    pub origin: String,
    pub reason: String,
}

/// What an import from another tool did with its notes, each named by its
/// origin. The translation fills it, and the import adds what it found here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForeignReport {
    /// The tool the export came from, and the version of its format.
    pub source: String,
    pub version: String,
    /// Notes in the export, whether they give rows or not.
    pub notes: usize,
    pub by_source_type: Vec<TypeCount>,
    pub by_source_project: Vec<ProjectCount>,
    /// Project names of notes that the map does not name.
    pub unmapped: Vec<String>,
    /// Names in the map that place no note.
    pub unused_map: Vec<String>,
    /// Rows the notes gave, and the notes that gave more than one.
    pub rows: usize,
    pub split_notes: usize,
    /// Notes whose text held a secret.
    pub masked_notes: usize,
    /// Keys clawg does not take, and the ones they became.
    pub rewritten_keys: Vec<KeyChange>,
    /// Keys two projects of the source share in one project here, and the
    /// ones each became.
    pub qualified_keys: Vec<KeyChange>,
    pub superseded_in_file: Vec<SupersededNote>,
    pub absorbed: Vec<AbsorbedNote>,
    /// Notes with a date after the import, or one that could not be read,
    /// stored with the time of the import instead.
    pub clamped_dates: Vec<String>,
    /// Notes the source deleted, stored deleted.
    pub deleted: usize,
    /// Notes whose session the source names in a form that is not kept.
    pub sessions_left_out: Vec<String>,
    pub without_text: Vec<String>,
    /// What the export holds with no place in a row, by field.
    pub unplaced: BTreeMap<String, u64>,
    /// Notes the source rewrote, whose earlier versions it did not export.
    pub rewritten_at_source: u64,
    /// Sessions without a summary, which give no note.
    pub discarded_sessions: u64,
    pub merged: Vec<MergedNote>,
    pub topic_taken: Vec<TakenKey>,
    pub taken_projects: Vec<TakenProject>,
    pub left_out: Vec<LeftOut>,
    /// Notes already here, by the project they are in here (`""` for none),
    /// wherever the map would put them now.
    pub present: BTreeMap<String, usize>,
    /// Notes stored as a newer version of the one their origin has here, and
    /// notes stored as an older one.
    pub revised: Vec<String>,
    pub older_versions: Vec<String>,
    /// Notes the source deleted whose rows here were left as they are.
    pub deleted_at_source: Vec<String>,
    /// Relations stored as pairs, by the pair's state: `pending`,
    /// `escalated`, or the verdict of a judged one.
    pub relations: BTreeMap<String, usize>,
    /// Relations left out, by why: `orphaned`; `replaced`, by the source or
    /// by a newer relation of the same two notes; `missing_note`, an end that
    /// gives no row of this export; `same_note`, two ends the same text made
    /// one; `unknown:<verb>`.
    pub relations_left_out: BTreeMap<String, usize>,
}

/// Importance of a note, and of one the source pinned, which is never
/// demoted.
const IMPORTANCE: f64 = 5.0;
const PINNED_IMPORTANCE: f64 = 9.0;
/// The tier a save starts from before `compute_tier` promotes it.
const START_TIER: i64 = 3;
/// Hex digits of the original's SHA-256 a rewritten key ends with: 40 bits,
/// so two originals never give one key in a store of any size.
const KEY_HASH_HEX: usize = 10;
/// A session id is kept when it is 1 to this many visible ASCII characters,
/// as the daemon takes one.
const MAX_SESSION_LEN: usize = 128;
/// Qualifying keys can give a note the key of another project's note, which
/// takes another pass. Each pass lengthens the keys it touches, so a file
/// settles in one or two; the bound only keeps the loop finite.
const QUALIFY_PASSES: usize = 8;

/// Where a note was in its source: in a project or in none, and whether the
/// source kept it out of what its projects share (`personal`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Group {
    project: Option<String>,
    personal: bool,
}

/// A note on its way to rows.
struct Draft<'a> {
    note: &'a ForeignNote,
    /// Its text, masked.
    text: String,
    masked: bool,
    memory_type: MemoryType,
    group: Group,
    project_id: Option<String>,
    written: i64,
    last_accessed: i64,
    deleted_at: Option<i64>,
    /// A date after the import, or one that could not be read, was replaced.
    clamped: bool,
    /// The date replaced was its text's.
    undated: bool,
    accesses: i64,
    pinned: bool,
    session: Option<String>,
    key: Option<String>,
    /// Its pieces, each once, in order, and the head the later ones are
    /// embedded behind.
    pieces: Vec<String>,
    head: Option<String>,
}

impl<'a> Draft<'a> {
    /// `None` for a note with nothing to store.
    fn new(note: &'a ForeignNote, options: &TranslateOptions, now_ms: i64) -> Option<Self> {
        let masked = redact(&text_of(note)?);
        let clamp = |when: When| match when {
            When::At(at) if at <= now_ms => (at, false),
            _ => (now_ms, true),
        };
        let (written, late) = clamp(note.written);
        let last_seen = note.last_seen.map(clamp);
        let deleted = note.deleted.map(clamp);
        let mapped = note.project.as_deref().filter(|_| !note.personal).and_then(|name| mapping(options, name));
        Some(Draft {
            note,
            masked: !masked.kinds.is_empty(),
            text: masked.text,
            memory_type: memory_type_of(note),
            group: Group { project: note.project.clone(), personal: note.personal },
            project_id: mapped.map(|m| m.project_id.clone()),
            written,
            // Writing a note is seeing it too, and a row is never read before
            // it was written.
            last_accessed: last_seen.map_or(written, |(at, _)| at.max(written)),
            deleted_at: deleted.map(|(at, _)| at),
            clamped: late || last_seen.is_some_and(|(_, c)| c) || deleted.is_some_and(|(_, c)| c),
            undated: late,
            accesses: i64::try_from(note.seen).unwrap_or(i64::MAX).max(1),
            pinned: note.pinned,
            session: note.session.clone().filter(|id| kept_session(id)),
            key: None,
            pieces: Vec::new(),
            head: None,
        })
    }
}

/// Translates `export`, every date after `now_ms` taken as `now_ms`.
pub fn translate(export: &ForeignExport, options: &TranslateOptions, chunker: &ChatChunker, now_ms: i64) -> Translated {
    let mut report = ForeignReport {
        source: export.source.to_owned(),
        version: export.version.clone(),
        notes: export.notes.len(),
        unplaced: export.unplaced.clone(),
        rewritten_at_source: export.rewritten,
        discarded_sessions: export.discarded_sessions,
        ..Default::default()
    };
    let mut drafts = Vec::with_capacity(export.notes.len());
    for note in &export.notes {
        match Draft::new(note, options, now_ms) {
            Some(draft) => drafts.push(draft),
            None => report.without_text.push(note.origin.clone()),
        }
    }
    count_notes(&drafts, options, &mut report);
    let mut drafts = absorb_repeats(drafts, &mut report);
    set_keys(&mut drafts, &mut report);
    for d in &mut drafts {
        let split = chunker.split_save(&d.text);
        let mut seen = HashSet::new();
        d.pieces = split.pieces.into_iter().map(|c| c.text).filter(|p| seen.insert(content_hash(p))).collect();
        d.head = split.head;
    }
    // A text the tokenizer finds nothing in gives a save no rows either.
    let (mut drafts, empty): (Vec<_>, Vec<_>) = drafts.into_iter().partition(|d| !d.pieces.is_empty());
    report.without_text.extend(empty.into_iter().map(|d| d.note.origin.clone()));
    drafts.sort_by(|a, b| (a.written, &a.note.origin).cmp(&(b.written, &b.note.origin)));
    let mut translated = lines(&drafts, options, report);
    relate(&export.relations, now_ms, &mut translated);
    translated
}

/// The title on the first line and the content under it, or whichever of
/// the two the note has.
fn text_of(note: &ForeignNote) -> Option<String> {
    let has = |text: &str| !text.trim().is_empty();
    match (has(&note.title), has(&note.content)) {
        (true, true) => Some(format!("{}\n{}", note.title, note.content)),
        (true, false) => Some(note.title.clone()),
        (false, true) => Some(note.content.clone()),
        (false, false) => None,
    }
}

fn memory_type_of(note: &ForeignNote) -> MemoryType {
    match note.kind {
        NoteKind::Prompt => MemoryType::ChatMessage,
        NoteKind::SessionSummary => MemoryType::SessionSummary,
        NoteKind::Observation => match note.source_type.as_deref() {
            Some("decision" | "architecture") => MemoryType::ArchitecturalDecision,
            Some("preference") => MemoryType::UserPreference,
            Some("session_summary") => MemoryType::SessionSummary,
            _ => MemoryType::SemanticFact,
        },
    }
}

fn mapping<'o>(options: &'o TranslateOptions, name: &str) -> Option<&'o MappedProject> {
    options.project_map.iter().find(|m| m.name == name)
}

fn kept_session(id: &str) -> bool {
    (1..=MAX_SESSION_LEN).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic())
}

/// What the notes with text are, before any is stored in another.
fn count_notes(drafts: &[Draft<'_>], options: &TranslateOptions, report: &mut ForeignReport) {
    let mut types: BTreeMap<(&str, Option<String>), (&str, usize)> = BTreeMap::new();
    let mut projects: BTreeMap<Group, (Option<String>, usize)> = BTreeMap::new();
    for d in drafts {
        let (kind, source_type) = match d.note.kind {
            NoteKind::Observation => ("observation", d.note.source_type.clone()),
            NoteKind::Prompt => ("prompt", None),
            NoteKind::SessionSummary => ("session", None),
        };
        types.entry((kind, source_type)).or_insert((d.memory_type.as_str(), 0)).1 += 1;
        projects.entry(d.group.clone()).or_insert((d.project_id.clone(), 0)).1 += 1;
        report.masked_notes += usize::from(d.masked);
        if d.clamped {
            report.clamped_dates.push(d.note.origin.clone());
        }
        if d.note.session.is_some() && d.session.is_none() {
            report.sessions_left_out.push(d.note.origin.clone());
        }
    }
    report.by_source_type = types
        .into_iter()
        .map(|((kind, source_type), (memory_type, notes))| TypeCount {
            kind: kind.to_owned(),
            source_type,
            memory_type: memory_type.to_owned(),
            notes,
        })
        .collect();
    report.by_source_project = projects
        .into_iter()
        .map(|(group, (project_id, notes))| ProjectCount {
            root: project_id.as_ref().and(group.project.as_deref()).and_then(|name| mapping(options, name)).map(|m| m.root.clone()),
            project: group.project,
            personal: group.personal,
            project_id,
            notes,
        })
        .collect();
    let named: BTreeSet<&str> =
        drafts.iter().filter(|d| !d.note.personal).filter_map(|d| d.note.project.as_deref()).collect();
    report.unmapped = named.iter().filter(|name| mapping(options, name).is_none()).map(|name| name.to_string()).collect();
    for m in &options.project_map {
        if !named.contains(m.name.as_str()) && !report.unused_map.contains(&m.name) {
            report.unused_map.push(m.name.clone());
        }
    }
}

/// Notes of one group with one text are one save said again: the oldest
/// keeps them all, with every access, the latest one and any pin.
fn absorb_repeats<'a>(mut drafts: Vec<Draft<'a>>, report: &mut ForeignReport) -> Vec<Draft<'a>> {
    let mut order: Vec<usize> = (0..drafts.len()).collect();
    order.sort_by(|&a, &b| (drafts[a].written, &drafts[a].note.origin).cmp(&(drafts[b].written, &drafts[b].note.origin)));
    let mut keepers: HashMap<(Group, String), usize> = HashMap::new();
    let mut absorbed = vec![false; drafts.len()];
    for i in order {
        if drafts[i].deleted_at.is_some() {
            continue;
        }
        let text = (drafts[i].group.clone(), content_hash(&drafts[i].text));
        let Some(&k) = keepers.get(&text) else {
            keepers.insert(text, i);
            continue;
        };
        let (accesses, last, pinned) = (drafts[i].accesses, drafts[i].last_accessed, drafts[i].pinned);
        let keeper = &mut drafts[k];
        keeper.accesses = keeper.accesses.saturating_add(accesses);
        keeper.last_accessed = keeper.last_accessed.max(last);
        keeper.pinned |= pinned;
        absorbed[i] = true;
        report.absorbed.push(AbsorbedNote { origin: drafts[i].note.origin.clone(), into: drafts[k].note.origin.clone() });
    }
    drafts.into_iter().zip(absorbed).filter(|(_, absorbed)| !absorbed).map(|(d, _)| d).collect()
}

/// Gives each note a key clawg takes, then qualifies every key that notes of
/// two groups share in one project here, so neither hides the other.
fn set_keys(drafts: &mut [Draft<'_>], report: &mut ForeignReport) {
    let mut rewritten = BTreeSet::new();
    for d in drafts.iter_mut() {
        let Some(original) = d.note.topic_key.as_deref() else { continue };
        let key = rewrite_key(original);
        if key != original {
            rewritten.insert(KeyChange { from: original.to_owned(), to: key.clone() });
        }
        d.key = Some(key);
    }
    let unqualified: Vec<Option<String>> = drafts.iter().map(|d| d.key.clone()).collect();
    for _ in 0..QUALIFY_PASSES {
        let mut groups: HashMap<(Option<&str>, &str), HashSet<&Group>> = HashMap::new();
        for d in drafts.iter() {
            if let Some(key) = &d.key {
                groups.entry((d.project_id.as_deref(), key)).or_default().insert(&d.group);
            }
        }
        let shared: HashSet<(Option<String>, String)> = groups
            .into_iter()
            .filter(|(_, groups)| groups.len() > 1)
            .map(|((project_id, key), _)| (project_id.map(str::to_owned), key.to_owned()))
            .collect();
        if shared.is_empty() {
            break;
        }
        for d in drafts.iter_mut() {
            let Some(key) = d.key.clone() else { continue };
            if shared.contains(&(d.project_id.clone(), key.clone())) {
                d.key = Some(rewrite_key(&format!("{}/{key}", group_segment(&d.group))));
            }
        }
    }
    let mut qualified = BTreeSet::new();
    for (d, from) in drafts.iter().zip(unqualified) {
        if let (Some(from), Some(to)) = (from, &d.key) {
            if from != *to {
                qualified.insert(KeyChange { from, to: to.clone() });
            }
        }
    }
    report.rewritten_keys = rewritten.into_iter().collect();
    report.qualified_keys = qualified.into_iter().collect();
}

/// A key clawg takes for `original`: the same when it is one already;
/// otherwise in lowercase, every other character a `-` and no empty segment,
/// cut to leave room for a `-` and the start of the original's SHA-256, so
/// the same original always gives the same key and two never give one.
pub fn rewrite_key(original: &str) -> String {
    if validate_topic_key(original).is_ok() {
        return original.to_owned();
    }
    let segments: Vec<String> = original.split('/').map(plain).filter(|s| !s.is_empty()).collect();
    hashed(original, &segments.join("/"))
}

/// `text` in lowercase, with a `-` for every character outside `[a-z0-9._-]`.
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect()
}

/// `shape` cut to leave room, then a `-` and the hash of `original`; `key-`
/// and the hash when nothing of `shape` is left.
fn hashed(original: &str, shape: &str) -> String {
    let hash: String = Sha256::digest(original.as_bytes()).iter().take(KEY_HASH_HEX / 2).map(|b| format!("{b:02x}")).collect();
    let room = MAX_TOPIC_KEY_LEN - 1 - KEY_HASH_HEX;
    // `shape` is ASCII, so a byte cut is a character cut.
    let cut = shape[..shape.len().min(room)].trim_end_matches('/');
    if cut.is_empty() {
        format!("key-{hash}")
    } else {
        format!("{cut}-{hash}")
    }
}

/// A group as the first segment of a qualified key: its project's name, or
/// `no-project`, then `.personal` for the notes the source kept to the user.
/// A name that is not a plain segment, or reads as another group's, carries
/// its hash, so two groups never share a segment.
fn group_segment(group: &Group) -> String {
    let base = match group.project.as_deref() {
        None => "no-project".to_owned(),
        Some(name) => {
            let shape = plain(name);
            if shape == name && name != "no-project" && !name.ends_with(".personal") {
                shape
            } else {
                hashed(name, &shape)
            }
        }
    };
    if group.personal {
        format!("{base}.personal")
    } else {
        base
    }
}

/// Numbers the rows of `drafts`, already in date order, and writes their
/// lines. Of the live notes of one key in one project here, the newest stays
/// live and the others are its earlier revisions.
fn lines(drafts: &[Draft<'_>], options: &TranslateOptions, mut report: ForeignReport) -> Translated {
    let mut firsts = Vec::with_capacity(drafts.len());
    let mut next = 1;
    for d in drafts {
        firsts.push(next);
        next += d.pieces.len() as i64;
    }
    let mut newest: HashMap<(Option<&str>, &str), usize> = HashMap::new();
    for (i, d) in drafts.iter().enumerate() {
        if let (None, Some(key)) = (d.deleted_at, &d.key) {
            newest.insert((d.project_id.as_deref(), key), i);
        }
    }

    let mut parsed = Parsed::default();
    let mut notes = Vec::with_capacity(drafts.len());
    let mut pieces = Vec::new();
    for (i, d) in drafts.iter().enumerate() {
        let first = firsts[i];
        let by = match (d.deleted_at, &d.key) {
            (None, Some(key)) => newest.get(&(d.project_id.as_deref(), key.as_str())).copied().filter(|&n| n != i),
            _ => None,
        };
        if let Some(n) = by {
            report.superseded_in_file.push(SupersededNote { origin: d.note.origin.clone(), by: drafts[n].note.origin.clone() });
        }
        let importance = if d.pinned { PINNED_IMPORTANCE } else { IMPORTANCE };
        let split = d.pieces.len() > 1;
        for (k, piece) in d.pieces.iter().enumerate() {
            let id = first + k as i64;
            parsed.memories.push((
                id as usize,
                MemoryLine {
                    id,
                    scope: "agent".to_owned(),
                    agent_id: Some(options.agent.clone()),
                    project_id: d.project_id.clone(),
                    content: piece.clone(),
                    memory_type: d.memory_type.as_str().to_owned(),
                    source: "import".to_owned(),
                    created_at: d.written,
                    created_by_type: if d.note.kind == NoteKind::Prompt { "user" } else { "agent" }.to_owned(),
                    created_by_ref: d.note.origin.clone(),
                    source_session_id: d.session.clone(),
                    last_modified_at: d.written,
                    last_modified_by: None,
                    update_count: 0,
                    last_accessed_at: d.last_accessed,
                    access_count: d.accesses,
                    importance,
                    tier: compute_tier(START_TIER, importance, d.accesses, d.memory_type.as_str()),
                    status: if by.is_some() { "superseded" } else { "active" }.to_owned(),
                    superseded_by: by.map(|n| firsts[n]),
                    supersede_cause: by.map(|_| "revision".to_owned()),
                    dispute_count: 0,
                    disputed_against: Vec::new(),
                    last_disputed_at: None,
                    deleted_at: d.deleted_at,
                    topic_key: d.key.clone(),
                    revived_from: None,
                    lang: None,
                    origin_project: d.note.project.clone(),
                },
            ));
            if split {
                let context = if k == 0 { String::new() } else { d.head.clone().unwrap_or_default() };
                pieces.push(PieceLine { save: first, piece: k as i64 + 1, memory: id, context });
            }
        }
        report.split_notes += usize::from(split);
        report.deleted += usize::from(d.deleted_at.is_some());
        notes.push(NoteRows {
            origin: d.note.origin.clone(),
            project_id: d.project_id.clone(),
            rows: first..first + d.pieces.len() as i64,
            masked: d.masked,
            undated: d.undated,
        });
    }
    report.rows = parsed.memories.len();
    parsed.pieces = pieces.into_iter().enumerate().map(|(k, p)| (report.rows + k + 1, p)).collect();
    // Each root once, in the order of the map: the import plans the ids of
    // the roots in that order and lines them up with these lines.
    let mut recorded = HashSet::new();
    for m in &options.project_map {
        if recorded.insert(m.root.as_str()) {
            parsed.projects.push((0, ProjectLine { id: m.project_id.clone(), root_path: m.root.clone() }));
        }
    }
    Translated { parsed, notes, report }
}

/// What a relation of the source becomes here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PairFor {
    Pending,
    /// Judged with this verdict.
    Judged(&'static str),
    /// Left to the user.
    Escalated,
    /// Not stored, counted by this reason.
    LeftOut(String),
}

/// The pair a relation `verb` the source judged `judgment` becomes. A
/// `supersedes` over a preference or a decision (`protected`) is a verdict
/// only the user gives, so one the user did not mark is left to them.
fn pair_for(verb: &str, judgment: &str, by_user: bool, protected: bool) -> PairFor {
    if judgment == "orphaned" {
        return PairFor::LeftOut("orphaned".to_owned());
    }
    if verb == "pending" || judgment == "pending" {
        return PairFor::Pending;
    }
    if judgment == "ignored" {
        return PairFor::Judged("compatible");
    }
    match verb {
        "related" | "compatible" | "not_conflict" => PairFor::Judged("compatible"),
        "scoped" => PairFor::Judged("scoped"),
        "conflicts_with" => PairFor::Escalated,
        "supersedes" if protected && !by_user => PairFor::Escalated,
        "supersedes" => PairFor::Judged("supersedes"),
        other => PairFor::LeftOut(format!("unknown:{other}")),
    }
}

/// Writes each relation between two notes that give rows as a pair of their
/// first rows, the newer note's the `new_id`, and gives their rows the state
/// the pair means: the live rows of a `supersedes`'s target replaced by its
/// source, unless the source is deleted, and the newer row of an escalated
/// pair disputed. Of the relations of two notes the newest counts.
fn relate(relations: &[ForeignRelation], now_ms: i64, t: &mut Translated) {
    let at = |when: When| match when {
        When::At(at) if at <= now_ms => at,
        _ => now_ms,
    };
    let mut left_out: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |reason: &str| *left_out.entry(reason.to_owned()).or_default() += 1;
    let mut newest: BTreeMap<(i64, i64), (&ForeignRelation, Range<i64>, Range<i64>)> = BTreeMap::new();
    {
        let into: HashMap<&str, &str> = t.report.absorbed.iter().map(|a| (a.origin.as_str(), a.into.as_str())).collect();
        let rows_of: HashMap<&str, Range<i64>> = t.notes.iter().map(|n| (n.origin.as_str(), n.rows.clone())).collect();
        let end = |origin: &str| rows_of.get(into.get(origin).copied().unwrap_or(origin)).cloned();
        let date = |r: &ForeignRelation| (at(r.updated), at(r.created));
        for r in relations {
            if r.superseded {
                count("replaced");
                continue;
            }
            if r.judgment == "orphaned" {
                count("orphaned");
                continue;
            }
            let (Some(source), Some(target)) = (end(&r.source), end(&r.target)) else {
                count("missing_note");
                continue;
            };
            if source.start == target.start {
                count("same_note");
                continue;
            }
            let pair = (source.start.max(target.start), source.start.min(target.start));
            if newest.get(&pair).is_some_and(|(kept, ..)| date(kept) >= date(r)) {
                count("replaced");
                continue;
            }
            if newest.insert(pair, (r, source, target)).is_some() {
                count("replaced");
            }
        }
    }

    let mut stored: BTreeMap<String, usize> = BTreeMap::new();
    let mut conflicts = Vec::new();
    // A memory line's number is its id, and its index is one less.
    let memories = &mut t.parsed.memories;
    let live = |m: &MemoryLine| m.deleted_at.is_none() && matches!(m.status.as_str(), "active" | "disputed");
    for ((new_id, old_id), (r, source, target)) in newest {
        let protected = MemoryType::is_protected_name(&memories[(target.start - 1) as usize].1.memory_type);
        let (state, verdict) = match pair_for(&r.verb, &r.judgment, r.by_user, protected) {
            PairFor::LeftOut(reason) => {
                count(&reason);
                continue;
            }
            PairFor::Pending => ("pending", None),
            PairFor::Escalated => {
                let newer = &mut memories[(new_id - 1) as usize].1;
                if live(newer) {
                    newer.status = "disputed".to_owned();
                    newer.disputed_against.push(old_id);
                }
                ("escalated", None)
            }
            PairFor::Judged(verdict) => {
                if verdict == "supersedes" && memories[(source.start - 1) as usize].1.deleted_at.is_none() {
                    for id in target {
                        let replaced = &mut memories[(id - 1) as usize].1;
                        if live(replaced) {
                            replaced.status = "superseded".to_owned();
                            replaced.superseded_by = Some(source.start);
                            replaced.supersede_cause = Some("verdict".to_owned());
                        }
                    }
                }
                ("judged", Some(verdict))
            }
        };
        *stored.entry(verdict.unwrap_or(state).to_owned()).or_default() += 1;
        let reason = match (&r.reason, &r.evidence) {
            (Some(reason), Some(evidence)) => Some(format!("{reason}\n{evidence}")),
            (reason, evidence) => reason.clone().or_else(|| evidence.clone()),
        };
        conflicts.push(ConflictLine {
            new_id,
            old_id,
            distance: None,
            state: state.to_owned(),
            verdict: verdict.map(str::to_owned),
            reason,
            created_at: at(r.created),
            judged_at: verdict.map(|_| at(r.updated)),
            topic_key_warned: false,
        });
    }
    let first_line = t.parsed.memories.len() + t.parsed.pieces.len() + 1;
    t.parsed.conflicts = conflicts.into_iter().enumerate().map(|(k, c)| (first_line + k, c)).collect();
    t.report.relations = stored;
    t.report.relations_left_out = left_out;
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::memory::identity::derive_project_id;

    /// 2025-01-01 10:00:00 UTC.
    pub(crate) const T0: i64 = 1_735_725_600_000;
    /// When every translation here runs.
    pub(crate) const NOW: i64 = 1_760_000_000_000;

    /// An observation of type `discovery`, seen once, written at `T0`.
    pub(crate) fn note(origin: &str, title: &str, content: &str) -> ForeignNote {
        ForeignNote {
            origin: origin.to_owned(),
            kind: NoteKind::Observation,
            source_type: Some("discovery".to_owned()),
            title: title.to_owned(),
            content: content.to_owned(),
            project: None,
            personal: false,
            topic_key: None,
            session: None,
            written: When::At(T0),
            seen: 1,
            last_seen: None,
            deleted: None,
            pinned: false,
        }
    }

    pub(crate) fn export(notes: Vec<ForeignNote>) -> ForeignExport {
        ForeignExport {
            source: "engram",
            version: "0.2.0".to_owned(),
            notes,
            relations: Vec::new(),
            unplaced: BTreeMap::new(),
            rewritten: 0,
            discarded_sessions: 0,
        }
    }

    /// Each root to the project a registration gives it in an empty store.
    fn options(map: &[(&str, &str)]) -> TranslateOptions {
        TranslateOptions {
            agent: MIGRATED_AGENT.to_owned(),
            project_map: map
                .iter()
                .map(|(name, root)| MappedProject { name: name.to_string(), root: root.to_string(), project_id: id_of(root) })
                .collect(),
        }
    }

    fn run(notes: Vec<ForeignNote>, map: &[(&str, &str)]) -> Translated {
        translate(&export(notes), &options(map), &ChatChunker::with_fallback(), NOW)
    }

    /// The rows of the note `origin`, in order.
    fn rows<'a>(t: &'a Translated, origin: &str) -> Vec<&'a MemoryLine> {
        t.parsed.memories.iter().map(|(_, m)| m).filter(|m| m.created_by_ref == origin).collect()
    }

    fn row<'a>(t: &'a Translated, origin: &str) -> &'a MemoryLine {
        let rows = rows(t, origin);
        assert_eq!(rows.len(), 1, "{origin} gives one row");
        rows[0]
    }

    fn in_project(origin: &str, project: &str, content: &str) -> ForeignNote {
        ForeignNote { project: Some(project.to_owned()), ..note(origin, origin, content) }
    }

    fn keyed(origin: &str, project: Option<&str>, personal: bool, key: &str) -> ForeignNote {
        ForeignNote {
            project: project.map(str::to_owned),
            personal,
            topic_key: Some(key.to_owned()),
            ..note(origin, origin, "a note under a key")
        }
    }

    fn id_of(root: &str) -> String {
        derive_project_id(Path::new(root))
    }

    /// The distinct pieces a save of `text` stores, in order.
    fn save_pieces(text: &str) -> (Vec<String>, Option<String>) {
        let split = ChatChunker::with_fallback().split_save(text);
        let mut seen = HashSet::new();
        let pieces = split.pieces.into_iter().map(|c| c.text).filter(|p| seen.insert(content_hash(p))).collect();
        (pieces, split.head)
    }

    #[test]
    fn each_source_type_lands_on_its_memory_type() {
        let mut notes: Vec<ForeignNote> = ["decision", "architecture", "preference", "session_summary", "bugfix", "mood"]
            .iter()
            .map(|t| ForeignNote { source_type: Some(t.to_string()), ..note(&format!("engram:{t}"), "Title", t) })
            .collect();
        notes.push(ForeignNote { source_type: None, ..note("engram:untyped", "Untyped", "no type") });
        notes.push(ForeignNote { kind: NoteKind::Prompt, source_type: None, ..note("engram:prompt:1", "", "how?") });
        notes.push(ForeignNote {
            kind: NoteKind::SessionSummary,
            source_type: None,
            ..note("engram:session:s1", "", "Wired the importer")
        });
        let t = run(notes, &[]);
        let ty = |origin: &str| row(&t, origin).memory_type.clone();
        assert_eq!(ty("engram:decision"), "architectural_decision");
        assert_eq!(ty("engram:architecture"), "architectural_decision");
        assert_eq!(ty("engram:preference"), "user_preference");
        assert_eq!(ty("engram:session_summary"), "session_summary");
        assert_eq!(ty("engram:bugfix"), "semantic_fact");
        assert_eq!(ty("engram:mood"), "semantic_fact");
        assert_eq!(ty("engram:untyped"), "semantic_fact");
        assert_eq!(ty("engram:prompt:1"), "chat_message");
        assert_eq!(ty("engram:session:s1"), "session_summary");

        let count = |kind: &str, source_type: Option<&str>, memory_type: &str| TypeCount {
            kind: kind.to_owned(),
            source_type: source_type.map(str::to_owned),
            memory_type: memory_type.to_owned(),
            notes: 1,
        };
        assert_eq!(
            t.report.by_source_type,
            vec![
                count("observation", None, "semantic_fact"),
                count("observation", Some("architecture"), "architectural_decision"),
                count("observation", Some("bugfix"), "semantic_fact"),
                count("observation", Some("decision"), "architectural_decision"),
                count("observation", Some("mood"), "semantic_fact"),
                count("observation", Some("preference"), "user_preference"),
                count("observation", Some("session_summary"), "session_summary"),
                count("prompt", None, "chat_message"),
                count("session", None, "session_summary"),
            ]
        );
    }

    #[test]
    fn the_title_opens_the_text() {
        let t = run(
            vec![
                note("engram:both", "Use SQLite", "WAL on\nalways"),
                note("engram:title", "Only a title", "  "),
                note("engram:content", " ", "Only content"),
            ],
            &[],
        );
        assert_eq!(row(&t, "engram:both").content, "Use SQLite\nWAL on\nalways");
        assert_eq!(row(&t, "engram:title").content, "Only a title");
        assert_eq!(row(&t, "engram:content").content, "Only content");
    }

    #[test]
    fn a_note_without_text_writes_nothing_and_is_reported() {
        let t = run(
            vec![
                note("engram:empty", "", ""),
                note("engram:blank", "  ", "\n"),
                ForeignNote { kind: NoteKind::Prompt, ..note("engram:prompt:8", "", "") },
                note("engram:kept", "Kept", ""),
            ],
            &[],
        );
        assert_eq!(t.parsed.memories.len(), 1);
        assert_eq!(t.report.without_text, ["engram:empty", "engram:blank", "engram:prompt:8"]);
        assert_eq!((t.report.notes, t.report.rows), (4, 1));
        assert_eq!(t.notes.len(), 1);
    }

    #[test]
    fn a_long_note_splits_into_the_pieces_of_a_save() {
        let words: Vec<String> = (0..300).map(|i| format!("w{i}")).collect();
        let content = format!("{} {}", words.join(" "), "word ".repeat(1000));
        let t = run(vec![note("engram:long", "Notes on clawg-ledger", &content), note("engram:short", "Short", "one piece")], &[]);

        let text = format!("Notes on clawg-ledger\n{content}");
        let (expected, head) = save_pieces(&text);
        let cut = ChatChunker::with_fallback().split_save(&text).pieces.len();
        assert!(expected.len() < cut, "the text repeats a piece, which a save stores once");
        let long = rows(&t, "engram:long");
        assert_eq!(long.iter().map(|m| m.content.clone()).collect::<Vec<_>>(), expected);
        let first = long[0].id;
        let ids: Vec<i64> = long.iter().map(|m| m.id).collect();
        assert_eq!(ids, (first..first + expected.len() as i64).collect::<Vec<_>>());

        let head = head.expect("a long text has a head");
        let pieces: Vec<(i64, i64, i64, String)> =
            t.parsed.pieces.iter().map(|(_, p)| (p.save, p.piece, p.memory, p.context.clone())).collect();
        let linked: Vec<(i64, i64, i64, String)> = long
            .iter()
            .enumerate()
            .map(|(i, m)| (first, i as i64 + 1, m.id, if i == 0 { String::new() } else { head.clone() }))
            .collect();
        assert_eq!(pieces, linked, "the short note has no piece lines");
        assert_eq!((t.report.rows, t.report.split_notes), (expected.len() + 1, 1));
        let plan = t.notes.iter().find(|n| n.origin == "engram:long").unwrap();
        assert_eq!(plan.rows, first..first + expected.len() as i64);
    }

    #[test]
    fn a_secret_across_a_piece_cut_is_masked_whole() {
        let key = format!("ghp_{}", "aB3".repeat(12));
        // The title, a line break and this put the key at byte 374: the first
        // piece, of 384 bytes, ends ten bytes into it.
        let lead = format!("{} ", "y ".repeat(180));
        let content = format!("{lead}{key}{}", " tail".repeat(400));
        let text = format!("Deploy notes\n{content}");
        let raw = ChatChunker::with_fallback().split_save(&text);
        let first = &raw.pieces[0].text;
        assert!(first.ends_with("ghp_aB3aB3"), "{first}");
        assert!(redact(first).kinds.is_empty(), "the fragment alone does not look like a secret");

        let t = run(vec![note("engram:deploy", "Deploy notes", &content)], &[]);
        let stored = rows(&t, "engram:deploy");
        assert!(stored.len() > 1);
        for m in &stored {
            assert!(!m.content.contains("ghp_") && !m.content.contains("aB3aB3"), "{}", m.content);
        }
        assert!(stored.iter().any(|m| m.content.contains("[REDACTED:")));
        assert!(t.parsed.pieces.iter().all(|(_, p)| !p.context.contains("ghp_")));
        assert_eq!(t.report.masked_notes, 1);
        assert!(t.notes[0].masked);
    }

    #[test]
    fn the_project_map_places_notes_and_the_rest_have_none() {
        let t = run(
            vec![
                in_project("engram:a", "a", "one"),
                in_project("engram:a2", "a", "two"),
                in_project("engram:b", "b", "three"),
                note("engram:none", "loose", "four"),
                ForeignNote { personal: true, ..in_project("engram:mine", "a", "five") },
            ],
            &[("a", "/work/a"), ("c", "/work/c")],
        );
        let (a, c) = (id_of("/work/a"), id_of("/work/c"));
        let placed = |origin: &str| {
            let m = row(&t, origin);
            (m.project_id.clone(), m.origin_project.clone())
        };
        assert_eq!(placed("engram:a"), (Some(a.clone()), Some("a".into())));
        assert_eq!(placed("engram:a2"), (Some(a.clone()), Some("a".into())));
        assert_eq!(placed("engram:b"), (None, Some("b".into())));
        assert_eq!(placed("engram:none"), (None, None));
        assert_eq!(placed("engram:mine"), (None, Some("a".into())));

        let count = |project: Option<&str>, personal, root: Option<&str>, notes| ProjectCount {
            project: project.map(str::to_owned),
            personal,
            root: root.map(str::to_owned),
            project_id: root.map(id_of),
            notes,
        };
        assert_eq!(
            t.report.by_source_project,
            vec![
                count(None, false, None, 1),
                count(Some("a"), false, Some("/work/a"), 2),
                count(Some("a"), true, None, 1),
                count(Some("b"), false, None, 1),
            ]
        );
        assert_eq!((t.report.unmapped.as_slice(), t.report.unused_map.as_slice()), (&["b".to_owned()][..], &["c".to_owned()][..]));
        let projects: Vec<(String, String)> = t.parsed.projects.iter().map(|(_, p)| (p.id.clone(), p.root_path.clone())).collect();
        assert_eq!(projects, vec![(a, "/work/a".into()), (c, "/work/c".into())], "every root of the map is recorded");
    }

    #[test]
    fn every_row_carries_its_origin_and_author() {
        let notes = vec![
            ForeignNote { session: Some("s-1".into()), ..note("engram:obs", "Observed", "a fact") },
            ForeignNote { kind: NoteKind::Prompt, session: Some("bad session".into()), ..note("engram:prompt:1", "", "a question") },
            ForeignNote { kind: NoteKind::SessionSummary, session: Some("x".repeat(129)), ..note("engram:session:s2", "", "a summary") },
        ];
        let t = run(notes.clone(), &[]);
        for (_, m) in &t.parsed.memories {
            assert_eq!(
                (m.scope.as_str(), m.agent_id.as_deref(), m.source.as_str(), m.status.as_str(), m.update_count),
                ("agent", Some(MIGRATED_AGENT), "import", "active", 0),
                "{}",
                m.created_by_ref
            );
            assert_eq!((m.lang.clone(), m.last_modified_by.clone(), m.deleted_at), (None, None, None));
        }
        let by = |origin: &str| {
            let m = row(&t, origin);
            (m.created_by_type.clone(), m.source_session_id.clone())
        };
        assert_eq!(by("engram:obs"), ("agent".into(), Some("s-1".into())));
        assert_eq!(by("engram:prompt:1"), ("user".into(), None));
        assert_eq!(by("engram:session:s2"), ("agent".into(), None));
        assert_eq!(t.report.sessions_left_out, ["engram:prompt:1", "engram:session:s2"]);

        let dev = translate(
            &export(notes),
            &TranslateOptions { agent: "dev".into(), project_map: Vec::new() },
            &ChatChunker::with_fallback(),
            NOW,
        );
        assert!(dev.parsed.memories.iter().all(|(_, m)| m.agent_id.as_deref() == Some("dev")));
    }

    #[test]
    fn a_valid_key_is_kept_and_an_invalid_one_rewritten_stably() {
        assert_eq!(rewrite_key("sdd/x/proposal"), "sdd/x/proposal");
        let rewritten = rewrite_key("SDD/My Key");
        assert!(rewritten.starts_with("sdd/my-key-") && rewritten.len() == "sdd/my-key-".len() + 10, "{rewritten}");
        assert_eq!(rewrite_key("SDD/My Key"), rewritten, "the same original gives the same key");
        assert_ne!(rewrite_key("a b"), rewrite_key("a!b"), "two originals never give one key");
        let long = "k".repeat(81);
        let cut = rewrite_key(&long);
        assert!(cut.len() == 80 && cut.starts_with(&format!("{}-", "k".repeat(69))), "{cut}");
        assert!(rewrite_key("a//b").starts_with("a/b-"));
        assert!(rewrite_key("///").starts_with("key-"));
        let slash = format!("{}/{}!", "a".repeat(68), "b".repeat(20));
        assert_eq!(rewrite_key(&slash).len(), 68 + 11, "a cut that ends a segment drops the slash");
        for original in ["SDD/My Key", "a b", &long, "a//b", "///", "Ñandú", &slash, "trailing/"] {
            let key = rewrite_key(original);
            assert!(validate_topic_key(&key).is_ok(), "{original:?} gave {key:?}");
        }

        let t = run(
            vec![
                ForeignNote { topic_key: Some("SDD/My Key".into()), ..note("engram:bad", "Bad", "key") },
                ForeignNote { topic_key: Some("sdd/fine".into()), ..note("engram:good", "Good", "key") },
            ],
            &[],
        );
        assert_eq!(row(&t, "engram:bad").topic_key.as_deref(), Some(rewritten.as_str()));
        assert_eq!(row(&t, "engram:good").topic_key.as_deref(), Some("sdd/fine"));
        assert_eq!(t.report.rewritten_keys, vec![KeyChange { from: "SDD/My Key".into(), to: rewritten }]);
    }

    #[test]
    fn keys_of_two_origins_in_one_destination_are_qualified() {
        let key = "sdd/x/proposal";
        let t = run(
            vec![
                keyed("engram:a", Some("a"), false, key),
                keyed("engram:b", Some("b"), false, key),
                keyed("engram:alone", Some("e"), false, key),
                keyed("engram:c", Some("c"), false, "k"),
                keyed("engram:mine", Some("c"), true, "k"),
                keyed("engram:loose", None, false, "k"),
                keyed("engram:app", Some("My App"), false, "k"),
                keyed("engram:literal", Some("no-project"), false, "k"),
                keyed("engram:dotted", Some("c.personal"), false, "k"),
            ],
            &[("a", "/work/r"), ("b", "/work/r"), ("e", "/work/e")],
        );
        let key_of = |origin: &str| row(&t, origin).topic_key.clone().unwrap();
        assert_eq!(key_of("engram:a"), "a/sdd/x/proposal");
        assert_eq!(key_of("engram:b"), "b/sdd/x/proposal");
        assert_eq!(key_of("engram:alone"), key, "alone in its project here");
        assert_eq!(key_of("engram:c"), "c/k");
        assert_eq!(key_of("engram:mine"), "c.personal/k");
        assert_eq!(key_of("engram:loose"), "no-project/k");
        assert!(key_of("engram:app").starts_with("my-app-"), "{}", key_of("engram:app"));
        let unprojected = ["engram:c", "engram:mine", "engram:loose", "engram:app", "engram:literal", "engram:dotted"];
        let keys: HashSet<String> = unprojected.iter().map(|o| key_of(o)).collect();
        assert_eq!(keys.len(), unprojected.len(), "no two projects of the source share a key: {keys:?}");
        for (_, m) in &t.parsed.memories {
            assert_eq!(m.status, "active");
            assert!(validate_topic_key(m.topic_key.as_deref().unwrap()).is_ok());
        }
        assert!(t.report.qualified_keys.contains(&KeyChange { from: key.into(), to: "a/sdd/x/proposal".into() }));
        assert!(t.report.qualified_keys.contains(&KeyChange { from: "k".into(), to: "no-project/k".into() }));
        assert!(!t.report.qualified_keys.iter().any(|k| k.to == key));
    }

    #[test]
    fn a_qualified_key_that_takes_another_projects_key_is_qualified_again() {
        let t = run(
            vec![
                keyed("engram:x", Some("a"), false, "b/k"),
                keyed("engram:y", Some("b"), false, "k"),
                keyed("engram:z", Some("c"), false, "k"),
            ],
            &[("a", "/work/r"), ("b", "/work/r"), ("c", "/work/r")],
        );
        let keys: HashSet<String> = t.parsed.memories.iter().map(|(_, m)| m.topic_key.clone().unwrap()).collect();
        assert_eq!(keys.len(), 3, "{keys:?}");
    }

    #[test]
    fn keys_of_two_roots_of_one_project_here_are_qualified() {
        let clone = |name: &str, root: &str| MappedProject { name: name.into(), root: root.into(), project_id: "app-1a2b".into() };
        let options = TranslateOptions { agent: MIGRATED_AGENT.into(), project_map: vec![clone("a", "/work/app"), clone("b", "/home/app")] };
        let notes = vec![keyed("engram:a", Some("a"), false, "sdd/x"), keyed("engram:b", Some("b"), false, "sdd/x")];
        let t = translate(&export(notes), &options, &ChatChunker::with_fallback(), NOW);

        let placed = |origin: &str| {
            let m = row(&t, origin);
            (m.project_id.clone().unwrap(), m.topic_key.clone().unwrap(), m.status.clone())
        };
        assert_eq!(placed("engram:a"), ("app-1a2b".into(), "a/sdd/x".into(), "active".into()));
        assert_eq!(placed("engram:b"), ("app-1a2b".into(), "b/sdd/x".into(), "active".into()));
        let projects: Vec<(String, String)> = t.parsed.projects.iter().map(|(_, p)| (p.id.clone(), p.root_path.clone())).collect();
        let recorded = vec![("app-1a2b".into(), "/work/app".into()), ("app-1a2b".into(), "/home/app".into())];
        assert_eq!(projects, recorded, "each root is recorded");
    }

    #[test]
    fn two_notes_of_one_origin_and_key_leave_the_newest_live() {
        let at = |origin: &str, content: &str, written: i64| ForeignNote {
            written: When::At(written),
            topic_key: Some("storage/engine".into()),
            ..in_project(origin, "a", content)
        };
        let t = run(
            vec![
                at("engram:old", "Postgres", T0),
                at("engram:new", "SQLite", T0 + 1000),
                ForeignNote { deleted: Some(When::At(T0 + 3000)), ..at("engram:gone", "DuckDB", T0 + 2000) },
                at("engram:long", &"older and long ".repeat(300), T0 - 1000),
            ],
            &[("a", "/work/a")],
        );
        let new = row(&t, "engram:new");
        assert_eq!((new.status.as_str(), new.superseded_by), ("active", None));
        let replaced: Vec<&MemoryLine> = rows(&t, "engram:old").into_iter().chain(rows(&t, "engram:long")).collect();
        assert!(replaced.len() > 2);
        for m in replaced {
            assert_eq!(
                (m.status.as_str(), m.superseded_by, m.supersede_cause.as_deref()),
                ("superseded", Some(new.id), Some("revision")),
                "{}",
                m.created_by_ref
            );
        }
        let gone = row(&t, "engram:gone");
        assert_eq!((gone.status.as_str(), gone.superseded_by, gone.deleted_at), ("active", None, Some(T0 + 3000)));
        let superseded = |origin: &str| SupersededNote { origin: origin.into(), by: "engram:new".into() };
        assert_eq!(t.report.superseded_in_file, vec![superseded("engram:long"), superseded("engram:old")]);
        assert_eq!(t.report.deleted, 1);
    }

    #[test]
    fn dates_accesses_importance_and_tier_follow_the_note() {
        let t = run(
            vec![
                ForeignNote {
                    source_type: Some("decision".into()),
                    written: When::At(T0 + 5),
                    seen: 3,
                    last_seen: Some(When::At(T0 + 10)),
                    ..note("engram:decision", "Decided", "three times")
                },
                ForeignNote { pinned: true, ..note("engram:pinned", "Pinned", "never demoted") },
                ForeignNote { seen: 0, ..note("engram:unseen", "Unseen", "counted once") },
                ForeignNote {
                    written: When::At(T0 + 100),
                    last_seen: Some(When::At(T0)),
                    ..note("engram:rewritten", "Rewritten", "after it was last seen")
                },
                ForeignNote { source_type: Some("preference".into()), ..note("engram:taste", "Taste", "tabs") },
            ],
            &[],
        );
        let shape = |origin: &str| {
            let m = row(&t, origin);
            (m.created_at, m.last_modified_at, m.last_accessed_at, m.access_count, m.importance, m.tier)
        };
        assert_eq!(shape("engram:decision"), (T0 + 5, T0 + 5, T0 + 10, 3, 5.0, 1));
        assert_eq!(shape("engram:pinned"), (T0, T0, T0, 1, 9.0, 1));
        assert_eq!(shape("engram:unseen"), (T0, T0, T0, 1, 5.0, 3));
        assert_eq!(shape("engram:rewritten"), (T0 + 100, T0 + 100, T0 + 100, 1, 5.0, 3));
        assert_eq!(shape("engram:taste"), (T0, T0, T0, 1, 5.0, 1));
        assert!(t.report.clamped_dates.is_empty());
    }

    #[test]
    fn a_date_after_the_import_is_clamped_and_reported() {
        let t = run(
            vec![
                ForeignNote { written: When::At(NOW + 5000), ..note("engram:future", "Future", "a") },
                ForeignNote { written: When::Unreadable, ..note("engram:unreadable", "Unreadable", "b") },
                ForeignNote { last_seen: Some(When::At(NOW + 1)), ..note("engram:seen", "Seen", "c") },
                ForeignNote { deleted: Some(When::Unreadable), ..note("engram:deleted", "Deleted", "d") },
                note("engram:fine", "Fine", "e"),
            ],
            &[],
        );
        assert_eq!(row(&t, "engram:future").created_at, NOW);
        assert_eq!(row(&t, "engram:unreadable").created_at, NOW);
        assert_eq!(row(&t, "engram:seen").last_accessed_at, NOW);
        assert_eq!(row(&t, "engram:deleted").deleted_at, Some(NOW));
        assert_eq!(row(&t, "engram:fine").created_at, T0);
        assert_eq!(t.report.clamped_dates, ["engram:future", "engram:unreadable", "engram:seen", "engram:deleted"]);
        let undated: Vec<&str> = t.notes.iter().filter(|n| n.undated).map(|n| n.origin.as_str()).collect();
        assert_eq!(undated, ["engram:future", "engram:unreadable"], "only a replaced text date leaves a note without its own");
    }

    #[test]
    fn notes_of_one_origin_with_the_same_text_store_once_with_every_access() {
        let same = |origin: &str, project: &str, written: i64| ForeignNote {
            project: Some(project.to_owned()),
            written: When::At(written),
            ..note(origin, "Same", "the same text")
        };
        let t = run(
            vec![
                ForeignNote { seen: 2, last_seen: Some(When::At(T0 + 5000)), ..same("engram:n1", "a", T0 + 2000) },
                same("engram:n2", "a", T0 + 1000),
                ForeignNote { seen: 4, last_seen: Some(When::At(T0 + 9000)), pinned: true, ..same("engram:n3", "a", T0 + 3000) },
                same("engram:other", "b", T0),
                ForeignNote { deleted: Some(When::At(T0 + 1)), ..same("engram:gone", "a", T0) },
            ],
            &[],
        );
        let kept = row(&t, "engram:n2");
        assert_eq!((kept.access_count, kept.last_accessed_at, kept.created_at, kept.importance), (7, T0 + 9000, T0 + 1000, 9.0));
        assert!(rows(&t, "engram:n1").is_empty() && rows(&t, "engram:n3").is_empty());
        assert_eq!(row(&t, "engram:other").access_count, 1, "another project of the source keeps its own");
        assert_eq!(row(&t, "engram:gone").deleted_at, Some(T0 + 1), "a deleted note is not repeated");
        let into = |origin: &str| AbsorbedNote { origin: origin.into(), into: "engram:n2".into() };
        assert_eq!(t.report.absorbed, vec![into("engram:n1"), into("engram:n3")]);
    }

    #[test]
    fn prompts_and_session_summaries_land_as_their_types() {
        let t = run(
            vec![
                ForeignNote {
                    kind: NoteKind::Prompt,
                    source_type: None,
                    session: Some("s1".into()),
                    title: String::new(),
                    ..in_project("engram:prompt:7", "a", "how does it work?")
                },
                ForeignNote {
                    kind: NoteKind::SessionSummary,
                    source_type: None,
                    session: Some("s1".into()),
                    written: When::At(T0 + 7),
                    title: String::new(),
                    ..in_project("engram:session:s1", "a", "Wired the importer")
                },
            ],
            &[],
        );
        let prompt = row(&t, "engram:prompt:7");
        assert_eq!(
            (prompt.memory_type.as_str(), prompt.created_by_type.as_str(), prompt.content.as_str()),
            ("chat_message", "user", "how does it work?")
        );
        let summary = row(&t, "engram:session:s1");
        assert_eq!(
            (summary.memory_type.as_str(), summary.created_by_type.as_str(), summary.content.as_str()),
            ("session_summary", "agent", "Wired the importer")
        );
        assert_eq!((summary.created_at, summary.origin_project.as_deref()), (T0 + 7, Some("a")));
    }

    #[test]
    fn relations_map_to_pair_states() {
        use PairFor::*;
        assert_eq!(pair_for("pending", "", false, false), Pending);
        assert_eq!(pair_for("related", "pending", false, false), Pending);
        assert_eq!(pair_for("conflicts_with", "pending", false, false), Pending, "a judgment still pending wins");
        assert_eq!(pair_for("conflicts_with", "ignored", false, false), Judged("compatible"));
        for verb in ["related", "compatible", "not_conflict"] {
            assert_eq!(pair_for(verb, "judged", false, false), Judged("compatible"), "{verb}");
        }
        assert_eq!(pair_for("scoped", "judged", false, false), Judged("scoped"));
        assert_eq!(pair_for("conflicts_with", "judged", false, false), Escalated);
        assert_eq!(pair_for("supersedes", "judged", false, false), Judged("supersedes"));
        assert_eq!(pair_for("supersedes", "judged", false, true), Escalated, "a protected note is replaced on the user's word");
        assert_eq!(pair_for("supersedes", "judged", true, true), Judged("supersedes"));
        assert_eq!(pair_for("related", "orphaned", false, false), LeftOut("orphaned".into()));
        assert_eq!(pair_for("mentions", "judged", false, false), LeftOut("unknown:mentions".into()));
    }

    #[test]
    fn rows_are_numbered_by_the_date_of_their_note() {
        let at = |origin: &str, content: &str, written: i64| ForeignNote { written: When::At(written), ..note(origin, origin, content) };
        let t = run(
            vec![
                at("engram:late", &"a long late note ".repeat(200), T0 + 2000),
                at("engram:early", "early", T0),
                at("engram:b", "tie b", T0 + 1000),
                at("engram:a", "tie a", T0 + 1000),
            ],
            &[],
        );
        let order: Vec<&str> = t.notes.iter().map(|n| n.origin.as_str()).collect();
        assert_eq!(order, ["engram:early", "engram:a", "engram:b", "engram:late"]);
        assert_eq!(t.notes[0].rows, 1..2);
        assert_eq!(t.notes[3].rows.start, 4);
        let ids: Vec<(usize, i64)> = t.parsed.memories.iter().map(|(line, m)| (*line, m.id)).collect();
        let expected: Vec<(usize, i64)> = (1..=ids.len() as i64).map(|id| (id as usize, id)).collect();
        assert_eq!(ids, expected, "a memory line's number is its id");
    }
}
