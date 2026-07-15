# DESIGN: Opaque types — export the type handle, keep the constructor private (`rcdzc`)

Status: proposal (nothing landed). Worktree `.claude/worktrees/opaque-types`, branched off `spec`.
Author intent (operator ask): *"a way to make a type constructor private while also exporting the
type handle."*

This is the classic **abstract data type** / **smart-constructor** feature: a module publishes a type
`T` so other modules can name it, hold its values, and pass them around, **without** publishing the way
to build or take apart a `T`. Outside the module a `T` is an opaque token; the only way to make one or
look inside one is through the functions the module chose to export. That is how you enforce an
invariant a type is supposed to carry (a non-empty list, a validated email, a sorted vector, a
balanced tree, a positive `Money`) — the invariant is established once, in the private constructor, and
no importer can fabricate a value that skips it.

## Why this is a small change, not a new subsystem

Cadenza already has every ingredient. This feature is almost entirely a matter of **not** doing
something the current model would otherwise do (leak constructors when a type is exported). The three
load-bearing facts:

1. **Visibility is per-name and explicit.** A program is an implicit module — a top-level `(do …)` of
   `def` / `type` / `export` forms — and *"whether a definition is visible outside its module MUST be
   determined by an explicit rule … not by its position in the source"* (`modules-and-namespaces.md`
   §Visibility Is Explicit). The only public surface is what an `(export …)` clause names; a name not
   exported *"MUST NOT be importable by another module."* Visibility is therefore already decided one
   name at a time.

2. **A `(type …)` declaration binds several *distinct* names.** `(type T (A Int64) (B))` binds, into
   the enclosing scope: the **type handle** `T` (a first-class value of kind `Type`, usable in
   annotations and as a generic argument), and each **constructor** `A`, `B` — bound both bare and
   through member access `(. T A)` (per `rcdzc-user-type-decls-increment-b`). Dually, `A`/`B` are the
   patterns a match uses. These are separate bindings; the language just has not, so far, drawn a
   visibility line *between* them.

3. **Nominal identity is a compile-time tag with no runtime cost.** *"A nominal type MUST be
   represented as its underlying structural value together with a compile-time tag … the tag adds
   nothing to the value's runtime representation"* (`type-system.md` §Nominal Is An Orthogonal Modifier
   Over Any Structural Type). Opacity is likewise a **compile-time visibility fact**: it constrains
   what a use site may write, and is fully erased before emit. An opaque type and its underlying shape
   are byte-identical.

So the whole feature is: **exporting a type handle exports *only* the handle — its constructors,
its variant/field structure, and its match capability stay private unless separately exported.** No new
runtime representation, no new IR node for "opaque", no ABI change for the intra-package case.

## The surface (recommended: zero new syntax)

Because visibility is already per-name, the abstract/concrete distinction falls out of *which names a
module exports*, exactly like ML's `type t` (abstract) versus `type t = …` (concrete):

```
; ---- module "nel" : a non-empty list, invariant enforced by a private constructor ----
(do
  (type Nel (Mk (Tuple Int64 (List Int64))))     ; Mk is PRIVATE (never exported)

  (def (of (: head Int64) (: rest (List Int64)))  ; the smart constructor — the ONLY way in
    (Nel.Mk (tuple head rest)))

  (def (head (: xs Nel))                           ; a public accessor — the ONLY way to look
    (match xs ((Nel.Mk (tuple h _)) h)))

  (export Nel of head))                            ; export the HANDLE + two functions, NOT Mk
```

- `(export Nel)` exports the **type handle only** → `Nel` is **abstract** to importers.
- The constructor `Mk` and the pattern `Nel.Mk` are simply *not exported* → private.
- An importer:
  ```
  (do (import "nel" (Nel of head))
      (def (main) (head (of 1 (list 2 3))))       ; OK: builds + reads via exported fns
      (export main))
  ```
  can name `Nel` (annotations, signatures, generic args), hold and pass `Nel` values, and use the
  exported `of`/`head`. It **cannot** write `(Nel.Mk …)`, cannot `match` on `Nel.Mk`, and cannot strip
  `Nel` to its underlying tuple — those names/capabilities are not in scope.

