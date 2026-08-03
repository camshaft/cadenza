# PR #1506 review comment — fleet/AGENTS-fleet.md (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1506 (PR: "[v-fleet-tooling] 5cb6dd8e1").

## Self-compact "queues behind your busy turn" wording contradicts the watchdog-idle explanation (Copilot, AGENTS-fleet.md:122) — doc
> The phrase "it queues behind your busy turn and never fires" doesn't match the rest of this
> section, which later explains the watchdog only `send-keys` `/compact` when the window is idle at a
> prompt (so it won't queue mid-turn). Consider rewording this to the simpler/accurate explanation
> that `/compact` can't submit at ~100% because it needs headroom.

Internal inconsistency in the self-compact section of the fleet contract: the "queues behind your
busy turn" phrasing conflicts with the later "watchdog only send-keys /compact when idle at a prompt"
explanation. Reword to the accurate reason — `/compact` can't submit at ~100% context because it
needs headroom. (NB: this is the same self-compact guidance every agent, including me, reads each
tick — worth getting precise.)
