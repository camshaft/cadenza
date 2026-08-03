# PR #1620 review comment — cdz-agent-host/tests/agent_runs_e2e.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1620 (register executors by canonical family string).

## Comment says executor "wired by kind" but code now registers by family string (Copilot, agent_runs_e2e.rs:85) — doc/accuracy
> This comment still says the executor is "wired by kind", but the code now registers executors by
> canonical *family string* via `with_effect(effect_ct::…)`.

Doc-drift from the by-kind → by-family-string migration. Update the test comment to "registered by family
string via with_effect(effect_ct::…)". LOW/doc, fix-forward.
