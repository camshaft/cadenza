# Design — parser/printer golden test corpus (`spec/syntax/`)

*Owning area: `cadenza-syntax` (the surface front-end) + the corpus harness. Coordinate with
`v-syntax` / `v-syntax-nonrec-reader` (the parsers/printers being rewritten), `v-syntax-comments`
(comment-in-tree + fmt-canonical), and `v-corpus-harness` (grader/baseline patterns).*

## 0. The crux — a language-agnostic golden corpus for the PARSERS/PRINTERS, so they can be rewritten in Cadenza

The semantics corpus (`spec/semantics/*.sexp`) made the compiler's *behavior* language-agnostic: one
runnable oracle of `input program → output value`, so the compiler can be re-implemented and validated
against a neutral golden set (see `spec/semantics/README.md`, `design/DESIGN-c1-diagnostic-quality-corpus-migration.md`).

This design does the same thing one level up, for the **surface layer** — the parsers and printers that
today live in the `cadenza-syntax` crate, explicitly *"the decoupled ML front-end… REFERENCE
implementation destined to be rewritten in Cadenza"* (`cadenza-syntax/src/lib.rs:1-9`). We build a
golden corpus of:

- an **input** in some surface (ML text, s-expression text, …),
- its **expected parse tree** — the arena the parser produces, rendered as a canonical s-expression
  with comments preserved as explicit tree nodes,
- an **optional expected canonical format** — present only when the input was not already
  well-formatted.

The corpus is the neutral oracle. Today the reference `cadenza-syntax` (Rust) parser/printer produces
and validates the goldens. Tomorrow a Cadenza-written parser/printer must reproduce the **identical**
goldens, byte-for-byte — same corpus, swapped implementation-under-test. That is the whole point: this
corpus is the acceptance gate that makes the parser rewrite *"really easy in the future"* (operator,
2026-08-30).

**And it REPLACES the in-crate tests, not just supplements them** (operator, 2026-08-30). The
parser/printer crates carry **~889 `#[test]`s** today (`cadenza-syntax` 638, `cadenza-syntax-sexpr` 58,
`cadenza-syntax-core` 45, `cadenza-ast` 99, plus the json/toml/cedar readers). The behavioral ones —
"this input parses to this tree", "this input formats to this text", round-trip assertions — must be
**migrated into this corpus** as language-agnostic cases (one `#[test]` → one case directory), exactly
as the compiler tests were delanguaged (`design/DESIGN-c1-diagnostic-quality-corpus-migration.md`). Only
genuinely-internal unit tests stay as Rust `#[test]`s (§6, Increment 6 spells out the split). After
migration, a parser/printer behavior lives in ONE language-neutral place, not in Rust assertions the
Cadenza rewrite could never reuse.

**What this is NOT.** It is not a second semantics corpus. It never runs a program or checks a runtime
value. It pins *syntax* — what tree the parser builds and what text the printer emits — and stops at
the front-end boundary. The heavy desugar to Core (`handle`→projection, `match`→`let`, …) happens
downstream in `rcdzc` and is already covered by the semantics corpus; this corpus deliberately stays
above that line.

## 1. Surface — directory-per-case under `spec/syntax/`

