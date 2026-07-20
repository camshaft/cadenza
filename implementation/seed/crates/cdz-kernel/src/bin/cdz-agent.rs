//! `cdz-agent` — the MINIMAL CLI for the agent-runtime (minimal-kernel re-charter, rung KC-bin).
//!
//! Operator fork-5: the CLI does almost NOTHING — it (1) bootstraps the log, (2) injects the genesis program,
//! (3) starts the daemon. All behavior is the Cadenza program in the log; the CLI is a thin bootstrapper over
//! the landed [`cdz_kernel::boot`] + [`cdz_kernel::daemon`] APIs. Hand-rolled arg parsing (no clap dep — the
//! kernel stays minimal).
//!
//! Subcommands:
//!   `cdz-agent bootstrap <log>`               — create/open the event log at <log> (idempotent).
//!   `cdz-agent inject-genesis <log> <prog>`   — append the Cadenza program source file <prog> as the genesis
//!                                               `program` event (a later inject supersedes it — self-mod).
//!   `cdz-agent authz-grant <log> <permit> [--expiry-ms <ms>]` — operator GRANT: append a narrow (optionally
//!                                               time-boxed) Cedar permit that ADDS to the base policy.
//!   `cdz-agent authz-revoke <log> <grant-seq>` — operator REVOKE: pull a prior grant by its seq.
//!   `cdz-agent authz-requests <log>`           — list the standing authorization requests (read-only).
//!   `cdz-agent schedule-create <log> <id> <trigger-kind> --first-ms <ms> [--period-ms <ms>] [--payload <t>]`
//!                                               — register a one-shot/periodic timer that fires <trigger-kind>.
//!   `cdz-agent schedule-cancel <log> <id>`     — cancel a schedule by id.
//!   `cdz-agent schedule-list <log>`            — list the active schedules (read-only).
//!   `cdz-agent run <log> <event-kind>`        — one daemon step: read the log → latest genesis → drive an
//!                                               interpret turn on the scalar <event-kind>. (The full event-
//!                                               source loop is a later rung; this runs ONE tick.)
//!   `cdz-agent replay <log> <event-kind>`     — RE-FOLD a recorded turn from the prim-result trail with NO
//!                                               live effect (time-travel; reports missing=0 when faithful).
//!   `cdz-agent fork <src-log> <new-log> [--upto <seq>]` — FORK a history into a new timeline (copy events,
//!                                               optionally up to a seq cutoff); the branch re-folds + extends.

use anyhow::{anyhow, Context, Result};
use cdz_kernel::{boot, daemon, policy, FileLog, Log};

