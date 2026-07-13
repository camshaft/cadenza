//! Diagnostics — a "no" as a first-class value produced where the decision is made.
//!
//! The compiler distinguishes three kinds of "no", ordered by safety
//! (`reference-compiler.md` §Outcomes Are Ordered By Safety): a **reject** (the program is
//! ill-formed — carries a stable machine-readable [`Code`]), a **decline** (the compiler does not
//! yet realize the construct — uncoded, an honest "not built yet"), and a **trap** (a run-time halt;
//! its compile-provable form is a poison, which is a reject with a code). This module gives the
//! value that carries a reject/decline; the kind is fixed at the point it is produced and carried
//! concretely, never reconstructed downstream from an artifact's shape
//! (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It Is Produced).
//!
//! The taxonomy grows one variant per added check; its `str` form is the stable `CDZ####` string a
//! tool branches on.

/// A stable, machine-readable diagnostic code. Its `code()` string is the durable identity a
/// consumer matches on; the enum variant is the compiler-internal handle.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-has-a-stable-code
//# Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Code {
    /// A LEXICAL well-formedness defect the READER detected but cannot itself report through the
    /// artifact channel (the front-end's stderr is not the diagnostic surface) — an unrecognized string
    /// escape `\q` (`collections-and-text.md` §A String Literal's Escapes Are A Closed Set). The reader
    /// emits a marker leaf (`Leaf::BadEscape`) that survives the binary AST codec; the COMPILER turns
    /// that marker into this coded rejection, so a lexically-malformed literal fails the build with a
    /// stable code rather than silently reading `\q` as the bare `q`.
    BadEscape,
    /// A char literal that names a NON-scalar code point — a surrogate (`#\u+D800`) or a value outside
    /// `U+0000..=U+10FFFF` — or is otherwise malformed. Like `BadEscape`, a LEXICAL defect the READER
    /// detected (emitting a `Leaf::BadChar` marker) but cannot itself report; the COMPILER turns the
    /// marker into this coded rejection (`collections-and-text.md` §A Char Is A Single Unicode Scalar
    /// Value). The static companion of the dynamic `(Char.from-int 55296)` → None.
    BadChar,
    /// A reference to a name with no binding in scope — the unbound-name rule, unconditional and not
    /// gated on reachability (`core-semantics.md` §Binding Is Lexical).
    Unbound,
    /// A binder position binds the same name more than once — a non-linear binder. A pattern
    /// (`core-semantics.md` §Patterns Compose: "A pattern MUST bind each name at most once … rather than
    /// silently shadowing an earlier binder") and a function's PARAMETER LIST are the same binder-linearity
    /// surface, so `(def (f x x) …)` (or `(tuple a a)` in a pattern) is this error rather than a last-wins
    /// shadow that makes the first binder unreachable.
    NonLinearBinder,
    /// A malformed or ill-typed construct that the compiler positively proves ill-formed.
    Malformed,
    /// A type mismatch (e.g. an `if` condition that is not a boolean; branches of differing type).
    TypeMismatch,
    /// Two operands of different numeric types with no explicit conversion (no silent promotion).
    NumericMismatch,
    /// An integer literal that does not fit the width its use requires.
    IntOutOfRange,
    /// A constant operation whose defined outcome on its (compile-time-known) operands is a trap —
    /// e.g. a provable overflow. A compile-provable trap fails the build rather than shipping a
    /// component that traps at run time (`reference-compiler.md` §A Compile-Provable Trap Fails The
    /// Build).
    ConstTrap,
    /// A `match` that does not cover its scrutinee — a coverage defect (the pattern engine's
    /// non-exhaustiveness rejection, distinct from a shape defect). For a scalar scrutinee this is a
    /// match with no wildcard tail; for a sum it is a missing variant (a later increment).
    NonExhaustive,
    /// A computation the compiler PROVES would trap (`ConstTrap`'s outcome) was ELIMINATED because its
    /// value is unobserved — an unprojected tuple/record element, an unreferenced `let` binding, an
    /// argument bound to an unused parameter. NOT a rejection: the build succeeds (the dead computation
    /// need not run — `core-semantics.md` §A Trap Occurs Only Where Its Computation Is Observed). This
    /// is the WARNING severity's code, emitted so a program does not silently discard a computation that
    /// could never have produced a value (almost always a defect). The error-severity companion is
    /// `ConstTrap` (CDZ0304), emitted when the same provable trap IS observed.
    DeadTrap,
    /// A binding is DECLARED but never referenced — a `let` binding, a `fn`/`def` parameter, or a
    /// top-level definition (not exported) that nothing uses. A WARNING (not a rejection): an unused
    /// binding is well-formed, just likely a defect (a typo, a leftover, a forgotten use). Suppressed
    /// when the name begins with `_` — the deliberate "intentionally unused" convention (as in Rust),
    /// so `_x`/`_` never warn. The reference check is the same resolution-column read `UsesOf` uses.
    UnusedBinding,
    /// An effect operation is reached at a point with NEITHER an enclosing handler for its effect NOR an
    /// enclosing host delegation of it — the merged "no home for a reached effect" check
    /// (`capabilities-and-effects.md` §An Ungranted Effect Is A Compile-Time Error). This single code
    /// subsumes both the reached-but-undelegated host operation and the undischarged intra-program effect
    /// (the retired CDZ0402), because host-binding is an entrypoint routing decision, not a
    /// declaration-time property — an effect reached the entrypoint's top with no home.
    EffectNoHome,
    /// A handler arm names an operation the arm's effect does not declare — a closed-set violation
    /// (`capabilities-and-effects.md` §A Handler Arm Names An Operation Its Effect Declares). An effect's
    /// operations are a closed, statically-known set (like a sum's variants), so discharging an operation
    /// that does not exist is ill-formed.
    HandlerUndeclaredOp,
    /// A host delegation names an effect the delegated computation never reaches — latent authority
    /// (`capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative). The manifest must
    /// be exactly the effects that escape, no more and no fewer, so a granted-but-unexercised capability
    /// is rejected rather than carried.
    LatentAuthority,
    /// An ILL-FORMED binary form `(bin …)` — a compile-time well-formedness defect decidable from the
    /// segment list alone (`options/binary-syntax/`): bit-fields whose widths do not close a whole byte
    /// (the whole `bin` must be byte-aligned), a non-final unsized `(bytes …)` segment, or a `bits` width
    /// that is not a compile-time constant. The binary analogue of a non-exhaustive match — a static
    /// structural rejection, not a runtime surprise (a value that does not fit its segment traps at run
    /// time instead, "binary value does not fit segment"). The CDZ02xx types-and-patterns band.
    IllFormedBinary,
}

