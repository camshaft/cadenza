# Vertical-ready brief: host capability discovery + mid-session upgrade

**Design doc:** `implementation/design/DESIGN-host-capability-discovery.md` (LANDED on trunk at 0eb0064ea;
a cosmetic review-fix follow-up 19db7eafd is queued). PROPOSAL, peer-reviewed by v-agent-harness +
v-agent-harness-host. It is a PROPOSAL, not a final decision — operator away all week; 4 forks are flagged
⟨pending operator ratification⟩ but do NOT block the buildable increments (I1–I3).

**Subsystem / area:** `cdz-kernel` (agent-harness). Suggest a vertical with `area=agent-harness` (or a
sub-agent under v-agent-harness). Shared surfaces with **v-agent-harness** (extensible content-type /
`control/*` partition) and **v-agent-harness-host** (Cedar authorizer + real executor registry) —
coordinate, don't fork parallel mechanisms.

**What it is:** a reducer needs to know which effect families the host can serve, at what resource scope,
and whether policy will let it — and must be able to UPGRADE mid-session when the host gains a capability.
Three forks decided with the operator: (1) discovery = BOTH a genesis-seeded initial manifest AND a
`capabilities` query effect; (2) mid-session upgrade = a PUSHED `capabilities-changed` event (reactive,
no polling); (3) the manifest is a 3-state grant per family — GRANTED / REQUESTABLE / ABSENT — resource-
scoped (SEC-F1) and policy-projected. Reuses the `effect_ct` family vocabulary.

**Sequencing (v-agent-harness):** this vertical is a CONSUMER of the `control/*` partition — sequence it
AFTER the extensible-effects register-by-string / family-routing slices land (beat 3: content_type field +
family-keyed authz/routing). I1–I3 (pure projection + accessor + probe loop) can be built against the
current `EffectKind` family bridge; only I4+ (control/* query wiring) needs the partition to exist.

**First increment (I1):** `CapabilityManifest` + `project_manifest(...)` pure function in
`src/effect.rs` — `CapabilityEntry` / `GrantState` types (scope REUSES the existing `ResourcePredicate`,
NOT a new type) + the projection that, per canonical `effect_ct` family, computes ABSENT (via
`CompositeExecutor::handles`) vs GRANTED/denied (via ONE `authorize` probe — Cedar is decide-only, NO
enumeration API; the `Authorize` trait is UNCHANGED). Unit-test the grant-states. No kernel/guest wire
change yet — fully testable in isolation. See the doc's Increments section for I2–I7. NOTE: the
REQUESTABLE-vs-hard-DENY split is a ⟨pending operator ratification⟩ policy-model item — I1 ships GRANTED +
a single denied state, with the type leaving room for the split.

**Gate:** `cargo test -p cdz-kernel` (both default + `live-exec`) + `cargo clippy --all-targets
--features live-exec -- -D warnings` (grep for `warning:`) + `cargo fmt`. Not in `xtask check` — own the
crate gate. wit changes additive-only; codec/golden touches re-pinned consciously.
</content>
