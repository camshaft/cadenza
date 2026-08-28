//! A pretty-printing engine: Oppen's algorithm behind a small box/break token API.
//!
//! This is the layout core the printer targets. The API — `word`, `space`/`zerobreak`/`hardbreak`,
//! and consistent/inconsistent boxes (`cbox`/`ibox`) with an indent offset — is the NORMATIVE
//! contract: a construct is described as a token stream, and this module lays it out to fit a width.
//! The engine here is Oppen's (greedy, O(n), lookahead bounded by the width, no backtracking); a
//! later rewrite may swap in an equivalent strict-Wadler backend behind the same token API without
//! changing any caller.
//!
//! Semantics (Oppen):
//! - A **box** whose flat width fits the remaining space is printed flat (on one line), regardless
//!   of style.
//! - A box that does NOT fit breaks. **Consistent**: every `space`/`line` break inside it fires
//!   (one item per line). **Inconsistent** (fill): a break fires only when the next chunk would
//!   overflow.
//! - A break's `blank_space` is the spaces emitted when it does NOT fire; its `offset` is extra
//!   indent (added to the enclosing box indent) when it DOES fire.
//! - `hardbreak` is a break wide enough to never fit, so it always fires and forces its box to break.
//!
//! We are not streaming (the whole program is in hand), so this is a simple two-phase batch: build a
//! flat token vector computing each token's flat width, then render. This is equivalent to Oppen's
//! interleaved scan/print but far easier to get right (no ring buffer / absolute-index bookkeeping).

/// Break style of a box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Breaks {
    /// If the box breaks, every break inside it fires (one item per line).
    Consistent,
    /// If the box breaks, a break fires only when the next chunk would overflow (fill).
    Inconsistent,
}

/// A layout token. Build a `Vec<Token>` via [`Doc`], then [`render`] it.
#[derive(Clone, Debug)]
enum Token {
    /// Literal text (never broken internally).
    Text(String),
    /// An optional break: `blank_space` spaces when flat, or a newline + (box indent + `offset`).
    Break { blank_space: usize, offset: isize },
    /// Open a box with an indent `offset` and a break style.
    Begin { offset: isize, breaks: Breaks },
    /// Close the most recent box.
    End,
}

/// A builder for a layout document — a flat token stream. The printer emits into one of these.
#[derive(Default)]
pub struct Doc {
    tokens: Vec<Token>,
}