fn main() {
    if let Err(e) = run() {
        eprintln!("cdz-agent: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str);
    match cmd {
        Some("bootstrap") => {
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent bootstrap <log>"))?;
            // Opening a FileLog creates it if absent — bootstrapping is idempotent (re-open = same log).
            let log =
                FileLog::open(log_path).with_context(|| format!("bootstrap log at {log_path}"))?;
            let n = log.tail(0)?.len();
            println!("bootstrapped log at {log_path} ({n} event(s))");
            Ok(())
        }
        Some("inject-genesis") => {
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent inject-genesis <log> <program.cdz>"))?;
            let prog_path = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent inject-genesis <log> <program.cdz>"))?;
            let src = std::fs::read_to_string(prog_path)
                .with_context(|| format!("read genesis program {prog_path}"))?;
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = boot::inject_genesis(&mut log, &src)?;
            println!("injected genesis program at seq {seq} (from {prog_path})");
            Ok(())
        }
        Some("emit-policy") => {
            // Append a CEDAR capability policy to the log (operator model: capability policies are Cedar docs
            // written to the log, retrieved + evaluated at invocation to attenuate a program to its minimal
            // privilege set). Reads the Cedar policy file <policy.cedar> and appends it as a `policy` event; a
            // later emit-policy SUPERSEDES the prior (the daemon's tick_hosted_log_policy evaluates the latest).
            // This is the operator's write-Cedar-docs-to-the-log entry point (the policy counterpart of
            // inject-genesis for programs).
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent emit-policy <log> <policy.cedar>"))?;
            let pol_path = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent emit-policy <log> <policy.cedar>"))?;
            let doc = std::fs::read_to_string(pol_path)
                .with_context(|| format!("read Cedar policy {pol_path}"))?;
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = policy::append_policy(&mut log, &doc)?;
            println!("appended capability policy at seq {seq} (from {pol_path})");
            Ok(())
        }
        Some("authz-grant") => {
            // The operator's answer to an `authz-request` (the can't-brick escape hatch): append a NARROW
            // Cedar `permit` doc as an `authz-grant` event, optionally time-boxed with `--expiry-ms <ms>` (an
            // ABSOLUTE ms-since-epoch deadline; the effective-policy fold drops the grant once now >= expiry).
            // Grants ADD to the base `policy` (they widen only within what the operator explicitly writes; a
            // program can't self-issue one). The grant's own seq is its identity — `authz-revoke <seq>` pulls it.
            let log_path = args.get(2).ok_or_else(|| {
                anyhow!("usage: cdz-agent authz-grant <log> <permit.cedar> [--expiry-ms <ms>]")
            })?;
            let permit_path = args.get(3).ok_or_else(|| {
                anyhow!("usage: cdz-agent authz-grant <log> <permit.cedar> [--expiry-ms <ms>]")
            })?;
            let permit = std::fs::read_to_string(permit_path)
                .with_context(|| format!("read Cedar permit {permit_path}"))?;
            let expiry_ms: Option<u64> = match flag_value(&args, "--expiry-ms") {
                Some(ms) => Some(ms.parse().context("--expiry-ms must be a non-negative integer (ms since epoch)")?),
                None => None,
            };
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = policy::append_grant(&mut log, &permit, expiry_ms)?;
            println!(
                "granted capability at seq {seq} (from {permit_path}){}",
                expiry_ms
                    .map(|e| format!(" — expires at {e}ms since epoch"))
                    .unwrap_or_else(|| " — no expiry".to_string())
            );
            Ok(())
        }
        Some("authz-revoke") => {
            // The operator PULLS a prior grant: append an `authz-revoke` event naming the grant's seq. The
            // effective-policy fold thereafter excludes that grant (operators can revoke authorization later,
            // per the operator model). Idempotent in effect — revoking a non-grant/absent seq simply removes
            // nothing (the fold only drops grants whose seq is named).
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent authz-revoke <log> <grant-seq>"))?;
            let grant_seq: cdz_kernel::Seq = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent authz-revoke <log> <grant-seq>"))?
                .parse()
                .context("grant-seq must be a non-negative integer (the granted event's seq)")?;
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = policy::append_revoke(&mut log, grant_seq)?;
            println!("revoked grant at seq {grant_seq} (revoke event at seq {seq})");
            Ok(())
        }
        Some("authz-requests") => {
            // List the standing authorization REQUESTS (operator read verb): each denied op auto-emits an
            // authz-request; this shows them so the operator can see what's waiting on a grant and answer with a
            // narrow authz-grant. Read-only. One line per request: `seq <n>  prim:<op>  <payload>`. Lists ALL
            // requests (matching a grant to a request needs full Cedar eval; the operator decides).
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent authz-requests <log>"))?;
            let log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let reqs = policy::requests(&log.tail(0)?);
            if reqs.is_empty() {
                println!("no authorization requests");
            } else {
                println!("{} authorization request(s):", reqs.len());
                for r in &reqs {
                    println!("  seq {}  prim:{}  {}", r.seq, r.op, r.payload);
                }
            }
            Ok(())
        }
        Some("schedule-create") => {
            // Register a SCHEDULE (operator scheduling entry point, the counterpart of emit-policy for timers):
            // fire the TRIGGER event `<trigger-kind>` (+ `--payload`) at `--first-ms` (absolute ms since epoch),
            // then — if `--period-ms` is given — every period thereafter until cancelled (one-shot without it).
            // A later create with the same `<id>` supersedes. The daemon's fire_due_schedules emits the trigger
            // when due; the trigger-kind must NOT be a reserved kind (a timer can't forge kernel bookkeeping).
            let log_path = args.get(2).ok_or_else(|| {
                anyhow!("usage: cdz-agent schedule-create <log> <id> <trigger-kind> --first-ms <ms> [--period-ms <ms>] [--payload <text>]")
            })?;
            let id = args.get(3).ok_or_else(|| {
                anyhow!("usage: cdz-agent schedule-create <log> <id> <trigger-kind> --first-ms <ms> [--period-ms <ms>] [--payload <text>]")
            })?;
            let trigger_kind = args.get(4).ok_or_else(|| {
                anyhow!("usage: cdz-agent schedule-create <log> <id> <trigger-kind> --first-ms <ms> [--period-ms <ms>] [--payload <text>]")
            })?;
            if id.contains('\n') {
                return Err(anyhow!("schedule id must not contain a newline (the codec is line-delimited)"));
            }
            if trigger_kind.contains('\n') {
                return Err(anyhow!("trigger-kind must not contain a newline (the codec is line-delimited)"));
            }
            if daemon::is_reserved_kind(trigger_kind) {
                return Err(anyhow!(
                    "refusing a reserved trigger-kind `{trigger_kind}` — a schedule must fire a real trigger, \
                     not forge kernel bookkeeping (genesis/policy/authz/prim-*/schedule-*)"
                ));
            }
            let first_ms: u64 = flag_value(&args, "--first-ms")
                .ok_or_else(|| anyhow!("schedule-create requires --first-ms <ms> (absolute ms since epoch)"))?
                .parse()
                .context("--first-ms must be a non-negative integer")?;
            let period_ms: Option<u64> = match flag_value(&args, "--period-ms") {
                Some(ms) => {
                    let p: u64 = ms.parse().context("--period-ms must be a non-negative integer")?;
                    if p == 0 {
                        // A zero period would divide-by-zero the daemon's `due` fold every tick — reject it up
                        // front (an actionable error, not a poison event Schedule::decode silently drops). Omit
                        // --period-ms for a one-shot.
                        return Err(anyhow!(
                            "--period-ms must be a POSITIVE integer (0 is invalid; omit --period-ms for a one-shot)"
                        ));
                    }
                    Some(p)
                }
                None => None,
            };
            let payload = flag_value(&args, "--payload").unwrap_or_default();
            let schedule = cdz_kernel::schedule::Schedule {
                id: id.clone(),
                first_ms,
                period_ms,
                trigger_kind: trigger_kind.clone(),
                payload: payload.into_bytes(),
            };
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = cdz_kernel::schedule::append_create(&mut log, &schedule)?;
            println!(
                "scheduled `{id}` at seq {seq}: fire `{trigger_kind}` at {first_ms}ms{}",
                period_ms
                    .map(|p| format!(", then every {p}ms (periodic)"))
                    .unwrap_or_else(|| " (one-shot)".to_string())
            );
            Ok(())
        }
        Some("schedule-cancel") => {
            // Cancel a schedule by id — the daemon's active-set fold thereafter excludes it (an operator stops
            // a periodic timer, or a one-shot before it fires). A later schedule-create with that id re-registers.
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent schedule-cancel <log> <id>"))?;
            let id = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent schedule-cancel <log> <id>"))?;
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = cdz_kernel::schedule::append_cancel(&mut log, id)?;
            println!("cancelled schedule `{id}` (cancel event at seq {seq})");
            Ok(())
        }
        Some("schedule-list") => {
            // Inspect the ACTIVE schedules (operator read verb): fold the log for the newest create per id minus
            // cancelled ones (schedule::active_schedules), and print each — so an operator can see what timers
            // are registered before cancelling/superseding. Read-only (no append). One line per schedule:
            // `<id>  <one-shot|every <period>ms>  first=<first_ms>ms  -> <trigger-kind>`.
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent schedule-list <log>"))?;
            let log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let active = cdz_kernel::schedule::active_schedules(&log.tail(0)?);
            if active.is_empty() {
                println!("no active schedules");
            } else {
                println!("{} active schedule(s):", active.len());
                for s in &active {
                    let cadence = match s.period_ms {
                        Some(p) => format!("every {p}ms"),
                        None => "one-shot".to_string(),
                    };
                    println!(
                        "  {}  {cadence}  first={}ms  -> {}",
                        s.id, s.first_ms, s.trigger_kind
                    );
                }
            }
            Ok(())
        }
        Some("emit") => {
            // Append an external TRIGGER event to the log — a minimal event SOURCE so the live daemon
            // (`start`) has something to perform. `<kind>` is a free event-kind tag + `<payload>` its body.
            // REFUSES a RESERVED kind (`program` = genesis, `prim-*` = the daemon's own effect records,
            // `policy` = a capability doc written via emit-policy): an operator must not forge a genesis, a
            // fake effect, or a policy through this door. Any other kind is a trigger the daemon's kind_of maps to a run.
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent emit <log> <kind> <payload>"))?;
            let kind = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent emit <log> <kind> <payload>"))?;
            let payload = args
                .get(4)
                .ok_or_else(|| anyhow!("usage: cdz-agent emit <log> <kind> <payload>"))?;
            if daemon::is_reserved_kind(kind) {
                return Err(anyhow!(
                    "refusing to emit a reserved event kind `{kind}` (genesis→inject-genesis; policy→emit-policy; \
                     grant→authz-grant; revoke→authz-revoke; schedule→schedule-create/schedule-cancel; `{}`* \
                     effects + authz-request + schedule-fire are written only by the daemon) — pick a trigger kind",
                    daemon::PRIM_RECORD_PREFIX
                ));
            }
            let mut log =
                FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            let seq = log.append(kind, payload.as_bytes())?;
            println!("emitted `{kind}` event at seq {seq} ({} byte payload)", payload.len());
            Ok(())
        }
        Some("run") => {
            let (log, kind, runtime) = open_and_resolve(&args, "run")?;
            let tick = daemon::tick(&log, kind, runtime)?;
            println!(
                "tick: interpret scheduled {} host-op(s) for event kind {kind}",
                tick.op_count
            );
            Ok(())
        }
        Some("perform") => {
            // The daemon's real EXECUTION step (K1c): unlike `run` (which only COUNTS the scheduled ops),
            // `perform` drives `daemon::tick_performing` so each op fires a real `Prim.run` perform, and the
            // fold SUMS the per-op results (Append→1, Exec→2 = 3 for kind=1 — the exec/http/log shape).
            let (log, kind, runtime) = open_and_resolve(&args, "perform")?;
            let result = daemon::tick_performing(&log, kind, runtime)?;
            println!(
                "perform: executed the interpret plan for event kind {kind}; summed per-op result = {result}"
            );
            Ok(())
        }
        Some("hosted") => {
            // The daemon's real-PRIMITIVE step (K1c→host): `hosted` drives `daemon::tick_hosted`, which binds
            // Prim to a real host closure — each performed op is RECORDED in the log as a `prim-<op>` event
            // (the recorded-effect trail) and its per-op result summed. Unlike `perform` (in-program mock),
            // this is the daemon actually executing through a real host primitive (record-only cut for now).
            //
            // With `--policies <file>` (an operator-controlled Cedar policy — the external trust anchor the
            // agent can't widen), each op is AUTHORIZED against it before performing (tick_hosted_authorized):
            // an allowed op performs + records `prim-<op>`, a denied op records `prim-denied-<op>` and is
            // skipped. Without `--policies`, the ungated record-only path (tick_hosted) runs.
            let (log, kind, runtime) = open_and_resolve(&args, "hosted")?;
            let policies = match flag_value(&args, "--policies") {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("read Cedar policy file {path}"))?,
                ),
                None => None,
            };
            let log = std::sync::Arc::new(std::sync::Mutex::new(log));
            let result = match policies {
                Some(root) => {
                    daemon::tick_hosted_authorized(std::sync::Arc::clone(&log), kind, runtime, root)?
                }
                None => daemon::tick_hosted(std::sync::Arc::clone(&log), kind, runtime)?,
            };
            // Count performed (`prim-<op>` REQUEST records) vs denied events for the report. Exclude BOTH
            // `prim-denied-<op>` (a denial, not a perform) AND `prim-result-<op>` (the §2.3 RESPONSE half —
            // one per perform, else each performed op would be counted twice).
            let events = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?.tail(0)?;
            let denied = events
                .iter()
                .filter(|e| e.kind.starts_with(daemon::PRIM_DENIED_PREFIX))
                .count();
            let performed = events
                .iter()
                .filter(|e| {
                    e.kind.starts_with(daemon::PRIM_RECORD_PREFIX)
                        && !e.kind.starts_with(daemon::PRIM_DENIED_PREFIX)
                        && !e.kind.starts_with(daemon::PRIM_RESULT_PREFIX)
                })
                .count();
            println!(
                "hosted: performed the interpret plan for event kind {kind} via real host primitives; \
                 summed per-op result = {result}; {performed} performed + {denied} denied prim event(s) in the log"
            );
            Ok(())
        }
        Some("replay") => {
            // TIME-TRAVEL: RE-FOLD a recorded hosted turn from the log WITHOUT performing any live effect
            // (`daemon::replay_hosted_turn`, the §2.3 determinism proof). Each Prim.exec/http/append is answered
            // from the recorded `prim-result-<op>` trail a prior `hosted`/`start` run appended — reproducing the
            // same summed cognition with the world's non-determinism frozen. A FAITHFUL replay reports
            // `missing = 0` and appends NOTHING (fork / hand-off / time-travel are all a pure re-fold); a
            // `missing > 0` means the re-fold diverged from what was recorded (more ops than recorded, or a
            // corrupt result payload) — a loud signal, never a silent live perform.
            let (log, kind, runtime) = open_and_resolve(&args, "replay")?;
            let events = log.tail(0)?;
            let program = boot::latest_program(&events)
                .ok_or_else(|| anyhow!("no genesis program in the log — run inject-genesis first"))?;
            let replay = daemon::replay_hosted_turn(&events, &program, kind, runtime)?;
            println!(
                "replay: re-folded event kind {kind} from the recorded prim-result trail (no live effect); \
                 summed per-op result = {}; {} op(s) replayed, {} missing{}",
                replay.sum,
                replay.replayed,
                replay.missing,
                if replay.missing == 0 {
                    " (faithful — deterministic re-fold)"
                } else {
                    " (DIVERGED — the re-fold asked for results the recorded trail lacks)"
                }
            );
            Ok(())
        }
        Some("fork") => {
            // FORK a recorded history into a NEW timeline (vision §3: fork / hand-off / time-travel). Copy the
            // source log's events into a fresh destination log (`boot::fork_log`), optionally only up to a seq
            // CUTOFF (`--upto <seq>`, EXCLUSIVE — branch the world "as of just before event <seq>"; omit to
            // mirror the whole log). The destination is a valid log the daemon runs unchanged — it re-folds the
            // copied prefix and EXTENDS the branch independently of the source. Read-only on the source.
            let src_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent fork <src-log> <new-log> [--upto <seq>]"))?;
            let dst_path = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent fork <src-log> <new-log> [--upto <seq>]"))?;
            let upto: Option<u64> = match flag_value(&args, "--upto") {
                Some(s) => Some(s.parse().context("--upto must be a non-negative seq")?),
                None => None,
            };
            let src = FileLog::open(src_path).with_context(|| format!("open src log {src_path}"))?;
            let mut dst =
                FileLog::open(dst_path).with_context(|| format!("open new log {dst_path}"))?;
            let copied = boot::fork_log(&src, &mut dst, upto)?;
            println!(
                "fork: copied {copied} event(s) from {src_path} into {dst_path}{} — a new timeline the daemon can re-fold + extend",
                upto.map(|k| format!(" (up to seq {k}, exclusive)"))
                    .unwrap_or_else(|| " (full history)".to_string())
            );
            Ok(())
        }
        Some("start") => {
            // START THE DAEMON (the operator entry point): poll the log and PERFORM each new TRIGGER event via
            // the real-primitive hosted path (daemon::run over daemon::run_once → tick_hosted). Bounded by
            // `--max-rounds N` (deterministic, so a smoke run terminates) and/or `--stop-file <path>` (the
            // operator touches it to stop). `<poll-ms>` is the sleep between rounds. This is the standalone
            // tool's OWN stop control — NOT coupled to the fleet's orchestration stop dir.
            let (log, _kind0, runtime) = open_and_resolve(&args, "start")?;
            let poll_ms: u64 = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent start <log> <poll-ms> [--from <seq>] [--stop-file <path>] [--max-rounds N] [--policies <file>]"))?
                .parse()
                .context("poll-ms must be a non-negative integer")?;
            let stop_file = flag_value(&args, "--stop-file");
            let max_rounds: Option<u64> = match flag_value(&args, "--max-rounds") {
                Some(n) => Some(n.parse().context("--max-rounds must be an integer")?),
                None => None,
            };
            // CRASH-RECOVERY cursor: `--from <seq>` resumes the daemon at a known cursor instead of re-draining
            // the WHOLE log from 0 (which would re-perform every historical trigger — an at-most-once violation
            // on restart). An operator restarting the daemon passes the cursor a prior run reported (the daemon
            // prints its final cursor on a clean stop). Defaults to 0 (behavior-preserving: a fresh log or a
            // full re-fold). NOT past the log tail — a `from` beyond the current length simply drains nothing.
            let from: u64 = match flag_value(&args, "--from") {
                Some(s) => s.parse().context("--from must be a non-negative seq cursor")?,
                None => 0,
            };
            // Optional external Cedar trust-anchor: with --policies, every op the live daemon performs is
            // authorized against it first (the agent can't widen it); without it, the ungated record-only path.
            let policies = match flag_value(&args, "--policies") {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("read Cedar policy file {path}"))?,
                ),
                None => None,
            };
            let log = std::sync::Arc::new(std::sync::Mutex::new(log));
            // Trigger mapping: every event EXCEPT the daemon's own RESERVED bookkeeping (genesis `program`,
            // recorded `prim-*` effects, capability `policy`/authz docs, and `schedule-*` events — a fired
            // schedule's TRIGGER event is a normal kind, only the create/cancel/fire bookkeeping is skipped)
            // fires the interpret plan for event-kind 1. Skipping our own events is load-bearing — else the
            // daemon re-performs its own recorded effects/bookkeeping as triggers and never converges.
            let kind_of = |e: &cdz_kernel::Event| {
                if daemon::is_reserved_kind(&e.kind) {
                    None
                } else {
                    Some(1i64)
                }
            };
            let mut round = 0u64;
            let should_stop = || {
                if let Some(max) = max_rounds {
                    if round >= max {
                        return true;
                    }
                }
                if let Some(path) = &stop_file {
                    if std::path::Path::new(path).exists() {
                        return true;
                    }
                }
                round += 1;
                false
            };
            println!(
                "start: daemon polling {} every {poll_ms}ms{}{}{}",
                args.get(2).map(String::as_str).unwrap_or("<log>"),
                stop_file
                    .as_ref()
                    .map(|p| format!(" (stop-file {p})"))
                    .unwrap_or_default(),
                max_rounds
                    .map(|n| format!(" (max {n} round(s))"))
                    .unwrap_or_default(),
                if policies.is_some() {
                    " (Cedar-gated)"
                } else {
                    ""
                },
            );
            let final_cursor = daemon::run(
                log,
                from,
                runtime,
                std::time::Duration::from_millis(poll_ms),
                policies,
                kind_of,
                should_stop,
            )?;
            // Report the resume cursor so a restart can pass `--from <cursor>` and NOT re-perform history.
            println!(
                "start: daemon stopped cleanly at cursor {final_cursor} (restart with --from {final_cursor} to resume without re-performing processed events)"
            );
            Ok(())
        }
        _ => Err(anyhow!(
            "usage: cdz-agent <bootstrap|inject-genesis|emit-policy|authz-grant|authz-revoke|authz-requests|schedule-create|schedule-cancel|schedule-list|emit|run|perform|hosted|replay|fork|start> ...\n\
             \x20 bootstrap <log>                    — create/open the event log\n\
             \x20 inject-genesis <log> <program.cdz> — append the genesis program\n\
             \x20 emit-policy <log> <policy.cedar>    — append a Cedar capability policy (attenuates each invocation; latest supersedes)\n\
             \x20 authz-grant <log> <permit.cedar> [--expiry-ms <ms>] — operator GRANT: append a narrow Cedar permit (optionally time-boxed); adds to the base policy\n\
             \x20 authz-revoke <log> <grant-seq>      — operator REVOKE: pull a prior grant by its seq (the effective policy thereafter excludes it)\n\
             \x20 authz-requests <log>                — list the standing authorization requests (read-only)\n\
             \x20 schedule-create <log> <id> <trigger-kind> --first-ms <ms> [--period-ms <ms>] [--payload <t>] — register a one-shot/periodic timer\n\
             \x20 schedule-cancel <log> <id>          — cancel a schedule by id (a later create re-registers)\n\
             \x20 schedule-list <log>                 — list the active schedules (read-only)\n\
             \x20 emit <log> <kind> <payload>        — append an external trigger event (a minimal event source; refuses reserved kinds)\n\
             \x20 run <log> <event-kind>             — one daemon tick (COUNT the scheduled host-ops)\n\
             \x20 perform <log> <event-kind>         — one daemon tick that EXECUTES the ops (K1c, in-program mock), summing per-op results\n\
             \x20 hosted <log> <event-kind> [--policies <file>] — one daemon tick that PERFORMS via real host primitives (K1c→host); with --policies, each op is Cedar-authorized against the external policy first\n\
             \x20 replay <log> <event-kind>          — RE-FOLD a recorded turn from the prim-result trail (time-travel, no live effect); reports missing=0 when faithful\n\
             \x20 fork <src-log> <new-log> [--upto <seq>] — FORK a recorded history into a new timeline (copy events, optionally up to a seq cutoff); the branch re-folds + extends independently\n\
             \x20 start <log> <poll-ms> [--from <seq>] [--stop-file <path>] [--max-rounds N] [--policies <file>] — START the daemon: poll + perform each new trigger event (--from resumes at a cursor without re-performing history); with --policies each op is Cedar-authorized"
        )),
    }
}

