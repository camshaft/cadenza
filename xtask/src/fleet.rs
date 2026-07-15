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
        /// Sender name (defaults to `$FLEET_AGENT` if the caller is itself an agent, else `unknown`).
        #[arg(long)]
        from: Option<String>,
        /// Deliver without nudging the recipient's window awake (it will pick the message up on its
        /// next scheduled `/loop` tick). Use when seeding a batch of messages.
        #[arg(long)]
        no_wake: bool,
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
    /// Self-heal the fleet, in two passes. RE-ARM: any ACTIVE agent whose `/loop` has stalled — each
    /// agent stamps a heartbeat touch-file (`.claude/fleet/heartbeat/<agent>`) at the top of every
    /// tick; if that file is older than `min(--stale-mult × interval, --stale-cap)`, its loop is
    /// presumed dead and this nudges the window back to life (`tmux send-keys "/loop <interval>" Enter`
    /// — the same recovery an operator does by hand). The `--stale-cap` bound is what keeps a
    /// long-interval agent (e.g. 30m) from getting an hour-long dead window. Skips: agents with no live
    /// tmux window, agents mid-tick
    /// ("esc to interrupt" — real work in flight), and agents re-armed within `--grace-secs`
    /// (anti-thrash). REAP: any genuinely-DONE agent (registry status=stopped AND a stop-file present)
    /// whose tmux window is still live gets that window killed — role-agnostic, so it catches design/
    /// self-removed agents the PM's `remove --close` reaper never gets a note about. The registry row
    /// is kept (history/archive); only the panel goes away, and a `--grace-secs` window off the stop
    /// keeps a just-stopped agent's final scrollback glanceable for one cycle. Meant to run from a
    /// short cron (~every 4 min); one pass then exit. This is the fleet's reliability backbone — a
    /// fleet-wide `/loop` stall froze every agent once, and stopped windows otherwise pile up.
    Watchdog {
        /// Report what WOULD be re-armed/reaped, but send no keys and kill no windows (safe anytime).
        #[arg(long)]
        dry_run: bool,
        /// Presume a loop stalled once its heartbeat is older than this multiple of its interval.
        #[arg(long, default_value_t = 2)]
        stale_mult: u32,
        /// Hard CAP (seconds) on the stale window, regardless of interval × mult. The interval is how
        /// often a HEALTHY agent wants to tick; the stale window is how long we tolerate SILENCE before
        /// presuming death — they must not scale together unboundedly, or a 30m agent gets a 60min
        /// dead window (2×30m) and sits stalled for an hour. A heartbeat is stamped at the TOP of every
        /// tick, so >~10min of silence means stalled no matter the interval. Default 600s (10 min).
        #[arg(long, default_value_t = 600)]
        stale_cap: u64,
        /// Grace window (seconds), used two ways: don't re-arm an agent re-armed this recently (gives
        /// the nudge time to land), and don't reap a window whose agent stopped this recently (keeps
        /// its final scrollback glanceable for one cycle).
        #[arg(long, default_value_t = 120)]
        grace_secs: u64,
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
    /// A monotonic ordinal (not wall-clock — the toolchain forbids `Date::now`); makes filenames
    /// sort in send order within a run. Combined with pid for cross-process uniqueness.
    seq: u64,
}

