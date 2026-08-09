//! Session-status rendering — the `session-status <id>` READ surface (step 2 of the agreed 3-way seam:
//! v-agent-harness provides `Session::status_snapshot`, this host owns the registry + exposes it, the
//! concierge parses the JSON into a human report).
//!
//! The kernel's [`cdz_kernel::kernel::StatusSnapshot`] is a cheap, out-of-band structural read — no event
//! appended, no fold, the session doesn't know it was asked (so a status query can never derail a session
//! mid-work). This module turns that snapshot into a stable JSON string a caller (the `session-status`
//! CLI, wired next) emits and the concierge parses. It answers "is X alive / stalled / idle, and what is
//! it waiting on?" for free; the semantic "what is X actually DOING?" answer is the later fork-for-query
//! path.
//!
//! **Zero-dep JSON.** The crate keeps a minimal dep floor (no `serde`/`serde_json`), so this hand-renders
//! a small, fixed-shape JSON object with a correct string escaper (the private `escape` fn below). The shape is documented
//! here and is the contract the concierge parses against — keep it stable (add fields, don't rename).

use crate::host::{AgentHost, HostedSession, SessionId};
use cdz_kernel::kernel::{SessionState, StatusSnapshot};

/// The default stall threshold: an in-flight effect outstanding longer than this marks the session
/// `Stalled` (likely wedged). 5 minutes — the value the kernel's status tests use.
pub const DEFAULT_STALL_AFTER_MS: u64 = 300_000;

/// Render one session's status as a JSON object string. `now_ms` is the caller's wall clock (for the
/// stall derivation — `None` lets the kernel skip the time-based stall check); `stall_after_ms` is the
/// wedge threshold. Reads `session.status_snapshot` (out-of-band; appends nothing).
///
/// Shape (the concierge's parse contract):
/// ```json
/// {
///   "session_id": "agent-1",
///   "state": "Active",              // Closed | Stalled | Active | Quiescent (kernel-derived)
///   "errored": false,               // true if the tip event is a FoldFailed (a reducer fault)
///   "error_reason": "guest trap: …",// present ONLY when errored:true — the FoldFailed reason
///   "event_count": 5,
///   "last_event_kind": "Dispatched",
///   "armed_timers": 0,
///   "in_flight": [{"kind": "Model", "target": "claude-test"}],
///   "published": {"public/phase": "prompting"}   // the session's own published/ KV view
/// }
/// ```
/// `errored`/`error_reason` are the host's OWN derivation (the kernel `state` has no errored variant): a
/// faulted-then-idle session would read `Quiescent`, so this flag is what tells a supervisor it faulted.
pub fn session_status_json(
    id: &SessionId,
    hosted: &HostedSession,
    now_ms: Option<u64>,
    stall_after_ms: u64,
) -> String {
    let session = hosted.session();
    let snap = session.status_snapshot(now_ms, stall_after_ms);
    render(id, &snap, errored_reason(session))
}

/// Whether the session most-recently FAULTED — its tip event is a `FoldFailed` (a reducer trapped /
/// exhausted fuel / failed to instantiate; §17 the kernel CAPTURES it as a first-class log event rather
/// than a silent stall). The kernel's derived [`SessionState`] has NO "errored" variant (a just-faulted
/// session with no in-flight work reads `Quiescent`, masking the fault), so the host surfaces it here for
/// the supervisor/concierge: `Some(reason)` = errored, `None` = not.
///
/// Delegates to [`Session::last_fault_reason`](cdz_kernel::kernel::Session::last_fault_reason) — the kernel's
/// derived tip-fault accessor (log/state-decouple I5), which reads the resident `tip` (NOT `log().last()`),
/// so this host read stands on its own once the resident log Vec is dropped. Tip-only semantics: a
/// `FoldFailed` the session later progressed past reads `None` (the freshest signal a "what is X doing?"
/// query wants). Returns the kernel's cheaply-clonable `Arc<str>` reason verbatim.
fn errored_reason(session: &cdz_kernel::kernel::Session) -> Option<std::sync::Arc<str>> {
    session.last_fault_reason()
}

/// Look up `id` in the host and render its status JSON, or `None` if no such session (the caller emits an
/// "unknown session" error). The convenience the `session-status <id>` CLI calls.
pub fn host_session_status_json(
    host: &AgentHost,
    id: &SessionId,
    now_ms: Option<u64>,
    stall_after_ms: u64,
) -> Option<String> {
    host.get(id)
        .map(|hosted| session_status_json(id, hosted, now_ms, stall_after_ms))
}

fn state_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Closed => "Closed",
        SessionState::Stalled => "Stalled",
        SessionState::Active => "Active",
        SessionState::Quiescent => "Quiescent",
    }
}

