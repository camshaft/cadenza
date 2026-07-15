//! The Cadenza calculator engine — the state + evaluation layer a REPL/CLI/GUI shares.
//!
//! A calculator is not a new evaluator: it is a small STATE layer over the existing pipeline. This crate
//! owns that state — the user's accumulated variable bindings + an implicit `ans` — and the one operation
//! over it: given a line the user typed, classify it (assignment vs expression), assemble the accumulated
//! bindings + this line into a runnable program, compile it (`rcdzc`), run it (`cdz_run`), and render the
//! value in the reader's surface.
//!
//! ## How bindings accumulate — a NESTED-`let` chain over stored SOURCES
//!
//! Each binding stores its SOURCE expression, and evaluating a line wraps the target expression in a
//! chain of `let`s — one per binding, oldest outermost:
//!
//! ```text
//!   x = 5          →  bindings: [(x, "5")]
//!   x * x          →  eval  (let ((x 5)) (* x x))                    = 25, ans := "(* x x)"
//!   ans + 1        →  eval  (let ((x 5)) (let ((ans (* x x))) (+ ans 1)))  = 26
//! ```
//!
//! This is the model that makes everything work at once (learned the hard way — a top-level-`def`
//! assembly stack-overflowed on `ans = ans + 5` and choked on non-re-readable value forms like `1/2`):
//! - **No circularity.** A re-binding `ans = ans + 5` becomes an INNER `let ((ans (+ ans 5)))` whose
//!   right-hand `ans` resolves to the OUTER (previous) `ans` — ordinary lexical shadowing, `= 25`, never
//!   a self-reference. (A top-level `(def ans (+ ans 5))` recurses forever; `let`-shadowing does not.)
//! - **No value-form re-readability problem.** Storing SOURCE means a Rational never has to round-trip
//!   through its display form `1/2` (which the lexer rejects as a malformed literal — it is display-only,
//!   not re-readable source); the source `(Rational.of 1 3)` re-reads fine.
//! - **Live recalc, for free.** Because sources are re-evaluated each turn, a later re-binding of an
//!   earlier variable is reflected (a spreadsheet-like nicety the design doc wanted).
//!
//! `=` in the language is EQUALITY; the calculator's `x = expr` sugar becomes a `let` binding. It is a
//! front-end convenience, NOT a new language form. The classifier tells `x = expr` (assignment) from
//! `a == b` (an equality expression) by the ML `==` glyph / a single leading identifier.

use cadenza_syntax::convert::Format;

/// The implicit binding that holds the last computed expression's SOURCE — so `ans` recalls (and, via
/// `let`-shadowing, composes with) the previous result. Chosen to not collide with a plausible variable.
pub const ANS: &str = "ans";

/// What one evaluated line produced — the shape a REPL/GUI renders.
#[derive(Debug, Clone, PartialEq)]
pub enum Eval {
    /// An expression (or an assignment's echoed value) evaluated to a rendered value.
    Value(String),
    /// A binding `name = expr` was recorded (and its value echoed). `name` is the bound variable.
    Bound { name: String, value: String },
    /// The program ran but trapped at run time (message) — e.g. division by zero.
    Trap(String),
    /// The line did not compile (the first error's message) — a type error, unbound name, parse fault.
    Error(String),
}

/// How to classify a typed line.
enum Line<'a> {
    /// `name = expr` — an assignment. The rest is the value expression source.
    Assign { name: &'a str, expr: &'a str },
    /// A bare expression to evaluate.
    Expr(&'a str),
}