impl Code {
    /// The stable `CDZ####` string. These are the identities a tool and the corpus branch on, so
    /// they change only by the coordinated act a code taxonomy change is.
    pub fn code(self) -> &'static str {
        match self {
            Code::BadEscape => "CDZ0001",
            Code::BadChar => "CDZ0002",
            Code::Unbound => "CDZ0101",
            Code::NonLinearBinder => "CDZ0102",
            Code::Malformed => "CDZ0201",
            Code::TypeMismatch => "CDZ0203",
            Code::NumericMismatch => "CDZ0301",
            Code::IntOutOfRange => "CDZ0302",
            Code::ConstTrap => "CDZ0304",
            Code::DeadTrap => "CDZ0305",
            Code::UnusedBinding => "CDZ0306",
            Code::NonExhaustive => "CDZ0210",
            Code::EffectNoHome => "CDZ0401",
            Code::HandlerUndeclaredOp => "CDZ0403",
            Code::LatentAuthority => "CDZ0404",
            Code::IllFormedBinary => "CDZ0220",
        }
    }
}

/// How confident the compiler is that a proposed [`Fix`] is the RIGHT edit — the branch an agent
/// reads before applying it blind (`spec/capabilities/diagnostics.md` §An Unconfirmed Fix Carries An
/// Applicability Marker). This is the rustc `Applicability` distinction, minus `Unspecified`: a fix
/// the compiler recompiled clean is `Verified` (apply it without review); a fix derived by a heuristic
/// — the nearest-name "did you mean", a wrapping suggestion — is `Heuristic` (an agent may apply it but
/// should confirm it matches intent, and a batch tool should default to leaving it for a human).
///
/// The default is `Heuristic`: a producer must positively PROVE a fix correct (by recompiling with it
/// applied) to mark it `Verified`, so an unproven fix never masquerades as machine-applicable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applicability {
    /// The compiler confirmed applying this fix recompiles the program clean and clears the diagnostic
    /// — apply it without review (`spec/capabilities/diagnostics.md` §A Confirmed Fix Is Marked
    /// Verified). Machine-applicable.
    Verified,
    /// A best-effort suggestion the compiler could NOT so confirm — a nearest-name replacement, a
    /// wrapping edit. Likely right, but an agent should confirm it matches intent before applying.
    Heuristic,
}

