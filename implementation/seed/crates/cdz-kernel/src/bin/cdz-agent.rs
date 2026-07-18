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
//!   `cdz-agent run <log> <event-kind>`        — one daemon step: read the log → latest genesis → drive an
//!                                               interpret turn on the scalar <event-kind>. (The full event-
//!                                               source loop is a later rung; this runs ONE tick.)

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
            if kind == boot::PROGRAM
                || kind == policy::POLICY
                || kind.starts_with(daemon::PRIM_RECORD_PREFIX)
            {
                return Err(anyhow!(
                    "refusing to emit a reserved event kind `{kind}` (genesis→inject-genesis; policy→emit-policy; \
                     `{}`* events are written only by the daemon) — pick a trigger kind",
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
            // Count performed (`prim-<op>`, excluding `prim-denied-<op>`) vs denied events for the report.
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
                })
                .count();
            println!(
                "hosted: performed the interpret plan for event kind {kind} via real host primitives; \
                 summed per-op result = {result}; {performed} performed + {denied} denied prim event(s) in the log"
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
                .ok_or_else(|| anyhow!("usage: cdz-agent start <log> <poll-ms> [--stop-file <path>] [--max-rounds N] [--policies <file>]"))?
                .parse()
                .context("poll-ms must be a non-negative integer")?;
            let stop_file = flag_value(&args, "--stop-file");
            let max_rounds: Option<u64> = match flag_value(&args, "--max-rounds") {
                Some(n) => Some(n.parse().context("--max-rounds must be an integer")?),
                None => None,
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
            // Trigger mapping: every event EXCEPT the daemon's own bookkeeping (the genesis `program` event
            // and the recorded `prim-*` effects) fires the interpret plan for event-kind 1. Skipping our own
            // events is load-bearing — else the daemon re-performs its own recorded effects and never converges.
            let kind_of = |e: &cdz_kernel::Event| {
                if e.kind == boot::PROGRAM || e.kind.starts_with(daemon::PRIM_RECORD_PREFIX) {
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
            daemon::run(
                log,
                0,
                runtime,
                std::time::Duration::from_millis(poll_ms),
                policies,
                kind_of,
                should_stop,
            )?;
            println!("start: daemon stopped cleanly");
            Ok(())
        }
        _ => Err(anyhow!(
            "usage: cdz-agent <bootstrap|inject-genesis|emit-policy|emit|run|perform|hosted|start> ...\n\
             \x20 bootstrap <log>                    — create/open the event log\n\
             \x20 inject-genesis <log> <program.cdz> — append the genesis program\n\
             \x20 emit-policy <log> <policy.cedar>    — append a Cedar capability policy (attenuates each invocation; latest supersedes)\n\
             \x20 emit <log> <kind> <payload>        — append an external trigger event (a minimal event source; refuses reserved kinds)\n\
             \x20 run <log> <event-kind>             — one daemon tick (COUNT the scheduled host-ops)\n\
             \x20 perform <log> <event-kind>         — one daemon tick that EXECUTES the ops (K1c, in-program mock), summing per-op results\n\
             \x20 hosted <log> <event-kind> [--policies <file>] — one daemon tick that PERFORMS via real host primitives (K1c→host); with --policies, each op is Cedar-authorized against the external policy first\n\
             \x20 start <log> <poll-ms> [--stop-file <path>] [--max-rounds N] [--policies <file>] — START the daemon: poll + perform each new trigger event; with --policies each op is Cedar-authorized"
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
