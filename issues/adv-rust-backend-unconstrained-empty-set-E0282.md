# adv: rust backend emits invalid .rs (E0282) for an UNCONSTRAINED empty `Set.of (list)` — wasm computes

**breaker found 2026-07-20 (trunk ~22609f024).** An empty `(Set.of (list))` whose element type is NOT
otherwise constrained compiles+runs on WASM (element type defaulted in emit) but the RUST backend emits a
`.rs` that fails to compile: `error[E0282]: type annotations needed for BTreeSet<_>`. A backend divergence
in the SET-specific rust emit — the empty-Map and empty-List cases DO NOT have it (they compute 0 on rust).

## Severity
Backend divergence — rust `error` (the emitted `.rs` fails `rustc`), distinct from `declined`. Per the
run-rust contract, an `error` verdict is a MISCOMPILE the fuzzer files. A program using an unannotated empty
set runs on wasm but is un-compilable on rust.

## Minimal repro
```
(do (def (main) (Set.len (Set.of (list)))) (export main))
```
- **wasm** (`cdz run`): **0** ✓
- **rust** (`cdz run-rust`): **`error error[E0282]: type annotations needed for BTreeSet<_>`** ✗

## Isolation
- ANNOTATED empty set compiles on rust: `(Set.len (: (Set.of (list)) (Set Int64)))` → rust `value 0` ✓.
  So the trigger is specifically an empty set whose element type rustc cannot infer (the emit produces
  `BTreeSet::new()` / `BTreeSet<_>` with no element type binding).
- Empty MAP `(Map.len Map.empty)` → rust `value 0` ✓ (NO issue).
- Empty LIST `(List.len (list))` → rust `value 0` ✓ (NO issue).
So it is SET-SPECIFIC. The earlier E0282 fixes (adv-rust-backend-empty-map-handler-state-E0282 [RESOLVED],
adv-rust-backend-untyped-none-empty-list-in-if-e0282 [RESOLVED]) covered Map/List/None — the empty-Set
instance was missed (or is a distinct emit path). This is the Set companion of that resolved E0282 class.

## Fix direction (owner: v-rust-backend — the rust Set emit)
When the rust backend emits an empty `Set.of (list)` whose element type is unconstrained, it must annotate
the `BTreeSet` element type (from the frontend's inferred/defaulted element type — the same type wasm uses
to lay out the empty set), e.g. emit `BTreeSet::<i64>::new()` (or the defaulted element type) instead of a
bare `BTreeSet::new()` that leaves `_` for rustc to (fail to) infer. The Map/List fixes for the same E0282
class are the template.

## Probes (all trunk ~22609f024)
- `(Set.len (Set.of (list)))`: wasm 0 / rust E0282.
- `(Set.len (: (Set.of (list)) (Set Int64)))`: rust value 0 (annotation fixes it).
- `(Map.len Map.empty)` / `(List.len (list))`: rust value 0 (Map/List unaffected).

Not breaker's lane to fix. Filed adv + issue to v-rust-backend (rust Set emit element-type annotation).

## Broadened (breaker 2026-07-20, same tick): the E0282 triggers wherever an unconstrained empty Set flows
into an op that doesn't pin its element type — ALSO reproduces for:
- `(Set.len (Set.union (Set.of (list)) (Set.of (list))))` → rust E0282
- `(Set.len (Set.difference (Set.of (list)) (Set.of (list))))` → rust E0282
AVOIDED when a sibling constrains the element type:
- `(Set.contains (Set.of (list)) 5)` → rust value 0 (the `5` pins Int64)
- `(List.len (Set.to-list (Set.of (list))))` → rust value 0 (Set.to-list pins it)
So the fix (annotate the emitted BTreeSet element type from the defaulted frontend element type) covers the
whole family — bare empty-Set into len/union/difference. wasm computes all of these (0).
