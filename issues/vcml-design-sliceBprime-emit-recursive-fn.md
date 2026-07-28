# Slice B' — EMIT a recursive wasm function + call (run-EMITTED fac, not just eval)

**Goal:** a self-recursive user function (`fac`) compiles to REAL wasm — a function per recursive def + a
`call` instruction — so `cdz-run` runs the emitted module to 120 (today only the EVAL interpreter runs it;
emit declines CCall). W4 differential: emitted-run == eval. Concierge ENDORSED (tick-134); foundation-first
before generics.

## Current emit-db model (tick-135 scoping — single-function only)
- `emit-wasm-module(c)`: ONE `main` fn. Sections: `type-section()` = one type `() -> i64` (0 params/1 result,
  bytes `01 60 00 01 7e`); `func-section()` = 1 func (`01 00`); `export-section()`; `code-section(body)` = one body.
- `emit-instrs(c, binders)`: CNum→i64.const, CVar→local.get idx, CBin→postfix, CLet→local.set, CIf→if/else.
  **CCall → `Bytes.of([])` (unreached; `can-emit` returns FALSE for CCall — line 121, gates it out).**
- `collect-binders` → pre-order CLet binder list = local indices. `body-of(instrs, nlocals)` = locals vec + instrs + `0b`.
- `emit-src-for` = `lower-tree(tree,root)` (single main Core) → `target-emit`. NO def-env (unlike eval's run-of-db).

## B' plan (5 parts — each its own gated MR)
1. **Bp1 — multi-arity type section + funcidx model.** Emit a type per distinct recursive-def arity `(i64^N)->i64`
   + keep main's `()->i64`. Build a def→funcidx map (mirrors lower-def-env: enumerate self-recursive defs).
   Foundation only; main still emits as today. Gate: emit-db unit test that a 1-param-type section encodes right.
2. **Bp2 — params as locals + a single recursive-def body.** A def's N params occupy locals 0..N-1 (before its
   CLet binders which shift by N). Emit ONE recursive def's code body (fac) standalone via a `lower-def-env`-style
   standalone lower (already have it) → emit-instrs with a param-aware binder list. Gate: the emitted fac body
   bytes validate (a helper that params→local.get).
3. **Bp3 — CCall arm in emit-instrs → `call <funcidx>` (0x10).** Emit each arg (emit-instrs), then `10 <uleb funcidx>`.
   Add CCall to `can-emit` (recursive-callee only). Gate: emit-instrs of a CCall encodes call+args.
4. **Bp4 — assemble the multi-function module.** func-section lists [fac-typeidx, main-typeidx]; code-section
   has [fac-body, main-body]; main calls fac. `emit-src-for` builds the def-env (self-recursive defs) like
   run-of-db does, threads funcidxs. Gate: `emit-src("(do (def (fac n) …) (def (main) (fac 5)) (export main))")`
   → Some(bytes); `cdz-run` → 120.
5. **Bp5 — W4 differential + sread pin.** A sread-eval-fns/emit pin: emitted-run(fac 5) == eval(fac 5) == 120.
   Mutual recursion stays out (emit declines it, like eval — the self-only def-env from PR#785 applies).

## First buildable sub-slice (next tick): Bp1
Start with the type-section generalization + def→funcidx map — pure additive foundation, gate an emit-db unit
test, no behavior change to the single-main path yet. Then Bp2..Bp5 one MR/tick.

## Notes / gotchas
- CCall carries `List(Core)` args; emit them left-to-right before `call` (wasm call convention: args on stack).
- funcidx ordering: wasm indexes funcs in func-section order. Decide fac-first or main-first + keep the export
  (`main`) funcidx correct in export-section.
- Use the SAME self-recursive-only def-env (`call-is-self-recursive`, PR#785) for emit inclusion — mutual declines.
- KEEP the eval path (run-of-db) as the differential oracle; emit must match it.
- emit-db is IN emit-db's own build closure (obviously) — gate emit-db + cdz-run; watch the reduce_nodes budget
  (v-inference's fix landed, but a big multi-fn emit closure could re-approach limits — if CDZ0201 unlocated
  errors reappear, it's the budget again, ping v-inference).