fn render(id: &SessionId, snap: &StatusSnapshot, errored: Option<std::sync::Arc<str>>) -> String {
    let mut out = String::from("{");
    out.push_str(&format!("\"session_id\":{},", escape(&id.to_hex())));
    out.push_str(&format!("\"state\":{},", escape(state_str(snap.state))));
    // A just-faulted session (tip = FoldFailed) is `errored:true` with the trap reason — the kernel's
    // structural `state` can't express this (no Errored variant), so a supervisor/concierge reads this
    // flag to distinguish "errored" from a benign Quiescent/Active. `error_reason` is present only when
    // errored (omitted otherwise to keep the common object small).
    match &errored {
        Some(reason) => {
            out.push_str("\"errored\":true,");
            out.push_str(&format!("\"error_reason\":{},", escape(reason.as_ref())));
        }
        None => out.push_str("\"errored\":false,"),
    }
    out.push_str(&format!("\"event_count\":{},", snap.event_count));
    out.push_str(&format!(
        "\"last_event_kind\":{},",
        escape(snap.last_event_kind)
    ));
    out.push_str(&format!("\"armed_timers\":{},", snap.armed_timers));

    out.push_str("\"in_flight\":[");
    for (i, f) in snap.in_flight.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"kind\":{},\"target\":{}}}",
            escape(f.kind),
            escape(&f.target)
        ));
    }
    out.push_str("],");

    // The session's own published view (public/ KV entries). Keys/values are bytes rendered as UTF-8
    // lossily for the JSON (a published status is text by convention; non-UTF-8 degrades to replacement
    // chars rather than failing the whole render — a status read must never panic on odd bytes).
    out.push_str("\"published\":{");
    for (i, (k, v)) in snap.published.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let key = String::from_utf8_lossy(k);
        let val = String::from_utf8_lossy(v);
        out.push_str(&format!("{}:{}", escape(&key), escape(&val)));
    }
    out.push('}');

    out.push('}');
    out
}

