//! What indexing a project's docs cost: the `index` block of a docs results
//! file and the lines of the report that read it.

use clawg_core::eval_api::{longest_first, EvalCpu, EvalIndexRun, EvalMemoReport, EvalPassageLengths, EMBED_BATCH};
use serde::Serialize;
use std::collections::HashMap;

/// Seconds a refresh spent in each of its phases, summed over its docs. The
/// removal of docs that are gone counts as a write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Phases {
    pub scan: f64,
    pub read: f64,
    pub mask: f64,
    pub split: f64,
    pub lookup: f64,
    pub embed: f64,
    pub write: f64,
}

impl Phases {
    pub fn of(run: &EvalIndexRun) -> Self {
        let secs = |f: fn(&clawg_core::eval_api::EvalPhases) -> std::time::Duration| {
            run.docs.iter().map(|d| f(&d.phases)).sum::<std::time::Duration>().as_secs_f64()
        };
        Self {
            scan: run.scan.as_secs_f64(),
            read: secs(|p| p.read),
            mask: secs(|p| p.mask),
            split: secs(|p| p.split),
            lookup: secs(|p| p.lookup),
            embed: secs(|p| p.embed),
            write: secs(|p| p.write) + run.remove.as_secs_f64(),
        }
    }

    /// One line for the report, in the order a refresh runs them.
    pub fn line(&self) -> String {
        format!(
            "phases: scan {:.2} s  read {:.2} s  mask {:.2} s  split {:.2} s  lookup {:.2} s  embed {:.2} s  write {:.2} s\n",
            self.scan, self.read, self.mask, self.split, self.lookup, self.embed, self.write,
        )
    }
}

/// The machine a run measured on: what decides the kernels of the model and
/// the threads its session runs with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Machine {
    #[serde(flatten)]
    pub cpu: EvalCpu,
    pub threads: usize,
}

/// How many passages came from the vector memo and how many the model
/// embedded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MemoBlock {
    pub used: bool,
    pub from_memo: usize,
    pub embedded: usize,
    pub seconds: f64,
}

impl MemoBlock {
    /// A run with no memo embeds every passage of the corpus.
    fn of(memo: Option<&EvalMemoReport>, passages: usize) -> Self {
        match memo {
            Some(m) => Self { used: m.used, from_memo: m.from_memo, embedded: m.embedded, seconds: m.seconds },
            None => Self { used: false, from_memo: 0, embedded: passages, seconds: 0.0 },
        }
    }
}

/// The report line for the vector memo: the file a run used and what came
/// from it, or that it used none.
pub fn memo_line(memo: Option<&EvalMemoReport>) -> String {
    match memo {
        Some(m) => format!(
            "vector memo {}: {} from the memo, {} embedded, {:.2} s\n",
            m.file, m.from_memo, m.embedded, m.seconds
        ),
        None => "vector memo: not used (--no-cache)\n".to_string(),
    }
}

/// Tokens a model run pads a list of passage lengths to: the passages in runs
/// of `EMBED_BATCH`, in order, each run as many rows as it holds times its
/// longest.
pub fn padded(lengths: &[usize]) -> usize {
    lengths.chunks(EMBED_BATCH).map(|run| run.len() * run.iter().copied().max().unwrap_or(0)).sum()
}

/// [`padded`] after sorting the lengths from the longest to the shortest,
/// ties in their order, as an import orders the rows it embeds.
pub fn padded_sorted(lengths: &[usize]) -> usize {
    let sorted: Vec<usize> = longest_first(lengths).into_iter().map(|k| lengths[k]).collect();
    padded(&sorted)
}

/// What the passages of the corpus cost the model in tokens.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tokens {
    /// Inside the model's window, without padding.
    pub real: usize,
    /// Doc by doc, in the order each was embedded: what the model runs
    /// without the memo.
    pub padded: usize,
    /// What the embedder reports it ran.
    pub padded_ran: usize,
    /// `padded` with each doc's passages sorted longest first.
    pub padded_sorted_in_doc: usize,
    /// `padded` with every passage of the run sorted longest first.
    pub padded_sorted_all: usize,
    /// What the calls to the tokenizer took.
    pub count_seconds: f64,
}

