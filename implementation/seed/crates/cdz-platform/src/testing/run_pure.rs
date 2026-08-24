//! The pure-run drive path (`design/cadenza-platform.md` §3): run a program blob as a pure function via
//! [`Runner`](crate::Runner) and observe its output — for the conformance case that asserts a pure run
//! RETURNS its output while its emitted effects are DENIED. Complements [`Harness`](super::Harness): the
//! harness drives a live reducer set through spawns and deliveries to quiescence, whereas this runs ONE
//! program with no capabilities and captures its single output.

use crate::{
    BlobStore, Bytes, ContractId, InMemoryBlobStore, ProgramHash, ProgramStore, RunError, Runner,
};
use std::sync::{Arc, Mutex};

/// Run the program `bytes` as a pure function of `input` against `contract`, returning its output value
/// (`Ok`) or a [`RunError`]. The program is instantiated with NO capabilities (§3), so every effect it emits
/// is DENIED — dropped, never routed — and the only observable is the fold's output. A returned `Ok(output)`
/// therefore proves BOTH that the program produced its output AND that its emitted effects were denied (they
/// did not block the run or fault it): the §3 pure-run effect-denial invariant, observable end to end.
///
/// Seeds a content-addressed store with `bytes`, builds the program store via `make_store` (a wasm program
/// store for a real guest; a native factory store for a test), constructs a [`Runner`](crate::Runner) over
/// it, and drives under the bach simulator — so the result is deterministic. The program is resolved by the
/// content hash of its bytes, the same name-not-hash resolution [`Harness::run`](super::Harness::run) uses.
pub fn run_pure<P, F>(
    bytes: Bytes,
    contract: ContractId,
    input: Bytes,
    make_store: F,
) -> Result<Bytes, RunError>
where
    P: ProgramStore + 'static,
    F: FnOnce(Arc<dyn BlobStore>) -> P + Send + 'static,
{
    use bach::ext::*;

    let program = ProgramHash::of(&bytes);
    // The run happens inside the sim's async closure; carry its result out through a shared cell the primary
    // task fills (the sim runs the primary to completion before returning, so the cell is set by then).
    let result_cell: Arc<Mutex<Option<Result<Bytes, RunError>>>> = Arc::new(Mutex::new(None));
    let out = result_cell.clone();

    bach::sim(move || {
        let out = out.clone();
        async move {
            // Seed the content-addressed store with the program's bytes, then let the factory build the
            // program store over it (a wasm store fetches the component by hash; a native store ignores the
            // seeded bytes and instantiates by the same hash).
            let mut cas = InMemoryBlobStore::new();
            cas.put(bytes).await;
            let runner = Runner::new(Arc::new(make_store(Arc::new(cas))));
            let result = runner.run(program, contract, input).await;
            *out.lock().expect("run_pure result lock") = Some(result);
        }
        .group("run-pure")
        .primary()
        .spawn();
    });

    result_cell
        .lock()
        .expect("run_pure result lock")
        .take()
        .expect("the primary task set the run_pure result")
}

#[cfg(test)]
mod tests {
    use super::run_pure;
    use crate::{
        Bytes, ContractId, Message, Notification, Outcome, ProgramHash, Reducer, Request, Response,
    };

    /// A pure guest that emits one effect AND breaks with an output — the shape whose emitted effect must be
    /// denied by a pure run (a pure reducer has no capabilities, so the request is dropped, never routed).
    struct EmitAndClose;

    #[async_trait::async_trait]
    impl Reducer for EmitAndClose {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![Request {
                    id: ContractId::of(b"effect"),
                    payload: Bytes::from_static(b"e"),
                    continuation_token: Bytes::new(),
                    deadline: None,
                }],
                Outcome::Break {
                    schema: ContractId::of(b"done"),
                    reason: Bytes::from_static(b"OUTPUT"),
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

    #[test]
    fn run_pure_returns_the_output_and_denies_the_emitted_effect() {
        // The guest emits one request AND breaks with OUTPUT. Because the pure run grants no capabilities, the
        // emitted request is denied (dropped) — so run_pure returns Ok(OUTPUT): the output came back AND the
        // effect was denied, not routed or blocking.
        let bytes = Bytes::from_static(b"pure-guest-bytes");
        let hash = ProgramHash::of(&bytes);
        let out = run_pure(
            bytes,
            ContractId::of(b"c"),
            Bytes::from_static(b"in"),
            move |_cas| {
                let mut store = crate::program::testing::Store::new();
                store.register(hash, || Box::new(EmitAndClose));
                store
            },
        );
        assert_eq!(out, Ok(Bytes::from_static(b"OUTPUT")));
    }
}
