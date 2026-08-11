# Strict compiler↔platform separation — the compiler is generic over any WIT contract, knows no specific one

**Status:** design/scoping only — nothing landed. Written 2026-08-10 by the
`design-compiler-platform-split` fleet design agent, AUTONOMOUSLY (operator: "The design will need to be
autonomous. But it has the mandate."). It grounds in the already-decided architecture — `design/agent-harness-kernel.md`
§9b/§23, `DESIGN-binary-ast-abi.md`, `DESIGN-userspace-effects.md`, `spec/contracts/component-abi.md` — and
hands a build plan to `v-compiler-ml`/`v-rust-backend` (the rcdzc emit owner) with `v-agent-harness`
(reducer.wit / kernel owner) confirming the platform side. File anchors are landmarks at trunk `862b6a2f0`.

> **Both open technical decisions are now RESOLVED by the owning verticals (2026-08-10, confirming — the
> boundary was unchanged, only the increment mechanics sharpened):**
> - **D3 (v-compiler-ml, emit owner):** the generic handle→canonical-ABI marshal ALREADY EXISTS on the
>   non-reducer path (`select.rs:11463-11522`, keyed on the param's declared TYPE, not op-name) —
>   `reducer_kv_shim_body` is a DUPLICATE of it. So M1 = **un-fork-reuse**, plus ONE generic generalization:
>   lift the existing single-compound-arg cap (`select.rs:11492`) to N args (reducer `put(key,value)` has
>   two `list<u8>` args; this also fixes the non-reducer two-arg decline). Still compiler-side + generic.
> - **D1 (v-agent-harness, kernel owner):** confirm **P(a)** — keep `kv` a named WIT import. §4b (a reducer
>   reading its OWN KV is a direct query, NOT a world-touching effect) is a REAL semantic distinction, not an
>   implementation accident; P(b) would erase it. P(a) fully satisfies the compiler mandate; P(b) stays a
>   userspace-effects follow-up. Nuance: the kv import NAME is already data-driven in the emit
>   (`bindings.get(effect)`, not a baked `"kv"` literal) — so P(a) needs zero compiler change for genericity
>   beyond M1's generic marshal.

> **The operator's mandate (2026-08-10), verbatim + refinement.** First, the principle: *"The compiler
> should not know anything about the platform. The platform should not know anything about the compiler.
> These things should be strictly separated."* Then the sharpening that fixes the exact target: *"The
> compiler should have knowledge of WIT, for sure. That's the contract/abi. But it shouldn't know about
> any one WIT contract. Cadenza programs should be able to create and interface with any WIT. That's the
> whole point."*
>
> This SUPERSEDES the incremental "signature-driven generic shim + cabi_realloc" fix that was proposed to
> de-hard-code the B3 kv-shim in place. The operator rejected that framing: incremental de-hard-coding is
> not the point; a clean architectural boundary is. The trigger was B3 (rcdzc binary-AST reducer emit)
> putting reducer/kv/kernel-protocol knowledge INTO the compiler.

---

## 1. The target, stated precisely

The boundary is **not** "the compiler knows no WIT." WIT is the component-model ABI — the contract every
Cadenza program crosses to reach anything outside itself, and the whole point of the language is that a
Cadenza program can *create and interface with any WIT*. So the boundary is:

- **The compiler knows WIT GENERICALLY** — the canonical component-model ABI: how to lift/lower a value of
  any declared WIT signature, how to emit a component that imports/exports interfaces, how to marshal the
  language's value model across the canonical ABI. This is inherent compiler work and STAYS.
- **The compiler knows NO SPECIFIC contract** — no `kv`, no `cadenza:agent-kernel/fold`, no `put`/`get`, no
  `EffectRequest` record shape, no reducer-fold event projection. A Cadenza *program* declares whatever WIT
  interface it wants to import or export; the compiler marshals that declared signature canonically without
  ever knowing what it *means*.
- **The platform knows NO compiler internals** — it never sees a value-heap handle, never binds a heap op,
  never depends on how the compiler represents values. It defines WIT contracts and consumes/produces
  canonical-ABI components. This is the standing §23 "kernel is runtime-agnostic — it knows nothing about
  the value-heap runtime" invariant, restated for the whole platform.

The two sides meet at exactly one thing: **a WIT interface + the canonical component-model ABI.** Neither
names a concept private to the other.

## 2. What exists today — the clean line already drawn, and the one place it was crossed

The crucial finding (from `v-compiler-ml`, who owns the rcdzc emit and mapped both paths this week): **the
compiler ALREADY HAS a generic, platform-agnostic host-import path. The reducer path FORKED from it and
hand-rolled the platform specifics.** The fix is to un-fork, not to invent.

### 2a. The generic host-import path (the correct template, already shipping)

A normal effectful Cadenza program's host imports are emitted generically
(`rcdzc/src/backend/wasm/envelope.rs:1711`/`1909` `assemble_host_runtime`/`assemble_host_runtime_mem`):

- The import is keyed on the effect's **own declared name** (`host_imports[0].effect` → the WIT interface;
  `mod.rs:1174`) — generic, no platform allow-list, no hard-coded interface string.
- A `string`/`list<u8>` parameter crosses via the **canonical component-model lowering**: the guest lowers
  the value to `(ptr,len)` in a shared memory by the STANDARD mechanism, and the WIT import's canon-lower
  reads it (`envelope.rs:1892`: "the `(ptr,len)` a string lowers to is read from a memory both the program
  and host share"). NO hand-rolled copy loop — the canonical ABI does the marshalling.

This path is exactly "the compiler emits a component importing effect ops by their declared WIT signature,
and knows nothing about what they are." It is the template the whole design generalizes to.

### 2b. Where the line was crossed — the reducer path (B3)

The reducer emit path (`rcdzc/src/backend/wasm/mod.rs:592`+, gated by `is_reducer_fold_apply`) forked from
2a and baked in platform knowledge. Three distinct leaks, all in `rcdzc/src/backend/wasm/`:

1. **The kv-shim marshal (the headline).** `KvShimOp::Put/Get` enum (`serialize.rs:7430`), selected by
   matching the literal string `"put"` (`mod.rs:683`), with a hand-rolled handle↔linmem copy loop through
   fixed scratch offsets `4096`/`8192`/`12288` (`serialize.rs:7447-7620`, `reducer_kv_shim_body`). The
   compiler here KNOWS the `kv` effect exists and knows its op shapes. **This is the operator's exact
   objection.**
2. **The kernel fold-protocol shape.** `REDUCER_FOLD_IFACE = "cadenza:agent-kernel/fold"` (`mod.rs:110`);
   the event-record field names `content_type`/`payload`/`resumes` (`mod.rs:143`, commented "the
   KERNEL-PROTOCOL field names"); the effect-request flat record `{correlation, kind, target, payload}`
   (`mod.rs:127`). The compiler bakes the agent-kernel platform's wire contract.
3. **The effect-binding classification.** `db.effect_bindings` (`db.rs:863`, scanned `db.rs:2514`) decides
   which effects are "bound peers" vs runtime ops — platform wiring living in compiler state.

**Why it forked (the real technical reason, load-bearing for the design):** a reducer guest returns/consumes
value-heap **handles** (opaque `u32` into the shared `cadenza:runtime/heap`), not values already lowered to
`(ptr,len)` in linear memory. The fold boundary reshapes to `list<u8>` bytes (B2, good), but the `kv` args
are still heap handles the canonical lowering can't see — so the shim copies handle→linmem by hand. The fork
is a workaround for "a canonical lowering that only knew how to lower already-in-linmem values, not
handle-backed values."

### 2c. The parts that are inherently compiler-side and STAY (not leaks)

Per `v-compiler-ml`'s classification — these are the language's own value model, not a platform:

1. Lowering Cadenza source to core wasm.
2. The **value-heap handle ABI** — Cadenza values ARE `u32` handles into `cadenza:runtime/heap`. This is the
   LANGUAGE's value representation, not a platform contract. The compiler owns it.
3. The **canonical component-model lift/lower** for any `list<u8>`/declared-signature boundary (standard WIT
   ABI). Generic over any WIT — exactly what the mandate wants the compiler to know.
4. `value-decode`/`value-encode` of a value against its structural shape descriptor (the guest's own data
   shapes). The runtime does the heap↔bytes work (`runtime.wit` idx 62/90); the compiler bakes the
   descriptor for its own static types.

The insight: (2)+(3) TOGETHER are "the language's value model crossing the canonical ABI." The
value-heap-handle↔canonical-ABI bridge is the meeting of the two — and it is **compiler-side and generic**,
never platform-side (§4 resolves this fork explicitly).

## 3. The direction — un-fork the reducer path onto the generic host-import path; make the handle↔ABI marshal generic-over-signature

**The design in one sentence:** the reducer path stops hand-rolling a `kv`-specific handle↔linmem shim and
instead marshals *any declared WIT import signature* through the SAME canonical-ABI machinery the
non-reducer path uses — extended once so it lowers a **value-heap-handle-backed** value (not only an
already-in-linmem value) — so the compiler emits ONE generic import surface, knows no specific contract,
and the platform never sees a heap handle.

Concretely the compiler emits, for a reducer exactly as for any program:

- **Exports:** whatever WIT the program declares it exports (a reducer program declares it exports the
  agent-kernel fold interface — but the *compiler* just sees "export interface X with signature
  `apply: func(list<u8>) -> list<u8>`"; it does not know X is "the reducer fold"). The `list<u8>` boundary
  lifts/lowers canonically (already true post-B2).
- **Imports:** whatever WIT the program declares it imports (a reducer program declares it imports the `kv`
  interface with signatures `get: func(list<u8>) -> option<list<u8>>`, etc. — but the *compiler* just sees
  "import interface Y with these signatures"; it does not know Y is "kv" or that `put` is special). Each
  import call marshals the guest's value-heap-handle arguments to the declared WIT signature via the generic
  canonical-ABI marshal, and marshals the WIT result back to a handle.

The `kv`-ness, the reducer-ness, the fold-protocol — ALL of it moves to where it belongs:

- **The program source** declares the WIT interfaces (import `kv`, export the fold interface). This is the
  "Cadenza programs create and interface with any WIT" the operator wants — the reducer's contract is
  written in Cadenza + a WIT declaration, not baked into the compiler.
- **The platform** (host) provides the `kv` import and drives the fold export. It defines what `kv` means
  and what the fold boundary carries. It consumes a plain canonical-ABI component and never inspects a heap
  handle.

### 3a. This is generic over ANY WIT, which is the whole point

Once the reducer path is un-forked, there is nothing reducer-specific in the compiler. A Cadenza program
that declares it imports some *other* host interface — a userspace effect handler's WIT, a Rust host SDK's
WIT, an arbitrary component's exported interface — compiles the same way: the compiler marshals the declared
signature canonically. The reducer is just "a program that happens to import `kv` and export a fold
interface," with zero compiler privilege. That is the mandate realized: **generic-over-WIT, not WIT-free.**

## 3b. The full-A end-state (operator override 2026-08-11): rcdzc ingests the TARGET WIT WORLD; one value-bridging rule for every member, exports AND imports

The operator overrode the interim "narrow fold-only bytes-wrap" (concierge B ruling): the mandate is **full
A only, end-state defined first then sliced backwards, no workarounds.** This section pins that end-state on
the compiler-emit side (v-compiler-ml owns it; v-agent-harness owns the first concrete target world —
`reducer.wit` + the Event/effect-request value-form contract).

**End-state input: rcdzc ingests a TARGET WIT WORLD, not a name.** Today the driver hands rcdzc the bare
interface NAME (`KIND_COMPONENT_NAME`) plus, as a stepping-stone, a `KIND_EXPORT_BYTES_MEMBERS` list naming
which export members cross as `list<u8>` (the `db.export_bytes_members` signal). The end-state replaces both
with the **full target WIT world**: every exported/imported interface with its complete member signatures.

**rcdzc does NOT parse WIT (operator refinement 2026-08-11).** Verbatim: *"I'd prefer the compiler not to
have to parse WIT. It should take a preparsed WIT description in the binary AST form as one of the
artifacts."* So the target world reaches rcdzc as a **PREPARSED binary-AST artifact** — a WIT world lowered
into the SAME `cadenza-ast` binary codec the rest of the pipeline already speaks (the artifact stream
`compile()` consumes, alongside the program AST + today's `KIND_COMPONENT_NAME`). A SEPARATE producer does
the WIT-text→binary-AST-world lowering (a toolchain step / a `cdz` subcommand, NOT the compiler); rcdzc just
consumes the artifact and reads each member's declared param/result canonical-ABI types off it. **Two
sources, one structured world (operator, 2026-08-11):** the target world reaches rcdzc EITHER as this
external preparsed-binary-AST artifact OR as an INLINE world declaration in the module itself (self-contained,
no external resource — likely the common case for a simple fold). v-agent-harness's binary-AST schema covers
BOTH sources, lowering each to the SAME structured world the emit reads — so the emit is source-agnostic:
rcdzc never cares whether the world came inline or from an artifact, only that it has the member signatures.
This keeps WIT-parsing entirely out of the compiler — consistent with the binary-ast-abi direction and the
generic-compiler mandate (the compiler consumes a declared world, knows no specific contract). Two schema
pieces to pin (coordinate: **v-agent-harness** owns the binary-AST/kernel side, **v-syntax** if the AST needs
a new WIT-world node): (1) the binary-AST SHAPE of a preparsed WIT-world artifact (interfaces → members →
param/result canonical-ABI types), and (2) how a program REFERENCES its target world (a world-name artifact,
or the world artifact carries the binding). `export_bytes_members` is then an interim NARROWING (just "which
members are bytes") that collapses into "read the member's declared type from the preparsed world artifact" —
the emit MECHANISM is unchanged, only the trigger generalizes to the artifact read.

**The one value-bridging rule (the general rule the operator wants), symmetric across exports and imports.**
At every WIT boundary member, for each param and result position, compare the DECLARED canonical-ABI type
(from the world) against the GUEST value-model type (what the Cadenza program actually has there):

- **Match → pass through.** A declared scalar the guest also has as that scalar; a declared `list<u8>` the
  guest already produces/consumes as raw bytes. No bridge — the canonical lift/lower is the identity on the
  value model. (This is the non-reducer common case.)
- **Mismatch → bridge via the value-heap codec.** Where the guest value-model type differs from the declared
  canonical-ABI type, the compiler inserts the value-heap `value-encode`/`value-decode` bridge (the language's
  own value model crossing the canonical ABI, §2c items 2-4; §4's RESOLVED compiler-side-generic decision):
  - **Import call (guest → host), already specified in §4/§6:** value-heap-handle arg → marshal to the
    declared canonical type (`value-encode` for a compound handle; the S0 `bytes-len`/`bytes-get` copy for a
    Bytes/String handle); declared canonical result → lift back to a handle (`value-decode` for a compound).
  - **Export member (host → guest → host), the SYMMETRIC twin this section adds:** a member the guest
    PROVIDES whose declared param is `list<u8>` (or any canonical type) but whose guest value is a COMPOUND
    → `value-decode(param_bytes, param_shape_desc)` on entry to reconstruct the guest value; run the guest
    body; `value-encode(result, result_shape_desc)` to lower the compound result back to the declared
    `list<u8>`. This is exactly the export-side bridge the doc previously assumed "already works post-B2" —
    B2 was reverted, so it is a real emit the compiler must produce, and it is NOT reducer-specific: it fires
    for ANY exported member whose declared canonical type differs from the guest value-model type.

**Why world-targeting adds emit information the ad-hoc export does NOT (operator question, 2026-08-11).**
"Aren't we already exporting an ad-hoc world today — the host binds structurally, name-agnostic — so what
does explicit world-targeting add?" HOST BINDING is indeed already structural + name-agnostic; what
world-targeting adds is entirely on the **compiler-emit side**. The compiler needs each member's DECLARED
signature to know (a) which canonical-ABI type to emit that member to, and (b) whether the value-bridge
fires (declared-type vs guest-value-model type). Without a declared target, the compiler emits the guest
value's NATURAL ABI — a value-heap handle or a canonical record for a compound — NOT `list<u8>`; that is
exactly the genesis mismatch (guest emits a handle/record, kernel expects bytes). Auto-emitting bytes for a
"fold" would be hard-coded fold knowledge the mandate forbids. So the PROGRAM declares its export ABI
(inline or artifact) and the compiler emits to match. Net: world-targeting supplies (1) the emit-to
signature that drives the bridge, and (2) a checkable guest-satisfies-world contract (the guest's
value-model type must be bridgeable to the declared type, else a compile error — not a silent wrong emit).
The ad-hoc export suffices for host BINDING but cannot tell the compiler to emit bytes.

**The reducer is ONE instance.** `reducer.wit` (v-agent-harness's A1 branch) declares the fold export
directly as `apply: func(list<u8>) -> list<u8>` — a pure bytes boundary — and imports `kv`. IMPORTANT (v-ah
clarification): the `content-type`/`payload`/`resumes` Event structure and the `effect-request` shape are NOT
WIT params — they are the VALUE-FORM CONTRACT carried INSIDE the one event doc / result doc (v-ah deleted the
earlier structured-types interface). So the WIT itself is bytes↔bytes; the compound lives in the value form.
Under the rule: the exported `apply`'s declared `list<u8>` param/result differ from the guest's compound
Event / `List<EffectRequest>` value-model type → the export-side bridge fires (value-decode the Event doc,
value-encode the effect-list doc); the `kv` imports marshal per §4. Nothing reducer-specific in the compiler
— it is the general rule applied to this world. (The Event/effect-request value-form contract itself —
`content-type{family,version}`, `effect-request{kind:enum, target, payload, correlation, family}` — is
v-agent-harness's to pin alongside the target-world artifact; the compiler only needs each member's declared
canonical-ABI type from the artifact + the guest value-model type it already knows.)

**Backward slice (what v-compiler-ml is building, reframed as the first full-A slice — NOT the killed
wrap).** The export-side bridge MECHANISM is built: `emit_bytes_roundtrip_apply_body` (value-decode param →
body → value-encode result), `emit_bump_realloc_body` (real `cabi_realloc` for the input-list lowering), and
`bytes_roundtrip_core_module` (assembles the `apply(list<u8>)->list<u8>` core). This is member-signature-
driven and generic over any bytes-boundary export member — the export-side twin of §4's import marshal, not a
fold-hard-coded wrap. Its current trigger (`db.export_bytes_members`) is the interim narrowing above; wiring
it to the full-WIT-world read is the generalization step. So the slice ORDER backward from the end-state is:
(i) export-side bytes bridge mechanism [built], (ii) drive it from the target WIT world instead of
`export_bytes_members`, (iii) generalize the bridge to any declared-type↔guest-type mismatch (not only
`list<u8>`), (iv) the import side is §4/§6's S0-S3 (S0 landed).

## 4. The crux — where the value-heap-handle↔canonical-ABI bridge lives (RESOLVED: compiler-side, generic)

`v-compiler-ml` named the one genuine architectural fork: *"the value-heap-handle↔linmem BRIDGE is
unavoidable SOMEWHERE (Cadenza handles are not component-ABI values) — does it live in the compiler (as a
generic per-signature marshal, platform-agnostic) or does the platform inject it?"* Two options:

- **Option A — the bridge is compiler-side + generic.** The compiler marshals a value-heap-handle-backed
  value to/from any declared WIT signature via the canonical ABI. The platform only ever sees canonical WIT
  values (a `list<u8>`, an `option<list<u8>>`, a record), NEVER a heap handle.
- **Option B — the bridge is platform-side.** The compiler emits the guest importing its effects with the
  handles crossing as the runtime's own handle type; the PLATFORM/host does the handle↔`list<u8>` bridging.

**DECISION: A.** B is rejected because it forces the platform to know the value-heap handle representation —
which directly violates §23 ("the kernel MUST have ZERO special knowledge of the Cadenza value-heap
runtime … it is NOT a built-in the kernel knows by name, interface prefix, or identity"). Under B the
platform would have to bind heap ops to read/write handles — the exact `HeapHandle` coupling the
binary-ast-abi design already deleted from the kernel host. A is the only option that keeps BOTH the
compiler-side mandate AND §23:

- The handle↔ABI bridge IS the language's own value model crossing the canonical ABI (§2c items 2+3). It is
  inherently compiler work — the compiler is the only component that knows a Cadenza value is a heap handle,
  and that knowledge must never leak out. Keeping the bridge compiler-side is what KEEPS the platform
  runtime-agnostic.
- It is **generic over the signature**, not `kv`-specific: the marshal reads the declared WIT param/result
  types and lowers/lifts each handle-backed value accordingly. No `KvShimOp`, no `op == "put"`, no fixed
  scratch offsets — the same canonical mechanism for `get`, `put`, `delete`, `prefix-scan`, or any op of any
  interface the program declares.

**The M1 mechanics — RESOLVED by v-compiler-ml (emit owner, 2026-08-10): the generic marshal ALREADY
EXISTS.** The non-reducer `Core::HostCall` path already lowers a value-heap Bytes/String *handle* to a
canonical WIT `(ptr,len)` GENERICALLY (`select.rs:11463-11522`): for a runtime `String`/`Bytes` arg (a heap
handle — rope OR slice-view), it does NOT assume linmem residency — it MARSHALS by copying the handle's
logical bytes into a linmem scratch region via the rep-agnostic `bytes-len`/`bytes-get` walk (transparent
through rope/slice), then pushes `(scratch_base, len)` for the canonical `list<u8>`/`string` lower to read
(comment at `:11467`). It is keyed on the param's declared TYPE (`String`/`Bytes` → marshal; scalar →
passthrough), NOT on an op name. **`reducer_kv_shim_body` is a DUPLICATE of this exact `bytes-len`/`bytes-get`
copy loop.** So:

- **M1 = un-fork-REUSE, not build.** Route the reducer's bound-effect args through the SAME `String`/`Bytes`
  marshal the non-reducer `HostCall` uses, instead of the parallel `KvShimOp`/`op == "put"` hand-roll.
- **The ONE genuine addition (a generic generalization that benefits BOTH paths):** the existing non-reducer
  marshal caps at ONE runtime compound arg per call — "a host call with TWO runtime string/Bytes arguments
  is not yet emitted" (`select.rs:11492`; a single fixed scratch buffer, `runtime_string_arg_seen` declines
  the second). But reducer `put(key: list<u8>, value: list<u8>)` has TWO `Bytes` args. So M1 lifts the
  one-compound-arg cap to N args (a per-arg scratch/bump — which the fixed-scratch→`cabi_realloc` move also
  wants). This generalization is COMPILER-side + generic (a per-signature N-compound-arg marshal), fixes the
  non-reducer two-arg decline too, and lets the reducer path drop `KvShimOp` entirely.

Net: **mostly un-fork-reuse, plus one generic multi-compound-arg-marshal increment.** Either way the boundary
from §3/§4 is unchanged — this only fixes M1/S0's shape (a generalization of an existing generic marshal, not
a new bespoke one).

## 5. The platform side — kv stays a declared WIT import (the compiler never names it)

For the compiler to be generic, `kv`-knowledge must live OUTSIDE it. Where? Two sub-options for the platform
(sent to `v-agent-harness`, the reducer.wit/kernel owner, 2026-08-10):

- **P(a) — kv stays a named WIT import a reducer PROGRAM declares.** `reducer.wit` keeps the `kv` interface
  (`reducer.wit:51-58`); a reducer program's source declares "I import `kv`." The compiler marshals
  `get`/`put`/`delete`/`prefix-scan` against the declared signature WITHOUT knowing what `kv` means; the
  host provides the import. `kv`-knowledge lives in the program source + the host. This keeps `kv` a
  first-class direct-query per §4b ("a reducer reading its own KV is NOT a world-touching effect").
- **P(b) — collapse the synchronous kv channel into the generic register-by-string / schema-identity effect
  model** the async `apply`-return channel already uses (per the userspace-effects capstone: "anything
  stateful/kernel-side looks like any other effect handler"). Fuller minimize-kernel realization, but
  re-touches the §4b "own-KV-is-not-an-effect" distinction.

**P(a) — CONFIRMED by v-agent-harness (kernel owner, 2026-08-10), and for a SEMANTIC reason, not just size.**
§4/§4b is a REAL distinction (`reducer.wit:10-11`, `:47-48` pin it): a reducer reading its OWN KV is a
synchronous, deterministic, replay-stable INLINE query of its own session state — no host round-trip, no
authz, no world-visibility. An EFFECT is a REQUEST to touch the WORLD (async, authorized, executor-routed,
world-visible). P(b) would ERASE that distinction — make an own-state read look like a world-effect — which
is semantically wrong AND bolts authz/routing/async machinery onto a pure inline query. P(a) keeps `kv` a
first-class §4b direct-query AND fully satisfies the compiler mandate: the compiler marshals `get`/`put`/
`delete` against the DECLARED SIGNATURE (the S0/§4 generic marshal) without knowing the name `kv` or its op
shapes. **Nuance (v-agent-harness):** the kv import interface NAME is ALREADY data-driven in the emit
(`bindings.get(effect)`, not a baked `"kv"` literal) — so P(a) needs ZERO compiler change for name-genericity
beyond M1's generic marshal. P(b) stays a **userspace-effects follow-up** (`DESIGN-userspace-effects.md`) —
it re-opens the §4b own-KV question, which is exactly that capstone's call; NOT coupled to this design (which
P(a) already completes). This design's boundary is identical under either.

## 6. Increments (each its own commit + gate; top-to-bottom, the way a vertical lands them)

**S0 — generalize the existing generic marshal to N compound args (the foundation).** The generic
per-signature handle→canonical-ABI marshal ALREADY EXISTS (`select.rs:11463-11522`, §4) but caps at ONE
runtime compound arg (`select.rs:11492`). S0 lifts that cap to N args — a per-arg scratch/bump layout (the
fixed-scratch→`cabi_realloc` move wants this anyway) — so a host call with multiple runtime `String`/`Bytes`
args (like reducer `put(key, value)`) marshals each canonically. This is a generalization of an existing
generic mechanism, NOT a new bespoke marshal, and it fixes the non-reducer two-arg decline too. Gate: a unit
test emitting a host call with TWO runtime `Bytes` args, each marshalled to canonical `(ptr,len)` against the
declared signature — no op-name matching, no single-fixed-scratch cap. **This is the probe increment** —
prove the N-arg generic marshal before the reducer path consumes it. (v-compiler-ml, the emit owner, will
prototype it.)

**S1 — un-fork the reducer imports onto the generic marshal; DELETE the kv-shim (`v-compiler-ml`).** The
reducer emit path's bound-effect import calls route through the S0 N-arg marshal — the SAME `String`/`Bytes`
marshal the non-reducer `HostCall` uses (`select.rs:11463`), keyed on the declared import signature. DELETE
`KvShimOp` (`serialize.rs:7430`), the `op == "put"` match (`mod.rs:683`), and `reducer_kv_shim_body` + the
fixed scratch offsets `4096`/`8192`/`12288` (`serialize.rs:7447-7620`) — full collapse, no compat path
(operator's no-adapter-layers directive; `reducer_kv_shim_body` is a proven duplicate of the marshal S0
generalizes, so nothing is lost). Note the kv import interface NAME is already data-driven
(`bindings.get(effect)`, not a baked `"kv"` literal) — so S1 needs no name-de-hardcoding, only the marshal
un-fork. Gate: the existing reducer E2E (the 4 reducers, `component_reducer_e2e`) passes with `kv` calls
going through the generic canonical marshal; `KvShimOp`/`reducer_kv_shim_body`/`op == "put"` grep-absent;
`cargo xtask gate` additive-only. This is the increment that removes the headline violation.

**S2 — hoist the fold-protocol shape out of compiler constants into program-level WIT declaration
(`v-compiler-ml`, coordinate `v-agent-harness`).** Remove `REDUCER_FOLD_IFACE` (`mod.rs:110`), the
kernel-protocol event field names (`mod.rs:143`), and the `EffectRequest` record shape (`mod.rs:127`) from
compiler constants. The reducer program declares the fold interface it exports (as WIT, in source); the
compiler emits it generically as "export interface X with signature `apply: list<u8> -> list<u8>`." The
event/effect AST shapes are the program's own data shapes (already handled by value-decode/encode
descriptors, §2c item 4) — the compiler bakes descriptors for the program's static types, not for a
kernel-protocol it names. Gate: the reducer path has NO string literal `"cadenza:agent-kernel/…"` and no
hard-coded protocol field names (grep-proven); reducer E2E still green. This is where `is_reducer_fold_apply`
(`mod.rs:116`) dissolves — there is no longer a special reducer branch, just "a program exporting a declared
interface."

**S3 — generalize `db.effect_bindings` classification (`v-compiler-ml`).** The "bound peer vs runtime op"
decision (`db.rs:863`/`2514`) becomes purely a function of the program's declared imports (an import with a
declared WIT signature is a host import; a `cadenza:runtime/heap` op is a runtime op) — no platform table,
no reducer-specific classification. Gate: the classification is driven by the declared import set only;
reducer + non-reducer paths use the same rule. (May fold into S1/S2 if small — a coherent unit, not a drip.)

(S0 is independent + first. S1 depends on S0. S2 is independent of S1 but same subsystem — sequence after S1
to keep each reducer-E2E green. S3 is cleanup, foldable. Each is independently green: S0 doesn't touch the
reducer path; S1 flips the marshal with E2E re-validated in-commit.)

## 7. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — platform kv model: P(a) named WIT import vs P(b) schema-identity effect (§5).** RESOLVED:
  **P(a)** — CONFIRMED by v-agent-harness (kernel owner, 2026-08-10) for a semantic reason (§4b own-KV =
  direct query, NOT a world-effect; P(b) would erase it). P(b) is a userspace-effects follow-up, NOT folded
  in here. No operator escalation needed — the platform owner ruled.
- **D2 — handle↔ABI bridge: compiler-side generic (A) vs platform-side (B) (§4).** RESOLVED: **A** — B
  violates §23 (would recouple the platform to the heap representation). No escalation; recorded as the
  load-bearing architectural call.
- **D3 — S0 shape: reuse the existing canonical lowering vs build a generic handle→ABI marshal (§4, §6).**
  RESOLVED: **reuse** — v-compiler-ml (emit owner, 2026-08-10) confirmed the generic type-driven marshal
  already exists (`select.rs:11463`); `reducer_kv_shim_body` duplicates it. M1 = un-fork-reuse + one generic
  N-compound-arg generalization (lift the `select.rs:11492` single-arg cap). No operator escalation — the
  emit owner ruled.
- **D4 — does the fold export interface get WIT-declared in source, or does the toolchain supply the reducer
  WIT?** Default: the reducer program declares the interfaces it imports/exports (import `kv`, export the
  fold interface) as WIT alongside its source — "Cadenza programs create and interface with any WIT" taken
  literally. The compiler consumes the declaration; the platform publishes the WIT of the interfaces it
  provides/drives. Recorded for the S2 implementer; no fork.

## 8. Migration against the frozen contracts

- **`component-abi.md` — UNCHANGED.** This design REMOVES compiler-side specialization; it does not change
  the canonical component-model ABI (it makes the reducer path USE the canonical ABI it was bypassing). The
  value-heap handle ABI (Boundary A, `runtime.wit`) is untouched — no new op, no hash bump. The fold
  boundary (`reducer.wit` `apply: list<u8> -> list<u8>`) is untouched — B2 already put it in the right
  shape; this design just stops the compiler from baking the protocol constants around it.
- **`reducer.wit` — UNCHANGED under P(a) default.** `kv` stays a declared host import; `apply` stays
  `list<u8> -> list<u8>`. Under P(b) (NOT this design's default) it would change — deferred to the
  userspace-effects arc.
- **No frozen-contract escalation is required by this design.** It is a compiler-internal un-forking: delete
  the reducer-specific marshal + protocol constants, route through the existing generic canonical-ABI path.
  The only external coordination is confirming the platform side stays P(a) (D1).

## 9. Watch-outs (for the implementing vertical)

- **Delete, don't wrap (S1/S2).** `KvShimOp`, `reducer_kv_shim_body`, `REDUCER_FOLD_IFACE`, the baked
  protocol field names — DELETED, not kept behind a flag. The generic path is the only path after S1/S2
  (operator no-adapter-layers directive).
- **The bridge stays compiler-side — never let a handle leak to the platform (§4/D2).** If any increment
  finds itself wanting the host to read/write a heap handle, that is re-introducing the §23 violation —
  STOP and re-route through the compiler-side marshal.
- **Generic means signature-driven, not name-driven.** The marshal must read the declared WIT param/result
  types — NEVER match an op name (`"put"`), an interface name (`"kv"`, `"cadenza:agent-kernel/…"`), or a
  field name. A grep for those literals in the reducer path is the structural gate.
- **The reducer E2E is the behavior gate (S1/S2).** The 4 reducers folding events → effects through the
  bytes boundary must stay green across the un-fork — re-validate the fixtures in the same commit that flips
  the marshal.
- **rcdzc emit changes verify on the compiler-ml self-host, not just the lib suite + gate** — a reducer emit
  change can pass `-p rcdzc --lib` + `xtask gate` yet break the self-compile; run the compiler-ml
  self-host path (memory: `rcdzc-emit-change-must-verify-on-compiler-ml-self-host-not-just-lib-suite-and-gate`).
- **Coordinate the WIT-declaration mechanism with `v-agent-harness` (S2).** How a reducer program declares
  the fold interface it exports (D4) touches the reducer.wit contract's authoring story — confirm the
  platform publishes the WIT of what it drives so the program can declare against it.

## 10. Verification (the gate that protects this)

- S0: the generic handle↔canonical-ABI marshal round-trips a handle-backed `list<u8>` + `option<list<u8>>`
  against a declared signature, signature-driven (no op-name/scratch-offset), unit-tested.
- S1: reducer E2E green through the generic marshal; `KvShimOp`/`reducer_kv_shim_body`/`op == "put"`/fixed
  scratch offsets grep-absent; `cargo xtask gate` additive-only (no `Todo→Fail`).
- S2: no `cadenza:agent-kernel/…` string literal or hard-coded protocol field name in the reducer path
  (grep); `is_reducer_fold_apply` dissolved; reducer E2E green.
- S3: import classification driven only by the declared import set; reducer + non-reducer paths share the
  rule.
- Throughout: `cargo test -p rcdzc --lib` 0 failed; `cargo xtask check` fmt+clippy+`codegen --check` clean;
  the compiler-ml self-host path green (rcdzc emit change discipline); `cargo xtask gate` additive-only.

## 11. Relationship to the schema-identity + binary-ast-abi arcs

This design is the natural COMPLETION of `DESIGN-binary-ast-abi.md`. That design cleaned the kernel↔reducer
value CROSSING (bytes over the fold boundary; the byte↔handle bridge moved from the kernel host into the
runtime). This design cleans the remaining compiler-side coupling: the reducer EMIT path still baked the
platform's protocol + a kv-specific marshal. Together they achieve the full §23 posture — the platform sees
only bytes + declared WIT; the compiler emits only generic canonical-ABI components.

It does NOT depend on the schema-identity arc (`schema-identity-wiring-execution-plan-post-b2`) and does not
block it — they are orthogonal. Schema-identity is about how the PLATFORM identifies/authorizes an effect
family (Hash of the effect schema tree); this design is about the COMPILER emitting generically. Under P(b)
(NOT the default here) they would converge — kv-as-schema-identity-effect — but that convergence is the
userspace-effects arc's, not this one's. Kept deliberately separate so this design lands as a
compiler-internal un-forking with no platform-contract dependency.
