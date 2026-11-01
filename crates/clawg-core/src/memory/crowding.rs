//! What the window of the vector search leaves out, measured against the
//! exact nearest rows the caller may read.
//!
//! vec0 cuts its `k` nearest neighbours out of the whole vector table, and
//! the status, agent and project of a row are tested only after that cut
//! (`SEARCH_SQL_MEMORY` in `search.rs`), so rows a caller cannot read can
//! push the ones it can out of the window. The corpus here holds several
//! agents and projects and chains of revisions, and its vectors are chosen
//! rather than embedded: each is the normalized sum of a direction every row
//! shares (the anisotropy of the model), the direction of its topic, the one
//! of its claim and its own. Two rows of different topics land near distance
//! 0.71, below the cosine floor; two of one topic near 0.49; two of one claim
//! near 0.32, inside the dispute band of the contradiction detector.

use std::collections::HashMap;

use rusqlite::params;

use crate::memory::clock::TimestampMs;
use crate::memory::contradiction::{detect_contradiction, CONTRADICTION_THRESHOLD, NEAR_DUPLICATE_FLOOR};
use crate::memory::score::ScoringConfig;
use crate::memory::search::{memory_project_predicate, reach_predicate, reader_bind, search_all_memories, SearchScope};
use crate::memory::sharing::{Reach, Reader};
use crate::memory::store_insert::MemoryType;
use crate::memory::store_lock::StoreHolder;
use crate::memory::vector::{to_unit, EMBEDDING_DIM};
use crate::memory::{MemoryStore, TableKind};
use crate::metrics::SeriesKey;
use crate::test_dir::{test_dir, TestDir};

/// The squared weights of the directions a vector is the sum of. A row with
/// no claim of its own to share puts the weight of the claim into its noise.
const GLOBAL: f32 = 0.75;
const TOPIC: f32 = 0.13;
const CLAIM: f32 = 0.07;
const NOISE: f32 = 0.05;

/// About how many rows a project keeps on one topic.
const BLOCK: usize = 40;
/// How many superseded versions a chain keeps before its live one.
const CHAIN_LENGTHS: [usize; 4] = [5, 10, 20, 40];
const CHAINS_PER_LENGTH: usize = 15;
/// Claims every agent saves in every project, as a person repeats a fact of
/// all their projects.
const REPEATED_CLAIMS: usize = 15;
const UNOWNED_PERCENT: usize = 20;
/// One plain row in this many is a preference, and another one is deleted.
const ONE_IN: usize = 20;
/// How many projects the reader searches from, and how many topics of each
/// kind it asks about in each.
const QUERY_PROJECTS: usize = 5;
const TOPICS_PER_KIND: usize = 20;

/// What a search asks of the `memory` KNN (`SearchScope::all_default`,
/// which [`search`] checks), and the `limit` a search and `recall_context`
/// get by default (`DEFAULT_SEARCH_LIMIT` of the daemon, `K_FINAL` of
/// `prompt_builder.rs`).
const POOL: usize = 30;
const LIMIT: usize = 10;
/// `CANDIDATE_POOL` of `contradiction.rs`, which a probe without siblings
/// asks for; a test holds the two together.
const DETECTOR_POOL: usize = 5;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    /// No `sharing.json`: every agent reads every other one.
    Open,
    /// Every agent hides every other one.
    Closed,
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    name: &'static str,
    agents: usize,
    projects: usize,
    rows: usize,
    reading: Reading,
}

const SHAPES: [Shape; 5] = [
    Shape { name: "F1", agents: 1, projects: 3, rows: 10_000, reading: Reading::Open },
    Shape { name: "F2", agents: 3, projects: 5, rows: 10_000, reading: Reading::Open },
    Shape { name: "F3", agents: 3, projects: 5, rows: 10_000, reading: Reading::Closed },
    Shape { name: "F4", agents: 3, projects: 5, rows: 50_000, reading: Reading::Open },
    Shape { name: "E1", agents: 5, projects: 20, rows: 50_000, reading: Reading::Closed },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    OwnTopic,
    SharedTopic,
    /// A claim with this many superseded versions.
    Chain(usize),
    /// A claim every agent saved in every project.
    Repeated,
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    agent: usize,
    project: Option<usize>,
    topic: usize,
    claim: Option<usize>,
    preference: bool,
    superseded: bool,
    deleted: bool,
    vector: Vec<f32>,
}

