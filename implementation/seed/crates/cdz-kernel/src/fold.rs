//! The single-threaded FOLD OWNER (agent-runtime L1b).
//!
//! The vision's minimal core (DESIGN-agent-runtime-vision.md §2.2/§2.3): one owner tails the ordered log,
//! folds it, and drives the agent loop — and **every non-deterministic touch (the model call) is an
//! effect-request the fold emits, whose result is appended as an immutable event**, so the fold over
//! `(request-event, response-event)` is pure and replayable.
//!
//! L1b drives ONE agent-loop turn end-to-end over a [`Log`]: it runs a compiled Cadenza agent-loop
//! consumer through [`cdz_run::run_agent_hosted`], binding the model effect (`cadenza:model/api`.converse)
//! to a closure that, around each real `converse` call, appends a `model-request` event (the prompt) and a
//! `model-response` event (the completion) to the log. So a live run RECORDS the model interaction into
//! the log; the replay-determinism gate (L1c) will re-fold that log with the recorded responses and get
//! the identical outcome with no live call. The model backend is injected (mock for CI, Bedrock for real),
//! reusing the shipped embedder — the fold owner adds only the log-appending + orchestration.

use crate::Log;
use anyhow::{anyhow, Result};
use cdz_run::{HostOp, HostOpBinding, Outcome, RunOpts};

/// The kind tag for a model-request event (the prompt the fold emitted).
pub const MODEL_REQUEST: &str = "model-request";
/// The kind tag for a model-response event (the completion appended after the live call).
pub const MODEL_RESPONSE: &str = "model-response";

/// Drive ONE agent-loop turn over `log`: run the `consumer` agent-loop component (which performs
/// `cadenza:model/api`.converse), answering the model effect with `converse` — and appending a
/// `model-request` event (the prompt) then a `model-response` event (the completion) to `log` around each
/// call. Returns the loop's [`Outcome`]. This is the live "tail → fold → execute effect-request → append
/// the result event" cycle; `log` afterward holds the recorded model interaction that L1c replays.
///
/// `converse` is the model backend (e.g. `cdz_agent::mock_converse` for tests, `bedrock_converse` for
/// real). `opts` carries the value-heap runtime bytes + the export to invoke, as for the embedder.
///
/// The log is shared with the appending closure via a `Mutex` (the closure runs inside the wasmtime host
/// call and must be `Send + Sync`); the owner is single-threaded, so this never actually contends — the
/// lock is just how the closure reaches the owner's log.
pub fn drive_one_turn<L, F>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    consumer: &[u8],
    opts: &RunOpts,
    converse: F,
) -> Result<Outcome>
where
    L: Log + Send + 'static,
    F: Fn(String) -> String + Send + Sync + 'static,
{
    let recording = {
        let log = std::sync::Arc::clone(&log);
        move |prompt: String| {
            // Emit the effect-REQUEST event (the prompt the fold is asking the model), then perform the
            // live call, then emit the RESPONSE event (the completion) — the §2.3 record. A lock-poison or
            // append error can't be surfaced through the `String`-returning host-op contract, so on such a
            // failure we fall back to just returning the completion (the run still produces an answer; the
            // gap would show as a missing event, not a wrong one). Live runs here are single-threaded +
            // file-backed, so this path is not expected to fail.
            if let Ok(mut l) = log.lock() {
                let _ = l.append(MODEL_REQUEST, prompt.as_bytes());
            }
            let completion = converse(prompt);
            if let Ok(mut l) = log.lock() {
                let _ = l.append(MODEL_RESPONSE, completion.as_bytes());
            }
            completion
        }
    };

    cdz_run::run_agent_hosted(
        consumer,
        opts,
        vec![HostOpBinding {
            iface: cdz_agent::MODEL_IFACE.to_string(),
            op: cdz_agent::MODEL_OP.to_string(),
            host: HostOp::StringToString(Box::new(recording)),
        }],
    )
}

/// The outcome of a REPLAY (L1c): the loop's [`Outcome`] plus how many model calls were answered from the
/// recorded log (`replayed`) vs how many had NO recorded response left (`missing`). A faithful replay of a
/// log recorded by [`drive_one_turn`] has `missing == 0` and makes zero LIVE model calls — that is the
/// recorded-effect determinism proof (vision §2.3).
pub struct Replay {
    pub outcome: Outcome,
    pub replayed: usize,
    pub missing: usize,
}

