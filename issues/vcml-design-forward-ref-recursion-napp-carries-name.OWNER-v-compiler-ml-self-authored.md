# DESIGN (v-compiler-ml, self): forward-ref + recursion — make NApp carry the NAME, resolve the body LATE

Scoped 2026-07-21 (base 7b078cf9c, param4-wiring MR pending). The single biggest remaining gap in the ML
compiler's function subset. Execute-ready spec so it lands the moment trunk clears (it's a cross-cutting refactor
that MUST start from a clean base — cannot be a stacked slice).

## The gap (confirmed by probe)
```
self-recursion:   (do (def (fac n) (if (< n 2) 1 (* n (fac (- n 1))))) (def (main) (fac 5)) (export main))   ml=declined  ref=120
forward-ref:      (do (def (a x) (b x)) (def (b x) (+ x 1)) (def (main) (a 5)) (export main))                 ml=declined  ref=6
mutual-recursion: (do (def (ev n) (if (< n 1) 1 (od (- n 1)))) (def (od n) …) (def (main) (ev 4)) …)          ml=declined  ref=1
```
BACKWARD refs already work (se-helper-calls-earlier-helper, se-helper-chain-nested-calls) — a call to an
EARLIER-read def resolves fine. Only forward/self/mutual fail.

## Root cause (sread.cdz:169)
`read-app-or-bin` resolves `def-body-of(name-id(sym))` at READ TIME to get the callee's bodyId, and emits
`NApp(calleeBodyId, argId)` — calleeId IS the body node-id. A forward/self/mutual call references a def whose
body node hasn't been added to the arena yet → `def-body-of` returns None → falls through to `read-bin-form` →
op-code -1 → infer TErr → declines. (Self-recursion fails identically: `fac`'s body is mid-read when its own
recursive call is encountered, so `fac` isn't in the def-table yet.)

## Fix: NApp carries the callee NAME-ID, not the body-id; resolve the body in a LATER pass
Change the NApp node to store `name-id(sym)` (always known at read time — it's just the symbol) instead of the
callee's bodyId. Then a NEW binding pass (or resolve-db, which already runs after the whole tree is built) maps
name → bodyId via `def-body-of` AFTER all defs are recorded. Every downstream consumer that currently treats
calleeId as a body node-id must instead go name → bodyId first.

### Site inventory (measured this tick)
- **24** `Node.NApp(` construction sites (mostly reader + tests).
- calleeId consumers by file: resolve-db **8**, infer-db **24**, lower-db **17**, eval-db **10**, sread **11**
  (mostly the call-readers + @tests). emit-db 0 (it consumes lower's Core, no NApp). ≈59 consumer sites.
- param/arg lookups (`param-of`/param2-of/param3-of/param4-of, arg2-of..arg4-of) are ALREADY keyed by bodyId
  (recorded under BOTH nameId and bodyId in read-do-def) — so once a consumer has resolved name→bodyId it can
  reuse every existing keyed-by-bodyId lookup UNCHANGED. This is the key simplifier: the param/arg tables need
  no re-keying; only the NApp.calleeId indirection changes.

### Recommended shape (least churn)
1. Reader emits `NApp(nameId, argId)` (nameId = name-id(sym)); STOP calling def-body-of at read time — accept ANY
   symbol that isn't a keyword/op (forward refs included). An unknown name still declines, but LATE (resolve),
   not at read.
2. Add a resolve helper `callee-body(tree, nameId) = def-body-of(tree, nameId)` used by resolve/infer/lower/eval
   wherever they currently use calleeId directly as a body node-id. One-line indirection per consumer.
3. resolve-db already walks the whole tree post-build — its NApp arm resolves the call's body scope via
   name→bodyId, and the body itself is resolved once (memoized by the Db column, so self/mutual recursion does
   NOT infinitely recurse — the body's Core is built once and CVar/CLet reference it; eval ties the knot via the
   memoized column, exactly as rcdzc does).
4. eval/emit: a recursive call is a CVar/CApp to the memoized body — verify the Db `run-of-db` memoization
   already breaks the cycle (it should: lower fills-once per node; eval reads the filled column). If eval needs
   an explicit fixpoint for self-calls, that's the one genuinely new bit — spec it as a CFix or rely on the
   name→body late-binding in the eval environment.

### Gate
run-src @tests: fac(5)=120, forward-ref a→b=6, mutual ev/od, PLUS all existing backward-ref tests still green +
the whole conformance-db differential. This unlocks a HUGE corpus slice (nearly every non-trivial corpus program
is recursive). Verify each against rcdzc.

## ⚠ CRITICAL REFINEMENT (2026-07-21, base 0708bfd38): the lowerer INLINES — recursion needs a CYCLE GUARD, not just late name-resolution
Confirmed by reading lower-db + eval-db + the Core type: the Core IR has NO call/fix node. It is
`CNum | CVar | CBin | CLet | CIf` only, and the NApp arm LOWERS A CALL BY INLINING the callee body's Core
(`lower-node(tree, calleeId, …)` splices the body at the call site; a param call wraps it in CLet(param, arg,
bodyCore)). eval carries just `env: Map(binderId, Int64)` — zero call machinery. Consequences:

- **Forward-ref (non-recursive) works with inlining** — the callee body NODE exists in the arena after the full
  parse, so lowering can inline it regardless of definition order. Forward-ref needs ONLY: (a) the reader emit an
  NApp whose callee is resolvable LATE (carry name-id, since the bodyId doesn't exist at read time for a forward
  call), and (b) resolve/infer/lower map name→bodyId via def-body-of (whole do-block already recorded by
  lower-time). The param/arg tables are ALREADY keyed by BOTH nameId and bodyId (read-do-def records under both),
  so a name-keyed lookup needs no new table.
- **Recursion CANNOT be inlined** — a self/mutual call would expand Core INFINITELY at lower-time → the compiler
  HANGS. Today recursion cleanly DECLINES (def-body-of misses at read time → NBin -1 → TErr). So naively enabling
  forward-ref (late name-resolution + inline) would REGRESS recursion from clean-decline to compiler-hang. That
  is strictly worse and must not ship.
- **THEREFORE the forward-ref slice MUST include a lower-time CYCLE GUARD**: thread a "currently-inlining" set of
  bodyIds through lower-node; when an NApp targets a callee already on the inline stack, DECLINE (Option.None → no
  Core), preserving the clean-decline for true recursion. Only then is it sound to inline forward calls. Real
  recursion (a fac that actually recurses at RUNTIME) needs a genuine non-inlining call form (add Core.CApp +
  Core.CFix or a name→body env in eval + emit a wasm call) — a SEPARATE, larger runtime/emit slice, NOT this one.

### Revised landing plan (supersedes the 2-slice sketch above)
- **Slice A — forward-ref, non-recursive (near-term win):** reader NApp-carries-name (≈24 sites) +
  resolve/infer/lower name→bodyId indirection (≈32 calleeId derefs, each gains one def-body-of) + lower-time
  cycle guard that DECLINES a recursive/mutually-recursive call (so they stay clean-decline, not hang). Gate:
  forward a→b=6 RUNS; fac/ev-od still DECLINE (not hang); all backward-ref + existing tests green. Unlocks
  definition-order independence across the corpus.
- **Slice B — true recursion (later, bigger):** add a non-inlining call form to Core (CApp/CFix), teach eval a
  name→body binding + emit a real wasm call/loop. Flips fac/ev-od from decline to value. Runtime+emit change.

Measured site counts (base 0708bfd38): NApp construction 24; calleeId derefs — resolve-db 8, infer-db 24 (funcs
carry calleeId as a param), lower-db 17, eval-db 10 (mostly @test builders). The infer/eval counts are inflated
by helper-fn signatures + test NApp-builders; the real semantic derefs are ~1 NApp arm per file.

### Why not now
MR 7b078cf9c (param4-wiring) is queued at pr-sync; sync REFUSES to rebase (would orphan the --ref), and this
refactor touches 5 files across the whole pipeline — it must start from a synced clean trunk, as a single large
well-gated slice (or 2: reader+resolve first behind a still-declines fallback, then infer/lower/eval flips the
cases green). Pick up with `cargo xtask fleet sync --force` once the param4 MR lands.

## DIFF-READY SITE PLAN + STRATEGY (2026-07-21, trunk c71ed7c03) — for a fast verify-light implementation
Exact NApp arms to touch (measured):
  - resolve-db.cdz:59  `| Option.Some(Node.NApp(calleeId, argId)) =>`  (the resolve NApp arm)
  - infer-db.cdz:95    `| Option.Some(Node.NApp(calleeId, argId)) =>`  (the infer NApp arm — recurses infer-node(calleeId))
  - lower-db.cdz:107   `| Option.Some(Node.NApp(calleeId, argId)) =>`  (the lower NApp arm — inlines lower-node(calleeId))
  - eval-db has NO calleeId arm (consumes lower's Core) — the many eval-db NApp(...) are @test BUILDERS, not a semantic arm.
  - reader emission sites: read-nullary-call, read-param-call, read-2nd-arg, read-4th-arg (all pass bodyId to Node.NApp).

TWO CANDIDATE STRATEGIES (decide at implementation, verify against run-ml once the lease frees):
- **(A) NApp-carries-name** (the original): reader emits NApp(name-id(sym), argId); resolve/infer/lower each do
  `def-body-of(name)→bodyId` before using it. ~24 emission + 3 consumer-deref sites. The consumer param/arg
  lookups are ALREADY keyed by bodyId (recorded under both nameId+bodyId), so after name→bodyId they're unchanged.
- **(B) reader TWO-PASS def-table pre-scan** (lower blast radius, PREFERRED if feasible): keep NApp carrying
  bodyId; before read-do-def reads bodies, PRE-SCAN the do-block to assign each def a bodyId placeholder + record
  name→bodyId, so a forward call's def-body-of succeeds at read time. CRUX: a forward callee's body NODE isn't
  built until its def is read, so the pre-scan must either (b1) two-pass: first pass reads ALL def bodies
  (assigning real bodyIds) recording name→bodyId, second pass re-reads call sites now that the table is full — but
  re-reading is awkward with the arena; OR (b2) reserve a bodyId per def-name in pass 1 (add a placeholder node),
  fill it in pass 2. (A) is cleaner given the arena's append-only add-node; (B)'s "reserve then fill" fights it.
  → LEAN (A) unless the pre-scan reserve-then-fill turns out simple.

RECURSION CYCLE GUARD (both strategies need this — the HANG trap): infer-node + lower-node INLINE the callee body
(infer-db:95 infer-node(calleeId), lower-db:107 lower-node(calleeId)). A self/mutual call would infinite-loop.
Thread a "currently-inlining" Set(bodyId) through infer-node + lower-node; when an NApp targets a callee already
on the stack, return TErr / None (clean decline), NOT inline. Keeps recursion a clean-decline (not a hang) until
the separate Slice B (true recursion: add Core.CApp/CFix) lands.

GATE (needs run-ml, currently lease-blocked): forward a→b=6 RUNS; fac/ev-od still DECLINE (verify NOT hang —
run under timeout); all backward-ref + existing tests green; full compiler-ml suite. HOLD implementation until
run-ml/full-gate is feasible (host was saturated: load 157→55 easing, but run-ml still lease-timed-out at 100s).
