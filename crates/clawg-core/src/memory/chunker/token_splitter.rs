//! Token-cap splitting helper with configurable overlap.
//!
//! Knows NOTHING about chat speakers. `ChatChunker` feeds each speaker turn
//! through here. Decouples the cap policy from upstream semantic split.

use std::sync::Arc;

use super::Chunk;

/// Pure "split this text by token cap with overlap" helper.
pub(crate) struct TokenSplitter {
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
}

impl TokenSplitter {
    /// Real tokenizer-backed splitter (preferred, post-bootstrap).
    pub fn with_tokenizer(tokenizer: Arc<tokenizers::Tokenizer>) -> Self {
        Self { tokenizer: Some(tokenizer) }
    }

    /// Byte-length fallback when `tokenizer.json` is not yet on disk.
    /// 1 token ≈ 4 bytes for the languages we support.
    pub fn fallback() -> Self {
        Self { tokenizer: None }
    }

    /// Test-only: the raw address of the wrapped `Tokenizer`, or `None` in
    /// fallback mode. Lets a test prove two splitters share the identical
    /// `Arc` (one parse of `tokenizer.json`, reused) rather than each
    /// holding its own independently-loaded copy.
    #[cfg(test)]
    pub(crate) fn tokenizer_ptr(&self) -> Option<*const tokenizers::Tokenizer> {
        self.tokenizer.as_deref().map(std::ptr::from_ref)
    }

    /// Split `text` so each output `Chunk` has `<= max_tokens` tokens, with
    /// `overlap` tokens repeated between adjacent chunks. `overlap == 0`
    /// reproduces the pre-overlap behavior exactly. Preserves UTF-8 boundaries.
    pub fn split(&self, text: &str, max_tokens: usize, overlap: usize) -> Vec<Chunk> {
        if max_tokens == 0 || text.is_empty() {
            return vec![];
        }
        // overlap >= max would zero the stride and the window would never
        // advance; every caller of split() supplies its own overlap value
        // (mod.rs's constants are const-asserted, but nothing stops a test or
        // a future caller from passing something else), so the clamp lives
        // here too, not only at the constant definition.
        let overlap = overlap.min(max_tokens - 1);
        match &self.tokenizer {
            Some(tok) => self.split_by_tokens(text, max_tokens, overlap, tok),
            None => {
                let max_b = max_tokens.saturating_mul(4);
                // 4 bytes is the widest UTF-8 scalar; keeping the byte stride
                // at >= 4 guarantees it always crosses at least one char
                // boundary, so the window can never stall on multibyte input.
                let ov_b = overlap.saturating_mul(4).min(max_b.saturating_sub(4));
                self.split_by_bytes(text, max_b, ov_b)
            }
        }
    }

    fn split_by_tokens(
        &self,
        text: &str,
        max: usize,
        overlap: usize,
        tok: &tokenizers::Tokenizer,
    ) -> Vec<Chunk> {
        let enc = match tok.encode(text, false) {
            Ok(e) => e,
            Err(_) => {
                let max_b = max.saturating_mul(4);
                let ov_b = overlap.saturating_mul(4).min(max_b.saturating_sub(4));
                return self.split_by_bytes(text, max_b, ov_b);
            }
        };
        let offsets = enc.get_offsets();
        if offsets.is_empty() {
            return vec![];
        }
        plan_windows(offsets.len(), max, overlap)
            .into_iter()
            .map(|(start, end)| {
                let start_byte = offsets[start].0;
                let end_byte = offsets[end - 1].1;
                // Safe: tokenizers guarantees offsets land on char boundaries.
                let slice = &text[start_byte..end_byte];
                Chunk { text: slice.to_string(), token_count: end - start }
            })
            .collect()
    }