To publish a type **concretely** (constructors and matching visible — this is what `Ast` and most
prelude sums want), export the constructors too. Two forms, both first-class:

```
(export Color Color.Red Color.Green Color.Blue)   ; explicit: name each constructor
(export Color.*)                                  ; wildcard: the handle + ALL its constructors
```

### The wildcard `(. T *)` / `T.*` — export the handle and every constructor at once

`(export Color.*)` — canonically `(export (. Color *))`, to be spelled `Color.*` on the ML surface once
its printer lands (below) — exports the type handle `Color` **and** every one of its constructors. It is the concise concrete-export form:
"publish this type and everything needed to build and match it." For a wide sum (a syntax-tree type
with a dozen variants, `Ordering`, a big enum) this is the difference between one clause and a
thirteen-name list that drifts out of date every time a variant is added.

This is **not a new arena shape** — it reuses the member-access projector the language already has.
`(. T A)` already resolves to the constructor `A` of type `T` (per [[rcdzc-user-type-decls-increment-b]],
where a qualified ctor is the *same node* as the bare one). The wildcard adds one reserved final
segment: `*` in a member-access position **against a type handle**, inside an `(export …)` clause, means
"every constructor field of `T`."

**s-expr surface — works today, zero changes (verified against the live parser).** `*` already lexes as
a bare `Leaf::Name` atom (it is the multiply operator), so `(export (. Color *))` reads and round-trips
faithfully with no lexer or reader change — the arena is `(export (. Color *))` with `*` preserved as
the member field. The whole feature is expressible on s-expr immediately.

**ML surface — the ergonomic `Color.*` spelling needs printer work (part of O4, not free).** Measured
against the current ML printer, two gaps stand between the arena form and a clean `export { Color.* }`:
(a) the `export { … }` printer only handles bare-name children and bails to the generic-call form
(`` `export`(…) ``) as soon as a child is a member access — so *even the explicit* concrete export
`(export Color.Red Color.Green)` does not render into `export { … }` today; and (b) the member-access
printer bails on a `*` field (`*` is not a bare-safe ML identifier), rendering `` `.`(Color, `*`) ``
rather than `Color.*`. Both are printer/lexer ergonomics on the *decoupled front-end*, not compiler or
semantics changes — the arena and all downstream passes are surface-agnostic. O4 covers teaching the ML
`export { … }` printer to render member-access children (fixing the explicit case too) and reserving
`*` as a bare-safe member segment. Until then, `Color.*` is a first-class **s-expr** form and an ML
round-trip-but-ugly form; the ergonomic ML spelling lands in O4.

Precise meaning of `(export T.*)`:
- Exports the handle `T` **and** each of `T`'s constructor names (both bare `A` and qualified `(. T A)`),
  and with them the capability to `match` on `T` — i.e. `T` is **concrete** to importers.
- It is **exactly equivalent** to writing `(export T A B C …)` for `T`'s full current variant set — it
  expands to that list at resolve time. So it is meaning-preserving sugar (the [[no-keys-outside-the-prelude]]
  discipline: `*` is a fixed reserved segment with one meaning, not a new open key), and adding a
  variant to `T` automatically keeps the export complete — the intended ergonomic win.
- The wildcard is **only** a member-access-against-a-type-handle form. `*` as a bare export name, or
  `(. m *)` against a module record, is **not** this form and is rejected (`CDZ0201` ill-formed export)
  — the reserved meaning is scoped narrowly so it can never collide with the multiply operator or a
  glob over arbitrary names.

Because `T.*` expands to the explicit list, the **abstract vs. concrete** choice stays crisp and binary
at the export site: `(export T)` = handle only (abstract); `(export T.*)` = handle + all ctors
(concrete); `(export T A B)` = handle + a chosen subset (a *partially* concrete type — e.g. expose some
smart-constructor-free variants but keep one private). All three are the same one mechanism.

