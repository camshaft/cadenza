//! Run a checker over a completed observation log (`design/cadenza-platform.md` §9).
//!
//! The operator's checker design is that a checker is a reducer program the harness *executes*: it is
//! delivered the whole observation log and emits a pass/fail verdict. [`run_checker`] is that execution as a
//! single call — the reusable primitive between the low-level [`check_message`](super::check_message) /
//! [`verdict_in`](super::verdict_in) and the full integration-test binary: spawn the checker, deliver it the
//! log, drive to quiescence, and read the verdict it emits. The harness knows nothing of how the checker was
//! authored (a compiled Cadenza reducer, or hand-written); it just runs a reducer.

use super::checker::CheckOutcome;
use super::checker_protocol::{check_message, verdict_in};
use super::harness::{Harness, SpawnSpec};
use super::observation::Record;
use crate::{Bytes, ProgramStore};
use std::sync::Arc;

/// The blob name of the placeholder system reducer the checker run routes any emitted effect to. The verdict
/// is read from the emitted-request observation, not from a system reducer handling it, so a non-component
/// placeholder is enough — a routed verdict is recorded and then declined, never crashing the run.
const CHECKER_SYSTEM: &str = "$checker-system";

/// Run the checker program `checker` over the observation log `log`: seed the checker blob, spawn it, deliver
/// it the whole log as a `check` message, drive the checker to quiescence under bach, and read the
/// [`verdict`](super::verdict_contract) it emits.
///
/// `checker` is `(name, bytes)` — the checker program's blob name and opaque bytes (a compiled reducer
/// component for a real run; the native store keys its factory by the bytes' content hash for a test).
/// `make_store` builds the program store from the seeded content-addressed store, exactly as
/// [`Harness::run`] takes it — a wasm program store for a real run, a store of Rust factories for a test.
///
/// Returns the checker's [`CheckOutcome`]. A checker that emits **no** verdict — e.g. its bytes are not a
/// valid component, so it never runs — is a [`Fail`](CheckOutcome::Fail): a declared check that did not
/// report is a failed check, never silently a pass.
#[must_use]
pub fn run_checker<P, F>(log: &[Record], checker: (&str, Bytes), make_store: F) -> CheckOutcome
where
    P: ProgramStore + 'static,
    F: FnOnce(Arc<dyn crate::BlobStore>) -> P + Send + 'static,
{
    let (name, bytes) = checker;
    let run = Harness::new(CHECKER_SYSTEM)
        .blob(
            CHECKER_SYSTEM,
            Bytes::from_static(b"cdz-platform.checker-run:no-system"),
        )
        .blob(name.to_string(), bytes)
        .spawn(SpawnSpec::new("checker", name))
        .deliver("checker", check_message(log))
        .run(make_store);
    verdict_in(&run.records).unwrap_or_else(|| CheckOutcome::fail("the checker emitted no verdict"))
}

#[cfg(test)]
mod tests {
    use super::run_checker;
    use crate::testing::{
        CheckOutcome, Entry, EventKind, EventOp, Record, check_contract, decode_check,
        deserialize_log, encode_verdict, verdict_contract,
    };
    use crate::{
        Bytes, ContractId, HostId, Message, Notification, Origin, Outcome, ProgramHash, Reducer,
        ReducerId, Request, Response,
    };
    use std::sync::Arc;

    /// A checker reducer that passes iff the delivered log contains a delivered message on a given contract.
    struct SawContractChecker {
        wanted: ContractId,
    }
    #[async_trait::async_trait]
    impl Reducer for SawContractChecker {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let verdict = if m.id == check_contract() {
                match decode_check(&m.payload).and_then(|log| deserialize_log(&log)) {
                    Some(records) => {
                        let saw = records.iter().any(|r| {
                            matches!(&r.entry, Entry::Event(EventOp::Delivered {
                                kind: EventKind::Message, contract, ..
                            }) if *contract == self.wanted)
                        });
                        if saw {
                            encode_verdict(true, &[])
                        } else {
                            encode_verdict(
                                false,
                                &[crate::Str::from("wanted contract not delivered")],
                            )
                        }
                    }
                    None => encode_verdict(false, &[crate::Str::from("log did not decode")]),
                }
            } else {
                encode_verdict(false, &[crate::Str::from("unexpected contract")])
            };
            (
                vec![Request {
                    id: verdict_contract(),
                    payload: verdict,
                    continuation_token: Bytes::new(),
                    deadline: None,
                }],
                Outcome::Break {
                    schema: ContractId::of(b"checked"),
                    reason: Bytes::new(),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A one-record log: a message on `contract` was delivered to some reducer.
    fn log_with_delivered(contract: ContractId) -> Vec<Record> {
        vec![Record {
            seq: 0,
            time_ns: 0,
            source: Origin {
                reducer: ReducerId::of(b"target"),
                host: HostId::of(b"node"),
            },
            entry: Entry::Event(EventOp::Delivered {
                kind: EventKind::Message,
                contract,
                from: None,
                continuation_token: Bytes::new(),
                payload: Bytes::from_static(b"p"),
                error: None,
            }),
        }]
    }

    /// The native store: register the checker factory under the checker blob's content hash.
    fn checker_store(
        checker_bytes: &'static [u8],
        wanted: ContractId,
    ) -> impl FnOnce(Arc<dyn crate::BlobStore>) -> crate::testing::program::Store + Send + 'static
    {
        move |_cas| {
            let mut store = crate::testing::program::Store::new();
            store.register(ProgramHash::of(checker_bytes), move || {
                Box::new(SawContractChecker { wanted })
            });
            store
        }
    }

    #[test]
    fn run_checker_reads_a_passing_verdict() {
        let contract = ContractId::of(b"go");
        let outcome = run_checker(
            &log_with_delivered(contract),
            ("checker", Bytes::from_static(b"checker-bytes")),
            checker_store(b"checker-bytes", contract),
        );
        assert_eq!(outcome, CheckOutcome::Pass);
    }

    #[test]
    fn run_checker_reads_a_failing_verdict_with_reasons() {
        // The log has a delivered message on `go`, but the checker wants `other` → it fails with its reason.
        let outcome = run_checker(
            &log_with_delivered(ContractId::of(b"go")),
            ("checker", Bytes::from_static(b"checker-bytes")),
            checker_store(b"checker-bytes", ContractId::of(b"other")),
        );
        assert_eq!(
            outcome,
            CheckOutcome::Fail {
                reasons: vec!["wanted contract not delivered".to_string()],
            }
        );
    }

    #[test]
    fn a_checker_that_never_runs_is_a_failed_check() {
        // A checker blob that is not a valid native factory registration → never spawns → emits no verdict →
        // a failed check (the store registers nothing, so the checker is never instantiated).
        let outcome = run_checker(
            &log_with_delivered(ContractId::of(b"go")),
            ("checker", Bytes::from_static(b"unregistered")),
            |_cas| crate::testing::program::Store::new(),
        );
        assert!(
            matches!(outcome, CheckOutcome::Fail { .. }),
            "a checker that never runs fails: {outcome:?}"
        );
    }
}
