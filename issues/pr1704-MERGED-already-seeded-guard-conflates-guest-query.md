# PR #1704 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1704 (MERGED — make seed_capabilities_async idempotent, the fix
for my #1687 seed-once finding). The idempotency guard is good; Copilot flags its detection logic conflates
two sources — worth more than the "misleading wording" framing.

## `already_seeded_capabilities` can't distinguish kernel-seed from guest-issued capabilities query (Copilot, kernel.rs:413) — correctness (framed as doc) [VERIFIED]
> The doc for `already_seeded_capabilities` claims the seed is the ONLY source of a
> `control/capabilities` `Dispatched` frame the kernel originates, but `drive_worklist_async` ALSO appends
> a `Dispatched` frame for guest-issued `control/capabilities` effects (answered inline). Misleading; the
> helper really detects "a capabilities query has already been dispatched (seeded OR guest-issued)".

VERIFIED against trunk: `already_seeded_capabilities` (kernel.rs:414) = `self.log.iter().any(|e| matches!
(e.body, Dispatched{family,..} if family == CAPABILITIES))`. But the guest-issued inline path
(kernel.rs:559-568) appends the SAME `Dispatched{ family: req.content_type.family (= "control/
capabilities") }`. So the guard cannot tell "kernel seeded" from "a guest already queried capabilities".

The substance is more than wording: if a guest capabilities query is ever dispatched BEFORE
`seed_capabilities_async` runs, the guard sees the frame and SKIPS the real seed. Today that's guarded by
"seed immediately after genesis" ordering, so it's latent — but the guard's stated invariant is wrong and
would silently mis-fire if seed-ordering changes. RECOMMEND v-agent-harness either (a) distinguish the
seed's own dispatch (a dedicated marker / cause=genesis check) so idempotency keys on the SEED not any
capabilities dispatch, or (b) if "detect any capabilities dispatch" is genuinely the intended idempotency
semantics, reword the doc to say so AND confirm a pre-seed guest query is impossible-by-construction.
LOW-MED — fix-forward; verify the ordering guarantee before downgrading to doc-only.
