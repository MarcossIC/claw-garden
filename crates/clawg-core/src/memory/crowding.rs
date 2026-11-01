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

use crate::memory::contradiction::{CONTRADICTION_THRESHOLD, NEAR_DUPLICATE_FLOOR};
use crate::memory::sharing::{Reach, Reader};
use crate::memory::vector::{to_unit, EMBEDDING_DIM};

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
}

impl Builder {
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

    let mut rng = SplitMix64(seed);
    let global = rng.direction();
    let mut b = Builder { rng, global, topics: Vec::new(), claims: 0, plain: 0, rows: Vec::with_capacity(shape.rows) };
    let shared: Vec<usize> = (0..(per_project[0] / 2).div_ceil(BLOCK).max(1)).map(|_| b.topic()).collect();
    let own: Vec<Vec<usize>> = per_project
        .iter()
        .map(|n| (0..(n - n / 2).div_ceil(BLOCK).max(1)).map(|_| b.topic()).collect())
        .collect();
    let searched = projects.min(QUERY_PROJECTS);
    let mut queries = Vec::new();
    let mut probes = Vec::new();

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
        for list in [&mut queries, &mut probes] {
            let vector = b.vector(topic, Some(&direction));
            list.push(Query { kind: Kind::Chain(length), project, claim: Some(claim), live: Some(live), vector });
        }
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
            probes.push(Query { kind: Kind::Repeated, project, claim: Some(claim), live: None, vector });
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
            queries.push(Query { kind, project, claim: None, live: None, vector });
        }
    }

    Corpus { rows: b.rows, queries, probes }
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

    #[test]
    fn a_quantile_is_the_nearest_rank() {
        let sorted: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(quantile(&sorted, 0.50), 50.0);
        assert_eq!(quantile(&sorted, 0.99), 99.0);
        assert_eq!(quantile(&sorted, 1.0), 100.0);
        assert_eq!(quantile(&[4.0], 0.5), 4.0);
    }
}
