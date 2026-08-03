# Host capability discovery + mid-session capability upgrade

Owner: TBD (design by `design-host-capabilities`, for a `v-agent-harness`-area vertical). Status:
DESIGNED (operator idea via concierge 2026-08-03; three forks decided with the operator this session).
Subsystem: `cdz-kernel` (+ its guest ABI `wit/reducer.wit`), coordinated with `v-agent-harness` (the
extensible content-type / `effect_ct` family arc) and `v-agent-harness-host` (the executor registry +
Cedar authorizer that PRODUCE the manifest).

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

## The three decided forks

### 1. Discovery = BOTH genesis-seed AND a `capabilities` query effect (decided)

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

### 2. Mid-session upgrade = a PUSHED `capabilities-changed` event (decided; polling rejected)

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

### 3. Manifest = 3-state grant per family: GRANTED / REQUESTABLE / ABSENT (decided)

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

## Where it lives (seams / file anchors)

- **`wit/reducer.wit`** — additive: a `capabilities` variant on the effect-request kind (or, under the
  extensible arc, a well-known `capabilities` content-type in the `control/*` partition — see the note
  below), plus the `capability-entry` / `grant-state` / `resource-scope` value types the guest folds. The
  `capabilities-manifest` and `capabilities-changed` events arrive through the existing `apply` entrypoint
  as ordinary content-typed events (no new export).
- **`src/effect.rs`** — the manifest is DERIVED from `Capability` (already resource-scoped) + the
  executor registry; add a `CapabilityManifest` type (a `Vec<CapabilityEntry>`) + the projection function
  `project_manifest(session_caps, authorizer, executor_registry) -> CapabilityManifest`.
- **`src/executor.rs`** — the `CompositeExecutor` must expose its registered family set (`fn families(&self)
  -> impl Iterator<Item = &str>` or similar) so the projection can ask "does the host have X". Today the
  registry is private; this is a read-only accessor.
- **`src/kernel.rs`** — (a) answer the `capabilities` query effect (build the manifest, freeze it into a
  result event); (b) on an executor-registry change or authorizer/policy-pointer write, compute affected
  sessions + append `capabilities-changed`. This is the reactive push machinery.
- **`src/event_ast.rs` / codec** — the `capabilities-manifest` / `capabilities-changed` payloads are
  Cadenza binary-sexpr like every other payload (opaque to the kernel envelope; the content-type family
  routes them). No kernel-parses-payload coupling.
- **`v-agent-harness-host` (cdz-agent-host)** — the Cedar authorizer produces the GRANTED/REQUESTABLE
  decision per family; the real executor registry produces the mechanism set. Coordinate the projection
  seam (who computes it: the kernel calls the authorizer component with "enumerate my grants" rather than
  only "authorize this one effect" — a new authorizer query, additive to the `Authorize` trait).

### Control-plane partition (coordinate with v-agent-harness)

`capabilities` / `capabilities-manifest` / `capabilities-changed` are **control-plane**, not world-actions.
Per the extensible-effects design's `control/*` vs `effect/*` partition, they should live under `control/*`:
authz-EXEMPT (asking what you may do is not itself a world-action, and gating it would be circular — you'd
need a capability to ask what capabilities you have), and NEVER routed to the `CompositeExecutor` (the
kernel/host answers them in-process, exactly like the `summary` control effect). This must be coordinated
with `v-agent-harness`, who owns the control/effect partition shape — the capability query is a second
concrete `control/*` member alongside `summary`, so it validates that partition's generality.

## Increments (top-to-bottom, the way a vertical lands them)

Each slice is independently green + additive (the cdz-kernel discipline: never a bare break; own the crate
gate for both feature sets; tests in-crate).

- **I1 — `CapabilityManifest` + projection (src/effect.rs, no wire yet).** The `CapabilityEntry` /
  `GrantState` / `ResourceScope` types + `project_manifest(...)` pure function over a session's caps + a
  supplied executor-family set + an authorizer. Unit tests: a granted family, a requestable family (host has
  executor, policy denies), an absent family (no executor). No kernel/guest change — just the type + the
  projection, fully testable.
- **I2 — executor registry family accessor (src/executor.rs).** `CompositeExecutor` exposes its registered
  family set read-only, so I1's projection has a real mechanism source. Tiny; unit test the accessor.
- **I3 — authorizer "enumerate grants" query (src/authz.rs + `Authorize` trait, coordinate v-ah-host).**
  Additive method: given a session's capability context, return the per-family GRANTED/REQUESTABLE decision
  (not just the single-effect authorize). The Cedar component grows the corresponding query; the interim
  in-kernel authorizer implements it over the `Capability` set. This is the shared-surface step — ping
  v-agent-harness-host before landing (they own the Cedar authorizer).
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

## Open decisions (with a chosen default)

- **Manifest granularity — per-family only, or per-(family, resource-scope)?** DEFAULT: per-family with a
  single `scope` predicate (mirrors one `Capability`). If a session holds multiple grants for the same
  family at different scopes (e.g. `http` to host A AND host B), the entry's `scope` is the UNION (a
  `one-of` / `host-in` list). A future refinement could list multiple scoped sub-grants per family, but the
  union keeps I1 simple and matches how `HostIn`/`OneOf` already aggregate. Revisit if a reducer needs to
  distinguish which scope came from which grant.
- **Who computes the projection — kernel or authorizer component?** DEFAULT: the KERNEL orchestrates
  (it holds the executor registry + the session's capability context) and CALLS the authorizer component
  for the per-family GRANTED/REQUESTABLE decision (I3). Keeps the kernel authz-agnostic (§20b: kernel =
  mechanism, Cedar = policy) — the kernel never decides grant-state itself, it asks the authorizer, exactly
  as it does for a single-effect authorize today.
- **`capabilities-changed` batching.** DEFAULT: one event per change is fine for v0 (changes are rare —
  executor registration, policy swap). If a burst of changes lands, the kernel MAY coalesce into one event
  carrying the final manifest (the frozen content is a snapshot, so coalescing is sound). Not a v0 concern.
- **Control-plane surfacing shape.** Depends on v-agent-harness's `control/*` return-channel decision (a
  typed control-plane channel vs. a well-known content-type) — the capability query rides whatever that arc
  lands. Coordinate; don't fork a parallel mechanism.

## Coordination

- **v-agent-harness** owns the extensible content-type arc + the `control/*` vs `effect/*` partition. The
  capability query is a `control/*` member; the manifest families REUSE `effect_ct`. Ping before I4 (the
  control-plane surfacing shape) — do NOT invent a parallel routing mechanism.
- **v-agent-harness-host** owns the Cedar authorizer + the real executor registry. I3 (the "enumerate
  grants" authorizer query) is a shared-surface, additive `Authorize`-trait change — coordinate before
  landing, same seam discipline as the async-trait arc.

Related design context: `design/agent-harness-kernel.md` §3 (genesis), §4b (bridge rule — the load-bearing
constraint), §9b (effect-row-as-manifest), §9d (reactive), §12c/§12f (three-way authz + delegation),
§20a/§20b (rescoping components + Cedar-as-component). Memory: `agent-harness-v2-kernel-design-and-v0-plan`,
`agent-harness-extensible-effects-and-summary-effect-design`.
</content>
