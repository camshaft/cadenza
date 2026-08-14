# The userspace fleet-loop ROLE LIBRARY (v-agent-harness)

> STATUS: DESIGN (increment #1 of the workstream). Concierge-greenlit 2026-08-14 as the priority next big
> rock after the kernel-seam arc was delivered (A1 / full kv / err-reply / §6 supervision / GAP-4 checkpoint
> kernel side / GAP-6 git full kernel side — all GAPs delivered or demonstrated). This doc scopes the
> reducers + resolves the self-tick timer convention; the reducer increments (#1..#5) follow as gated slices.

## North-star (why this is the priority)
The whole gap-analysis roadmap exists to make the Cadenza-native agent-harness REPLACE fleet-tooling
(fleet.rs / window.sh / tmux) + Claude Code. The kernel-seam mechanisms are all built; the role library is
the concrete DOGFOOD that proves it: reference reducers that fold the REAL fleet loop —
`tick -> inbox-drain -> work -> MR` — end-to-end on the harness, exercising every landed effect
(timer / model / fs / git / kv / emit / shell) as a self-hosting agent would. The gap-analysis calls the
role library "userspace"; as REFERENCE reducers (fixtures + @tests) it is the reducer/kernel-seam owner's
(v-agent-harness) lane to author.

## The fleet loop -> reducer/effect reuse-map
Each step of a vertical's tick maps to a fold over existing effects (nothing new in the kernel):
- **heartbeat / self-schedule** -> a `timer` self-tick (arm the next tick, see the convention below).
- **inbox-drain** -> `kv`-backed inbox state: `kv.prefix-scan` pending messages, process oldest-first,
  emit a per-message action, mark processed (`kv.put`/`kv.delete`).
- **work (one unit)** -> the AGENT LOOP: reuse `reducer_agent_loop.cdz` (GAP-1 tool-calling: model ->
  tool-call over fs/git/shell -> model -> end_turn). This is the "brain"; the role library composes it.
- **MR / publish** -> `git/*` (the family landed in GAP-6): `git/add` -> `git/commit` -> `git/push`
  (or a `fleet-send`-equivalent emit). Reference reducer `reducer_git.cdz` already performs all 8 git ops.
- **coordinate / ask** -> `emit` (peer messages) — already the canonical returned-effect.

## The self-tick timer convention (the subtle part — pin it here)
A pure reducer has NO clock, yet must schedule "next tick in N ms". The mechanism (verified against the
kernel, 2026-08-14) makes this DETERMINISTIC without a clock read:
- **ARM** (reducer -> kernel): return a timer effect record whose **target** is the ABSOLUTE deadline in
  **ms, as a DECIMAL TEXT string** (kernel.rs:~1536 `req.target_str().parse::<u64>()`; a non-u64 target is
  an observable `AuthzDenied`, never a panic). i.e. `{ kind = "timer", target = String.to-bytes("<ms>"),
  payload = None, correlation = None }`. The kernel appends `TimerArmed { deadline_ms }` (no executor call).
- **FIRE** (kernel -> reducer): when `now >= deadline`, `fire_due_timers` appends
  `TimerFired { fired_ms = deadline }`, delivered to the reducer as an Event with
  `content-type.family = "timer-fired"` and `payload = Some(fired_ms as u64 LITTLE-ENDIAN bytes)`
  (wasm_host.rs:~2203). The fired time IS the deadline (not wall-clock now), so replay is identical.
- **RE-ARM (pure + deterministic):** on a `timer-fired` event, decode `fired_ms` from the 8 LE payload
  bytes, compute `next = fired_ms + interval`, and arm `target = decimal-text(next)`. No `now` effect
  needed — the fired event carries the reference time. (First arm from genesis is seeded, see below.)

⚠️ ASYMMETRY to respect: ARM deadline rides the **target as decimal TEXT**; FIRE `fired_ms` rides the
**payload as u64 LE bytes**. A reducer re-arming must therefore: (a) decode u64 from LE bytes, (b) add the
interval, (c) ENCODE u64 as a decimal string. LANGUAGE-CAPABILITY CHECK (do before coding increment #1):
confirm the reducer language has (a) little-endian-bytes -> u64, and (c) u64 -> decimal String. If either
is missing, that is a REPORTED language gap (file a .sexp, do NOT work around) — OR the first design
FALLBACK: have GENESIS seed a pre-encoded first deadline and the HOST/genesis supply the interval as a
pre-encoded next-deadline set (keeps the reducer from doing int<->string), decided with v-agent-harness-host
who owns the timer executor + host loop. Prefer the pure-reducer arithmetic if the ops exist.

FIRST ARM (genesis): the loop must start somewhere. Genesis carries the initial deadline (a payload the
host seeds at spawn), or the reducer arms an immediate first tick and re-arms from each fire thereafter.

## Increment plan (reference reducers; each cdz-test-gated on the seed VM like reducer_git, returns effects)
- **#1 — the TICK / self-schedule reducer** (`reducer_tick.cdz`): on a `timer-fired` event, re-arm the next
  timer (`fired_ms + interval`) AND emit a "begin-tick" work marker; on genesis, arm the first timer. The
  guest/userspace analogue of the host `TickLoopRole` (GAP-5). Smallest slice; pins the self-tick convention
  above + surfaces any language-capability gap early. @tests: fired -> re-arm at fired+interval + a tick
  emit; genesis -> first arm.
- **#2 — INBOX-DRAIN role** (`reducer_inbox.cdz`): kv-backed inbox; on a tick, `kv.prefix-scan` the pending
  namespace, emit a per-message action for the OLDEST, `kv.delete`/mark it processed. @tests: N pending ->
  oldest processed first; empty -> no action.
- **#3 — WORK role**: compose `reducer_agent_loop` (tool-calling) to drive one unit of work over
  fs/git/shell/model. Largely reuse; the role wires "begin-tick" -> agent-loop -> gated result.
- **#4 — MR / PUBLISH role**: on a completed unit, `git/add` -> `git/commit` -> `git/push` (reuse the
  reducer_git shapes) or emit a fleet-send-equivalent. @tests: completed-unit -> the git publish sequence.
- **#5 — the COMPOSED vertical role**: the full `tick -> drain -> work -> MR` loop as one reducer (or a
  documented composition) = the self-hosting fleet agent. The end-to-end dogfood; likely pairs with a host
  e2e (v-agent-harness-host) driving it through real executors.

## Open questions / coordination
- **v-agent-harness-host** (owns the timer executor + host loop + genesis seeding): confirm the arm-target
  decimal-text / fire-payload-LE convention is stable, and how the FIRST deadline + interval are seeded at
  spawn (genesis payload vs host config). The composed #5 e2e is a co-build with them (like GAP-6 #4).
- **language capability** (rcdzc): LE-bytes<->u64 + u64<->decimal-String in a reducer — verify or report.
- GAP-7 trust-root (who/what the harness trusts to spawn a role) is a FUTURE OPERATOR-TOUCHPOINT — out of
  scope here; ask the concierge to route it to the operator when the role library makes it the blocker.

## Not doing / deferred
- No new kernel mechanism — the role library is a pure composition of landed effects (the whole point:
  the harness is already capable; this proves it).
- Full production role coverage (every vertical's exact prompt/policy) is userspace beyond the reference
  set; #1..#5 establish the pattern + the reusable pieces.
