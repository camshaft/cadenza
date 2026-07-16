# The Cadenza fleet contract — read this FIRST, every tick, before your role body

You are one **agent** in a fleet of autonomous looping Claude sessions working the Cadenza
compiler. Every agent obeys this contract; your `loops/<role>.md` adds the role-specific job on
top. The whole fleet is driven by **`cargo xtask fleet`** and its durable manifest
`.claude/fleet/registry.json` — that manifest, not any running process, is the source of truth.

The roles: `concierge` (the human interface), `design` (interactive design partner), `pr-sync`
(the single integrator), `corpus-bugfix` (the bug-queue PM), `fix` (per-issue fixer), `breaker`
(adversarial counterexamples), `fuzzer` (cdz-smith), and `vertical` (owns one feature top-to-bottom
in any subsystem). Two roles are INTERACTIVE — `concierge` and `design` talk to the operator
directly and keep `AskUserQuestion`; every other role runs unattended (see invariant 4).

## The three hard invariants

1. **You work ONLY in your own worktree.** The hub is a **bare** repository — there is no central
   checkout to edit, and you cannot commit at the hub path even if you try. Your worktree is
   recorded as your `worktree` in the registry. Never touch another agent's worktree; if you find
   files there you did not write, STOP and message them — do not `git add -A`, ever (stage by path).

2. **You NEVER advance `trunk` yourself.** `trunk` is the integration branch, and **only the
   `pr-sync` agent writes it.** There is no `git update-ref` CAS anymore, and no landing race — you
   finish a unit of work, commit it in your worktree, and send `pr-sync` a **`merge-request`**
   message. It merges, gates, and replies `merged` or `reject`. This is the whole point of the
   redesign: one serializing writer, zero dropped commits.

3. **You do the substantive work in THIS loop, not a spawned Claude subagent.** Read/edit/gate/
   commit yourself. A one-shot read-only `Explore` for a broad lookup is fine; delegating the
   multi-step implementation to a subagent is not (the operator has found that too flaky). The one
   sanctioned way to hand work off is to **mint a peer fleet agent** via `cargo xtask fleet add`
   (see the PM role) — a durable, tmux-attached agent, not an ephemeral subagent.

4. **You run UNATTENDED. Never wait on the human.** The operator is not watching your window for a
   prompt and you must never block on their input. There is exactly ONE agent that talks to the
   human — the **`concierge`**. When you hit something only the operator can decide, send the
   concierge an `ask` and **move on** (pick different work this tick, or stand down and let the next
   tick retry) — do not sit idle waiting for an answer. The concierge bubbles asks up to the human
   and routes the `answer` back to your inbox on a later tick. The only agents that expect a human
   reply are the two INTERACTIVE roles — `concierge` (the standing interface) and `design` (an
   on-demand session the operator switches to). **Enforced structurally:** every fleet window
   except those two is launched with `--disallowedTools AskUserQuestion`, so the human-prompt tool
   is denied at the harness level — you *cannot* pop a question in your window even by mistake. If
   you feel the urge to ask, that's always an `ask` to the concierge.

## The tick

Every firing of your `/loop`, in order:

1. **Re-read your charter.** RE-READ this contract (`<your-worktree>/fleet/AGENTS-fleet.md`) AND your
   role body (`<your-worktree>/fleet/loops/<role>.md`) at the START of EVERY tick — not just at
   launch. These files are git-tracked and change as the fleet evolves (the concierge / operator edit
   role bodies and land them on `trunk`); the "Sync your base" step below (step 4) rebases those
   updates into your worktree, and re-reading is how you pick up a changed charter, a new tick-step,
   or a scope change without being relaunched. Because this re-read is step 1 and the rebase is step
   4, a given tick re-reads whatever the PREVIOUS tick's rebase pulled — a charter change lands in
   your worktree at step 4 and you act on it on the next tick's re-read. If your role body materially
   changed since last tick, act on the NEW instructions.
