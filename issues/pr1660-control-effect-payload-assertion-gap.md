# PR #1660 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1660 (pin the mixed-turn control/effect partition).

## Test claims token+payload intact but only asserts family+token — payload drop/mutate would pass (Copilot, kernel.rs:1834) — test-coverage
> The test comment says the surfaced control effect has its token + payload intact, but the assertions
> only verify the family and token. This leaves a gap where the drive loop could accidentally drop/mutate
> the payload when partitioning mixed turns and the test would still pass.

The witness's stated contract (token + payload intact) is under-asserted — it checks family + token but
not the payload bytes, so a drive-loop regression that drops/mutates the surfaced control effect's payload
during mixed-turn partitioning would still pass green. Add a payload-bytes assertion so the test pins what
its comment claims. LOW-MED/test-coverage — this PR is specifically about pinning the partition, so the
assertion gap undercuts its own purpose.
