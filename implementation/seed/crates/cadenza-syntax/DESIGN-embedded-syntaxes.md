# First-class embedded syntaxes in the front-end (design sketch)

v-syntax charter. Operator direction: JSX, Markdown, JSON, TOML (and more) as first-class
sub-languages you can switch into, each parsed into the SAME arena/AST the toolchain already
round-trips, so fmt / LSP / refactor / codec work on them for free. Design-first — no parser
code until the switch mechanism is locked with the operator.

This is DISTINCT from v-metaprogramming's tagged-template-macro JSX (a library-level macro
producing `Ast` at compile time). This is a parser-level, first-class syntax switch producing
native arena nodes the whole toolchain understands. They coexist (see §5).

## Why this is a small step, not a rebuild

The crate ALREADY has the hard half. Every surface — sexpr, ML, json, toml, cedar, markdown —
already exposes `read` / `read_spanned` / `print` and all produce the ONE shared `Arenas`
(flat structure + leaf arenas, `StructId`/`LeafId`). markdown ALREADY does the inverse of what
we want: a fenced ```cdz block whose body parses cleanly carries the parsed PROGRAM as a real
arena subtree child (markdown.rs `embed_program`). So "a region of source in sub-grammar G
parses to a subtree in the shared arena" is a shipped, tested pattern. What is missing is the
GENERAL, BIDIRECTIONAL switch: enter sub-grammar G from ML (not only ML-in-markdown), and the
printer/codec/LSP treating the embedded subtree uniformly.

## 1. The switch mechanism

A single lexer/parser primitive: a DELIMITED embedded-syntax region tagged by a grammar name.
Surface spelling (proposal, one to lock with the operator):

    <grammar> { ... raw region ... }        e.g.  json { {"a": [1, true]} }
                                                    toml { a = 1 }
                                                    jsx  { <div class="x">{expr}</div> }
                                                    md   { # Title\n\nbody }

- The lexer, on seeing a known grammar tag immediately followed by an open delimiter, switches
  to RAW mode: it does NOT tokenize the body as ML; it tracks only nesting of the chosen
  delimiter (brace-aware, string/comment-aware per the sub-grammar's minimal rules) to find the
  matching close, then hands the raw slice + the grammar tag to the parser.
- The parser calls the sub-grammar's existing `read_spanned(raw)` (json/toml/cedar/markdown
  today; jsx new) and GRAFTS the returned arena as a subtree, exactly as markdown's
  `embed_program` grafts a fenced program today (graft_subtree already exists in parser.rs).
- Exit is purely the matching close delimiter — the sub-grammar reader consumes the whole raw
  region, so there is no ambiguity about "when to hand back."
- ML holes inside a sub-grammar (e.g. JSX `{expr}`) are the ONE place the sub-grammar re-enters
  ML: the sub-grammar reader, on its hole delimiter, calls back into ML `read_ml` on the hole
  slice and grafts THAT subtree. This is the same re-entry markdown fences already do in reverse.

Delimiter choice (`{ }` vs a fence vs a sigil) is the core thing to lock with the operator.
`<grammar> { }` reads naturally in expression position and nests; a triple-fence reads
naturally at statement/doc scope. Recommend supporting BOTH: braces for inline/expression
embedding, fences for block/document embedding — both lower to the same node (§2).

## 2. The shared-AST target

A single new generic node, NOT one node kind per sub-language:

    (embedded <grammar:Name> <subtree>)

- `grammar` is a Name leaf: `#json` / `#toml` / `#cedar` / `#markdown` / `#jsx`.
- `subtree` is the arena the sub-grammar's `read` already produces — native nodes
  (`(object ...)`, `(document ...)`, `(element ...)`), NOT an opaque string.
- No codec change, no version bump: `(embedded ...)` is an ordinary Name-headed list, so
  `codec::encode`/`decode` handle it for free, and canonicalization already normalizes it.