/// RE-FOLD a turn from a log recorded by [`drive_one_turn`], WITHOUT a live model call (agent-runtime L1c).
///
/// This is the load-bearing recorded-effect-determinism proof (vision §2.3): the model effect is answered
/// not by a live backend but by REPLAYING the `model-response` events the live run appended, in order. So
/// re-folding the same log reproduces the same cognition with the world's non-determinism frozen — the
/// property that makes an agent's whole existence a fold over its log (§3: fork / hand-off / time-travel
/// are all re-fold, needing no continuation capture).
///
/// The recorded responses are the payloads of the `model-response` events in `recorded`, consumed in log
/// order. If the loop performs MORE model calls than were recorded (a divergent fold), the extra calls get
/// an empty completion and `missing` counts them — a loud signal the replay didn't match, never a silent
/// live fallback.
pub fn replay_one_turn(
    recorded: &[crate::Event],
    consumer: &[u8],
    opts: &RunOpts,
) -> Result<Replay> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // The recorded model completions, in log order — the only thing the replayed model effect returns.
    let responses: Vec<String> = recorded
        .iter()
        .filter(|e| e.kind == MODEL_RESPONSE)
        .map(|e| String::from_utf8_lossy(&e.payload).into_owned())
        .collect();

    let cursor = Arc::new(AtomicUsize::new(0));
    let missing = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(responses));

    let replay = {
        let cursor = Arc::clone(&cursor);
        let missing = Arc::clone(&missing);
        let queue = Arc::clone(&queue);
        move |_prompt: String| {
            // Answer purely from the recorded tail — NO live converse. Pop the next recorded response;
            // count a miss (and return "") if the fold asked for more calls than were recorded.
            let i = cursor.fetch_add(1, Ordering::SeqCst);
            let q = queue.lock().unwrap();
            match q.get(i) {
                Some(resp) => resp.clone(),
                None => {
                    missing.fetch_add(1, Ordering::SeqCst);
                    String::new()
                }
            }
        }
    };

    let outcome = cdz_run::run_agent_hosted(
        consumer,
        opts,
        vec![HostOpBinding {
            iface: cdz_agent::MODEL_IFACE.to_string(),
            op: cdz_agent::MODEL_OP.to_string(),
            host: HostOp::StringToString(Box::new(replay)),
        }],
    )?;

    Ok(Replay {
        outcome,
        replayed: cursor.load(Ordering::SeqCst) - missing.load(Ordering::SeqCst),
        missing: missing.load(Ordering::SeqCst),
    })
}

