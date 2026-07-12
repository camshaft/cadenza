# Design — a syntax query/modification engine, driven by a Cadenza sidecar

**Author:** front-end engineer. **Audience:** whoever builds structural query/rewrite tooling over the
Cadenza AST (and, eventually, the self-hosted version of it).
**Status:** **DESIGN ONLY — nothing landed.** The seam this builds on is real *today*
(`cadenza-syntax`: arenas + codec + spans + `convert`; `rcdzc`: the component ABI for the working
language subset). The self-hosted end state is gated on the generics/recursion work the compiler has
not finished, so this doc stages the build so each rung is useful before that lands. Line/module
references are landmarks at this commit, not promises they won't drift.

This is a how-to, not a mandate. It states one architectural move — **treat the compiler as one
instance of a general `engine(target, sidecar, output)` shape** — and designs the query/modify engine
against the pieces `cadenza-syntax` already has.

---

## 1. TL;DR — the win, the seam, the one insight

**The win.** Give an agent (or a human) a way to *select* groups of AST nodes by a condition, *project*
them (their type, their span, a count, the sub-trees themselves), or *modify* them (a validated
rewrite), where the query/modification is **written in Cadenza itself** and the result is emitted in
whatever format the caller wants. Structural search-and-replace and semantic queries become ordinary
programs, not bespoke tooling.

