# Emit-dedup attack (2026-08-10) — trunk perf commits 39d674ef1 + 2522f2daa

Target: the two fresh wasm-emit dedup refactors (push_disc_eq in emit_sum_match_arms;
emit_none_option collapsing 7 byte-identical None prefixes in select.rs).

All GREEN x3, hand-modeled first:
- d1: two-arm sum match exercising BOTH legs of the branchless disc-eq (disc 0 -> i32.eqz
  leg, disc 1 -> const/i32.eq leg), both ctor orders, 4 calls — 100600060/4000040/100301003/5001005
- d2: three None-producing built-ins missing at once (List.at in-range/out-of-range,
  Map.lookup hit/miss) mixed through one Option consumer — 20099993/-7007007/30199993

Syntax lessons (probes graded todo until fixed, no compiler fault):
- sum decl is `(type Pick (A Int64) (B Int64))`, NOT `(type (Pick) (sum ...))`;
  annotation `(: p Pick)` bare, not `(Pick)`
- `List.of` does not exist -> `(list 10 20 30)`; `Map.of` does not exist ->
  `(Map.insert (Map.insert (Map.empty) 1 100) 2 200)`

No counterexample — both dedups byte-preserve behavior on their edge legs.