Each test is a **directory** (operator's proposed shape, confirmed 2026-08-30). A parser corpus needs
the input as a real file so its bytes — significant whitespace, trailing newlines, its own comments —
are exact and not smuggled through an s-expression string literal:

```
spec/syntax/<surface>/NN-name/
  input.<ext>      # the surface source, byte-exact  (required)
  tree.sexp        # the expected parse tree, canonical s-expression  (required)
  format.<ext>     # the expected canonical format   (OPTIONAL — see §3)
```

- `<surface>` groups cases by surface (`ml/`, `sexp/`, later `json/` `toml/` `cedar/` `md/`). The
  surface is also implied by the input extension, so the grader is surface-generic.
- `NN-name` is a zero-padded ordinal + short slug (`03-let-binding`), for stable reading order, exactly
  as `spec/semantics/NN-feature.sexp` numbers its files.
- `input.<ext>` uses the surface's real extension (`.cdz`/`.ml` for ML, `.sexp` for s-expr) — the same
  extension→`Format` mapping the CLI already uses (`cadenza-syntax/src/convert.rs:98-104`).
- `tree.sexp` is always `.sexp` regardless of the input surface — the tree is *one* representation for
  *all* surfaces (§2), which is what makes the corpus language-agnostic.

This is a sibling to `spec/semantics/`, not a replacement. The two corpora answer different questions
and are graded by different comparisons.

## 2. The parse-tree golden — `tree.sexp`

`tree.sexp` is the **structural** s-expression rendering of the arena the parser produces for
`input.<ext>`. Two decisions fix its exact shape (operator, 2026-08-30), both verified against the
current reference impl:

1. **It is our normal s-expression form — no synthetic wrappers.** All six surfaces build the *same*
   two-arena `cadenza-ast` (`(head child…)`, keywords-are-data — `cadenza-ast/src/ast.rs:17-20`), so the
   tree is that arena, printed. It does NOT introduce an `(op …)` layer for infix — infix desugars to a
   plain operator-headed list, precedence and all. Verified:
   - `1 + 2 * 3` → `(+ 1 (* 2 3))`
   - `foo.bar(1, 2)` → `((. foo bar) 1 2)`
   - `let x = 1 + 2 * 3 in x` → `(let ((x (+ 1 (* 2 3)))) x)`
   (`sexpr` reader and `ml` reader produce byte-identical arenas — this is exactly what makes the corpus
   language-agnostic.)

2. **Comments are EXPANDED into explicit tree nodes — never s-expr line-comments.** This is the crux the
   operator flagged, and it turns out to require a small piece of new work, so it is Increment 1. The
   facts:
   - Comments are already first-class arena nodes (`cadenza-syntax/src/parser.rs:112-115` "Comments no
     longer vanish"): `(comment "text" form)` (leading own-line), `(comment-after "text" form)`
     (trailing same-line), and `(doc "text")` (a `///` item-doc the def/module parser splices as a body
     form). Verified — the `convert --to debug` view of `// start value\nlet x = 10 in x  // in scope`
     shows the arena as `(comment "in scope" (comment "start value" (let ((x 10)) x)))`.
     - **NOTE (corrected 2026-08-30, v-syntax the `cadenza-syntax` owner): there is NO `(module-doc …)`
       node and NO `//!` handling in the reference ML reader.** The lexer recognizes exactly two comment
       kinds — `//` (→ `comment`) and `///` (→ item-`doc`); a `//!` lexes as an ordinary `//` comment
       whose body starts with `!` (e.g. `//! header` → `(comment "! header" …)`, pinned by
       `spec/syntax/ml/09-bang-comment`). A file/module-level `(module-doc …)` (the natural `//!`
       parallel to `///` item-doc) would be a SEPARATE, operator-gated syntax feature — not implemented,
       not queued. (An earlier draft of this doc claimed `//!` → `(module-doc "text")` "verified"; that
       was inaccurate and is retracted here.)
   - **But the current s-expression *printer* collapses those comment nodes back to `;` line-comments**
     (verified: `convert --to sexpr` of the same input prints `; in scope` / `; start value` lines above
     `(let ((x 10)) x)`, and even feeding an explicit `(comment "…" …)` list *back* through the printer
     re-collapses it to `;`). A golden written with `;` lines is no good: `;` is **trivia** the reader
     could drop, so those comments would not be part of the compared structure.
   - The golden must therefore be the **structural** form — comment nodes printed as ordinary
     `(comment "text" form)` / `(comment-after "text" form)` / `(doc …)` lists:

     ```
     ;; input.cdz
     // start value
     let x = 10 in x  // in scope

     ;; tree.sexp  (structural: comment nodes are ordinary lists, NOT ; trivia)
     (comment "in scope"
       (comment "start value"
         (let ((x 10)) x)))
     ```
   - This form **re-reads to the identical arena** — verified: `(comment "start value" …)` fed to the
     `sexpr` reader parses to the same `comment` node a `;`/`//` comment produces. So the golden is
     unambiguous and round-trippable; we reuse the existing node vocabulary exactly and invent nothing.
   - **The one build item:** a *structural* s-expression print mode (comment nodes rendered as lists, not
     `;`) — either a flag on `cadenza-syntax-sexpr`'s pretty-printer or a thin arena→sexpr walker in the
     corpus generator that treats `comment`/`comment-after`/`doc`/`module-doc` as ordinary heads. It is
     small (the `debug` view already walks the arena to this exact structure) and is the corpus's
     canonical `render_sexpr`.

**Comparison.** Byte-exact equality of the reference-produced structural `tree.sexp` against the golden
on disk. Because both surfaces feed the *same* arena, an `sexp/` case's `tree.sexp` and an equivalent
`ml/` case's `tree.sexp` are literally identical files — the corpus makes surface-independence auditable.

## 3. The optional canonical-format golden — `format.<ext>`

The formatter (`fmt`) already re-prints a surface canonically and is idempotent by contract
(`cadenza-syntax/src/printer.rs:17`; `fmt` treats "already canonical" as `formatted == input` byte
compare, `cadenza-syntax/src/cli.rs:815`). The corpus reuses that notion directly (operator confirmed
"present iff not-canonical", 2026-08-30):

- **`format.<ext>` present** → grader asserts `fmt(input) == format.<ext>` (bytes). Use this when the
  input is deliberately messy (bad indentation, extra blank lines) to pin how it canonicalizes.
- **`format.<ext>` absent** → grader asserts `fmt(input) == input` (bytes). The input is thereby
  asserted to be *already canonical*. This keeps the corpus minimal (no redundant format file for the
  common already-clean case) and doubles as a free fmt-idempotence guard on every clean input.

`format.<ext>` is in the input's surface (an `ml/` case formats to `.cdz`), because `fmt` re-prints in
the *same* surface — it never changes surface and never changes arena shape (the `Normalize` codemod is
a separate opt-in, `cadenza-syntax/src/cli.rs:84-87`, and is out of scope here).

## 4. The grader — verdicts, baseline, selection

A dedicated syntax grader that **follows `v-corpus-harness`'s patterns** rather than reinventing them.
It is a sibling to the semantics `gate()` (`xtask/src/main.rs:3691`), not an extension of it, because
the comparison is fundamentally different (byte-exact tree/format vs. runtime value) and the on-disk
layout is directory-per-case vs. flat files. What it reuses:

- **Enumerate** `spec/syntax/**/` case directories; `--files <dir>`/`--case <needle>` selection mirrors
  the semantics gate's flags.
- **Verdicts — the same additive `Pass`/`Todo`/`Fail` ladder** (`cdz-corpus-grade/src/lib.rs:36`):
  - `Pass` — reference output matches `tree.sexp` (and `format` per §3).
  - `Todo` — the parser *declines* the construct (a surface/feature it does not yet realize): a clean
    error, not a wrong tree. Never a false fail. This is the delanguaging move (semantics README
    §"reject-don't-miscompile") applied to syntax: a not-yet-built parser feature declines → `Todo`,
    it does not miscompile.
  - `Fail` — reference produced a *wrong* tree/format (mismatch), or an ICE/panic.
- **Baseline — additive/regression-only** (`spec/syntax/.gate-baseline`, same format + `merge=union`
  as `spec/semantics/.gate-baseline`, `xtask-support/src/lib.rs:807`): fail only on `Pass →
  not-Pass` regression or a vanished case; new cases and `Todo→Pass` gains report but never fail.

The exact wiring (new `cargo xtask gate-syntax` subcommand vs. a `--kind syntax` mode on the existing
gate; whether to shell a small `cdz-syntax records`-style shredder or read files directly) is an
implementation fork to settle *with* `v-corpus-harness` at Increment 2 — the default is a thin new
subcommand that links the shared `cdz-corpus-grade` verdict/baseline library.

### 4.1 Each case is its OWN nix derivation — parallel + cached (operator, 2026-08-30)

Every case directory grades as an **independent nix derivation**, so cases run in parallel and each is
content-addressed/cached — a case whose inputs are unchanged is never re-run. This mirrors the
semantics corpus's per-case nix caching (`design/DESIGN-corpus-nix-per-case-caching.md`,
`gate_via_nix_cache` at `xtask/src/main.rs:3618`), and directory-per-case (§1) makes it clean: each
`spec/syntax/<surface>/NN-name/` directory is one derivation.

- **Derivation inputs (the cache key):** the case directory's bytes (`input.<ext>`, `tree.sexp`,
  optional `format.<ext>`) + the reference tool (the `cdz`/`cadenza-syntax` binary the flake already
  builds). Change any of those → that one case's derivation re-runs; change none → cache hit, no work.
  Editing case `47` never invalidates case `48`.