/// A break's flat width when it is a hard break: larger than any real line width, so a box
/// containing it never "fits" and thus breaks — but small (Oppen's `0xffff` sentinel) so summing
/// many of them into the running total cannot overflow. Callers must use a width below this.
const HARDBREAK_WIDTH: usize = 0xffff;

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    /// Emit literal text.
    pub fn word(&mut self, s: impl Into<String>) {
        self.tokens.push(Token::Text(s.into()));
    }

    /// A break that is a single space when flat.
    pub fn space(&mut self) {
        self.tokens.push(Token::Break {
            blank_space: 1,
            offset: 0,
        });
    }

    /// A break that is nothing when flat (a soft break).
    pub fn zerobreak(&mut self) {
        self.tokens.push(Token::Break {
            blank_space: 0,
            offset: 0,
        });
    }

    /// A break that always fires (a hard newline), forcing the enclosing box to break.
    pub fn hardbreak(&mut self) {
        self.tokens.push(Token::Break {
            blank_space: HARDBREAK_WIDTH,
            offset: 0,
        });
    }

    /// A hard newline (always fires) with an explicit firing indent offset — e.g. `-INDENT` to dedent a
    /// closing delimiter back to its opener's column. `hardbreak()` is the `offset == 0` case.
    pub fn hardbreak_with(&mut self, offset: isize) {
        self.tokens.push(Token::Break {
            blank_space: HARDBREAK_WIDTH,
            offset,
        });
    }

    /// A break with an explicit flat width and firing indent offset.
    pub fn break_with(&mut self, blank_space: usize, offset: isize) {
        self.tokens.push(Token::Break {
            blank_space,
            offset,
        });
    }

    /// Open a consistent box (all-or-nothing: if it breaks, every break inside fires).
    pub fn cbox(&mut self, offset: isize) {
        self.tokens.push(Token::Begin {
            offset,
            breaks: Breaks::Consistent,
        });
    }

    /// Open an inconsistent box (fill: breaks fire only on overflow).
    pub fn ibox(&mut self, offset: isize) {
        self.tokens.push(Token::Begin {
            offset,
            breaks: Breaks::Inconsistent,
        });
    }

    /// Close the most recent box.
    pub fn end(&mut self) {
        self.tokens.push(Token::End);
    }

    /// Render to a string that fits `width` columns where possible. `indent` is the number of spaces
    /// per box level offset — passed through the `offset`s the caller chose, so this just needs the
    /// target `width`.
    pub fn render(&self, width: usize) -> String {
        // Phase 1: flat width of each token span.
        //
        // `size[i]` for a Begin/Break is the flat width from that token up to (and including) the
        // matching End / up to the next break-or-end. For text it is the text width. We compute it
        // with a stack of open positions, resolving each Begin/Break when its span closes — the
        // batch analogue of Oppen's negative-placeholder `scan`.
        let n = self.tokens.len();
        let mut size = vec![0isize; n];
        // running flat width consumed so far (Oppen's right_total)
        let mut total: isize = 0;
        // stack of indices of open Begins and pending Breaks awaiting their size
        let mut stack: Vec<usize> = Vec::new();

        for i in 0..n {
            match &self.tokens[i] {
                Token::Text(s) => {
                    let w = s.chars().count() as isize;
                    size[i] = w;
                    total = total.saturating_add(w);
                }
                Token::Begin { .. } => {
                    // placeholder: resolved at the matching End
                    size[i] = -total;
                    stack.push(i);
                }
                Token::Break { blank_space, .. } => {
                    // A pending break's span ends at the NEXT break or the box's end. Resolve the
                    // previous pending break (if this break is a sibling in the same box).
                    if let Some(&top) = stack.last()
                        && matches!(self.tokens[top], Token::Break { .. })
                    {
                        size[top] += total;
                        stack.pop();
                    }
                    size[i] = -total;
                    stack.push(i);
                    total = total.saturating_add(flat_break_width(*blank_space));
                }
                Token::End => {
                    // Resolve a trailing pending break, then the matching Begin.
                    if let Some(&top) = stack.last()
                        && matches!(self.tokens[top], Token::Break { .. })
                    {
                        size[top] += total;
                        stack.pop();
                    }
                    if let Some(begin) = stack.pop() {
                        size[begin] += total;
                    }
                    size[i] = 0;
                }
            }
        }

        // Phase 2: print, deciding each break from its enclosing box.
        let mut out = String::new();
        // remaining columns on the current line
        let mut space: isize = width as isize;
        // per-open-box: (indent-at-open, breaks, fits?) — `fits` means the whole box was flat.
        struct Frame {
            indent: isize,
            breaks: Breaks,
            fits: bool,
        }
        let mut frames: Vec<Frame> = Vec::new();
        let mut indent: isize = 0;
        let mut pending_indent: isize = 0;

        for (token, &tok_size) in self.tokens.iter().zip(&size) {
            match token {
                Token::Text(s) => {
                    if pending_indent > 0 {
                        for _ in 0..pending_indent {
                            out.push(' ');
                        }
                        pending_indent = 0;
                    }
                    out.push_str(s);
                    space -= s.chars().count() as isize;
                }
                Token::Begin { offset, breaks } => {
                    let fits = tok_size <= space;
                    frames.push(Frame {
                        indent,
                        breaks: *breaks,
                        fits,
                    });
                    if !fits {
                        indent += offset;
                    }
                }
                Token::End => {
                    if let Some(f) = frames.pop() {
                        indent = f.indent;
                    }
                }
                Token::Break {
                    blank_space,
                    offset,
                } => {
                    let top = frames.last();
                    let fires = match top {
                        None => true,               // a break outside any box always fires
                        Some(f) if f.fits => false, // whole box is flat: never break
                        Some(f) => match f.breaks {
                            Breaks::Consistent => true,               // every break fires
                            Breaks::Inconsistent => tok_size > space, // fill: break on overflow
                        },
                    };
                    if fires {
                        out.push('\n');
                        let ind = indent + offset;
                        pending_indent = ind.max(0);
                        space = width as isize - ind.max(0);
                    } else {
                        for _ in 0..*blank_space {
                            out.push(' ');
                        }
                        space -= *blank_space as isize;
                    }
                }
            }
        }
        out
    }
}

