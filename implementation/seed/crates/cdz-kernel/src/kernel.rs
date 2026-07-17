//! The tiny kernel — compile + run a Cadenza `interpret` program (minimal-kernel re-charter, rung K1).
//!
//! Operator mandate (2026-07-17, confirmed): the daemon is DEPLOY-ONCE and understands NO events. It EMBEDS
//! the compiler (rcdzc) + wasmtime (cdz-run) and, per event, runs a self-modifiable Cadenza `interpret`
//! program that decides the host-ops to execute. This is the codeact-spike shape realized as the kernel:
//! "compile + run any Cadenza program from the log" IS the kernel's job — agents append NEW programs that
//! must run without a redeploy, so the kernel compiles at RUNTIME.
//!
//! **K1 (this module) is the compile+compose+run spine**, built on the PEER path v-effects + v-peer-linking
//! verified end-to-end (a `(List HostOp)` provider result — and a `(List Event)` arg — cross to/from a
//! Cadenza peer executor as runtime handles over the ONE shared value-heap runtime, NO host-ABI widening).
//! So the kernel:
//!   1. COMPILES the interpret `.cdz` source as a PROVIDER component (`cadenza:agent/kernel`) — [`compile_interpret_provider`].
//!   2. COMPOSES it with a peer EXECUTOR (a Cadenza consumer that binds the provider, performs `interpret`,
//!      and reduces the crossed `(List HostOp)` handle) and RUNS via `cdz_run::run_with_peers` — [`run_interpret`].
//! The executor's per-op dispatch to the broad primitives (`exec`/`http`/`log`/…) is rung K1b; K1 proves the
//! compile→provider→peer-executor→consume-the-result loop end-to-end, which is the load-bearing spine.
//!
//! Gated behind runtime-store availability (like `fold`'s tests): a runtime bump can stale the store, so a
//! run SKIPS rather than fails when the value-heap runtime wasm is absent.

use anyhow::{anyhow, Result};

/// The provider interface name the kernel publishes `interpret` under — the peer executor binds this.
pub const KERNEL_IFACE: &str = "cadenza:agent/kernel";

/// Compile a Cadenza `interpret` source string into a PROVIDER component published as [`KERNEL_IFACE`]. This
/// is the kernel's live-compilation step (rcdzc embedded): parse the s-expr source → encode AST → compile
/// with a component-name artifact so the export is a provider the peer executor can bind. Returns the wasm
/// component bytes, or an error carrying the compiler diagnostic (a malformed program is a loud failure — the
/// daemon reports it, it does not silently skip).
pub fn compile_interpret_provider(src: &str) -> Result<Vec<u8>> {
    use rcdzc::abi::Artifact;
    use rcdzc::backend::Target;

    // Parse the s-expr surface → cadenza-syntax arenas → AST BYTES via cadenza-syntax's codec. The bytes are
    // the bridge between the two crates' distinct `Arenas` types (rcdzc's compile takes AST bytes + decodes
    // them with its own byte-compatible codec copy — same path the gate feeds rcdzc).
    let arenas = cadenza_syntax::sexpr::read(src)
        .map_err(|e| anyhow!("interpret source did not parse: {e:?}"))?;
    let ast = cadenza_syntax::codec::encode(&arenas);

    // Compile with the component-name artifact so the output is a PROVIDER (its export is bound by a peer),
    // under the compiler stack (rcdzc::host::run_with_compiler_stack sets up the arena/thread context).
    let out = rcdzc::host::run_with_compiler_stack(|| {
        rcdzc::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "interpret", ast),
                rcdzc::cli::component_name_artifact(KERNEL_IFACE),
            ],
            &[Target::Wasm],
        )
    });
    out.artifact(Target::Wasm.artifact_kind())
        .map(|b| b.to_vec())
        .ok_or_else(|| anyhow!("interpret provider did not compile: {:?}", out.diagnostics))
}