/// The calculator's session state: the ordered variable bindings (name → SOURCE expression) plus the
/// surface the user works in. Ordered oldest-first — that is the order the `let` chain nests (oldest
/// outermost), so a later binding's source can reference an earlier name via lexical scope.
pub struct Calculator {
    /// The surface the input expression, the stored binding sources, and the displayed result all use
    /// (ML by default) — one surface per session (fixed at construction), so the `let` chain is built in
    /// the same surface the sources were typed in.
    surface: Format,
    /// The bindings, in insertion order, deduped by name (last write wins — see [`Self::set_binding`]).
    /// Each is `(name, source)`. Sources (not value forms) so a Rational never has to round-trip through
    /// its non-re-readable display `1/2`, and so `ans` composes by lexical shadowing rather than
    /// recursing (see the module docs' `let`-chain explanation).
    bindings: Vec<(String, String)>,
    /// EXACT MODE: when true (the default — the operator's "forced rationals by default"), a bare numeric
    /// literal grounds to an exact `Rational`, so `1 / 3` is `1/3` (not integer-truncated 0) with no `R`
    /// suffix. Realized by assembling through `repl::assemble_repl_program_exact` (a do-local
    /// `(pragma default-fraction Rational)` module, C6). Off → ordinary Int64/Float defaults.
    exact: bool,
    /// PLAIN mode: render a result as the BARE value (`1/3`, `1500 meter`, `42`), stripping the
    /// `(: value type)` / `` `value` : Type`` wrapper — what a launcher (Raycast/Alfred, C5) shows the
    /// user. Off (the default) keeps the full typed form the REPL shows. Also displays a whole rational
    /// `5/1` as `5` (a calculator reads `5` more naturally than `5/1`).
    plain: bool,
}

impl Calculator {
    /// A fresh calculator in `surface`, EXACT MODE ON (forced rationals by default) — the operator's
    /// intended calculator behavior.
    pub fn new(surface: Format) -> Self {
        Calculator::new_with_exact(surface, true)
    }

    /// A fresh calculator in `surface` with exact mode set explicitly (`--exact=off` turns it off, giving
    /// ordinary integer/float literal defaults). Plain mode off.
    pub fn new_with_exact(surface: Format, exact: bool) -> Self {
        Calculator {
            surface,
            bindings: Vec::new(),
            exact,
            plain: false,
        }
    }

    /// Set PLAIN result rendering (bare value, no `: Type` wrapper) — the launcher-facing display. Chained
    /// on a constructor: `Calculator::new(surface).with_plain(true)`.
    pub fn with_plain(mut self, plain: bool) -> Self {
        self.plain = plain;
        self
    }

    /// The surface this calculator reads + renders in.
    pub fn surface(&self) -> Format {
        self.surface
    }