/// A search, or a probe of the detector, that the reader runs.
#[derive(Clone, Debug, PartialEq)]
struct Query {
    kind: Kind,
    project: usize,
    claim: Option<usize>,
    /// The live version of the chain the query is about.
    live: Option<usize>,
    vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
struct Corpus {
    rows: Vec<Row>,
    queries: Vec<Query>,
    probes: Vec<Query>,
}

fn agent_id(agent: usize) -> String {
    format!("a{agent}")
}

fn project_id(project: usize) -> String {
    format!("p{project}")
}

/// Every row states something, so the detector weighs every one.
fn memory_type(row: &Row) -> MemoryType {
    if row.preference {
        MemoryType::UserPreference
    } else {
        MemoryType::SemanticFact
    }
}

impl Shape {
    /// The reader every search and probe runs as: `a0`, under the reading
    /// of the shape.
    fn reader(&self) -> Reader {
        let hidden = match self.reading {
            Reading::Open => Vec::new(),
            Reading::Closed => (1..self.agents).map(agent_id).collect(),
        };
        Reader::new(agent_id(0), Reach::AllBut(hidden))
    }
}

/// SplitMix64, as `perf_bench.rs` and `revision-eval` copy it: a fixed
/// sequence for a seed, whatever version of `rand`.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in (0, 1], so its logarithm is finite.
    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) + 1) as f64 / (1u64 << 53) as f64
    }

    /// A standard normal draw, by Box-Muller.
    fn gaussian(&mut self) -> f64 {
        let (u, v) = (self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }

    /// A direction drawn uniformly on the unit sphere.
    fn direction(&mut self) -> Vec<f32> {
        let raw: Vec<f32> = (0..EMBEDDING_DIM).map(|_| self.gaussian() as f32).collect();
        to_unit(&raw).expect("a gaussian draw is never zero")
    }
}

/// The unit vector of the sum of `parts`, each a direction under its squared
/// weight. It goes through `to_unit`, as a stored vector and a query do.
fn mix(parts: &[(f32, &[f32])]) -> Vec<f32> {
    let mut sum = vec![0f32; EMBEDDING_DIM];
    for (weight, direction) in parts {
        let weight = weight.sqrt();
        for (s, d) in sum.iter_mut().zip(direction.iter()) {
            *s += weight * d;
        }
    }
    to_unit(&sum).expect("a weighted sum of directions is never zero")
}

/// The L2 distance of two vectors, in `f64`.
fn l2(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>().sqrt()
}

struct Builder {
    rng: SplitMix64,
    global: Vec<f32>,
    topics: Vec<Vec<f32>>,
    claims: usize,
    plain: usize,
    rows: Vec<Row>,
    queries: Vec<Query>,
    probes: Vec<Query>,
}

impl Builder {
    fn new(seed: u64) -> Self {
        let mut rng = SplitMix64(seed);
        let global = rng.direction();
        Builder { rng, global, topics: Vec::new(), claims: 0, plain: 0, rows: Vec::new(), queries: Vec::new(), probes: Vec::new() }
    }

    fn corpus(self) -> Corpus {
        Corpus { rows: self.rows, queries: self.queries, probes: self.probes }
    }

    fn topic(&mut self) -> usize {
        let direction = self.rng.direction();
        self.topics.push(direction);
        self.topics.len() - 1
    }

    /// A vector of `topic` that states `claim`, or something of its own.
    fn vector(&mut self, topic: usize, claim: Option<&[f32]>) -> Vec<f32> {
        let noise = self.rng.direction();
        match claim {
            Some(claim) => mix(&[(GLOBAL, &self.global), (TOPIC, &self.topics[topic]), (CLAIM, claim), (NOISE, &noise)]),
            None => mix(&[(GLOBAL, &self.global), (TOPIC, &self.topics[topic]), (CLAIM + NOISE, &noise)]),
        }
    }

    fn claim(&mut self) -> (usize, Vec<f32>) {
        self.claims += 1;
        (self.claims - 1, self.rng.direction())
    }

    fn push(&mut self, row: Row) -> usize {
        self.rows.push(row);
        self.rows.len() - 1
    }

    /// A row that belongs to no chain and repeats no claim: one in
    /// [`ONE_IN`] of them is a preference, and another one is deleted.
    fn plain(&mut self, agent: usize, project: Option<usize>, topic: usize) {
        let k = self.plain;
        self.plain += 1;
        let vector = self.vector(topic, None);
        self.push(Row {
            agent,
            project,
            topic,
            claim: None,
            preference: k % ONE_IN == 0,
            superseded: false,
            deleted: k % ONE_IN == ONE_IN / 2,
            vector,
        });
    }
}

