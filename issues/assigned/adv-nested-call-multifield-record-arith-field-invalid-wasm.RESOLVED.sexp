; ADVERSARIAL FINDING (producer, iter-397, 2026-07-14) — 🔴 CRASH / INVALID WASM (pre-existing, NOT a
; regression — reproduces at daf3c10f): a function that takes a MULTI-FIELD (≥2) record and returns a
; multi-field record with ≥1 field initialized by a CHECKED-ARITH expression (`(+ (. r a) 1)`, `(* … )`),
; when COMPOSED WITH ITSELF (or fed a record that is another call's result) — `f(f(x))` — emits INVALID WASM.
; `wasm-tools validate` reports `type mismatch: expected i64, found i32`; `cdz-run` rejects the component
; ("invalid component: failed to compile: wasm[0]::function[N]"). A single call `f(x)` is FINE; the defect is
; the composed/nested call where the inner call's multi-field-record result flows into the outer call.
;
; The checked-arith field initializer computes into a scratch slot with an overflow guard; when the resulting
; multi-field record is BOTH a function result (of the inner call) and a function argument (to the outer
; call), the record-field ABI at the call boundary mis-wires the arith field's slot — an i64 arith result
; lands where the record assembler / call frame expects an i32 (a field-count / discriminant / pointer slot),
; so the module fails validation.
;
; REPRODUCER (INVALID WASM — cannot run; must return 2):
;   (do (def (f (: r (Record (a Int64) (b Int64))))
;         (record (a (+ (. r a) 1)) (b (. r b))))
;       (def (main) (. (f (f (record (a 0) (b 5)))) a))
;       (export main))
;   → cdz-run: invalid component: failed to compile: wasm[0]::function[6]
;   → wasm-tools validate: "func 6 failed to validate … type mismatch: expected i64, found i32"
;   (f increments field a and copies b; f(f({a:0,b:5})) should give {a:2,b:5}, so `.a` = 2)
;
; ISOLATION (needs ALL THREE: a ≥2-field record, a CHECKED-ARITH field init, and a COMPOSED/nested call):
;   f(x) SINGLE call, arith field a                                → 1     [OK — one call is fine]
;   f(f(x)) NESTED, arith field a  = (+ (. r a) 1)                 → 🔴 invalid wasm (i64/i32 mismatch)
;   f(f(x)) NESTED, arith field a  = (* (. r a) 2)                 → 🔴 invalid wasm (any checked-arith)
;   f(f(x)) NESTED, arith on field b instead, read a               → 🔴 invalid wasm
;   f(f(x)) NESTED, field a = CONSTANT 99 (no arith)               → 99    [OK — a non-arith field init]
;   f(f(x)) NESTED, field a = (. r a) (pure projection copy)       → 3     [OK — copy, no arith]
;   f(f(x)) IDENTITY (returns r unchanged, no rebuild)             → 3     [OK]
;   ONE-FIELD record, arith field, nested                          → 2     [OK — needs ≥2 fields]
;   g(mk()) — mk nullary returns the record, ONE rebuild call g     → 3     [OK — not composed with a
;                                                                            record-taking call's result]
;   deterministic across 3 runs; reproduces at daf3c10f (pre-existing)
;   → the three necessary ingredients: (1) the record has ≥2 fields, (2) at least one field is initialized by
;     a checked-arith op (+/-/*, which allocates an overflow-guard scratch slot), (3) the record-building fn
;     is composed — its argument is itself a call returning a record (f(f(x))). Drop any one → valid.
;
; ROOT CAUSE (hypothesis, backend/wasm record-field ABI × checked-arith slot × call boundary): a multi-field
; record passed as a call argument is materialized field-by-field into the call frame / a heap record. A
; checked-arith field initializer emits its op into an i64 scratch `$r` (+ overflow guard). When the record is
; simultaneously the inner call's RESULT and the outer call's ARGUMENT, the field-materialization interleaves
; the arith `$r` slot with the record's own slots, and the arith field's i64 value is read where an i32 slot
; (a field count, a heap pointer, or the record header) is expected — the i64/i32 validation failure. A
; single call doesn't nest the two record ABIs, so the slots don't collide; a constant / pure-projection field
; needs no `$r` scratch, so it doesn't collide.
;
; FIX (hypothesis): when materializing a multi-field record whose field initializer is a checked-arith op at a
; call boundary (result feeding an argument), keep the arith `$r` scratch distinct from the record-assembler
; slots and store the i64 field value at the correct field offset — the record-field emit must not reuse the
; checked-arith scratch slot as a record slot. Likely the same emit-into-slot machinery the CSE/operand
; slot-reuse uses (cycle 158/165), missing the record-field-at-call-boundary case.
;
; SEVERITY: 🔴 CRASH / INVALID WASM — a valid, well-typed program cannot be compiled: the emitted module fails
; wasm validation and the component is rejected at load. Reachable from the everyday idiom of a
; record-transforming function (`step : State -> State` that bumps a counter field and copies the rest)
; applied more than once by composition — e.g. `(step (step s0))`, a two-step state machine, an
; accumulator-record folded twice, a `render (transform node)` pipeline. Pre-existing (reproduces at
; daf3c10f). Grades Fail (invalid wasm, no value produced).

(case "a composed call over a record-transforming function with an arithmetic field compiles"
  (doc    "`(f (f (record (a 0) (b 5))))` where `f : (Record (a Int64) (b Int64)) -> (Record …)` increments
           field `a` via `(+ (. r a) 1)` and copies `b`. f applied twice gives {a:2, b:5}, so `.a` = 2.
           Instead the compiler emits INVALID WASM: `wasm-tools validate` reports `func 6 failed to validate
           … type mismatch: expected i64, found i32`, and cdz-run rejects the component. Needs all three: a
           ≥2-field record, a checked-arith field initializer (the overflow-guard scratch slot), and a
           composed/nested call (the inner call's record result feeding the outer call's argument). A single
           `f(x)` → 1 (fine); a constant or pure-projection field init nested → fine; a one-field record
           nested → fine. The arith field's i64 `$r` scratch collides with a record-assembler i32 slot when
           the record is both a call result and a call argument. Pre-existing (reproduces at daf3c10f). Fix:
           keep the checked-arith scratch distinct from record-field slots at the call boundary. Expected:
           2.")
  (input  (do
            (def (f (: r (Record (a Int64) (b Int64))))
              (record (a (+ (. r a) 1)) (b (. r b))))
            (def (main) (. (f (f (record (a 0) (b 5)))) a))
            (export main)))
  (output (: 2 Int64)))

;; RESOLVED 2026-07-15 (trunk@2ac25eab): fix landed, gate PASSes. Agent self-removed.
