//! The daemon loop (minimal-kernel re-charter, rung KC-daemon — the "start the daemon" half).
//!
//! Operator fork-5: the CLI (1) bootstraps the log + injects genesis ([`crate::boot`]), then (2) STARTS THE
//! DAEMON. This module is (2): the daemon's per-event step — read the log, find the latest genesis `program`
//! ([`crate::boot::latest_program`], no hardcoded genesis), and drive an interpret turn on the landed event
//! ([`crate::kernel::run_interpret`], which compiles the program + runs it + executes its host-ops). The
//! daemon understands NO events; the `program` (Cadenza) decides everything. This ties the boot half (genesis
//! lookup) + the kernel half (compile+run) into the loop spine.
//!
//! [`tick`] is one step (pure over the [`crate::Log`] + the runtime store, testable with a `FileLog` + an
//! injected genesis); a real daemon calls it per landed event. The event-source loop (subscribe/poll the log
//! for new events, advance a cursor) is a thin driver on top — [`tick`] is the load-bearing per-event core.

use crate::Log;
use anyhow::{anyhow, Result};

/// The outcome of one daemon tick: the program that ran + the number of host-ops the interpret scheduled for
/// the event (via the counting executor — [`crate::kernel::run_interpret`]). A caller/loop records this; a
/// perform-executor variant would instead execute the ops (K1c). `None` program means the log has no genesis
/// yet (the CLI hasn't injected one — nothing to run).
#[derive(Debug, PartialEq, Eq)]
pub struct Tick {
    pub op_count: i64,
}

/// Run ONE daemon step for a landed event of kind `event_kind`: read the log tail, find the LATEST genesis
/// `program` source ([`crate::boot::latest_program`] — no hardcoded genesis, self-superseding), and drive an
/// interpret turn ([`crate::kernel::run_interpret`], which compiles the program to a provider + runs it via a
/// peer executor over the value-heap `runtime`). Returns the [`Tick`] outcome. Errors if the log has no
/// genesis program yet (the CLI must inject one first) or the interpret run fails. `runtime` is the value-heap
/// wasm (resolve via [`crate::kernel::find_runtime_for`] on the compiled program; a caller skips if absent).
pub fn tick(log: &impl Log, event_kind: i64, runtime: Vec<u8>) -> Result<Tick> {
    let program = latest_or_err(log)?;
    let run = crate::kernel::run_interpret(&program, event_kind, runtime)?;
    Ok(Tick {
        op_count: run.op_count,
    })
}

/// Run ONE daemon step that EXECUTES the ops (not just counts them): same genesis lookup as [`tick`], but
/// drives the PERFORMING executor ([`crate::kernel::run_interpret_performing`], K1c) so each host-op the
/// interpret schedules fires a real `Prim.run` perform (the exec/http/log primitives, in-program-mocked at
/// this rung). Returns the number of ops PERFORMED. This is the daemon's real execution step — the counting
/// [`tick`] is the projection/inspection variant; a live daemon uses this to actually run the plan. (Binding
/// `Prim` to the real host primitives instead of the mock is the K1c→host-primitive rung.)
pub fn tick_performing(log: &impl Log, event_kind: i64, runtime: Vec<u8>) -> Result<i64> {
    let program = latest_or_err(log)?;
    crate::kernel::run_interpret_performing(&program, event_kind, runtime)
}

/// The event kind prefix a performed host-op is RECORDED under in the log (`prim-exec`/`prim-http`/
/// `prim-append`). Each perform appends one such event carrying the op's String payload — the §2.3
/// recorded-effect trail (like `fold`'s `model-request`/`model-response`), so a re-fold replays the plan.
pub const PRIM_RECORD_PREFIX: &str = "prim-";

/// Run ONE daemon step that PERFORMS the ops via a REAL host primitive and RECORDS each perform in the log
/// (the K1c→host-primitive rung). Same genesis lookup as [`tick`], but drives the HOSTED executor
/// ([`crate::kernel::run_interpret_hosted`]): each scheduled `Prim.exec`/`Prim.http`/`Prim.append` is answered
/// by a real closure that (1) appends a `prim-<op>` event carrying the op's String payload to `log` (the
/// recorded-effect trail, like [`crate::fold::drive_one_turn`]), and (2) returns a per-op scalar the fold sums.
///
/// This first cut is a SAFE record-only primitive — it logs each perform, no subprocess/network. Live `exec`
/// (subprocess) / `http` (network) cross the capability boundary and must be gated behind Cedar authorization
/// (the charter's on-behalf-of model) before they run for real; this rung proves the daemon→hosted→real-
/// closure→log-event path with a primitive that only records. Returns the summed per-op result. `log` is shared
/// with the recording closure via `Arc<Mutex>` (the closure runs inside the wasmtime host call, must be
/// `Send + Sync + 'static`); the daemon is single-threaded so the lock is uncontended.
pub fn tick_hosted<L>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64>
where
    L: Log + Send + 'static,
{
    let program = {
        let l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
        crate::boot::latest_program(&l.tail(0)?).ok_or_else(|| {
            anyhow!("no genesis program in the log (CLI must inject one before the daemon runs)")
        })?
    };
    // The real (record-only) primitive: append a `prim-<op>` event carrying the payload, return a per-op
    // scalar (Append→1, Exec→2, Http→3 — the same tags the mock used, so behavior is comparable). An append
    // failure can't surface through the `-> i64` host-op contract, so it's a best-effort record (a gap shows
    // as a missing event, not a wrong result), matching `fold::drive_one_turn`'s recording discipline.
    let prim = {
        let log = std::sync::Arc::clone(&log);
        move |op: &str, payload: String| -> i64 {
            if let Ok(mut l) = log.lock() {
                let _ = l.append(&format!("{PRIM_RECORD_PREFIX}{op}"), payload.as_bytes());
            }
            perform_op_body(op, &payload)
        }
    };
    crate::kernel::run_interpret_hosted(&program, event_kind, runtime, prim)
}

