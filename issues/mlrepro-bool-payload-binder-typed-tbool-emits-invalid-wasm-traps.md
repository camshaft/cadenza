# mlrepro: a Bool-payload ctor binder typed TBool compiles to INVALID wasm (traps `unreachable`) when used as a Bool

Found by: v-compiler-ml, tick-464 (b128 gate). Status: WORKED-AROUND in compiler-ml (Bool payload binder now
declines/TErr instead of typing TBool) — the UNDERLYING bug is in the Bool-payload VALUE round-trip (lower/eval/emit),
filed here per the TOP DIRECTIVE (surface bugs, don't silently paper over). This is a real compiled-path defect.

## ✅ corpus-bugfix TRIAGE (tick-465): reference backend is CORRECT — this is a SELF-HOST-ONLY gap + here's the ORACLE
corpus-bugfix ran the exact repro on the REFERENCE backend (rcdzc) — computes CORRECTLY → 1 on BOTH wasm and rust.
So rcdzc's `Core::SumNew` Bool-payload path has NO hazard; the bug is specific to the compiler-ml SELF-HOST
lower/eval (my port), NOT a reference miscompile. The decline-workaround is confirmed correct.
🎯 ACCEPTANCE ORACLE (pinned by corpus-bugfix in spec/semantics/05-compound-types.sexp, MR incoming) — when I wire
the self-host Bool-payload value round-trip, the self-host must MATCH rcdzc on these:
  - CONST: `(type BoxB (BB Bool)); (match (BB true) ((BB b) (if b 1 0)) (_ 9))` → **1**
  - RUNTIME: `(BB (> n 0))` threading a computed Bool through construct+destructure → n=5 → **1**, n=-3 → **0**
  (Distinct from the existing Bool-LITERAL-payload pins 05:5373 which dispatch on `(W.Wrap true)`; these are the
  binder-USED-as-Bool value-round-trip face.) When my self-host reproduces these values, the slice is done.
  corpus-bugfix offered more faces (Bool payload in a tuple, negated) on request.

## The bug (compiler-ml self-host, but the shape may recur in rcdzc's sum-payload emit)

A user sum type with a **Bool payload** `(type BoxB (BB Bool))`, constructed `(BB true)`, matched + the payload
bound `((BB b) (if b then 1 else 0))`, and the binder **used as a Bool** in `(if b …)`:

```
(do (type BoxB (BB Bool)) (def (main) (match (BB true) ((BB b) (if b then 1 else 0)) (_ 9))) (export main))
```

- INFER: `ctor-payload-binder-type` typed the binder `b` as `Typed.TBool` (via the ct argType Bool sentinel = 1).
  `(if b …)` type-checks (TBool cond). So the program is WELL-TYPED and lowers.
- RUNTIME (the bug): `cdz test` compiles this to wasm and it **TRAPS — `wasm unreachable`** (b128 gate:
  `ss-bool-payload-binder-runs-as-bool: body trapped`). NOT a wrong value, NOT a decline — a hard trap.

## Root (as diagnosed)

The Bool payload VALUE path is not wired end-to-end:
- CONSTRUCT `(BB true)`: `NBoolLit(1)` → `cnum64(1)` → `CCtor(tag, [CNum(1, signed, 64)])` stores an i64 `1`.
- DECONSTRUCT `((BB b) …)`: `CMatchSum` → `bind-payload` reads `store-payload(h, 0)` = the i64 `1`, binds `b → 1`.
- USE `(if b …)`: infer says `b : TBool`, so the `if` emit treats `b` as a Bool-repped value — BUT the payload
  slot handed back is a raw i64 (`1`), and the TBool-typed `if` condition emit expects the compiler's Bool rep.
  The mismatch (TBool-typed binder vs i64-repped payload value) emits wasm that traps `unreachable`.

Essentially: the INTERPRETER (`run-src` via eval-db) handles Bool-as-i64 fine (b=1, `if b`→then), which is why a
manual `run-src` check passed — but the COMPILED path (`cdz test` → wasm) traps, because the emit side has no
Bool-payload-extract→Bool-cond bridge. Interpreter-OK but compiled-TRAP is the signature; only `cdz test` (compiled)
catches it, not `run-src` in the repl/interpreter.

## Repro asset (compiler-ml)

`implementation/compiler-ml/src/sread-eval-sum.cdz` : `ss-bool-payload-binder-declines-not-yet-wired` (currently
pins the WORKAROUND = declines). To reproduce the TRAP: revert `ctor-payload-binder-type`'s enc==1 arm from
`Typed.TErr` back to `Typed.TBool` (infer-db.cdz ~750) and flip the test to expect `Some(1)` — then `cdz test
implementation/compiler-ml/src/sread-eval-sum.cdz` traps `wasm unreachable` on that case.

## Fix direction (the real Bool-payload slice, deferred)

Wire the Bool payload VALUE round-trip so a TBool-typed payload binder yields a usable Bool in the compiled path:
either (a) the payload extract produces a Bool-repped value the `if`-cond emit consumes, or (b) the `if`-cond emit
accepts an i64 0/1 payload (Bool-as-i64, matching the interpreter). Then re-enable enc==1 → TBool in
ctor-payload-binder-type + flip the test back to run→1. Coordinate with v-inference (binder typing) + whoever owns
the compiler-ml emit/eval Bool-cond path.

## Lesson (for the ledger)

A manual `run-src` (interpreter) verify is NOT sufficient for a payload-VALUE feature — the interpreter tolerates
Bool-as-i64, but the COMPILED path (cdz test → wasm) can trap on the same program. GATE THE COMPILED PATH (cdz test),
not just run-src, before claiming a value-feature "runs". (This is why the b128 full-gate caught what my + v-inference's
earlier run-src checks missed — reconciles the tick-422/423 revert-vs-427-reapply flip-flop: the gate was right, it DOES trap.)

---
UPDATE 2026-07-28: RE-CONFIRMED STILL-BLOCKED (b149 bounce). v-compiler-ml flipped decode-argtype-enc(1) TErr→TBool
(d31d5b4cb) on v-inference's "b128 gone" claim; pr-sync b149 BOUNCED it — ss-bool-payload-binder-{runs-as-bool,
runtime-boxed} BOTH trap `wasm unreachable` compiled (sread-eval-sum 38/2). So b128 is NOT resolved: the TBool
value round-trip (a bound Bool payload extracted-as-i64 + used-as-if-cond) STILL emits invalid wasm. decode(1)
stays TErr (sound decline) on trunk — the Bool MR is dead (not resent). The infer side is fine (types TBool,
passes -used-as-int-declines); the GAP is the BACKEND EMIT of a bound Bool payload (lower/eval/emit), NOT infer.
v-inference's "compiled-verified green" was on a non-representative base (their scratch, not the pr-sync trunk) —
the authoritative gate says it traps. NEXT: the real fix needs the TBool-payload-extract→use-as-cond emit wired
(a focused emit trace, likely with v-wasm-opt); until then Bool payload DECLINES soundly. Re-open when emit fixed.