- **Derivation body (per case):** run the reference `render_sexpr(read(input))` and compare to
  `tree.sexp`; run `fmt(input)` and compare to `format.<ext>` (or to `input` if absent, §3); emit the
  case's `Pass`/`Todo`/`Fail` verdict as the derivation output.
- **The aggregate check** (`.#checks.<arch>-linux.syntax-corpus`, the parallel of the semantics
  corpus check) depends on all per-case derivations and folds their verdicts against
  `spec/syntax/.gate-baseline` (§4) — additive/regression-only. Nix schedules the leaves in parallel
  under the fleet's build budget; unchanged leaves are free.
- **Generation of a leaf list** follows the semantics per-case pattern (a nix expression enumerating
  the case directories → one derivation each). Reuse that generator shape rather than inventing a new
  one; coordinate with `v-corpus-harness` (owner of the per-case caching machinery).

This is why directory-per-case (not one flat file) is the right on-disk shape: independent directories
give independent derivation cache keys for free.

## 5. Relationship to the binary-AST and the semantics corpus

**Binary-AST is THE data-exchange format (standing directive) — this corpus does not violate that.**
The parser produces an arena. That arena has two renderings: the **binary-AST** (the exchange form, how
tools hand trees to each other) and the **s-expression** (the human-auditable surface form). `tree.sexp`
is the *human golden* — readable, diffable, editable in review — and it is a *total function of the same
arena* the binary-AST encodes. So asserting on `tree.sexp` transitively pins the binary-AST too, without
committing opaque binary blobs to the corpus (which would be unreviewable). Golden = s-expression;
exchange = binary-AST; both are views of one arena.

