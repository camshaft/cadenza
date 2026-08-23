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

3. **The wrapper core body — DONE (W4c-b-i, merged #3039/#3047/#3048 + PR #3065).** The canon lift hands
   the core function the *flattened* boundary values (a `list<u8>` leaf as `(ptr, len)` into linear
   memory; scalars direct); a spilling result is returned as a pointer to the result at its canonical
   offsets. The compiled `on-message` def takes/returns a **guest value-heap record**, so per export the
   core carries a **wrapper** (exported name; the def is internal) that:
   - **param side** (`record_param_rebuild` + `emit_cell_rebuild`): reads each `list<u8>` leaf's
     `(ptr, len)` and builds a guest `Bytes` (`bytes-alloc` + copy-in), reads scalar leaves direct, builds
     the record cell — permuting each WIT field into the guest's name-lex cell slot BY NAME (a
     declaration-ordered WIT record → the right slots). The core DEFINES memory 0 + a real `cabi_realloc`
     bump allocator when a leaf/spill needs memory.
   - calls the compiled def (`op::CALL` `Layout::abs(def)`);
   - **result side** — the recursive canonical WRITER `serialize::CanonWrite` { Scalar, Record, List,
     Bytes, Option, named-Variant } (`mod.rs::canon_write_of`, driven off WIT+`Ty`): allocates a return
     area (`cabi_realloc`) and writes the `step` there — a record permutes fields by name to their WIT
     canonical offsets; a `list<request>` bump-allocates an element array + writes each element at its
     stride; a `list<u8>` leaf copies its bytes out + writes `(ptr, len)`; an `option`/named `variant`
     writes `sum-disc`→the boundary disc + (payload arm) `sum-payload`→the payload at the variant's
     payload offset (variant cases NAME-matched, kebab-normalized; a None-only option's dead Some-arm
     write is derived from the WIT inner type). Returns the return-area pointer.
   Validated **end to end** under wasmtime (not a synthetic core): a full reducer-`step`-shaped guest
   (record permute + `list<record>` + three `list<u8>` leaves + `option` + named variant with a record
   payload) compiles, runs, and lifts back field-for-field (`a_full_step_shaped_guest_compiles_and_runs`).

4. **The cascade wiring — DONE (W4c-b-ii, merged).** `mod.rs::record_interface_export` builds the
   `TypedInterface` (record/variant named types synthesized + exported via `add_wit_type_deduped` so
   nested named types share one index) + the wrapper `WrapperDesc`s, and routes to
   `assemble_typed_interface_with_runtime` (composing the value-heap runtime import). Additive over the
   former record-boundary decline; a scalar interface still takes the no-wrapper `scalar_interface_export`.

5. **The platform host imports (`identity` / `state` / `blobs`) — TODO (W4c-b-iii).** A reducer that calls
   a world import (reducer-echo sets `request.token = identity::id()`) needs that host-import interface
   composed into the reducer-boundary component alongside the runtime. The envelope side is ready:
   `envelope::assemble_host_runtime` already composes a host-effect import + the runtime + boundary
   exports (host ops lowered to core funcs `0..h`, runtime `h..h+k`, boundary aliases after; documented
   index spaces). The reducer path just needs the analogous assembler that composes the host import(s)
   with the **typed interface-instance** export (combine `assemble_host_runtime`'s two-import bookkeeping
   with `assemble_typed_interface_with_runtime`'s instance export), and `record_interface_export` must
   collect the guest's host imports (the `Core::HostCall`s a body performs, recognized via
   `wit_world::is_world_import_op`) and thread them through. **Open (asked v-platform):** the guest-SOURCE
   form for calling a 0-arg `list<u8>`-returning world import (`identity.id : () -> reducer-id`) — whether
   the perform→world-import lowering (the `state.get`/kv path) already covers it, or `identity.id()` needs
   a prelude/reify arm (v-inference/v-syntax territory, currently stopped in the platform-only pause).

6. **World supply surface — TODO (W4c-b-iv).** The in-source `(world …)` decl → binary-AST (`KIND_WIT_WORLD`)
   lowering, coordinated with v-syntax + v-nix (the CLI reads either the decl or a `--world` artifact).

## Validation

Unit + oracle tests cover the type model + envelope with synthetic cores (steps 1–2). Steps 3–4 are
validated **end to end** under wasmtime (`run_reducer_typed`): every boundary shape + the full
reducer-`step`-shaped guest run field-for-field (rcdzc lib suite). Steps 5–6 close the loop with
`v-platform` as the emission oracle: emit a reducer-echo-shaped guest (message param, `token =
identity::id()`, `deadline = none`, `outcome = continue`), send `v-platform` the `(component bytes, input
message, reducer-id)` triple; it seeds the guest into a test CAS, spawns it via `WasmProgramStore`, drives
`on_message`, and diffs the returned `step` against reducer-echo's **echo relation** (requests.len == 1,
contract/payload echoed, token == reducer-id, deadline none, outcome continue) via `step_from_wit`. The
CAS-import composition resolves + composes `cadenza:runtime/heap+<addr>` (and `identity`) from the store
in the same drive — closing the import loop and validating the whole reducer-world guest path (the sunset
milestone: the interim Rust guests + checker retire once a `.cdz` guest drives green through the host).
