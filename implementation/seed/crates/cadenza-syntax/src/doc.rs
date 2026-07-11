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
        self.tokens.push(Token::Break { blank_space: 1, offset: 0 });
    }

    /// A break that is nothing when flat (a soft break).
    pub fn zerobreak(&mut self) {
        self.tokens.push(Token::Break { blank_space: 0, offset: 0 });
    }

    /// A break that always fires (a hard newline), forcing the enclosing box to break.
    pub fn hardbreak(&mut self) {
        self.tokens.push(Token::Break { blank_space: HARDBREAK_WIDTH, offset: 0 });
    }

    /// A break with an explicit flat width and firing indent offset.
    pub fn break_with(&mut self, blank_space: usize, offset: isize) {
        self.tokens.push(Token::Break { blank_space, offset });
    }

    /// Open a consistent box (all-or-nothing: if it breaks, every break inside fires).
    pub fn cbox(&mut self, offset: isize) {
        self.tokens.push(Token::Begin { offset, breaks: Breaks::Consistent });
    }

    /// Open an inconsistent box (fill: breaks fire only on overflow).
    pub fn ibox(&mut self, offset: isize) {
        self.tokens.push(Token::Begin { offset, breaks: Breaks::Inconsistent });
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
                    frames.push(Frame { indent, breaks: *breaks, fits });
                    if !fits {
                        indent += offset;
                    }
                }
                Token::End => {
                    if let Some(f) = frames.pop() {
                        indent = f.indent;
                    }
                }
                Token::Break { blank_space, offset } => {
                    let top = frames.last();
                    let fires = match top {
                        None => true, // a break outside any box always fires
                        Some(f) if f.fits => false, // whole box is flat: never break
                        Some(f) => match f.breaks {
                            Breaks::Consistent => true, // every break fires
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
}
