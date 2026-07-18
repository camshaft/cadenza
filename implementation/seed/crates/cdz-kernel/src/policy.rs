//! Capability policies in the log (minimal-kernel re-charter, capability-attenuation — operator ruling).
//!
//! Operator model (Slack ruling, relayed 2026-07): capability policies are CEDAR DOCUMENTS written to the
//! EVENT LOG (like everything else — append-a-Cedar-doc), retrieved AT PROGRAM-INVOCATION, evaluated to
//! DE-ESCALATE a program's privileges to the minimal set it actually needs (a calculator runs with ZERO
//! privileges it doesn't use). The kernel grants the broad primitives; Cedar-from-the-log attenuates each
//! invocation. The "unforgeable" part: a program can't grant itself capabilities Cedar didn't — the kernel
//! checks the Cedar-evaluated grant, not the program's self-assertion.
//!
//! This module is the LOG side of that model — the counterpart of [`crate::boot`] for policies: a `policy`
//! event kind, an appender, and the retrieve-the-latest fold ([`latest_policy`]) the invocation path uses to
//! get the effective Cedar doc. A later `policy` event supersedes the prior (self-superseding, same as
//! `program`) — and because supersession can only REPLACE with a narrower/other Cedar doc that the kernel then
//! evaluates fresh, a program still cannot escalate beyond what the current doc grants (attenuation is enforced
//! at evaluation, per [`crate::daemon::tick_hosted_authorized`], not by trusting the appended doc).
//!
//! The actual Cedar EVALUATION (grant → attenuated capability set) is `cdz_agent::cedar`; this module only
//! decides WHICH Cedar doc is in force (the latest in the log). Wiring `latest_policy` into the daemon's
//! authorize path (so the daemon reads the policy FROM THE LOG instead of an external `--policies` file) is the
//! next increment; this establishes the log-as-policy-store foundation.

use crate::{Log, Seq};
use anyhow::Result;

/// The event `kind` tag for a capability-policy event — its payload is a CEDAR POLICY DOCUMENT (the same
/// policy TEXT `cdz_agent::cedar::authorize` parses). Appended like any event; a later `policy` event
/// supersedes the prior (the invocation path evaluates the LATEST). Reserved like `program`/`prim-*`: an
/// operator emits it via a dedicated path, not the free-`emit` trigger door.
pub const POLICY: &str = "policy";

/// Append a capability-policy Cedar document to `log` as a `policy` event, returning its assigned `seq`. This
/// is how a policy enters the log (operator or a delegating agent writes it); [`latest_policy`] reads the
/// newest back at invocation. A later append SUPERSEDES the prior — the self-superseding policy path (same
/// shape as [`crate::boot::inject_genesis`] for programs).
pub fn append_policy(log: &mut impl Log, cedar_doc: &str) -> Result<Seq> {
    log.append(POLICY, cedar_doc.as_bytes())
}

/// The LATEST capability-policy Cedar doc in `log` — the effective policy the invocation path evaluates
/// (fold the log for the newest `policy` event, decode its payload as UTF-8). `None` if the log holds no
/// `policy` event yet (deny-by-default upstream: with no policy in force, the authorize path denies — Cedar
/// is deny-by-default, and the daemon fails closed). A later `policy` event transparently supersedes — same
/// fold, newer winner, exactly like [`crate::boot::latest_program`].
pub fn latest_policy(events: &[crate::Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|e| e.kind == POLICY)
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
            std::env::temp_dir().join(format!("cdz-kernel-policy-{}-{n}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    #[test]
    fn latest_policy_is_none_before_any_policy_event() {
        // No policy in the log → None (the authorize path then fails closed / deny-by-default).
        assert!(latest_policy(&[]).is_none(), "empty log → no policy");
        let program = crate::Event {
            seq: 0,
            kind: crate::boot::PROGRAM.to_string(),
            payload: b"(do)".to_vec(),
        };
        assert!(
            latest_policy(&[program]).is_none(),
            "a non-policy event does not count as a policy"
        );
    }

    #[test]
    fn append_then_latest_round_trips_the_cedar_doc() {
        let (path, mut log) = temp_log();
        let doc = r#"permit(principal, action == Action::"prim:append", resource);"#;
        let seq = append_policy(&mut log, doc).unwrap();
        assert_eq!(seq, 0, "the first policy lands at seq 0");
        let events = log.tail(0).unwrap();
        assert_eq!(
            latest_policy(&events).as_deref(),
            Some(doc),
            "latest_policy reads back the appended Cedar doc"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_later_policy_supersedes_the_earlier() {
        // Self-superseding: a newer `policy` event wins the fold (the attenuation model's policy-update path).
        let (path, mut log) = temp_log();
        append_policy(&mut log, "permit(principal, action, resource);").unwrap();
        let narrower = r#"permit(principal, action == Action::"prim:append", resource);"#;
        append_policy(&mut log, narrower).unwrap();
        let events = log.tail(0).unwrap();
        assert_eq!(
            latest_policy(&events).as_deref(),
            Some(narrower),
            "the LATEST policy event is the effective one (newer supersedes)"
        );
        let _ = std::fs::remove_file(&path);
    }
}
