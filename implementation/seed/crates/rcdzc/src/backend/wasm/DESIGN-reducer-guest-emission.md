# Emitting a Cadenza reducer as a WIT-typed component

This is the plan for compiling a Cadenza reducer program (`.cdz`) to a WASM **component** that targets
the platform's typed reducer world (`cdz-platform/wit/world.wit`, `world reducer-world`). It is the
"general WIT-bindings emission" capability: the component **exports** a named WIT interface whose
functions carry typed WIT records/variants, and **imports** external interfaces — driven by a `.wit`
world, not hard-coded to one world.

Everyone downstream depends on this one primitive: the platform host instantiates the emitted guest and
drives it (`v-platform`); the integration-test harness compiles inline / named / checker reducer programs
through it (`v-platform-itest`, `v-nix`).

## What the guest looks like

A reducer guest is a Cadenza program that:

- declares the target world in source: `(world reducer-world = | import identity = … | export guest = …)`
  (the `(world …)` surface; the compiler also accepts a preparsed `KIND_WIT_WORLD` binary-AST artifact);
- defines three top-level functions bound **by name** to the exported `guest` interface —
  `on-message`, `on-response`, `on-notification` — each taking the typed envelope record
  (`message` / `response` / `notification`) and returning `step`;
- uses only the subset of the imports (`state` / `blobs` / `identity`) it actually calls (the host wires
  all of them; unused wired imports are harmless). A minimal guest imports just `identity` (or nothing).

Per `design/cadenza-platform.md` §12, the **envelope** records/variants are typed, but each per-contract
`payload` field is opaque `list<u8>` the guest decodes against the contract itself. So the boundary leaf
types are scalars, `list<u8>`, and nested records/variants/lists — never an open value-form blob.

## The compile CLI (for `v-nix` / the harness)

`cdz` compiles the guest `.cdz` to a `reducer-world` component `.wasm`: the world is read from the
in-source `(world …)` decl (or a `--world <path>` binary-AST artifact) plus the component name, and the
`--target wasm` emit produces the component. Reproducible + offline like `cargo-component` (no network;
the runtime dep is imported by content address, resolved from the CAS by the host / nix compose). The
exact subcommand + flags are pinned when the emit lands (see status); a sample guest `.cdz` ships with it.

The emitted `.wasm` imports `cadenza:runtime/heap@0.0.0+<addr>` (building records/`Bytes` uses the value
heap); the address is the base64url of the full tagged `Hash` (§8 canonical text — the tree-wide
lockstep flip from lowercase-hex), which the host / nix compose resolves from the CAS.

## The emit pipeline

1. **Type model + type section — DONE** (`wit_ctype.rs`, merged #2977/#2998, PR #3017): read the world's
   WIT types (`WitType`), lay the component-model defined types + functypes (`add_wit_type` / `emit_cdef`
   / `emit_functype`), and compute the canonical-ABI flattening (`flatten` / `flatten_func_core_sig`) and
   memory layout (`canonical_size` / `canonical_align` / `record_field_offsets`).

2. **The interface-export envelope — DONE** (`envelope::assemble_typed_interface`, merged #3001 + PR
   #3017): emit a component that exports the `guest` interface as a component **instance** (a top-level
   func export of a named type is invalid — named types must live in an exported interface that also
   exports the type). A func whose signature touches linear memory (a `list<u8>`/`string` leaf, or a
   spilling param/result) lifts with the Memory+Realloc canon options and aliases the core's `memory` +
   `cabi_realloc`; a pure-scalar func plain-lifts. Every reducer boundary shape (typed record/variant
   export, `list<u8>`-leaf params, spilling record result) validates + runs under wasmtime with synthetic
   cores.

3. **The wrapper core body — TODO (W4c-b-i, runtime-dependent).** The canon lift hands the core function
   the *flattened* boundary values (a `list<u8>` leaf as a `(ptr, len)` into linear memory; scalars
   direct); a spilling result is returned as a pointer to the result laid at its canonical offsets. But
   the compiled Cadenza `on-message` def takes a **guest value-heap record** and returns one. So per
   export the core module carries a **wrapper** function (its exported name; the compiled def is internal)
   whose body:
   - reads each `list<u8>` leaf's `(ptr, len)` and builds a guest `Bytes` (`bytes-alloc` + copy from
     memory), and reads scalar leaves directly;
   - builds the guest record cell from those fields (`serialize::emit_cell_rebuild`, the same primitive the
     closure-arg / resource-`make` paths use, driven by a `FieldRebuild` descriptor from
     `mod.rs::tuple_field_abi`);
   - calls the compiled def (`op::CALL` `Layout::abs(def)`);
   - lowers the returned `step` record: reads its fields (`arr-get` / `get-int`) and **writes** them to the
     return area at `record_field_offsets` (a `list<u8>` field copies its bytes to memory + writes
     `(ptr, len)`; the `outcome` variant writes its discriminant + payload), then returns the return-area
     pointer.
   The wrapper calls the value-heap runtime ops, so the guest imports `cadenza:runtime/heap` and this is
   validated **end to end** (not with a synthetic core).

4. **The cascade wiring — TODO (W4c-b-ii).** Widen the `mod.rs` boundary-export construction (the point
   that currently declines a record-typed boundary — "a parameter type has no component valtype") so a
   typed-record interface-export world builds a `TypedInterface` (via `tuple_field_abi` for the field
   descriptors) + emits the wrappers into the core module + routes to `assemble_typed_interface`, instead
   of erroring. This is additive over the current decline (those cases are errors today), so it does not
   change scalar / `list<u8>` emit — but it touches the shared component-emit cascade, so it is
   **full-gated** (whole corpus × backends) before landing, not just `dev-gate`. The `TargetWorld` model
   is extended to carry the interface's named types (today it models only funcs).

## Validation

Unit + oracle tests cover the type model, the envelope, and each boundary shape under wasmtime with
synthetic cores (steps 1–2, landed). The full pipeline (steps 3–4) is validated **end to end**: emit a
minimal guest (`export cadenza:platform/guest`, `on-message` returning `outcome = continue` + one
request, `import identity`), and `v-platform` seeds it into a test CAS, spawns it via its
`WasmProgramStore`, drives `on_message`, and diffs the returned `step` against its Rust reference guest.
When the guest imports `cadenza:runtime/heap+<addr>`, the platform's CAS-import composition resolves +
composes the runtime from the store, closing the runtime-import loop in the same drive.
