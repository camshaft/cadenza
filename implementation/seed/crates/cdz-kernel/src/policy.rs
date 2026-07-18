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

/// The event `kind` for an operator's capability GRANT (the can't-brick escape hatch: an agent's denied op
/// files an `authz-request`; the operator answers with a grant). Payload: the FIRST line is the absolute
/// EXPIRY in ms since the Unix epoch (empty = no expiry), the REST is a narrow Cedar `permit` doc. The grant's
/// own `seq` is its identity — an [`AUTHZ_REVOKE`] references it. Grants ADD to the base [`POLICY`] (they widen
/// only within what the operator explicitly grants; they can't be self-issued — an operator writes them).
pub const AUTHZ_GRANT: &str = "authz-grant";

/// The event `kind` for an operator's REVOKE of a prior grant — payload is the granted event's `seq` (decimal
/// text). [`effective_policy`] drops a grant whose seq has been revoked (operators can pull authorization
/// later, per the operator model).
pub const AUTHZ_REVOKE: &str = "authz-revoke";

/// Append an operator capability GRANT: a narrow Cedar `permit` doc with an optional absolute `expiry_ms`
/// (ms since epoch; `None` = never expires). Encoded as `expiry_line\n<permit>` — the first line is the expiry
/// (empty for none). Returns the grant's `seq` (its identity for a later [`revoke`]). This is the operator's
/// answer to an `authz-request`; grants ADD to the base policy but only what the operator writes.
pub fn append_grant(log: &mut impl Log, permit: &str, expiry_ms: Option<u64>) -> Result<Seq> {
    let expiry_line = expiry_ms.map(|e| e.to_string()).unwrap_or_default();
    let payload = format!("{expiry_line}\n{permit}");
    log.append(AUTHZ_GRANT, payload.as_bytes())
}

/// Append a REVOKE of the grant at `grant_seq` — the operator pulls a prior authorization. [`effective_policy`]
/// thereafter excludes that grant.
pub fn append_revoke(log: &mut impl Log, grant_seq: Seq) -> Result<Seq> {
    log.append(AUTHZ_REVOKE, grant_seq.to_string().as_bytes())
}

/// Parse an [`AUTHZ_GRANT`] payload into `(expiry_ms, permit)`: the first line is the expiry (empty → `None`),
/// the rest is the Cedar permit. A malformed expiry line → `None` (fail-safe: treat as no expiry rather than
/// erroring, since the operator wrote it; the permit still gates).
fn parse_grant(payload: &[u8]) -> Option<(Option<u64>, String)> {
    let text = String::from_utf8(payload.to_vec()).ok()?;
    let (first, rest) = text.split_once('\n')?;
    let expiry = first.trim().parse::<u64>().ok();
    Some((expiry, rest.to_string()))
}

/// The EFFECTIVE capability policy at invocation time `now_ms`: the base [`latest_policy`] doc PLUS every
/// active [`AUTHZ_GRANT`]'s permit — where "active" = NOT revoked (no later [`AUTHZ_REVOKE`] names its seq) AND
/// NOT expired ([`crate::clock::is_expired`]`(expiry, now_ms)` is false). Grants are concatenated after the base
/// (Cedar unions permits: any matching permit allows, deny-by-default otherwise). This is what
/// [`crate::daemon::tick_hosted_log_policy`] evaluates — the operator's request→grant→revoke lifecycle applied
/// per invocation, with wall-clock expiry honored (time is passed in as `now_ms`, so the fold stays pure).
/// Empty string if no base policy and no active grants → Cedar deny-by-default (zero capabilities).
pub fn effective_policy(events: &[crate::Event], now_ms: u64) -> String {
    // Which grant seqs have been revoked.
    let revoked: std::collections::HashSet<crate::Seq> = events
        .iter()
        .filter(|e| e.kind == AUTHZ_REVOKE)
        .filter_map(|e| String::from_utf8(e.payload.clone()).ok())
        .filter_map(|s| s.trim().parse::<crate::Seq>().ok())
        .collect();

    let mut doc = latest_policy(events).unwrap_or_default();
    for e in events.iter().filter(|e| e.kind == AUTHZ_GRANT) {
        if revoked.contains(&e.seq) {
            continue; // operator pulled this grant
        }
        if let Some((expiry, permit)) = parse_grant(&e.payload) {
            if crate::clock::is_expired(expiry, now_ms) {
                continue; // time-boxed grant has lapsed
            }
            if !doc.is_empty() {
                doc.push('\n');
            }
            doc.push_str(&permit);
        }
    }
    doc
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

    #[test]
    fn effective_policy_unions_base_with_active_grants() {
        // The authz lifecycle applied per invocation: effective_policy = base policy ∪ active grants. A base
        // policy permitting prim:append + a grant permitting prim:exec → both permits present. now_ms=0.
        let (path, mut log) = temp_log();
        append_policy(
            &mut log,
            r#"permit(principal, action == Action::"prim:append", resource);"#,
        )
        .unwrap();
        append_grant(
            &mut log,
            r#"permit(principal, action == Action::"prim:exec", resource);"#,
            None,
        )
        .unwrap();
        let eff = effective_policy(&log.tail(0).unwrap(), 0);
        assert!(
            eff.contains("prim:append"),
            "base policy permit present: {eff}"
        );
        assert!(
            eff.contains("prim:exec"),
            "active grant permit unioned in: {eff}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn effective_policy_drops_revoked_and_expired_grants() {
        // A revoked grant + an expired grant are both excluded; a live grant stays.
        let (path, mut log) = temp_log();
        append_policy(&mut log, "// base\n").unwrap();
        // grant A (seq 1) — will be REVOKED.
        let a = append_grant(
            &mut log,
            r#"permit(principal, action == Action::"prim:a", resource);"#,
            None,
        )
        .unwrap();
        // grant B (seq 2) — EXPIRES at ms 50.
        append_grant(
            &mut log,
            r#"permit(principal, action == Action::"prim:b", resource);"#,
            Some(50),
        )
        .unwrap();
        // grant C (seq 3) — live (expires at ms 1000).
        append_grant(
            &mut log,
            r#"permit(principal, action == Action::"prim:c", resource);"#,
            Some(1000),
        )
        .unwrap();
        append_revoke(&mut log, a).unwrap(); // revoke A

        // now_ms = 100: A revoked, B expired (50 <= 100), C live (1000 > 100).
        let eff = effective_policy(&log.tail(0).unwrap(), 100);
        assert!(!eff.contains("prim:a"), "revoked grant excluded: {eff}");
        assert!(!eff.contains("prim:b"), "expired grant excluded: {eff}");
        assert!(eff.contains("prim:c"), "live grant retained: {eff}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn effective_policy_is_empty_with_no_base_and_no_grants() {
        // Deny-by-default substrate: nothing granted → empty doc (Cedar denies all).
        assert_eq!(
            effective_policy(&[], 0),
            "",
            "no base + no grants → empty (deny-by-default)"
        );
    }
}