/// Resolve the value-heap runtime a `consumer` requires (by content address) from the content-addressed
/// store on some ancestor of `start` — the same walk the cdz-agent driver uses. Returns the runtime bytes,
/// or None if no ancestor store holds it (a runtime bump can stale a fixture; the caller then skips).
pub fn find_runtime(start: &std::path::Path, hash: &str) -> Option<Vec<u8>> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir
            .join("target/cadenza-store")
            .join(format!("{hash}.wasm"));
        if let Ok(bytes) = std::fs::read(&candidate) {
            return Some(bytes);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Read the runtime requirement a `consumer` declares (its content-addressed value-heap runtime hash), or
/// an error if it imports none (not an agent loop).
pub fn required_runtime_hash(consumer: &[u8]) -> Result<String> {
    Ok(cdz_run::required_runtime(consumer)?
        .ok_or_else(|| anyhow!("consumer imports no value-heap runtime (not an agent loop?)"))?
        .hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // The cdz-agent model-consumer fixture: a compiled Cadenza loop that binds Model:String->String to
    // cadenza:model/api, converses "hi", and returns the completion's byte-len. Reused here so L1b drives a
    // REAL agent-loop component, not a hand-rolled stub. (Kept in sync by cdz-agent's CI fixture regen.)
    const MODEL_CONSUMER: &[u8] =
        include_bytes!("../../cdz-agent/tests/fixtures/model-consumer.wasm");

    fn store_root() -> std::path::PathBuf {
        env!("CARGO_MANIFEST_DIR").into()
    }

    /// Resolve `RunOpts` for the model-consumer fixture (runtime by content address + export "main"), or
    /// None if the store/fixture is absent (a runtime bump can stale the fixture) — the caller then skips.
    fn opts_or_skip() -> Option<RunOpts> {
        let hash = match required_runtime_hash(MODEL_CONSUMER) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[cdz-kernel] fixture has no runtime requirement ({e}); skipping");
                return None;
            }
        };
        let runtime = find_runtime(&store_root(), &hash).or_else(|| {
            eprintln!(
                "[cdz-kernel] runtime {hash} not in any ancestor store or stale fixture; skipping"
            );
            None
        })?;
        Some(RunOpts {
            export: Some("main".to_string()),
            args: Vec::new(),
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        })
    }

    #[test]
    fn drive_one_turn_folds_and_records_the_model_effect_to_the_log() {
        // The L1b cycle end-to-end: driving the consumer performs converse("hi"); the fold owner records a
        // model-request("hi") + model-response("HI") to the log (mock uppercases), and the loop returns the
        // completion's byte-len (2). Proves tail→fold→execute-effect→APPEND against a real Log.
        let hash = match required_runtime_hash(MODEL_CONSUMER) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[cdz-kernel] fixture has no runtime requirement ({e}); skipping");
                return;
            }
        };
        let Some(runtime) = find_runtime(&store_root(), &hash) else {
            eprintln!(
                "[cdz-kernel] runtime {hash} not in any ancestor store (run `cargo xtask build`) or stale \
                 fixture; skipping"
            );
            return;
        };

        let logfile =
            std::env::temp_dir().join(format!("cdz-kernel-fold-{}-{}.log", std::process::id(), 0));
        let _ = std::fs::remove_file(&logfile);
        let log = Arc::new(Mutex::new(crate::FileLog::open(&logfile).unwrap()));

        let opts = RunOpts {
            export: Some("main".to_string()),
            args: Vec::new(),
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };

        let outcome = drive_one_turn(
            Arc::clone(&log),
            MODEL_CONSUMER,
            &opts,
            cdz_agent::mock_converse,
        );

        // The loop returned the mock completion's byte-len: "hi" -> "HI" -> 2.
        match outcome {
            Ok(Outcome::Value(s)) => {
                assert_eq!(s, "2", "the loop folded the model turn to byte-len 2")
            }
            Ok(Outcome::Trap(t)) => panic!("fold-owner turn trapped: {t}"),
            Err(e) => panic!("fold-owner turn errored: {e}"),
        }

        // The model interaction was RECORDED to the log as request+response events (the §2.3 record L1c
        // will replay): exactly one model-request "hi" then one model-response "HI".
        let events = log.lock().unwrap().tail(0).unwrap();
        let _ = std::fs::remove_file(&logfile);
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![MODEL_REQUEST, MODEL_RESPONSE],
            "the fold owner recorded the model request then response, in order"
        );
        assert_eq!(
            events[0].payload, b"hi",
            "the request event carries the prompt"
        );
        assert_eq!(
            events[1].payload, b"HI",
            "the response event carries the completion"
        );
    }

    #[test]
    fn replay_reproduces_the_live_outcome_from_the_recorded_log_with_no_live_call() {
        // THE load-bearing L1c proof (recorded-effect determinism, vision §2.3): a live turn records the
        // model interaction to the log; re-folding that log with the model effect answered by REPLAYING the
        // recorded responses reproduces the identical outcome — and makes ZERO live model calls. This is
        // why an agent's whole existence can be a fold over its log (fork / hand-off / time-travel = re-fold).
        let Some(opts) = opts_or_skip() else { return };

        // 1) LIVE run: records model-request/response events into the log.
        let logfile =
            std::env::temp_dir().join(format!("cdz-kernel-replay-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&logfile);
        let log = Arc::new(Mutex::new(crate::FileLog::open(&logfile).unwrap()));
        let live = drive_one_turn(
            Arc::clone(&log),
            MODEL_CONSUMER,
            &opts,
            cdz_agent::mock_converse,
        );
        let live_value = match live {
            Ok(Outcome::Value(s)) => s,
            other => panic!("live turn did not produce a value: {other:?}"),
        };
        let recorded = log.lock().unwrap().tail(0).unwrap();
        let _ = std::fs::remove_file(&logfile);

        // 2) REPLAY: re-fold from the recorded log, answering the model effect from the recorded responses
        //    only. `replay_one_turn` binds ONLY a recorded-response closure (no live backend is passed in
        //    at all), so a live model call is structurally impossible — the model effect can only be
        //    answered from the log.
        let replay = replay_one_turn(&recorded, MODEL_CONSUMER, &opts)
            .expect("replay re-folds the recorded log");

        // Identical outcome, every model call answered from the record, nothing missing (no divergence),
        // and nothing left over — the fold over (request, response) events was pure.
        match replay.outcome {
            Outcome::Value(s) => assert_eq!(
                s, live_value,
                "replay reproduces the live outcome exactly from the recorded log"
            ),
            Outcome::Trap(t) => panic!("replay trapped: {t}"),
        }
        assert_eq!(
            replay.missing, 0,
            "no model call lacked a recorded response (no divergence)"
        );
        assert_eq!(
            replay.replayed, 1,
            "exactly the one recorded model call was replayed (the turn's single converse)"
        );
    }

    #[test]
    fn replay_reports_missing_when_the_log_lacks_a_recorded_response() {
        // A replay against a log with NO model-response events (a truncated/empty record) must NOT silently
        // fall back to a live call — it answers "" and COUNTS the miss, a loud divergence signal. Proves the
        // replay is honest about an incomplete record rather than papering over it.
        let Some(opts) = opts_or_skip() else { return };
        let empty: Vec<crate::Event> = Vec::new();
        let replay = replay_one_turn(&empty, MODEL_CONSUMER, &opts).expect("replay runs");
        assert_eq!(
            replay.replayed, 0,
            "no recorded responses were available to replay"
        );
        assert_eq!(
            replay.missing, 1,
            "the turn's model call had no recorded response → counted as missing, not a live fallback"
        );
    }

    #[test]
    fn required_runtime_hash_rejects_a_non_loop_component() {
        // A byte string that isn't a valid agent-loop component is a clear ERROR, not a panic or a bogus
        // hash — the owner needs a real runtime requirement to drive the loop. (Garbage bytes fail at the
        // component parse; a well-formed component that imports no runtime fails the "not an agent loop?"
        // check. Both are `Err` — the invariant is that a non-loop input never yields an Ok(hash).)
        assert!(
            required_runtime_hash(b"not a wasm component").is_err(),
            "a non-loop input must be rejected as an error, never a bogus runtime hash"
        );
    }
}
