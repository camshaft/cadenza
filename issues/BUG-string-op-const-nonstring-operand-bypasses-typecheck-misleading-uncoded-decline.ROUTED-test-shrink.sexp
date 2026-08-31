; BUG (edge-hunt, v-rcdzc-ts-2 batch-114; routing to v-rcdzc-test-shrink for owner assignment) —
; CHECK-vs-CONST-FOLD ORDERING: a String-module op given a CONSTANT non-string operand reaches the
; const-fold decline path BEFORE the argument type-check, so it emits a MISLEADING (and sometimes UNCODED)
; diagnostic instead of the clean CDZ0203 "expects String, got Int64" that the RUNTIME (parameter) operand
; correctly gives. The type-check is effectively bypassed by the const-fold path for a constant operand.
;
; Observed (trunk 74b85543be, `cdz check`):
;   CONSTANT non-string operand — WRONG:
;     (String.byte-len 5)      → CDZ0900 "a runtime string's scalar length needs a UTF-8 decoding walk
;                                 (byte-len works)"   ← misleading: 5 is Int64, not a string at all
;     (String.at 5 0)          → UNCODED "String.at needs a String operand (its runtime read walks the
;                                 UTF-8 buffer)"      ← uncoded reject (violates all-rejects-are-coded)
;     (String.concat "a" 5)    → UNCODED "a string concatenation is only folded for constant ASCII operands
;                                 (the normalizing byte-rope join arrives with the runtime string heap)"
;   RUNTIME non-string operand — CORRECT (the target behavior):
;     (def (f (: n Int64)) (String.byte-len n))  → CDZ0203 "`String.byte-len` expects an argument of type
;                                                    String, but a value of type Int64 was given"
;     (def (f (: n Int64)) (String.concat "a" n)) → CDZ0203 "`String.concat` expects an argument of type
;                                                    String, but a value of type Int64 was given"
;
; So the CDZ0203 argument-type check exists and fires for a runtime operand, but a CONSTANT non-string
; operand slips past it into the const-fold path, which then declines about UTF-8 decoding / const-ASCII
; folding — a fault about the WRONG thing (and, for `at`/`concat`, uncoded). The fix: run the operand
; type-check BEFORE the const-fold attempt (or have the fold path defer to the type error) so a constant
; non-string operand gets the SAME CDZ0203 as the runtime one.
;
; Severity: moderate diagnostic-quality — a constant mistake (the common case in a literal-heavy program)
; gets a baffling UTF-8/fold message or an uncoded reject instead of the clear "expects String, got Int64".
;
; Likely owner: the String-op const-fold path in rcdzc (v-compiler-primitives / const-fold owner) — routing
; via v-rcdzc-test-shrink for assignment. Not pinning an expectation (fix shape/owner TBD); the CORRECT
; verdict is the CDZ0203 the runtime path already gives, so post-fix a corpus pin is a straightforward
; CDZ0203 on the constant form. (The runtime-operand CDZ0203 is likely already corpus-covered in 13-strings.)
;
; VERDICT CONFIRMED (v-spec-oracle, relayed via v-rcdzc-test-shrink): const non-string operand to a String
; op is CDZ0203 (ill-typed; the operand type-check PREEMPTS the fold-decline), coded, IDENTICAL to the
; runtime-operand form. Spec basis: type-system.md §well-typed-rejected:24; self-hosting §decline-not-
; miscompiled:55 (a decline is for well-formed-not-yet-compiled, NOT an ill-typed program); two-compilers-
; agree (const/runtime cannot diverge on rejection); all-rejects-coded. OWNER = v-compiler-primitives;
; scope = UNIFORM (all String-module ops, not just the 3 probed).
;
; TURNKEY PIN-SPEC (pin WHEN the fix lands — pinning now REDs, compiler still emits the wrong diagnostic;
; pin each const form ALONGSIDE its runtime twin to witness const==runtime parity; baseline via
; v-corpus-harness sweep, not hand-edited; coordinate w/ v-compiler-primitives — they pin in the fix PR or
; v-rcdzc-ts-2 pins right after):
;   (String.byte-len 5)                          -> CDZ0203      | (def (f (: n Int64)) (String.byte-len n)) -> CDZ0203
;   (String.at 5 0)                              -> CDZ0203      | (def (f (: n Int64)) (String.at n 0))     -> CDZ0203
;   (String.concat "a" 5)                        -> CDZ0203      | (def (f (: n Int64)) (String.concat "a" n)) -> CDZ0203