/// Escape a string as a JSON string literal (including the surrounding quotes). Handles the control set
/// JSON requires (`"`, `\`, `\n`, `\r`, `\t`, and other control chars via `\u00XX`) so the output is
/// valid JSON the concierge can parse with any standard parser. Total — never panics.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::hash::Hash;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// An agent that, on "go", publishes a status under `public/` and arms a far-future TIMER. A timer is
    /// a kernel-internal open obligation (it needs no executor), so it stays armed → the session reads
    /// `Active` with a published view — the state this test asserts the status render exposes.
    struct PublishAndTime;
    #[async_trait::async_trait(?Send)]
    impl Reducer for PublishAndTime {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    kv.put(b"public/phase".to_vec(), b"working".to_vec());
                    // Arm a timer far in the future → stays armed → session is Active.
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::TIMER,
                        "999999999",
                        None,
                        Timeliness::Interactive,
                    )])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn inbound_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    fn timer_host() -> HostedSession {
        // Timer effects are kernel-internal (no executor needed); a ClockExecutor is registered but unused.
        let executor = cdz_kernel::executor::CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        HostedSession::genesis(
            Hash::of(b"publish-time-v1"),
            Box::new(PublishAndTime),
            Box::new(authz),
            executor,
        )
    }

    #[tokio::test]
    async fn renders_active_session_with_published_view_as_json() {
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"agent-1"));
        host.spawn(id, timer_host());
        host.deliver(&id, inbound_go(), None).await;

        let json = host_session_status_json(&host, &id, Some(1000), DEFAULT_STALL_AFTER_MS)
            .expect("session exists");
        // Structural facts: an armed timer → Active; the published view is exposed.
        assert!(
            json.contains(&format!("\"session_id\":\"{}\"", id.to_hex())),
            "{json}"
        );
        assert!(json.contains("\"state\":\"Active\""), "{json}");
        assert!(json.contains("\"armed_timers\":1"), "{json}");
        assert!(
            json.contains("\"public/phase\":\"working\""),
            "published view exposed: {json}"
        );
        // A healthy session is NOT errored, and no reason field appears.
        assert!(json.contains("\"errored\":false"), "{json}");
        assert!(
            !json.contains("error_reason"),
            "no reason when not errored: {json}"
        );
    }

    #[tokio::test]
    async fn a_faulted_session_reports_errored_with_the_reason() {
        // A reducer whose fold FAILS (FoldOutput::failed — the Rust analogue of a wasm guest trap) makes
        // the kernel record a FoldFailed log event (§17: captured, never a panic). The kernel's structural
        // `state` has no errored variant, so the host surfaces `errored:true` + the reason.
        struct Faulter;
        #[async_trait::async_trait(?Send)]
        impl Reducer for Faulter {
            async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
                if matches!(event.body, EventBody::Inbound { .. }) {
                    FoldOutput::failed("guest trap: divide by zero")
                } else {
                    FoldOutput::none()
                }
            }
        }
        let executor = cdz_kernel::executor::CompositeExecutor::new();
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"boom"));
        host.spawn(
            id,
            HostedSession::genesis(
                Hash::of(b"faulter"),
                Box::new(Faulter),
                Box::new(Authorizer::deny_all()),
                executor,
            ),
        );
        host.deliver(&id, inbound_go(), None).await;

        let json = host_session_status_json(&host, &id, Some(0), DEFAULT_STALL_AFTER_MS).unwrap();
        assert!(json.contains("\"errored\":true"), "{json}");
        assert!(
            json.contains("\"error_reason\":\"guest trap: divide by zero\""),
            "the FoldFailed reason is surfaced: {json}"
        );
        assert!(
            json.contains("\"last_event_kind\":\"FoldFailed\""),
            "{json}"
        );
    }

    #[tokio::test]
    async fn a_fault_the_session_progressed_past_reads_not_errored() {
        // TIP-ONLY semantics (the property that distinguishes `errored` from "ever faulted"): a session that
        // FAULTS and then makes further progress is NOT errored — the fault is no longer the tip. This pins
        // the host's `errored` derivation onto the kernel's `last_fault_reason()` tip read (log/state-decouple
        // I5): a regression that reported "ever-faulted" (e.g. scanning the whole log) would flip this to true.
        struct FaultThenProgress {
            seen: u32,
        }
        #[async_trait::async_trait(?Send)]
        impl Reducer for FaultThenProgress {
            async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
                if matches!(event.body, EventBody::Inbound { .. }) {
                    self.seen += 1;
                    if self.seen == 1 {
                        // First message FAULTS.
                        FoldOutput::failed("guest trap: transient")
                    } else {
                        // A later message PROGRESSES (writes KV) — the tip is no longer the FoldFailed.
                        kv.put(b"recovered".to_vec(), b"1".to_vec());
                        FoldOutput::none()
                    }
                } else {
                    FoldOutput::none()
                }
            }
        }
        let executor = cdz_kernel::executor::CompositeExecutor::new();
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"recovers"));
        host.spawn(
            id,
            HostedSession::genesis(
                Hash::of(b"fault-then-progress"),
                Box::new(FaultThenProgress { seen: 0 }),
                Box::new(Authorizer::deny_all()),
                executor,
            ),
        );
        host.deliver(&id, inbound_go(), None).await; // faults
        host.deliver(&id, inbound_go(), None).await; // progresses past the fault

        let json = host_session_status_json(&host, &id, Some(0), DEFAULT_STALL_AFTER_MS).unwrap();
        assert!(
            json.contains("\"errored\":false"),
            "a fault the session progressed past is NOT errored (tip-only): {json}"
        );
        assert!(
            !json.contains("error_reason"),
            "no reason once the fault is no longer the tip: {json}"
        );
    }

    #[test]
    fn unknown_session_is_none() {
        let host = AgentHost::new();
        assert!(host_session_status_json(
            &host,
            &SessionId::new(Hash::of(b"nope")),
            Some(0),
            DEFAULT_STALL_AFTER_MS
        )
        .is_none());
    }

    #[test]
    fn escape_handles_quotes_and_controls() {
        assert_eq!(escape("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape("a\nb\tc"), "\"a\\nb\\tc\"");
        assert_eq!(escape("x\u{0001}y"), "\"x\\u0001y\"");
        assert_eq!(escape("plain"), "\"plain\"");
    }

    #[tokio::test]
    async fn a_quiescent_session_reports_no_in_flight() {
        // Drive a clock agent to completion → Quiescent, empty in_flight.
        struct ClockOnce;
        #[async_trait::async_trait(?Send)]
        impl Reducer for ClockOnce {
            async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
                if matches!(event.body, EventBody::Inbound { .. }) {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::NOW,
                        String::new(),
                        None,
                        Timeliness::Interactive,
                    )])
                } else {
                    FoldOutput::none()
                }
            }
        }
        let executor = cdz_kernel::executor::CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }]);
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"q"));
        host.spawn(
            id,
            HostedSession::genesis(
                Hash::of(b"clock-once"),
                Box::new(ClockOnce),
                Box::new(authz),
                executor,
            ),
        );
        host.deliver(&id, inbound_go(), None).await;

        let json = host_session_status_json(&host, &id, Some(0), DEFAULT_STALL_AFTER_MS).unwrap();
        assert!(json.contains("\"state\":\"Quiescent\""), "{json}");
        assert!(json.contains("\"in_flight\":[]"), "{json}");
    }
}