/// The flat width a break contributes. A hardbreak's width is huge (so any box containing it never
/// "fits" and thus breaks), but for the running total we must not overflow, so cap it.
fn flat_break_width(blank_space: usize) -> isize {
    if blank_space >= HARDBREAK_WIDTH {
        HARDBREAK_WIDTH as isize
    } else {
        blank_space as isize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_when_it_fits() {
        // f(a, b) fits in 80 cols -> one line.
        let mut d = Doc::new();
        d.ibox(2);
        d.word("f(");
        d.zerobreak();
        d.word("a,");
        d.space();
        d.word("b)");
        d.end();
        assert_eq!(d.render(80), "f(a, b)");
    }

    #[test]
    fn consistent_box_breaks_all() {
        // A consistent box that overflows: every space becomes a newline (one item per line).
        let mut d = Doc::new();
        d.cbox(2);
        d.word("f(");
        d.space();
        d.word("aaaa,");
        d.space();
        d.word("bbbb,");
        d.space();
        d.word("cccc");
        d.end();
        // width 10 forces the break
        let out = d.render(10);
        assert_eq!(out, "f(\n  aaaa,\n  bbbb,\n  cccc");
    }

    #[test]
    fn inconsistent_box_fills() {
        // A fill box: pack until overflow, then break — not one-per-line.
        let mut d = Doc::new();
        d.ibox(0);
        for (i, w) in ["11", "22", "33", "44", "55"].iter().enumerate() {
            if i > 0 {
                d.space();
            }
            d.word(*w);
        }
        d.end();
        // width 8: "11 22 33" is 8, next " 44" overflows -> break; then "44 55".
        let out = d.render(8);
        assert_eq!(out, "11 22 33\n44 55");
    }

    #[test]
    fn hardbreak_always_fires() {
        let mut d = Doc::new();
        d.cbox(0);
        d.word("a");
        d.hardbreak();
        d.word("b");
        d.end();
        // Even though "a b" fits in 80, the hardbreak forces two lines.
        assert_eq!(d.render(80), "a\nb");
    }

    #[test]
    fn nested_indent() {
        // Outer breaks, inner fits.
        let mut d = Doc::new();
        d.cbox(2);
        d.word("outer(");
        d.space();
        d.ibox(2);
        d.word("f(a,");
        d.space();
        d.word("b)");
        d.end();
        d.end();
        let out = d.render(10);
        // outer breaks (>10 flat), inner "f(a, b)" fits on its own indented line
        assert_eq!(out, "outer(\n  f(a, b)");
    }

    #[test]
    fn empty_doc() {
        assert_eq!(Doc::new().render(80), "");
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Emit a random, BALANCED box subtree into `d` (every `cbox`/`ibox` gets its matching `end`).
    /// Between any two atomic tokens (words / nested boxes) it ALWAYS emits a break (space/zerobreak/
    /// hardbreak), so there is a breakable point between every pair of tokens — that makes the width
    /// bound meaningful: an overflow can then only come from a single too-wide word, never from an
    /// unbreakable concatenation of adjacent words. Returns the widest single WORD emitted.
    fn gen_doc(rng: &mut Rng, d: &mut Doc, depth: usize) -> usize {
        let words = ["a", "bb", "ccc", "word", "longer-token", "x"];
        let mut widest = 0usize;
        let n = 1 + rng.below(4);
        for i in 0..n {
            if i > 0 {
                // A break BETWEEN tokens (never two adjacent words with no break between them).
                match rng.below(3) {
                    0 => d.space(),
                    1 => d.zerobreak(),
                    _ => d.hardbreak(),
                }
            }
            // An atomic token: a word, or a nested box (only while depth remains).
            if depth == 0 || rng.below(3) == 0 {
                let w = words[rng.below(words.len())];
                d.word(w);
                widest = widest.max(w.chars().count());
            } else if rng.below(2) == 0 {
                d.cbox(rng.below(4) as isize);
                widest = widest.max(gen_doc(rng, d, depth - 1));
                d.end();
            } else {
                d.ibox(rng.below(4) as isize);
                widest = widest.max(gen_doc(rng, d, depth - 1));
                d.end();
            }
        }
        widest
    }

    /// Emit a random BALANCED, HARDBREAK-FREE box subtree into `d`, and simultaneously build the
    /// FLAT string it should render to when it fits: a word contributes its text, a break contributes
    /// `blank_space` spaces (space→1, zerobreak→0), boxes contribute nothing. Every pair of tokens still
    /// gets a break between them, but never a hardbreak — so the whole tree CAN lay out flat.
    fn gen_flat_doc(rng: &mut Rng, d: &mut Doc, depth: usize, flat: &mut String) {
        let words = ["a", "bb", "ccc", "word", "longer-token", "x"];
        let n = 1 + rng.below(4);
        for i in 0..n {
            if i > 0 {
                // A soft break (space or zerobreak) between tokens — NEVER a hardbreak.
                if rng.below(2) == 0 {
                    d.space();
                    flat.push(' '); // a space break is one blank when flat
                } else {
                    d.zerobreak(); // zero blanks when flat
                }
            }
            if depth == 0 || rng.below(3) == 0 {
                let w = words[rng.below(words.len())];
                d.word(w);
                flat.push_str(w);
            } else if rng.below(2) == 0 {
                d.cbox(rng.below(4) as isize);
                gen_flat_doc(rng, d, depth - 1, flat);
                d.end();
            } else {
                d.ibox(rng.below(4) as isize);
                gen_flat_doc(rng, d, depth - 1, flat);
                d.end();
            }
        }
    }

    #[test]
    fn flat_render_equals_the_flat_string_when_the_width_admits_it() {
        // The "fits ⇒ flat" contract, the counterpart to the overflow sweep: a HARDBREAK-FREE doc wrapped
        // in one outer box, rendered at a width ≥ its own flat width, must (a) contain NO newline and
        // (b) render EXACTLY the flat concatenation (words + the blank-spaces of unfired breaks). This
        // pins phase-1's flat-width computation AND the `fits` decision — the existing sweep only bounds
        // line length, never asserts a fitting doc actually lays out flat. Rendered both at the exact
        // flat width and comfortably above it (the decision must be monotone: more room never breaks).
        let mut rng = Rng(0xf1a7_c0de_1eee_7777);
        for _ in 0..3000 {
            let mut d = Doc::new();
            let mut flat = String::new();
            // An OUTER box so there are no top-level bare breaks (a break outside any box always fires).
            d.cbox(0);
            let depth = 1 + rng.below(4);
            gen_flat_doc(&mut rng, &mut d, depth, &mut flat);
            d.end();
            let flat_width = flat.chars().count();
            for &width in &[flat_width, flat_width + 1, flat_width + 50] {
                let out = d.render(width);
                assert!(
                    !out.contains('\n'),
                    "a doc that fits in width {width} (flat_width {flat_width}) broke a line:\n{out}"
                );
                assert_eq!(
                    out, flat,
                    "flat render at width {width} must equal the flat concatenation"
                );
            }
        }
    }

    /// Emit a random BALANCED box subtree (words, soft/hard breaks, nested boxes) and RECORD every word
    /// emitted, in order. The words are drawn from an alphabet with NO whitespace, so the words are the
    /// ONLY source of non-whitespace output — everything the engine adds (blanks, indent, newlines) is
    /// whitespace. Returns nothing; appends words to `words`.
    fn gen_doc_recording(rng: &mut Rng, d: &mut Doc, depth: usize, words: &mut Vec<&'static str>) {
        // No whitespace inside any of these — so stripping whitespace from the render recovers exactly
        // the word sequence (a space/hardbreak inside a word would defeat that).
        let alphabet = ["a", "bb", "ccc", "word", "longer-token", "x", "(", ")", ","];
        let n = 1 + rng.below(4);
        for i in 0..n {
            if i > 0 {
                match rng.below(3) {
                    0 => d.space(),
                    1 => d.zerobreak(),
                    _ => d.hardbreak(),
                }
            }
            if depth == 0 || rng.below(3) == 0 {
                let w = alphabet[rng.below(alphabet.len())];
                d.word(w);
                words.push(w);
            } else if rng.below(2) == 0 {
                d.cbox(rng.below(4) as isize);
                gen_doc_recording(rng, d, depth - 1, words);
                d.end();
            } else {
                d.ibox(rng.below(4) as isize);
                gen_doc_recording(rng, d, depth - 1, words);
                d.end();
            }
        }
    }

    #[test]
    fn render_preserves_word_content_and_order_at_every_width() {
        // Content preservation — the invariant orthogonal to the width-bound and fits⇒flat sweeps: a
        // pretty-printer decides WHERE to break, never WHAT to emit. Whatever the width or the resulting
        // break decisions, the rendered text must contain EXACTLY the input words, in order — never
        // dropping, duplicating, reordering, or corrupting one. A phase-1/phase-2 desync (e.g. a break's
        // span mis-resolved so a token is skipped, or an off-by-one in the scan stack) would surface here
        // as changed content, which neither existing sweep checks (they bound geometry, not payload).
        // Because the word alphabet carries NO whitespace, every non-whitespace char in the output comes
        // from a word — so stripping ALL whitespace yields the words concatenated in order.
        let mut rng = Rng(0xc047_e27a_0aad_0001);
        for _ in 0..4000 {
            let mut d = Doc::new();
            let mut words: Vec<&str> = Vec::new();
            // An outer box so top-level breaks live inside a frame (a bare top-level break always fires —
            // harmless for content, but keeps the shape like a real printed form).
            d.cbox(0);
            let depth = 1 + rng.below(4);
            gen_doc_recording(&mut rng, &mut d, depth, &mut words);
            d.end();
            let expected: String = words.concat();
            for &width in &[1usize, 3, 7, 16, 40, 120] {
                let out = d.render(width);
                let stripped: String = out.chars().filter(|c| !c.is_whitespace()).collect();
                assert_eq!(
                    stripped, expected,
                    "render at width {width} changed word content/order\n  words: {words:?}\n  out: {out:?}"
                );
            }
        }
    }

    #[test]
    fn render_never_panics_and_respects_the_width_bound() {
        // The pretty-printer's core correctness property, swept over random BALANCED token streams (a
        // break between every pair of tokens) at a range of widths: `render` (a) never PANICS (no OOB /
        // underflow in the Oppen scan), and (b) respects the WIDTH BOUND — no output line is longer than
        // `max(width, widest word) + the deepest indent`. Because every pair of tokens has a break
        // between it, a line can exceed `width` ONLY because a single unbreakable `word` (offset by a box
        // indent) is itself that wide; the engine must never gratuitously overflow when a break was
        // available. Fixed seed → reproducible; the failing doc's render is printed.
        let mut rng = Rng(0x0bad_c0de_dead_beef);
        for _ in 0..3000 {
            let mut d = Doc::new();
            let depth = 1 + rng.below(4);
            let widest = gen_doc(&mut rng, &mut d, depth);
            for &width in &[1usize, 4, 8, 20, 80] {
                let out = d.render(width);
                // Ceiling: `max(width, widest word)` plus the deepest achievable indent (≤ 4 per box
                // level × the max depth), since a broken line is prefixed by its box's indent.
                let ceiling = width.max(widest).saturating_add(4 * 8);
                for line in out.split('\n') {
                    assert!(
                        line.chars().count() <= ceiling,
                        "line {:?} (len {}) exceeds ceiling {ceiling} at width {width}\n--- full ---\n{out}",
                        line,
                        line.chars().count()
                    );
                }
            }
        }
    }
}