The AST type must be exportable concretely (`type-system.md` §The Abstract Syntax Tree Is An Ordinary
Sum Type); `(export Ast.*)` is the intended spelling for it.

### Why not make it a keyword on the declaration?

An alternative is a `private`/`abstract` marker on the `(type …)` form itself. Rejected: it would put
the visibility decision *at the declaration* rather than in the `(export …)` clause, splitting the one
"explicit visibility rule" the spec mandates across two places, and it re-introduces a
position/decoration channel for something the export list already expresses. Keeping the whole
visibility story in `(export …)` is the cohesive choice and matches [[no-keys-outside-the-prelude]] in
spirit (one place decides).

## What "abstract" means, precisely (the importer's view)

For a type `T` whose handle is exported but whose constructors are not, at a **use site outside the
declaring module**:

| Operation | Allowed? | Rationale |
|---|---|---|
| Name `T` in an annotation / signature / generic arg | **Yes** | The handle is exported; this is its purpose. |
| Bind, pass, return a `T` value | **Yes** | A value's identity does not require seeing its shape. |
| Call an exported function that takes/returns `T` | **Yes** | Ordinary use of the module's surface. |
| Construct `(T.Mk …)` / apply a bare ctor | **No** | The constructor name is not in scope. |
| `match` on `T`'s variants | **No** | Variant patterns are the constructor names; not in scope. |
| Strip `T` to its underlying structural value | **No** | The strip escape hatch would defeat opacity (below). |
| Built-in `=` / `compare` on two `T` values | **No** (default) | See "Equality" below. |

Inside the declaring module `T` is fully transparent, exactly as today — construction, matching,
strip, and structural equality all work. Opacity is a *boundary* property, not a property of the type.

### Equality and the strip escape hatch

The spec gives every nominal type a **strip-to-structural** reinterpretation
(`type-system.md` §A Nominal Value Is Convertible To Its Underlying Structural Value) and a
same-shape structural equality. Both would leak the hidden representation if available to an importer:
strip exposes the shape directly, and built-in structural `=` observes structural equality of the
private rep. To preserve the abstraction guarantee:

- **The strip reinterpretation is available only where the type is concrete** (declaring module, or a
  module that imported its constructors). An importer of an abstract `T` cannot strip it.
- **Built-in `=`/`compare` on an abstract `T` is rejected at the use site.** A module that wants its
  abstract type to be comparable exports a function (`(def (eq (: a T) (: b T)) …)`). This mirrors ML,
  where an abstract type carries no operations except those the signature publishes, and keeps the
  representation genuinely hidden. (Within the declaring module, `=` on `T` behaves exactly as the
  nominal rules already specify.)

This is the one place the proposal *adds* a requirement to `type-system.md` rather than only to
`modules-and-namespaces.md`: the strip reinterpretation and the built-in structural comparison are
gated on the constructors' visibility, not merely the handle's.

## Semantics at the boundary — compile-time only (intra-package)

Under the current package model (`package-linking-imports-landed`), a package's files are linked into
**one** arena and monomorphized by β-reduction into **one** component; imports are a compile-time
name-visibility overlay (`Db::load_linked` + `FileScopeTable`). Therefore:

- Opacity is enforced entirely by **name resolution**: the importer's `FileScopeTable` never binds the
  private constructor names, so `(T.Mk …)` / a `Nel.Mk` pattern is an ordinary "unbound name / not a
  visible constructor" rejection at resolve time.
- A `T` value that crosses a link boundary is just its underlying structural value — the nominal tag
  is compile-time-only and adds nothing (already true). There is **no runtime handle, no ABI change**.
- The abstraction is erased before emit: two identically-optimized programs, one using the type
  concretely inside the module and one using it abstractly across a link, emit **byte-identical** code
  for the shared operations. Opacity costs nothing.

