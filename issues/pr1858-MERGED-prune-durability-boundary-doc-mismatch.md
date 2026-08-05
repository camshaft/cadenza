# PR #1858 review comments — cdz-kernel/src/{kernel,name_store}.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1858 (MERGED — the #1852 applied_set_keys prune fix). Doc-vs-
durability-boundary mismatch on the prune.

## Prune doc says keys drop once EffectResult is "durably recorded", but append() can latch persist_error (not durable) (Copilot, kernel.rs:694 + name_store.rs:223, also :464) — doc/accuracy
> The comment says the EffectResult is "durably recorded", but `append()` can latch `persist_error` and
> skip durable writes (including for this EffectResult). The prune happens after the IN-MEMORY session-log
> record, which may not be durable if persistence has latched. Wording should match the actual boundary.
The #1852 prune (dropping applied_set_keys once the EffectResult is recorded) is correct for the common
path, but the doc overclaims the DURABILITY boundary: the prune fires after the in-memory record, and
`append()` can latch persist_error (non-durable). So on a persist-latched path the key is pruned before a
DURABLE EffectResult exists — a subtle re-drive-safety edge (if recovery re-reads a non-durable log). At
minimum reword the doc to the actual boundary (pruned after the in-memory record; durability is separate);
better, confirm the prune-vs-persist-latch ordering can't reintroduce the #1852/#1844 re-apply on a
latched-then-recovered path. LOW-MED (doc + a durability-edge to confirm). Fix-forward. (3 sites: kernel.rs
:694, name_store.rs:223 + :464.)
