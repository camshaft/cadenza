# PR #1654 review comment — xtask/src/main.rs (v-rust-backend) — OPEN

https://github.com/camshaft/cadenza/pull/1654 (higher-order closure round-trips cross the Rust export
boundary — host-closure S4). Substantive harness-codegen correctness point.

## Substring type-pairing false-matches a higher-order producer to a first-order consumer param (Copilot, main.rs:2590/2630) — correctness [VERIFIED]
> `build_closure_consumer_call` now treats any non-factory `pub fn` (including higher-order producers
> like `mk(f: Rc<dyn Fn..>)->i64`) as a peeled producer candidate, but the pairing predicate `ty_matches`
> uses substring checks (`closure_ty.contains(cty) || cty.contains(closure_ty)`). For higher-order closure
> types the OUTER erased text contains the INNER closure text, so a higher-order producer can falsely
> match a first-order consumer param (e.g. `Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->i64>` contains `Rc<dyn
> Fn(i64)->i64>`), → nondeterministic mis-pairing by module order and potentially ill-typed Rust calls.

VERIFIED against the code: `ty_matches` (main.rs:2622) computes `let erased_ok = closure_ty.contains(cty)
|| cty.contains(closure_ty.as_str())` (:2630) — a pure substring containment. A higher-order producer's
erased type literally CONTAINS a first-order consumer param's type as a substring, so `erased_ok` fires
spuriously. The `shape` guard only NARROWS when BOTH shapes are known (`_ => true` otherwise, :2637), so a
shape-less higher-order producer bypasses it. Result: nondeterministic mis-pairing by module/source order
→ the harness can synthesize ill-typed Rust closure calls. MED (test-harness codegen, not shipped runtime,
but it can generate uncompilable/wrong test drivers → false gate signal). Fix: match erased closure types
by STRUCTURAL EQUALITY (or a balanced-paren-aware compare), not substring containment; or require the
shape guard for higher-order producers. Recommend v-rust-backend tighten before/after land.