/// The per-op effect BODY: given an op name + payload, do the effect and return its scalar result. The
/// `append`/`http` ops (and the default) are RECORD-ONLY — the caller already appended the `prim-<op>` event;
/// this returns the op's tag (Append→1, Http→3, Noop/other→0). `exec` is the one LIVE effect, and only when
/// built with the `live-exec` feature: it spawns a REAL subprocess (`std::process::Command`, the payload as a
/// shell line) and returns its exit code — a SECOND, build-time boundary on top of the Cedar gate (a build
/// WITHOUT `live-exec` cannot spawn, whatever the policy says — the safe default). Without the feature, `exec`
/// stays record-only (tag 2), identical to before.
fn perform_op_body(op: &str, _payload: &str) -> i64 {
    match op {
        "exec" => {
            #[cfg(feature = "live-exec")]
            {
                // Live: run the payload as a shell command; the exit code is the op's scalar result (127 if
                // the process couldn't even be spawned — the shell's convention for "command not found").
                use std::process::Command;
                match Command::new("sh").arg("-c").arg(_payload).status() {
                    Ok(status) => status.code().unwrap_or(-1) as i64,
                    Err(_) => 127,
                }
            }
            #[cfg(not(feature = "live-exec"))]
            {
                2 // record-only default: the same tag the mock used (no subprocess spawned).
            }
        }
        "http" => 3, // record-only (no HTTP-client dep in the minimal kernel).
        "append" => 1,
        _ => 0,
    }
}

/// The event kind prefix a DENIED host-op is recorded under (`prim-denied-exec`/…). A perform the Cedar policy
/// forbids appends this instead of the effect + returns 0, so the log carries the denial (audit trail).
pub const PRIM_DENIED_PREFIX: &str = "prim-denied-";

/// The agent principal UID authorization requests are made under (the daemon acts as this Cedar entity).
pub const AGENT_PRINCIPAL: &str = "Agent::\"cdz-agent\"";

/// Escape a string for embedding inside a Cedar string literal (an `EntityUid` like `Resource::"<s>"`).
/// A raw `"`/`\`/control char would break the literal — closing it early (matching an UNINTENDED policy) or
/// failing to parse (a spurious deny). Escapes backslash + double-quote + the common control chars per Cedar's
/// string grammar, so a policy can safely key on an ARBITRARY op payload. (Copilot PR#556 hardening.)
fn cedar_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

/// Like [`tick_hosted`], but GATES each real primitive behind Cedar authorization against an EXTERNAL trust-
/// anchor policy (the charter's capability boundary; concierge ruling `5828` — build the external anchor
/// first). `root_policies` is operator-controlled Cedar policy TEXT the agent CANNOT modify or widen (a
/// `--policies` file at the CLI); before performing each op the closure asks
/// [`cdz_agent::cedar::authorize`]`(root_policies, `[`AGENT_PRINCIPAL`]`, Action::"prim:<op>", <payload>, {})`.
/// ALLOW → perform + record `prim-<op>` (as [`tick_hosted`]); DENY (Cedar is deny-by-default) → record
/// `prim-denied-<op>` + return 0, skipping the effect. So an op runs ONLY if the external policy permits it.
///
/// This is the external-anchor half of the hybrid; attenuation-only `policy` LOG EVENTS (narrow-never-widen,
/// via `authorize_on_behalf_of`) are the follow-on. Still record-only for the effect body (no live subprocess/
/// network yet) — this rung proves the AUTHORIZE GATE sits on the perform path. `resource` is derived from the
/// op's payload as a `Resource` UID so a policy can condition on it (`Resource::"<payload>"`).
pub fn tick_hosted_authorized<L>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    event_kind: i64,
    runtime: Vec<u8>,
    root_policies: String,
) -> Result<i64>
where
    L: Log + Send + 'static,
{
    let program = {
        let l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
        crate::boot::latest_program(&l.tail(0)?).ok_or_else(|| {
            anyhow!("no genesis program in the log (CLI must inject one before the daemon runs)")
        })?
    };
    let prim = {
        let log = std::sync::Arc::clone(&log);
        move |op: &str, payload: String| -> i64 {
            // Authorize against the external root policy BEFORE performing. A policy PARSE error or any
            // authz error is treated as DENY (fail-closed — never perform an op we couldn't authorize).
            // ESCAPE the payload before embedding it in the Cedar EntityUid string literal: a raw `"`/`\`/
            // control char would break the UID (spurious parse-fail → wrong deny) or, worse, close the literal
            // early and match an UNINTENDED policy rule (Copilot PR#556). `op` is a fixed identifier, no escape.
            let action = format!("Action::\"prim:{op}\"");
            let resource = format!("Resource::\"{}\"", cedar_escape(&payload));
            let allowed = cdz_agent::cedar::authorize(
                &root_policies,
                AGENT_PRINCIPAL,
                &action,
                &resource,
                "{}",
            )
            .map(|d| d.is_allow())
            .unwrap_or(false);
            if !allowed {
                // Denied (or unauthorizable) → record the denial AND AUTO-EMIT an authz-request (the can't-
                // brick guarantee, operator ruling): a denial files the narrow ask (this op + payload) in the
                // log so the operator always sees it and can grant it. Do NOT perform; contribute 0 to the sum.
                if let Ok(mut l) = log.lock() {
                    let _ = l.append(&format!("{PRIM_DENIED_PREFIX}{op}"), payload.as_bytes());
                    let _ = crate::policy::append_request(&mut *l, op, &payload);
                }
                return 0;
            }
            // Allowed → perform the effect body (live `exec` iff the `live-exec` feature is on) + record it.
            if let Ok(mut l) = log.lock() {
                let _ = l.append(&format!("{PRIM_RECORD_PREFIX}{op}"), payload.as_bytes());
            }
            perform_op_body(op, &payload)
        }
    };
    crate::kernel::run_interpret_hosted(&program, event_kind, runtime, prim)
}