    /// The distinct names currently in scope (for a REPL's completion / a variables panel). A re-bound
    /// name appears once; the newest binding is the visible one (it shadows the older `let`s).
    pub fn names(&self) -> Vec<&str> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        // Walk newest-first so the visible (innermost) binding wins, then reverse for stable display.
        for (n, _) in self.bindings.iter().rev() {
            if seen.insert(n.as_str()) {
                out.push(n.as_str());
            }
        }
        out.reverse();
        out
    }

    /// Record `name = expr` by APPENDING (never replacing in place). A re-binding of an existing name is
    /// a new, INNERMOST `let` in the chain (see [`Self::wrap_in_lets`]) that SHADOWS the prior one — so a
    /// self-referential re-binding (`ans = ans + 5`, `x = x + 1`) reads the OUTER (previous) value rather
    /// than referencing an absent binding. The shadowed outer `let`s stay in the chain (dead but
    /// harmless); the growth is bounded by the lines typed in a session.
    fn set_binding(&mut self, name: &str, expr: &str) {
        self.bindings.push((name.to_string(), expr.to_string()));
    }

    /// Evaluate one typed line against the current bindings, updating state. An assignment records the
    /// binding source (and echoes its value); an expression evaluates and sets `ans` to its source. A line
    /// that fails to compile or traps does NOT commit any binding (state is unchanged), so a mistyped
    /// assignment never poisons the session. `ans` is committed only on success too.
    pub fn eval(&mut self, line: &str) -> Eval {
        match classify(line, self.surface) {
            Line::Assign { name, expr } => match self.eval_expr(expr) {
                Ok(display) => {
                    // Store the SOURCE; the `let` chain (see `wrap_in_lets`) makes a re-binding of an
                    // existing name shadow the old one lexically, so `x = x + 1` is not a recursion.
                    self.set_binding(name, expr);
                    Eval::Bound {
                        name: name.to_string(),
                        value: display,
                    }
                }
                Err(e) => e, // a failing assignment does not commit
            },
            Line::Expr(expr) => match self.eval_expr(expr) {
                Ok(display) => {
                    // `ans` records THIS expression's source; the next line's `let` chain binds it as the
                    // innermost `ans`, so `ans + 5` sees this result (and, if it re-binds `ans`, shadows).
                    self.set_binding(ANS, expr);
                    Eval::Value(display)
                }
                Err(e) => e,
            },
        }
    }

    /// Compile + run `expr` against the current bindings, returning the SURFACE-RENDERED value or a
    /// decline/trap as an [`Eval`] error. `expr` is wrapped in the binding `let` chain (`wrap_in_lets`),
    /// parsed in the session surface, assembled into a runnable program via the shared REPL assembler
    /// (the SAME path the browser playground uses — the whole `let`-wrapped form becomes the entry body),
    /// compiled, and run.
    fn eval_expr(&self, expr: &str) -> Result<String, Eval> {
        let wrapped = self.wrap_in_lets(expr);
        // An empty buffer (no defs) — the assembler tolerates it; all the state rides in the `let` chain.
        let buffer = parse_buffer("").map_err(Eval::Error)?;
        let expr_arena = parse_one(&wrapped, self.surface)
            .map_err(|m| Eval::Error(format!("in the expression: {m}")))?;
        // In exact mode the expression's bare literals default to Rational (forced rationals by default).
        let program = if self.exact {
            cadenza_syntax::repl::assemble_repl_program_exact(&buffer, &expr_arena)
        } else {
            cadenza_syntax::repl::assemble_repl_program(&buffer, &expr_arena)
        };
        eval_program(&program).map(|value_form| {
            if self.plain {
                plain_value(&value_form, self.surface)
            } else {
                render_value(&value_form, self.surface)
            }
        })
    }

    /// Wrap `expr` in a chain of `let` bindings, one per stored binding, OLDEST OUTERMOST — so a later
    /// binding sees the earlier ones, and evaluating `expr` sees them all. `x=5`, `y=x+1`, then expr `y*2`
    /// →  ML `let x = 5 in let y = x + 1 in y * 2`  /  s-expr `(let ((x 5)) (let ((y (+ x 1))) (* y 2)))`.
    /// With no bindings, `expr` is returned unwrapped.
    fn wrap_in_lets(&self, expr: &str) -> String {
        let mut wrapped = expr.to_string();
        // Fold from the INNERMOST (newest) binding outward, so the oldest ends up outermost.
        for (name, src) in self.bindings.iter().rev() {
            wrapped = if self.surface == Format::Ml {
                format!("let {name} = {src} in {wrapped}")
            } else {
                format!("(let (({name} {src})) {wrapped})")
            };
        }
        wrapped
    }
}

/// Classify a typed line as an assignment `name = expr` or a bare expression. An assignment is a single
/// leading identifier, then a single `=` (ML) that is NOT `==` (equality), then a non-empty expression.
/// Everything else — including an equality test `a == b`, a comparison, or any multi-token left side — is
/// an expression. (s-expr has no infix `=`; an s-expr assignment is written the ML way, `name = expr`,
/// as a calculator convenience.)
fn classify(line: &str, _surface: Format) -> Line<'_> {
    let trimmed = line.trim();
    // Find the first `=` that is not part of `==`, `<=`, `>=`, `!=`.
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            let next = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
            let is_comparison =
                next == b'=' || prev == b'=' || prev == b'<' || prev == b'>' || prev == b'!';
            if !is_comparison {
                let lhs = trimmed[..i].trim();
                let rhs = trimmed[i + 1..].trim();
                if is_identifier(lhs) && !rhs.is_empty() {
                    return Line::Assign {
                        name: lhs,
                        expr: rhs,
                    };
                }
                // A `=` that isn't a clean `ident = expr` (e.g. `f x = …`, or an empty rhs) → treat the
                // whole line as an expression; the compiler will give a real diagnostic if it's wrong.
                return Line::Expr(trimmed);
            }
            // Skip the whole comparison operator so `a == b == c` isn't misread.
            i += if next == b'=' { 2 } else { 1 };
            continue;
        }
        i += 1;
    }
    Line::Expr(trimmed)
}

