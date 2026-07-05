# The Value-Heap Runtime Is A Shared Component (2026-07-05)

**Decision.** Cadenza's runtime values (the heap of tuples, records, sums, lists, bytes, strings —
everything beyond a scalar) do not live in each program's own component. The compiler emits a
program against a single, well-known **value-heap runtime**: a shared component the program imports
(`cadenza:runtime/heap`) and the host composes at instantiation. The runtime owns the entire value
heap — allocation, in-memory layout, the acyclic reference-count discipline, and reclamation — and
exposes the **constructors and accessors** that build a compound value from its parts and read a
component out of one by position. A runtime value crosses between program and runtime as an
**opaque handle** the program never dereferences (it inspects a value only through the accessors).

**The runtime does not name or render values (operator refinement).** A record is a *positional
product* at run time and a sum is a *tagged payload* — the runtime holds no field names, no variant
names, no source-level names at all. The association of a position with a name is compile-time
knowledge. So **rendering is type-directed code the compiler emits into the program itself**: the
program walks its result value through the runtime's accessors and assembles the canonical text,
baking every name and keyword (`(tuple `, ` `, `)`, field and variant names) as a static constant.
A compound result therefore crosses the boundary as an **ordinary string the program returns** — the
existing `() -> string` result ABI — and the host reads it back verbatim. This keeps the names with
the only party that has them (the compiler) and keeps the runtime a minimal, name-free value store.

**Why.** The heap/reference-counting/allocation machinery is ordinary, growing code. Authored once in
a real language (Rust, for now) it is far more maintainable and far less error-prone than hand-emitted
wasm bytes per compound type — and this was borne out immediately: writing it as a real module
surfaced two bugs (a heap/data-segment collision and an `i64::MIN` itoa overflow) that inline
byte-emission would have buried. It matches the state of the art (a language runtime is a linked
library, not open-coded into every output; see the RC/persistent-DS research). And because the
runtime **owns the storage representation** behind the opaque handle, that representation can evolve —
precise Perceus reference counting, then an efficient size-classed allocator, then CHAMP/RRB
persistent collections — with **zero change to the emitted program shape and no re-derivation of
programs**. The handle boundary and the accessor interface are stable; everything behind them is
free. (An efficient embedded allocator replacing the interim index-table `Vec` is tracked debt, not
in the first correct version — it changes no emitted byte.)

**What it replaces.** The earlier compound-result convention baked a constant string into a
component-owned `display()` resource, which only worked when the whole value was known at compile
time. A runtime that constructs values at run time is what lets a compound *carrying a runtime
element* (`(def (f n) (tuple n 1)) (f 3)` → `(tuple 3 1)`) be produced at all — the concrete M2
gap this closes.

**The governance edge, and how it was handled.** In Cadenza a component's imports ARE its host
effects: "a program's escaping effect row MUST equal the set of host functions it imports," and
capability-safety — a *never-downgradable* Governance Floor — was auditable simply by counting a
component's imports. A runtime import is not a host effect, so the shipped program keeps one import
that is not a capability. This does not downgrade the guarantee (reaching an undeclared host
operation is still a compile-time rejection no configuration can soften), but it does refine the
floor's *audit rule* from "count the imports" to "every import **other than the one well-known
runtime interface** is a capability the manifest enumerates." The exemption is a **closed allowlist
of exactly one interface**, not an open class of non-effect imports — the runtime module is the only
exception. Because this touches a never-downgradable floor's auditability, it required and received
**explicit human (operator) approval** per the constitution's Amendment Discipline.

**Spec changes (Amendment 0.6.0).**
- `constitution.md` — Amendment 0.6.0; version 0.6.0.
- `spec/contracts/component-abi.md` — **FROZEN, version 2 → 3** (+ migration path): §The Value-Heap
  Runtime (well-known import to construct + inspect, runtime owns storage + representation, opaque
  per-run handle, runtime does not name or render, compound result rendered by compiler-emitted
  type-directed code returning a string).
- `spec/capabilities/capabilities-and-effects.md` — §The Value-Heap Runtime Is The One Import That
  Is Not A Capability (closed allowlist of one; not a host function; not a suspension point; not in
  the manifest).
- `spec/contracts/host-interface-binding.md` — the runtime interface is not counted among host
  imports for the manifest projection.
- `spec/contracts/reproducible-derivation.md` — the runtime is pinned by its **content address**
  (a hash, not a version label): execution is deterministic in the pair (program, runtime content
  address), and a runtime built from different bytes is a distinct, explicitly-identified
  environment rather than a silent substitution. This composes with the durable-continuation model —
  a continuation is canonical data, and the runtime it ran against is just another content-address
  in that data.

**How the pin travels (operator design, same amendment).** The compiler and runtime are ONE
versioned pair, and the pin is self-describing end to end:
- The compiler is BUILT against a fixed runtime interface + a fixed runtime content hash (change the
  runtime → new hash → rebuild the compiler → new generation).
- Each emitted component RECORDS the required runtime's content hash in itself (alongside its
  `cadenza:runtime/heap` import), so the artifact carries both *what interface* and *which exact
  implementation*.
- The host RESOLVES by hash: reads the required hash from the component, looks it up in a
  content-addressed store (`<store>/<hash>.wasm`), composes; refuses (does not substitute) if the
  hash is absent. Programs pinned to different runtime versions coexist.
- Build choreography is an **xtask**: compile runtime → hash it → compile compiler passing that hash
  → both land in the content-addressed store. Build ORDER encodes the dependency; this is the
  concrete realization of "versioned together" (component-abi.md §The Emitted Component Records Its
  Required Runtime, §The Host Resolves The Runtime By Content Address; reproducible-derivation.md
  build-pair invariant).
Payoff: the handle boundary is stable, so the runtime REPRESENTATION can evolve (Phase D RC, Phase E
CHAMP/RRB) — each change is just a new hash a new compiler generation targets, no ambiguity about
which runtime a given program expects.

**Debt.** The runtime is authored in Rust now; at self-hosting (M8/M9) it must be re-authored in
Cadenza — a foreign-language artifact the self-hosted compiler cannot itself derive is a gap to
close, tracked, not blocking.

**Emitter is self-contained (operator).** The compiler emits every component byte itself — no
external tooling at compile time — because the goal is to compile Cadenza with only a wasm runtime.
`wasm-tools` is a dev-desk oracle used to derive a validated reference byte-shape, which is baked
into the compiler as fixed constant byte-arrays (exactly as the current runnable-component envelope
was produced); the compiler emits by concatenating those constants with a core module it builds
byte-by-byte.

**Proof of mechanism (2026-07-05, before the spec edits).** A Rust `cadenza:runtime/heap` component
plus a throwaway importing program component, composed in wasmtime-37 by forwarding the program's
import to the runtime instance, produced a correct `(tuple 3 1)`. (That prototype had the runtime
render; under the operator refinement the rendering moves into the program — `program.run() -> string`
walks the value through the runtime's accessors and assembles the text — but the composition
mechanism the proof validated, the host forwarding the program's runtime import to the runtime
instance, is unchanged.) cargo-component tree-shook the program's imports to only the funcs it used —
the lean-artifact win, automatic.