- Totality/non-panic is inherited: each sub-grammar reader is already TOTAL + never-panics on
  arbitrary input (the sweeps I have been landing — span geometry on all six read_spanned
  surfaces, printer totality). A malformed region reads to a best-effort recovery arena or a
  clean diagnostic, never a crash — same contract as the standalone surface.
- Round-trip: the ML PRINTER, at an `(embedded g sub)` node, emits `g { <sub-grammar>::print(sub) >}`
  — it delegates to the sub-grammar's existing `print`. So `read(print(x))` round-trips because
  each half already round-trips. (Data surfaces are arena-idempotent, not byte-identical, exactly
  as they are standalone — the same guarantee, unchanged.)

## 3. Tool transparency (the operator's stated payoff)

Because the embedded region is native arena nodes under `(embedded g sub)`:
- codec: works now — ordinary nodes.
- fmt: the ML printer delegates to the sub-grammar printer for the subtree; `cdz fmt` reflows a
  JSON/TOML/JSX region using that surface's existing pretty-printer.
- LSP: `read_spanned` already gives a total span table; the switch just composes span tables
  (the embedded reader's spans, offset by the region's start — the same remap
  `canonicalize_with_map` + `SpanTable::remap` already do). Hover/go-to-def/rename land inside
  the region because the spans are real source ranges (the span-geometry invariant I swept
  guarantees they are safe-to-slice).
- refactor/query: the codemod engine (query.rs `Tree`/`rewrite`) walks the shared arena, so a
  rename or structural rewrite crosses into a JSX/TOML region with no special-casing — the
  operator's "refactoring just works."

The design's whole point: NOTHING downstream of the arena needs to know an embedded syntax
exists. The switch is confined to the lexer/parser seam; everything past `Arenas` is unchanged.

## 4. Sequencing (incremental, each independently gated)

1. SWITCH PRIMITIVE + `(embedded g sub)` node + codec/canon/printer round-trip, proven with
   JSON (simplest grammar, reader already exists + is swept). Ship: `json { ... }` reads, prints,
   round-trips, LSP spans compose.
2. TOML (reader exists + swept) — proves the mechanism is grammar-agnostic (second grammar, zero
   new switch machinery).
3. MARKDOWN block embedding via the general switch (reconciles with the existing fence embed —
   unify `embed_program` onto the general path).
4. JSX (NEW sub-grammar reader — the only real parser work; element/attr/child + `{expr}` ML
   re-entry). Richest; lands last on a proven mechanism.
5. Cedar + future grammars fall out for free.

Each step is one gated slice (crate tests + roundtrip + rcdzc lib + xtask gate additive + check).

## 5. Seam with v-metaprogramming (tagged-template JSX)

They compose; they are NOT rivals.
- v-metaprogramming's tagged template is a LIBRARY macro: `` html`<div>${x}</div>` `` lexes as a
  template literal, and the `html` macro turns the cooked strings + interpolations into `Ast` at
  COMPILE time. Entry point: the template-literal lexer + a user-space macro.
- This is a FRONT-END switch: `jsx { <div>{x}</div> }` produces native `(embedded #jsx ...)` at
  PARSE time, before macros run.
- Composition: a template-literal hole can contain a first-class embedded region
  (`` html`${ jsx { <b/> } }` `` ), and a first-class `{expr}` hole can contain a template
  literal — because BOTH ultimately produce the same arena, the graft points are uniform. The
  ONE thing to coordinate: the `jsx`/`html` grammar-tag namespace, so a tag resolves
  unambiguously to either the front-end switch or a user macro. Proposal: front-end grammar tags
  are a fixed, reserved set (`json/toml/cedar/markdown/jsx`); everything else is a template-macro
  tag. Lock the reserved set with the operator + v-metaprogramming.

## Open questions to lock with the operator (before any parser code)

- Delimiter: `<grammar> { }` vs fence vs sigil (recommend both brace + fence, one node).
- Reserved grammar-tag set + the v-metaprogramming namespace boundary (§5).
- Whether `(embedded g sub)` is the final node shape or the operator wants the sub-nodes hoisted
  directly (recommend the wrapper: it keeps the grammar tag for the printer + tooling).
