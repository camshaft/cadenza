# Design — package linking: multi-file `import` into one compilation unit (rcdzc)

**Author:** design pass (compiler). **Audience:** the implementer picking this up, + future me.
**Status:** proposal / handoff — **nothing landed** (verified 2026-07-12: `link.rs` does not exist;
`compile()` still selects only the FIRST `ast` artifact — compile.rs:38). Written 2026-07-11; **fact-
refreshed 2026-07-12** against `spec` @`2504bbb` (291 commits after the original `9f76c1b` anchor).
Line anchors below are landmarks at `2504bbb`, NOT `9f76c1b` — they drifted heavily and were re-read.

> **What changed since the first draft (read this before the body).** Two new **kinded input
> artifacts landed** — `sidecar` (`KIND_SIDECAR`, sidecar.rs:38) and `spans` (`KIND_SPANS`,
> spans.rs:34). `compile()` now does `inputs.iter().find(|a| a.kind == …)` for THREE kinds — `ast`,
> `sidecar`, `spans` (compile.rs:38/56/79) — and a malformed non-`ast` input DECLINES with its own
> diagnostic (compile.rs:59/82). **This is the exact shape §3c/§6 recommended for the `entry` /
> `link-map` artifacts, now proven in tree.** The two open decisions that were "recommend the
> artifact, but it's a judgment call" are now "follow the landed `spans`/`sidecar` precedent" — see the
> notes added to §3c and §6. Everything else in this design is unchanged and still applies.

## The one-paragraph statement

Let a caller hand the compiler **all the `.cdz` files of a package as named AST artifacts**, name
one of them the **entry**, and get back **one wasm component**. Each file is a module; a file reaches
another file's public definitions only through an **explicit `(import "path" …)`**. All files are
**linked into a single compilation unit** (one arena, one `Db`), where the existing β-reduction
**monomorphizes everything** exactly as it does today, and the existing backend emits one component.
This is **intra-package linking only** — nothing crosses a component boundary, so there is **zero**
component-ABI / envelope / `import_base` work. It replaces the `implementation/compiler/Makefile`
concat (which exists only because "the language has no imports yet" — Makefile:2) with a real,
structured, hygienic multi-file front-end. The motivating use is **bootstrapping the next-gen
compiler in Cadenza**: `cdzc.cdz` is authored as `cdzc/*.cdz` and merged by `make` today; this lets
those files `import` each other directly.