**vs. the semantics corpus.** `spec/semantics/` asserts `input → runtime value` (compiler + runtime).
`spec/syntax/` asserts `input → parse tree` and `input → canonical format` (front-end only). A single
construct may appear in both, testing different layers. This corpus never executes anything.

## 6. Increments (top-to-bottom, each gated)

1. **Structural s-expression print mode** (`render_sexpr`, §2). Add the comment-expanding structural
   printer (comment/`comment-after`/`doc`/`module-doc` nodes as ordinary lists, not `;` trivia) — a flag
   on `cadenza-syntax-sexpr`'s printer or a thin arena→sexpr walker. Gate: `cargo test -p cadenza-syntax
   --lib` — structural render of a comment-bearing arena re-reads to the identical arena. This is the
   corpus's canonical golden-generation function and unblocks everything else.
2. **Corpus skeleton + one worked case per surface.** Create `spec/syntax/ml/` and `spec/syntax/sexp/`
   with a handful of hand-authored cases (a literal, a `let`, a comment-bearing case, one messy-format
   case) whose `tree.sexp` is generated by Increment-1's `render_sexpr`. Gate: goldens self-consistent
   (`render_sexpr(read(input)) == tree.sexp`; `fmt(input) == format`-or-`input`) via a `cargo test -p
   cadenza-syntax` unit test — no new harness yet. Proves format + comment-expansion on real output.
3. **The syntax grader + baseline + per-case nix derivations** (coordinate `v-corpus-harness`). Add the
   enumerate→compare→verdict driver, `spec/syntax/.gate-baseline`, `--files`/`--case` selection, AND the
   per-case nix derivation shape + `.#checks.<arch>-linux.syntax-corpus` aggregate (§4.1). Gate:
   `Pass`/`Todo`/`Fail` tally on the Increment-2 cases, each as its own cached derivation; baseline
   committed; the aggregate check (advisory) runs it.