**Cross-component (future, out of scope here).** When two *separately compiled* components exchange an
opaque `T`, the value cannot be a bare structural value (the other component has no shape for it) — it
becomes a **resource handle** in the exporting component's WIT world, exactly the mechanism closures
already use to cross the host boundary (`closures-across-host-boundary`). That is a natural extension,
not a prerequisite; the intra-package feature stands alone and is what the operator asked for.

## Diagnostics

Reuse the existing families; add one dedicated code for the sharp case so the fix is machine-actionable
(`diagnostics.md` / Amendment 0.5.0 — a rejection carries a verified route to a compliant program):

- **Constructing / matching an abstract type's variant outside the module** → new **CDZ0214**
  *"the constructor of an abstract type is not visible here"* (the `02xx` type/nominal family; `0213`
  is the last assigned). The diagnostic MUST name the type `T`, note that its handle is exported but its
  constructors are not, and — when the module exports a function whose result type is `T` — suggest that
  function as the way to obtain a value (the verified-fix obligation). If the name is simply not in
  scope at all, the plain `CDZ0101` unbound-name path already applies; `CDZ0214` is for the case where
  `T` *is* visible but its constructor is deliberately withheld, so the message distinguishes
  "you can't see this" from "this is hidden on purpose, use the door."
- **Stripping / built-in `=` on an abstract type outside the module** → **CDZ0202** (the existing
  nominal-boundary code): an abstract type's underlying shape is, from the importer's side, not a shape
  it may compare against or reinterpret.
- **Exporting a constructor whose type handle is not also exported** → **CDZ0201** ill-formed export: a
  public constructor that returns a type no importer can name is incoherent (the importer could build a
  value it cannot annotate or hold by name). Require handle-export whenever a constructor is exported.
  (`T.*` never trips this — it exports the handle by construction.)
- **A malformed wildcard export** — `*` as a bare export name, or `(. x *)` where `x` is not a type
  handle with constructors (a module record, a scalar, an abstract-from-elsewhere type) → **CDZ0201**
  ill-formed export, with a message steering to either the explicit list or a valid `T.*`. This keeps
  the reserved `*` segment from silently degrading into a name glob.

## Interaction with existing rules

- **Exhaustiveness (CDZ0210)** is unaffected: it is checked where a `match` is written, which is only
  where the constructors are visible (declaring module or a concrete importer). An importer of an
  abstract type cannot write the match at all.
- **The AST sum type** must be exported **concretely** — a compiler written in Cadenza walks it by the
  ordinary variant match (`type-system.md` §The AST Is An Ordinary Sum Type). The concrete-export form
  covers this; abstractness is opt-in per type, never forced.
- **Generics** compose: an abstract *generic* type `(type Stack a (Mk (List a)))` exported as
  `(export Stack push pop)` is opaque at every instantiation; the handle `Stack` takes its type
  argument as usual, monomorphized before the boundary (`type-system.md` §A Generic Definition Is
  Monomorphized Before The Component Boundary). Nothing special is needed — opacity is orthogonal to
  arity.
- **Newtype erasure** (`DESIGN-nominal-newtype-erasure-rcdzc`) composes cleanly and is in fact the
  *most common* abstract type: a single-variant `(type UserId (Mk Int64))` exported as `(export UserId
  parse render)` is an erased-at-runtime, opaque-at-compile-time newtype — zero-cost representation
  hiding, the whole point.
- **The single-module record form** `(module m …)` with `(. m x)` access: exporting `m`'s type `T`
  abstractly means `(. m T)` yields the opaque handle and `(. m T.Mk)` is a `CDZ0214` — the same rule,
  expressed through the record projector instead of the link overlay.

## Empirically confirm before starting (as the newtype doc did)

1. Cross-file **type** visibility is currently flat/unimplemented — `package-linking-imports-landed`
   §RESIDUALS (a): *"`(type …)` resolution NOT yet file-scoped."* So this is **greenfield**: there is
   no back-compat break, but it also means step 1 below (file-scoped type resolution) is a genuine
   prerequisite, not a tweak.
