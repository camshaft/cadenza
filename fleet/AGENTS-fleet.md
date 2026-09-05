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
   redesign: one serializing writer, zero dropped commits. `trunk` only ever moves FORWARD (merges/
   cherry-picks) — never a backward `git reset`. This single-writer invariant is guarded + monitored
   by tooling `fleet up` installs (or `cargo xtask fleet install-hooks` re-deploys on demand): a
   `pre-commit` hook refuses a direct commit on `trunk` outside pr-sync's worktree, and a fail-open
   `reference-transaction` hook LOGS any backward `trunk`→origin/main move (with the writer's parent
   command-line) to `.claude/fleet/trunk-clobber.log`. `cargo xtask fleet status` surfaces such a
   "trunk regression" if one is in effect. If you see that alarm/log, it means something reset the
   `trunk` ref backward — route it to `v-fleet-tooling`/the concierge, don't work around it.

3. **You do the substantive work in THIS loop, not a spawned Claude subagent.** Read/edit/gate/
   commit yourself. A one-shot read-only `Explore` for a broad lookup is fine; delegating the
   multi-step implementation to a subagent is not (the operator has found that too flaky). The one
   sanctioned way to hand work off is to **mint a peer fleet agent** via `cargo xtask fleet add`
   (see the PM role) — a durable, tmux-attached agent, not an ephemeral subagent.

