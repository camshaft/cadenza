# Design — spread in collection CONSTRUCTION: `[a, b, ..c, d, ..e]` / `#list(a b ..c d ..e)`

**Author:** design pass (fleet `v-syntax`), at concierge/operator request.
**Audience:** the `vertical`s that build it — `v-syntax` (surface — done; printer pins + corpus/guide)
and `v-inference`/`rcdzc` (CDZ0201 relaxation + typing + lowering). Build sequenced list → record/map/set.
**Status:** DESIGN — **APPROVED, BUILD GO** (operator, 2026-08-29, via concierge). **Full scope: all
four collection types — list + record + map + set** (NOT lists-only). Un-held. Sequenced list-first
(unblock the exact ask) then record/map/set via the SAME segment-and-fold. Operator, verbatim: *"Yes
please build literal list construction.. same goes for record, map, and set construction. All of those
would be really nice to have."*
**Subsystem:** spans `cadenza-syntax` (surface — already parses), `rcdzc` (`resolve` CDZ0201 guard +
`lower` construction) and typing (`v-inference`).

## 0a. ⚠ ARENA-SHAPE CORRECTION (operator, 2026-08-29): the spread is `(.. v)` EVERYWHERE

**Operator, verbatim:** *"for the spread operator i want it to be `(.. v)` in sexpr. so like
`#record((= k v) (.. other-record))`"* and *"i want the `(.. v)` operator to be everywhere. patterns
and constructors. it's a lot more consistent imo. otherwise we're putting an infix operator in sexpr
and it just feels wrong."*

So the arena marker for a spread/rest is a **self-contained wrapped node `(.. operand)`** (a `List`
whose head is the `..` `Name`), in **BOTH construction AND pattern** position — NOT the legacy **flat**
`Name("..")` + next-sibling marker. Repo-wide shape:

| | legacy flat (being replaced) | NEW `(.. v)` (operator) |
|---|---|---|
| list ctor | `(list a .. c)` | `(list a (.. c))` |
| record ctor | — | `(record (= k v) (.. r))` |
| map ctor | — | `(map (= k v) (.. m))` |
| set ctor | — | `(set x (.. s))` |
| list pattern | `(list a .. rest)` | `(list a (.. rest))` |
| map pattern | `(map (= k v) .. rest)` | `(map (= k v) (.. rest))` |

This supersedes §1's "flat `Name("..")` marker" description and the flat shape landed in PR #5826.

**Staged migration (M2/M3-style, no flag-day / broken window):**
- **Phase 1 (`v-inference`/`rcdzc`, ADDITIVE, lands alone):** every rest-marker scan
  (`position(|e| as_name(e) == Some(".."))` in `resolve.rs` list/map pattern destructuring, `compile.rs`
  cover, `accum.rs`, `db.rs`, `eval_ast.rs`) recognizes a `(.. operand)` **child** in ADDITION to the
  flat marker. No behavior change (flat still works). A helper `spread_operand(a, child) -> Option`
  (`Some` iff `child` is a `(.. x)` node) at each site, wrapped-first then flat-fallback.
- **Phase 2 (`v-syntax`, after Phase 1):** the surface PRODUCES `(.. operand)` — parser `rest_marker`
  wraps; ML printer renders `(.. operand)` as `..operand`; the sexpr reader NORMALIZES a read flat
  `.. x` → `(.. x)` (back-compat, so old corpus text still reads); printers emit `(.. v)`. The arena is
  now always wrapped; Phase-1 recognition handles it. Corpus text may stay flat (reader normalizes).
- **Phase 3 (cleanup):** migrate corpus text `.. rest` → `(.. rest)` (cosmetic); drop the flat
  recognition + the reader normalize.

## 0. The ask, and where we are — READ FIRST

**Operator (verbatim, via concierge):** "Do we have the ability to concat lists with `..` patterns?
Like `[a, b, ..c, d, ..e]`, and would it compile into push/concat calls if that makes sense? That's a
lot more readable."

**Current state — surface YES, compiler NO.**
- The **surface already parses it.** `..` is a lexer token (`Kind::DotDot`), and the parser's
  `rest_marker` runs **per element** inside `list_literal`, so `[a, b, ..c, d, ..e]` — multiple,
  interior spreads — parses today to the flat arena `(list a b .. c d .. e)` (a `Name("..")` sibling
  immediately followed by each spread operand). The s-expr twin `#list(a b ..c d ..e)` accepts the same
  flat form. This is the *identical* marker shape the collection **pattern** rest uses.
