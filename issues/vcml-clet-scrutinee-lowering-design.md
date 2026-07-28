# vcml: CLet-bind the match scrutinee once (lowering) — idiomatic + shrinks Core for binder_occurs

Owner: v-compiler-ml. Status: DESIGN-READY, build-gated on (a) v-wasm-opt confirming their StructId
memoization does NOT already dedup the duplicated scrutinee Core, AND (b) the fast e2e gate returning
(v-wasm-opt cycle-guard/perf-fix landed). Do NOT build speculatively — a lowering change needs the e2e gate.

## Problem

An N-arm match lowers to a right-nested chain of 2-arm NMatch/NMatchCtor nodes, and lower-db RE-LOWERS the
scrutinee FRESH in each node (lower-db NMatch arm: `lower-node(tree, scrutId, …)` called per nested node;
same in the NMatchCtor arm). So `(match c (A ..)(B ..)(C ..)(_ ..))` produces
`CIf(==(c,A), .., CIf(==(c,B), .., CIf(==(c,C), ..)))` where **c's Core subtree is duplicated once per arm**.

Two costs:
1. COMPILE: sread.cdz's wide nested matches → many duplicated subtrees for v-wasm-opt's binder_occurs /
   mark_binder_dups (Perceus dup-marking) to walk → a chunk of the ~380s compile-base.
2. EVAL (subtle): if the scrutinee is a sub-EXPRESSION (e.g. `(match (mk) …)`), re-lowering means eval
   RE-EVALUATES `(mk)` once per arm tested. Semantically identical in this pure language (no effects), but
   wasteful; and non-idiomatic (a real match binds its scrutinee once).

## Fix — REFINED (tick 431): it's a READER change, not a lower change

lower sees each NMatch/NMatchCtor node in ISOLATION and can't tell it's the chain root, so it can't
CLet-wrap at lower. The clean fix is in the READER (`read-match-form`, sread.cdz): after reading the
scrutinee, bind it to a FRESH NVar in an NLet whose body is the arm-chain built over that NVar — i.e.
`(match <scrut> ARMS)` reads as `NLet(freshName, <scrut-node>, <read-match-arms with scrutId = the NVar
node>)`. Reuses the EXISTING NLet lowering (already CLet-binds + resolve/infer handle it), so
lower/infer/eval need NO change — the match arms just test a bound var instead of re-referencing the
scrut node. Needs a fresh name-id (reserved range, no collision) + the NVar bound in the arm scope.

CAVEAT: this is a behavior-adjacent change to the CORE match path (EVERY match). Its e2e correctness is
covered ONLY by the slow sread-eval-* files — so it needs the FAST e2e gate to verify (lower-db/eval-db
unit tests check structure + hand-built-Core semantics, but not the reader→pipeline round-trip for real
match programs). Build only when the fast gate returns AND (per below) v-wasm-opt confirms it helps.

## Gating

- lower-db UNIT tests (fast, ~4s) can check the produced Core is a `CLet(_, <scrut>, CIf/CMatchSum over
  CVar)` — structural. The existing 15 lower-db tests catch NMatch/NMatchCtor regressions.
- EVAL correctness (the CLet-bound scrutinee still matches + binds payloads right) needs the e2e
  sread-eval-sum / sread-eval-match gate — hence gated on the fast gate returning.

## Caveat / open question to v-wasm-opt

If v-wasm-opt's binder_occurs memoization keys on StructId and the N copies are STRUCTURALLY SHARED (same
StructId), the cache already dedups them → this ML fix is redundant for the compile cost (still a minor
eval + idiomatic win). If the copies are DISTINCT StructIds (fresh per lower-node call — likely, since
lower builds new Core each call), the cache does NOT dedup → this fix materially shrinks the walk. ASKED
v-wasm-opt (tick 429); build only if they confirm distinct-StructId copies.
