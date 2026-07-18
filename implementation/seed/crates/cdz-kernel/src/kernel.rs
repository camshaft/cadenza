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

/// Like [`compile_interpret_provider`], but compiles by RUNNING the compiler-wasm (`rcdzc_wasm`) instead of
/// native rcdzc — the operator-#54 wasm-swap at the KERNEL API level. Parses the s-expr source → encodes AST →
/// drives `rcdzc.wasm`'s `compile_named` export via [`crate::wasm_compiler::compile_via_wasm_named`], publishing
/// the provider under [`KERNEL_IFACE`]. Same `(src) -> provider component` contract as the native fn (the
/// differential test proves byte-identical output), so a caller holding the compiler-wasm bytes can swap the
/// kernel's compile path with no other change. `rcdzc_wasm` is the wasm32-wasip1 build the caller supplies (a
/// daemon reads it once; falling back to [`compile_interpret_provider`] when absent is the caller's choice).
/// Feature-gated (`wasm-compiler`) alongside the host glue.
#[cfg(feature = "wasm-compiler")]
pub fn compile_interpret_provider_via_wasm(rcdzc_wasm: &[u8], src: &str) -> Result<Vec<u8>> {
    let arenas = cadenza_syntax::sexpr::read(src)
        .map_err(|e| anyhow!("interpret source did not parse: {e:?}"))?;
    let ast = cadenza_syntax::codec::encode(&arenas);
    crate::wasm_compiler::compile_via_wasm_named(rcdzc_wasm, &ast, KERNEL_IFACE)
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

/// The K1b DISPATCHING executor: instead of `List.len`, it FOLDS the crossed `(List HostOp)` and dispatches
/// EACH op by its variant — the shape real per-op execution takes (each variant → its broad primitive). Here
/// the dispatch sums a per-variant COST (Append→1, Exec→10, Http→100, Noop→0) rather than performing real
/// `exec`/`http` side-effects, so it is deterministic + gate-able with no external I/O; swapping the cost for
/// a real broad-primitive `perform` is the same match, one rung on (K1c). This proves the executor can WALK
/// the crossed handle + pattern-match each element over the shared runtime — the core of op execution.
fn dispatch_executor_src() -> String {
    "(do \
       (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
       (effect K (op interpret (-> Int64 Int64 (List HostOp)))) \
       (bind K \"cadenza:agent/kernel\") \
       (def (op-cost (: op HostOp)) \
         (match op \
           ((Append s) 1) \
           ((Exec s) 10) \
           ((Http s) 100) \
           ((Noop n) 0))) \
       (def (run-ops (: ops (List HostOp))) \
         (match ops \
           ((list head .. rest) (+ (op-cost head) (run-ops rest))) \
           (_ 0))) \
       (def (main (: kind Int64)) (host (K) (run-ops (K.interpret kind 0)))) \
       (export main))"
        .to_string()
}

/// The K1c PERFORMING executor: each op fires a real `Prim.run` PERFORM (the shape a real broad primitive
/// takes — `Prim.run` stands in for exec/http/log; handled in-program here by a mock that echoes the op's tag
/// as the perform's RESULT, like a real primitive returns its own call result). Two structural points
/// v-effects confirmed: (1) bind the crossed `(List HostOp)` to a `let` OUTSIDE the `host (K)` block, then
/// fold+perform outside it (a perform-fold nested INSIDE the peer host block declines — a separate un-reduced
/// increment, reported); (2) SUM the PER-OP RESULTS (each `Prim.run` returns its own value), NOT a threaded
/// handler-state counter — a handler-state accumulator across the cross-fn `run-ops` recursion is a separate
/// QUEUED miscompile (v-effects pinned it; drops the recursion's final out-state). Per-op-result is exactly
/// the real exec/http/log shape (each call returns its own result), and it folds cleanly. So kind=1 →
/// [Append, Exec] → tags [1, 2] → sum 3; the sum PROVES each op fired a real effect + its result came back.
fn performing_executor_src() -> String {
    "(do \
       (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
       (effect K (op interpret (-> Int64 Int64 (List HostOp)))) \
       (bind K \"cadenza:agent/kernel\") \
       (effect Prim (op run (-> Int64 Int64))) \
       (def (op-tag (: op HostOp)) \
         (match op \
           ((Append s) 1) \
           ((Exec s) 2) \
           ((Http s) 3) \
           ((Noop n) 0))) \
       (def (run-ops (: ops (List HostOp))) \
         (match ops \
           ((list head .. rest) (+ (Prim.run (op-tag head)) (run-ops rest))) \
           (_ 0))) \
       (def (main (: kind Int64)) \
         (let ((ops (host (K) (K.interpret kind 0)))) \
           (handle Prim 0 ((run (tag) s (resume tag s))) \
             (run-ops ops)))) \
       (export main))"
        .to_string()
}

/// The interface the kernel publishes the broad host PRIMITIVES under — `Prim.exec`/`Prim.http`/`Prim.append`,
/// each `(String) -> Int64` (the op's String payload crosses as a rope handle; the primitive returns a scalar
/// result). The executor binds this; the HOST answers it with a real Rust closure (exec/http/log), unlike
/// [`KERNEL_IFACE`] which a Cadenza PROVIDER peer answers. This is the real-primitive counterpart of the
/// in-program `Prim` mock in [`performing_executor_src`].
pub const PRIM_IFACE: &str = "cadenza:agent/prim";

/// The K1c-HOSTED executor: like [`performing_executor_src`], but instead of HANDLING `Prim` in-program (the
/// tag-echo mock), it declares `Prim` as a HOST effect (`Prim.exec`/`Prim.http`/`Prim.append`, each
/// `(-> String Int64)`) bound to [`PRIM_IFACE`], and dispatches each `HostOp` variant to the matching op with
/// the op's STRING payload. The host answers each with a REAL closure (exec/http/log) — so the fold sums the
/// primitives' actual results. `interpret` STAYS a separately-compiled PROVIDER peer (bound via `K`); `Prim` is
/// answered at the host. This is why the kernel needs `run_with_peers_hosted` (peer + host bindings together).
/// (Option b of the dispatch fork: one host op per kind, each `String -> Int64`, no new cdz-run HostOp shape.)
fn hosted_executor_src() -> String {
    "(do \
       (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
       (effect K (op interpret (-> Int64 Int64 (List HostOp)))) \
       (bind K \"cadenza:agent/kernel\") \
       (effect Prim \
         (op exec (-> String Int64)) \
         (op http (-> String Int64)) \
         (op append (-> String Int64))) \
       (bind Prim \"cadenza:agent/prim\") \
       (def (run-op (: op HostOp)) \
         (match op \
           ((Append s) (Prim.append s)) \
           ((Exec s) (Prim.exec s)) \
           ((Http s) (Prim.http s)) \
           ((Noop n) 0))) \
       (def (run-ops (: ops (List HostOp))) \
         (match ops \
           ((list head .. rest) (+ (run-op head) (run-ops rest))) \
           (_ 0))) \
       (def (main (: kind Int64)) \
         (let ((ops (host (K) (K.interpret kind 0)))) \
           (host (Prim) (run-ops ops)))) \
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

/// The shared K1 SPINE: compile `interpret_src` as a provider, compose it with the given `executor` peer
/// source, run via `cdz_run::run_with_peers` passing the scalar `event_kind`, and return the executor's scalar
/// result. Every `run_interpret*` variant is this spine + a specific executor (the difference is only what the
/// Cadenza executor DOES with the crossed `(List HostOp)` — count it, cost it, or perform it). `runtime` is the
/// value-heap runtime wasm (resolve via [`find_runtime_for`]; a run SKIPS when it is absent, since a runtime
/// bump can stale the store). `what` labels the executor in error messages.
fn run_with_executor(
    interpret_src: &str,
    executor: &str,
    what: &str,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64> {
    let provider = compile_interpret_provider(interpret_src)?;
    let peers = vec![cdz_run::Peer {
        bytes: provider,
        interface: KERNEL_IFACE.to_string(),
    }];
    let executor_arenas = cadenza_syntax::sexpr::read(executor)
        .map_err(|e| anyhow!("{what} executor source did not parse: {e:?}"))?;
    let consumer =
        rcdzc::compile::compile_component(&cadenza_syntax::codec::encode(&executor_arenas))
            .map_err(|d| {
                anyhow!(
                    "{what} executor peer did not compile: {} [{:?}]",
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
        cdz_run::Outcome::Value(s) => s
            .parse::<i64>()
            .map_err(|_| anyhow!("{what} executor returned a non-integer result: {s:?}")),
        cdz_run::Outcome::Trap(t) => Err(anyhow!("{what} run trapped: {t}")),
    }
}

/// Run one interpret turn (K1): compile interpret as a provider, compose with the counting executor, and
/// return the number of host-ops interpret scheduled (the executor's `List.len` over the crossed list) — the
/// whole compile → provide → peer-execute → consume-the-crossed-list spine, end to end.
pub fn run_interpret(
    interpret_src: &str,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<InterpretRun> {
    let op_count = run_with_executor(interpret_src, &executor_src(), "count", event_kind, runtime)?;
    Ok(InterpretRun { op_count })
}

/// Run one interpret turn with the K1b DISPATCHING executor: it folds the crossed `(List HostOp)` and
/// dispatches each op by variant, returning the summed per-op cost — proof the executor WALKED the crossed
/// list + matched each element's variant over the shared runtime.
pub fn run_interpret_dispatched(
    interpret_src: &str,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64> {
    run_with_executor(
        interpret_src,
        &dispatch_executor_src(),
        "dispatch",
        event_kind,
        runtime,
    )
}

/// Run one interpret turn with the K1c PERFORMING executor: each op fires a real `Prim.run` PERFORM and the
/// fold sums the PER-OP RESULTS (each perform returns its own value — the real exec/http/log shape, NOT a
/// threaded handler-state counter). Returns that sum — proof each op fired a real effect AND its result came
/// back through the cross-fn fold. (v-effects confirmed per-op-result folds today; the handler-state-counter
/// variant is a separate QUEUED miscompile, not needed here. Fetch-plan-outside-host structure per K1c doc.)
pub fn run_interpret_performing(
    interpret_src: &str,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64> {
    run_with_executor(
        interpret_src,
        &performing_executor_src(),
        "perform",
        event_kind,
        runtime,
    )
}

/// Run one interpret turn with the K1c-HOSTED executor: `interpret` stays a PROVIDER peer, but each op's
/// `Prim.exec`/`Prim.http`/`Prim.append` is answered by a REAL host closure `prim` (exec/http/log), composed
/// via [`cdz_run::run_with_peers_hosted`] (peer + host bindings together). `prim` is called with `(op_name,
/// payload)` for each performed op and returns that primitive's scalar result; the fold sums them. This is the
/// real-primitive counterpart of [`run_interpret_performing`] (which mocks `Prim` in-program). Returns the sum.
/// `prim` is the ONLY non-Cadenza surface — bind it to the actual broad primitives (a daemon passes a closure
/// dispatching `op_name` → exec/http/log + recording each as a log event, like `fold::drive_one_turn`).
pub fn run_interpret_hosted<P>(
    interpret_src: &str,
    event_kind: i64,
    runtime: Vec<u8>,
    prim: P,
) -> Result<i64>
where
    P: Fn(&str, String) -> i64 + Send + Sync + Clone + 'static,
{
    let provider = compile_interpret_provider(interpret_src)?;
    let peers = vec![cdz_run::Peer {
        bytes: provider,
        interface: KERNEL_IFACE.to_string(),
    }];
    let executor = hosted_executor_src();
    let executor_arenas = cadenza_syntax::sexpr::read(&executor)
        .map_err(|e| anyhow!("hosted executor source did not parse: {e:?}"))?;
    let consumer =
        rcdzc::compile::compile_component(&cadenza_syntax::codec::encode(&executor_arenas))
            .map_err(|d| {
                anyhow!(
                    "hosted executor peer did not compile: {} [{:?}]",
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
    // Bind each Prim op to the real closure — one HostOpBinding per op (each `String -> Int64`), all
    // dispatching to `prim` tagged with the op name. This is option (b): one host op per kind, no new
    // cdz-run HostOp shape (each is a `StringToScalar`).
    let bindings = ["exec", "http", "append"]
        .into_iter()
        .map(|op| {
            let prim = prim.clone();
            let op_name = op.to_string();
            cdz_run::HostOpBinding {
                iface: PRIM_IFACE.to_string(),
                op: op.to_string(),
                host: cdz_run::HostOp::StringToScalar(Box::new(move |payload| {
                    prim(&op_name, payload)
                })),
            }
        })
        .collect();
    match cdz_run::run_with_peers_hosted(&consumer, &peers, &opts, bindings)? {
        cdz_run::Outcome::Value(s) => s
            .parse::<i64>()
            .map_err(|_| anyhow!("hosted executor returned a non-integer result: {s:?}")),
        cdz_run::Outcome::Trap(t) => Err(anyhow!("hosted run trapped: {t}")),
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

    /// Locate the built `rcdzc.wasm` (wasm32-wasip1 artifact). Returns None if not built (test skips).
    #[cfg(feature = "wasm-compiler")]
    fn find_rcdzc_wasm() -> Option<Vec<u8>> {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("rcdzc-wasm/target/wasm32-wasip1");
        for profile in ["debug", "release"] {
            let p = base.join(profile).join("rcdzc_wasm.wasm");
            if let Ok(bytes) = std::fs::read(&p) {
                return Some(bytes);
            }
        }
        None
    }

    #[cfg(feature = "wasm-compiler")]
    #[test]
    fn provider_via_wasm_matches_native_at_the_kernel_api() {
        // The kernel-API wasm-swap (operator #54): compile_interpret_provider_via_wasm produces a BYTE-IDENTICAL
        // provider to the native compile_interpret_provider for the same source — so a caller can swap the
        // kernel's compile path (native ↔ wasm) with no downstream change. Skips if rcdzc.wasm isn't built or
        // predates the compile_named export (older artifact → missing-export error).
        let Some(rcdzc_wasm) = find_rcdzc_wasm() else {
            eprintln!("[kernel] rcdzc.wasm not built; skipping provider_via_wasm differential");
            return;
        };
        let via_wasm = match compile_interpret_provider_via_wasm(&rcdzc_wasm, INTERPRET_SRC) {
            Ok(bytes) => bytes,
            Err(e) if format!("{e:#}").contains("compile_named` export") => {
                eprintln!("[kernel] rcdzc.wasm predates compile_named; skipping");
                return;
            }
            Err(e) => panic!("compile the interpret provider via wasm: {e:#}"),
        };
        let native = compile_interpret_provider(INTERPRET_SRC).expect("native provider compile");
        assert_eq!(
            via_wasm, native,
            "the kernel-API wasm-swap produces the SAME provider as native (faithful, drop-in)"
        );
    }

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

    #[test]
    fn the_dispatch_executor_walks_the_list_and_matches_each_op_variant() {
        // K1b: the executor FOLDS the crossed (List HostOp) and dispatches EACH op by variant (Append→1,
        // Exec→10, Http→100, Noop→0). kind=1 → [Append, Exec] → 1 + 10 = 11 (proves it walked BOTH elements
        // AND matched each variant, not just counted). kind=9 → [Noop] → 0.
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(runtime) = compile_interpret_provider(INTERPRET_SRC)
            .ok()
            .and_then(|p| find_runtime_for(&p, store_root))
        else {
            eprintln!(
                "[cdz-kernel::kernel] value-heap runtime absent/stale; skipping the dispatch run"
            );
            return;
        };
        let cost = run_interpret_dispatched(INTERPRET_SRC, 1, runtime.clone())
            .expect("the dispatch executor runs end-to-end");
        assert_eq!(
            cost, 11,
            "kind=1 → [Append(1), Exec(10)] → the executor matched each variant + summed = 11"
        );
        let cost0 = run_interpret_dispatched(INTERPRET_SRC, 9, runtime)
            .expect("the dispatch executor runs for a non-message event");
        assert_eq!(cost0, 0, "kind=9 → [Noop(0)] → 0");
    }

    #[test]
    fn the_performing_executor_fires_a_real_effect_per_op_and_sums_results() {
        // K1c: each op fires a real `Prim.run` PERFORM; the fold sums the PER-OP RESULTS (each perform returns
        // its tag — the real exec/http/log per-call-result shape, not a threaded counter). kind=1 → [Append(1),
        // Exec(2)] → 1+2 = 3 (proves both ops performed AND their results came back through the cross-fn fold);
        // kind=9 → [Noop(0)] → 0. A dropped perform or lost result would mis-sum.
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(runtime) = compile_interpret_provider(INTERPRET_SRC)
            .ok()
            .and_then(|p| find_runtime_for(&p, store_root))
        else {
            eprintln!(
                "[cdz-kernel::kernel] value-heap runtime absent/stale; skipping the perform run"
            );
            return;
        };
        let sum = run_interpret_performing(INTERPRET_SRC, 1, runtime.clone())
            .expect("the performing executor runs end-to-end");
        assert_eq!(
            sum, 3,
            "kind=1 → [Append(1), Exec(2)] → each op performed + its result summed = 3"
        );
        let sum0 = run_interpret_performing(INTERPRET_SRC, 9, runtime)
            .expect("the performing executor runs for a non-message event");
        assert_eq!(sum0, 0, "kind=9 → [Noop(0)] → 0");
    }

    #[test]
    fn the_hosted_executor_answers_prim_with_a_real_host_closure_per_op() {
        // K1c-HOSTED (the real-Prim slice): `interpret` stays a PROVIDER peer, but each op's Prim.exec/http/
        // append is answered by a REAL HOST CLOSURE (composed via run_with_peers_hosted — a PEER and a HOST
        // binding TOGETHER, which is the whole point of the new runner). The closure returns a distinct value
        // per op (exec→2, http→3, append→1) AND records the (op, payload) it saw. kind=1 → [Append "ack",
        // Exec "handle"] → the host is called append("ack")→1 + exec("handle")→2 → sum 3, and we assert it
        // received BOTH the right op names AND the right payloads (proves the String payload crossed as a rope
        // to the host, not just a count). This is the peer-AND-host-together gate v-peer-linking required.
        use std::sync::{Arc, Mutex};
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(runtime) = compile_interpret_provider(INTERPRET_SRC)
            .ok()
            .and_then(|p| find_runtime_for(&p, store_root))
        else {
            eprintln!(
                "[cdz-kernel::kernel] value-heap runtime absent/stale; skipping the hosted run"
            );
            return;
        };
        // The real host primitive: records each (op, payload) and returns a per-op scalar result.
        let calls: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let prim = {
            let calls = Arc::clone(&calls);
            move |op: &str, payload: String| -> i64 {
                calls.lock().unwrap().push((op.to_string(), payload));
                match op {
                    "exec" => 2,
                    "http" => 3,
                    "append" => 1,
                    _ => 0,
                }
            }
        };
        let sum = run_interpret_hosted(INTERPRET_SRC, 1, runtime, prim)
            .expect("the hosted executor runs interpret-as-peer + Prim-as-host-closure end-to-end");
        assert_eq!(
            sum, 3,
            "kind=1 → append(\"ack\")→1 + exec(\"handle\")→2, summed by the fold = 3"
        );
        // The host closure saw BOTH ops IN ORDER with their real String payloads (crossed as ropes).
        let seen = calls.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                ("append".to_string(), "ack".to_string()),
                ("exec".to_string(), "handle".to_string()),
            ],
            "the real host primitive was called per op with the op's String payload, in list order"
        );
    }
}