/// The daemon's real-primitive step with the Cedar policy RETRIEVED FROM THE LOG (the operator's capability
/// model, retrieve-at-invocation half): read the LATEST `policy` event ([`crate::policy::latest_policy`]) and
/// gate every op against it via [`tick_hosted_authorized`]. So a `policy` Cedar doc APPENDED to the log governs
/// the NEXT invocation — no external `--policies` file, no kernel redeploy: the policy is data in the log,
/// exactly like the genesis program. A later `policy` event supersedes (attenuation is re-evaluated fresh each
/// invocation against whatever doc is latest — a program can't escalate past it).
///
/// DENY-BY-DEFAULT, STRICT (operator ruling: "actions should never go through without a policy"): if the log
/// holds NO `policy` event, every op is DENIED — gated against an EMPTY policy (Cedar is deny-by-default: no
/// matching `permit` → deny). There is NO permissive bootstrap seed; the out-of-box state is deny. The
/// can't-brick escape hatch is a SEPARATE request→grant→revoke lifecycle (an agent requests narrow
/// authorization from the operator; the operator grants with optional expiry, or denies, or revokes later) —
/// NOT a broad default grant. This fn's job is enforcement; the request/grant lifecycle rides on top.
pub fn tick_hosted_log_policy<L>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64>
where
    L: Log + Send + 'static,
{
    // The EFFECTIVE policy at THIS invocation: the base `policy` doc UNIONED with every active operator GRANT
    // (not revoked, not expired) — read the wall clock ONCE here and pass it in as data so the fold stays pure
    // ([`crate::policy::effective_policy`]). No policy + no active grant → empty doc → Cedar deny-by-default
    // (zero capabilities; actions never go through without a policy). This is the authz request→grant→revoke
    // lifecycle applied per invocation with WALL-CLOCK expiry (operator ruling).
    let now_ms = crate::clock::now_millis();
    let doc = {
        let l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
        crate::policy::effective_policy(&l.tail(0)?, now_ms)
    };
    tick_hosted_authorized(log, event_kind, runtime, doc)
}

/// Drive `step` over every log event NOT YET processed (seq >= `from`), in order, and return the NEW cursor
/// (the seq AFTER the last event seen, i.e. the `from` to pass next time) — the event-source LOOP's pure,
/// single-pass core (the "start the daemon" half's load-bearing step). A real daemon calls this repeatedly
/// (sleep + poll), advancing the returned cursor each round; this pure form is testable with a `FileLog` and
/// no sleep/thread. `step` decides what to do per event (typically pick an event kind + call [`tick`] /
/// [`tick_hosted`]) — the daemon itself understands NO events, exactly as the per-event core does. A `step`
/// error aborts the drain at that event and propagates (the caller retries from the returned-so-far cursor is
/// NOT attempted here — a failing step is a loud stop, not a silent skip); the cursor returned on success is
/// `last_seq + 1`, or `from` unchanged when nothing new landed.
pub fn drain_new_events<L, F>(log: &L, from: crate::Seq, mut step: F) -> Result<crate::Seq>
where
    L: Log,
    F: FnMut(&crate::Event) -> Result<()>,
{
    let events = log.tail(from)?;
    let mut cursor = from;
    for event in &events {
        step(event)?;
        cursor = event.seq + 1;
    }
    Ok(cursor)
}

/// Run ONE round of the live daemon over the shared `log`: drain every event since `from` and PERFORM each
/// via [`tick_hosted`] (real host primitive + recorded-effect trail), returning `(new_cursor, processed)` —
/// the count of events ticked. This is the live daemon's testable loop BODY (no sleep/thread); [`run`] is the
/// thin blocking wrapper that calls it in a poll loop. `kind_of` maps each landed event to the scalar event
/// kind the interpret program sees (the daemon understands NO events — the caller decides the mapping).
///
/// `runtime` is the value-heap runtime wasm, resolved ONCE by the caller. It is content-addressed and the same
/// across interpret programs, so a genesis supersession ([`crate::boot`]) that keeps the same runtime is picked
/// up automatically (each [`tick_hosted`] re-reads the LATEST program). A supersession to a program needing a
/// DIFFERENT runtime hash requires a restart (a minimal-kernel limitation at this rung — the driver resolves
/// runtime up front, not per tick). Reads the tail under a brief lock (clone + release) so per-event
/// [`tick_hosted`] can re-lock without nesting.
///
/// `kind_of` maps each landed event to `Some(kind)` (a TRIGGER — perform the interpret plan for that scalar
/// kind) or `None` (SKIP — not a trigger). This skip is load-bearing: the daemon RECORDS its own effects back
/// into the same log (`prim-*` events) and the genesis is a `program` event, so without a skip the daemon
/// would re-process its own bookkeeping as triggers and never converge. The cursor still advances PAST skipped
/// events (they're seen-and-dismissed, not re-read), so a round always drains to the log tail.
///
/// `policies` selects the capability boundary: `Some(root)` is an explicit external Cedar trust-anchor that
/// OVERRIDES the log — every op gates against `root` via [`tick_hosted_authorized`]; `None` is the operator's
/// default — the policy is read FROM THE LOG per invocation via [`tick_hosted_log_policy`] (the latest `policy`
/// event governs; NO policy in the log → DENY-BY-DEFAULT, every op denied). So a live daemon started with no
/// `--policies` file enforces whatever `policy` Cedar doc is appended to the log — and denies until one is —
/// the retrieve-at-invocation model; an explicit file is the override for a fixed external anchor.
pub fn run_once<L, K>(
    log: &std::sync::Arc<std::sync::Mutex<L>>,
    from: crate::Seq,
    runtime: Vec<u8>,
    policies: Option<String>,
    kind_of: K,
) -> Result<(crate::Seq, usize)>
where
    L: Log + Send + 'static,
    K: Fn(&crate::Event) -> Option<i64>,
{
    let events = {
        let l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
        l.tail(from)?
    };
    let mut cursor = from;
    let mut processed = 0usize;
    for event in &events {
        if let Some(kind) = kind_of(event) {
            match &policies {
                // Explicit external anchor: gate against the fixed doc (overrides the log).
                Some(root) => {
                    tick_hosted_authorized(
                        std::sync::Arc::clone(log),
                        kind,
                        runtime.clone(),
                        root.clone(),
                    )?;
                }
                // Default: policy FROM THE LOG per invocation (latest `policy` event; none → ungated).
                None => {
                    tick_hosted_log_policy(std::sync::Arc::clone(log), kind, runtime.clone())?;
                }
            }
            processed += 1;
        }
        cursor = event.seq + 1;
    }
    Ok((cursor, processed))
}