    fn split_by_bytes(&self, text: &str, max_b: usize, ov_b: usize) -> Vec<Chunk> {
        if max_b == 0 || text.is_empty() {
            return vec![];
        }
        let n = text.len();
        let stride_b = max_b - ov_b;
        let mut out = Vec::new();
        let mut s = 0usize;
        loop {
            let raw_e = s.saturating_add(max_b).min(n);
            // Floor to the last char boundary <= raw_e so the slice never
            // splits a multibyte char. Flooring (rather than ceiling) is also
            // what keeps the chunk at or under the cap.
            let mut e = if raw_e >= n { n } else { floor_char_boundary(text, raw_e) };
            if e <= s {
                // Only reachable when the cap is narrower than the character
                // starting at `s`. `split()` never produces that (it derives
                // `max_b` as `max_tokens * 4`, and 4 bytes is the widest
                // UTF-8 scalar), but `split_by_bytes` takes `max_b` directly,
                // so emit that character whole rather than an empty window.
                e = ceil_char_boundary(text, s + 1).min(n);
            }
            out.push(Chunk { text: text[s..e].to_string(), token_count: 0 });
            if e == n {
                break;
            }
            // Ceil (not floor) on the start: any advance rounds forward, so
            // `s` strictly increases every iteration and the loop terminates.
            // Capping it at `e` is what keeps the windows gap-free: `e` was
            // floored to a char boundary at or below `s + max_b`, so with a
            // small (or zero) overlap the ceil'd start can otherwise land
            // BEYOND that end and silently drop the character in between —
            // and, when the leftover is shorter than one character, land on
            // `text.len()` itself and push an empty chunk.
            s = ceil_char_boundary(text, s + stride_b).min(e);
        }
        out
    }
}

