## 13. 🟢 PARTIAL-DONE 2026-07-07 (spec + STATIC path; RUNTIME half re-tagged) — RE-PROBED

**Landed:** spec clause `core-semantics.md` §Pattern Matching *"A List Is Deconstructed By Element
Patterns With An Optional Rest"* (element patterns `(list)`, `(list a b)`, `(list x .. rest)`;
representation-opaque; ZERO proper names) + the STATIC/const-fold seed lowering (`try_match_list`
`list`-headed arm: length-check, recursive leading binds composing with tuple/ctor, `rest`=fresh
`(list …)` SUB-NODE). RE-PROBED via the oracle: `(match (list 10 20 30) ((list) 0) ((list x .. rest)
x))`→10, fixed-arity→15, empty→1, nested-tuple-element→3, zero-leading→whole list; malformed `(list x
..)` declines cleanly. Corpus: NEW realized case *"an element pattern matches a list by its length and
elements"* passes (`list-patterns` ∈ REALIZED). Probe `list_element_patterns_over_a_static_scrutinee`.
Gate: behavior 575/0, ignition, cc-vs-Rust 580/0, cargo test green.

**Deferred (re-tagged `list-pattern-runtime-tail`):** the RUNTIME-recursion fold — the ORIGINAL pinned
case `(def (sum xs) (match xs ((list) 0) ((list x .. rest) (+ x (sum rest)))))`→60, where `xs` is a
PARAMETER (Kind::Heap) — needs a materialized list TAIL for the rest binder (a runtime list-tail
primitive / `List.rest`). That is RUNTIME-side (needs the value-heap runtime to gain the op + rebuild),
currently owned by the concurrent CHAMP-runtime agent. It now declines HONESTLY *"runtime list
element-pattern (rest binder) needs a list-tail primitive"* and its corpus case skips behind
`list-pattern-runtime-tail`. Learning: `list-element-rest-pattern-static-vs-runtime` (memory). Original
below.

---

## 13. 🔴 The built-in `list` has no pattern-matching surface — spec addition needed

**Finding.** The built-in `list` cannot be pattern-matched at all: `(cons h t)`/`nil`, positional
`(list a b c)`, and empty `(list)` all decline "unsupported list pattern"; `(List.Cons …)` gives
"runtime sum match on an undeclared variant". `core-semantics.md` §Pattern Matching specifies tuple
and sum-constructor patterns but says **nothing about lists** — so this is unspecified, not just
unimplemented. A `list` is consume-only via `List.at`/`List.len` + index recursion.

**Why it touches the spec.** Pattern matching is a core-semantics surface, and "how is a list
deconstructed" is a hole in it. The gap shapes *every* list-consuming pass a compiler writes (module
def list, code stream, CBOR array children), forcing each to hand-roll a custom cons-sum (`(type FList
(FNil | FCons …))`) that duplicates the persistent sequence the language already has — the single
biggest ergonomic gap for authoring the compiler idiomatically. It is a language-surface decision (a
MUST about how `match` sees a list), the operator's to make, not seed-only work.

**Proposed design (element patterns with a rest binder — keeps representation opaque).** NOT Lisp
`Cons`/`Nil` (a `list` is a persistent tree, not cons cells; exposing cells leaks a hidden
representation). ML/Rust-style instead:
```
(match xs
  ((list)           empty)               ; exactly zero elements
  ((list x)         one x)               ; exactly one (sugar for a length check)
  ((list x .. rest) first x, tail rest)) ; first element + the rest AS A LIST
```
An exhaustive fold needs the empty case and a rest-pattern case; fixed-arity cases are length-check
sugar. The matcher asks `len`/`first`/`rest` (expressible over existing `List.at`/`List.len`/`List.slice`),
so the representation stays opaque. Spec: a new `core-semantics.md` §Pattern Matching clause *"A List
Is Deconstructed By Element Patterns With An Optional Rest"*; plus corpus cases + seed lowering.

**Status.** 🔴 **Operator decision (spec addition).** Pinned by `05-compound-types.sexp` *"the built-in
list is folded by an element-with-rest pattern"* (`(match xs ((list) 0) ((list x .. rest) (+ x (sum
rest))))` → 60), tagged `(needs list-patterns)` so it **skips** until specified+realized. Once landed,
the compiler's `Code`/`FList`/`DList` cons-sums collapse to the built-in `list` — a real simplification.
Learning: `spec/learnings/2026-07-07-the-built-in-list-cannot-be-pattern-matched.md`.

---