/// FIRE the schedules due as of the wall clock, returning how many fired — the read-side that makes the
/// [`crate::schedule`] substrate LIVE. Reads [`crate::clock::now_millis`] ONCE (the single ambient-time read),
/// folds [`crate::schedule::due`] to the due set, and for each: appends the schedule's TRIGGER event (its
/// `trigger_kind` + `payload`) so [`run_once`] performs it this same round, then appends a
/// [`crate::schedule::append_fire`] record so the pure `due` fold advances that schedule's occurrence counter
/// (a one-shot won't re-fire; a periodic's next occurrence moves one period ahead). Fire-record AFTER the
/// trigger so a crash between them re-fires rather than silently dropping (at-least-once, not at-most-once).
/// Called at the TOP of each [`run`] round, before [`run_once`] drains — so a fired trigger is picked up in
/// the same round. One occurrence per schedule per round (the `due` fold reports each due schedule once; its
/// fire record advances `k`). Skips a schedule whose `trigger_kind` is a RESERVED kind (a genesis/policy/prim/
/// schedule bookkeeping kind) — a schedule must fire a real trigger, not forge kernel bookkeeping.
pub fn fire_due_schedules<L>(log: &std::sync::Arc<std::sync::Mutex<L>>) -> Result<usize>
where
    L: Log + Send + 'static,
{
    let now_ms = crate::clock::now_millis();
    let mut l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
    let due = crate::schedule::due(&l.tail(0)?, now_ms);
    let mut fired = 0usize;
    for s in &due {
        // A schedule must not forge kernel bookkeeping (genesis/policy/prim/schedule-*) via its trigger.
        if is_reserved_kind(&s.trigger_kind) {
            continue;
        }
        l.append(&s.trigger_kind, &s.payload)?;
        crate::schedule::append_fire(&mut *l, &s.id)?;
        fired += 1;
    }
    Ok(fired)
}

/// Whether `kind` is a RESERVED event kind the daemon/boot own — a genesis `program`, a `policy`/authz doc, a
/// `prim-*` effect record, or a `schedule-*` bookkeeping event. A `fire_due_schedules` trigger and the CLI's
/// free `emit` both refuse these (an operator/schedule can't forge kernel bookkeeping through a trigger door).
pub fn is_reserved_kind(kind: &str) -> bool {
    kind == crate::boot::PROGRAM
        || kind == crate::policy::POLICY
        || kind == crate::policy::AUTHZ_GRANT
        || kind == crate::policy::AUTHZ_REVOKE
        || kind == crate::policy::AUTHZ_REQUEST
        || kind == crate::schedule::SCHEDULE_CREATE
        || kind == crate::schedule::SCHEDULE_CANCEL
        || kind == crate::schedule::SCHEDULE_FIRE
        || kind.starts_with(PRIM_RECORD_PREFIX)
}

/// The LIVE daemon: poll `log` for new events and PERFORM each via [`run_once`], sleeping `poll` between
/// rounds — the "start the daemon" half realized as a blocking loop. Runs until `should_stop()` returns true
/// (checked each round, before sleeping) or a tick errors. Starts at `from` and advances the cursor across
/// rounds. Each round FIRES due schedules first ([`fire_due_schedules`]) so a timer's trigger is performed the
/// same round. A thin wrapper over [`run_once`] (the tested body) + `std::thread::sleep`; the loop itself
/// carries no logic. `should_stop` lets a caller (a stop-file poll, a signal flag) end the loop cleanly between
/// rounds. `policies` is the OPTIONAL external Cedar trust-anchor forwarded to each [`run_once`] round (`Some`
/// gates every op via [`tick_hosted_authorized`]; `None` runs ungated) — the operator's `--policies` flag.
pub fn run<L, K, S>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    mut from: crate::Seq,
    runtime: Vec<u8>,
    poll: std::time::Duration,
    policies: Option<String>,
    kind_of: K,
    mut should_stop: S,
) -> Result<()>
where
    L: Log + Send + 'static,
    K: Fn(&crate::Event) -> Option<i64>,
    S: FnMut() -> bool,
{
    while !should_stop() {
        // Fire due schedules FIRST — each appends its trigger event, which run_once then drains this same round.
        fire_due_schedules(&log)?;
        let (next, _processed) = run_once(&log, from, runtime.clone(), policies.clone(), &kind_of)?;
        from = next;
        std::thread::sleep(poll);
    }
    Ok(())
}

