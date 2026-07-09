## 6. ⚪ Byte-identity target must account for optimization depth (cdz-rustc needs DCE)

**Finding.** The Cadenza compiler folds `(+ 20 22)` to `KConst 42` at the **Core layer before
emission**, so its component is 89 bytes with no dead code. cdz-rustc emits **128 bytes** for the same
program — it folds `run`'s body but leaves a **dead** overflow-check helper. The two agree on the
*result* and on `run`'s body, but not byte-for-byte. This is not a bug in either backend; it is the
two compilers folding at different depths.

**Why it touches the spec.** `self-hosting-and-bootstrap.md` §(the two compilers produce components
satisfying identical guarantees) and the component-check gate treat byte-identity as the convergence
signal. The finding reframes that target: **byte-identity is gated on both backends running the same
optimization depth** — specifically cdz-rustc gaining dead-code elimination (a Core→Core concern
separable from folding), or the Cadenza compiler matching cdz-rustc's shallower fold. Near-term
verification stays "validates + runs to the right answer"; byte-identity is a *named later milestone*,
not a silent expectation.

**Proposed resolution (⚪ deferred).** No spec edit needed now; recorded so the byte-identity gate is
read correctly. When DCE lands in cdz-rustc, revisit whether component-check can require byte-identity
on the folded path. Learning:
`spec/learnings/2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md`.

**Update 2026-07-07 — a SECOND byte-divergence axis, now that checked `+ - *` landed (ask-37).** The
overflow-trapping fix makes the divergence sharper and two-dimensional:
- **Folded path** (the original finding): `(+ 2 3)` → native 128B (const `5` in `run` + a DEAD standalone
  `checked_add` helper func 1), compiler.cdz 89B (const `5`, no helper). The helper is dead because both
  const-fold `run`'s body; native emits it unconditionally per-module, compiler.cdz has no standalone
  helpers at all. → needs DCE in cdz-rustc (or compiler.cdz emitting the dead helper too).
- **Runtime path** (NEW): `(+ a b)` on parameters → native emits `call <checked_add helper>` (a shared
  standalone function), compiler.cdz emits the guard sequence INLINE at each use site (`local.set` to
  scratch, sign-test, `unreachable`). Both TRAP on overflow and return the same value in-range — the
  observable behavior agrees — but the bytes differ structurally (call-to-helper vs inlined guard). So
  the `soft` bucket's "arithmetic within one integer type" is this divergence, not a defect.
So byte-identity on ANY arithmetic-bearing program is gated on compiler.cdz gaining a **cross-function
helper mechanism** (emit `checked_add/sub/mul` as standalone module funcs, call them) to match native's
structure — the same missing capability the inline-lowering was a workaround for. That is a real
self-inclusion frontier item (relates to ask-20). Until then, arithmetic stays `soft`/`trap-ok`
(value-correct, byte-different), never `agree`. Not a blocker; scoping the milestone.

**Update 2026-07-07 (spike) — compiler.cdz gained CONSTANT LET-PROPAGATION; agree 23 → 27, soft 9 → 5.**
Native folds `(let ((x V)) body)` when V is constant by SUBSTITUTING V into body (`(let ((x 10)) (+ x 5))` →
`i64.const 15`, no local); compiler.cdz kept the `let` (declared a local, `i64.const 10; local.set; local.get`
— 6 extra bytes), so the `let`-scope / underscore-identifier cases were `soft`. Added `fold-let` +
`subst-reindex` (a Core→Core constant-propagation in `fold`): a constant-bound `let` substitutes its value into
the body and DROPS the slot. This closed **4 soft → agree** (the const-`let` cases). ⚠Found + fixed a latent
INVALID-EMISSION on the way: dropping slot `ix` SHIFTS the local-index space, so remaining slots above `ix` must
decrement — an un-reindexed substitution made `(let ((p 99)) (let ((q a)) q))` declare 1 local but use index 2
(out-of-bounds, invalid component); `subst-reindex` shifts `KLocal j`/`KLet j` (j>ix) down by one. The REMAINING
5 soft cases are all THIS ask's inline-vs-helper / dead-helper divergence (mine SMALLER: e.g. 89B vs native's
128B — compiler.cdz folds+inlines, native emits the standalone checked-arith helper), which still needs the
cross-function-helper mechanism. So let-fold was NOT one of these — it was a genuine independent fold gap, now
closed. This ask's remaining scope is unchanged (cross-fn helpers / DCE).

---
