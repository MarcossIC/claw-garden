//! Token-cap splitting helper with configurable overlap.
//!
//! Knows NOTHING about chat speakers or Markdown. `ChatChunker` feeds each
//! speaker turn through here, and the documentation index each section too long
//! for one passage. Decouples the cap policy from upstream semantic split.

use std::sync::Arc;

use super::Chunk;
use crate::memory::pieces::MIN_OVERLAP_CHARS;

/// Pure "split this text by token cap with overlap" helper.
#[derive(Clone)]
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

    /// How many tokens `text` is, by the same measure [`TokenSplitter::split`]
    /// caps at: the tokenizer's count, or a quarter of the bytes (rounded up)
    /// in fallback mode.
    pub fn count(&self, text: &str) -> usize {
        match self.tokenizer.as_deref().map(|tok| tok.encode(text, false)) {
            Some(Ok(enc)) => enc.get_offsets().len(),
            _ => text.len().div_ceil(4),
        }
    }

    /// Split `text` so each output `Chunk` has `<= max_tokens` tokens, with at
    /// least `overlap` tokens repeated between adjacent chunks; `overlap == 0`
    /// tiles the text. With a tokenizer, every cut falls where a word starts
    /// when one is in reach ([`plan_word_windows`]). Preserves UTF-8 boundaries.
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
        plan_word_windows(&word_starts(text, offsets), &char_spans(text, offsets), max, overlap)
            .into_iter()
            .map(|(start, end)| {
                let start_byte = offsets[start].0;
                let end_byte = offsets[end - 1].1;
                // Safe: tokenizers guarantees offsets land on char boundaries.
                let slice = &text[start_byte..end_byte];
                Chunk { text: slice.to_string() }
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
            out.push(Chunk { text: text[s..e].to_string() });
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

/// Which tokens open a word of `text`: a cut just before token `i` leaves no
/// word in two when `i` is the first token, or when whitespace comes right
/// before its offset or right at it. The second rule is for a tokenizer
/// whose first token of a word holds the space before it (`▁foo` over
/// `" foo"`), the first for one that leaves it out. A token of whitespace
/// alone, as a lone `▁` is, opens the word that follows and the token after
/// it does not: the tokenizer puts the `▁` back when it counts a piece cut
/// after it, one token over what the window held. Signs are not boundaries,
/// so a path or an identifier is never cut at a `/` or a `::`.
fn word_starts(text: &str, offsets: &[(usize, usize)]) -> Vec<bool> {
    let blank = |(start, end): (usize, usize)| text.get(start..end).is_some_and(|t| t.chars().all(char::is_whitespace));
    offsets
        .iter()
        .enumerate()
        .map(|(i, &(start, _))| {
            i == 0
                || (!blank(offsets[i - 1])
                    && (text.get(..start).and_then(|t| t.chars().next_back()).is_some_and(char::is_whitespace)
                        || text.get(start..).and_then(|t| t.chars().next()).is_some_and(char::is_whitespace)))
        })
        .collect()
}

/// The characters of `text` before the start and before the end of each
/// token: what `join_pieces` counts when it looks for an overlap.
fn char_spans(text: &str, offsets: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut before = vec![0usize; text.len() + 1];
    let mut count = 0;
    for (byte, _) in text.char_indices() {
        before[byte] = count;
        count += 1;
    }
    before[text.len()] = count;
    let at = |byte: usize| before.get(byte).copied().unwrap_or(count);
    offsets.iter().map(|&(start, end)| (at(start), at(end))).collect()
}

/// The windows of at most `max` tokens a text is cut into, as half-open
/// `[start, end)` token ranges, given which tokens open a word and the
/// characters before each ([`char_spans`]). A window ends at the last word
/// start within `max` of its start, and the next one starts where
/// [`next_start`] says. Where no word start is in reach, as in a word longer
/// than the window or a text without spaces, the cut falls on the token. With
/// every token a word start the windows advance by exactly `max - overlap`.
/// Pure and tokenizer-free.
fn plan_word_windows(starts: &[bool], chars: &[(usize, usize)], max: usize, overlap: usize) -> Vec<(usize, usize)> {
    let n = starts.len();
    if max == 0 || n == 0 {
        return vec![];
    }
    // Keeps every end more than `overlap` past its start, which is what makes
    // the next start advance; `split()` clamps too, but nothing stops another
    // caller from passing a larger overlap.
    let overlap = overlap.min(max - 1);
    let last_start_in = |from: usize, to: usize| (from..=to).rev().find(|&i| starts[i]);
    let mut out = Vec::new();
    let (mut start, mut previous_end) = (0usize, 0usize);
    loop {
        // Stopping once a window reaches the end is what keeps a tail window
        // from lying inside the one before it.
        if start + max >= n {
            out.push((start, n));
            return out;
        }
        let cap = start + max;
        // Past the overlap and past the end before it, so no window but the
        // last is shorter than the overlap or lies inside the one before.
        let end = last_start_in((start + overlap).max(previous_end) + 1, cap).unwrap_or(cap);
        out.push((start, end));
        previous_end = end;
        start = next_start(starts, chars, start, end, overlap);
    }
}

/// Where the window after `[start, end)` starts: at the word start nearest
/// the token that leaves `overlap` shared, the one before it on a tie, so the
/// overlap stays about `overlap` on average. Always taking the word start
/// before it would add rows wherever a long word (a path, an identifier)
/// crosses that token. When the text it shares holds fewer than
/// [`MIN_OVERLAP_CHARS`], `join_pieces` would find no overlap, so it moves
/// back to the last word start that keeps them. Without a word start to move
/// to, it falls on the token. With no overlap asked for there is no floor,
/// and the windows tile the text.
fn next_start(starts: &[bool], chars: &[(usize, usize)], start: usize, end: usize, overlap: usize) -> usize {
    let target = end - overlap;
    let before = (start + 1..=target).rev().find(|&i| starts[i]);
    if overlap == 0 {
        return before.unwrap_or(target);
    }
    let after = (target..end).find(|&i| starts[i]);
    let nearest = match (before, after) {
        (Some(b), Some(a)) if a - target < target - b => a,
        (Some(b), _) => b,
        (None, Some(a)) => a,
        (None, None) => return target,
    };
    let joins = |i: usize| chars[end - 1].1 - chars[i].0 >= MIN_OVERLAP_CHARS;
    if joins(nearest) {
        return nearest;
    }
    (start + 1..nearest).rev().find(|&i| starts[i] && joins(i)).or(before).unwrap_or(target)
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
pub(crate) mod tests {
    use super::*;

    // ── word_starts and plan_word_windows: pure, no tokenizer needed ────────

    #[test]
    fn word_starts_marks_tokens_after_or_on_whitespace() {
        // `retri ##evi ##es`, `are` and `found`, each offset without the space.
        let text = "retrievies are\nfound";
        assert_eq!(word_starts(text, &[(0, 5), (5, 8), (8, 10), (11, 14), (15, 20)]), [true, false, false, true, true]);
        // Signs are not boundaries: a path and an identifier stay whole.
        let text = "crates/clawg-core Store::insert";
        let offsets = [(0, 6), (6, 7), (7, 17), (18, 23), (23, 25), (25, 31)];
        assert_eq!(word_starts(text, &offsets), [true, false, false, true, false, false]);
    }

    #[test]
    fn word_starts_takes_an_offset_that_holds_the_leading_space() {
        // `▁are` and `▁found` over " are" and " found".
        let text = "retrievies are found";
        assert_eq!(word_starts(text, &[(0, 5), (5, 10), (10, 14), (14, 20)]), [true, false, true, true]);
        // A lone `▁` over the space, or with an empty offset, opens the word
        // and the token after it does not: a piece cut after the `▁` would
        // gain it back when the tokenizer counts the piece alone.
        assert_eq!(word_starts(text, &[(0, 10), (10, 11), (11, 14), (14, 15), (15, 20)]), [true, true, false, true, false]);
        assert_eq!(word_starts(text, &[(0, 10), (11, 11), (11, 14)]), [true, true, false]);
    }

    #[test]
    fn char_spans_count_the_characters_before_each_token() {
        // `ñ` and `文` are two and three bytes, one character each.
        let text = "año 文 x";
        assert_eq!(char_spans(text, &[(0, 4), (5, 8), (9, 10)]), [(0, 3), (4, 5), (6, 7)]);
    }

    /// The characters `char_spans` gives `n` tokens of `width` each.
    fn even_chars(n: usize, width: usize) -> Vec<(usize, usize)> {
        (0..n).map(|i| (i * width, (i + 1) * width)).collect()
    }

    /// The plan of `starts` with every token so wide that one shared token
    /// already holds what `join_pieces` looks for.
    fn wide(starts: &[bool], max: usize, overlap: usize) -> Vec<(usize, usize)> {
        plan_word_windows(starts, &even_chars(starts.len(), MIN_OVERLAP_CHARS), max, overlap)
    }

    /// The plan of `n` tokens that each open a word.
    fn every_word(n: usize, max: usize, overlap: usize) -> Vec<(usize, usize)> {
        wide(&vec![true; n], max, overlap)
    }

    #[test]
    fn plan_word_windows_with_every_start_marked_keeps_a_fixed_stride() {
        let table: [(usize, usize, usize, &[(usize, usize)]); 7] = [
            (0, 480, 64, &[]),
            (1, 480, 64, &[(0, 1)]),
            (480, 480, 64, &[(0, 480)]),
            (300, 100, 20, &[(0, 100), (80, 180), (160, 260), (240, 300)]),
            (5760, 1920, 256, &[(0, 1920), (1664, 3584), (3328, 5248), (4992, 5760)]),
            (5, 1, 0, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]),
            (8, 4, 4, &[(0, 4), (1, 5), (2, 6), (3, 7), (4, 8)]),
        ];
        for (n, max, overlap, expected) in table {
            assert_eq!(every_word(n, max, overlap), expected, "n={n} max={max} overlap={overlap}");
        }
        assert!(every_word(100, 0, 0).is_empty(), "a cap of zero cuts nothing");

        // Every window starts one stride after the one before and is `max`
        // wide but for the last, which reaches the end: the count is 1 up to
        // the cap and ceil((n - overlap) / stride) past it.
        for max in 1..40usize {
            for overlap in 0..max {
                let stride = max - overlap;
                for n in 0..300usize {
                    let windows = every_word(n, max, overlap);
                    let count = match n {
                        0 => 0,
                        n if n <= max => 1,
                        n => (n - overlap).div_ceil(stride),
                    };
                    assert_eq!(windows.len(), count, "n={n} max={max} overlap={overlap}");
                    for (k, &(s, e)) in windows.iter().enumerate() {
                        assert_eq!((s, e), (k * stride, (k * stride + max).min(n)), "n={n} max={max} overlap={overlap}");
                    }
                }
            }
        }
    }

    #[test]
    fn plan_word_windows_clamps_the_overlap_and_never_hangs() {
        // A stride of 0 would re-emit `(0, max)` forever, every window of it
        // non-empty: what the clamp shows is that the starts advance.
        assert_eq!(every_word(1000, 4, 4), every_word(1000, 4, 3), "overlap == max behaves as max - 1");
        assert_eq!(every_word(1000, 4, 4).len(), 997);
        let words_of_three: Vec<bool> = (0..100).map(|i| i % 3 == 0).collect();
        assert_eq!(wide(&words_of_three, 5, 50), wide(&words_of_three, 5, 4));
    }

    /// Mark patterns for the sweep: every token, only the first, words of a
    /// fixed length, and words of mixed lengths with an occasional one of 30
    /// tokens, longer than most caps of the sweep.
    fn mark_patterns(n: usize) -> Vec<Vec<bool>> {
        let mut patterns = vec![vec![true; n], (0..n).map(|i| i == 0).collect()];
        for len in 2..=5 {
            patterns.push((0..n).map(|i| i % len == 0).collect());
        }
        for seed in 1..=4u64 {
            let mut state = seed;
            let mut marks = vec![false; n];
            let mut i = 0;
            while i < n {
                marks[i] = true;
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                i += if state >> 61 == 0 { 30 } else { 1 + (state >> 33) as usize % 6 };
            }
            patterns.push(marks);
        }
        patterns
    }

    /// Character widths for the sweep: so wide that one shared token joins,
    /// so narrow that the floor moves most starts back, and mixed.
    fn char_patterns(n: usize) -> Vec<Vec<(usize, usize)>> {
        let mut mixed = Vec::with_capacity(n);
        let mut at = 0;
        for i in 0..n {
            let width = 1 + (7 * i + 3) % 6;
            mixed.push((at, at + width));
            at += width;
        }
        vec![even_chars(n, MIN_OVERLAP_CHARS), even_chars(n, 1), mixed]
    }

    /// Fails when `windows` is not the plan D2 and D3 describe for `starts`
    /// and `chars`: bounded, covering, never nested, never shorter than the
    /// overlap but at the end, cut at the last word start in reach, and each
    /// next window started at the word start nearest the overlap that still
    /// joins.
    fn check_plan(starts: &[bool], chars: &[(usize, usize)], max: usize, overlap: usize, windows: &[(usize, usize)]) {
        let n = starts.len();
        let at = || format!("n={n} max={max} overlap={overlap} starts={starts:?} chars={chars:?} windows={windows:?}");
        if n == 0 {
            assert!(windows.is_empty(), "{}", at());
            return;
        }
        let overlap = overlap.min(max - 1);
        let marked = |from: usize, to: usize| (from..=to).filter(|&i| starts[i]).last();
        assert_eq!((windows[0].0, windows[windows.len() - 1].1), (0, n), "{}", at());
        let mut previous_end = 0;
        for (k, &(s, e)) in windows.iter().enumerate() {
            assert!(e > s && e - s <= max, "window {k} is empty or over the cap: {}", at());
            let Some(&(next_s, next_e)) = windows.get(k + 1) else { break };
            assert!(e - s > overlap, "window {k} is shorter than the overlap: {}", at());
            let expected_end = marked((s + overlap).max(previous_end) + 1, s + max).unwrap_or(s + max);
            assert_eq!(e, expected_end, "window {k} does not end at the last word start in reach: {}", at());
            assert!(next_s > s && next_e > e, "window {} lies inside window {k}: {}", k + 1, at());

            let target = e - overlap;
            let joins = |i: usize| chars[e - 1].1 - chars[i].0 >= MIN_OVERLAP_CHARS;
            let before = marked(s + 1, target);
            let after = (target..e).find(|&i| starts[i]);
            let nearest = match (before, after) {
                (Some(b), Some(a)) => Some(if a - target < target - b { a } else { b }),
                (b, a) => b.or(a),
            };
            let expected_start = match nearest {
                _ if overlap == 0 => before.unwrap_or(target),
                None => target,
                Some(c) if joins(c) => c,
                Some(c) => (s + 1..c).rev().find(|&i| starts[i] && joins(i)).or(before).unwrap_or(target),
            };
            assert_eq!(next_s, expected_start, "window {} does not start where D3 says: {}", k + 1, at());
            if overlap > 0 && (s + 1..e).any(|i| starts[i] && joins(i)) {
                assert!(joins(next_s), "windows {k} and {} share under 16 characters: {}", k + 1, at());
            }
            previous_end = e;
        }
    }

    #[test]
    fn plan_word_windows_terminates_covers_and_never_nests() {
        // Every pattern of marks up to ten tokens, the first always marked as
        // `word_starts` marks it.
        for n in 1..=10usize {
            for bits in 0..1u32 << (n - 1) {
                let starts: Vec<bool> = (0..n).map(|i| i == 0 || (bits >> (i - 1)) & 1 == 1).collect();
                for chars in char_patterns(n) {
                    for max in 1..=12usize {
                        for overlap in 0..=max {
                            check_plan(&starts, &chars, max, overlap, &plan_word_windows(&starts, &chars, max, overlap));
                        }
                    }
                }
            }
        }
        for n in 0..100usize {
            for starts in mark_patterns(n) {
                for chars in char_patterns(n) {
                    for max in 1..24usize {
                        for overlap in 0..max {
                            check_plan(&starts, &chars, max, overlap, &plan_word_windows(&starts, &chars, max, overlap));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn plan_word_windows_cuts_at_the_last_word_start_under_the_cap() {
        // Words of three tokens: no window ends at its cap of 10, every one at
        // the word start just under it.
        let starts: Vec<bool> = (0..30).map(|i| i % 3 == 0).collect();
        assert_eq!(wide(&starts, 10, 2), [(0, 9), (6, 15), (12, 21), (18, 27), (24, 30)]);
    }

    #[test]
    fn plan_word_windows_cuts_a_word_longer_than_the_cap_by_token() {
        // Five words of one token, one of 25 (tokens 5 to 29) and ten more.
        let starts: Vec<bool> = (0..40).map(|i| !(6..30).contains(&i)).collect();
        assert_eq!(
            wide(&starts, 10, 2),
            [(0, 5), (3, 13), (5, 15), (13, 23), (21, 31), (30, 40)],
            "inside the long word the cuts fall on tokens 13, 15, 21 and 23, and the window after it opens at the word that follows"
        );
    }

    #[test]
    fn plan_word_windows_starts_at_the_word_start_nearest_the_overlap() {
        // After the window [0, 10) an overlap of 4 points at token 6. The word
        // start at 7 is one token from it and the one at 2 four: the next
        // window starts at 7.
        let starts: Vec<bool> = (0..40).map(|i| [0, 2, 7].contains(&i) || i >= 10).collect();
        assert_eq!(wide(&starts, 10, 4)[..2], [(0, 10), (7, 17)]);
        // Two tokens on either side: the one before, so the overlap grows.
        let starts: Vec<bool> = (0..40).map(|i| [0, 4, 8].contains(&i) || i >= 10).collect();
        assert_eq!(wide(&starts, 10, 4)[..2], [(0, 10), (4, 14)]);
    }

    #[test]
    fn plan_word_windows_keeps_the_characters_join_pieces_needs() {
        // Tokens of two characters: 4 shared tokens are 8 characters, under
        // the 16 `join_pieces` looks for, so each next window starts back
        // where the shared text holds 16.
        let starts = vec![true; 40];
        let narrow = even_chars(40, 2);
        let windows = plan_word_windows(&starts, &narrow, 10, 4);
        assert_eq!(windows[..2], [(0, 10), (2, 12)]);
        for pair in windows.windows(2) {
            assert!(narrow[pair[0].1 - 1].1 - narrow[pair[1].0].0 >= MIN_OVERLAP_CHARS, "{windows:?}");
        }
        // No overlap asked for, no floor: the windows tile the text.
        assert_eq!(plan_word_windows(&starts, &narrow, 10, 0), [(0, 10), (10, 20), (20, 30), (30, 40)]);
        // A window of 10 characters cannot share 16: the next one starts at
        // the word start the overlap points at.
        assert_eq!(plan_word_windows(&starts, &even_chars(40, 1), 10, 4)[..2], [(0, 10), (6, 16)]);
    }

    // ── TokenSplitter, byte-fallback path ───────────────────────────────────

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
    fn the_fallback_count_is_a_quarter_of_the_bytes_rounded_up() {
        let splitter = TokenSplitter::fallback();
        assert_eq!(splitter.count(""), 0);
        assert_eq!(splitter.count("abcd"), 1);
        assert_eq!(splitter.count("abcde"), 2);
        assert_eq!(splitter.count(&"x".repeat(1920)), 480);
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

    /// Roots of the subword fixture: an opening consonant, a vowel and a
    /// closing consonant. All of one length, and no opening is `r`, so the
    /// greedy match of WordPiece takes exactly three letters at the start of
    /// any word the fixture generates.
    fn subword_roots() -> Vec<String> {
        let mut roots = Vec::new();
        for opening in "bcdfgklmnp".chars() {
            for vowel in "aeiou".chars() {
                for closing in "lmnrst".chars() {
                    roots.push(format!("{opening}{vowel}{closing}"));
                }
            }
        }
        roots
    }

    /// What follows `##`: a vowel and a closing consonant. None starts with
    /// `ev`, so `##evi` never matches inside a generated word.
    fn subword_continuations() -> Vec<String> {
        "aeiou".chars().flat_map(|vowel| "lmnrst".chars().map(move |closing| format!("{vowel}{closing}"))).collect()
    }

    /// CJK scalars in the vocabulary, as roots and as continuations: a run of
    /// them with no space between is one word of one token per character.
    const SUBWORD_CJK: u32 = 400;

    /// A `WordPiece` tokenizer over a whitespace split, which cuts a word into
    /// a root and `##` continuations, as the model's tokenizer does: the
    /// word-level tokenizers above make one token of every word and so never
    /// show a cut inside one. `retrievies` is `retri ##evi ##es`; a word it has
    /// no pieces for is one `[UNK]`.
    pub(crate) fn subword_tokenizer() -> Arc<tokenizers::Tokenizer> {
        let mut tokens = vec!["[UNK]".to_string(), "retri".to_string(), "##evi".to_string()];
        tokens.extend(subword_roots());
        tokens.extend(subword_continuations().iter().map(|c| format!("##{c}")));
        for c in (0..SUBWORD_CJK).map(|i| char::from_u32(0x4E00 + i).expect("CJK block is dense here")) {
            tokens.push(c.to_string());
            tokens.push(format!("##{c}"));
        }
        let vocab: serde_json::Map<String, serde_json::Value> =
            tokens.into_iter().enumerate().map(|(id, token)| (token, id.into())).collect();
        let json = serde_json::json!({
            "version": "1.0", "truncation": null, "padding": null, "added_tokens": [],
            "normalizer": null, "pre_tokenizer": {"type": "WhitespaceSplit"},
            "post_processor": null, "decoder": null,
            "model": {
                "type": "WordPiece", "unk_token": "[UNK]", "continuing_subword_prefix": "##",
                "max_input_chars_per_word": 1000, "vocab": vocab
            }
        });
        Arc::new(json.to_string().parse::<tokenizers::Tokenizer>().expect("the subword tokenizer parses"))
    }

    /// `count` distinct words for [`subword_tokenizer`], alternating a root and
    /// one continuation (two tokens) with a root and two (three tokens).
    pub(crate) fn subword_words(count: usize) -> Vec<String> {
        let (roots, continuations) = (subword_roots(), subword_continuations());
        (0..count)
            .map(|i| {
                let mut word = format!("{}{}", roots[i % roots.len()], continuations[(i / roots.len()) % continuations.len()]);
                if i % 2 == 1 {
                    word.push_str(&continuations[(7 * i + 3) % continuations.len()]);
                }
                word
            })
            .collect()
    }

    #[test]
    fn the_subword_fixture_cuts_a_word_into_its_pieces() {
        let tok = subword_tokenizer();
        let enc = tok.encode("a retrievies b", false).expect("the fixture encodes");
        assert_eq!(enc.get_tokens(), ["[UNK]", "retri", "##evi", "##es", "[UNK]"]);
        assert_eq!(enc.get_offsets()[1..4], [(2, 7), (7, 10), (10, 12)], "every piece lies inside the word");

        let words = subword_words(1600);
        assert_eq!(words.iter().collect::<std::collections::HashSet<_>>().len(), words.len(), "the words are distinct");
        for (i, word) in words.iter().enumerate() {
            let enc = tok.encode(word.as_str(), false).expect("the fixture encodes");
            assert_eq!(enc.len(), 2 + i % 2, "{word}: {:?}", enc.get_tokens());
        }
        let run: String = (0..300).map(|i| char::from_u32(0x4E00 + i).expect("CJK block is dense here")).collect();
        assert_eq!(tok.encode(run.as_str(), false).expect("the fixture encodes").len(), 300, "one token per character");
    }

    /// Whether a cut at byte `b` of `text` leaves every word of it whole: at
    /// either end, or next to whitespace.
    pub(crate) fn is_word_boundary(text: &str, b: usize) -> bool {
        b == 0
            || b == text.len()
            || text[..b].chars().next_back().is_some_and(char::is_whitespace)
            || text[b..].chars().next().is_some_and(char::is_whitespace)
    }

    /// Where each of `pieces` lies in `text`, in order: each is looked for from
    /// just after the start of the one before.
    pub(crate) fn piece_ranges(text: &str, pieces: &[&str]) -> Vec<(usize, usize)> {
        let mut from = 0;
        pieces
            .iter()
            .enumerate()
            .map(|(k, piece)| {
                let start = from + text[from..].find(piece).unwrap_or_else(|| panic!("piece {k} is not in the text: {piece:?}"));
                from = start + piece.chars().next().map_or(1, char::len_utf8);
                (start, start + piece.len())
            })
            .collect()
    }

    /// The text on both sides of byte `b`, for a failure message.
    fn around(text: &str, b: usize) -> String {
        let before: String = text[..b].chars().rev().take(16).collect::<Vec<_>>().into_iter().rev().collect();
        let after: String = text[b..].chars().take(16).collect();
        format!("{before:?} | {after:?}")
    }

    /// Fails when a piece of `text` starts or ends inside one of its words.
    pub(crate) fn assert_cut_at_word_boundaries(text: &str, pieces: &[&str]) {
        for (k, (start, end)) in piece_ranges(text, pieces).into_iter().enumerate() {
            assert!(is_word_boundary(text, start), "piece {k} starts inside a word: {}", around(text, start));
            assert!(is_word_boundary(text, end), "piece {k} ends inside a word: {}", around(text, end));
        }
    }

    #[test]
    fn no_piece_starts_or_ends_inside_a_word() {
        let text = subword_words(600).join(" ");
        let splitter = TokenSplitter::with_tokenizer(subword_tokenizer());
        let chunks = splitter.split(&text, 96, 24);
        let pieces: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        assert!(pieces.len() > 10, "the fixture yields many pieces, got {}", pieces.len());

        assert_cut_at_word_boundaries(&text, &pieces);
        let ranges = piece_ranges(&text, &pieces);
        assert_eq!((ranges[0].0, ranges[ranges.len() - 1].1), (0, text.len()), "the pieces reach both ends");
        for piece in &pieces {
            assert!(splitter.count(piece) <= 96, "a piece of {} tokens", splitter.count(piece));
        }
        // Words are two or three tokens, so the word start nearest the 24th
        // token from a piece's end is at most one token off.
        for (k, pair) in ranges.windows(2).enumerate() {
            let shared = &text[pair[1].0..pair[0].1];
            let tokens = splitter.count(shared);
            assert!((23..=25).contains(&tokens), "pieces {k} and {} share {tokens} tokens", k + 1);
            assert!(shared.chars().count() >= 16, "pieces {k} and {} share under 16 characters", k + 1);
        }
    }

    #[test]
    fn a_word_longer_than_the_cap_is_cut_by_token_and_loses_no_byte() {
        let words = subword_words(60);
        let long: String = std::iter::once("bal".to_string())
            .chain((0..149).map(|i| char::from_u32(0x4E00 + i).expect("CJK block is dense here").to_string()))
            .collect();
        let text = format!("{} {long} {}", words[..20].join(" "), words[20..].join(" "));
        let long_start = text.find(&long).expect("the long word is in the text");
        let long_end = long_start + long.len();
        let splitter = TokenSplitter::with_tokenizer(subword_tokenizer());
        assert_eq!(splitter.count(&long), 150, "the long word is 150 tokens");

        let chunks = splitter.split(&text, 96, 24);
        let pieces: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let ranges = piece_ranges(&text, &pieces);
        assert_eq!((ranges[0].0, ranges[ranges.len() - 1].1), (0, text.len()), "the pieces reach both ends");
        for (k, pair) in ranges.windows(2).enumerate() {
            assert!(pair[1].0 <= pair[0].1, "bytes {}..{} after piece {k} are in no piece", pair[0].1, pair[1].0);
        }
        for piece in &pieces {
            assert!(splitter.count(piece) <= 96, "a piece of {} tokens", splitter.count(piece));
        }
        let cuts: Vec<usize> = ranges.iter().flat_map(|&(s, e)| [s, e]).collect();
        assert!(cuts.iter().any(|&b| b > long_start && b < long_end), "the long word is cut by token");
        for &b in cuts.iter().filter(|&&b| b <= long_start || b >= long_end) {
            assert!(is_word_boundary(&text, b), "a cut outside the long word falls inside a word at byte {b}");
        }
    }

    #[test]
    fn a_text_without_spaces_is_cut_as_before() {
        let text: String = (0..300).map(|i| char::from_u32(0x4E00 + i).expect("CJK block is dense here")).collect();
        let chunks = TokenSplitter::with_tokenizer(subword_tokenizer()).split(&text, 96, 24);
        let chars: Vec<char> = text.chars().collect();
        let expected: Vec<String> =
            [(0, 96), (72, 168), (144, 240), (216, 300)].iter().map(|&(s, e)| chars[s..e].iter().collect()).collect();
        assert_eq!(chunks.into_iter().map(|c| c.text).collect::<Vec<_>>(), expected, "one window every 72 tokens");
    }

    #[test]
    fn token_path_splits_on_token_windows_with_the_requested_overlap() {
        let (text, words) = word_fixture(300);
        let tok = word_level_tokenizer(&words);
        let splitter = TokenSplitter::with_tokenizer(tok);

        let (max, overlap) = (100usize, 20usize);
        let chunks = splitter.split(&text, max, overlap);
        // Every word is one token, so every token opens a word.
        let expected = [(0, 100), (80, 180), (160, 260), (240, 300)];
        assert_eq!(chunks.len(), expected.len(), "one window every 80 tokens");

        for (chunk, &(s, e)) in chunks.iter().zip(expected.iter()) {
            // Exact text: pins BOTH offset lookups (start uses the token's
            // start offset, end uses the last token's END offset) — reading
            // the wrong member of either pair changes this string.
            assert_eq!(
                chunk.text,
                words[s..e].join(" "),
                "chunk for window ({s},{e}) does not carry exactly those tokens"
            );
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
    }

    #[test]
    fn token_path_text_that_tokenizes_to_nothing_yields_no_chunks() {
        let (_, words) = word_fixture(10);
        let splitter = TokenSplitter::with_tokenizer(word_level_tokenizer(&words));
        assert!(splitter.split("   ", 100, 20).is_empty());
    }

    #[test]
    fn split_ascii_byte_path_cuts_windows_of_the_cap_at_its_stride() {
        // On ASCII the byte path cuts 1920-byte windows every 1664 bytes, as
        // the token path does when every token opens a word.
        let content = "y".repeat(4000);
        let chunks = TokenSplitter::fallback().split(&content, 480, 64);
        assert_eq!(chunks.iter().map(|c| c.text.len()).collect::<Vec<_>>(), [1920, 1920, 672]);
    }
}
