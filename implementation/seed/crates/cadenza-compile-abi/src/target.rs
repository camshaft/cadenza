//! The output target a compile is requested for — the `Target` enum the query program names via
//! `Compiler.Target`. A plain boundary type; the backend `emit` seam that dispatches on it (and reads a
//! live `Db`/`Layout`) stays in `rcdzc`.

/// The output target a compile is requested for — the value the query program names via
/// `Compiler.Target`. `Ord` so it can key a stable per-target artifact map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Target {
    /// A WebAssembly component (the Stage-0 backend).
    Wasm,
    /// A WebAssembly component carrying EMBEDDED debug information (Mode E of
    /// `DESIGN-debug-info-rcdzc.md`) — the same component as [`Target::Wasm`] with the wasm `name`
    /// custom section (and, later increments, `.debug_*` DWARF) appended to its embedded core module.
    /// The debug sections are inert (they move no executed byte) and strippable, so a `WasmDebug`
    /// artifact stripped of custom sections is byte-identical to the `Wasm` artifact — the
    /// reproducibility anchor (§5). Its artifact kind stays `"component"`: it is a decorated component,
    /// not a new output kind (that is Mode S, [`Target::Dwarf`], a separate `"dwarf"` artifact).
    ///
    //= spec/capabilities/debug-information.md#debug-information-may-be-embedded-or-emitted-as-a-sidecar
    //# Debug information MAY be emitted embedded in the artifact it describes, so that the debug information travels with the runnable artifact as a single self-describing file.
    WasmDebug,
    /// A standalone DWARF SIDECAR module (Mode S of `DESIGN-debug-info-rcdzc.md` §9.2) — a
    /// `kind == "dwarf"` artifact SEPARATE from the runnable component: a bare core wasm module carrying
    /// only the `.debug_*` custom sections, which a debugger loads alongside the (lean, undecorated)
    /// runnable. The second enablement mode ("support both", the operator's third ruling): Mode E embeds
    /// the sections; Mode S detaches them. Its code offsets reference the runnable component's code
    /// section, which is byte-identical whether debug rides embedded or here (the sections are appended
    /// inertly after the code), so the two modes share the offset computation.
    ///
    //= spec/capabilities/debug-information.md#debug-information-may-be-embedded-or-emitted-as-a-sidecar
    //# Debug information MAY instead be emitted as a separate artifact linked to the artifact it describes, so that a deployment can ship the runnable artifact lean and the debug information alongside it.
    Dwarf,
    /// Rust source — a self-contained `.rs` module (one `pub fn` per export) that links into an
    /// existing Rust codebase as ordinary source, with no component boundary and no FFI. The
    /// structured second backend (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its
    /// Target Is Linear).
    Rust,
    /// Rust source in ASYNC, GAS-METERED form — every emitted function is an `async fn` taking a
    /// caller-supplied `env: &mut impl CdzEnv` and awaiting `env.consume(1)` at entry, so the host meters
    /// fuel and can yield COOPERATIVELY at each step (the emitted computation is bounded and pausable
    /// rather than a runaway synchronous call). Same value semantics as [`Target::Rust`]; the difference
    /// is the async/`env`-threaded calling convention, so it composes into an async Rust codebase where
    /// untrusted Cadenza code must be fuel-bounded.
    RustAsync,
    /// Cadenza surface — the optimized program lowered BACK to a Cadenza binary-AST artifact
    /// (`backend::cadenza`). Not a runnable artifact: it re-emits the program itself, AFTER
    /// resolution/inference/const-fold/optimization, as the binary AST (`kind == "ast"`), so it can be
    /// fed back through `compile` for round-trip idempotence, piped into the syntax system for sexpr/ML
    /// inspection of what lowering did, or handed to the Lean oracle. The "inspect the meaning" backend.
    Cadenza,
}

impl Target {
    /// The artifact kind a target produces — how the emitted artifact is tagged (`build-tool-
    /// interface.md`; `backends-and-targets.md` §The Emitted Artifact Is Self-Describing By Kind).
    pub fn artifact_kind(self) -> &'static str {
        match self {
            // A debug-carrying component is still a `component` — a decorated one, not a new kind.
            Target::Wasm | Target::WasmDebug => "component",
            // The detached DWARF sidecar is its own output kind.
            Target::Dwarf => "dwarf",
            // Both Rust forms are `rust`-kinded `.rs` source — they differ in calling convention, not
            // in artifact kind (a consumer picks the flavor by which target it asked to emit).
            Target::Rust | Target::RustAsync => "rust",
            // The re-emitted program is a binary-AST artifact — the same `"ast"` kind an `ast` input
            // artifact carries, since it IS a Cadenza program (round-trippable straight back in).
            Target::Cadenza => "ast",
        }
    }

    /// Whether emitting this target requires the `spans` input artifact (the source side-table debug
    /// info is drawn from — `DESIGN-debug-info-rcdzc.md` §9.4). A debug target requested without spans
    /// DECLINES (`compile`), rather than silently producing an undecorated artifact. Both the embedded
    /// (`WasmDebug`) and the detached (`Dwarf`) debug modes draw from the `spans` side-table.
    pub fn needs_spans(self) -> bool {
        matches!(self, Target::WasmDebug | Target::Dwarf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_kind_tags_each_target() {
        assert_eq!(Target::Wasm.artifact_kind(), "component");
        // A debug-carrying component is still a `component` — a decorated one, not a new kind.
        assert_eq!(Target::WasmDebug.artifact_kind(), "component");
        // The detached DWARF sidecar is its own output kind.
        assert_eq!(Target::Dwarf.artifact_kind(), "dwarf");
        // Both Rust forms are `rust`-kinded source (they differ in calling convention, not kind).
        assert_eq!(Target::Rust.artifact_kind(), "rust");
        assert_eq!(Target::RustAsync.artifact_kind(), "rust");
        // The re-emitted program is a binary-AST `ast` artifact.
        assert_eq!(Target::Cadenza.artifact_kind(), "ast");
    }

    #[test]
    fn only_debug_targets_need_spans() {
        // The two debug modes draw from the source `spans` side-table; nothing else does.
        assert!(Target::WasmDebug.needs_spans());
        assert!(Target::Dwarf.needs_spans());
        assert!(!Target::Wasm.needs_spans());
        assert!(!Target::Rust.needs_spans());
        assert!(!Target::RustAsync.needs_spans());
        assert!(!Target::Cadenza.needs_spans());
    }
}