- The **compiler rejects it in value position.** `resolve.rs` emits **CDZ0201**: "`..` is a rest/spread
  marker, valid only inside a `(list …)` or `(map …)` PATTERN … it is not a value or a form head here."
  So `..` spread works ONLY for destructuring (`(list a .. rest)`, `(map (k v) .. rest)`) today; a
  construction spread does not compile.
- **Lowering primitives already exist:** runtime `List.push` / `List.concat` (`vec-push`/`vec-concat`)
  and a derived phase-1 lowering `concat(list-new(elem), list)` in `lower.rs`.

So the missing pieces are narrow: (a) relax the CDZ0201 guard for construction, (b) a construction
lowering that folds segments into `concat`, (c) the spread-operand typing rule. Assessment: **bounded
build, no new primitives** — but the typing rule + a few semantic choices are worth pinning here first.

## 1. Surface (settled — no change needed)

Both surfaces already produce the flat form; the feature adds NO new grammar:

| | surface | flat arena |
|---|---|---|
| ML | `[a, b, ..c, d, ..e]` | `(list a b .. c d .. e)` |
| s-expr | `#list(a b ..c d ..e)` | `(list a b .. c d .. e)` |

`..` binds one operand (the element immediately after it); an inline element is itself. Order is
preserved. Nothing in the lexer/parser changes.

## 2. Semantics — eager, left-to-right concat

A construction spread is **eager** (construction is value position — no laziness): the literal
evaluates to a fresh list equal to the in-order concatenation of its segments, where an inline run
`x, y` is the singleton-append sequence and a spread `..c` is `c`'s elements inlined. E.g.
`[a, b, ..c, d, ..e]` ≡ `List.concat(List.concat(List.concat([a, b], c), [d]), e)` (associativity is
free — `concat` is associative; the lowering picks whatever shape is cheapest, §4).

## 3. Typing rule (the one real design point — coordinate with v-inference)

For a construction of element type `T`:
- an **inline** element must have type `T` (as today);
- a **spread** operand `..s` must have type `List<T>` — i.e. the spread's element type unifies with the
  literal's element type. `[1, ..xs]` forces `xs : List<Int64>`; `[..a, ..b]` unifies `a`,`b` to the
  same `List<T>`; an empty literal with only spreads infers `T` from the spreads.
- **result type** is `List<T>`.
This is the list analogue of the pattern-rest typing (`(list a .. rest)` already types `rest : List<T>`
on the binding side), so the unifier work is symmetric and small. **Per type** (§6): set spread is
`Set<T>`, map spread is `Map<K,V>`, and **record** spread is the one exception — a `..r` contributes
`r`'s whole ROW (its field set may be only partially static), so record-merge typing is a row union, not
a single element type; it touches the `Record.with`/`Record.extend` row-ops neighborhood (coordinate
with `v-inference`).

## 4. Lowering (rcdzc — segment then fold)

Relax CDZ0201 to ALLOW `..` **only as a direct child of a `(list …)`/`#list(…)` construction** (and, if
scoped in, `(map …)`) — NOT a blanket value-position `..` (a bare `(.. x)` / `(f ..)` stays CDZ0201).
Then lower `(list … .. s …)` by scanning for the `Name("..")` markers, segmenting into maximal inline
runs and spread operands, and folding left-to-right:
- inline run `[x, y]` → `List.new`/`list-new` of those elements;
- spread `..s` → `s` itself (already a `List<T>`);
- combine adjacent segments with `List.concat`.
Reuse the existing `vec-concat`/`vec-push` ops + the derived `concat(list-new(elem), list)` lowering; a
leading `..s` followed by a single inline `x` is exactly the existing `concat(list-new(x), list)` shape
run in reverse, so no new runtime op is needed.

### 4a. Implementation mechanism (rcdzc — CONCRETE, resolves §4's hand-wave; v-ast-compound, 2026-09-01)

§4 said "segment then fold" without pinning HOW, and the shipped IR forces a specific realization.
Findings from a build spike:

- **`Core` nodes reference AST `StructId` OCCURRENCES, not boxed `Core`.** `Core::ListNew { elems:
  Rc<[StructId]> }` and `Core::ListConcat { lhs: StructId, rhs: StructId }` (core.rs ~419/443) both hold
  occurrence ids that the backend RE-LOWERS via `core_of`. So a fold cannot be built by nesting `Core`
  values directly — each `ListConcat` arm needs an AST occurrence that itself lowers to a list.
- **NO new IR is needed.** The existing `List.concat` fold already collapses two constant `#list` operands
  into one `Core::ListNew` and otherwise emits the runtime `Core::ListConcat` (compute.rs ~2315). So the
  desugar target is an **AST-level rewrite into `(List.concat …)` calls over synthetic `#list(…)`
  inline-run nodes**, then normal lowering handles everything (fold, typing, const-bake) for free.
- **Synthesis primitives exist:** `db.push_compound(CompoundCtor::List, children)` (a `#list(…)` node),
  `db.push_name("List.concat")`, `db.push_list([...])` (db.rs ~3871-3979) — the same family
  `lower/match_desugar.rs`'s `fuse_match_into_if` uses.

**The desugar (list):** segment the ctor children into maximal inline runs and spread operands via
`db.ast.as_form(elem, "..")` (`Some([operand])` iff the child is a `(.. operand)` node). Then fold
left-to-right: an inline run `[x, y]` → `push_compound(List, [x, y])`; a spread `..s` → `s` (the operand
occurrence, reused as-is — already a `List<T>`); combine adjacent segments with a synthesized
`(List.concat A B)` call. A single leading spread with no inline is just `s` copied (`concat([], s)`).