impl Tokens {
    /// `docs` holds the lengths of each doc's passages in the order they were
    /// embedded. The three paddings do not bound each other: sorting can pad
    /// more when the last run is short.
    pub fn new(docs: &[Vec<usize>], count_seconds: f64, padded_ran: f64) -> Self {
        let all: Vec<usize> = docs.iter().flatten().copied().collect();
        Self {
            real: all.iter().sum(),
            padded: docs.iter().map(|d| padded(d)).sum(),
            padded_ran: padded_ran.round() as usize,
            padded_sorted_in_doc: docs.iter().map(|d| padded_sorted(d)).sum(),
            padded_sorted_all: padded_sorted(&all),
            count_seconds,
        }
    }

    pub fn of(lengths: &EvalPassageLengths, padded_ran: f64) -> Self {
        let docs: Vec<Vec<usize>> = lengths.docs.iter().map(|d| d.lengths.clone()).collect();
        Self::new(&docs, lengths.count_seconds, padded_ran)
    }

    /// One line for the report.
    pub fn line(&self) -> String {
        format!(
            "tokens: real {}  padded {}  ran {}  sorted in doc {}  sorted in run {}  counted in {:.2} s\n",
            self.real, self.padded, self.padded_ran, self.padded_sorted_in_doc, self.padded_sorted_all, self.count_seconds,
        )
    }
}

/// Said when the padding computed and the padding that ran differ although
/// every passage was embedded: then both are the same runs.
pub fn padding_warning(tokens: &Tokens, from_memo: usize) -> Option<String> {
    (from_memo == 0 && tokens.padded != tokens.padded_ran).then(|| {
        format!(
            "warning: the padded tokens computed ({}) differ from those the embedder ran ({})",
            tokens.padded, tokens.padded_ran
        )
    })
}

/// Where a doc of the corpus sits: the project's own docs, its code, or the
/// archive of finished changes, whose volume would otherwise hide the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Docs,
    Code,
    Archive,
}

/// The area of a doc by its path relative to the root, `/`-separated, and its
/// kind. The prefix is compared whole: a folder or a file that only starts
/// with the same letters is not the archive.
pub fn area_of(path: &str, code: bool) -> Area {
    if path.starts_with(concat!("openspec/changes/archive", "/")) {
        Area::Archive
    } else if code {
        Area::Code
    } else {
        Area::Docs
    }
}

/// [`Phases`] of an area: `scan` belongs to the whole walk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct AreaPhases {
    pub read: f64,
    pub mask: f64,
    pub split: f64,
    pub lookup: f64,
    pub embed: f64,
    pub write: f64,
}

/// Tokens of the passages of an area. `multi_batch_*` count only the docs with
/// more passages than one embedding run holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct AreaTokens {
    pub real: usize,
    pub padded: usize,
    pub multi_batch_real: usize,
    pub multi_batch_padded: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct AreaBlock {
    pub docs: usize,
    pub passages: usize,
    pub multi_batch_docs: usize,
    pub phases: AreaPhases,
    /// `None` without the model's tokenizer.
    pub tokens: Option<AreaTokens>,
}

impl AreaBlock {
    fn line(&self, name: &str) -> String {
        format!(
            "  {name}: {} docs, {} passages ({} over one batch)  read {:.2} s  mask {:.2} s  split {:.2} s  lookup {:.2} s  embed {:.2} s  write {:.2} s
",
            self.docs,
            self.passages,
            self.multi_batch_docs,
            self.phases.read,
            self.phases.mask,
            self.phases.split,
            self.phases.lookup,
            self.phases.embed,
            self.phases.write,
        )
    }
}

/// What indexing cost in each [`Area`]; together they add up to the totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ByArea {
    pub docs: AreaBlock,
    pub code: AreaBlock,
    pub archive: AreaBlock,
}

