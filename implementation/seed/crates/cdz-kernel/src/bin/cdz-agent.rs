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
use cdz_kernel::{boot, daemon, FileLog, Log};

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
        Some("run") => {
            let log_path = args
                .get(2)
                .ok_or_else(|| anyhow!("usage: cdz-agent run <log> <event-kind>"))?;
            let kind: i64 = args
                .get(3)
                .ok_or_else(|| anyhow!("usage: cdz-agent run <log> <event-kind>"))?
                .parse()
                .context("event-kind must be an integer")?;
            let log = FileLog::open(log_path).with_context(|| format!("open log {log_path}"))?;
            // Resolve the value-heap runtime the genesis program needs (walk up for the store).
            let program = boot::latest_program(&log.tail(0)?).ok_or_else(|| {
                anyhow!("no genesis program in {log_path} — run inject-genesis first")
            })?;
            let provider = cdz_kernel::kernel::compile_interpret_provider(&program)?;
            let here = std::env::current_dir()?;
            let runtime = cdz_kernel::kernel::find_runtime_for(&provider, &here)
                .ok_or_else(|| anyhow!("value-heap runtime not found in any ancestor store of {} (run `cargo xtask build`)", here.display()))?;
            let tick = daemon::tick(&log, kind, runtime)?;
            println!(
                "tick: interpret scheduled {} host-op(s) for event kind {kind}",
                tick.op_count
            );
            Ok(())
        }
        _ => Err(anyhow!(
            "usage: cdz-agent <bootstrap|inject-genesis|run> ...\n\
             \x20 bootstrap <log>                    — create/open the event log\n\
             \x20 inject-genesis <log> <program.cdz> — append the genesis program\n\
             \x20 run <log> <event-kind>             — one daemon tick over the latest genesis"
        )),
    }
}
