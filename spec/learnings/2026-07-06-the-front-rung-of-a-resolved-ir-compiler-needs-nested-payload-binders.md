# The front rung of a resolved-IR compiler needs nested payload binders — and folding early leaves cdz-rustc's dead code behind

*2026-07-06*

**What happened.** After the exponential-inlining fix and the heap-sub-node-through-a-helper fix,
the Cadenza-authored compiler (`compiler.cdz`) reached a nearly-complete vertical slice: the whole
pipeline is authored as four composable IR transformations —
`resolve : Node → Core` (name/opcode resolution into the resolved middle IR), `fold : Core → Core`
(constant folding, the first Core→Core rewrite), `lower : Core → Lir` (into the typed instruction
sum), `serialize : Lir → Bytes`, and `frame`/`wrap-component` — matching the resolved-IR shape
[`compiler-pipeline.md` §Representation] pins ([[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]]).
Every downstream rung compiles to a valid component when fed `Core` directly. Compiling the *whole*
file surfaced one clean decline and one byte-level finding:

1. **The front rung `resolve` declines — nested tuple binder in a sum payload.** The resolved-IR
   front rung's natural node pairs a head opcode with its two operands:
   `(NPrim (Tuple Int64 (Tuple Node Node)))`, matched
   `((Node.NPrim (tuple op (tuple a b))) …)`. That is a tuple *nested inside* a sum payload's
   binder, and the seed declines it: *"runtime sum match: nested tuple binder not supported."* The
   boundary is precise and was probed directly: a **flat** payload binder `(tuple a b)` works, a
   **flat 3-tuple** `(tuple op a b)` works (→ 43), and the obvious hand-desugaring — a bare
   `(match rest ((tuple a b) …))` on the inner tuple — *also* declines (a runtime-tuple match with no
   constructor arm is itself unsupported), so there is no in-language workaround. Only the payload
   binder's recursion into a compound slot is missing: the seed's `bind_sum_payload` reads each slot
   of a flat payload tuple via the array accessor but hits the decline when a slot binder is itself a
   `(tuple …)`. The fix is for that routine to *recurse* — read the nested slot's heap handle and
   destructure it by the same slot logic. This is the sole remaining blocker on the compiler's front
   rung; `resolve`/`lower` are exactly "a tagged node carrying a tuple of sub-nodes," so it is the
   idiom's staple, not a corner.

2. **Folding at the Core layer leaves cdz-rustc's dead code behind — a real byte divergence, and a
   point *for* the resolved-IR architecture.** For the target `(module m (def (main) (+ 20 22)))`,
   `compiler.cdz` folds the whole `Core` tree to a single `KConst 42` *before* `lower` emits
   anything, so the code section is `42 2A 0B` and the component is 89 bytes — a valid component that
   runs to 42 (verified independently by wasm-tools + wasmtime). The Rust seed `cdz-rustc` emits
   **128 bytes** for the same program: it *also* folds `run`'s body to `i64.const 42`, but it
   additionally emits a **dead** overflow-check helper (the `i64.add` + sign-XOR trap it would have
   called before folding) that nothing invokes. The two compilers agree on the result and on `run`'s
   body but not byte-for-byte. Byte-identity here would require cdz-rustc to drop the unreachable
   helper — **dead-code elimination, a Core→Core concern separable from folding**. So the divergence
   is not a bug in either backend; it is the two compilers folding at different depths.

**Why.** Both findings are consequences of authoring the compiler as *the language's most demanding
program* ([[2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses]]).
The nested-binder gap sat undisturbed because no floor-outward conformance case ever *bound* a
compound slot of a sum payload — several cases *construct* `(tuple n (tuple n n))` and one binds a
*flat* payload `(tuple a b)`, but binding a nested tuple through a match arm is exactly the shape a
resolved-IR front rung composes and nothing before it required. The byte divergence is the sharper
lesson: it is empirical evidence that *where* an optimization runs is observable in the output. A
compiler that folds on a resolved, analyzed IR before emission produces no dead helper to eliminate;
a compiler that emits first and folds shallowly leaves one. This is the resolved-IR architecture
paying off concretely — emission is a serializer of an already-optimized form, so the optimization is
visible in the bytes rather than layered on afterward — and it reframes the eventual byte-identity
target: the two compilers converge only once cdz-rustc gains DCE (or folds as early), which is a
named, separable milestone, not a mystery diff. Near-term verification stays "validates + runs to the
right answer," with byte-identity deferred until both take the same optimization path.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` —
*"a match arm binds a nested tuple inside a sum payload"* — pins the nested payload binder as a
permanent gate obligation: an `Expr` whose `Bin` variant carries
`(Tuple Int64 (Tuple Expr Expr))`, matched `((Expr.Bin (tuple op (tuple a b))) …)` and folded
recursively to a scalar (→ 34, the `resolve` shape exactly). It records the true `(output …)` oracle
and is tagged `sum-type-declaration`, so it scores as *todo* (the seed declines it cleanly today,
per reject-don't-miscompile) and turns green the moment `bind_sum_payload` learns to recurse. The
byte-divergence finding drives no new requirement — it is a design observation recorded here (and in
the compiler spike's handoff) that the two-compilers byte-identity check must be understood against
the optimization depth each backend runs at, with cdz-rustc DCE as the named convergence step. Both
are captured with reproducers in `implementation/compiler/SEED-GAPS-FOR-SELF-HOSTING.md` (Tier 2b)
so the seed fix is scoped work, not lore.
