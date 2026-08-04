# Host capability discovery + mid-session capability upgrade

Owner: `v-agent-harness` (builds it; design by `design-host-capabilities`). Status: **PROPOSAL — peer-reviewed,
awaiting operator ratification.** Operator idea via concierge 2026-08-03;
the operator is away all week (no live iteration), so this is a proposal shaped WITH the two harness
owners (`v-agent-harness`, `v-agent-harness-host`) rather than decided in a live design session. The three
core forks below carry a RECOMMENDED path (mine, endorsed/refined by the harness owners where noted) with
the specific points the operator should ratify on return flagged **⟨pending operator ratification⟩**.
Subsystem: `cdz-kernel` (+ its guest ABI `wit/reducer.wit`), coordinated with `v-agent-harness` (the
extensible content-type / `effect_ct` family arc) and `v-agent-harness-host` (the executor registry +
Cedar authorizer that PRODUCE the manifest).

> **Ownership & build sequencing.** The feature lives entirely in the `cdz-kernel` crate, so it is owned
> by `v-agent-harness` (coordinate-not-fork, not a separate vertical). I1–I3 (the pure projection +
> accessor + probe loop) are buildable against the current `effect_ct` family bridge with no wire change.
> The I4+ slices (the `control/capabilities` query + manifest/`capabilities-changed` event wiring) consume
> the `control/*` partition, so they are sequenced AFTER the register-by-string / family-routing slices
> that establish it, and take a dedicated control-plane design pass with `v-agent-harness-host` (where the
> I4+ shape below is finalized). The forks flagged **⟨pending operator ratification⟩** below (esp. the REQUESTABLE
> policy model) gate I3+/policy behaviour, not the I1–I3 mechanics.

