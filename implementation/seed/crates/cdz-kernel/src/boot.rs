//! Bootstrap + genesis injection (minimal-kernel re-charter, rung KC — the thin CLI's log half).
//!
//! Operator ruling (fork 5): the CLI is MINIMAL — it (1) bootstraps the log and (2) injects the GENESIS
//! program into it, then (3) starts the daemon. The kernel has NO hardcoded genesis: the genesis interpret
//! program is DATA in the log, injected by the CLI, so even the first program is self-modifiable. This module
//! is the log/genesis half (1)+(2) — pure over the [`crate::Log`] trait, testable with a `FileLog`; wrapping
//! it in an actual `cdz-agent` binary + "start the daemon" (3) is the CLI/daemon rung on top.
//!
//! The GENESIS convention: the genesis interpret program is appended as a `program`-kind event whose payload
//! is the Cadenza SOURCE. The kernel finds "the program to run" by folding the log for the LATEST `program`
//! event ([`latest_program`]) — so a NEW `program` event (a self-modification) supersedes the genesis with no
//! kernel change. The kernel names `program` here (the one event kind the boot path knows), but it does NOT
//! interpret the source — it hands it to the compiler ([`crate::kernel::compile_interpret_provider`]).

use crate::{Log, Seq};
use anyhow::Result;

/// The event `kind` tag for a program event — its payload is the Cadenza interpret SOURCE. The CLI injects
/// the genesis as one of these; a later `program` event supersedes it (the kernel runs the latest). This is
/// the single event kind the boot/genesis path names; the kernel never parses the source (it compiles it).
pub const PROGRAM: &str = "program";

/// Inject a genesis program into `log`: append `interpret_src` (the Cadenza interpret SOURCE) as a `program`
/// event, returning its assigned `seq`. This is the CLI's step (2) — seeding the log with the first program.
/// Idempotent only in the sense that re-injecting appends a NEW `program` event that SUPERSEDES the prior one
/// (the kernel runs the latest), which is exactly the self-modification path — the CLI can re-inject an
/// updated genesis without any kernel change.
pub fn inject_genesis(log: &mut impl Log, interpret_src: &str) -> Result<Seq> {
    log.append(PROGRAM, interpret_src.as_bytes())
}

/// The LATEST program source in `log` — the kernel's genesis lookup (fold the log for the newest `program`
/// event, decode its payload as UTF-8 source). `None` if the log holds no `program` event yet (the CLI has
/// not injected a genesis). This is how the daemon decides WHAT to run without a hardcoded genesis: whatever
/// the latest `program` event carries. A later `program` event (a self-modification appended by a running
/// agent) transparently supersedes the genesis — same fold, newer winner.
pub fn latest_program(events: &[crate::Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|e| e.kind == PROGRAM)
        .and_then(|e| String::from_utf8(e.payload.clone()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileLog;

    fn temp_log() -> (std::path::PathBuf, FileLog) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p =
            std::env::temp_dir().join(format!("cdz-kernel-boot-{}-{n}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    const GENESIS: &str = "(do (def (interpret (: kind Int64)) kind) (export interpret))";

    #[test]
    fn inject_genesis_appends_a_program_event_the_kernel_can_find() {
        // The CLI bootstraps + injects: the genesis source lands as a `program` event, and the kernel's
        // genesis lookup (latest_program) reads it back — no hardcoded genesis, the source is data in the log.
        let (path, mut log) = temp_log();
        let seq = inject_genesis(&mut log, GENESIS).unwrap();
        assert_eq!(seq, 0, "the genesis is the first event");
        let events = log.tail(0).unwrap();
        assert_eq!(
            latest_program(&events).as_deref(),
            Some(GENESIS),
            "the kernel finds the injected genesis source"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_later_program_supersedes_the_genesis_self_modification() {
        // A NEW `program` event (an agent self-modifying the interpret) supersedes the genesis — the kernel
        // runs the LATEST, no kernel change. This is the whole point: everything above the kernel is
        // self-modifiable Cadenza in the log.
        let (path, mut log) = temp_log();
        inject_genesis(&mut log, GENESIS).unwrap();
        let updated = "(do (def (interpret (: kind Int64)) (+ kind 1)) (export interpret))";
        inject_genesis(&mut log, updated).unwrap();
        let events = log.tail(0).unwrap();
        assert_eq!(
            latest_program(&events).as_deref(),
            Some(updated),
            "the latest program event wins — a self-modification supersedes the genesis"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn latest_program_is_none_before_any_genesis() {
        // A freshly-bootstrapped log with no genesis yet → the kernel has nothing to run (the CLI must inject
        // first). Also: a non-program event does not count as a program.
        assert!(latest_program(&[]).is_none(), "empty log → no program");
        let non_program = crate::Event {
            seq: 0,
            kind: "message".into(),
            payload: b"not a program".to_vec(),
        };
        assert!(
            latest_program(&[non_program]).is_none(),
            "a non-program event is not the genesis"
        );
    }
}
