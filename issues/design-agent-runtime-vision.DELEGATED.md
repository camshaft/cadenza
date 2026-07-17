# Vertical-ready brief: the log-native agent runtime (agent OS)

**Design landed:** `implementation/design/DESIGN-agent-runtime-vision.md` (merge-request `113c145a9`
sent to pr-sync 2026-07-16; docs-only). Shaped live with the operator ("build hivemind here").

**Owner suggestion:** this is v-agent-harness's domain — they own the IMPLEMENTATION side of the
agent-runtime and shipped Inc 0–3 (the Cadenza agent loop, the Bedrock embedder, the Cedar authorizer),
which the new vision names as **L0** of its ladder. The vision doc SUPERSEDES the ambition ceiling of
their `DESIGN-agent-harness.md` but is fully grounded in that doc's shipped reality + 4 hard
constraints. So: **assign to v-agent-harness** (area = agent-runtime) rather than mint a new vertical —
or, if the operator wants the vision-scale work owned separately from the increment-hardening, a new
`area=agent-runtime` vertical that coordinates tightly with v-agent-harness.

**Subsystem:** `implementation/agent-harness/` (Cadenza package) + `implementation/seed/crates/cdz-agent`
(Rust embedder/driver) + a new microkernel crate (the fold owner) when L1 starts. Leans on: v-effects
(capability=effect-type), v-peer-linking (String-ABI Route A), v-verification (proof-gated governance),
the compiler-port vertical (compiler-as-tool + the lazy query DB that becomes the log index),
v-metaprogramming (author-tools-as-data).

**First increment (per §15 ladder):** **L1 — the fold owner over a real log.** A single-threaded Rust
owner that tails a DynamoDB log, folds it with a Cadenza program, and drives one agent loop end-to-end
(reusing the shipped embedder for the model call). Proves the microkernel shape + recorded-effect
determinism (`RunOpts::host_responses`) against a real log. This is the smallest rung that demonstrates
the "minimal core = tail → fold → execute effect-requests" thesis.

**The 6 operator decisions already locked (see §16):** type-only capability (compiler is the sandbox);
operator-genesis governance + HOL proof-gate on capability EXPANSION; single leased fold-owner; log-keyed
compaction (degrades to cache-miss); DynamoDB many-writer write plane decoupled from the single fold;
messaging first-class. Self-mod ceiling for the first cut (author-tools vs rewrite-own-loop) is the one
call to confirm with the operator when L6 approaches.

**Open leaf-level (don't block L1):** snapshot cadence for owner failover re-fold; the subscription
predicate language; S3-vs-query-DB split for the body tier at scale.