/// The corpus of `shape`, the same for the same `seed`: its rows, the
/// searches the reader runs over them and the probes of the detector.
fn lay_out(shape: Shape, seed: u64) -> Corpus {
    let (agents, projects) = (shape.agents, shape.projects);
    let chain_rows: usize = CHAIN_LENGTHS.iter().map(|n| (n + 1) * CHAINS_PER_LENGTH).sum();
    let copies = REPEATED_CLAIMS * agents * projects;
    let unowned = shape.rows * UNOWNED_PERCENT / 100;
    let in_projects =
        shape.rows.checked_sub(chain_rows + copies + unowned).expect("the shape has room for its chains and copies");
    let per_project: Vec<usize> =
        (0..projects).map(|p| in_projects / projects + usize::from(p < in_projects % projects)).collect();

    let mut b = Builder::new(seed);
    let shared: Vec<usize> = (0..(per_project[0] / 2).div_ceil(BLOCK).max(1)).map(|_| b.topic()).collect();
    let own: Vec<Vec<usize>> = per_project
        .iter()
        .map(|n| (0..(n - n / 2).div_ceil(BLOCK).max(1)).map(|_| b.topic()).collect())
        .collect();
    let searched = projects.min(QUERY_PROJECTS);

    let lengths = CHAIN_LENGTHS.iter().flat_map(|&n| std::iter::repeat_n(n, CHAINS_PER_LENGTH));
    for (c, length) in lengths.enumerate() {
        let project = c % projects;
        let topic = if (c / projects) % 2 == 0 {
            own[project][(c / 2) % own[project].len()]
        } else {
            shared[(c / 2) % shared.len()]
        };
        let (claim, direction) = b.claim();
        let version = |b: &mut Builder, superseded| {
            let vector = b.vector(topic, Some(&direction));
            let row = Row {
                agent: 0,
                project: Some(project),
                topic,
                claim: Some(claim),
                preference: false,
                superseded,
                deleted: false,
                vector,
            };
            b.push(row)
        };
        for _ in 0..length {
            version(&mut b, true);
        }
        let live = version(&mut b, false);
        let query = |b: &mut Builder| Query {
            kind: Kind::Chain(length),
            project,
            claim: Some(claim),
            live: Some(live),
            vector: b.vector(topic, Some(&direction)),
        };
        let (search, probe) = (query(&mut b), query(&mut b));
        b.queries.push(search);
        b.probes.push(probe);
    }

    for j in 0..REPEATED_CLAIMS {
        let topic = shared[j % shared.len()];
        let (claim, direction) = b.claim();
        for project in 0..projects {
            for agent in 0..agents {
                let vector = b.vector(topic, Some(&direction));
                let row = Row {
                    agent,
                    project: Some(project),
                    topic,
                    claim: Some(claim),
                    preference: false,
                    superseded: false,
                    deleted: false,
                    vector,
                };
                b.push(row);
            }
        }
        for project in 0..searched {
            let vector = b.vector(topic, Some(&direction));
            b.probes.push(Query { kind: Kind::Repeated, project, claim: Some(claim), live: None, vector });
        }
    }

    // Within a topic the rows go to each agent in turn.
    for (project, &n) in per_project.iter().enumerate() {
        for j in 0..n / 2 {
            b.plain((j / shared.len()) % agents, Some(project), shared[j % shared.len()]);
        }
        let mine = &own[project];
        for j in 0..n - n / 2 {
            b.plain((j / mine.len()) % agents, Some(project), mine[j % mine.len()]);
        }
    }
    for u in 0..unowned {
        b.plain((u / shared.len()) % agents, None, shared[u % shared.len()]);
    }

    for project in 0..searched {
        let topics = own[project].iter().take(TOPICS_PER_KIND).map(|&t| (Kind::OwnTopic, t));
        let topics = topics.chain(shared.iter().take(TOPICS_PER_KIND).map(|&t| (Kind::SharedTopic, t)));
        for (kind, topic) in topics.collect::<Vec<_>>() {
            let vector = b.vector(topic, None);
            b.queries.push(Query { kind, project, claim: None, live: None, vector });
        }
    }

    b.corpus()
}

/// Whether `reader`, searching from `project`, may receive `row`: the
/// predicates `SEARCH_SQL_MEMORY` applies after the cut of vec0, read off
/// what the layout knows of the row.
fn eligible(row: &Row, reader: &Reader, project: usize) -> bool {
    !row.superseded
        && !row.deleted
        && reader.admits(Some(&agent_id(row.agent)))
        && (row.project.is_none() || row.project == Some(project) || row.preference)
}

/// The share of `reference` that `got` left out; none for an empty reference.
fn loss(reference: &[i64], got: &[i64]) -> Option<f64> {
    if reference.is_empty() {
        return None;
    }
    let missing = reference.iter().filter(|id| !got.contains(id)).count();
    Some(missing as f64 / reference.len() as f64)
}

