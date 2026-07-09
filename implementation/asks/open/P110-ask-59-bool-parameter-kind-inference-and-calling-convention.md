## 59. 🟡 (compiler.cdz) Bool-typed PARAMETERS decline — the i64-parameter model can't compile a function whose param is used as a Bool (if-cond / bool-match / not), nor a call passing a Bool arg

**Finding.** `compiler.cdz` fixes EVERY function parameter to i64 (`params-bytes` emits `0x7E` per param;
`kind-of (KLocal n) = Ki64`; `resolve`/`ck-of` default a param to i64/CKUnk). So a function whose parameter is
used AS A BOOL — an `if`-condition, a `not` operand, a bool-`match` scrutinee — and any CALL passing a Bool
argument both DECLINE (to honour decline-don't-miscompile; see `lower`'s KCall arm ~L2066 `args-have-bool` →
`unreachable`, and the KLet Bool-value decline ~L2053). Native infers each param's type from its uses and emits
the right valtype (a Bool param is i32), so it COMPILES these; mine declines them.

**Verified declines (native compiles all; mine declines — value-oracle component-check, 2026-07-07):**
- `(module m (def (f b) (match b (true 1) (false 0))) (def (main) (f true)))` → native `1`, mine DECLINE
- `(module m (def (f b) (if b 1 0)) (def (main) (f true)))` → native `1`, mine DECLINE
- `(module m (def (g b) b) (def (main) (g true)))` → native `true`, mine DECLINE (a Bool ARG to a call)
- corpus: "a function takes a boolean parameter and branches on it", "a boolean-parameter function applied to
  false", "the identity function applied to a boolean returns the boolean", "a boolean literal pattern matches a
  runtime scrutinee" — all DECLINE for this reason. (The bool-`match` DESUGAR itself works — a const-scrutinee
  bool match agrees; it's the Bool PARAM that declines.)

**Root — the i64-parameter model (documented in-code).** `lower` KCall arm (~L2057): *"PARAMS ARE i64 in this
compiler's model … passing a BOOL argument is UNSOUND here: widening it to i64 either loses the Bool on a
pass-through (`(id true)` → the integer 1, a WRONG-VALUE miscompile) or mismatches a Bool-position use → an
invalid component. To honour decline-don't-miscompile, DECLINE any call with a Bool argument."* The decline is
CORRECT (ask-34 eliminated the miscompile this way); this ask is the COVERAGE follow-on: actually COMPILE them.

**The fix — per-parameter kind inference + a Bool-aware calling convention (the compiler.cdz half).** A parameter's
valtype should be inferred from its USES in the body, not fixed to i64:
- Scan a function body for each param: used as an `if`-condition / `not`-operand / bool-`match` scrutinee ⇒ Bool
  (i32); used in arithmetic / comparison-as-i64 / i64 call arg ⇒ i64; unconstrained pass-through ⇒ a kind
  variable resolved per call-site (the ask-35 return-kind half). Conflicting uses (both Bool and i64) ⇒ a
  genuine type error native rejects, or decline.
- Emit the inferred valtype per param (extend `params-bytes` from "np × 0x7E" to a per-param valtype list).
- Calling convention: a Bool argument (i32) to a Bool param (i32) passes DIRECTLY (no `i64.extend_i32_u`
  widening); an i64 arg to an i64 param as today. `args-have-bool` stops being an unconditional decline — it
  becomes "decline only if the arg kind ≠ the callee's inferred param kind" (a real type mismatch), else emit
  the direct/widened call as appropriate.
- This is the same monomorphization-over-the-i64/i32-boundary the ask-35 return-kind half needs; the two are one
  feature (per-parameter/return kind inference). The seed does this generically (host-value-agnostic monomorph);
  compiler.cdz needs its scalar (i64/Bool) slice.

**Why it's NOT a clean gap-independent slice to rush.** It touches (a) the function-signature emission (per-param
valtypes), (b) the calling convention (Bool-arg widening rule), and (c) `kind-of`/`ck-of`/`build-ktab` (a Bool
param becomes provably-Bool, which also affects the type-check pass — a Bool param used in arithmetic should then
REJECT, matching native). A half-version risks re-introducing the exact Bool-arg wrong-value miscompile ask-34
carefully eliminated. So it wants a deliberate design pass, not a loop-cycle patch — flagged here for the compiler
agent / a focused cycle.

**Priority.** 🟡 MEDIUM. This is the LARGEST remaining compiler.cdz-ownable scalar coverage cluster (Bool-param
functions — a recurring corpus shape), and it's NOT seed-gated (pure codegen). But it's a real inference +
calling-convention subsystem, below the byte-gate-soundness work (already 0 disagree) and the operator-directed
ask-58 (builtin-modules-as-records) in leverage. It subsumes ask-35 (return-kind is the pass-through case of the
same per-param kind inference).

**Acceptance signal.** `(module m (def (f b) (if b 1 0)) (def (main) (f true)))` compiles → `1` (decline→agree);
a Bool param used in arithmetic (`(def (f b) (+ b 1))` applied to a Bool) REJECTS CDZ0201 (matching native's
type error); an i64 param in arithmetic unchanged; no Bool-arg miscompile reappears (ask-34 stays fixed). Related:
ask-34 (the decline that made this safe — DONE), ask-35 (the return-kind half — this subsumes it), ask-30 (the
type-check pass a Bool param interacts with).
