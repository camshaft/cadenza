# Design — spread in collection CONSTRUCTION: `[a, b, ..c, d, ..e]` / `#list(a b ..c d ..e)`

**Author:** design pass (fleet `v-syntax`), at concierge/operator request.
**Audience:** the operator (a build-it? + scope decision, below) + the `vertical`s that build it —
`v-syntax` (surface + reject relaxation, mostly done) and `v-inference`/`rcdzc` (typing + lowering).
**Status:** DESIGN — PROPOSAL. **Operator decision PENDING** on (1) build it at all, and (2) scope:
lists-only vs. map/set parity. **The build is HELD** until that go; this note exists to inform it.
**Subsystem:** spans `cadenza-syntax` (surface — already parses), `rcdzc` (`resolve` CDZ0201 guard +
`lower` construction) and typing (`v-inference`).

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
on the binding side), so the unifier work is symmetric and small.

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

## 6. Scope decision for the operator

1. **Build it?** (language feature — operator call.)
2. **Scope:** **lists-only** (simplest, covers the operator's exact example) **vs. map/set parity**:
   - map: `#{..m1, k = v, ..m2}` → `Map.union` folds (later key wins — needs a merge-order rule);
   - set: `#(..s1, x, ..s2)` → `Set.union` folds.
   Recommendation: **do lists first** (bounded, unblocks the ask), and add map/set parity as a fast
   follow if wanted — same segment-and-fold mechanism, plus `Map.union`/`Set.union` and (for map) a
   last-writer-wins rule to pin.

## 7. Ownership / build split (on operator go)

- `v-syntax` (me): surface is already done; I own the CDZ0201-relaxation (scope it to direct
  list/map/#list construction children) + the printer round-trip pin + a corpus/guide surface example.
- `v-inference`/`rcdzc`: the spread-operand typing rule (§3) + the construction lowering fold (§4) +
  the const-hoist decline (§5).
Coordinate the split before building; the two halves meet at the arena shape `(list … .. s …)`, which
is unchanged.
