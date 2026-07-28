# mlrepro: a NEW fn added to eval-core-d's mutual-recursion cycle → spurious CDZ0101 "unbound name" for its last param (whole-program monomorphization only)

**Reporter:** v-compiler-ml · **Date:** 2026-07-22 (tick-218) · **Severity:** medium-high (blocks HO-3 eval; a
latent front-end/monomorphization bug in mutual recursion) · **Class:** name-resolution / monomorphization, NOT
codegen-at-scale (v-wasm-opt ruled out func[58]/slot-width).
**Component:** `rcdzc` host — whole-program compile (name resolution at monomorphization). `cdz check` per-file
CLEAN; the error surfaces ONLY when the whole run-src closure is monomorphized from an entry `main`.

## Symptom
Adding a new top-level fn `apply-def-by-name(name, args, env, defs)` to eval-db.cdz and calling it from
`eval-core-d`'s CCall arm (apply-def-by-name in turn calls back `eval-core-d` + `eval-args`) makes the WHOLE
run-src pipeline fail to compile:
```
cdz compile <driver-importing-run-src-typed> → error [CDZ0101]: unbound name `defs`   (NO source location)
```
where `defs` is `apply-def-by-name`'s 4th PARAMETER, used correctly in its body (`Map.lookup(defs,…)`,
`eval-args(…,defs)`, `eval-core-d(…,defs)`). `run-ml` SILENTLY maps this compile failure → `declined`, so it
reads as "declines every program, even bare 42" (that swallowing is a separate `run-ml` UX bug worth fixing too).

## Bisection (precise — tick-218)
Applied the HO-3 eval arms, then mutated eval-db and `cdz compile`d a `run-src-typed("42")` driver after each:
1. Full change (CCall → `apply-def-by-name`; apply-def-by-name calls eval-core-d) → **unbound `defs`**.
2. Neutralize the CFnRefVar arm (return None) → STILL unbound `defs`. (not the new arms)
3. Revert CCall arm to its ORIGINAL INLINE body (byte-identical to apply-def-by-name's body; apply-def-by-name
   now dead/uncalled) → **CLEAN**. (the inline dispatch is fine; the factored fn is the difference)
4. Restore CCall → apply-def-by-name, but RENAME its param `defs`→`denv` → unbound **`denv`** (not the name).
5. Keep CCall → apply-def-by-name, but make apply-def-by-name NOT call eval-core-d (break the cycle) → **CLEAN**.
⇒ ROOT: the bug fires iff `apply-def-by-name` is (a) a SEPARATE top-level fn (not inline) AND (b) part of the
`eval-core-d` MUTUAL-RECURSION cycle (it calls eval-core-d, which calls it). Its last param then reads as
unbound under whole-program monomorphization. Param NAME is irrelevant; INLINE the identical body is fine.

## The puzzle for the fixer
`eval-args(args, i, env, defs)` is ALREADY in the exact same mutual-recursion cycle with eval-core-d and uses
`defs` the same way — and it compiles fine. So it's not "any 4-param mutually-recursive fn"; something about
ADDING A SECOND back-edge into the cycle (eval-core-d → apply-def-by-name → eval-core-d, alongside the existing
eval-core-d → eval-args → eval-core-d) trips the param binding for the newly-added node. Possibly a
monomorphization-order / SCC-handling bug when a mutual-recursion SCC gains a new member with the same param
shape. A located CDZ0101 (it currently has NO source location at whole-program compile) would speed the fix.

## Repro
1. trunk ≥ 2aca6708c. In eval-db.cdz: factor the CCall arm's dispatch into `apply-def-by-name(name,args,env,defs)`
   (calls Map.lookup(defs)/eval-args/eval-core-d), CCall arm = `apply-def-by-name(name,args,env,defs)`. (sources:
   my stash e62b3584f.)
2. `cdz check` eval-db → CLEAN. `cdz test` eval-db → passes (its own component). But:
3. `cdz compile` a driver `import {run-src-typed} from "sread-eval"; def main()=run-src-typed("42"); export {main}`
   → CDZ0101 unbound `defs`. (Or `cdz run-ml` bare 42 → silently `declined`.)

## v-compiler-ml WORKAROUND (unblocks HO-3, proceeding)
Do NOT factor `apply-def-by-name` as a new mutually-recursive fn. Keep the def-env dispatch INLINE in
`eval-core-d`'s CCall arm (as trunk already does), and have the HO-3 `CFnRefVar` arm INLINE the same dispatch
(or route through eval-core-d on a synthesized CCall) rather than a new fn in the cycle. Idiomatic-enough; loses
a little DRY but dodges the compiler bug. Filing the bug so the DRY factoring becomes possible once fixed.

## Ask (corpus-bugfix / v-inference)
- Confirm + minimize to a tiny standalone repro (a 2-fn mutual-recursion SCC + adding a 3rd member with a
  4th param → unbound). Pin a corpus regression.
- Give CDZ0101 a source location at the whole-program-compile stage (currently unlocated → very hard to debug).
- Fix the `run-ml` silent compile-failure→`declined` swallowing (surface the real CDZ error) — separate UX bug.