> **Scope fence (the operator's, verbatim intent).** No cross-package monomorphization, no
> cross-component calls, no WIT/envelope changes in this phase. One package → one component. Files
> link *before* the pure pipeline runs; everything downstream of the link is unchanged.

---

## 0. The load-bearing finding (READ FIRST — it reshapes the obvious approach)

The obvious design is "wrap each file in `(module <path> …)` and splice them under one `(do …)`, then
reuse modules-as-records + `(. m f)`." **That does not work on the current rewrite**, and here is the
measured reason:

- The `rcdzc-modules-as-records` memory (a module = a compile-time record of its exports; `(. m f)` =
  record projection; `Gather::gather_scope`; `RawFunc::Synthetic`) describes the **pre-rewrite**
  `rcdzc`. **None of that machinery exists in the columns rewrite.** `grep -rn 'Gather|gather_scope|
  Synthetic' rcdzc/src/` = **0 hits** (re-verified @`2504bbb`).
- The rewrite has exactly **one flat top-level namespace**: `db::top_items` (db.rs:1155) strips a
  *root-level* `(module NAME …)` head and returns its children as the top-level items (it also strips a
  `(do …)` root — db.rs:1160); `resolve.rs`'s `resolve_name` step 2 resolves a bare name by
  `db.def_by_name` (resolve.rs:272, calling db.rs:748) — a **flat global scan** of every def. There is
  no per-file scope and **no nested-module → record resolution** (the resolve steps are lexical → own
  defs → own `(type …)` decls → prelude; resolve.rs:260–onward).
- **Empirically (re-measured 2026-07-12): `xtask gate spec/semantics/11-modules.sexp` → `3 pass, 13
  todo, 0 fail`** (16 cases; the file grew from 14). ⚠ The original draft said "0 pass, 14 todo" — that
  DRIFTED, but the finding still holds: the **3 passing cases are all REJECTIONS** (a duplicate
  `(export …)` clause, a duplicate export of the entry, and a two-`def` module rejected for a colliding
  field — all CDZ0201-family "reject, don't pick a winner"), NOT nested-module member access. Every
  case that *runs a module to a value* — `a module declaration binds its name …`, `each definition …
  registers a reachable export field`, `(. m answer)` — still **declines** (verified per-case with
  `gate --case`). So a nested `(module m …)` followed by `(. m f)` does **not** compile today, exactly
  as before. The passing `member_access_chains_through_a_nested_record` test uses `(record …)`
  literals, **not** `(module …)`.

**Consequence:** package linking on the rewrite is NOT "reuse modules-as-records." It is its own,
simpler mechanism that operates on the **flat namespace the rewrite already has** — a per-file scope
overlay at name-resolution time, plus an arena splice. We do NOT need nested-module-as-record to ship
this; in fact building on the flat namespace is *less* work than first reviving modules-as-records.
(Reviving modules-as-record is a separate, orthogonal increment that closes 11-modules.sexp; this
design deliberately does not depend on it.)

---

## 1. What already exists (the plumbing is mostly there)

The **outer** plumbing for "provide all files as named artifacts" is already built:

- **Artifacts are kinded and named.** `Artifact { kind, name, bytes }` (abi.rs:16). `KIND_AST =
  "ast"` (abi.rs:32). Sibling kinds `KIND_SIDECAR = "sidecar"` (sidecar.rs:38) and `KIND_SPANS =
  "spans"` (spans.rs:34) now show the *multi-kind* input pattern is established.
- **`compile()` already takes `&[Artifact]`** (compile.rs:35) — it grabs the *first* `KIND_AST` and
  ignores every other `ast` (compile.rs:38: `inputs.iter().find(|a| a.kind == Artifact::KIND_AST)`),
  then *separately* looks up the `sidecar` and `spans` inputs by kind (compile.rs:56/79). So the
  "select an input by kind" muscle already exists three times over — package linking adds the
  select-and-merge of the *multiple* `ast` inputs it currently drops.
- **The CLI already accepts many named artifacts.** `rcdzc kind:name=path …`, spec parsed by
  `parse_input_spec` (cli.rs:246, doc at cli.rs:19); it pushes each into `inputs: Vec<Artifact>`
  (cli.rs:115/134) and calls `compile(&inputs, &targets)` (via `run_with_compiler_stack`, cli.rs:157).
  So `rcdzc a.ast b.ast c.ast -o out/` already *delivers* every file — the compiler just drops all but
  the first today. ⚠ Note: the CLI still lives at `rcdzc/src/cli.rs` on committed `spec`; a rename to
  `src/bin/rcdzc.rs` is in-flight in the main working tree (uncommitted) — anchor to `cli.rs`.
- **The arena is two Vecs + a root.** `Arenas { leaves: Vec<Leaf>, structure: Vec<Struct>, root:
  StructId }` (ast.rs:320). `Struct` is `Atom(LeafId)` | `List(Vec<StructId>)` (ast.rs:287). Both ids
  are `u32` newtypes. This is trivially **append-with-offset** splice-able.
- **`Db::load(ast: Arenas)`** installs the prelude, scans top-level defs/exports/`type`s, synthesizes
  sum records, builds the parent + scope-skip + by-name/by-body indices (db.rs:464). One arena in →
  one `Db`.

So the entire feature is: **turn the N input artifacts into ONE `Arenas` (+ a file-scope map) before
`Db::load`, and add ONE name-resolution rule for `import`.** Nothing else in the pipeline changes.

---

## 2. The surface form (spec-first — this is a prerequisite, not optional)

The constitution makes the corpus the source of truth, and `11-modules.sexp:15–20` says explicitly
that multi-module composition "**has no pinned surface form in the core symbol table yet**, so it is
intentionally not witnessed here; cases arrive with the generation that realizes it." So step one is
**pin the form + add corpus witnesses** in `spec/semantics/11-modules.sexp` and the
`options/code-shape/` symbol table (homoiconic-decoupled-display.md:127 form table). The behavior is
already normative in `spec/capabilities/modules-and-namespaces.md` — this only adds the *syntax*.

**Proposed form** (a new `cadenza/core` symbol, sibling of `module`/`def`/`do`):

```
(import "<path>" (<name>…))     ; bind the listed public names of module "<path>" into this file's scope
(import "<path>" <alias>)       ; bind the whole module-record of "<path>" as <alias> (reached by (. <alias> f))
```

- `<path>` is the **artifact name** of another file in the package (a string leaf — the `name` field
  of its `Artifact`). Not a filesystem path at the pure-core layer; the *host* (the CLI bin) maps a
  filesystem path → artifact name, exactly as it already maps a file → `Artifact` (rcdzc.rs:120).
  Keeping `<path>` = artifact-name keeps `compile()` pure (no I/O), per compile.rs:1 ("NO I/O … that
  is the CLI bin's job").
- The **named-list form** `(import "p" (f g))` brings exactly `f` and `g` into the flat scope of the
  importing file — "imports introduce no names beyond those they name" (modules-and-namespaces.md
  §Imports Are Explicit). This is the primary form for the bootstrap (a pass file wants `resolve` and
  `ty`, not a qualified `(. resolve-mod resolve)` at every call site).
- The **alias form** `(import "p" r)` is the qualified escape hatch; `(. r f)` then projects — but see
  §0, this needs module-as-record projection, so **defer the alias form** to the same increment that
  revives modules-as-records. **Ship the named-list form first.**

> **Why a string path and not a bare name.** A bare-name import (`(import p (f))`) would collide with
> the flat def namespace and force `p` to be a resolvable name. A string literal is inert data the
> linker reads, never a name the resolver must bind — cleaner, and it's what "identified by a path"
> asked for. Content-address resolution (modules-and-namespaces.md §Dependencies Resolve By Content
> Address) is a *host* concern layered on top later: the host maps path → content hash → artifact; the
> pure core only ever sees the resolved artifact set. This phase does not implement content-address
> resolution — it's a package the host assembles — but the string-path form is forward-compatible
> with it.

Diagnostics codes reused/added: colliding imported names → **CDZ0201** (the same code
`11-modules.sexp:81/94` already ties to a duplicate module definition — "resolved by an implicit
precedence" is exactly what's forbidden, and this is now one of the **3 passing** module cases so the
reject path + code are live); an import of an unknown path → a new coded reject or a decline (§7); an
import of a name a module doesn't make public → CDZ0101-family (unbound in that module) or a dedicated
code.

---

## 3. The link step — a new front-end pass BEFORE `Db::load`

New module, e.g. `link.rs`, exporting one function called from `compile()`:

```rust
/// Link a package of named AST artifacts into one compilation unit: one merged Arenas, a file-scope
/// map (which global defs each file's names may see), and the file-offset table for diagnostics.
pub fn link(files: &[(String /*artifact name*/, Arenas)], entry: &str)
    -> Result<LinkedProgram, Reject>;

pub struct LinkedProgram {
    pub arenas: Arenas,                 // all files' structure/leaves appended, ids offset per file
    pub files: Vec<FileSpan>,           // (path, struct_base, struct_count) — diagnostics demux (§6)
    pub scope: FileScopeMap,            // §4 — per-file visible-name overlay from imports
    pub entry: usize,                   // index into files: whose (export …) becomes the component
}
```

### 3a. Arena splice (the one new mechanical primitive)

Append each file's `leaves` and `structure` into a combined arena, shifting ids by that file's base:

- `leaf_base[f] = sum of previous files' leaves.len()`; `struct_base[f] = sum of previous files'
  structure.len()`.
- For file `f`, copy each `Leaf` verbatim into the combined `leaves` (leaves are values; a `Name` or
  `Int` copies as-is — dedup is optional and not required for correctness since `Builder` dedups only
  within a file anyway, ast.rs:326/352). ⚠ Note the deliberate NON-dedup path `leaf_unique`
  (ast.rs:357) exists for placeholders that must stay distinct occurrences — a splice copies leaves as
  values, so it neither needs nor breaks that; just don't re-intern across files expecting collapse.
- For each `Struct`, remap: `Atom(LeafId(l))` → `Atom(LeafId(l + leaf_base[f]))`;
  `List(children)` → `List(children.map(|c| StructId(c.0 + struct_base[f])))`.
- The combined `root` is **not** any file's root — instead the link **synthesizes a `(do …)` root**
  whose children are each file's (remapped) top-level items, in a deterministic order (§3c). This is
  the one built node the splice adds; use `Builder` or push directly.

This is deterministic (a fixed function of the input artifact bytes and their order — no addresses,
no hashing needed for correctness), matching `parent_index`'s determinism note (db.rs:818–820). It is
the structured analogue of the Makefile's `cat` (implementation/compiler/Makefile), but at the arena
level with real ids.

> **Order determinism (spec).** modules-and-namespaces.md §Initialization Order Is Deterministic
> requires init order to be a deterministic function of the source and to follow import dependencies.
> Since every def is nullary-or-a-function and the rewrite has no top-level side-effecting init
> (definitions are values/lambdas, evaluated lazily by demand), "initialization order" is trivially
> satisfied — but the **emission order** (layout.rs `order`) must be deterministic, which it already
> is (declaration order). Splice files in **topological order of the import graph** (§5) so that a
> deterministic, dependency-respecting order is the one recorded.

### 3b. Prelude installed ONCE

`Db::load` captures `user_node_count = ast.structure.len()` BEFORE it calls `prelude::install(&mut
ast)` (db.rs:469/473) — that count is the boundary. It also appends the built-in sum decls and
synthesizes sum records after the user scan (db.rs:480–506), all still above `user_node_count`. With a
merged arena this must all happen **once, after the splice** — all files share one prelude, one
`user_node_count`, one set of built-in sums. The cleanest shape: `link()` produces the merged `Arenas`
(user nodes only), and `Db::load` installs the prelude on top exactly as today — so `user_node_count`
= the *combined* user-node count, and every file's nodes are `< user_node_count` (all "user" — correct
for `is_user_node`, db.rs:635). **No change to `Db::load`'s prelude/sum logic**; it just receives a
bigger arena.

### 3c. Entry selection (operator's choice: named in the compile request)

The compile request names the entry. Two ways to thread it, pick one:

- **(Recommended — now with a landed precedent) A dedicated non-AST input artifact** carrying the
  entry path — e.g. an artifact of `kind == "entry"` whose bytes are the entry's artifact-name.
  `compile()` reads it alongside the AST artifacts. Keeps the `compile(&[Artifact], &[Target])`
  signature **unchanged** (compile.rs:35) — the entry rides in the artifact stream, consistent with
  "artifacts-in" (abi.rs:1). **This is exactly how `sidecar` and `spans` were subsequently added**
  (compile.rs:56/79): a new `KIND_ENTRY` const beside `KIND_SIDECAR`/`KIND_SPANS`, a
  `inputs.iter().find(|a| a.kind == KIND_ENTRY)`, and a decline if it is present-but-malformed
  (mirroring compile.rs:59/82). The judgment call from the first draft is now settled by two shipped
  examples — **do this.**
- ~~A new parameter on `compile()`~~ — rejected in practice: neither `sidecar` nor `spans` took a
  parameter; both rode the artifact stream. Adding an `entry` parameter would diverge from the
  now-established convention for no benefit.

Only the **entry file's `(export …)`** forms become the component's boundary (reusing the existing
export scan — `scan_top_level`, db.rs:997, feeding `db.exports` and `layout::compute`, layout.rs:121).
A non-entry (library) file's `(export …)`, if any, is **ignored for the component boundary** but its
exported *names* are what its `import`-ers may bind (§4). (Decision: a library file marks its public
surface with `(export …)` too — one visibility mechanism, not two. See §4.)

---

## 4. Name resolution across files — the ONE new resolver rule

Today `resolve_name` (resolve.rs:260) is: (1) lexical scope → (2) `db.def_by_name` (flat, ALL defs,
resolve.rs:272) → (3) `db.type_decl_by_name` (flat, ALL `(type …)` decls, resolve.rs:294) → (4)
prelude (resolve.rs:301). With multiple files spliced flat, the `def_by_name` **and**
`type_decl_by_name` global scans would let **any** file see **any** other file's defs/types —
violating "imports are explicit" (modules-and-namespaces.md). So **both** flat global steps must
become **file-scoped** (the original draft named only `def_by_name`; the `type_decl_by_name` step
landed since and needs the same treatment — a cross-file `(type …)` reference must also go through an
import):

> A bare name in file `f` resolves against: (1) lexical scope; (2) `f`'s **own** top-level defs AND
> `(type …)` decls; (3) the names `f` **imported** (each an `(import "p" (…))` mapping a local name →
> a def/type in module `p`); (4) prelude. It does **NOT** see another file's defs/types unless
> imported.

Mechanically:

- `link()` computes, per file, a `visible: HashMap<String, usize /*db.defs index*/>` = own defs (and
  `(type …)` decls) ∪ imported names. Collisions (two imports, or an import shadowing… — decide the
  precedence rules per modules-and-namespaces.md §Colliding Imported Names Are Rejected) → **CDZ0201**.
- A def carries its **owning file** (extend `Def`, db.rs:41 — currently `{ name, sig_occ, params,
  body }`, add a `file: usize`; or derive it from the def's `sig_occ` StructId against the `FileSpan`
  table). `resolve_name` determines the current reference's file from its `StructId` (which `FileSpan`
  range it falls in — a binary search over `files`), then looks up in **that file's** `visible` map
  instead of the global `def_by_name`/`type_decl_by_name`. ⚠ `resolve_name` today takes `&Db`
  (resolve.rs:260) and reads only `db.def_by_name`/`db.type_decl_by_name`/`db.prelude` — the file-
  scoped map has to hang off the `Db` (or a sidecar table threaded in) so this stays a `&Db` read.