This dovetails with the in-flight extensible-effects arc: routing + authz already key on the effect
**family string** (`effect_ct::{SHELL,HTTP,MODEL,NOW,TIMER,EMIT,…}`, `EffectKind::family()`, landed
#1475). A capability manifest is exactly *a list of those families* the reducer may use — so this design
REUSES that vocabulary rather than inventing a parallel one.

## The problem

A reducer (`wit/reducer.wit`) emits effect *requests*; the host authorizes (SEC-F1), dispatches, and folds
the result back. But the reducer today has **no way to know which effect families the host can serve, at
what resource scope, and whether policy will let it**. It can only emit an effect and discover the answer
from the outcome (`EffectOutcome::Err "no executor registered for kind …"`, or an authz denial). The
operator wants the reducer to KNOW its effect capabilities — and, if the host gains a capability
mid-session, to be able to UPGRADE (learn the new capability) without a restart.

## The load-bearing constraint (from the kernel design, §4b "bridge rule")

Host capabilities are **mutable current-view** state ("what can this host do *right now*"), NOT
immutable-by-hash. The kernel design already rules that a mutable current-view read **must be a query
effect frozen into the local log**, never a live read — a live read poisons replay (the same event would
fold differently tomorrow, §3/§16c-S3). So capability discovery has the SAME shape as every other
mutable-current-view query in the design (status checks, "is session B alive", "which memories are valid
now"): the reducer asks → the kernel answers as-of-now → the answer freezes into the reducer's own log at a
hash → replay reads the frozen answer. Nondeterministic *when asked*, deterministic *once folded*.

This constraint is why "just let the reducer read a global capability table directly" is WRONG: that table
is mutable, so a direct read is a replay hazard. Discovery is a query effect (or a folded event), full stop.

## The two sets, and the one projected answer

There are two distinct notions of "capability", and the manifest must be their **projection**, not either
one raw:

- **Mechanism** — what the host CAN execute: the `CompositeExecutor`'s registered executors, keyed by
  effect family (src/executor.rs `by_kind: HashMap<EffectKind, Box<dyn Executor>>`; under the extensible
  arc, keyed by family string). This is a host fact, session-independent.
- **Policy** — what THIS session MAY do: the session's resource-scoped `Capability` grants (SEC-F1;
  src/effect.rs `Capability { kind, predicate }`) as decided by the current authorizer component (Cedar,
  §20b).

The manifest handed to a reducer = **the authorized projection**: for each family, does the host have an
executor, and what does policy allow this session, at what resource scope. Least-authority by construction
(you see what you may do), but with a visible "you could request more" path (below).

## The three core forks (recommended path + ⟨pending operator ratification⟩)

### 1. Discovery = BOTH genesis-seed AND a `capabilities` query effect (recommended)

- **Genesis seed** — the initial manifest arrives as an early folded event (§3 genesis + context-as-events),
  so the reducer is **born knowing** its starting capability set without a round-trip on its first fold. The
  genesis/session-factory that mints the session already emits setup events; the initial manifest is one of
  them (a `capabilities-manifest` content-type event the reducer folds into KV under a well-known key, e.g.
  `sys/capabilities`).
- **`capabilities` query effect** — a reducer can RE-READ the current manifest on demand by emitting a
  `capabilities` query effect. Per the bridge rule the kernel answers as-of-now and the answer freezes into
  the log as a `capabilities-manifest` result event, folded exactly like the genesis seed (same
  content-type, same KV key → one fold path handles both).

Genesis = the starting snapshot; the query = the mutable-current-view refresh. Not either/or.

### 2. Mid-session upgrade = a PUSHED `capabilities-changed` event (recommended; polling rejected)

The kernel is REACTIVE — append-wakes-the-reducer, no polling (§9d; §12e "swap signal"). When the host
gains (or loses) a capability — a new executor registered, an authorizer/policy swap (§20b `set` of the
Cedar-engine or policy pointer), a delegated grant landing — the kernel **appends a `capabilities-changed`
event to each affected live session**. The reducer folds it, updates its `sys/capabilities` KV, and can now
emit the newly-available effect. A re-query (fork 1's effect) is the PULL fallback for a reducer that wants
to force a refresh; the push is the normal path.

- **Polling a capability-version is REJECTED** — it contradicts the reactive, no-poll principle (§9d) and
  makes every reducer carry version-watch bookkeeping. The kernel already knows exactly when the set
  changed (it owns the executor registry + the authorizer-pointer writes), so it pushes.
- **Determinism (§16c-S3):** the pushed event freezes the new manifest into the log at fold time, same as a
  query result — replay re-folds the identical bytes. The push is just the kernel choosing to inject the
  refresh rather than waiting to be asked; the *content* is a frozen manifest either way.
- **Scope:** a `capabilities-changed` is delivered ONLY to sessions whose projected manifest actually
  changed (a policy change touching family X wakes only sessions that could use X) — the kernel computes
  the projection, so it knows who is affected. Avoids waking every session on every host change.
- **Durable, not a live signal (v-agent-harness):** the push is APPENDED to each affected session's log
  (same §4b reason), so replay sees the upgrade at the same log position — not an out-of-band poke.
- **Coalesce bursts (v-agent-harness):** if the executor registry / policy changes N times in a tick, do
  NOT append N `capabilities-changed` events to every session — coalesce to the net-new manifest (the
  content is a snapshot, so coalescing is sound), else you flood every session's log. The kernel emits at
  most one `capabilities-changed` per session per settle point carrying the final projected manifest.

### 3. Manifest = 3-state grant per family: GRANTED / REQUESTABLE / ABSENT (recommended)

The manifest must represent "the host HAS this but policy DENIES this session" as a first-class, visible
state — not an indistinguishable failure. Each manifest entry:

```
capability-entry {
  family:     string,            // the effect_ct family ("http", "shell", "model", …) — the SAME
                                 //   vocabulary routing + authz key on (v-agent-harness arc). REUSED.
  version:    u32,               // the content-type version (tolerant readers range-check; §9b envelope)
  grant:      grant-state,       // GRANTED | REQUESTABLE | ABSENT (below)
  scope:      resource-scope,    // for GRANTED/REQUESTABLE: the resource predicate the grant is bounded to
                                 //   (mirrors ResourcePredicate: any / exact / one-of / host-in / prefix)
}
```

`grant-state`:
- **GRANTED** — the host has an executor for this family AND policy admits this session (at `scope`). The
  reducer may emit the effect now; it will pass authz for targets satisfying `scope`.
- **REQUESTABLE** — the host HAS an executor, but policy DENIES this session right now. The reducer may
  `request` the grant (capability-request is itself an effect — §9b "read the shortfall, request it",
  delegating down the attenuating spawn tree §4c/§12f). This is what makes "have-but-denied" actionable
  rather than a silent wall. `scope` here = the maximal scope the reducer *could* be granted (informs the
  request).
- **ABSENT** — the host has NO executor for this family at all. Requesting is futile; the reducer must do
  without (or the operator must extend the host — a binary/executor change, §12e). Distinguishing ABSENT
  from REQUESTABLE tells the reducer whether asking for more authority can possibly help.

This makes the capability surface honest: the reducer sees what it can do, what it could ask for, and what
is simply impossible — the three sides of §9b's "effect row IS the manifest", now resource-scoped (SEC-F1)
and policy-projected.

**How the three states are computed (v-agent-harness-host input, LOCKED).** The split is exactly
mechanism × policy, from the two ACTUAL sources — never a hand-maintained third list that could drift:
- **ABSENT vs present** = `CompositeExecutor::handles(family)` (mechanism): no executor → ABSENT.
- **GRANTED vs REQUESTABLE** (both require an executor present) = the authorizer's decision for this
  session on that family.

⚠ **REQUESTABLE-vs-hard-DENY needs a POLICY-MODEL decision (⟨pending operator ratification⟩).** The Cedar
authorizer today returns a single `allow / deny` decision — it does NOT distinguish "denied now but the
session *may request* this" from "hard-forbidden, never grantable." So the manifest can't derive the
REQUESTABLE/hard-DENY line from one decision unless policy ENCODES it. Options (for the operator to pick):
(a) a separate Cedar action `may-request(family)` the probe also checks → REQUESTABLE = `deny(use) ∧
allow(may-request)`, hard-DENY = `deny(use) ∧ deny(may-request)`; (b) a reason-string convention on the
deny; (c) v0 simplification — collapse to two states GRANTED/DENIED (drop REQUESTABLE) and defer the
request-the-shortfall path to a later increment. RECOMMENDED: (a) — it keeps the request path first-class
and stays within Cedar's decide-only model (just another action to decide), but it is a policy-model choice
the operator should ratify. Until ratified, the vertical builds the manifest with the states it CAN derive
(GRANTED where `allow(use)`, otherwise a single denied state) and leaves the REQUESTABLE/hard-DENY split as
a typed hole the policy decision fills.

## Where it lives (seams / file anchors)

- **`wit/reducer.wit`** — additive: under the extensible arc, the `capabilities` query is a well-known
  `control/*` content-type (`effect_ct::CAPABILITIES`), NOT a new effect-kind enum variant — the reducer
  emits it as a content-typed effect request. The `capabilities-manifest` / `capabilities-changed` events
  arrive through the existing `apply` entrypoint as ordinary content-typed events (no new export). The
  `capability-entry` / `grant-state` value types are the guest-side decode of the `capabilities-manifest`
  payload (Cadenza binary-sexpr), matched by `content_type.matches_family("capabilities-manifest")`.
- **`src/effect.rs`** — the manifest is DERIVED by PROBING (see the crux below): add a `CapabilityManifest`
  type (a `Vec<CapabilityEntry>`) + a projection function `project_manifest(families, session_ctx,
  authorizer, executor_registry) -> CapabilityManifest` that, for each family in the canonical
  `effect_ct` set, checks `executor_registry.handles(family)` (mechanism) and calls the authorizer
  (policy). NO authorizer-enumeration API — see the crux.
- **`src/executor.rs`** — the `CompositeExecutor` must expose its registered family set (`fn families(&self)
  -> impl Iterator<Item = &str>` or similar) so the projection can ask "does the host have X". Today the
  registry is private; this is a read-only accessor. (v-agent-harness-host: TRIVIAL — one-liner over the
  `by_kind` map keys; `handles(kind)->bool` already exists.)
- **`src/kernel.rs`** — (a) answer the `capabilities` query effect (build the manifest, freeze it into a
  result event); (b) on an executor-registry change or authorizer/policy-pointer write, compute affected
  sessions + append `capabilities-changed`. This is the reactive push machinery.
- **`src/event_ast.rs` / codec** — the `capabilities-manifest` / `capabilities-changed` payloads are
  Cadenza binary-sexpr like every other payload (opaque to the kernel envelope; the content-type family
  routes them). No kernel-parses-payload coupling.
- **`v-agent-harness-host` (cdz-agent-host)** — owns the two SOURCES: the real executor registry
  (mechanism) and the Cedar authorizer (policy). Provides the `families()` accessor (C) and emits the
  register/swap signal (D). Does NOT gain an enumeration API — the projection probes (below).

### THE CRUX (LOCKED with v-agent-harness-host): projection = host-side PROBE, not authorizer enumeration

The manifest is built by **probing the authorizer over the canonical `effect_ct` family set**, NOT by
asking the authorizer to enumerate a session's grants. Reason: the Cedar authorizer is **decide-only** — its
locked WIT contract (`authorizer.wit`, owned by v-agent-harness) exports exactly one function,
`authorize(auth-request{principal, action, target}) -> decision{allow, reason}`. There is NO "list this
principal's granted actions" entry point, and adding one would enlarge the locked authorizer world AND force
the guest to expose Cedar's entity/policy-store enumeration — a much bigger surface that cuts against the
minimal-immutable-host directive (the authorizer should stay a pure decision function). So:

- The **kernel** (which holds the executor registry + the session's capability context) iterates the
  **finite, canonical `effect_ct` family const set** (`http`/`model`/`shell`/`now`/`timer`/`emit`/… — the
  same const set the extensible-effects slices establish). For each family it (1) checks
  `executor_registry.handles(family)` for the mechanism dimension, and (2) calls
  `authorize(principal=session, action=family, target=scope-probe)` for the policy dimension.
- **"Probe each known family" is complete BY CONSTRUCTION** — the family set is finite and canonical, so
  there is nothing to miss. This is why it is the CLEANER design, not merely a fallback: it keeps the WIT
  contract + guest surface minimal and reuses the existing single `authorize` decision function unchanged.
- **Cost** = N `authorize` calls (N ≈ a dozen), each a fast wasm decision; cacheable per session until
  policy changes. Negligible.
- This means **I3 is NOT a new authorizer API** (as an earlier draft assumed) — it is the host-side probe
  loop using the EXISTING `Authorize` trait / `authorize` WIT function. The `Authorize` trait is unchanged.

### Control-plane partition + result surfacing (LOCKED with v-agent-harness)

`capabilities` / `capabilities-manifest` / `capabilities-changed` are **control-plane**, not world-actions.
Per the extensible-effects design's `control/*` vs `effect/*` partition, they live under `control/*`:
authz-EXEMPT (asking what you may do is not itself a world-action, and gating it would be circular — you'd
need a capability to ask what capabilities you have), and NEVER routed to the `CompositeExecutor` (the
kernel/host answers them in-process, exactly like the `summary` control effect). v-agent-harness confirms:
**`capabilities` is a clean `control/*` member alongside `summary` — the first two control/* families.**

**Result surfacing = a WELL-KNOWN CONTENT-TYPE, NOT a bespoke typed side-channel (v-agent-harness LOCKED).**
- Query effect family = **`effect_ct::CAPABILITIES`** (family string `"capabilities"`, a `control/*` member).
- Manifest result / genesis-seed / push all use the SAME well-known content-type **`capabilities-manifest`
  v1**. It then rides the exact same `EffectResult`/`Inbound` fold path as everything else — the reducer
  matches `content_type.matches_family("capabilities-manifest") && version_in(...)`, reusing the
  tolerant-reader helpers v-agent-harness landed (`matches_family` / `version_in`) — with ZERO new kernel
  plumbing. A typed side-channel would fork the fold path; rejected for that reason.
- **One shape for genesis + query + push (pin with a test).** The genesis-seeded manifest, a query answer,
  and a `capabilities-changed` push MUST all be the same `capabilities-manifest` content-type + the same
  manifest struct, so the reducer folds all three through ONE code path. Do not let genesis emit a bespoke
  shape and the query a different one — pin it: a genesis-seed event and a query-result event fold to the
  same KV shape.

### Sequencing dependency (v-agent-harness): AFTER the register-by-string slice

Capability-discovery is a **consumer** of the `control/*` partition, not a prerequisite. It depends on the
extensible-effects arc reaching the point where routing/authz key on **family strings** and the
`control/*` vs `effect/*` partition (fail-closed unknown-family + control-vs-effect routing) exists — that
is the "register-by-string" slice, which lands after the in-flight ctor-first bridge (beat 1
`EffectRequest::new` landed #1513; beat 2 = v-ah-host literal migration; beat 3 = `content_type` field +
family-keyed authz/routing). So the PM should sequence this vertical **after the family-routing /
register-by-string slices**. I1–I3 (the pure projection + accessor + probe loop) can be built against the
current `EffectKind` family bridge and don't hard-block on register-by-string; only I4+ (the actual
control/* query wiring) needs the partition to exist.

## Increments (top-to-bottom, the way a vertical lands them)

Each slice is independently green + additive (the cdz-kernel discipline: never a bare break; own the crate
gate for both feature sets; tests in-crate).

- **I1 — `CapabilityManifest` + projection by probing (src/effect.rs, no wire yet).** The `CapabilityEntry`
  / `GrantState` types + `project_manifest(families, session_ctx, authorizer, registry) -> CapabilityManifest`
  pure function that, for each family in the canonical set, computes ABSENT (via `registry.handles`) vs
  GRANTED/denied (via one `authorize` probe). **`scope` REUSES the existing `ResourcePredicate`**
  (HostIn/Prefix/Any/Exact/OneOf) — NOT a new scope type (v-agent-harness-host). The states are computed
  from the two ACTUAL sources, never a hand-maintained third list (avoids drift). Unit tests: a granted
  family, a denied family (host has executor, policy denies), an absent family (no executor). No kernel/guest
  change — just the type + the projection, fully testable. (Until the REQUESTABLE policy-model is ratified,
  the denied state is a single "denied" — see the ⟨pending⟩ note; the type leaves room for the split.)
- **I2 — executor registry family accessor (src/executor.rs).** `CompositeExecutor` exposes its registered
  family set read-only (one-liner over `by_kind` keys; `handles(kind)` already exists), so I1's projection
  has a real mechanism source. Tiny; unit test the accessor. (v-agent-harness-host: trivial, will add.)
- **I3 — the host-side probe loop (src/effect.rs or kernel, using the EXISTING `Authorize` trait — NO new
  authorizer API).** Cedar is decide-only (see the crux); the manifest is built by probing the existing
  `authorize` function over the canonical family set — not by an enumeration API (the earlier "enumerate
  grants" framing is DROPPED; the `Authorize` trait / `authorizer.wit` are UNCHANGED). This slice is the
  loop that calls `authorize(session, family, scope-probe)` per family and assembles the manifest via I1's
  projection. Coordinate the scope-probe-target convention with v-agent-harness-host (what `target` to probe
  a family with when the reducer hasn't named a concrete one). No shared-WIT change — much smaller than the
  earlier draft implied.
- **I4 — the `capabilities` control query effect + manifest result event (src/kernel.rs + event_ast +
  wit).** Wire the query: reducer emits `control/capabilities` → kernel builds the manifest via I1's
  projection → freezes it as a `capabilities-manifest` result event → guest folds it. Control-plane:
  authz-exempt, not executor-routed. E2E test with a real (fixture) reducer that queries and folds the
  manifest.
- **I5 — genesis seed (session-factory emits the initial manifest event).** The session-mint path emits a
  `capabilities-manifest` as an early event so the reducer is born knowing. Reuses I4's fold path (same
  content-type, same KV key). Test: a freshly-minted session's first fold sees `sys/capabilities` populated.
- **I6 — reactive `capabilities-changed` push (src/kernel.rs).** On executor-registry change or
  authorizer/policy-pointer write, compute affected sessions (projection delta) + append
  `capabilities-changed` to each. Test: register a new executor for a session that could use it → the
  session receives a `capabilities-changed` folding the upgraded manifest; a session that couldn't use it
  does NOT. This is the mid-session upgrade the operator asked for.
- **I7 (optional / follow-on) — `request` the shortfall.** The reducer, seeing a REQUESTABLE family, emits a
  capability-request effect (§9b/§12f delegation). This may be its own design increment (it touches the
  spawn-tree attenuation model); scope it separately if I1–I6 land first. Noted here so the manifest's
  REQUESTABLE state has a concrete consumer.

## The gate that protects it

- `cargo test -p cdz-kernel` (both default + `live-exec` feature sets — the crate is NOT in `xtask check`;
  own its cargo gate) + `cargo clippy --all-targets --features live-exec -- -D warnings` (grep for
  `warning:`, the io_other_error lesson) + `cargo fmt`.
- I4/I5/I6 each ship an in-crate E2E test driving a fixture reducer through the query / genesis-seed /
  upgrade-push path (the `component_reducer_e2e` pattern).
- The manifest projection (I1) has exhaustive unit coverage of the 3 grant-states — the correctness heart.
- `wit/reducer.wit` changes are additive-only (append variants/fields, never reorder — the frozen-contract
  discipline); any codec/golden touch (event_ast forms for the new events) re-pins consciously.

## ⟨Pending operator ratification⟩ — the short list for the operator's return

The harness owners and I converged on the recommended path above; these are the points where the operator
should confirm or redirect on return (the vertical can start on the parts NOT on this list — see the
hand-off note). None of these block I1–I3 (the pure projection + accessor + probe-loop slices, using the
existing `Authorize` trait unchanged).

1. **The whole shape** — discovery as genesis-seed + query effect, upgrade as a pushed event, manifest as
   a 3-state grant. High confidence, peer-endorsed, but it is the operator's runtime to shape.
2. **Is capability-discovery authz-EXEMPT?** The recommendation treats `control/capabilities` as
   authz-exempt (asking what you may do isn't a world-action, and gating it is circular). This mirrors the
   `summary` control effect's authz-exemption the operator already ruled — but it is a security-surface
   decision worth an explicit nod.
3. **REQUESTABLE visibility** — showing a session capabilities it does NOT hold (so it can request them) is
   a mild information-disclosure choice (a sandboxed reducer learns what the host *could* do). Recommended
   because it enables the §9b/§12f request-the-shortfall path, but the operator may prefer a stricter
   authorized-only manifest for high-isolation sessions (could be a per-session policy flag).
4. **REQUESTABLE-vs-hard-DENY policy model** (surfaced by v-agent-harness-host — the concrete crux of #3).
   Cedar is decide-only (`allow`/`deny`), so the manifest can't tell "denied but requestable" from
   "hard-forbidden" without policy encoding it. RECOMMENDED: add a separate Cedar action `may-request(family)`
   the probe also checks. The operator should ratify the policy model (add `may-request`, a reason-string
   convention, or collapse to two states for v0). Until ratified, the vertical ships GRANTED + a single
   denied state, with the type leaving room for the split.

## Open decisions (with a chosen default)

- **Manifest granularity — per-family only, or per-(family, resource-scope)?** DEFAULT: per-family with a
  single `scope` predicate (mirrors one `Capability`). If a session holds multiple grants for the same
  family at different scopes (e.g. `http` to host A AND host B), the entry's `scope` is the UNION (a
  `one-of` / `host-in` list). A future refinement could list multiple scoped sub-grants per family, but the
  union keeps I1 simple and matches how `HostIn`/`OneOf` already aggregate. Revisit if a reducer needs to
  distinguish which scope came from which grant.
- **Who computes the projection — kernel or authorizer component?** RESOLVED: the KERNEL orchestrates
  (it holds the executor registry + the session's capability context) and PROBES the authorizer's existing
  `authorize` function per family (the crux — Cedar is decide-only, no enumeration). Keeps the kernel
  authz-agnostic (§20b: kernel = mechanism, Cedar = policy) — the kernel never decides grant-state itself,
  it asks the authorizer per family, exactly as it does for a single-effect authorize today.
- **`capabilities-changed` batching.** RESOLVED (v-agent-harness): coalesce — at most one
  `capabilities-changed` per session per settle point, carrying the final projected manifest (the content is
  a snapshot, so coalescing is sound). Prevents log flooding on a burst of registry/policy changes.
- **Control-plane surfacing shape.** RESOLVED (v-agent-harness): a WELL-KNOWN CONTENT-TYPE
  (`capabilities-manifest` v1), not a typed side-channel — it rides the existing fold path + reuses the
  `matches_family`/`version_in` tolerant-reader helpers, zero new kernel plumbing.
- **Scope-probe target convention (I3, open).** When probing `authorize(session, family, target)` for the
  manifest, the reducer hasn't named a concrete `target` — so what target does the probe use? DEFAULT: probe
  with a wildcard/sentinel target and have the authorizer report the admitted scope (or probe the session's
  own held-capability scope). Settle the exact convention with v-agent-harness-host at I3 (it depends on how
  Cedar expresses "admitted for any target of this family").

## Coordination

- **v-agent-harness** owns the extensible content-type arc + the `control/*` vs `effect/*` partition. The
  capability query is a `control/*` member (`effect_ct::CAPABILITIES`); result = well-known
  `capabilities-manifest` content-type; families REUSE `effect_ct`. **Sequencing: this vertical lands AFTER
  the register-by-string / family-routing slices** (it consumes `control/*`, doesn't provide it). Ping
  before I4 — do NOT invent a parallel routing mechanism.
- **v-agent-harness-host** owns the Cedar authorizer (DECIDE-ONLY — no enumeration; the manifest PROBES) +
  the real executor registry (provides the `families()` accessor, I2, trivial) + emits the register/swap
  signal for the mid-session push (I6, D). NO shared-WIT change — the `Authorize`/`authorizer.wit` contract
  is unchanged. Coordinate the scope-probe-target convention (I3) + the register/swap signal shape (I6).

## The control-plane return channel (I4 prerequisite — LOCKED with both harness owners)

I1–I3 (manifest projection, the executor-family accessor, the per-family authorizer probe) are BUILT and
landed. I4 — wiring the `capabilities` query effect + its result event — requires the `control/*` partition
to exist, which requires the extensible-effects **register-by-string** slice. That slice needs a settled
control-plane return-channel shape; this section is that design, agreed in a three-party pass
(`design-host-capabilities` drove; `v-agent-harness` owns the partition + register-by-string;
`v-agent-harness-host` owns the `fork_query` consumer + Cedar authorizer). It governs BOTH control families
that exist so far — `capabilities` (this doc) and `summary` (v-agent-harness's fork-query reshape) — so it is
designed once, here, for both.

### Partition: a `control/` family-string PREFIX, decided BEFORE authorize (LOCKED)

The partition is the single **`control/` family-string prefix**, tested at drive BEFORE authorization: a
family is control-plane iff it `starts_with("control/")`; everything else is a **world-effect family
(bare)**. (**Notation:** `effect/*` elsewhere in this section is shorthand for "a world-action effect
family" — those families are **BARE** (`http`, `model`, …) and do NOT carry an `effect/` prefix. Only
`control/` is a literal prefix; the partition test is purely `starts_with("control/")`.)
- **Control families carry the `control/` prefix** — `control/capabilities`, `control/summary`. They are all
  NEW, so they have no wire history to preserve.
- **Well-known effect families STAY BARE** — `http` / `model` / `shell` / `now` / `timer` / `emit`, NOT
  `effect/http`. ⚠ **Hard wire constraint** (v-agent-harness): the family string is a DURABLE wire value —
  the codec writes the bare family into the append-only log (`event_ast` `kind_atom = kind.family()`) and
  `from_family` routes it back, and it is also the deployed Cedar action name. Renaming the six well-known
  families to an `effect/` namespace would break on-disk log compatibility + every deployed Cedar action-map
  for zero benefit. So the convention is **asymmetric**: control families are prefixed (new), effect families
  are bare (existing, or a future bare world-effect family). No `is_control` flag is needed — the prefix is
  self-describing. (Explicitly namespacing effect families later is a separate wire-migration project; do NOT
  couple it here.)
- **Authz-EXEMPT means SKIPPED, not allow.** A `control/*` effect never reaches the authorizer at all — the
  partition short-circuits before `authorize` (NOT "authorize returns allow"). Rationale (both owners
  concur, §20b): asking what you may do and emitting a summary are not world-actions, emit nothing outward
  (host-captured in-process), and gating `capabilities` would be circular (you'd need a capability to ask
  what capabilities you have). The Cedar authorizer stays a pure world-action (bare-family) gate.

### Disposition: ONE family→`Disposition` registry (LOCKED)

register-by-string's registry is keyed by family string; its value is a disposition enum that uniformly
covers effects and both control kinds:

```
enum Disposition {
    Effect(Box<dyn Executor>),   // bare world-effect family : authorize → CompositeExecutor → EffectResult(token) folds back
    ControlKernel(<handler>),    // control/* K : kernel produces the result INLINE, records EffectResult(token),
                                 //   folds back to the SAME reducer (capabilities → project_manifest)
    ControlHostSurfaced,         // control/* H : returned to drive's caller in Vec<ControlEffect{request,token}>
                                 //   (summary → the fork_query watch captures it + implicit-closes the fork)
}
```

- **capabilities → `ControlKernel(project_manifest)`** — the kernel runs I1–I3's projection inline, records
  an `EffectResult` carrying the well-known `capabilities-manifest` payload + the guest token, and folds it
  back to the requesting reducer through the EXISTING result/resume/token machinery. Purely reducer-facing;
  the host never sees it. (v-agent-harness-host confirmed: nothing needed on its side beyond the landed
  I1–I3 — `handles_family` + the authorizer probe.)
- **summary → `ControlHostSurfaced`** — NOT handled in-kernel; `drive` accumulates it and RETURNS it to its
  caller. (v-agent-harness-host: a returned typed `Vec<ControlEffect{request,token}>` is exactly what
  `fork_query` wants — its loop is already pull-shaped: `fork.deliver_async(...).await` then inspect; it
  swaps today's `kv.get("public/summary")` read for scraping the returned control effects, captures the
  summary payload, and drops the fork. A push/callback would be MORE awkward — it would thread a sink
  through the fork drive. So: `drive` accumulates control-host-surfaced effects across the fold-to-quiescence
  and returns them all in one call; no polling.)
- **`drive` return-shape change:** `drive` (and `deliver_async`) gain a returned `Vec<ControlEffect>` (or a
  richer `DriveOutcome` carrying it) for the host-surfaced control effects collected during the run.
  Effect/* and control/kernel dispositions add NOTHING to the return (they fold back internally); only
  control/host-surfaced accumulates into it. Coordinate the exact `ControlEffect` struct + the `drive`
  signature change with v-agent-harness-host (their `fork_query` is the consumer) when it is built.

### Consistency invariant + fail-closed (LOCKED)

- **Partition and disposition MUST agree** — a `control/`-prefixed family must register a `Control*`
  disposition, and a bare family an `Effect` disposition. A mismatch (e.g. a `control/` family registered as
  `Effect`, or a bare family registered `ControlKernel`) is a **registration-time error**, not a silent
  route. This keeps the namespace prefix and the registry from drifting apart.
- **Fail-closed on the unknown family** — a family not in the registry → **permanent decline / observable
  `Err`**, never a silent drop and never a fallthrough to authorize or route. This is ONE arm covering both
  `effect/<unknown>` and `control/<unknown>` (they share the single registry). It lands in the
  register-by-string slice (below).

### Sequencing (what register-by-string introduces, then I4)

The register-by-string arc (in `cdz-kernel`) introduces, in one pass:
(a) the family-string registration API — `with(family: impl Into<String>, Disposition)` replacing
`with(EffectKind, Executor)` (the peer bridge already scoped with v-agent-harness-host; its ~10 registration
sites migrate behind the signature change, bare effect families, unchanged behavior);
(b) the `starts_with("control/")` partition at drive + authz-exempt routing for control;
(c) the fail-closed unknown-family arm + test;
(d) the `Disposition` enum + the partition/disposition-agreement check.

Then **I4** (this vertical) wires `capabilities` as `ControlKernel(project_manifest)` on top — a small slice
once (a)–(d) exist. The `summary` reshape (v-agent-harness / v-agent-harness-host) wires `summary` as
`ControlHostSurfaced` + re-points `fork_query` off the `public/summary` KV read; that is coordinated
separately but shares this exact channel. Net chain: **[this channel design] → [register-by-string
(a)–(d)] → [I4 capabilities wiring] (‖ summary reshape).**

## I4 detail — the `capabilities-manifest` payload encoding + the reactive half

I4 builds on the register-by-string control-plane foundation in `cdz-kernel` (the `control/` partition in
`drive_worklist_async`, `effect_ct::CAPABILITIES`/`SUMMARY`, the host-surfaced `ControlEffect` return via
`deliver_async_control`, and the projection: `project_manifest` + `CompositeExecutor::handles_family` +
`effect_ct::probe_target`). I4 makes `control/capabilities` KERNEL-ANSWERED-INLINE: in
`drive_worklist_async`'s control-family branch, a `capabilities` arm builds the manifest via
`project_manifest(effect_ct::ALL, |f| executor.handles_family(f), authz, effect_ct::probe_target)`,
serializes it to a `capabilities-manifest` content-typed payload, and `record_result_async`s it so it folds
back to the requesting reducer (NOT pushed to `control_out`). The kernel wiring is a `drive_worklist_async`
edit (`src/kernel.rs`); this section settles the two design pieces it depends on — the payload encoding (a)
and the reactive half (b).

### (a) `capabilities-manifest` payload serialization (Cadenza binary-sexpr — the guest decodes it)

The payload is a Cadenza binary-sexpr value (the shared `cadenza-ast` codec, same as every other payload),
encoded from `CapabilityManifest`. Proposed s-expr form, mirroring the `event_ast` idioms already in the
tree (`Name`/`Int`/`Str`/`Bytes` leaves; `(none)`/`(some …)` optionals; `list` nesting):

```
(capabilities-manifest <version:int>
  (entries
    (entry <family:str> <grant> <scope>)
    …))                                  ; one (entry …) per well-known family, in effect_ct::ALL order

<grant>  = (granted) | (denied) | (absent)         ; a Name-headed nullary list (mirrors GrantState;
                                                   ;   grows to (requestable …) when that policy model lands)
<scope>  = (none) | (some <predicate>)             ; None for Absent/Denied entries
<predicate> =                                       ; mirrors ResourcePredicate, one head per variant
    (any)
  | (exact <str>)
  | (one-of <str>…)
  | (host-in <str>…)
  | (prefix <str>)
```

Notes that make this durable + tolerant-reader-friendly (§9b envelope discipline):
- **Leading `<version:int>` inside the payload** (=1) in ADDITION to the envelope `content_type.version` —
  cheap, and lets a decoder that already holds the bytes range-check without re-reading the envelope.
- **`<grant>` and `<predicate>` are Name-HEADED lists, not bare atoms** — so the `(requestable …)` split
  (the ⟨pending operator ratification⟩ policy model) appends a new head with no wire break, and a predicate
  variant can carry fields. A tolerant guest matches the head and treats an unknown head as "ignore/deny"
  (never decodes garbage — the open-sums posture).
- **`entries` in `effect_ct::ALL` order** (canonical, deterministic) so the encoded bytes are stable →
  content-address-stable → replay-stable (§16c-S3).
- **Guest decode** reverses this into whatever the reducer's language binding is; the kernel treats the
  bytes as opaque (it only produces them here — it does not re-parse them). The genesis-seed, the query
  answer, and the `capabilities-changed` push ALL carry this identical form (the "one shape, pinned with a
  test" rule from the control-plane section).

### (b) The reactive half — genesis-seed (I5) + `capabilities-changed` push (I6)

I4b (kernel-answered inline query) is the reference shape: `project_manifest` → `encode_capability_manifest`
→ a folded `EffectResult` carrying the manifest bytes, logged so replay reads the logged answer. I5 and I6
reuse that exact encode + logged-result shape; they differ only in WHO triggers the emission and WHEN. Both
are **logged, replay-deterministic events** (mirroring I4b) — NOT transient control surfaces — so a
capability the reducer learned survives replay at the same log position.

**I5 — genesis-seed (born-knowing).** Concrete decisions for the open questions:
- **Not the `Genesis` event itself.** `Session::genesis` carries no effects and folds nothing (verified in
  `kernel.rs`), so the seed cannot be an effect the genesis emits. Instead, **immediately after genesis, the
  kernel folds a SYNTHETIC `capabilities-manifest` `EffectResult`** — identical in shape to the I4b answer
  (same `encode_capability_manifest` bytes, folded through `record_result_async`-style machinery) but
  triggered by the kernel at session-birth rather than by a guest `control/capabilities` request. This
  reuses I4b's code path wholesale — the seed IS "the query answer, asked by the kernel on the guest's
  behalf at birth." One shape, one decoder in the guest (the "one shape, pinned with a test" rule).
- **Always seed, do NOT condition on whether the guest reads it.** The cost is one projection + one small
  logged event at birth — negligible — and conditioning on guest behavior would (a) require the kernel to
  predict guest reads (it can't; the guest is opaque) and (b) break the born-knowing guarantee. Always-seed
  keeps `sys/capabilities` populated before the first substantive fold, deterministically.
- Seeded manifest = `project_manifest` over the session's initial caps + the executor registry as of birth.

**I6 — `capabilities-changed` push (mid-session upgrade).** Concrete decisions:
- **Shape = a logged `capabilities-changed` event, mirroring I4b** (v-agent-harness's lean, concurred):
  DURABLE (appended to the log, not a transient control surface — §4b), so replay sees the upgrade at the
  same position and folds the identical bytes. Same `encode_capability_manifest` payload as I4b/I5.
- **The mutation HOOK is DESIGN-AHEAD; there is NO in-kernel mutable executor/policy registry to hook.**
  Stable constraint (not a transient current-state note): the kernel holds no executor set or authorizer of
  its own — `deliver_async` takes ONE `executor: &mut (impl Executor)` value (in production a
  `CompositeExecutor` that routes to many leaf executors by family) + `authz: &(impl Authorize)`, supplied
  by the caller PER CALL, and mechanism/policy are whatever that call passes. So there is nothing
  in-kernel for an I6 trigger to observe changing. The reactive trigger therefore attaches to whatever
  DURABLE MUTATION §20b introduces (a policy-log append that repoints
  the Cedar-engine/policy pointer, or a delegated-grant landing) — it should be built WHEN that mutation
  path exists, keyed off the same policy-log append that changes what a probe would return. ⚠ Do NOT
  fabricate a mutation hook I6 fires on before the mechanism/policy actually becomes mutable at runtime;
  I6 is correctly sequenced AFTER the runtime-mutable-policy work (§20b), not after I4b. (Flag to keep the
  vertical honest: I5 is buildable now on I4b's shape; I6 waits on runtime-mutable policy/mechanism.)
- **When it does fire:** on a mutation, the kernel RE-PROJECTS each affected live session and, only if that
  session's manifest actually CHANGED, appends one `capabilities-changed`. **Coalescing rule = one push per
  drive-quiescence** (not per-mutation): a burst of mutations that settles before the next fold yields a
  single push carrying the final manifest — bounded log growth, and the reducer only ever sees net-new state.
- **Delivered only to sessions whose projection changed** — the kernel computes the projection, so it knows
  which sessions a given policy/mechanism change affects; unaffected sessions get nothing. Polling is
  rejected (anti-§9d).

**Sequencing:** I4b LANDED (query answer, exercises the encode + fold-back). **I5 (genesis-seed) is next and
buildable now** — it reuses I4b's exact path, triggered at birth. **I6 (the push) is gated on §20b
runtime-mutable policy/mechanism** — its trigger hook has nothing to fire on until then; spec is ready so it
drops in when that path exists.

## Related design context

Related design context, all in `design/agent-harness-kernel.md`: §3 (genesis + context-as-events), §4b
(bridge rule — the load-bearing constraint), §9b (effect-row-as-manifest), §9d (reactive execution),
§12c/§12f (three-way authz + attenuating delegation), §20a/§20b (resource-rescoping components +
Cedar-as-a-component). The extensible content-type / `control/*` partition arc this consumes is owned by
`v-agent-harness`; see the `effect_ct` family consts in `implementation/seed/crates/cdz-kernel/src/effect.rs`.