pub fn run(paths: &Paths, cmd: FleetCmd) {
    let fleet = Fleet::new(paths);
    match cmd {
        FleetCmd::Up => up(&fleet),
        FleetCmd::Down => down(&fleet),
        FleetCmd::Status => status(&fleet),
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
        } => send(&fleet, &to, &kind, &subject, &r#ref, &body, from, no_wake),
        FleetCmd::Heartbeat { name } => heartbeat(&fleet, &name),
        FleetCmd::Describe { name } => describe(&fleet, &name),
        FleetCmd::Archive { no_commit } => archive(&fleet, no_commit),
        FleetCmd::Watchdog {
            dry_run,
            stale_mult,
            stale_cap,
            grace_secs,
        } => watchdog(&fleet, dry_run, stale_mult, stale_cap, grace_secs),
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

fn status(fleet: &Fleet) {
    let reg = fleet.load();
    let session = if in_tmux() {
        Some(tmux_current_session())
    } else {
        None
    };
    let live_windows = session.as_deref().map(tmux_windows).unwrap_or_default();

    println!("Fleet board ({} agent(s)):", reg.agents.len());
    println!(
        "  {:<18} {:<13} {:<7} {:<8} {:<7} INBOX",
        "AGENT", "ROLE", "MODEL", "STATUS", "WINDOW"
    );
    for a in &reg.agents {
        let window = if live_windows.iter().any(|w| w == &a.name) {
            "live"
        } else {
            "-"
        };
        let inbox = inbox_depth(fleet, &a.name);
        let role = if a.vertical.is_empty() {
            a.role.clone()
        } else {
            format!("{}:{}", a.role, a.vertical)
        };
        println!(
            "  {:<18} {:<13} {:<7} {:<8} {:<7} {}",
            a.name, role, a.model, a.status, window, inbox
        );
    }

    // Work queue depth (breaker/fuzzer produce, the PM consumes). Count only un-handled items.
    let queue = fleet.root.join("queue");
    let qn = count_dir(&queue, |name| {
        !name.contains(".RESOLVED.") && !name.contains(".REJECTED.")
    });
    println!("\n  queue: {qn} open work item(s) in {}", queue.display());

    // trunk vs origin/main, so the operator sees the publish backlog at a glance.
    if let Some((ahead, behind)) = trunk_vs_origin_main(&fleet.repo) {
        println!("  trunk: {ahead} ahead / {behind} behind origin/main");
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
) {
    let from = from
        .or_else(|| std::env::var("FLEET_AGENT").ok())
        .unwrap_or_else(|| "unknown".to_string());
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
        },
    );
    println!("fleet send: {from} → {to} [{kind}] {subject}");

    // Wake the recipient so it reacts to this message NOW rather than at its next scheduled tick.
    // Delivery == wake. `/loop` stays the safety net for any nudge that doesn't land.
    if !no_wake {
        match wake_window(fleet, to) {
            WakeOutcome::Woke => println!("  ↑ nudged '{to}' awake (immediate tick)"),
            WakeOutcome::Skipped(why) => println!("  (not woken: {why} — picks it up next /loop)"),
        }
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
fn watchdog(fleet: &Fleet, dry_run: bool, stale_mult: u32, stale_cap: u64, grace_secs: u64) {
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
    let mut checked = 0usize;
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

        // Liveness = the heartbeat touch-file's mtime (stamped at the top of every tick). A missing
        // file means the agent has never ticked (freshly launched) — treat its age as "just now" so we
        // give the first tick its full stale window before judging it, rather than nuking a booting agent.
        let hb_age = heartbeat_age_secs(fleet, &a.name, now);
        let interval = parse_interval_secs(&a.interval);
        let stale_after = stale_window_secs(interval, stale_mult, stale_cap);
        let Some(age) = hb_age else {
            continue; // never stamped yet — a booting agent; leave it for the next pass.
        };
        if age <= stale_after {
            continue; // ticked recently — healthy.
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
        if window_is_working(&session, &a.name) {
            println!(
                "  = {} heartbeat stale but pane shows work in flight — left alone",
                a.name
            );
            continue;
        }

        if dry_run {
            println!(
                "  DRY-RUN would re-arm '{}' (idle {age}s > {stale_after}s stale window; interval {})",
                a.name, a.interval
            );
            rearmed += 1;
            continue;
        }
        if rearm_window(&session, &a.name, &a.interval) {
            stamp_rearm(fleet, &a.name);
            rearmed += 1;
            println!(
                "  + re-armed '{}' (idle {age}s > {stale_after}s; sent `/loop {}`)",
                a.name, a.interval
            );
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

    println!(
        "fleet watchdog: checked {checked} active windowed agent(s); {}{rearmed} re-armed, {reaped} stopped window(s) reaped.",
        if dry_run { "DRY-RUN: " } else { "" }
    );
}

/// Seconds since the epoch, from the wall clock. (Unlike the Cadenza toolchain, xtask may read the
/// clock — this is host tooling, not compiled-program logic.)
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

/// A file's modification time as seconds since the epoch, or `None` if it can't be read.
fn file_mtime_unix(path: &Path) -> Option<u64> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
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

/// How long to tolerate heartbeat SILENCE before presuming a loop dead: `interval × mult`, but hard-
/// capped at `cap`. The cap is the key: the interval is a healthy agent's tick CADENCE, while this is
/// our patience for silence — they must not scale together, or a 30m agent gets a 60min dead window.
/// A heartbeat is stamped at the TOP of every tick, so a `cap` of ~10min is a safe "definitely stalled"
/// bound for any interval, and the mult keeps a short-interval agent from being judged too eagerly.
fn stale_window_secs(interval_secs: u64, mult: u32, cap: u64) -> u64 {
    interval_secs.saturating_mul(mult as u64).min(cap)
}

/// Does the agent's tmux pane show Claude actively working? Claude Code prints an "esc to interrupt"
/// affordance in its status line while a turn is in flight; its presence means the loop is alive and
/// mid-tick, so a stale heartbeat is just a long tick — don't re-arm (that would inject a `/loop`
/// into the middle of real work). Captures only the visible pane (no scrollback).
fn window_is_working(session: &str, agent: &str) -> bool {
    let target = format!("{session}:{agent}");
    Command::new("tmux")
        .args(["capture-pane", "-p", "-t", &target])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("esc to interrupt"))
        .unwrap_or(false)
}

/// Re-arm a stalled agent's loop by typing `/loop <interval>` + Enter into its idle pane — exactly
/// the manual recovery an operator does. Two send-keys calls: the literal text (`-l`, so the slash
/// isn't interpreted as a tmux command), then `Enter` to submit. Returns whether both keystrokes were
/// delivered.
fn rearm_window(session: &str, agent: &str, interval: &str) -> bool {
    let target = format!("{session}:{agent}");
    let text = format!("/loop {interval}");
    let sent = Command::new("tmux")
        .args(["send-keys", "-t", &target, "-l", &text])
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

/// Mirror the live gitignored work queue into the TRACKED `issues/` archive so the reproducers are
/// preserved in git history, not just agent-local state. The archive is written into the CURRENT
/// worktree (whoever runs this — pr-sync, on `trunk`), which is where the commit lands. The queue
/// itself is read from the hub's `.claude/fleet/queue` (shared). Mirror semantics: copy every queue
/// entry into `issues/`, delete tracked `issues/` files no longer present in the queue (git history
/// still holds them), then commit unless `--no-commit`.
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
    let queue_set: std::collections::HashSet<PathBuf> = queue_rel.iter().cloned().collect();

    // Copy every queue file into issues/, preserving subdirectory structure.
    let mut copied = 0usize;
    for rel in &queue_rel {
        let src = queue.join(rel);
        let dst = archive.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if std::fs::copy(&src, &dst).is_ok() {
            copied += 1;
        }
    }

    // Remove archive files that are no longer in the queue (resolved-and-migrated, or renamed). Git
    // history preserves them, so this keeps `issues/` a faithful mirror of live state.
    let mut archive_rel: Vec<PathBuf> = Vec::new();
    collect_rel(&archive, &archive, &mut archive_rel);
    let mut removed = 0usize;
    for rel in &archive_rel {
        if !queue_set.contains(rel) && std::fs::remove_file(archive.join(rel)).is_ok() {
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

/// Write the STANDING agents from the live runtime registry into the tracked `fleet/roster.json`
/// (in `cwd`, the caller's worktree). Standing = active + a role that belongs in the reproducible
/// fleet (everything except the ephemeral `fix`/`design` roles). Only machine-independent fields are
/// written; runtime state (worktree path, status, window) is re-derived by `up`. Returns the count.
fn sync_roster(fleet: &Fleet, cwd: &Path) -> usize {
    let reg = fleet.load();
    let mut entries = String::new();
    let mut n = 0usize;
    for a in &reg.agents {
        if a.status != "active" || matches!(a.role.as_str(), "fix" | "design") {
            continue;
        }
        if n > 0 {
            entries.push_str(",\n");
        }
        // Compact one-line object per agent (readable + stable diffs). Optional fields omitted empty.
        entries.push_str("    { ");
        entries.push_str(&format!("\"name\": {:?}, \"role\": {:?}", a.name, a.role));
        if !a.vertical.is_empty() {
            entries.push_str(&format!(", \"vertical\": {:?}", a.vertical));
        }
        if !a.area.is_empty() {
            entries.push_str(&format!(", \"area\": {:?}", a.area));
        }
        entries.push_str(&format!(
            ", \"model\": {:?}, \"effort\": {:?}, \"interval\": {:?} }}",
            a.model, a.effort, a.interval
        ));
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

/// Deliver a message: write it to a temp file then rename into the recipient's inbox, so a reader
/// never observes a partial file. The filename sorts in send order (`<seq>-<pid>-<kind>.json`).
fn deliver(fleet: &Fleet, msg: &Message) {
    let inbox = fleet.inbox(&msg.to);
    std::fs::create_dir_all(&inbox).expect("create recipient inbox");
    let fname = format!("{:012}-{}-{}.json", msg.seq, std::process::id(), msg.kind);
    let json = serde_json::to_string_pretty(msg).expect("serialize message");
    let tmp = inbox.join(format!(".{fname}.tmp"));
    std::fs::write(&tmp, json).expect("write message tmp");
    std::fs::rename(&tmp, inbox.join(&fname)).expect("rename message into inbox");
}

/// A process-local monotonic sequence for message ordering (wall-clock is unavailable in the
/// toolchain, and even so a counter is enough to order messages within one sender's run).
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
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

fn inbox_depth(fleet: &Fleet, name: &str) -> String {
    let inbox = fleet.inbox(name);
    let n = count_dir(&inbox, |f| f.ends_with(".json"));
    if n == 0 {
        "empty".to_string()
    } else {
        format!("{n} msg")
    }
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
    let mut it = s.split_whitespace();
    let behind = it.next()?.parse().ok()?; // left  = origin/main-only
    let ahead = it.next()?.parse().ok()?; // right = trunk-only
    Some((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn stale_window_is_capped_for_long_intervals() {
        // The bug: a 30m agent at mult=2 got a 3600s (60min) dead window. With the 600s cap it's
        // bounded to 10min — no agent sits dead for an hour regardless of its interval.
        let cap = 600;
        assert_eq!(stale_window_secs(1800, 2, cap), 600); // 30m×2 = 3600 → capped to 600
        assert_eq!(stale_window_secs(3600, 2, cap), 600); // 60m×2 = 7200 → capped to 600
    }

    #[test]
    fn stale_window_uncapped_below_the_cap() {
        // A short-interval agent still gets the full interval×mult (under the cap), so it isn't
        // judged stalled too eagerly during a normal-length tick.
        let cap = 600;
        assert_eq!(stale_window_secs(60, 2, cap), 120); // 1m×2 = 120, under cap
        assert_eq!(stale_window_secs(300, 2, cap), 600); // 5m×2 = 600, exactly at cap
        assert_eq!(stale_window_secs(299, 2, cap), 598); // just under
    }

    #[test]
    fn stale_window_never_overflows() {
        // A huge interval must saturate, not wrap, before the cap applies.
        assert_eq!(stale_window_secs(u64::MAX, 2, 600), 600);
    }
}