/// Read the value following a `--flag` in `args` (e.g. `--stop-file /tmp/stop` → `Some("/tmp/stop")`), or
/// `None` if the flag is absent or has no following value. Hand-rolled (no clap — the kernel stays minimal).
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Shared boilerplate for the `run`/`perform` verbs: parse `<log> <event-kind>`, open the log, find the latest
/// genesis, and resolve the value-heap runtime the compiled provider requires (walking up for the store).
/// Returns the opened `(log, event-kind, runtime bytes)` the daemon step needs. `verb` labels usage errors.
fn open_and_resolve(args: &[String], verb: &str) -> Result<(FileLog, i64, Vec<u8>)> {
    let log_path = args
        .get(2)
        .ok_or_else(|| anyhow!("usage: cdz-agent {verb} <log> <event-kind>"))?;
    let kind: i64 = args
        .get(3)
        .ok_or_else(|| anyhow!("usage: cdz-agent {verb} <log> <event-kind>"))?
        .parse()
        .context("event-kind must be an integer")?;
    let log = FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
    // Resolve the value-heap runtime the genesis program needs (walk up for the store).
    let program = boot::latest_program(&log.tail(0)?)
        .ok_or_else(|| anyhow!("no genesis program in {log_path} — run inject-genesis first"))?;
    let provider = cdz_kernel::kernel::compile_interpret_provider(&program)?;
    let here = std::env::current_dir()?;
    let runtime = cdz_kernel::kernel::find_runtime_for(&provider, &here).ok_or_else(|| {
        anyhow!(
            "value-heap runtime not found in any ancestor store of {} (run `cargo xtask build`)",
            here.display()
        )
    })?;
    Ok((log, kind, runtime))
}