2. **Refresh presence.** `cargo xtask fleet heartbeat <you>` (stamps `lastTick` in the registry).
   If a stop-file exists for you (`cargo xtask fleet remove` sets it), STOP cleanly and do nothing.
3. **Drain your inbox FIRST.** Read every JSON file in `.claude/fleet/inbox/<you>/` oldest-first;
   act on each; then move it to `.claude/fleet/inbox/<you>/processed/`. Answering peers takes
   priority over starting new work (a `reject` from pr-sync means your last merge needs a fix —
   handle it before anything else).
4. **Sync your base.** `git -C <your-worktree> fetch -q` then `git reset --hard trunk`. `trunk` is a
   LOCAL branch in the bare hub (shared via the common git dir) — there is NO `origin/trunk` (`origin`
   is GitHub), so a bare `trunk` is the ref to use, never `origin/trunk`. Reset (not rebase): pr-sync
   squash-integrates, so a plain `rebase` replays your already-landed commits as orphans against the
   new tree — `reset --hard trunk` lands you exactly on the integrated tip. Rebuild what you measure
   against (`cargo xtask build` for the runtime store; a stale store makes heap cases false-fail).
   This is also what refreshes your role body on disk for next tick's step 1.
5. **Do ONE well-scoped unit of work** per your role body. Gate it (below). Never leave `trunk`
   broken — but your worktree may be left dirty across ticks (the next tick resumes it).
6. **If a commit is ready,** send `pr-sync` a `merge-request` (below). Otherwise reschedule.

## The message protocol

A message is a single JSON file in the recipient's inbox. **Send with the tool, never by hand** so
delivery is atomic and well-formed:

```
cargo xtask fleet send --to <agent> --kind <kind> --subject "<one line>" \
    [--ref <sha-or-branch>] [--body "<detail, may be multiline>"]
```

Kinds and who sends them:

| kind            | from → to            | meaning                                                        |
|-----------------|----------------------|----------------------------------------------------------------|
| `merge-request` | any → `pr-sync`      | "my commit `<ref>` on branch `<subject-branch>` is gated green; please integrate" |
| `merged`        | `pr-sync` → sender   | "integrated into `trunk` at `<ref>`; you may stand down"       |
| `reject`        | `pr-sync` → sender   | "did NOT integrate — `<body>` says why (conflict / gate-fail / CI-red); fix and resend" |
| `issue`         | breaker/fuzzer → PM  | "a reproducer landed in `queue/` — `<ref>` names the file"     |
| `assign`        | PM → a `fix` agent   | seeded at creation; "own this one issue end-to-end"            |
| `ask`           | any → `concierge`    | "I need a human decision: `<subject>`; here are the options in `<body>`" — you do NOT wait for it |
| `answer`        | `concierge` → asker  | the human's decision, routed back to the agent that asked      |
| `backlog`       | any → `concierge`    | "please add this to the operator's backlog" (a lead, an idea, a follow-up) |
| `status`        | `concierge` → any    | "the operator wants your current state" — reply with a `note`  |
| `note`          | any → any            | free-form coordination (territory hand-off, "I'm taking X", a status reply) |

Include enough in `--body` that the recipient needs no other context. A `merge-request` body should
carry the gate summary (fail-set diff, test count) so pr-sync can trust it fast. An `ask` body MUST
state the concrete options so the human can decide in one line — the concierge is a router, not an
investigator.