/// The latest genesis `program` source in `log`, or a loud error if none (the CLI must inject one first).
fn latest_or_err(log: &impl Log) -> Result<String> {
    let events = log.tail(0)?;
    crate::boot::latest_program(&events).ok_or_else(|| {
        anyhow!("no genesis program in the log (CLI must inject one before the daemon runs)")
    })
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
            std::env::temp_dir().join(format!("cdz-kernel-daemon-{}-{n}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    /// A genesis interpret program: kind==1 → a 2-op plan, else → 1-op. (Same shape as the kernel fixture — a
    /// branch-built (List HostOp) that escapes as a heap handle.)
    const GENESIS: &str = "(do \
        (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
        (def (interpret (: kind Int64) (: turn Int64)) \
          (if (= kind 1) (list (Append \"ack\") (Exec \"handle\")) (list (Noop 0)))) \
        (export interpret))";

    #[test]
    fn a_tick_with_no_genesis_is_a_loud_error() {
        // The daemon can't run before the CLI injects a genesis program — a loud error, not a silent no-op.
        let (path, log) = temp_log();
        // No runtime needed to reach the no-genesis error (it's before compile) — pass empty bytes.
        assert!(
            tick(&log, 1, Vec::new()).is_err(),
            "no genesis program in the log → the daemon errors"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_tick_reads_the_genesis_and_drives_an_interpret_turn() {
        // The full daemon step: inject a genesis, then tick — the daemon reads the LATEST program from the log
        // (no hardcoded genesis) and drives an interpret turn on the event, returning its op-count.
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();

        // Resolve the runtime the genesis program requires (skip if the store is absent/stale).
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!(
                "[cdz-kernel::daemon] value-heap runtime absent/stale; skipping the tick run"
            );
            return;
        };

        let t = tick(&log, 1, runtime.clone()).expect("the daemon ticks over the genesis");
        assert_eq!(
            t.op_count, 2,
            "kind=1 → the genesis interpret schedules [Append, Exec] → 2"
        );
        let t0 = tick(&log, 9, runtime).expect("the daemon ticks for a non-message event");
        assert_eq!(t0.op_count, 1, "kind=9 → [Noop] → 1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_later_genesis_supersedes_the_first_the_daemon_runs_the_latest() {
        // Self-modification through the daemon: a later `program` event supersedes the first, and the daemon
        // runs the LATEST — proving behavior evolves without a kernel change. (Uses op-count as the observable:
        // the updated genesis schedules a DIFFERENT plan for the same event.)
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();
        // An updated genesis: kind==1 now → a 1-op plan (Noop), so the same event schedules fewer ops.
        let updated = "(do \
            (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
            (def (interpret (: kind Int64) (: turn Int64)) (list (Noop 0))) \
            (export interpret))";
        crate::boot::inject_genesis(&mut log, updated).unwrap();

        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(updated) {
            Ok(p) => p,
            Err(e) => panic!("updated genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping");
            return;
        };
        let t = tick(&log, 1, runtime).expect("the daemon runs the latest genesis");
        assert_eq!(
            t.op_count, 1,
            "the daemon ran the UPDATED genesis (kind=1 → [Noop] → 1), not the original (would be 2)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_performing_executes_each_op_not_just_counts() {
        // The daemon's real execution step: tick_performing drives the PERFORMING executor, so each op the
        // genesis interpret schedules fires a real Prim.run perform whose per-op RESULT is summed (Append→1,
        // Exec→2 — the real exec/http/log shape, each call returns its own value). kind=1 → [Append, Exec] →
        // 1+2 = 3 (proves both ops PERFORMED and their results came back through the cross-fn fold — NOT a
        // count, which is `tick`'s job). Mirrors the kernel's `the_performing_executor_fires_a_real_effect…`.
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_performing");
            return;
        };
        let performed =
            tick_performing(&log, 1, runtime).expect("the daemon executes the genesis's ops");
        assert_eq!(
            performed, 3,
            "kind=1 → [Append(1), Exec(2)] → each op fired a real perform + its result summed → 3"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_performing_without_a_genesis_is_a_loud_error() {
        let (path, log) = temp_log();
        assert!(
            tick_performing(&log, 1, Vec::new()).is_err(),
            "no genesis → tick_performing errors (like tick)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_performs_via_a_real_primitive_and_records_each_op_in_the_log() {
        // The K1c→host-primitive rung: tick_hosted drives the HOSTED executor (Prim bound to a REAL host
        // closure), so each op is PERFORMED by the daemon's record-only primitive AND appended to the log as a
        // `prim-<op>` event (the recorded-effect trail, like fold's model-request/response). kind=1 → [Append
        // "ack", Exec "handle"] → append("ack")→1 + exec("handle")→2 → sum 3, and the log gains `prim-append`
        // (payload "ack") + `prim-exec` (payload "handle") IN ORDER. This is the daemon actually executing via
        // a real host primitive (record-only cut; live exec/http gate behind Cedar), not the in-program mock.
        use std::sync::{Arc, Mutex};
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_hosted");
            let _ = std::fs::remove_file(&path);
            return;
        };
        let log = Arc::new(Mutex::new(log0));
        let performed = tick_hosted(Arc::clone(&log), 1, runtime)
            .expect("the daemon performs the genesis's ops via a real host primitive");
        assert_eq!(
            performed, 3,
            "kind=1 → append(\"ack\")→1 + exec(\"handle\")→2 → summed by the fold = 3"
        );
        // The recorded-effect trail: the log gained a `prim-append`/`prim-exec` event per performed op, in
        // list order, each carrying the op's String payload.
        let events = log.lock().unwrap().tail(0).unwrap();
        let prim_events: Vec<(String, String)> = events
            .iter()
            .filter(|e| e.kind.starts_with(PRIM_RECORD_PREFIX))
            .map(|e| {
                (
                    e.kind.clone(),
                    String::from_utf8_lossy(&e.payload).into_owned(),
                )
            })
            .collect();
        assert_eq!(
            prim_events,
            vec![
                ("prim-append".to_string(), "ack".to_string()),
                ("prim-exec".to_string(), "handle".to_string()),
            ],
            "each performed op was recorded in the log as prim-<op> with its payload, in order"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_without_a_genesis_is_a_loud_error() {
        use std::sync::{Arc, Mutex};
        let (path, log) = temp_log();
        assert!(
            tick_hosted(Arc::new(Mutex::new(log)), 1, Vec::new()).is_err(),
            "no genesis → tick_hosted errors (like tick/tick_performing)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn perform_op_body_is_record_only_by_default_for_exec() {
        // The safe DEFAULT (no `live-exec` feature): exec does NOT spawn a process — it returns the record-only
        // tag (2), identical to the mock. Defense-in-depth: a default build simply cannot run a subprocess.
        // (This assertion holds in the default build; under --features live-exec the sibling test runs instead.)
        #[cfg(not(feature = "live-exec"))]
        {
            assert_eq!(
                perform_op_body("exec", "exit 7"),
                2,
                "default build: exec is record-only (tag 2), no subprocess spawned"
            );
        }
        // http/append/other are record-only tags in BOTH builds.
        assert_eq!(perform_op_body("http", "x"), 3, "http record-only tag");
        assert_eq!(perform_op_body("append", "x"), 1, "append tag");
        assert_eq!(perform_op_body("noop", "x"), 0, "unknown op → 0");
    }

    #[cfg(feature = "live-exec")]
    #[test]
    fn perform_op_body_runs_a_real_subprocess_under_live_exec() {
        // With the `live-exec` feature: exec SPAWNS a real subprocess and returns its EXIT CODE. `exit 7` → 7;
        // `true` → 0. Proves the live path is wired (only compiled + run when the feature is on).
        assert_eq!(
            perform_op_body("exec", "exit 7"),
            7,
            "live exec returns the subprocess exit code"
        );
        assert_eq!(
            perform_op_body("exec", "true"),
            0,
            "a succeeding command exits 0"
        );
    }

    #[test]
    fn cedar_escape_neutralizes_quote_backslash_and_control_chars() {
        // The Copilot PR#556 fix: a payload with `"`/`\`/control chars must be escaped so it can't close the
        // Cedar EntityUid string literal early (matching an UNINTENDED rule) or fail to parse (spurious deny).
        assert_eq!(
            cedar_escape("plain"),
            "plain",
            "an ordinary payload is unchanged"
        );
        assert_eq!(
            cedar_escape(r#"foo"bar"#),
            r#"foo\"bar"#,
            "a double-quote is escaped so it can't close the literal"
        );
        assert_eq!(
            cedar_escape(r"a\b"),
            r"a\\b",
            "a backslash is doubled (else it could escape the closing quote)"
        );
        assert_eq!(
            cedar_escape("a\nb\tc"),
            "a\\nb\\tc",
            "control chars are escaped"
        );
        // The attack shape: a payload trying to break out of Resource::"..." and inject policy structure.
        let hostile = r#"x" == Resource::"y"#;
        let escaped = cedar_escape(hostile);
        assert!(
            !escaped.contains('"') || escaped.contains(r#"\""#),
            "every bare quote in a hostile payload is escaped: {escaped}"
        );
        // Embedding the escaped payload yields a well-formed, single Cedar string literal (still parses as ONE
        // UID via the authorizer — a policy keying on the exact payload matches; no injection).
        let policies = format!(
            r#"permit(principal, action, resource == Resource::"{}");"#,
            cedar_escape(hostile)
        );
        // Same request → the policy that keys on the escaped payload ALLOWS; a mismatched payload would deny.
        let resource = format!("Resource::\"{}\"", cedar_escape(hostile));
        assert_eq!(
            cdz_agent::cedar::authorize(&policies, AGENT_PRINCIPAL, "Action::\"prim:exec\"", &resource, "{}")
                .expect("the escaped policy + resource parse as well-formed Cedar"),
            cdz_agent::cedar::AuthzDecision::Allow,
            "a policy keying on the escaped hostile payload matches it exactly (no parse break, no injection)"
        );
    }

    #[test]
    fn tick_hosted_authorized_gates_each_op_on_the_external_policy() {
        // The capability boundary (concierge ruling 5828, external-anchor half): tick_hosted_authorized
        // authorizes each op against an EXTERNAL root policy BEFORE performing. Policy permits ONLY prim:append
        // (not prim:exec). kind=1 → [Append "ack", Exec "handle"]: append ALLOWED → performed (1) + prim-append
        // recorded; exec DENIED → NOT performed (0) + prim-denied-exec recorded. So sum = 1 (not 3), proving the
        // gate sits on the perform path per-op. Deny-by-default: an op with no matching permit is denied.
        use std::sync::{Arc, Mutex};
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_hosted_authorized");
            let _ = std::fs::remove_file(&path);
            return;
        };
        // External root policy: permit ONLY prim:append; everything else (prim:exec) is deny-by-default.
        let root = r#"permit(principal, action == Action::"prim:append", resource);"#.to_string();
        let log = Arc::new(Mutex::new(log0));
        let sum = tick_hosted_authorized(Arc::clone(&log), 1, runtime, root)
            .expect("the authorized hosted tick runs");
        assert_eq!(
            sum, 1,
            "append allowed (1) + exec denied (0) → sum 1, not 3 — the policy gates per op"
        );
        let events = log.lock().unwrap().tail(0).unwrap();
        let kinds: Vec<String> = events
            .iter()
            .filter(|e| {
                e.kind.starts_with(PRIM_RECORD_PREFIX) || e.kind.starts_with(PRIM_DENIED_PREFIX)
            })
            .map(|e| e.kind.clone())
            .collect();
        assert!(
            kinds.contains(&"prim-append".to_string()),
            "the allowed append was performed + recorded; got {kinds:?}"
        );
        assert!(
            kinds.contains(&"prim-denied-exec".to_string()),
            "the denied exec was recorded as prim-denied-exec, not performed; got {kinds:?}"
        );
        assert!(
            !kinds.contains(&"prim-exec".to_string()),
            "the denied exec must NOT produce a prim-exec (performed) event; got {kinds:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_authorized_denies_all_under_an_empty_policy() {
        // Deny-by-default end to end: an empty policy set permits nothing → every op denied → sum 0, no effect
        // performed, AND each denied op AUTO-EMITS an authz-request (the can't-brick guarantee). kind=1 →
        // [Append, Exec] → 2 denied → 2 prim-denied + 2 authz-request events. Proves the gate fails CLOSED and
        // leaves the operator standing requests to grant.
        use std::sync::{Arc, Mutex};
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping empty-policy authz");
            let _ = std::fs::remove_file(&path);
            return;
        };
        let log = Arc::new(Mutex::new(log0));
        let sum = tick_hosted_authorized(Arc::clone(&log), 1, runtime, String::new())
            .expect("the authorized hosted tick runs under an empty policy");
        assert_eq!(
            sum, 0,
            "empty policy → every op denied → sum 0 (deny-by-default)"
        );
        let events = log.lock().unwrap().tail(0).unwrap();
        let performed = events
            .iter()
            .filter(|e| {
                e.kind.starts_with(PRIM_RECORD_PREFIX) && !e.kind.starts_with(PRIM_DENIED_PREFIX)
            })
            .count();
        assert_eq!(performed, 0, "no op performed under an empty policy");
        // Each denied op auto-emitted an authz-request (the can't-brick escape hatch).
        let requests = events
            .iter()
            .filter(|e| e.kind == crate::policy::AUTHZ_REQUEST)
            .count();
        assert_eq!(
            requests, 2,
            "each of the 2 denied ops auto-emits an authz-request → 2 standing asks for the operator"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_log_policy_reads_the_policy_from_the_log() {
        // The operator's retrieve-at-invocation model: tick_hosted_log_policy reads the LATEST `policy` Cedar
        // doc FROM THE LOG and gates on it — no external --policies file. A policy permitting ONLY prim:append
        // → kind=1 [Append, Exec] → append performed(1) + exec DENIED(0) → sum 1. With NO policy event in the
        // log, DENY-BY-DEFAULT (operator ruling: actions never go through without a policy) → sum 0. So an
        // APPENDED policy governs the invocation, and no-policy means zero privileges.
        use std::sync::{Arc, Mutex};
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_hosted_log_policy");
            return;
        };

        // (a) NO policy event → DENY-BY-DEFAULT → sum 0 (zero privileges without a granting policy).
        let (path_a, mut log_a) = temp_log();
        crate::boot::inject_genesis(&mut log_a, GENESIS).unwrap();
        let log_a = Arc::new(Mutex::new(log_a));
        let sum_denied = tick_hosted_log_policy(Arc::clone(&log_a), 1, runtime.clone())
            .expect("no-policy deny-by-default runs");
        assert_eq!(
            sum_denied, 0,
            "no policy in the log → deny-by-default → every op denied → sum 0 (actions never go through without a policy)"
        );
        let _ = std::fs::remove_file(&path_a);

        // (b) a policy event permitting ONLY prim:append is APPENDED → gated → sum 1 (append 1 + exec denied 0).
        let (path_b, mut log_b) = temp_log();
        crate::boot::inject_genesis(&mut log_b, GENESIS).unwrap();
        crate::policy::append_policy(
            &mut log_b,
            r#"permit(principal, action == Action::"prim:append", resource);"#,
        )
        .unwrap();
        let log_b = Arc::new(Mutex::new(log_b));
        let sum_gated = tick_hosted_log_policy(Arc::clone(&log_b), 1, runtime)
            .expect("log-policy gated tick runs");
        assert_eq!(
            sum_gated, 1,
            "policy from the log permits only append → append(1) + exec denied(0) → sum 1"
        );
        let denied = log_b
            .lock()
            .unwrap()
            .tail(0)
            .unwrap()
            .iter()
            .filter(|e| e.kind.starts_with(PRIM_DENIED_PREFIX))
            .count();
        assert_eq!(
            denied, 1,
            "exec was denied by the log policy → one prim-denied event"
        );
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn tick_hosted_log_policy_honors_an_operator_grant() {
        // The authz lifecycle THROUGH the daemon: a base policy permitting only prim:append DENIES exec — until
        // an operator GRANT adds prim:exec. Then kind=1 [Append, Exec] → BOTH performed → sum 3. Proves the
        // daemon evaluates effective_policy (base ∪ active grants) per invocation, so an operator's grant widens
        // an agent's capabilities live (the can't-brick escape hatch's grant half). No expiry (never expires).
        use std::sync::{Arc, Mutex};
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping grant test");
            return;
        };
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        crate::policy::append_policy(
            &mut log0,
            r#"permit(principal, action == Action::"prim:append", resource);"#,
        )
        .unwrap();
        // Operator grants prim:exec (no expiry) — widens the base policy.
        crate::policy::append_grant(
            &mut log0,
            r#"permit(principal, action == Action::"prim:exec", resource);"#,
            None,
        )
        .unwrap();
        let log = Arc::new(Mutex::new(log0));
        let sum = tick_hosted_log_policy(Arc::clone(&log), 1, runtime)
            .expect("the daemon evaluates base policy ∪ grant");
        assert_eq!(
            sum, 3,
            "base permits append(1); the operator grant permits exec(2) → both performed → sum 3"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn drain_new_events_steps_each_new_event_in_order_and_advances_the_cursor() {
        // The event-source loop's pure core: drain_new_events drives `step` over every event since the cursor,
        // in seq order, and returns the cursor to resume from. Two events → both stepped in order, cursor
        // advances past both; a second drain from that cursor sees NOTHING new (cursor unchanged) until a new
        // event lands, which the next drain picks up. No sleep/thread — the loop DRIVER (poll+sleep) rides this.
        let (path, mut log) = temp_log();
        log.append("evt", b"a").unwrap(); // seq 0
        log.append("evt", b"b").unwrap(); // seq 1

        let seen = std::cell::RefCell::new(Vec::<(u64, String)>::new());
        let cursor = drain_new_events(&log, 0, |e| {
            seen.borrow_mut()
                .push((e.seq, String::from_utf8_lossy(&e.payload).into_owned()));
            Ok(())
        })
        .expect("drain the two events");
        assert_eq!(
            *seen.borrow(),
            vec![(0, "a".to_string()), (1, "b".to_string())],
            "both events stepped, in seq order"
        );
        assert_eq!(cursor, 2, "cursor advances to last_seq + 1 = 2");

        // A second drain from the cursor: nothing new → step is never called, cursor unchanged.
        let cursor2 = drain_new_events(&log, cursor, |_| {
            panic!("no new events → step must not be called");
        })
        .expect("drain with nothing new");
        assert_eq!(cursor2, 2, "no new events → cursor stays at 2");

        // A new event lands → the next drain picks up ONLY it.
        log.append("evt", b"c").unwrap(); // seq 2
        let seen2 = std::cell::RefCell::new(Vec::<u64>::new());
        let cursor3 = drain_new_events(&log, cursor2, |e| {
            seen2.borrow_mut().push(e.seq);
            Ok(())
        })
        .expect("drain the new event");
        assert_eq!(
            *seen2.borrow(),
            vec![2],
            "only the new event (seq 2) is stepped"
        );
        assert_eq!(cursor3, 3, "cursor advances to 3");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn drain_new_events_propagates_a_step_error_as_a_loud_stop() {
        // A failing step aborts the drain loudly (not a silent skip) — the caller decides recovery.
        let (path, mut log) = temp_log();
        log.append("evt", b"x").unwrap();
        let r = drain_new_events(&log, 0, |_| Err(anyhow!("boom")));
        assert!(
            r.is_err(),
            "a step error propagates out of drain_new_events"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fire_due_schedules_appends_triggers_and_fire_records_and_advances_occurrences() {
        // The scheduling read-side: a due schedule appends its TRIGGER event + a schedule-fire record; a
        // not-yet-due schedule is untouched. first_ms=0 is always due (wall clock > 0); a far-future first_ms
        // is never due. A reserved trigger_kind is refused (can't forge kernel bookkeeping). Uses the REAL
        // clock (fire_due_schedules reads now_millis) — the schedule module's own tests cover the pure `due`
        // fold at fixed times; this asserts the daemon side (append trigger + fire, advance the counter).
        use crate::schedule::{self, Schedule};
        use std::sync::{Arc, Mutex};
        let (path, log0) = temp_log();
        let log = Arc::new(Mutex::new(log0));

        let due_now = Schedule {
            id: "hb".to_string(),
            first_ms: 0, // epoch → always due vs the real clock
            period_ms: Some(1),
            trigger_kind: "heartbeat".to_string(),
            payload: b"tick".to_vec(),
        };
        let future = Schedule {
            id: "later".to_string(),
            first_ms: u64::MAX, // never due
            period_ms: None,
            trigger_kind: "wakeup".to_string(),
            payload: b"x".to_vec(),
        };
        let forger = Schedule {
            id: "evil".to_string(),
            first_ms: 0,
            period_ms: None,
            trigger_kind: crate::boot::PROGRAM.to_string(), // reserved → must be refused
            payload: b"(do)".to_vec(),
        };
        {
            let mut l = log.lock().unwrap();
            schedule::append_create(&mut *l, &due_now).unwrap();
            schedule::append_create(&mut *l, &future).unwrap();
            schedule::append_create(&mut *l, &forger).unwrap();
        }

        // First fire: only the due, non-forging schedule fires.
        let fired = fire_due_schedules(&log).expect("fire due schedules");
        assert_eq!(fired, 1, "only the due, non-reserved schedule fires (future + forger skipped)");
        {
            let events = log.lock().unwrap().tail(0).unwrap();
            assert_eq!(
                events.iter().filter(|e| e.kind == "heartbeat").count(),
                1,
                "the trigger event was appended once"
            );
            assert!(
                events.iter().any(|e| e.kind == "heartbeat" && e.payload == b"tick"),
                "the trigger carries the schedule's payload"
            );
            assert_eq!(
                events.iter().filter(|e| e.kind == schedule::SCHEDULE_FIRE).count(),
                1,
                "one schedule-fire record advances the occurrence counter"
            );
            assert!(
                !events.iter().any(|e| e.kind == crate::boot::PROGRAM
                    && e.payload == b"(do)"),
                "the forging schedule (reserved trigger_kind) never fired"
            );
        }

        // Second fire: the periodic advances by one period (period=1ms) — with the real clock well past 1ms,
        // it is due again, so exactly one more heartbeat fires (one occurrence per call).
        let fired2 = fire_due_schedules(&log).expect("fire again");
        assert_eq!(fired2, 1, "the periodic fires once more (occurrence advanced by the prior fire record)");
        {
            let events = log.lock().unwrap().tail(0).unwrap();
            assert_eq!(
                events.iter().filter(|e| e.kind == "heartbeat").count(),
                2,
                "two heartbeats total after two fire rounds (one per round)"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_once_performs_trigger_events_skips_own_events_and_advances_the_cursor() {
        // The live daemon's testable loop BODY: run_once drains every event since the cursor, PERFORMS each
        // TRIGGER via tick_hosted (real host primitive + recorded trail), and SKIPS non-triggers. The skip is
        // load-bearing: the daemon records its own `prim-*` effects + the genesis is a `program` event, so
        // kind_of returns Some only for `trigger` events. Inject genesis, append 2 triggers (kind 1 → [Append,
        // Exec] → 2 prim records each). run_once from 0 → 2 processed (genesis skipped), cursor past all, 4
        // prim events. A SECOND round drains the 4 recorded prim events but SKIPS them (not triggers) →
        // processed 0, converges. No sleep/thread — `run` wraps this in a poll loop.
        use std::sync::{Arc, Mutex};
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap(); // seq 0 (a `program` event — skipped)
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping run_once");
            let _ = std::fs::remove_file(&path);
            return;
        };
        log0.append("trigger", b"one").unwrap(); // seq 1
        log0.append("trigger", b"two").unwrap(); // seq 2
        let log = Arc::new(Mutex::new(log0));

        // Only `trigger` events fire the interpret plan (kind 1); the genesis `program` + recorded `prim-*`
        // events are skipped — the daemon must not re-process its own bookkeeping.
        let kind_of = |e: &crate::Event| (e.kind == "trigger").then_some(1i64);

        let (cursor, processed) = run_once(&log, 0, runtime.clone(), None, kind_of)
            .expect("run_once performs the two trigger events");
        assert_eq!(
            processed, 2,
            "2 triggers performed; the genesis `program` event skipped"
        );
        assert!(
            cursor >= 3,
            "cursor advanced past the two triggers (and any prim events appended)"
        );
        let prim_count = log
            .lock()
            .unwrap()
            .tail(0)
            .unwrap()
            .iter()
            .filter(|e| e.kind.starts_with(PRIM_RECORD_PREFIX))
            .count();
        assert_eq!(
            prim_count, 4,
            "2 trigger events × [Append, Exec] = 4 prim events recorded"
        );

        // A second round from the cursor: it drains the recorded prim events but SKIPS them (not triggers) →
        // nothing performed. This is why kind_of must return None for non-triggers: else the daemon loops on
        // its own effects forever.
        let (_cursor2, processed2) =
            run_once(&log, cursor, runtime, None, kind_of).expect("run_once with only own events");
        assert_eq!(
            processed2, 0,
            "the daemon's own prim events are skipped, not re-performed → converges"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_once_with_no_external_policy_reads_the_policy_from_the_log() {
        // The full operator model at the loop level: run_once(policies=None) reads the policy FROM THE LOG per
        // invocation (via tick_hosted_log_policy). With a `policy` event permitting ONLY prim:append in the log,
        // a trigger performs append (1) but exec is DENIED — recording a prim-denied-exec event. Proves a live
        // daemon (no --policies file) enforces whatever policy is appended to the log: emit-policy → start.
        use std::sync::{Arc, Mutex};
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping run_once log-policy");
            return;
        };
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        crate::policy::append_policy(
            &mut log0,
            r#"permit(principal, action == Action::"prim:append", resource);"#,
        )
        .unwrap();
        log0.append("trigger", b"go").unwrap();
        let log = Arc::new(Mutex::new(log0));
        // Only `trigger` fires (kind 1); skip genesis/policy/prim-* bookkeeping.
        let kind_of = |e: &crate::Event| (e.kind == "trigger").then_some(1i64);
        // No external --policies → the log policy governs.
        let (_cursor, processed) = run_once(&log, 0, runtime, None, kind_of)
            .expect("run_once with the log policy in force");
        assert_eq!(
            processed, 1,
            "the one trigger was processed under the log policy"
        );
        let denied = log
            .lock()
            .unwrap()
            .tail(0)
            .unwrap()
            .iter()
            .filter(|e| e.kind.starts_with(PRIM_DENIED_PREFIX))
            .count();
        assert_eq!(
            denied, 1,
            "the log policy permits only prim:append → exec denied → one prim-denied event (no --policies file)"
        );
        let _ = std::fs::remove_file(&path);
    }
}
