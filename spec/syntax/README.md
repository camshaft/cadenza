# Parser/printer golden corpus (`spec/syntax/`)

This directory is a **language-agnostic golden corpus for the surface layer** — the parsers and
printers that today live in the `cadenza-syntax` crate (*"the decoupled ML front-end… REFERENCE
implementation destined to be rewritten in Cadenza"*). It does for the **front-end** what
[`spec/semantics/`](../semantics/README.md) does for the compiler's *behavior*: it makes one neutral,
runnable oracle so the parsers/printers can be **re-implemented in Cadenza and validated against the
same goldens, byte-for-byte** — swap the implementation-under-test, keep the corpus.

See the design: [`implementation/design/DESIGN-parser-test-corpus.md`](../../implementation/design/DESIGN-parser-test-corpus.md).

**What this is NOT.** It is not a second semantics corpus. It never runs a program or checks a runtime
value. It pins *syntax* — what tree the parser builds and what text the printer emits — and stops at
the front-end boundary. The desugar to Core and everything downstream is `spec/semantics/`'s job.

## The form: directory-per-case

Each case is a **directory** `spec/syntax/<surface>/NN-name/`:

```
spec/syntax/<surface>/NN-name/
  input.<ext>      # the surface source, byte-exact               (required)
  tree.sexp        # the expected STRUCTURAL parse tree            (required)
  format.<ext>     # the expected canonical format                 (OPTIONAL — see below)
```

- `<surface>` groups cases by surface: `ml/` (`.cdz`/`.ml`), `sexp/` (`.sexp`), later `json/` `toml/`
  `cedar/` `md/`. The surface is implied by `input`'s extension.
- `NN-name` is a zero-padded ordinal + short slug (`03-let`), for stable reading order.
- `tree.sexp` is always `.sexp` regardless of the input surface — the parse tree is *one*
  representation for *all* surfaces, which is what makes the corpus language-agnostic. An `ml/` case
  and an equivalent `sexp/` case have **byte-identical** `tree.sexp` (compare `ml/02-arith-precedence`
  and `sexp/02-arith-precedence`).
- **Surface-independence convention (gated):** a case with the SAME slug (the dir name minus its
  leading `NN-` ordinal) in more than one surface is a **parity twin**, and its `tree.sexp` MUST be
  byte-identical across those surfaces — enforced by `matching_slug_cases_are_surface_independent` in
  `syntax_corpus_tests`. So two cases that are *intentionally different* constructs must use *distinct
  slugs*; a shared slug is a promise of identical parse trees.

## `tree.sexp` — the structural parse-tree golden

`tree.sexp` is `render_sexpr(read(input))` — the arena the parser produces, rendered as a canonical
s-expression, with **comments expanded to explicit tree nodes** (`(comment "text" form)`,
`(comment-after "text" form)`, `(doc "text")`, `(module-doc "text")`) rather than collapsed to `;`
trivia. A `;` is trivia the reader may drop, so a golden written with `;` would not pin the comment as
part of the compared tree; the structural form makes every comment part of the arena the golden pins,
and it re-reads to the identical arena. Infix desugars to a plain operator-headed list with precedence
(no synthetic `(op …)` layer): `1 + 2 * 3` → `(+ 1 (* 2 3))`.

## `format.<ext>` — the optional canonical-format golden

The formatter (`cdz fmt`) re-prints a surface canonically and is idempotent by contract.

- **`format.<ext>` present** → assert `fmt(input) == format.<ext>` (bytes). Use it when `input` is
  deliberately messy (extra spaces, bad indentation) to pin how it canonicalizes.
- **`format.<ext>` absent** → assert `fmt(input) == input` (bytes): `input` is thereby asserted to be
  *already canonical*. This keeps clean cases minimal and doubles as a free fmt-idempotence guard.

`fmt` here is "read the surface, re-print it in the SAME surface, terminate with one newline" — exactly
what `cdz fmt` compares against.

## `normalize.<pass>.<ext>` — the optional codemod golden

Beyond parse + format, the corpus also pins surface **codemods** — the opt-in `cdz normalize` rewrites
(distinct from `fmt`), so the language's canonicalizations are *specified*, not just implemented, and
the Cadenza rewrite must reproduce them.

- A `normalize.<pass>.<ext>` file (e.g. `normalize.match-to-let.sexp`) asserts `cdz normalize --<pass>
  --stdout input.<ext> == normalize.<pass>.<ext>` (bytes). It is **same-surface** (like `format`): the
  codemod edits the arena, never changes surface. `<pass>` is the CLI flag name (`match-to-let` →
  `--match-to-let`); a case may carry several (one per pass it exercises).
- A case with a codemod golden still has its ordinary `tree.sexp` (the *unchanged* parse tree of the
  input) + optional `format` — the `normalize` golden is an *additional* comparison, not a replacement.
  Example: `sexp/32-match-to-let-tuple` — input `(match v ((tuple a b) (+ a b)))`, `tree.sexp` that same
  match arena, `normalize.match-to-let.sexp` = `(let (((tuple a b) v)) (+ a b))`.

Graded by `gate-syntax` (+ the per-case nix gate). This is the operator-blessed **codemod corpus** — a
follow-on that grows as more `cdz normalize` passes land.

## The gate

- **`cargo xtask gate-syntax`** — the corpus grader. It drives the reference `cdz` tool over every
  case and prints an additive `pass`/`todo`/`fail` verdict per case:
  - `pass` — `tree.sexp` and the format both match.
  - `todo` — the reader DECLINES the input (a clean parse error): a not-yet-realized surface/feature,
    never a false fail. (This is what lets the future Cadenza-parser rewrite land feature-by-feature:
    a construct it hasn't reached declines → `todo`, it does not miscompile.)
  - `fail` — a wrong tree/format, a missing `tree.sexp`, or an ICE.

  Flags mirror the semantics `gate`: `--case <substr>` / `<case-dir>…` select a subset, `--save`
  rewrites `.gate-baseline` from the current verdicts, `--check` compares to the baseline and fails on
  a regression. Only `pass → not-pass` regresses; `todo → pass` is a silent additive tighten. A FULL
  run (`--check` with no selection) also reds a **vanished** case — a baseline title with no
  corresponding case (a silently dropped/renamed case); a subset run skips that check.

- **`spec/syntax/.gate-baseline`** — the committed per-case verdicts (`<verdict>\t<title>`, sorted,
  `merge=union`), identical in shape to `spec/semantics/.gate-baseline`.
  **⚠ Durable hygiene rule:** a PR that RENAMES a case (its `<surface>/<name>` directory) or FLIPS a
  verdict MUST co-update `.gate-baseline` in the SAME PR — otherwise the vanished/regression check
  reds. Regenerate with `cargo xtask gate-syntax --save`.

- **Self-consistency check** — `cadenza-syntax/src/syntax_corpus_tests.rs` (a `#[cfg(test)] mod`, NOT a
  `tests/*.rs` binary — the no-integration-tests mandate is zero-tolerance) also enforces the two
  equalities against the reference reader/printer (a bootstrapping guard). Run it with `cargo test -p
  cadenza-syntax --lib syntax_corpus_tests`; regenerate goldens after editing an input (or adding a
  case) with `CDZ_BLESS=1 cargo test -p cadenza-syntax --lib syntax_corpus_tests`.
- **Per-case nix gate** — `.#checks.<arch>-linux.syntax-corpus`: one classify derivation per case dir
  (inputs = that case dir + the `cdz` bin only, so editing one case never re-runs another), verdicts
  harvested + folded vs `.gate-baseline` through `gate-syntax --compare`. The authoritative cached,
  parallel gate (advisory per the hourly-advisory land model).

## Migrating the in-crate parser/printer tests into this corpus

The corpus also **replaces** the behavioral parser/printer `#[test]`s in the `cadenza-syntax`* crates
(the delanguaging move — so the Cadenza rewrite validates against this neutral corpus, not Rust-only
tests). Recipe: one behavioral `#[test]` → one case directory; the `#[test]` is **deleted in the same
commit** its case lands (coverage never dips); land per-module batches, each a green MR.

**What this corpus can express (⇒ migrate here):**
- an `input → parse-tree` assertion → a case with `input.<ext>` + `tree.sexp`.
- an `input → canonical-format` / round-trip assertion → add `format.<ext>` (or rely on the
  format-absent `fmt(input) == input` idempotence check).
- a `this input should fail to parse` assertion → a decline case (no `tree.sexp`; optional `error.txt`).

**What stays a Rust `#[test]` (⇒ do NOT migrate):** it pins only the *two* functions above
(single-surface parse-tree + same-surface fmt). These are OUT of scope and stay Rust:
- **internal invariants** — arena/codec builder guards (`cadenza-ast`), `SpanTable` totality,
  printer-is-iterative / printer-total-over-arbitrary-arenas structural guards, lexer edge cases.
- **the query/rewrite engine** (`query.rs`, `cdz query`/metavar/splice/rewrite) — a different feature;
  it would want its own corpus, not this parser/printer one.
- **cross-surface conversions** (`ml → sexpr`, `binary → ml`, …) beyond what `input → tree` already
  subsumes — the corpus pins one surface → arena, not surface-to-surface printing.
- **surface codemods** (`match_to_let` / `cdz normalize …`, a `normalize(input) → tree` transform) —
  a *third* function this corpus does not currently compare; migrating them would need a new
  `normalize`-golden dimension (a scope decision, not yet adopted).

## Migration status (snapshot 2026-08-31)

The delanguaging is tracked by module. This is a point-in-time snapshot, not a contract — the live
truth is the corpus itself (`ml/` + `sexp/` case dirs) and the `#[test]`s that remain.

- **`printer.rs` behavioral tests — COMPLETE.** Every `input → parse-tree` / `input → canonical-format`
  / round-trip printer test has been migrated to an `ml/` case (the corpus is at ~488 cases). What
  remains in `printer.rs` is *only* the out-of-scope guards this corpus deliberately does not express
  (per the list above): printer-totality / iterative-printer structural guards over arbitrary and
  sexpr-sourced arenas, generated/property sweeps (`…over_generated…`, `…over widths`, the reserved-word
  and `emit_name` lexer round-trip sweeps), **width-specific** layout breaks (a break that only triggers
  below the corpus width — `wide_*`, `*_when_wide`, `def_record_body_hugs`, `multi_binding_let_breaks…`),
  cross-surface `Display`/value-render, arena-sourced comment guards (`multiple_comment_wrappers…`,
  `a_nonlast_comment_after_in_a_decoded_ast`), the read-OK-but-fmt-refuses edges
  (`a_same_line_comment_on_a_non_last_collection_elem`, comment-only empty module), and the
  cross-surface disambiguation oracle (`const_expression_is_distinct…`).
- **`parser.rs` behavioral tests — COMPLETE.** Every `input → parse-tree` / round-trip assertion that
  the migrated parser tests carried has been mirrored into an `ml/` case, including the surface features
  that were once the frontier: **brace record-type annotations**, tuple/type-application args, `forall`
  binders and derived-unit infix in type position, the **pipeline operator**, set/bin/quantity/compound-unit
  desugar, construction-spread, rest/destructuring patterns, backtick-escape, effect declarations, nested
  multi-arg constructors, and the top-level `;`-separator / ambiguous-juxtaposition rule (the
  `f() g()` ruling — see `ml/476-ambiguous-toplevel-juxtaposition-rejected`, a `Todo` decline case). What
  remains in `parser/tests.rs` is *only* out-of-scope guards, by the same rules the printer list uses:
  error-recovery and diagnostic-quality assertions (`recover…`/`does_not_bail`/`missing_*_recovers`/
  `*_does_not_cascade`/`…recovers_without_panic`), span totality/slice guards
  (`spans_are_total…`, `every_ml_span_is_a_valid_source_slice…`, `…spans_cover_the_whole…`),
  depth-bound "diagnosed not crashed" guards, fuzz/property sweeps (`never_panics`,
  `exhaustive_short_token_soup…`, `recovered_arena_invariants…`), the recursive-vs-iterative
  reader-agreement guards, the lexer unescape/NFC round-trip (`string_unescape_and_nfc`), the WIT-builder
  codec-equivalence tests (`…encodes_identically…`, `world_decl_builds…`, `inline_world_*`), and the
  read-OK-but-`fmt`-refuses / width-specific edges (`a_closer_comment_does_not_reorder…`,
  `a_chained_else_if_ladder…`). Genuine reject-with-guidance guards (`…is_rejected_steering_to_the_colon_form`,
  `malformed_wit_member_type_spellings_are_rejected…`, `…forall…is_a_reserved_keyword_error`) stay Rust for
  their recovery-arena invariants; their language-agnostic "this declines" content is a candidate for
  future `Todo` decline cases alongside `ml/476`.

## How it drives the future Cadenza-parser rewrite

For every case, the implementation-under-test must satisfy `render_sexpr(parse(input)) == tree.sexp`
and `fmt(input) == format.<ext>`-or-`input`. Today that impl is the reference `cadenza-syntax` (Rust);
tomorrow it is the Cadenza-written parser/printer, which must reproduce the identical goldens. A
construct the new parser has not yet reached *declines* → `Todo`, so the rewrite lands incrementally,
feature by feature, never a flag-day — the same "one oracle, swappable implementation" property the
semantics corpus gave the compiler.