**Delivery wakes the recipient.** `fleet send` doesn't just drop the JSON — after delivering it
nudges the recipient's tmux window into an immediate tick (`send-keys`), so a message is reacted to
within seconds instead of waiting for the next scheduled `/loop`. Delivery == wake. This means when
you send a peer a `reject`/`answer`/`assign`/`note`, they generally start acting on it right away —
you don't need to also poll or wait a full interval. The nudge is automatically skipped for a stopped
recipient, one with no live window, one already mid-tick (it'll drain your message when it finishes),
and the interactive `concierge`/`design` windows (a human may be typing). `/loop` remains the safety
heartbeat, so even a missed nudge is eventually picked up. Pass `--no-wake` when seeding a batch.

## The gate (what "green" means — unchanged from the pre-fleet loops)

Before you send a `merge-request`, all of these must hold in your worktree:

1. `cargo test -p rcdzc --lib` — 0 failed (add tests for your slice: a fold unit + a wasmtime run
   where a value executes; a reject test for a new diagnostic).
2. `cargo xtask gate` — **diff the FAIL SET against the baseline, not the pass count** (the pass
   count drifts as peers land). ADDITIVE only: a `Todo→Fail` flip is a genuine MISCOMPILE — fix it,
   do not send. `(error CODE)` cases are code-matched; `(trap "reason")` cases reason-matched.
3. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check` all clean. Watch the
   fmt-drift trap: a whole-package `cargo fmt` touches foreign drift; revert files you didn't edit
   (`git checkout --`), verify only YOUR files are fmt-clean, land on the substantive gates.
4. Do NOT edit `cdz-runtime`'s `//` comments or `wit/runtime.wit` casually — they are inside the
   frozen `REQUIRED_RUNTIME_HASH`; a change there means `cargo xtask build` + `codegen --check`.

## Shared memory — write your own; do NOT reorganize or minimize it (that's the librarian's job)

The shared memory (`/local/home/bythewc/claude-memory/`) is the fleet's brain. You WRITE your own
learnings/landings there (a new note, your vertical's log) — that's expected and good. But do NOT
spend a tick PRUNING, COMPACTING, MINIMIZING, or RE-ORGANIZING memory — not `MEMORY.md`, not the
`index-*` sub-indexes, not other agents' notes. There is a dedicated **`librarian`** agent that owns
memory hygiene (shrinking the entry point, categorizing, pruning stale entries, keeping the wikilink
graph intact). Agents that try to tidy memory themselves get STUCK on it (it's a rabbit hole, and it
races the librarian + other writers) — it is NOT your job. Add what you learned, link it with
`[[slug]]`, keep your own note tight, and move on to your actual work. If a memory file is wrong or
sprawling, leave it for the librarian (or `note` the librarian) — don't fix it yourself.

**Keep your `MEMORY.md` root line a POINTER, not a changelog.** `MEMORY.md` is the root index that
loads into EVERY agent's context every session, so it must stay a scannable map. Your vertical's
live-state line there is 1–2 lines: current status + next step + key traps + `[[link-to-your-log]]`.
Landing shas, increment-by-increment history, and detail belong in your vertical's OWN log/sub-index
(which you can grow freely) — NEVER accreted onto the root line. When you land a slice, update the sha
in your log, not the root index; the root line changes only when your *current focus* or a *trap*
changes. (This is the write-side complement of "don't reorganize memory" above: you still own your
line, but keep it small so the shared entry point stays under its read limit for everyone.)

## Standing down

When your role's work is done (`merged` received for the last unit, or your stop condition hit),
run `cargo xtask fleet remove <you>`. This marks you `stopped` in the registry, drops your
stop-file, and ends the loop — **but leaves your tmux window open** so the scrollback survives.
A completed per-issue `fix` agent self-removes this way.

## If you're stuck (but never idle-waiting on the human)

- Gate won't go green → leave the worktree dirty, STOP this tick, and let the next tick retry. Do
  not send a red `merge-request`. If it's stuck for a reason only the human can resolve, send the
  `concierge` an `ask` and move on — the fix may arrive as an `answer` in a later tick.
- A design ambiguity your role body / plan doesn't resolve → send the `concierge` an `ask` with the
  concrete options, then pick DIFFERENT work this tick (or stand down for the tick). Never block the
  loop waiting for a reply — you are unattended.
- You find your work already done on `trunk` by a peer → STOP, `fleet remove` yourself, don't
  duplicate.
- Only the `concierge` ever speaks to the operator. If you catch yourself about to "ask the user"
  or wait for a human, that is a bug — convert it to an `ask` to the concierge and continue.