**The one insight (the operator's framing).** *The compiler is already a query engine with a sidecar.*
It takes a target tree, runs a program that guides what to do with it (lower it to a component), and
emits a result. Generalize the sidecar:

```
engine(target: Ast, sidecar: Program, output: Format) → Bytes
```

- **Compile** = sidecar says "lower to a wasm component"; output = component bytes.
- **Query** = sidecar says "collect nodes matching X"; output = a location list / count / sub-trees.
- **Modify** = sidecar says "rewrite Y→Z"; output = the new tree (binary / s-expr / ML).
- **Type projection** = sidecar says "infer the type of each def"; output = types.

Same machine, different sidecar. **The query "language" is not a new language** — because the AST is
homoiconic `(head child…)` data, a program that manipulates it is ordinary Cadenza over ordinary
values. The query surface is a *library* over the `Ast` value type, not a grammar.

**The seam already exists.** The compiler's ABI is already `ast_bytes → result_bytes`
(`rcdzc/src/abi.rs`; the component exports `compile : list<u8> → list<u8>`). A query sidecar is a
component with the *same* ABI whose result is a query result rather than a component. The compiler is
just the sidecar whose output happens to be a component.

---

## 2. What already exists (the rungs that are built)

| Piece | Where | Role in the engine |
|---|---|---|
| `Arenas` / `Leaf` / `Struct` | `cadenza-syntax/src/ast.rs` | the tree a sidecar queries/rewrites |
| span side-table `StructId → (file, range)` | `cadenza-syntax/src/spans.rs` | joins nodes → source locations for output |
| binary codec `bytes ⟷ arena` (total decode) | `cadenza-syntax/src/codec.rs` | how the target crosses into a sidecar and back |
| `convert` / `Format` (`binary`/`sexpr`/`ml`/`debug`/`flat`) | `cadenza-syntax/src/convert.rs` | **the output-projection layer already exists** — one arena, many renderings |
| quasiquote / unquote (`` `{…} ``, `,x`, `,@xs`) | parser + printer | building replacement trees in a rewrite |
| pretty-printer (ML surface) | `cadenza-syntax/src/printer.rs` | renders any result tree as readable code |
| `rcdzc` → wasm component, run via wasmtime | `rcdzc`, `cadenza-seed` | runs a sidecar for the working language subset |

The load-bearing fact: **output projection is done.** `convert::write(arena, format)` already turns one
arena into binary, s-expr, ML, or a debug view. The engine's "emit in the format the caller wants" is
that function. What's missing is (a) the query/rewrite *vocabulary* and (b) the *driver* that wires
target + sidecar + output.

---

## 3. The sidecar — what a query program looks like

A sidecar is an ordinary Cadenza module exporting a `query` entry. Its parameter is the target `Ast`
(the homoiconic tree, already modeled in the metaprogramming corpus as a sum: `Ast.Int`, `Ast.Name`,
`Ast.List`, …). Examples, in the ML surface:

```
/// Find every call site of a named function.
fn query(ast) = select(ast, call-to("deprecated-fn"))

/// Rename every reference `old` → `new` (a validated edit).
fn query(ast) = rewrite(ast, fn(n) => match n {
  Name(s) => if s = "old" then Some(Name("new")) else None,
  _ => None,
})

/// The inferred type of each top-level definition (a query INTO the compiler).
fn query(ast) = map(defs-of(ast), fn(d) => (name-of(d), type-of(d)))

/// Wrap every risky call in logging — build the new subtree with quasiquote.
fn query(ast) = rewrite(ast, fn(n) => match n {
  App(Name("risky"), args) => Some(`{ log(risky(,@args)) }),
  _ => None,
})
```

Each reads as idiomatic Cadenza and dogfoods the pretty-printer.

---

## 4. The combinator library — the actual vocabulary

"Select groups of nodes, project or modify them" is a small library over `Ast`. This is the durable
API; naming here is the proposal.

**Traverse.** `children(n)`, `descendants(n)`, `descendants-with-path(n)` — a path is the sequence of
child indices from the root, the stable identifier of an *occurrence* (matches the arena's model:
occurrences are distinct even when they share a leaf).

**Select.** `select(ast, pred) → List<Ast>`. Predicates are `Ast → Bool` values and compose:
`p-and`, `p-or`, `p-not`. Semantic selectors (from `options/structural-interface/content-addressed-nodes.md`):
`named(s)`, `call-to(s)`, `defines(s)`, `refs(s)`, `head-is(s)`, `is-literal`. A selector is just a
predicate; users write their own inline.

**Project.** `map` / `filter` / `count` / `first` over the selection; accessors `head(n)`, `args(n)`,
`name-text(n)`, `int-value(n)`; and the two that reach *into the compiler*:
- `type-of(n)` — runs inference on the node and returns its `Type` (this is why the query engine and
  the compiler share a spine: `type-of` *is* a compiler query).
- `span-of(n)` — the source `(file, range)` from the span side-table, for location output.

**Modify.** `rewrite(ast, fn) → Ast` — bottom-up: apply `fn : Ast → Option<Ast>` to each node,
replacing where it returns `Some`. Plus `replace-at(ast, path, new)`, `wrap(ast, path, template)`,
`remove-at(ast, path)`. `quasiquote` builds the replacement subtree. Note: the compiler's own nanopass
passes *are* selects-and-rewrites, so ideally the same machinery expresses a user rewrite and a
compiler pass.

---

## 5. Modification is a validated transaction (the biggest agent win)

When a sidecar returns a modified `Ast`, the engine **re-parses and type-checks it before accepting
it** — well-formed-or-reject. The edit either lands as a validated, diagnostic-free tree, or is
rejected with a specific error and *no change is written*. There is never a half-applied text patch.

This is the "edit-as-atomic-transaction" / "program-transformation-is-a-program" direction
(`options/structural-interface/content-addressed-nodes.md`; the learnings on transformation-is-a-program).
The external research pass found this the highest-value authorship affordance (Meta "Code the
Transforms": F1 0.97 vs 0.75 for text edits; SWE-agent's reject-broken-edit gain). The validation loop
is the point, and the AST + type checker is exactly what enables it.

---

## 6. Output projection — and why it needs the deferred schema-hash

A sidecar returns a *value*, and the engine renders it in the caller's `output` format. But the engine
must know **what kind** of value came back to pick a renderer:

- an `Ast` → render via `convert` (binary / s-expr / ml / debug)
- a `List<Ast>` → render each; or, with `span-of`, a location list
- a `Type` → render the type
- a `List<(Name, Type)>` → a table
- an `Int` (a count) → the number

This is exactly what the **schema-hashed value envelope** (`options/value-interchange/schema-hashed-envelope.md`)
provides, and it is the concrete use for the header we *deferred* while building the codec.
`to-tagged-bytes` stamps the sidecar's result with the 8-byte hash of its type; the driver reads the tag
and dispatches to the matching renderer. So the deferred schema-hash is not dead weight — **it is the
mechanism that lets one driver project arbitrary query results.** Building the query engine is the
reason to finish it.

Convergence worth stating plainly: the value form hashes a *type* the way the AST form hashes a
*program* (same construction). A query result and a compiled component both cross the same ABI as
tagged bytes; the driver treats them uniformly.

---

## 7. The bootstrap ladder (the "tricky bit," honestly)

Running Cadenza sidecars needs Cadenza working. It stages cleanly, and each rung is useful on its own:

**Rung 1 — the seam, today.** `cadenza-syntax` reads/writes the tree, the codec crosses it, `convert`
projects output, and `rcdzc`+wasmtime run components for the working subset. A sidecar that stays inside
that subset compiles and runs *now*.

**Rung 2 — interim Rust host (buildable immediately).** A `query` module in `cadenza-syntax` plus a
`cdz-syntax query …` subcommand that runs a *fixed, built-in* set of transforms directly over `Arenas`
— rename, select-by-head, count, extract-spans — and projects output through `convert`. This proves the
driver and output seam end-to-end, gives agents something usable *before* self-hosting, and is the exact
driver shape the Cadenza version drops into later. The built-in transforms are Rust closures over
`Arenas`; only *where the query logic lives* changes at rung 3.

Illustrative CLI (extends the existing `cdz-syntax`):
```
cdz-syntax query rename:old=new      prog.ml  -t ml       # modify → new tree
cdz-syntax query calls-to:foo        prog.ml  -t spans    # select → locations
cdz-syntax query count:head=match    prog.ml  -t text     # project → a number
```

**Rung 3 — self-hosted.** The combinator library (§4) and the sidecars are *Cadenza*, compiled by
`rcdzc`, run via the component ABI. The Rust driver from rung 2 is unchanged — it just loads a
user-supplied sidecar component instead of dispatching a built-in. The result crosses back as
schema-tagged bytes (§6) and is rendered.

**The gating dependency for rung 3** is the same one blocking self-hosting generally: the combinator
library needs generics (`select`/`map`/`rewrite` are parametric) and recursion over sum types
(traversing `Ast`). That is the FCT/generics workstream already in progress. So this design *motivates*
that work — it is a concrete, high-value driver for it — rather than being indefinitely blocked by it.

> **How declarative + do we need effects — resolved in a companion doc.** This design leaves open how
> much *logic* the sidecar carries (a flat manifest vs. a branching program) and whether driving
> compilation needs effects. [`DESIGN-sidecar-api.md`](./DESIGN-sidecar-api.md) resolves both: the
> sidecar is one pure `drive : Ast -> List Request`, a flat manifest is the degenerate case of a
> branching program (same interface), and effects are *not* needed because every fact — including the
> emitted artifact — is a pure column read, so `if this-expr has this-type then …` is ordinary control
> flow over a value. Read it for the request vocabulary (`Emit`/`Query`/`Rewrite`) and how it generalizes
> the existing `compile(inputs, targets)` entry.

---

## 8. Open questions (for a later pass, not resolved here)

- **Occurrence identity across an edit.** `descendants-with-path` uses positional paths, which shift
  under a rewrite. For multi-edit sidecars, a content-hash node id (the deferred front-end
  content-addressing overlay) would give stable handles. Positional is fine for single-pass rewrites.
- **Selector surface sugar.** §4 is a plain library. If querying becomes pervasive, a thin surface
  (e.g. `(def foo)`, `(refs foo)` selector literals) could sit on top — but only after the library
  proves the vocabulary. Do not invent syntax first.
- **`type-of` for the un-compilable subset.** A query that asks the type of a node using a
  not-yet-implemented feature must *decline* with a specific diagnostic, not crash — same
  reject-don't-miscompile discipline as the compiler.
- **Streaming vs. whole-tree.** The engine loads the whole arena (fine for a file). A workspace-scale
  query (all files) is a fan-out over per-file engine runs, not a new mechanism.

---

## 9. What to build first (if/when this is greenlit)

Rung 2, in this order — each testable against the corpus and the pretty-printer:

1. `query` module: a `Transform` enum of the built-in ops + a `run(arena, transform) → Result` over
   `Arenas` (Rust closures for now); `Result` is an `Ast` | `List<Span>` | `Int` | `List<(Name,Type)>`.
2. `cdz-syntax query <spec> FILE -t <fmt>` subcommand wiring target + transform + output.
3. Output renderers for the non-`Ast` results (spans, counts, tables) — the `Ast` case reuses `convert`.
4. The validated-transaction wrapper: after a modify, re-parse+recheck, reject-or-emit.
5. Tests: each transform over corpus inputs, asserting the projected output and (for modifies)
   round-trip + type-validity of the result.

Rung 3 is deferred behind generics; the library sketch in §4 becomes its spec and its first real
generics/recursion client.
