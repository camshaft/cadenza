# FINDING: emit-once on two mutually-inlined Db-threading defs strands a param ("no local slot")

Owner-candidates: v-compiler-ml (found it) · v-compiler-perf (owns emit-once eligibility + memo drift-guard).
Status: OPEN. Blocks emit-once slice-3 (threshold 40→20). Trunk at threshold 40 is SOUND (this never fires there).

## Symptom
Lowering `INLINE_COST_THRESHOLD` 40→20 (emit-once slice-3, commit 27359c2e0 on branch
`vcml-slice3-held-27359c2e0`) makes the compiler-ml SELF-HOST suite fail:
```
cdz: error: parameter reference has no local slot
```
during `cdz test implementation/compiler-ml/src/conformance-db.cdz`. rcdzc lib suite (2400/0) and
`xtask gate` do NOT compile the compiler-ml sources, so it looked green locally — self-host-only.

## Bisected root cause (precise)
The stranded binder is `db`, OWNED BY `infer-into-db` (db-infer.cdz:36), appearing free inside the
STANDALONE-emitted body of recursive `count-passing` (conformance-db.cdz:47, params cases/i/acc).

The trigger is TWO defs both flipping to emit-once TOGETHER:
- `lower-of-db(db: Db, root)` (db-lower.cdz:23) = `let db1 = infer-into-db(db, root) in (db1, lower-node(…))`
- `infer-into-db(db: Db, root)` (db-infer.cdz:36) = `let db1 = resolve-into-db(db, root) in … merge-typed(db1, tcol)`

Both share the pattern `let db1 = g(db, root) in <use db1>`, and `lower-of-db` calls `infer-into-db`.
`count-passing` inlines `check-case → case-passes → run-tokens-db → run-of-db → lower-of-db`.

Bisect (VCML_NOEMIT temp knob forcing named defs inline):
- exclude `infer-into-db` ALONE → still strands.
- exclude `lower-of-db` ALONE → still strands.
- exclude BOTH `infer-into-db,lower-of-db` → 22/0 PASS. (also `run-of-db,infer-into-db,lower-of-db` → pass.)
- exclude `infer-into-db,resolve-into-db` (the other pair) → still strands. So it is specifically the
  `lower-of-db ↔ infer-into-db` mutual-emit-once pair.

## Two NEGATIVE results (rule-outs)
- NOT v-cperf's PR#959 memo bug: bypassing `emit_shared` (recompute `emit_once_callee_eligible_uncached`
  per-site) does NOT fix the strand. Genuine per-callee eligibility gap, not a stale/provisional cache.
- NOT the const-forward shape slice-3 already gates (`body_forwards_capture_to_const_param`): these are
  plain Db-threading defs with NO const params. The gate correctly doesn't fire; the bug is elsewhere.

## Hypothesis (UNVERIFIED — needs emit-once/inline owner)
When both `lower-of-db` and `infer-into-db` are emit-once (Core::Call, not inlined), the `let db1 = …`
binding + the mutual call interact so that, as `count-passing` inlines `run-of-db → lower-of-db`, a
`Core::Param{binder=infer-into-db.db}` reference survives into count-passing's standalone body without a
slot. Likely: an emit-once callee whose body BINDS-and-RETURNS a Db-through-`let` and is itself inlined
into a recursive standalone parent leaves a param reference from the mutually-called emit-once sibling.
The `should_emit_once_by_cost` per-args soundness gate (`arg_captures_runtime_binding`) does NOT prevent
this — a `db`-capturing arg ENABLES emit-once (the result can't be const-demanded), the opposite of a
stranding guard.

## Repro
1. `git checkout vcml-slice3-held-27359c2e0` (or cherry-pick 27359c2e0; threshold=20) — REBASE onto
   current trunk first (PR#959 `7d705cefd` landed; won't fix this per the negative result, but keep current).
2. `cargo build --release -p cdz`
3. `./target/release/cdz test implementation/compiler-ml/src/conformance-db.cdz` → "no local slot".
   (Debug: a `VCML_PROBE` eprintln in select.rs `Core::Param` None-arm + a thread-local naming the
    current select_function pinpointed binder=`db` owner=`infer-into-db` in fn params cases/i/acc.)

## SURVEY (2026-08-01, v-compiler-ml): conformance-db is the ONLY strand
Built threshold-20 on trunk 7d705cefd (has PR#959) and ran ALL 25 compiler-ml self-host suites.
ONLY `conformance-db` strands. All others PASS at 20: conformance-db-cx 28/0, conformance-db-rel 25/0,
db-demand 16/0, eval-db 66/0, sread-eval 14/0, sread-eval-fns 37/0, sread-eval-sum 17/0,
sread-eval-sum-payload 26/0, sread-eval-params, sread-eval-match 9/0, sread-eval-ho, sread-eval-nonrec,
sread-eval-ann 26/0, infer-db 67/0, lower-db 19/0, resolve-db 12/0, emit-db 57/0, emit-rec-db 4/0,
tycheck 16/0, db-eval/infer/lower/resolve. ⇒ the narrow mutual-param-forwarding gate is COMPLETE for the
current corpus — it ships the -40% everywhere and excludes exactly one shape, NO hidden future rejects.

## PLAN (co-designed w/ v-compiler-perf, 2026-08-01)
v-cperf WRITES a narrow per-callee gate in emit_once_callee_eligible_uncached; v-compiler-ml self-host
co-verifies (all 25 suites + rcdzc lib 2400/0) before MR. General closedness-re-lower pass (attribution +
force-inline override + re-lower) tracked as a SEPARATE proper increment. Proposed robust predicate:
exclude an emit-once-eligible def whose body passes one of its OWN params as an arg to a call of ANOTHER
emit-once-eligible def (mutual param-forwarding among emit-once defs) — per-callee pre-gateable, sound,
degrades safely.

## Disposition
Slice-3's -40% emit-size win (co-verified, correctness+runtime green on the sread-eval oracle) is worth
landing ONCE this eligibility gap is closed. Held, NOT worked-around (no threshold-dodge — that violates
the idiomatic-code directive; the bug would just resurface at another closure). Needs the emit-once owner
to design the correct eligibility exclusion (or the inline-structure fix). Trunk stays at 40 meanwhile.
