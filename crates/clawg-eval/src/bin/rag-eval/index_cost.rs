//! What indexing a project's docs cost: the `index` block of a docs results
//! file and the lines of the report that read it.

use clawg_core::eval_api::{longest_first, EvalCpu, EvalIndexRun, EvalMemoReport, EvalPassageLengths, EMBED_BATCH};
use serde::Serialize;

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
}

impl IndexBlock {
    pub fn new(
        run: &EvalIndexRun,
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
}