/// The nearest-rank quantile `q` of `sorted`, which is not empty.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// A corpus in a store of its own under the system temp dir, and the id
/// each row got.
struct Seeded {
    store: MemoryStore,
    ids: Vec<i64>,
    rows: HashMap<i64, usize>,
    _dir: TestDir,
}

const INSERT_ROW: &str = "INSERT INTO memory (scope, agent_id, project_id, content, memory_type, source, created_at, \
     created_by_type, created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
     VALUES ('agent', ?1, ?2, ?3, ?4, 'chat', ?5, 'user', ?1, ?5, ?5, 5.0, 3, ?6, ?7)";

fn bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Writes `corpus` in one transaction. Every row gets the same importance,
/// tier and age, and an age a day ahead of the clock a search reads leaves
/// every decay at exactly 1 (`score.rs` clamps the age at 0): the score of a
/// row is its distance alone. A deleted row has no vector, as a delete leaves
/// it.
async fn seed(corpus: &Corpus) -> Seeded {
    let dir = test_dir();
    let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.expect("a store in the test dir");
    let at = TimestampMs::now().as_millis_i64() + DAY_MS;
    let rows: Vec<_> = corpus
        .rows
        .iter()
        .map(|r| {
            let status = if r.superseded { "superseded" } else { "active" };
            (agent_id(r.agent), r.project.map(project_id), memory_type(r).as_str(), status, (!r.deleted).then(|| bytes(&r.vector)))
        })
        .collect();
    let ids = store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<i64>> {
            let tx = c.transaction()?;
            let mut ids = Vec::with_capacity(rows.len());
            {
                let mut insert_row = tx.prepare_cached(INSERT_ROW)?;
                let mut insert_vector = tx.prepare_cached("INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)")?;
                for (i, (agent, project, memory_type, status, vector)) in rows.into_iter().enumerate() {
                    let deleted = vector.is_none().then_some(at);
                    insert_row.execute(params![agent, project, format!("row {i}"), memory_type, at, status, deleted])?;
                    let id = tx.last_insert_rowid();
                    if let Some(vector) = vector {
                        insert_vector.execute(params![id, vector])?;
                    }
                    ids.push(id);
                }
            }
            tx.commit()?;
            Ok(ids)
        })
        .await
        .expect("the corpus is written");
    let rows = ids.iter().enumerate().map(|(row, &id)| (id, row)).collect();
    Seeded { store, ids, rows, _dir: dir }
}

impl Seeded {
    /// Closes the store before its directory goes, so nothing is left behind.
    async fn close(self) {
        self.store.conn.clone().close().await.expect("the store closes");
    }
}

/// What a search or a probe returned, and what it cost.
#[derive(Debug)]
struct Ran {
    ids: Vec<i64>,
    /// Whether the `memory` KNN ran its second, wider pass.
    retried: bool,
    /// Microseconds of the whole search and of its `memory` KNN arm.
    total_us: u64,
    knn_us: u64,
}

/// What the recorder of `store` holds of the one search since the last
/// read, which empties it: `search.knn_retry` and `search.duration`.
fn ran(store: &MemoryStore, ids: Vec<i64>) -> Ran {
    let window = store.metrics().take_minute();
    let retried = window.gauges.contains_key(&SeriesKey::new("search.knn_retry", &[("table", "memory")]));
    let took = |arm| window.histograms.get(&SeriesKey::new("search.duration", &[("arm", arm)])).map_or(0, |h| h.sum());
    Ran { ids, retried, total_us: took("total"), knn_us: took("knn_memory") }
}

/// A search as the daemon runs it (`search` in `daemon/api.rs`): the default
/// scope from `project` with the reach of `reader`, asking for the whole pool
/// so a cut at any `limit` reads its first rows. No text, so no lexical
/// channel: what is measured is the dense window.
async fn search(store: &MemoryStore, reader: &Reader, project: usize, vector: &[f32]) -> Ran {
    let scope = SearchScope::all_default().with_project(Some(&project_id(project))).with_reach(reader.reach.clone());
    assert_eq!(scope.memory, Some(POOL), "the reference is cut at the pool a search asks for");
    store.metrics().take_minute();
    let hits = search_all_memories(store, vector, None, &reader.agent_id, scope, POOL).await.expect("the search runs");
    ran(store, hits.into_iter().filter(|h| h.kind == TableKind::Memory).map(|h| h.id).collect())
}

/// A probe of the detector as a save without a key or siblings runs it.
async fn probe(store: &MemoryStore, reader: &Reader, project: usize, vector: &[f32]) -> Ran {
    store.metrics().take_minute();
    let project = project_id(project);
    let found = detect_contradiction(store, vector, "", reader, Some(&project), MemoryType::SemanticFact, &[]).await;
    ran(store, found.into_iter().map(|(_, c)| c.id).collect())
}

