# Design — package linking: multi-file `import` into one compilation unit (rcdzc)

**Author:** design pass (compiler). **Audience:** the implementer picking this up, + future me.
**Status:** proposal / handoff — **nothing landed**. Written 2026-07-11 on branch `spec`
(rcdzc-rewrite merged @`2bb7c68`). Line anchors are landmarks at `9f76c1b`.

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
  `rcdzc`. **None of that machinery exists in the columns rewrite.** `grep Gather|gather_scope|
  Synthetic resolve.rs` = 0 hits.
- The rewrite has exactly **one flat top-level namespace**: `db::top_items` (db.rs:410) strips a
  *root-level* `(module NAME …)` head and returns its children as the top-level items; `resolve.rs`
  step 2 resolves a bare name by `db.def_by_name` (db.rs:335) — a **flat global scan** of every def.
  There is no per-file scope and **no nested-module → record resolution** (resolve.rs:161 comment:
  "Stage 0 handles module/def/export/do at the top level, *not here*").
- **Empirically: all 14 cases in `spec/semantics/11-modules.sexp` are `todo`, 0 pass** (measured via
  `xtask gate spec/semantics/11-modules.sexp` → `0 pass, 14 todo, 0 fail`). A nested `(module m …)`
  followed by `(. m f)` does **not** compile today. The passing `member_access_chains_through_a_
  nested_record` test (tests.rs:2099) uses `(record …)` literals, **not** `(module …)`.

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
  "ast"` (abi.rs:32).
- **`compile()` already takes `&[Artifact]`** (compile.rs:32) — it just currently grabs the *first*
  `KIND_AST` and ignores the rest (compile.rs:35: `inputs.iter().find(|a| a.kind == KIND_AST)`).
- **The CLI bin already accepts many named artifacts.** `rcdzc kind:name=path.ast …` (rcdzc.rs:14,
  parse at rcdzc.rs:221); it pushes each into an `inputs: Vec<Artifact>` (rcdzc.rs:101) and calls
  `compile(&inputs, &targets)` (rcdzc.rs:129). So `rcdzc a.ast b.ast c.ast -o out.wasm` already
  *delivers* every file — the compiler just drops all but the first today.
- **The arena is two Vecs + a root.** `Arenas { leaves: Vec<Leaf>, structure: Vec<Struct>, root:
  StructId }` (ast.rs:315). `Struct` is `Atom(LeafId)` | `List(Vec<StructId>)` (ast.rs:282). Both ids
  are `u32` newtypes. This is trivially **append-with-offset** splice-able.
- **`Db::load(ast: Arenas)`** installs the prelude, scans top-level defs/exports, builds the parent
  index (db.rs:222). One arena in → one `Db`.

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
`11-modules.sexp:97` already ties to a duplicate module definition — "resolved by an implicit
precedence" is exactly what's forbidden); an import of an unknown path → a new coded reject or a
decline (§7); an import of a name a module doesn't make public → CDZ0101-family (unbound in that
module) or a dedicated code.

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
  within a file anyway, ast.rs:337).
- For each `Struct`, remap: `Atom(LeafId(l))` → `Atom(LeafId(l + leaf_base[f]))`;
  `List(children)` → `List(children.map(|c| StructId(c.0 + struct_base[f])))`.
- The combined `root` is **not** any file's root — instead the link **synthesizes a `(do …)` root**
  whose children are each file's (remapped) top-level items, in a deterministic order (§3c). This is
  the one built node the splice adds; use `Builder` or push directly.

This is deterministic (a fixed function of the input artifact bytes and their order — no addresses,
no hashing needed for correctness), matching `parent_index`'s determinism note (db.rs:347). It is the
structured analogue of the Makefile's `cat` (Makefile:30), but at the arena level with real ids.

> **Order determinism (spec).** modules-and-namespaces.md §Initialization Order Is Deterministic
> requires init order to be a deterministic function of the source and to follow import dependencies.
> Since every def is nullary-or-a-function and the rewrite has no top-level side-effecting init
> (definitions are values/lambdas, evaluated lazily by demand), "initialization order" is trivially
> satisfied — but the **emission order** (layout.rs `order`) must be deterministic, which it already
> is (declaration order). Splice files in **topological order of the import graph** (§5) so that a
> deterministic, dependency-respecting order is the one recorded.

### 3b. Prelude installed ONCE

`Db::load` calls `prelude::install(&mut ast)` (db.rs:231), which appends prelude nodes and captures
`user_node_count` as the boundary. With a merged arena this must happen **once, after the splice** —
all files share one prelude, one `(Int W)` build cache (db.rs:136), one `user_node_count`. The
cleanest shape: `link()` produces the merged `Arenas` (user nodes only), and `Db::load` installs the
prelude on top exactly as today — so `user_node_count` = the *combined* user-node count, and every
file's nodes are `< user_node_count` (all "user" — correct for `is_user_node`, db.rs:289). **No
change to `Db::load`'s prelude logic**; it just receives a bigger arena.

### 3c. Entry selection (operator's choice: named in the compile request)

The compile request names the entry. Two ways to thread it, pick one:

- **(Recommended) A dedicated non-AST input artifact** carrying the entry path — e.g. an artifact of
  `kind == "entry"` whose bytes are the entry's artifact-name. `compile()` reads it alongside the AST
  artifacts. Keeps the `compile(&[Artifact], &[Target])` signature **unchanged** (compile.rs:32) —
  the entry rides in the artifact stream, consistent with "artifacts-in" (abi.rs:1). This is the
  least invasive and most in-keeping with the kinded-artifact ABI.
- **A new parameter** on `compile()` (e.g. `compile(inputs, targets, entry: Option<&str>)`). More
  explicit but touches the frozen-ish entry signature and every caller.

Only the **entry file's `(export …)`** forms become the component's boundary (reusing the existing
export scan, db.rs:386). A non-entry (library) file's `(export …)`, if any, is **ignored for the
component boundary** but its exported *names* are what its `import`-ers may bind (§4). (Decision: a
library file marks its public surface with `(export …)` too — one visibility mechanism, not two. See
§4.)

---

## 4. Name resolution across files — the ONE new resolver rule

Today `resolve_name` (resolve.rs:189) is: lexical scope → `db.def_by_name` (flat, ALL defs) →
prelude. With multiple files spliced flat, `def_by_name`'s global scan would let **any** file see
**any** other file's defs — violating "imports are explicit" (modules-and-namespaces.md). So the flat
global step must become **file-scoped**:

> A bare name in file `f` resolves against: (1) lexical scope; (2) `f`'s **own** top-level defs; (3)
> the names `f` **imported** (each an `(import "p" (…))` mapping a local name → a def in module `p`);
> (4) prelude. It does **NOT** see another file's defs unless imported.

Mechanically:

- `link()` computes, per file, a `visible: HashMap<String, usize /*db.defs index*/>` = own defs ∪
  imported names. Collisions (two imports, or an import shadowing… — decide the precedence rules per
  modules-and-namespaces.md §Colliding Imported Names Are Rejected) → **CDZ0201**.
- A def carries its **owning file** (extend `Def`, db.rs:37, with a `file: usize`, or derive it from
  the def's `sig_occ` StructId against the `FileSpan` table). `resolve_name` determines the current
  reference's file from its `StructId` (which `FileSpan` range it falls in — a binary search over
  `files`), then looks up in **that file's** `visible` map instead of the global `def_by_name`.
- **Visibility:** a name is importable from module `p` only if `p` makes it public. Reuse `(export
  …)`: a file's importable surface = its export list (modules-and-namespaces.md §Visibility Is
  Explicit — "determined by an explicit rule fixed by this specification"; the export list IS that
  rule, already scanned at db.rs:386). An `(import "p" (f))` where `f` is not in `p`'s exports →
  reject (name not public). This means every non-entry file lists its public API with `(export …)`,
  and the entry file's `(export …)` doubles as the component boundary — one mechanism.

> **Why this is small.** It's a **single** change to resolve step 2: replace the global
> `db.def_by_name` scan with a file-scoped lookup keyed by the reference's `StructId → file`. Steps 1
> (lexical), 3 (prelude) are untouched. `infer`, `lower`, `eval`/fold, `select`, `layout`, `envelope`
> — **all unchanged**, because after resolution every reference points at a concrete def occurrence in
> the one merged arena, and β-reduction monomorphizes across files exactly as within one file (it's
> the same arena, the same `Db`, the same `apply_lambda`).

---

## 5. Cycles & collisions (spec-mandated rejections)

- **Cyclic imports** (modules-and-namespaces.md §Cyclic Module Dependencies Are Rejected): build the
  import graph (files = nodes, `(import "p" …)` = edge f→p) and DFS for a back-edge — the **same
  shape** as the existing static-recursion call-graph DFS (`eval::is_recursive`,
  [[rcdzc-rewrite-static-recursion-detection]]). A cycle → a coded reject. *(Note: value-level mutual
  recursion across files is fine and handled by the existing `Core::Call` recursion path; the
  forbidden thing is an* import *cycle — a compile-time dependency loop, which topo-sort for splice
  order, §3c, detects for free.)*
- **Colliding imported names** (§Colliding Imported Names Are Rejected): two names bound into one
  file's scope under the same key → **CDZ0201**, never implicit precedence (mirrors the duplicate-def
  rejection the corpus already pins, 11-modules.sexp:88–110).
- **Duplicate def within a file** stays whatever the rewrite does today (the flat namespace); across
  files, same-named defs in *different* files are fine (they're in different file scopes) — that's the
  whole point of per-file scoping.

---

## 6. Diagnostics — the one architectural touch beyond "pure front-end"

`Diagnostic.node` is a single `u32` StructId (abi.rs:56) and the consumer maps it to a source span via
its own span table (query-engine.md §Provenance Is Recovered By Back-Reference). With a merged arena,
a global StructId no longer maps to a single file's span table. Fix, minimally:

- `LinkedProgram.files: Vec<FileSpan { path, struct_base, struct_count }>` (§3) lets a consumer demux:
  a diagnostic's global `node` falls in exactly one file's `[struct_base, struct_base+struct_count)`
  range → `(path, node - struct_base)` = the per-file local id the file's own span table is keyed by.
- Surface this table to the consumer. Options: (a) a new artifact of `kind == "link-map"` in the
  `CompileOutput` (consistent with kinded artifacts); (b) an added field on `CompileOutput`
  (abi.rs:73). (a) keeps the struct frozen. The CLI bin (rcdzc.rs), which holds the file list already,
  can also just demux locally.
- `is_user_node` (db.rs:289) still works unchanged: **every** file's node is `< user_node_count`
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
  existing layout.rs:74 "no export declines").
- Anything the single-file pipeline already declines still declines (unchanged).

---

## 8. Increment plan (each independently gate-able)

1. **Spec-first (§2):** pin `(import "path" (names…))` in `options/code-shape/` + add the deferred
   multi-file cases to `11-modules.sexp` (they'll be `todo` until step 4 realizes them — additive,
   gate-neutral). This is the constitutional prerequisite.
2. **Arena splice + `link()` skeleton (§3):** merge N artifacts → one `Arenas` under a synthesized
   `(do …)` root, prelude installed once, `FileSpan` table built. Gate: a package of files with NO
   cross-file imports (each self-contained) compiles byte-identically to the same files concatenated —
   i.e. reproduce the Makefile concat's result structurally. **This alone retires the Makefile** for
   the no-import case.
3. **File-scoped resolution + `(import …)` (§4):** the one resolve-step-2 change + import binding +
   visibility via `(export …)`. Gate: the new `11-modules.sexp` import cases go `todo`→`pass`; a
   cross-file call monomorphizes and folds (assert the callee's body is inlined, no `Core::Call`
   unless recursive).
4. **Cycles & collisions (§5):** import-graph DFS + collision CDZ0201. Gate: a 2-file import cycle
   rejects; a colliding import rejects.
5. **Diagnostics link-map (§6):** surface the `FileSpan` table so a cross-file error maps to the right
   file:line. Gate: an unbound name in file B reports against B's span, not a global offset.
6. **Bootstrap payoff:** re-author `implementation/compiler/cdzc/*.cdz` to `import` each other instead
   of relying on the Makefile concat; delete the Makefile's concat role. (This is the *why* — do it
   once steps 2–5 are green.)

## 9. What explicitly does NOT change (the payoff of intra-package scope)

`infer.rs`, `lower.rs`, `eval.rs`/fold, `ty.rs`, `unify.rs`, `select.rs`, `layout.rs`, `envelope.rs`,
`serialize.rs`, `runtime_abi.rs`, `cdz-run`, the component ABI, the value-heap runtime, `import_base`.
All untouched. After the link step, the compiler sees **one program in one arena** — the same thing it
sees today, just assembled from many files with an explicit visibility overlay. Monomorphization is
the existing β-reduction; one component is the existing backend. That is the whole point of doing
package linking *before* the pipeline and *inside* one component.

## 10. Open decisions for the implementer

- **Entry threading (§3c):** dedicated `kind=="entry"` artifact (recommended, signature-preserving)
  vs. a new `compile()` parameter. *Recommend the artifact.*
- **Visibility rule (§4):** reuse `(export …)` as the public surface (recommended, one mechanism) vs.
  a distinct `pub`/visibility marker. *Recommend reusing `(export …)`.*
- **Link-map surfacing (§6):** new `kind=="link-map"` artifact vs. a `CompileOutput` field.
  *Recommend the artifact (keeps the struct frozen).*
- **Splice order (§3a/c):** topological over the import graph (recommended — deterministic +
  dependency-respecting) vs. request order. *Recommend topological, falling back to request order for
  independent files to keep it stable.*
- **Whether to also revive modules-as-records** (closes 11-modules.sexp's *single*-file cases and
  enables the `(import "p" alias)` qualified form) — orthogonal; this design does not need it, but
  they're natural companions.