2. `(type T …)` binds `T`, `A`, `B` as separate resolvable names today (verify in `resolve.rs`
   `collect_user_types`), and `(. T A)` resolves straight to the ctor node — confirm the handle binding
   is separable from the ctor bindings so the export list can name them independently.
3. `(export …)` today resolves each name to a **func** index — confirm what happens when it names a
   **type** (likely a decline/`CDZ0101`); the core work teaches the export collector to accept a type
   handle and to record it in the module's public surface distinctly from its constructors.

## Increment plan

Land each increment green (`cargo xtask gate` 0 fail, `--check` clean, `cargo test -p rcdzc`). Spec
first, per repo discipline (a new gating requirement with no impl is a red promotion bar).

> **STATUS: COMPLETE — all increments landed on `spec` (5 commits).**
> - **A** (`@b4700bb9`) — **O1** file-scoped type + constructor resolution. Had to come first: before it,
>   cross-file type privacy did not exist at all (the flat `type_decl_index` let any file name any
>   sibling's type and construct its variants with no import). A user sum's identity is its declaration
>   (`type-system.md` §Nominal), not its shape — so a corpus case relying on two same-named `(type L …)`
>   being interchangeable (a forge-by-re-declare the spec forbids) was migrated to the import form.
> - **B** (`@a6a029d7`) — **O2 + O3 + most of O0**. The three `(export …)` forms (`T` abstract /
>   `(. T A)` partial / `(. T *)` wildcard concrete), `CtorVis` on the link surface, the
>   `add_type_to_file` ctor-visibility gate, `withheld_ctor_reject` → **CDZ0214**, the
>   `modules-and-namespaces.md` §"A Type's Handle And Its Constructors Are Independently Visible"
>   requirement (3 MUSTs, cited), and the corpus witnesses.
> - **doc** (`@e087a9f4`).
> - **O4** (`@1763fdf4`) — **ML-surface ergonomics.** `export { Color.* }` / `export { Color.Red, main }`
>   read + print (parser: `*` a reserved member segment after `.`, member-aware `brace_export_list`;
>   printer: `plain_key` admits `*`, `is_export_shape` accepts member-access elements). Units `Unit.*`,
>   multiply, float `*.`, positional `.0` all unaffected (full-corpus roundtrip 2059/0).
> - **O0-tail** (`@8b77b03f`) — **representation hiding.** A built-in `=`/`compare` on an abstract-type
>   value → **CDZ0202** (`Db::is_abstract_type_at`, gated at the compare site). `type-system.md` §"An
>   Abstract Type's Representation Is Not Observable Across Its Boundary" (comparison MUST enforced+cited;
>   strip MUST is the companion — no strip op exists yet, vacuous until one lands).
>
> Gate 2050 pass / 0 fail; feature usable on both surfaces; runs end-to-end.
>
> **Only leftover (a SEPARATE, larger feature — not opaque types):** recursive-sum SYNTHESIS is not
> file-scoped (a recursive `(type L …)` re-declared in two files splits its spine's type; the composing
> form is to import the type, not re-declare it). Fix = file-scope payload resolution in `sums::synthesize`.

- [x] **O0 — spec + corpus (spec-first).** *(Done except the type-system.md strip/`=` gating, deferred.)* Add to `modules-and-namespaces.md` §Visibility a requirement
  that a type declaration's handle and its constructors are *independently* exportable; that exporting
  the handle alone yields an abstract type whose constructors, match capability, strip, and structural
  comparison are not importable; and that the wildcard export form exports the handle together with its
  full constructor set, equivalent to naming each. Add to `type-system.md` §Nominal the gate on strip +
  structural `=` by constructor visibility. Register `CDZ0214`. Add witnesses to `11-modules.sexp`
  (abstract export: construct-outside → CDZ0214; explicit concrete export: construct-outside → runs;
  **`T.*` wildcard concrete export: construct + match outside → runs, and equals the explicit list**;
  malformed `*` export → CDZ0201) and `07-type-system.sexp` (strip/`=` on an abstract type → CDZ0202).
  Wire `traceability.md`.