/// The nearest rows `reader` may receive from `project`, at most `limit`,
/// with the distance vec0 gives, from every row: what a window nothing
/// crowded would hold. It reads the predicates of the search through the
/// same macros, so the two cannot drift apart.
const EXACT_SQL: &str = concat!(
    "SELECT m.id, vec_distance_l2(v.embedding, ?1) AS distance
     FROM memory m JOIN memory_vectors v ON v.rowid = m.id
     WHERE m.status IN ('active', 'disputed')
       AND m.deleted_at IS NULL
       AND ",
    reach_predicate!("m.agent_id", "?2"),
    "
       AND ",
    memory_project_predicate!("m.", "?3"),
    "
     ORDER BY distance, m.id
     LIMIT ?4"
);

async fn exact(store: &MemoryStore, reader: &Reader, project: usize, vector: &[f32], limit: usize) -> Vec<(i64, f64)> {
    // The search turns its query into a unit vector before vec0 sees it.
    let query = bytes(&to_unit(vector).expect("a query of the layout is usable"));
    let reach = reader_bind(Some(reader));
    let project = project_id(project);
    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<(i64, f64)>> {
            let mut stmt = c.prepare_cached(EXACT_SQL)?;
            let rows = stmt.query_map(params![query, reach, project, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .await
        .expect("the exact query runs")
}

/// [`EXACT_SQL`] worked out from the layout alone: [`eligible`] and [`l2`].
fn pure_reference(corpus: &Corpus, ids: &[i64], reader: &Reader, project: usize, vector: &[f32], limit: usize) -> Vec<(i64, f64)> {
    let query = to_unit(vector).expect("a query of the layout is usable");
    let mut near: Vec<(i64, f64)> = corpus
        .rows
        .iter()
        .zip(ids)
        .filter(|(row, _)| eligible(row, reader, project))
        .map(|(row, &id)| (id, l2(&query, &row.vector)))
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    near.truncate(limit);
    near
}

/// The L2 distance under which a row scores above 0: the cosine floor of the
/// default scoring, between unit vectors.
fn floor_distance() -> f64 {
    let floor = ScoringConfig::DEFAULT.cosine_floor.expect("the default scoring has a cosine floor");
    (2.0 * (1.0 - floor)).sqrt()
}

/// A search against its reference.
struct Searched {
    kind: Kind,
    /// The share of the reference at [`LIMIT`] and at [`POOL`] the search did
    /// not return; none when no eligible row scores above 0.
    loss10: Option<f64>,
    loss30: Option<f64>,
    /// Where the live version of a chain came, from 1; none when it did not.
    live_rank: Option<usize>,
    ran: Ran,
}

/// A probe of the detector against its reference.
struct Probed {
    kind: Kind,
    /// The candidates of the reference, and how many of them the probe missed.
    reference: usize,
    missed: usize,
    ran: Ran,
}

struct Measured {
    searches: Vec<Searched>,
    probes: Vec<Probed>,
}

/// Runs every search and probe of `corpus` as `reader` and holds each against
/// the exact nearest rows. With every score down to the distance, the
/// reference of a search is the rows a window nothing crowded would rank
/// first, past those scoring 0, and the reference of a probe is the pool of
/// the detector over the eligible rows, through its band and its types.
async fn measure(seeded: &Seeded, corpus: &Corpus, reader: &Reader) -> Measured {
    let floor = floor_distance();
    let mut searches = Vec::with_capacity(corpus.queries.len());
    for query in &corpus.queries {
        let ran = search(&seeded.store, reader, query.project, &query.vector).await;
        let near = exact(&seeded.store, reader, query.project, &query.vector, POOL).await;
        let scored: Vec<i64> = near.iter().filter(|n| n.1 < floor).map(|n| n.0).collect();
        let at = |limit: usize| loss(&scored[..scored.len().min(limit)], &ran.ids[..ran.ids.len().min(limit)]);
        let live_rank = query.live.and_then(|row| ran.ids.iter().position(|&id| id == seeded.ids[row]).map(|p| p + 1));
        searches.push(Searched { kind: query.kind, loss10: at(LIMIT), loss30: at(POOL), live_rank, ran });
    }
    let band = f64::from(NEAR_DUPLICATE_FLOOR)..f64::from(CONTRADICTION_THRESHOLD);
    let mut probes = Vec::with_capacity(corpus.probes.len());
    for query in &corpus.probes {
        let ran = probe(&seeded.store, reader, query.project, &query.vector).await;
        let near = exact(&seeded.store, reader, query.project, &query.vector, DETECTOR_POOL).await;
        let candidates: Vec<i64> = near
            .iter()
            .filter(|(id, d)| band.contains(d) && memory_type(&corpus.rows[seeded.rows[id]]).is_claim_bearing())
            .map(|n| n.0)
            .collect();
        let missed = candidates.iter().filter(|id| !ran.ids.contains(id)).count();
        probes.push(Probed { kind: query.kind, reference: candidates.len(), missed, ran });
    }
    Measured { searches, probes }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shape small enough to lay out in a unit test, with every source of
    /// crowding: two agents, two projects, chains and repeated claims.
    const SMALL: Shape = Shape { name: "small", agents: 2, projects: 2, rows: 3_000, reading: Reading::Closed };

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len() as f64
    }

    #[test]
    fn the_same_seed_lays_out_the_same_corpus() {
        let first = lay_out(SMALL, 7);
        assert_eq!(first, lay_out(SMALL, 7));
        let other = lay_out(SMALL, 8);
        assert_eq!(first.rows.len(), other.rows.len());
        assert_ne!(first.rows[0].vector, other.rows[0].vector);
    }

    #[test]
    fn a_shape_holds_its_chains_copies_and_unowned_rows() {
        let f2 = SHAPES.iter().copied().find(|s| s.name == "F2").unwrap();
        let corpus = lay_out(f2, 37);
        assert_eq!(corpus.rows.len(), 10_000);
        assert_eq!(corpus.rows.iter().filter(|r| r.project.is_none()).count(), 2_000);
        assert_eq!(corpus.rows.iter().filter(|r| r.superseded).count(), 1_125);

        let chains: Vec<&Query> = corpus.queries.iter().filter(|q| matches!(q.kind, Kind::Chain(_))).collect();
        assert_eq!(chains.len(), 60);
        for length in CHAIN_LENGTHS {
            assert_eq!(chains.iter().filter(|q| q.kind == Kind::Chain(length)).count(), CHAINS_PER_LENGTH);
        }
        for query in &chains {
            assert_eq!(corpus.rows[query.live.unwrap()].project, Some(query.project), "a chain is searched from its project");
        }
        assert!(corpus.queries.iter().chain(&corpus.probes).all(|q| q.project < QUERY_PROJECTS));
        let mut live: Vec<usize> = chains.iter().map(|q| q.live.unwrap()).collect();
        live.sort_unstable();
        live.dedup();
        assert_eq!(live.len(), 60, "every chain has its own live version");
        for &row in &live {
            let row = &corpus.rows[row];
            assert!(row.agent == 0 && !row.superseded && !row.deleted && row.project.is_some());
        }

        let copies = corpus.rows.iter().filter(|r| r.claim.is_some() && !r.superseded).count() - live.len();
        assert_eq!(copies, 15 * 3 * 5);
        assert_eq!(corpus.probes.iter().filter(|p| p.kind == Kind::Repeated).count(), 15 * 5);
        assert_eq!(corpus.probes.iter().filter(|p| matches!(p.kind, Kind::Chain(_))).count(), 60);

        // 8 590 rows are neither chains nor copies: one in 20 is a
        // preference and another one in 20 is deleted.
        assert_eq!(corpus.rows.iter().filter(|r| r.preference).count(), 430);
        assert_eq!(corpus.rows.iter().filter(|r| r.deleted).count(), 429);
        let topics = |kind| corpus.queries.iter().filter(|q| q.kind == kind).count();
        assert_eq!((topics(Kind::OwnTopic), topics(Kind::SharedTopic)), (5 * 17, 5 * 17));
    }

    #[test]
    fn the_layout_puts_topics_claims_and_unrelated_rows_at_their_distances() {
        let corpus = lay_out(SMALL, 7);
        let plain: Vec<&Row> = corpus.rows.iter().filter(|r| r.claim.is_none()).take(400).collect();
        let (mut same_topic, mut other_topic) = (Vec::new(), Vec::new());
        for (i, a) in plain.iter().enumerate() {
            for b in &plain[i + 1..] {
                let d = l2(&a.vector, &b.vector);
                if a.topic == b.topic { same_topic.push(d) } else { other_topic.push(d) }
            }
        }
        let mut same_claim = Vec::new();
        for (i, a) in corpus.rows.iter().enumerate() {
            let Some(claim) = a.claim else { continue };
            for b in corpus.rows[i + 1..].iter().filter(|b| b.claim == Some(claim)) {
                same_claim.push(l2(&a.vector, &b.vector));
            }
        }
        assert!(!same_topic.is_empty() && !other_topic.is_empty() && !same_claim.is_empty());
        assert!((mean(&other_topic) - 0.71).abs() < 0.02, "{}", mean(&other_topic));
        assert!((mean(&same_topic) - 0.49).abs() < 0.02, "{}", mean(&same_topic));
        assert!((mean(&same_claim) - 0.32).abs() < 0.02, "{}", mean(&same_claim));
        let band = f64::from(NEAR_DUPLICATE_FLOOR)..f64::from(CONTRADICTION_THRESHOLD);
        assert!(same_claim.iter().all(|d| band.contains(d)), "a pair of one claim left the dispute band");
        for probe in &corpus.probes {
            let claim = corpus.rows.iter().find(|r| r.claim.is_some() && probe.claim == r.claim).unwrap();
            assert!(band.contains(&l2(&probe.vector, &claim.vector)));
        }
    }

    #[test]
    fn eligibility_follows_the_production_predicates() {
        let row = |agent, project, preference, superseded, deleted| Row {
            agent,
            project,
            topic: 0,
            claim: None,
            preference,
            superseded,
            deleted,
            vector: Vec::new(),
        };
        let reader = Reader::new(agent_id(0), Reach::AllBut(vec![agent_id(2)]));
        assert!(eligible(&row(0, Some(1), false, false, false), &reader, 1), "its own project");
        assert!(eligible(&row(1, None, false, false, false), &reader, 1), "no project");
        assert!(eligible(&row(1, Some(3), true, false, false), &reader, 1), "a preference of another project");
        assert!(!eligible(&row(0, Some(3), false, false, false), &reader, 1), "another project");
        assert!(!eligible(&row(2, Some(1), false, false, false), &reader, 1), "a hidden agent");
        assert!(!eligible(&row(0, Some(1), false, true, false), &reader, 1), "superseded");
        assert!(!eligible(&row(0, Some(1), false, false, true), &reader, 1), "deleted");
    }

    #[test]
    fn a_closed_reading_hides_every_other_agent() {
        let shape = |reading| Shape { reading, ..SMALL };
        let open = shape(Reading::Open).reader();
        let closed = shape(Reading::Closed).reader();
        assert_eq!(open.agent_id, "a0");
        assert!(open.admits(Some("a1")));
        assert!(closed.admits(Some("a0")) && !closed.admits(Some("a1")));
    }

    #[test]
    fn loss_counts_the_reference_rows_the_result_left_out() {
        let reference: Vec<i64> = (1..=10).collect();
        let got = [1, 2, 3, 4, 5, 6, 7, 20, 21, 22];
        assert_eq!(loss(&reference, &got), Some(0.3));
        assert_eq!(loss(&reference, &reference), Some(0.0));
        assert_eq!(loss(&[], &got), None);
    }

    /// `topics` topics of `per_topic` rows each, all `a0`'s in `p0`, and a
    /// search of each topic.
    fn one_reader(seed: u64, topics: usize, per_topic: usize) -> Builder {
        let mut b = Builder::new(seed);
        for _ in 0..topics {
            let topic = b.topic();
            for _ in 0..per_topic {
                b.plain(0, Some(0), topic);
            }
            let vector = b.vector(topic, None);
            b.queries.push(Query { kind: Kind::OwnTopic, project: 0, claim: None, live: None, vector });
        }
        b
    }

    #[tokio::test]
    async fn the_exact_query_returns_the_pure_reference() {
        let corpus = lay_out(SMALL, 7);
        let seeded = seed(&corpus).await;
        let reader = SMALL.reader();
        for query in corpus.queries.iter().chain(&corpus.probes).step_by(3) {
            let sql = exact(&seeded.store, &reader, query.project, &query.vector, POOL).await;
            let pure = pure_reference(&corpus, &seeded.ids, &reader, query.project, &query.vector, POOL);
            let ids = |near: &[(i64, f64)]| near.iter().map(|n| n.0).collect::<Vec<_>>();
            assert_eq!(ids(&sql), ids(&pure), "{:?} from p{}", query.kind, query.project);
            for ((_, a), (_, b)) in sql.iter().zip(&pure) {
                assert!((a - b).abs() < 1e-5, "{a} against {b}");
            }
        }
        seeded.close().await;
    }

    #[tokio::test]
    async fn without_foreign_rows_nothing_is_lost_in_one_pass() {
        let corpus = one_reader(11, 3, BLOCK).corpus();
        let seeded = seed(&corpus).await;
        let reader = Reader::new(agent_id(0), Reach::AllBut(Vec::new()));
        let measured = measure(&seeded, &corpus, &reader).await;
        assert_eq!(measured.searches.len(), 3);
        for searched in &measured.searches {
            assert_eq!((searched.loss10, searched.loss30), (Some(0.0), Some(0.0)));
            assert!(!searched.ran.retried, "a window of eligible rows needs no second pass");
            assert!(searched.ran.total_us > 0 && searched.ran.knn_us > 0);
        }
        seeded.close().await;
    }

    #[tokio::test]
    async fn rows_past_the_widened_window_are_counted_lost() {
        // The reader's 40 rows of the topic sit near 0.49 from the query, and
        // 200 rows of another project state the query's own claim, near 0.32.
        let mut b = one_reader(13, 1, BLOCK);
        let topic = b.rows[0].topic;
        let (claim, direction) = b.claim();
        for _ in 0..200 {
            let vector = b.vector(topic, Some(&direction));
            b.push(Row {
                agent: 0,
                project: Some(1),
                topic,
                claim: Some(claim),
                preference: false,
                superseded: false,
                deleted: false,
                vector,
            });
        }
        b.queries[0].vector = b.vector(topic, Some(&direction));
        let corpus = b.corpus();
        let seeded = seed(&corpus).await;
        let reader = Reader::new(agent_id(0), Reach::AllBut(Vec::new()));
        let measured = measure(&seeded, &corpus, &reader).await;
        let searched = &measured.searches[0];
        assert!(searched.ran.retried, "a crowded window runs its second pass");
        assert_eq!(searched.loss30, Some(1.0), "the 120 rows of the wider window are all the other project's");
        seeded.close().await;
    }

    #[tokio::test]
    async fn the_live_version_behind_its_revisions_is_found_first() {
        // Ten superseded versions and the live one are the eleven rows nearest
        // the claim; the wider passes of the search (120) and the detector
        // (20) both reach past them.
        let mut b = one_reader(19, 1, BLOCK);
        b.queries.clear();
        let topic = b.rows[0].topic;
        let (claim, direction) = b.claim();
        let mut live = 0;
        for superseded in std::iter::repeat_n(true, 10).chain([false]) {
            let vector = b.vector(topic, Some(&direction));
            live = b.push(Row {
                agent: 0,
                project: Some(0),
                topic,
                claim: Some(claim),
                preference: false,
                superseded,
                deleted: false,
                vector,
            });
        }
        let query = |b: &mut Builder| Query {
            kind: Kind::Chain(10),
            project: 0,
            claim: Some(claim),
            live: Some(live),
            vector: b.vector(topic, Some(&direction)),
        };
        let (search, probe) = (query(&mut b), query(&mut b));
        b.queries.push(search);
        b.probes.push(probe);
        let corpus = b.corpus();
        let seeded = seed(&corpus).await;
        let reader = Reader::new(agent_id(0), Reach::AllBut(Vec::new()));
        let measured = measure(&seeded, &corpus, &reader).await;
        let (searched, probed) = (&measured.searches[0], &measured.probes[0]);
        assert_eq!((searched.kind, searched.live_rank), (Kind::Chain(10), Some(1)));
        assert!(searched.ran.retried, "the superseded versions crowd the first window");
        assert_eq!((probed.kind, probed.reference, probed.missed), (Kind::Chain(10), 1, 0));
        assert_eq!(probed.ran.ids, vec![seeded.ids[live]]);
        seeded.close().await;
    }

    #[tokio::test]
    async fn the_detector_reference_keeps_the_pool_of_the_detector() {
        // Seven rows of one claim, all eligible and inside the band: the
        // detector weighs as many as its pool, and so must its reference.
        let mut b = Builder::new(17);
        let topic = b.topic();
        let (claim, direction) = b.claim();
        for _ in 0..7 {
            let vector = b.vector(topic, Some(&direction));
            b.push(Row {
                agent: 0,
                project: Some(0),
                topic,
                claim: Some(claim),
                preference: false,
                superseded: false,
                deleted: false,
                vector,
            });
        }
        let vector = b.vector(topic, Some(&direction));
        b.probes.push(Query { kind: Kind::Repeated, project: 0, claim: Some(claim), live: None, vector });
        let corpus = b.corpus();
        let seeded = seed(&corpus).await;
        let reader = Reader::new(agent_id(0), Reach::AllBut(Vec::new()));
        let measured = measure(&seeded, &corpus, &reader).await;
        let probed = &measured.probes[0];
        assert_eq!((probed.reference, probed.missed), (DETECTOR_POOL, 0));
        assert_eq!(probed.ran.ids.len(), DETECTOR_POOL);
        seeded.close().await;
    }

    #[test]
    fn a_quantile_is_the_nearest_rank() {
        let sorted: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(quantile(&sorted, 0.50), 50.0);
        assert_eq!(quantile(&sorted, 0.99), 99.0);
        assert_eq!(quantile(&sorted, 1.0), 100.0);
        assert_eq!(quantile(&[4.0], 0.5), 4.0);
    }
}
