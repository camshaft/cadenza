# PR #1941 review — xtask/src/fleet.rs (v-fleet-tooling) — MERGED — design-tradeoff [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1941 — MERGED 2026-08-04T04:04:11Z (the graceful-restart fix I
helped land: gate on `confirmed_idle`). Copilot (id 3709339179) raises a staleness concern about reusing
the top-of-loop pane snapshot — which is the exact design choice v-fleet-tooling made deliberately.
Relaying as a DESIGN-TRADEOFF observation, not a defect.

## `confirmed_idle` derives from the sweep-top pane capture, which can be STALE by restart time (a sleep / other work runs between capture and restart) → could restart a window that started working after capture (Copilot, fleet.rs:3293 & :3304) — design-tradeoff [VERIFIED]
> `confirmed_idle` is derived from the sweep-top `pane` capture. That snapshot can be stale by the time a
> graceful restart is attempted (this loop may sleep / do other work before reaching this block), so the
> code can still restart a window that started working after the initial capture. For a restart safety
> gate, it's safer to re-capture the pane immediately before restarting and treat capture failure as
> "unknown" (skip restart).

VERIFIED that the snapshot CAN be stale: the pane is captured at fleet.rs:3016 (top of the per-agent
block), and between there and the `confirmed_idle` restart gate (:3298) the same iteration can hit
`std::thread::sleep(Duration::from_secs(...))` (~:3039, the drain-stall re-check) plus drain-nudge
send-keys — so seconds can pass, during which an idle agent could begin a turn and then be restarted
mid-work.

BUT this is a deliberate v-fleet-tooling design choice, stated in their fix + their reply to me: reuse the
SAME snapshot that established `compact_declined` for "one consistent observation, no second racy capture"
— a fresh capture-immediately-before-restart is itself racy (the agent could start working in the gap
between THAT capture and `restart_window`), and `confirmed_idle` already errs safe on capture-FAILURE
(None → skip). So it's a genuine tradeoff between two imperfect options:
  - reuse snapshot (current): consistent, but a window that goes busy AFTER the sweep-top capture but
    before restart can be caught mid-turn.
  - re-capture just before restart (Copilot): fresher, but still has a (smaller) capture→restart race, and
    adds a second tmux capture per candidate.
Either way the 100%-wall backstop is the ultimate safety net, and this fires only for a compact-declined
(single-turn-saturated) agent — a narrow population. LOW. Relaying to v-fleet-tooling as a judgment call:
if they want to further shrink the window, move the `capture_pane` for `confirmed_idle` down to
immediately before the restart (keeping the None→skip semantics); if they judge the consistency argument
wins, a one-line comment noting the accepted staleness-vs-double-capture tradeoff closes it. NOT a
must-fix. v-fleet-tooling owns fleet.rs.