4. **Seed breadth for ML + sexpr.** Grow the corpus to cover the surface grammar (bindings, control,
   operators/precedence, functions, records/sums, patterns, doc + comment placements, malformed inputs
   that should decline). Each is one directory (= one derivation); messy inputs get a `format.<ext>`.
   Gate: grader green, baseline additive.
5. **Format goldens + fmt-idempotence sweep.** Ensure every clean case exercises the `format`-absent
   idempotence assertion and every messy case has a pinned `format`. Gate: grader green.
6. **Migrate the in-crate parser/printer tests (the bulk of the effort).** Walk the ~889 `#[test]`s in
   the syntax crates and, for each that asserts a *behavior* (input→tree, input→format, round-trip),
   convert it to a case directory (one `#[test]` → one case, the `DESIGN-c1-…-corpus-migration.md`
   recipe). The split:
   - **Migrate → corpus:** parse-tree assertions, fmt/round-trip assertions, decline/parse-error
     assertions (→ `Todo` + optional `error.txt`, §10). These are exactly what the Cadenza rewrite must
     re-satisfy, so they belong in the neutral corpus.
   - **Keep as Rust `#[test]`:** genuinely-internal unit tests with no language-neutral surface — arena
     builder/helper invariants (`cadenza-ast`), `SpanTable` totality, the "printer is iterative not
     recursive" structural guards (`cadenza-syntax-sexpr/src/lib.rs:2552,2577`), lexer-internal edge
     cases. These test the *implementation*, not the *language*, and a Cadenza rewrite would write its
     own; do NOT force them into the corpus.
   Land it **incrementally, per test-module**, each batch its own green MR (a `#[test]` deleted in the
   same commit its corpus case lands, so coverage never dips), mirroring the per-submodule migration the
   compiler-test shrink used. Gate per batch: grader green + `cargo test -p <crate> --lib` still green on
   the retained unit tests. This is the largest, longest increment and the point of the whole design.
7. **(Deferred, operator-gated) Other surfaces + the acceptance harness for the Cadenza rewrite.** Add
   `json/`/`toml/`/`cedar/`/`md/` cases (free from the generic harness), and wire the corpus as the
   validation gate for the Cadenza-written parser/printer once that rewrite starts (§9). Whether the
   rewrite covers the data readers is a separate operator scope question (§10).

## 7. Seams / file anchors (landmarks at 2026-08-30)

- **Surface front-end / reference impl:** `cadenza-syntax` crate. ML reader/printer
  `cadenza-syntax/src/parser.rs` (recursive-descent Pratt — the one still awaiting the non-recursive
  rewrite), `cadenza-syntax/src/printer.rs` (idempotent by contract, `:17`). Owner: `v-syntax` /
  `v-syntax-nonrec-reader`.
- **s-expression reader/printer:** `cadenza-syntax-sexpr/src/lib.rs` — `read`/`read_spanned` (`:52`,
  `:69`) build the arena; `print`/`print_from` (`:162`,`:170`) and `print_pretty*` (`:298-326`) render
  it. Already converted to an iterative worklist (`read_node`, `:1163`). **This printer defines the
  golden `tree.sexp` shape.**
- **Arena / AST:** `cadenza-ast/src/ast.rs` — two arenas, `enum Struct { Atom | List }` (`:1-20`);
  comment-peel helpers `comment_wrapped_form` (`:1859`), `peel_comments` (`:1875`).
- **Comment nodes:** produced at `cadenza-syntax/src/parser.rs:112-115`, attachment logic `:93-150`,
  `take_comments_here` (`:638`), `take_trailing_comment_here` (`:658`). Owner: `v-syntax-comments`.
- **fmt / canonical format:** `cadenza-syntax/src/cli.rs` — `Cmd::Fmt` (`:67`), `run_fmt` (`:754`),
  already-canonical byte compare (`:815`). CI gate `cdz-fmt-check` (driver
  `xtask/crates/xtask-fmt/src/main.rs`).
