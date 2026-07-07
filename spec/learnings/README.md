# Learnings

Dated post-mortems that drove this specification. Each entry follows the format in
[`templates/learning.md`](../../templates/learning.md): **What happened / Why / The requirement it
drove**. Learnings are descriptive, not normative — they carry no RFC-2119 requirements and are not
listed in the requirement gate. They are the one place a specification artifact may name a prior
prototype or a concrete implementation, because a learning is historical reference for why a durable
change exists.

The learnings here are the reasons this clean-room specification is shaped as it is. Earlier
generations of Cadenza taught these lessons the expensive way; the specification is the response.

- [A diagnostics capability spec raised the bar to error recovery and a machine-branchable kind — and it names the distinction the loop has been improvising](./2026-07-07-a-diagnostics-capability-spec-raised-the-bar-to-error-recovery-and-a-branchable-kind.md)
  — a new tracked `spec/capabilities/diagnostics.md` formalized the compiler's diagnostics contract. Probing each
  requirement against the stable seed split them: MET (stable codes — pinned by the corpus's `rejected CDZ####`
  cases; severity; machine-readable) vs SPEC-AHEAD — maximal-independent-set-in-one-pass (seed reports only the
  FIRST error; `(do (+ 1 true) (< 2 false))` → one diagnostic, no error recovery), primary/derived, a
  machine-branchable rejection/decline/trap KIND, and structural fixes. The striking one: the machine-branchable
  KIND is exactly the distinction this loop has spent a dozen cycles reconstructing from emitted bytes (the byte
  gate's decline/trap discriminators, ask-26/29/33) — the spec now makes the compiler exposing it a requirement,
  which would retire the loop's discriminator apparatus. Lesson: at a capability-spec landing, probe the seed
  against each new requirement and record the spec-ahead gaps; and when the spec formalizes a distinction the loop
  has been improvising from indirect evidence, that improvisation was a workaround for a missing first-class
  output — the fix is the artifact exposing it, after which the loop reads instead of reconstructs. No corpus
  (diagnostics-shape/behavior, not `(output …)` values; the single-rejection code the corpus pins is met). Filed
  as ask-48.
- [The diagnostics pivot from Result-return to effects hit a parallel compile-entry lowering gap](./2026-07-07-the-diagnostics-pivot-from-result-to-effects-hit-a-parallel-compile-entry-lowering-gap.md)
  — the diagnostics channel has two shapes and the compiler tried both, hitting a compile-entry lowering gap on
  each: Result-return declines on a deep `Core` sum-match under Result-shaping (ask-42), and the effects route
  (operator's "diagnostics via effects": `Diag.emit` collected by a handler — collection works, ask-45) declines
  when the `handle` is installed at the `compile` entry (NEW ask-46). Verified ask-46 with its discriminator:
  same recursive-effectful `handle` compiles under `main`/`run` but declines under `compile`, and it's
  presence-triggered (a handle anywhere in a compile-entry module, even unreachable). Careful non-overclaim: I
  probed for a single unifying root and didn't cleanly find one — ask-46 is entry-kind-gated, ask-42 is a
  scrutinee-kind divergence; distinct mechanisms, shared shape. Shared shape = **the `compile` entry is a
  less-complete lowering path than `run`** (it's the newer ABI), and the features that lag are exactly the ones
  the compiler needs to run AS that entry (a diagnostics handler, an internal-state effect). Lesson: a new entry
  ABI forks a self-hosting compiler's lowering coverage — probe the run-vs-compile-entry discriminator to tell
  "unimplemented" from "unimplemented AT THIS ENTRY" (the latter a smaller, ABI-path-local fix). No corpus
  (compile-entry ABI gap; effect value-behavior pinned separately under the run entry).
- [A pinned toolchain snapshot gives the loop a reproducible probe target — and settles the churn readings](./2026-07-07-a-pinned-toolchain-snapshot-gives-the-loop-a-reproducible-probe-target.md)
  — an `implementation/stable/` snapshot appeared: a frozen all-gates-green `cadenza-seed` + runtime +
  cdz-rustc reference + `SHA256SUMS`, so self-hosting work runs against a FIXED seed, not the mid-cycle-rebuilding
  `implementation/seed/`. Verified: hashes OK, gate green (569), byte gate 65/124/385, WRONG=0 — and those match
  the last several cycles, confirming the earlier fluctuation (183→137→124 disagree) was transient churn, not
  movement. Adopting it: several recent cycles were muddied by the seed rebuilding WHILE I probed (a false
  gate-timeout regression, a byte-count swing from a half-migrated ABI, mtimes moving twice per cycle) — a probe
  is only as trustworthy as the target's stability, and measuring a moving target reads something that no longer
  exists. Standing-procedure update: probe against `stable/` (CADENZA_RUNTIME by ABSOLUTE path — relative fails
  the write silently) for reproducible readings; watch `implementation/seed/` mtimes as the ACTIVITY signal, not
  the measurement target; cross-check live-vs-stable only when the divergence is the question (e.g. "did the
  latest rebuild fix ask-42?"). No corpus/ask — a loop-procedure improvement.
- [A result-typed entry can mis-shape a deep sum-match in its call graph](./2026-07-07-a-result-typed-entry-can-mis-shape-a-deep-sum-match-in-its-call-graph.md)
  — wiring the self-hosted compiler's rejection path to `compile: … → result<list<u8>, list<diagnostic>>`
  made it decline itself ("runtime match with a non-literal pattern") on its deep `Core` sum-matchers, though
  they compile fine when `compile` returns bare `Bytes`. Root cause: a match arm's payload-slot binder
  (`func-body`'s `Core`-typed `body`) mis-infers as `Int64` because inference doesn't seed arm binders with
  their declared slot kinds; that wrong return kind mismatches a callee param, forces an inline, and the
  inlined body's constructor-`match` drops to the scalar path and declines. Amplified by inline-on-mismatch;
  surfaces only at self-host scale. The seeding fix re-walks the inference fixpoint → compile-cost blowup, so
  the durable response is the kinded-artifact interface (Amendment 0.8.0): one `{artifacts, diagnostics}`
  record on both success and rejection has same-shaped branches, so the choosing sum-match lowers cleanly.
- [A gate timeout is not a regression until you rule out contention — isolate before escalating](./2026-07-07-a-gate-timeout-is-not-a-regression-until-you-rule-out-contention.md)
  — after a seed rebuild the gate (normally ~2 s) timed out at 2 min, twice — looked like a hang/blowup
  regression. Isolating: `10-bytes.sexp` showed a per-file timeout, but every bisected half/quarter passed in
  ~1 s, the file alone ran 1 s / 51 PASS, and the full gate re-run when the box was quiet ran 2 s / 569 green. No
  hang — transient CONTENTION (a concurrent sibling seed rebuild competing for CPU/IO). Lesson: a timeout is a
  wall-clock signal that conflates "work got slower" (regression) with "box got busier" (contention); before
  escalating, re-run the suspect ALONE and compare to baseline — a real slowdown reproduces in isolation,
  contention doesn't. Don't cry regression on a timeout; don't dismiss a real blowup either — isolate and time.
  No seed defect (the finding was about the loop's own reaction). Also this cycle: ask-44 (the stray DBG
  eprintln) removed by the rebuild → done. Gate green 2 s, WRONG=0.
- [A stray debug print on stderr is invisible to every gate — the self-hosting probe caught it by reading the whole output](./2026-07-07-a-stray-debug-print-on-stderr-is-invisible-to-the-gate-but-caught-by-reading-the-self-host-output.md)
  — probing `compile-run` surfaced `DBG ctor-arm match, scrut_kind=Int64, scrutinee=Name("node")` — a leftover
  `eprintln!` in the seed's ctor-arm-match codegen, firing once as the seed compiles compiler.cdz itself. No gate
  caught it: it's on stderr, so emitted bytes / gate stdout / WRONG sweep are all blind to it (gate green 569,
  0 DBG on the corpus). Only a full-output read on the self-hosting path saw it. Two lessons: a gate measures the
  channel it's built to measure — noise (or a should-be-error warning, a perf cliff) on any other channel is
  invisible; read the WHOLE artifact output, not just the verdict. And a debug tripwire maps where the
  implementer is actively uncertain — the guard (ctor-pattern arm + non-Heap scrutinee) is the live inference
  edge. Filed ask-44 (LOW: remove/gate the eprintln). No corpus (stderr print, not a value-behavior).
- [The build-tool interface is a kinded-artifact list, not a two-arm result](./2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result.md)
  — the frozen build-tool-interface's derivation entry was `result<component-bytes, diagnostics>`, mutually
  exclusive: no warnings alongside a module, one byte output only, one input only. Generalized (Amendment
  0.8.0) to artifacts-in / {artifacts, diagnostics}-out — the component is one kinded artifact among DWARF /
  source map / manifest, the input list admits source units + a cache + dependencies, and per-diagnostic
  severity lets a warning ride alongside a produced component. Compilation is artifacts-in, artifacts-out
  with an always-live diagnostics channel; the component is not a privileged return value. Realized interface
  stays `result<list<u8>, list<diagnostic>>` (the degenerate case) until the artifact-list ABI is built out.
- [A frozen interface contract supersedes the in-flight asks that assumed its old shape — and freezes a seed-migration gap open](./2026-07-07-a-frozen-interface-contract-supersedes-in-flight-asks-and-opens-a-seed-migration-gap.md)
  — the new frozen `build-tool-interface.md` (Amendment 0.8.0) reshaped the diagnostics/output surface from a
  two-arm `result<list<u8>, list<diagnostic>>` to a KINDED-ARTIFACT interface (`compile-output = { artifacts,
  diagnostics }`; distinct channels, not arms). This supersedes the loop's open asks that assumed the old shape:
  ask-40's "return a Result" is now the wrong target, ask-38's Option-vs-Result flag is moot. Probe: the seed's
  DRIVER ABI hasn't migrated — `compile-run`/`component-check` still return a single `list<u8>`, type-rejections
  still bare-decline; the ~30 type-rejections stay `decline`-blocked on the seed+checker migration. Byte gate
  unchanged (65 agree, WRONG=0) because it measures the OLD ABI — the contract change is invisible to it. Lesson:
  a frozen contract is a SPEC event, not an implementation event; at a freeze the loop re-probes the seed against
  the new shape, re-targets the asks that assumed the old one, and records the migration gap — so a green gate
  (measuring the old ABI) isn't mistaken for conformance to the new contract. (The sibling's learning above covers
  WHY the shape changed; this is the loop's reconciliation + the seed-migration gap.)
- [Sharing the scratch-local mechanism cost right-shift its byte-identity — reuse has a fidelity price](./2026-07-07-sharing-the-scratch-local-mechanism-cost-right-shift-its-byte-identity.md)
  — a regression spot-check on the `agree` anchors caught `(>> 256 4)`, byte-identical in Run 73, now `soft`
  (value-correct → 16, byte-different; WRONG=0 — not a correctness regression). Cause: shift emit reuses the
  checked-arithmetic 3-slot scratch reservation, but `>>` needs only 2 (no overflow guard; native declares 2),
  so `>>` over-declares one local and drops out of byte-identity (`<<` needs 3 and stays agree). Lesson: a
  shared mechanism emits the UNION of its clients' needs — reuse (which made shifts cheap wiring) costs
  byte-fidelity on the client that needs less, and `agree` (byte-identical) is the only bucket that shows it;
  the last mile to agree on a reused mechanism is per-client tailoring. Process note: a rising `agree` count is
  not a superset — "61→65" doesn't prove the 61 stayed; spot-check the anchors. Filed ask-41 (LOW: direction-
  specific shift scratch-local count). No corpus (shift value/trap already pinned; this is byte-fidelity).
- ["Disagree" rising can be progress — cases moving off the decline floor into the soft/heap middle ground](./2026-07-07-disagree-rising-is-progress-when-cases-move-off-the-decline-floor.md)
  — the byte gate moved declines 377→330, disagrees 137→183 — reads like a regression, but the standing WRONG
  sweep stayed 0. Probing the ~46 that moved: the +3.3 KB compiler.cdz change EXPANDED coverage — many `let`/
  `match`/pattern constructs that previously declined (bare-`unreachable` stub) now COMPILE, to value-correct or
  heap results, leaving the decline floor for the soft/heap middle ground (which the 3-bucket gate scores
  `disagree`). Of 151 native=ok disagreements: 29 soft, 37 still decline-stub, 85 heap/other (WRONG=0 → none a
  miscompile). Lesson: a gate with fewer buckets than the phenomenon has states can't express direction — a
  rising `disagree` with WRONG=0 is coverage moving off the decline floor (good), not a regression; only `agree`
  (up=good) and the loop's `WRONG` (up=bad) move unambiguously. Read `component-check` deltas THROUGH the WRONG
  sweep. (ask-40 diagnostics still not landed — type-rejections still bare-decline, not coded Diagnostics.)
- [The type-rejection pass landed as a decline — and the diagnostics channel is the last gap from decline to agree](./2026-07-07-the-type-rejection-pass-landed-as-a-decline-and-the-diagnostics-channel-is-the-last-gap-to-agree.md)
  — ask-30's harder half landed: a `well-typed?` type-rejection pass run PRE-FOLD. Verified by discriminating
  disassembly — `(if true 1 false)` DECLINES (both branches supported types, so only the branch MISMATCH can be
  the cause → it's a real type-check, not an unsupported-operand decline), `(if true 1 2)` compiles, `(if true
  (+ 1 1) false)` declines (mismatch survives the fold → proves pre-fold placement). So genuine type mismatches
  now decline instead of mis-compiling. Byte gate stayed flat (61 agree) because the ~21 cases moved mis-accept →
  **decline**, but `component-check` scores them `disagree` still: native gives a CODED rejection, compiler.cdz a
  trap — decline ≠ coded-rejection. The sole gap to `agree` is the DIAGNOSTICS CHANNEL (ask-40: `compile` returns
  `result<_, list<diagnostic>>`). Self-caught misread: I first read the traps as unsupported-operand declines;
  the both-operands-supported discriminator settled it. Lessons: a type-rejection pass belongs pre-fold (test it
  with a mismatch whose branch would fold to a value); to tell a type-check from an unsupported-operand decline,
  probe where every operand is supported and only the mismatch is wrong; mis-accept → decline → agree is the
  reject-don't-miscompile ladder — decline is the milestone that removes the miscompile, agree needs diagnostics.
- [The arity subset of the type-checker landed first, exactly as scoped — with a let-form tail the fixed-arity check didn't reach](./2026-07-07-the-arity-subset-of-the-type-checker-landed-first-exactly-as-scoped-with-a-let-form-tail.md)
  — ask-30's cheap arity/well-formedness half landed (one `read-app` fixed-arity guard, as scoped): `(+ 1)`,
  `(+ 1 2 3)`, `(if true 1)`, `(< 5)`, `(not 1 2)` moved mis-accept → decline (trap), well-formed unregressed.
  Byte gate 59→61 agree, 148→136 disagree; WRONG sweep 0. Of the 33 native-rejected mis-accepts, ~12 moved; 21
  remain: ~19 TYPE-INFERENCE (int-vs-float no-promotion across all operators, mismatched-type, match
  exhaustiveness — the bigger half, needs the reject-on-kind-mismatch pass) + a **2-case LET-FORM tail** the
  fixed-arity check didn't reach (`let` is variable-arity; needs a small `read-let` check). Lesson: the
  enumerate-then-root-cause analysis held ("~10 arity errors = one guard, not ten"), but re-enumerating the
  RESIDUE after the landing caught what the category rounded off — "arity subset" was really "fixed-arity subset";
  a fix closes exactly the shape it matches, and the residue names both the next subset AND the fix's boundary.
  No new corpus (cases already pinned; gate measured the mis-accept→decline flip).
- [A flagged match-arm limitation was fixed — so the corpus workaround tightens to the precise pattern](./2026-07-07-a-flagged-match-arm-limitation-was-fixed-so-the-corpus-workaround-tightens-to-the-precise-form.md)
  — the explicit `((Ok a) …) ((Err _) …)` match arm on a `Result` decode (flagged as a seed limitation last cycle,
  worked around with `(else …)`) now type-checks. So the 4 decode corpus cases were tightened from the `(else)`
  workaround to the precise `((Err _) …)` arm (the one genuine catch-all stays `else`). Why it matters: `else` and
  `((Err _) …)` aren't equivalent — `else` passes whether the second variant is `Err`, another `Ok` shape, or
  nothing; the explicit arm pins the type is exactly `Result` and the error path is `Err` (exhaustive). Lesson:
  the mirror of the withheld-case discipline — a case shipped WITH a workaround to keep the gate green is a debt
  to spec precision; carry it as a flag and pay it down when the seed removes the workaround's cause. Gate 569, no
  new case/ask. (Re-flagged: compiler.cdz's "NOT YET: shifts" header is still stale.)
- [Shifts landed as the second guarded op — the local-allocating-machinery prediction paid off](./2026-07-07-shifts-landed-as-the-second-guarded-op-the-local-allocating-machinery-prediction-paid-off.md)
  — when ask-37 (checked arithmetic) closed, it was recorded that "shifts are unblocked — the local-allocating
  lower pass they also need is now real." This cycle shifts landed and probing confirmed it exactly: `<< >>` emit
  through the same scratch-local machinery with both guards, byte-faithful to native (in-range `256>>4=16`;
  count ≥ 64 TRAPS — no silent mask-mod-64; `1<<63` overflow TRAPS). Byte gate 58 → 59 agree; standing WRONG
  sweep stayed 0. No new corpus (shift behavior — const/runtime, in-range/guarded-trap — is ALREADY fully pinned;
  the byte gate measured compiler.cdz against it). Lesson: when the first instance of an architectural capability
  lands, the ops that were declined WAITING on it become cheap wiring, not fresh work — naming the acceptance
  list at decline time (the shifts-decline learning listed shifts + checked-arith as the local-allocating pass's
  acceptance list) makes the second op a verification, not a rediscovery. The two guarded ops (checked `+ - *`,
  shifts) share one mechanism. Stale-comment flag to the agent: the header still says "NOT YET: shifts."
- [A mid-flight signature change turns the gate red — the corpus must follow the seed, and the spec wording must be reconciled](./2026-07-07-a-mid-flight-signature-change-turns-the-gate-red-and-the-corpus-must-follow-the-seed.md)
  — ask-38 landed: `Ast.decode` became total, `Bytes → Result<Ast, e>` (Ok/Err, rejects invalid AND trailing
  bytes, never traps). The standing gate check caught it RED — 4 round-trip corpus cases still asserted the bare
  `Ast` form (now `(= (Ok ast) x)` → false). The loop migrated them to `(match … ((Ok a) (= a x)) (else false))`
  (the explicit `((Err _) …)` arm tripped a seed limitation — `(else)` works), added 2 error-case cases (garbage →
  Err, trailing → Err), restored green (569). Three lessons: (1) a signature change is a gate event — when the
  seed moves to the correct type, the corpus is downstream and migrates in the same cycle. (2) Migrate to the
  shape the seed ACCEPTS (probe first), don't iterate hand-written forms on a red gate. (3) The seed chose Result
  where value-interchange.md says "absence of a value" (Option-shaped) — a green gate means the corpus matches the
  SEED, not that the seed matches the SPEC; the Option-vs-Result divergence is flagged for the operator, not
  papered over.
- [The arithmetic-overflow arc closed — checked emit with scratch locals landed, and the wrong-value frontier is now empty](./2026-07-07-the-arithmetic-overflow-arc-closed-checked-emit-with-scratch-locals-landed-correctly.md)
  — the runtime `+ - *` overflow miscompile (ask-37) is fixed. The arc: miscompile (bare opcode wraps) → crash
  (checked emit with unreserved scratch locals → stack overflow) → reverted-miscompile → FIXED (checked emit with
  `sb` reserved past params+lets, `locals-decl` +3 i64). Verified: overflow TRAPS, in-range computes, and NESTED
  checked ops share scratch correctly (`(* (+ a b) c)` → 30). Byte-gate declines 369 → 335; corrected full-oracle
  dangerous sweep reports **WRONG = 0** — the arithmetic class is gone, the wrong-value frontier is empty. Lessons:
  the reject-don't-miscompile ORDERING (wrong-value < crash < decline < correct) held across the arc — every step
  UP was progress even when still broken, the one step DOWN (crash → reverted-miscompile) was the regression; and
  this is the first faithfully-emitted GUARDED op, so the "local-allocating lower pass" shifts+checked-arith both
  needed is now real. The nested-ops check is load-bearing (proves scratch slots are shared, not just allocated).
  No new corpus (cases already pinned; the byte gate measured it, WRONG=0).
- [The decode direction became a general value-interchange capability — and it picked Option, resolving the signature](./2026-07-07-the-decode-direction-became-a-general-value-interchange-capability-that-picks-option.md)
  — last cycle's operator correction (`Ast.decode` must be total, not trap; signature left open) resolved this
  cycle: a sibling landed `spec/capabilities/value-interchange.md`, a GENERAL capability for serializing/decoding
  any value, whose §"Decode Inverts Serialize And Refuses Otherwise" mandates decode "MUST yield the ABSENCE OF A
  VALUE rather than a value… an optional result rather than trapping." So the signature is **Option** (`Bytes →
  Option<Ast>`, matching `String.from-bytes`), and the principle generalized from AST to all values. Re-probe: the
  seed's `Ast.decode` still returns bare `Ast` and traps — unmet. Two lessons: (1) an operator correction naming
  a boundary condition ("decode of external bytes must not fail hard") is best promoted to a GENERAL capability at
  that boundary, not a patch to the one operation — every serializable type then inherits total-decode. (2) A new
  fallible op should wear the language's existing fallible surface (Option) rather than a bespoke error channel
  unless it needs the detail. ask-38 updated with the resolved signature; error-case corpus withheld until the
  seed makes decode Option-returning.
- [A decode over external bytes must be total (a Result), not trap — "refuse" is the error case, not a failure](./2026-07-07-a-new-decode-contract-landed-the-refuse-invalid-half-holds-the-no-trailing-bytes-half-does-not.md)
  — a new `deterministic-value-form.md` decode contract (inverting decode; refuse invalid; trailing bytes are an
  error). Probing the seed's `Ast.decode`: it TRAPS on invalid bytes and SILENTLY IGNORES trailing bytes. I first
  recorded the trap-on-garbage as correct ("refuse = trap") — the operator corrected it: `Ast.decode` consumes
  bytes that can come from an EXTERNAL source, so it must be TOTAL (return a Result/Option), never trap. That
  re-frames BOTH clauses as unmet: invalid bytes must return the error case (not trap), trailing bytes must be
  detected (not dropped). Lesson: the TRUST BOUNDARY decides trap-vs-Result — a partial op on a program's own
  values may trap (defined outcome), but a decode of possibly-external bytes must be total; "refuse" at that
  boundary means return the error case. I wrongly imported the compiler's honest-trap reflex onto a data decoder —
  reject-don't-miscompile (trap on an uncompilable construct) and total-decode (error value on unparseable data)
  are different disciplines for different layers. Filed ask-38 (make `Ast.decode` total; signature Option vs
  Result is an operator call — ripples to 9 round-trip cases). No corpus this cycle (the trap-asserting case I
  briefly added was reverted; error-case cases withheld until the signature is decided).
- [The checked-arithmetic fix regressed the emit path — and it's a crash, not a miscompile, which is the right kind of regression to have](./2026-07-07-the-checked-arithmetic-fix-regressed-the-emit-path-a-decline-would-have-been-safer-first.md)
  — ask-37's fix landed (+6.5 KB: `KAdd/KSub/KMul` → `checked-binop` inline overflow guards over 3 scratch
  locals; the emit sequence is correct). But re-probing showed it REGRESSED: runtime `+`/`-`/`*` now make the
  compiler.cdz component TRAP (infinite recursion, wasm fn 64 → stack overflow) instead of emitting — isolated to
  the 3 checked ops (`id`/`<`/`&` still compile). Root cause: scratch-local base `sb` not reserved past
  params+lets (`locals-decl` must declare 3 more i64 slots). Byte gate regressed 140 → 172 disagree. Two lessons:
  (1) only the self-hosting re-probe catches it — the behavior gate stayed green (runs native, unaffected); a
  self-hosted regression is invisible to every non-self-hosting gate. (2) **A crash-regression is the RIGHT kind
  to have** — it moved runtime overflow from a silent WRONG VALUE (`* MAX 2` → -2) to a TRAP; reject-don't-
  miscompile held through the mistake. Sequencing lesson: when the faithful fix needs new machinery (scratch
  locals the fold-only Lir never had), land the DECLINE first and the emit behind it — a half-built emit that
  crashes is tolerable only because the decline underneath would catch it. No new corpus (cases already pinned).
  **Follow-up next cycle:** the crash was fixed by REVERTING to the bare opcode — which restored the original
  wrong-value miscompile (`* MAX 2` → -2), a step BACKWARD (traded a safe crash for an unsafe wrong value). Lesson
  sharpened: when a fix breaks, revert toward the SAFER failure, not the original — outcome ordering is
  wrong-value < crash < decline < correct; unblock toward DECLINE, never back toward wrong value. A revert isn't
  safe just because it restores a known-"working" state; if that state was a miscompile, the revert reintroduces it.
- [The compiler emits bare arithmetic that wraps instead of trapping — and a scalar-only scan hid the overflow miscompiles](./2026-07-07-the-compiler-emits-bare-arithmetic-and-a-scalar-only-scan-hid-the-overflow-miscompiles.md)
  — a quiet-cycle completeness sweep for wrong-value miscompiles came back "0 WRONG" (108 native=ok disagreements
  = 28 soft + 77 hidden declines + 3 other) — but the 3 "other" I nearly dismissed as "no scalar oracle" were
  TRAP-oracle cases the scan filtered out, and `compiler.cdz` runs them to a value: it emits bare
  `i64.add/sub/mul` that WRAP on overflow (`Int64.max+1`→MIN, `*2`→-2) where the default `+ - *` MUST trap. A
  wrong-value miscompile class, same severity as `(id true)` (ask-34), the arithmetic core — filed high-priority
  ask-37 (emit a checked lowering as `/ %` already do, or decline). Two lessons: it's a real miscompile; AND my
  own scan had the exact proxy-leak this loop keeps documenting — "0 WRONG" meant "0 wrong among scalar-oracle
  cases," and filtering out trap oracles dropped precisely the cases where a wrap-instead-of-trap lives. A scan is
  only as complete as the oracle it consults; "no scalar oracle → skip" is how a trap-required miscompile hides
  in the `other` pile. No new corpus (overflow-traps cases already pinned; behavior gate green because native
  traps).
- [The ask lifecycle closed its first validation round-trip — and a miscompile fixed by declining is a valid resolution](./2026-07-07-the-ask-lifecycle-closed-its-first-validation-round-trip-and-a-miscompile-fixed-by-declining.md)
  — the compiler agent adopted the ask-lifecycle and moved four asks into `pending-validation/`; the loop
  re-probed all four against the live artifact and confirmed → `done`: ask-19 (nested ctor under `Some` on a
  param list — now compiles, pinned as a gate case → 5), ask-25 (the `main`-named entry reorder, unblocked by
  gap-3m — a helper-first module now runs to 42; byte gate 153 → 141 disagree), ask-31 (checked arithmetic), and
  ask-34 (the first miscompile — `(id true)` → `1` now TRAPS). Two lessons: (1) the lifecycle is a two-party
  protocol where `done` = the loop re-probed the live binary, never the implementer's claim — which mattered
  because compiler.cdz moved twice mid-cycle and only re-running gave honest numbers. (2) **A miscompile fixed by
  DECLINING is the right first fix, not a half-fix** — ask-34 was resolved via decline (trap, don't mis-widen a
  Bool to i64), moving it `disagree → decline` (out of the real-miscompile column); byte-identity is a separate
  low-priority follow-on (ask-35). Principle: when a miscompile can't yet be compiled correctly, make it decline
  — restore reject-don't-miscompile now, chase byte-identity later; the wrong value in between is the dangerous
  state and should exist briefly. One new corpus case (ask-19's shape → 5, gate 562).
- [The byte gate found its first real miscompile — a polymorphic identity loses its Bool return — and the decline discriminator is too narrow to see it](./2026-07-07-the-byte-gate-found-its-first-real-miscompile-a-polymorphic-identity-loses-its-bool-return.md)
  — running every one of the 153 byte-gate disagreements (not trusting the aggregate) split them: 28 soft, **77
  hidden declines** (trap at runtime but NOT a bare-`unreachable` entry, so ask-29's discriminator misses them),
  33 native-rejected (ask-30), and **1 REAL MISCOMPILE**: `(def (id x) x) (def (main) (id true))` returns `1`,
  not `true` — compiler.cdz frames the polymorphic `id` as i64, widens the Bool `1` to i64, and lifts `(result
  s64)` where native lifts `bool`. Root cause: the return-kind fixpoint propagates a BODY-shaped Bool return but
  not an ARGUMENT-shaped one (a function whose return kind is its argument's). Two findings → ask-34 (the
  miscompile: specialize the pass-through return to the applied argument's kind, or decline — never mis-widen)
  and ask-33 (widen the discriminator to a runtime-trap check so `disagree` means running-wrong-value, ~1, not
  153). No new corpus (`(id true)` already pinned; behavior gate green because native handles it). Lesson: a
  gate's discriminator is only as good as the failure shape it models — "decline = bare unreachable" hid 77
  declines; a decline is "traps at runtime." Run the artifact; the entry-func shape is a proxy, and proxies leak.
- [The byte-level gate's decline discriminator exposed the real self-hosting frontier: the compiler has no type-checker](./2026-07-07-the-byte-level-gate-decline-discriminator-exposes-the-missing-type-checker.md)
  — the decline discriminator (ask-29) landed: `component-check` went 58 agree / 496 disagree → 58 agree / 152
  disagree / **344 decline** / 204 skip. Splitting declines off made the 152 legible: 117 are the fold-vs-helper
  `soft` set (fine), but **33 are `native=rejected / component=ok`** — `compiler.cdz` COMPILES ill-typed programs
  native REJECTS (`(if true 1 false)`, `(+ 1 true)` → `Ok`, native declines). It has NO type-checker: reads →
  resolves → folds → lowers → emits, no type-rejection pass. 33 span CDZ0201 (19, cond branch/condition), CDZ0301
  (11, no-promotion operands), CDZ0210 (3, non-exhaustive match). A whole-program reject-don't-miscompile
  violation, invisible until the discriminator split off the decline noise. Filed ask-30 (type-checker + the
  diagnostics ABI it needs). No new corpus (the 33 are already rejection cases native realizes). Lesson: the
  strictest gate you can afford is worth its discriminator — byte-identity against a reference that REJECTS
  ill-typed programs is the only differential that catches a missing type-checker; every weaker gate accepts the
  same programs the buggy compiler does. And a discriminator doesn't just make the count honest — it makes the
  residue legible (the 33 named themselves once decline noise was subtracted).
- [The byte-level self-hosting gate runs — and its "disagree" count conflates honest declines with real miscompiles](./2026-07-07-the-byte-level-self-hosting-gate-runs-and-its-disagree-count-conflates-declines-with-miscompiles.md)
  — the last wiring step landed (`compile-run --emit-component`, SPEC-BACKLOG #28), so the real byte gate runs:
  persist compiler.cdz → 27 KB component, `component-check <it> spec/semantics` → **58 agree, 496 disagree, 204
  skip**. But 496 is misleading: 158 disagreements emit the byte-IDENTICAL 88-byte component = `func 0 →
  unreachable`, an honest `KError` decline (two different unhandled programs → same 88 bytes, traps when run).
  `component-check` byte-compares a decline stub against native's real output and calls it `disagree` — the same
  decline-vs-result blind spot as the trap oracle (#26), now at the byte level. True frontier once declines are
  excluded: ~58 agree + soft set, rest = reader doesn't decode records/strings/floats/effects yet (expected). Fix
  handed to agent: `component-check` must classify a bare-`unreachable` entry as `decline`, not `disagree`. No
  corpus case (gate classification, not spec). Lesson, now proven across THREE gates (value/trap/byte): every new
  differential inherits the decline-vs-result blind spot and needs the discriminator explicitly — a headline
  count is trustworthy only where the shared observable can't be counterfeited (agree/byte-identical UP, never
  disagree DOWN).
- [Gap 3n is fixed — the self-hosting loop is operational for arbitrary programs, and the byte-level gate is one step away](./2026-07-07-gap-3n-fixed-the-self-hosting-loop-is-operational-and-the-byte-gate-is-one-step-away.md)
  — the `compile`-return mod-4 alignment bug (narrowed over the prior cycles, fix `(p+3)&!3` converged with the
  compiler agent) landed in the seed. The loop re-probed every input that failed last cycle (`0`/`1`/`true`/`256`/
  len-31/33/34) — ALL now `Ok`, across all mod-4 residues. So `compile-run` works for arbitrary programs, and a
  byte-level differential runs: `compiler.cdz` is byte-IDENTICAL to native on `(main) 42`/`(< 3 5)`/depth-2 chain,
  `soft` on `(+ 20 22)`/`(dbl 21)` (native overflow helpers vs mine folding) — the real self-hosting agreement
  through the ABI it ships. Full report→fix→confirm round trip through the loop→agent channel. No corpus case
  (seed ABI defect; the values are already pinned). Next: `component-check` is the byte gate but reads a compiler
  component from a fixed path and can't be pointed at a compiler.cdz-built one — one seed step (persist the
  compile-component) unblocks the whole-corpus byte gate. Lesson: a loop that hands the agent a root cause AND
  verifies the fix closes the feedback edge — confirming gaps stay closed is as much the job as finding them.
- [The self-hosting loop runs end-to-end — and the compile-return alignment bug has a sharp value threshold the handoff doc missed](./2026-07-07-the-self-hosting-loop-runs-end-to-end-but-the-compile-return-trips-on-a-value-threshold.md)
  — `compiler.cdz`'s entry was rewired to `(def (compile b) (compile-bytes b))` — the real self-hosting seam
  (pending step 1 from the bytes→bytes learning, now landed). `compile-run` compiles `(module m (def (main) 42))`
  → the correct 89-byte component through the full pipeline: the compiler is a genuine byte-transform now. But
  the seed's `compile`-RETURN wrapper trips "return pointer not aligned" (gap 3n). Probing the CURRENT seed
  corrected the handoff doc twice: (1) its fixed-output repro `(Bytes.of (list 0 0 0 0))` now PASSES — a partial
  fix landed, the doc is stale; (2) the real failure is a SHARP DETERMINISTIC VALUE THRESHOLD, not
  "allocation-dependent" — `(main) N` fails for N ≤ 23, succeeds for N ≥ 24 (bisected), identical 89-byte output
  both sides, single operand-byte difference. So `0`/`1`/`true`/`256` fail but `42`/chains succeed — the simplest
  inputs are the minimal reproducer, opposite the doc's implication. The bug is the seed's computed-`list<u8>`
  return marshalling (rope-flatten-to-retarea offset), not the compiler (all these `emit` byte-identically to
  native). No corpus case (seed component-ABI defect). Lesson: a handoff doc's open-bug characterization is an
  aggregate to re-probe — "fails at every size" became "fixed-output works; real compiler fails for N ≤ 23," a
  different and actionable shape. The self-hosting loop is functionally CLOSED; the last blocker to a byte-level
  gate is this one alignment bug, not any compiler capability. **Follow-up next cycle:** the "value threshold at
  24" is a proxy — the real trigger is INPUT-LENGTH mod 4 (`input_len % 4 == 0` aligns, else misaligned retarea;
  24 is the CBOR 1→2-byte int boundary that flips input length 31→32). Fix = round the bump pointer up to 4
  (`(p+3)&!3`) before the retarea. The loop first read this as parity from an under-sampled table, then a len ≡ 2
  probe + cross-check with the compiler agent's note settled it as mod 4 — agent and loop converged on the same
  root cause and fix. Progression of proxies: wrapper → value → CBOR-boundary → parity → mod 4 → bump-align-up
  (the parity step was an over-generalization from an under-sampled table — over-sample a re-probe before publishing).
- [The return-kind table is a monotone fixpoint, and it propagates a Bool result to any call depth — the capability gap 3k unblocked](./2026-07-07-the-return-kind-table-is-a-monotone-fixpoint-and-it-propagates-bool-to-any-depth.md)
  — the compiler needs each function's result kind (i32/Bool vs i64/Int) to frame its wasm signatures and calls.
  A single-pass table handles a directly-Bool-bodied helper, but a TRANSITIVE chain (`a` returns `b`'s result,
  `b` returns `c`'s, only `c` has a Bool body) is a FIXPOINT over the call graph. The spike landed
  `build-ktab`/`ktab-iterate` (a monotone fixpoint), and probing compiler.cdz confirmed it byte-identical to the
  seed at depth 1/2/3 (108/124/140 B, every func framed `result i32`). This is the capability gap 3k /
  [[a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop]] was blocking — the compiler shipped
  a single-pass STOPGAP because the seed OOM'd on the fixpoint shape; 3k fixed → the true fixpoint became
  expressible and replaced the stopgap. Pinned `09-functions` *"a boolean result propagates through a three-deep
  chain of forwarding functions"* (→ true, byte-identical 131 B) — depth-3 is what distinguishes a fixpoint from
  a single pass (the two-deep case one propagation step also passes). Lesson: the compiler's "single-pass / NOT
  YET / reverted" comments are a live map of the seed's frontier — a stopgap approximation (this) and a stopgap
  decline (shifts) both resolve by the seed growing, not the compiler working around.
- [A decline that lands on a trap-expecting oracle is coincidental agreement, not a semantic trap — the trap-oracle dual of reject-don't-miscompile](./2026-07-07-a-decline-that-lands-on-a-trap-oracle-is-coincidental-agreement-not-a-semantic-trap.md)
  — a quiet-cycle completeness sweep of the interim harness's new `trap-ok` bucket (oracle expects a trap, mine
  also traps). The board looked clean — 22 agree / 6 soft / 4 trap-ok / 0 hard — but probing the four realized
  `trap-ok` cases showed every one is a bare `unreachable` DECLINE: `compiler.cdz` doesn't support `record` /
  `Bytes.of` yet, so it traps by declining, NOT by the byte-range or missing-field check the case pins (a valid
  in-range `(Bytes.of (list 65 66))`, which must NOT trap, also traps). Coincidental agreement: right observable
  (a trap), wrong reason (unsupported). This is the dual of reject-don't-miscompile — on a value oracle a decline
  is visibly distinct, but on a TRAP oracle a decline and a semantic trap produce the identical `unreachable`, so
  a value-only harness can't separate them, and a wrong range check added later would still score `trap-ok`
  (silent regression). No corpus change (the cases are correct); the gap is in the measurement. Fix: read
  `trap-ok` as "traps, reason unverified"; pair each out-of-range case with an in-range companion that must NOT
  trap (the discriminator a value-only trap oracle lacks) — recorded for the real `component-check` gate too.
  Lesson: a bucket that agrees on an observable a decline can counterfeit is the WEAKEST evidence on the board,
  not the strongest.
- [A `bytes → bytes` compile entry unblocks the real differential harness — the seam is landed, the compiler just hasn't moved onto it](./2026-07-07-a-bytes-to-bytes-compile-entry-unblocks-the-real-differential-harness.md)
  — SEED-GAPS gap 3l (the seed could only lift a nullary `run`, not a `compile : list<u8> → list<u8>` component)
  is RESOLVED on the seed side. Probing the rebuilt seed confirmed it: a new `compile-run` subcommand builds a
  compiler as a compile component and drives it over an input's AST bytes; an identity `(def (compile b) b)`
  builds a valid 3,059-byte component and round-trips the input's 32 canonical AST bytes through the list ABI.
  BUT a second probe caught the un-crossed seam: `compile-run` on the actual `compiler.cdz` fails "expected 0
  argument(s), got 1" — the committed compiler still exports a nullary `(def (main) …)` with the target bytes
  HARDCODED, so it's lifted as `run`, not `compile`. The rewire (`main` → `(def (compile b) (compile-bytes b))`)
  is one line and `compile-bytes` already exists; the full `component-check` corpus diff additionally waits on
  the value-heap runtime component building again (CHAMP mid-implementation). Neither is a language/correctness
  gap. No corpus case (a bytes→bytes entry is an ABI contract, not a scalar oracle). Lesson: a resolved gap is a
  CAPABILITY, not a CONNECTION — "is 3l fixed?" is yes for the seed, no for the end-to-end loop; only running the
  actual artifact (not the handoff banner's "VERIFIED end-to-end", which described a since-reverted rewire)
  distinguishes them.
- [The self-hosted reader compiles a multi-def call — but picks the entry by position, and the name-based reorder is blocked on a seed blowup](./2026-07-07-the-self-hosted-reader-compiles-a-multi-def-call-but-picks-the-entry-by-position.md)
  — the harness's new `error` bucket (invalid emission, distinct from a clean decline) flagged a two-def module
  whose entry calls a user function. Direct probing reduced it: the underscore in the "underscore parameter"
  case is a red herring (the plain-name twin fails identically), and disassembling the invalid bytes showed the
  real cause — the reader takes the FIRST def as the nullary `run` entry positionally, while native selects the
  def NAMED `main`. So a helper-first module lifts a param'd `f` as the nullary entry and strands its argument
  (`values remaining on stack`). The multi-def user-function CALL works end-to-end whenever the entry is first
  (`(def (main) (f 41)) (def (f x) (+ x 1))` → 42, valid); only entry SELECTION is the gap. The name-based
  reorder is written but reverted — it tips the seed's compile-time evaluator into an exponential blowup
  ([[compiler-exponential-in-nesting-depth]], SEED-GAPS 3m) — so `entry-guard` makes the mismatch a clean
  decline, never invalid bytes. Caught the fix land MID-PROBE (invalid → clean decline as the spike edited
  compiler.cdz live). Pinned `09-functions` *"the module entrypoint is the def named main regardless of its
  position"* (→42, AGREE). Lesson: a harness bucket is an aggregate to probe, not a diagnosis — "invalid on a
  param name" was really "positional-vs-named entry"; and probe the artifact as it is NOW, the spike may fix it
  under you.
- [A fixpoint loop's compile blowup is the fresh-re-seed-plus-list-result conjunction — not the loop, and not either half alone](./2026-07-07-a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop.md)
  — the self-hosting return-kind machinery's next step is a monotone fixpoint, and two reproducers still OOM the
  seed. The handoff doc blamed "a `list` parameter re-seeded with a fresh `(list)` each round"; four direct
  `emit` probes narrowed it to a CONJUNCTION — fresh re-seed AND the result consumed as a list. Threading the
  incoming list (even growing it by `List.push` each round) compiles; re-seeding fresh while consuming the result
  as an Int64 compiles. Only both together diverge. Same class as [[eval-const-let-memoization-blowup]] /
  [[threaded-compound-accumulator-inference-blowup]] — an inference fixpoint that fails to reach a fixed KIND.
  The OOMing program can't be a corpus case (it hangs the gate), so the pin is the passing side of the boundary:
  05-compound-types *"a fixpoint loop that threads a growing list accumulator returns that list"* (→5, AGREE).
  Backlog carries the corrected trigger + the four controls. Lesson: a handoff doc's one-line trigger is an
  aggregate to probe, not trust — the probe turned a one-variable claim into a two-variable conjunction, the
  difference between a fix that works and one aimed at a shape that was never broken.
- [The compiler core was restarted four times](./2026-07-02-compiler-core-restarted-four-times.md) —
  why the specification, not the compiler, is the durable artifact.
- [Component output never materialized](./2026-07-02-component-output-never-materialized.md) — why the
  component ABI and determinism are frozen contracts written before the capabilities.
- [Four parallel semantics drifted](./2026-07-02-parallel-semantics-drifted.md) — why there is one
  executable semantics, gated by execution.
- [Multiple front-ends diluted one surface](./2026-07-02-multiple-frontends-diluted-one-surface.md) —
  why there is one canonical representation with decoupled displays.
- [Verification was baked through the tree](./2026-07-02-verification-baked-through-the-tree.md) — why
  verification is progressive and meaning-preserving.
- [There was no line of sight to self-hosting](./2026-07-02-no-line-of-sight-to-self-hosting.md) — why
  the reference interpreter is the oracle and the seam to the flywheel.
- [A modeled subsystem passes a shape check](./2026-07-02-a-modeled-subsystem-passes-a-shape-check.md)
  — why behavior requirements are discharged by execution and every requirement binds to an enforcing
  line. (Adopted from the host project's own hard-won lesson.)
- [The seed is a dynamic interpreter](./2026-07-02-seed-is-a-dynamic-interpreter.md) — why the seed
  generation defers static typing and realizes evaluation dynamically to get the flywheel turning, and
  the Core Principle VII bootstrap carve-out that records the amendment.
- [The ignition path is de-risked](./2026-07-02-ignition-path-de-risked.md) — the two Phase-2 spikes:
  duvet's quoted-sentence gate works for Rust (but exits 0 on citation errors), and the
  source→derive→run→re-derive path is real and byte-reproducible in this environment.
- [Decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md) —
  interpreted derivation embeds the interpreter *component* over the program's AST (so the component
  actually interprets, not replays a transcript); the host providing capability functions is a
  separate minimal artifact. Avoids the modeled-derivation trap.
- [Bootstrap is interpreter-first, not compiler-first](./2026-07-02-interpreter-first-not-compiler-first.md)
  — why a compiler-first self-hosting proposal was considered and rejected (it has no behavioral
  oracle and revives the meaning-in-the-compiler failure), while its compatible ideas were adopted;
  switching would be a deliberate constitution IX/XIV amendment.
- [An effect-only program had no normal-termination value](./2026-07-02-effect-only-programs-need-a-unit-value.md)
  — why a Unit value was pinned (additively) so event-emitting programs carry a definite terminal
  condition; surfaced by four corpus cases that had only an `(events …)` observation and no primary
  result clause.
- [Real components, not a bespoke module model](./2026-07-03-real-components-not-a-bespoke-module-model.md)
  — why the bootstrap uses real WebAssembly components (`wit-bindgen` core module → `wasm-tools
  component new`) rather than a hand-managed `wasm32-unknown-unknown` core module with an AST slot and
  trimmed imports; the WIT world makes "imports mirror the manifest" hold natively, which reverted the
  short-lived 0.3.0 import amendment. Includes the offline de-risk-spike findings.
- [The seed needs first-class functions](./2026-07-03-the-seed-needs-first-class-functions.md) — why the
  seed realizes functions and closures (core-semantics.md §Functions): the first Cadenza artifact is a
  compiler, which is not expressible without them.
- [Bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md) —
  why the staged path collapsed to seed interpreter → Cadenza compiler → self-hosting, dropping the
  re-author-the-interpreter-in-Cadenza rung; the reference interpreter stays the oracle, so IX/XIV hold.
- [The seed realizes a byte-sequence form so the Cadenza compiler emits component bytes](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)
  — why the codegen is authored in Cadenza (not the seed) and the seed realizes a `Bytes` value form:
  an attended halt when the build was about to write the codegen in Rust, and the seed↔compiler seam it
  hardened (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed").
- [The Cadenza compiler emits the whole component](./2026-07-03-the-compiler-emits-the-whole-component.md)
  — why the compiler emits the complete component binary as a value rather than a core module a tool
  completes, so a derivation's bytes are a function of the Cadenza compiler alone and self-hosting is a
  clean fixpoint (no external wrapping tool in the byte path).
- [One accessor, everything is a record](./2026-07-03-one-accessor-modules-are-records.md) — why `.` is
  the sole record accessor (`a.b` is sugar for `(. a b)`), why modules/records/prelude namespaces are
  all records while maps stay dynamic, and the `(meta …)` metadata channel; killed a lowercase/uppercase
  dotted-atom heuristic that re-parsed meaning from an atom's spelling.
- [Exhaustion is a trap across the compiled seam](./2026-07-03-exhaustion-is-a-trap-across-the-compiled-seam.md)
  — why a derived component that exhausts the resource measure halts as a trap and is judged as agreeing
  with the interpreter's `exhausted` terminal condition; surfaced by the differential gate before growing
  the compiler to recursion, so the two recursion cases don't flip to a false `disagree`.
- [Decline, do not miscompile](./2026-07-03-decline-do-not-miscompile.md) — why a compiler grown
  incrementally MUST trap/decline a construct it cannot yet compile rather than emit divergent bytes or
  silently skip it, keeping "cannot yet" and "does wrong" observably distinct so a green differential gate
  means every compiled program agrees.
- [The corpus is a differential gate](./2026-07-03-the-corpus-is-a-differential-gate.md) — why the
  generated path is exercised against the oracle over every corpus case the compiler compiles, turning the
  executable-semantics corpus into a live regression surface as the compiler grows (agree/todo/skip/disagree).
- [The assembler lives in Cadenza](./2026-07-03-the-assembler-lives-in-cadenza.md) — why even the
  instruction-to-bytes assembly step is authored in Cadenza (a WAT-like structured layer folded to bytes),
  not delegated to a host `wat`-crate pass, so no part of the translation escapes the Cadenza compiler.
- [The compile seam is statically typed](./2026-07-03-the-compile-seam-is-statically-typed.md) — why the
  seed invokes the Cadenza compiler through a byte-to-byte interface (`compile : list<u8> -> list<u8>`)
  rather than through its dynamic value type, so no dynamic-language assumption is baked into the
  compiler's contract and a later generation can type-check the same seam; surfaced when the self-hosting
  harness needed the interpreted and compiled compilers to share one static type to be comparable.
- [Author Cadenza as static even though the seed is dynamic](./2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic.md)
  — why every line of Cadenza source is written as a well-typed static program (sum types + `match`, not
  runtime `Ast.is-*` kind-reflection) even though the seed defers type-checking, so the source is accepted
  unchanged by the later type-checking generation rather than rewritten and the §VII deferral stays a stage.
- [Uniform single-arity constructors eliminate cascading special cases](./2026-07-03-uniform-single-arity-constructors.md)
  — why all sum type constructors are single-arity functions (including "nullary" variants that take Unit),
  rather than nullary-as-pre-applied-Sums vs unary-as-Constructors, eliminating arity-based special cases in
  pattern matching, type synthesis, and compilation; the dual representation compounded (each feature checked
  "which kind?"), and adding unit broke all tests when one check was missed.
- [Types first-class in the dynamic seed sets up static self-hosting](./2026-07-03-types-first-class-in-dynamic-seed.md)
  — why the seed makes types first-class values even though it's dynamically checked (§VII defers checking,
  not types themselves), and why the AST is quotable as a sum type: compiling dynamically-written code to
  static is incredibly hard, but runtime-checked types written with type annotations transition smoothly to
  compile-time checking (move validation earlier, not infer what wasn't written); quote/unquote lets the
  compiler operate on AST values natively rather than string-tagged reflection.
- [Quasiquote for programmatic AST construction](./2026-07-03-quasiquote-for-programmatic-ast-construction.md)
  — why quasiquote with selective evaluation (`,` unquote, `,@` splice) is necessary once the compiler
  operates on AST values: `quote` is uniform (never evaluates), but instruction construction needs to embed
  computed values; without quasiquote, building `(+ x 10)` where `x` varies means verbose
  `(Ast.List (list ...))` calls; `` `(+ ,x 10)`` reads like the instruction and makes the compiler maintainable.
- [AST construction vs AST evaluation: the compiler needs construction only](./2026-07-03-ast-construction-vs-ast-evaluation.md)
  — why the compiler needs quasiquote (AST construction) but not `eval` (AST execution): inside quasiquote,
  `,expr` evaluates `expr` normally to embed its value (statically checkable); top-level `(eval ast-value)`
  executes AST as code (meta-interpretation, needs embedded interpreter, hard to do statically). The compiler
  constructs and analyzes AST but never executes dynamically-constructed AST. Eval is optional for macros/REPL.
- [Two compilers, not an interpreter and a compiler; the runtime is wasm](./2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md)
  — why the seed stops being a reference *interpreter* and becomes a reference *compiler* (`cdz-rustc`): the
  runtime is wasm, an interpreter and a compiler share almost nothing, and codegen was being grown blind. The
  oracle becomes the conformance corpus, and independence comes from two implementations of the compiler that
  must agree — in place of an interpreter-vs-compiler differential (Constitution Amendment 0.3.0).
- [Static typing is mandatory once the seed is a compiler](./2026-07-04-static-typing-is-mandatory-post-pivot.md)
  — why Constitution Amendment 0.4.0 retires the Principle VII dynamic-seed carve-out: the carve-out was
  conditioned on realizing evaluation dynamically, which the two-compiler pivot removed, so the seed compiler
  must reject ill-typed programs with a machine-readable code (incrementally, reject-don't-miscompile) rather
  than defer typing; the corpus `(compiler …)` clauses become the seed's own rejections.
- [Nominal is an orthogonal tag over any structural type](./2026-07-04-nominal-is-orthogonal-tag-over-structural-types.md)
  — why nominal-versus-structural is one orthogonal axis over every structural type (record, tuple, sum), a
  nominal value being its structural value plus a compile-time, fully-qualified name tag that adds nothing to
  the runtime representation; nominal types are not comparable across their boundary, and identity is the
  module path plus declared name.
- [Generics are type-valued parameters, not a separate polymorphism mechanism](./2026-07-04-generics-are-type-valued-parameters.md)
  — why generics fall out of first-class types plus compile-time evaluation: a generic is an ordinary
  definition taking type-valued parameters, a type constructor is a compile-time type→type function, and
  monomorphization is the existing compile-time reduction — no separate polymorphism or trait-resolution engine.
- [The host is value-agnostic; the compiler owns the reader and printer](./2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer.md)
  — why a compiled program's result crosses the boundary as its proper component type, exported as a resource
  owning a `display` method, rather than teaching the host Cadenza's value shapes or collapsing the boundary to
  a string; the reader/printer are compiler-exposed text↔binary surfaces so the host stays value-agnostic.
- [Type inference is Hindley-Milner](./2026-07-04-inference-is-hindley-milner.md)
  — why inference is unification over type variables yielding principal types with let-generalization, not
  ad-hoc guessing from a single call site; a parameter's type is the solution derived from all its uses at
  once, contradictory constraints are a compile-time rejection, and let-generalization is the same mechanism as
  generics being type-valued parameters.
- [An immutable heap is acyclic, so reference counting is complete](./2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md)
  — why immutability + strict evaluation forbid heap cycles, which makes reference counting sound AND complete
  (no tracing GC, no cycle collector); the allocator is emitted into the component so the host provides only
  linear memory; Perceus-style in-place reuse makes persistence free when unshared. Drives
  memory-and-resource-model.md and the new `options/memory-ownership-model/`.
- [Effects are algebraic; a capability is a boundary effect; mutation is a State effect](./2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects.md)
  — why the effect open question resolves to algebraic handlers unified with capabilities (the manifest is the
  effect row that escapes to the host), mutation re-enters as a pure-state-passing `State` effect, and
  continuations are one-shot (affine) to keep fuel accounting and RC sound. Drives capabilities-and-effects.md
  and `options/effects-model/`.
- [Records are rows: row polymorphism does triple duty](./2026-07-04-records-are-rows-open-by-default.md)
  — why records gain row polymorphism (open over fields), which also types effect rows and preserves principal-
  type inference; subset comparison is explicit projection-then-`=`, never an overloaded `=`; row variables are
  monomorphized to closed shapes before the boundary. Drives type-system.md §The Declarable Type Universe.
- [Ad-hoc polymorphism: traits are dictionaries, scoped, not coherent](./2026-07-04-traits-are-dictionaries-scoped-not-coherent.md)
  — why a trait is a dictionary record type and an instance an ordinary value (Scala-`given`/OCaml-implicits/
  F#-SRTP shape), resolved by deterministic source-ordered scoped search and monomorphized away — NOT Haskell
  global coherence or orphan rules, which fight content-addressed modules. Drives type-system.md and
  `options/ad-hoc-polymorphism/`.
- [The refinement layer is liquid types; verification is extrinsic](./2026-07-04-refinements-are-liquid-verification-is-extrinsic.md)
  — why refinements are liquid (decidable predicate logic, SMT-discharged into a checkable certificate) and
  machine-checked verification is extrinsic (about behavior, not propositions-as-types), which is what keeps
  `Type : Type` sound; discharge must be proof-producing. Drives verification-layers.md, type-system.md, and
  `options/verification-strategy/`.
- [Linearity is surgical, not core; graded types are the aim](./2026-07-04-linearity-is-surgical-not-core.md)
  — why linear/affine types are NOT mandatory core (immutability + RC already cover memory) but ARE used
  surgically (one-shot continuations, linear capability handles, an optional usage layer); graded/quantitative
  types with an erased `0` multiplicity are the course to aim at. Course-setting; drives annotations across
  memory/effects/verification specs.
- [HM inference and first-class types meet at a bidirectional boundary](./2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary.md)
  — why principal-type HM inference and computable first-class types are reconciled: HM over a non-computational
  term core, with a bidirectional-checking boundary at type-valued-parameter positions (synthesized by
  monomorphization or checked against an annotation), closing a literal contradiction in type-system.md §Inference.
- [Compile-time evaluation is one tier](./2026-07-04-compile-time-evaluation-is-one-tier.md)
  — why macros, generics, monomorphization, and const-folding are the SAME pure, bounded, deterministic
  compile-time evaluation (one mechanism, not four subsystems that drift); a macro is an ordinary phase-1 Cadenza
  function over Ast, and the tier runs in the empty effect row so purity is a consequence of the effect model.
- [Macros are typed (Expr[T]) and hygienic (sets-of-scopes)](./2026-07-04-macros-are-typed-and-hygienic.md)
  — why the static spine forces typed quotes (Expr[T] over the untyped Ast analysis substrate, so ill-typed
  macro output is rejected at the macro, not downstream) and why hygiene is realized by Racket's set-of-scopes
  model; drives an ADDITIVE ast-encoding.md extension (identifiers carry scope sets), operator-approved to enact.
- [Macro phases; the reader stays fixed](./2026-07-04-macro-phases-and-the-reader-stays-fixed.md)
  — why macros are dispatched by binding (not a call-site heuristic), a minimal two-phase (runtime/compile-time)
  model with expand-to-fixpoint before type-checking, and the deliberate exclusion of reader macros (syntax grows
  at the Ast level, keeping the reader out of the trusted path) — a principled contrast with the LISP inspiration.
- [A rejection carries a verified route to a compliant program](./2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program.md)
  — why a diagnostic must carry not just a reason but a machine-applicable fix (a structural AST edit), verified
  by apply-and-recompile where the repair is determinable (capability delta, match arms, conversions) and marked
  with an applicability marker where it is a guess — stronger than Rust's suggestions. Drives Constitution XI
  Amendment 0.5.0.
- [Diagnosis is complete and cascade-aware](./2026-07-04-diagnosis-is-complete-and-cascade-aware.md)
  — why the compiler must recover and report the maximal set of independent problems in one pass (not first-error),
  mark primary vs. derived so an agent fixes root causes, and expose a machine-branchable rejection/decline/trap
  kind so the agent routes around compiler limits instead of chasing fixes for them. Drives diagnostics.md + XI.
- [Type errors report the minimal conflict, both sites](./2026-07-04-type-errors-report-the-minimal-conflict.md)
  — why an HM type rejection must report the minimal unsatisfiable constraint set naming BOTH disagreeing
  locations (type-error slicing), not one blamed site — the bidirectional boundary decides the phrasing; showing
  both ends of the contradiction IS the fix. Drives type-system.md §Inference reporting discipline.
- [Program transformation is a program](./2026-07-04-program-transformation-is-a-program.md)
  — why refactoring is a Cadenza component over the AST (the same rep→rep seam as `compile`), the structural
  edit ops are a library of `Ast` functions, and text patching is never the mechanism; the tools that modify
  programs are themselves gated programs, generalizing the flywheel. Drives agent-authoring.md + structural-interface.
- [The compiler is a queryable oracle](./2026-07-04-the-compiler-is-a-queryable-oracle.md)
  — why an agent queries the compiler for any static fact (type of any node, name resolution, inferred manifest/
  effects, solved constraints) — total, deterministic, agreeing with a full compile — instead of instrumenting
  the program to learn it; generalizes the machine-readable-output + tooling-is-one-compiler reqs. Drives tooling-and-lsp.md.
- [Deterministic replay is the debugger](./2026-07-04-deterministic-replay-is-the-debugger.md)
  — why determinism (adopted for safety) buys lossless replay and fuel-indexed time-travel debugging for free
  (record only inputs + capability responses), so the agent observes runtime facts by replay not by inserting
  prints — and why the debug view is a tool-time projection NOT part of observable behavior. Drives tooling-and-lsp.md.
- [Capabilities attenuate: a handler forwards a narrower row](./2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row.md)
  — why a handler may grant a sub-computation FEWER capabilities than it holds (never more): object-capability
  attenuation realized as the effect-row-subset relationship handlers already track, making "no ambient authority"
  transitive; required by the target's cross-participant/tool-invocation model. Drives capabilities-and-effects.md.
- [The host interface IS the effect vocabulary](./2026-07-04-the-host-interface-is-the-effect-vocabulary.md)
  — why the abstract effect/capability labels are anchored to the four concrete frozen host operations
  (read-projection, emit-event, read-blob, invoke-tool): the manifest is the escaping effect row over that
  vocabulary, purity is the empty row, and the operation set is pinned once in options/execution-model/. Target-anchored.
- [Cadenza and its target share one seam](./2026-07-04-cadenza-and-the-target-share-one-seam.md)
  — why Cadenza is the source language + derivation tool for a specific target system (behavior-is-data over an
  event log), the derivation/host-interface/manifest touchpoints already correspond, and both must be ONE shared
  definition (not two that drift) with consistent governance floors across the seam. Drives the two frozen contracts + traceability.
- [Durable execution is effects + determinism](./2026-07-04-durable-execution-is-effects-plus-determinism.md)
  — why the target's suspend-record-resume-anywhere agent step (Temporal-style durable execution) falls out of
  algebraic effects (a boundary effect is a suspension point) + determinism (replay from recorded effect responses)
  + one-shot continuations (resume exactly once); demands a durable continuation capture only canonical-form data + manifest caps.
- [A fold module is provably pure; role bounds the effect row](./2026-07-04-fold-modules-are-provably-pure.md)
  — why a module's role fixes its mandatory effect profile (fold = empty row / pure; agent-step quarantines
  nondeterminism into a recorded reasoning tool call), and the compiler must REJECT a fold that reaches a forbidden
  effect AND emit a machine-readable purity certificate the activation review trusts. Target-anchored; drives capabilities-and-effects.md.
- [Fold order-independence is the verification layers' killer app](./2026-07-04-fold-order-independence-is-a-verified-property.md)
  — why the target's byte-identical-regardless-of-delivery-order fold rule (a CRDT-style commutative/latest-wins
  convergence property, stronger than purity) is the first load-bearing use of the optional verification layers:
  discharged by property testing (permutation invariance) / liquid refinement / proof, off the byte path. 
- [Open vocabulary needs open sums + schema-typed payloads](./2026-07-04-open-vocabulary-needs-open-sums-and-schema-typed-payloads.md)
  — why the target's open event-kind space (a fold is inert to unknown kinds) makes OPEN sum types (polymorphic
  variants, the sum dual the rows learning deferred) REQUIRED — exhaustiveness via a mandatory open-tail arm — and
  makes payloads schema-typed (bytes decoded against a run-time-resolved schema → typed Result). Ast stays a closed sum.
- [Host functions are un-named; the language binds any WIT-typed function](./2026-07-05-host-functions-are-un-named-the-language-binds-any-wit-function.md)
  — why the four concrete host ops (read-projection/emit-event/read-blob/invoke-tool) are a target leak removed from
  the language: the sole requirement is binding to WIT-typed host functions (complete signature), the vocabulary is the
  target's, the manifest is the escaping row, purity is the empty row, and the compiler imports nothing. host-interface-binding v2.
- [A host call suspends and resumes by replay from the host's log](./2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log.md)
  — why every host call is a mandatory suspension point resumed by Temporal-style replay: the program holds no resume
  state, the host owns the response log, the continuation is (component + input + log) canonical data resumable on any
  federated host, and resumption strategy (in-process / live / teardown) is the host's determinism-guaranteed choice. component-abi v2.
- [The seed stays Rust, not Lean](./2026-07-05-the-seed-stays-rust-not-lean.md)
  — why the seed's implementation language is orthogonal to Cadenza's verification aims (the seed is disposable, off the
  critical path, and independence comes from two compilers agreeing against the corpus, not a trusted verified seed);
  Rust's wasm/bytes/component ecosystem wins on fit; Lean is admissible only as an optional third oracle. Confirms the default.
- [Bool offers a total order, with false less than true](./2026-07-05-bool-offers-a-total-order.md)
  — why the conditional "ordering where offered is total" invariant needed a ground clause fixing which primitive types
  offer an order; an adversarial corpus case `(< true false)` had no definite outcome because Bool's ordering was never
  stated. Drove a sentence in core-semantics.md §"Ordering Where Offered Is Total" (Bool is totally ordered, false < true),
  witnessed by cases in the equality-and-observation corpus.
- [The value-heap runtime is a shared component](./2026-07-05-the-value-heap-runtime-is-a-shared-component.md)
  — why a program's runtime values (tuples, records, sums, …) do not live in each program's own component but in a single
  shared value-heap runtime the program imports and the host composes: the heap/reference-counting machinery is growing
  code better authored once and linked than open-coded per compound type, and because the runtime owns the storage behind
  an opaque handle its representation can evolve (Perceus RC, CHAMP/RRB) with no change to emitted programs. Drove
  component-abi.md v3 §"The Value-Heap Runtime" and the pin-by-content-address / build-pair rules.
- [The runtime is name-free; rendering is type-directed](./2026-07-05-the-runtime-is-name-free-rendering-is-type-directed.md)
  — why `render` was removed from the runtime: at run time a record is a positional product and a sum an integer tag, so
  the runtime holds no field or variant names and cannot render; rendering is type-directed code the compiler emits into
  the program, which walks the value through the runtime's accessors and returns an ordinary string. Refined
  component-abi.md v3 (§"The Runtime Does Not Name Or Render Values", §"A Compound Result Is Rendered By Compiler-Emitted
  Code").
- [Emitting a component that imports is a fixed envelope around a variable core module](./2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md)
  — the engineering technique for self-contained component emission with an import: bake a `wasm-tools`-validated
  reference as fixed HEAD/TAIL byte constants around a compiler-built core module (no compile-time tooling), and shift
  every defined-function index by a fixed base because imports occupy the low index space. Realizes
  reproducible-derivation.md §"Derivation Is A Function Of Source And Toolchain" and component-abi.md §"The Value-Heap
  Runtime Crosses By A Well-Known Import" in the emitter.
- [The runtime is tag-free; rendering walks a static shape, not a runtime tag](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md)
  — M2 Phase C removed the per-object type tag entirely: with no type erasure the compiler knows the static type at every
  use site, so a type-directed renderer walks a known `Shape` and never dispatches on a runtime tag. One positional array
  backs tuple/record/list; a sum keeps only a variant discriminant (runtime data, not a type tag). Deletes `tag-of` and the
  shared `mod tag`; the compiler-emitted renderer became per-program (one fn per distinct shape) rather than a fixed body.
  Pushes the name-free learning one level deeper (no type identity either). Gate 326→369, IGNITION byte-identical,
  COMPONENT-CHECK 412 agree.
- [A persistent collection fits the tagless heap with no new machinery](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md)
  — the first persistent collection (a persistent vector as a 32-way radix trie) was added to the value-heap runtime with
  **no new `Node` field, no new discriminant, and no new reference-counting code**: its interior/leaf nodes carry children
  in the existing tagless node's `handles`, so structural sharing is just a refcount above one, the existing iterative free
  cascade reclaims a whole trie, and it renders exactly like a list. Confirms the tag-free learning's explicit prediction
  that "CHAMP/RRB collections" would land with zero emitted-byte impact, and realizes (rather than adds) the memory model's
  #Sharing Is Not Observable and #Retained Storage requirements. The seam grows only by appending operations that name
  *what* a collection does, never *how* it is stored — so a later CHAMP map or RRB tree is the same cheap move.
- [A Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)
  — a Bytes value became a rope of shared concat/slice nodes over byte leaves, so concat/slice are O(1) and copy no bytes,
  killing the O(n²) copy cascade the self-hosting compiler hits concatenating module sections. Same tagless trick as the
  persistent vector (no new `Node` field; leaf/slice/concat by child count), but its new mechanism — flatten a rope node to
  a leaf in place on first full read, so the emit loop stays O(n) not O(n²) — is licensed by the memory model's #Sharing Is
  Not Observable **deferred-materialization** clause: the flattened bytes are identical, so it changes representation, not
  value. The finding: **structural sharing and deferred materialization are one mechanism**, both authorized by the same
  "not observable" test, both consequences of immutability making representation invisible to meaning. A slice pins its
  whole parent (#Retained Storage); the compaction op is the release valve.
- [Wiring the Bytes rope exercised the frozen-envelope recipe a second time — and caught a compact that did nothing](./2026-07-06-wiring-the-rope-exercised-the-envelope-recipe-a-second-time-and-caught-a-no-op-compact.md)
  — exposing the rope to the language appended three runtime imports (envelope 29→32, the second re-derivation), and the
  recipe ran mechanically start-to-finish: split-and-compare reproduced the existing constants, the new envelope emitted a
  valid component *before* any logic change. Two compiler-side findings the append surfaced: (a) runtime `Bytes.compact` was
  a **no-op identity** — value-preserving but keeping the parent pinned, defeating its whole purpose; the const-fold path
  hid it because a value-equality oracle *cannot see* a resource-only operation. Route reset/reuse/compaction ops to a
  runtime that actually reclaims, or their correctness is untested. (b) A fallible runtime `Bytes.slice`/`Bytes.at` needed
  its result **shape** (Option-with-payload, for the type-directed renderer), not just its **kind** — a renderable value
  declined at the boundary until `shape_of` mapped it to the Option shape with the right payload. Native concat is now the
  O(n²)→O(n) unlock; +4 runtime corpus cases.
- [The compiler's envelope byte-blobs are generated from the runtime contract, not pasted from ephemeral scripts](./2026-07-06-the-envelope-blobs-are-generated-from-the-runtime-contract.md)
  — the fixed component-model envelope (HEAD/TAIL, import section, indices, core signatures, count) had been hand-derived by
  a throwaway `/tmp` WAT+`wasm-tools` script and pasted as opaque arrays, and the interface it encodes was duplicated across
  SEVEN copies (the runtime WIT + six in the compiler/host) that had to agree by hand. Now the runtime **WIT is the single
  source of truth**: a Rust generator in `xtask` parses it (`wit-parser`), builds each reference component (`wasm-encoder`),
  self-validates (`wasmparser`), splits at the core-module boundary, and emits the compiler's + host's Rust source — folded
  into the one `build` command, write-if-changed to preserve the incremental cache. Generic over the contract; the compiler
  supplies only an ordered allow-list of the ops it lowers. The principle: **a derived artifact whose derivation is an
  ephemeral script is a latent defect even when its bytes are correct** — correctness you cannot re-derive is unmaintainable,
  so make the derivation a checked-in, re-runnable, self-validating program with one source of truth. Retires the manual
  recipe of the two prior envelope re-derivation learnings; also baked the runtime-hash pin into the generated source
  (killing a dead env var). `wasm-encoder` stays out-of-band (never a shipped-compiler dep).
- [The leaf fast path derefs twice — the rope taxes the program that never ropes](./2026-07-06-the-leaf-fast-path-derefs-twice-so-the-rope-taxes-the-program-that-never-ropes.md)
  — a cost review asked what a pure-leaf program (build a flat buffer, read it back, never concat/slice) pays for the rope
  existing. Answer: almost nothing, except `bytes-get`'s leaf path now dereferences the node **twice** — once to classify
  leaf-vs-rope (dropping the borrow so the rope case can take a mutable borrow to flatten), then again to read the byte —
  where pre-rope code derefed once. That ~2× per-byte constant lands on the compiler's hottest loop (`for i in 0..len`
  reading an assembled module out), still O(n) but needlessly. The mechanism for the deferred case (flatten needs to release
  the shared borrow) seeped into the common case; fold classify+read into one borrow so the leaf path is one deref again. No
  ABI/spec impact. The general trap: an **implicit** representation optimization tends to tax the value that never uses it
  unless the common-case accessor is deliberately carved back out — and a value oracle is blind to the cost, like the no-op
  `compact` beside it.
- [A keyed collection needs no serialization seam — tag-free structural hashing and comparison suffice](./2026-07-06-a-keyed-collection-needs-no-serialization-seam-structural-comparison-is-tag-free.md)
  — designing the persistent map (a CHAMP) and set raised the first hard tag-free question: a hash trie must **hash**
  and **compare** keys, but the runtime holds no type identity. The resolution rejects both a per-operation
  canonical-bytes serialization seam (allocates on every insert/lookup) and an equality-function upcall (reentrancy
  across the boundary) in favor of the runtime **hashing and comparing keys by a direct structural walk of the node
  graph** — keys cross as plain handles, nothing is serialized, no upcall. Correct because (1) keys within a map are
  homogeneous (cross-type is a compile-time rejection), so the node-level ambiguity a tag would resolve is harmless —
  both operands share whatever type the bytes are; and (2) every value form is canonical **except the Bytes rope**
  (compacted before use as a key). The finding: **structural equality and hashing need no run-time type system,
  because a canonical representation already encodes the identity a tag would carry** — the same
  immutability-makes-representation-invisible argument as the rope. Names the tripwire: a future non-canonical RRB
  vector used as a key would break structural compare unless normalized on use.
- [An iterator over an immutable heap is a stateless cursor — in-place when unique, forkable when shared](./2026-07-06-an-iterator-over-an-immutable-heap-is-a-stateless-cursor-that-is-in-place-when-unique.md)
  — the map's iteration surface replaced a materialized entries array (an O(n) allocation per traversal that defeats
  fusion) with a **cursor**: a bounded descent-stack read through head/tail projections (`iter-key`/`iter-val` +
  `iter-next`), the ML lazy-cons-stream (OCaml `Seq`) / stream-fusion source shape fitted to single-handle ops. The
  cursor is **stateless** (`iter-next` returns a new cursor) yet costs nothing, because at reference-count one the
  reuse-when-unique discipline makes advance an in-place refit — physically a mutation, semantically a new value — so
  the loop is as cheap as a mutable pointer; and when shared (the caller `dup`'d it), advance path-copies, giving
  forkable iterators (peekable/tee/backtracking) **for free by the identical `count==1 ? reuse : copy` rule** every
  persistent structure turns on. The reconciliation of stateful iteration with an immutable heap: **a cursor is not a
  value** — linear, ephemeral, borrowing — so in-place-when-unique is the frame-limited-reuse identity, not a mutation.
  Dispatch is static (`impl Iterator`, not `dyn`); the uniform non-allocating pull protocol is where stream fusion
  becomes possible in the language.
  — a three-front gap analysis (seed-compiler-as-a-program inventory, spec survey, realized-capability audit) for
  authoring the Cadenza compiler in Cadenza. The self-host target is the pure `bytes → bytes` core
  (`codegen.rs`+`ast.rs`+`diagnostics.rs`, ~7,300 lines), not the workspace; the seam is a pure function, so
  effects/handlers and the text reader are OFF the critical path (the core consumes CBOR AST). Gaps sort into four
  tiers: **Tier 1 language-level** (generics + full HM inference — the linchpin everything else waits on; deterministic
  ordered maps; closure edge-declines; a growable-buffer story); **Tier 2 library-level, authored in Cadenza** (CBOR
  codec, Unicode NFC, int→string/formatting, float-bits, baked envelope bytes) — the substance of the M8 re-authoring;
  **Tier 3 seed scale defects** (no TCO/bounded stack, 2ⁿ nesting) that bite a 7,300-line self-compile; **Tier 4 defer**
  (multi-module, traits, symbol interning, macros, `bin`, units, verification, width-indexed ints). Confirms the
  operator M0–M9 ladder and names its single gate: **M3 (static types + rows) is where generics + HM inference lands,
  and nothing polymorphic — containers, width-indexed ints, the port itself — moves until it does.**
- [A recursive consumer of a runtime heap value must be typed Heap, or the compiler diverges](./2026-07-05-a-recursive-consumer-of-a-runtime-value-must-be-typed-heap.md)
  — the seed chooses inline-vs-`call` for a callee by the argument's inferred kind; a recursive function consuming a
  runtime heap value whose parameter is *under-constrained* defaults to a scalar kind, which selects the inline path,
  so the expansion never bottoms out and **the compiler itself diverges** (stack overflow / hang). Hit twice on one
  cause (runtime sum-match `sm`; runtime bytes `sumb`); the fix is identical and lives in inference — a heap-value
  CONSUMER (constructor-pattern match arm; `Bytes.len`/`at`/`slice`) must constrain its operand to the heap kind so a
  recursive caller emits a runtime `call`, not an inline. Sharpens decline-don't-miscompile: **a non-terminating
  compilation is a miscompile, not a decline.**
- [The compiler's own I/O type must be a first-class runtime value, and the frozen import set decides which](./2026-07-05-the-compilers-io-type-must-be-a-first-class-runtime-value.md)
  — the compile seam is `list<u8> → result<list<u8>>`, yet `Bytes` existed only as a compile-time constant; the
  compiler's own input/output type was unavailable as a runtime value. Runtime Bytes (construct/len/concat/recursive-
  builder) landed on the value-heap runtime. Two invariants: **(1)** among a compiler's needed runtime values, realize
  first the one the FROZEN emission envelope already imports (Bytes' ops were reserved in it → no re-derivation; String's
  were not → deferred) — the import set, not feature difficulty, sets the order; **(2)** when a runtime primitive
  truncates a value the language bounds more tightly (`bytes-set` does `value as u8`), the compiler emits the range check
  on the language value BEFORE the primitive, so a partial operation's trap isn't silently swallowed. Input consumption
  (index a runtime buffer + match its `Option`) remains blocked on runtime polymorphic-payload sum-match.
- [Wiring the persistent vector re-derived the frozen envelope for the first time — and forced a fixpoint fix](./2026-07-05-wiring-the-persistent-vector-re-derived-the-frozen-envelope.md)
  — exposing the runtime's persistent vector (`Vec.empty`/`push`/`update`/`len`/`get`) to the language was the FIRST
  actual extension of the frozen component-emission envelope: five new lowered imports meant re-deriving HEAD (1200→1440),
  TAIL (344→400, run/realloc core-func aliases shifting 24/25→29/30), and the import section (24→29). Done the prescribed
  dev-desk way (author extended WAT → `wasm-tools` validate → split at the core-module boundary → re-bake constants), with
  a split-and-compare check that reproduced the EXISTING constants before trusting the new ones. Confirms the fixed-envelope
  learning's "append-only, one-time re-derivation" claim (IGNITION byte-identity + COMPONENT-CHECK survived the bump). ALSO
  fixed a fixpoint hole a recursive vector BUILDER exposed: `if` inference now prefers `Kind::Heap` when its branches
  disagree, so a recursive compound builder's return kind converges to a heap value instead of locking to the Int64 default
  — the builder-side dual of the recursive-consumer-must-be-Heap rule (an under-determined kind at a heap boundary resolves
  toward the heap).
- [Authoring the compiler in Cadenza surfaces the language's real gaps](./2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps.md)
  — why the compiler's vertical slice was authored in aspirational Cadenza (as if every capability were
  realized, with inline `DECLINE` markers): the marker frequency is a prioritization signal (effects ≫
  numeric-model > sum-types), and where the language fights the author is a design signal. Drove three
  sibling learnings and a family of compiler-idiom corpus cases; the spike lives in the disposable
  `implementation/` tree, so its durable output is the learnings and cases, not the source.
- [Effects are declared with one surface; a host-bound declaration is the grant](./2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant.md)
  — why an effect is declared `(effect Name (op … (-> T… R))…)` (a record of operations reached as
  `Name.op`), a host import is the same form with a `(host)` marker, and that host-bound declaration IS
  the manifest grant — removing both `(import (host …))` and `(use (capability …))` so there is one way to
  declare, not several. Adds `CDZ0402` (undischarged effect) and `CDZ0403` (handler arm names an
  undeclared op). Drives capabilities-and-effects.md + options/effects-model + code-shape + corpus.
- [Dynamic-extent context is an effect; lexical-extent data is a parameter](./2026-07-05-dynamic-extent-is-an-effect-lexical-extent-is-a-parameter.md)
  — why "effects for everything" is right for diagnostics / fresh-supply / the unification store (dynamic
  extent — alive until a handler returns) but wrong for the lexical environment (lexical extent — a
  threaded immutable map, since argument-passing IS lexical scoping); a handler forced to snapshot/restore
  to fake nesting is the tell the effect is the wrong tool. Refines the mutation-as-State resolution.
- [The compiler's internal IR is a typed sum; the public AST stays homoiconic](./2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic.md)
  — why the instruction backend uses a typed `Instr` sum (exhaustive serializer ⇒ a new opcode is a
  compile error, extending decline-don't-miscompile to codegen and enabling structural const-fold/peephole)
  while the frontend stays homoiconic `Ast` + quasiquote where values truly are syntax; and why `Symbol`
  interning lives in the internal term (at the `Ast → term` boundary), not the quotable `Ast`.
  Ratified 2026-07-05 into compiler-pipeline.md §Representation (typed instruction sum + exhaustive
  serializer; quasiquote re-scoped to the frontend/macro layer).
- [Lower through a resolved IR so emission is a serializer, not a construction site](./2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer.md)
  — why the *middle* rung (a resolved, analyzed representation) must exist, not just the backend
  instruction sum: a single AST→bytes pass that also type-checks, folds, inlines, and resolves effect
  handlers as it emits has no seam to add a feature or an optimization at, makes the compiler exponential
  in nesting (resolution + env redone per branch), and hides a miscompile where the discharging handler
  is decided by state the emitter accumulates (the effects under-frame). Drives compiler-pipeline.md
  §Representation §"The Compiler Resolves Names Before It Selects Instructions" and §"Emission Serializes
  A Lowered Representation" — name resolution / type-checking / folding / effect lowering are
  transformations of the IR, and byte emission resolves nothing. Requirements state the obligations, not
  the pass ladder.
- [Optimizing-compiler techniques for a functional/immutable IR — a grounded catalog](./2026-07-06-optimizing-compiler-techniques-for-a-functional-immutable-ir.md)
  — forward-looking design input (drives NO requirement) for the deferred IR-layer question: a
  fact-checked catalog of prior art grounded in Cadenza's constraints. Converges on direct-style
  ANF + explicit join points (not CPS; recovers CPS's power without its fixed evaluation order),
  nanopass single-task passes over checkable per-rung grammars, and MLIR-style progressive lowering
  (don't lower too far too early). Confirms Cadenza's two headline optimizations are Core→Core passes:
  Perceus RC/reuse/FBIP (acyclicity is the precision precondition; needs ownership/borrow annotations)
  and evidence-passing + tail-resumptive effect lowering to stock wasm (Tier-3 reified continuations are
  the one thing needing a lower/CFG-shaped rung). Honest gaps: pass-correctness apparatus and several
  classic functional opts (fusion, worker/wrapper, let-floating, CHAMP/RRB × reuse) unassessed. Six
  primary sources (CwC PLDI'17, nanopass ICFP'13, MLIR, Perceus PLDI'21, Immutable Beans, Generalized
  Evidence Passing ICFP'21).
- [Record and tuple reshaping is explicit row operations](./2026-07-05-record-and-tuple-reshaping-is-explicit-row-operations.md)
  — why the requested "pop a field / add a field / merge / split" operations are the explicit
  `project`/narrowing the rows learning promised but never pinned: three record primitives
  (`Record.project`/`without`/`merge`) plus derived `extend`/`with`/`pop`, and positional tuple
  analogues (`Tuple.cat`/`split-at`/`pop`). Each yields a NEW closed value (no mutation), is shaped
  statically, and stays a special form (field names/positions are static, not runtime values). `merge`
  is strict-unbiased (shared field → `CDZ0211`, no silent clobber); `extend` (absent) and `with`
  (present, may retype) are deliberately distinct; `pop` is row-typed not `Option` (field presence is
  static). Drives type-system.md §The Declarable Type Universe (7 subsections), `CDZ0211`/`CDZ0212`,
  and `options/record-tuple-operations/`.
- [Fuel is host-owned runtime policy, not a compiler-emitted measure](./2026-07-06-fuel-is-host-owned-runtime-policy-not-a-compiler-emitted-measure.md)
  — why resource exhaustion is delegated entirely to the execution environment (wasmtime `consume_fuel`
  instruments emitted wasm at JIT time; async-fiber yield refuels/yields/aborts on the same stack with no
  recompute) rather than modeled as a compiler-emitted measure or a boundary effect: fuel can run out at
  any loop back-edge, so an effect framing makes the whole program a fine-grained state machine, and a
  program that could read its remaining fuel would make the host's grant schedule observable. The program
  is fuel-blind by mandate; a completing run is byte-identical regardless of budget, and abort is a
  resource terminal like OOM, outside observable behavior. The compiler emits nothing; Core Principle V's
  obligation relocates from the compiler to "the execution environment MUST be interruptible at a bounded
  point." Drives constitution V, determinism-and-fuel.md §Resource Accounting, core-semantics.md, glossary.
- [Compiling effect handlers: classify first, and the tail-resumptive common case is plain code](./2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code.md)
  — why intra-program `handle`/perform/`resume` lowers to wasm by a classification-first strategy: a
  compile-time pass sorts each handler arm into tail-resumptive / abortive / general-one-shot and lowers each
  to a minimal stock-wasm shape. Cadenza's lexical+static resolution over a monomorphized closed row collapses
  Koka's runtime evidence vector to a direct arm reference, and because EVERY corpus arm and the compiler's own
  `Fresh`/`Diag`/`Unify` are tail-resumptive, the whole shipping surface needs ZERO continuation machinery
  (perform = direct call, tail `resume e` = `e`). The general-one-shot fallback is a defunctionalized frame on
  the FROZEN value-heap prefix (envelope-neutral, no new WIT op). Rejects native stack-switching (not in any
  Wasmtime tier, >4× slower than Asyncify, opaque native stack can't cross the boundary as data). A reified
  intra-program continuation must not span a host suspension point (the invariant reconciling durable-data vs
  non-durable-handle). Ten SOTA lanes adversarially fact-checked; caught fuel-retired (Amendment 0.7.0) as a
  stale premise. Drives options/effects-model/lowering-to-wasm.md; Stage 1 clears the #1 self-host blocker.
- [Implementing effects in the seed: inlining resolves cross-function effects — until recursion](./2026-07-06-implementing-effects-in-the-seed-inlining-resolves-cross-function-until-recursion.md)
  — executing the classify-first design in the seed (Stages 0–2 landed, gate 466 pass/0 fail). Three findings:
  (1) cross-function effect resolution reduces to INLINING an effectful callee into the handled region (the
  existing lambda-arg alias path), turning all six cross-function corpus cases green with no new mechanism — but
  a RECURSIVE effectful function inlines its own body without bound, so it DECLINES cleanly (an `inlining`-stack
  guard), the precise Stage-3/monomorphization boundary; the non-recursive cases pass without the guard, so the
  wall is invisible until you write the recursive-effect test (two new corpus cases pin it). (2) A delegated
  operation `log.emit` cannot be a top-level component IMPORT NAME (the model requires kebab-case externs), which
  forced the design's effect=WIT-interface / op=function-in-it encoding — the dot lives only in the recorded
  call name. (3) A new tail value-form must be taught to `emit`, `infer_list` (so the return kind flows and
  `call_base` is right), AND `shape_of_list` (so a compound result drives the runtime-compound renderer) — miss
  one and the paths disagree into an invalid component, not a decline. State threading is a mutable wasm local
  whose handed-back value reads the OLD state; unit-state is the zero-cost degenerate case.
- [Authoring the compiler in Cadenza surfaces gaps a corpus grown from a floor misses](./2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses.md)
  — (re-)authoring the compiler in Cadenza — a resolved `Core` → `Lir` typed-instruction-sum → bytes ladder
  whose emission is a pure serializer — reached a working vertical slice (it compiled `(+ 20 22)` to a valid
  component that runs to 42, via `i64.const 20 · i64.const 22 · i64.add`). Getting there surfaced four
  seed/spec gaps that isolated conformance cases never had: (1) compile-time inlining was EXPONENTIAL — a
  recursive value function threading a compound accumulator had that parameter's kind inferred as a scalar, so
  a heap argument met a scalar parameter and the recursive call inlined without bound (>30 GB, OS-killed);
  fixed in kind inference (back-propagate a `match`'s result kind to arms returning a parameter; let the
  more-defined heap kind win an order-dependent constraint race) and pinned by a new corpus case. (2) NO boolean
  connectives (own learning). (3) runtime `String` is unrealized (walls off name dispatch + the reader's symbol
  table — the keystone remaining self-host blocker). (4) a `match` arm returning a heap value bound by its
  pattern through a helper could emit an invalid component (fixed to compile). The methodological lesson: a
  corpus grown outward from a mandatory floor is structurally blind to the STAPLES and INTERACTIONS a real
  program composes; authoring the second compiler is the most demanding conformance test the language has, and
  it earns its keep as a gap-finder long before it is self-hosting.
- [The front rung of a resolved-IR compiler needs nested payload binders — and folding early leaves cdz-rustc's dead code behind](./2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md)
  — the sequel to the gap-finder above: with the exponential-inlining and heap-sub-node fixes landed, the whole
  pipeline is now authored (`resolve → fold → lower → serialize → frame`) and every rung compiles when fed
  `Core` directly, but the FRONT rung `resolve` declines on ONE gap — a nested tuple binder inside a sum payload
  (`((Node.NPrim (tuple op (tuple a b))) …)`, the exact node a resolved-IR front rung takes). Flat and flat-3
  payload binders work; only the recursion into a compound slot is missing (`bind_sum_payload` must recurse), and
  there is no in-language workaround (a bare runtime-tuple match arm also declines). Pinned by a new
  `05-compound-types.sexp` case (→ 34, scores todo). Second finding, a point FOR the resolved-IR architecture:
  the Cadenza compiler folds `(+ 20 22)` to `KConst 42` at the Core layer BEFORE emission, so its 89-byte
  component has no dead code — while cdz-rustc emits 128 bytes because it folds shallowly and leaves a dead
  overflow-check helper. The two agree on result + `run`'s body but not bytes; byte-identity awaits DCE in
  cdz-rustc (a separable Core→Core concern), which reframes the byte-identity target as a named milestone.
- [A no-scratch-local Lir must decline the ops that need guard locals — shifts are the honest decline, not a miscompile](./2026-07-07-a-no-scratch-local-lir-must-decline-ops-that-need-guard-locals.md)
  — the compiler reached `<< >>` and deliberately DECLINES them (KError→unreachable). Reason: wasm's shl/shr mask
  the count mod 64 and never trap, so an unguarded emit MISCOMPILES (shift-by-64 → silently shift-by-0); faithful
  lowering needs a count-range trap guard + overflow guard, both needing SCRATCH LOCALS — but compiler.cdz's Lir
  is a pure Core→Code fold with NO local allocation. So faithful shifts are an ARCHITECTURAL step (a
  local-allocating lower pass), not a one-line binop; declining is the only honest option a fold-only backend has
  (bare emit = miscompile). The CORRECT face of reject-don't-miscompile (vs the reader's atom-decode leak, #23):
  the backend recognizes it can't faithfully emit and declines. General: a backend IR's shape (fold-only vs
  local-allocating) BOUNDS which operators it can faithfully emit; guarded ops (shifts, checked-arith-with-a-held-
  operand, bin-fit-checks) are declined until the IR grows locals, and that's coverage-gap-in-the-pass, not
  in the operators. No corpus case (seed's shift-trap already pinned; this is a compiler.cdz emit-path choice).
  Notes SPEC-BACKLOG #20: faithful guarded-op emit is gated on a local-allocating lower pass.
- [The self-hosted reader miscompiles unsupported constructs instead of declining — and correcting my own wrong call](./2026-07-07-the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining.md)
  — the harness's `0 mine-declines` is the ALARMING signal, not noise: `compiler.cdz` NEVER declines an
  unsupported construct — it emits a valid-but-WRONG component. Verified: a CBOR float `0xfb` (major 7, info 27)
  hits `read-node`'s major-7 branch which assumes bool (`arg==21`?), so `arg 27 ≠ 21` → `NBool 0` → the program
  returns `false`; strings/records fall through to `NInt`/`"?"` stubs. A reject-don't-miscompile violation INSIDE
  the Cadenza-authored compiler's reader. CORRECTS my prior-cycle call (I said the disagrees were a byte-patching
  artifact — WRONG; the bytes reach the decode path and are miscompiled). I made the very error the spike keeps
  teaching against — reasoned from a proxy (size clustering) instead of probing the actual behavior; a suspicious
  aggregate (0 declines) deserves a direct probe before an explanation. Fix (SPEC-BACKLOG #23): route the reader's
  unrecognized atom kinds to KError→unreachable, mirroring the PUnknown head path; `mine-declines` rising from 0
  is the acceptance signal. Pinned `10-bytes.sexp` "a CBOR simple value that is not a known boolean is classified
  as not-a-boolean" (three-way classify: arg 20→false, 21→true, else→not-a-bool; → -90).
- [Verifying the self-hosted compiler needs a `compile`-exporting component — the interim byte-patching harness mis-measures](./2026-07-07-verifying-the-self-hosted-compiler-needs-a-compile-exporting-component.md)
  — the spike wants to run compiler.cdz over the WHOLE corpus (feed each case's AST bytes, diff its output vs
  native cdz-rustc). The host has this (`component-check` + the `compiler.wit` `compile: list<u8> →
  result<list<u8>,…>` world), but it's BLOCKED by seed gap 3l: the seed emits only nullary `run : () → output`,
  so a `main` that IS `compile : Bytes → Bytes` declines "must take no parameters". Workaround: an interim
  harness (`run_corpus.py`) that byte-PATCHES each case into compiler.cdz's main. But it MIS-MEASURES: ~147
  "disagree", 0 "mine-declines" (contradicting its own docstring that declines are expected), counts DRIFT
  between runs, and "mine" sizes cluster at 88–102B while native ranges 89–3332B → the patched bytes mostly
  don't reach the decode path; it's classifying a degenerate stub as disagreement. Modeled-subsystem trap: a
  workaround routing around the real ABI reports numbers that measure the workaround. Trust only its AGREE set;
  DON'T chase the 147. Fix = SPEC-BACKLOG #22 (gap 3l: emit a `compile`-exporting entry). No corpus case
  (infra, and a mismeasured table isn't an oracle).
- [Over-applying a user function declines as "partial application needs closures" — not the arity error the corpus says it mirrors](./2026-07-07-over-applying-a-user-function-declines-as-closures-not-as-an-arity-error.md)
  — surfaced by a mid-refactor compiler.cdz (a `kind-of` call/def arity mismatch): `(f 5 9)` on a unary `f`
  declines "partial application needs closures", but the corpus records the PARALLEL constructor case `(Some 1 2)`
  as `(error CDZ0201)` (apply-a-non-function) and its prose says user-fn over-application is "arity-checked the
  same way" — yet only the constructor case is pinned, and the seed treats the user-fn case differently (a
  closure-feature gap, not a type error). Should be CDZ0201 (same `((f a) b)` desugaring). NO corpus case: pinning
  `(f 5 9) → CDZ0201` FAILed the gate via a CROSS-CASE interaction — it flipped an unrelated passing case
  (`(let ((ctor None)) (ctor unit))`) to a wrong "CDZ0401 undeclared capability: ctor", exposing that
  head-position name classification (value / constructor / capability / over-applied-fn) is fragile and
  order-sensitive. SPEC-BACKLOG #21 (fix = emit CDZ0201 AND make head-position classification total). The spike's
  trigger was transient WIP, not a compiler regression.
- [Runtime bitwise `&`/`|` are emitted — the compiler's own LEB128 encoder now runs on runtime values](./2026-07-07-runtime-bitwise-ops-emitted-the-leb128-encoder-runs-on-runtime-values.md)
  — subset-growth (#20 operator coverage): emit-side Core gained `KBitAnd`/`KBitOr`, so runtime `&`/`|` (value
  through a parameter, not a constant) now emit. Verified `(& n 127)` on 200 → 72, and the composed LEB128 byte
  `(Int.to-byte (| (& n 127) 128))` on runtime 300 → 172. Matters because the compiler's OWN LEB128 encoder runs
  these ops on runtime values (every section length/operand), so self-emission needs them emitted not just folded.
  Recurring const-masks-the-runtime-gap trap AGAIN: the corpus had `&`/`|` only on CONSTANT operands (fold, never
  exercise the emitted i64.and/or). Rule holds: a const case for an operator isn't evidence it emits — the
  runtime-through-a-parameter case is; the LEB128 encoder is the sharpest witness. Pinned `06-numeric-model.sexp`
  "the LEB128 byte composition runs on a runtime operand" (→172). Frontier still `match` on user sums + TCO.
- [`match` on user sums is the last major emit frontier — self-hosting is now an emit-coverage checklist, not a blocker](./2026-07-07-match-on-user-sums-is-the-last-major-emit-frontier.md)
  — a bookkeeping cycle (spike only updated docs) prompted taking stock: the compiler's OWN source uses ~41
  `match` over 11 user sum types, ~19 String ops, pervasive recursion — but its emit-side `Core` has NO `KMatch`,
  no user-sum declaration/construction. Each is a SUBSET-FRONTIER item, not a seed gap (the seed compiles user-sum
  `match` fine → 31; it's the Cadenza compiler's EMIT path that must grow). Self-hosting is now a countable
  emit-coverage CHECKLIST, not an open blocker: emit `match` on user sums + construction (THE big one — every pass
  is a match over a user sum, so it's not one feature but THE feature the compiler is written in) → string/bytes
  comparison on the emit path → TCO for deep recursion. No unknown blocker left, only known coverage to fill. It's
  the emit-side dual of the reader's node-dispatch (decode a tagged node ↔ produce code for one). No new corpus
  case (behavior already pinned as a SEED capability; compiler-emit not yet a shape to pin). SPEC-BACKLOG #20 (the
  self-inclusion inventory, priority-ordered).
- [N-ary calls wired end-to-end — the arg-list round-trip became the feature, as pure wiring](./2026-07-07-n-ary-calls-wired-end-to-end-the-round-trip-becomes-the-feature.md)
  — with both arg-list halves fixed (read #17, build #18), the spike wired N-ARY user-function calls through the
  whole pipeline — the "pure wiring" the prior cycle predicted. `NCall` → `(Tuple Int64 (list Node))`; `read-call`
  reads any arity via a push-loop (`read-call-args`); `resolve` maps over the list; `lower` pushes args L-to-R
  before `call`. Verified `(add2 20 22)` → 42, `(add3 10 20 12)` → 42. Closes the multi-arg-call arc. Payoff of
  decomposing a capability into independently-failing directions: "handle multi-arg calls" was never one fix — it
  was read-the-list + build-the-list, two instances of the runtime-value-kind family cycles apart; once both held,
  the feature was composition not invention (composition thesis at FEATURE granularity). Corollary: the round-trip
  case certifies the feature is REACHABLE, the feature case certifies it was REACHED. Pinned `09-functions.sexp`
  "a named multi-argument function applies to all its arguments at once" (`(add2 20 22)` → 42, direct vs curried).
- [The argument-list round-trip works — build by push-recursion, read by indexed iteration](./2026-07-07-the-arg-list-round-trip-works-build-by-push-read-by-index.md)
  — seed fixed Tier 3i / #18: a recursive push-accumulator now infers a list return (my todo case flipped PASS).
  With it, both halves of a multi-arg call's argument handling work together: BUILD (push-loop accumulates
  operands, item 18) + READ (index the built list, item 17). Verified end-to-end: build [0 1 2] by push-recursion,
  sum by List.at iteration → 3. Names a matched pair: an arg list is a list a compiler both builds and reads, and
  each direction was blocked by a different instance of the same "value must carry its list kind" family — now
  both fixed, round-trip closes. Lesson: a singular-looking capability ("multi-arg calls") decomposes into build
  + read sides that fail independently; pin the ROUND-TRIP (build then read in one program) to check they compose.
  Pinned `05-compound-types.sexp` "a list built by a recursive push-loop is then iterated by index" (→3). #18
  RESOLVED. ⚠compiler.cdz `read-call` still only handles UNARY calls with a now-STALE "blocked" comment — multi-arg
  is pure wiring now, not a blocker.
- [A recursive push-accumulator loses its list return kind — the Tier-00 race again, now blocking the arg-list reader](./2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind.md)
  — with payload-bound List.at fixed (arg list READABLE), the gap moved to BUILDING it: a recursive fn threading
  a `list` accumulator grown by `List.push` has its return kind collapse to non-list (`List.len` → "of a non-list
  value"). Boundary = exactly {recursive ∧ list-accumulator ∧ push}; drop any one and it works. THE blocker for
  multi-arg calls (the reader's `(read-args … (List.push out (read-node …)))` loop). FIFTH instance of arc-pattern
  #1 (order/position-independent recursive-result inference): `acc` returned in the base arm seeds scalar,
  `List.push acc n`'s list result must UPGRADE it not be collapsed — same cause + fix as the Tier-00
  threaded-accumulator race, now on a `list` return. Distinct from the passing recursive-builder (push as FIRST
  arg, forced positionally). Pinned `05-compound-types.sexp` "a recursive list accumulator grown by push and
  returned in the base arm stays a list" (→3, todo). SPEC-BACKLOG #18 (+ #19 = the secondary 3j nested-ctor-under-
  Some-on-a-param-list gap, has a two-step workaround).
- [The reader tells a call from an operator by function-environment membership — two namespaces, one lookup](./2026-07-07-the-reader-tells-a-call-from-an-operator-by-function-environment-membership.md)
  — `read-app` grew to distinguish a user-function CALL from a primitive OPERATOR: it carries a function
  environment (`fenv` = the module's `def` names' prelude indices), looks the head index up in it — present → a
  call to that function slot (`read-call`), absent → a binary operator (`read-op-name`, now the full surface
  `+ - * / % < > = <= >= != and or not`). Key design fact: a canonical AST head is untyped as to what it names,
  and scope resolution and call-vs-operator resolution are the SAME operation — ordered-index-environment
  membership (`ienv-pos`) — applied to different environments (param env vs function env). Clean because the
  namespaces don't overlap (operators never appear in `fenv`, so lookup-fails-means-operator is total, not a
  heuristic). Lets a multi-def module's functions call each other. NO new corpus case (reader-internal step over
  pre-parsed bytes; mechanism already pinned by the shadowing case, behaviors already covered) — a
  reader-internal completeness step realizing already-witnessed behaviors earns a learning, not a duplicate case.
- [The self-hosting arc — what a language hits growing to compile its own compiler, and the four patterns that recurred](./2026-07-07-the-self-hosting-arc-what-a-language-hits-growing-to-compile-itself.md)
  — SYNTHESIS of the ~25 dated-07-07 spike learnings: the one place to start understanding the whole self-hosting
  push (backend exists → language holes → front rung → reader → runtime-value plumbing), with every stage linked.
  Drives no requirement; it's the map. The FOUR recurring patterns (reusable diagnostics for the work ahead): (1)
  order/position-independent inference — a self-call placeholder pinned by a concrete sibling, same race on Heap /
  Bool / compound-shape; (2) payload-bound = runtime, and const-folding hides the runtime gap — reduce the FAILING
  program, a clean analogue that folds isn't evidence; (3) input/output are duals over one small byte vocabulary —
  no separate reader runtime; (4) write it honestly — the contortion (Bytes-hack trap, concat-anchor) is often the
  bug, and handoff docs lag the seed so probe the rebuilt binary. Self-host architecture complete + gate-witnessed
  (module bytes → component); remaining = subset growth (emit `match` on user sums) + scale (TCO).
- [A recursive cons-list→Bytes fold now infers its shape as the direct result — the serialize spine, no concat anchor needed](./2026-07-07-a-recursive-bytes-fold-infers-its-shape-as-the-direct-result.md)
  — seed fixed Tier 3d: a recursive fold of a cons-list of byte fragments to Bytes (`(match xs ((Nil) empty)
  ((Cons (tuple h t)) (Bytes.concat h (rec t))))`) now compiles as `main`'s DIRECT result (verified `cat-all
  (build 3)` → `b"CBA"`), where it previously declined "cannot infer runtime compound result shape" unless
  anchored by a literal concat operand. The compiler's SERIALIZE spine (fold a code stream into the output byte
  vector). Same family as the recursive-Bool and Tier-00 Heap races: a self-call's shape is a placeholder during
  the function's own inference; the fix lets a concrete sibling (the `Bytes.of (list)` base arm) pin the result
  regardless of position — now extended to the compound-SHAPE axis (was kind). The concat-anchor workaround was
  the same contortion as the Bytes-hack; removing the need is the honest fix. Pinned `10-bytes.sexp` "a recursive
  fold of a cons-list to bytes is the whole program result" (→ b"CBA"). NOTE: distinct from the still-declining
  recursive-sum-VALUE render (unbounded shape); a Bytes fold has a determinate result.
- [The reader resolves names to local slots — lexical shadowing is deepest-position-wins in an index environment](./2026-07-07-the-reader-resolves-names-to-local-slots-with-lexical-shadowing.md)
  — the reader grew SCOPE resolution: a bare name (CBOR tag 39 `d8 27 <idx>` wrapping a prelude index) resolves to
  a local SLOT via a parameter environment (in-scope names' prelude indices, in order); `let` extends the env
  (append-at-end → next slot). The "resolve names to bindings" step (companion to "resolve names to codes"), on
  runtime bytes. Load-bearing: shadowing = DEEPEST-position-wins — `ienv-pos` searches deepest-first, so env
  `[5,7,5]` looking up 5 → slot 2 (innermost), not 0. A FIRST-match search silently resolves a shadowing `let` to
  the shadowed outer slot — a valid-component-wrong-value scope bug. Append-at-end + search-deepest-first gives
  lexical scope with shadowing over a flat slot array, no nesting structure needed. Pinned `02-binding-and-control.sexp`
  "resolving a name in a shadowing environment returns the innermost binding's slot" (→2). Reader now decodes
  functions with params + `let`, not just closed constant bodies.
- [Payload-bound `List.at` fixed — multi-argument calls are now representable (and const-folding masked the real gap)](./2026-07-07-payload-bound-list-at-fixed-multi-arg-calls-are-representable.md)
  — the seed fixed Tier 3h / #17: `List.at` on a payload-bound list now reads its element (→10, my todo case
  flipped to PASS). Unblocks the natural N-ary call rep `KCall (Tuple Int64 (List Core))` — iterate the payload
  arg list via `List.len` + `List.at`, each arg a recursive sum consumed (verified `KCall(9,[10 20 12])` → 42).
  ⚠ I MIS-FRAMED the cause as "payload-shape mismatch"; the seed fix shows there was simply NO runtime `List.at`
  emitter — `List.at (list …) i` only ever "worked" by const-FOLDING the literal list; a payload-bound Heap
  handle can't fold, so that's where the mask came off. SAME trap as the scale-limit misdiagnosis: reasoning from
  a const-folding clean analogue hides a missing runtime path. RULE (proven twice): when a construct works on a
  literal/at the entrypoint but fails on a runtime value / through a boundary, check whether the runtime emitter
  EXISTS before theorizing about shapes — a const-foldable positive control is not evidence the runtime path
  works. Pinned `05-compound-types.sexp` "a multi-argument call node is evaluated by iterating its payload arg
  list" (→42). #17 RESOLVED; #13 (list patterns) now purely ergonomic.
- [The reader gate (built-in Option across a boundary) closed fully — and `List.at` on a payload-bound list is the next accessor](./2026-07-07-the-reader-gate-closed-and-list-at-on-a-payload-list-is-the-next.md)
  — the seed rebuilt and closed backlog #12 (built-in Option/Result losing its payload kind across a boundary)
  across ALL facets at once: `String.from-bytes` through a helper works (→2, ill-formed → None arm; real
  `gen_runtime_string_from_bytes`, validates with the existing runtime — `bytes-is-utf8` not needed on this path),
  AND the bare `(Some 42)` through a helper (the general kind-recovery facet, deepest, untouched by prior
  per-accessor fixes) → 42. Both corpus cases withheld/todo in earlier cycles flipped todo→PASS. Vindicates the
  accessor-by-accessor learning: per-accessor patching closes symptoms, the general fix closes the class. New gap
  surfaced — Tier 3h: `List.at` on a list BOUND FROM A SUM PAYLOAD declines (List.len on it works; List.at on a
  top-level list works) — same "payload binder yields a shape the accessor doesn't recognize" pattern one level
  out; blocks the natural multi-arg-call rep `KCall (Tuple Int64 (list Core))`. Pinned `05-compound-types.sexp`
  "indexing a list bound from a sum payload yields the element" (→10, todo). #12 RESOLVED; new #17.
- [The reader realizes the prelude-index name-resolution contract — head names resolve by byte-comparing prelude symbols, no runtime String](./2026-07-07-the-reader-realizes-the-prelude-index-name-resolution-contract.md)
  — the reader's name-resolution seam directly realizes `ast-encoding.md` (a node names its kind by a prelude
  INDEX, not inline; the prelude lists the distinct symbols). `prelude-entry k` locates the Nth symbol,
  `name-eq` byte-compares its payload against an operator name (`b"+"`) — LENGTH first, then bytes, no runtime
  String. So "resolve names to codes" is a property of the FORMAT (the code is already the index), not a pass the
  reader implements — "the input format is an ally," fully realized on runtime bytes. Load-bearing detail:
  length-BEFORE-bytes, because operator names aren't prefix-free (`+`/`++`, `<`/`<=`); a byte loop stopping at the
  shorter length mis-resolves `"++"` as `+`. Pinned `10-bytes.sexp` "resolving a head against a prelude symbol
  rejects a length-mismatched prefix" (`"++"` vs `b"+"` → 0, PASS). (The symbol-table→String materialization is
  the separate in-flight from-bytes work; head resolution doesn't need it.)
- [`String.from-bytes` validates in the runtime — a String is a UTF-8 Bytes leaf, so decode is a check, not a copy](./2026-07-07-string-from-bytes-validates-in-the-runtime-a-string-is-a-utf8-bytes-leaf.md)
  — the reader's symbol-table decode needs runtime `String.from-bytes` (backlog #12). In-flight fix (WIT append +
  codegen, mid-landing — binary NOT yet rebuilt): a runtime String IS the same Bytes-backed UTF-8 leaf, so
  `from-bytes` needs no decode/copy — only a validity CHECK, via a new runtime primitive `bytes-is-utf8` (WIT idx
  54), not a compiler-emitted state machine. Two design points: (1) String-as-Bytes-leaf collapses a decode to a
  predicate + zero-cost retag (same shape as tag-free heap / rope slice — an op that looks like copy is a check
  over shared bytes); (2) validation belongs in the runtime — a hand-emitted validator checking only byte SHAPE
  accepts overlong (`C0 80`) + surrogate (`ED A0 80`) encodings, both security-relevant, both forbidden by strict
  UTF-8. Pinned two `13-strings.sexp` cases (overlong + surrogate → None) recording the strict-UTF-8 requirement
  (skip on `needs binary-matching`). NOT claimed landed — design + requirement captured, probe when built.
- [The reader's decode surface is complete — dispatch, iterate, and atom-decode are the three legs of a canonical-AST reader](./2026-07-07-the-reader-decode-surface-is-complete-dispatch-iterate-atom.md)
  — the spike rounded out the reader's ATOM decode + variable-arity applications: `read-node` now decodes every
  CBOR scalar major (0 uint → NInt, 1 negint → `NInt (-1 - arg)`, 7 simple → NBool `0xF5`/`0xF4`), and `read-app`
  dispatches by head AND arity (`if`→3, `not`→1, else binary). A canonical-AST reader is exactly THREE legs, all
  now built + gate-witnessed: DISPATCH (scalar head → operation), ITERATE (array length → loop), ATOM-DECODE (leaf
  scalar → value by major type). Exhaustive over what a reader does. Reached by accretion of small verified arms,
  not a "reader algorithm" — the composition thesis again. Negint note: CBOR encodes -n as `-1-n`, so reading it
  as a plain uint silently corrupts a literal (9 for -10) → earns a known-answer case. Pinned `10-bytes.sexp` "a
  CBOR atom decodes each scalar major type to its value" (negint -10 + bool 1 + uint 10 = 1, PASS).
- [The self-hosting gate shifted from "seed capability" to "the compiler's source is within its own accepted subset"](./2026-07-07-self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset.md)
  — a CATEGORY SHIFT in the blocker. For ~20 cycles the gate was a seed capability (a shape the seed couldn't
  compile); all fixed. Now the gate is "does the compiler accept the language its OWN source is written in?" — a
  bootstrapping-completeness question. `compile-bytes` reads+compiles the subset {arith, comparison, bool, if,
  let, call, multi-def, Int64/Bool}; the compiler's own source uses more (sum types, match, String, heap
  recursion), so `compiler compiles compiler` is gated on the Cadenza compiler's front end/backend GROWING to
  accept those (which the seed already compiles — the compiler just doesn't yet EMIT for them). Changes the loop's
  job: from defect-finding (probe seed → pin corpus case) to coverage-measuring (a subset frontier / capability
  inventory — a roadmap artifact, not a per-shape case). #12/#13 recategorized "reader gate" → "subset growth".
  Pinned `10-bytes.sexp` "a CBOR skip steps over a tagged item" (tag 39 `d8 27 01` → 3), completing cbor-skip's
  item-kind coverage (array/string/tag/scalar).
- [The whole-module reader is wired — the compiler reads a multi-def module's canonical AST and compiles it](./2026-07-07-the-whole-module-reader-is-wired-module-bytes-to-component.md)
  — the reader went from a single expression to a WHOLE MODULE: `compiler.cdz`'s `main` now compiles `module
  bytes → component`. The CBOR of `(module m (def (main) 42))` → read-module → resolve-module → fold → lower →
  serialize → frame → valid 89-byte component. New machinery (`read-module`/`read-defs`) reads CBOR array LENGTHS
  as structural counts — def-count = root array length − 2, param-count = signature length − 1 — the count half of
  `bytes → AST` (vs the head-index-dispatch scalar half). Added no new primitive: it's `cbor-arg` + `skip-elems` +
  `read-node`, proven pieces assembled at one more level. A canonical-AST reader is fundamentally two things:
  dispatch on a decoded scalar, and iterate by a decoded count — both now built + gate-witnessed. Caveat: still
  architecture at small scale (compiler-compiles-compiler needs TCO for deep sources). Pinned `10-bytes.sexp` "a
  CBOR reader walks a variable-length array using its decoded length as the element count" (`[10 20 30 40]` → 100).
- [The workaround was the bug — correcting the "scale limit" diagnosis of the final self-host blocker](./2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis.md)
  — CORRECTION: two cycles ago I diagnosed Tier 2f ("resolve on a runtime Node can't box") as a seed SCALE limit
  (every tractable resolver passed, only the full 18-variant failed → "no minimal witness"). WRONG. The real
  cause was self-inflicted: `resolve`'s PUnknown arm used `(Bytes.len (Bytes.of (list 256)))` — an out-of-range
  Bytes hack as a placeholder trap (backlog #11's stub) — a Never value that poisoned the WHOLE runtime `resolve`.
  Replacing it with an honest `KError → unreachable` fixed it. THE WORKAROUND WAS THE BUG. Underneath was a real
  (differently-shaped) seed invariant: a Never value on the runtime-heap path emitted invalid code, now hardened
  ([[never-typed-value-on-the-runtime-heap-path]]). Meta-lesson correcting the scale-limit rule: my bisection
  rebuilt a clean ANALOGUE (structure) and dropped the culprit (the Bytes content), so it confirmed a false
  hypothesis — REDUCE THE FAILING PROGRAM BY DELETING ITS ARMS, not by rebuilding a clean one. And "write it
  honestly" isn't just style — the contortion can be the defect. Withdraws backlog #16 (mis-framed); resolves #11.
- [The reader is wired — the compiler now reads a program's canonical AST bytes and compiles it, end to end](./2026-07-07-the-reader-is-wired-bytes-to-component-end-to-end.md)
  — MILESTONE: with every self-host seed blocker cleared, the reader is WIRED into the pipeline. `compiler.cdz`'s
  `main` now compiles a program READ FROM ITS OWN CANONICAL AST BYTES: `read-node : Bytes → Node` decodes the CBOR
  `83 01 81 61 2B 83 00 01 02` (= `(+ 1 2)`) → resolve → fold → lower → serialize → frame → a VALID component
  whose code is `i64.const 1; i64.const 2; i64.add`. `bytes → component`, verified. The reader needed no big new
  mechanism — it COMPOSES primitives each landed as a verified, corpus-pinned step (head decode, navigation, name
  matcher, resolver join). Lesson: a self-hosted front end is a composition of small individually-verifiable byte
  ops, symmetric to the output side. Caveat: this is the SINGLE-EXPRESSION read path; multi-def module read +
  scale (TCO for deep sources) remain, so it's `bytes → component` for an expression, not yet compiler-compiles-
  compiler. Pinned `10-bytes.sexp` "a recursive reader decodes a CBOR application tree and evaluates it by head
  index" (`[+ 1 [* 2 11]]` → 23, PASS).
- [The final self-host blocker is fixed — the reader can now join the pipeline, and the scale-limit case became pinnable](./2026-07-07-the-final-self-host-blocker-is-fixed-the-reader-can-join-the-pipeline.md)
  — Tier 2f (the "cannot box" decline feeding a runtime `Node` to the real `resolve`) is FIXED: the full
  18-variant resolve on a runtime-built Node, scalar-consumed, now runs (verified → 1). This was THE last hard
  blocker on `bytes → bytes` self-hosting — the reader (`read-node`, already built) can now join
  `read → resolve → fold → lower → serialize → frame`. EVERY self-host seed blocker (Tier 00/0/2b/2c/2d/2e/3a/2f)
  is now cleared. Flip side of last cycle's rule: a scale limit resists a minimal case WHILE broken (every
  reduction passes), but once FIXED a representative case at natural size is a fine guard. Lifecycle: broken →
  bisect+backlog; fixed → pin a representative case. Pinned `05-compound-types.sexp` "a recursive resolver
  transforms one runtime sum tree into another, then consumes it" (Node→Core→scalar, cross-sum-type, → 42, PASS);
  closes #16. Remaining: WIRING the join in compiler.cdz + non-blocking #12/#13. (Handoff docs still lag — 3rd/4th
  stale claims this session; the probe confirmed the fix, not the doc.)
- [The final self-host blocker is a scale limit, not a shape gap — and a scale limit resists a minimal corpus case](./2026-07-07-the-final-self-host-blocker-is-a-scale-limit-not-a-shape-gap.md)
  — the reader can now be JOINED to the pipeline, and the join is the last blocker. `read-node : Bytes → Node` is
  verified (`read (quote (+ 1 2))` builds the right Node), but feeding it to the real `resolve : Node → Core`
  declines "runtime compound element of a kind the runtime cannot box yet". Decisive: it does NOT reduce — a
  3-variant resolver runs, a 6-variant heterogeneous one runs (→4, verified), only the full 18-variant `Core`
  fails; and even `resolve` on a runtime `(NInt 42)` declines, so it's a full-FUNCTION scale/union property, not
  an input shape. Different KIND of blocker from every prior reader gap (all were shape gaps, minimally pinnable):
  a scale limit has NO minimal witness. So NO corpus case (a giant resolver is brittle; a tractable one guards
  nothing) — the honest artifact is a bisected backlog entry (#16) + the regression guard being the whole
  `compiler.cdz` resolve compiling once fixed. RULE: shape gap → minimal case; scale limit → bisection + backlog,
  real artifact as the guard. Tier 2f is the single remaining hard gate on bytes → bytes self-hosting.
- [The invalid-component violation is fixed — completing the withheld-case cycle — and the handoff doc lags the seed](./2026-07-07-the-invalid-component-violation-fixed-and-the-handoff-lags-the-seed.md)
  — the `let`-free `tuple.N`-on-a-named-def violation (item 15, emitted an INVALID component) is FIXED, thoroughly
  (whole-program result 40, tuple.1 → 5, consumed → 140, compound element matched → 7). Completes a clean cycle:
  invalid → WITHHELD (couldn't pin as corpus without FAILing the gate) → fixed → pinned GREEN. That's the right
  lifecycle for a decline-don't-miscompile violation — never sits as a FAIL, never lost (backlog carries it until
  the fix). Second-order lesson: the spike's handoff docs LAG the seed — SEED-GAPS still says this case "still
  produces a VALID component that TRAPS" (wrong twice: it runs now, and it was INVALID not valid-but-traps), the
  2nd stale claim in 2 cycles (the 1st: `compiler.cdz` calling live `name-eq` dead code). So the loop PROBES the
  running seed, never trusts a fast-moving handoff's status — a doc is a lead, the corpus (which executes) is the
  oracle. Pinned `05-compound-types.sexp` "a scalar element is projected directly from a function's runtime tuple
  result" (→ 40, PASS); closes #15.
- [The recursive-Bool fix unblocked the reader's name matcher — and the full surface language composes in one program](./2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes.md)
  — the recursive-Bool return-kind race (item 14) is FIXED: its corpus case flipped todo→PASS with no oracle
  change, and the reader's `name-eq` (byte-by-byte prelude-symbol comparator, the `(if (= a b) (recurse) false)`
  shape) came alive (`b"++"` vs `b"++"` → 1) — the dead-code bet paid off. Third confirmation that kind-inference
  order-independence is ONE property (Heap=Tier00, Bool=item14, same fix). MILESTONE: the full surface language
  composes in ONE realistic program — `classify x = (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0)`
  compiles end-to-end (`4 → 15`, `20 → 0`): short-circuit `and`, runtime `let` (real local, not alias), nested
  `if`, arithmetic — all from surface names, all threading correctly. The CONVERSE of the gap-finder lesson: when
  features finally compose, that composition is a conformance obligation the floor-outward corpus lacks. Pinned 2
  `02-binding-and-control.sexp` integration cases (both PASS).
- [Runtime tuple projection works through a `let` — and the direct path is a decline-don't-miscompile violation, not a clean trap](./2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles.md)
  — the spike fixed `tuple.N` on a runtime (`let`-bound) tuple (the decoder's `(node, index)` pair): `arr-get` +
  unbox from the `Local`'s carried `Shape`. Verified `(let ((r (dec 4))) (+ (tuple.0 r) (tuple.1 r))) → 45`. But
  probing the `let`-FREE path found it WORSE than the handoff recorded: SEED-GAPS says "valid component that
  traps"; measurement shows `(tuple.0 (dec 4))` emits an INVALID component (fails wasm validation) — a
  decline-don't-miscompile violation, strictly worse than a clean decline or defined trap. The `let` is
  load-bearing (shape recovery is wired to the binding site, not the projection operator). Durable point:
  "valid-but-traps" ≠ "invalid component" — different severities; a handoff recording the milder one HIDES a
  miscompile, and the gate scores invalid as FAIL not todo (so it can't be pinned green until the seed compiles or
  declines it). No corpus case landed (would FAIL the gate); SPEC-BACKLOG #15.
- [The reader's whole foundation is built and verified — gated on a single inference bug, as dead code](./2026-07-07-the-reader-foundation-is-built-and-gated-on-one-inference-bug.md)
  — the reader's three sub-capabilities are all written + verified on real `(quote 42)` bytes: head decode
  (`cbor-major`/`arg`/…), structural navigation (`cbor-skip`/`skip-elems`, walks past a nested item), and name
  resolution (`prelude-entry`/`name-eq`, byte-compares a prelude symbol to `b"+"` — no runtime String). But
  `name-eq` is the recursive-Bool shape that declines (item 14), so the spike parked it as DEAD CODE: the compiler
  still builds (nothing calls it yet), and it comes alive when item 14 is fixed + the top-level `read` walk is
  wired. So the reader is fully scaffolded, blocked on ONE seed inference bug. A method: build+verify everything
  else, park the blocked fn as dead code (sibling of "route around the blocker"). Honest caveat: foundation built
  ≠ reader works (name-eq inert → no end-to-end `bytes → AST` yet). Pinned `10-bytes.sexp` "a CBOR skip walks past
  a whole nested item" (`82 82 01 02 03` → 5, PASS). Self-host gate = #12 + #13 + #14.
- [A recursive Bool function's return kind is inferred branch-order-dependently — the same kind race as Tier 00, now on Bool](./2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent.md)
  — probing the reader's name matcher found: a self-recursive Bool-returning fn DECLINES ("if condition is not
  Bool" / "branches differ in kind") when the self-call is the THEN branch and a Bool literal the ELSE; the mirror
  (self-call in ELSE) compiles, and an Int-returning version compiles. So it's Bool-specific + branch-order-specific
  — the SAME order-dependent kind race as Tier 00, now on Bool instead of Heap. Fix = the proven one: a concrete
  branch pins the if/match result kind regardless of order (self-call placeholder yields to a concrete sibling).
  Lesson: kind-inference order-independence is a property EVERY kind needs, not a Heap-specific patch. The reader's
  head resolver IS a recursive Bool name-eq in exactly this shape → blocks self-hosting. Pinned `09-functions.sexp`
  "a self-recursive Bool-returning function whose recursive call is the then-branch" (→ 1, todo). SPEC-BACKLOG #14.
- [The built-in list cannot be pattern-matched — the biggest ergonomic gap for authoring the compiler](./2026-07-07-the-built-in-list-cannot-be-pattern-matched.md)
  — as the reader grew (walking CBOR arrays of children), the spike hit a spec+seed gap: the built-in `list`
  cannot be pattern-matched AT ALL (`(cons h t)`, `(list a b)`, `(list)` all decline "unsupported list pattern";
  `core-semantics.md` §Pattern Matching says NOTHING about lists). So every list-consuming pass — module def list,
  code stream, CBOR children — is hand-rolled as a custom cons-sum (`FList`/`Code`/`DList`) duplicating the
  sequence type. Right design keeps representation OPAQUE (a `list` is a persistent tree, not cons cells):
  ML/Rust-style element patterns with a rest binder — `(list)` empty, `(list x .. rest)` first+tail — matcher asks
  len/first/rest. A spec decision (pattern matching is core-semantics), not just seed work. Pinned
  `05-compound-types.sexp` "the built-in list is folded by an element-with-rest pattern" (→ 60, `(needs
  list-patterns)`, skips). SPEC-BACKLOG #13. Also: `String.from-bytes`-thru-boundary reclassified NOT blocking the
  reader (raw-byte decode doesn't need it; only the symbol table does).
- [The reader decodes CBOR as the input dual of the output spine — built on the byte primitives that already work](./2026-07-07-the-reader-decodes-cbor-as-the-input-dual-of-the-output-spine.md)
  — the spike started its READER (last major piece before self-hosting), and it fell out as the input dual of the
  LEB128 output spine: `cbor-major` (`>> byte 5`), `cbor-info` (`& byte 31`), `cbor-arg` (1/2/4/8-byte big-endian
  argument), all built on `byte-at` = `(match (Bytes.at b i) ((Some x) x) (None 0))` + bit ops — the SAME small
  vocabulary the encoder composes upward, so there's no separate reader runtime and it could start the moment
  Bytes.at-across-a-boundary landed. Verified: major of 0x83 → 4, arg of `18 2A` → 42, be-assembly of `01 2C` →
  300. It's authored AROUND the open #12 facets — decodes raw bytes with Bytes.at, not String.from-bytes (the
  symbol table is where from-bytes becomes unavoidable, the reader's next dependency). Like the output side, the
  decode is a COMPOSITION needing a known-answer case, not just verified primitives. Pinned `10-bytes.sexp`
  "a CBOR head decodes its major type and big-endian argument" (`19 01 2C` → `(tuple 0 300)`, PASS).
- [The reader gate is being closed accessor-by-accessor — `Bytes.at` crosses a boundary now, `String.from-bytes` is next](./2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor.md)
  — the built-in-fallible-result-across-a-boundary gate (item 12) is being closed one accessor at a time, as that
  learning warned. This cycle `Bytes.at` through a helper was fixed (the reader's per-byte idiom → works); but the
  fix is accessor-specific: `String.from-bytes` through a helper still declines ("unsupported dotted-application" —
  a DIFFERENT message, so it needs its own runtime lowering, not just payload-kind work), and a literal `(Some 42)`
  through a helper still declines "arms differ in kind". `List.at`/`Bytes.at` green, `String.from-bytes`/bare-`Some`
  todo. Vindicates item 12: per-accessor patching closes the symptom, not the class — the general fix is to give
  built-in `Option`/`Result` the payload-type registration a user sum gets. A reader uses ALL these at once, so it
  compiles only when the last accessor lands. Pinned `13-strings.sexp` "a helper decodes bytes to a string and
  consumes the fallible result" (→ 2, todo).
- [Shape inference through `match` unblocks the type-driven emit spine — and prelude variant names must not shadow a program's](./2026-07-07-shape-inference-through-match-unblocks-the-type-driven-emit-spine.md)
  — two seed fixes jointly unblock the compiler's emit spine (the recursive `lower`/`emit` walk turning an AST
  node into instruction bytes). (1) `shape_of` now handles `match`: a `match`'s shape is the UNIFIED shape of its
  arm bodies (as `if` unifies branches), so a `match`-arm-returns-fresh-compound infers directly — the
  `if`-on-discriminant workaround is retired. (2) a prelude variant name (nullary `Sign.Neg`) no longer shadows a
  program's same-named UNARY variant (`Expr.Neg Expr`) — nullary detection is now last-writer-wins (arity is the
  property the check needs; per-type namespacing deferred). Both are the same lesson as type-directed valtype: a
  tree-walking compiler needs shape/kind/arity to come from the value's TYPE, recovered uniformly. Pinned a
  `10-bytes.sexp` emit-spine case (3-variant `Expr → Bytes`, opcode per variant, `emit (Add (Lit 1) (Neg (Lit 2)))
  → b"BB|j"`, PASS); closes SEED-GAPS Tier 3a.
- [The built-in Option loses its payload kind across a function boundary — the last blocker before the reader](./2026-07-07-the-built-in-option-loses-its-payload-kind-across-a-boundary.md)
  — the reader walks input with `(match (Bytes.at input i) ((Some b) …) (None …))`, and that idiom declines
  "runtime sum match arms differ in kind" once the built-in `Option` crosses a function boundary. Boundary is
  sharp AND wider than the spike's Tier-2c framing: `(match (Some 42) …)` at the entrypoint works, but the SAME
  match in a helper declines — even a plain literal `(Some 42)`, not just `Bytes.at`. `List.at`'s Option and
  every USER sum bind payloads across boundaries fine (Tier-2b fix), so the gap is the BUILT-IN Option/Result
  constructor carrying no per-slot payload type (`sum_payload_types`) — its payload kind is recoverable only
  where local type context supplies it. Fix = register the built-in sums' payload types like a user sum's, NOT
  patch `Bytes.at`. A value's kind must come from its TYPE, not the expression that produced it (cf. type-directed
  valtype). Pinned `05-compound-types.sexp` "a built-in Option is unwrapped by a helper that binds its payload"
  (→ 42, scores todo). The current gate on the reader → self-hosting.
- [The nested-payload-binder fix closes the front end — a multi-def surface module now compiles end-to-end](./2026-07-07-the-nested-payload-binder-fix-closes-the-front-end.md)
  — the Tier-2b blocker (a `match` arm binding a nested tuple in a sum payload, `(Ctor (tuple op (tuple a b)))`)
  is FIXED in the seed: `bind_sum_payload` now recurses into a nested `(tuple …)` slot, exactly as predicted. The
  corpus case that pinned it flipped todo→PASS with NO edit to the oracle — reject-don't-miscompile working as
  designed. With it fixed, the spike CLOSED its front end end-to-end: a `Def`/`DList` multi-definition surface +
  `resolve-module` (DList→FList, name→code) means a whole textual module now flows read → resolve → fold → lower →
  serialize → frame → bytes. Verified `(module m (def (main) (+ 20 22)) (def (dbl x) (* x 2)))` → valid 2-function
  component. Only the READER (bytes → DList, CBOR decode) remains before self-hosting. Two notes: flat surface
  nodes are now a CHOICE not a workaround (nesting no longer declines — prune the stale comments); and the
  unknown-head path is still a placeholder TRAP, not a real diagnostic (new backlog item).
- [Runtime strings landed — the keystone unblocked, and the front rung now resolves a name to a code](./2026-07-07-runtime-strings-unblock-the-name-based-front-rung.md)
  — runtime `String`, the Tier-0 keystone blocker of a self-hosting front end, landed in the seed: string fn
  parameters, runtime string `=` dispatch, string return across a call, and string sum-payloads all compile now
  (the SEED-GAPS Tier-0 probes that all declined). The spike rewrote its front rung to resolve a form's head by
  NAME — `main` compiles `(+ 20 22)` from a STRING-headed node `(NPrim (tuple "+" …))`, `resolve` maps `"+"` to a
  typed `Prim` via `head-prim`, no string survives into Core (looked up once at the resolve seam), and an unknown
  head → `PUnknown` → DECLINE (reject-don't-miscompile at the surface). "Resolve names to codes" is now the REAL
  front rung, not an integer-opcode stand-in. Sidesteps the still-open nested-binder blocker (backlog #1) via a
  FLAT node payload. Pinned a `13-strings.sexp` multi-way head-dispatch case (PASS); the Tier-0 probe cases a
  sibling pinned are all green. Front end's critical path now: nested-payload decode (#1) + the CBOR reader.
- [The compiler emits a multi-function module with a real call — and routes around the front-rung blocker to prove the backend](./2026-07-06-the-compiler-emits-a-multi-function-module-with-a-real-call.md)
  — milestone: `compiler.cdz` now compiles to a valid component that is MORE THAN ONE FUNCTION and threads a real
  `call` with a parameter (`main = dbl(21)`, `dbl x = x+x` → 42 via `call 1`, not a fold). New: a multi-function
  assembler (`compile-program` over an `FList` of `Func`, N-entry sections), `Core` constructors `KLocal`
  (→ `local.get`) and `KCall` (→ `arg ++ call fi`), and `KIf` → structured `if/else/end` for a RUNTIME condition.
  The headline is METHOD: the agent ROUTED AROUND the front-rung blocker (Tier 2b nested binder still declines) by
  hand-building the folded `Core`/`Func` list `main` feeds the assembler — so the backend is proven FROM THE
  RESOLVED IR INWARD while `resolve` stays stubbed. The resolved-IR seam is the right TESTING seam too (Core is a
  user sum, so it can be built by hand). Honest status: backend proven, front rung still blocked on backlog #1.
  Pinned two `02-binding-and-control.sexp` runtime-conditional cases (both PASS).
- [Folding a constant-condition conditional must preserve short-circuit shielding — the third face of trap-preserving rewrites](./2026-07-06-folding-a-constant-condition-preserves-short-circuit-shielding.md)
  — the fold pass grew to conditionals: `fold-if` reduces `(if c t f)` when `c` folds to a constant by BECOMING
  the taken branch and DROPPING the other, so a trap/effect in the untaken branch never occurs — correct because a
  run-time conditional already shields its unselected branch. Verified `(if (< 1 2) 7 (% 5 0)) → 7` (condition
  folds to true, `(% 5 0)` dropped). This is the third face of the trap-preservation principle
  ([[2026-07-06-constant-folding-must-preserve-runtime-traps]]): don't ERASE a trap (`/ 10 (- 3 3)` still traps),
  don't MANUFACTURE one in arithmetic (`% Int64.min -1` must yield 0), don't manufacture one in CONTROL (drop the
  untaken trapping branch). The control face is easiest to get wrong because folding an `if` reads as an
  optimization, not a shielding obligation. Pinned in `02-binding-and-control.sexp` (PASS); same reasoning governs
  `and`/`or` short-circuit.
- [The component's result valtype is type-directed — through an exhaustively-matched Kind sum, the same discipline as the instruction sum](./2026-07-06-result-valtype-is-type-directed-through-an-exhaustive-kind-sum.md)
  — the spike grew comparisons (`<`, `=`) whose result type is Bool, not Int64, forcing the framing to present
  `run` at the right boundary valtype (Int64 → s64, Bool → bool). Solved with a type-directed `kind-of : Core →
  Kind` pass where `Kind` is a SUM (`Ki64 | KBool`) matched EXHAUSTIVELY by the pass and both valtype maps — so
  adding a kind (a float result) is a compile error until every consumer handles it, the same reject-don't-
  miscompile discipline the `Instr` sum gives the backend. Also completed "no integer/string tag dispatch" at the
  SURFACE: the head moved from integer opcode to a `Prim` sum variant. This is the seam where full type inference
  will live (operand kinds are fixed today, so `kind-of` is a direct read, no unification yet). Pinned two
  `03-equality-and-observation.sexp` cases (same `main` shape, Bool boundary vs Int64 boundary, both PASS).
- [The compiler's byte-emitting spine needs a known-answer corpus case, not just verified primitives](./2026-07-06-the-compilers-byte-emitting-spine-needs-a-known-answer-corpus-case.md)
  — the spike reported its LEB128 encoders "verified byte-correct" (`uleb 624485 → E5 8E 26`), but that check
  lived only in an ephemeral `emit` probe in the gitignored spike; the corpus pinned every INGREDIENT (`<`, `&`,
  `|`, `>>`, `Int.to-byte`, `Bytes.concat`, recursive-by-count Bytes) in isolation but never the COMPOSITION —
  the actual recursive encoder run to a known-answer multibyte output. Verifying primitives separately does not
  verify they compose to the right bytes; a single slip (wrong mask/shift, dropped continuation bit) is invisible
  per-primitive yet miscompiles the component. Pinned two `10-bytes.sexp` cases (multibyte `624485 → b"\xe5\x8e&"`
  + base-case `100 → b"d"`, both PASS). Rule: when a spike says "verified byte-correct" via a probe, it is NOT
  durable until it is a corpus case — the gate only protects what the corpus pins.
- [Constant folding must preserve runtime traps — and whether a certain trap should be a compile error is a separate decision](./2026-07-06-constant-folding-must-preserve-runtime-traps.md)
  — the compiler's first Core→Core rewrite (constant folding) must be MEANING-PRESERVING: `(/ 5 0)`'s recorded
  meaning is a trap, so folding it to a value would erase the trap (a miscompile) and folding it to a rejection
  also changes the meaning. The fold is guarded (`foldable-divisor`) — fold a division/modulo ONLY when the
  divisor is a non-zero, non-overflowing constant; otherwise keep the primitive so the trap fires at run time.
  Verified: `(/ 10 (- 3 3))` folds the divisor to 0 but still TRAPS (preserved, not erased); the mirror bug is the
  seed's over-eager const-fold TRAPPING `(% Int64.min -1)` which must yield 0 (manufacturing a trap). SEPARATE
  open question the operator flagged: should a provable-certain trap be a COMPILE-TIME rejection? Kept out of the
  fold — reachability (`(if false (/ 5 0) 42)` → 42, verified) and the ragged boundary (`(/ 5 0)` rejected but
  `(/ 5 (id 0))` not) push it to a later reachability pass, tied to compile-time-mandatory-eval contexts à la
  Rust/Zig. Recorded as SPEC-BACKLOG item 9; corpus case "a division whose divisor folds to zero still traps"
  pins the erase-direction floor.
- [A language with conditionals still needs boolean connectives — the spec had none](./2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives.md)
  — a routine compiler predicate (the signed-LEB128 terminator, an `and`/`or` of bit tests) could not be
  written: `(and a b)`/`(or a b)`/`(not a)` were absent from the seed, EVERY conformance case, and the spec —
  a language with a proven-short-circuit `if`, comparison operators, and a totally-ordered `Bool`, but no way to
  COMBINE two booleans without nesting a conditional per condition. The gap survived because the corpus grew case
  by case and none happened to need a connective, and because connectives are so basic their absence reads as
  impossible rather than as an omission to check. Drove a *Boolean Connectives Short-Circuit* requirement in
  `core-semantics.md` (adjacent to *Conditionals Evaluate One Branch*): the language MUST offer conjunction,
  disjunction, negation; conjunction evaluates its right operand only when the left is true (disjunction only
  when false), SHORT-CIRCUITING so a connective shields a trapping/effectful right operand exactly as an
  unselected conditional branch does; each operand is type-checked as a boolean whether or not evaluated. The
  short-circuit choice is load-bearing — it fixes behavior on a right operand that traps or performs an effect.
- [A list and a persistent vector are one type — representation is the runtime's choice, not the author's](./2026-07-06-a-list-and-a-persistent-vector-are-one-type-representation-is-the-runtimes-choice.md)
  — the language had grown TWO surfaces for the same idea (an ordered, homogeneous, immutable, indexed
  sequence): the specified `list` (flat array, `(list …)`) and an unspecified `Vec` (a 32-way radix trie,
  `(vec …)`, `Vec.push`/`update`) that arrived as a self-hosting output accumulator and quietly acquired a
  surface type, render form, and API namespace. Only `list` was ever in `collections-and-text.md`; a flat
  array IS the trie's ≤32-element base case (a single leaf), so they are one data structure split at an
  arbitrary size threshold, not two representations across a type boundary; and their observable contracts
  are identical but for the performance curve. Two surface types makes representation OBSERVABLE at the
  surface — contradicting the tag-free runtime, #Sharing Is Not Observable, and the persistent-collections
  learning's explicit "representation can change freely with zero emitted-byte impact." Merge to ONE sequence
  type (keep `list`'s name/literal/render, absorb `push`/`update` + the trie as its representation, delete
  `Vec` + `(vec …)`). Author-controlled representation (Clojure/Rust) is a coherent but OPPOSITE philosophy
  Cadenza never chose — it drifted in through an accumulator. The sharpened thesis: **a new representation is
  a new way to store an existing type, not a new type.** Drives collections-and-text.md §"A List Is An Ordered
  Homogeneous Sequence" (functional growth + representation-unspecified); shrinks deterministic-value-form by
  one value form; corpus `(needs persistent-vector)` cases fold into `list` ops; no envelope re-derivation.

## Spec gaps (found by adversarial-corpus probing) — all four RESOLVED 2026-07-05 in a clarity pass

These entries record behavior the specification had **not fixed** — an adversarial corpus run reached
a construct with two or more defensible, observably-distinct outcomes and no requirement selecting between
them, so the corpus recorded no oracle for it. Each named the gap, the candidate readings, and a recommended
resolution, deferring the requirement edit to a follow-up clarity pass. **That clarity pass ran on 2026-07-05
and resolved all four** (each bullet below carries its RESOLVED note: the RFC-2119 sentence added, the witnessing
corpus case, and — where the seed does not yet enforce the new rule — the `(needs …)` capability tag that skips
the witness rather than FAILing the gate). The string/bytes indexing gap was resolved by an operator direction
that went *beyond* the recommended fix — making indexing fallible (Option-returning) with an `expect` combinator,
superseding total-or-trap entirely — a behavior change the seed realizes in a later generation.

- [Spec gap: `let` binding sequencing is unspecified](./2026-07-05-spec-gap-let-binding-sequencing.md)
  — whether multiple bindings in one `let` are sequential (`let*`, each initializer sees the earlier names) or
  parallel (each evaluated in the enclosing scope) is undetermined; `(let ((x 1) (y (+ x 1))) y)` is 2 under one
  reading and an unbound-name rejection under the other.
  **RESOLVED 2026-07-05 (sequential):** core-semantics.md §"The Bindings Of One `let` Take Effect In Order" fixes
  each initializer to observe the bindings written before it (a repeat shadows), matching the seed and the
  `do`-sequencing rule the spec already commits to. Witnessed by two core cases in 02-binding-and-control.sexp
  (`(let ((x 1) (y (+ x 1))) y)` → 2; `(let ((x 1) (x (+ x 10))) x)` → 11), both of which the seed passes.
- [Spec gap: duplicate pattern binder](./2026-07-05-spec-gap-duplicate-pattern-binder.md)
  — whether a pattern may bind the same name twice (`(tuple x x)`), and if so whether it shadows, errors, or
  imposes an equality constraint, is unspecified.
  **RESOLVED 2026-07-05 (linear, compile-time error):** core-semantics.md §"Bindings Introduced By A Pattern Are
  Scoped To Its Branch" now requires each name be bound at most once, rejected as new code `CDZ0102`. Witnessed by
  a 05-compound-types.sexp case `(match (tuple 1 2) ((tuple x x) x) (_ 0))` → `(error CDZ0102)`, gated
  `(needs linear-patterns)` because the seed today mis-accepts the pattern (it lets the second binder shadow and
  returns 2), so the case skips until a generation enforces linearity rather than FAILing the gate.
- [Spec gap: String/Bytes indexing lacks a total-or-trap requirement](./2026-07-05-spec-gap-string-bytes-total-or-trap.md)
  — only *list* indexing has a dedicated total-or-trap MUST; String and Bytes out-of-bounds reads rely on the
  weaker general partial-operations clause, which permits a trap *or* a defined value, so the corpus's recorded
  traps are one permitted choice rather than required behavior.
  **RESOLVED 2026-07-05 (made FALLIBLE, superseding total-or-trap — operator direction):** the operator chose that
  indexing/lookup should be *fallible*, not trapping. collections-and-text.md §"List Operations Are Total Or Trap"
  became §"Indexing And Lookup Are Fallible, Not Trapping": `List.at`/`String.at`/`Bytes.at`, sub-sequence `slice`,
  and map `get` all return an `Option` (`Some` in bounds, `None` out of bounds / slice-out-of-range / missing key),
  and core-semantics.md §"Requiring The Value Of An Optional Traps On Absence" adds `expect` — an Option-specific
  combinator taking a *mandatory message* that becomes the trap reason (chosen over a generic `unwrap` across
  Option+Result, and over the `unwrap` name, so intent is stated at the call site). This retires the three specific
  OOB trap reasons (`list`/`bytes index`, `bytes slice out of bounds`). All ~27 indexing corpus cases across
  05/10/13 (and one match-over-slice in 02) were flipped to Option outputs and gated `(needs fallible-access)` — a
  capability the seed does not yet realize (it still traps directly) — so they skip until a `/build` returns the
  Option, keeping the gate green. This is a *behavior change*, larger than the original gap, not merely a MUST added.
- [The behavior gate is not byte-exact for floats](./2026-07-05-behavior-gate-not-byte-exact-for-floats.md)
  — the whole-float renderer used `f as i64`, which *saturates*, so distinct floats ≥ 2^63 (1e19, 1e20, 1e100)
  collapsed to one canonical form — violating deterministic-value-form injectivity — and the gate couldn't catch
  it because it rendered both sides through the same `display_float` (testing the renderer against itself).
  **RESOLVED 2026-07-05:** both renderers now use `format!("{f:.0}.0")` (injective), and the gate gained an
  INDEPENDENT round-trip oracle — a float output's observed text must `parse` back to the recorded f64
  bit-identically. General lesson: a canonical-form / injectivity requirement cannot be discharged by comparing
  two outputs of the same function; the gate needs an oracle computed by an independent path (the parse inverse).