**⚠ THE REPARENTING TRAP (must handle, per match_desugar's precedent):** reusing an element occurrence
(`x`, or the spread operand `s`) as a child of a SYNTHETIC node reparents it — "a single node cannot have
two parents; push_list reparents" (match_desugar.rs:26). The resolver resolves a name by walking UP the
AST to the nearest binder, so a reused occurrence `n` (a bound param) MUST still reach its enclosing
scope after reparenting. The synthesized top-level concat node has to be spliced where the original
`(list …)` node sat (inherit its parent), and — as match_desugar does — reused sub-nodes may need
re-resolution. This is the one real correctness risk; get it right before landing (test a spread whose
inline element AND spread operand are both enclosing-scope binders, e.g. `#list(1 (.. xs) n)` with `n` a
param — exactly the `cspr1` fence).

**Consumer sites that must treat a `(.. operand)` list child as the operand (else the `..` head resolves
→ CDZ0201):** (1) `lower/compute.rs` `Resolved::List` arm (~714) — the desugar entry; (2) `infer.rs`
`type_of` `Resolved::List` (~586) and (3) `reflected_ty` `Resolved::List` (~3695) — a spread elem
contributes `peel_list(type_of(operand))` to the element-type join, not `type_of(elem)`; (4) the
error-collection walk that surfaces the CDZ0201 — must recurse into the operand, not the `(.. )` wrapper.
Doing (1)-(4) means the bare-`..` `resolve_name` reject (resolve.rs:1090) stays intact for a `..` that is
NOT a direct construction child, so NO blanket resolve relaxation is required (the reject simply never
fires for a properly-desugared construction child). §5's const-hoist decline: a spread-bearing literal is
not a constant compound — the desugar produces a `ListConcat` (runtime) whenever a spread operand is
runtime, so the const path is not reached; a fully-constant spread (`#list(1 (.. #list(2 3)) 4)`) folds
to one `ListNew` via the existing `List.concat` constant-fold, which is correct (still a fresh list).

**Corpus witness that auto-flips:** `cspr1` (ch05 ~34981) — `#list(1 (.. xs) n)` must build `[1,10,20,n]`
— currently declines CDZ0201 (`todo`), flips to PASS when this lands. Add sibling witnesses per edge case
(empty spread, all-spread, leading/trailing/interior, constant-only fold).

**Edge cases (all lower cleanly with the fold):**
- **no spread** — the ordinary `(list …)` construction (unchanged; NOT this path).
- **single spread, no inline** — `[..c]` ≡ a copy of `c` (`List.concat([], c)` or just `c`-copy; pick the
  copy so identity/aliasing matches a fresh-list literal).
- **all spreads** — `[..a, ..b]` ≡ `concat(a, b)`.
- **empty spread** — `..c` with `c = []` contributes nothing (concat with empty is identity).
- **leading/trailing/interior spreads** — all handled uniformly by the segment-and-fold (this is why the
  parser accepts `..` per-element, not just trailing).

## 5. Interaction with the native `#list` value-render / const-hoist (IMPORTANT)

A spread literal is **NOT a constant compound** even if some elements are constant: it depends on the
runtime spread operands. So the **const-compound-hoist / build-once path must DECLINE a spread-bearing
`(list …)`** and fall through to the concat-fold lowering. (A fully-constant *non-spread* list literal
still hoists as today.) The native `#list(…)` VALUE render (`#ctor` form) is unaffected — a spread
literal is a *construction expression*, not a value literal, so it never reaches the value-render path;
it renders as its source `#list(a b ..c d ..e)` (surface round-trip, already handled by the printer's
flat `..` emit).

## 6. Scope — DECIDED: all four (list + record + map + set)

Operator ruling (2026-08-29): build **all four** collection constructions with spread, sequenced
list-first then record/map/set (same segment-and-fold, per-type combine op):

| type | spread surface | combine op | duplicate rule |
|---|---|---|---|
| list | `[a, ..c, d]` / `#list(a ..c d)` | `List.concat` | order-preserving, no dedup |
| set | `#(..s1, x, ..s2)` / `#set(..s1 x ..s2)` | `Set.union` | dedup (set semantics) |
| map | `#{..m1, k = v, ..m2}` / `#map(..m1 (= k v) ..m2)` | `Map.union` | **last-writer-wins** on key (left→right) |
| record | `{ ..r1, a = 1, ..r2 }` / `#record(..r1 (= a 1) ..r2)` | record-merge (row) | **last-writer-wins** on field (left→right) |

**Per-type notes:**
- **list** — §2/§4 as written; `List.concat` fold, order preserved.
- **set** — `Set.union` fold; result dedups (a spread element already present is absorbed) — that IS set
  semantics, not a surprise.
- **map** — `Map.union` fold, **last spread/entry wins** on a duplicate key, matching a left-to-right
  overwrite reading (`#{..defaults, key = override}`); this is the readable intent.
- **record** — record-**merge** (a row operation over statically-known + spread fields), **last wins** on
  a duplicate field name, same left-to-right rule as map. NB: a record spread's field set may be only
  partially static (a spread `..r` contributes `r`'s row); typing (§3) must handle the row union. This is
  the one type whose typing is more than "element T" — coordinate closely with `v-inference` (it touches
  the record row-ops path, `Record.with`/`Record.extend` neighborhood).

Empty-spread, all-spread, single-spread (§4) apply per type. The const-hoist DECLINE (§5) applies to all
four (a spread-bearing literal of any type is not a constant compound).

## 7. Ownership / build split

- `v-syntax`: surface is already done (all four types parse the flat `.. s` marker); owns the printer
  round-trip pins + corpus/guide surface examples + this note. Nominally also the CDZ0201-relaxation —
  but see the coupling below.
- `v-inference`/`rcdzc`: the per-type spread-operand typing rule (§3, incl. the record row-union) + the
  segment-and-fold lowering (§4/§6) + the const-hoist decline (§5).

**Coupling — the CDZ0201 relaxation must co-land with the lowering.** The relaxation lives in
`resolve.rs` and the fold in `lower.rs` (both rcdzc); relaxing the reject WITHOUT the lowering leaves
`..` accepted at resolve but unhandled at lower (a half-built window). So the rcdzc-side change is ONE
atomic unit PER TYPE — recommended owner: `v-inference`/`rcdzc` lands {relaxation + typing + lowering +
hoist-decline} together, `v-syntax` co-lands the surface round-trip pin + corpus example for that type.
The relaxation scope: allow `..` ONLY as a direct child of a list/record/map/set CONSTRUCTION node (not
blanket value position — a bare `(.. x)`/`(f ..)` stays CDZ0201). The two halves meet at the unchanged
flat arena shape `(<ctor> … .. s …)`.