/// The Cadenza EXECUTOR peer source: binds the kernel provider, performs `interpret`, and consumes the
/// crossed `(List HostOp)` — here reduced with `List.len` to a scalar (K1 proves the handle crossed + is a
/// live list; K1b replaces `List.len` with per-op dispatch to the broad primitives). The `HostOp` type is
/// re-declared BY NAME (v-effects wiring finding: an effect-op result type must be a named type, not an inline
/// `(Sum …)`), structurally identical to the provider's so they agree over the shared runtime. `kind` is the
/// scalar event stand-in (the compound `(List Event)` arg is K1b, per the confirmed inbound handle path).
fn executor_src() -> String {
    "(do \
       (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
       (effect K (op interpret (-> Int64 Int64 (List HostOp)))) \
       (bind K \"cadenza:agent/kernel\") \
       (def (main (: kind Int64)) (host (K) (List.len (K.interpret kind 0)))) \
       (export main))"
        .to_string()
}

/// The result of running one interpret turn through the kernel: the number of host-ops the interpret program
/// scheduled for the given event (the executor's `List.len` over the crossed `(List HostOp)` handle). K1b
/// turns this into the executed op-effects; K1 surfaces the count as proof the loop ran end-to-end.
#[derive(Debug, PartialEq, Eq)]
pub struct InterpretRun {
    pub op_count: i64,
}

/// Run one interpret turn: COMPILE `interpret_src` as a provider, COMPOSE it with the executor peer, and RUN
/// via `cdz_run::run_with_peers`, passing the scalar `event_kind`. Returns the number of host-ops interpret
/// scheduled. `runtime` is the value-heap runtime wasm (resolve from the store; `None` if absent → the caller
/// skips, since a runtime bump can stale it). This is the whole K1 spine: compile → provide → peer-execute →
/// consume the crossed list.
pub fn run_interpret(
    interpret_src: &str,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<InterpretRun> {
    let provider = compile_interpret_provider(interpret_src)?;
    let peers = vec![cdz_run::Peer {
        bytes: provider,
        interface: KERNEL_IFACE.to_string(),
    }];
    let executor_arenas = cadenza_syntax::sexpr::read(&executor_src())
        .map_err(|e| anyhow!("executor source did not parse: {e:?}"))?;
    let consumer =
        rcdzc::compile::compile_component(&cadenza_syntax::codec::encode(&executor_arenas))
            .map_err(|d| {
                anyhow!(
                    "executor peer did not compile: {} [{:?}]",
                    d.message,
                    d.code
                )
            })?;
    let opts = cdz_run::RunOpts {
        export: Some("main".to_string()),
        args: vec![event_kind.to_string()],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run_with_peers(&consumer, &peers, &opts)? {
        cdz_run::Outcome::Value(s) => {
            let op_count = s
                .parse::<i64>()
                .map_err(|_| anyhow!("executor returned a non-integer op-count: {s:?}"))?;
            Ok(InterpretRun { op_count })
        }
        cdz_run::Outcome::Trap(t) => Err(anyhow!("interpret run trapped: {t}")),
    }
}

/// Resolve the value-heap runtime wasm the compiled provider requires, from the content-addressed store on
/// some ancestor of `start`. Returns `None` if no ancestor store holds it (a runtime bump can stale it — the
/// caller then skips). Reuses the same walk as [`crate::fold::find_runtime`] but resolves the hash the
/// provider itself declares.
pub fn find_runtime_for(consumer_or_provider: &[u8], start: &std::path::Path) -> Option<Vec<u8>> {
    let hash = cdz_run::required_runtime(consumer_or_provider).ok()??.hash;
    crate::fold::find_runtime(start, &hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal interpret provider: kind==1 → a 2-op plan [Append, Exec]; else → 1-op [Noop]. The list is
    /// branch-built so it ESCAPES as a runtime handle (not const-folded) — the shape v-agent-harness's
    /// interpret.cdz + v-effects' probe use.
    const INTERPRET_SRC: &str = "(do \
        (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
        (def (interpret (: kind Int64) (: turn Int64)) \
          (if (= kind 1) (list (Append \"ack\") (Exec \"handle\")) (list (Noop 0)))) \
        (export interpret))";

    #[test]
    fn the_interpret_provider_compiles_as_a_provider() {
        // The live-compilation step alone: a valid interpret source compiles to a provider component. (No
        // runtime needed — compilation is store-independent; running is what needs the runtime.)
        let provider = compile_interpret_provider(INTERPRET_SRC)
            .expect("a valid interpret source compiles to a provider");
        assert!(!provider.is_empty(), "the provider component has bytes");
        // It imports the value-heap runtime (its (List HostOp) result is a heap handle) — proves it is a real
        // heap program, not a const-folded scalar.
        assert!(
            cdz_run::required_runtime(&provider)
                .ok()
                .flatten()
                .is_some(),
            "the interpret provider imports the value-heap runtime (builds a heap list)"
        );
    }

    #[test]
    fn a_malformed_interpret_source_is_a_loud_compile_error() {
        assert!(
            compile_interpret_provider("(do (def (interpret").is_err(),
            "a malformed interpret source fails loudly, not silently"
        );
    }

    #[test]
    fn the_kernel_runs_interpret_end_to_end_and_counts_the_scheduled_ops() {
        // The full K1 spine: compile interpret as a provider → compose with the executor peer → run → the
        // executor consumes the crossed (List HostOp) handle. kind=1 → [Append, Exec] → op_count 2.
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match compile_interpret_provider(INTERPRET_SRC) {
            Ok(p) => p,
            Err(e) => panic!("interpret compiles: {e}"),
        };
        let Some(runtime) = find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::kernel] value-heap runtime not in any ancestor store (run `cargo xtask build`) or stale; skipping the run");
            return;
        };
        let run = run_interpret(INTERPRET_SRC, 1, runtime.clone())
            .expect("the kernel runs interpret end-to-end");
        assert_eq!(
            run.op_count, 2,
            "kind=1 schedules [Append, Exec] → the executor reads the crossed list's length = 2"
        );
        // A non-message event → the single-Noop plan → op_count 1.
        let run0 = run_interpret(INTERPRET_SRC, 9, runtime)
            .expect("the kernel runs interpret for a non-message event");
        assert_eq!(run0.op_count, 1, "kind=9 → [Noop] → len 1");
    }
}