impl ByArea {
    /// Joins each doc's timings with its passage lengths by path; a doc with no
    /// lengths adds none.
    pub fn of(run: &EvalIndexRun, lengths: Option<&EvalPassageLengths>) -> Self {
        let by_path: HashMap<&str, &[usize]> = lengths
            .map(|l| l.docs.iter().map(|d| (d.path.as_str(), d.lengths.as_slice())).collect())
            .unwrap_or_default();
        let mut areas = Self::default();
        if lengths.is_some() {
            for area in [&mut areas.docs, &mut areas.code, &mut areas.archive] {
                area.tokens = Some(AreaTokens::default());
            }
        }
        for doc in &run.docs {
            let area = match area_of(&doc.path, doc.code) {
                Area::Docs => &mut areas.docs,
                Area::Code => &mut areas.code,
                Area::Archive => &mut areas.archive,
            };
            let multi = doc.passages > EMBED_BATCH;
            area.docs += 1;
            area.passages += doc.passages;
            area.multi_batch_docs += usize::from(multi);
            let p = &doc.phases;
            area.phases.read += p.read.as_secs_f64();
            area.phases.mask += p.mask.as_secs_f64();
            area.phases.split += p.split.as_secs_f64();
            area.phases.lookup += p.lookup.as_secs_f64();
            area.phases.embed += p.embed.as_secs_f64();
            area.phases.write += p.write.as_secs_f64();
            if let Some(tokens) = area.tokens.as_mut() {
                let own = by_path.get(doc.path.as_str()).copied().unwrap_or(&[]);
                let (real, padded) = (own.iter().sum::<usize>(), padded(own));
                tokens.real += real;
                tokens.padded += padded;
                if multi {
                    tokens.multi_batch_real += real;
                    tokens.multi_batch_padded += padded;
                }
            }
        }
        areas
    }

    /// One line per area for the report.
    pub fn lines(&self) -> String {
        [("docs", &self.docs), ("code", &self.code), ("archive", &self.archive)]
            .into_iter()
            .map(|(name, area)| area.line(name))
            .collect()
    }
}

/// The `index` block of a docs results file: numbers, flags and the names of
/// the build, the system and the architecture, never a path or a text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexBlock {
    pub build: &'static str,
    pub machine: Machine,
    pub phases: Phases,
    pub memo: MemoBlock,
    /// `None` without the model's tokenizer.
    pub tokens: Option<Tokens>,
    pub by_area: ByArea,
}