- [x] **O1 — file-scoped type resolution.** *(Landed `@b4700bb9` — `FileScopeTable` gained
  `visible_types`/`visible_ctors`; `resolve_name` steps 3/3c consult them when linked.)* Make
  `type_decl_by_name` respect the `FileScopeTable`
  (the residual (a) above). A sibling file's type is invisible unless imported. Byte-neutral for
  single-file programs (`Db::load` path unchanged).
- [x] **O2 — export the handle, the constructors, or both — distinctly.** *(Landed `@a6a029d7`.)* The export collector accepts
  (a) a type name → records the handle *without* implying its constructors; (b) a `(. T A)` ctor name →
  records that one constructor public; (c) the wildcard `(. T *)` → records the handle **and** expands
  to every constructor of `T`'s current variant set (read off the resolved `SumDef`), the same public
  surface as naming each explicitly. A constructor is public iff named explicitly or covered by a `T.*`.
  `import` of an abstract type binds `T` (handle) but not its ctor names.
- [x] **O3 — reject construct/match on an abstract type across the boundary.** *(Landed `@a6a029d7` —
  `withheld_ctor_reject` → CDZ0214 for construct + match of a withheld ctor, bare or qualified. The
  strip + structural-`=` gating: the `=`/`compare` half LANDED `@8b77b03f` (CDZ0202); the strip half is
  vacuous until a strip op exists.)*
  Pure resolve-time visibility checks; no lowering change — an unbound constructor never reaches infer/lower.
- [x] **O4 — the concrete-export paths + AST + ML ergonomics.** *(Landed `@1763fdf4` — `export { Color.* }`
  / `export { Color.Red, main }` read + print; units/multiply/float/positional unaffected.)* Verify both concrete forms — the
  explicit ctor list and the `T.*` wildcard — make a sum importable-and-matchable and produce the
  **same** public surface, and that `(export Ast.*)` round-trips the AST type concretely. Then close the
  **ML-surface printer gaps** (measured, not free): (a) teach the `export { … }` printer to render a
  member-access child so `(export Color.Red)` prints `export { Color.Red }` instead of falling back to
  `` `export`(…) `` — this also fixes the *explicit* concrete export's ML rendering, a pre-existing
  papercut; (b) reserve `*` as a bare-safe member segment so `(. Color *)` prints `Color.*` rather than
  `` `.`(Color, `*`) ``. Both are `cadenza-syntax` printer/lexer changes with round-trip tests; the
  arena is unchanged. (s-expr already works end to end — verified — so O2/O3/O5 do not depend on O4.)
- [x] **O5 — end-to-end.** *(Verified: an ML-authored package `export { Color, mk }` abstract → the entry's
  `Color.Green` construction is CDZ0214 + `mk()` usage runs; `export { Color.*, rank }` concrete →
  construct+match runs, all through the real runtime.)* A two-file package: abstract lib + entry runs via
  the exported functions; a withheld-ctor construction → CDZ0214; a concrete `Color.*` package
  constructs + matches → runs.

## Traps to respect (from repo memory)

- Land only in this worktree; merge to `spec` via guarded CAS (`git update-ref refs/heads/spec HEAD
  <old-sha>`), never touch main's tree ([[landing-on-spec-checked-out-in-main]]).
- `cargo xtask build` a fresh runtime before gating, or heap-case verdicts are a false alarm.
- A new diagnostic code is a spec event: register it in the semantics README + the code registry, add a
  corpus witness that pins it BOTH ways (abstract → CDZ0214, concrete → runs), or the gate can't grade
  it ([[gate-error-code-matching]]).
- β-copy hygiene (`package-linking-imports-landed`): a synthesized node's file can be indeterminate;
  the visibility check must key on the *resolution* of the constructor name, not a raw StructId→file
  lookup, or an inlined cross-file body could see a private ctor. Decline-don't-miscompile if ambiguous.
- Diff the FAIL set, not the pass count (P/todo drift is worktree-local).