/// The windowing arithmetic shared by both the token path (units = tokens)
/// and the byte-fallback path (units = bytes, char-snapped by the caller).
/// Pure and tokenizer-free so it is unit-testable without loading a model.
///
/// Returns half-open `[start, end)` index pairs into the unit sequence.
/// `overlap` MUST already be `< max` (callers clamp before reaching here);
/// an unclamped `overlap >= max` collapses the stride to 0 and this function
/// would loop forever, which is exactly why `split()` never lets that happen.
fn plan_windows(n: usize, max: usize, overlap: usize) -> Vec<(usize, usize)> {
    if max == 0 || n == 0 {
        return vec![];
    }
    let overlap = overlap.min(max - 1);
    let stride = max - overlap;
    let mut out = Vec::new();
    let mut s = 0usize;
    loop {
        let e = (s + max).min(n);
        out.push((s, e));
        // Breaking the instant the window reaches `n` is what makes the tail
        // unreachable-by-construction: continuing with `while s < n` would
        // keep emitting windows whose range is fully inside the one just
        // pushed (e.g. n=1920, max=1920, overlap=256 emits [0,1920) then
        // [1664,1920) — the second is a strict subset of the first).
        if e == n {
            break;
        }
        s += stride;
    }
    out
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── plan_windows: pure arithmetic, no tokenizer needed ──────────────────

    #[test]
    fn plan_windows_empty_input_yields_no_windows() {
        assert_eq!(plan_windows(0, 480, 64), vec![]);
    }

    #[test]
    fn plan_windows_zero_max_yields_no_windows() {
        assert_eq!(plan_windows(100, 0, 0), vec![]);
    }

    #[test]
    fn plan_windows_input_at_or_under_cap_yields_one_window() {
        assert_eq!(plan_windows(1, 480, 64), vec![(0, 1)]);
        assert_eq!(plan_windows(480, 480, 64), vec![(0, 480)]);
    }

    #[test]
    fn plan_windows_count_formula_three_cases() {
        // n <= overlap would give a negative/zero numerator under the naive
        // ceil((n-ov)/stride) formula — but n <= max here still yields count 1.
        assert_eq!(plan_windows(30, 480, 64).len(), 1); // n <= max
        // n > max: ceil((n - ov) / stride)
        let n = 5760usize; // matches the worker fixture (a*1920+b*1920+c*1920)
        let max = 1920;
        let ov = 256;
        let stride = max - ov;
        let expected = ((n - ov) as f64 / stride as f64).ceil() as usize;
        assert_eq!(plan_windows(n, max, ov).len(), expected);
        assert_eq!(expected, 4);
    }

    #[test]
    fn plan_windows_terminates_and_every_window_is_bounded() {
        for max in 1..40usize {
            for overlap in 0..max {
                for n in 0..600usize {
                    let windows = plan_windows(n, max, overlap);
                    // Termination: bounded number of windows for bounded n/stride.
                    assert!(windows.len() <= n + 1, "max={max} overlap={overlap} n={n}");
                    for &(s, e) in &windows {
                        assert!(e > s, "empty window at max={max} overlap={overlap} n={n}");
                        assert!(e - s <= max, "window exceeds max at max={max} overlap={overlap} n={n}");
                        assert!(e <= n, "window exceeds n at max={max} overlap={overlap} n={n}");
                    }
                }
            }
        }
    }

    #[test]
    fn plan_windows_no_window_range_is_contained_in_another() {
        // `s` strictly increases every iteration, so starts are strictly
        // ascending and `e = min(s + max, n)` is therefore non-decreasing —
        // which means containment between ANY pair (not just neighbours)
        // can only exist if two ADJACENT windows already share the same end
        // (a run of equal ends is what makes a later, narrower window sit
        // inside an earlier one; a strict increase anywhere in the middle
        // rules out containment on either side of it). Checking neighbours
        // is therefore equivalent to the full O(w^2) pairwise check for this
        // shape of sequence, and turns the exhaustive sweep from
        // quadratic-in-window-count into linear.
        for max in 1..40usize {
            for overlap in 0..max {
                for n in 0..600usize {
                    let windows = plan_windows(n, max, overlap);
                    for pair in windows.windows(2) {
                        let (s0, e0) = pair[0];
                        let (s1, e1) = pair[1];
                        assert!(s1 > s0, "starts must strictly increase at max={max} overlap={overlap} n={n}");
                        assert!(
                            e1 != e0,
                            "adjacent windows share an end (later window contained in earlier one) at max={max} overlap={overlap} n={n}: {pair:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn plan_windows_adjacent_pairs_share_exactly_overlap_units() {
        for max in 2..40usize {
            for overlap in 0..max {
                for n in (max + 1)..600usize {
                    let windows = plan_windows(n, max, overlap);
                    for pair in windows.windows(2) {
                        let (_, e0) = pair[0];
                        let (s1, _) = pair[1];
                        // shared span is [s1, e0) when e0 > s1
                        let shared = e0.saturating_sub(s1);
                        assert_eq!(shared, overlap, "max={max} overlap={overlap} n={n} pair={pair:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn plan_windows_window_length_equals_end_minus_start_for_every_window() {
        // Pins the exact quantity `split_by_tokens` reads off as `token_count`
        // (`end - start`) so an off-by-one there cannot silently drift from
        // what the windowing arithmetic actually produced.
        for &(max, overlap, n) in &[(480, 64, 6000usize), (1920, 256, 5760), (4, 0, 20), (1, 0, 5)] {
            let windows = plan_windows(n, max, overlap);
            for &(s, e) in &windows {
                assert!(e - s >= 1, "window length must be >= 1 at max={max} overlap={overlap} n={n}");
                assert!(e - s <= max, "window length must be <= max at max={max} overlap={overlap} n={n}");
            }
        }
    }

    #[test]
    fn plan_windows_non_final_windows_are_exactly_max_wide() {
        let windows = plan_windows(5760, 1920, 256);
        for &(s, e) in &windows[..windows.len() - 1] {
            assert_eq!(e - s, 1920, "non-final window must be exactly max wide");
        }
    }

    #[test]
    fn plan_windows_clamps_overlap_and_never_hangs() {
        // overlap >= max: the clamp must pull it to `max - 1` so the stride
        // stays >= 1. Asserting only "non-empty, every window non-degenerate"
        // does NOT test the clamp: a stride of 0 re-emits `(0, max)` forever,
        // and every such window is non-empty too. The observable consequence
        // of the clamp is that the starts ADVANCE — so pin the exact plan.
        let clamped = plan_windows(1000, 4, 4);
        assert_eq!(
            clamped,
            plan_windows(1000, 4, 3),
            "overlap == max must behave exactly as overlap == max - 1"
        );
        assert_eq!(clamped.len(), 997, "stride 1 over n=1000, max=4 gives 997 windows");
        assert_eq!(clamped[0], (0, 4));
        assert_eq!(*clamped.last().unwrap(), (996, 1000));
        for pair in clamped.windows(2) {
            assert!(pair[1].0 > pair[0].0, "clamped stride must still advance the start");
        }

        // Far out of contract, not merely one over.
        let far = plan_windows(100, 5, 50);
        assert_eq!(far, plan_windows(100, 5, 4), "any overlap >= max clamps to max - 1");
    }

    #[test]
    fn plan_windows_max_one_yields_one_unit_per_window() {
        let windows = plan_windows(5, 1, 0);
        assert_eq!(windows, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]);
    }

    // ── TokenSplitter (byte-fallback path — the only path exercised here;
    //    the tokenizer path has no bundled model to test against, see the
    //    ASCII parity test below for how its arithmetic is still covered) ──

    #[test]
    fn split_short_input_single_chunk_identical_to_input() {
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split("hello world", 480, 64);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
    }

    #[test]
    fn split_exactly_cap_size_input_is_one_chunk_byte_identical() {
        let splitter = TokenSplitter::fallback();
        let content = "x".repeat(1920);
        let chunks = splitter.split(&content, 480, 64);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, content);
    }

    #[test]
    fn split_adjacent_chunks_share_exactly_overlap_bytes() {
        let splitter = TokenSplitter::fallback();
        let content = format!("{}{}{}", "a".repeat(1920), "b".repeat(1920), "c".repeat(1920));
        let chunks = splitter.split(&content, 480, 64);
        assert_eq!(chunks.len(), 4, "5760 bytes at max=1920/ov=256 must yield 4 chunks");
        for pair in chunks.windows(2) {
            let tail = &pair[0].text[pair[0].text.len() - 256..];
            let head = &pair[1].text[..256];
            assert_eq!(tail, head, "adjacent chunks must share exactly 256 overlap bytes");
        }
    }

    #[test]
    fn split_chunks_reconstruct_input_when_overlap_removed() {
        let splitter = TokenSplitter::fallback();
        let content = format!("{}{}{}", "a".repeat(1920), "b".repeat(1920), "c".repeat(1920));
        let chunks = splitter.split(&content, 480, 64);
        let mut reconstructed = chunks[0].text.clone();
        for c in &chunks[1..] {
            reconstructed.push_str(&c.text[256..]);
        }
        assert_eq!(reconstructed, content, "removing each overlap must reconstruct the original input exactly");
    }

    #[test]
    fn split_chunk_count_matches_the_formula_for_every_pinned_fixture() {
        // count = 1 for 0 < n <= max, else ceil((n - overlap) / stride), with
        // max_b = 1920, overlap_b = 256, stride_b = 1664. These are the input
        // sizes the rest of the suite and the worker fixtures are built on;
        // pin the count outright instead of leaving it implied by a `>= 2`.
        let splitter = TokenSplitter::fallback();
        for (n, expected) in [(11usize, 1usize), (1920, 1), (3584, 2), (3840, 3), (5760, 4), (6000, 4)] {
            let content = "x".repeat(n);
            assert_eq!(
                splitter.split(&content, 480, 64).len(),
                expected,
                "n={n} bytes must yield {expected} chunks"
            );
        }
    }

    #[test]
    fn split_boundary_straddling_span_survives_whole_in_one_chunk() {
        // Old 0-overlap cut points at multiples of 1920. Place a unique
        // marker straddling the first cut point (1920) and confirm it
        // survives whole inside at least one chunk under the new overlap.
        let mut content = "a".repeat(1900);
        content.push_str("MARKER_SPANNING_BOUNDARY");
        content.push_str(&"b".repeat(2000));
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&content, 480, 64);
        assert!(
            chunks.iter().any(|c| c.text.contains("MARKER_SPANNING_BOUNDARY")),
            "marker straddling the old cut point must be whole in >= 1 chunk"
        );
    }

    #[test]
    fn split_overlap_zero_reproduces_pre_overlap_behavior_byte_identical() {
        let splitter = TokenSplitter::fallback();
        let content = format!("{}{}{}", "a".repeat(1920), "b".repeat(1920), "c".repeat(1920));
        let chunks = splitter.split(&content, 480, 0);
        assert_eq!(chunks.len(), 3, "overlap=0 must restore the pre-overlap chunk count");
        let concatenated: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(concatenated, content, "overlap=0 concatenation must equal the original input, no gaps or overlaps");
    }

    #[test]
    fn split_no_chunk_range_contained_in_another_over_a_unique_window_fixture() {
        // 350 unique 10-byte tokens ("000000000 ".."000000349 ") = 3500 bytes,
        // long enough that a residual tail lands strictly inside (stride, max].
        let content: String = (0..350).map(|i| format!("{i:09} ")).collect();
        assert_eq!(content.len(), 3500);
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&content, 480, 64);
        for i in 0..chunks.len() {
            for j in 0..chunks.len() {
                if i == j {
                    continue;
                }
                assert!(
                    !chunks[j].text.contains(&chunks[i].text),
                    "chunk {i} is contained in chunk {j} — degenerate tail not prevented"
                );
            }
        }
    }

    #[test]
    fn split_multibyte_input_never_panics_and_respects_cap() {
        let cjk = "文".repeat(2000); // 3 bytes/char in UTF-8
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&cjk, 480, 64);
        assert!(!chunks.is_empty());
        for c in &chunks {
            assert!(c.text.len() <= 1920, "chunk byte length {} exceeds cap 1920", c.text.len());
            assert!(c.text.is_char_boundary(0) && c.text.is_char_boundary(c.text.len()));
        }
    }

    #[test]
    fn split_emoji_input_never_panics_and_respects_cap() {
        let emoji = "🦀".repeat(1000); // 4 bytes/char, the widest UTF-8 scalar
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&emoji, 480, 64);
        assert!(!chunks.is_empty());
        for c in &chunks {
            assert!(c.text.len() <= 1920, "chunk byte length {} exceeds cap 1920", c.text.len());
        }
    }

    #[test]
    fn split_oversized_single_char_is_emitted_whole_not_looped() {
        // max_tokens=1 -> max_b=4, exactly one emoji's width; must not loop.
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split("🦀🦀", 1, 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "🦀");
        assert_eq!(chunks[1].text, "🦀");
    }

    #[test]
    fn split_out_of_contract_overlap_never_hangs_and_every_chunk_nonempty() {
        let splitter = TokenSplitter::fallback();
        let content = "x".repeat(5000);
        let chunks = splitter.split(&content, 4, 4); // overlap == max
        assert!(!chunks.is_empty());
        for c in &chunks {
            assert!(!c.text.is_empty());
        }
    }

    #[test]
    fn split_never_emits_an_empty_chunk_when_the_stride_rounds_past_the_tail() {
        // With overlap 0 (what the rollback lever sets) the stride equals the
        // cap, so the next start is `ceil_char_boundary(s + max_b)`. When the
        // bytes left over after the cap are fewer than one whole character,
        // that ceil rounds the start onto `text.len()` itself and the window
        // that follows spans nothing. An empty chunk is not a harmless
        // artefact: `worker.rs` only checks that the chunk VEC is non-empty,
        // so an empty chunk is hashed, embedded and inserted as a `memory`
        // row holding no content but a real vector.
        let splitter = TokenSplitter::fallback();
        for pad in 1915..=1925usize {
            let mut content = "a".repeat(pad);
            content.push('\u{1F980}'); // 4-byte scalar straddling the cap residue
            let chunks = splitter.split(&content, 480, 0);
            let lens: Vec<usize> = chunks.iter().map(|c| c.text.len()).collect();
            assert!(
                chunks.iter().all(|c| !c.text.is_empty()),
                "empty chunk emitted for pad={pad} (n={}): lens={lens:?}",
                content.len()
            );
        }
        // Minimal reproduction: cap of one token (4 bytes) against a 1-byte
        // char followed by a 4-byte one.
        let chunks = splitter.split("a\u{1F980}", 1, 0);
        assert!(
            chunks.iter().all(|c| !c.text.is_empty()),
            "empty chunk emitted at the minimal cap: {:?}",
            chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_concatenated_chunks_still_cover_every_byte_of_multibyte_input() {
        // Companion to the test above: dropping the trailing window entirely
        // would also make "no empty chunk" true, so pin that the union of the
        // windows still reaches the end of the input.
        let splitter = TokenSplitter::fallback();
        for pad in 1915..=1925usize {
            let mut content = "a".repeat(pad);
            content.push('\u{1F980}');
            let chunks = splitter.split(&content, 480, 0);
            let last = chunks.last().expect("non-empty input must yield >= 1 chunk");
            assert!(
                content.ends_with(last.text.as_str()),
                "last chunk must reach the end of the input for pad={pad}"
            );
            let covered: usize = chunks.iter().map(|c| c.text.len()).sum();
            assert!(
                covered >= content.len(),
                "windows lost {} bytes for pad={pad}",
                content.len() - covered
            );
        }
    }

    #[test]
    fn split_by_bytes_emits_an_oversized_char_whole_when_the_cap_is_narrower_than_it() {
        // The `e <= s` branch inside `split_by_bytes` is only reachable when
        // the cap is narrower than the character starting at `s` — which
        // `split()` itself can never produce (it derives `max_b` as
        // `max_tokens * 4`, and 4 bytes is the widest UTF-8 scalar). Drive
        // `split_by_bytes` directly so the branch is exercised rather than
        // carried as unreachable code.
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split_by_bytes("\u{1F980}\u{1F980}", 2, 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "\u{1F980}");
        assert_eq!(chunks[1].text, "\u{1F980}");
    }

    #[test]
    fn split_empty_content_returns_empty_vec() {
        let splitter = TokenSplitter::fallback();
        assert!(splitter.split("", 480, 64).is_empty());
    }

    // ── Multibyte: the char-boundary snapping itself ────────────────────────

    /// `n` distinct 3-byte CJK scalars after `prefix`, so no two positions in
    /// the fixture look alike. The repeated-character fixtures elsewhere in
    /// this file cannot express an offset property: any window of `"文"*k`
    /// equals any other window of the same length, so a boundary that moved
    /// is indistinguishable from one that did not.
    fn unique_cjk(prefix: &str, count: u32) -> String {
        let mut out = String::from(prefix);
        for i in 0..count {
            out.push(char::from_u32(0x4E00 + i).expect("CJK block is dense here"));
        }
        out
    }

    /// Bytes shared between two adjacent chunks, measured as the longest
    /// suffix of `a` that is a prefix of `b`. Unambiguous only because the
    /// fixture's characters are all distinct.
    fn shared_bytes(a: &str, b: &str) -> usize {
        (1..=a.len().min(b.len()))
            .rev()
            .find(|&k| b.is_char_boundary(k) && a.as_bytes().ends_with(&b.as_bytes()[..k]))
            .unwrap_or(0)
    }

    #[test]
    fn split_multibyte_unaligned_to_the_cap_snaps_the_window_end_to_a_char_boundary() {
        // 1920 is a multiple of BOTH 3 and 4, so a fixture built from a
        // single repeated CJK (3 B) or emoji (4 B) scalar puts every window
        // edge on a char boundary already and exercises no snapping at all.
        // One ASCII byte in front shifts every boundary off that alignment,
        // and the cap then lands strictly INSIDE a scalar — which is the only
        // arrangement under which flooring the end is load-bearing (slicing
        // at the raw index there panics).
        let content = unique_cjk("a", 2200);
        assert!(!content.is_char_boundary(1920), "fixture must not be cap-aligned");

        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&content, 480, 64);
        assert!(chunks.len() >= 3, "expected >= 3 chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.text.is_empty());
            assert!(c.text.len() <= 1920, "chunk byte length {} exceeds cap 1920", c.text.len());
        }

        // No byte of the input is lost: dropping each shared prefix in turn
        // must rebuild the original exactly.
        let mut reconstructed = chunks[0].text.clone();
        for pair in chunks.windows(2) {
            let shared = shared_bytes(&pair[0].text, &pair[1].text);
            reconstructed.push_str(&pair[1].text[shared..]);
        }
        assert_eq!(reconstructed, content, "char-snapped windows must still cover every byte");
    }

    #[test]
    fn split_multibyte_adjacent_chunks_share_between_overlap_minus_six_and_overlap_bytes() {
        // On ASCII the shared span is exactly `overlap * 4` bytes. On
        // multibyte input it is not: the end is floored DOWN by up to 3 bytes
        // and the next start is ceiled UP by up to 3, so the shared span can
        // lose up to 6 bytes — never more, and never any at all on the upper
        // side. `overlap * 4 - 3` is NOT the lower bound: the two roundings
        // are independent and both land on the same pair.
        let splitter = TokenSplitter::fallback();
        const OV_B: usize = 64 * 4;
        for prefix in ["", "a", "ab", "abc"] {
            let content = unique_cjk(prefix, 2200);
            let chunks = splitter.split(&content, 480, 64);
            for pair in chunks.windows(2) {
                let shared = shared_bytes(&pair[0].text, &pair[1].text);
                assert!(
                    (OV_B - 6..=OV_B).contains(&shared),
                    "prefix={prefix:?}: shared {shared} outside [{}, {OV_B}]",
                    OV_B - 6
                );
            }
        }

        // The lower bound is tight, not conservative: a 4-byte scalar placed
        // so that the end floors back 3 bytes AND another placed so the next
        // start ceils forward 3 bytes reaches exactly `overlap * 4 - 6`.
        let mut worst = "a".repeat(1663);
        worst.push('\u{1F980}'); // occupies 1663..1667 -> ceil(1664) = 1667
        worst.push_str(&"a".repeat(250));
        worst.push('\u{1F980}'); // occupies 1917..1921 -> floor(1920) = 1917
        worst.push_str(&unique_cjk("", 1000));
        let chunks = splitter.split(&worst, 480, 64);
        assert_eq!(
            shared_bytes(&chunks[0].text, &chunks[1].text),
            OV_B - 6,
            "the worst-case arrangement must actually reach the lower bound"
        );
    }

    #[test]
    fn split_multibyte_with_zero_overlap_still_loses_no_bytes() {
        // overlap = 0 is what the rollback lever sets. The stride then equals
        // the cap, so the next start is the CEIL of the same index the window
        // end was FLOORED to — the two round in opposite directions and the
        // character between them is the one at risk of falling through the gap.
        let splitter = TokenSplitter::fallback();
        for prefix in ["", "a", "ab", "abc"] {
            let content = unique_cjk(prefix, 2200);
            let chunks = splitter.split(&content, 480, 0);
            assert!(chunks.len() >= 3);
            let concatenated: String = chunks.iter().map(|c| c.text.as_str()).collect();
            assert_eq!(
                concatenated, content,
                "prefix={prefix:?}: zero-overlap windows must tile the input with no gaps"
            );
        }
    }

    // ── Token path: the branch production actually runs ──────────────────────

    /// A real `Tokenizer` built entirely in memory — a `WordLevel` model over
    /// a whitespace pre-tokenizer, so one whitespace-delimited word is one
    /// token and a token's offsets are that word's byte range. No model
    /// download, no `tokenizer.json` on disk, no 470 MB artefact: enough to
    /// drive `split_by_tokens`, which every other test in this repo reaches
    /// only through the byte fallback.
    fn word_level_tokenizer(words: &[String]) -> Arc<tokenizers::Tokenizer> {
        let vocab: String = words
            .iter()
            .enumerate()
            .map(|(i, w)| format!("\"{w}\":{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],
                "normalizer":null,"pre_tokenizer":{{"type":"Whitespace"}},
                "post_processor":null,"decoder":null,
                "model":{{"type":"WordLevel","vocab":{{"[UNK]":0,{vocab}}},"unk_token":"[UNK]"}}}}"#
        );
        Arc::new(json.parse::<tokenizers::Tokenizer>().expect("fixture tokenizer must parse"))
    }

    /// 300 distinct words (`w000 w001 ...`), one token each under the fixture
    /// tokenizer. Distinct on purpose: with a repeated word every window would
    /// stringify identically and no offset property would be observable.
    fn word_fixture(count: usize) -> (String, Vec<String>) {
        let words: Vec<String> = (0..count).map(|i| format!("w{i:03}")).collect();
        (words.join(" "), words)
    }

    #[test]
    fn token_path_splits_on_token_windows_with_the_requested_overlap() {
        let (text, words) = word_fixture(300);
        let tok = word_level_tokenizer(&words);
        let splitter = TokenSplitter::with_tokenizer(tok);

        let (max, overlap) = (100usize, 20usize);
        let chunks = splitter.split(&text, max, overlap);
        let expected = plan_windows(words.len(), max, overlap);
        assert_eq!(chunks.len(), expected.len(), "token path must follow plan_windows");
        assert!(chunks.len() >= 3, "fixture must yield >= 3 chunks, got {}", chunks.len());

        for (chunk, &(s, e)) in chunks.iter().zip(expected.iter()) {
            // Exact text: pins BOTH offset lookups (start uses the token's
            // start offset, end uses the last token's END offset) — reading
            // the wrong member of either pair changes this string.
            assert_eq!(
                chunk.text,
                words[s..e].join(" "),
                "chunk for window ({s},{e}) does not carry exactly those tokens"
            );
            assert_eq!(chunk.token_count, e - s, "token_count must be the window length");
        }
    }

    #[test]
    fn token_path_adjacent_chunks_share_exactly_overlap_tokens() {
        let (text, words) = word_fixture(300);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        let (max, overlap) = (100usize, 20usize);
        let chunks = splitter.split(&text, max, overlap);

        for pair in chunks.windows(2) {
            let tail: Vec<&str> = pair[0].text.split_whitespace().rev().take(overlap).collect();
            let head: Vec<&str> = pair[1].text.split_whitespace().take(overlap).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            assert_eq!(
                tail, head,
                "adjacent token-path chunks must share exactly {overlap} tokens"
            );
        }
    }

    #[test]
    fn token_path_boundary_straddling_span_survives_whole_in_one_chunk() {
        // The feature's actual claim, on the path production runs: a span
        // shorter than the stride that crosses a zero-overlap cut point must
        // still be whole inside at least one chunk.
        let (text, words) = word_fixture(300);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        let (max, overlap) = (100usize, 20usize);

        // Tokens 98..102 straddle the old hard cut at token 100.
        let straddling = words[98..103].join(" ");
        let chunks = splitter.split(&text, max, overlap);
        assert!(
            chunks.iter().any(|c| c.text.contains(&straddling)),
            "span crossing the old cut point must be whole in >= 1 chunk"
        );
        // Control: with overlap 0 it is NOT, which is why the overlap exists.
        let cut = splitter.split(&text, max, 0);
        assert!(
            !cut.iter().any(|c| c.text.contains(&straddling)),
            "control: with zero overlap the same span must be split across chunks"
        );
    }

    #[test]
    fn token_path_with_zero_overlap_reconstructs_the_input() {
        let (text, words) = word_fixture(300);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        let chunks = splitter.split(&text, 100, 0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" "), text);
    }

    #[test]
    fn token_path_input_under_the_cap_is_one_chunk_holding_the_whole_text() {
        let (text, words) = word_fixture(10);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        let chunks = splitter.split(&text, 100, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, text);
        assert_eq!(chunks[0].token_count, 10);
    }

    #[test]
    fn token_path_text_that_tokenizes_to_nothing_yields_no_chunks() {
        let (_, words) = word_fixture(10);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        assert!(splitter.split("   ", 100, 20).is_empty());
    }

    #[test]
    fn split_ascii_byte_path_matches_plan_windows_shape() {
        // Parity: for ASCII input, the byte-fallback path's chunk boundaries
        // must land on exactly the same offsets plan_windows would compute
        // over raw bytes — this is the coverage substitute for the untested
        // tokenizer path, which shares the same plan_windows call.
        let content = "y".repeat(4000);
        let max_b = 1920;
        let ov_b = 256;
        let expected = plan_windows(content.len(), max_b, ov_b);
        let splitter = TokenSplitter::fallback();
        let chunks = splitter.split(&content, 480, 64);
        assert_eq!(chunks.len(), expected.len());
        for (chunk, &(s, e)) in chunks.iter().zip(expected.iter()) {
            assert_eq!(chunk.text.len(), e - s);
        }
    }
}