impl IndexBlock {
    pub fn new(
        run: &EvalIndexRun,
        lengths: Option<&EvalPassageLengths>,
        memo: Option<&EvalMemoReport>,
        cpu: EvalCpu,
        threads: usize,
        debug: bool,
        tokens: Option<Tokens>,
    ) -> Self {
        Self {
            build: if debug { "debug" } else { "release" },
            machine: Machine { cpu, threads },
            phases: Phases::of(run),
            memo: MemoBlock::of(memo, run.report.passages),
            tokens,
            by_area: ByArea::of(run, lengths),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(docs: &[&[usize]]) -> Tokens {
        let docs: Vec<Vec<usize>> = docs.iter().map(|d| d.to_vec()).collect();
        Tokens::new(&docs, 0.0, 0.0)
    }

    /// 20 passages alternating 10 and 100 tokens, starting with 10.
    fn alternating() -> Vec<usize> {
        (0..20).map(|i| if i % 2 == 0 { 10 } else { 100 }).collect()
    }

    #[test]
    fn padding_of_a_batch_is_rows_times_the_longest() {
        let t = tokens_of(&[&[10, 40, 25]]);

        assert_eq!((t.real, t.padded, t.padded_sorted_in_doc), (75, 120, 120));
    }

    #[test]
    fn a_doc_over_embed_batch_runs_several_batches() {
        let t = tokens_of(&[&alternating()]);

        assert_eq!((t.real, t.padded, t.padded_sorted_in_doc), (1100, 2000, 1640));
    }

    #[test]
    fn one_batch_pads_the_same_in_any_order() {
        let lengths = [7, 90, 3, 41, 41, 12, 1, 88, 5, 60, 2, 33, 70, 9, 15, 30];
        let mut reversed = lengths;
        reversed.reverse();

        let (a, b) = (tokens_of(&[&lengths]), tokens_of(&[&reversed]));

        assert_eq!(a.padded, 16 * 90);
        assert_eq!((a.padded, a.padded_sorted_in_doc), (b.padded, b.padded_sorted_in_doc));
        assert_eq!(a.padded, a.padded_sorted_in_doc);
    }

    #[test]
    fn sorting_can_pad_more_when_the_last_batch_is_short() {
        let mut lengths = vec![10; 16];
        lengths.push(100);

        let t = tokens_of(&[&lengths]);

        assert_eq!((t.padded, t.padded_sorted_in_doc), (260, 1610));
    }

    #[test]
    fn sorting_keeps_ties_in_their_order() {
        assert_eq!(clawg_core::eval_api::longest_first(&[7, 7, 7, 3, 7]), [0, 1, 2, 4, 3]);
        assert_eq!(padded_sorted(&[7, 3, 7]), padded(&[7, 7, 3]));
    }

    #[test]
    fn sorting_across_docs_batches_the_whole_run() {
        let ten = [10, 10, 10];
        let hundred = [100; 10];

        let t = tokens_of(&[&ten, &ten, &hundred]);
        let same = tokens_of(&[&[7; 3], &[7; 3], &[7; 10]]);

        assert_eq!((t.padded, t.padded_sorted_in_doc, t.padded_sorted_all), (1060, 1060, 1600));
        assert_eq!((same.padded, same.padded_sorted_in_doc, same.padded_sorted_all), (112, 112, 112));
    }

    #[test]
    fn an_empty_corpus_has_no_tokens() {
        for docs in [tokens_of(&[]), tokens_of(&[&[]])] {
            assert_eq!(
                (docs.real, docs.padded, docs.padded_sorted_in_doc, docs.padded_sorted_all),
                (0, 0, 0, 0)
            );
        }
    }

    #[test]
    fn the_ran_padding_is_a_whole_number_of_tokens() {
        let t = Tokens::new(&[vec![10, 40, 25]], 0.5, 120.0);

        assert_eq!((t.padded_ran, t.count_seconds), (120, 0.5));
    }

    #[test]
    fn a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo() {
        let t = Tokens { padded: 1000, padded_ran: 1200, ..tokens_of(&[]) };
        let same = Tokens { padded: 1000, padded_ran: 1000, ..tokens_of(&[]) };

        let warning = padding_warning(&t, 0).expect("a warning");

        assert!(warning.contains("1000") && warning.contains("1200"), "{warning}");
        assert_eq!(padding_warning(&t, 30), None, "with passages from the memo, the model ran fewer");
        assert_eq!(padding_warning(&same, 0), None);
    }

    fn timing(path: &str, code: bool, passages: usize, ms: [u64; 6]) -> clawg_core::eval_api::EvalDocTiming {
        use std::time::Duration;
        let [read, mask, split, lookup, embed, write] = ms.map(Duration::from_millis);
        clawg_core::eval_api::EvalDocTiming {
            path: path.to_string(),
            code,
            passages,
            phases: clawg_core::eval_api::EvalPhases { read, mask, split, lookup, embed, write },
        }
    }

    fn run_of(docs: Vec<clawg_core::eval_api::EvalDocTiming>) -> EvalIndexRun {
        let passages = docs.iter().map(|d| d.passages).sum();
        EvalIndexRun {
            report: clawg_core::eval_api::EvalDocReport { docs: docs.len(), passages, skipped: 0 },
            scan: std::time::Duration::from_millis(500),
            remove: std::time::Duration::ZERO,
            docs,
        }
    }

    fn lengths_for(docs: &[(&str, Vec<usize>)]) -> EvalPassageLengths {
        EvalPassageLengths {
            docs: docs
                .iter()
                .map(|(path, lengths)| clawg_core::eval_api::EvalDocLengths { path: path.to_string(), lengths: lengths.clone() })
                .collect(),
            count_seconds: 0.0,
        }
    }

    fn block_of(run: &EvalIndexRun, lengths: &EvalPassageLengths) -> IndexBlock {
        IndexBlock::new(run, Some(lengths), None, EvalCpu::this(), 8, true, Some(Tokens::of(lengths, 0.0)))
    }

    #[test]
    fn area_of_a_path_is_decided_by_its_prefix_and_kind() {
        let cases = [
            ("openspec/changes/archive/2026-01-01-x/proposal.md", false, Area::Archive),
            ("docs/a.md", false, Area::Docs),
            ("Cargo.toml", false, Area::Docs),
            ("crates/x/src/lib.rs", true, Area::Code),
            ("openspec/changes/archive/x/script.rs", true, Area::Archive),
            ("openspec/changes/archived/x.md", false, Area::Docs),
            ("openspec/changes/archive.md", false, Area::Docs),
        ];

        for (path, code, area) in cases {
            assert_eq!(area_of(path, code), area, "{path}");
        }
    }

    #[test]
    fn areas_add_up_to_the_totals() {
        let run = run_of(vec![
            timing("docs/a.md", false, 3, [250, 500, 1000, 0, 3000, 750]),
            timing("crates/x/src/lib.rs", true, 2, [250, 250, 500, 250, 1000, 250]),
            timing("openspec/changes/archive/y/p.md", false, 4, [500, 250, 250, 250, 2000, 500]),
        ]);
        let lengths = lengths_for(&[
            ("docs/a.md", vec![10, 40, 25]),
            ("crates/x/src/lib.rs", vec![7, 9]),
            ("openspec/changes/archive/y/p.md", vec![5, 5, 80, 1]),
        ]);

        let block = block_of(&run, &lengths);
        let areas = [&block.by_area.docs, &block.by_area.code, &block.by_area.archive];
        let sum = |f: fn(&AreaBlock) -> f64| areas.iter().map(|a| f(a)).sum::<f64>();
        let tokens = block.tokens.as_ref().unwrap();

        assert_eq!(sum(|a| a.phases.read), block.phases.read);
        assert_eq!(sum(|a| a.phases.mask), block.phases.mask);
        assert_eq!(sum(|a| a.phases.split), block.phases.split);
        assert_eq!(sum(|a| a.phases.lookup), block.phases.lookup);
        assert_eq!(sum(|a| a.phases.embed), block.phases.embed);
        assert_eq!(sum(|a| a.phases.write), block.phases.write);
        assert_eq!(areas.iter().map(|a| a.passages).sum::<usize>(), run.report.passages);
        assert_eq!(areas.iter().map(|a| a.docs).sum::<usize>(), 3);
        assert_eq!(areas.iter().map(|a| a.tokens.unwrap().real).sum::<usize>(), tokens.real);
        assert_eq!(areas.iter().map(|a| a.tokens.unwrap().padded).sum::<usize>(), tokens.padded);
        assert_eq!((block.by_area.docs.docs, block.by_area.code.docs, block.by_area.archive.docs), (1, 1, 1));
    }

    #[test]
    fn the_removal_of_docs_is_in_the_total_write_and_in_no_area() {
        let mut run = run_of(vec![
            timing("docs/a.md", false, 1, [0, 0, 0, 0, 0, 750]),
            timing("crates/x/src/lib.rs", true, 1, [0, 0, 0, 0, 0, 250]),
        ]);
        run.remove = std::time::Duration::from_millis(400);
        let lengths = lengths_for(&[("docs/a.md", vec![3]), ("crates/x/src/lib.rs", vec![4])]);

        let block = block_of(&run, &lengths);
        let areas = [&block.by_area.docs, &block.by_area.code, &block.by_area.archive];
        let written: f64 = areas.iter().map(|a| a.phases.write).sum();

        assert!((written - 1.0).abs() < 1e-9, "{written}");
        assert!((block.phases.write - (written + 0.4)).abs() < 1e-9, "{}", block.phases.write);
    }

    #[test]
    fn only_docs_over_a_batch_count_as_multi_batch() {
        let run = run_of(vec![timing("docs/big.md", false, 20, [0; 6]), timing("docs/small.md", false, 3, [0; 6])]);
        let lengths = lengths_for(&[("docs/big.md", (0..20).map(|i| 10 + i).collect()), ("docs/small.md", vec![4, 5, 6])]);

        let docs = block_of(&run, &lengths).by_area.docs;
        let tokens = docs.tokens.unwrap();

        assert_eq!((docs.docs, docs.passages, docs.multi_batch_docs), (2, 23, 1));
        let big: Vec<usize> = (0..20).map(|i| 10 + i).collect();
        assert_eq!(tokens.multi_batch_real, big.iter().sum::<usize>());
        assert_eq!(tokens.multi_batch_padded, padded(&big));
        assert_eq!(tokens.real, big.iter().sum::<usize>() + 15);
    }

    #[test]
    fn the_areas_hold_no_path() {
        let run = run_of(vec![timing("secret/dir/a.md", false, 1, [0; 6])]);
        let lengths = lengths_for(&[("secret/dir/a.md", vec![3])]);

        let text = serde_json::to_string(&block_of(&run, &lengths).by_area).unwrap();

        assert!(!text.contains("secret") && !text.contains("a.md"), "{text}");
    }

    #[test]
    fn a_doc_without_lengths_counts_no_tokens() {
        let run = run_of(vec![timing("docs/a.md", false, 1, [0; 6])]);

        let block = IndexBlock::new(&run, None, None, EvalCpu::this(), 8, true, None);

        assert!(block.by_area.docs.tokens.is_none());
        assert_eq!(block.by_area.docs.passages, 1);
    }
}