- **Visibility:** a name is importable from module `p` only if `p` makes it public. Reuse `(export
  …)`: a file's importable surface = its export list (modules-and-namespaces.md §Visibility Is
  Explicit — "determined by an explicit rule fixed by this specification"; the export list IS that
  rule, already scanned by `scan_top_level`, db.rs:997, into `db.exports`). An `(import "p" (f))` where
  `f` is not in `p`'s exports → reject (name not public). This means every non-entry file lists its
  public API with `(export …)`, and the entry file's `(export …)` doubles as the component boundary —
  one mechanism.

> **Why this is small.** It's essentially a **single** change to resolve steps 2–3: replace the global
> `db.def_by_name`/`db.type_decl_by_name` scans with a file-scoped lookup keyed by the reference's
> `StructId → file`. Steps 1 (lexical), 4 (prelude) are untouched. `infer`, `lower`, `eval`/fold,
> `select`, `layout`, `serialize` — **all unchanged**, because after resolution every reference points
> at a concrete def occurrence in the one merged arena, and β-reduction monomorphizes across files
> exactly as within one file (it's the same arena, the same `Db`, the same `apply_lambda`).

> **⚠ The one non-obvious hazard the first draft missed — β-copy hygiene (learned while implementing
> Inc 3).** "Key the lookup by the reference's `StructId → file`" is correct ONLY for a node that is
> still in its home file's `FileSpan`. But β-reduction **copies a callee's body to FRESH StructIds**
> (`eval::copy_structural` mints a new occurrence per name via `push_atom`/`push_list`), and those
> copies are appended AFTER every file, so they fall outside all `FileSpan` ranges. A copied *free*
> reference — e.g. a helper `h` in file `lib` whose body references a sibling `k` also in `lib` — must
> still resolve `k` in **`lib`'s** scope after `h` is inlined into a caller in file `app`; a naive
> `StructId → FileSpan` lookup finds no file for the copy and would either report `k` unbound or (worse,
> once names collide) bind it to `app`'s `k` — a silent miscompile. **Fix:** carry a `node_file:
> Vec<Option<usize>>` column on the `Db` (a node's home file index), seeded from the `FileSpan` ranges
> at load and **extended in `push_atom`/`push_list` so a β-copy inherits its source node's file**
> (parallel to how `push_list` already maintains `parent`/`scope_binders`). Resolution reads
> `node_file[id]` instead of a range search. This keeps the change local (the resolver + two push
> helpers + a load-time seed) but it is NOT the zero-touch "single lookup swap" the first draft implied.
> A safe interim (what Inc 3's first slice ships): file-scope only for a package of >1 file, and when a
> copied node's file is indeterminate AND the name is defined in more than one file, **DECLINE** rather
> than guess — decline-don't-miscompile buys correctness while the `node_file` column is proven out.

---

## 5. Cycles & collisions (spec-mandated rejections)

- **Cyclic imports** (modules-and-namespaces.md §Cyclic Module Dependencies Are Rejected): build the
  import graph (files = nodes, `(import "p" …)` = edge f→p) and DFS for a back-edge — the **same
  shape** as the existing static-recursion call-graph DFS (`eval::is_recursive`, eval.rs:498, an
  iterative worklist DFS over the call graph; [[rcdzc-rewrite-static-recursion-detection]]). A cycle →
  a coded reject. *(Note: value-level mutual recursion across files is fine and handled by the existing
  `Core::Call` recursion path; the forbidden thing is an* import *cycle — a compile-time dependency
  loop, which topo-sort for splice order, §3c, detects for free.)*
- **Colliding imported names** (§Colliding Imported Names Are Rejected): two names bound into one
  file's scope under the same key → **CDZ0201**, never implicit precedence (mirrors the duplicate-def
  rejection the corpus already pins — and now PASSES — at 11-modules.sexp:81–96).
- **Duplicate def within a file** stays whatever the rewrite does today (the flat namespace); across
  files, same-named defs in *different* files are fine (they're in different file scopes) — that's the
  whole point of per-file scoping.

---

## 6. Diagnostics — the one architectural touch beyond "pure front-end"

`Diagnostic.node` is a single `Option<u32>` StructId (abi.rs:56) and the consumer maps it to a source
span via its own span table (query-engine.md §Provenance Is Recovered By Back-Reference). ⚠ Note that
a `spans` INPUT artifact now also exists (spans.rs, `SpanData::range(id) -> (u32,u32)`, spans.rs:54) —
but it is keyed by the SAME per-file `StructId` space and does not itself solve the merged-arena demux;
the debug path (compile.rs:94) currently assumes one file. With a merged arena, a global StructId no
longer maps to a single file's span table (nor to a single `spans` input). Fix, minimally:

- `LinkedProgram.files: Vec<FileSpan { path, struct_base, struct_count }>` (§3) lets a consumer demux:
  a diagnostic's global `node` falls in exactly one file's `[struct_base, struct_base+struct_count)`
  range → `(path, node - struct_base)` = the per-file local id the file's own span table (or its
  `spans` input) is keyed by.
- Surface this table to the consumer. Options: (a) a new artifact of `kind == "link-map"` in the
  `CompileOutput.artifacts` list (consistent with kinded artifacts); (b) an added field on
  `CompileOutput` (abi.rs:91). **(a) is now the clear choice** — it keeps the frozen `CompileOutput`
  struct unchanged AND matches how every other side-channel (`type-info`, `uses`, and the emitted
  `component` itself) already rides `artifacts` as a distinct kind (sidecar.rs:41/44, the query
  artifacts pushed at compile.rs:107–113). The CLI (cli.rs), which holds the file list already, can
  also just demux locally.
- `is_user_node` (db.rs:635) still works unchanged: **every** file's node is `< user_node_count`
  (all user, none prelude), so the prelude/synthesized boundary is preserved. The file-demux is a
  *finer* partition of the user range, layered on top — it doesn't disturb the user/prelude boundary.

This is a clean extension of the span-free design, not a violation of it: the compiler still emits
only node identity; the link map is the same kind of consumer-side back-reference table, now
two-level (global id → (file, local id) → span).

---

## 7. Decline-don't-miscompile boundary

Per reference-compiler.md §Outcomes Are Ordered By Safety, anything not-yet-handled **declines**
(no artifact) rather than miscompiles:

- An `(import …)` of an unknown path, or a name a module doesn't export → a coded reject / decline
  (never silently resolve to nothing).
- The **alias form** `(import "p" r)` (needs module-as-record, §2) → **decline** ("qualified import
  is a later phase") until modules-as-records is revived. The named-list form works now.
- A file with no `(export …)` used as the entry → decline (nothing public to emit — same as the
  existing layout.rs:121–123 "no export declines").
- Anything the single-file pipeline already declines still declines (unchanged).

---

## 8. Increment plan (each independently gate-able)

> **Landed status (2026-07-12):** steps 2 and 3 are **DONE** on branch `imports` (module
> `rcdzc/src/link.rs` + `Db::load_linked` + a file-scoped step 2 in `resolve.rs`). Step 1 (spec/corpus
> witnesses) is **blocked on a corpus-format gap** — see the note under it. Steps 4–6 remain.

1. **Spec-first (§2):** pin `(import "path" (names…))` as a core form + add the deferred multi-file
   cases to `11-modules.sexp`. ⚠ **BLOCKED / re-scoped.** The behavior is already normative in
   `spec/capabilities/modules-and-namespaces.md`; what's missing is a *surface-form* pin (the
   `options/code-shape/` path the first draft named does NOT exist — the corpus references it
   aspirationally). **The real blocker:** the corpus is **single-`(input …)` per case** (`cdz-corpus`
   `parse_case` reads ONE `input`; the gate driver pipes ONE file through `rcdzc compile -`), so a
   multi-file package **cannot be expressed as a `.sexp` case today**. Realizing this step needs a
   corpus extension first (e.g. an `(input (file "name" …)… (entry "name"))` shape + a gate-driver path
   that feeds several named `ast` artifacts + an `entry` artifact to `compile`). Until then, steps 2–3
   are gated by **Rust integration tests** in `link.rs` (splice, id-offset, FileSpan demux, cross-file
   import emits, unimported-sibling-unbound, non-exported-import declines, unknown-module declines,
   alias-form declines, β-copy hygiene) — 12 tests, all green.
2. **Arena splice + `link()` skeleton (§3):** ✅ **DONE.** `link()` merges N artifacts → one `Arenas`
   under a synthesized `(do …)` root, prelude installed once (via `Db::load`), `FileSpan` table built.
   `compile()` selects all `ast` inputs + an optional `entry` artifact and routes through
   `link_inputs`; a single `ast` decodes directly (byte-identical to before). Gate: 7 unit tests +
   full behavior gate green with 0 regressions.
3. **File-scoped resolution + `(import …)` (§4):** ✅ **DONE (def scope).** `link()` reads `(import
   "path" (name…))` clauses (compile-time directives, NOT spliced) + each file's `(export …)` surface;
   only the ENTRY file's exports survive into the merged `(do …)` (so `db.exports` IS the boundary).
   `Db::load_linked` derives a `FileScopeTable`; `resolve_name` step 2 is file-scoped when a package is
   linked (own defs + imports; a sibling's def is invisible unless imported), with β-copy hygiene (a
   synthesized/inlined node whose file is indeterminate resolves an unambiguous name flat and DECLINES
   an ambiguous one — decline-don't-miscompile). **RESIDUALS (deliberately deferred):** (a) `(type …)`
   resolution (`type_decl_by_name`, step 3) is NOT yet file-scoped — cross-file type visibility stays
   flat (a follow-up, mirrors the def change); (b) the ALIAS form `(import "p" alias)` declines
   (needs modules-as-record).
4. **Cycles & collisions (§5):** ✅ **DONE.** `find_import_cycle` runs a back-edge DFS over the import
   graph (file → each file it imports from) — a cycle → CDZ0201; a duplicate imported local name →
   CDZ0201. Both are CODED rejects (a positively-proven ill-formed package), not declines. Gate: 4 unit
   tests — a 2-file cycle, a 3-file cycle, a colliding import (all reject), and an acyclic diamond
   (`util` imported twice, no back-edge — must NOT false-positive). ⚠ Uses the existing `Code::Malformed`
   (CDZ0201); a dedicated cyclic-import code can be minted when this folds into the spec taxonomy.
5. **Diagnostics link-map (§6):** surface the `FileSpan` table so a cross-file error maps to the right
   file:line. Gate: an unbound name in file B reports against B's span, not a global offset.
6. **Bootstrap payoff:** re-author `implementation/compiler/cdzc/*.cdz` to `import` each other instead
   of relying on the Makefile concat; delete the Makefile's concat role. (This is the *why* — do it
   once steps 2–5 are green.)

## 9. What explicitly does NOT change (the payoff of intra-package scope)

`infer.rs`, `lower.rs`, `eval.rs`/fold, `ty.rs`, `unify.rs`, `select.rs`, `layout.rs`, `serialize.rs`,
`runtime_abi.rs`, `cdz-run`, the component ABI, the value-heap runtime, `import_base`, **and the
landed `sidecar`/`spans` query paths** (a query still reads the merged `Db`'s columns — it just now
answers over a package instead of a file). All untouched. After the link step, the compiler sees **one
program in one arena** — the same thing it sees today, just assembled from many files with an explicit
visibility overlay. Monomorphization is the existing β-reduction; one component is the existing
backend. That is the whole point of doing package linking *before* the pipeline and *inside* one
component.

## 10. Open decisions for the implementer

- **Entry threading (§3c): SETTLED — dedicated `kind=="entry"` artifact.** Was "recommend the
  artifact"; now backed by two landed precedents (`sidecar`, `spans`), both of which chose the artifact
  over a `compile()` parameter. Follow them.
- **Link-map surfacing (§6): SETTLED — new `kind=="link-map"` artifact.** Same reasoning: every other
  side-channel (`type-info`, `uses`, `component`) already rides `CompileOutput.artifacts` as a kind;
  match that and leave the frozen struct alone.
- **Visibility rule (§4):** reuse `(export …)` as the public surface (recommended, one mechanism) vs.
  a distinct `pub`/visibility marker. *Recommend reusing `(export …)`.* (Still open — a genuine design
  choice, not settled by the landed work.)
- **Splice order (§3a/c):** topological over the import graph (recommended — deterministic +
  dependency-respecting) vs. request order. *Recommend topological, falling back to request order for
  independent files to keep it stable.* (Still open.)
- **Whether to also revive modules-as-records** (closes 11-modules.sexp's *single*-file cases and
  enables the `(import "p" alias)` qualified form) — orthogonal; this design does not need it, but
  they're natural companions. (Still open — and note the 3 now-passing module cases are all rejects;
  the value-producing single-file module cases remain `todo`, so this revival is still unstarted.)
