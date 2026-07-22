//! `cargo xtask fleet` — the orchestrator for the autonomous-agent fleet.
//!
//! The fleet is a set of long-running Claude sessions ("agents"), each looping over one role
//! (integrator, bug-fix PM, per-issue fixer, adversarial breaker, fuzzer, per-feature vertical
//! owner, and the human-facing concierge). This subcommand is the single, idempotent way to bring
//! them up as named tmux windows, tear them down, inspect the board, and add/remove agents at
//! runtime — replacing the fragile session-only crons that a reboot wiped.
//!
//! Division of responsibility (see .claude/fleet/AGENTS-fleet.md for the agent-facing contract):
//!   * THIS FILE owns all the LOGIC — the durable manifest (`registry.json`), git-worktree creation,
//!     atomic inbox message delivery, and tmux window management. It is tracked Rust, so it is
//!     reviewable, type-checked, and shared across every worktree via git.
//!   * The STATE lives in the gitignored `.claude/fleet/` (registry, inboxes, per-agent worktrees,
//!     the work queue, backups). Nothing here is committed — it is agent-local, like `.claude/`.
//!   * `.claude/fleet/window.sh` is a ~30-line shim (the ONLY shell): each tmux window runs it, and
//!     it just `exec`s `claude` in the agent's worktree with the model + denied-tools + kickoff this
//!     orchestrator hands it via `fleet describe`.
//!
//! The integration branch is `trunk` (a rename of the old local `spec`). Only the `pr-sync` agent
//! advances it; everyone else works in their own worktree and sends `pr-sync` a `merge-request`
//! message — so there is no multi-writer `update-ref` CAS anymore.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Paths;

/// The integration branch every agent worktree is created off, and the only branch `pr-sync` writes.
/// (Historically the local `spec` branch; renamed to `trunk` when the fleet landed.)
const TRUNK: &str = "trunk";

/// Context-% at/above which the watchdog surfaces a saturation warning (report-only). Below the 100%
/// wall so the concierge can `/compact` proactively while it still submits — a session that reaches
/// 100% is unrecoverable (even `/compact` can't submit).
const CTX_SATURATION_THRESHOLD: u8 = 85;

/// Context-% at which an agent is UNRECOVERABLY WEDGED — `/compact` can no longer submit (it needs
/// headroom the full window lacks), so the agent can't self-heal and only an operator restart clears it.
/// Distinct from the lower `CTX_SATURATION_THRESHOLD` warn level, where the agent can still self-`/compact`.
const CTX_WEDGE_THRESHOLD: u8 = 100;

/// PRE-WALL escalation threshold: the context-% at which the watchdog escalates to the concierge for a
/// NON-pr-sync agent — high enough that "it hasn't self-`/compact`ed yet" is a real actionable signal,
/// but BELOW the 100% wall so a `/compact` can still submit. Escalating only at 100% (the old behavior)
/// was too late: the agent was already unrecoverable. An agent stuck in a long continuous turn never
/// returns to a prompt to self-compact, so it walks silently past 85% to the wall — this catches it
/// while a nudge/restart can still save it. (concierge ruling 2026-07-19: pure safety hardening.)
const CTX_PREWALL_THRESHOLD: u8 = 95;

/// PRE-WALL threshold for pr-sync SPECIFICALLY — lower (earlier) than the general one because pr-sync is
/// the SINGLE WRITER of `trunk`: its wedge stalls the ENTIRE merge queue fleet-wide, so it's worth
/// escalating with more margin to guarantee a `/compact` lands before the wall. (concierge: "escalate
/// earlier/harder for the single-writer".)
const CTX_PREWALL_THRESHOLD_PRSYNC: u8 = 92;

// The threshold ordering invariant, guarded at COMPILE TIME beside the definitions it constrains.
// pr-sync (single `trunk` writer) must escalate no later than a general agent — ideally earlier — and
// every pre-wall bound must sit below the 100% wall so a `/compact` can still submit when it fires. A
// `const _: () = assert!(…)` fails the BUILD (not a test run) the instant a future edit inverts the
// margin or lifts a bound to/over the wall — stronger than a runtime test assert, and clippy-clean
// (a `const` context is the intended home for a const comparison, so `assertions_on_constants` doesn't
// fire — no lint-suppression needed). Supersedes the earlier `#[allow(clippy::assertions_on_constants)]`
// runtime asserts in `prewall_threshold_is_lower_for_the_single_writer_pr_sync`.
const _: () = assert!(
    CTX_PREWALL_THRESHOLD_PRSYNC < CTX_PREWALL_THRESHOLD,
    "the single writer (pr-sync) must escalate earlier than a general agent"
);
const _: () = assert!(
    CTX_PREWALL_THRESHOLD < CTX_WEDGE_THRESHOLD,
    "the pre-wall bound must sit below the 100% wall so /compact can still submit"
);

/// The pre-wall escalation threshold for `agent` — the lower pr-sync-specific bound for the single-writer,
/// else the general pre-wall bound. Pure so the per-agent policy is unit-testable.
fn prewall_threshold_for(agent: &str) -> u8 {
    if agent == "pr-sync" {
        CTX_PREWALL_THRESHOLD_PRSYNC
    } else {
        CTX_PREWALL_THRESHOLD
    }
}

/// Rate-limit (seconds) for the context-wedge concierge escalation, so a wedged agent generates at most
/// one concierge `note` per this window until the operator acts (the watchdog sweeps every ~4 min).
const SAT_NOTIFY_GRACE: u64 = 1800;

/// Short recheck window (seconds) for a drain-nudge that DIDN'T stick: if the SAME flagged message is
/// still unconsumed this long after we auto-nudged, re-nudge (and flag loudly) rather than waiting out
/// the full `--drain-nudge-grace` anti-churn window. Two ticks of a typical vertical (30m interval is
/// the outlier; most re-arm on a few-minute cadence) — long enough for the agent to act on the nudge,
/// short enough that a nudge that landed-but-didn't-act doesn't sit ~8min like v-metaprogramming did.
const DRAIN_NUDGE_STUCK_GRACE: u64 = 180;

/// Delay (seconds) before the CONFIRMING re-capture of a suspected drain-stalled pane. A single
/// instantaneous `capture-pane` can catch a busy agent in the sub-second gap BETWEEN its tool-turns —
/// the prior turn has ended (bare `❯` prompt) but the next model turn hasn't yet re-rendered the "esc to
/// interrupt" affordance — so it mislabels a mid-tick agent as idle-at-prompt and fires a FALSE
/// drain-stall (observed on pr-sync mid-batch: it was actively draining a 26-MR queue, just snapshotted
/// between two Bash turns). A genuinely idle agent sits at the prompt indefinitely, so a working
/// affordance reappears within a second or two if the agent is alive; a truly idle one never shows it.
/// 2s is enough for the next turn to render without bloating the ~4-min sweep — and it's only paid for
/// the rare agent already suspected of a stall, never every idle pane. (Enforces the fleet's own wedge
/// discipline: a wedge/stall verdict needs a non-instantaneous sample, not one snapshot.)
const DRAIN_STALL_CONFIRM_DELAY_SECS: u64 = 2;

/// One agent's durable row in the manifest. The registry is the source of truth that survives a
/// reboot; `fleet up` reconstitutes every `Active` agent's worktree + tmux window from these rows.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Agent {
    /// Unique agent name — also the tmux window name and the inbox directory name.
    name: String,
    /// The role prompt body under `.claude/fleet/loops/<role>.md` this agent runs
    /// (`pr-sync` | `corpus-bugfix` | `fix` | `breaker` | `fuzzer` | `vertical` | `concierge`).
    role: String,
    /// For a `vertical` agent, the feature it owns (e.g. `iterators`); empty otherwise.
    #[serde(default)]
    vertical: String,
    /// For a `vertical` agent, the subsystem the feature lives in (`rcdzc` | `compiler-ml` |
    /// `runtime` | `guide` | …); empty otherwise. Defaults to `rcdzc` at add-time when a vertical
    /// gives none.
    #[serde(default)]
    area: String,
    /// The agent's git worktree (absolute path). Created off `trunk` if missing at `up` time.
    worktree: String,
    /// The branch checked out in `worktree`. `pr-sync` is the sole holder of `trunk`; every other
    /// agent has its own topic branch.
    branch: String,
    /// The `/loop` interval the window drives the role at (e.g. `10m`).
    interval: String,
    /// The Claude model this agent runs under (`opus` for most; `fable` for `breaker`).
    model: String,
    /// The reasoning-effort level the window launches with (`low`|`medium`|`high`|`xhigh`|`max`).
    #[serde(default = "default_effort")]
    effort: String,
    /// `active` (loop should be running) or `stopped` (removed — window kept for scrollback).
    status: String,
    /// Whether the window is launched with `--disallowedTools AskUserQuestion`. True for every role
    /// except `concierge` (the sole human interface).
    #[serde(default = "default_true")]
    disallow_ask: bool,
}

fn default_true() -> bool {
    true
}

/// The on-disk manifest: just the list of agents. Kept deliberately flat so it is easy to read,
/// hand-edit in a pinch, and diff.
#[derive(Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    agents: Vec<Agent>,
}

/// One standing agent declared in the TRACKED roster (`fleet/roster.json`). This is the reproducible
/// intent — what a fresh clone should run. Runtime-only fields (worktree path, live status, window)
/// are NOT here; they are derived into the machine-local registry at `up` time.
#[derive(Clone, Debug, Deserialize)]
struct RosterEntry {
    name: String,
    role: String,
    #[serde(default)]
    vertical: String,
    #[serde(default)]
    area: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default = "default_effort")]
    effort: String,
}

fn default_interval() -> String {
    "10m".to_string()
}
fn default_model() -> String {
    "opus".to_string()
}
fn default_effort() -> String {
    "high".to_string()
}

/// Resolve a roster/`--model` alias to the full model id `claude --model` receives. The fleet runs
/// on the 1M-token context variants, so the short aliases map to those — this is the ONE place the
/// long ids live, so the roster and `fleet add` stay readable (`opus` / `fable`). An id that is not a
/// known alias passes through unchanged (so a full id or a future model still works).
fn resolve_model(alias: &str) -> String {
    match alias {
        "opus" => "us.anthropic.claude-opus-4-8[1m]".to_string(),
        "fable" => "us.anthropic.claude-fable-5[1m]".to_string(),
        other => other.to_string(),
    }
}

/// The tracked roster file (`fleet/roster.json`): the standing fleet that reproduces on any machine.
#[derive(Default, Deserialize)]
struct Roster {
    #[serde(default)]
    agents: Vec<RosterEntry>,
}

/// Filesystem anchors: the TRACKED source (`<worktree>/fleet/`, reproducible) and the machine-local
/// runtime STATE (`<hub>/.claude/fleet/`, gitignored).
struct Fleet {
    /// `<hub>/.claude/fleet` — the machine-local runtime state dir.
    root: PathBuf,
    /// `<hub>/.claude/worktrees` — where per-agent worktrees are created.
    worktrees: PathBuf,
    /// The HUB root (for `git worktree add` and resolving `trunk`).
    repo: PathBuf,
    /// `<current-worktree>/fleet` — the TRACKED source (roster + role bodies + contract + window.sh).
    /// Whoever runs `fleet` is in a worktree that has this checked out (it lives on `trunk`).
    src: PathBuf,
}

impl Fleet {
    /// Anchor RUNTIME state to the HUB (`.claude/` is gitignored, exists only at the hub — shared by
    /// every worktree via `--git-common-dir`, and it stays put after the bare conversion). Anchor the
    /// TRACKED source to the current worktree (`paths.repo` = `CARGO_MANIFEST_DIR`'s parent = the
    /// worktree root, which has `fleet/` checked out from `trunk`).
    fn new(paths: &Paths) -> Self {
        let hub = hub_root(&paths.repo).unwrap_or_else(|| paths.repo.clone());
        Fleet {
            root: hub.join(".claude/fleet"),
            worktrees: hub.join(".claude/worktrees"),
            repo: hub,
            src: paths.repo.join("fleet"),
        }
    }
    fn registry_path(&self) -> PathBuf {
        self.root.join("registry.json")
    }
    fn roster_path(&self) -> PathBuf {
        self.src.join("roster.json")
    }
    /// Load the tracked roster (the standing fleet). Empty if absent/malformed.
    fn load_roster(&self) -> Roster {
        match std::fs::read_to_string(self.roster_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!(
                    "fleet: roster {:?} is not valid JSON ({e})",
                    self.roster_path()
                );
                Roster::default()
            }),
            Err(_) => Roster::default(),
        }
    }
    /// Copy the tracked role bodies + contract + launcher into the runtime dir, so `window.sh` has a
    /// stable hub-anchored path and every window reads a consistent snapshot. Mirrors `xtask setup`'s
    /// tracked→.claude materialization. Idempotent.
    fn materialize_source(&self) {
        let loops_dst = self.root.join("loops");
        std::fs::create_dir_all(&loops_dst).ok();
        if let Ok(rd) = std::fs::read_dir(self.src.join("loops")) {
            for e in rd.filter_map(Result::ok) {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "md") {
                    let _ = std::fs::copy(&p, loops_dst.join(e.file_name()));
                }
            }
        }
        for f in ["AGENTS-fleet.md", "window.sh"] {
            let src = self.src.join(f);
            if src.exists() {
                let dst = self.root.join(f);
                let _ = std::fs::copy(&src, &dst);
                #[cfg(unix)]
                if f == "window.sh" {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
    }
    fn inbox(&self, agent: &str) -> PathBuf {
        self.root.join("inbox").join(agent)
    }
    fn window_sh(&self) -> PathBuf {
        self.root.join("window.sh")
    }
    fn stopfile(&self, agent: &str) -> PathBuf {
        self.root.join("stop").join(agent)
    }
    /// The durable REJECT-LOG (`<hub>/.claude/fleet/reject-log`): one rejected commit sha per line,
    /// appended by `send` whenever pr-sync delivers a `reject`. `fleet sync` consults it to refuse
    /// re-replaying a known-bad commit onto a fresh branch (the re-break-the-gate churn: a gate-failing
    /// commit that rode a discarded test-merge tree gets fleet-synced onto peer branches every tick).
    /// Machine-local (gitignored, like the rest of the runtime state) — a reject is a runtime event.
    fn reject_log_path(&self) -> PathBuf {
        self.root.join("reject-log")
    }

    fn load(&self) -> Registry {
        let p = self.registry_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!("fleet: {p:?} is not valid JSON ({e}); starting from an empty registry");
                Registry::default()
            }),
            Err(_) => Registry::default(),
        }
    }

    /// Persist the registry, pretty-printed, creating the fleet dir if needed. Written to a temp file
    /// then renamed so a concurrent reader never sees a half-written manifest.
    fn save(&self, reg: &Registry) {
        std::fs::create_dir_all(&self.root).expect("create .claude/fleet");
        let json = serde_json::to_string_pretty(reg).expect("serialize registry");
        let tmp = self.registry_path().with_extension("json.tmp");
        std::fs::write(&tmp, json).expect("write registry tmp");
        std::fs::rename(&tmp, self.registry_path()).expect("rename registry into place");
    }
}

// ── clap surface ────────────────────────────────────────────────────────────────────────────────

/// `cargo xtask fleet <subcommand>`.
#[derive(clap::Subcommand)]
pub enum FleetCmd {
    /// Reconstitute the fleet: for every `active` agent, ensure its worktree exists (create off
    /// `trunk` if missing) and ensure a tmux window named after it is running `window.sh`. Idempotent
    /// — a window that already exists is left alone, so re-running never duplicates anything. This is
    /// the "single command" that recreates the whole fleet after a reboot.
    Up,
    /// Stop every agent (mark `stopped`, drop each a stop-file the loop checks) but LEAVE the tmux
    /// windows open, so their scrollback survives for inspection.
    Down,
    /// Print the board: each agent's role/model/status, whether its tmux window is live, its inbox
    /// depth, the work-queue depth, and how far `trunk` is ahead of / behind `origin/main`.
    Status,
    /// (Re)install the fleet's git hooks into the hub's shared hooks dir, WITHOUT a full `up`. `up`
    /// installs them too, but this lets the operator/concierge deploy (or refresh) them on demand —
    /// e.g. to activate the trunk-clobber logger immediately without restarting the fleet. Idempotent
    /// and safe (never clobbers a foreign hook). See `install_git_hooks`.
    InstallHooks,
    /// Register a new agent, create its worktree off `trunk`, optionally seed a work item into its
    /// inbox, and bring it up. This is what the corpus-bugfix PM calls to mint a per-issue `fix`
    /// agent, and what the concierge calls to spin up a vertical on the operator's behalf.
    Add {
        /// Unique agent name (also the tmux window + inbox dir name).
        name: String,
        /// The role body under `.claude/fleet/loops/<role>.md`.
        #[arg(long)]
        role: String,
        /// For `--role vertical`: the feature owned (e.g. `iterators`).
        #[arg(long, default_value = "")]
        vertical: String,
        /// For `--role vertical`: the subsystem (`rcdzc` | `compiler-ml` | `runtime` | `guide` | …).
        #[arg(long, default_value = "")]
        area: String,
        /// The `/loop` interval (e.g. `10m`).
        #[arg(long, default_value = "10m")]
        interval: String,
        /// The Claude model alias (`opus` default; `fable` for the breaker). Resolved to the full
        /// 1M-context model id by the launcher.
        #[arg(long, default_value = "opus")]
        model: String,
        /// The reasoning-effort level (`low`|`medium`|`high`|`xhigh`|`max`).
        #[arg(long, default_value = "high")]
        effort: String,
        /// A file (e.g. a queue `.sexp`) to seed into the new agent's inbox as an `assign` message,
        /// so it starts with its one job in hand.
        #[arg(long)]
        seed: Option<PathBuf>,
    },
    /// Mark an agent `stopped`, drop its stop-file (the loop exits cleanly on its next tick), but keep
    /// the tmux window open for scrollback. A finished per-issue `fix` agent calls this on itself.
    ///
    /// With `--close`, ALSO kill the tmux window (`tmux kill-window`) after marking it stopped —
    /// reaping the panel so the session doesn't pile up hundreds of dead windows. The registry row is
    /// kept (status `stopped`) either way, so history/archive survive; only the window goes away.
    /// A fix agent NEVER `--close`s itself: the `corpus-bugfix` PM is the sole reaper — it verifies the
    /// fix truly landed on `trunk`, THEN `remove <fix-agent> --close`, so a wrongly-closed window can't
    /// lose an unfinished fix's scrollback.
    Remove {
        /// The agent to remove.
        name: String,
        /// Also kill the tmux window (reap the panel). Default keeps it open for scrollback.
        #[arg(long)]
        close: bool,
    },
    /// Deliver a message into another agent's inbox as one atomic JSON file. The whole fleet
    /// coordinates through this — see the message-kind table in AGENTS-fleet.md.
    ///
    /// After delivering, this WAKES the recipient: it nudges the recipient's tmux window into an
    /// immediate tick (`tmux send-keys "continue" Enter`) so a message is reacted to within seconds
    /// instead of waiting for the next scheduled `/loop`. Delivery == wake. The nudge is skipped for
    /// a stopped recipient, one with no live window, one already working (mid-tick — the message will
    /// be drained when it finishes), and — to protect a human — the interactive `concierge`/`design`
    /// windows. `/loop` remains the safety heartbeat, so a missed nudge is still eventually picked up.
    /// Pass `--no-wake` to deliver silently (e.g. seeding many messages in a batch).
    Send {
        /// Recipient agent name.
        #[arg(long)]
        to: String,
        /// Message kind (`merge-request` | `merged` | `reject` | `issue` | `assign` | `ask` |
        /// `answer` | `backlog` | `status` | `note`).
        #[arg(long)]
        kind: String,
        /// One-line subject.
        #[arg(long)]
        subject: String,
        /// A git sha or branch this message is about (e.g. the commit a `merge-request` names).
        #[arg(long, default_value = "")]
        r#ref: String,
        /// Free-form detail (may be multiline).
        #[arg(long, default_value = "")]
        body: String,
        /// Sender name. If omitted: falls back to `$FLEET_AGENT`, then to the current worktree's
        /// `fleet/<agent>` branch (so a forgotten `--from` still routes), then `unknown`. A
        /// reply-expecting kind (merge-request/ask/issue) is REFUSED if the sender resolves to
        /// `unknown` (its reply would dead-letter).
        #[arg(long)]
        from: Option<String>,
        /// Deliver without nudging the recipient's window awake (it will pick the message up on its
        /// next scheduled `/loop` tick). Use when seeding a batch of messages.
        #[arg(long)]
        no_wake: bool,
        /// Bypass the merge-request contamination guard (which refuses an MR whose `trunk..<ref>`
        /// range carries a commit touching ONLY another vertical's zone — a peer's work a load-spike
        /// sync-replay may have stranded on your branch). Use only for a GENUINELY cross-cutting
        /// change you really do own.
        #[arg(long)]
        force: bool,
    },
    /// Stamp an agent's `lastTick` — the presence heartbeat the loop calls at the top of every tick.
    Heartbeat {
        /// The agent stamping presence.
        name: String,
    },
    /// Print an agent's config as shell-safe `KEY=VALUE` lines (WORKTREE/ROLE/MODEL/INTERVAL/
    /// DISALLOW_ASK) — consumed by `window.sh` via `eval`. Exits non-zero if the agent is unknown.
    Describe {
        /// The agent to describe.
        name: String,
    },
    /// List an agent's queued inbox messages (oldest-first) at the canonical HUB path. This is the
    /// SAFE way for an agent to drain its inbox: the inbox lives at the MAIN repo's
    /// `.claude/fleet/inbox/<agent>/` (the hub), NOT the agent's worktree — an `ls`/glob of a RELATIVE
    /// `.claude/fleet/inbox/...` from the worktree cwd silently finds NOTHING, so the agent sees an
    /// empty inbox every tick and concludes "idle" while real messages (an `assign`, a `reject`) pile
    /// up unread (this cost v-try-operator ~8 ticks). `fleet inbox <me>` resolves the hub path the same
    /// way `send`/`heartbeat` do, so it can never mis-resolve, and it prints the resolved PATH + a LOUD
    /// `0 messages` line when empty (so a mis-set path shows as a visible anomaly, not silent idle).
    Inbox {
        /// The agent whose inbox to list.
        name: String,
    },
    /// Mirror the live gitignored work queue (`.claude/fleet/queue/`) into the TRACKED `issues/`
    /// archive at the repo root, so these hard-won reproducers are preserved in git history rather
    /// than living only in agent-local state. Copies every queue file into `issues/`, removes tracked
    /// files no longer in the queue (git history still holds them), and — unless `--no-commit` —
    /// commits the change. pr-sync runs this periodically; the commit flows onto `trunk` like any
    /// other. Run from a worktree that can commit (e.g. pr-sync's).
    Archive {
        /// Mirror into `issues/` but do not commit (leave the change staged for inspection).
        #[arg(long)]
        no_commit: bool,
    },
    /// Safely sync the current worktree onto the integrated `trunk` tip (contract step 4) WITHOUT
    /// orphaning an in-flight merge-request. A bare `git reset --hard trunk` moves your branch off the
    /// commit a queued MR's `--ref` points at, so pr-sync can't fetch it and silently SKIPS the MR
    /// forever. This resets onto `trunk`, then cherry-picks back ONLY the local commits not yet upstream
    /// by patch-id (dropping any pr-sync already landed, even under a re-parented sha), so the branch tip
    /// keeps containing your unlanded work + any queued `--ref` stays reachable. Refuses on a dirty tree
    /// and restores your pre-sync HEAD on any cherry-pick conflict, so it can never lose work. Also
    /// refuses (unless `--force`) if replaying would re-sha a commit that a merge-request you ALREADY
    /// sent pr-sync still points at — the cherry-pick gives it a new sha, orphaning the queued `--ref`;
    /// leave the branch alone until that MR resolves.
    Sync {
        /// Sync even if it would re-sha (and thus orphan) a merge-request you already have queued with
        /// pr-sync. By default `sync` REFUSES in that case. Use this only when the queued MR is dead
        /// (e.g. you're about to resend a fresh `--ref` anyway).
        #[arg(long)]
        force: bool,
    },
    /// Resolve a `merge-request` ATOMICALLY: reply to its sender AND archive the request in one step.
    /// This closes a reliability hole in the single-integrator model — pr-sync used to reply and move
    /// the request to `processed/` as two decoupled manual steps, so a missed reply left the sender
    /// idling forever on a silently-dropped MR (`process(mr)` MUST emit exactly one `merged`/`reject`).
    /// `fleet ack` reads the sender + ref from the request file, delivers the reply, then moves the
    /// request into `processed/` — you cannot archive without replying. pr-sync calls this instead of a
    /// bare `send` + hand-move. Run it from pr-sync's worktree (the inbox is hub-anchored either way).
    Ack {
        /// The merge-request file to resolve — a path, or just the basename in pr-sync's inbox.
        request: String,
        /// The outcome: `merged` (integrated) or `reject` (not integrated; body says why).
        #[arg(long)]
        outcome: String,
        /// The new `trunk` sha for a `merged`, or the branch/base for a `reject` (goes in the reply's
        /// `ref`). Optional.
        #[arg(long, default_value = "")]
        r#ref: String,
        /// The reply body — for `merged`, the gate summary / trunk sha; for `reject`, WHY + what to do
        /// (e.g. "conflict in X; rebase on trunk@<sha> and resend").
        #[arg(long, default_value = "")]
        body: String,
    },
    /// Audit the integration record for SILENT DROPS: every merge-request pr-sync archived into
    /// `processed/` MUST have produced exactly one `merged`/`reject` reply to its sender. This is the
    /// backstop for the intermittent bug where an MR is consumed without a reply — the sender's
    /// gated-green work then vanishes invisibly. For each processed merge-request, this looks for a
    /// reply (in the sender's inbox + `processed/`) whose `in_reply_to` names that request file, and
    /// reports any ORPHAN (archived, no reply). Requests archived before the `in_reply_to` field
    /// existed, or resolved by a hand-`send` instead of `fleet ack`, are reported as UNVERIFIABLE
    /// (not counted as orphans). It ALSO runs the mirror check — a merge-request still QUEUED in
    /// pr-sync's LIVE inbox whose `--ref` is already on trunk by patch-id (pr-sync landed the content
    /// under a re-parented sha but never acked the file); these no-op MRs would gate to an empty merge,
    /// so they're surfaced for a batch reject-as-superseded. Report-only by default (exit 0); pass
    /// `--strict` to exit non-zero when any orphan OR queued-but-landed MR is found. Run it from any
    /// worktree.
    Audit {
        /// Show every checked request, not just the orphans/unverifiable summary.
        #[arg(long)]
        verbose: bool,
        /// Exit non-zero if any orphan is found. OFF by default: `in_reply_to` correlation only exists
        /// for MRs resolved via `fleet ack`, so the historical backlog (pre-`ack`, or hand-`send`
        /// resolutions) reports as orphans that may actually have landed — noise for a gate. Use
        /// `--strict` once `fleet ack` is the universal resolution path (then an orphan is a real drop).
        #[arg(long)]
        strict: bool,
    },
    /// Reconcile the `inbox/unknown/` graveyard: replies that got addressed to `to=unknown` (the
    /// send-side identity bug — a merge-request arrived `from=unknown`, so pr-sync's reply went to
    /// `unknown`, which NOBODY drains). Each carries its real recipient in the subject
    /// (`<kind>: fleet/<agent>`). This re-routes each such message to that agent's inbox as a `note`
    /// (so it's clearly a reconciliation, not a live reply the sender might act on twice), skipping any
    /// whose recipient can't be derived. `--dry-run` reports what would move without moving it.
    RerouteUnknown {
        /// Report what would be re-routed without moving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// git MERGE DRIVER for `.duvet/coverage-floor.json` (registered by `fleet up`; not run by hand).
    /// The floor is a single monotonic counter every citation-adding agent bumps, so concurrent slices
    /// textually CONFLICT on it (`cited` 644 vs 645). This resolves such a conflict by taking the MAX
    /// of each field across the two sides — the floor only ever moves UP, so max(ours,theirs) is always
    /// the correct merged floor, and no citation slice ever conflicts on it again. git invokes it as
    /// `merge-floor <ours> <theirs>`: it reads both JSONs and OVERWRITES <ours> with the field-wise max.
    MergeFloor {
        /// `%A` — our side (the current branch's floor); the merged result is written back here.
        ours: PathBuf,
        /// `%B` — their side (the incoming floor).
        theirs: PathBuf,
    },
    /// OPTIMISTIC-BATCH + BISECT integration planner for `pr-sync` — the throughput cure for the
    /// per-MR-gate bottleneck (a batch of N MRs cost N full gate cycles; batches ran 13-30 min). This
    /// command PLANS the round on a throwaway scratch branch and prints a machine-readable decision per
    /// merge-request (LAND / BOUNCE-conflict / REJECT-broken); it does NOT advance `trunk` or reply to
    /// anyone — `pr-sync` stays the single writer and still `fleet ack`s each MR per its reply-invariant.
    ///
    /// The loop (mirrors the design in pr-sync.md step 2): (1) CONFLICT-PREFILTER — `git merge --no-ff`
    /// each queued MR's `--ref` onto a scratch branch off `trunk`, IN QUEUE ORDER; an MR that conflicts
    /// is dropped from the batch as BOUNCE (cheap — no gate), so the batch is the cleanly-mergeable set.
    /// (2) OPTIMISTIC GATE ONCE — run the full gate (`cargo xtask check`) on the combined scratch tree;
    /// green → the whole batch is LAND (one gate run for N MRs, the common+cheap case). (3) BISECT on
    /// red — binary-search the cleanly-merged set (re-form each half onto `trunk`, gate, recurse on the
    /// failing half) to isolate the culprit(s); each isolated bad MR is REJECT, then re-form the
    /// remainder and confirm green, so multiple independently-bad MRs are all found.
    ///
    /// `--dry-run` plans without leaving the scratch branch behind (always cleaned up regardless).
    GateBatch {
        /// Plan only; print the decision and delete the scratch branch (default true — this command is
        /// advisory to pr-sync, which executes the real trunk advance).
        #[arg(long, default_value_t = true)]
        dry_run: bool,
        /// Cap the number of MRs planned in one round (0 = all queued). A very large queue can be
        /// planned in bounded rounds.
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Self-heal the fleet, in two passes. RE-ARM: any ACTIVE agent whose `/loop` has stalled — each
    /// agent stamps a heartbeat touch-file (`.claude/fleet/heartbeat/<agent>`) at the top of every
    /// tick; if that file is older than `min(--stale-mult × interval, --stale-cap)`, its loop is
    /// presumed dead and this nudges the window back to life (`tmux send-keys continue Enter` — makes
    /// the idle agent run its next tick; NOT a bare `/loop <interval>`, which the loop skill treats as
    /// an empty-prompt no-op and never actually revives anything). ESCALATION: if an agent was re-armed
    /// and has NOT heartbeated since (a one-shot `continue` demonstrably didn't establish a self-
    /// sustaining loop — the fresh-mint cold-start whose recurring cron never armed: heartbeat stamped
    /// once, then frozen), the next re-arm re-issues the FULL `/loop <interval> <tick>` (same kickoff
    /// contract), which ARMS a cron instead of running one inline tick — converting that stall from
    /// permanent to self-healing. (Keyed on "heartbeat since the last re-arm", NOT "ever re-armed": the
    /// rearm marker is never cleared, so an agent nudged once long ago that recovered still has it, and
    /// must not be escalated on a later slow tick.) The `--stale-cap` bound is what keeps a
    /// long-interval agent (e.g. 30m) from getting an hour-long dead window. Skips: agents with no live
    /// tmux window, agents mid-tick ("esc to interrupt" — real work in flight, but ONLY trusted for an
    /// agent that has EVER stamped a heartbeat; a never-heartbeated agent past the cold-start window
    /// with a busy pane is FLAILING, not working, so it re-arms), agents re-armed within `--grace-secs` (anti-thrash),
    /// and `pr-sync` when `trunk` advanced within the stale window (it does minutes-long synchronous
    /// gate work per MR, so its heartbeat legitimately goes stale mid-batch while it's alive — a recent
    /// commit on `trunk`, which only pr-sync writes, proves liveness). REAP: any genuinely-DONE agent
    /// (registry status=stopped AND a stop-file present)
    /// whose tmux window is still live gets that window killed — role-agnostic, so it catches design/
    /// self-removed agents the PM's `remove --close` reaper never gets a note about. The registry row
    /// is kept (history/archive); only the panel goes away, and a `--grace-secs` window off the stop
    /// keeps a just-stopped agent's final scrollback glanceable for one cycle. INTEGRATION-RECORD
    /// SWEEP (report-only): each run also folds in `fleet audit`'s queued-but-already-landed check —
    /// merge-requests still in pr-sync's inbox whose `--ref` is already on trunk by patch-id (no-ops
    /// that would gate to empty merges) — and surfaces them. This lives here because the audit's value
    /// is inspecting LIVE hub state, which only exists on the fleet machine (a gitignored `.claude/
    /// fleet` means it can't run in CI). It only SURFACES them; pr-sync owns rejecting its own MRs.
    /// HEALTH LOG: on a real run (not `--dry-run`), a sweep that flagged ANYTHING (re-armed/reaped/
    /// drain-stall/saturated/queued-but-landed) appends one timestamped tab-separated summary line to
    /// `.claude/fleet/watchdog.log`, so a finding isn't lost when the terminal scrolls — the operator
    /// can `grep`/tail it to see recurrence. A clean sweep writes nothing (the log records anomalies).
    /// Meant to run from a
    /// short cron (~every 4 min); one pass then exit. This is the fleet's reliability backbone — a
    /// fleet-wide `/loop` stall froze every agent once, and stopped windows otherwise pile up.
    Watchdog {
        /// Report what WOULD be re-armed/reaped, but send no keys and kill no windows (safe anytime).
        #[arg(long)]
        dry_run: bool,
        /// Presume a loop stalled once its heartbeat is older than this multiple of its interval.
        #[arg(long, default_value_t = 2)]
        stale_mult: u32,
        /// CAP (seconds) on the GRACE portion of the stale window (`stale_window_secs` =
        /// `interval + min(interval×(mult-1), cap)`). The window always includes one full interval — a
        /// heartbeat is stamped at the TOP of each tick, so a healthy agent is legitimately silent for up
        /// to one interval between ticks and must NOT be judged stalled there. This cap bounds only the
        /// EXTRA patience beyond that one interval, so a dead loop is caught within `interval + cap` (a
        /// 30m agent within ~40min) rather than the unbounded interval×mult. Default 600s (10 min grace).
        #[arg(long, default_value_t = 600)]
        stale_cap: u64,
        /// Grace window (seconds), used two ways: don't re-arm an agent re-armed this recently (gives
        /// the nudge time to land), and don't reap a window whose agent stopped this recently (keeps
        /// its final scrollback glanceable for one cycle).
        #[arg(long, default_value_t = 120)]
        grace_secs: u64,
        /// OPT-IN: auto-nudge a probable DRAIN-STALL's idle pane with the canonical drain instruction
        /// (`cargo xtask fleet inbox <agent>`), instead of only reporting it. OFF by default (report-
        /// only) — auto-sending keystrokes is the highest-risk action, so it's the operator's choice to
        /// enable (the concierge runs its own watchdog with this set, since it owns tmux). Hard-guarded:
        /// only nudges a pane that is idle-at-prompt (not working) AND not context-saturated (a
        /// saturated pane needs a RESTART, not a nudge — reported, never nudged), and rate-limited to at
        /// most one nudge per agent per `--drain-nudge-grace` seconds so a truly-wedged one isn't spammed.
        #[arg(long)]
        nudge_drain_stalls: bool,
        /// Rate-limit (seconds) for `--nudge-drain-stalls`: don't re-nudge an agent auto-nudged within
        /// this window. Default 900s (15 min) — a nudged agent that's going to recover does so within a
        /// tick or two; a longer suppression avoids keystroke-spamming a genuinely-wedged one.
        #[arg(long, default_value_t = 900)]
        drain_nudge_grace: u64,
    },
}

/// A message as delivered into an inbox. Serialized one-per-file so delivery is a single atomic
/// rename and two agents never corrupt a shared file.
#[derive(Serialize, Deserialize)]
struct Message {
    from: String,
    to: String,
    kind: String,
    subject: String,
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    body: String,
    /// A per-process ordinal from `next_seq` (not wall-clock — the toolchain forbids `Date::now`).
    /// Metadata only: it is WRITTEN into every message but NOT read for ordering. The inbox sorts by
    /// FILENAME, whose leading field is the DURABLE cross-process `next_delivery_seq` counter — NOT
    /// this field, which is process-local and so is always 1 for a one-shot `fleet send`. Do not use
    /// `seq` to order messages (see `next_delivery_seq`); it is kept for on-disk-format compatibility.
    seq: u64,
    /// For a `merged`/`reject` reply emitted by `fleet ack`: the FILENAME of the merge-request it
    /// resolves (in pr-sync's inbox). Empty for every other message. `fleet audit` uses it to prove
    /// each archived merge-request got exactly one reply — the structural backstop against the silent
    /// drop (an MR moved to `processed/` with no reply). Absent on replies sent before this field
    /// existed / by a hand-`send` rather than `ack`, which audit reports as "unverifiable", not orphaned.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    in_reply_to: String,
}

pub fn run(paths: &Paths, cmd: FleetCmd) {
    let fleet = Fleet::new(paths);
    match cmd {
        FleetCmd::Up => up(&fleet),
        FleetCmd::Down => down(&fleet),
        FleetCmd::Status => status(&fleet),
        FleetCmd::InstallHooks => install_git_hooks(&fleet),
        FleetCmd::Add {
            name,
            role,
            vertical,
            area,
            interval,
            model,
            effort,
            seed,
        } => add(
            &fleet, name, role, vertical, area, interval, model, effort, seed,
        ),
        FleetCmd::Remove { name, close } => remove(&fleet, &name, close),
        FleetCmd::Send {
            to,
            kind,
            subject,
            r#ref,
            body,
            from,
            no_wake,
            force,
        } => send(
            &fleet, &to, &kind, &subject, &r#ref, &body, from, no_wake, force,
        ),
        FleetCmd::Heartbeat { name } => heartbeat(&fleet, &name),
        FleetCmd::Describe { name } => describe(&fleet, &name),
        FleetCmd::Inbox { name } => inbox_list(&fleet, &name),
        FleetCmd::Archive { no_commit } => archive(&fleet, no_commit),
        FleetCmd::Sync { force } => sync(&fleet, force),
        FleetCmd::Watchdog {
            dry_run,
            stale_mult,
            stale_cap,
            grace_secs,
            nudge_drain_stalls,
            drain_nudge_grace,
        } => watchdog(
            &fleet,
            dry_run,
            stale_mult,
            stale_cap,
            grace_secs,
            nudge_drain_stalls,
            drain_nudge_grace,
        ),
        FleetCmd::Ack {
            request,
            outcome,
            r#ref,
            body,
        } => ack(&fleet, &request, &outcome, &r#ref, &body),
        FleetCmd::Audit { verbose, strict } => audit(&fleet, verbose, strict),
        FleetCmd::RerouteUnknown { dry_run } => reroute_unknown(&fleet, dry_run),
        FleetCmd::MergeFloor { ours, theirs } => merge_floor(&ours, &theirs),
        FleetCmd::GateBatch { dry_run, limit } => gate_batch(&fleet, dry_run, limit),
    }
}

// ── up / down / status ────────────────────────────────────────────────────────────────────────

fn up(fleet: &Fleet) {
    // Materialize the tracked source (role bodies + contract + window.sh) into the runtime dir so
    // window.sh has a stable hub-anchored path. Then reconcile the tracked ROSTER into the runtime
    // registry: a standing agent declared in the roster but absent from the registry is added
    // (status active); an agent already in the registry keeps its runtime state (status, worktree).
    // This is what makes the fleet reproducible — a fresh clone's `fleet up` seeds every standing
    // agent from the committed roster. Ephemeral fix/design agents live only in the registry.
    fleet.materialize_source();
    register_merge_drivers(fleet);
    install_git_hooks(fleet);
    let mut reg = fleet.load();
    let roster = fleet.load_roster();
    let mut added = 0usize;
    for e in &roster.agents {
        if reg.agents.iter().any(|a| a.name == e.name) {
            continue; // already known — keep its runtime state
        }
        reg.agents.push(agent_from_roster(fleet, e));
        added += 1;
    }
    if added > 0 {
        fleet.save(&reg);
        println!("fleet up: seeded {added} standing agent(s) from the roster.");
    }
    if reg.agents.is_empty() {
        println!("fleet: no agents (empty roster + registry). Add one with `fleet add`.");
        return;
    }
    if !in_tmux() {
        eprintln!(
            "fleet up: not inside a tmux session (no $TMUX). Start/attach one first — the fleet\n\
             lives as named windows in your current session."
        );
        std::process::exit(1);
    }
    let session = tmux_current_session();
    for a in &reg.agents {
        if a.status != "active" {
            continue;
        }
        // Clear any stale stop-file so a re-activated agent's loop doesn't immediately exit.
        let _ = std::fs::remove_file(fleet.stopfile(&a.name));
        ensure_worktree(fleet, a);
        ensure_inbox(fleet, &a.name);
        ensure_window(fleet, &session, a);
    }
    println!(
        "fleet up: {} active agent(s) ensured in tmux session '{session}'.",
        reg.agents.iter().filter(|a| a.status == "active").count()
    );
    println!("  (windows already present were left running; re-run any time — it is idempotent.)");
}

/// Register the fleet's custom git merge drivers in the HUB's `.git/config` (idempotent). A custom
/// driver named in `.gitattributes` (`merge=fleet-maxfloor`) only activates if `merge.<name>.driver`
/// is configured, and that config is machine-local (not committed) — so `fleet up` sets it, the same
/// way it materializes other runtime state. The hub is single-machine, so one registration covers all
/// worktrees (they share the common `.git`). The `fleet-maxfloor` driver resolves a
/// `.duvet/coverage-floor.json` conflict by taking the field-wise MAX (via `xtask fleet merge-floor`),
/// so concurrent citation-floor bumps stop conflicting. `%A` = ours (result), `%B` = theirs.
/// The `fleet-maxfloor` driver command written into the hub's `.git/config`.
///
/// It MUST be worktree-portable: the value lives in the HUB-shared `.git/config`, but a merge can run
/// from ANY worktree, and each worktree has its OWN `target/` (none is shared). So we must NOT bake in
/// an absolute `std::env::current_exe()` path — that points into the ONE worktree that happened to run
/// `fleet up`, and won't exist for a merge driven from another worktree (or after that worktree is
/// rebuilt/removed), silently disabling the driver so baselines merge WITHOUT the max-dedup (PR #426).
/// The tracked `cargo xtask` alias (`.cargo/config.toml` is on `trunk`, so every worktree has it)
/// resolves from whatever worktree git runs the merge in — git invokes the driver with cwd at the
/// working-tree root, which is the cargo workspace root. `%A` = ours (result), `%B` = theirs.
fn maxfloor_driver_command() -> &'static str {
    "cargo xtask fleet merge-floor %A %B"
}

/// The fail-OPEN `reference-transaction` hook body. Installed by `fleet up` into the hub's shared
/// hooks dir; git runs it on EVERY ref update in EVERY worktree, so it is engineered to be incapable
/// of blocking or wedging anything: it acts only on the post-update `committed` state, touches only
/// `refs/heads/trunk`, does a couple of cheap `git` reads, appends one line to a log on a detected
/// backward move, and ALWAYS exits 0. (A `reference-transaction` hook that exits non-zero on the
/// `prepared` state ABORTS the transaction — that's exactly the fleet-wide git wedge we must avoid, so
/// this never returns non-zero and never keys off `prepared`.)
///
/// Purpose (concierge-approved, log-only): give exact TIMING + confirmation of the recurring
/// trunk-clobber — an out-of-band job resetting `trunk` backward to `origin/main` (single-writer
/// invariant violation, latent data-loss). It only records; the `fleet status` alarm reports the
/// reflog count, and an operator kills the source. Deliberately NOT a blocking guard.
const REF_TXN_HOOK_MARKER: &str = "# fleet:reference-transaction-clobber-logger";
fn reference_transaction_hook_body(log_path: &str) -> String {
    // Quote the log path in the shell redirect so a path with spaces/metachars is safe (PR #458).
    let q = format!("'{}'", log_path.replace('\'', "'\\''"));
    format!(
        "#!/usr/bin/env bash\n\
         {REF_TXN_HOOK_MARKER}\n\
         # Fail-OPEN trunk-clobber logger. NEVER blocks: acts only on the post-update `committed`\n\
         # state, logs a backward move of refs/heads/trunk, and always exits 0. See install_git_hooks.\n\
         set +e\n\
         state=\"$1\"\n\
         [ \"$state\" = \"committed\" ] || exit 0\n\
         while read -r old new ref; do\n\
         \t[ \"$ref\" = \"refs/heads/trunk\" ] || continue\n\
         \t# Only a real MOVE: skip a creation (old all-zeros) OR a deletion (new all-zeros) — PR #458.\n\
         \tcase \"$old\" in *[!0]*) : ;; *) continue ;; esac\n\
         \tcase \"$new\" in *[!0]*) : ;; *) continue ;; esac\n\
         \t# A backward/sideways move: new is NOT a descendant of old (a fast-forward would be).\n\
         \tgit merge-base --is-ancestor \"$old\" \"$new\" 2>/dev/null && continue\n\
         \t# Only the CLOBBER signature — a reset to origin/main — is worth logging. pr-sync's own\n\
         \t# legit history rewrites (git commit --amend, a rebase) are also non-FF moves but are NOT\n\
         \t# clobbers; logging them makes the operator's signal noisy (the 07:56 --amend false-positive).\n\
         \t# So require new == origin/main (resolve it fresh; skip if it can't be resolved).\n\
         \tom=$(git rev-parse --verify -q origin/main 2>/dev/null) || continue\n\
         \t[ \"$new\" = \"$om\" ] || continue\n\
         \tts=\"$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo unknown)\"\n\
         \t# (Target is always trunk since we scoped to it — don't resolve $new, it is a SHA — PR #459.)\n\
         \techo \"$ts trunk NON-FF move ${{old:0:12}} -> ${{new:0:12}} (-> origin/main) pid=$$ ppid=$PPID\" >> {q} 2>/dev/null || true\n\
         \t# The clobber writer is SUB-SECOND, so external `ps` post-mortem can't trace it. Capture\n\
         \t# the ancestry INLINE now — the hook runs SYNCHRONOUSLY in the writer's own process, so\n\
         \t# its parent chain is still alive. Walk up from PPID ~5 levels, logging each command line;\n\
         \t# that argv (the parent shell's `-c <script>`, the cron, …) NAMES the source.\n\
         \tp=$PPID\n\
         \tfor _ in 1 2 3 4 5; do\n\
         \t\t{{ [ -n \"$p\" ] && [ \"$p\" != 0 ] && [ \"$p\" != 1 ]; }} || break\n\
         \t\tline=$(ps -o ppid=,args= -p \"$p\" 2>/dev/null)\n\
         \t\t[ -n \"$line\" ] || break\n\
         \t\techo \"  ^ pid=$p $line\" >> {q} 2>/dev/null || true\n\
         \t\tp=$(echo \"$line\" | awk '{{print $1}}')\n\
         \tdone\n\
         done\n\
         exit 0\n"
    )
}

/// Install the fleet's git hooks into the hub's shared hooks dir (idempotent, best-effort). Today:
/// the fail-open `reference-transaction` clobber logger. Hooks live under `<git-common-dir>/hooks/`
/// (shared by all worktrees). We only ever (over)write a hook that carries OUR marker line — a
/// pre-existing foreign hook of the same name is left untouched + a warning printed, so we never
/// clobber a hand-placed hook (e.g. the existing `pre-commit` trunk guard). This makes the hook
/// REPRODUCIBLE via `fleet up` rather than hand-placed (the pre-commit guard currently is not).
fn install_git_hooks(fleet: &Fleet) {
    // Resolve the shared hooks dir: `<git-common-dir>/hooks`. The hub is bare; its common dir is
    // `<hub>/.git`. Ask git so we're robust to a non-standard layout.
    let common = Command::new("git")
        .current_dir(&fleet.repo)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .unwrap_or_else(|| fleet.repo.join(".git"));
    let hooks = common.join("hooks");
    if std::fs::create_dir_all(&hooks).is_err() {
        eprintln!(
            "fleet: WARNING — could not create hooks dir {}; skipping git-hook install.",
            hooks.display()
        );
        return;
    }
    let log_path = fleet.root.join("trunk-clobber.log");
    let hook_path = hooks.join("reference-transaction");
    // Don't clobber a foreign hook of the same name — only overwrite one we own (marker present).
    if let Ok(existing) = std::fs::read_to_string(&hook_path)
        && !existing.contains(REF_TXN_HOOK_MARKER)
    {
        eprintln!(
            "fleet: WARNING — a non-fleet `reference-transaction` hook already exists at {}; leaving \
             it (not installing the clobber logger). Merge it by hand if you want clobber logging.",
            hook_path.display()
        );
        return;
    }
    let body = reference_transaction_hook_body(&log_path.to_string_lossy());
    if std::fs::write(&hook_path, &body).is_err() {
        eprintln!(
            "fleet: WARNING — could not write the reference-transaction hook to {}.",
            hook_path.display()
        );
        return;
    }
    // Make it executable (0o755); a non-executable hook is silently ignored by git.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755));
    }
    println!(
        "fleet: installed the fail-open trunk-clobber logger at {} (logs backward trunk moves to {}).",
        hook_path.display(),
        log_path.display()
    );
}

fn register_merge_drivers(fleet: &Fleet) {
    let set = |key: &str, val: &str| -> bool {
        Command::new("git")
            .current_dir(&fleet.repo)
            .args(["config", key, val])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    // Surface a failed registration instead of swallowing it (`.status().ok()` hid PR #426's failure
    // mode): if the driver isn't registered the .gitattributes `merge=fleet-maxfloor` line silently
    // does nothing, reintroducing the coverage-floor conflict tax with no signal.
    let ok_name = set(
        "merge.fleet-maxfloor.name",
        "max of the two coverage floors",
    );
    let ok_driver = set("merge.fleet-maxfloor.driver", maxfloor_driver_command());
    if !ok_name || !ok_driver {
        eprintln!(
            "fleet: WARNING — failed to register the fleet-maxfloor merge driver in {}/.git/config; \
             concurrent .duvet/coverage-floor.json bumps will fall back to a normal git conflict.",
            fleet.repo.display()
        );
    }
}

/// Build a runtime [`Agent`] from a tracked [`RosterEntry`], deriving the runtime-only fields (branch,
/// worktree path, status, disallow_ask) the same way `add` does.
fn agent_from_roster(fleet: &Fleet, e: &RosterEntry) -> Agent {
    let branch = if e.role == "pr-sync" {
        TRUNK.to_string()
    } else {
        format!("fleet/{}", e.name)
    };
    Agent {
        name: e.name.clone(),
        role: e.role.clone(),
        vertical: e.vertical.clone(),
        area: e.area.clone(),
        worktree: fleet.worktrees.join(&e.name).to_string_lossy().to_string(),
        branch,
        interval: e.interval.clone(),
        model: e.model.clone(),
        effort: e.effort.clone(),
        status: "active".to_string(),
        disallow_ask: !matches!(e.role.as_str(), "concierge" | "design"),
    }
}

fn down(fleet: &Fleet) {
    let mut reg = fleet.load();
    std::fs::create_dir_all(fleet.root.join("stop")).expect("create stop dir");
    for a in reg.agents.iter_mut() {
        a.status = "stopped".to_string();
        // Drop a stop-file the loop checks at the top of each tick, so it exits cleanly.
        std::fs::write(fleet.stopfile(&a.name), "stopped by `fleet down`\n").ok();
    }
    fleet.save(&reg);
    println!(
        "fleet down: {} agent(s) marked stopped (stop-files dropped). tmux windows left OPEN for\n\
         scrollback — close them by hand when done, or `fleet up` to restart.",
        reg.agents.len()
    );
}

/// Whether an agent's row should carry the `⚠STALE` flag on the board. Pure so it can be unit-tested
/// and so the board's verdict provably MATCHES the watchdog's re-arm gate (a mismatch would show a
/// stall the watchdog silently ignores, or vice-versa — either way the operator loses trust in the
/// board). An agent reads stale only when it is `active`, has a live window, and its heartbeat age
/// exceeds its stale window. The one exemption mirrors `watchdog`: for `pr-sync`, a `trunk` commit
/// newer than the stale window proves liveness (only pr-sync writes `trunk`), so a stale heartbeat
/// mid-batch is NOT a stall. `trunk_commit_age` is `None` for every non-pr-sync agent (and for
/// pr-sync when trunk's age is unknown), in which case only the heartbeat gate applies.
fn agent_reads_stale(
    name: &str,
    agent_status: &str,
    has_window: bool,
    hb_age: u64,
    window_secs: u64,
    trunk_commit_age: Option<u64>,
) -> bool {
    if agent_status != "active" || !has_window || hb_age <= window_secs {
        return false;
    }
    // pr-sync trunk-advance exemption — a fresh trunk commit means it's alive integrating, not stalled.
    if name == "pr-sync"
        && let Some(commit_age) = trunk_commit_age
        && commit_age <= window_secs
    {
        return false;
    }
    true
}

/// Is `line` the redundant title line of `<name>`'s status file — `# <name> status` or
/// `# <name> — status` (any heading/emphasis/dash punctuation)? Matched so it's skipped in favor of the
/// first REAL status line. Strips only markdown markers and the `—`/`|` separators (NOT the `-` inside a
/// hyphenated agent name like `v-fleet-tooling`), then checks the words are exactly `<name>` + `status`.
fn is_status_title_line(line: &str, name: &str) -> bool {
    let stripped: String = line
        .chars()
        .map(|c| {
            if matches!(c, '#' | '*' | '—' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = stripped.split_whitespace().collect();
    words.len() == 2
        && words[0].eq_ignore_ascii_case(name)
        && words[1].eq_ignore_ascii_case("status")
}

/// The agent's self-reported status HEADLINE: the first substantive line of `status/<agent>.md`, with
/// leading markdown heading `#`/bold `*` markers stripped and truncated to keep the board tidy. `None`
/// if the file is absent or has no substantive line. This is what an agent chooses to say about its own
/// state — folded onto the board so `fleet status` shows self-report alongside registry/heartbeat facts.
fn status_headline(fleet: &Fleet, name: &str) -> Option<String> {
    let path = fleet.root.join("status").join(format!("{name}.md"));
    let text = std::fs::read_to_string(&path).ok()?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_status_title_line(l, name))?;
    // Strip leading markdown noise (`#`, `*`, `-`, spaces) so the headline reads cleanly, then drop any
    // bold `**` markers left in the middle (e.g. `**Did:** …` → `Did: …`).
    let cleaned = line
        .trim_start_matches(['#', '*', '-', ' '])
        .replace("**", "");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }
    // Truncate on a char boundary to keep one line; ellipsize if cut.
    const MAX: usize = 96;
    let headline = if cleaned.chars().count() > MAX {
        let cut: String = cleaned.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        cleaned.to_string()
    };
    Some(headline)
}

fn status(fleet: &Fleet) {
    let reg = fleet.load();
    let session = if in_tmux() {
        Some(tmux_current_session())
    } else {
        None
    };
    let live_windows = session.as_deref().map(tmux_windows).unwrap_or_default();
    let now = now_unix();

    println!("Fleet board ({} agent(s)):", reg.agents.len());
    println!(
        "  {:<18} {:<13} {:<7} {:<8} {:<7} {:<9} INBOX",
        "AGENT", "ROLE", "MODEL", "STATUS", "WINDOW", "HB-AGE"
    );
    let mut stale = 0usize;
    for a in &reg.agents {
        let has_window = live_windows.iter().any(|w| w == &a.name);
        let window = if has_window { "live" } else { "-" };
        let inbox = inbox_depth(fleet, &a.name);
        let role = if a.vertical.is_empty() {
            a.role.clone()
        } else {
            format!("{}:{}", a.role, a.vertical)
        };
        // Heartbeat age + a stale flag: an ACTIVE, windowed agent whose heartbeat is older than its
        // stale window (the same bound `fleet watchdog` re-arms on) is almost certainly a dead loop —
        // surface it here so the board is a single pane of glass for the stall problem, not just the
        // watchdog's job. A stopped/never-stamped agent shows a plain age with no flag.
        let (hb_age, flag) = match heartbeat_age_secs(fleet, &a.name, now) {
            None => ("never".to_string(), ""),
            Some(age) => {
                let window_secs = stale_window_secs(parse_interval_secs(&a.interval), 2, 600);
                // Mirror the watchdog's pr-sync exemption so the board and the watchdog AGREE: pr-sync
                // does minutes-long synchronous gate work per MR, so its heartbeat legitimately goes
                // stale mid-batch — a recent commit on `trunk` (which only pr-sync writes) proves it's
                // alive. Only pr-sync's row consults trunk; every other agent passes None (unread).
                let trunk_commit_age = if a.name == "pr-sync" {
                    last_commit_age_secs(&fleet.repo, TRUNK)
                } else {
                    None
                };
                let is_stale = agent_reads_stale(
                    &a.name,
                    &a.status,
                    has_window,
                    age,
                    window_secs,
                    trunk_commit_age,
                );
                if is_stale {
                    stale += 1;
                }
                (fmt_age(age), if is_stale { " ⚠STALE" } else { "" })
            }
        };
        println!(
            "  {:<18} {:<13} {:<7} {:<8} {:<7} {:<9} {}{}",
            a.name, role, a.model, a.status, window, hb_age, inbox, flag
        );
        // Fold the agent's OWN self-reported status headline (first substantive line of its
        // `status/<agent>.md`) onto an indented sub-line, so the board shows what each agent SAYS it's
        // doing — not just registry/heartbeat facts. Absent file → no sub-line (silent).
        if let Some(headline) = status_headline(fleet, &a.name) {
            println!("      ↳ {headline}");
        }
    }
    if stale > 0 {
        println!(
            "\n  ⚠ {stale} active agent(s) STALE (heartbeat past their stale window) — \
             `cargo xtask fleet watchdog` will re-arm them."
        );
    }

    // Work queue depth (breaker/fuzzer produce, the PM consumes). Count only un-handled items.
    let queue = fleet.root.join("queue");
    let qn = count_dir(&queue, |name| {
        !name.contains(".RESOLVED.") && !name.contains(".REJECTED.")
    });
    println!("\n  queue: {qn} open work item(s) in {}", queue.display());

    // trunk vs origin/main, so the operator sees the publish backlog at a glance.
    let trunk_om = trunk_vs_origin_main(&fleet.repo);
    if let Some((ahead, behind)) = trunk_om {
        println!("  trunk: {ahead} ahead / {behind} behind origin/main");
    }

    // Trunk-ref-regression watch. `trunk` should only ever move FORWARD (pr-sync cherry-picks). A
    // `reset: moving to origin/main` in its reflog is a backward move — root-caused to pr-sync's own
    // PUBLISH re-parent when done in-place (`git reset --hard origin/main` in its trunk worktree)
    // rather than in a scratch worktree; the fix is the scratch-worktree publish (pr-sync.md step 3).
    // A past reset that trunk has since ADVANCED PAST is benign history (pr-sync stashed + replayed),
    // so only WARN LOUDLY when trunk is currently AT/BEHIND origin/main (a regression still in effect);
    // otherwise note it quietly as self-recovered. Read-only.
    if let Some(n) = trunk_clobber_count(&fleet.repo, 40)
        && n > 0
    {
        // Only trunk being DEMONSTRABLY ahead (Some(ahead > 0)) proves the reset self-recovered and
        // downgrades to the quiet note. `None` (couldn't compare — origin/main unresolved) must NOT be
        // read as "ahead" (PR #463): that would suppress the warning on the very state we can't verify.
        // So warn LOUDLY unless we can prove trunk is ahead: Some(0) = regression in effect;
        // None = can't tell, still surface it.
        match trunk_om {
            Some((ahead, _)) if ahead > 0 => println!(
                "  · trunk: {n} `reset: moving to origin/main` in the recent reflog, but trunk is ahead \
                 again (self-recovered — pr-sync's in-place publish re-parent; prefer the scratch-worktree \
                 form so the ref never regresses)."
            ),
            Some(_) => println!(
                "  ⚠ trunk REGRESSED: {n} `reset: moving to origin/main` in the last 40 reflog entries \
                 AND trunk is not ahead of origin/main — a backward reset is currently IN EFFECT. This \
                 is pr-sync's publish re-parent resetting the trunk ref in-place; it should re-parent \
                 in a SCRATCH worktree (pr-sync.md step 3) so trunk only moves forward."
            ),
            None => println!(
                "  ⚠ trunk: {n} `reset: moving to origin/main` in the last 40 reflog entries, and \
                 trunk-vs-origin/main could NOT be compared (origin/main unresolved) — can't confirm \
                 it self-recovered. Check `git rev-list --left-right --count origin/main...trunk`; if \
                 trunk is behind, it's a live regression (pr-sync's in-place publish re-parent — should \
                 use the scratch-worktree form, pr-sync.md step 3)."
            ),
        }
    }
}

// ── add / remove ──────────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn add(
    fleet: &Fleet,
    name: String,
    role: String,
    vertical: String,
    mut area: String,
    interval: String,
    model: String,
    effort: String,
    seed: Option<PathBuf>,
) {
    // Materialize the tracked source so a freshly-added agent's role body is present in the runtime
    // dir window.sh reads from.
    fleet.materialize_source();
    // Validate the role has a body (in the tracked source), so a typo doesn't silently create an
    // agent that can't launch.
    let body = fleet.src.join("loops").join(format!("{role}.md"));
    if !body.exists() {
        eprintln!(
            "fleet add: no role body at {} — valid roles are the files in .claude/fleet/loops/",
            body.display()
        );
        std::process::exit(1);
    }
    let mut reg = fleet.load();
    if reg.agents.iter().any(|a| a.name == name) {
        eprintln!("fleet add: an agent named '{name}' already exists (names must be unique)");
        std::process::exit(1);
    }
    if role == "vertical" && area.is_empty() {
        area = "rcdzc".to_string(); // the common case; a vertical elsewhere passes --area
    }
    // pr-sync is the sole holder of `trunk`; every other agent gets its own topic branch named for it.
    let branch = if role == "pr-sync" {
        TRUNK.to_string()
    } else {
        format!("fleet/{name}")
    };
    let worktree = fleet.worktrees.join(&name);
    // The INTERACTIVE roles keep AskUserQuestion — they talk to the operator by design. The
    // `concierge` is the standing human interface; a `design` agent is an on-demand interactive
    // session the operator switches to and iterates with. Every other role runs unattended and is
    // denied the human-prompt tool (it routes anything human-shaped to the concierge as an `ask`).
    let disallow_ask = !matches!(role.as_str(), "concierge" | "design");

    let agent = Agent {
        name: name.clone(),
        role: role.clone(),
        vertical,
        area,
        worktree: worktree.to_string_lossy().to_string(),
        branch,
        interval,
        model,
        effort,
        status: "active".to_string(),
        disallow_ask,
    };

    ensure_worktree(fleet, &agent);
    ensure_inbox(fleet, &name);
    let _ = std::fs::remove_file(fleet.stopfile(&name));

    // Seed the one work item into the new agent's inbox as an `assign`, so a fix agent starts with
    // its job in hand. Copy the seed file alongside so the agent can read the full case.
    if let Some(seed) = seed {
        if let Ok(text) = std::fs::read_to_string(&seed) {
            let fname = seed
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "seed".into());
            let dest = fleet.inbox(&name).join(&fname);
            std::fs::write(&dest, &text).ok();
            deliver(
                fleet,
                &Message {
                    from: "fleet".into(),
                    to: name.clone(),
                    kind: "assign".into(),
                    subject: format!("own this issue: {fname}"),
                    r#ref: fname,
                    body: format!("Seed case copied to your inbox at {}.", dest.display()),
                    seq: next_seq(),
                    in_reply_to: String::new(),
                },
            );
        } else {
            eprintln!(
                "fleet add: could not read seed {} — added agent without a seed",
                seed.display()
            );
        }
    }

    reg.agents.push(agent);
    fleet.save(&reg);
    println!("fleet add: registered '{name}' (role={role}). Bringing up its window…");

    // Bring just this one up if we're in tmux; otherwise it'll come up on the next `fleet up`.
    if in_tmux() {
        let session = tmux_current_session();
        let a = reg.agents.last().unwrap().clone();
        ensure_window(fleet, &session, &a);
    } else {
        println!(
            "  (not in tmux — run `cargo xtask fleet up` from your tmux session to launch it.)"
        );
    }
}

fn remove(fleet: &Fleet, name: &str, close: bool) {
    let mut reg = fleet.load();
    let Some(a) = reg.agents.iter_mut().find(|a| a.name == name) else {
        eprintln!("fleet remove: no agent named '{name}'");
        std::process::exit(1);
    };
    a.status = "stopped".to_string();
    std::fs::create_dir_all(fleet.root.join("stop")).expect("create stop dir");
    std::fs::write(fleet.stopfile(name), "removed by `fleet remove`\n").ok();
    fleet.save(&reg);
    if close {
        // Reap the tmux window too (the PM does this after verifying a fix landed). The registry row
        // stays `stopped` — only the panel goes away, so history/archive survive. Report the exact
        // reason when nothing was killed, so "already gone" isn't confused with "tmux errored".
        let prefix = format!("fleet remove --close: '{name}' marked stopped");
        if !in_tmux() {
            println!("{prefix}; not in a tmux session, so no window to kill. Registry row kept.");
        } else {
            match kill_window(&tmux_current_session(), name) {
                KillOutcome::Killed => println!(
                    "{prefix} AND its tmux window killed (panel reaped; registry row kept)."
                ),
                KillOutcome::NotFound => println!(
                    "{prefix}; no live tmux window by that name (already closed). Registry row kept."
                ),
                KillOutcome::TmuxError => eprintln!(
                    "{prefix}, but `tmux kill-window` FAILED (tmux missing or errored) — the window \
                     may still be open; close it by hand. Registry row kept."
                ),
            }
        }
    } else {
        println!(
            "fleet remove: '{name}' marked stopped (stop-file dropped; its loop exits next tick).\n\
             Its tmux window is left OPEN for scrollback."
        );
    }
}

// ── send / heartbeat / describe ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn send(
    fleet: &Fleet,
    to: &str,
    kind: &str,
    subject: &str,
    r#ref: &str,
    body: &str,
    from: Option<String>,
    no_wake: bool,
    force: bool,
) {
    // Resolve the sender robustly. Priority: explicit `--from`, then `$FLEET_AGENT`, then DERIVE it
    // from the current worktree's branch (`fleet/<agent>` → `<agent>`). The derivation is the key
    // hardening: an agent that forgets `--from` (and whose env lacks FLEET_AGENT) still sends under its
    // real name, instead of `from=unknown` — which dead-letters pr-sync's merged/reject reply and
    // silently loses the sender's knowledge that its MR landed/bounced (a confirmed fleet-wide drop
    // amplifier). Only if NONE of those resolve do we fall to `unknown`.
    let from = from
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("FLEET_AGENT")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| sender_from_branch(fleet))
        .unwrap_or_else(|| "unknown".to_string());
    // The resolved `from` becomes the RECIPIENT of any reply pr-sync sends back (it addresses the
    // reply to `mr.from`), and `deliver` validates the recipient name. So an INVALID `from` — e.g.
    // `--from foo/bar`, or a branch `fleet/foo/bar` deriving `foo/bar` — would pass here but make the
    // REPLY dead-letter at deliver-time, leaving the sender stuck (never sees merged/reject). Validate
    // the resolved sender HERE, at the source, and refuse early with a clear error, so a malformed
    // sender can never silently lose its reply. (`unknown` is the sentinel handled just below.)
    if from != "unknown"
        && let Err(why) = validate_agent_name(&from)
    {
        eprintln!(
            "fleet send: REFUSING — resolved sender `{from}` is not a valid agent name ({why}). A reply \
             addressed back to it would dead-letter at delivery, stranding you. Pass a valid \
             `--from <agent>` (ASCII alphanumerics + `-`; no slashes/dots), or fix your branch name."
        );
        std::process::exit(1);
    }
    // A reply-EXPECTING message (the sender waits for a merged/reject/answer) MUST have a real sender,
    // or the reply dead-letters and the sender idles forever. Refuse rather than send into the void.
    if from == "unknown" && matches!(kind, "merge-request" | "ask" | "issue") {
        eprintln!(
            "fleet send: REFUSING a `{kind}` from an UNRESOLVED sender (would dead-letter the reply). \
             Pass `--from <your-agent-name>` explicitly (or run from your agent worktree so it derives \
             from the `fleet/<agent>` branch). A merge-request/ask/issue needs a routable sender."
        );
        std::process::exit(1);
    }
    // A `merge-request` with no `--ref` is malformed: pr-sync integrates by the commit sha in `ref`,
    // and an empty one forces it to parse the body / guess — which has caused a premature merged-ack
    // against the WRONG commit. Warn loudly (still deliver — non-fatal) so the sender fixes it.
    if kind == "merge-request" && r#ref.trim().is_empty() {
        eprintln!(
            "⚠ fleet send: merge-request with an EMPTY --ref. pr-sync resolves by commit sha; \
             pass `--ref $(git rev-parse HEAD)` so it integrates + acks the right commit (an empty \
             ref has caused a mis-verified merged-ack). Delivering anyway."
        );
    }
    // CONTAMINATION GUARD (v-compiler-ml issue 10801): during an extreme-load window a fleet-sync
    // replay can strand a PEER's unlanded commit onto this branch (the shared-base HEAD/reflog vector).
    // If that stray commit rides along in a merge-request's `trunk..<ref>` range, the sender would
    // submit another vertical's work under its OWN MR — a wrong-attribution merge (or an empty/
    // conflicting gate). The sync-side guard only WARNS (dropping on the reset+replay hot-path could
    // lose real work); at SEND it's the last safe moment and refusing loses NOTHING (the commit still
    // exists), so here we REFUSE — with a `--force` escape for a genuinely cross-cutting change. Signal
    // = the same sound CHANGED-PATHS-vs-roster-ZONE test the sync guard uses. Scoped to `merge-request`
    // with a resolvable ref + a known sender zone; skipped silently (deliver) if any input is missing,
    // so a normal in-lane MR is never impeded. `--force` bypasses (a real cross-zone change is legal).
    if !force
        && kind == "merge-request"
        && !r#ref.trim().is_empty()
        && from != "unknown"
        && let Some(foreign) = merge_request_foreign_commits(fleet, &from, r#ref.trim())
        && !foreign.is_empty()
    {
        eprintln!(
            "fleet send: REFUSING — {} commit(s) in the range `trunk..{}` this merge-request \
             names touch ONLY files outside your lane (zone '{}'), i.e. a PEER's work your base \
             likely carried after a load-spike sync replay. Submitting it would merge another \
             vertical's commit under YOUR name (wrong-attribution / stale-dup gate):",
            foreign.len(),
            &r#ref.trim()[..r#ref.trim().len().min(9)],
            roster_zone_of(fleet, &from).unwrap_or_default(),
        );
        for sha in &foreign {
            eprintln!("    {}", &sha[..sha.len().min(9)]);
        }
        eprintln!(
            "  Reset your branch to clean trunk + cherry-pick back ONLY your own commits, then \
             resend. If this MR is a GENUINELY cross-cutting change (you really do own these \
             files), re-run with `--force`."
        );
        std::process::exit(1);
    }

    // Deliver even if the recipient isn't in the registry yet (e.g. seeding before add commits) —
    // the inbox dir is created on demand.
    deliver(
        fleet,
        &Message {
            from: from.clone(),
            to: to.to_string(),
            kind: kind.to_string(),
            subject: subject.to_string(),
            r#ref: r#ref.to_string(),
            body: body.to_string(),
            seq: next_seq(),
            in_reply_to: String::new(),
        },
    );
    println!("fleet send: {from} → {to} [{kind}] {subject}");

    // Record a rejected commit sha in the durable reject-log so `fleet sync` can refuse to re-replay a
    // known-bad commit onto a fresh branch (the re-break-the-gate churn a gate-failing commit causes as
    // it rides discarded test-merge trees onto peer branches). Best-effort; a reject with no ref is a
    // no-op. Covers a direct `fleet send --kind reject`; the merge-request reply path records via `ack`.
    if kind == "reject" {
        append_reject_log(&fleet.reject_log_path(), r#ref);
    }

    // Wake the recipient so it reacts to this message NOW rather than at its next scheduled tick.
    // Delivery == wake. `/loop` stays the safety net for any nudge that doesn't land.
    if !no_wake {
        match wake_window(fleet, to) {
            WakeOutcome::Woke => println!("  ↑ nudged '{to}' awake (immediate tick)"),
            WakeOutcome::Skipped(why) => println!("  (not woken: {why} — picks it up next /loop)"),
        }
    }
}

/// Resolve a `merge-request` atomically: reply to its sender, THEN archive the request — so a request
/// can never be archived without a reply (the silent-drop hole that left senders idling forever). See
/// the `Ack` doc comment. Reads the sender + ref from the request file; the outcome must be `merged`
/// or `reject`.
fn ack(fleet: &Fleet, request: &str, outcome: &str, r#ref: &str, body: &str) {
    if outcome != "merged" && outcome != "reject" {
        eprintln!("fleet ack: --outcome must be `merged` or `reject` (got `{outcome}`)");
        std::process::exit(1);
    }
    // Resolve the request file: an explicit path to an existing file, OR a bare basename in pr-sync's
    // inbox. The basename branch joins `request` into a path, so an unvalidated value like
    // `../registry.json` (or `processed/../…`) would make ack READ then RENAME a file OUTSIDE the
    // inbox — a path-traversal write primitive. So in the basename branch require a single, safe path
    // component (no separators, no `.`/`..`). An explicit existing-file path is trusted as-is (the
    // caller is pr-sync naming a real request file; `is_file()` already proves it exists).
    let path = {
        let p = PathBuf::from(request);
        if p.is_file() {
            p
        } else {
            if !is_safe_component(request) {
                eprintln!(
                    "fleet ack: request {request:?} is neither an existing file nor a safe inbox \
                     basename (no path separators, no `.`/`..`) — refusing (path-traversal guard)"
                );
                std::process::exit(1);
            }
            fleet.inbox("pr-sync").join(request)
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "fleet ack: cannot read merge-request {}: {e}",
                path.display()
            );
            std::process::exit(1);
        }
    };
    let mr: Message = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fleet ack: {} is not a valid message ({e})", path.display());
            std::process::exit(1);
        }
    };
    if mr.kind != "merge-request" {
        eprintln!(
            "fleet ack: {} is a `{}`, not a `merge-request` — refusing to ack",
            path.display(),
            mr.kind
        );
        std::process::exit(1);
    }

    // 1) Reply to the sender FIRST (deliver, then archive) — if the archive somehow fails, the sender
    // has still been told, which is the safe direction: a stray un-archived request re-processes
    // (idempotent-ish: a second ack just sends a second reply), whereas a lost reply is invisible.
    let subject = if outcome == "merged" {
        format!("merged: {}", mr.subject)
    } else {
        format!("reject: {}", mr.subject)
    };
    // Record WHICH merge-request this reply resolves (its filename), so `fleet audit` can prove the
    // request↔reply pairing and catch a silent drop (an archived MR with no matching reply).
    let request_fname = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    deliver(
        fleet,
        &Message {
            from: "pr-sync".to_string(),
            to: mr.from.clone(),
            kind: outcome.to_string(),
            subject,
            r#ref: r#ref.to_string(),
            body: body.to_string(),
            seq: next_seq(),
            in_reply_to: request_fname,
        },
    );
    println!(
        "fleet ack: pr-sync → {} [{outcome}] {}",
        mr.from, mr.subject
    );
    // On a REJECT, record the REJECTED COMMIT'S sha (the merge-request's own `ref`, not ack's `--ref`
    // which may name trunk) in the durable reject-log, so `fleet sync` refuses to re-replay this
    // known-bad commit onto a fresh branch. Best-effort; empty ref is a no-op. This is the primary
    // reject path (pr-sync rejects merge-requests via `ack`); the direct `send --kind reject` path
    // records too.
    if outcome == "reject" {
        append_reject_log(&fleet.reject_log_path(), &mr.r#ref);
    }

    // 2) Archive the request into pr-sync's processed/ (create it on demand). Only now that the reply
    // is delivered.
    let processed = fleet.inbox("pr-sync").join("processed");
    std::fs::create_dir_all(&processed).ok();
    let dest = processed.join(
        path.file_name()
            .unwrap_or(std::ffi::OsStr::new("request.json")),
    );
    match std::fs::rename(&path, &dest) {
        Ok(()) => println!("  ✓ archived {} → processed/", path.display()),
        Err(e) => eprintln!(
            "  ! reply sent, but could not archive {} ({e}) — move it by hand",
            path.display()
        ),
    }

    // 3) Wake the sender so it acts on the reply immediately (a `reject` needs a fast fix; a `merged`
    // lets a one-shot agent stand down). Same guards as any delivery.
    match wake_window(fleet, &mr.from) {
        WakeOutcome::Woke => println!("  ↑ nudged '{}' awake (immediate tick)", mr.from),
        WakeOutcome::Skipped(why) => println!("  (sender not woken: {why})"),
    }
}

/// Audit the integration record for silent drops: every merge-request pr-sync archived into
/// `processed/` should have produced exactly one reply carrying its filename in `in_reply_to`. See the
/// `Audit` doc comment. Exits non-zero if any ORPHAN (archived, no reply) is found.
fn audit(fleet: &Fleet, verbose: bool, strict: bool) {
    // The set of request filenames some reply claims to answer (its `in_reply_to`), gathered across
    // EVERY agent's inbox + processed/ (a reply could be sitting unread in the sender's inbox, or
    // already archived there).
    let mut answered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let inbox_root = fleet.root.join("inbox");
    if let Ok(rd) = std::fs::read_dir(&inbox_root) {
        for agent_dir in rd.filter_map(Result::ok).map(|e| e.path()) {
            if !agent_dir.is_dir() {
                continue;
            }
            for sub in [agent_dir.clone(), agent_dir.join("processed")] {
                for msg in read_messages(&sub) {
                    if !msg.in_reply_to.is_empty() {
                        answered.insert(msg.in_reply_to);
                    }
                }
            }
        }
    }

    // Precompute the set of ACTIVE agent names ONCE (registry.json is parsed a single time here, not
    // per-request — the processed/ history can be hundreds of files). An orphan only matters if its
    // sender is still active and thus stuck waiting.
    let active: std::collections::HashSet<String> = fleet
        .load()
        .agents
        .into_iter()
        .filter(|a| a.status == "active")
        .map(|a| a.name)
        .collect();

    // Every merge-request archived in pr-sync/processed is a resolution we expect a reply for.
    let processed = fleet.inbox("pr-sync").join("processed");
    let mut orphans: Vec<(String, String)> = Vec::new(); // (filename, sender)
    let mut unverifiable = 0usize;
    let mut verified = 0usize;
    let mut total = 0usize;
    let Ok(rd) = std::fs::read_dir(&processed) else {
        println!("fleet audit: no pr-sync processed/ dir yet — nothing to audit.");
        return;
    };
    for entry in rd.filter_map(Result::ok) {
        let p = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.ends_with("merge-request.json") {
            continue;
        }
        total += 1;
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let mr: Message = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if answered.contains(&fname) {
            verified += 1;
            if verbose {
                println!("  ✓ {fname} (from {}) — reply found", mr.from);
            }
        } else {
            // No reply names this request. It's either a genuine orphan (silent drop) or was resolved
            // before `in_reply_to` existed / via a hand-`send`. We can't prove a reply for the latter,
            // so classify conservatively: flag as ORPHAN only when the sender is a STILL-ACTIVE agent
            // that would be stuck waiting (a stale one-shot fix agent's pre-audit request is noise) AND
            // the request isn't a PRE-DURABLE-SEQ (`000000000001-…`) send. A long-lived vertical is
            // always active, so its ancient pre-`ack` MRs (which can NEVER carry an `in_reply_to`
            // correlation) would otherwise be flagged as "stuck waiting" every audit forever — 38 such
            // permanent false orphans here, enough to drown a REAL recent drop. A recent drop carries a
            // high monotonic delivery-seq, so excluding the `…0001` sentinel can't hide one.
            if active.contains(&mr.from) && !is_pre_durable_seq_filename(&fname) {
                orphans.push((fname.clone(), mr.from.clone()));
            } else {
                unverifiable += 1;
                if verbose {
                    let why = if is_pre_durable_seq_filename(&fname) {
                        "pre-durable-seq (predates in_reply_to; uncorrelatable)"
                    } else {
                        "sender not active (pre-audit / stale)"
                    };
                    println!(
                        "  ? {fname} (from {}) — no reply recorded; {why} — unverifiable",
                        mr.from
                    );
                }
            }
        }
    }

    println!(
        "fleet audit: {total} archived merge-request(s) — {verified} verified-replied, \
         {} orphan(s), {unverifiable} unverifiable (pre-audit / not via `fleet ack`).",
        orphans.len()
    );
    if !orphans.is_empty() {
        eprintln!(
            "\n  ⚠ SILENT DROP(S) — an active agent's merge-request was archived with NO reply:"
        );
        for (fname, from) in &orphans {
            eprintln!("    • {fname}  (sender '{from}' is active and waiting)");
        }
        eprintln!(
            "  These senders may be stuck waiting on a merged/reject that never came. pr-sync should \
             re-process them via `cargo xtask fleet ack`, or the senders should resend. (Some may have \
             actually landed pre-`ack` without a reply-stamp — verify before assuming loss.)\n"
        );
        if strict {
            std::process::exit(1);
        }
    }

    // Second check — the MIRROR of a silent drop: a merge-request still QUEUED in pr-sync's LIVE inbox
    // (not yet processed) whose `--ref` content is ALREADY on trunk by patch-id. This happens when
    // pr-sync integrates a commit under a re-parented/squashed sha but never acks the original file, so
    // the no-op MR lingers and would gate to an empty merge. Surface them so pr-sync can batch-reject
    // (`fleet ack <file> --outcome reject --body "already landed by patch-id; superseded"`) rather than
    // re-gate each. Only checks merge-requests from still-active senders (a stale one-shot's leftover is
    // noise). Read-only; skipped silently if git can't resolve `trunk`.
    let landed_queued = find_queued_but_landed(fleet, &active);
    if !landed_queued.is_empty() {
        eprintln!(
            "\n  ⚠ QUEUED-BUT-ALREADY-LANDED — a merge-request still in pr-sync's inbox whose --ref is \
             already on trunk by patch-id (would gate to an empty merge):"
        );
        for (fname, from, ref_sha) in &landed_queued {
            eprintln!(
                "    • {fname}  (from '{from}', ref {ref_sha} landed) — reject as superseded"
            );
        }
        eprintln!(
            "  pr-sync: clear each with `cargo xtask fleet ack <file> --outcome reject --body \
             \"already landed by patch-id; superseded\"` — no gate needed.\n"
        );
        if strict {
            std::process::exit(1);
        }
    }
}

/// Scan pr-sync's LIVE inbox for merge-requests whose `--ref` content is ALREADY on trunk by patch-id
/// (pr-sync integrated it under a re-parented/squashed sha but never acked the original file, so the
/// no-op MR lingers and would gate to an empty merge). Returns `(filename, sender, ref)` for each,
/// considering only merge-requests from still-`active` senders (a stale one-shot's leftover is noise).
/// Shared by `audit` and the `watchdog` health sweep so both surface the same set. Read-only; a git
/// failure on any single ref just skips it. Kept as a fn (not pure) because it does I/O + git, but the
/// per-ref decisions are the pure `mr_qualifies_for_landed_check` (worth the git call?) and
/// `cherry_says_landed` (is it actually landed?).
///
/// The pure eligibility gate for a queued merge-request: is it worth spending a `git cherry` on? YES iff
/// it carries a non-empty `--ref` (an empty ref has nothing to check against trunk) AND its sender is
/// still `active` (a stale one-shot agent's leftover MR is noise, not a live orphan pr-sync should
/// reject). Split out of `find_queued_but_landed`'s fs+git loop so both skip conditions are unit-pinned:
/// dropping either would either waste a git call on a ref-less message or resurrect dead-agent noise.
fn mr_qualifies_for_landed_check(
    r#ref: &str,
    from: &str,
    active: &std::collections::HashSet<String>,
) -> bool {
    !r#ref.is_empty() && active.contains(from)
}

fn find_queued_but_landed(
    fleet: &Fleet,
    active: &std::collections::HashSet<String>,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(fleet.inbox("pr-sync")) else {
        return out;
    };
    for entry in rd.filter_map(Result::ok) {
        let p = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if !p.is_file() || !fname.ends_with("merge-request.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(mr) = serde_json::from_str::<Message>(&text) else {
            continue;
        };
        if !mr_qualifies_for_landed_check(&mr.r#ref, &mr.from, active) {
            continue;
        }
        let cherry = Command::new("git")
            .current_dir(&fleet.repo)
            .args(["cherry", TRUNK, &mr.r#ref])
            .output();
        if let Ok(o) = cherry
            && o.status.success()
            && cherry_says_landed(&String::from_utf8_lossy(&o.stdout))
        {
            out.push((fname, mr.from.clone(), mr.r#ref.clone()));
        }
    }
    out
}

/// Read every `*.json` message in a directory (non-recursive), skipping unparseable files. Used by
/// `audit` to scan inboxes + processed dirs.
fn read_messages(dir: &Path) -> Vec<Message> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|t| serde_json::from_str::<Message>(&t).ok())
        .collect()
}

/// Reconcile the `inbox/unknown/` graveyard — re-route each dead-lettered reply to the recipient named
/// in its subject (`<kind>: fleet/<agent>`). See the `RerouteUnknown` doc. Each moved message is
/// re-delivered to `<agent>` as a `note` (so it reads as a reconciliation, not a fresh live reply the
/// sender might double-act-on), preserving the original kind + ref + body in the note text. The source
/// file is removed on a successful re-route (moved into the target inbox), so re-running is idempotent.
fn reroute_unknown(fleet: &Fleet, dry_run: bool) {
    let graveyard = fleet.inbox("unknown");
    let Ok(rd) = std::fs::read_dir(&graveyard) else {
        println!("fleet reroute-unknown: no inbox/unknown/ dir — nothing to reconcile.");
        return;
    };
    let mut routed = 0usize;
    let mut skipped = 0usize;
    let mut total = 0usize;
    for entry in rd.filter_map(Result::ok) {
        let p = entry.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        total += 1;
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(msg) = serde_json::from_str::<Message>(&text) else {
            skipped += 1;
            continue;
        };
        let Some(agent) = recipient_from_subject(&msg.subject) else {
            skipped += 1;
            if dry_run {
                println!("  ? skip (no fleet/<agent> in subject): {:?}", msg.subject);
            }
            continue;
        };
        if dry_run {
            println!(
                "  → would route to '{agent}': [{}] {:?}",
                msg.kind, msg.subject
            );
            routed += 1;
            continue;
        }
        // Re-deliver as a NOTE to the derived recipient, folding the original reply into the body so
        // the agent sees what happened without it looking like a fresh merged/reject to act on twice.
        deliver(
            fleet,
            &Message {
                from: "fleet-reroute".to_string(),
                to: agent.clone(),
                kind: "note".to_string(),
                subject: format!("[reconciled from unknown/] {}", msg.subject),
                r#ref: msg.r#ref.clone(),
                body: format!(
                    "A reply pr-sync addressed to `unknown` (the send-side identity bug) was re-routed \
                     to you from inbox/unknown/. Original kind: `{}`. ref: `{}`.\n\n{}",
                    msg.kind, msg.r#ref, msg.body
                ),
                seq: next_seq(),
                in_reply_to: msg.in_reply_to.clone(),
            },
        );
        // Remove the graveyard copy now that it's re-delivered (idempotent re-runs).
        std::fs::remove_file(&p).ok();
        routed += 1;
    }
    println!(
        "fleet reroute-unknown: {}{routed} re-routed, {skipped} un-derivable, {total} total in unknown/.",
        if dry_run { "DRY-RUN: " } else { "" }
    );
}

/// git merge driver for `.duvet/coverage-floor.json` (see the `MergeFloor` doc). Resolve a floor
/// conflict by writing the field-wise MAX of `ours` and `theirs` back to `ours`. The floor is monotone
/// (it only moves up as coverage grows), so max is always the correct merged value — two agents each
/// bumping `cited` merge to the higher, with zero conflict. Exits 0 on success (git then treats the
/// merge as resolved); non-zero tells git the driver failed (falls back to a conflict, the old behavior
/// — safe). A `_note`/other fields on either side are preserved from `ours`.
/// The PURE core of the floor merge: given both parsed sides, return OUR object with `cited`/`total`
/// replaced by the field-wise MAX. Missing/garbage COUNTERS *inside* an object read as 0 so the other
/// side wins; all other keys on `ours` (e.g. `_note`) are preserved.
///
/// Returns `None` if EITHER side is valid JSON but NOT an object (`null`, a number, an array). The
/// driver contract is strictly `{"cited": u64, "total": u64, …}`; a non-object is a corrupt/wrong floor
/// file, and rewriting it (or silently returning it unchanged) would "resolve" the conflict with an
/// invalid floor (PR #430). `None` tells `merge_floor` to leave the conflict for a human instead. Kept
/// separate from `merge_floor`'s file I/O + `process::exit` so the semantics are unit-testable.
fn merged_floor_value(
    mut ours: serde_json::Value,
    theirs: &serde_json::Value,
) -> Option<serde_json::Value> {
    // Both sides must be JSON objects — reject a valid-JSON-non-object rather than rewrite it.
    if !ours.is_object() || !theirs.is_object() {
        return None;
    }
    let field = |v: &serde_json::Value, k: &str| v.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
    let merged_cited = field(&ours, "cited").max(field(theirs, "cited"));
    let merged_total = field(&ours, "total").max(field(theirs, "total"));
    // Overwrite the two counters on OUR object (keeps ours' `_note` etc.). The is_object() guard above
    // guarantees as_object_mut() succeeds.
    if let Some(map) = ours.as_object_mut() {
        map.insert("cited".to_string(), serde_json::json!(merged_cited));
        map.insert("total".to_string(), serde_json::json!(merged_total));
    }
    Some(ours)
}

fn merge_floor(ours: &Path, theirs: &Path) {
    let load = |p: &Path| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
    };
    let (Some(o), Some(t)) = (load(ours), load(theirs)) else {
        eprintln!(
            "fleet merge-floor: could not parse both floor files — leaving the conflict for a human"
        );
        std::process::exit(1);
    };
    let Some(o) = merged_floor_value(o, &t) else {
        eprintln!(
            "fleet merge-floor: a floor file is valid JSON but not a {{cited,total}} object — leaving the conflict for a human"
        );
        std::process::exit(1);
    };
    let merged_cited = o.get("cited").and_then(|n| n.as_u64()).unwrap_or(0);
    let merged_total = o.get("total").and_then(|n| n.as_u64()).unwrap_or(0);
    match serde_json::to_string_pretty(&o) {
        Ok(s) => {
            if std::fs::write(ours, format!("{s}\n")).is_err() {
                eprintln!(
                    "fleet merge-floor: could not write merged floor to {}",
                    ours.display()
                );
                std::process::exit(1);
            }
            eprintln!(
                "fleet merge-floor: resolved coverage-floor conflict → cited={merged_cited}, total={merged_total} (field-wise max)"
            );
        }
        Err(_) => std::process::exit(1),
    }
}

/// Why a delivery did or didn't wake the recipient.
enum WakeOutcome {
    /// The recipient's window was nudged into an immediate tick.
    Woke,
    /// Not nudged; the reason (recipient will still drain the inbox on its next scheduled tick).
    Skipped(&'static str),
}

/// Nudge a message recipient's tmux window into an immediate tick, subject to the wake guards:
///   * not in a tmux session → nothing to nudge;
///   * a stopped recipient (stop-file present) MUST stay down;
///   * the interactive `concierge`/`design` windows are left alone — a human may be typing, and a
///     nudge would clobber their input (they poll their inbox on their own cadence);
///   * no live tmux window by that name → nothing to nudge (a not-yet-launched or reaped agent);
///   * a window already mid-tick ("esc to interrupt") does NOT need a nudge — it will drain the
///     freshly-delivered message when the current tick finishes, and a keystroke mid-turn is noise.
///
/// A recipient absent from the registry is still nudged if it has a live window (matches `send`'s
/// "deliver even if not yet registered" behavior); only an explicit `stopped` status suppresses it.
fn wake_window(fleet: &Fleet, to: &str) -> WakeOutcome {
    if !in_tmux() {
        return WakeOutcome::Skipped("not in a tmux session");
    }
    if fleet.stopfile(to).exists() {
        return WakeOutcome::Skipped("recipient is stopped");
    }
    // Interactive roles talk to the human directly; never inject keystrokes into their window.
    let reg = fleet.load();
    if let Some(a) = reg.agents.iter().find(|a| a.name == to) {
        if a.status == "stopped" {
            return WakeOutcome::Skipped("recipient is stopped");
        }
        if matches!(a.role.as_str(), "concierge" | "design") {
            return WakeOutcome::Skipped("interactive role — left for the human");
        }
    }
    let session = tmux_current_session();
    if !tmux_windows(&session).iter().any(|w| w == to) {
        return WakeOutcome::Skipped("no live window");
    }
    if window_is_working(&session, to) {
        return WakeOutcome::Skipped("already mid-tick");
    }
    // Nudge an idle loop to run its tick now. We type the word `continue` + Enter rather than
    // re-invoking `/loop` (which would schedule a DUPLICATE recurring cron each time): the idle
    // agent's context still holds its role, so a bare "continue" prompt makes it run one tick, and
    // the existing `/loop` schedule keeps driving the cadence.
    if nudge_tick(&session, to) {
        WakeOutcome::Woke
    } else {
        WakeOutcome::Skipped("send-keys failed")
    }
}

/// Type a one-shot tick prompt into an idle agent's pane (`continue` + Enter). Unlike
/// [`rearm_window`], this does NOT re-invoke `/loop`, so it never stacks duplicate cron schedules —
/// it just makes an already-scheduled, currently-idle agent run its next tick immediately.
fn nudge_tick(session: &str, agent: &str) -> bool {
    let target = format!("{session}:{agent}");
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &target, "-l", "continue"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sent {
        return false;
    }
    Command::new("tmux")
        .args(["send-keys", "-t", &target, "Enter"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Auto-nudge a drain-stalled agent's idle pane with the canonical drain instruction (opt-in, from the
/// watchdog under `--nudge-drain-stalls`). Sends a literal message telling it to drain its hub inbox via
/// `cargo xtask fleet inbox <agent>` (the resolver — never a relative glob), then TWO Enters: the first
/// submits the line, the second clears any paste-buffering that can otherwise leave the text unsent (a
/// quirk the concierge had to work around by hand). The message is what makes a re-armed agent re-read
/// its charter (step 1) + adopt the canonical resolver, self-healing the stall.
fn nudge_drain_stall(session: &str, agent: &str) -> bool {
    let target = format!("{session}:{agent}");
    let msg = format!(
        "cargo xtask fleet inbox {agent} — drain your hub inbox via this resolver (NOT a relative \
         .claude/fleet/inbox glob, which silently matches nothing); process each message, then continue."
    );
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &target, "-l", &msg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sent {
        return false;
    }
    // Two Enters: submit + clear paste-buffering (the by-hand workaround, now automated).
    for _ in 0..2 {
        let ok = Command::new("tmux")
            .args(["send-keys", "-t", &target, "Enter"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return false;
        }
    }
    true
}

fn heartbeat(fleet: &Fleet, name: &str) {
    // Presence is a touch-file per agent under `.claude/fleet/heartbeat/<name>` — cheap, and avoids
    // rewriting the whole registry on every tick (which would contend with `add`/`remove`).
    let dir = fleet.root.join("heartbeat");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), "tick\n").ok();
    // If a stop-file exists, tell the caller to stand down (exit code 2 = "you are stopped").
    if fleet.stopfile(name).exists() {
        println!("STOPPED");
        std::process::exit(2);
    }
    println!("ok");
}

fn describe(fleet: &Fleet, name: &str) {
    let reg = fleet.load();
    let Some(a) = reg.agents.iter().find(|a| a.name == name) else {
        eprintln!("fleet describe: no agent named '{name}'");
        std::process::exit(1);
    };
    // Shell-safe KEY=VALUE lines for window.sh to `eval`. Values are our own controlled strings
    // (paths, a role name, a model id) — single-quote them defensively anyway. The stored model is a
    // short alias (`opus`/`fable`); expand it to the full 1M-context id here, the point where
    // window.sh consumes it and hands it to `claude --model`.
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    println!("WORKTREE={}", q(&a.worktree));
    println!("ROLE={}", q(&a.role));
    println!("MODEL={}", q(&resolve_model(&a.model)));
    println!("EFFORT={}", q(&a.effort));
    println!("INTERVAL={}", q(&a.interval));
    println!("VERTICAL={}", q(&a.vertical));
    println!("AREA={}", q(&a.area));
    println!("DISALLOW_ASK={}", if a.disallow_ask { 1 } else { 0 });
}

// ── watchdog ───────────────────────────────────────────────────────────────────────────────────

/// Self-heal a stalled fleet: re-arm any active agent whose `/loop` heartbeat has gone stale. See the
/// `Watchdog` doc comment for the full contract. One pass, then return; a cron drives the cadence.
fn watchdog(
    fleet: &Fleet,
    dry_run: bool,
    stale_mult: u32,
    stale_cap: u64,
    grace_secs: u64,
    nudge_drain_stalls: bool,
    drain_nudge_grace: u64,
) {
    if !in_tmux() {
        eprintln!(
            "fleet watchdog: not inside a tmux session (no $TMUX) — nothing to re-arm. Run it from\n\
             the tmux session the fleet windows live in."
        );
        return;
    }
    let session = tmux_current_session();
    let live = tmux_windows(&session);
    let reg = fleet.load();
    let now = now_unix();

    let mut rearmed = 0usize;
    // Of the re-arms, how many ESCALATED to re-issuing `/loop` (a prior nudge didn't stick → the agent's
    // recurring cron is presumed DEAD, not merely a missed tick). This split is the loop-liveness signal:
    // `rearmed` alone can't distinguish a healthy net catching an occasional missed tick from `/loop`
    // crons systematically dying. A persistently non-zero `reissued` across sweeps is the tell that the
    // self-reschedule is failing (the root the watchdog only papers over) — worth an operator/tooling fix.
    let mut reissued = 0usize;
    let mut checked = 0usize;
    let mut drain_stalls = 0usize;
    let mut saturated = 0usize;
    let mut wedge_escalations = 0usize;
    for a in &reg.agents {
        // Only ACTIVE agents with no stop-file are candidates — a stopped/removed agent SHOULD be idle.
        if a.status != "active" || fleet.stopfile(&a.name).exists() {
            continue;
        }
        // No live window → `up` hasn't launched it (or it was closed); the watchdog doesn't create
        // windows, only revives loops in existing ones.
        if !live.iter().any(|w| w == &a.name) {
            continue;
        }
        checked += 1;

        // Liveness = the heartbeat touch-file's mtime (stamped at the top of every tick).
        let hb_age = heartbeat_age_secs(fleet, &a.name, now);
        let interval = parse_interval_secs(&a.interval);
        let stale_after = stale_window_secs(interval, stale_mult, stale_cap);
        let age = match hb_age {
            Some(age) => age,
            None => {
                // NEVER stamped a heartbeat: either still booting its first tick, OR its cold start
                // FAILED (launched, never completed tick 1, so never stamped — the fresh-fix-agent stall
                // the concierge flagged: heartbeat never advances, seed msg never drained, and a plain
                // `continue` nudge alone didn't revive it). We can't tell "still booting" from "cold
                // start died" without a clock, so mark first-sight (`firstseen/<agent>`) and give a
                // GENEROUS cold-start window (2× the stale window — a first tick may build). If the
                // marker is older than that and there's STILL no heartbeat, the cold start failed →
                // fall through and re-arm it (below), rather than skipping it forever.
                let coldstart_window = stale_after.saturating_mul(2);
                match coldstart_verdict(firstseen_age_secs(fleet, &a.name, now), coldstart_window) {
                    ColdStartVerdict::MarkAndWait => {
                        stamp_firstseen(fleet, &a.name);
                        continue; // just noticed it booting — give it the cold-start window.
                    }
                    ColdStartVerdict::StillWaiting => continue, // still within cold-start grace.
                    ColdStartVerdict::Failed(seen) => {
                        // Cold start failed: live window, no heartbeat, past the window. Treat as stale.
                        println!(
                            "  ! {} never stamped a heartbeat in {seen}s (> {coldstart_window}s cold-start window) — FAILED cold start, re-arming",
                            a.name
                        );
                        seen // use the first-seen age as the "staleness" for the logic below.
                    }
                }
            }
        };
        // DRAIN-STALL detection (report-only). A silent drain-stall is INVISIBLE to the heartbeat
        // check below: the loop IS running (so the heartbeat stays fresh and `age <= stale_after`), it
        // just isn't consuming its inbox — e.g. it globs a worktree-relative `.claude/...` that matches
        // nothing, sees "empty" every tick, and idles at prompt while an `assign`/`reject` piles up
        // unread (this stalled v-try-operator ~8 ticks). So BEFORE the healthy-`continue`, cross-check
        // the hub inbox depth against the pane: an agent sitting IDLE (not mid-tick) WITH unconsumed hub
        // messages is a probable drain-stall. Report it loudly (don't auto-nudge — the loop runs; a
        // human/operator wants to see the anomaly, and a spurious nudge mid-anything is riskier than a
        // warning). Interactive roles (concierge/design) legitimately sit idle with mail a human reads,
        // so exempt them. Uses the canonical hub inbox (fleet.inbox), never a relative path.
        // Capture the pane ONCE for the two report-only pane signals below (drain-stall + saturation).
        let pane = capture_pane(&session, &a.name);
        let pane_working = pane.as_deref().is_some_and(pane_shows_working);

        let ctx_pct = pane.as_deref().and_then(parse_context_pct);
        // Only ACTIONABLE queued mail counts toward a drain-stall — stale informational notes/merged
        // acks an agent left un-archived are untidy, not stuck work (see `message_kind_is_actionable`).
        let actionable_depth = actionable_inbox_depth(&fleet.inbox(&a.name));
        // Read the PRIOR sweep's actionable depth BEFORE recording this one, for the draining-delta
        // signal below (a strict drop proves the loop is consuming its own mail — see `queue_is_draining`).
        let prev_depth = last_actionable_depth(fleet, &a.name);
        record_actionable_depth(fleet, &a.name, actionable_depth);
        let pane_idle = hb_age.is_some() && !pane_working;
        // The instantaneous snapshot only SUSPECTS a stall — a busy agent shows a bare prompt for a
        // sub-second gap between tool-turns, so one capture can't distinguish "idle at prompt" from
        // "between two turns". CONFIRM with a second capture after a short delay: if the pane shows work
        // in flight on the recheck, it was mid-tick, not stalled — clear the flag (no false alarm, no
        // spurious nudge). Only the rare already-suspected agent pays the delay, never every idle pane.
        let suspected_stall = is_probable_drain_stall(&a.role, actionable_depth, pane_idle);
        // Confirming recapture (read-only, safe in dry-run too): a single snapshot can catch a busy
        // agent in the sub-second gap between tool-turns, so only SUSPECT from the first capture and
        // CONFIRM with a second after a short delay. If work is in flight on the recheck it was mid-tick,
        // not stalled — clear the flag (no false alarm, no spurious nudge).
        let working_on_recheck = if suspected_stall {
            std::thread::sleep(std::time::Duration::from_secs(
                DRAIN_STALL_CONFIRM_DELAY_SECS,
            ));
            window_is_working(&session, &a.name)
        } else {
            false
        };
        // Second exoneration (besides "work on the recheck"): the actionable queue is strictly SMALLER
        // than last sweep. A single-writer like pr-sync bakes a multi-MR gate BATCH in a DETACHED
        // subprocess, so its pane sits at a bare `❯` on BOTH captures (the recapture only catches a
        // sub-second between-turns gap, not a minutes-long detached bake) AND trunk stays flat until the
        // whole batch lands — both drain-stall-shaped, yet it's healthily draining. A depth that DROPPED
        // since last sweep is positive proof the loop is consuming its own mail (only self-consumption
        // lowers it; new mail raises it). First sweep (no prior) can't drop → not exonerated (correct: a
        // genuinely wedged agent must not get a free pass on the sweep we first see it).
        let queue_draining = queue_is_draining(prev_depth, actionable_depth);
        let drain_stall =
            drain_stall_confirmed(suspected_stall, working_on_recheck || queue_draining);
        if drain_stall {
            drain_stalls += 1;
            eprintln!(
                "  ⚠ '{}' is IDLE at prompt with {actionable_depth} UNCONSUMED ACTIONABLE message(s) in \
                 its hub inbox ({}) — probable DRAIN-STALL (loop alive, heartbeat fresh, but not \
                 draining; e.g. a worktree-relative inbox glob). Check it drains via \
                 `cargo xtask fleet inbox {}`.",
                a.name,
                fleet.inbox(&a.name).display(),
                a.name
            );
            // OPT-IN auto-nudge (`--nudge-drain-stalls`): send the canonical drain instruction to the
            // idle pane so it re-reads its charter + adopts the resolver, self-healing — instead of the
            // concierge hand-arming each one. Hard-guarded (see `decide_drain_nudge`): skips a
            // context-saturated pane (needs a restart, not a nudge) and rate-limits per agent — with a
            // SHORT re-nudge window when the SAME message persists (our prior nudge didn't stick).
            let flagged_id = oldest_actionable_inbox_message(fleet, &a.name);
            let last = last_drain_nudge(fleet, &a.name, now);
            let action = decide_drain_nudge(
                nudge_drain_stalls,
                drain_stall,
                ctx_pct,
                CTX_SATURATION_THRESHOLD,
                flagged_id.as_deref(),
                last.as_ref().map(|(id, age)| (id.as_str(), *age)),
                drain_nudge_grace,
                DRAIN_NUDGE_STUCK_GRACE,
            );
            if action == DrainNudge::Stuck {
                // A prior auto-nudge didn't stick (same message still unconsumed). Surface it loudly so
                // the concierge can hand-arm immediately instead of waiting out the full grace.
                eprintln!(
                    "  ⚠ '{}' STILL has message '{}' unconsumed after an auto-nudge — the nudge didn't \
                     STICK. Re-nudging; if it persists again, hand-arm the '{}' pane.",
                    a.name,
                    flagged_id.as_deref().unwrap_or("?"),
                    a.name
                );
            }
            if matches!(action, DrainNudge::Fresh | DrainNudge::Stuck) {
                if dry_run {
                    println!("  DRY-RUN would auto-nudge '{}' to drain its inbox", a.name);
                } else if nudge_drain_stall(&session, &a.name) {
                    stamp_drain_nudge(fleet, &a.name, flagged_id.as_deref().unwrap_or(""));
                    println!(
                        "  + auto-nudged '{}' to drain its inbox (--nudge-drain-stalls){}",
                        a.name,
                        if action == DrainNudge::Stuck {
                            " [re-nudge: prior didn't stick]"
                        } else {
                            ""
                        }
                    );
                } else {
                    eprintln!("  ! failed to send drain-nudge keys to '{}'", a.name);
                }
            }
            // fall through — still run the normal heartbeat logic below (a drain-stall usually has a
            // FRESH heartbeat and would `continue` as healthy, so the warning above is the signal).
        }

        // CONTEXT-SATURATION detection (report-only). A saturated agent is invisible to EVERY other
        // check: its loop is alive (heartbeat fresh → sails past the staleness `continue`) and its pane
        // is NOT idle-at-prompt (it's mid-turn / queuing → the drain-stall signal doesn't fire), yet at
        // 100% context it's UNRECOVERABLE — `/compact` can't even submit. Scrape the pane's "N% context"
        // marker and warn at/above the threshold (default 85, below the 100% wall) so the concierge can
        // `/compact` proactively while it still submits, or route an operator restart. Report-only, like
        // the other watchdog signals.
        if context_saturation_warning(ctx_pct, CTX_SATURATION_THRESHOLD) {
            saturated += 1;
            let pct = ctx_pct.unwrap_or(0);
            let tail = if pct >= CTX_WEDGE_THRESHOLD {
                "UNRECOVERABLE (/compact can't submit); needs an operator restart"
            } else {
                "approaching the 100% wall — proactive `/compact` NOW while it still submits"
            };
            eprintln!("  ⚠ '{}' is at {pct}% context — {tail}.", a.name);

            // Escalate to the concierge at the PRE-WALL threshold (95% general / 92% pr-sync) — BEFORE the
            // 100% wall, while a `/compact` can still submit — so the concierge can nudge a proactive
            // compact (or route a restart) rather than being told only after the agent is already
            // unrecoverable (the old 100%-only escalation, which was too late). The stderr warning above
            // fires from 85% but is unread live; the concierge `note` is the actionable signal. Rate-
            // limited so a climbing agent doesn't regenerate a note every sweep. NEVER keystroke the
            // victim — the concierge/operator decides nudge-vs-restart.
            let notified_recently =
                sat_notify_age_secs(fleet, &a.name, now).is_some_and(|s| s < SAT_NOTIFY_GRACE);
            // A CONCIERGE at the wall can't be helped by a note (it lands in the inbox it can't drain) —
            // only the operator can restart it. Emit a loud, distinct stderr line for it. (The escalation
            // below is skipped for the concierge by the guard in `should_notify_saturation`.)
            if a.name == "concierge" && pct >= CTX_WEDGE_THRESHOLD {
                eprintln!(
                    "  ‼ CONCIERGE itself is at {pct}% — the human interface is WEDGED and cannot be \
                     auto-notified (a note lands in an inbox it can't drain). OPERATOR must restart the \
                     concierge window."
                );
            }
            if should_notify_saturation(&a.name, ctx_pct, notified_recently) {
                // Distinguish PRE-WALL (still compactable — nudge) from AT-WALL (unrecoverable — restart),
                // and flag pr-sync loudly (its wedge stalls the whole merge queue).
                let at_wall = pct >= CTX_WEDGE_THRESHOLD;
                let prsync_tag = if a.name == "pr-sync" {
                    " [SINGLE-WRITER: a wedge stalls the ENTIRE merge queue]"
                } else {
                    ""
                };
                if dry_run {
                    println!(
                        "  DRY-RUN would escalate '{}' context {} ({pct}%) to concierge",
                        a.name,
                        if at_wall { "WEDGE" } else { "pre-wall" }
                    );
                } else {
                    let (subject, action) = if at_wall {
                        (
                            format!(
                                "context-WEDGE: '{}' at {pct}% — /compact can't submit; needs an operator restart{prsync_tag}",
                                a.name
                            ),
                            format!(
                                "'{}' is at {pct}% — the UNRECOVERABLE wall. It cannot self-`/compact` (no \
                                 headroom) and a watchdog nudge can't preempt a full window. Needs an \
                                 OPERATOR action: interrupt+restart the '{}' window, or Esc its pane to \
                                 clear a queued-but-unsubmittable `/compact`.",
                                a.name, a.name
                            ),
                        )
                    } else {
                        (
                            format!(
                                "context PRE-WALL: '{}' at {pct}% — nudge a /compact NOW while it still submits{prsync_tag}",
                                a.name
                            ),
                            format!(
                                "'{}' is at {pct}% context and has NOT self-`/compact`ed — approaching the \
                                 100% wall but STILL COMPACTABLE. Nudge it to `/compact` (it fires at the \
                                 next turn boundary, pre-empting the wall) before it becomes unrecoverable. \
                                 Escalated pre-wall (not at 100%) precisely so a `/compact` can still land.",
                                a.name
                            ),
                        )
                    };
                    deliver(
                        fleet,
                        &Message {
                            from: "watchdog".to_string(),
                            to: "concierge".to_string(),
                            kind: "note".to_string(),
                            subject,
                            body: format!(
                                "{action} Auto-surfaced so it doesn't sit unseen in watchdog stderr; \
                                 rate-limited to one note per ~30min per agent until it clears."
                            ),
                            seq: next_seq(),
                            r#ref: String::new(),
                            in_reply_to: String::new(),
                        },
                    );
                    stamp_sat_notify(fleet, &a.name);
                    wedge_escalations += 1;
                    println!(
                        "  + escalated '{}' context {} ({pct}%) to concierge (rate-limited)",
                        a.name,
                        if at_wall { "WEDGE" } else { "pre-wall" }
                    );
                }
            }
        }

        if hb_age.is_some() && age <= stale_after {
            continue; // ticked recently — healthy.
        }

        // pr-sync liveness via TRUNK ADVANCE: pr-sync does minutes-long SYNCHRONOUS work per tick (a
        // full gate cycle — `cargo test` + `xtask gate` + `check` — per MR in a batch), so its
        // heartbeat mtime legitimately goes 15-25min stale MID-BATCH while it's alive and integrating.
        // A stale heartbeat alone falsely reads as "stalled" for the one agent that advances `trunk`.
        // So for pr-sync, also consult trunk: if `trunk` has a commit newer than the stale window, it's
        // demonstrably alive (only pr-sync writes trunk) — don't re-arm. (Any agent on the `trunk`
        // branch is pr-sync; keyed by name to be explicit.)
        if a.name == "pr-sync"
            && let Some(commit_age) = last_commit_age_secs(&fleet.repo, TRUNK)
            && commit_age <= stale_after
        {
            println!(
                "  = pr-sync heartbeat stale ({age}s) but trunk advanced {commit_age}s ago — alive mid-batch, left alone"
            );
            continue;
        }

        // Anti-thrash: don't re-arm an agent we nudged within the grace period — give the nudge time
        // to land and refresh the heartbeat before we judge it stale again.
        if let Some(since) = rearm_age_secs(fleet, &a.name, now)
            && since < grace_secs
        {
            println!(
                "  ~ {} stale ({age}s > {stale_after}s) but re-armed {since}s ago (< {grace_secs}s grace) — waiting",
                a.name
            );
            continue;
        }

        // Don't interrupt a real tick: if the pane shows Claude working ("esc to interrupt"), the loop
        // is alive and mid-work — a stale heartbeat just means a long tick, not a dead loop.
        //
        // BUT this pane-busy guard is only trustworthy for an agent that has EVER stamped a heartbeat.
        // A genuine tick stamps its heartbeat at step 1 (`fleet heartbeat`) BEFORE any long work, so a
        // has-looped agent with a busy pane is genuinely mid-tick. A NEVER-heartbeated agent past the
        // cold-start window, by contrast, is FLAILING (e.g. stuck retrying a failed command) — its busy
        // pane is not progress, and skipping it here is exactly the false-positive that let the
        // origin/trunk-flail mints sit forever (corpus-bugfix's capture). So: honor the busy pane only
        // when the agent has stamped a heartbeat at least once; a never-heartbeated agent re-arms even
        // if the pane looks busy. (`busy pane + zero heartbeats ever` = stuck, not a long tick.)
        // Only trust the pane when the agent has looped before (never-heartbeated → pane can't be
        // trusted). Reuse the single capture taken above (`pane_working`), no second tmux read.
        let pane_busy = hb_age.is_some() && pane_working;
        if pane_busy_means_working(hb_age.is_some(), pane_busy) {
            println!(
                "  = {} heartbeat stale but pane shows work in flight — left alone",
                a.name
            );
            continue;
        }

        if dry_run {
            let action = rearm_action(rearm_age_secs(fleet, &a.name, now), hb_age);
            let how = match action {
                RearmAction::NudgeContinue => "nudge `continue`",
                RearmAction::ReissueLoop => "re-issue `/loop` (prior nudge didn't stick)",
            };
            println!(
                "  DRY-RUN would re-arm '{}' via {how} (idle {age}s > {stale_after}s stale window; interval {})",
                a.name, a.interval
            );
            rearmed += 1;
            if matches!(action, RearmAction::ReissueLoop) {
                reissued += 1;
            }
            continue;
        }
        // Choose the re-arm action (see `rearm_action`): cheap `continue` for a loop that merely missed
        // a tick; escalate to re-issuing `/loop` only when a prior nudge didn't stick — i.e. we re-armed
        // this agent and it has NOT heartbeated since (the no-cron fresh-mint signature). Passing
        // `hb_age` (not just "ever re-armed") avoids mis-escalating a healthy agent that was nudged once
        // long ago, recovered, and is now merely slow — its rearm marker is never cleared.
        let action = rearm_action(rearm_age_secs(fleet, &a.name, now), hb_age);
        let sent = match action {
            RearmAction::NudgeContinue => rearm_window(&session, &a.name, &a.interval),
            RearmAction::ReissueLoop => {
                let prompt = watchdog_tick_prompt(fleet, a);
                reissue_loop(&session, &a.name, &a.interval, &prompt)
            }
        };
        if sent {
            stamp_rearm(fleet, &a.name);
            rearmed += 1;
            match action {
                RearmAction::NudgeContinue => println!(
                    "  + re-armed '{}' (idle {age}s > {stale_after}s; nudged `continue` to run a tick)",
                    a.name
                ),
                RearmAction::ReissueLoop => {
                    reissued += 1;
                    println!(
                        "  ++ re-armed '{}' (idle {age}s > {stale_after}s; prior nudge didn't stick → re-issued `/loop {}` to ARM a cron)",
                        a.name, a.interval
                    )
                }
            }
        } else {
            eprintln!("  ! failed to send-keys to '{}'", a.name);
        }
    }

    // ── Reaping pass: close a genuinely-done agent's lingering tmux window ────────────────────────
    // A stopped agent's window otherwise piles up: the PM-verified reaper (`remove --close`) only
    // reaps FIX agents it gets a note from — design/self-removed agents have no reaper, and a busy PM
    // tick misses some. This pass is role-agnostic and automatic: any agent that is BOTH registry
    // status=stopped AND has a stop-file (belt-and-suspenders: its loop has genuinely been told to
    // exit — never close a merely-idle active agent) and STILL has a live window gets its window
    // killed. The registry row is kept (history/archive), only the panel goes away. A short grace off
    // the stop-file mtime keeps a just-stopped agent's final scrollback glanceable for one cycle.
    let mut reaped = 0usize;
    for a in &reg.agents {
        if a.status != "stopped" {
            continue;
        }
        let stopfile = fleet.stopfile(&a.name);
        if !stopfile.exists() {
            continue; // status stopped but no stop-file → not confirmed done; leave it.
        }
        if !live.iter().any(|w| w == &a.name) {
            continue; // window already gone (reaped, or never launched) — nothing to do.
        }
        // Grace: leave a freshly-stopped agent's window open for one cycle so its final scrollback is
        // glanceable. Reuse the stop-file mtime as the stop time; reap once it's older than grace_secs.
        if let Some(stopped_ago) = file_mtime_unix(&stopfile).map(|m| now.saturating_sub(m))
            && stopped_ago < grace_secs
        {
            println!(
                "  ~ {} stopped {stopped_ago}s ago (< {grace_secs}s grace) — leaving window one more cycle",
                a.name
            );
            continue;
        }
        if dry_run {
            println!(
                "  DRY-RUN would reap '{}' (stopped; window still live)",
                a.name
            );
            reaped += 1;
            continue;
        }
        match kill_window(&session, &a.name) {
            KillOutcome::Killed => {
                reaped += 1;
                println!(
                    "  ⌫ reaped '{}' (stopped agent's window killed; registry row kept)",
                    a.name
                );
            }
            KillOutcome::NotFound => {} // raced with another close — fine.
            KillOutcome::TmuxError => eprintln!("  ! tmux error reaping '{}'", a.name),
        }
    }

    // ── Integration-record sweep (report-only) ───────────────────────────────────────────────────
    // Fold `fleet audit`'s queued-but-already-landed check into the periodic health sweep (concierge
    // ruling A: the audit's value is inspecting LIVE hub state, which exists only on the fleet machine
    // where the watchdog runs — a gitignored `.claude/fleet` means it can't run in CI). REPORT-ONLY:
    // pr-sync owns rejecting its own MRs (its role-body step 5 sweeps these each tick), so the watchdog
    // just SURFACES them each cycle for visibility, never rejects (a second mutator would race pr-sync).
    let active: std::collections::HashSet<String> = reg
        .agents
        .iter()
        .filter(|a| a.status == "active")
        .map(|a| a.name.clone())
        .collect();
    let landed_queued = find_queued_but_landed(fleet, &active);
    if !landed_queued.is_empty() {
        eprintln!(
            "  ⚠ {} QUEUED-BUT-ALREADY-LANDED merge-request(s) in pr-sync's inbox (content on trunk by \
             patch-id; would gate to empty merges) — pr-sync should reject as superseded:",
            landed_queued.len()
        );
        for (fname, from, ref_sha) in &landed_queued {
            eprintln!("    • {fname}  (from '{from}', ref {ref_sha})");
        }
    }

    // ── Out-of-band check-lease reap ─────────────────────────────────────────────────────────────
    // A SIGKILL'd `cargo xtask check` runs no `Drop`, so its lease FILE leaks; the acquire-path reaper
    // (`scan_check_leases` inside `acquire_check_lease`) only runs when SOMEONE tries to acquire — but
    // the recurring fleet-stall is exactly when pr-sync's OWN gate is the wedged priority-acquirer
    // (stuck retrying against a dead-PID priority lease), so no fresh acquire ever scans + reaps it and
    // it lingers the full 45-min TTL, blocking every vertical (the concierge hand-reaped ~10 in one
    // session). This periodic out-of-band sweep — run by the concierge's ~4-min watchdog cron, holding
    // NO lease itself — reaps a dead-PID (or TTL-stale) lease of EITHER class regardless of who is
    // acquiring, closing that gap. A dry-run previews the count without removing (report-only).
    let leases_reaped = if dry_run {
        // Count what a real sweep WOULD reap, without removing (mirror the reap predicate, no mutation).
        check_lease_dir(&fleet.repo)
            .map(|dir| {
                std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .filter(|e| {
                        e.path()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| {
                                n.ends_with(".lease")
                                    && (check_lease_holder_alive(n) == Some(false)
                                        || file_mtime_unix(&e.path()).is_some_and(|m| {
                                            now.saturating_sub(m) > CHECK_LEASE_TTL_SECS
                                        }))
                            })
                    })
                    .count()
            })
            .unwrap_or(0)
    } else {
        reap_check_leases(&fleet.repo, now)
    };
    if leases_reaped > 0 {
        println!(
            "  ⌫ {}{leases_reaped} leaked check-lease(s) reaped (dead-PID/TTL-stale — would have stalled the merge gate)",
            if dry_run { "DRY-RUN would reap " } else { "" }
        );
    }

    println!(
        "fleet watchdog: checked {checked} active windowed agent(s); {}{rearmed} re-armed{}, {reaped} stopped window(s) reaped, {leases_reaped} leaked lease(s) reaped, {} queued-but-landed MR(s) surfaced.",
        if dry_run { "DRY-RUN: " } else { "" },
        // Surface the cron-death subset inline only when it fired — `reissued` of the re-arms escalated to
        // re-issuing `/loop` because a prior nudge didn't stick (the agent's recurring cron was dead, not a
        // mere missed tick). Persistently >0 = the loop-liveness root, not just the watchdog netting ticks.
        if reissued > 0 {
            format!(" ({reissued} via /loop re-issue — dead cron)")
        } else {
            String::new()
        },
        landed_queued.len()
    );

    // Persist a health record. The signals above go to stdout/stderr, which SCROLLS OFF the terminal —
    // so a drain-stall/saturation/no-op finding is lost once the pane advances, and the operator can't
    // review recurrence ("was anything at 100% an hour ago?"). Append a one-line timestamped summary to
    // `.claude/fleet/watchdog.log` (the trunk-clobber logger set the append-only-log precedent), but
    // ONLY on a real run (not --dry-run, which is a preview) and ONLY when something is notable — a
    // clean sweep writes nothing, so the log is a greppable record of ANOMALIES, not noise.
    if !dry_run {
        let summary = watchdog_log_line(
            now,
            checked,
            &WatchdogCounts {
                rearmed,
                reissued,
                reaped,
                drain_stalls,
                saturated,
                queued_but_landed: landed_queued.len(),
                wedge_escalations,
                leases_reaped,
            },
        );
        if let Some(line) = summary {
            let log = fleet.root.join("watchdog.log");
            // Surface an IO failure rather than swallowing it (PR #492 Copilot): this log IS the
            // operator's durable liveness signal, so a read-only / full FS silently dropping the append
            // would make the record stop with no warning. Warn on BOTH open and write failure.
            use std::io::Write;
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
            {
                Ok(mut f) => {
                    if let Err(e) = writeln!(f, "{line}") {
                        eprintln!(
                            "  ! watchdog: could not WRITE the health-log line to {} ({e}) — record not appended",
                            log.display()
                        );
                    }
                }
                Err(e) => eprintln!(
                    "  ! watchdog: could not OPEN the health log {} ({e}) — findings not persisted this run",
                    log.display()
                ),
            }
        }
    }
}

/// Format a watchdog health-sweep summary for the append-only `watchdog.log`, or `None` when the sweep
/// was CLEAN (nothing re-armed/reaped/flagged) — a clean run writes no line, so the log stays a record
/// of ANOMALIES only. Pure so the "notable?" gate + format are unit-testable. `now` is epoch seconds
/// (stamped by the caller; the log is a host-tooling artifact, not compiled-program state).
/// The health-sweep counters a watchdog run tallies — grouped so the log formatter takes one value,
/// not a long positional arg list (each field is a distinct anomaly class the log records).
#[derive(Default)]
struct WatchdogCounts {
    rearmed: usize,
    /// Subset of `rearmed` that ESCALATED to re-issuing `/loop` (a prior nudge didn't stick → the
    /// agent's recurring cron is presumed dead). The loop-liveness signal: persistently >0 across sweeps
    /// means the `/loop` self-reschedule is failing, not just an occasional missed tick.
    reissued: usize,
    reaped: usize,
    drain_stalls: usize,
    saturated: usize,
    queued_but_landed: usize,
    wedge_escalations: usize,
    leases_reaped: usize,
}

/// `checked` is the sweep's DENOMINATOR (active windowed agents examined) — pure CONTEXT, deliberately
/// NOT a `WatchdogCounts` anomaly counter and NOT part of the notability gate: it is always >0, so gating
/// on it would make every sweep write a line and defeat the anomaly-only log. Logging it lets a reader
/// (and the parked reissued/freeze analysis) tell a fleet-wide freeze (`rearmed=20` of `checked=42`) from
/// scattered missed-ticks (`rearmed=2` of `checked=42`) — the same `rearmed` count means opposite things
/// at different denominators, which the count alone can't distinguish.
fn watchdog_log_line(now: u64, checked: usize, c: &WatchdogCounts) -> Option<String> {
    // Exhaustive "is this sweep an anomaly?" gate — EVERY anomaly counter must appear here, else a sweep
    // that only sets an omitted counter writes no line and its anomaly is silently dropped from the log.
    // `reissued ⊆ rearmed` makes listing it redundant *today* (reissued>0 ⇒ rearmed>0), but the gate
    // documents the contract: a new counter added without a line here is the latent bug. `checked` is
    // NOT gated on purpose (see the fn doc) — it is context, not an anomaly.
    if c.rearmed == 0
        && c.reissued == 0
        && c.reaped == 0
        && c.drain_stalls == 0
        && c.saturated == 0
        && c.queued_but_landed == 0
        && c.wedge_escalations == 0
        && c.leases_reaped == 0
    {
        return None;
    }
    Some(format!(
        "{now}\tchecked={checked}\trearmed={}\treissued={}\treaped={}\tdrain_stalls={}\tsaturated={}\tqueued_but_landed={}\twedge_escalations={}\tleases_reaped={}",
        c.rearmed,
        c.reissued,
        c.reaped,
        c.drain_stalls,
        c.saturated,
        c.queued_but_landed,
        c.wedge_escalations,
        c.leases_reaped,
    ))
}

/// Seconds since the epoch, from the wall clock. (Unlike the Cadenza toolchain, xtask may read the
/// clock — this is host tooling, not compiled-program logic.)
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render a duration in seconds as a compact human age (`45s`, `12m`, `3h`, `2d`) for the board.
fn fmt_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Age in seconds of the agent's heartbeat touch-file (`now - mtime`), or `None` if it has never been
/// stamped (the file is absent — a freshly-launched agent that has not run its first tick yet).
fn heartbeat_age_secs(fleet: &Fleet, name: &str, now: u64) -> Option<u64> {
    file_mtime_unix(&fleet.root.join("heartbeat").join(name)).map(|m| now.saturating_sub(m))
}

/// Age in seconds since we last re-armed this agent (mtime of `.claude/fleet/rearm/<name>`), or
/// `None` if we have never re-armed it.
fn rearm_age_secs(fleet: &Fleet, name: &str, now: u64) -> Option<u64> {
    file_mtime_unix(&fleet.root.join("rearm").join(name)).map(|m| now.saturating_sub(m))
}

/// Touch `.claude/fleet/rearm/<name>` to record that we just re-armed this agent (for the grace check).
fn stamp_rearm(fleet: &Fleet, name: &str) {
    let dir = fleet.root.join("rearm");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), "rearm\n").ok();
}

/// Age in seconds since we last notified the concierge that this agent is context-WEDGED (mtime of
/// `.claude/fleet/sat-notify/<name>`), or `None` if never. Rate-limits the auto-escalation so a wedged
/// agent doesn't generate a fresh concierge `note` on every 4-min sweep until the operator acts.
fn sat_notify_age_secs(fleet: &Fleet, name: &str, now: u64) -> Option<u64> {
    file_mtime_unix(&fleet.root.join("sat-notify").join(name)).map(|m| now.saturating_sub(m))
}

/// Touch `.claude/fleet/sat-notify/<name>` to record that we escalated this agent's context-wedge to the
/// concierge (for the rate-limit).
fn stamp_sat_notify(fleet: &Fleet, name: &str) {
    let dir = fleet.root.join("sat-notify");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), "sat-notify\n").ok();
}

/// Should the watchdog auto-escalate a context-WEDGED agent to the concierge this sweep? Pure so the
/// gate is unit-tested. Fires only when ALL hold:
///   * at/above the PRE-WALL threshold (`ctx_pct >= prewall_threshold_for(agent)`, 95% general / 92%
///     pr-sync) — NOT the old 100%-only gate (escalation there was TOO LATE: the agent was already
///     unrecoverable). At the pre-wall level a `/compact` can still submit, so "hasn't self-compacted
///     yet at 95%" is a real actionable signal for the concierge to nudge/restart before the wall;
///   * NOT rate-limited (`!notified_recently`) — one note per grace window, not every sweep;
///   * the wedged agent is NOT the concierge itself — a `note` to a wedged concierge lands in the very
///     inbox it can't drain, so it's useless. A concierge wedge is the one case only the OPERATOR can
///     fix directly; the caller emits a LOUD stderr line for it instead of a self-note.
///
/// The escalation is a `note` only — never keystrokes to the victim (the concierge/operator decides
/// whether to nudge a `/compact` or restart; a `/loop`/Esc keystroke can't preempt a full window).
fn should_notify_saturation(agent: &str, ctx_pct: Option<u8>, notified_recently: bool) -> bool {
    agent != "concierge"
        && matches!(ctx_pct, Some(p) if p >= prewall_threshold_for(agent))
        && !notified_recently
}

/// The last auto drain-nudge we sent this agent, as `(flagged-message-id, age-in-secs)`, or `None`
/// if never nudged. The marker file `.claude/fleet/drain-nudge/<name>` records the id of the OLDEST
/// unconsumed message at nudge time in its body (mtime = when we nudged). Recording the id lets the
/// watchdog tell a NEW message churning each tick (the agent IS draining — a different id — healthy,
/// don't re-nudge) from the SAME message still unconsumed after our nudge (the nudge didn't stick —
/// re-nudge on a shorter cooldown + flag loudly). An empty id (legacy marker written before ids were
/// tracked) reads as `""`, which never equals a real filename, so it degrades to the churn path.
fn last_drain_nudge(fleet: &Fleet, name: &str, now: u64) -> Option<(String, u64)> {
    let path = fleet.root.join("drain-nudge").join(name);
    let age = file_mtime_unix(&path).map(|m| now.saturating_sub(m))?;
    let id = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Some((id, age))
}

/// Record an auto drain-nudge: write the flagged message-id to `.claude/fleet/drain-nudge/<name>`
/// (its mtime is the nudge time). The id is the oldest unconsumed message we saw — a later sweep
/// compares against it to detect a nudge that didn't stick (same id) vs healthy churn (new id).
fn stamp_drain_nudge(fleet: &Fleet, name: &str, flagged_id: &str) {
    let dir = fleet.root.join("drain-nudge");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), format!("{flagged_id}\n")).ok();
}

/// The oldest (by durable delivery seq) unconsumed ACTIONABLE message filename in this agent's hub
/// inbox, or `None` if none. Used as the drain-stall "flagged message id": if the same one persists
/// after a nudge, the nudge didn't stick; a different one means the agent drained the old one (healthy
/// churn). MUST filter to actionable kinds to match the signal that fired — the drain-stall triggers on
/// `actionable_inbox_depth`, so keying the stuck-identity on the oldest message of ANY kind would let a
/// stale un-archived informational note (older than the real actionable item) hijack the identity: the
/// agent could drain the actionable message yet keep the note → id unchanged → a false `Stuck`
/// re-nudge; or archive the note while the actionable item persists → id flips → a real stall wrongly
/// treated as fresh churn (full grace instead of `Stuck`). Both are avoided by tracking the actionable
/// message the signal is actually about.
fn oldest_actionable_inbox_message(fleet: &Fleet, name: &str) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(fleet.inbox(name))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| f.ends_with(".json"))
        .filter(|f| message_kind_from_filename(f).is_some_and(message_kind_is_actionable))
        .collect();
    sort_inbox_filenames(&mut names);
    names.into_iter().next()
}

/// Age in seconds since the watchdog FIRST saw this (never-heartbeated) agent with a live window
/// (mtime of `.claude/fleet/firstseen/<name>`), or `None` if never marked. Gives a clock to tell a
/// still-booting agent from one whose cold start FAILED (launched, never completed tick 1).
fn firstseen_age_secs(fleet: &Fleet, name: &str, now: u64) -> Option<u64> {
    file_mtime_unix(&fleet.root.join("firstseen").join(name)).map(|m| now.saturating_sub(m))
}

/// Touch `.claude/fleet/firstseen/<name>` the first time the watchdog sees a never-heartbeated agent
/// with a live window, so a later pass can measure how long its cold start has been pending.
fn stamp_firstseen(fleet: &Fleet, name: &str) {
    let dir = fleet.root.join("firstseen");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), "firstseen\n").ok();
}

/// The actionable inbox depth this agent had on the PRIOR watchdog sweep, or `None` if we've never
/// recorded one (first sweep). Persisted as a tiny touch-file per agent (mirrors `firstseen`), read
/// before `record_actionable_depth` overwrites it — feeds `queue_is_draining` for the drop-detection
/// that exonerates a healthily-draining single-writer whose pane reads idle mid-batch.
fn last_actionable_depth(fleet: &Fleet, name: &str) -> Option<usize> {
    std::fs::read_to_string(fleet.root.join("actionable-depth").join(name))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Record this sweep's actionable inbox depth for `name`, for next sweep's draining-delta check. Best
/// effort (ignore IO errors — a missed write just means no drop is detected next sweep, i.e. the safe/
/// conservative direction: we'd warn rather than silently miss a real stall).
fn record_actionable_depth(fleet: &Fleet, name: &str, depth: usize) {
    let dir = fleet.root.join("actionable-depth");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(name), format!("{depth}\n")).ok();
}

/// A file's modification time as seconds since the epoch, or `None` if it can't be read.
fn file_mtime_unix(path: &Path) -> Option<u64> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

// ── check-lease — fleet-wide concurrency limiter for `cargo xtask check` ──────────────────────────
//
// OPERATOR-MANDATED (2026-07-17): "limit the number of checks taking place at any given time — can't
// keep overloading the system" + "prioritize the main merge over other checks". Uncoordinated
// concurrent `cargo xtask check` runs (each a full build + ~3700-case gate) drove the 64-core host to
// load 127-180 and SIGKILL'd pr-sync's gate-batch, so the merge queue starved behind a thundering herd
// of one-off vertical checks. This advisory lease caps concurrency and lets the merge integrator WIN.
//
// A lease is a file `<hub>/.claude/fleet/check-leases/<pid>-<class>.lease`; `class` is `priority`
// (pr-sync's merge-queue gate) or `vertical` (everyone else). Acquire = (reap stale) → obey the rules
// below → create my file. Release = remove my file (RAII on `CheckLease::drop`, so a normal exit frees
// the slot). A crash/SIGKILL can't free it, so a lease older than the TTL is reclaimed by any acquirer
// (mtime-based liveness, the same idiom as heartbeats — no `/proc`/`libc` needed). Rules:
//   * PRIORITY (pr-sync) never waits behind a vertical — it acquires immediately (its own uncapped
//     class), so the main merge is never held up for one-off agent checks.
//   * A VERTICAL yields to priority: it waits while any priority lease is held, AND waits for a free
//     slot under the cap. So when pr-sync gates the queue, vertical checks pause rather than compete.
// Fail-OPEN: if the lease dir can't be created/read, `check` proceeds unthrottled (a coordination
// convenience must never HARD-BLOCK a developer's gate on an fs hiccup).

/// Max concurrent NON-priority (`vertical`) checks. Small by design — a full check is a whole build +
/// gate; ~3 keeps the 64-core host usable while several verticals make progress. Tunable for a bigger/
/// smaller host via `CDZ_CHECK_LEASE_MAX`.
fn check_lease_max() -> usize {
    std::env::var("CDZ_CHECK_LEASE_MAX")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3)
}

/// A lease older than this (no live holder refreshing it) is reclaimed. Must EXCEED the longest legit
/// check (a slow full gate under load is ~35min) so a still-running check is never wrongly evicted;
/// 45min gives margin. A SIGKILL'd check's lease is reclaimed after this instead of leaking a slot.
const CHECK_LEASE_TTL_SECS: u64 = 45 * 60;

/// An acquired check-lease; releasing (removing the lease file) happens on drop, so a normal `check`
/// exit — or an early `return`/panic — frees the slot without an explicit release call.
pub struct CheckLease {
    file: Option<PathBuf>,
}

impl Drop for CheckLease {
    fn drop(&mut self) {
        if let Some(f) = &self.file {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// The `.claude/fleet/check-leases` dir for the hub that owns `repo`. `None` if the hub can't be
/// resolved or the dir can't be created (→ caller fails open, unthrottled).
fn check_lease_dir(repo: &Path) -> Option<PathBuf> {
    let hub = hub_root(repo)?;
    let dir = hub.join(".claude/fleet/check-leases");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Is the process that owns a lease still alive? A lease file is named `{pid}-{class}.lease`; the
/// holder drops it on a CLEAN exit, but a SIGKILL (the 137s this throttle exists to prevent, or
/// pr-sync preempting a gate) runs no `Drop`, so the file LEAKS. On Linux a live pid has a `/proc/<pid>`
/// entry; a dead one does not. `None` = "can't tell" (unparseable name / non-Linux) → treat as alive so
/// we NEVER reap a lease we're unsure about (the TTL is the backstop for that case).
///
/// ZOMBIE GAP (concierge: a leaked priority lease stalled the fleet twice, `leases_reaped=0` during the
/// stall): a SIGKILL'd gate sub-process becomes a DEFUNCT (zombie) entry — `/proc/<pid>` still EXISTS
/// until its parent `wait()`s it, but it will never run `Drop`, so a bare-existence check reports it
/// ALIVE and the lease is never reaped. So don't stop at existence: read `/proc/<pid>/stat`'s process
/// state and treat `Z` (zombie) as DEAD. A live process in any other state (`R`/`S`/`D`/`T`/…) is alive.
fn check_lease_holder_alive(lease_name: &str) -> Option<bool> {
    let pid: u32 = lease_name.split('-').next()?.parse().ok()?;
    // Only trust a definitive "/proc exists but this pid is gone/zombie" as dead; if /proc itself is
    // absent (non-Linux) we can't tell, so report alive and let the TTL handle staleness.
    if !Path::new("/proc").is_dir() {
        return None;
    }
    let proc_entry = Path::new("/proc").join(pid.to_string());
    if !proc_entry.exists() {
        return Some(false); // reaped/gone → dead
    }
    // Present in /proc — but a zombie (SIGKILL'd, not yet wait()'d) is effectively dead for our purpose
    // (it holds the lease but will never Drop it). Read its `stat` state; a Z means reap now.
    match std::fs::read_to_string(proc_entry.join("stat")) {
        Ok(stat) => Some(!proc_stat_is_zombie(&stat)),
        // /proc/<pid> exists but stat is unreadable (a race: it just exited) — treat as alive and let
        // the next scan / the TTL settle it, rather than reaping on an ambiguous read.
        Err(_) => Some(true),
    }
}

/// Is a `/proc/<pid>/stat` line a ZOMBIE (defunct) process? The state is the field AFTER the parenthesized
/// comm, e.g. `1234 (cargo) Z 1200 …` → `Z`. The comm can contain spaces/parens, so scan to the LAST `)`
/// and take the first non-space token after it. Returns false if the line can't be parsed (caller then
/// treats an unparseable-but-present pid as alive — the TTL is the backstop). Pure, so it's unit-testable.
fn proc_stat_is_zombie(stat: &str) -> bool {
    match stat.rfind(')') {
        Some(i) => stat[i + 1..]
            .split_whitespace()
            .next()
            .is_some_and(|state| state == "Z"),
        None => false,
    }
}

/// Live (`priority`, `vertical`) lease counts in `dir`, reaping leaked leases first. A lease is leaked —
/// and reaped immediately — when its holder pid is DEAD (SIGKILL ran no `Drop`); as a backstop for the
/// can't-tell case (pid reuse, non-Linux) a lease older than the TTL is also reaped. Returns
/// `(priority_live, vertical_live)`.
fn scan_check_leases(dir: &Path, now: u64) -> (usize, usize) {
    let (prio, vert, _reaped) = scan_check_leases_with(dir, now, check_lease_holder_alive);
    (prio, vert)
}

/// Reap leaked check-leases OUT OF BAND and return how many were reaped — the same reclaim logic the
/// acquire loop runs, but callable by `fleet watchdog` (the concierge cron) WITHOUT taking a lease.
/// This closes the recurring fleet-stall the acquire-path reaper can't: when pr-sync's OWN gate is the
/// wedged priority-acquirer, a dead-PID priority lease it's blocking on is never scanned by a fresh
/// acquire (there is none — pr-sync is stuck retrying), so it lingers the full 45-min TTL. A periodic
/// out-of-band sweep reaps it regardless of who (if anyone) is trying to acquire. `None` dir → 0.
fn reap_check_leases(repo: &Path, now: u64) -> usize {
    let Some(dir) = check_lease_dir(repo) else {
        return 0;
    };
    reap_check_leases_in(&dir, now)
}

/// Reap leaked leases in an already-resolved lease `dir`, returning the count — the reap core split out
/// from `reap_check_leases`'s hub/git dir-resolution so it's unit-testable against a plain temp dir.
fn reap_check_leases_in(dir: &Path, now: u64) -> usize {
    let (_prio, _vert, reaped) = scan_check_leases_with(dir, now, check_lease_holder_alive);
    reaped
}

/// The scan mechanics, parameterized on the pid-liveness probe so it's unit-testable without touching
/// `/proc` or real pids. `is_alive(lease_name) -> Option<bool>`: `Some(false)` = reap now, anything else
/// = keep (subject to the TTL backstop). Returns `(priority_live, vertical_live, reaped)` — the reaped
/// count lets the out-of-band watchdog sweep report how many leaked leases it reclaimed.
fn scan_check_leases_with(
    dir: &Path,
    now: u64,
    is_alive: impl Fn(&str) -> Option<bool>,
) -> (usize, usize, usize) {
    let mut priority = 0usize;
    let mut vertical = 0usize;
    let mut reaped = 0usize;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, 0, 0);
    };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".lease") => n.to_string(),
            _ => continue,
        };
        // Reclaim a leaked lease. PRIMARY: the holder pid is dead (SIGKILL left the file behind) —
        // reap NOW rather than waiting out the 45-min TTL, which would stall every vertical check (a
        // leaked PRIORITY lease blocks ALL verticals). BACKSTOP: `None` (can't tell) falls through to
        // the TTL so a reused pid or a non-Linux host still self-heals eventually. A dead-PID reap and
        // a TTL reap BOTH count — the out-of-band watchdog sweep reports the total reclaimed.
        if is_alive(&name) == Some(false) {
            let _ = std::fs::remove_file(&p);
            reaped += 1;
            continue;
        }
        if file_mtime_unix(&p).is_some_and(|m| now.saturating_sub(m) > CHECK_LEASE_TTL_SECS) {
            let _ = std::fs::remove_file(&p);
            reaped += 1;
            continue;
        }
        if name.contains("-priority.lease") {
            priority += 1;
        } else {
            vertical += 1;
        }
    }
    (priority, vertical, reaped)
}

/// The acquire DECISION (pure, so it's unit-testable without the fs/poll loop): may a caller take a
/// slot NOW given its class and the live counts? PRIORITY (pr-sync's merge gate) always may — it never
/// waits behind vertical checks. A VERTICAL may only when NO priority lease is held (yield to the merge
/// queue) AND the vertical count is under the cap.
fn check_lease_go(priority: bool, prio_live: usize, vert_live: usize, max: usize) -> bool {
    if priority {
        true
    } else {
        prio_live == 0 && vert_live < max
    }
}

/// Acquire a check-lease for `cargo xtask check`, blocking (with a poll loop) until it's this process's
/// turn under the fleet-wide concurrency rules. `priority` (pr-sync's merge-queue gate) acquires
/// immediately; a vertical waits for a free slot AND for zero priority leases. Fail-open: an
/// unresolvable/unwritable lease dir returns an inert guard that throttles nothing.
///
/// `priority` defaults from the `CDZ_CHECK_PRIORITY` env (set by pr-sync's gate invocation) but callers
/// may pass an explicit value.
pub fn acquire_check_lease(repo: &Path, priority: bool) -> CheckLease {
    let Some(dir) = check_lease_dir(repo) else {
        eprintln!("check-lease: no lease dir (failing OPEN — unthrottled).");
        return CheckLease { file: None };
    };
    let pid = std::process::id();
    let class = if priority { "priority" } else { "vertical" };
    let file = dir.join(format!("{pid}-{class}.lease"));
    let max = check_lease_max();

    // Poll until it's our turn. A priority acquirer skips the wait entirely.
    let mut waited_notice = false;
    loop {
        let now = now_unix();
        let (prio_live, vert_live) = scan_check_leases(&dir, now);
        let go = check_lease_go(priority, prio_live, vert_live, max);
        if go {
            // Create the lease; its mtime marks acquisition (and is our liveness stamp).
            if std::fs::write(&file, format!("{}\t{}", now, class)).is_err() {
                eprintln!("check-lease: could not write lease (failing OPEN — unthrottled).");
                return CheckLease { file: None };
            }
            return CheckLease { file: Some(file) };
        }
        if !waited_notice {
            println!(
                "check-lease: waiting for a check slot ({vert_live}/{max} vertical in use, \
                 {prio_live} priority) — yielding to the merge queue…"
            );
            waited_notice = true;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// Parse a `/loop` interval like `10m` / `2h` / `30s` / `1d` into seconds. Falls back to 600s (the
/// 10m default) on anything unrecognized, so a malformed interval never yields a 0-second stale
/// window that would re-arm on every pass.
fn parse_interval_secs(s: &str) -> u64 {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num.parse().unwrap_or(0);
    if n == 0 {
        return 600;
    }
    match unit {
        "s" => n,
        "m" | "" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => 600,
    }
}

/// How long to tolerate heartbeat SILENCE before presuming a loop dead. A heartbeat is stamped at the TOP
/// of every tick, so a HEALTHY agent's heartbeat age cycles 0 → interval → (tick) → 0; it is only
/// genuinely overdue once it has been silent for LONGER than one full tick interval. The window is
/// therefore ONE INTERVAL (so an idle-but-healthy agent between its ticks is NEVER flagged) plus a
/// CAPPED grace beyond it: `interval + min(interval×(mult-1), cap)`.
///
/// This fixes the false-re-arm the old `min(interval×mult, cap)` caused: that capped the window BELOW
/// the interval for a long-interval agent (a 30m agent got a 600s window), so it read "stale" for ~20 of
/// every 30 min between legitimate ticks and got re-armed almost every ~4-min sweep — effectively
/// ticking ~7× faster than its configured interval (concierge ruling 2026-07-19, option (c): preserve
/// the real cadence). The `cap` now bounds only the GRACE beyond one interval, so a genuinely-dead loop
/// is still caught within `interval + cap` (e.g. a 30m agent within ~40min; a 10m within ~20min) while a
/// healthy one is left to tick on its own schedule.
fn stale_window_secs(interval_secs: u64, mult: u32, cap: u64) -> u64 {
    let grace = interval_secs
        .saturating_mul(mult.saturating_sub(1) as u64)
        .min(cap);
    interval_secs.saturating_add(grace)
}

/// Whether a busy-looking pane should be TRUSTED as "genuinely mid-tick, leave alone" for a stale
/// agent. Pure so the invariant is unit-tested. A pane only counts as real work if the agent has EVER
/// stamped a heartbeat (`hb_ever` = `heartbeat_age_secs(...).is_some()`): a genuine tick stamps its
/// heartbeat at step 1 before any long work, so a has-looped agent with a busy pane is mid-tick. A
/// never-heartbeated agent past the cold-start window with a busy pane is FLAILING (stuck retrying),
/// not working — so its pane must NOT be trusted, else it's skipped forever (the origin/trunk-flail
/// false-positive). Returns true ⇒ honor the busy pane and skip re-arm; false ⇒ ignore the pane.
fn pane_busy_means_working(hb_ever: bool, pane_busy: bool) -> bool {
    hb_ever && pane_busy
}

/// Whether an agent looks like it's in a silent DRAIN-STALL the watchdog should flag: it has unconsumed
/// ACTIONABLE messages in its (hub) inbox AND its pane is idle (not mid-tick). Pure so the signal's gate
/// is unit-testable. This is orthogonal to the heartbeat/stale check — a drain-stalled agent typically
/// has a FRESH heartbeat (the loop runs; it just doesn't drain), so the normal staleness path would wave
/// it through as healthy. The two INTERACTIVE roles (`concierge`, `design`) sit idle with mail a human
/// reads on their own cadence — that is NOT a stall, so exempt them.
///
/// 🔑 `actionable_depth` counts only queued messages of an ACTIONABLE kind (assign/reject/ask/issue/
/// request/answer/…), NOT informational mail (note/merged/backlog/…) — see `message_kind_is_actionable`.
/// An agent idling with only stale informational mail it never archived is UNTIDY, not stalled; counting
/// it would fire a false drain-stall (and, under `--nudge-drain-stalls`, a pointless nudge). `pane_idle`
/// should already fold in "has ever heartbeated" (an un-booted agent's idle pane is a cold start, not a
/// drain-stall).
fn is_probable_drain_stall(role: &str, actionable_depth: usize, pane_idle: bool) -> bool {
    if matches!(role, "concierge" | "design") {
        return false;
    }
    actionable_depth > 0 && pane_idle
}

/// Final drain-stall verdict after the CONFIRMING recapture. `is_probable_drain_stall` fires off ONE
/// instantaneous pane snapshot, which can catch a busy agent in the sub-second gap between its tool-turns
/// (prompt shown, "esc to interrupt" not yet re-rendered) and mislabel it idle. So the caller takes a
/// second capture after `DRAIN_STALL_CONFIRM_DELAY_SECS` and passes whether the pane shows work in flight
/// on that recheck: if it does, the agent was mid-tick, not stalled → NOT confirmed. A genuinely idle
/// agent still shows no work on the recheck → confirmed. Pure so the two-sample AND is unit-tested.
fn drain_stall_confirmed(suspected: bool, working_on_recheck: bool) -> bool {
    suspected && !working_on_recheck
}

/// Is the agent's actionable inbox DRAINING across sweeps? A strict drop from the prior sweep's depth is
/// positive proof the loop is consuming its own mail — only self-consumption lowers the count (new mail
/// raises it). This exonerates a single-writer (e.g. pr-sync) that bakes a multi-MR gate BATCH in a
/// detached subprocess: its pane reads idle-at-prompt on BOTH watchdog captures and trunk stays flat
/// until the whole batch lands, so it looks drain-stalled, yet the shrinking queue proves it isn't.
/// `None` prev (first sweep we've recorded a depth for this agent) can't establish a drop → NOT draining
/// (a genuinely wedged agent must not get exonerated on the very sweep we first observe it). Pure so the
/// monotonicity is unit-tested; the caller supplies the persisted prior depth.
fn queue_is_draining(prev_depth: Option<usize>, current_depth: usize) -> bool {
    prev_depth.is_some_and(|prev| current_depth < prev)
}

/// What to do about a windowed agent that has NEVER stamped a heartbeat, given how long ago we first
/// saw it (`firstseen_age`, `None` = never marked) and its cold-start grace window. Pure so the
/// `seen <= window` grace boundary is unit-tested (mirroring `agent_reads_stale`/`queue_is_draining`):
/// a first tick may legitimately spend the whole window building before it ever stamps, so the grace is
/// INCLUSIVE — `seen == window` is still booting, not yet a failed start. The caller performs the I/O
/// (stamping first-seen, printing, re-arming); this only classifies.
#[derive(Debug, PartialEq, Eq)]
enum ColdStartVerdict {
    /// Never marked first-seen before — stamp it now and give it the cold-start window.
    MarkAndWait,
    /// Marked, still within the grace window — keep waiting (no re-arm).
    StillWaiting,
    /// Marked, past the window, still no heartbeat — cold start failed; re-arm. Carries the first-seen
    /// age to reuse as the staleness measure downstream.
    Failed(u64),
}

fn coldstart_verdict(firstseen_age: Option<u64>, coldstart_window: u64) -> ColdStartVerdict {
    match firstseen_age {
        None => ColdStartVerdict::MarkAndWait,
        Some(seen) if seen <= coldstart_window => ColdStartVerdict::StillWaiting,
        Some(seen) => ColdStartVerdict::Failed(seen),
    }
}

/// What the watchdog should do about a probable drain-stall this sweep. Distinguishes a first/fresh
/// nudge from a re-nudge whose PRIOR nudge didn't stick (same message still unconsumed) — the latter
/// gets a shorter cooldown AND a louder "auto-nudge failed" line so the concierge can hand-arm it
/// immediately instead of waiting out the full anti-spam grace.
#[derive(Debug, PartialEq, Eq)]
enum DrainNudge {
    /// Don't nudge (opt-in off, not a stall, saturated, or still within a cooldown).
    Skip,
    /// Nudge normally — first nudge, or the flagged message changed (the agent drained the old one;
    /// this is a fresh/different stall, rate-limited by the full anti-churn grace).
    Fresh,
    /// Re-nudge because the SAME message is still unconsumed past the short recheck window — our prior
    /// nudge didn't land. The caller should also emit a louder "nudge didn't stick" signal.
    Stuck,
}

/// Decide the AUTO-NUDGE action for a probable drain-stall (only under the opt-in
/// `--nudge-drain-stalls`). Pure so the guard combination is unit-tested — auto-sending keystrokes to
/// panes is the highest-risk watchdog action, so every guard is explicit here:
///   * `nudge_enabled` — the opt-in flag (default OFF; report-only unless the operator sets it).
///   * `is_drain_stall` — the signal already fired (idle-at-prompt + unconsumed hub mail).
///   * NOT context-saturated — a `>=` saturation-threshold pane needs a RESTART, not a nudge (a nudge
///     can't help a session that can't submit); those are report-only, never nudged. `ctx_pct` None
///     (no marker) is treated as NOT saturated (safe to nudge).
///
/// The two-cooldown logic (the concierge's refinement — v-metaprogramming sat ~8min because a nudge
/// that didn't stick was suppressed by the single 15-min grace): compare the currently-flagged message
/// id against the one we last nudged for.
///   * SAME id → our nudge didn't stick. Suppress only within the SHORT `stuck_grace` (give it a tick
///     or two to act), then re-nudge (`Stuck`) rather than waiting out the full grace.
///   * DIFFERENT id (or never nudged) → the agent drained what we last nudged; this is a fresh stall
///     (or possibly healthy churn — a new message that drains next tick). Rate-limit by the FULL
///     `churn_grace` so a churning inbox isn't nudged every sweep, then `Fresh`.
///
/// The pane-idle + role-exempt conditions are already folded into `is_drain_stall`.
#[allow(clippy::too_many_arguments)]
fn decide_drain_nudge(
    nudge_enabled: bool,
    is_drain_stall: bool,
    ctx_pct: Option<u8>,
    saturation_threshold: u8,
    flagged_id: Option<&str>,
    last_nudge: Option<(&str, u64)>,
    churn_grace: u64,
    stuck_grace: u64,
) -> DrainNudge {
    if !nudge_enabled || !is_drain_stall {
        return DrainNudge::Skip;
    }
    // A saturated pane can't act on a nudge — leave it for a restart (report-only).
    if matches!(ctx_pct, Some(p) if p >= saturation_threshold) {
        return DrainNudge::Skip;
    }
    match last_nudge {
        // Never nudged this agent → fresh nudge.
        None => DrainNudge::Fresh,
        Some((prev_id, age)) => {
            let same_message = flagged_id.is_some_and(|id| id == prev_id) && !prev_id.is_empty();
            if same_message {
                // Prior nudge didn't stick. Suppress only briefly, then re-nudge loudly.
                if age < stuck_grace {
                    DrainNudge::Skip
                } else {
                    DrainNudge::Stuck
                }
            } else {
                // Different (or unknown) message — healthy churn / fresh stall. Full anti-spam grace.
                if age < churn_grace {
                    DrainNudge::Skip
                } else {
                    DrainNudge::Fresh
                }
            }
        }
    }
}

/// Capture an agent's visible tmux pane text (no scrollback), or `None` if tmux errors / the window is
/// gone. Shared by the pane-based signals (working-detection, context-saturation) so the watchdog reads
/// each pane once per concern via one primitive.
fn capture_pane(session: &str, agent: &str) -> Option<String> {
    let target = format!("{session}:{agent}");
    Command::new("tmux")
        .args(["capture-pane", "-p", "-t", &target])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
}

/// Does a captured pane's TEXT indicate Claude is alive + busy (so re-arming or drain-flagging it would
/// interrupt real work / misread liveness)? Two signals, pure so both watchdog sites share one rule:
///   * "esc to interrupt" — a turn is in flight (the normal working affordance).
///   * "Retrying in …" / "attempt N/N" — Claude Code is in API-retry BACKOFF (a 429/5xx transient). The
///     loop IS alive and will resume when the retry succeeds; the retry status line can momentarily
///     REPLACE the "esc to interrupt" affordance, so without this an agent riding out a rate-limit reads
///     as idle-at-prompt and gets falsely flagged as a drain-stall (observed: `breaker` under repeated
///     429s flagged 2 sweeps running). An API-retry is liveness, not idleness — treat it as working.
///
/// Pure + unit-testable (no tmux).
fn pane_shows_working(pane_text: &str) -> bool {
    pane_text.contains("esc to interrupt")
        || pane_text.contains("Retrying in ")
        || pane_text.contains("Retrying…")
}

/// Does the agent's tmux pane show Claude actively working? Claude Code prints an "esc to interrupt"
/// affordance in its status line while a turn is in flight (and a "Retrying …" line during API-retry
/// backoff); either means the loop is alive and mid-tick, so a stale heartbeat is just a long tick / a
/// transient rate-limit — don't re-arm (that would inject a `/loop` into the middle of real work).
/// Captures only the visible pane (no scrollback).
fn window_is_working(session: &str, agent: &str) -> bool {
    capture_pane(session, agent)
        .map(|s| pane_shows_working(&s))
        .unwrap_or(false)
}

/// Parse the "% context used" indicator Claude Code renders in its status line (e.g. "97% context
/// used", or just "100% context" mid-render) out of a captured pane. Returns the integer percent, or
/// `None` if no such marker is present (the agent isn't near any threshold, or the pane didn't render
/// it this capture). Pure so the parse is unit-testable. Takes the LAST match (the live status line is
/// at the bottom; an older value may linger higher in the visible buffer).
fn parse_context_pct(pane_text: &str) -> Option<u8> {
    let mut found: Option<u8> = None;
    for (i, _) in pane_text.match_indices("% context") {
        // Walk back over the digits immediately preceding the '%'.
        let bytes = pane_text.as_bytes();
        let mut start = i;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        if start < i
            && let Ok(pct) = pane_text[start..i].parse::<u16>()
        {
            found = Some(pct.min(100) as u8); // clamp; a status line never exceeds 100%
        }
    }
    found
}

/// Whether a captured context-percent crosses the report threshold for the watchdog's saturation
/// signal. Pure so the gate is unit-testable. A context-saturated agent is INVISIBLE to the other
/// liveness checks — its loop is alive (heartbeat fresh) and its pane is not idle-at-prompt (it's
/// mid-turn / "Noodling"), so neither the staleness check nor the drain-stall signal flags it — yet at
/// 100% it's unrecoverable (even `/compact` can't submit). Flagging at a threshold BELOW 100% (default
/// 85) lets the concierge `/compact` proactively while it still submits. `None` (no marker) → not
/// flagged.
fn context_saturation_warning(pct: Option<u8>, threshold: u8) -> bool {
    matches!(pct, Some(p) if p >= threshold)
}

/// Re-arm a stalled agent by nudging its idle pane to run a tick NOW — by typing `continue` + Enter,
/// the SAME proven primitive as [`nudge_tick`]. NOTE: this deliberately does NOT send `/loop
/// <interval>`. That was the original re-arm, but it is a NO-OP: the `/loop` skill treats an interval
/// with no prompt as an empty prompt and does nothing (a stalled concierge literally printed "`/loop
/// 30m` — empty prompt, nothing to schedule. No-op"), so the watchdog's re-arm never actually revived
/// anything. The recurring `/loop` schedule already exists on a stalled agent; what it needs is a
/// keystroke that makes it run its next tick, and `continue` does exactly that (the idle agent's
/// context still holds its role + `/loop` cadence). The `_interval` arg is kept for signature/logging
/// compatibility but no longer used to build the keystroke.
fn rearm_window(session: &str, agent: &str, _interval: &str) -> bool {
    nudge_tick(session, agent)
}

/// How the watchdog should revive a stalled agent this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RearmAction {
    /// One-shot keystroke: run one more tick now. Enough when the recurring `/loop` cron is intact and
    /// merely missed a tick — the common case for a mature looping agent.
    NudgeContinue,
    /// Re-issue the full `/loop <interval> <tick>`: ARM a recurring cron. For a stall a prior one-shot
    /// nudge failed to fix — the fresh-mint cold-start whose cron never armed, so `continue` ticks once
    /// and freezes again. Re-issuing `/loop` establishes the self-sustaining loop the one-shot can't.
    ReissueLoop,
}

/// Decide the re-arm action from the re-arm and heartbeat ages (both "seconds ago"; larger = older;
/// `None` = never). Pure so it's unit-tested.
///
/// A cheap one-shot `continue` revives a loop whose cron is intact but merely missed a tick. Escalate
/// to re-issuing `/loop` (which ARMS a cron) ONLY when a prior nudge demonstrably didn't stick — i.e.
/// we re-armed this agent AND it has NOT heartbeated since (the no-cron fresh-mint signature: the
/// nudge ran one inline tick, then nothing).
///
/// The discriminator is "heartbeat since the last re-arm", NOT merely "ever re-armed": the `rearm`
/// marker is never cleared, so an agent nudged once long ago (that then recovered and looped healthily
/// for days) still has the marker. Keying escalation on "ever re-armed" would wrongly re-issue `/loop`
/// into that healthy agent the next time it has a slow tick. So a re-armed agent that DID heartbeat
/// after the re-arm (`hb_age < rearm_age`) is treated as healthy-but-currently-stale → cheap nudge;
/// escalate only when `hb_age >= rearm_age` (no tick since) or it never heartbeated at all.
fn rearm_action(rearm_age: Option<u64>, hb_age: Option<u64>) -> RearmAction {
    match rearm_age {
        // Never re-armed → first-time nudge.
        None => RearmAction::NudgeContinue,
        Some(ra) => match hb_age {
            // Heartbeated more recently than the re-arm ⟹ the nudge worked + it looped since; a fresh
            // stall is a genuine new one → cheap nudge.
            Some(hb) if hb < ra => RearmAction::NudgeContinue,
            // No heartbeat since the re-arm (older-or-equal), or never heartbeated → nudge didn't
            // stick → escalate to arm a cron.
            _ => RearmAction::ReissueLoop,
        },
    }
}

/// Build the recurring tick prompt the watchdog passes when it re-issues `/loop` for a stalled agent.
/// Mirrors `window.sh`'s kickoff TICK recipe (heartbeat → drain inbox → one gated unit of work) so a
/// watchdog-armed loop runs the SAME contract as a freshly-launched one — the point of the fix is that
/// a never-looped mint ends up with a real recurring loop, not a degraded one. Paths resolve against
/// the hub-anchored fleet state + the agent's own worktree, exactly like the launcher.
fn watchdog_tick_prompt(fleet: &Fleet, a: &Agent) -> String {
    let inbox = fleet.root.join("inbox").join(&a.name);
    let role_body = format!("{}/fleet/loops/{}.md", a.worktree, a.role);
    let vnote = if a.vertical.is_empty() {
        String::new()
    } else {
        format!(
            " Your vertical is '{}' in subsystem '{}'.",
            a.vertical,
            if a.area.is_empty() { "rcdzc" } else { &a.area }
        )
    };
    format!(
        "Run one tick of your role ({role}){vnote}: (1) cargo xtask fleet heartbeat {name} (stop \
         cleanly if a stop-file exists); (2) drain your inbox {inbox}/ oldest-first, acting on each \
         message then moving it to processed/; (3) sync (git fetch && rebase trunk) and do ONE \
         well-scoped unit of work per {role_body}, gating it green before sending pr-sync a \
         merge-request. Coordinate with peers only via 'cargo xtask fleet send'; if you need a human \
         decision, send the concierge an 'ask' and keep working — never wait for a reply.",
        role = a.role,
        name = a.name,
        inbox = inbox.display(),
    )
}

/// Re-issue a full `/loop <interval> <tick>` into an agent's tmux window to ARM a recurring cron (the
/// escalation path — see [`RearmAction::ReissueLoop`]). Unlike [`nudge_tick`]'s one-shot `continue`,
/// this types the whole `/loop` command so the loop skill schedules the recurring job AND runs the
/// first tick. The prompt is sent with `-l` (literal) so no chars are interpreted as tmux key names.
fn reissue_loop(session: &str, agent: &str, interval: &str, tick_prompt: &str) -> bool {
    let target = format!("{session}:{agent}");
    let line = format!("/loop {interval} {tick_prompt}");
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &target, "-l", &line])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sent {
        return false;
    }
    Command::new("tmux")
        .args(["send-keys", "-t", &target, "Enter"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── archive ────────────────────────────────────────────────────────────────────────────────────

/// Is a queue entry in a DONE state (resolved/rejected/works-as-specified), by its filename tag? The
/// fleet convention tags a closed issue with a dotted `RESOLVED`/`REJECTED` marker anywhere in the name
/// (`foo.RESOLVED.sexp`, `bar.sexp.RESOLVED`, `baz.WORKS-AS-SPECIFIED-diag-RESOLVED.sexp`). Match on the
/// uppercase tag as a dot-delimited component so a substring in ordinary prose can't false-positive.
/// Pure, so the done-routing decision is unit-testable. Checks the file NAME only (subdirs don't matter).
fn entry_is_done(rel: &Path) -> bool {
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.split('.')
        .any(|seg| seg == "RESOLVED" || seg == "REJECTED" || seg.ends_with("-RESOLVED"))
}

/// Mirror the live gitignored work queue into the TRACKED `issues/` archive so the reproducers are
/// preserved in git history, not just agent-local state. The archive is written into the CURRENT
/// worktree (whoever runs this — pr-sync, on `trunk`), which is where the commit lands. The queue
/// itself is read from the hub's `.claude/fleet/queue` (shared). Mirror semantics: copy every queue
/// entry into `issues/` (a RESOLVED/REJECTED-tagged entry → `issues/done/`), delete tracked `issues/`
/// files the mirror no longer writes (git history still holds them), then commit unless `--no-commit`.
fn archive(fleet: &Fleet, no_commit: bool) {
    let queue = fleet.root.join("queue");
    // The archive lives in the CURRENT worktree (cwd), not the hub — the hub is bare (no working
    // tree). pr-sync runs this from its trunk checkout, so the commit is on trunk.
    let cwd = std::env::current_dir().expect("cwd");
    let archive = cwd.join("issues");
    std::fs::create_dir_all(&archive).expect("create issues/ archive dir");

    // Snapshot current queue entry names (recursively, relative to queue/) so we can both copy them
    // in and detect archive files that should be removed.
    let mut queue_rel: Vec<PathBuf> = Vec::new();
    collect_rel(&queue, &queue, &mut queue_rel);

    // Copy every queue file into issues/. A RESOLVED/REJECTED-tagged entry is routed to `issues/done/`
    // (by basename) regardless of its queue subdir, so a closed issue stops listing as assigned; every
    // other entry preserves its queue-relative path under `issues/`. This makes the mirror itself honor
    // the resolved tag rather than reverting a manual `git mv … issues/done/` on the next run (concierge
    // ask: agents were hand-moving resolved issues to done and the mirror kept dragging them back to
    // assigned). The `issues/done/<name>` destination is derived, so a later un-tagging correctly moves
    // it back (the stale-removal pass below deletes the done/ copy once the tag is gone).
    // The DESTINATION relative path each queue entry mirrors to: a RESOLVED/REJECTED-tagged entry goes
    // to `done/<basename>` (so a closed issue stops listing as assigned) regardless of its queue subdir;
    // every other entry keeps its queue-relative path. Computed once and reused for the stale-removal
    // pass below, so removal deletes exactly what the copy DIDN'T write (routing the two consistently —
    // otherwise done/ copies, whose rel-path isn't a queue path, would be wrongly reaped every run).
    let dest_rel = |rel: &Path| -> PathBuf {
        if entry_is_done(rel) {
            Path::new("done").join(rel.file_name().unwrap_or(rel.as_os_str()))
        } else {
            rel.to_path_buf()
        }
    };
    let dest_set: std::collections::HashSet<PathBuf> =
        queue_rel.iter().map(|r| dest_rel(r)).collect();

    let mut copied = 0usize;
    for rel in &queue_rel {
        let src = queue.join(rel);
        let dst = archive.join(dest_rel(rel));
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if std::fs::copy(&src, &dst).is_ok() {
            copied += 1;
        }
    }

    // Remove archive files no longer written by the mirror (resolved-and-migrated, renamed, or a tag
    // that flipped so the destination moved). Compare against the DESTINATION set, not the raw queue
    // set, so a `done/<name>` copy is kept while a stale assigned-path copy of a now-resolved issue is
    // reaped (and vice-versa). Git history preserves anything removed.
    let mut archive_rel: Vec<PathBuf> = Vec::new();
    collect_rel(&archive, &archive, &mut archive_rel);
    let mut removed = 0usize;
    for rel in &archive_rel {
        if !dest_set.contains(rel) && std::fs::remove_file(archive.join(rel)).is_ok() {
            removed += 1;
        }
    }

    println!(
        "fleet archive: mirrored {copied} queue item(s) into issues/ ({removed} stale removed)"
    );

    // Sync the STANDING fleet back into the tracked roster (fleet/roster.json), so a runtime change
    // to the standing agents (e.g. the concierge spun up a new vertical) is persisted to the repo and
    // reproduces on another machine — the roster's analogue of the issues/ mirror. Ephemeral agents
    // (per-issue `fix`, on-demand `design`) and stopped agents are NOT standing, so they are dropped;
    // only the machine-independent fields are written (name/role/vertical/area/interval/model).
    let synced = sync_roster(fleet, &cwd);

    if no_commit {
        println!("  --no-commit: changes left in the working tree, not committed.");
        return;
    }
    // Stage the tracked mirrors and commit if anything changed. Run git in the cwd worktree.
    let run_git = |args: &[&str]| {
        Command::new("git")
            .current_dir(&cwd)
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    run_git(&["add", "issues", "fleet/roster.json"]);
    // Anything staged? `git diff --cached --quiet` exits non-zero if there are staged changes.
    let has_staged = !Command::new("git")
        .current_dir(&cwd)
        .args([
            "diff",
            "--cached",
            "--quiet",
            "--",
            "issues",
            "fleet/roster.json",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(true);
    if !has_staged {
        println!("  nothing changed — issues/ + roster already up to date.");
        return;
    }
    if run_git(&[
        "commit",
        "-m",
        "fleet: mirror the work queue + standing roster into the tracked archive",
    ]) {
        println!("  committed the archive update ({synced} standing agent(s) in roster).");
    } else {
        eprintln!("  ! git commit failed — the change is staged; commit it by hand.");
    }
}

/// Safe base-sync for the tick's "sync your base" step (contract step 4). Encodes the ONE correct way
/// to land on the integrated tip WITHOUT orphaning an in-flight merge-request — a footgun that has cost
/// several agents multiple ticks of silent non-integration (a bare `git reset --hard trunk` moves the
/// agent's branch off the commit its queued `--ref` points at, and pr-sync then can't fetch it and
/// SKIPS the MR forever, with no reject; see the ref-reachability trap).
///
/// The recipe: fetch, then reset onto `trunk`, then cherry-pick back ONLY the local commits that are
/// not yet upstream by PATCH-ID (`git cherry trunk <old-head>` → the `+` lines via `commits_to_replay`).
/// A commit pr-sync already integrated — even under a re-parented sha — has an equivalent patch upstream,
/// so `git cherry` marks it `-` and we DROP it (no empty/duplicate cherry-pick). The remaining `+`
/// commits are genuinely-unlanded local work, replayed so the branch tip still CONTAINS them and any
/// queued `--ref` stays reachable.
///
/// Safety: refuses on a dirty worktree (a reset would clobber uncommitted work — the OTHER half of the
/// footgun); and if any cherry-pick fails (e.g. a real conflict against the advanced trunk), it aborts
/// the pick and RESTORES the pre-sync HEAD, so the worst case is exactly where the agent started —
/// `fleet sync` can never lose or half-apply work. On a conflict it exits non-zero with guidance.
/// A worktree base trailing `trunk` by MORE than this many commits earns a warning on a refused sync —
/// far enough behind that a stale-base conflict/rejection is a real risk when the queued MR is gated
/// (concierge 9740). Short lags are fine (pr-sync rebases the sent ref onto trunk), so the threshold is
/// generous; it flags only a base that has drifted well behind a fast-moving trunk.
const SYNC_BASE_LAG_WARN: usize = 20;

/// Decide whether `sync` should emit the RE-PARENT warning, given whether the pre-sync base is still
/// an ancestor of `trunk` and how many commits `git cherry` marked unlanded.
///
/// Trunk should only ever move FORWARD — pr-sync cherry-picks integrated MRs onto it. But a batch tip
/// is sometimes REBUILT/republished under a fresh sha (or rewound to `origin/main`), and then the
/// agent's PRIOR base (`old_head`, last tick's trunk) DIVERGES from the new trunk: neither is an
/// ancestor of the other. `git cherry trunk old_head` then marks the old batch's now-divergent
/// `integrate MR:` commits as "unlanded" (`+`), so sync is about to replay a stack of FOREIGN
/// re-baked history as if it were the agent's own work — the recurring contamination 5 peers
/// (v-effects/fuzzer/breaker/v-compiler-perf/v-diagnostics) hit every tick, each shedding it by hand
/// with exactly this `git merge-base --is-ancestor <old-base> trunk` check. We warn (never auto-drop —
/// same never-lose-work discipline as the rest of the hot path) iff the base diverged AND there is a
/// non-empty replay set. A clean fast-forward of trunk (old base still an ancestor) is the normal case
/// and stays silent. Pure so the trigger is unit-testable without a git fixture.
fn base_reparent_should_warn(base_is_ancestor_of_trunk: bool, replay_len: usize) -> bool {
    !base_is_ancestor_of_trunk && replay_len > 0
}

/// Roles that NEVER commit authored work to their branch, so they have ZERO unlanded local commits BY
/// DEFINITION — any commit `git cherry trunk HEAD` marks "unlanded" for them is therefore, without
/// ambiguity, a FOREIGN peer commit their base transiently carried (the recurring trunk-rewind
/// contamination 7 peers flagged; reviewer's insight that its role is a 100% false-replay case). For
/// these roles `fleet sync` can safely SKIP the patch-id replay entirely and hard-reset to trunk — no
/// ownership-guessing, no cherry-pick conflict, because the ROLE itself is the proof there is nothing of
/// theirs to preserve. Sound counterpart to the RE-PARENT WARNING (which surfaces the divergence for
/// AUTHORING roles that must eyeball).
///
/// ⚠ CORRECTNESS: this list gates a DESTRUCTIVE `reset --hard` (drops the branch's commits with no
/// replay), so a role listed here that ACTUALLY authors would lose unlanded work. So membership is NOT
/// the loop-body's "never send merge-request(s)" TEXT (which can go stale): breaker's loop still says
/// that, but per the 2026-07-15 operator directive breaker PROMOTES passing probes as corpus
/// merge-requests every tick (35 landed on trunk) — listing it here would have silently discarded an
/// unlanded corpus pin (regression caught by breaker on the first landing). Membership is the roles that
/// EMPIRICALLY author zero MRs: pure coordination/routing/maintenance roles. Verify with
/// `git log trunk | grep 'integrate MR:.*(<role>)'` before adding one. Any authoring role (`vertical`,
/// `fix`, `breaker`, `corpus-bugfix`'s spawned fixers, …) is never listed → normal replay path. Pure so
/// the policy is unit-testable.
fn role_never_authors(role: &str) -> bool {
    matches!(
        role,
        "reviewer" | "tracker" | "librarian" | "concierge" | "github-liaison" | "pr-sync"
    )
}

/// Whether a `git reset --hard trunk` on the syncing branch would be CONTENT-LOSSLESS: true iff HEAD's
/// tree-hash equals trunk's tree-hash (both resolved, non-empty). This is the CONTENT-derived counterpart
/// to [`role_never_authors`]'s role-derived proof, and it closes the gap that path leaves for AUTHORING
/// verticals: when trunk is rebuilt/rewound under FRESH shas (a re-parent), an authoring branch with NO
/// own unlanded work still carries the OLD `integrate MR:` commits, whose fresh-sha + differing patch-id
/// make `git cherry` mark them `+`/"unlanded" — so `sync` would fall through to replay+warn a stack of
/// FOREIGN re-baked history as the agent's own work (the recurring toil v-diagnostics, v-property-testing,
/// v-memory-safety, and v-fleet-tooling each hit, shedding it by hand). But if HEAD's TREE is byte-
/// identical to trunk's, the branch has ZERO unlanded CONTENT by definition — any own commit's content is
/// already on trunk (that equality is exactly what makes it landed) — so resetting drops only orphaned
/// commit OBJECTS whose content is fully preserved on trunk. A branch with genuine own unlanded content
/// has a DIFFERENT tree, so this returns false and the normal replay+warn path runs untouched (we NEVER
/// auto-drop real work). Pure so the trigger is unit-testable without a git fixture.
fn tree_equal_reset_is_lossless(head_tree: &str, trunk_tree: &str) -> bool {
    !head_tree.is_empty() && !trunk_tree.is_empty() && head_tree == trunk_tree
}

fn sync(fleet: &Fleet, force: bool) {
    let cwd = std::env::current_dir().expect("cwd");
    let git = |args: &[&str]| -> std::process::Output {
        Command::new("git")
            .current_dir(&cwd)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"))
    };
    let git_ok = |args: &[&str]| -> bool { git(args).status.success() };
    let git_stdout = |args: &[&str]| -> String {
        String::from_utf8_lossy(&git(args).stdout)
            .trim()
            .to_string()
    };

    // Refuse on a dirty tree — a reset --hard would silently discard uncommitted work. (The tick
    // contract expects the agent to commit before syncing; guard it rather than trust that.)
    if !git_ok(&["diff", "--quiet"]) || !git_ok(&["diff", "--cached", "--quiet"]) {
        eprintln!(
            "fleet sync: worktree is DIRTY — refusing to reset (it would discard uncommitted work). \
             Commit or stash your changes first, then re-run `cargo xtask fleet sync`."
        );
        std::process::exit(1);
    }

    let old_head = git_stdout(&["rev-parse", "HEAD"]);
    if old_head.is_empty() {
        eprintln!("fleet sync: could not resolve HEAD.");
        std::process::exit(1);
    }

    // Bring trunk current (the hub shares the object store, but `fetch` refreshes origin for the
    // ahead/behind reporting and is harmless if there's nothing new).
    let _ = git_ok(&["fetch", "-q", "origin"]);

    // NO-OP GUARD (the re-sha churn v-inference hit): if `trunk` is ALREADY an ancestor of HEAD, the
    // branch fully contains trunk with any local commits cleanly on top — there is nothing to rebase
    // ONTO, so a reset+cherry-pick would only re-parent those commits and give them FRESH shas (new
    // committer-dates) for no reason, orphaning every queued merge-request `--ref` on EACH sync even
    // though trunk never advanced. Skip entirely and leave the branch byte-identical. When trunk HAS
    // advanced (not an ancestor of HEAD), fall through to the real reset+replay below. `--is-ancestor`
    // exits 0 iff TRUNK is an ancestor of HEAD; treat a spawn failure as "advanced" (do the safe replay).
    if git_ok(&["merge-base", "--is-ancestor", TRUNK, &old_head]) {
        let trunk_sha = git_stdout(&["rev-parse", "--short", TRUNK]);
        let ahead = git_stdout(&["rev-list", "--count", &format!("{TRUNK}..{old_head}")]);
        let n: usize = ahead.parse().unwrap_or(0);
        if n == 0 {
            println!("fleet sync: on trunk ({trunk_sha}); already current, nothing to replay.");
        } else {
            println!(
                "fleet sync: already on trunk ({trunk_sha}) with {n} local commit(s) on top — trunk \
                 has not advanced, so nothing to rebase; leaving the branch UNCHANGED (no re-sha, so \
                 any queued merge-request --ref stays valid)."
            );
        }
        return;
    }

    // NEVER-AUTHORING ROLE FAST-PATH (reviewer's insight; the 100%-false-replay case). A role whose loop
    // never sends a merge-request authors NOTHING on its branch, so it has ZERO unlanded commits BY
    // DEFINITION — every commit `git cherry` would mark "unlanded" for it is a FOREIGN peer commit its
    // base transiently carried (the recurring trunk-rewind contamination). For such a role there is
    // nothing to preserve, so skip the patch-id replay entirely and hard-reset to trunk: this eliminates
    // BOTH the false replay AND the cherry-pick conflict that the replay path hits on a rewound trunk,
    // with zero false-positive risk (the ROLE is the proof, not a path/ownership heuristic). Authoring
    // roles fall through to the normal replay below. Resolve role from the branch → roster; if we can't
    // (unknown branch/role), take the safe normal path.
    let branch_for_role = git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let my_role = sender_from_branch_name(&branch_for_role).and_then(|me| {
        fleet
            .load_roster()
            .agents
            .into_iter()
            .find(|a| a.name == me)
            .map(|a| a.role)
    });
    if let Some(role) = my_role.as_deref()
        && role_never_authors(role)
    {
        if !git_ok(&["reset", "--hard", TRUNK]) {
            eprintln!("fleet sync: `git reset --hard {TRUNK}` failed.");
            std::process::exit(1);
        }
        let trunk_sha = git_stdout(&["rev-parse", "--short", "HEAD"]);
        println!(
            "fleet sync: role '{role}' never authors merge-requests → reset to trunk ({trunk_sha}) \
             WITHOUT patch-id replay (nothing of yours to preserve; any 'unlanded' commit would be a \
             peer's your base transiently carried). Branch is now clean at trunk."
        );
        return;
    }

    // CONTENT-IDENTICAL FAST-PATH (the re-parent contamination v-diagnostics, v-property-testing,
    // v-memory-safety, and v-fleet-tooling each hit — corroborated 4×). We are here only because trunk
    // is NOT an ancestor of HEAD (the no-op guard above returned otherwise) — i.e. trunk DIVERGED, was
    // rebuilt/rewound under fresh shas. An AUTHORING vertical with no own unlanded work still carries the
    // OLD `integrate MR:` commits at their stale shas; their fresh-sha + differing patch-id make `git
    // cherry` mark them `+`, so the normal path below would replay+warn a stack of FOREIGN re-baked
    // history as the agent's own work (manual `reset --hard trunk` toil, and in my own case a REJECTED
    // in-lane MR whose ancestry carried a peer's rejected commit). The role-derived never-authors path
    // can't help an authoring vertical; the CONTENT is the proof instead: if HEAD's TREE is byte-
    // identical to trunk's, the branch has ZERO unlanded content — every own commit's content is already
    // on trunk (that equality is what makes it landed) — so `reset --hard trunk` is provably lossless,
    // dropping only orphaned commit OBJECTS whose content is fully preserved upstream. A branch with
    // genuine own unlanded content has a DIFFERENT tree, so this stays silent and the replay+warn path
    // runs untouched (real work is never auto-dropped). No `--ref` orphaning risk: we don't cherry-pick
    // (no re-sha), and a tree-equal branch adds nothing over trunk for a queued MR to lose.
    let head_tree = git_stdout(&["rev-parse", &format!("{old_head}^{{tree}}")]);
    let trunk_tree = git_stdout(&["rev-parse", &format!("{TRUNK}^{{tree}}")]);
    if tree_equal_reset_is_lossless(&head_tree, &trunk_tree) {
        if !git_ok(&["reset", "--hard", TRUNK]) {
            eprintln!("fleet sync: `git reset --hard {TRUNK}` failed.");
            std::process::exit(1);
        }
        let trunk_sha = git_stdout(&["rev-parse", "--short", "HEAD"]);
        println!(
            "fleet sync: your branch is CONTENT-IDENTICAL to trunk (same tree) but trunk was re-parented \
             under fresh shas → reset to trunk ({trunk_sha}) WITHOUT patch-id replay. Any 'unlanded' \
             commit `git cherry` would flag is FOREIGN re-baked history your base carried, not yours — \
             its content is already on trunk, so nothing is lost. Branch is now clean at trunk."
        );
        return;
    }

    // Which local commits are genuinely unlanded (patch-id not upstream)? `git cherry trunk <head>`.
    let cherry = git_stdout(&["cherry", TRUNK, &old_head]);
    let replay = commits_to_replay(&cherry);

    // RE-PARENT WARNING (breaker's angle b; the trunk-rewind contamination 5 peers hit every tick).
    // If the pre-sync base (`old_head`) is NO LONGER an ancestor of `trunk`, trunk was rebuilt/rewound
    // under a fresh sha — the base DIVERGED, so `git cherry` marks the old batch's now-orphaned
    // `integrate MR:` commits as "unlanded" and we are about to replay a stack of FOREIGN re-baked
    // history as the agent's own work. This is the ROOT signal that explains a large foreign replay set
    // (the per-commit zone warnings below catch individual commits; this names the CAUSE up front). We
    // don't drop anything (never-lose-work), but tell the agent the divergence is real so it lane-checks
    // + `reset --hard trunk` instead of trusting/`fleet send`ing the stack. `--is-ancestor` exits 0 iff
    // old_head is an ancestor of trunk; a spawn failure is treated as "still ancestor" (no false alarm).
    let base_still_ancestor = git_ok(&["merge-base", "--is-ancestor", &old_head, TRUNK]);
    if base_reparent_should_warn(base_still_ancestor, replay.len()) {
        eprintln!(
            "fleet sync: ⚠ trunk was RE-PARENTED — your pre-sync base ({}) is no longer an ancestor of \
             trunk (a batch tip was rebuilt/rewound under a fresh sha). That divergence is why `git \
             cherry` now marks {} commit(s) as 'unlanded': they are almost certainly FOREIGN re-baked \
             `integrate MR:` history from the old batch, NOT your work. Eyeball the replayed titles \
             below; if you have no own unlanded commits, `git reset --hard {TRUNK}` to shed the whole \
             stack (safe with no WIP), and NEVER `fleet send` any of them.",
            &old_head[..old_head.len().min(9)],
            replay.len()
        );
    }

    // REJECT-LIST GUARD (pr-sync's idea, 2026-07-20): a commit pr-sync has REJECTED (gate-failing) can
    // ride a discarded test-merge tree onto peer branches and get re-replayed every tick — 6 MRs bounced
    // across 2 ticks for carrying one rejected rust-async commit. Unlike the general "foreign but
    // maybe-good" problem, a reject is an EXPLICIT authoritative signal, so we can name it with zero
    // false-positive risk. WARN loudly (not auto-drop — same never-lose-work discipline as the hot path;
    // resetting is the agent's call) for any to-be-replayed commit whose sha is on the durable
    // reject-log. Full-sha match, so a FIXED version (new sha) is never wrongly flagged.
    if !replay.is_empty() {
        let rejected = read_reject_log(&fleet.reject_log_path());
        let hits: Vec<&String> = replay
            .iter()
            .filter(|s| sha_is_rejected(s, &rejected))
            .collect();
        if !hits.is_empty() {
            eprintln!(
                "fleet sync: ⚠ {} of your {} unlanded commit(s) are on pr-sync's REJECT-LIST — \
                 gate-failing commits pr-sync already bounced, riding your base after a discarded \
                 test-merge. Replaying + resending them just re-breaks the gate:",
                hits.len(),
                replay.len()
            );
            for sha in &hits {
                let title = git_stdout(&["show", "-s", "--format=%s", sha]);
                eprintln!("    {} {title}", &sha[..sha.len().min(9)]);
            }
            eprintln!(
                "  These are NOT yours to land. `git reset --hard {TRUNK}` to shed them (safe with no \
                 own WIP), or cherry-pick back ONLY your own commits. NEVER `fleet send` a reject-listed \
                 commit — pr-sync will just reject it again."
            );
        }
    }

    // GUARD: refuse if replaying would re-sha a commit a merge-request we already sent still points at.
    // The cherry-pick below gives replayed commits NEW shas, so the `--ref` on a queued MR would go
    // dangling and pr-sync would silently skip it. Derive our agent name from the current branch
    // (`fleet/<me>`) and scan pr-sync's inbox for a still-queued MR from us naming a to-be-replayed sha.
    if !force && !replay.is_empty() {
        let branch = git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"]);
        if let Some(me) = sender_from_branch_name(&branch) {
            let queued = read_messages(&fleet.inbox("pr-sync"));
            if let Some(orphaned) = queued_ref_would_orphan(&replay, &me, &queued) {
                eprintln!(
                    "fleet sync: REFUSING — a merge-request you already sent pr-sync names commit \
                     {orphaned}, which this sync would re-sha (cherry-pick → new sha), orphaning that \
                     queued --ref so pr-sync silently skips it. Leave your branch alone until that MR \
                     lands or is rejected (being behind trunk is fine — pr-sync merges your sent ref \
                     onto current trunk and re-gates). If that MR is dead and you'll resend a fresh \
                     --ref, re-run with `--force`."
                );
                // BASE-LAG WARNING (concierge 9740): the refuse leaves the branch on its current base.
                // That's fine for a short lag (pr-sync rebases the sent ref onto trunk + re-gates), but a
                // base that trails trunk by MANY commits risks a stale-base conflict/rejection when the
                // queued MR is finally judged. Surface how far behind the base is so the agent notices
                // and can plan a `--force` resync (after its MR resolves) rather than drifting silently.
                let behind = git_stdout(&["rev-list", "--count", &format!("{old_head}..{TRUNK}")]);
                if behind.parse::<usize>().unwrap_or(0) > SYNC_BASE_LAG_WARN {
                    eprintln!(
                        "  ⚠ your base is {behind} commits behind trunk — a lag this large risks a \
                         stale-base conflict when your queued MR is gated. Once it lands/rejects, \
                         `cargo xtask fleet sync --force` to rebase onto current trunk before new work."
                    );
                }
                std::process::exit(1);
            }
        }
    }

    // FOREIGN-COMMIT WARNING (non-destructive; the coordination-integrity report from fuzzer, v-des,
    // v-diagnostics, v-compiler-perf, github-liaison). `git cherry` is patch-id-based but the replay set
    // it feeds us is only as clean as `old_head`: if the agent's base transiently carried a PEER's
    // unlanded WIP (the shared `trunk` ref sat on a pr-sync TEST-integrate tip, or was reset to a stale
    // sha, during a sync/reset window), that peer commit is genuinely not-upstream → marked `+` → about
    // to be replayed onto this branch. We do NOT drop it (misjudged ownership on this reset+replay
    // hot-path could lose real work) — we NAME the suspected-foreign commit + its out-of-lane paths so
    // the agent never accidentally `fleet send`s a peer's commit as its own MR. Signal = CHANGED-PATHS
    // vs the agent's roster ZONE (sound, unlike the reachability heuristic that false-flagged shared-base
    // own-commits). Conservative: only whole-commit unambiguous cross-zone mismatches warn. Skipped
    // silently if we can't resolve our agent name or its zone.
    if !replay.is_empty() {
        let branch = git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"]);
        let my_zone = sender_from_branch_name(&branch)
            .and_then(|me| {
                fleet
                    .load_roster()
                    .agents
                    .into_iter()
                    .find(|a| a.name == me)
                    .map(|a| a.area)
            })
            .unwrap_or_default();
        if !my_zone.is_empty() {
            let foreign: Vec<(String, Vec<String>)> = replay
                .iter()
                .filter_map(|sha| {
                    let paths: Vec<String> = git_stdout(&["show", "--name-only", "--format=", sha])
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect();
                    commit_is_foreign_to_lane(&paths, &my_zone).then_some((sha.clone(), paths))
                })
                .collect();
            if !foreign.is_empty() {
                eprintln!(
                    "fleet sync: ⚠ {} of your {} unlanded commit(s) touch ONLY files outside your lane \
                     (zone '{my_zone}') — likely a PEER's in-flight work your base transiently carried, \
                     NOT yours:",
                    foreign.len(),
                    replay.len()
                );
                for (sha, paths) in &foreign {
                    let shown: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
                    let more = if paths.len() > 3 {
                        format!(" (+{} more)", paths.len() - 3)
                    } else {
                        String::new()
                    };
                    eprintln!(
                        "    {} → {}{more}",
                        &sha[..sha.len().min(9)],
                        shown.join(", ")
                    );
                }
                eprintln!(
                    "  This sync will still replay them (never dropped, so nothing is lost), but do NOT \
                     `fleet send` these to pr-sync — that would hijack the peer's MR. If they aren't \
                     yours, `git reset --hard {TRUNK}` after this sync to shed them (safe when you have \
                     no own WIP), or cherry-pick back ONLY your own commits before sending."
                );
            }

            // SOFT tier (option 2): commits the loud test left silent because they touch an
            // UNATTRIBUTABLE path (e.g. `implementation/seed/crates/rcdzc/…`) yet have NO path in my
            // lane and at least one provably-other-zone path. This is the systemic case that slipped
            // through unwarned (a rejected rust-async resend replayed onto ≥6 branches, all touching
            // rcdzc/… + xtask/…). Flag it as "ambiguous — verify" so the agent lane-checks before it
            // bites, without the false-positive risk of dropping a genuinely-shared edit. Exclude any
            // already reported LOUD above.
            let loud: std::collections::HashSet<&str> =
                foreign.iter().map(|(s, _)| s.as_str()).collect();
            let ambiguous: Vec<(String, Vec<String>)> = replay
                .iter()
                .filter(|sha| !loud.contains(sha.as_str()))
                .filter_map(|sha| {
                    let paths: Vec<String> = git_stdout(&["show", "--name-only", "--format=", sha])
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect();
                    commit_is_ambiguous_to_lane(&paths, &my_zone).then_some((sha.clone(), paths))
                })
                .collect();
            if !ambiguous.is_empty() {
                eprintln!(
                    "fleet sync: ⚠ {} of your {} unlanded commit(s) touch NO file in your lane (zone \
                     '{my_zone}') but mix a peer's zone with shared/unattributable paths — AMBIGUOUS, \
                     possibly a peer's work your base carried. VERIFY before trusting them as yours:",
                    ambiguous.len(),
                    replay.len()
                );
                for (sha, paths) in &ambiguous {
                    let shown: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
                    let more = if paths.len() > 3 {
                        format!(" (+{} more)", paths.len() - 3)
                    } else {
                        String::new()
                    };
                    eprintln!(
                        "    {} → {}{more}",
                        &sha[..sha.len().min(9)],
                        shown.join(", ")
                    );
                }
                eprintln!(
                    "  Eyeball each title/diff: if it isn't yours, `git reset --hard {TRUNK}` (safe with \
                     no own WIP) and never `fleet send` it. (Replayed regardless — nothing lost.)"
                );
            }
        }
    }

    // Land on the integrated tip.
    if !git_ok(&["reset", "--hard", TRUNK]) {
        eprintln!("fleet sync: `git reset --hard {TRUNK}` failed.");
        std::process::exit(1);
    }
    let trunk_sha = git_stdout(&["rev-parse", "--short", "HEAD"]);

    if replay.is_empty() {
        println!("fleet sync: on trunk ({trunk_sha}); no unlanded local commits to replay.");
        return;
    }

    // Replay only the not-yet-upstream commits, oldest-first, so the branch tip contains them again
    // (keeping any queued merge-request --ref reachable). On ANY failure, restore the pre-sync HEAD.
    for sha in &replay {
        if !git_ok(&["cherry-pick", sha]) {
            let _ = git_ok(&["cherry-pick", "--abort"]);
            let _ = git_ok(&["reset", "--hard", &old_head]);
            eprintln!(
                "fleet sync: cherry-pick of {sha} onto trunk ({trunk_sha}) FAILED (likely a real \
                 conflict with the advanced trunk). Restored your pre-sync HEAD ({}) — nothing lost. \
                 Resolve the conflict by hand: `git reset --hard {TRUNK}` then cherry-pick + fix.",
                &old_head[..old_head.len().min(9)]
            );
            std::process::exit(1);
        }
    }
    let new_head = git_stdout(&["rev-parse", "--short", "HEAD"]);
    println!(
        "fleet sync: reset onto trunk ({trunk_sha}) and replayed {} unlanded commit(s) → {new_head}. \
         (Dropped any already-landed commit by patch-id, incl. re-parented merges.)",
        replay.len()
    );
    // ALWAYS itemize WHAT was replayed (sha + title), so the agent can eyeball each for a foreign title
    // WITHOUT a manual `git log --stat`. The zone-based foreign-commit WARNING above only fires on an
    // unambiguous whole-commit cross-zone mismatch and stays SILENT when any path is unattributable
    // (e.g. a peer's rcdzc/seed commit riding a shared base — the recurring contamination this session
    // hit on ≥6 branches). This unconditional listing closes that visibility gap: a title like
    // "rcdzc: emit … on the rust-async backend" on a v-syntax branch is instantly recognizable as not
    // mine → `git reset --hard trunk`. Purely informational (no drop, no false-positive risk).
    for sha in &replay {
        let title = git_stdout(&["show", "-s", "--format=%s", sha]);
        println!("    replayed {} {title}", &sha[..sha.len().min(9)]);
    }
    eprintln!(
        "  ↳ eyeball each replayed title above: any NOT in your lane's voice is a peer commit your base \
         transiently carried — if it's foreign and you have no own unlanded work, `git reset --hard \
         {TRUNK}` to shed it, and never `fleet send` it."
    );
}

/// Write the STANDING agents from the live runtime registry into the tracked `fleet/roster.json`
/// (in `cwd`, the caller's worktree). Standing = active + a role that belongs in the reproducible
/// fleet (everything except the ephemeral `fix`/`design` roles). Only machine-independent fields are
/// written; runtime state (worktree path, status, window) is re-derived by `up`. Returns the count.
/// Render ONE agent as its tracked-roster JSON line, or `None` if it should NOT be persisted.
///
/// The tracked `roster.json` is the STANDING fleet that reproduces on any machine, so only agents
/// worth reviving belong in it: an agent that is `active` AND whose role is not ephemeral. `fix` and
/// `design` are ephemeral (a fix agent is minted per bug and removed when done; design is interactive
/// and hand-started), so persisting them would make `fleet up` try to recreate dead/transient agents.
/// Optional fields (`vertical`/`area`) are omitted when empty so the tracked file stays minimal +
/// diff-stable. Pure (no I/O) so the exclusion + field-omission rules are unit-testable.
fn roster_entry_json(a: &Agent) -> Option<String> {
    if a.status != "active" || matches!(a.role.as_str(), "fix" | "design") {
        return None;
    }
    let mut s = String::from("    { ");
    s.push_str(&format!("\"name\": {:?}, \"role\": {:?}", a.name, a.role));
    if !a.vertical.is_empty() {
        s.push_str(&format!(", \"vertical\": {:?}", a.vertical));
    }
    if !a.area.is_empty() {
        s.push_str(&format!(", \"area\": {:?}", a.area));
    }
    s.push_str(&format!(
        ", \"model\": {:?}, \"effort\": {:?}, \"interval\": {:?} }}",
        a.model, a.effort, a.interval
    ));
    Some(s)
}

fn sync_roster(fleet: &Fleet, cwd: &Path) -> usize {
    let reg = fleet.load();
    let mut entries = String::new();
    let mut n = 0usize;
    for a in &reg.agents {
        let Some(line) = roster_entry_json(a) else {
            continue;
        };
        if n > 0 {
            entries.push_str(",\n");
        }
        entries.push_str(&line);
        n += 1;
    }
    let doc = format!(
        "{{\n  \"//\": \"The STANDING fleet — tracked so it reproduces on any machine. \
         Synced from the live registry by `cargo xtask fleet archive` (pr-sync runs it each tick) \
         and read back by `fleet up`. Ephemeral fix/design agents are not listed. Edit by hand or \
         via `fleet add/remove`; the next archive persists the change.\",\n\
         \"agents\": [\n{entries}\n  ]\n}}\n"
    );
    let dst = cwd.join("fleet/roster.json");
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&dst, doc).ok();
    let _ = fleet; // fleet only used for load(); silence if inlined
    n
}

/// Recursively collect file paths under `dir`, as paths RELATIVE to `base`. Skips nothing except
/// directories themselves (files only). Used to mirror the queue ↔ archive.
fn collect_rel(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        if path.is_dir() {
            collect_rel(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
}

// ── worktree / inbox / tmux helpers ───────────────────────────────────────────────────────────

/// Ensure the agent's git worktree exists, creating it off `trunk` if missing. pr-sync's worktree
/// checks out `trunk` itself; every other agent gets its own topic branch. Idempotent.
fn ensure_worktree(fleet: &Fleet, a: &Agent) {
    let wt = Path::new(&a.worktree);
    if wt.is_dir() {
        return;
    }
    std::fs::create_dir_all(&fleet.worktrees).ok();
    let mut cmd = Command::new("git");
    cmd.current_dir(&fleet.repo).arg("worktree").arg("add");
    if a.branch == TRUNK {
        // pr-sync: check out the existing trunk branch (no new branch).
        cmd.arg(wt).arg(TRUNK);
    } else {
        // A fresh topic branch off trunk for this agent.
        cmd.arg("-b").arg(&a.branch).arg(wt).arg(TRUNK);
    }
    match cmd.status() {
        Ok(s) if s.success() => println!("  + worktree {} [{}]", a.worktree, a.branch),
        Ok(_) => eprintln!(
            "  ! failed to create worktree {} (branch {} may already be checked out elsewhere)",
            a.worktree, a.branch
        ),
        Err(e) => eprintln!("  ! could not run git worktree add: {e}"),
    }
}

fn ensure_inbox(fleet: &Fleet, name: &str) {
    let inbox = fleet.inbox(name);
    std::fs::create_dir_all(inbox.join("processed")).ok();
}

/// A DURABLE, hub-global delivery sequence for message filenames — monotonic ACROSS processes.
///
/// The message-bus filename is `<seq>-<pid>-<kind>.json` and the drain reads oldest-first by filename
/// sort, so the leading field must increase with send order. `Message::seq` (from `next_seq`) can't do
/// that: it's process-local and each `fleet send` is a one-shot process, so every message would be
/// `000000000001-…` and the inbox would sort by PID (only accidentally monotonic, and it recycles).
/// So the FILENAME uses this counter instead: read `<root>/.delivery-seq`, increment, write back.
/// Best-effort atomic (temp+rename); a concurrent-send race can at worst hand two messages the same
/// number — the `pid` field then breaks the tie and they sort adjacently, never lost/corrupted. The
/// stored `Message::seq` is left as-is (metadata only; reply pairing keys on the filename, not seq).
fn next_delivery_seq(fleet: &Fleet) -> u64 {
    let path = fleet.root.join(".delivery-seq");
    let cur = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = cur.saturating_add(1);
    // temp+rename so a concurrent reader never sees a torn write. The temp name must be UNIQUE per
    // writer (PR #453): many agents run `fleet send` at once, and a FIXED `.delivery-seq.tmp` would let
    // two processes open/truncate/write the SAME temp concurrently → a torn/clobbered write or a
    // half-file surviving the rename. pid + a process-local counter makes it distinct across processes
    // AND across a single process's concurrent calls. (The read-modify-write of the counter itself is
    // still not locked, so two racers can compute the same `next` — a benign DUPLICATE that the `<pid>`
    // field in the message filename disambiguates; what this prevents is the far worse CORRUPTION of the
    // counter file from a shared temp.)
    let tmp = fleet.root.join(format!(
        ".delivery-seq.{}.{}.tmp",
        std::process::id(),
        next_seq()
    ));
    if std::fs::write(&tmp, format!("{next}\n")).is_ok() {
        // rename is atomic on the same filesystem; a losing racer's rename simply wins-last with an
        // equal-or-higher value, never a partial file. If the rename itself FAILS (transient perm/IO),
        // clean up the temp so it doesn't linger (PR #457) — the write succeeded but the swap didn't.
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        // Best-effort cleanup if the write half-failed, so a stray temp doesn't linger.
        let _ = std::fs::remove_file(&tmp);
    }
    next
}

/// Deliver a message: write it to a temp file then rename into the recipient's inbox, so a reader
/// never observes a partial file. The filename sorts in send order via a durable delivery sequence
/// (`<delivery-seq>-<pid>-<kind>.json`) — see `next_delivery_seq`.
fn deliver(fleet: &Fleet, msg: &Message) {
    // Rescue a reply addressed to `unknown`: pr-sync (and others) write the recipient into the SUBJECT
    // as `<kind>: fleet/<agent>` even when the incoming `to`/`from` was `unknown` (the send-side
    // identity bug — now fixed, but ~160 replies already dead-lettered into inbox/unknown/, which
    // nobody drains, so senders never saw their merged/reject). At the single delivery chokepoint,
    // if `to == "unknown"` and the subject names a `fleet/<agent>`, route there instead. This fixes
    // future stragglers structurally regardless of the caller's routing logic.
    let to: String = if msg.to == "unknown" {
        match recipient_from_subject(&msg.subject) {
            Some(real) => {
                eprintln!(
                    "fleet deliver: rescued a `to=unknown` message → routing to '{real}' (from subject {:?})",
                    msg.subject
                );
                real
            }
            None => msg.to.clone(),
        }
    } else {
        msg.to.clone()
    };
    // Path-traversal guard: the recipient name becomes a path component (`inbox/<to>`). An unchecked
    // name like `..` / `../../x` would write OUTSIDE the inbox tree. Names are our own controlled
    // identifiers (registry rows / CLI), but a bridge that forwards an external sender's name (the
    // Slack bridge) makes this a real write primitive — so validate at the single write chokepoint.
    if let Err(why) = validate_agent_name(&to) {
        eprintln!("fleet: refusing to deliver to invalid agent name {to:?}: {why}");
        std::process::exit(1);
    }
    let inbox = fleet.inbox(&to);
    std::fs::create_dir_all(&inbox).expect("create recipient inbox");
    // Durable hub-global sequence (NOT msg.seq, which is process-local → always 1 for a one-shot send)
    // so the filename actually sorts in delivery order for the oldest-first drain. See next_delivery_seq.
    let fname = format!(
        "{:012}-{}-{}.json",
        next_delivery_seq(fleet),
        std::process::id(),
        msg.kind
    );
    let json = serde_json::to_string_pretty(msg).expect("serialize message");
    let tmp = inbox.join(format!(".{fname}.tmp"));
    std::fs::write(&tmp, json).expect("write message tmp");
    std::fs::rename(&tmp, inbox.join(&fname)).expect("rename message into inbox");
}

/// Validate an agent name that will become a filesystem path component (`inbox/<name>`,
/// `stop/<name>`, `heartbeat/<name>`, a tmux window target). Enforces `^[A-Za-z0-9][A-Za-z0-9-]*$`:
/// a leading ASCII alphanumeric, then alphanumerics and hyphens only. This is deliberately IDENTICAL
/// to the Slack bridge's sink validation (`v-slack-bridge`, PR#391) so the two boundaries agree — a
/// dot is exactly what enables the `..` traversal and NO real agent name uses a dot or underscore
/// (checked the whole roster: `pr-sync`, `corpus-bugfix`, `v-*`, `fix-*`, `github-liaison`, …), so
/// banning them outright is safe and simpler than allowing dots-minus-`..`. This is the Rust-boundary
/// suspenders to the bridge's JS-boundary belt: no path separator, no `.`/`..`, no dotfile/flag lookalike.
fn validate_agent_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("empty");
    }
    if name.len() > 128 {
        return Err("too long");
    }
    // Must start with an ASCII alphanumeric (no leading `-` flag-lookalike; and since dots are
    // disallowed entirely, no `.`/`..`/dotfile can even form).
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err("must start with an ASCII letter or digit");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(
            "only ASCII alphanumerics and `-` are allowed (no dots/underscores/separators)",
        );
    }
    Ok(())
}

/// Is `s` a single, safe path component — one that stays inside its parent directory when joined?
/// Rejects a path separator (`/` or `\`), a `.`/`..` component, an embedded `..`, an empty string, and
/// a NUL. Unlike [`validate_agent_name`] this is charset-PERMISSIVE (an inbox basename legitimately has
/// digits, dots and dashes, e.g. `000000000001-42-merge-request.json`); it guards only against
/// TRAVERSAL, not against arbitrary characters. Used by `fleet ack` for the inbox-basename branch.
fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
        && !s.contains("..")
}

/// A process-local counter for the `Message::seq` metadata field. NOT used for inbox ordering —
/// that's `next_delivery_seq`, a durable cross-process counter, because a one-shot `fleet send` process
/// only ever calls this once (so it's always 1). Kept because `seq` is part of the on-disk message
/// format; a multi-message process (e.g. `reroute-unknown`) still gets a distinct value per message.
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Derive the sending agent's name from the current worktree's git branch, which is `fleet/<agent>`
/// for every non-pr-sync agent (pr-sync's is `trunk`). Returns `<agent>` for a `fleet/…` branch, or
/// `pr-sync` when on `trunk` (that's pr-sync's worktree), else `None`. The worktree root is the parent
/// of `fleet.src` (`<worktree>/fleet`). This is the `--from` fallback so a forgotten flag doesn't
/// produce `from=unknown`.
fn sender_from_branch(fleet: &Fleet) -> Option<String> {
    let worktree = fleet.src.parent()?;
    let out = Command::new("git")
        .current_dir(worktree)
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8(out.stdout).ok()?;
    sender_from_branch_name(branch.trim())
}

/// The PURE branch-name → sender-agent mapping (kept separate from `sender_from_branch`'s git shell so
/// it's unit-testable). An agent's worktree is on `fleet/<agent>`, so its branch names the sender;
/// pr-sync runs on `trunk`. A branch that is neither → `None` (the caller then falls back to `unknown`,
/// which dead-letters replies — the reply-graveyard trap this derivation exists to avoid), and the
/// derived `<agent>` must pass the name charset so we never resolve a sender to a garbage/traversal
/// token. The empty string (detached HEAD → `git branch --show-current` prints nothing) is `None`.
fn sender_from_branch_name(branch: &str) -> Option<String> {
    if branch.is_empty() {
        None
    } else if let Some(agent) = branch.strip_prefix("fleet/") {
        // A `fleet/<garbage>` branch must not resolve to a garbage sender.
        validate_agent_name(agent).ok().map(|()| agent.to_string())
    } else if branch == TRUNK {
        Some("pr-sync".to_string())
    } else {
        None
    }
}

/// Derive the intended recipient from a reply's subject. pr-sync writes `<kind>: fleet/<agent>` (e.g.
/// `merged: fleet/v-diagnostics`, `reject: fleet/breaker`), so a reply that got addressed to `unknown`
/// still names its real recipient there. Returns `<agent>` if the subject contains a `fleet/<agent>`
/// token whose `<agent>` is a valid agent name, else `None`. Used by `deliver` (rescue a live
/// `to=unknown`) and `reroute-unknown` (reconcile the graveyard).
fn recipient_from_subject(subject: &str) -> Option<String> {
    // Find a `fleet/<agent>` token anywhere in the subject; take the chars up to the first whitespace
    // or end. `<agent>` must pass the agent-name charset (so we never route to a garbage path).
    let idx = subject.find("fleet/")?;
    let rest = &subject[idx + "fleet/".len()..];
    let agent: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if agent.is_empty() || validate_agent_name(&agent).is_err() {
        None
    } else {
        Some(agent)
    }
}

/// The hub (main) repo root as an ABSOLUTE path, resolved from any worktree via the shared common
/// git dir: `git -C <dir> rev-parse --path-format=absolute --git-common-dir` yields `<hub>/.git`
/// (the ONE object store all worktrees share), whose parent is the hub root. `None` if `dir` is not
/// in a git repo. Works identically before and after the bare conversion (the `.git` never moves).
fn hub_root(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let common = String::from_utf8(out.stdout).ok()?;
    let common = PathBuf::from(common.trim());
    // `<hub>/.git` → `<hub>`. (A bare hub's common dir is `<hub>/.git` too, since we keep `.git` in
    // place and only flip core.bare + strip the working tree.)
    common.parent().map(Path::to_path_buf)
}

fn in_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

fn tmux_current_session() -> String {
    Command::new("tmux")
        .args(["display-message", "-p", "#S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".to_string())
}

/// The window names live in `session`.
fn tmux_windows(session: &str) -> Vec<String> {
    tmux_windows_checked(session).unwrap_or_default()
}

/// List the window names in `session`, distinguishing a genuine EMPTY list from a tmux invocation
/// FAILURE: `Some(vec)` on a successful `tmux list-windows` (possibly empty), `None` if tmux could not
/// be run or exited non-zero (missing binary, no server, bad session). Callers that must not confuse
/// "no such window" with "tmux errored" (e.g. `kill_window`) use this; the rest use the infallible
/// [`tmux_windows`] wrapper.
fn tmux_windows_checked(session: &str) -> Option<Vec<String>> {
    let out = Command::new("tmux")
        .args(["list-windows", "-t", session, "-F", "#W"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.lines().map(str::to_string).collect())
}

/// Ensure a tmux window named after the agent exists in `session`, running `window.sh <name>`.
/// Idempotent: if the window already exists we leave it running (do not relaunch — that would kill a
/// live agent). A stopped agent whose window is gone is not relaunched by `up`; only `active` reach
/// here.
fn ensure_window(fleet: &Fleet, session: &str, a: &Agent) {
    if tmux_windows(session).iter().any(|w| w == &a.name) {
        println!("  = window '{}' already live — left running", a.name);
        return;
    }
    let sh = fleet.window_sh();
    // `new-window -d` (detached) so bringing up many agents doesn't yank the operator's focus around.
    let target = format!("{session}:");
    let cmdline = format!(
        "{} {}",
        shell_quote(&sh.to_string_lossy()),
        shell_quote(&a.name)
    );
    let status = Command::new("tmux")
        .args(["new-window", "-d", "-t", &target, "-n", &a.name, &cmdline])
        .current_dir(&fleet.repo)
        .status();
    match status {
        Ok(s) if s.success() => println!("  + window '{}' launched ({})", a.name, a.role),
        Ok(_) => eprintln!("  ! tmux new-window failed for '{}'", a.name),
        Err(e) => eprintln!("  ! could not run tmux new-window: {e}"),
    }
}

/// The outcome of trying to kill a tmux window — distinguished so the caller can report the real
/// reason rather than lumping "already gone" together with "tmux failed".
enum KillOutcome {
    /// The window existed and was killed.
    Killed,
    /// No window by that name — already closed (or never launched). Not an error.
    NotFound,
    /// The window existed but `tmux kill-window` failed (tmux missing, or errored).
    TmuxError,
}

/// Kill the tmux window named `agent` in `session` (reaping a dead agent's panel). Returns a
/// [`KillOutcome`] so the caller can tell "already closed" from "tmux errored". Targets
/// `session:agent` by NAME, so it never hits the wrong window.
fn kill_window(session: &str, agent: &str) -> KillOutcome {
    // Only kill a window that actually exists — `kill-window` on a missing target errors noisily.
    // Use the CHECKED enumeration so a tmux failure (empty list from an errored tmux) isn't
    // misreported as NotFound: no list at all → TmuxError, a real list without the name → NotFound.
    let Some(windows) = tmux_windows_checked(session) else {
        return KillOutcome::TmuxError;
    };
    if !windows.iter().any(|w| w == agent) {
        return KillOutcome::NotFound;
    }
    let target = format!("{session}:{agent}");
    let ok = Command::new("tmux")
        .args(["kill-window", "-t", &target])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        KillOutcome::Killed
    } else {
        KillOutcome::TmuxError
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// List an agent's queued inbox messages at the canonical HUB path, oldest-first. See the `Inbox`
/// command doc for WHY this exists (the silent relative-path drain-stall). Prints the resolved
/// absolute path so a mis-set path is visible, and prints a LOUD `0 messages` line when empty (rather
/// than nothing) so "empty inbox" can't be silently confused with "couldn't find the inbox". For each
/// message, prints its filename (leading field = the durable delivery seq, so filename sort == arrival
/// order), sender, and kind — enough to drain oldest-first.
fn inbox_list(fleet: &Fleet, name: &str) {
    let dir = fleet.inbox(name);
    // Distinguish a READ ERROR from a genuinely-empty inbox: `read_dir` errs when the path is missing
    // or unreadable — and collapsing that into an empty Vec (the old `unwrap_or_default`) would print
    // "0 messages", the EXACT confusion this command exists to prevent (PR #483 Copilot). On an error,
    // say so loudly + distinctly (an unreadable inbox path is the wrong-path signal, not "no mail").
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            // Always print the resolved path first, then the error — so a wrong/missing path is obvious.
            eprintln!(
                "inbox for '{name}' at {}: COULD NOT READ ({e}). That path is missing or unreadable — \
                 this is NOT an empty inbox. Verify it's the HUB path (`.claude/` exists only at the \
                 main repo, shared via the common git dir), not a worktree-relative `.claude/...`.",
                dir.display()
            );
            std::process::exit(1);
        }
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json"))
        .collect();
    sort_inbox_filenames(&mut names);
    // ALWAYS print the resolved hub path — a wrong path is the whole failure mode we're guarding, so
    // make it visible every time, not just on error.
    println!(
        "inbox for '{name}' at {} ({} message(s)):",
        dir.display(),
        names.len()
    );
    if names.is_empty() {
        // LOUD zero: distinct, explicit line so "0 messages" reads as a real state, not a silent no-op.
        println!(
            "  0 messages — nothing to drain (if you EXPECTED mail, verify this is the HUB path, \
                  not a worktree-relative `.claude/...`)."
        );
        return;
    }
    let mut actionable = 0usize;
    for n in &names {
        // Cheap peek at from/kind without a hard serde dependency on every field being present.
        let (from, kind) = std::fs::read_to_string(dir.join(n))
            .ok()
            .and_then(|t| serde_json::from_str::<Message>(&t).ok())
            .map(|m| (m.from, m.kind))
            .unwrap_or_else(|| ("?".to_string(), "?".to_string()));
        // Flag actionable mail (must DO something) vs informational (read-and-archive) — same
        // classifier the watchdog's drain-stall signal uses, so the CLI and the health check agree.
        let is_act = message_kind_is_actionable(&kind);
        if is_act {
            actionable += 1;
        }
        let mark = if is_act { "⚑" } else { "·" };
        println!("  {mark} {n}  [{kind}] from {from}");
    }
    // Summary: the actionable/informational split so an agent knows at a glance whether idling is a
    // real drain-stall (has ⚑ work) or just un-archived FYI mail (only ·). Mirrors the drain-stall
    // signal exactly (see `message_kind_is_actionable`), reinforcing the drain discipline. The legend
    // is DERIVED from `INFORMATIONAL_KINDS` (the same source `message_kind_is_actionable` uses), so the
    // human text can't drift from the classifier — actionable is honestly "everything else" (the
    // classifier is a deny-list, so there is no fixed actionable enum to spell out).
    let informational = names.len().saturating_sub(actionable);
    println!(
        "  ── {actionable} actionable (⚑ = anything not below), \
         {informational} informational (· {})",
        INFORMATIONAL_KINDS.join("/")
    );
    if actionable == 0 {
        println!(
            "  ✓ nothing ACTIONABLE queued — the informational mail is safe to archive to processed/ \
             (it is not a drain-stall)."
        );
    }
}

/// Sort inbox message filenames into delivery order. Filenames lead with the zero-padded durable
/// delivery-seq (`next_delivery_seq`), so a plain lexicographic sort == oldest-first arrival order.
/// Pure so the ordering contract is unit-testable (and so a future filename-scheme change is caught).
fn sort_inbox_filenames(names: &mut [String]) {
    names.sort();
}

fn inbox_depth(fleet: &Fleet, name: &str) -> String {
    let inbox = fleet.inbox(name);
    let n = count_dir(&inbox, |f| f.ends_with(".json"));
    if n == 0 {
        "empty".to_string()
    } else {
        format!("{n} msg")
    }
}

/// Is a message of this `kind` ACTIONABLE — i.e. the recipient must DO something (drain + act), so an
/// idle agent still holding one is a genuine drain-stall — versus INFORMATIONAL (read-and-archive; a
/// tidy-up, not work)? The message-bus filename is `<seq>-<pid>-<kind>.json`, so `kind` is the trailing
/// dash-segment before `.json` (see `deliver`/`next_delivery_seq`).
///
/// This is the "severity depends on the queued KIND" refinement: the raw inbox count treats a stale
/// `note`/`merged` confirmation the same as an unread `assign`/`reject`, so an agent idling with only
/// already-actioned informational mail it never bothered to archive looks identical to one truly stuck
/// on unread work — a FALSE drain-stall (and, under `--nudge-drain-stalls`, a pointless nudge). Observed
/// live: v-cdz-tooling/v-agent-harness/v-notebook each sit at 1 informational message, 0 actionable.
///
///   * ACTIONABLE: `assign` (do this work), `reject` (fix + resend), `ask` (a peer needs YOUR decision —
///     only the concierge/design are exempt as roles, handled separately), `issue` (triage this bug),
///     `merge-request`/`request` (pr-sync must integrate), `answer` (the reply to an ask this agent is
///     itself waiting on), `redelivery` (a re-sent actionable message).
///   * INFORMATIONAL: `note` (FYI), `merged` (confirmation your OWN MR landed — nothing to do),
///     `backlog` (concierge's own list), `status` (a status reply), `reply` (generic ack).
///
/// Unknown/malformed kinds default to ACTIONABLE (fail-safe: better a spurious flag than a silent miss
/// of real work).
fn message_kind_is_actionable(kind: &str) -> bool {
    !INFORMATIONAL_KINDS.contains(&kind)
}

/// The message kinds that are INFORMATIONAL (read-and-archive; not a drain-stall). This is the SINGLE
/// source of truth: `message_kind_is_actionable` is "not in this list", and the `fleet inbox` summary
/// derives its human text from it — so the classifier and the CLI legend can never drift (the nit
/// Copilot flagged on PR #506: a hard-coded example list in the summary can go stale vs the classifier).
/// A kind added here becomes informational everywhere at once; everything else is actionable.
const INFORMATIONAL_KINDS: &[&str] = &["note", "merged", "backlog", "status", "reply"];

/// Does this bus filename carry the PRE-DURABLE-SEQ sentinel leading field (`000000000001`)? Before the
/// hub-global `.delivery-seq` counter existed (see `next_delivery_seq`), the filename's leading field
/// was the process-local `Message::seq`, which is always `1` for a one-shot `fleet send` — so a
/// `000000000001-…` name provably predates durable-seq AND the `in_reply_to`/`ack` correlation that
/// arrived with it. The audit uses this to avoid flagging such an ancient, inherently-uncorrelatable
/// request as an active-sender "silent drop" forever (a long-lived vertical is always active, so its
/// pre-`ack` MRs would be permanent false orphans that drown a REAL recent drop — which carries a high
/// monotonic seq, so this check can never hide one).
fn is_pre_durable_seq_filename(name: &str) -> bool {
    name.starts_with("000000000001-")
}

/// The message `kind` of a bus filename `<seq>-<pid>-<kind>.json`, or `None` if it doesn't match that
/// pattern (see `deliver`: `format!("{:012}-{}-{}.json", seq, pid, kind)`). The `kind` itself may
/// contain a dash (`merge-request`, `merge-request`-style), so we CANNOT just take the last
/// dash-segment (that yields `request`, and a dash-free non-bus file like `registry.json` would
/// wrongly parse to `Some("registry")` instead of `None`). Instead, strip `.json`, then split off the
/// first two fields (the NUMERIC seq and pid) and return everything after them as the full kind. A name
/// without a `.json` suffix, without at least three fields, or whose seq/pid aren't all-digits → `None`
/// (a stray non-bus file must NOT be miscounted as a message; see `actionable_inbox_depth`).
fn message_kind_from_filename(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".json")?;
    let mut parts = stem.splitn(3, '-');
    let seq = parts.next()?;
    let pid = parts.next()?;
    let kind = parts.next()?; // the remainder — keeps an embedded dash (`merge-request`)
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if numeric(seq) && numeric(pid) && !kind.is_empty() {
        Some(kind)
    } else {
        None
    }
}

/// Count only the ACTIONABLE queued messages in `dir` (non-recursive; ignores `processed/`). This is
/// the drain-stall signal's numerator: informational mail (notes/merged acks) an agent hasn't archived
/// is untidy, not stalled work. `message_kind_from_filename` returns `None` for any non-bus file (e.g. a
/// stray `registry.json`), so such a file is NOT miscounted as an actionable message — only real
/// `<seq>-<pid>-<kind>.json` entries with an actionable kind count.
fn actionable_inbox_depth(dir: &Path) -> usize {
    count_dir(dir, |f| {
        message_kind_from_filename(f).is_some_and(message_kind_is_actionable)
    })
}

/// Count immediate entries in `dir` whose file name passes `keep` (non-recursive; ignores subdirs
/// like `processed/`).
fn count_dir(dir: &Path, keep: impl Fn(&str) -> bool) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter(|e| keep(&e.file_name().to_string_lossy()))
                .count()
        })
        .unwrap_or(0)
}

/// Count `reset: moving to origin/main` entries in `trunk`'s recent reflog (the last `window` entries).
///
/// The fleet's core invariant is that ONLY pr-sync writes `trunk`, and it only ever CHERRY-PICKS
/// (forward). A `reset: moving to origin/main` on `trunk` is therefore a SECOND writer clobbering the
/// branch backward to the published main — an out-of-band sync job running `git reset --hard
/// origin/main` while cwd'd in pr-sync's worktree (which is the only worktree with `trunk` checked
/// out). pr-sync recovers via reflog each time, but a clobber inside its read→reset→replay window can
/// silently drop a whole batch. This is a pure READ of the reflog (never blocks anything) so `status`
/// can SURFACE the violation instead of it being silently self-healed. `None` if the reflog is
/// unreadable; `Some(n)` with the count of clobbers seen in the window (0 = clean).
fn trunk_clobber_count(repo: &Path, window: usize) -> Option<usize> {
    // Bound the read at the source with `-n <window>` (PR #456): `git reflog show` is otherwise
    // unbounded, so a large trunk reflog would be read + parsed in full each call just to look at the
    // most recent `window` entries. The in-process `.take(window)` stays as a belt-and-suspenders cap.
    let out = Command::new("git")
        .current_dir(repo)
        .args([
            "reflog",
            "show",
            "-n",
            &window.to_string(),
            "trunk",
            "--format=%gs",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(
        s.lines()
            .take(window)
            .filter(|line| trunk_reflog_entry_is_clobber(line))
            .count(),
    )
}

/// Whether a `trunk` reflog subject line is a backward-clobber to `origin/main` (a non-pr-sync writer
/// resetting the branch), vs a legit pr-sync `cherry-pick`/`commit` or a self-recovery reset to a
/// `trunk@{…}` reflog point. Pure so it's unit-testable.
fn trunk_reflog_entry_is_clobber(subject: &str) -> bool {
    // git writes `reset: moving to <target>`; the clobber targets origin/main. pr-sync's own recovery
    // resets to a `trunk@{N}` reflog spec, not `origin/main`, so it's correctly NOT flagged.
    subject.trim() == "reset: moving to origin/main"
}

/// Parse `git cherry <upstream> <head>` output into the list of local commits (oldest-first) that
/// are NOT yet upstream and so must be replayed after a reset onto `<upstream>`. `git cherry` marks
/// each commit reachable from `<head>` but not `<upstream>` with a leading `+` (no equivalent patch
/// upstream) or `-` (an equivalent patch — same patch-id — IS already upstream). Only `+` lines need
/// replaying; `-` lines already landed (this is exactly what catches a commit pr-sync integrated under
/// a RE-PARENTED sha — same patch-id, so `git cherry` prints `-` and we correctly DROP it rather than
/// re-applying a now-empty/duplicate cherry-pick). Pure + patch-id-aware so `fleet sync` can never
/// re-stack an already-landed commit. Returns shas in the order git emits them (oldest-first — the
/// order they must be cherry-picked). Malformed/blank lines are ignored.
fn commits_to_replay(cherry_output: &str) -> Vec<String> {
    cherry_output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Expect `+ <sha>` / `- <sha>`; take only the `+` (not-yet-upstream) ones.
            let rest = line.strip_prefix("+ ")?;
            let sha = rest.split_whitespace().next()?;
            if sha.is_empty() {
                None
            } else {
                Some(sha.to_string())
            }
        })
        .collect()
}

/// Map a repo-relative changed path to the coarse OWNERSHIP ZONE it belongs to, or `None` if the path
/// doesn't map to a zone that a roster `area` names. Zones mirror the roster's `area` values where the
/// top-level layout makes ownership UNAMBIGUOUS: `xtask/` (+ `fleet/` and `.cargo/`, the tracked
/// build-throttle config, both v-fleet-tooling's) →
/// "xtask"; `guide/` → "guide"; `spec/` → "spec"; `implementation/compiler-ml/` → "compiler-ml";
/// `implementation/music/` → "music"; `implementation/cad/` → "cad"; `implementation/des/` → "des";
/// `integrations/` → "integrations".
///
/// Deliberately CONSERVATIVE — it returns `None` for anything under `implementation/seed/crates/*`
/// (rcdzc, cdz, cadenza-syntax, runtime, … all co-live there and 15+ verticals share `area: rcdzc`, so
/// a crate path can't be attributed to ONE agent) and for any other unmapped path. The lane guard only
/// warns when EVERY changed path in a replay commit maps to a zone that ISN'T the syncing agent's, so a
/// `None` (unknown-owner) path SUPPRESSES the warning — a false-negative (no warning) is safe, whereas a
/// false-positive that flags an agent's OWN work is what sank the earlier reachability heuristic. Match
/// longest/most-specific prefix first.
fn path_to_zone(path: &str) -> Option<&'static str> {
    // (prefix, zone) — most-specific first so `implementation/compiler-ml/` wins over any broader rule.
    const TABLE: &[(&str, &str)] = &[
        ("xtask/", "xtask"),
        ("fleet/", "xtask"), // v-fleet-tooling owns the fleet tooling + roster/loops source
        (".cargo/", "xtask"), // the tracked build-throttle config (jobs cap) is fleet-tooling's
        ("guide/", "guide"),
        ("spec/", "spec"),
        ("integrations/", "integrations"),
        ("implementation/compiler-ml/", "compiler-ml"),
        ("implementation/music/", "music"),
        ("implementation/cad/", "cad"),
        ("implementation/des/", "des"),
    ];
    let p = path.trim();
    TABLE
        .iter()
        .find(|(prefix, _)| p.starts_with(prefix))
        .map(|(_, zone)| *zone)
}

/// Whether a replay commit (identified by its list of changed repo-relative paths) is FOREIGN to the
/// syncing agent's lane — i.e. EVERY changed path maps to a known ownership zone OTHER than `my_zone`,
/// with at least one path mapping to a known zone. This is the sound, changed-paths-based signal that
/// replaces the unsound reachability heuristic: a commit that touches only `guide/` files replayed onto
/// the `xtask`-lane fleet-tooling branch is clearly a peer's work the base transiently carried.
///
/// CONSERVATIVE by construction (false-negatives over false-positives): returns `false` (NOT foreign, no
/// warning) if ANY changed path is in `my_zone`, OR if ANY changed path is unmappable (`path_to_zone`
/// None — e.g. an `implementation/seed/crates/*` shared-crate path we can't attribute), OR if `my_zone`
/// is empty/unknown, OR if there are no changed paths. So the guard stays silent whenever ownership is
/// the least bit ambiguous, and only speaks up on an unambiguous whole-commit cross-zone mismatch. Pure
/// + unit-testable.
fn commit_is_foreign_to_lane(changed_paths: &[String], my_zone: &str) -> bool {
    if my_zone.is_empty() || changed_paths.is_empty() {
        return false;
    }
    let mut saw_known = false;
    for path in changed_paths {
        match path_to_zone(path) {
            None => return false, // unattributable path → could be mine → stay silent
            Some(z) if z == my_zone => return false, // touches my lane → not foreign
            Some(_) => saw_known = true, // a known OTHER zone
        }
    }
    saw_known
}

/// Append a rejected commit sha to the durable reject-log (deduped — a sha already logged is not
/// re-appended, so the file stays small even if pr-sync rejects the same ref repeatedly). Best-effort:
/// a write failure never blocks the reject delivery (the log is an optimization, not a correctness
/// dependency). Full 40-char shas are stored; the sync guard matches by full-sha equality so a short
/// prefix from a message never yields a partial/ambiguous match.
fn append_reject_log(path: &Path, sha: &str) {
    let sha = sha.trim();
    if sha.is_empty() {
        return;
    }
    if read_reject_log(path).iter().any(|s| s == sha) {
        return; // already recorded
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{sha}");
    }
}

/// Read the reject-log into a set of shas (empty if absent/unreadable — the guard then simply never
/// fires, failing OPEN so a missing log never blocks a sync).
fn read_reject_log(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `sha` (a replay commit) is on the reject-list. Full-sha equality — the caller passes the
/// resolved full sha of a to-be-replayed commit. Pure so it's unit-testable.
fn sha_is_rejected(sha: &str, reject_list: &[String]) -> bool {
    let sha = sha.trim();
    !sha.is_empty() && reject_list.iter().any(|r| r == sha)
}

/// SOFT tier (concierge-approved option 2, 2026-07-20): whether a replay commit is AMBIGUOUS w.r.t.
/// `my_zone` — worth a heads-up even though the loud [`commit_is_foreign_to_lane`] stays silent. The
/// loud test bails the instant ANY path is unattributable, which means a peer's commit touching an
/// `implementation/seed/crates/rcdzc/…` path (unmapped) rides in UNWARNED — the exact systemic
/// contamination that hit ≥6 branches (a rejected rust-async resend spread by `fleet sync`). This
/// softer signal fires when NO changed path is in my lane AND at least one maps to a known OTHER zone,
/// EVEN IF other paths are unattributable — i.e. "nothing here is provably mine, and some of it is
/// provably a peer's". Still conservative: silent if ANY path is in my lane (`Some(z==my_zone)`), or if
/// the commit is entirely unattributable (all `None` — a pure shared-crate edit that could be mine), or
/// if the loud test already flags it (caller checks loud first). Pure + unit-testable.
fn commit_is_ambiguous_to_lane(changed_paths: &[String], my_zone: &str) -> bool {
    if my_zone.is_empty() || changed_paths.is_empty() {
        return false;
    }
    let mut saw_other_zone = false;
    for path in changed_paths {
        match path_to_zone(path) {
            Some(z) if z == my_zone => return false, // provably touches my lane → not ambiguous
            Some(_) => saw_other_zone = true,        // a known OTHER zone
            None => {}                               // unattributable → neither confirms nor clears
        }
    }
    // No in-lane path, and at least one path is provably another zone's → ambiguous heads-up.
    saw_other_zone
}

/// The subset of `commits` (each a `(sha, changed_paths)` pair, e.g. the `trunk..<ref>` range a
/// merge-request would introduce) that are FOREIGN to `my_zone` per [`commit_is_foreign_to_lane`].
/// Pure aggregate over the per-commit test so the send-time contamination guard is unit-testable
/// independent of git: a merge-request whose range carries ANY commit that touches ONLY another
/// vertical's zone is a wrong-attribution risk — the sender would submit a peer's stranded commit
/// (from a load-spike shared-base replay) under its own MR — so the guard refuses when this is
/// non-empty (overridable with `--force` for a genuinely cross-cutting change). Inherits the
/// conservative false-negatives-over-false-positives bias of the per-commit test: an all-mine or
/// ambiguous range yields an empty result (no refusal).
fn foreign_commits_in_range(commits: &[(String, Vec<String>)], my_zone: &str) -> Vec<String> {
    commits
        .iter()
        .filter(|(_, paths)| commit_is_foreign_to_lane(paths, my_zone))
        .map(|(sha, _)| sha.clone())
        .collect()
}

/// The ownership `area`/zone the roster records for agent `name` (empty/`None` if unknown or arealess).
/// Single source of truth for the send-time + sync-time lane guards so they agree on "my lane".
fn roster_zone_of(fleet: &Fleet, name: &str) -> Option<String> {
    fleet
        .load_roster()
        .agents
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.area)
        .filter(|z| !z.is_empty())
}

/// Git-backed driver for the send-time contamination guard: the foreign-to-`from`'s-lane commits in the
/// `trunk..<ref>` range a merge-request names, or `None` when the guard cannot/should-not run (unknown
/// sender zone, unresolvable ref, or trunk not present — a MISSING signal must never manufacture a
/// refusal). Returns `Some([])` when the range is clean or entirely in-lane (caller treats empty as
/// "no contamination" → deliver). The per-commit ownership decision is the pure, unit-tested
/// [`foreign_commits_in_range`]; this wrapper only gathers the (sha, changed-paths) range from git.
fn merge_request_foreign_commits(fleet: &Fleet, from: &str, r#ref: &str) -> Option<Vec<String>> {
    let my_zone = roster_zone_of(fleet, from)?;
    let git_lines = |args: &[&str]| -> Option<Vec<String>> {
        let out = Command::new("git")
            .current_dir(&fleet.repo)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
        )
    };
    // The commits the MR would INTRODUCE relative to trunk. If trunk or ref is unresolvable, bail
    // (None) rather than refuse — an unresolvable ref is pr-sync's problem to report, not ours to block.
    let range = format!("{TRUNK}..{}", r#ref);
    let shas = git_lines(&["rev-list", &range])?;
    let commits: Vec<(String, Vec<String>)> = shas
        .into_iter()
        .filter_map(|sha| {
            let paths = git_lines(&["show", "--name-only", "--format=", &sha])?;
            Some((sha, paths))
        })
        .collect();
    Some(foreign_commits_in_range(&commits, &my_zone))
}

/// Whether `git cherry <trunk> <ref>` output says `<ref>`'s patch is ALREADY upstream (landed) — i.e.
/// every commit line is prefixed `-` (equivalent patch on trunk), with at least one line. `git cherry`
/// prints `- <sha>` for a commit whose patch-id is upstream and `+ <sha>` for one that is not; for a
/// single `<ref>` it's normally one line. A merge-request still QUEUED in pr-sync's inbox whose `--ref`
/// reads landed is a no-op (pr-sync integrated the content under a re-parented sha but never acked the
/// file) — `audit` surfaces these so they can be batch-rejected instead of gated to empty merges. Pure
/// so it's unit-testable. Empty output (ref == trunk, nothing to compare) is NOT "landed" — return
/// false so we never mis-flag a ref we can't prove landed (conservative: a false negative just leaves an
/// MR queued; a false positive would tell pr-sync to reject live work).
fn cherry_says_landed(cherry_output: &str) -> bool {
    let mut saw_line = false;
    for line in cherry_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        saw_line = true;
        // Any `+ ` (not-yet-upstream) line means NOT fully landed.
        if line.starts_with("+ ") {
            return false;
        }
        // Anything that isn't a `- ` marker is unexpected → don't claim landed.
        if !line.starts_with("- ") {
            return false;
        }
    }
    saw_line
}

/// Guard for `fleet sync`: would replaying `replay_shas` orphan a merge-request THIS agent already sent?
/// `fleet sync` cherry-picks unlanded commits onto the fresh trunk, giving them NEW shas — so if a
/// commit in the replay set is the exact `--ref` of a merge-request still queued in pr-sync's inbox
/// (sent by `me`), syncing would move the branch off that ref and pr-sync could no longer fetch it
/// (the silent-skip orphan trap). Returns the FULL sent ref that would be orphaned (so the caller can
/// name it), or `None` if no queued MR from `me` names a to-be-replayed commit. Pure so it's unit
/// testable: the caller passes the already-read pr-sync messages + the replay shas. A queued MR whose
/// ref is NOT in the replay set is fine — either it already landed (so it's not a `+` replay commit) or
/// it's some older ref that the sync doesn't touch. Matching is by prefix in EITHER direction, since a
/// `git cherry` sha is full-length but a hand-sent `--ref` may be abbreviated (or vice-versa).
fn queued_ref_would_orphan(
    replay_shas: &[String],
    me: &str,
    pr_sync_messages: &[Message],
) -> Option<String> {
    for m in pr_sync_messages {
        if m.kind != "merge-request" || m.from != me || m.r#ref.is_empty() {
            continue;
        }
        let sent = m.r#ref.as_str();
        // Compare CASE-INSENSITIVELY: a `git cherry`/`rev-parse` sha is lowercase hex, but a `--ref`
        // may be hand-entered UPPERCASE (git treats commit IDs case-insensitively), and a case-sensitive
        // prefix match would then miss the orphan and let the guard pass a sync that drops the MR ref
        // (PR #473 Copilot). Lowercase both, then prefix-match either direction (the sent ref may be
        // abbreviated relative to the full replay sha, or vice-versa).
        let sent_lc = sent.to_ascii_lowercase();
        if replay_shas.iter().any(|s| {
            let s_lc = s.to_ascii_lowercase();
            s_lc.starts_with(&sent_lc) || sent_lc.starts_with(&s_lc)
        }) {
            return Some(sent.to_string());
        }
    }
    None
}

/// `(ahead, behind)` of `trunk` relative to `origin/main`, or `None` if either ref is unresolvable.
fn trunk_vs_origin_main(repo: &Path) -> Option<(usize, usize)> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["rev-list", "--left-right", "--count", "origin/main...trunk"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    parse_left_right_count(&s)
}

/// Parse `git rev-list --left-right --count A...B` output into `(ahead, behind)`.
///
/// git prints two whitespace-separated counts: LEFT (commits reachable from A only) then RIGHT
/// (reachable from B only). We invoke it as `origin/main...trunk`, so LEFT = origin/main-only =
/// how far trunk is BEHIND the published main, and RIGHT = trunk-only = how far trunk is AHEAD
/// (unpublished). The ordering is load-bearing for the board's publish state and easy to flip in a
/// refactor, so it's pinned here as a pure parse. Returns `None` on malformed/short output.
fn parse_left_right_count(s: &str) -> Option<(usize, usize)> {
    let mut it = s.split_whitespace();
    let behind = it.next()?.parse().ok()?; // left  = origin/main-only ⟹ trunk is behind
    let ahead = it.next()?.parse().ok()?; // right = trunk-only        ⟹ trunk is ahead
    // Reject trailing tokens: the expected output is EXACTLY two counts, so `"3\t5 extra"` is
    // malformed → None (PR #449), matching the doc rather than silently ignoring the surplus.
    if it.next().is_some() {
        return None;
    }
    Some((ahead, behind))
}

/// Age in seconds of the newest commit on `refname` (`now - committer-time of HEAD of ref`), or `None`
/// if the ref can't be resolved. Used as a liveness signal for pr-sync (recent trunk advance ⟹ alive).
fn last_commit_age_secs(repo: &Path, refname: &str) -> Option<u64> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["log", "-1", "--format=%ct", refname])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let ct: u64 = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    Some(now_unix().saturating_sub(ct))
}

// ── gate-batch: optimistic-batch + bisect integration planner ───────────────────────────────────

/// One queued merge-request considered for the round.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BatchMr {
    /// The inbox filename (so pr-sync can `fleet ack` exactly this request).
    file: String,
    /// The sender agent (for the plan report).
    from: String,
    /// The commit sha (`--ref`) to merge.
    r#ref: String,
    /// The branch/subject (for the report).
    subject: String,
}

/// Partition a batch into a LANDABLE subset (confirmed green together) and a REMOVAL set that makes the
/// rest green, given a gate oracle.
///
/// `items[i]` merges cleanly (conflicts were already filtered out by the caller); `gate(subset)` returns
/// `true` iff the combined tree of that subset passes the gate. Pure w.r.t. the oracle — no git/IO here,
/// so the bisection strategy is unit-testable with a synthetic predicate. Strategy: gate the whole set;
/// if green, all land. If red, binary-search for the smallest failing PREFIX and remove its last element
/// (the item whose addition flipped that prefix green→red), then repeat on the remainder until it gates
/// green. The returned landable set is thus CONFIRMED green by a final gate.
///
/// ⚠ The removal set is a set that makes the batch green — NOT necessarily the minimal or the "truly
/// broken" set. For INDEPENDENT failures (each bad item fails on its own) it isolates exactly those. But
/// for an INTERACTION failure (two items that pass alone yet fail TOGETHER), it removes one of the pair
/// — that item is not individually "broken", it just breaks the batch in combination. So a removed item
/// means "excluded so the rest can land this round; rebase+resend to re-attempt", which is the correct
/// bounce for pr-sync either way (the author re-lands against the new trunk, where the interacting
/// partner is now present or absent). Callers/reports should frame a removed item as "excluded this
/// round", not "this MR is definitively buggy". Returns `(landable_indices, removed_indices)` (both into
/// `items`), each sorted ascending.
///
/// `gate` is called on index SUBSETS; the caller maps indices→refs and does the real merge+check.
fn partition_batch(n: usize, mut gate: impl FnMut(&[usize]) -> bool) -> (Vec<usize>, Vec<usize>) {
    let mut remaining: Vec<usize> = (0..n).collect();
    let mut broken: Vec<usize> = Vec::new();
    loop {
        if remaining.is_empty() {
            break;
        }
        if gate(&remaining) {
            // The whole remaining set is green → everything left lands.
            break;
        }
        // Red: isolate one culprit. Find the shortest PREFIX of `remaining` that already fails — its
        // last element is a culprit (adding it flipped green→red for that prefix). Binary-search the
        // prefix length in [1, len]: the smallest `k` with gate(remaining[..k]) == red.
        let mut lo = 1usize; // gate(prefix of len 0) is vacuously green, so a culprit exists at ≥1
        let mut hi = remaining.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if gate(&remaining[..mid]) {
                lo = mid + 1; // prefix still green → culprit is later
            } else {
                hi = mid; // prefix already red → culprit is at or before mid
            }
        }
        // `lo` is the 1-based length of the shortest failing prefix; its last element is the culprit.
        let culprit = remaining[lo - 1];
        broken.push(culprit);
        remaining.retain(|&i| i != culprit);
        // Loop: re-gate the remainder (another independent culprit may still be present); when the
        // remainder finally gates green (or empties), we stop and it's the confirmed landable set.
    }
    remaining.sort_unstable();
    broken.sort_unstable();
    (remaining, broken)
}

/// Ensure a DEDICATED, reusable work tree for the gate-batch scratch branch, returning its path.
///
/// gate-batch does `git checkout -B`, `git merge`, and `cargo xtask check` — all working-tree
/// operations. It CANNOT run them in `fleet.repo`, which is the BARE hub (`git checkout` fatals with
/// "this operation must be run in a work tree"), and it must NOT run them in pr-sync's own worktree
/// (that holds `trunk` and is the single writer mid-integration — displacing its HEAD would corrupt a
/// live merge). So we carve out a separate hub-anchored work tree under `.claude/worktrees/` and reuse
/// it across rounds so its `target/` stays warm (a fresh worktree would rebuild the world every gate).
/// Created DETACHED (no branch) — the scratch BRANCH is created inside it per round via `checkout -B`.
/// Returns `None` if the worktree cannot be created (caller aborts, nothing changed).
fn ensure_gate_batch_worktree(fleet: &Fleet) -> Option<PathBuf> {
    let wt = fleet.worktrees.join("fleet-gate-batch");
    if wt.join(".git").exists() {
        // CRASH-RECOVERY SCRUB. A prior gate-batch run SIGKILLed mid-`git merge` (the 137/SIGTERM
        // scenario the whole hardening arc addresses) leaves the reused worktree with a half-done merge
        // (`.git/MERGE_HEAD` + a conflicted index) and/or a dirty tree + stray untracked files. The
        // per-round `cleanup` closure's `checkout --detach trunk` FAILS on an in-progress merge ("you
        // need to resolve your current index first"), so without this scrub every subsequent run would
        // abort at `git_branch_at`. Reset it to pristine detached-trunk before reuse.
        scrub_gate_batch_worktree(&wt);
        return Some(wt);
    }
    std::fs::create_dir_all(&fleet.worktrees).ok();
    // `--detach` at trunk: HEAD is not on any branch, so the per-round `checkout -B scratch trunk`
    // never collides with a branch this worktree "owns", and cleanup can just detach back.
    let ok = Command::new("git")
        .current_dir(&fleet.repo)
        .args(["worktree", "add", "--detach"])
        .arg(&wt)
        .arg(TRUNK)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Some(wt)
    } else {
        eprintln!(
            "gate-batch: could not create scratch worktree at {wt:?} (git worktree add failed)."
        );
        None
    }
}

/// Scrub a REUSED gate-batch worktree back to a pristine detached-trunk state, recovering from a prior
/// run that was killed mid-operation. Idempotent, and a no-op cost when the worktree is already clean:
///   1. `git merge --abort` — clear a half-finished merge (`MERGE_HEAD` + conflicted index) left by a
///      SIGKILL mid-`git merge`; harmless (nonzero, ignored) when no merge is in progress.
///   2. `git checkout --detach trunk` — get off any scratch branch onto trunk (now possible: the merge
///      is aborted, so the index is clean).
///   3. `git reset --hard trunk` — discard any residual staged/worktree changes.
///   4. `git clean -fd -e target` — remove stray untracked files (a killed store-build's partial
///      artifacts, a foreign leaked `.cdz`) that could perturb the next gate, PRESERVING `target/`
///      (the warm build cache — the whole reason the worktree is reused) via `-e target`.
///
/// Every step ignores its exit status: on an already-clean tree each is a benign no-op, and we want the
/// scrub to push as far toward pristine as it can rather than bail on the first no-op.
fn scrub_gate_batch_worktree(wt: &Path) {
    let git = |args: &[&str]| {
        let _ = Command::new("git").current_dir(wt).args(args).output();
    };
    git(&["merge", "--abort"]);
    git(&["checkout", "--detach", TRUNK]);
    git(&["reset", "--hard", TRUNK]);
    // `-e target` keeps the reused worktree's warm `target/` (build cache + cadenza-store); `-ffd` would
    // nuke it and force a full rebuild every gate, defeating the reuse.
    git(&["clean", "-fd", "-e", "target"]);
}

/// Build the value-heap runtime store in the gate-batch scratch worktree (`cargo xtask build`), so the
/// combined-tree gate can resolve the runtime by content address instead of false-failing every heap
/// case as a store miss. Returns true on success. Runs in `wt` (its own `target/cadenza-store`).
///
/// The scratch worktree is REUSED across rounds (its `target/` stays warm), so its `target/cadenza-store`
/// accumulates AOT caches (`<hash>-wt<v>.cwasm`, written by cdz-run) from every prior round's runtime.
/// When a round MERGES a runtime-changing MR, `xtask build` writes the NEW runtime `.wasm` but leaves the
/// OLD rounds' `.cwasm` behind — and a stale AOT artifact whose wasmtime-version header still matches can
/// deserialize into code that mismatches the fresh source, OOB-trapping a heap case (observed: cad
/// `rotate-bounds-conservatively-encloses`) as a FALSE red the pr-sync worktree + CI both pass. A
/// `.cwasm` is a pure cache (it regenerates from the `.wasm` on next run), so PRUNE every `.cwasm` before
/// rebuilding: cheap, and it can only force a recompile, never lose correctness. (pr-sync's recurring
/// scratch-cad false-red; the durable form of its manual `rm -rf target/cadenza-store`.)
fn gate_batch_build_store(wt: &Path) -> bool {
    let store = wt.join("target/cadenza-store");
    if let Ok(rd) = std::fs::read_dir(&store) {
        let mut pruned = 0usize;
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "cwasm") && std::fs::remove_file(&p).is_ok() {
                pruned += 1;
            }
        }
        if pruned > 0 {
            println!(
                "gate-batch: pruned {pruned} stale AOT cache file(s) (.cwasm) from the scratch store \
                 before rebuild (avoids a stale-cache OOB false-red on a runtime-changing round)."
            );
        }
    }
    println!("gate-batch: building the runtime store in the scratch worktree (cargo xtask build)…");
    Command::new("cargo")
        .current_dir(wt)
        // `xtask build` now takes a fleet-wide build/check concurrency lease (host-hang fix); pr-sync's
        // gate-batch store rebuild is the MERGE QUEUE, so it takes the uncapped PRIORITY slot and never
        // waits behind one-off agent store rebuilds — same discipline as its gate check.
        .env("CDZ_CHECK_PRIORITY", "1")
        .args(["xtask", "build"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Plan a pr-sync integration round with the optimistic-batch + bisect model, printing a
/// machine-readable per-MR decision. Advisory only: never advances `trunk`, never replies — pr-sync
/// executes the plan (single writer + reply-invariant). See the `GateBatch` doc for the loop.
fn gate_batch(fleet: &Fleet, dry_run: bool, limit: usize) {
    // The queued merge-requests, oldest first (the round order — same order the conflict-prefilter uses).
    let mut mrs = queued_merge_requests(fleet);
    if limit > 0 && mrs.len() > limit {
        mrs.truncate(limit);
    }
    if mrs.is_empty() {
        println!("gate-batch: no queued merge-requests — nothing to plan.");
        return;
    }
    println!(
        "gate-batch: planning {} queued merge-request(s)…",
        mrs.len()
    );

    // All the git working-tree ops (checkout -B / merge / `cargo xtask check`) run in a DEDICATED
    // scratch WORK TREE, never in `fleet.repo` (the bare hub — checkout fatals there) and never in
    // pr-sync's worktree (that holds `trunk` and is the single writer). Reused across rounds so its
    // `target/` stays warm. Abort cleanly if it can't be created.
    let wt = match ensure_gate_batch_worktree(fleet) {
        Some(p) => p,
        None => return,
    };

    // Build the value-heap runtime STORE in the scratch worktree BEFORE gating. `cargo xtask check`
    // (what `gate_subset` shells) does NOT build the content-addressed store — it resolves the runtime
    // by content address from `target/cadenza-store`, which a fresh scratch worktree lacks. Without this
    // EVERY heap case fails "no runtime of content address … in the store" (~1154 false store-miss reds
    // observed), so the combined tree reads RED and the bisect isolates INNOCENT MRs on a false red.
    // Mirror a normal pr-sync tick (build the store, then gate). Built once here off the scratch's trunk
    // base; a runtime-bump MR is re-covered because `cargo xtask check`'s own wasm-runtime step rebuilds
    // the runtime and any residual mismatch reds only that subset (bisect then isolates the real bumper).
    if !gate_batch_build_store(&wt) {
        eprintln!(
            "gate-batch: could not build the runtime store in the scratch worktree — aborting \
             (gating now would false-fail every heap case as a store miss)."
        );
        return;
    }

    let scratch = "fleet-gate-batch-scratch";
    // Delete the scratch branch — but you CANNOT `git branch -D` the branch you are ON, so DETACH HEAD
    // (at trunk) FIRST, then delete. We detach rather than `checkout trunk` because pr-sync's worktree
    // holds the `trunk` branch and git forbids checking out a branch that's already checked out in
    // another worktree. `--dry-run` RETAINS the scratch branch for inspection (still detaches off it).
    let cleanup = |branch: &str, keep_branch: bool| {
        // Detach HEAD at trunk so we're never sitting on the branch we're about to delete (and never
        // trying to check out trunk, which pr-sync's worktree already holds).
        let _ = Command::new("git")
            .current_dir(&wt)
            .args(["checkout", "--detach", TRUNK])
            .output();
        if !keep_branch {
            let _ = Command::new("git")
                .current_dir(&wt)
                .args(["branch", "-D", branch])
                .output();
        }
    };

    // CONFLICT-PREFILTER: merge each ref, in order, onto a fresh scratch branch off trunk; an MR that
    // does not merge cleanly is a BOUNCE (no gate cost). The survivors are the cleanly-mergeable set.
    // Start from a clean slate: force-delete any stale scratch branch from a prior run (we're detached
    // now, not on scratch, so this delete is safe).
    cleanup(scratch, false);
    if !git_branch_at(&wt, scratch, "trunk") {
        eprintln!(
            "gate-batch: cannot create scratch branch off trunk — aborting (nothing changed)."
        );
        return;
    }
    let mut clean: Vec<BatchMr> = Vec::new();
    let mut bounced: Vec<BatchMr> = Vec::new();
    for mr in &mrs {
        if git_merge_no_ff_onto(&wt, scratch, &mr.r#ref) {
            clean.push(mr.clone());
        } else {
            // Abort the failed merge so the scratch branch stays at the last clean state.
            let _ = Command::new("git")
                .current_dir(&wt)
                .args(["merge", "--abort"])
                .output();
            bounced.push(mr.clone());
        }
    }

    // OPTIMISTIC GATE + BISECT over the cleanly-merged set. The oracle re-forms a subset onto trunk on
    // the scratch branch and runs `cargo xtask check`.
    let (land_idx, broken_idx) = partition_batch(clean.len(), |subset| {
        gate_subset(&wt, scratch, &clean, subset)
    });

    // Detach off scratch back to trunk; keep the scratch branch only under --dry-run (for inspection).
    cleanup(scratch, dry_run);

    // ── Report the plan (machine-readable: one line per MR + a summary). ──
    println!(
        "\n=== gate-batch plan ({}) ===",
        if dry_run { "dry-run" } else { "execute" }
    );
    for &i in &land_idx {
        let m = &clean[i];
        println!("LAND\t{}\t{}\t{}\t{}", m.file, m.from, m.r#ref, m.subject);
    }
    // AUTO-QUARANTINE DIAGNOSIS: for each broken MR, try to name a DETERMINISTIC, inspectable reason so
    // pr-sync can auto-reject it with a precise, actionable message instead of a generic "broken" that
    // needs a human to re-bisect. Currently we detect baseline-title drift (a wasm-only baseline edit
    // whose rust twin wasn't updated — unambiguous, content-only, no false positives). A broken MR with
    // no recognized deterministic reason stays a plain REJECT-BROKEN (needs a look). Cheap: re-runs only
    // the instant `baseline-titles-agree` lint on that one MR merged, NOT the full ~30min gate.
    for &i in &broken_idx {
        let m = &clean[i];
        match diagnose_broken_reason(&wt, scratch, &m.r#ref) {
            Some(reason) => println!(
                "REJECT-BASELINE-DRIFT\t{}\t{}\t{}\t{}\t{}",
                m.file, m.from, m.r#ref, m.subject, reason
            ),
            None => println!(
                "REJECT-BROKEN\t{}\t{}\t{}\t{}",
                m.file, m.from, m.r#ref, m.subject
            ),
        }
    }
    // Diagnosis left the scratch branch on the last-diagnosed MR; detach back to trunk (keep under
    // --dry-run for inspection) so we leave the worktree in the same clean state as before.
    if !broken_idx.is_empty() {
        cleanup(scratch, dry_run);
    }
    for m in &bounced {
        println!(
            "BOUNCE-CONFLICT\t{}\t{}\t{}\t{}",
            m.file, m.from, m.r#ref, m.subject
        );
    }
    println!(
        "\ngate-batch: {} land / {} reject-broken / {} bounce-conflict (of {} queued). \
         ONE gate run landed {} MRs; pr-sync executes this plan (advances trunk + acks each).",
        land_idx.len(),
        broken_idx.len(),
        bounced.len(),
        mrs.len(),
        land_idx.len()
    );
}

/// List pr-sync's queued merge-requests as `BatchMr`, oldest-first (by the durable `<seq>-` filename).
fn queued_merge_requests(fleet: &Fleet) -> Vec<BatchMr> {
    let dir = fleet.inbox("pr-sync");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with("-merge-request.json"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(p).ok()?;
            let v: serde_json::Value = serde_json::from_str(&text).ok()?;
            Some(BatchMr {
                file: p.file_name()?.to_str()?.to_string(),
                from: v
                    .get("from")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                r#ref: v
                    .get("ref")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                subject: v
                    .get("subject")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .filter(|m| !m.r#ref.is_empty())
        .collect()
}

/// Reset `branch` to `trunk`, then `git merge --no-ff` each `subset` ref onto it, and run the gate.
/// Returns true iff every merge is clean AND `cargo xtask check` passes on the combined tree.
fn gate_subset(repo: &Path, branch: &str, clean: &[BatchMr], subset: &[usize]) -> bool {
    if !git_branch_at(repo, branch, "trunk") {
        return false;
    }
    for &i in subset {
        if !git_merge_no_ff_onto(repo, branch, &clean[i].r#ref) {
            let _ = Command::new("git")
                .current_dir(repo)
                .args(["merge", "--abort"])
                .output();
            return false; // shouldn't happen (subset is pre-filtered clean) but be safe
        }
    }
    // Run the omnibus gate on the combined tree, DETACHED + marker-polled (see gate_check_combined) so a
    // SIGTERM to a foreground `gate-batch` invocation (pr-sync's Bash-tool 120s cap kills the driver, not
    // the legit minutes-long check) neither kills the check nor throws its work away — the next
    // invocation resumes it. Falls back to a plain blocking check if detached mode can't be set up.
    gate_check_combined(repo)
}

/// Diagnose a DETERMINISTIC, inspectable reason a single broken MR fails, for auto-quarantine. Merges
/// just `mr_ref` onto a fresh scratch branch off trunk and runs the fast `baseline-titles-agree` lint
/// (via `cargo xtask gate --check-baseline-titles` — no build, ~instant). Returns `Some(reason)` if that
/// lint fails (a baseline-title divergence — e.g. a wasm-only baseline edit whose rust twin wasn't
/// updated), else `None` (no recognized deterministic reason → the MR stays a plain REJECT-BROKEN that
/// needs a human look). Conservative by construction: only a CLEAN, unambiguous lint failure yields a
/// reason, so we never mis-quarantine an MR that's broken for a subtler cause. Leaves the scratch branch
/// re-formed at this MR; the caller detaches afterward.
fn diagnose_broken_reason(repo: &Path, branch: &str, mr_ref: &str) -> Option<String> {
    // Re-form scratch = trunk + just this MR.
    if !git_branch_at(repo, branch, "trunk") {
        return None;
    }
    if !git_merge_no_ff_onto(repo, branch, mr_ref) {
        let _ = Command::new("git")
            .current_dir(repo)
            .args(["merge", "--abort"])
            .output();
        return None; // wouldn't merge alone → not a lint reason (a conflict is a BOUNCE, handled earlier)
    }
    // Run ONLY the deterministic baseline-titles lint on this tree, IN-PROCESS (it just reads the three
    // `spec/semantics/.gate-baseline*` files and compares their title sets — no build, ~instant). Point a
    // `Paths` at the scratch worktree so it reads THIS MR's baselines. A returned `Err` is the divergence
    // message → the quarantine reason; `Ok` means baselines agree (not the recognized reason).
    let scratch_paths = crate::Paths {
        repo: repo.to_path_buf(),
        seed: repo.join("implementation/seed"),
    };
    match crate::baseline_titles_agree_lint(&scratch_paths) {
        Ok(()) => None, // baselines agree → this isn't the (recognized) reason it's broken
        Err(msg) => Some(format!(
            "baseline-titles-agree: {}",
            first_divergence_line(&msg)
        )),
    }
}

/// The first substantive line of a baseline-titles-agree complaint (multi-line), as a one-line
/// quarantine reason. Falls back to a generic explanation if no line names the specific divergence.
fn first_divergence_line(msg: &str) -> String {
    msg.lines()
        .map(str::trim)
        .find(|l| l.contains("missing"))
        .unwrap_or("baselines diverge (a wasm-only baseline edit without its rust twin)")
        .to_string()
}

/// The full `cargo xtask check` on the current combined tree, run in the FOREGROUND (blocking). Used as
/// the fallback when detached mode can't be set up. `CDZ_CHECK_PRIORITY=1` marks this as the MERGE-QUEUE
/// gate so it takes a PRIORITY check-lease — it never waits behind vertical checks, and vertical checks
/// yield to it (operator: "prioritize the main merge over other checks").
fn blocking_combined_check(repo: &Path) -> bool {
    Command::new("cargo")
        .current_dir(repo)
        .args(["xtask", "check"])
        .env("CDZ_CHECK_PRIORITY", "1")
        // Skip the two trailing REPORT-ONLY conformance sweeps — they never change the merge verdict but
        // can add ~90min after it's decided (matches the detached-check path in `spawn_detached_check`).
        .env("CDZ_GATE_ONLY", "1")
        // Match CI's `CARGO_INCREMENTAL=0` (see .github/workflows/checks.yml). CI runs clippy as a
        // dedicated COLD job; the gate runs `check` in a REUSED, warm-`target/` worktree (kept warm on
        // purpose — see scrub_gate_batch_worktree's `clean -e target`) that is cycled through
        // reset→merge→re-check across partition_batch's binary search over the SAME files in the SAME
        // target dir. Incremental compilation caches at a finer grain than clippy's own fingerprint, and
        // this reset/re-merge cycling is exactly the shape where an incremental unit can be reused with a
        // stale lint verdict — a `#[cfg(test)]`/doc lint the gate reports green yet CI's cold clippy
        // reds (observed: PR#739 assertions_on_constants, PR#741 doc-bullet). Disabling incremental costs
        // nothing here (the gate is not an edit-recompile loop; crate-level warm reuse still applies) and
        // makes the gate's clippy verdict identical to CI's — a lint-red can't slip past the gate.
        .env("CARGO_INCREMENTAL", "0")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Directory holding gate-batch's detached-check markers/logs/scripts. `None` if the hub can't resolve.
fn gate_check_dir(repo: &Path) -> Option<PathBuf> {
    let hub = hub_root(repo)?;
    let dir = hub.join(".claude/fleet/gate-batch-checks");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The combined tree's object id — a STABLE key for the current scratch content. Unlike the merge COMMIT
/// sha (which embeds a timestamp and so differs every re-run), a TREE id is a pure function of content,
/// so an identical subset merged onto an identical trunk always maps to the same key → a killed-and-
/// restarted gate-batch reuses the same in-flight/completed check. `None` if git can't resolve it.
fn merged_tree_key(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "HEAD^{tree}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit())).then_some(key)
}

/// Parse a completion marker's contents (the check's exit code as text) into a verdict. `Some(true)` =
/// exit 0 (green), `Some(false)` = any non-zero, `None` = not-yet-written / garbage (keep polling).
fn parse_exit_marker(contents: &str) -> Option<bool> {
    contents.trim().parse::<i32>().ok().map(|code| code == 0)
}

/// Is `pid` still running? Linux: a live pid has a `/proc/<pid>` entry. No `/proc` (non-Linux) → assume
/// alive (fail-safe: we'd rather keep waiting for the marker than wrongly declare a running check dead).
fn pid_is_alive(pid: u32) -> bool {
    if !Path::new("/proc").is_dir() {
        return true;
    }
    Path::new("/proc").join(pid.to_string()).exists()
}

/// Read a `{key}.exit` marker into a verdict, if it exists and is complete.
fn read_gate_marker(marker: &Path) -> Option<bool> {
    std::fs::read_to_string(marker)
        .ok()
        .and_then(|s| parse_exit_marker(&s))
}

/// Is the detached check whose pid is recorded in `pidf` still alive?
fn gate_check_pidf_alive(pidf: &Path) -> bool {
    std::fs::read_to_string(pidf)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(pid_is_alive)
}

/// Reap gate-check artifacts (marker/log/pid/script) older than 6h — old combined trees no longer gated.
/// TTL for detached gate-check artifacts (`.exit`/`.pid`/`.log`/`.sh` keyed by combined-tree id): a
/// tree that hasn't been the merge target in this long is stale scaffolding, safe to sweep.
const GATE_CHECK_TTL_SECS: u64 = 6 * 3600;

fn reap_stale_gate_checks(dir: &Path) {
    reap_stale_gate_checks_in(dir, now_unix());
}

/// The reap core, split out from `now_unix()` so the `> TTL` mtime boundary is unit-testable against a
/// plain temp dir (mirrors `reap_check_leases`/`reap_check_leases_in`). Returns the count swept.
/// Boundary is STRICT `>`: an artifact whose mtime is exactly one TTL old is kept — the resume path may
/// still key off it — and only a strictly-older one is scaffolding to sweep.
fn reap_stale_gate_checks_in(dir: &Path, now: u64) -> usize {
    let mut reaped = 0usize;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if file_mtime_unix(&p).is_some_and(|m| now.saturating_sub(m) > GATE_CHECK_TTL_SECS)
                && std::fs::remove_file(&p).is_ok()
            {
                reaped += 1;
            }
        }
    }
    reaped
}

/// Launch the combined-tree check DETACHED in its own session (`setsid`), backgrounded so our direct
/// child exits at once (no zombie) and the check is reparented to init — it runs to completion no matter
/// what happens to this gate-batch process. The inner script records its own pid (for liveness/resume),
/// runs the check with output to `log` (a FILE, so a full pipe can never deadlock it), then writes the
/// exit code ATOMICALLY to `marker` (`.tmp` then rename) and clears the pid file.
fn spawn_detached_check(
    repo: &Path,
    dir: &Path,
    key: &str,
    marker: &Path,
    pidf: &Path,
    log: &Path,
) {
    let script_path = dir.join(format!("{key}.sh"));
    let script = format!(
        // `CDZ_GATE_ONLY=1` skips `check`'s two trailing REPORT-ONLY conformance sweeps (cadenza-ml +
        // emit≡interpret). They never affect the gate verdict but can add ~90min AFTER it's decided,
        // freezing the merge queue on already-green work (pr-sync's ">1h trailing-native-step" stall).
        // The merge gate only needs the green/red verdict, so it returns the moment that's known.
        // `CARGO_INCREMENTAL=0` matches CI (.github/workflows/checks.yml) — see the same env on
        // `blocking_combined_check` for why: this detached check runs in the REUSED warm-`target/`
        // worktree that partition_batch cycles reset→merge→re-check over the same files, the shape where
        // incremental can replay a STALE clippy lint verdict a cold CI clippy reds. Keeps the gate's
        // clippy identical to CI so a `#[cfg(test)]`/doc lint can't land green.
        "#!/bin/sh\n\
         echo $$ > '{pidf}'\n\
         CDZ_GATE_ONLY=1 CDZ_CHECK_PRIORITY=1 CARGO_INCREMENTAL=0 cargo xtask check > '{log}' 2>&1\n\
         code=$?\n\
         printf '%s' \"$code\" > '{marker}.tmp' && mv '{marker}.tmp' '{marker}'\n\
         rm -f '{pidf}'\n",
        pidf = pidf.display(),
        log = log.display(),
        marker = marker.display(),
    );
    if std::fs::write(&script_path, &script).is_err() {
        eprintln!(
            "gate-batch: could not write the detached-check script — poll will treat as RED."
        );
        return;
    }
    // `setsid` → new session (survives a SIGTERM to gate-batch's group); trailing `&` backgrounds it so
    // the outer `sh` returns immediately and the session leader is reparented to init.
    let launcher = format!(
        "setsid sh '{}' </dev/null >/dev/null 2>&1 &",
        script_path.display()
    );
    let launched = Command::new("sh")
        .current_dir(repo)
        .args(["-c", &launcher])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if launched {
        println!(
            "gate-batch: launched detached combined-tree check for {key} (log: {}).",
            log.display()
        );
    } else {
        eprintln!("gate-batch: could not launch the detached check — poll will treat as RED.");
    }
}

/// Run `cargo xtask check` on the current combined tree as a DETACHED (own-session) process that writes
/// its exit code to a marker, then POLL the marker. This makes gate-batch immune to the 2-minute
/// Bash-tool timeout that SIGTERMs pr-sync's foreground `gate-batch` invocation mid-check: the check runs
/// in its own session (survives the SIGTERM) and, because it's keyed by the STABLE combined-tree id, a
/// killed-and-restarted gate-batch RESUMES the same in-flight (or already-completed) check instead of
/// throwing minutes of work away and starting over — which is what caused the 7-restarts-never-completes
/// stall. Falls back to a blocking check if the marker dir or tree key can't be resolved (fail-safe;
/// only loses the timeout-immunity).
fn gate_check_combined(repo: &Path) -> bool {
    let (Some(dir), Some(key)) = (gate_check_dir(repo), merged_tree_key(repo)) else {
        return blocking_combined_check(repo);
    };
    reap_stale_gate_checks(&dir);
    let marker = dir.join(format!("{key}.exit"));
    let pidf = dir.join(format!("{key}.pid"));
    let log = dir.join(format!("{key}.log"));

    // 1. A completed verdict already on disk (a prior, possibly-killed invocation's check finished)?
    if let Some(ok) = read_gate_marker(&marker) {
        println!(
            "gate-batch: reusing a completed combined-tree check for {key} ({}).",
            if ok { "green" } else { "red" }
        );
        return ok;
    }
    // 2. A detached check for this exact tree still alive (a prior invocation was killed mid-check)? →
    //    resume by polling it; otherwise launch a fresh one.
    if gate_check_pidf_alive(&pidf) {
        println!(
            "gate-batch: resuming an in-flight detached check for tree {key} (survived a prior kill)."
        );
    } else {
        spawn_detached_check(repo, &dir, &key, &marker, &pidf, &log);
    }
    // Startup grace: let the freshly launched inner script register its pid (or already finish) before
    // the poll loop's crash-detection can fire.
    let mut waited = 0u64;
    while !pidf.exists() && !marker.exists() && waited < 30 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        waited += 2;
    }
    // 3. Poll the marker. If THIS process is SIGTERM'd mid-poll, the detached check keeps running and the
    //    next gate-batch invocation re-enters at step 1/2 and resumes it.
    loop {
        if let Some(ok) = read_gate_marker(&marker) {
            return ok;
        }
        // The detached check ended WITHOUT a verdict (OOM/crash/launch-fail) → treat as RED (never a
        // false green); bisect then isolates the offending subset just as for a normal failure.
        if !gate_check_pidf_alive(&pidf) && !marker.exists() {
            eprintln!(
                "gate-batch: detached check for tree {key} ended without a verdict (crash/OOM/launch-fail) — treating as RED."
            );
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// Force-create/reset `branch` to point at `at` and check it out. Returns false on any git failure.
fn git_branch_at(repo: &Path, branch: &str, at: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["checkout", "-B", branch, at])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `git merge --no-ff <ref>` onto the current branch (no edit). Returns true iff the merge is clean.
fn git_merge_no_ff_onto(repo: &Path, _branch: &str, r#ref: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["merge", "--no-ff", "--no-edit", r#ref])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_batch_all_green_lands_everything() {
        // No culprit → the whole set lands, no bisection.
        let (land, broken) = partition_batch(5, |_subset| true);
        assert_eq!(land, vec![0, 1, 2, 3, 4]);
        assert!(broken.is_empty());
    }

    #[test]
    fn partition_batch_isolates_a_single_culprit() {
        // Item 2 is poison: any subset CONTAINING it fails; every subset without it passes. The
        // bisection must land {0,1,3,4} and reject {2}, with log2-ish gate calls (not linear).
        let bad = 2usize;
        let (land, broken) = partition_batch(5, |subset| !subset.contains(&bad));
        assert_eq!(land, vec![0, 1, 3, 4]);
        assert_eq!(broken, vec![bad]);
    }

    #[test]
    fn partition_batch_finds_multiple_independent_culprits() {
        // Two independent culprits (1 and 3): a subset fails iff it contains EITHER. The loop must peel
        // both out and land the rest, confirming green only when neither remains.
        let bads = [1usize, 3usize];
        let (land, broken) = partition_batch(6, |subset| !subset.iter().any(|i| bads.contains(i)));
        assert_eq!(land, vec![0, 2, 4, 5]);
        assert_eq!(broken, vec![1, 3]);
    }

    #[test]
    fn partition_batch_empty_is_trivially_landable() {
        let (land, broken) = partition_batch(0, |_| false);
        assert!(land.is_empty());
        assert!(broken.is_empty());
    }

    #[test]
    fn partition_batch_first_item_culprit_is_found() {
        // Edge: the culprit is index 0 (the shortest failing prefix is length 1). Binary search must
        // still isolate it (lo starts at 1, prefix[..1] already red → culprit = remaining[0]).
        let (land, broken) = partition_batch(4, |subset| !subset.contains(&0));
        assert_eq!(land, vec![1, 2, 3]);
        assert_eq!(broken, vec![0]);
    }

    #[test]
    fn partition_batch_interaction_failure_still_lands_a_sound_green_set() {
        // NON-MONOTONE predicate: the batch fails ONLY when items 1 AND 3 are BOTH present; each is
        // perfectly fine on its own. This is the "over-approximation" case flagged in review — there is
        // no independently-broken MR, so the heuristic MUST attribute the failure to one member of the
        // interacting pair. The load-bearing invariant is NOT which member it blames but that the SET IT
        // LANDS ALWAYS GATES GREEN: removing the blamed item heals the batch, so no red tree is ever
        // published. Pin the exact behavior so a refactor can't silently start landing a red set.
        let (land, broken) =
            partition_batch(5, |subset| !(subset.contains(&1) && subset.contains(&3)));
        // It peels the LAST element of the shortest failing prefix (index 3), then the remainder
        // {0,1,2,4} no longer contains both → green, so it lands.
        assert_eq!(broken, vec![3], "blames one member of the interacting pair");
        assert_eq!(land, vec![0, 1, 2, 4]);
        // The soundness invariant, checked directly against the same predicate: the landed set is green.
        let landed_gates_green = !(land.contains(&1) && land.contains(&3));
        assert!(
            landed_gates_green,
            "the set gate-batch lands must always pass the gate"
        );
    }

    #[test]
    fn first_divergence_line_picks_the_specific_missing_titles_line() {
        // The lint's Err is multi-line; the quarantine reason should be the specific "missing" line.
        let msg = "baseline title sets diverge:\n  \
                   .gate-baseline-rust is missing 2 title(s) present in another baseline:\n    \
                   foo, bar\n  fix: regenerate with gate --save --target rust";
        assert_eq!(
            first_divergence_line(msg),
            ".gate-baseline-rust is missing 2 title(s) present in another baseline:",
            "picks the line that names which baseline is missing which titles"
        );
        // No line containing "missing" → generic fallback (still an actionable, non-empty reason).
        assert_eq!(
            first_divergence_line("some other divergence, no such keyword\nsecond line"),
            "baselines diverge (a wasm-only baseline edit without its rust twin)"
        );
    }

    #[test]
    fn is_status_title_line_matches_the_title_in_any_punctuation_without_mangling_hyphenated_names()
    {
        // The title line (redundant with the board's name column) is skipped in ANY form…
        assert!(is_status_title_line("# concierge status", "concierge"));
        assert!(is_status_title_line(
            "# v-fleet-tooling — status",
            "v-fleet-tooling"
        ));
        assert!(is_status_title_line(
            "v-fleet-tooling status",
            "v-fleet-tooling"
        ));
        assert!(is_status_title_line("## pr-sync — status", "pr-sync"));
        // …but a REAL first line (the actual headline) is NOT mistaken for the title.
        assert!(!is_status_title_line("**Did:** drained inbox", "concierge"));
        assert!(!is_status_title_line(
            "SLICE 2+3 LANDED (caa9c47c)",
            "v-fleet-tooling"
        ));
        assert!(!is_status_title_line(
            "status of the world: green",
            "v-fleet-tooling"
        ));
        // A different agent's title is not this agent's title.
        assert!(!is_status_title_line(
            "# concierge status",
            "v-fleet-tooling"
        ));
    }

    #[test]
    fn interval_parses_every_supported_unit() {
        assert_eq!(parse_interval_secs("30s"), 30);
        assert_eq!(parse_interval_secs("10m"), 600);
        assert_eq!(parse_interval_secs("30m"), 1800);
        assert_eq!(parse_interval_secs("2h"), 7200);
        assert_eq!(parse_interval_secs("1d"), 86400);
    }

    #[test]
    fn interval_bare_number_is_minutes() {
        // The registry always carries a unit, but a bare number should mean minutes, not seconds —
        // the watchdog must never treat "10" as a 10-second stale window.
        assert_eq!(parse_interval_secs("10"), 600);
        // "10"→600 coincides with the `_ => 600` malformed fallback, so on its own it can't catch a
        // mutant that DROPS `""` from the `"m" | ""` arm (bare numbers would then fall through to the
        // fallback and STILL return 600). Pin a bare number whose minutes-value differs from 600 so the
        // `""` arm is genuinely exercised: 3×60 = 180 ≠ 600.
        assert_eq!(parse_interval_secs("3"), 180);
    }

    #[test]
    fn interval_malformed_falls_back_to_ten_minutes() {
        // A malformed/zero interval must NOT collapse to a 0-second stale window (which would re-arm
        // a healthy agent on every pass). Fall back to the 10m default instead.
        assert_eq!(parse_interval_secs(""), 600);
        assert_eq!(parse_interval_secs("garbage"), 600);
        assert_eq!(parse_interval_secs("0m"), 600);
        assert_eq!(parse_interval_secs("5x"), 600);
        assert_ne!(parse_interval_secs("0m"), 0);
    }

    #[test]
    fn interval_tolerates_surrounding_whitespace() {
        assert_eq!(parse_interval_secs("  15m "), 900);
    }

    #[test]
    fn stale_window_is_one_interval_plus_capped_grace() {
        // The window = interval + min(interval×(mult-1), cap). The interval floor means a long-interval
        // agent is NEVER flagged BETWEEN its legitimate ticks (the false-re-arm bug): a 30m agent's
        // heartbeat cycles 0→1800→(tick)→0, so it's overdue only after >1800s + grace, not after 600s.
        let cap = 600;
        assert_eq!(stale_window_secs(1800, 2, cap), 1800 + 600); // 30m interval + 600 grace = 2400 (40m)
        assert_eq!(stale_window_secs(3600, 2, cap), 3600 + 600); // 60m + 600 grace = 4200 (70m)
        // A dead 30m loop is still caught within interval + cap (~40min), bounded — not the old 60min×.
    }

    #[test]
    fn stale_window_short_interval_grace_below_cap() {
        // A short-interval agent: grace = interval×(mult-1), under the cap, so window = interval×mult
        // (unchanged from before for these — the fix only matters where interval×(mult-1) exceeds cap).
        let cap = 600;
        assert_eq!(stale_window_secs(60, 2, cap), 60 + 60); // 1m + 1m grace = 120 (= old 1m×2)
        assert_eq!(stale_window_secs(300, 2, cap), 300 + 300); // 5m + 5m grace = 600 (= old 5m×2)
        // mult=3: grace = interval×2, capped. 5m×2=600 = cap exactly → 300+600 = 900.
        assert_eq!(stale_window_secs(300, 3, cap), 300 + 600);
    }

    #[test]
    fn stale_window_never_overflows() {
        // A huge interval must saturate, not wrap. interval saturates to u64::MAX; grace caps at 600;
        // the final add saturates → u64::MAX (never a wrap to a tiny window that would thrash-re-arm).
        assert_eq!(stale_window_secs(u64::MAX, 2, 600), u64::MAX);
        // mult=1 → zero grace → window is exactly one interval.
        assert_eq!(stale_window_secs(600, 1, 600), 600);
    }

    #[test]
    fn agent_name_accepts_real_names() {
        // Every real roster name: alphanumerics + hyphens, leading alphanumeric. No dots/underscores.
        for ok in [
            "pr-sync",
            "corpus-bugfix",
            "fix-two-string-payloads",
            "v-fleet-tooling",
            "concierge",
            "breaker",
            "github-liaison",
            "a",
        ] {
            assert!(validate_agent_name(ok).is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn recipient_from_subject_derives_the_agent() {
        // pr-sync's reply subjects — the recipient is recoverable even when to=unknown.
        assert_eq!(
            recipient_from_subject("merged: fleet/v-diagnostics").as_deref(),
            Some("v-diagnostics")
        );
        assert_eq!(
            recipient_from_subject("reject: fleet/fix-host-closure-declines").as_deref(),
            Some("fix-host-closure-declines")
        );
        // A branch token mid-sentence still resolves; trailing text after the agent stops at whitespace.
        assert_eq!(
            recipient_from_subject("integrated fleet/breaker onto trunk").as_deref(),
            Some("breaker")
        );
        // No fleet/<agent> → None (don't misroute).
        assert!(recipient_from_subject("some unrelated subject").is_none());
        // A traversal-y agent token is rejected by the name charset → None (never route to garbage).
        assert!(recipient_from_subject("merged: fleet/../etc").is_none());
        // The `[reconciled from unknown/]`-PREFIXED subject that `reroute_unknown` itself produces must
        // STILL resolve — else a re-routed note (if it ever re-hit unknown/) couldn't be re-derived, and
        // an anchored-at-start rewrite of this fn would silently break the reconciler. Pin the round-trip.
        assert_eq!(
            recipient_from_subject("[reconciled from unknown/] merged: fleet/v-quantity")
                .as_deref(),
            Some("v-quantity")
        );
    }

    #[test]
    fn sender_from_branch_name_maps_branch_to_agent() {
        // A vertical/fix agent's worktree is on `fleet/<agent>` → that branch names the sender. This is
        // the `--from` derivation that keeps pr-sync's replies from dead-lettering to `unknown`.
        assert_eq!(
            sender_from_branch_name("fleet/v-fleet-tooling").as_deref(),
            Some("v-fleet-tooling")
        );
        assert_eq!(
            sender_from_branch_name("fleet/fix-two-string-payloads").as_deref(),
            Some("fix-two-string-payloads")
        );
        // pr-sync runs on `trunk` itself.
        assert_eq!(sender_from_branch_name("trunk").as_deref(), Some("pr-sync"));
        // Neither a fleet/ branch nor trunk → None (caller falls back to `unknown`), and an empty
        // string (detached HEAD prints nothing) → None, never a bogus sender.
        assert!(sender_from_branch_name("main").is_none());
        assert!(sender_from_branch_name("some-feature").is_none());
        assert!(sender_from_branch_name("").is_none());
        // A `fleet/<garbage>` branch must NOT resolve to a garbage/traversal sender (name charset guard).
        assert!(sender_from_branch_name("fleet/../etc").is_none());
        assert!(sender_from_branch_name("fleet/").is_none());
        assert!(sender_from_branch_name("fleet/a b").is_none());
    }

    #[test]
    fn agent_name_rejects_path_traversal_and_non_roster_chars() {
        // The PR#391 class: a name that escapes the inbox tree must be refused — plus dots/underscores
        // (no real name uses them, and a dot is what enables the `..` traversal), matching the Slack
        // bridge's stricter `^[A-Za-z0-9][A-Za-z0-9-]*$`.
        for bad in [
            "..",
            ".",
            "../x",
            "../../etc",
            "a/b",
            "a\\b",
            "/abs",
            "",
            ".hidden",     // leading dot (dotfile-lookalike)
            "-flag",       // leading dash (flag-lookalike)
            "a..b",        // embedded `..`
            "a.b",         // any dot now rejected (aligns with the bridge)
            "a_b",         // underscore now rejected (no roster name uses one)
            "fix-pr391.2", // dot rejected
            "a b",         // space (not in the charset)
            "a\0b",        // NUL
        ] {
            assert!(validate_agent_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn safe_component_accepts_real_inbox_basenames() {
        // Inbox filenames legitimately carry digits, dashes, and dots — only traversal is barred.
        for ok in [
            "000000000001-1234-merge-request.json",
            "000000000099-99999-merge-request.json",
            "a.json",
            "request",
        ] {
            assert!(is_safe_component(ok), "should accept {ok:?}");
        }
    }

    #[test]
    fn safe_component_rejects_traversal() {
        // The PR#392 class: a `fleet ack` request basename that escapes pr-sync's inbox must be barred.
        for bad in [
            "",
            ".",
            "..",
            "../registry.json",
            "processed/../x",
            "a/b",
            "a\\b",
            "/abs/path",
            "x..y", // embedded `..`
            "a\0b",
        ] {
            assert!(!is_safe_component(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn in_reply_to_round_trips_and_is_omitted_when_empty() {
        // A reply from `fleet ack` carries the resolved request's filename so `fleet audit` can pair
        // them; a plain message omits the field entirely (so old JSON stays readable + diffs stay clean).
        let reply = Message {
            from: "pr-sync".into(),
            to: "v-x".into(),
            kind: "merged".into(),
            subject: "merged: fleet/v-x".into(),
            r#ref: "trunk@abc".into(),
            body: "ok".into(),
            seq: 1,
            in_reply_to: "000000000001-42-merge-request.json".into(),
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert!(json.contains("in_reply_to"));
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.in_reply_to, "000000000001-42-merge-request.json");

        let plain = Message {
            in_reply_to: String::new(),
            ..reply
        };
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("in_reply_to"), "empty field must be omitted");
        // And a message with NO in_reply_to key deserializes fine (default).
        let old: Message =
            serde_json::from_str(r#"{"from":"a","to":"b","kind":"note","subject":"s","seq":9}"#)
                .unwrap();
        assert_eq!(old.in_reply_to, "");
    }

    #[test]
    fn merged_floor_takes_field_wise_max() {
        // The driver's whole reason to exist: two agents each bumped the monotone floor, so the merge
        // must be the field-wise MAX of both sides — never one side clobbering the other's higher count.
        let ours = serde_json::json!({"cited": 645, "total": 900});
        let theirs = serde_json::json!({"cited": 644, "total": 901});
        let m = merged_floor_value(ours, &theirs).expect("two objects merge");
        assert_eq!(m.get("cited").and_then(|n| n.as_u64()), Some(645)); // ours higher
        assert_eq!(m.get("total").and_then(|n| n.as_u64()), Some(901)); // theirs higher
    }

    #[test]
    fn merged_floor_is_symmetric_and_idempotent() {
        // max is commutative, so which side is `ours` must not change the merged counters (only which
        // side's extra keys survive) — and merging a value with itself is a no-op.
        let a = serde_json::json!({"cited": 10, "total": 20});
        let b = serde_json::json!({"cited": 30, "total": 15});
        let ab = merged_floor_value(a.clone(), &b).expect("objects merge");
        let ba = merged_floor_value(b.clone(), &a).expect("objects merge");
        assert_eq!(ab.get("cited"), ba.get("cited"));
        assert_eq!(ab.get("total"), ba.get("total"));
        assert_eq!(ab.get("cited").and_then(|n| n.as_u64()), Some(30));
        assert_eq!(ab.get("total").and_then(|n| n.as_u64()), Some(20));
        let self_merged = merged_floor_value(a.clone(), &a).expect("objects merge");
        assert_eq!(self_merged.get("cited").and_then(|n| n.as_u64()), Some(10));
        assert_eq!(self_merged.get("total").and_then(|n| n.as_u64()), Some(20));
    }

    #[test]
    fn merged_floor_preserves_ours_extra_keys() {
        // Ours' non-counter fields (e.g. the `_note` describing the file) must survive the merge —
        // the driver rewrites only `cited`/`total`, everything else on ours is kept verbatim.
        let ours =
            serde_json::json!({"cited": 1, "total": 2, "_note": "coverage floor — do not lower"});
        let theirs = serde_json::json!({"cited": 5, "total": 5});
        let m = merged_floor_value(ours, &theirs).expect("objects merge");
        assert_eq!(
            m.get("_note").and_then(|s| s.as_str()),
            Some("coverage floor — do not lower")
        );
        assert_eq!(m.get("cited").and_then(|n| n.as_u64()), Some(5));
    }

    #[test]
    fn merged_floor_missing_or_garbage_counter_reads_as_zero() {
        // A missing/non-numeric counter must read as 0 so the OTHER side wins, never poisoning the
        // merged floor to 0 or panicking. (A monotone floor should never regress to 0 by a bad merge.)
        let ours = serde_json::json!({"total": 900}); // no `cited`
        let theirs = serde_json::json!({"cited": 644, "total": "oops"}); // garbage `total`
        let m = merged_floor_value(ours, &theirs).expect("objects merge (garbage counters ok)");
        assert_eq!(m.get("cited").and_then(|n| n.as_u64()), Some(644)); // theirs, ours missing→0
        assert_eq!(m.get("total").and_then(|n| n.as_u64()), Some(900)); // ours, theirs garbage→0
    }

    #[test]
    fn merged_floor_rejects_a_valid_json_non_object() {
        // PR #430: a floor file that is valid JSON but NOT an object (null, a number, an array, a
        // string) is a corrupt/wrong floor. The driver must NOT "resolve" the conflict by rewriting it
        // (or returning it unchanged) — it must decline (None) so merge_floor leaves the conflict for a
        // human. Distinct from missing/garbage COUNTERS inside an object (those correctly read as 0).
        let obj = serde_json::json!({"cited": 5, "total": 5});
        for bad in [
            serde_json::Value::Null,
            serde_json::json!(42),
            serde_json::json!("644"),
            serde_json::json!([1, 2, 3]),
            serde_json::json!(true),
        ] {
            // Either side being a non-object declines.
            assert!(
                merged_floor_value(bad.clone(), &obj).is_none(),
                "ours={bad:?} must decline"
            );
            assert!(
                merged_floor_value(obj.clone(), &bad).is_none(),
                "theirs={bad:?} must decline"
            );
        }
        // Two non-objects also decline (never rewrite).
        assert!(merged_floor_value(serde_json::Value::Null, &serde_json::json!(0)).is_none());
    }

    #[test]
    fn maxfloor_driver_is_worktree_portable() {
        // PR #426: the driver value lives in the hub-shared .git/config but a merge can run from ANY
        // worktree, each with its OWN target/. So the command must NOT embed an absolute path or a
        // `target/` binary (current_exe()'s form) — it must resolve via the tracked `cargo xtask`
        // alias from whatever worktree git runs the merge in.
        let cmd = maxfloor_driver_command();
        assert!(
            cmd.starts_with("cargo xtask "),
            "driver must invoke the portable cargo alias, got {cmd:?}"
        );
        assert!(
            !cmd.contains("target/") && !cmd.contains('/'),
            "driver must not bake in a worktree-local path, got {cmd:?}"
        );
        // git substitutes the two conflicting versions into %A (ours/result) and %B (theirs).
        assert!(
            cmd.contains("%A") && cmd.contains("%B"),
            "driver needs the %A/%B placeholders"
        );
        assert!(
            cmd.contains("fleet merge-floor"),
            "driver must call the merge-floor subcommand"
        );
    }

    #[test]
    fn rearm_escalates_only_when_a_prior_nudge_didnt_stick() {
        use RearmAction::*;
        // (rearm_age, hb_age) — both "seconds ago", larger = older, None = never.

        // Never re-armed → first-time cheap nudge, regardless of heartbeat state.
        assert_eq!(rearm_action(None, Some(900)), NudgeContinue);
        assert_eq!(rearm_action(None, None), NudgeContinue);

        // Re-armed, and NO heartbeat since (hb older-or-equal than the re-arm) → the nudge didn't stick
        // → escalate to arm a cron. (hb_age 900 ≥ rearm_age 300 ⟹ last tick predates the re-arm.)
        assert_eq!(rearm_action(Some(300), Some(900)), ReissueLoop);
        // Re-armed and NEVER heartbeated → definitely didn't stick → escalate.
        assert_eq!(rearm_action(Some(300), None), ReissueLoop);
        // Equal ages (can't prove a tick landed after) → escalate (conservative).
        assert_eq!(rearm_action(Some(300), Some(300)), ReissueLoop);

        // THE BUG THIS GUARDS: an agent re-armed long ago that RECOVERED and has heartbeated SINCE
        // (hb_age 60 < rearm_age 3600) must get a cheap nudge on a fresh stall — NOT a `/loop` re-issue.
        // The rearm marker is never cleared, so keying on "ever re-armed" would wrongly escalate here.
        assert_eq!(rearm_action(Some(3600), Some(60)), NudgeContinue);
    }

    #[test]
    fn pane_busy_is_trusted_only_when_the_agent_has_ever_heartbeated() {
        // A has-looped agent (heartbeat stamped ≥once) with a busy pane is genuinely mid-tick → trust
        // the pane, skip re-arm (never interrupt real work).
        assert!(pane_busy_means_working(true, true));
        // A has-looped agent whose pane is idle → not working → don't skip on this guard.
        assert!(!pane_busy_means_working(true, false));
        // A NEVER-heartbeated agent (past cold-start) with a BUSY pane is FLAILING, not working — the
        // origin/trunk-flail false-positive. Its pane must NOT be trusted, so re-arm proceeds.
        assert!(!pane_busy_means_working(false, true));
        // Never-heartbeated + idle pane: also not "working"; re-arm proceeds.
        assert!(!pane_busy_means_working(false, false));
    }

    #[test]
    fn watchdog_tick_prompt_mirrors_the_kickoff_contract() {
        // The re-issued loop must run the SAME contract as a freshly-launched window (heartbeat → drain
        // inbox → one gated unit of work), addressed to THIS agent's name/role/worktree/inbox.
        let fleet = Fleet {
            root: PathBuf::from("/hub/.claude/fleet"),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        let a = Agent {
            name: "fix-float-compare".into(),
            role: "fix".into(),
            vertical: String::new(),
            area: String::new(),
            worktree: "/wt/fix-float-compare".into(),
            branch: "fleet/fix-float-compare".into(),
            interval: "10m".into(),
            model: "opus".into(),
            effort: "high".into(),
            status: "active".into(),
            disallow_ask: true,
        };
        let p = watchdog_tick_prompt(&fleet, &a);
        assert!(p.contains("cargo xtask fleet heartbeat fix-float-compare"));
        assert!(p.contains("drain your inbox"));
        assert!(p.contains("inbox/fix-float-compare/"));
        assert!(p.contains("/wt/fix-float-compare/fleet/loops/fix.md"));
        assert!(p.contains("never wait for a reply"));
        // A role with no vertical must NOT emit the vertical clause.
        assert!(!p.contains("Your vertical is"));
    }

    #[test]
    fn watchdog_tick_prompt_includes_vertical_note_when_set() {
        let fleet = Fleet {
            root: PathBuf::from("/hub/.claude/fleet"),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        let a = Agent {
            name: "v-iterators".into(),
            role: "vertical".into(),
            vertical: "iterators".into(),
            area: "rcdzc".into(),
            worktree: "/wt/v-iterators".into(),
            branch: "fleet/v-iterators".into(),
            interval: "10m".into(),
            model: "opus".into(),
            effort: "high".into(),
            status: "active".into(),
            disallow_ask: true,
        };
        let p = watchdog_tick_prompt(&fleet, &a);
        assert!(p.contains("Your vertical is 'iterators' in subsystem 'rcdzc'."));
        assert!(p.contains("per /wt/v-iterators/fleet/loops/vertical.md"));
    }

    #[test]
    fn fmt_age_picks_the_right_unit() {
        assert_eq!(fmt_age(0), "0s");
        assert_eq!(fmt_age(59), "59s");
        assert_eq!(fmt_age(60), "1m");
        assert_eq!(fmt_age(3599), "59m");
        assert_eq!(fmt_age(3600), "1h");
        assert_eq!(fmt_age(86399), "23h");
        assert_eq!(fmt_age(86400), "1d");
    }

    #[test]
    fn shell_quote_wraps_and_escapes_for_eval() {
        // `describe` emits KEY='<value>' lines that window.sh `eval`s, so shell_quote MUST produce a
        // literal that a POSIX shell reads back as the exact input — no expansion, no injection. A plain
        // value is just single-quoted; an embedded single quote uses the POSIX close-reopen idiom
        // ('\'') since a single-quoted string can't contain a quote. (Each expected literal below was
        // verified to round-trip through a real `sh -c` in this slice.)
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("with space"), "'with space'");
        // A metacharacter payload stays inert (single quotes suppress $(...)/`...`/$VAR expansion).
        assert_eq!(shell_quote("$(evil)"), "'$(evil)'");
        assert_eq!(shell_quote("`evil`"), "'`evil`'");
        // The load-bearing case: an embedded single quote → close, escaped-quote, reopen.
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
        // Empty string is a valid empty quoted literal (not nothing) — else `eval` would drop the arg.
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn resolve_model_maps_aliases_and_passes_through() {
        // The ONE place the long 1M-context ids live — the roster/`--model` carries short aliases and
        // window.sh hands the resolved id to `claude --model`. A wrong mapping runs every agent on the
        // wrong model, so pin the two aliases and the pass-through (a full id / future model is used
        // verbatim, never mangled).
        assert_eq!(resolve_model("opus"), "us.anthropic.claude-opus-4-8[1m]");
        assert_eq!(resolve_model("fable"), "us.anthropic.claude-fable-5[1m]");
        // A non-alias (already a full id, or an unknown/future model) passes through unchanged.
        assert_eq!(
            resolve_model("us.anthropic.claude-opus-4-8[1m]"),
            "us.anthropic.claude-opus-4-8[1m]"
        );
        assert_eq!(resolve_model("some-future-model"), "some-future-model");
        assert_eq!(resolve_model(""), "");
    }

    #[test]
    fn agent_from_roster_derives_branch_worktree_and_disallow_ask() {
        let fleet = Fleet {
            root: PathBuf::from("/hub/.claude/fleet"),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        let entry = |name: &str, role: &str| RosterEntry {
            name: name.into(),
            role: role.into(),
            vertical: String::new(),
            area: String::new(),
            interval: "10m".into(),
            model: "opus".into(),
            effort: "high".into(),
        };

        // A vertical/fix agent: branch = fleet/<name>, worktree = worktrees/<name>, active.
        let v = agent_from_roster(&fleet, &entry("v-fleet-tooling", "vertical"));
        assert_eq!(v.branch, "fleet/v-fleet-tooling");
        assert_eq!(v.worktree, "/hub/.claude/worktrees/v-fleet-tooling");
        assert_eq!(v.status, "active");

        // pr-sync is the ONE agent on `trunk` itself (it integrates there), not a fleet/ branch.
        let ps = agent_from_roster(&fleet, &entry("pr-sync", "pr-sync"));
        assert_eq!(ps.branch, "trunk");

        // SAFETY INVARIANT: disallow_ask is FALSE only for the interactive roles (concierge/design) —
        // they may pop an AskUserQuestion to the human. EVERY other role is unattended, so it MUST be
        // denied that tool (window.sh passes --disallowedTools AskUserQuestion), else an unattended
        // agent could block its window forever on a human prompt.
        assert!(!agent_from_roster(&fleet, &entry("concierge", "concierge")).disallow_ask);
        assert!(!agent_from_roster(&fleet, &entry("design", "design")).disallow_ask);
        for unattended in [
            "vertical",
            "fix",
            "pr-sync",
            "breaker",
            "fuzzer",
            "corpus-bugfix",
        ] {
            assert!(
                agent_from_roster(&fleet, &entry("x", unattended)).disallow_ask,
                "role {unattended:?} is unattended and MUST disallow AskUserQuestion"
            );
        }
    }

    #[test]
    fn count_dir_counts_matching_top_level_files_only() {
        // count_dir backs the board's inbox-depth (`N msg`). Its invariants matter: it must count only
        // immediate FILES matching `keep`, and must NOT descend into subdirs — the inbox holds a
        // `processed/` archive, and counting it would massively over-report the live backlog.
        // Unique per-process dir so concurrent `cargo test` runs (or a prior crashed run) can't collide
        // on a shared fixed name (PR #444). pid is stable within this test process and distinct across
        // concurrent ones.
        let base = std::env::temp_dir().join(format!("cdz-count-dir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base); // clean any stale dir from a prior run of THIS pid
        std::fs::create_dir_all(&base).unwrap();
        // Two live .json messages + a non-.json file (should be skipped by the `keep` predicate).
        std::fs::write(base.join("000000000001-1-merge-request.json"), "{}").unwrap();
        std::fs::write(base.join("000000000001-2-note.json"), "{}").unwrap();
        std::fs::write(base.join("README.txt"), "x").unwrap();
        // A processed/ subdir with its own .json files — count_dir must NOT descend into it.
        std::fs::create_dir_all(base.join("processed")).unwrap();
        std::fs::write(base.join("processed/000000000001-0-old.json"), "{}").unwrap();
        std::fs::write(base.join("processed/000000000001-9-old.json"), "{}").unwrap();

        // Only the two top-level .json files count (not README.txt, not the subdir or its contents).
        assert_eq!(count_dir(&base, |f| f.ends_with(".json")), 2);
        // A predicate that matches nothing → 0 (not an error).
        assert_eq!(count_dir(&base, |f| f.ends_with(".sexp")), 0);
        // A missing dir → 0, never a panic (an agent with no inbox yet reads as empty).
        assert_eq!(count_dir(&base.join("does-not-exist"), |_| true), 0);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn roster_entry_json_excludes_ephemeral_and_omits_empty_fields() {
        let agent = |name: &str, role: &str, status: &str, vertical: &str, area: &str| Agent {
            name: name.into(),
            role: role.into(),
            vertical: vertical.into(),
            area: area.into(),
            worktree: format!("/wt/{name}"),
            branch: format!("fleet/{name}"),
            interval: "10m".into(),
            model: "opus".into(),
            effort: "high".into(),
            status: status.into(),
            disallow_ask: true,
        };

        // An active vertical is persisted, with its vertical+area fields present.
        let line = roster_entry_json(&agent(
            "v-iterators",
            "vertical",
            "active",
            "iterators",
            "rcdzc",
        ))
        .expect("active vertical is persisted");
        assert!(line.contains("\"name\": \"v-iterators\""));
        assert!(line.contains("\"role\": \"vertical\""));
        assert!(line.contains("\"vertical\": \"iterators\""));
        assert!(line.contains("\"area\": \"rcdzc\""));
        assert!(line.contains("\"model\": \"opus\""));

        // EXCLUSION invariant: ephemeral roles must NOT be persisted (else `fleet up` revives dead/
        // transient agents). fix = minted-per-bug; design = interactive/hand-started.
        assert!(roster_entry_json(&agent("fix-x", "fix", "active", "", "")).is_none());
        assert!(roster_entry_json(&agent("design", "design", "active", "", "")).is_none());
        // A non-active agent (stopped/removed) is not persisted either.
        assert!(roster_entry_json(&agent("v-old", "vertical", "stopped", "", "")).is_none());

        // Field omission: an agent with no vertical/area omits those keys (minimal, diff-stable file).
        let bare = roster_entry_json(&agent("pr-sync", "pr-sync", "active", "", ""))
            .expect("active pr-sync is persisted");
        assert!(
            !bare.contains("\"vertical\""),
            "empty vertical must be omitted: {bare}"
        );
        assert!(
            !bare.contains("\"area\""),
            "empty area must be omitted: {bare}"
        );
    }

    #[test]
    fn parse_left_right_count_maps_to_ahead_behind() {
        // git `rev-list --left-right --count origin/main...trunk` prints LEFT<tab>RIGHT where LEFT =
        // origin/main-only (trunk BEHIND) and RIGHT = trunk-only (trunk AHEAD). The board relies on
        // this order — a flip would mislabel publish state — so pin (ahead, behind) explicitly.
        assert_eq!(parse_left_right_count("3\t5"), Some((5, 3))); // 3 behind, 5 ahead
        assert_eq!(parse_left_right_count("0\t0"), Some((0, 0))); // in sync
        assert_eq!(parse_left_right_count("0\t7"), Some((7, 0))); // only ahead (unpublished)
        assert_eq!(parse_left_right_count("4\t0"), Some((0, 4))); // only behind
        // Space-separated (git uses a tab, but be lenient) parses the same.
        assert_eq!(parse_left_right_count("2 6"), Some((6, 2)));
        assert_eq!(parse_left_right_count("  3\t5\n"), Some((5, 3))); // surrounding whitespace ok
        // Malformed / short / non-numeric → None (never a bogus (0,0) that reads as "in sync").
        assert!(parse_left_right_count("").is_none());
        assert!(parse_left_right_count("5").is_none()); // only one count
        assert!(parse_left_right_count("x\ty").is_none()); // non-numeric
        // Trailing tokens are malformed → None (PR #449): the output must be EXACTLY two counts, not
        // two-plus-surplus that we silently ignore.
        assert!(parse_left_right_count("3\t5 extra").is_none());
        assert!(parse_left_right_count("3 5 7").is_none());
    }

    #[test]
    fn next_delivery_seq_is_monotonic_and_durable() {
        // The message filename sorts oldest-first by this sequence, so it MUST increase across calls
        // (and across processes — that's why it's a file, not the process-local Message::seq). Here we
        // verify the durable-counter mechanics within one process using a fresh state dir.
        let root = std::env::temp_dir().join(format!("cdz-delivery-seq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fleet = Fleet {
            root: root.clone(),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };

        // First call starts at 1 (no file yet), then strictly increments and PERSISTS across calls.
        assert_eq!(next_delivery_seq(&fleet), 1);
        assert_eq!(next_delivery_seq(&fleet), 2);
        assert_eq!(next_delivery_seq(&fleet), 3);
        // The value is durable: a fresh Fleet over the same root continues from the stored counter (a
        // NEW process — a separate `fleet send` — must not restart at 1, unlike the old Message::seq).
        let fleet2 = Fleet {
            root: root.clone(),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        assert_eq!(next_delivery_seq(&fleet2), 4);
        // A corrupt/garbage counter file recovers to 1 (never panics, never a bogus huge number).
        std::fs::write(root.join(".delivery-seq"), "not-a-number").unwrap();
        assert_eq!(next_delivery_seq(&fleet), 1);

        // PR #453: the per-writer temp file is renamed away (or cleaned on failure), so no stray
        // `.delivery-seq.*.tmp` lingers after a bump — and the temp name is UNIQUE per call, never the
        // shared fixed name two concurrent writers could clobber.
        let leftover_tmps: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".delivery-seq.") && n.ends_with(".tmp"))
            .collect();
        assert!(
            leftover_tmps.is_empty(),
            "no stray delivery-seq temp should remain, found: {leftover_tmps:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn trunk_reflog_clobber_flags_only_the_origin_main_reset() {
        // The clobber signature: a reset moving trunk backward to origin/main (a non-pr-sync writer).
        assert!(trunk_reflog_entry_is_clobber(
            "reset: moving to origin/main"
        ));
        assert!(trunk_reflog_entry_is_clobber(
            "  reset: moving to origin/main  "
        )); // trimmed
        // pr-sync's legit ops must NOT be flagged: cherry-picks (forward) and its OWN recovery reset
        // to a trunk@{N} reflog point (not origin/main).
        assert!(!trunk_reflog_entry_is_clobber(
            "cherry-pick: rcdzc: some fix"
        ));
        assert!(!trunk_reflog_entry_is_clobber(
            "commit: fleet: mirror the queue"
        ));
        assert!(!trunk_reflog_entry_is_clobber("reset: moving to trunk@{1}"));
        assert!(!trunk_reflog_entry_is_clobber("reset: moving to HEAD~1"));
        // A reset to some other branch/main-ish string isn't THIS clobber (be precise, avoid false +).
        assert!(!trunk_reflog_entry_is_clobber(
            "reset: moving to origin/main-backup"
        ));
        assert!(!trunk_reflog_entry_is_clobber(""));
    }

    #[test]
    fn reference_transaction_hook_is_fail_open() {
        // The hook runs on EVERY ref update in EVERY worktree, so its safety is load-bearing: it must
        // never be able to block/abort a transaction (that would wedge fleet-wide git) and must only
        // observe the post-update state. Pin those invariants on the generated body.
        let body = reference_transaction_hook_body("/hub/.claude/fleet/trunk-clobber.log");
        assert!(body.starts_with("#!/usr/bin/env bash"));
        assert!(body.contains(REF_TXN_HOOK_MARKER)); // so re-install recognizes + overwrites only ours
        // Acts ONLY on the post-update `committed` state (never `prepared`, whose non-zero exit aborts).
        assert!(body.contains("[ \"$state\" = \"committed\" ] || exit 0"));
        assert!(!body.contains("prepared"));
        // Fail-open: the only exits are `exit 0`; there is no non-zero exit anywhere.
        assert!(body.contains("exit 0"));
        assert!(!body.contains("exit 1"));
        // Scoped to trunk, and writes the configured log path — QUOTED in the redirect (PR #458).
        assert!(body.contains("refs/heads/trunk"));
        assert!(body.contains(">> '/hub/.claude/fleet/trunk-clobber.log'"));
        // A log path containing a single quote (or other shell metachars) must be SAFELY quoted — the
        // `'…'` wrap with `'\''` escaping (PR #458), else the redirect breaks or injects into a hook that
        // runs on EVERY fleet-wide ref update. The clean-path assert above never exercises the escape
        // branch, so a mutant dropping `log_path.replace('\'', "'\\''")` would survive it. Pin the escape:
        // `/x/it's/log` must appear as `'/x/it'\''s/log'` in BOTH the redirect uses (the echo + the
        // ancestry-walk appends all share the one `{q}`), and the raw unescaped `it's` must NOT leak.
        let esc = reference_transaction_hook_body("/x/it's/log");
        assert!(
            esc.contains(">> '/x/it'\\''s/log'"),
            "a single-quote in the log path must be shell-escaped as '\\'' in the redirect, got:\n{esc}"
        );
        assert!(
            !esc.contains(">> '/x/it's/log'"),
            "the raw unescaped single-quote path must NOT appear (that redirect would be broken/injectable)"
        );
        // Uses a descendant check to classify a backward (non-fast-forward) move.
        assert!(body.contains("merge-base --is-ancestor"));
        // Guards BOTH creation (old all-zeros) and deletion (new all-zeros) — PR #458.
        assert!(body.contains("case \"$old\" in"));
        assert!(body.contains("case \"$new\" in"));
        // Dropped the always-failing `rev-parse --abbrev-ref $new` (PR #459): $new is a SHA.
        assert!(!body.contains("abbrev-ref"));
        // Captures the parent-command-line ancestry INLINE (the enabler for naming the clobber source).
        assert!(body.contains("ps -o ppid=,args= -p"));
        assert!(body.contains("ppid=$PPID"));
        // Tightened to the CLOBBER SIGNATURE only: logs a non-FF move ONLY when the target is
        // origin/main — so pr-sync's legit --amend/rebase history rewrites (also non-FF) don't spam
        // the log with false positives. Requires `new == origin/main`.
        assert!(body.contains("rev-parse --verify -q origin/main"));
        assert!(body.contains("[ \"$new\" = \"$om\" ] || continue"));
    }

    #[test]
    fn default_model_resolves_to_a_known_alias_not_a_passthrough() {
        // A roster entry with no `model` gets `default_model()`, which window.sh feeds through
        // `resolve_model` → `claude --model`. So the default MUST be a KNOWN alias that resolves to a
        // full 1M-context id — not a bare string that falls through as-is (which would hand `claude` a
        // bogus model name). This pins the cross-fn contract: rename the alias in resolve_model and this
        // fails until default_model agrees.
        let d = default_model();
        let resolved = resolve_model(&d);
        assert_ne!(
            resolved, d,
            "default_model {d:?} must be an ALIAS resolve_model expands, not pass-through"
        );
        assert!(
            resolved.starts_with("us.anthropic.claude-") && resolved.ends_with("[1m]"),
            "default model resolves to a full 1M-context id, got {resolved:?}"
        );
    }

    #[test]
    fn inbox_depth_reports_empty_or_count() {
        // The board's per-agent inbox column: "empty" when no live message, "<n> msg" otherwise. Counts
        // only top-level .json (delegates to count_dir), so a processed/ archive doesn't inflate it.
        let root = std::env::temp_dir().join(format!("cdz-inbox-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fleet = Fleet {
            root: root.clone(),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        // No inbox dir yet → empty (never panics on a missing dir).
        assert_eq!(inbox_depth(&fleet, "v-x"), "empty");
        // One live message + a processed/ archive that must NOT be counted.
        let ib = fleet.inbox("v-x");
        std::fs::create_dir_all(ib.join("processed")).unwrap();
        std::fs::write(ib.join("000000000001-1-note.json"), "{}").unwrap();
        std::fs::write(ib.join("processed/000000000000-0-old.json"), "{}").unwrap();
        assert_eq!(inbox_depth(&fleet, "v-x"), "1 msg");
        std::fs::write(ib.join("000000000002-2-note.json"), "{}").unwrap();
        assert_eq!(inbox_depth(&fleet, "v-x"), "2 msg");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn commits_to_replay_keeps_only_not_yet_upstream() {
        // `git cherry` marks not-yet-upstream commits `+` and already-landed (same patch-id) `-`.
        // Only the `+` shas must be replayed after a reset onto trunk — the `-` ones already landed,
        // INCLUDING a commit pr-sync integrated under a re-parented sha (same patch-id → `-` → dropped).
        let out = "\
+ 1111111111111111111111111111111111111111
- 2222222222222222222222222222222222222222
+ 3333333333333333333333333333333333333333";
        assert_eq!(
            commits_to_replay(out),
            vec![
                "1111111111111111111111111111111111111111".to_string(),
                "3333333333333333333333333333333333333333".to_string(),
            ]
        );
        // All upstream (every commit landed, e.g. via re-parent) → nothing to replay.
        assert!(commits_to_replay("- aaaa\n- bbbb").is_empty());
        // Empty output (branch == trunk) → nothing to replay.
        assert!(commits_to_replay("").is_empty());
        // Order is preserved oldest-first (the order git emits = the cherry-pick order).
        assert_eq!(
            commits_to_replay("+ aaaa\n+ bbbb\n+ cccc"),
            vec!["aaaa", "bbbb", "cccc"]
        );
        // Blank / malformed lines are ignored; only a leading `+ ` counts (a bare `+sha` with no space,
        // or a `-`/other prefix, is not a replay target).
        assert_eq!(
            commits_to_replay("\n+ dddd\n  \n+eeee\nnoise\n- ffff"),
            vec!["dddd".to_string()]
        );
        // A `+ <sha> <subject>` form (some git configs append the subject) keeps just the sha.
        assert_eq!(commits_to_replay("+ 9999 wip: something"), vec!["9999"]);
    }

    #[test]
    fn path_to_zone_maps_unambiguous_prefixes_only() {
        assert_eq!(path_to_zone("xtask/src/fleet.rs"), Some("xtask"));
        assert_eq!(path_to_zone("fleet/roster.json"), Some("xtask")); // fleet source is v-fleet-tooling's
        assert_eq!(path_to_zone(".cargo/config.toml"), Some("xtask")); // build-throttle config is too
        assert_eq!(path_to_zone("guide/src/music/MusicPage.tsx"), Some("guide"));
        assert_eq!(
            path_to_zone("spec/semantics/06-numeric-model.sexp"),
            Some("spec")
        );
        assert_eq!(
            path_to_zone("implementation/compiler-ml/src/sread.cdz"),
            Some("compiler-ml")
        );
        assert_eq!(
            path_to_zone("implementation/music/src/piece.cdz"),
            Some("music")
        );
        assert_eq!(
            path_to_zone("implementation/cad/src/solid.cdz"),
            Some("cad")
        );
        assert_eq!(path_to_zone("implementation/des/src/sim.cdz"), Some("des"));
        assert_eq!(
            path_to_zone("integrations/slack/bridge.rs"),
            Some("integrations")
        );
        // Shared-crate paths are DELIBERATELY unattributable (15+ verticals share area=rcdzc) → None.
        assert_eq!(
            path_to_zone("implementation/seed/crates/rcdzc/src/infer.rs"),
            None
        );
        assert_eq!(
            path_to_zone("implementation/seed/crates/cdz/src/main.rs"),
            None
        );
        assert_eq!(
            path_to_zone("implementation/seed/crates/cadenza-syntax/src/lib.rs"),
            None
        );
        // Other unmapped paths → None (leading/trailing whitespace tolerated).
        assert_eq!(path_to_zone("README.md"), None);
        assert_eq!(path_to_zone("  xtask/src/main.rs "), Some("xtask"));
    }

    #[test]
    fn commit_is_foreign_to_lane_flags_only_unambiguous_cross_zone() {
        let p = |s: &str| s.to_string();
        // All paths in a DIFFERENT zone → foreign (the reported case: guide/music commit on xtask lane).
        assert!(commit_is_foreign_to_lane(
            &[
                p("guide/src/music/MusicPage.tsx"),
                p("guide/scripts/stage-wasm.mjs")
            ],
            "xtask"
        ));
        // A liaison (or any agent) whose zone differs from every changed path → foreign.
        assert!(commit_is_foreign_to_lane(
            &[p("implementation/compiler-ml/src/parse-db.cdz")],
            "xtask"
        ));
        // ANY path in MY zone → not foreign (even mixed with other zones — could be a legit cross-cut).
        assert!(!commit_is_foreign_to_lane(
            &[p("guide/x.tsx"), p("xtask/src/fleet.rs")],
            "xtask"
        ));
        // ANY unattributable path → not foreign (conservative: could be mine; a shared-crate path).
        assert!(!commit_is_foreign_to_lane(
            &[
                p("guide/x.tsx"),
                p("implementation/seed/crates/rcdzc/src/infer.rs")
            ],
            "xtask"
        ));
        // A commit entirely in unmappable paths → not foreign (no known zone at all).
        assert!(!commit_is_foreign_to_lane(
            &[p("implementation/seed/crates/cdz/src/main.rs")],
            "xtask"
        ));
        // Empty zone (couldn't resolve the agent's area) → never foreign.
        assert!(!commit_is_foreign_to_lane(&[p("guide/x.tsx")], ""));
        // No changed paths → never foreign.
        assert!(!commit_is_foreign_to_lane(&[], "xtask"));
        // My own zone's commit → not foreign (the false-positive the reachability version produced).
        assert!(!commit_is_foreign_to_lane(
            &[p("xtask/src/main.rs")],
            "xtask"
        ));
        // A compiler-ml agent seeing a guide commit → foreign (symmetry check, different my_zone).
        assert!(commit_is_foreign_to_lane(
            &[p("guide/x.tsx")],
            "compiler-ml"
        ));
    }

    #[test]
    fn commit_is_ambiguous_to_lane_flags_the_shared_path_case_the_loud_test_misses() {
        let p = |s: &str| s.to_string();
        // THE SYSTEMIC CASE: a peer's rust-async resend touching rcdzc/… (unattributable) + xtask/… ,
        // replayed onto a NON-xtask branch. Loud test is SILENT (unattributable path bails); ambiguous
        // fires because no path is in the syncing agent's lane and one is provably another zone's.
        let rust_async = [
            p("implementation/seed/crates/rcdzc/src/backend/rust/expr.rs"),
            p("xtask/src/main.rs"),
        ];
        assert!(commit_is_ambiguous_to_lane(&rust_async, "music")); // v-music syncing → ambiguous
        assert!(commit_is_ambiguous_to_lane(&rust_async, "guide")); // v-guide syncing → ambiguous
        // But for the agent whose lane IS xtask, that same commit touches its lane → NOT ambiguous
        // (it could legitimately be theirs — stay silent, don't false-flag an own cross-cut).
        assert!(!commit_is_ambiguous_to_lane(&rust_async, "xtask"));
        // Entirely unattributable (a pure shared-crate edit) → NOT ambiguous (could be anyone's; no
        // provably-other-zone path to justify a heads-up).
        assert!(!commit_is_ambiguous_to_lane(
            &[p("implementation/seed/crates/cdz/src/main.rs")],
            "xtask"
        ));
        // A path in my lane anywhere → not ambiguous (provably touches mine).
        assert!(!commit_is_ambiguous_to_lane(
            &[p("guide/x.tsx"), p("xtask/src/fleet.rs")],
            "xtask"
        ));
        // All-in-another-single-zone (the LOUD case) is also ambiguous-true, but the caller reports it
        // under the loud tier first and excludes it here — the helper itself still returns true.
        assert!(commit_is_ambiguous_to_lane(&[p("guide/x.tsx")], "xtask"));
        // Empty zone / no paths → never ambiguous.
        assert!(!commit_is_ambiguous_to_lane(&[p("guide/x.tsx")], ""));
        assert!(!commit_is_ambiguous_to_lane(&[], "xtask"));
    }

    #[test]
    fn base_reparent_warns_only_on_divergence_with_a_replay_set() {
        // The contamination case 5 peers hit: trunk was rebuilt/rewound → the pre-sync base is NO
        // LONGER an ancestor of trunk (base_is_ancestor=false) AND `git cherry` produced a non-empty
        // replay set (the old batch's now-orphaned integrate commits). WARN.
        assert!(base_reparent_should_warn(false, 16));
        assert!(base_reparent_should_warn(false, 1));
        // Diverged but NOTHING to replay (cherry found no unlanded commits) → the reset is clean and
        // there is no foreign stack to warn about. Silent.
        assert!(!base_reparent_should_warn(false, 0));
        // Normal fast-forward: the base is still an ancestor of trunk (trunk only moved forward). Any
        // replay set here is the agent's own genuinely-unlanded work — NOT a re-parent. Silent
        // regardless of how many commits replay (the per-commit zone warnings handle those).
        assert!(!base_reparent_should_warn(true, 5));
        assert!(!base_reparent_should_warn(true, 0));
    }

    #[test]
    fn role_never_authors_matches_only_the_non_authoring_support_roles() {
        // Roles that EMPIRICALLY author zero MRs (pure coordination/routing/maintenance) → skip-replay
        // fast-path. Verified `git log trunk | grep 'integrate MR:.*(<role>)'` = 0 for each.
        for role in [
            "reviewer",
            "tracker",
            "librarian",
            "concierge",
            "github-liaison",
            "pr-sync",
        ] {
            assert!(role_never_authors(role), "{role} should be never-authoring");
        }
        // AUTHORING roles must fall through to the normal patch-id replay (they DO produce merge-requests,
        // so their unlanded commits are real and must be preserved). `vertical` is the canonical author;
        // `fix` agents author bug-fix MRs; `corpus-bugfix` spawns fixers that send their own.
        // ⚠ REGRESSION PIN: `breaker` MUST be authoring. Its loop body still says "never send
        // merge-requests", but per the 2026-07-15 operator directive it promotes passing probes as corpus
        // MRs every tick (35 landed on trunk). Listing it silently discarded an unlanded corpus pin on
        // `reset --hard` — the destructive bug this pins against. Membership is EMPIRICAL, never loop text.
        for role in [
            "vertical",
            "fix",
            "corpus-bugfix",
            "breaker",
            "design",
            "fuzzer",
            "",
            "unknown",
        ] {
            assert!(
                !role_never_authors(role),
                "{role} must NOT be never-authoring"
            );
        }
    }

    #[test]
    fn tree_equal_reset_is_lossless_only_on_identical_nonempty_trees() {
        // The re-parent contamination gate: HEAD's tree byte-identical to trunk's → a `reset --hard
        // trunk` drops only orphaned commit objects, never content, so the auto-reset fast-path is safe
        // for an AUTHORING vertical too (the role-derived path can't help it).
        let t = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        assert!(tree_equal_reset_is_lossless(t, t));
        // DIFFERENT trees = genuine own unlanded content → must fall through to replay+warn, never
        // auto-drop. This is the regression pin: a false positive here would silently discard real work.
        assert!(!tree_equal_reset_is_lossless(
            t,
            "0000000000000000000000000000000000000000"
        ));
        // Fail CLOSED on an unresolved tree (empty stdout from a git spawn failure) — an empty==empty
        // must NOT trip the reset, or a git breakage would masquerade as "content-identical".
        assert!(!tree_equal_reset_is_lossless("", ""));
        assert!(!tree_equal_reset_is_lossless("", t));
        assert!(!tree_equal_reset_is_lossless(t, ""));
    }

    #[test]
    fn reject_log_round_trips_and_sha_is_rejected_matches_full_sha() {
        let dir = std::env::temp_dir().join(format!("cdz-reject-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = dir.join("reject-log");
        let a = "a5a94a6827c0148d5b4109ff2f88dea70a8cb2e6";
        let b = "de14d0b9300000000000000000000000000000000";
        // Empty is a no-op (no file spuriously created with a blank line).
        append_reject_log(&log, "");
        append_reject_log(&log, a);
        append_reject_log(&log, b);
        // Dedup: re-appending an existing sha does not grow the log.
        append_reject_log(&log, a);
        let list = read_reject_log(&log);
        assert_eq!(list.len(), 2, "deduped to 2 distinct shas");
        assert!(sha_is_rejected(a, &list));
        assert!(sha_is_rejected(b, &list));
        // A different (e.g. FIXED) sha is NOT flagged — full-sha match, no prefix/patch collision.
        assert!(!sha_is_rejected(
            "ffffffffffffffffffffffffffffffffffffffff",
            &list
        ));
        // Whitespace-trimmed match; empty never matches.
        assert!(sha_is_rejected(&format!("  {a}  "), &list));
        assert!(!sha_is_rejected("", &list));
        // Missing log → empty list, guard never fires (fail-open).
        assert!(read_reject_log(&dir.join("nope")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_commits_in_range_selects_only_out_of_lane_commits() {
        let p = |s: &str| s.to_string();
        let commit = |sha: &str, paths: &[&str]| {
            (
                sha.to_string(),
                paths.iter().map(|s| p(s)).collect::<Vec<_>>(),
            )
        };
        // A merge-request range the load-spike vector produces: one own commit + a stranded peer's.
        let range = vec![
            commit("aaaaaaaa", &["xtask/src/fleet.rs"]), // mine — kept out of the result
            commit("bbbbbbbb", &["implementation/compiler-ml/src/parse-db.cdz"]), // a peer's — flagged
            commit("cccccccc", &["guide/x.tsx", "guide/y.tsx"]), // another peer's — flagged
        ];
        assert_eq!(
            foreign_commits_in_range(&range, "xtask"),
            vec!["bbbbbbbb".to_string(), "cccccccc".to_string()]
        );
        // An all-in-lane range → nothing flagged (a normal MR must never be refused).
        let clean = vec![
            commit("dddddddd", &["xtask/src/main.rs"]),
            commit("eeeeeeee", &["fleet/roster.json"]),
        ];
        assert!(foreign_commits_in_range(&clean, "xtask").is_empty());
        // A mixed commit (touches my lane AND a peer's) is NOT foreign — could be a legit cross-cut.
        let mixed = vec![commit("ffffffff", &["xtask/x.rs", "guide/y.tsx"])];
        assert!(foreign_commits_in_range(&mixed, "xtask").is_empty());
        // A commit with an unattributable (shared-crate) path stays in-lane-conservative → not flagged.
        let ambiguous = vec![commit(
            "99999999",
            &["implementation/seed/crates/rcdzc/src/infer.rs"],
        )];
        assert!(foreign_commits_in_range(&ambiguous, "xtask").is_empty());
        // Empty range → empty result (no MR content to judge).
        assert!(foreign_commits_in_range(&[], "xtask").is_empty());
    }

    #[test]
    fn parse_context_pct_extracts_the_status_line_percent() {
        assert_eq!(parse_context_pct("... 97% context used ..."), Some(97));
        assert_eq!(parse_context_pct("100% context"), Some(100));
        assert_eq!(parse_context_pct("  9% context left"), Some(9));
        // No marker → None (not near any threshold / didn't render).
        assert_eq!(parse_context_pct("esc to interrupt"), None);
        assert_eq!(parse_context_pct(""), None);
        // A '% context' with no leading digits is not a valid reading.
        assert_eq!(parse_context_pct("% context"), None);
        // Multiple lines → take the LAST (the live status line is at the bottom; older value lingers up top).
        assert_eq!(
            parse_context_pct("70% context used\n... work ...\n88% context used"),
            Some(88)
        );
        // Clamp a nonsense over-100 reading to 100 (never trust a status line above the wall).
        assert_eq!(parse_context_pct("140% context"), Some(100));
    }

    #[test]
    fn pane_shows_working_counts_both_the_affordance_and_api_retry() {
        // The normal working affordance → working.
        assert!(pane_shows_working(
            "  ⏵⏵ bypass permissions on · esc to interrupt · ← for agents"
        ));
        // API-retry backoff → ALSO working (the breaker-class false drain-stall this fixes): a 429
        // shows a "Retrying in Ns" line that can replace the affordance, but the loop is alive.
        assert!(pane_shows_working(
            "✻ 429 Too many tokens, please wait · Retrying in 6s · attempt 5/10\n❯ Press up to edit"
        ));
        assert!(pane_shows_working(
            "✻ API error · Retrying in 1s · attempt 2/10"
        ));
        // An idle pane at the bare prompt with NEITHER signal → NOT working (a real idle/stall).
        assert!(!pane_shows_working(
            "❯ \n  ⏵⏵ bypass permissions on (shift+tab to cycle)"
        ));
        assert!(!pane_shows_working(""));
    }

    #[test]
    fn context_saturation_warning_gates_on_threshold() {
        assert!(context_saturation_warning(Some(85), 85)); // exactly at threshold → warn
        assert!(context_saturation_warning(Some(100), 85)); // at the wall → warn
        assert!(context_saturation_warning(Some(97), 85));
        assert!(!context_saturation_warning(Some(84), 85)); // just below → quiet
        assert!(!context_saturation_warning(Some(0), 85));
        assert!(!context_saturation_warning(None, 85)); // no marker → never warn
    }

    #[test]
    fn watchdog_log_line_writes_only_when_notable() {
        // A fully clean sweep → None (no log line; the log records anomalies only) — even with a nonzero
        // `checked` denominator, which is CONTEXT, not an anomaly, so it must NOT make a clean sweep notable.
        assert_eq!(
            watchdog_log_line(1000, 42, &WatchdogCounts::default()),
            None
        );
        // ANY nonzero counter → a tab-separated summary stamped with `now`, `checked` first after `now`.
        assert_eq!(
            watchdog_log_line(
                1700000000,
                42,
                &WatchdogCounts {
                    rearmed: 2,
                    reissued: 1,
                    reaped: 1,
                    drain_stalls: 0,
                    saturated: 3,
                    queued_but_landed: 5,
                    wedge_escalations: 1,
                    leases_reaped: 4,
                }
            ),
            Some(
                "1700000000\tchecked=42\trearmed=2\treissued=1\treaped=1\tdrain_stalls=0\tsaturated=3\tqueued_but_landed=5\twedge_escalations=1\tleases_reaped=4"
                    .to_string()
            )
        );
        // A single drain-stall is notable on its own.
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    drain_stalls: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        // A single saturated agent is notable on its own.
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    saturated: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        // A wedge escalation alone is notable (even with no other anomaly).
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    wedge_escalations: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        // A leaked-lease reap alone is notable (the out-of-band sweep reclaimed a fleet-stalling lease).
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    leases_reaped: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        // A `reissued` alone is notable — pins the exhaustive-gate contract so `reissued` (or any counter)
        // can't be dropped from the notability check while still living in the struct/format.
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    reissued: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        // `rearmed`, `reaped`, and `queued_but_landed` each notable ON THEIR OWN. The multi-counter case
        // above sets several counters at once, so it can't catch a mutant that DROPS one `== 0` conjunct
        // from the notability gate (with the other counters still nonzero the gate stays false → Some
        // either way). Only a "this counter alone is notable" pin kills that mutant — same reason the
        // `reissued`-alone case exists. These three were the gate conjuncts left unpinned.
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    rearmed: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    reaped: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
        assert!(
            watchdog_log_line(
                1,
                42,
                &WatchdogCounts {
                    queued_but_landed: 1,
                    ..Default::default()
                }
            )
            .is_some()
        );
    }

    #[test]
    fn should_notify_saturation_escalates_pre_wall_and_earlier_for_pr_sync() {
        // General agent: escalate at/above the PRE-WALL threshold (95), while /compact still submits —
        // NOT only at 100 (which was too late). Not recently notified, non-concierge.
        assert!(should_notify_saturation("v-inference", Some(95), false));
        assert!(should_notify_saturation("v-inference", Some(99), false));
        assert!(should_notify_saturation("v-inference", Some(100), false)); // at-wall still escalates
        // Below the general pre-wall bound → not yet (the 85% warn level only logs; self-compact expected).
        assert!(!should_notify_saturation("v-inference", Some(94), false));
        assert!(!should_notify_saturation("v-inference", Some(85), false));

        // pr-sync escalates EARLIER (92) — the single writer, whose wedge stalls the whole queue.
        assert!(
            should_notify_saturation("pr-sync", Some(92), false),
            "pr-sync escalates at its lower 92% pre-wall bound"
        );
        assert!(
            should_notify_saturation("pr-sync", Some(94), false),
            "pr-sync at 94 (below the general 95) still escalates"
        );
        assert!(
            !should_notify_saturation("pr-sync", Some(91), false),
            "just below pr-sync's 92 bound → not yet"
        );

        // No reading → nothing to escalate.
        assert!(!should_notify_saturation("v-inference", None, false));
        // Rate-limited (notified within grace) → suppress, even past the wall.
        assert!(!should_notify_saturation("v-inference", Some(100), true));
        // The concierge is NEVER auto-notified about its OWN wedge (a note it can't read) — the caller
        // handles a concierge wedge with a loud operator-facing stderr line instead.
        assert!(!should_notify_saturation("concierge", Some(100), false));
    }

    #[test]
    fn prewall_threshold_is_lower_for_the_single_writer_pr_sync() {
        // pr-sync (the sole `trunk` writer) escalates EARLIER than everyone else: its wedge stalls the
        // whole merge queue, so it's worth more margin below the 100% wall. This pins the two constants
        // AND their ordering invariant — a future edit that swaps them, drops the special-case, or makes
        // the general bound the lower of the two (inverting the single-writer margin) fails here.
        assert_eq!(
            prewall_threshold_for("pr-sync"),
            CTX_PREWALL_THRESHOLD_PRSYNC
        );
        assert_eq!(prewall_threshold_for("v-inference"), CTX_PREWALL_THRESHOLD);
        assert_eq!(prewall_threshold_for("concierge"), CTX_PREWALL_THRESHOLD);
        assert_eq!(prewall_threshold_for(""), CTX_PREWALL_THRESHOLD);
        // The margin invariant (pr-sync STRICTLY below the general bound, both below the 100% wall) is now
        // guarded at COMPILE TIME by the `const _: () = assert!(…)` pair beside the const definitions —
        // stronger than a runtime assert (fails the build, not a test run) and clippy-clean. This test
        // keeps only the per-agent DISPATCH pins above (which the compile-time guard can't express).
    }

    #[test]
    fn decide_drain_nudge_is_hard_guarded() {
        // churn_grace = 900, stuck_grace = 180 (representative of the real constants).
        let (cg, sg) = (900u64, 180u64);
        // Happy path: enabled + is-a-stall + not saturated + never nudged → fresh nudge.
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), None, cg, sg),
            DrainNudge::Fresh
        );
        assert_eq!(
            decide_drain_nudge(true, true, Some(40), 95, Some("m1"), None, cg, sg),
            DrainNudge::Fresh
        );
        // Opt-in OFF (default) → never nudge, whatever else holds (report-only).
        assert_eq!(
            decide_drain_nudge(false, true, None, 95, Some("m1"), None, cg, sg),
            DrainNudge::Skip
        );
        // Not actually a drain-stall → nothing to nudge.
        assert_eq!(
            decide_drain_nudge(true, false, None, 95, Some("m1"), None, cg, sg),
            DrainNudge::Skip
        );
        // Context-saturated (>= threshold) → needs a RESTART, not a nudge; skip.
        assert_eq!(
            decide_drain_nudge(true, true, Some(95), 95, Some("m1"), None, cg, sg),
            DrainNudge::Skip
        );
        assert_eq!(
            decide_drain_nudge(true, true, Some(100), 95, Some("m1"), None, cg, sg),
            DrainNudge::Skip
        );
        // Just below the saturation threshold → still nudge-able.
        assert_eq!(
            decide_drain_nudge(true, true, Some(94), 95, Some("m1"), None, cg, sg),
            DrainNudge::Fresh
        );
    }

    #[test]
    fn decide_drain_nudge_distinguishes_stuck_from_churn() {
        let (cg, sg) = (900u64, 180u64);
        // SAME message still unconsumed, recently nudged (within stuck_grace) → suppress briefly.
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), Some(("m1", 60)), cg, sg),
            DrainNudge::Skip
        );
        // SAME message, past the SHORT stuck window (but well within the full churn grace) → the nudge
        // didn't stick → re-nudge NOW (this is the v-metaprogramming fix: don't wait out 900s).
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), Some(("m1", 200)), cg, sg),
            DrainNudge::Stuck
        );
        // Stuck EXACT boundary: the suppression is `age < stuck_grace`, so `age == stuck_grace` is
        // already OUT of the short window → re-nudge (Stuck). Pins the `<` (a `<=` mutant would suppress
        // one extra sweep and survive the 60/200 cases above).
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), Some(("m1", 180)), cg, sg),
            DrainNudge::Stuck,
            "age == stuck_grace is past the short window → Stuck (grace is exclusive)"
        );
        // DIFFERENT message (agent drained the old one, a new one arrived) within churn grace → healthy
        // churn, DON'T re-nudge (the full anti-spam window applies to a fresh id).
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m2"), Some(("m1", 60)), cg, sg),
            DrainNudge::Skip
        );
        // DIFFERENT message past the churn grace → a genuinely fresh stall → nudge.
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m2"), Some(("m1", 950)), cg, sg),
            DrainNudge::Fresh
        );
        // Churn EXACT boundary: suppression is `age < churn_grace`, so `age == churn_grace` is already
        // OUT of the full grace → Fresh. Pins the `<` (a `<=` mutant would suppress one extra sweep and
        // survive the 60/950 cases above).
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m2"), Some(("m1", 900)), cg, sg),
            DrainNudge::Fresh,
            "age == churn_grace is past the full grace → Fresh (grace is exclusive)"
        );
        // Legacy marker with an EMPTY recorded id never matches a real filename → treated as churn, not
        // stuck (degrades safely: full grace, no premature "didn't stick" re-nudge).
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), Some(("", 200)), cg, sg),
            DrainNudge::Skip
        );
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, Some("m1"), Some(("", 950)), cg, sg),
            DrainNudge::Fresh
        );
        // The CURRENT flag has NO resolvable id (its filename didn't parse) but we DID nudge before.
        // `same_message` needs `flagged_id` to be Some, so a None current id can never match the prior
        // one → it degrades to the CHURN branch (safe): suppress within the full grace, then re-nudge as
        // Fresh — never a premature "didn't stick" Stuck re-nudge off an unidentifiable flag.
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, None, Some(("m1", 200)), cg, sg),
            DrainNudge::Skip,
            "unresolvable current id within churn grace → suppress (not Stuck)"
        );
        assert_eq!(
            decide_drain_nudge(true, true, None, 95, None, Some(("m1", 950)), cg, sg),
            DrainNudge::Fresh,
            "unresolvable current id past churn grace → Fresh, never Stuck"
        );
    }

    #[test]
    fn is_probable_drain_stall_needs_unconsumed_mail_and_an_idle_pane() {
        // The stall signature: unconsumed ACTIONABLE mail + idle pane, on a non-interactive role.
        assert!(is_probable_drain_stall("vertical", 1, true));
        assert!(is_probable_drain_stall("fix", 3, true));
        // No actionable mail (informational-only, or empty) → nothing to DO → not a stall (legit idle).
        assert!(!is_probable_drain_stall("vertical", 0, true));
        // Pane busy (mid-tick) → it may be about to drain → not flagged.
        assert!(!is_probable_drain_stall("vertical", 2, false));
        // Interactive roles legitimately sit idle with mail a human reads → never flagged.
        assert!(!is_probable_drain_stall("concierge", 5, true));
        assert!(!is_probable_drain_stall("design", 5, true));
        // A non-interactive role with actionable mail + idle IS flagged even if it's pr-sync (it should
        // be draining merge-requests; idle-with-queued-MRs is worth surfacing).
        assert!(is_probable_drain_stall("pr-sync", 4, true));
    }

    #[test]
    fn drain_stall_confirm_recapture_clears_a_mid_tick_agent() {
        // Suspected by the first snapshot, but the recheck shows work in flight (it was between two
        // tool-turns, not stalled) → NOT confirmed. This is the pr-sync-mid-batch false-positive the
        // confirming recapture exists to kill.
        assert!(!drain_stall_confirmed(true, true));
        // Suspected AND still no work on the recheck → a genuine stall, confirmed.
        assert!(drain_stall_confirmed(true, false));
        // Never suspected → never confirmed, regardless of the recheck (recheck isn't even taken).
        assert!(!drain_stall_confirmed(false, false));
        assert!(!drain_stall_confirmed(false, true));
    }

    #[test]
    fn queue_is_draining_only_on_a_strict_drop_across_sweeps() {
        // The pr-sync-mid-batch shape the recapture CAN'T catch: its gate batch runs in a detached
        // subprocess, so the pane reads idle-at-prompt on BOTH captures (working_on_recheck stays false)
        // and trunk is flat — yet a shrinking queue proves the loop is draining its own mail.
        assert!(
            queue_is_draining(Some(13), 12),
            "13→12 is a drop → draining"
        );
        assert!(queue_is_draining(Some(5), 0), "drained to empty → draining");
        // Flat or GROWING is not draining — a genuinely wedged agent's queue holds steady or climbs.
        assert!(
            !queue_is_draining(Some(12), 12),
            "flat → not draining (still suspect)"
        );
        assert!(
            !queue_is_draining(Some(12), 15),
            "grew → not draining (mail piling up)"
        );
        // First observed sweep (no prior) can't establish a drop → NOT exonerated, so a truly wedged
        // agent still gets flagged the first sweep we see it (no free pass on first sight).
        assert!(
            !queue_is_draining(None, 12),
            "no prior depth → cannot claim draining"
        );
        assert!(
            !queue_is_draining(None, 0),
            "no prior depth, even at zero → not a drop"
        );
        // And the draining exoneration composes with the confirm: the call site passes
        // `working_on_recheck || queue_draining` as the exoneration arg. With an idle recheck
        // (working_on_recheck = false), that reduces to queue_draining — so a dropping queue clears the
        // stall and a flat queue leaves it confirmed.
        assert!(
            !drain_stall_confirmed(true, queue_is_draining(Some(13), 12)),
            "suspected + idle-recheck but queue dropped 13→12 → cleared, not a stall"
        );
        assert!(
            drain_stall_confirmed(true, queue_is_draining(Some(12), 12)),
            "suspected + idle-recheck + flat queue → still a confirmed stall"
        );
    }

    #[test]
    fn coldstart_verdict_grace_boundary_is_inclusive() {
        // Window = 1200s (2× a 600s stale window) throughout.
        // Never marked first-seen → mark it now and start the clock (no re-arm yet).
        assert_eq!(coldstart_verdict(None, 1200), ColdStartVerdict::MarkAndWait);
        // Well within the grace → still booting, keep waiting.
        assert_eq!(
            coldstart_verdict(Some(300), 1200),
            ColdStartVerdict::StillWaiting
        );
        // EXACTLY at the window is still booting (grace is inclusive, `<=`) — a first tick may spend the
        // whole window building before it ever stamps. Pins the `<=`/`<` mutation on the grace edge.
        assert_eq!(
            coldstart_verdict(Some(1200), 1200),
            ColdStartVerdict::StillWaiting
        );
        // One second past the window with still no heartbeat → cold start FAILED, re-arm; the verdict
        // carries the first-seen age forward as the staleness measure.
        assert_eq!(
            coldstart_verdict(Some(1201), 1200),
            ColdStartVerdict::Failed(1201)
        );
    }

    #[test]
    fn message_kind_actionability_splits_work_from_fyi() {
        // Actionable — the recipient must DO something.
        for k in [
            "assign",
            "reject",
            "ask",
            "issue",
            "request",
            "merge-request",
            "answer",
            "redelivery",
        ] {
            assert!(message_kind_is_actionable(k), "{k} should be actionable");
        }
        // Informational — read-and-archive, no work. Iterate the const so this stays tied to the
        // single source of truth the CLI legend also derives from (can't drift — the PR#506 fix).
        for k in INFORMATIONAL_KINDS {
            assert!(
                !message_kind_is_actionable(k),
                "{k} should be informational"
            );
        }
        // The const is exactly the informational set (pins its membership so a stray add is caught).
        assert_eq!(
            INFORMATIONAL_KINDS,
            &["note", "merged", "backlog", "status", "reply"]
        );
        // Unknown kind → fail-safe to ACTIONABLE (never silently swallow real work).
        assert!(message_kind_is_actionable("some-new-kind"));
    }

    #[test]
    fn message_kind_from_filename_parses_the_full_kind_after_seq_and_pid() {
        // The kind KEEPS its embedded dash — `merge-request`, not the last segment `request`.
        assert_eq!(
            message_kind_from_filename("000000002045-3804702-merge-request.json"),
            Some("merge-request")
        );
        assert_eq!(
            message_kind_from_filename("000000001905-4025075-merged.json"),
            Some("merged")
        );
        assert_eq!(
            message_kind_from_filename("000000000652-1961391-note.json"),
            Some("note")
        );
        // A non-bus .json (no seq/pid structure) → None, so a stray `registry.json` in an inbox dir is
        // NOT miscounted as an actionable message (the bug Copilot flagged on PR #504).
        assert_eq!(message_kind_from_filename("registry.json"), None);
        // Missing the third (kind) field, or a non-numeric seq/pid → None.
        assert_eq!(
            message_kind_from_filename("000000000652-1961391.json"),
            None
        );
        assert_eq!(message_kind_from_filename("abc-123-note.json"), None);
        assert_eq!(
            message_kind_from_filename("000000000652-xyz-note.json"),
            None
        );
        // Not a .json at all → None (never counted).
        assert_eq!(message_kind_from_filename("not-a-message.txt"), None);
        // A trailing-dash bus name has numeric seq+pid but an EMPTY kind field
        // (`<seq>-<pid>-.json` → the splitn(3) third field is ""). The `!kind.is_empty()` guard rejects
        // it → None. Without that guard it would parse to `Some("")`, and since "" is not in
        // INFORMATIONAL_KINDS, `message_kind_is_actionable("")` is true — a garbage file would silently
        // inflate the drain-stall numerator. Pin both halves so a dropped `!kind.is_empty()` is caught.
        assert_eq!(
            message_kind_from_filename("000000000652-1961391-.json"),
            None,
            "empty kind field (trailing dash) is not a valid bus message"
        );
        assert!(
            message_kind_is_actionable(""),
            "guard rationale: an empty kind would count as actionable, so it MUST be rejected upstream"
        );
    }

    #[test]
    fn pre_durable_seq_filename_detects_the_sentinel_leading_field() {
        // Pre-durable-seq: leading field is the process-local seq, always 1 → `000000000001-…`.
        assert!(is_pre_durable_seq_filename(
            "000000000001-2450601-merge-request.json"
        ));
        assert!(is_pre_durable_seq_filename("000000000001-100-note.json"));
        // A real durable-seq filename (any leading field other than the `…0001` sentinel) → not pre-seq,
        // so a recent silent drop (high monotonic seq) is STILL eligible to be flagged as an orphan.
        assert!(!is_pre_durable_seq_filename(
            "000000002207-2976113-merge-request.json"
        ));
        // Guard the exact width: `…0001` must be the full 12-digit leading field, not a prefix of a
        // larger number like `000000000010` or `000000000012`.
        assert!(!is_pre_durable_seq_filename("000000000010-5-note.json"));
        assert!(!is_pre_durable_seq_filename("000000000012-5-note.json"));
    }

    #[test]
    fn actionable_inbox_depth_ignores_informational_mail() {
        let tmp = std::env::temp_dir().join(format!("vft-actionable-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // 2 informational + 1 actionable.
        for f in [
            "000000000001-10-note.json",
            "000000000002-11-merged.json",
            "000000000003-12-assign.json",
        ] {
            std::fs::write(tmp.join(f), "{}").unwrap();
        }
        assert_eq!(count_dir(&tmp, |f| f.ends_with(".json")), 3);
        assert_eq!(actionable_inbox_depth(&tmp), 1);
        // Add a reject → 2 actionable; a merge-request (embedded dash) also counts.
        std::fs::write(tmp.join("000000000004-13-reject.json"), "{}").unwrap();
        std::fs::write(tmp.join("000000000005-14-merge-request.json"), "{}").unwrap();
        assert_eq!(actionable_inbox_depth(&tmp), 3);
        // A stray non-bus file (no seq/pid structure) must NOT count as actionable (PR #504 fix): the
        // old last-dash-segment parse read `registry.json` → `Some("registry")` → a false actionable.
        std::fs::write(tmp.join("registry.json"), "{}").unwrap();
        assert_eq!(
            actionable_inbox_depth(&tmp),
            3,
            "a stray non-bus file must not be counted"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn oldest_actionable_inbox_message_skips_older_informational_mail() {
        // The drain-nudge stuck-identity must track the oldest ACTIONABLE message (the item the
        // drain-stall signal fires on), NOT the oldest message of any kind — else a stale un-archived
        // note older than the real actionable item hijacks the identity, producing false `Stuck`
        // re-nudges (or mis-graced real stalls) in `decide_drain_nudge`.
        let root =
            std::env::temp_dir().join(format!("cdz-oldest-actionable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fleet = Fleet {
            root: root.clone(),
            worktrees: PathBuf::from("/hub/.claude/worktrees"),
            repo: PathBuf::from("/hub"),
            src: PathBuf::from("/wt/fleet"),
        };
        // Empty inbox (missing dir) → None, never panics.
        assert_eq!(oldest_actionable_inbox_message(&fleet, "v-x"), None);
        let ib = fleet.inbox("v-x");
        std::fs::create_dir_all(&ib).unwrap();
        // Oldest message is an INFORMATIONAL note (seq 1); the actionable reject arrives later (seq 3).
        std::fs::write(ib.join("000000000001-10-note.json"), "{}").unwrap();
        std::fs::write(ib.join("000000000002-11-merged.json"), "{}").unwrap();
        std::fs::write(ib.join("000000000003-12-reject.json"), "{}").unwrap();
        // The flagged id must be the reject — the actionable item — not the older note.
        assert_eq!(
            oldest_actionable_inbox_message(&fleet, "v-x").as_deref(),
            Some("000000000003-12-reject.json"),
            "must skip the older informational note and flag the oldest actionable message"
        );
        // A still-older actionable message (seq 0) becomes the flagged id (oldest-first among actionable).
        std::fs::write(ib.join("000000000000-9-assign.json"), "{}").unwrap();
        assert_eq!(
            oldest_actionable_inbox_message(&fleet, "v-x").as_deref(),
            Some("000000000000-9-assign.json")
        );
        // With only informational mail, there is no actionable item → None (no false stuck-identity).
        let ib2 = fleet.inbox("v-y");
        std::fs::create_dir_all(&ib2).unwrap();
        std::fs::write(ib2.join("000000000001-1-note.json"), "{}").unwrap();
        std::fs::write(ib2.join("000000000002-2-merged.json"), "{}").unwrap();
        assert_eq!(oldest_actionable_inbox_message(&fleet, "v-y"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sort_inbox_filenames_is_oldest_first_by_delivery_seq() {
        // Leading field = zero-padded durable delivery seq, so lexicographic sort == arrival order.
        let mut names = vec![
            "000000000897-1947941-merged.json".to_string(),
            "000000000889-1947120-merged.json".to_string(),
            "000000001018-2538926-merge-request.json".to_string(),
            "000000000897-100-note.json".to_string(),
        ];
        sort_inbox_filenames(&mut names);
        assert_eq!(
            names,
            vec![
                "000000000889-1947120-merged.json".to_string(),
                "000000000897-100-note.json".to_string(),
                "000000000897-1947941-merged.json".to_string(),
                "000000001018-2538926-merge-request.json".to_string(),
            ]
        );
        // Idempotent + total on empty/singleton.
        let mut empty: Vec<String> = vec![];
        sort_inbox_filenames(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn cherry_says_landed_only_when_all_lines_are_upstream() {
        // `-` = patch upstream (landed). A single `-` line → landed.
        assert!(cherry_says_landed(
            "- 1111111111111111111111111111111111111111"
        ));
        // `+` = not upstream → not landed.
        assert!(!cherry_says_landed(
            "+ 1111111111111111111111111111111111111111"
        ));
        // Empty output (ref == trunk / nothing to compare) is NOT a landed proof — conservative false.
        assert!(!cherry_says_landed(""));
        assert!(!cherry_says_landed("\n  \n"));
        // Multiple lines: landed only if EVERY line is `-`.
        assert!(cherry_says_landed("- aaaa\n- bbbb"));
        assert!(!cherry_says_landed("- aaaa\n+ bbbb"));
        // Unexpected prefix (not `- `/`+ `) → don't claim landed.
        assert!(!cherry_says_landed("aaaa"));
        assert!(!cherry_says_landed("- aaaa\nnoise"));
        // Blank lines interspersed are tolerated as long as the real lines are all `-`.
        assert!(cherry_says_landed("\n- aaaa\n\n- bbbb\n"));
    }

    #[test]
    fn mr_qualifies_for_landed_check_needs_a_ref_and_a_live_sender() {
        // The pure gate `find_queued_but_landed` uses to decide whether a queued MR is worth a
        // `git cherry`: BOTH a non-empty ref AND a still-active sender. Pin each condition so dropping
        // either arm (`&&`→ short-circuit either way) is caught.
        let active: std::collections::HashSet<String> = ["v-fleet-tooling", "pr-sync"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Non-empty ref + active sender → check it.
        assert!(mr_qualifies_for_landed_check(
            "deadbeef",
            "v-fleet-tooling",
            &active
        ));
        // Empty ref → nothing to compare against trunk → skip (no wasted git call).
        assert!(
            !mr_qualifies_for_landed_check("", "v-fleet-tooling", &active),
            "an empty ref has nothing to cherry-check"
        );
        // Live sender but empty ref stays skipped even so (the ref arm alone must gate).
        assert!(!mr_qualifies_for_landed_check("", "pr-sync", &active));
        // Inactive/stale sender → leftover noise, skip even with a real ref (the active arm alone gates).
        assert!(
            !mr_qualifies_for_landed_check("deadbeef", "corpus-bugfix", &active),
            "a stale one-shot agent's leftover MR is noise, not a live orphan"
        );
    }

    #[test]
    fn queued_ref_would_orphan_detects_a_to_be_reshaed_sent_ref() {
        let mr = |from: &str, r#ref: &str| Message {
            from: from.into(),
            to: "pr-sync".into(),
            kind: "merge-request".into(),
            subject: "x".into(),
            r#ref: r#ref.into(),
            body: String::new(),
            seq: 1,
            in_reply_to: String::new(),
        };
        let replay = vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        ];

        // A queued MR from me naming a to-be-replayed commit → would orphan (returns that ref).
        let msgs = vec![mr("v-x", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
        assert_eq!(
            queued_ref_would_orphan(&replay, "v-x", &msgs),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
        );

        // Abbreviated sent ref still matches (prefix, either direction).
        let msgs = vec![mr("v-x", "bbbbbbb")];
        assert_eq!(
            queued_ref_would_orphan(&replay, "v-x", &msgs),
            Some("bbbbbbb".to_string())
        );

        // UPPERCASE hand-entered ref must still match the lowercase replay sha (PR #473 — git commit
        // IDs are case-insensitive; a case-sensitive compare would MISS the orphan and drop the MR).
        let msgs = vec![mr("v-x", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")];
        assert_eq!(
            queued_ref_would_orphan(&replay, "v-x", &msgs),
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string())
        );
        // Mixed-case abbreviated ref, too.
        let msgs = vec![mr("v-x", "BbBbBbB")];
        assert_eq!(
            queued_ref_would_orphan(&replay, "v-x", &msgs),
            Some("BbBbBbB".to_string())
        );

        // An MR from a DIFFERENT agent naming the same sha is not ours → no orphan.
        let msgs = vec![mr("v-other", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
        assert_eq!(queued_ref_would_orphan(&replay, "v-x", &msgs), None);

        // Our MR names a sha NOT in the replay set (e.g. already landed) → no orphan.
        let msgs = vec![mr("v-x", "cccccccccccccccccccccccccccccccccccccccc")];
        assert_eq!(queued_ref_would_orphan(&replay, "v-x", &msgs), None);

        // A non-merge-request message from us (e.g. a note) is ignored even if its ref matches.
        let mut note = mr("v-x", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        note.kind = "note".into();
        assert_eq!(queued_ref_would_orphan(&replay, "v-x", &[note]), None);

        // Empty ref / empty inbox → no orphan.
        assert_eq!(
            queued_ref_would_orphan(&replay, "v-x", &[mr("v-x", "")]),
            None
        );
        assert_eq!(queued_ref_would_orphan(&replay, "v-x", &[]), None);

        // Nothing to replay → never an orphan (caller also short-circuits on empty replay).
        assert_eq!(
            queued_ref_would_orphan(&[], "v-x", &[mr("v-x", "aaaaaaaa")]),
            None
        );
    }

    #[test]
    fn agent_reads_stale_matches_watchdog_gate() {
        // Window = 600s throughout; a plain vertical ignores the trunk arg entirely.
        // Healthy: heartbeat within the window → not stale.
        assert!(!agent_reads_stale("v-x", "active", true, 300, 600, None));
        // Genuinely stalled: active + live window + heartbeat past the window → stale.
        assert!(agent_reads_stale("v-x", "active", true, 900, 600, None));
        // Exactly at the window is NOT past it (strict `>`), so not stale.
        assert!(!agent_reads_stale("v-x", "active", true, 600, 600, None));
        // A stopped agent never reads stale, however old its heartbeat.
        assert!(!agent_reads_stale("v-x", "stopped", true, 9999, 600, None));
        // No live tmux window → can't be a live-but-stalled loop, so no flag.
        assert!(!agent_reads_stale("v-x", "active", false, 9999, 600, None));
        // The trunk arg is IGNORED for a non-pr-sync agent even if fresh — it stays stale.
        assert!(agent_reads_stale("v-x", "active", true, 900, 600, Some(1)));

        // pr-sync exemption (mirrors `watchdog`): stale heartbeat but a FRESH trunk commit → alive
        // mid-batch, NOT stale.
        assert!(!agent_reads_stale(
            "pr-sync",
            "active",
            true,
            1500,
            600,
            Some(120)
        ));
        // pr-sync with a STALE trunk commit too (nothing landed within the window) → genuinely stalled.
        assert!(agent_reads_stale(
            "pr-sync",
            "active",
            true,
            1500,
            600,
            Some(1200)
        ));
        // pr-sync with an UNKNOWN trunk age (None) falls back to the heartbeat gate → stale.
        assert!(agent_reads_stale(
            "pr-sync", "active", true, 1500, 600, None
        ));
        // pr-sync ticking within its window is healthy regardless of trunk.
        assert!(!agent_reads_stale(
            "pr-sync",
            "active",
            true,
            120,
            600,
            Some(9999)
        ));
        // Trunk-age EXACTLY at the window is still fresh (exemption is `<=`, mirroring the heartbeat
        // side's inclusive boundary), so pr-sync is exempt. Pins the `<=`/`<` mutation on the trunk
        // arm — without this the `commit_age == window_secs` edge (a commit that landed precisely one
        // window ago) is untested, and flipping to `<` would silently start flagging a live batch.
        assert!(!agent_reads_stale(
            "pr-sync",
            "active",
            true,
            1500,
            600,
            Some(600)
        ));
    }

    #[test]
    fn scan_check_leases_counts_priority_and_vertical_and_reaps_stale() {
        let dir = std::env::temp_dir().join(format!("cdz-check-lease-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_000_000u64;
        // Probe reports every holder alive so this test exercises ONLY the count + TTL paths
        // (the fake pids 111/222/… may or may not exist on the host — don't depend on /proc here).
        let all_alive = |_: &str| Some(true);

        // Two vertical + one priority live lease, plus a non-lease file that must be ignored.
        std::fs::write(dir.join("111-vertical.lease"), "x").unwrap();
        std::fs::write(dir.join("222-vertical.lease"), "x").unwrap();
        std::fs::write(dir.join("333-priority.lease"), "x").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();
        let (prio, vert, reaped) = scan_check_leases_with(&dir, now, all_alive);
        assert_eq!(
            (prio, vert, reaped),
            (1, 2, 0),
            "counts priority vs vertical, ignores non-.lease, reaps nothing when all alive + fresh"
        );

        // A lease whose mtime is older than the TTL is reclaimed (a crashed holder's leaked slot):
        // back-date one file and scan with a `now` far past its mtime + TTL.
        let stale = dir.join("444-vertical.lease");
        std::fs::write(&stale, "x").unwrap();
        let stale_now = file_mtime_unix(&stale).unwrap() + CHECK_LEASE_TTL_SECS + 10;
        let (prio2, vert2, reaped2) = scan_check_leases_with(&dir, stale_now, all_alive);
        // At stale_now ALL four originals are also older than TTL (written ~same time) → only reaping
        // remains; assert the stale one was removed and nothing lingers past the TTL.
        assert!(!stale.exists(), "a lease older than the TTL is reclaimed");
        assert_eq!((prio2, vert2), (0, 0), "everything past the TTL is reaped");
        assert_eq!(reaped2, 4, "all four TTL-stale leases counted as reaped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_check_leases_reaps_dead_pid_lease_immediately_before_ttl() {
        // The bug this fixes: a check proc SIGKILLed (137 under load, or pr-sync preempting a gate)
        // runs no `Drop`, so its lease file LEAKS. A leaked PRIORITY lease then blocks EVERY vertical
        // check for the full 45-min TTL — a fleet-wide check stall. Fix: reap a dead-pid lease NOW.
        let dir = std::env::temp_dir().join(format!("cdz-lease-deadpid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_000_000u64; // fresh mtimes → the TTL path would NOT reap; only pid-liveness can.

        std::fs::write(dir.join("111-priority.lease"), "x").unwrap(); // dead holder → must be reaped
        std::fs::write(dir.join("222-vertical.lease"), "x").unwrap(); // dead holder → must be reaped
        std::fs::write(dir.join("333-vertical.lease"), "x").unwrap(); // live holder → kept

        // Probe: pid 333 alive, 111 & 222 dead, everything else "can't tell" (kept via backstop).
        let probe = |name: &str| match name.split('-').next() {
            Some("333") => Some(true),
            Some("111") | Some("222") => Some(false),
            _ => None,
        };
        let (prio, vert, reaped) = scan_check_leases_with(&dir, now, probe);
        assert_eq!(
            (prio, vert),
            (0, 1),
            "dead-pid leases reaped immediately (well within the TTL); only the live vertical remains"
        );
        assert_eq!(
            reaped, 2,
            "both dead-PID leases (priority 111 + vertical 222) counted as reaped"
        );
        assert!(
            !dir.join("111-priority.lease").exists(),
            "the leaked PRIORITY lease that was stalling all verticals is gone"
        );
        assert!(
            dir.join("333-vertical.lease").exists(),
            "the live lease is kept"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_is_done_routes_resolved_and_rejected_tags_only() {
        use std::path::PathBuf;
        // The real tag variants observed in the queue (tag anywhere in the dotted name).
        assert!(entry_is_done(&PathBuf::from("foo.RESOLVED.sexp")));
        assert!(entry_is_done(&PathBuf::from("bar.sexp.RESOLVED")));
        assert!(entry_is_done(&PathBuf::from("adv-x.REJECTED.md")));
        assert!(entry_is_done(&PathBuf::from(
            "adv-dup.WORKS-AS-SPECIFIED-diag-RESOLVED.sexp"
        )));
        // A subdir doesn't matter — we check the file name only.
        assert!(entry_is_done(&PathBuf::from("assigned/foo.RESOLVED.sexp")));
        // NOT done: open work, other tags, and prose that merely contains the letters.
        assert!(!entry_is_done(&PathBuf::from(
            "witness-generic-driver-tie.cdz"
        )));
        assert!(!entry_is_done(&PathBuf::from("leak-x.CONFIRMED.sexp")));
        assert!(!entry_is_done(&PathBuf::from("des-inc4.PRE-STEP3.sexp")));
        // A word merely CONTAINING "resolved" (not a dotted tag) must NOT match — no false-positive.
        assert!(!entry_is_done(&PathBuf::from(
            "how-resolved-issues-work.md"
        )));
        assert!(!entry_is_done(&PathBuf::from(
            "DIRECTIVE-retire-needs-tag.md"
        )));
    }

    #[test]
    fn proc_stat_zombie_detection_reaps_defunct_lease_holders() {
        // The zombie gap: a SIGKILL'd gate proc lingers as a DEFUNCT `/proc/<pid>` entry (exists, but
        // will never Drop its lease). Its stat state is `Z` — must read as dead so the lease is reaped.
        // A running/sleeping/etc. holder is alive and kept.
        assert!(
            proc_stat_is_zombie("1234 (cargo) Z 1200 1234 1200 0 -1 4194560 0 0"),
            "state Z after the comm → zombie (reap)"
        );
        assert!(
            !proc_stat_is_zombie("1234 (cargo) R 1200 1234 1200 0 -1 4194560 0 0"),
            "state R → running, alive"
        );
        assert!(
            !proc_stat_is_zombie("1234 (cargo) S 1200 1234 1200 0 -1 4194560 0 0"),
            "state S → sleeping, alive"
        );
        // The comm field can contain spaces AND parens — scan to the LAST ')' so the state is read from
        // the right place, not fooled by an embedded `) Z`.
        assert!(
            proc_stat_is_zombie("1234 (weird ) name) Z 1200 1234"),
            "comm with spaces/parens: state after the LAST ) is Z → zombie"
        );
        assert!(
            !proc_stat_is_zombie("1234 (odd) R)ame) R 1200"),
            "comm with an embedded ) then R state → alive (read after the last ))"
        );
        // Unparseable / no paren → false (caller treats a present-but-unparseable pid as alive; TTL backs up).
        assert!(!proc_stat_is_zombie("garbage with no paren"));
        assert!(!proc_stat_is_zombie(""));
    }

    #[test]
    fn reap_check_leases_out_of_band_reclaims_ttl_stale_without_acquiring() {
        // The out-of-band watchdog sweep reaps a leaked (here TTL-stale) lease WITHOUT taking a lease
        // itself — the path that self-heals a dead-PID priority lease even when pr-sync's own wedged
        // gate is the (only, stuck) acquirer, so no fresh acquire-path scan ever runs. Drives the reap
        // core (`reap_check_leases_in`) against a plain temp dir; the enclosing `reap_check_leases` just
        // adds hub/git dir-resolution on top. Uses TTL staleness (not /proc pid-liveness) so the
        // assertion is host-independent.
        let dir = std::env::temp_dir().join(format!("cdz-oob-reap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("999-priority.lease");
        std::fs::write(&stale, "x").unwrap();
        let live = dir.join("888-vertical.lease"); // fresh mtime, can't-tell pid → kept via backstop
        std::fs::write(&live, "x").unwrap();
        // `now` far past the stale file's mtime + TTL → its TTL-reclaim path fires (no pid probe needed);
        // 888 was written at ~the same time, so at this `now` it too is TTL-stale and reaped.
        let now = file_mtime_unix(&stale).unwrap() + CHECK_LEASE_TTL_SECS + 10;
        let reaped = reap_check_leases_in(&dir, now);
        assert_eq!(
            reaped, 2,
            "the out-of-band sweep reaped both TTL-stale leases"
        );
        assert!(
            !stale.exists(),
            "the leaked priority lease is gone — the merge gate is no longer blocked on a dead holder"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reap_stale_gate_checks_sweeps_only_strictly_past_ttl() {
        // The gate-check scaffolding reaper (`.exit`/`.pid`/`.log`/`.sh` keyed by combined-tree id):
        // drive its pure core against a temp dir so the `> TTL` mtime boundary is pinned. Two artifacts
        // written together; choose `now` from each mtime so one lands EXACTLY at TTL (kept — the resume
        // path may still key off it) and one lands one second PAST TTL (swept). Without both, a `>`→`>=`
        // mutant that reaps an exactly-TTL-old artifact would survive.
        let dir = std::env::temp_dir().join(format!("cdz-gate-reap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let at_ttl = dir.join("treeA.exit");
        std::fs::write(&at_ttl, "0").unwrap();
        // `now` exactly one TTL past the artifact's mtime → age == TTL, which is NOT strictly greater →
        // kept. (Both files share ~the same mtime, so pick `now` off `at_ttl` and assert on it alone.)
        let now_at = file_mtime_unix(&at_ttl).unwrap() + GATE_CHECK_TTL_SECS;
        assert_eq!(
            reap_stale_gate_checks_in(&dir, now_at),
            0,
            "age == TTL is not strictly past → nothing swept (boundary is exclusive)"
        );
        assert!(at_ttl.exists(), "the exactly-TTL-old artifact is kept");
        // One second later → age == TTL+1 > TTL → swept.
        assert_eq!(
            reap_stale_gate_checks_in(&dir, now_at + 1),
            1,
            "age == TTL+1 is strictly past → swept"
        );
        assert!(!at_ttl.exists(), "the past-TTL artifact is gone");
        // A missing dir is inert (no panic, nothing swept) — mirrors the read_dir fail-open.
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            reap_stale_gate_checks_in(&dir, now_at + 1),
            0,
            "an absent dir sweeps nothing"
        );
    }

    #[test]
    fn check_lease_go_priority_wins_and_vertical_yields() {
        // PRIORITY (pr-sync merge gate) always goes — even at/over the cap and with priority peers.
        assert!(check_lease_go(true, 0, 0, 3));
        assert!(
            check_lease_go(true, 2, 99, 3),
            "priority never waits behind vertical or the cap"
        );

        // VERTICAL goes only when no priority is held AND under the cap.
        assert!(check_lease_go(false, 0, 0, 3));
        assert!(check_lease_go(false, 0, 2, 3), "under the cap → go");
        assert!(!check_lease_go(false, 0, 3, 3), "at the cap → wait");
        assert!(!check_lease_go(false, 0, 4, 3), "over the cap → wait");
        assert!(
            !check_lease_go(false, 1, 0, 3),
            "a held priority lease makes vertical yield even with free slots"
        );
    }

    #[test]
    fn check_lease_max_reads_env_with_sane_default() {
        // Default is 3; a positive override parses; garbage/zero fall back to the default.
        // (Env is process-global; set+clear around the assert to avoid cross-test bleed.)
        // SAFETY: single-threaded test, no other thread reads the env concurrently.
        unsafe { std::env::remove_var("CDZ_CHECK_LEASE_MAX") };
        assert_eq!(check_lease_max(), 3);
        unsafe { std::env::set_var("CDZ_CHECK_LEASE_MAX", "5") };
        assert_eq!(check_lease_max(), 5);
        unsafe { std::env::set_var("CDZ_CHECK_LEASE_MAX", "0") };
        assert_eq!(check_lease_max(), 3, "zero is rejected → default");
        unsafe { std::env::set_var("CDZ_CHECK_LEASE_MAX", "nonsense") };
        assert_eq!(check_lease_max(), 3, "garbage → default");
        unsafe { std::env::remove_var("CDZ_CHECK_LEASE_MAX") };
    }

    #[test]
    fn parse_exit_marker_maps_exit_code_to_verdict() {
        assert_eq!(parse_exit_marker("0"), Some(true), "exit 0 → green");
        assert_eq!(
            parse_exit_marker("0\n"),
            Some(true),
            "trailing newline tolerated"
        );
        assert_eq!(parse_exit_marker(" 1 "), Some(false), "any non-zero → red");
        assert_eq!(
            parse_exit_marker("143"),
            Some(false),
            "SIGTERM exit → red (not green)"
        );
        assert_eq!(
            parse_exit_marker(""),
            None,
            "not-yet-written → keep polling"
        );
        assert_eq!(
            parse_exit_marker("garbage"),
            None,
            "partial/garbage → keep polling"
        );
    }

    #[test]
    fn read_gate_marker_reuses_a_completed_verdict_and_ignores_an_absent_one() {
        // This is the RESUME path: a completed marker on disk (from a prior, possibly-SIGTERM'd
        // gate-batch invocation) is reused instead of re-running the whole minutes-long check.
        let dir = std::env::temp_dir().join(format!("cdz-gate-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let absent = dir.join("deadbeef.exit");
        assert_eq!(
            read_gate_marker(&absent),
            None,
            "no marker yet → keep polling (not a false verdict)"
        );

        let green = dir.join("cafe.exit");
        std::fs::write(&green, "0").unwrap();
        assert_eq!(
            read_gate_marker(&green),
            Some(true),
            "a completed green marker is reused"
        );

        let red = dir.join("f00d.exit");
        std::fs::write(&red, "143").unwrap();
        assert_eq!(
            read_gate_marker(&red),
            Some(false),
            "a SIGTERM'd check reads RED, never green"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
