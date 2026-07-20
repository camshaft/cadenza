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

/// FORK a recorded history into a NEW timeline (vision §3: fork / hand-off / time-travel are all re-fold over
/// the log). Copy every event of `src` with `seq < upto` — or the WHOLE log when `upto` is `None` — into `dst`
/// in order, preserving each event's `kind` + `payload` (the daemon re-derives the same cognition by re-folding
/// the copied prefix, then EXTENDS the branch independently of `src`). Returns how many events were copied.
///
/// The copy re-appends through the [`Log`] trait, so `dst` assigns its own gap-free seqs by append position
/// (0-based) — identical to how `src` was built and how the daemon reads, so a fork is itself a valid log the
/// daemon runs unchanged. `dst` SHOULD be fresh/empty (a fork creates a new timeline); if it already holds
/// events the copy appends AFTER them (the caller owns that choice). Pure over the trait — testable with two
/// `FileLog`s, no daemon. `upto` is a seq CUTOFF (exclusive): `Some(k)` branches the history "as of just
/// before event k" (time-travel to a point), `None` mirrors the full log (a plain branch/hand-off).
///
/// The parent's [`crate::daemon::DAEMON_CURSOR`] records are DROPPED from the fork — they are the parent
/// daemon's private RESUME BOOKMARK (progress meta-state keyed to the parent's seqs), NOT part of the recorded
/// history. If they carried over, a daemon started on the fork (with no `--from`) would auto-resume at the
/// PARENT's high-water mark ([`crate::daemon::latest_cursor`]) and SKIP re-performing the very history the fork
/// exists to re-fold + extend — and for a cutoff fork that bookmark could even point past the fork's own tail.
/// A fork is a FRESH timeline: it re-folds from the start. The recorded-effect trail (`prim-*`/`prim-result-*`)
/// and all real history still carry (replay over the fork needs them) — only the resume bookmark is meta.
pub fn fork_log(src: &impl Log, dst: &mut impl Log, upto: Option<Seq>) -> Result<usize> {
    let events = src.tail(0)?;
    let mut copied = 0usize;
    for e in &events {
        if let Some(cut) = upto {
            if e.seq >= cut {
                break; // exclusive cutoff — events at/after `cut` are NOT in the forked timeline
            }
        }
        // Drop the parent daemon's resume bookmark — the fork is a fresh timeline that re-folds from the start.
        if e.kind == crate::daemon::DAEMON_CURSOR {
            continue;
        }
        dst.append(&e.kind, &e.payload)?;
        copied += 1;
    }
    Ok(copied)
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
    fn fork_log_full_copy_mirrors_the_whole_history_into_a_new_timeline() {
        // A plain branch/hand-off (upto=None): every event of src is copied into a fresh dst, in order, with
        // kind+payload preserved and dst's seqs re-assigned gap-free — so the fork is a valid log the daemon
        // runs unchanged (its genesis is still findable).
        let (spath, mut src) = temp_log();
        inject_genesis(&mut src, GENESIS).unwrap(); // seq 0
        src.append("trigger", b"one").unwrap(); // seq 1
        src.append("prim-result-exec", b"2").unwrap(); // seq 2

        let (dpath, mut dst) = temp_log();
        let copied = fork_log(&src, &mut dst, None).unwrap();
        assert_eq!(copied, 3, "the whole 3-event history is mirrored");

        let de = dst.tail(0).unwrap();
        assert_eq!(de.len(), 3, "dst holds all 3 events");
        // Seqs are re-assigned by append position (0,1,2) — a valid gap-free log.
        assert_eq!(
            de.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "the fork re-assigns gap-free seqs by append position"
        );
        // kind+payload preserved verbatim; the genesis is still findable in the branch.
        assert_eq!(de[1].kind, "trigger");
        assert_eq!(de[1].payload, b"one");
        assert_eq!(
            latest_program(&de).as_deref(),
            Some(GENESIS),
            "the forked timeline carries the genesis — the daemon runs it unchanged"
        );
        let _ = std::fs::remove_file(&spath);
        let _ = std::fs::remove_file(&dpath);
    }

    #[test]
    fn fork_log_cutoff_branches_the_history_as_of_a_point_time_travel() {
        // Time-travel (upto=Some(k), EXCLUSIVE): only events with seq < k are copied — the branch is the world
        // "as of just before event k", which the daemon can then re-fold + extend on a divergent path.
        let (spath, mut src) = temp_log();
        inject_genesis(&mut src, GENESIS).unwrap(); // seq 0
        src.append("trigger", b"a").unwrap(); // seq 1
        src.append("trigger", b"b").unwrap(); // seq 2
        src.append("trigger", b"c").unwrap(); // seq 3

        let (dpath, mut dst) = temp_log();
        // Branch as of just before seq 2 → copies seq 0 (genesis) + seq 1 only.
        let copied = fork_log(&src, &mut dst, Some(2)).unwrap();
        assert_eq!(copied, 2, "cutoff excludes seq >= 2 → only 0 and 1 copied");
        let de = dst.tail(0).unwrap();
        assert_eq!(
            de.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 1],
            "the branch holds the prefix before the cutoff, re-sequenced"
        );
        assert_eq!(
            de[1].payload, b"a",
            "the last kept event is the pre-cutoff one"
        );
        // A cutoff of 0 forks an EMPTY timeline (nothing before seq 0); the src is untouched by the fork.
        let (epath, mut empty) = temp_log();
        assert_eq!(
            fork_log(&src, &mut empty, Some(0)).unwrap(),
            0,
            "cutoff 0 → empty"
        );
        assert!(
            empty.tail(0).unwrap().is_empty(),
            "the empty fork has no events"
        );
        assert_eq!(
            src.tail(0).unwrap().len(),
            4,
            "the source log is unchanged by a fork"
        );
        let _ = std::fs::remove_file(&spath);
        let _ = std::fs::remove_file(&dpath);
        let _ = std::fs::remove_file(&epath);
    }

    #[test]
    fn fork_drops_the_parent_daemon_cursor_so_the_branch_re_folds_from_the_start() {
        // A fork is a FRESH timeline — it must NOT inherit the parent daemon's resume bookmark (DAEMON_CURSOR),
        // else a daemon started on the fork (no --from) would auto-resume at the parent's high-water mark and
        // SKIP re-performing the branched history. The recorded-effect trail (prim-result-*) and real history
        // DO carry (replay over the fork needs them) — only the resume bookmark is dropped.
        use crate::daemon::{latest_cursor, DAEMON_CURSOR};
        let (spath, mut src) = temp_log();
        inject_genesis(&mut src, GENESIS).unwrap(); // seq 0
        src.append("trigger", b"go").unwrap(); // seq 1
        src.append("prim-result-exec", b"2").unwrap(); // seq 2 (effect trail — MUST carry)
        src.append(DAEMON_CURSOR, b"3").unwrap(); // seq 3 (resume bookmark — MUST be dropped)

        let (dpath, mut dst) = temp_log();
        let copied = fork_log(&src, &mut dst, None).unwrap();
        assert_eq!(
            copied, 3,
            "the daemon-cursor is dropped → 3 of 4 events copied"
        );
        let de = dst.tail(0).unwrap();
        assert!(
            de.iter().all(|e| e.kind != DAEMON_CURSOR),
            "the fork carries NO daemon-cursor record"
        );
        assert_eq!(
            latest_cursor(&de),
            None,
            "the fork has no resume bookmark → a daemon re-folds it from the start (not the parent's cursor)"
        );
        // The effect trail + genesis + trigger still carry (replay/re-fold over the fork needs the full history).
        assert!(
            de.iter().any(|e| e.kind == "prim-result-exec"),
            "the recorded-effect trail carries into the fork (replay needs it)"
        );
        assert_eq!(
            latest_program(&de).as_deref(),
            Some(GENESIS),
            "genesis carries"
        );
        assert!(
            de.iter().any(|e| e.kind == "trigger"),
            "the trigger history carries"
        );
        let _ = std::fs::remove_file(&spath);
        let _ = std::fs::remove_file(&dpath);
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