/// A single Cadenza identifier: a non-empty run of ident characters (letters, digits, `-`, `_`, and a
/// dotted `.` for member paths), not starting with a digit. Kebab-and-dotted names are the norm
/// (`String.scalar-len`), so `-`/`.` are part of the token.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The empty definitions buffer the REPL program is assembled against — a bare `(do)` shell (no items),
/// which the shared assembler tolerates. The calculator carries ALL its state in the `let` chain around
/// the expression (see [`Calculator::wrap_in_lets`]), so there are never any top-level defs to supply.
/// `src` is accepted (always `""`) to keep the seam explicit should a future surface want real defs.
fn parse_buffer(_src: &str) -> Result<cadenza_syntax::Arenas, String> {
    cadenza_syntax::sexpr::read("(do)").map_err(|e| format!("{e:?}"))
}

/// Parse a SINGLE form (the expression) in `surface` into one arena. The whole arena root becomes the
/// REPL entry's body, so it must be exactly one expression.
fn parse_one(src: &str, surface: Format) -> Result<cadenza_syntax::Arenas, String> {
    if surface == Format::Ml {
        let parsed = cadenza_syntax::parser::read_ml(src);
        if let Some(err) = parsed.errors.first() {
            return Err(err.message.clone());
        }
        Ok(cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas).0)
    } else {
        cadenza_syntax::sexpr::read(src).map_err(|e| format!("{e:?}"))
    }
}

/// Compile the assembled program and run it, returning the raw canonical VALUE FORM (`cdz-run`'s s-expr
/// text). This is where all three libraries meet: `rcdzc::compile` → wasm component, `cdz_run::run` →
/// value / trap. A compile decline surfaces the first error's message; a runtime trap surfaces its
/// reason. The caller renders the value form for display (`render_value`) and stores it verbatim.
pub fn eval_program(program: &cadenza_syntax::Arenas) -> Result<String, Eval> {
    let ast_bytes = cadenza_syntax::codec::encode(program);
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[rcdzc::Target::Wasm],
    );
    let Some(component) = out.artifact(rcdzc::Target::Wasm.artifact_kind()) else {
        // No component → a decline. Surface the first error diagnostic's message.
        let msg = out
            .diagnostics
            .iter()
            .find(|d| d.severity == rcdzc::Severity::Error)
            .map(|d| match &d.code {
                Some(code) => format!("[{code}] {}", d.message),
                None => d.message.clone(),
            })
            .unwrap_or_else(|| "declined (no component emitted)".to_string());
        return Err(Eval::Error(msg));
    };

    // Run through cdz-run, resolving the value-heap runtime from the store by content address (the same
    // path the `cdz-run` binary uses). The host layer owns the store lookup — see `run_component`. Return
    // the RAW value form; the caller renders it for display and stores it verbatim.
    match crate::runtime::run_component(component) {
        Ok(cdz_run::Outcome::Value(text)) => Ok(text),
        Ok(cdz_run::Outcome::Trap(msg)) => Err(Eval::Trap(msg)),
        Err(e) => Err(Eval::Error(format!("{e:#}"))),
    }
}