/// A proposed repair for a rejection, expressed as a STRUCTURAL edit of the program's AST rather than a
/// textual patch (`spec/capabilities/diagnostics.md` §A Rejection Carries A Structural Fix). This is the
/// rustc "suggestion" — the thing that makes a diagnostic actionable rather than merely descriptive: an
/// agent (or the front-end's fix-it UI) applies the named edit directly instead of re-deriving the
/// repair from prose.
///
/// The edit targets a NODE (`at`, a `StructId` — the same span-free identity a [`Reject`] anchors on),
/// so the consumer that holds the span table maps it to a text region; the compiler never emits a
/// source offset. The `replacement` is the SURFACE spelling of the node to put there (e.g. the
/// suggested name for a "did you mean", the wrapped form for a coercion) — a rendered s-expression the
/// front-end splices over the target node's span, which keeps the fix a tree edit at the point of
/// application while remaining a compact, printable payload here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fix {
    /// A one-line human label for the edit (`replace with `foo``, `wrap in `(Some …)``) — what an
    /// editor shows in its lightbulb menu. The machine-actionable part is `edit`, not this.
    pub label: String,
    /// The concrete structural edit. One variant today (replace a node's surface spelling); the enum
    /// leaves room for insert/delete/wrap edits as later checks gain fixes.
    pub edit: Edit,
    /// Whether the compiler proved this fix correct (`Verified`) or offers it as a heuristic.
    pub applicability: Applicability,
}

/// The structural edit a [`Fix`] carries. A tree operation keyed on a node identity — never a textual
/// diff (`spec/capabilities/diagnostics.md` §A Rejection Carries A Structural Fix).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// Replace the node `at` with the surface spelling `replacement` — the "did you mean `foo`?"
    /// edit (swap the misspelled reference for the candidate) and, later, a wrap/coerce edit whose
    /// `replacement` embeds the original as a sub-form.
    ReplaceNode {
        at: crate::ast::StructId,
        replacement: String,
    },
}

impl Fix {
    /// A heuristic node-replacement fix — the "did you mean `replacement`?" repair. Heuristic because
    /// the nearest-name match is a guess at intent, not a proof; an agent confirms it before applying.
    pub fn replace_heuristic(at: crate::ast::StructId, replacement: impl Into<String>) -> Fix {
        let replacement = replacement.into();
        Fix {
            label: format!("replace with `{replacement}`"),
            edit: Edit::ReplaceNode { at, replacement },
            applicability: Applicability::Heuristic,
        }
    }

    /// A VERIFIED node-replacement fix — one the producer knows is behaviour-preserving and clears the
    /// diagnostic by construction, so an agent applies it WITHOUT review (`spec/capabilities/
    /// diagnostics.md` §A Confirmed Fix Is Marked Verified). `label` states the concrete action (e.g.
    /// "prefix with `_`"). Use ONLY when the edit's correctness follows from a rule, not a guess — the
    /// caller vouches for it (there is no free lunch: an UNPROVEN edit must stay
    /// [`replace_heuristic`]).
    pub fn replace_verified(
        at: crate::ast::StructId,
        replacement: impl Into<String>,
        label: impl Into<String>,
    ) -> Fix {
        Fix {
            label: label.into(),
            edit: Edit::ReplaceNode {
                at,
                replacement: replacement.into(),
            },
            applicability: Applicability::Verified,
        }
    }
}

/// A produced "no": either a coded rejection or an uncoded decline, each carrying a human message and
/// the AST node it is about. The `code` is `Some` for a rejection/poison (an ill-formed program) and
/// `None` for a decline (a construct the compiler does not yet realize) — the branch a downstream sink
/// must preserve rather than collapse (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It
/// Is Produced).
///
/// **The origin is a node identity, not a span.** `at` is the `StructId` the fault is about. The
/// compiler holds NO source positions — a `StructId` is a stable node identity, and the FRONT-END
/// (which parsed the text into the binary AST and holds the span table keyed by that same identity)
/// maps the id back to a text region (`query-engine.md` §Provenance Is Recovered By Back-Reference).
/// So the whole compiler — and its eventual Cadenza port — stays span-free: a diagnostic names a node,
/// the application layer resolves it to a region. Program node ids are preserved through decode (the
/// prelude appends its nodes AFTER the program's), so an `at` on a program node IS the id the
/// front-end's span table is keyed by. `None` = an un-anchored "no" (a synthesized node, or a
/// producer that did not stamp one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reject {
    /// `Some(code)` = a rejection (ill-formed); `None` = a decline (not yet built).
    pub code: Option<Code>,
    pub message: String,
    /// The AST node this "no" is about — the front-end maps it to a text region. `None` if unstamped.
    pub at: Option<crate::ast::StructId>,
    /// A proposed structural repair, if the producer knows one — the "route to a fix"
    /// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). `None` when the
    /// compiler has no actionable suggestion (most declines, and rejections whose repair is not
    /// mechanical). Carried alongside the message so a consumer applies the edit rather than parsing
    /// prose.
    pub fix: Option<Fix>,
}