4. **You run UNATTENDED. Never wait on the human.** The operator is not watching your window for a
   prompt and you must never block on their input. There is exactly ONE agent that talks to the
   human — the **`concierge`** — and it does so through the **Slack bridge**, not a terminal prompt
   (operator directive 2026-08-01: "most of our interactions now are over Slack"). When you hit
   something only the operator can decide, send the concierge an `ask` and **move on** (pick different
   work this tick, or stand down and let the next tick retry) — do not sit idle waiting for an answer.
   The concierge routes your `ask`/`backlog`/`status` to the operator over Slack (the bridge watches
   the concierge inbox and threads the operator's reply back as an `answer`), which lands in your inbox
   on a later tick. **Enforced structurally:** every fleet window EXCEPT `design` (the on-demand
   terminal session the operator explicitly switches to and types into) is launched with
   `--disallowedTools AskUserQuestion`, so the human-prompt tool is denied at the harness level — you
   *cannot* pop a question in your window even by mistake. That now includes the concierge itself, and
   for a concrete reason, not aesthetics: `AskUserQuestion` BLOCKS the agent's turn waiting for a
   TERMINAL answer, and while the concierge is blocked in that prompt its `/loop` cannot drain its inbox
   — so any operator message arriving over the Slack bridge sits UNREAD until the terminal question is
   answered. One stray `AskUserQuestion` could thus pin the concierge to the terminal and make it go
   DEAF on Slack indefinitely. Denying it is the fix: the concierge never blocks on a terminal prompt,
   surfaces every operator-decision as an `ask`/`backlog` (which the bridge mirrors to Slack + threads
   the `answer` back), and keeps looping/draining meanwhile — the same never-block-on-human invariant
   every other role has. If you feel the urge to ask, that's always an `ask` to the concierge.

## 🥇 The corpus policy — OPERATOR STANDING DIRECTIVE, FLEET-WIDE (2026-08-31)

**Every agent must internalize this.** When writing or editing the corpus, **RAISE** any issue you find
(a decline, a miscompile, a wrong/weak diagnostic, a missing feature). **LOCK IN the IDEALISTIC
(spec-correct) behavior** in the corpus — assert what SHOULD happen. **TRACK the gap:** record it as a
`TODO` asserting the expected value (so it auto-flips to PASS when implemented), and route the underlying
compiler gap to its owner.

**NEVER work around a current compiler gap in the corpus.** No `(- 0 1)`-style workarounds. No pinning a
transient `(declines)`/`(error CODE)` for a should-work feature. No writing the corpus to match what the
compiler happens to do today.

**WHY:** the corpus is the implementation-INDEPENDENT runnable language SPECIFICATION. It specifies the
LANGUAGE, not compiler behavior at a point in time. A gap you find while writing the corpus is a WIN — a
bug located and locked-in — not something to paper over. Finding issues and locking in idealistic
behavior is exactly what the corpus is FOR.

(Genuine semantic errors — programs that are actually invalid per the spec — still assert their
diagnostic; that IS the spec. The rule targets working around IMPLEMENTATION gaps, not asserting real
errors.)

## The tick

Every firing of your `/loop`, in order:

1. **Re-read your charter.** RE-READ this contract (`<your-worktree>/fleet/AGENTS-fleet.md`) AND your
   role body (`<your-worktree>/fleet/loops/<role>.md`) at the START of EVERY tick — not just at
   launch. These files are git-tracked and change as the fleet evolves (the concierge / operator edit
   role bodies and land them on `trunk`); the "Sync your base" step below (step 4) syncs those
   updates into your worktree, and re-reading is how you pick up a changed charter, a new tick-step,
   or a scope change without being relaunched. Because this re-read is step 1 and the sync is step
   4, a given tick re-reads whatever the PREVIOUS tick's sync pulled — a charter change lands in
   your worktree at step 4 and you act on it on the next tick's re-read. If your role body materially
   changed since last tick, act on the NEW instructions.
2. **Refresh presence.** `cargo xtask fleet heartbeat <you>` (stamps `lastTick` in the registry).
   If a stop-file exists for you (`cargo xtask fleet remove` sets it), STOP cleanly and do nothing.
3. **Drain your inbox FIRST.** List it with **`cargo xtask fleet inbox <you>`** — this resolves your
   inbox at the canonical HUB path and prints it, so you can't be fooled by the trap below. Then read
   every JSON file it lists oldest-first, act on each, and archive it with **`cargo xtask fleet inbox
   <you> --processed <msg>`** (`<msg>` = the bare filename from the listing). That flag is the CWD-SAFE
   consume: it resolves the hub path on BOTH sides and moves the message to `processed/` for you, so you
   NEVER hand-`cd <inbox> && mv` a worktree-relative path (which targets an empty shadow copy, leaves the
   real hub message unconsumed → a next-tick idle drain-stall the watchdog escalates — observed 4+ times).
   It also avoids the `cd`-persistence trap below entirely (no `cd` needed — run it from your worktree
   root). Idempotent (already-in-`processed/` is a clean no-op).
   Answering peers takes priority over starting new work (a `reject` from pr-sync means your last merge
   needs a fix — handle it before anything else).
   - **🪤 Your inbox is at the HUB, not your worktree.** The runtime inbox lives at the MAIN repo's
     `<hub>/.claude/fleet/inbox/<you>/` (`.claude/` is gitignored and exists only at the hub, shared by
     every worktree via the common git dir). If you `ls`/glob a *worktree-relative* `.claude/fleet/
     inbox/<you>/`, you are looking at a path that DOES NOT EXIST in your worktree — it silently
     matches NOTHING, so you'll see an empty inbox EVERY tick and wrongly conclude "idle" while real
     messages (an `assign`, a `reject`) pile up unread. This silently stalled an agent for ~8 ticks.
     ALWAYS locate your inbox via `cargo xtask fleet inbox <you>` (or the absolute hub path it prints),
     never a bare relative `.claude/...` glob. A genuinely-empty inbox and a wrong-path inbox both look
     like "0 files" to `ls` — only the canonical resolver tells them apart.
   - **🪤 `cd` PERSISTS across your shell calls — a `cd` away from your worktree breaks the NEXT `cargo
     xtask`.** Your Bash cwd carries over between tool calls. If you `cd` to the hub for a queue sweep, or
     to the shared `claude-memory` repo to commit a note, and DON'T `cd` back, the next `cargo xtask fleet …`
     runs from that other directory and fails `error: no such command: xtask` (memory isn't a cargo repo)
     — or, worse, silently operates on the WRONG repo. This is easy to miss because the failing command
     looks correct; the bug is the invisible cwd. FIX: prefer cwd-NEUTRAL forms — `git -C <path> …` and
     absolute paths — over `cd`; and if you must `cd`, `cd` back to your worktree before the next `cargo
     xtask`. (Verify with `pwd` after any hub/memory operation.)
4. **Sync your base — at tick START, before you commit/send this tick's work.** Run **`cargo xtask
   fleet sync`** from your worktree — the safe base-sync. It `fetch`es, resets onto `trunk`, then
   cherry-picks back ONLY your commits that are not yet upstream BY PATCH-ID, so it lands you on the
   integrated tip WITHOUT losing work and WITHOUT re-stacking a commit pr-sync already landed under a
   re-parented sha (that one is dropped, no empty pick). It refuses on a dirty tree and restores your
   pre-sync HEAD on any conflict. `trunk` is a LOCAL branch in the bare hub (shared via the common git
   dir) — there is NO `origin/trunk` (`origin` is GitHub), so a bare `trunk` is the ref, never
   `origin/trunk`. Then rebuild what you measure against (`cargo xtask build` for the runtime store; a
   stale store makes heap cases false-fail). This is also what refreshes your role body on disk for
   next tick's step 1.
   - **⚠ Do NOT re-sync (this OR a bare reset) while you have a merge-request already queued.** Any
     re-sync moves your branch off the commit it was on: a bare `git reset --hard trunk` DANGLES an
     unlanded commit, and even `fleet sync` REPLAYS an unlanded commit under a NEW sha (cherry-pick) —
     either way, the `--ref <sha>` you already sent pr-sync no longer names a commit reachable from
     `fleet/<you>`. pr-sync fetches that `--ref` from your BRANCH and CANNOT fetch a now-unreachable
     commit, so it **silently SKIPS your MR every tick with no reject** while your work sits queued
     forever (this has cost multiple agents multiple ticks). So once you've sent an MR: LEAVE the branch
     alone — being behind `trunk` is fine, pr-sync merges your sent `--ref` onto current `trunk` and
     re-gates. Only re-sync after that MR lands or is rejected (on a reject you fix + resend a fresh
     `--ref` anyway). If you already re-synced and orphaned a sent `--ref`, recover with `git reset
     --hard <that-ref>` (the commit object survives in the shared store) — or just resend a new
     `merge-request` naming your current tip. Verify with `git branch --contains <ref>`. (`fleet sync`'s
     guarantee is "never lose WORK + never re-stack an already-landed commit", NOT "safe to run under a
     pending MR" — the sha still moves.) Reset — not `rebase`: pr-sync squash-integrates, so a plain
     `rebase` replays your already-landed commits as orphans against the new tree.
5. **Do ONE well-scoped unit of work** per your role body. Gate it (below). Never leave `trunk`
   broken — but your worktree may be left dirty across ticks (the next tick resumes it).
6. **If a commit is ready,** send `pr-sync` a `merge-request` (below). Otherwise reschedule.

**🧹 Clean up your own `/tmp` scratch when a probe/experiment finishes.** `/tmp` is a tmpfs with a FIXED
inode budget; agent-scratch dirs (`/tmp/<yourprobe>`, smoke-test dirs, etc.) that pile up across the fleet
creep toward inode-ENOSPC (which wedges everyone's Bash). `rm -rf` your scratch dir when you're done with
it. A safety-net reaper (`prune-tmp-inodes.sh` Class C) only sweeps allowlisted, >4h-idle, unheld scratch
and ONLY near the wedge (≥70% inode-use) — so it won't save a still-referenced or short-lived dir; own-cleanup is the reliable path.

### ⚠ Keep your context small — keep turns short; compaction is WATCHDOG-driven, not self-invoked
A saturated context is the fleet's worst failure mode: at ~100% even `/compact` can't submit — it
needs headroom the full window no longer has, so it can't run at all — and a fully-wedged agent then
needs an auto-RESTART — recurring churn across every role that does long turns.

**🔑 You CANNOT self-invoke `/compact`.** It is a built-in CLI slash command, NOT a tool/skill — an
unattended looping agent has no way to run it (the Skill tool rejects built-ins). So "run `/compact`"
is not an action you can take. Instead, **the watchdog does it FOR you**: it detects a pre-wall pane
(≥85%, below the 100% wall) and `send-keys` `/compact` into your window — but only when your window is
IDLE AT A PROMPT (a `/compact` can't land mid-turn). At the 100% wall it can no longer compact, so the
watchdog auto-restarts the window instead (durable state persists). Both are automatic; you don't
trigger either.

**Your job is therefore to STAY COMPACTABLE — keep each turn short** so you return to an idle prompt
often (that's when the watchdog can compact you) and never sprint from 70% to 100% inside one
uninterrupted turn:
- Do ONE well-scoped unit per turn, then yield to a prompt — don't chain a build + a big read + edits
  into a single continuous turn that climbs past the wall before the watchdog can catch it idle.
- The biggest single ingest is a gate/build/test run; after one, prefer ending the turn so the next
  tick (and the watchdog) can act, rather than immediately starting another heavy unit.
- If you're already very high and mid-turn, the safest move is to STOP the turn cleanly (finish the
  current small step, don't start another) so your window goes idle and the watchdog can compact it.

## The message protocol

A message is a single JSON file in the recipient's inbox. **Send with the tool, never by hand** so
delivery is atomic and well-formed:

```
cargo xtask fleet send --to <agent> --kind <kind> --subject "<one line>" \
    [--ref <sha-or-branch>] [--body "<detail, may be multiline>"] [--urgency <level>]
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
| `note`          | any → any            | free-form coordination (territory hand-off, "I'm taking X", a status reply) — INFORMATIONAL, not a work request |

**A `note` is INFORMATIONAL — do NOT use it for an ACTIONABLE request the recipient must act on.** A
recipient drains its ACTIONABLE mail (merge-request/assign/issue/ask) reliably, but `note`s are lower
priority and may sit undrained under load (a busy pr-sync mid-gate is the classic case); the watchdog
auto-reaps provably-spent notes and only escalates the rest. So if you need someone to DO something —
schedule a freeze, pause work, prioritize a ref, make a call — send an `ask` (to the concierge, for a
decision) or an `assign`/`merge-request`/`issue` (the actionable kinds), NOT a `note`. (A pr-sync-bound
`DROP-REF` withdraw is the one exception: it's a `note` by protocol and is honored passively by
`schedule-pass`'s queue filter — see the DROP-REF section — so it doesn't depend on pr-sync draining it.)
A scheduling/priority request mis-sent as a `note` once starved a flag-day ~2.5h because it sat unread.

Include enough in `--body` that the recipient needs no other context. A `merge-request` body should
carry the gate summary (fail-set diff, test count) so pr-sync can trust it fast. An `ask` body MUST
state the concrete options so the human can decide in one line — the concierge is a router, not an
investigator.

**🔴 NEVER put command output or environment into a `--subject`/`--body` — same leak class as PR bodies**
(operator P0 seq-198: a `fleet send` leaked a 48KB env-dump SUBJECT). A markdown-backtick or `$()` span
inside an INLINE double-quoted `--subject "…"`/`--body "…"` command-substitutes in YOUR shell BEFORE the
arg reaches the tool — one span wrapping `set`/`env` dumps the whole environment into the message. So:
build the body as a LITERAL heredoc file and pass **`--body-file <file>`** (never an inline double-quoted
`--body` with backticks/`$()`); keep the `--subject` short + single-quoted (no backticks/`$()`). `fleet
send` now SCANS `--subject`+`--body` and REFUSES an env-dump/secret (a hard error, not a delivered leak) —
but the substitution already fired in your shell, so the refusal only CONTAINS it; the `--body-file`
literal path is what PREVENTS it. Same discipline as the PR path below.

**`--urgency <low|normal|high|urgent>` (default `normal`) tells the recipient how to PRIORITIZE.** An
elevated (`high`/`urgent`) message is TAGGED in the recipient's `cargo xtask fleet inbox` listing
(`<high>` / `<<URGENT>>`) with a "prioritize reading these" summary line, so it stands out while you
drain. The listing stays OLDEST-FIRST (urgency is a SIGNAL, not a reorder — you still drain oldest-first
and handle a `reject`/`ask` by kind). Reserve `urgent` for genuinely time-critical coordination (a fleet
wedge, a trunk-red, a freeze) so the tag keeps its meaning; most mail is `normal`. Kind still governs
handling — urgency only nudges attention ORDER, it does not change what a message MEANS.

**A `merge-request` integrates the SINGLE `--ref` commit, NOT your branch range.** pr-sync applies
just the one commit you name in `--ref` onto `trunk` (verified empirically: every per-MR landing is a
single linear commit) — it does NOT pull in the other commits reachable from your branch below that
`--ref`. So if you have a LOCAL STACK of N commits, do NOT send one merge-request for the tip and
expect the whole stack to land — only the tip would. Send **one merge-request per commit, oldest-first,
and wait for each `merged` before sending the next**. After one lands, `cargo xtask fleet sync` cleanly
drops it by patch-id and replays the rest of your stack, giving you the next commit to send as a fresh
`--ref`. (Keep each commit independently green so this per-commit cadence never lands a broken `trunk`.)

**Make each merge-request carry MEANINGFUL change — iterations must count** (operator rule, fleet-wide).
Every candidate PR runs a FULL ~16-job CI gate, so a trivial MR (e.g. a 9-line one) costs the same
gate cycle as a substantial one — a per-line drip of tiny MRs wastes the fleet's CI + integration
capacity disproportionately. Both bounds apply:
- **FLOOR:** land a COHERENT UNIT — a whole slice / stage / fix / review-cleanup — not per-line
  increments. If several small edits are one logical change, they belong in ONE commit. Make the
  iteration worth its gate cycle.
- **CEILING (anti-gaming):** do NOT pad, invent, or bundle unrelated work just to look bigger — the
  goal is meaningful PROGRESS per iteration, not artificial size. Land what's genuinely ready as a
  coherent whole; don't split a natural unit into trivial pieces, and don't staple unrelated landables
  together for size.
This is about the SUBSTANCE of each unit, NOT batching: the per-commit-green + one-MR-per-commit
invariants above stay intact — keep independent landables as separate commits, just make each one a
meaningful unit rather than a sliver. (A genuinely small fix that IS the whole coherent change — a
one-line correctness fix, a targeted review nit — is fine; the rule targets needless fragmentation,
not honest small changes.)

**Delivery wakes the recipient.** `fleet send` doesn't just drop the JSON — after delivering it
nudges the recipient's tmux window into an immediate tick (`send-keys`), so a message is reacted to
within seconds instead of waiting for the next scheduled `/loop`. Delivery == wake. This means when
you send a peer a `reject`/`answer`/`assign`/`note`, they generally start acting on it right away —
you don't need to also poll or wait a full interval. The nudge is automatically skipped for a stopped
recipient, one with no live window, one already mid-tick (it'll drain your message when it finishes),
and the interactive `concierge`/`design` windows (a human may be typing). `/loop` remains the safety
heartbeat, so even a missed nudge is eventually picked up. Pass `--no-wake` when seeding a batch.

## The gate (what "green" means)

**🚦 THE LAND MODEL (OPERATOR DIRECTIVE 2026-08-28, fleet-wide): open your OWN PR, gate it LOCALLY with
`gate-local`, `--admin`-merge. GitHub Actions is HOURLY-ADVISORY only — never a per-PR/merge gate.** The
operator's words, verbatim: *"I want agents to be opening their own PRs and merging after testing locally."*
This is the STANDING model for EVERY agent (not just the platform lane), and it supersedes the
`pr-sync`-integrator framing further down while pr-sync is stood down:
- **`cargo xtask fleet gate-local` is the AUTHORITATIVE merge-truth.** It builds
  `.#checks.<arch>-linux.local-gate` (the aggregate of the 9 merge-required checks + mandate-lint) and
  prints `LANDABLE` (green) or `HOLD` — naming the failing sub-check(s) (see the gate-local paragraph below).
- **GitHub Actions CI is HOURLY-ADVISORY ONLY — it is NOT a per-PR/merge gate and NEVER blocks a PR or
  merge** (v-nix moved CI off PRs/merges to an hourly advisory run). Do NOT wait on GHA to land; a green
  GHA is a bonus signal, not a requirement, and a `reject`-for-CI-red is not a thing under this model.
- **Flow:** iterate with `dev-gate` + a scoped spot-check (below) → run `cargo xtask fleet gate-local` → on
  `LANDABLE` (or a `HOLD` whose ONLY failing sub-check is a KNOWN pre-existing red unrelated to your change)
  open your OWN PR against `main` and `gh pr merge --admin`. Each PR stays a meaningful coherent unit (the
  FLOOR/CEILING rules below still apply).
- **🔴 NEVER put command output or environment into a PR body/title — construct PR bodies from STATIC
  LITERAL text only** (operator P0, seq-198: a PR leaked the ENTIRE env dump into its description). A PR
  body/title must be hand-written prose describing the change — NEVER `$(...)`/backtick command
  substitution, NEVER `env`/`printenv`/`set`/a captured log, NEVER a variable that could hold command
  output. This is the SAME class as the fleet-send quoting discipline: a shell mishap (an apostrophe or
  backtick inside a double-quoted `--body "..."`) can splice `env`/command output into the body. To stay
  safe: SINGLE-quote the `--body` (or pass it from a file/stdin literal), and put ZERO dynamic content in
  it. **The sanctioned PR path is now `cargo xtask fleet pr create --title '…' --body-file <file> [--base
  main] [--head <branch>]`** — write the body as a LITERAL FILE (no command output), and the tool scans it
  for env-dump/secret material (REFUSING if found) + hands it to `gh` via `--body-file` (gh reads the file;
  no body string ever passes through a shell). PREFER it over raw `gh pr create`. If you must use raw `gh`,
  single-quote the `--body` and paste ZERO command output. (`gh pr merge --admin` for landing is unchanged.)
- **The hourly advisory run needs an eyeball:** a RED hourly run is surfaced by the `concierge`, who relays
  it to the owning lane to fix — it does not auto-block anyone.

(The `pr-sync` `merge-request` protocol documented elsewhere is the model WHEN pr-sync is the active
integrator; while pr-sync is stood down the direct-to-main model above is the standing one. `dev-gate` +
a scoped spot-check remain your fast inner loop under BOTH models — full gates are never your per-iteration job.)

**🚦 FULL GATES ARE pr-sync's JOB — you iterate on `dev-gate` + a scoped spot-check (OPERATOR DIRECTIVE
2026-08-11: "restrict full gates to the pr-sync").** The full `cargo xtask gate`/`check` battery is
~8-15min AND pr-sync RE-GATES the full battery on every MR anyway, so an agent running the full battery —
whether per iteration OR as a pre-send verify — is redundant work that was measurably killing fleet
iteration speed. So the policy is now: **the ONLY agent that runs the full authoritative gate is
pr-sync** (its integration pass is the single source of merge-truth). Every other agent iterates AND
pre-sends with the NARROW checks only:

- **`cargo xtask dev-gate`** — the fast inner-loop gate (auto-detects your touched crates from `git diff`
  and runs only their test+clippy+fmt; warm ≈ 4s; pass crate names to scope explicitly). Your primary
  self-check, every iteration.
- **a scoped corpus spot-check** when your slice changes behavior: `cargo xtask gate --files
  <your-file>.sexp --target wasm` (YOUR corpus file, one backend). Not the whole battery — just the
  case(s) your slice touches.
- `cargo test -p <your-crate> --lib` for a specific test `dev-gate` isn't surfacing.

**🚦 `cargo xtask fleet gate-local` — THE authoritative merge gate (per THE LAND MODEL above; standing while
pr-sync is stood down).** You run the authoritative required-set gate yourself before landing direct to main:
`cargo xtask fleet gate-local` (builds `.#checks.<arch>-linux.local-gate`, the aggregate of the 9
merge-required checks + mandate-lint). It prints a clear verdict: `LANDABLE` (green) or `HOLD`, and on
HOLD it now **names the failing sub-check(s)** — e.g. `HOLD — failing sub-check(s): wasm-runtime-build`
with the exact `nix build .#checks.<arch>-linux.<name> -L` to see that check's error (no more opaque "1
dependency failed"). If a sub-check is a KNOWN pre-existing red unrelated to your change, you may still
`--admin`-merge your own-sound change — but now you can SEE which check + confirm it's not your regression.

**🚫 NEVER run native `cargo test --workspace` (or `cargo xtask test`).** It is UNCACHED + full-workspace
+ fleet-hostile: it shares nothing across the fleet, cold-rebuilds, and fans test threads out to every
core (the `[build] jobs=4` cap bounds COMPILE jobs, NOT test-thread execution) — it caused an
operator-flagged host load spike (~57). `cargo xtask test` is a guardrail that REFUSES and points here.
Use the nix-cached `dev-gate` (above) for your inner loop; if you truly need a native run, SCOPE it to
one crate (`cargo test -p <crate>`) — never the whole workspace on the shared host.

**⚡ TIGHT DEV LOOP (implement→build→validate) — use `CDZ_NO_CARGO_SHIM=1` for real INCREMENTAL cargo.**
Under the all-nix mandate `cargo xtask build` / `cargo build` route to `nix run .#build` (front-end + store
from the shared /nix/store, zero per-worktree bloat) — correct for gate/CI/final-verify, but nix is
source-hash-keyed so it rebuilds the top crate FROM SCRATCH on every 1-file change (>2min, no cargo
incremental-object reuse — a fast-incremental nix dev-build is not feasible, v-nix-confirmed). Two
sanctioned tight-loop paths for the fast implement→build→validate loop (where you need the `cdz`/`cdz-compile`
bins to run a witness):
- **PREFERRED (no env): `cargo build -p rcdzc -p cdz -p cdz-run`** — the `-p` form is NOT routed by the shim
  (only a bare/no-`-p` `cargo build` routes to `.#build`), so it runs real INCREMENTAL cargo (~seconds after
  the first) AND produces the actual bins. No env var needed.
- **Full escape: `CDZ_NO_CARGO_SHIM=1 cargo xtask build`** — bypasses the shim entirely (e.g. if you want the
  whole `xtask build` flow incl codegen/store). `export CDZ_NO_CARGO_SHIM=1` for your dev shell while iterating.
UNSET the env (or a fresh shell) for gate/test, which stay on nix. (Pending-operator-confirm: this tight-loop
cargo carve-out is a scoped exception to the all-nix mandate, surfaced to the operator 2026-08-29 — the shared-
store nix path stays the default for everything except tight iteration; the GATE + witness-running still go
through nix. `cargo build -p` stays WARN-not-fail even at the eventual hard-fail flip — it is the sanctioned
incremental path.)

**🔎 DEBUG TRACING — instrument with `tracing::trace!`/`debug!` + enable via `RUST_LOG`; do NOT add ad-hoc
`if std::env::var("SOME_FLAG").is_ok() { eprintln!(…) }` gates** (operator dev-convention seq-210). A
`tracing` event is structured, level/target-filterable, and STAYS in the codebase — it benefits every future
debugging session; the env-var-`eprintln` is throwaway (one agent's session, then reverted or left as clutter).
The compiler is ALREADY richly instrumented (`rcdzc` has hundreds of `trace!`/`debug!` sites across
`infer`/`lower`/`resolve`/`eval`/…), so you almost never need to ADD wiring — just add a `tracing::trace!(…)`
where you're debugging (the crate already depends on `tracing`) and turn it on:
- **Via the `cdz` CLI (the usual witness run):** `RUST_LOG=rcdzc=trace cdz compile …` — the shared `RUST_LOG`
  knob. Filter to a module to cut noise: `RUST_LOG=rcdzc::infer=trace` (or `=debug`). Unset ⇒ zero overhead
  (no subscriber installed).
- **For `rcdzc` running INSIDE the `cargo xtask` pipeline:** use the tool-private **`CDZ_LOG`** (same
  `EnvFilter` syntax, e.g. `CDZ_LOG=rcdzc=trace`) — the pipeline shells `cdz-syntax | rcdzc | cdz-run`, and a
  bare `RUST_LOG` would fan out to cargo/wasmtime/etc.; `CDZ_LOG` scopes it to rcdzc only.
- **NUANCE (not a violation):** the `xtask`/fleet orchestration CLI is NOT `tracing`-instrumented (no
  subscriber; its `eprintln!` lines are the tool's normal user-facing output, not debug tracing) — so
  `eprintln!` there is fine. The one gap: if you want *filterable* debug tracing INSIDE `xtask`/`fleet` code,
  there's no subscriber yet — add one (small setup) or ask, rather than reaching for an env-gated `eprintln`.

**🔒 A HEAVY nix build MUST go through `cargo xtask fleet with-lease` — a RAW `nix build .#…` escapes the
concurrency cap.** `CDZ_CHECK_LEASE_MAX` (+ `fleet with-lease`) exists so heavy nix builds don't all run
at once and thrash the single big-nix-lock (the load-deadlock that makes EVERYONE's gates crawl —
observed with 8+ concurrent heavy builds serializing, a 1h31m aggregate starved at 0.2% CPU). `cargo xtask
check` acquires the lease itself, but a **raw `nix build .#checks.…` / `.#<heavy-attr>` you run directly
BYPASSES the cap.** So wrap any heavy raw nix build: `cargo xtask fleet with-lease nix build .#…` (not a
bare `nix build`). This is fleet-wide courtesy — one escapee crawls every peer's required gate too.
`with-lease`'s primary protection is that **slot cap** (fewer heavy builds run at once). It also injects a
per-holder `NIX_CONFIG` (`max-jobs`/`cores` ≈ `nproc / (2·(LEASE_MAX+1))`, tune with `CDZ_LEASE_NIX_BUDGET`)
that bounds concurrent derivations + `NIX_BUILD_CORES`-respecting builds — but NOT a single heavy **cargo**
build's rustc fan-out (a 60-crate `cargo build` self-parallelizes to nproc; cargo ignores `NIX_BUILD_CORES`).
That cargo fan-out is bounded elsewhere — the nix **daemon** (`cores=1`/`max-jobs=64` in `nix.custom.conf`,
for the make-ish side) and **flake-side `CARGO_BUILD_JOBS`** (for the cargo builds). So `with-lease` is
still the right courtesy for any heavy raw `nix build` (it takes the slot), but don't assume it alone caps a
compiler-rebuild's rustc storm.

**Do NOT run the full `cargo xtask gate` (whole corpus × 3 backends) or `cargo xtask check` (whole
workspace) yourself** — not per iteration, not "once before send." A green `dev-gate` + scoped spot-check
is your send bar; pr-sync's full-battery re-gate is the authoritative backstop, so a rare scoped-miss
costs ONE reject round-trip — far cheaper than every agent paying ~10min/verify. (pr-sync itself, and a
one-off deliberate whole-repo audit an owner explicitly decides to run, are the exceptions; the rule is
that routine per-MR full-gating belongs to pr-sync alone.)

Before you send a `merge-request`, the NARROW pre-send verify (NOT the full battery):

1. `cargo xtask dev-gate` green (your touched crates' test+clippy+fmt) — a `Todo→Fail` corpus flip is a
   genuine MISCOMPILE, so if your slice changes behavior also run the scoped `cargo xtask gate --files
   <your-file>.sexp --target wasm` and diff the FAIL SET against the baseline (ADDITIVE only; pass count
   drifts as peers land). `(error CODE)` cases are code-matched; `(trap "reason")` reason-matched.
2. For a new test/slice, add its coverage (a fold unit + a wasmtime run where a value executes; a reject
   test for a new diagnostic) and confirm it via `dev-gate` / `cargo test -p <crate> --lib`.
3. **Format with the PINNED rustfmt, NOT ambient `cargo fmt`** (recurring reject class — cost 4 MRs one
   session: v-inference/v-compiler-ml/v-agent-harness tests all reject-red on a pinned-fmt diff their
   local `cargo fmt` missed). The gate formats with the flake's PINNED rustfmt; a plain `cargo fmt` uses
   your ambient rustfmt, which wraps DIFFERENTLY (long string literals / test code line-wrap is the usual
   culprit) → gate reject even though the code is fine. Two ways to stay clean: `cargo xtask dev-gate`
   ALREADY runs the pinned `fmt --check` (a red there IS this diff — heed it), and to auto-FIX it, format
   through the pinned toolchain: **`nix develop -c cargo fmt --all`** (the devShell carries the pinned
   `rustToolchain`), NOT bare `cargo fmt`. Watch the fmt-drift trap too: a whole-workspace fmt touches
   foreign drift — revert files you didn't edit (`git checkout --`), verify only YOUR files changed.
4. Do NOT edit `cdz-runtime`'s `//` comments or `wit/runtime.wit` casually — they are inside the frozen
   `REQUIRED_RUNTIME_HASH`; a change there means `cargo xtask build` + `codegen --check` (the one place a
   heavier local check is unavoidable, since pr-sync can't recover a hash mismatch for you).

pr-sync's pass remains the AUTHORITATIVE full-battery gate — this policy doesn't weaken merge-truth, it
stops every agent from redundantly re-running what pr-sync runs anyway.

**A sudden MASS heap-test failure is almost always a STALE STORE, not a regression.** If dozens+ of
heap/gate cases flip to fail at once — ESPECIALLY right after a numeric/bignum/runtime change (which
bumps `REQUIRED_RUNTIME_HASH`) — do NOT escalate or revert. A one-commit change rarely flips hundreds
of cases; a stale runtime store flips every heap case at once (each traps `no runtime of content
address <hash> in the store`). REBUILD the store on current trunk (`cargo xtask build`) and re-run on
the FRESH build first — the mass-red almost always vanishes. Only after a clean rebuild still fails is
it a real regression. (This trap cost multiple false "fleet-red regression" escalations + a near-miss
revert in one session — see the stale-store memory traps.)

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

**NEVER edit the `MEMORY.md` root directly — it is LIBRARIAN-WRITE-ONLY** (operator directive,
2026-08-30; supersedes the older 2026-08-03 "RARELY edit / write-COLD" norm: "the memory really needs
to be collapsed into a very small index"). `MEMORY.md` loads into EVERY agent's context every session,
so it must stay a tiny, scannable, near-static map, and a SINGLE writer (the `librarian`) keeps it that
way. Your default write target is your OWN sub-index/log (`<vertical>-log.md`, `index-*` — grow those
freely); the root is NOT where your work goes.
- **Write your learnings, landings, sha history, increment detail, status — ALL of it — to your own
  sub-index/log, NEVER to the root line.** When you land a slice, update your log, not `MEMORY.md`.
- **REQUEST a root pointer change from the `librarian`** (`cargo xtask fleet send --to librarian`) only
  on a GENUINE FOCUS SHIFT or a new CRITICAL trap — never edit the root yourself, and not per-landing,
  per-increment, or per-status-tweak. Most ticks change NOTHING at the root. Agents appending
  single-finding pointers to the root tail is a recurring hygiene bug the librarian has to strip +
  relocate — put the detail in your log instead.
- The root line is a STABLE 1-line pointer in the shape the `librarian` maintains (~1 sentence, <~250B):
  ```
  - <emoji> **NAME (v-x)** — <one-clause current status/phase> [🔜 <next, if any>] [[<your-sub-index-or-log>]]
  ```
  So a genuine-focus-shift REQUEST is just: ask the librarian to change the one status clause (+ maybe
  the 🔜-next) — nothing else. ALL detail (traps, landed shas, in-flight MRs, increments) lives in the
  `[[sub-index]]`. Not a changelog, not a status board.
The `librarian` owns the structural collapse of existing root content (see "don't reorganize memory"
above) and maintains that collapsed root-line shape; this write-norm keeps it small going forward.
Keeping the root write-cold is how the shared entry point stays tiny for everyone.

## Standing down

When your role's work is done (`merged` received for the last unit, or your stop condition hit),
run `cargo xtask fleet remove <you>`. This marks you `stopped` in the registry, drops your
stop-file, and ends the loop — **but leaves your tmux window open** so the scrollback survives.
A completed per-issue `fix` agent self-removes this way.

To LOWER your cadence to a low-frequency monitor (still active, just ticking rarely — e.g. a role
at a rest point) instead of stopping, use `cargo xtask fleet set-interval <you> <interval>` (e.g.
`3h`). Do NOT do this with a raw `/loop <interval>` reschedule: `set-interval` persists the new
interval to the registry, and the watchdog's stale window AND its re-arm both read that registry
interval. A raw `/loop` leaves the registry at the old (short) interval, so the watchdog still
computes a short stale window, trips every ~stale-window, and re-arms you back to the old cadence —
silently reverting the change. `set-interval` is the only lever that actually sticks.

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