/// Re-render a value form (`cdz-run`'s canonical s-expr text, e.g. `(: 1/2 Rational)` or `(tuple 1 2)`)
/// into the reader's surface for DISPLAY. A no-op for s-expr; ML re-renders via the printer's display
/// conversion (the spec's "typed-result-to-text" surface) — so a `Rational` shows bare (`1/2`), a
/// quantity in its concise `<value> <unit>` form, and the result's type annotation is dropped. A value
/// that won't re-render (should not happen for a well-formed value form) falls back to the raw text.
fn render_value(sexpr_value: &str, surface: Format) -> String {
    if surface != Format::Ml {
        return sexpr_value.to_string();
    }
    let opts = cadenza_syntax::convert::Options {
        display: true,
        ..Default::default()
    };
    match cadenza_syntax::sexpr::read(sexpr_value) {
        Ok(arenas) => match cadenza_syntax::convert::write_with(&arenas, Format::Ml, opts) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).trim_end().to_string(),
            Err(_) => sexpr_value.to_string(),
        },
        Err(_) => sexpr_value.to_string(),
    }
}

/// Render a value form as the BARE value in `surface` — strip the `(: value type)` wrapper to just the
/// value (`(: 1/3 Rational)` → `1/3`, `(: 1500 (Qty …))` → `1500 meter`), and simplify a whole rational
/// `N/1` → `N`. What a launcher shows the user. A non-`(: …)` form (shouldn't happen for a run result)
/// falls back to the full surface render.
///
/// Extracts the VALUE subtree structurally (via the parsed arena's `(: …)` form), re-emits it as its own
/// program, and renders that in the surface — so a compound value (`(tuple 1 2)` → `(1, 2)`) renders
/// correctly, not by fragile text-stripping.
fn plain_value(sexpr_value: &str, surface: Format) -> String {
    let Ok(arenas) = cadenza_syntax::sexpr::read(sexpr_value) else {
        return render_value(sexpr_value, surface);
    };
    // A run result is `(: value type)`; pull the value child (index 0 of the `:` form's tail).
    let Some(tail) = arenas.as_form(arenas.root, ":") else {
        return render_value(sexpr_value, surface);
    };
    let Some(&value_id) = tail.first() else {
        return render_value(sexpr_value, surface);
    };
    // Copy the value subtree into its own arena + render it in the surface.
    let value_arena = cadenza_syntax::query::Tree::from_arena(&arenas, value_id).to_arena();
    let rendered = match cadenza_syntax::convert::write(&value_arena, surface) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(_) => return render_value(sexpr_value, surface),
    };
    prettify_plain(&rendered)
}

/// Tidy a plain-rendered value for a launcher: (1) drop the ML printer's backticks around a rational
/// (`` `1/3` `` → `1/3`) — a cosmetic wart where the ML printer treats `1/3` as an operator name, never
/// meaningful in a DISPLAYED value; (2) collapse a whole rational `N/1` → `N` (a calculator reads `5`
/// more naturally than `5/1`), including one embedded in a compound render (`1500/1` → `1500` inside a
/// quantity). Both are display-only string tidies over the already-correct value.
fn prettify_plain(s: &str) -> String {
    // Drop ALL backticks (they only ever wrap a rational literal the ML printer mis-quotes).
    let unbacktick = s.replace('`', "");
    // Collapse every `<digits>/1` token → `<digits>` (a whole rational), leaving a proper fraction
    // (`1/3`) untouched. Scan for a run of digits followed by exactly `/1` not followed by another digit.
    collapse_whole_rationals(&unbacktick)
}