- **Extension→surface map:** `cadenza-syntax/src/convert.rs:98-104` (`Format` enum `:22`).
- **Harness patterns to mirror:** semantics `gate()` `xtask/src/main.rs:3691`; verdict/baseline library
  `implementation/seed/crates/cdz-corpus-grade/src/lib.rs`; baseline (de)serialize
  `xtask/crates/xtask-support/src/lib.rs:807`. Owner: `v-corpus-harness`.
- **Per-case nix caching to mirror:** `design/DESIGN-corpus-nix-per-case-caching.md`; the cached path
  `gate_via_nix_cache` `xtask/src/main.rs:3618`. The syntax corpus reuses this leaf-per-case shape (§4.1).
  Owner: `v-corpus-harness`.
- **Structural print (new, Increment 1):** the comment-expanding golden renderer, added to
  `cadenza-syntax-sexpr/src/lib.rs` (printer, `:298-326`) as a mode, or a small arena walker; the
  `convert --to debug` view (`cadenza-syntax/src/debug.rs`) already walks the arena to the target
  structure and is the reference for what to emit as canonical s-expression.
- **New:** `spec/syntax/` (corpus root), `spec/syntax/README.md` (the syntax-corpus DSL doc, mirroring
  `spec/semantics/README.md`), `spec/syntax/.gate-baseline`, and the grader subcommand.

## 8. The gate that protects it

- Increment 1: `cargo test -p cadenza-syntax --lib` — the self-consistency check on the seed goldens.
- Increment 2+: the syntax grader, additive against `spec/syntax/.gate-baseline`, runnable as `cargo
  xtask gate-syntax --files <dir>` for a scoped spot-check and whole-corpus for CI (advisory, per the
  hourly-advisory land model).
- Per the fleet land model: agents iterate on `dev-gate` + the scoped `gate-syntax` spot-check;
  `gate-local` remains the authoritative required-set before a direct-to-main land.

## 9. How it drives the future Cadenza-parser rewrite

The corpus is the **acceptance gate** for the rewrite. For every case, the implementation-under-test
must satisfy:

```
render_sexpr( parse_<surface>(input.<ext>) )  ==  tree.sexp          (bytes)
fmt_<surface>(input.<ext>)                    ==  format.<ext>        (bytes; or == input if absent)
```

- **Today** the implementation-under-test is the reference `cadenza-syntax` (Rust) parser/printer — it
  generates the goldens, so the corpus is proven against the current, trusted impl.
- **Tomorrow** the implementation-under-test is the Cadenza-written parser/printer. It must reproduce
  the identical goldens. Nothing about the corpus changes; only the binary the grader drives changes.
  A construct the new parser has not yet reached *declines* → `Todo`, so the rewrite lands
  incrementally (feature by feature, `Todo`→`Pass`) exactly like the delanguaging effort — never a
  flag-day.

This is the same "one oracle, swappable implementation" property the semantics corpus gave the compiler
(`spec/semantics/README.md`).

## 10. Decisions deferred (chosen defaults; open forks routed to operator)

- **Grader wiring** — new `cargo xtask gate-syntax` subcommand (default) vs. a mode on the existing
  gate. Settle with `v-corpus-harness` at Increment 2. Default chosen; not blocking.
- **Surface scope of the *rewrite*** — the operator has a separate pending scope question ("does
  everything include the other readers" — JSON/TOML/Cedar/Markdown). The *corpus harness* is
  surface-generic regardless; only which surfaces we *seed* and when depends on that answer. Seeded ML
  + sexpr first (Increment 1-4); the rest is Increment 5, operator-gated.
- **Malformed-input cases** — how the corpus expresses "this input is *supposed* to fail to parse":
  default is a `Todo`/decline verdict plus an optional `error.txt` pinning the diagnostic code
  (mirroring semantics `(error CDZ…)`), so the parser corpus can also pin *parse-error quality*. To be
  firmed up with `v-syntax` when Increment 3 reaches malformed inputs.
- **`tree.sexp` for markdown/literate inputs** — markdown embeds code blocks as arena subtrees
  (`cadenza-syntax/src/markdown.rs`); its `tree.sexp` is just that document arena printed. No special
  case needed, but noted for Increment 5.