impl Reject {
    /// A coded rejection: the program is ill-formed, and this is why. Unanchored — a caller with the
    /// faulting node in hand attaches it via [`Reject::at`] (or a collector stamps it with
    /// [`Reject::set_origin_if_absent`]).
    pub fn coded(code: Code, message: impl Into<String>) -> Reject {
        Reject {
            code: Some(code),
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// An uncoded decline: a construct the compiler does not yet realize. NOT a statement that the
    /// program is wrong — it is the compiler declining to compile it, the safe outcome
    /// (`reference-compiler.md` §Outcomes Are Ordered By Safety).
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject {
            code: None,
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// Attach a proposed structural fix — the fluent form a producer uses when, alongside the "no", it
    /// can name the repair: `Reject::coded(..).at(id).with_fix(Fix::replace_heuristic(id, "foo"))`.
    pub fn with_fix(mut self, fix: Fix) -> Reject {
        self.fix = Some(fix);
        self
    }

    /// Attach (or replace) the node this "no" is about — the fluent form a producer uses when it holds
    /// the precise faulting node: `Reject::coded(..).at(id)`.
    pub fn at(mut self, id: crate::ast::StructId) -> Reject {
        self.at = Some(id);
        self
    }

    /// Stamp the origin node ONLY if none is set yet — so a precise producer's anchor is never
    /// overwritten, while a collector walking node `id` can supply a default origin for a "no" that
    /// was produced without one. The innermost frame that has the faulting node wins.
    pub fn set_origin_if_absent(&mut self, id: crate::ast::StructId) {
        if self.at.is_none() {
            self.at = Some(id);
        }
    }

    /// Whether this "no" is a decline (uncoded) rather than a coded rejection.
    pub fn is_decline(&self) -> bool {
        self.code.is_none()
    }
}

/// The shared "did you mean?" machinery — the ONE nearest-name search every suggestion draws on
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). A producer that
/// rejected an unknown name (an unbound reference, an absent record field, a mistyped variant) hands
/// this its candidate set — the names that WOULD have been valid there — and gets back the nearest
/// plausible typo, or `None` when nothing is close enough (a false suggestion is worse than none: an
/// agent would apply the wrong edit). Kept in `diag` so resolve/infer/… share one implementation and
/// one cutoff rather than each rolling its own.
pub mod suggest {
    /// Pick the closest of `candidates` to `name` under a length-relative edit-distance cutoff, or
    /// `None` if none is close enough. The cutoff (`max(1, len/3)`, rustc's `find_best_match_for_name`
    /// heuristic) keeps a suggestion only when the candidate is a plausible typo: a 3-char name tolerates
    /// 1 edit, a 9-char name up to 3. Ties break on the smaller distance, then the
    /// lexicographically-smaller name, so the result is a DETERMINISTIC function of the candidate SET —
    /// independent of the order they are supplied in (a hash-map iteration order never leaks through).
    pub fn nearest<I, S>(name: &str, candidates: I) -> Option<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let name_len = name.chars().count();
        let max_dist = (name_len / 3).max(1);
        let mut best: Option<(usize, String)> = None;
        for cand in candidates {
            let cand = cand.as_ref();
            // Never suggest the name itself (a shadowed / out-of-scope exact match is not a typo), nor
            // the wildcard.
            if cand == name || cand == "_" {
                continue;
            }
            let d = edit_distance(name, cand);
            if d > max_dist {
                continue;
            }
            let better = match &best {
                None => true,
                Some((bd, bn)) => d < *bd || (d == *bd && cand < bn.as_str()),
            };
            if better {
                best = Some((d, cand.to_string()));
            }
        }
        best.map(|(_, n)| n)
    }

    /// Levenshtein–Damerau edit distance (insertions, deletions, substitutions, and ADJACENT
    /// TRANSPOSITIONS) between two names, in Unicode scalar values. Transpositions count as one edit so a
    /// `fodl`→`fold` swap reads as a single typo. O(a·b) time, O(b) space — names are short, so this is
    /// negligible and only runs on a reject path.
    pub fn edit_distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }
        // Two rolling rows would suffice for plain Levenshtein; the transposition rule needs the row two
        // back, so keep three rows.
        let mut prev2: Vec<usize> = vec![0; b.len() + 1];
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut cur: Vec<usize> = vec![0; b.len() + 1];
        for i in 1..=a.len() {
            cur[0] = i;
            for j in 1..=b.len() {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
                // Adjacent transposition: `a[i-1] a[i-2]` swapped matches `b[j-2] b[j-1]`.
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    v = v.min(prev2[j - 2] + 1);
                }
                cur[j] = v;
            }
            std::mem::swap(&mut prev2, &mut prev);
            std::mem::swap(&mut prev, &mut cur);
        }
        prev[b.len()]
    }
}