/// Rewrite each `<digits>/1` occurrence (a whole rational, optionally the denominator not followed by a
/// further digit — so `1/13` is NOT touched) to just `<digits>`. Operates on the whole string so an
/// embedded quantity magnitude (`Qty.of(1500/1, …)` → `Qty.of(1500, …)`) is tidied too. A proper
/// fraction (`1/3`, `2/3`) is preserved.
fn collapse_whole_rationals(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        // At the start of a digit run? (ASCII digits only — `is_ascii_digit` is byte-safe.)
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(&s[start..i]); // the numerator digits (always emitted)
            // A `<digits>/1` where the `1` is not part of a longer denominator → drop the `/1`.
            if s[i..].starts_with("/1") && !bytes.get(i + 2).is_some_and(|c| c.is_ascii_digit()) {
                i += 2;
            }
        } else {
            // A non-digit byte: copy the whole UTF-8 char at this boundary (multibyte-safe).
            let ch = s[i..].chars().next().expect("valid char boundary");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub mod runtime;

/// The command surface (`CalcArgs` + `run`), embeddable so the unified `cdz` binary can mount
/// `cdz calc`. The standalone `cdz-calc` bin is a thin shim over it; this module owns only the
/// arg-parsing + REPL loop, never the engine above.
pub mod cli;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_value_strips_the_wrapper_and_tidies_rationals() {
        // `--plain` shows a launcher the BARE value: no `(: … type)` wrapper, no ML backticks, and a
        // whole rational `N/1` collapsed to `N`. (s-expr surface keeps the value form as-is otherwise.)
        assert_eq!(plain_value("(: 1/3 Rational)", Format::Sexpr), "1/3");
        assert_eq!(plain_value("(: 5/1 Rational)", Format::Sexpr), "5"); // whole rational
        assert_eq!(plain_value("(: 42 Int64)", Format::Sexpr), "42");
        // ML render of the rational value now resugars to `Rational.of(1, 3)` (the landed printer fix),
        // but plain mode extracts the VALUE subtree first, so it stays the bare `1/3`.
        assert_eq!(plain_value("(: 1/3 Rational)", Format::Ml), "1/3");
    }

    #[test]
    fn collapse_whole_rationals_leaves_proper_fractions() {
        assert_eq!(collapse_whole_rationals("5/1"), "5");
        assert_eq!(collapse_whole_rationals("1500/1"), "1500");
        assert_eq!(collapse_whole_rationals("1/3"), "1/3"); // proper fraction untouched
        assert_eq!(collapse_whole_rationals("1/13"), "1/13"); // /1 is a prefix of /13 → untouched
        assert_eq!(
            collapse_whole_rationals("Qty.of(1500/1, meter)"),
            "Qty.of(1500, meter)"
        );
    }

    #[test]
    fn classify_distinguishes_assignment_from_equality() {
        // `x = 5` is an assignment.
        assert!(matches!(
            classify("x = 5", Format::Ml),
            Line::Assign {
                name: "x",
                expr: "5"
            }
        ));
        // `a == b` is an equality EXPRESSION, not an assignment.
        assert!(matches!(
            classify("a == b", Format::Ml),
            Line::Expr("a == b")
        ));
        // `x <= 5` / `x >= 5` / `x != 5` are comparison expressions.
        for cmp in ["x <= 5", "x >= 5", "x != 5"] {
            assert!(matches!(classify(cmp, Format::Ml), Line::Expr(_)), "{cmp}");
        }
        // A bare expression.
        assert!(matches!(classify("1 + 2", Format::Ml), Line::Expr("1 + 2")));
        // A dotted/kebab name is a valid assignment target's ident, but `f x = …` (space in lhs) is not.
        assert!(matches!(classify("f x = 5", Format::Ml), Line::Expr(_)));
    }

    #[test]
    fn is_identifier_accepts_kebab_and_dotted() {
        assert!(is_identifier("x"));
        assert!(is_identifier("my-var"));
        assert!(is_identifier("String.scalar-len"));
        assert!(!is_identifier("5x")); // starts with a digit
        assert!(!is_identifier("a b")); // space
        assert!(!is_identifier("")); // empty
    }

    #[test]
    fn set_binding_appends_and_names_shows_each_once_newest_visible() {
        let mut calc = Calculator::new(Format::Ml);
        calc.set_binding("x", "1");
        calc.set_binding("y", "2");
        calc.set_binding("x", "9"); // re-bind x → a NEW inner let, shadows the old x
        // Two raw bindings for x are kept (append, not replace) so the shadowing let-chain forms.
        assert_eq!(calc.bindings.len(), 3, "append keeps both x bindings");
        // But `names` de-dups, newest visible: y still present, x once.
        assert_eq!(
            calc.names(),
            vec!["y", "x"],
            "distinct names, newest-first order"
        );
        // The innermost (last) x binding — the visible one — is the re-bind.
        assert_eq!(
            calc.bindings.last().unwrap(),
            &("x".to_string(), "9".to_string())
        );
    }

    #[test]
    fn wrap_in_lets_nests_oldest_outermost_per_surface() {
        // ML: `let x = 5 in let y = x + 1 in <expr>` (oldest x outermost, so y can see x).
        let mut ml = Calculator::new(Format::Ml);
        ml.set_binding("x", "5");
        ml.set_binding("y", "x + 1");
        assert_eq!(
            ml.wrap_in_lets("y * 2"),
            "let x = 5 in let y = x + 1 in y * 2"
        );
        // s-expr: `(let ((x 5)) (let ((y (+ x 1))) <expr>))`.
        let mut s = Calculator::new(Format::Sexpr);
        s.set_binding("x", "5");
        s.set_binding("y", "(+ x 1)");
        assert_eq!(
            s.wrap_in_lets("(* y 2)"),
            "(let ((x 5)) (let ((y (+ x 1))) (* y 2)))"
        );
        // No bindings → the expression is returned unwrapped.
        let empty = Calculator::new(Format::Ml);
        assert_eq!(empty.wrap_in_lets("1 + 2"), "1 + 2");
    }

    #[test]
    fn ans_rebind_shadows_via_the_let_chain_not_recursion() {
        // The regression that stack-overflowed: `ans` is stored as SOURCE, and a re-binding of `ans`
        // wraps as an INNER `let` whose right-hand `ans` resolves to the OUTER (previous) `ans` — lexical
        // shadowing, not a self-referential `def ans = ans + 5`. Assert the wrapped form has the shape
        // that shadows: an inner `let ans = ... ans ...` inside an outer `let ans = 20`.
        let mut calc = Calculator::new(Format::Ml);
        calc.set_binding(ANS, "20"); // prior result's source froze conceptually to a prior expr
        let wrapped = calc.wrap_in_lets("ans + 5");
        assert_eq!(
            wrapped, "let ans = 20 in ans + 5",
            "inner ans sees the outer ans"
        );
        // A NEW binding of ans (from evaluating `ans + 5`) APPENDS — so the chain has an outer `ans=20`
        // and an inner `ans = ans + 5`, and the inner's right-hand `ans` reads the outer. Its wrap:
        calc.set_binding(ANS, "ans + 5");
        assert_eq!(
            calc.wrap_in_lets("ans"),
            "let ans = 20 in let ans = ans + 5 in ans",
            "inner ans shadows, its rhs reads the outer ans"
        );
    }

    #[test]
    fn render_value_uses_the_display_surface_in_ml() {
        // The ML surface renders a result for DISPLAY: a rational bare (not backtick-quoted), a quantity
        // in its concise `<value> <unit>` surface, the result type annotation dropped — so a calculator
        // shows `1/4 meter/second`, not `Qty.of(`1/4`, Unit.base(#meter) / Unit.base(#second)) : Qty(…)`.
        let ml = |v: &str| render_value(v, Format::Ml);
        assert_eq!(ml("(: 1/3 Rational)"), "1/3");
        assert_eq!(ml("(: 8/1 Rational)"), "8");
        assert_eq!(
            ml(concat!(
                "(: (Qty.of 1/4 (Unit./ (Unit.base #\"meter\") (Unit.base #\"second\")))",
                "   (Qty Rational (Unit./ (Unit.base #\"meter\") (Unit.base #\"second\"))))"
            )),
            "1/4 meter/second"
        );
        assert_eq!(
            ml("(: (Qty.of 5.0 (Unit.base #\"meter\")) (Qty Float64 (Unit.base #\"meter\")))"),
            "5.0 meter"
        );
        // The s-expr surface is the canonical value form, untouched by display.
        assert_eq!(
            render_value("(: 1/3 Rational)", Format::Sexpr),
            "(: 1/3 Rational)"
        );
    }
}
