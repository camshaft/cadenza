
## IMPLEMENTATION FINDINGS (2026-07-21, trunk 61469f6c4, run-ml unblocked via release-cdz) — reader done+reverted, guard is the crux
PROTOTYPED the reader change (strategy A) and cdz-check-verified it, then REVERTED (a half-done NApp refactor
miscompiles; kept branch clean). Confirmed findings for the next session:

1. READER CHANGE (works, ~10 lines, cdz-check-clean): read-app-or-bin's discriminator becomes NAME-BASED, not
   def-body-of. Since keywords (if/let/do/:/and/or/not) are already filtered upstream, a `sym` reaching
   read-app-or-bin is either an OPERATOR (op-code-of ≠ -1 → read-bin-form) or a NAME (op-code-of == -1 → a CALL).
   So: `(if (op-code-of(sym) == (0-1)) then <nullary-or-param-call with name-id(sym)> else read-bin-form)`. The
   call-readers (read-nullary-call/read-param-call/read-2nd-arg) take `calleeName` (was `bodyId`) and emit
   NApp(name-id, argId). read-4th-arg takes the built NApp `id`, unaffected. This DROPS the read-time
   def-body-of + the read-time param-of gate (a call to a 0-param `mk` now emits NApp too; over-app handled
   downstream). cdz check clean.

2. CONSUMER INDIRECTION (Task #2): resolve/infer/lower NApp arms use calleeId as (a) a body NODE-id
   (resolve-node/infer-node/lower-node(tree, calleeId)) AND (b) table keys (param-of etc.). KEY SIMPLIFIER
   CONFIRMED: params/args are recorded under BOTH nameId AND bodyId (read-do-def), so param-of(tree, calleeName)
   works DIRECTLY with the name — NO change to the param/arg lookups. ONLY the body-traversal calls
   (infer-node/lower-node/resolve-node on calleeId) need `def-body-of(calleeName)→bodyId` first. ~1 def-body-of
   per NApp arm (3 files).

3. THE CRUX = RECURSION CYCLE GUARD (Task #3, the hard part): infer runs BEFORE lower (resolve→infer→lower) and
   BOTH recurse into the callee body (infer-node(calleeId) at infer-db:~102, lower-node(calleeId) at
   lower-db:114/119). So a self/mutual recursive call HANGS at INFER first (infinite inline). Today recursion
   cleanly declines (def-body-of misses at read → NBin -1 → TErr); the reader change REMOVES that accidental
   guard, so WITHOUT a real guard, recursion regresses decline→HANG (worse — must not ship). The guard needs
   per-descent state (a visiting-set or depth counter) threaded through infer-node AND lower-node — but each has
   ~14 call sites + helper fns (infer-param2/3/4, inner-body-with-param3/4) that also recurse → ~40 edits total.
   OPTIONS (pick next session, verify with release-cdz run-ml, ~1min/probe, NOT iterative):
   (a) DEPTH COUNTER: add `depth: Int64` param to infer-node/lower-node; NApp arm increments on the callee
       descent; decline when depth > bound (e.g. 64). Simplest logic, but touches all ~40 call sites (mechanical:
       thread `depth` unchanged everywhere except +1 at the NApp callee-descent).
   (b) STATIC CYCLE PRE-CHECK: a self-contained helper `reaches(tree, fromBodyName, targetName, fuel)` that
       walks a def-body's NApp callees looking for targetName within fuel; NApp arm declines if the callee
       reaches back to itself. NO threading through the main walk (only a new helper called at the NApp arm).
       LEAN (b) — lower churn, contained, and it makes recursion a clean COMPILE-decline (matches today's
       behavior) rather than a depth-bounded one. Downside: duplicates a bounded traversal.

4. VERIFY (release-cdz, `./target/release/cdz run-ml`, ~32-90s each): forward `(a x)→(b x)`, `(b x)→(+x1)`,
   `(main)→(a 5)` = 6 RUNS; `fac`/`ev-od` DECLINE (run under `timeout 120` to confirm NOT a hang); a backward-ref
   (se-helper-calls-earlier-helper) still = its value; then full compiler-ml suite via `cargo xtask check`.
   ADD run-src @tests in sread-eval-fns: forward-ref=6, recursion-declines.

STATUS: reader-change validated + reverted (branch clean). Next session: re-apply reader change + consumer
indirection + guard option (b), gate with release-cdz. ~40-edit change if (a); ~15-edit if (b). Budget a full
focused tick — do NOT start at the tail of a long session (hang-prone).

## ✅ SLICE A IMPLEMENTED + SENT (2026-07-21, ref 2b7d333cc)
Done exactly per the plan (strategy A + static-cycle-guard option b). Files: sread (reader: op-code-of
discriminator, NApp carries name-id, call-readers take calleeName), parse-db (call-is-recursive bounded static
check, fuel=4000, + export), resolve-db/infer-db/lower-db (NApp arms: def-body-of(name)→bodyId for the body
descent + call-is-recursive guard → decline; param/arg lookups unchanged since keyed by both name+bodyId),
sread-eval-fns (+4 @tests). VERIFIED vs rcdzc via RELEASE run-ml: forward=6 RUNS, self/mutual DECLINE (32s, not
hang), backward/nullary/2-arg preserved. cdz check clean.
KEY LEARNINGS: (1) the guard IS needed — infer+lower BOTH inline the callee body, so recursion hangs at infer
without call-is-recursive. (2) consumer indirection was SMALL because params/args are keyed by BOTH name and
bodyId (read-do-def) — only the 3 body-descent calls needed def-body-of. (3) RELEASE cdz for run-ml (debug
driver-compile hangs >2min; release ~32s).
⏭️ SLICE B (true runtime recursion): the current guard makes recursion DECLINE (calls inline, no Core call form).
To make `fac` actually RUN, add a non-inlining Core call form (CApp + CFix or a name→body env in eval) + emit a
real wasm call/loop — a runtime+emit change, separate slice. Until then recursion is a clean decline (correct,
matches pre-existing behavior, just not-yet-supported).
