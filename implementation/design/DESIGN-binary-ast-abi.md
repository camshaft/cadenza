# Passing a binary AST across the runtime/guest ABI — dissolve the hard-coded heap-op marshalling

**Status:** design/scoping only — nothing landed. Written 2026-08-09 by the `design-binary-ast-abi`
fleet agent on an operator direction REVERSAL (below). This is an architectural design of the
kernel↔reducer boundary, NOT of the value-heap runtime's own internal ABI (that stays); it hands a
build plan to `v-agent-harness` (the kernel/ABI owner) and coordinates `v-runtime` (one new runtime op),
`v-rust-backend` (the Rust-guest angle), and `v-syntax` (owns `cadenza-ast`, the wire codec). Line
numbers are landmarks at trunk `f669d5829`.

> **The operator's mandate (2026-08-09), verbatim.** "I really dont like hard-coding the heap ops in
> here. Its going to make it incredibly difficult to change those APIs in the future. I am wondering if
> we just need to pass a binary AST across the abi. Thats going to be a lot more stable than whatever
> were doing here and would actually work with a rust guest - the current one wont. I know this goes
> against my original direction but seeing it in its current state made me realize its not great and
> probably the wrong call."
>
> This REVERSES the earlier direction that composed a real Cadenza reducer against the shared value-heap
> runtime and marshalled its `apply` arguments field-by-field into heap handles (the `HeapHandle` in
> `cdz-kernel/src/wasm_host.rs:920`). The operator is unavailable to iterate for ~a week, so this design
> makes the engineering calls autonomously from the stated direction and records the open forks with a
> chosen default; only a genuine unresolvable fork is escalated as an `ask`.

---

## 1. What exists today — where the hard-coding actually is (two boundaries, not one)

There are TWO distinct component boundaries in play, and it matters which one the operator's complaint
lands on. They are NOT the same interface.

- **Boundary A — the value-heap runtime interface** (`cdz-runtime/wit/runtime.wit`): the `cadenza:runtime/
  heap` interface a **compiled Cadenza program** imports to construct/inspect its runtime values. It is a
  FROZEN, append-only, index-stable set of **90 ops** (0..=89: `box-int`, `arr-alloc`, `sum-new`, the
  CHAMP/RRB collection ops, `value-encode`, …). A program bakes a per-program import section against it;
  the interface identity + the runtime's content address (`REQUIRED_RUNTIME_HASH`) are the ABI
  (`component-abi.md §The Value-Heap Runtime`; `rcdzc/src/backend/wasm/runtime_abi.rs`).
- **Boundary B — the kernel↔reducer fold interface** (`cdz-kernel/wit/reducer.wit`): the
  `cadenza:agent-kernel/fold` world a **reducer component** exports (`apply`) and the kernel host drives
  once per event. `reducer.wit` DECLARES this structurally: `apply(content-type, option<list<u8>>,
  option<list<u8>>) -> list<effect-request>`.

**The hard-coding the operator hit is at Boundary B, not A.** Here is the exact mechanism. `reducer.wit`
_declares_ a structural signature, but a **real Cadenza reducer does not export that signature** — a
compiled Cadenza component crosses every compound as an opaque `u32` handle into the shared value-heap
runtime, so its actual export is `apply(u32, u32, u32) -> u32` (`wasm_host.rs:1335`
`call_apply_lowered`). To bridge the declared structural `reducer.wit` and the reducer's real handle-ABI
export, the kernel host stood up **`HeapHandle`** (`wasm_host.rs:920`): it composes the reducer against
the shared runtime, then **binds 16 individual value-heap ops as wasmtime `Func`s** — `box-int`,
`arr-alloc`, `arr-set`, `sum-new`, `str-new`, `arr-get`, `sum-disc`, `sum-payload`, `get-int`,
`bytes-*`, `vec-*` — and hand-marshals each `apply` argument (a content-type record, the `option<list<
u8>>` payloads) INTO heap handles the reducer consumes, then walks the returned effect-list handle back
OUT field-by-field (`wasm_host.rs:927-946` the bound-op fields; `:1000-1015` the by-name extraction;
`:1063-1320` the per-op call/read methods).

That `HeapHandle` marshalling layer is precisely "hard-coding the heap ops in here." Its problems, exactly
as the operator names them:

1. **Brittle to runtime-API change.** Every op the marshaller touches is a hand-bound `Func` pinned to a
   `runtime.wit` name; the marshalling logic encodes the value-heap's *structure* (a record is a sorted
   positional array, an option is `sum-new(disc, payload)`, …). Changing how the runtime represents any
   of these means changing the kernel-host marshaller in lockstep with the runtime — the tight coupling
   the operator wants gone.
2. **A Rust guest cannot participate.** A reducer authored in Rust (wit-bindgen) has NO value-heap
   runtime, does not export `apply(u32,u32,u32)->u32`, and cannot speak the 90-op handle ABI. The current
   fixture reducers are Rust guests that export the STRUCTURAL `reducer.wit` directly — but that path
   works ONLY because they carry no compound over the boundary as a real Cadenza handle; the moment a real
   Cadenza reducer is on one side and a Rust reducer on the other, the two "apply" shapes (structural vs
   handle) diverge and `HeapHandle` is the seam that only knows how to talk to the Cadenza side. There is
   no single `apply` shape both a Rust guest and a Cadenza guest export.

**What already exists that this design builds on (this is the crucial asset):**

- The runtime ALREADY renders a value OUT to a **canonical binary AST**: `value-encode` (runtime.wit idx
  62) walks a heap value to the `cadenza-ast` codec document (`header · leaf pool · struct table · root`),
  guided by a compiler-baked shape descriptor. This is the exact bytes a compound result crosses the host
  boundary as today (`DESIGN-recursive-sum-escape-walker.md`).
- The **`cadenza-ast` crate** is a dependency-light, spec-frozen (`spec/contracts/ast-encoding.md`),
  bijective binary codec (`cdzast\x00\x01` header, LEB128 counts, dedup leaf pool, TOTAL decode with a
  decode-bomb guard), already shared by `cadenza-syntax` + `rcdzc` + `cdz-kernel` (which even has a
  generic `ast_marshal.rs` mapping any wasmtime `Val` ↔ this same AST wire).
- `cadenza-ast` is ALSO the language's **canonical value form** (`deterministic-value-form.md`) and its
  **value-interchange** bytes (`value-interchange.md`) — one byte form for hashing, equality, a
  component's output, AND interchange. There is deliberately no second encoding.

So half of "pass a binary AST across the ABI" is already built and shipping: the OUT direction. This
design is its symmetric completion — accept a binary AST IN — which lets the whole `HeapHandle`
marshalling shim be deleted (`no-adapter/migration-layers` operator directive: full collapse).

## 2. The direction — the boundary is `bytes → bytes`, and only the runtime interprets them

**The design in one sentence:** the kernel↔reducer fold boundary (Boundary B) carries a **binary AST
(`cadenza-ast` bytes)** in both directions — the event/payload crosses in as `list<u8>` of value-form
AST, and the reducer returns its effect list as `list<u8>` of value-form AST — so the kernel host never
constructs or inspects a value-heap handle, `HeapHandle` is deleted, and **any** guest (Cadenza OR Rust)
that can decode/encode `cadenza-ast` bytes is a valid reducer.

Concretely, the reducer world's `apply` becomes a pure bytes function:

```wit
// reducer.wit — the fold export, after this design
apply: func(event: list<u8>) -> list<u8>;   // both sides are cadenza-ast value-form documents
```

(The three separate `apply` arguments — content-type, payload, resumes — fold into ONE `event` AST
document; §3a. The returned `list<u8>` is a value-form AST encoding the `list<effect-request>`.)

Who does what with the bytes:

- **A Rust guest** decodes the event bytes with `cadenza_ast::codec::decode`, folds in plain Rust, and
  encodes its effect list back. It needs `cadenza-ast` and NOTHING ELSE — no value-heap runtime, no
  90-op ABI. This is the operator's "would actually work with a rust guest."
- **A Cadenza guest** does NOT decode bytes into its own linear memory (it has none for compound values —
  the runtime owns the heap). Instead it asks the shared value-heap runtime to turn the event bytes into
  a heap value it can pattern-match, and to turn its result heap value back into bytes. That is: the
  Cadenza guest keeps composing against the runtime exactly as today, but the value that crosses the
  **kernel boundary** is bytes, and the runtime is what bridges bytes↔handle — INSIDE the guest's own
  composition, where the runtime already lives, NOT in the kernel host.

This is the key move: **the bytes↔handle marshalling does not disappear, it MOVES from the kernel host
(where it is hard-coded per-op and Rust-hostile) INTO the value-heap runtime (where it is one op the
runtime already half-implements).** The runtime is the one component that is ALLOWED to know the heap
representation — that is its entire job (`component-abi.md §The Runtime Owns The Value Heap`). The kernel,
per the standing §23 "kernel must be runtime-agnostic" directive, goes back to knowing NOTHING about the
heap: it ships opaque `list<u8>` across a boundary and never binds a heap op.

### 2a. The one new runtime op — `value-decode`, the inverse of `value-encode`

The runtime already has `value-encode(v, desc) -> bytes` (heap value → AST bytes, idx 62). This design
adds its inverse, **appended** as the next index (append-only is the frozen rule —
`value-heap-runtime.md §The Operation Set Is Index-Stable And Grows Only By Appending`):

```wit
// runtime.wit — appended (index 90), the inverse of value-encode (62)
value-decode: func(bytes: u32, desc: u32) -> u32;  // 90 — a cadenza-ast value-form Bytes handle + shape
                                                    //      descriptor → a fresh owned heap value
```

- `bytes` is a runtime `Bytes` handle holding a `cadenza-ast` value-form document; `desc` is the SAME
  compiler-baked shape descriptor `value-encode` already reads (a `Bytes` handle naming the
  tuple/record/sum/leaf shape, with a self-`Ref` closing recursion). The runtime walks the document
  guided by the descriptor and builds the heap value — the exact reverse of `value-encode`'s walk.
- It stays name-free/tag-free: every name (`:`, a variant head, `tuple`, a field name, the type name) is
  READ from the descriptor and matched against the document, never invented — same discipline as
  `value-encode`. A document that doesn't match the descriptor's shape returns `NULL` (the total-decode
  convention — a mismatch is a compiler bug, not a runtime trap; the compiler only ever bakes a
  well-formed descriptor + only ever hands bytes of the matching type).
- Ownership: CONSTRUCTS (consumes the input `Bytes`, produces a fresh owned heap value) — the constructor
  half of the consume/borrow contract, mirroring how `value-encode` is an inspector.

`value-decode` + `value-encode` are the runtime's own bytes↔handle bridge. A compiled Cadenza reducer's
`apply` becomes, in emitted terms: `value-decode(event_bytes, event_desc)` → the heap event value → fold
→ `value-encode(result, result_desc)` → the effect-list bytes. That emission is the compiler's concern
(`rcdzc`/`v-rust-backend`), and it REUSES the two runtime ops — no per-op hand-marshalling anywhere.

### 2b. Why this is more stable to API evolution (the operator's core ask)

The wire format at Boundary B is now `cadenza-ast` bytes — a **frozen, versioned, self-describing**
format (`ast-encoding.md`: a tree of `symbol applied to children`, a self-carried symbol prelude, a new
node kind is a new symbol with NO container-version bump). Contrast the two evolution stories:

- **Today:** adding/changing a value-heap concept that crosses the fold boundary means touching the
  kernel-host `HeapHandle` marshaller (a new bound `Func`, new marshalling logic), keeping it in lockstep
  with `runtime.wit`, AND it can never work with a non-Cadenza guest.
- **After:** the value-heap runtime's 90-op interface can change freely (it is Boundary A, private
  between a program and its content-pinned runtime — a runtime change already means a new
  `REQUIRED_RUNTIME_HASH` and is a program-vs-runtime concern, NOT a kernel concern). The kernel boundary
  only ever sees `cadenza-ast` bytes, whose evolution is additive-by-symbol. The kernel is decoupled from
  the heap ABI entirely — which is exactly the §23 "kernel knows nothing about the runtime" invariant the
  `HeapHandle` had quietly violated.

The self-describing property is the payoff the operator's directives repeatedly asked for (the log-format
directive: "self-describing AST, meta-inspectable by decoding"). A fold-boundary payload is now a decodable
AST document, inspectable/loggable with the same codec the log already uses.

## 3. The target shape

### 3a. `reducer.wit` — `apply` folds three args into one event AST document

Today `apply(content-type, option<list<u8>> payload, option<list<u8>> resumes) -> list<effect-request>`.
The three inputs become one `event` AST document (a value-form s-expr the guest decodes), and the result
becomes one `list<u8>` AST document:

```wit
world reducer {
  import kv;            // UNCHANGED — kv stays a host import (list<u8> keys/values already)
  export fold;
}
interface fold {
  // The whole event as ONE cadenza-ast value-form document: an s-expr carrying the content-type, the
  // optional payload, and the optional resume token as named fields, e.g.
  //   (event (content-type <ct>) (payload <bytes-or-absent>) (resumes <bytes-or-absent>))
  // Returns the requested effects as ONE value-form document: a list of effect-request records.
  apply: func(event: list<u8>) -> list<u8>;
}
```

- **Why fold the args:** a single AST document is the whole "pass a binary AST across the ABI" premise —
  the boundary carries ONE self-describing value, not a hand-split tuple whose shape the host must know.
  The guest decodes the one document; the kernel builds the one document. It also means the boundary
  signature never changes again as the event envelope grows (a new envelope field is a new named child in
  the AST, not a new WIT parameter — additive-by-symbol, §2b).
- **`content-type`, `effect-kind`, `effect-request` remain concepts**, but they are now *value-form AST
  shapes* the guest and kernel agree on (a small schema in `cadenza-ast` terms), NOT WIT records. This is
  where the marshalling that was in `HeapHandle` goes on the KERNEL side: the kernel builds the event
  document and parses the effect-list document with `cadenza-ast` (it already has `ast_marshal.rs` and the
  full codec) — pure byte work, no heap handle, no wasmtime `Func` binding.
- **`kv` is unchanged** — it already crosses `list<u8>` keys/values (`reducer.wit:92-97`); it is a host
  import the guest calls inline and never involved a heap handle.

### 3b. The kernel host — delete `HeapHandle`, ship bytes

`cdz-kernel/src/wasm_host.rs`:

- **DELETE `HeapHandle`** (`:920-947` struct, `:966-1373` all bind/call/marshal impls, the async twins
  `:1386+`) and the fold-boundary rebind that binds it (`:576` doc, the §19e rebind path). This is the
  "full collapse + delete the old thing" the operator's no-adapter-layers directive mandates — not a
  parallel path kept alive.
- `apply` is now a plain `func(list<u8>) -> list<u8>` call: hand the event bytes, get the result bytes.
  The reducer no longer exports a `(u32,u32,u32)->u32` handle ABI; both a Cadenza guest and a Rust guest
  export the SAME `(list<u8>)->list<u8>`. `call_apply_lowered` (`:1335`) collapses to a `list<u8>` call.
- **The kernel still composes a Cadenza guest against its declared deps** (the runtime, via
  `compose_dep_into_linker`, `:580`) — that machinery is the §23 generic dep resolution and STAYS. What
  goes away is the host *binding heap ops off that instance to marshal*. The guest's own emitted `apply`
  calls `value-decode`/`value-encode` on its composed runtime; the kernel just linked the dep in.
- `ast_marshal.rs` becomes the kernel's event-document builder + effect-document parser (it already maps
  WIT `Val` ↔ `cadenza-ast`; here it maps the kernel's own `EffectRequest`/`ContentType` Rust structs ↔
  the value-form AST). No heap involved.

### 3c. The Cadenza guest emission — `rcdzc` (`v-rust-backend` / `v-compiler-ml`)

A Cadenza reducer compiled by `rcdzc` today emits `apply(u32,u32,u32)->u32`. After this design it emits
`apply(list<u8>)->list<u8>`, with the body:

1. lift the incoming `list<u8>` into a runtime `Bytes` handle (the R0 `list<u8>` ABI already exists — the
   canonical `(ptr,len)`→retptr lift/lower in the resource-escape vertical, `DESIGN-value-heap-rcdzc.md
   §3a R0`);
2. `value-decode(bytes, event_desc)` → the heap event value (the compiler bakes `event_desc`, the event
   schema's shape descriptor, exactly as it already bakes `value-encode` descriptors);
3. fold — the ordinary compiled reducer body over a heap value, unchanged;
4. `value-encode(result, result_desc)` → the result `Bytes`;
5. lower that `Bytes` back to the exported `list<u8>`.

Steps 1/5 (`list<u8>`↔`Bytes`) and step 4 (`value-encode`) are ALREADY built (the compound-escape
vertical). Step 2 (`value-decode`) is the one new op (§2a). So the Cadenza-guest change is: emit a
`list<u8>`-shaped `apply` that wraps the existing fold in decode/encode, instead of a handle-shaped
`apply` the kernel host marshals around. The compiler already knows the static event/result types, so
baking the two shape descriptors is the same machinery `value-encode` uses.

### 3d. The Rust guest (`v-rust-backend` angle) — now first-class

A Rust reducer (wit-bindgen against the new `reducer.wit`) implements `apply(event: Vec<u8>) -> Vec<u8>`
by calling `cadenza_ast::codec::decode(&event)` → matching the value-form AST → building its effect list
as AST → `encode`. It depends on `cadenza-ast` (dependency-light: `num-bigint` + `unicode-normalization`)
and the `kv` import. It does NOT compose against the value-heap runtime at all — a Rust guest has no
value-heap values, so it needs no bytes↔handle bridge; it works directly in AST/Rust terms. This is the
operator's "would actually work with a rust guest," and it falls out for free once the boundary is bytes.

## 4. Migration against the frozen contracts (this is a coordinated, versioned change)

This touches frozen contracts; each change is additive OR carries a version increment + migration path,
per the constitution's Governance Floors.

- **`runtime.wit` (Boundary A) — ADDITIVE.** `value-decode` is an APPENDED op (index 90), which is
  exactly how `value-heap-runtime.md §The Operation Set … Grows Only By Appending` says the runtime
  evolves. Appending an op changes the runtime bytes → a new `REQUIRED_RUNTIME_HASH` (`xtask codegen`
  regenerates `runtime_abi.rs`; `codegen --check` gate enforces it). No existing op moves; every existing
  program is byte-unaffected. This is a one-time envelope re-derivation, the sanctioned cost.
- **`component-abi.md` (Boundary B) — VERSION INCREMENT (v6) with migration.** Today v5 says Cadenza
  components composed against a shared runtime "exchange values as HANDLES" (§Cross-Component Value
  Exchange). This design changes the KERNEL↔REDUCER fold boundary specifically to exchange values as
  **canonical value-form bytes** (`cadenza-ast`), NOT handles. The migration clause: the reducer fold
  boundary is a NEW, closed boundary (the kernel is not a "Cadenza component composed against the shared
  runtime exchanging values with a peer Cadenza component" — it is the HOST driving a reducer; the v5
  handle-exchange rule governs component↔component composition and is UNCHANGED for that case). So this is
  best framed as **additive**: it defines the fold-boundary value representation (value-form bytes) for a
  boundary that v5 did not cover, leaving the v5 cross-Cadenza-component handle exchange intact. If the
  requirement gate reads the fold boundary as already governed by v5's handle rule, then it is a v6
  increment with the migration "the fold boundary crosses value-form bytes rather than a handle; no
  deployed reducer predates this, so no artifact requires re-derivation." **OPEN DECISION D1 (§6) —
  chosen default: additive (new boundary), escalate only if the gate disagrees.**
- **`deterministic-value-form.md` / `value-interchange.md` — UNCHANGED.** The bytes crossing the boundary
  ARE the canonical value form (that is the whole point — one byte form for output, hashing, interchange;
  `value-interchange.md §Serialized Bytes Are The Canonical Value Form`). This design REUSES that contract
  rather than introducing a second encoding — it strengthens conformance (the fold boundary now uses the
  canonical form the OUT direction already used).
- **`ast-encoding.md` — UNCHANGED.** `value-decode` consumes the exact `cadenza-ast` value-form document
  `value-encode` produces; no codec/container change.
- **`reducer.wit`** is NOT a frozen constitutional contract (it is the kernel's own world), but it IS an
  ABI the guest fixtures pin — this change re-derives every reducer fixture. Since no reducer is deployed
  and the kernel + fixtures live in one repo, this is a coordinated in-repo re-derivation (rebuild the
  fixtures, re-pin the golden component bytes / validate).

## 5. Increments (each its own commit + gate; top-to-bottom, the way a vertical lands them)

**B0 — `value-decode` runtime op (`v-runtime`).** Append `value-decode` (idx 90) to `runtime.wit` + its
`cdz-runtime` impl (the inverse walk of `value-encode`, descriptor-guided, name/tag-free, NULL on
shape-mismatch). Gate: `xtask codegen` regenerates `runtime_abi.rs` with the new op + bumped
`REQUIRED_RUNTIME_HASH`; a round-trip unit test `value-decode(value-encode(v, desc), desc) == v`
(structural `value-eq`) over flat/nested/negative tuples + records + a sum + a recursive list (the
`value-encode` test corpus, run backwards). This is the foundation both guests need; it lands FIRST and
is independently useful (it completes the encode/decode symmetry the runtime was missing). **This is the
probe increment** — prove the byte↔handle bridge is a correct inverse before any boundary consumes it.

**B1 — the fold-boundary schema in `cadenza-ast` terms (`v-agent-harness`).** Define the value-form AST
shapes for the event document (`(event (content-type …) (payload …) (resumes …))`) and the effect-list
document (a list of `effect-request` records), as a small schema module in `cdz-kernel` over `cadenza-ast`
(reusing `ast_marshal.rs`). Kernel-side builder (`EffectRequest`/`ContentType` Rust → AST) + parser (AST →
Rust). Gate: round-trip unit tests (kernel builds an event doc a hand-written decode reads back; kernel
parses an effect-list doc a hand-written encode produced). NO boundary change yet — this is the byte
schema both sides will agree on, proven in isolation (the analogue of H0's structured-data-before-consumer
probe).

**B2 — flip `reducer.wit` to `apply(list<u8>)->list<u8>` + delete `HeapHandle` (`v-agent-harness`).**
Change the world; rebuild the Rust reducer fixtures against it (a Rust guest becomes trivial — decode,
fold, encode, §3d, so this is where the Rust-guest capability is PROVEN with a real fixture). Delete
`HeapHandle` + the fold-boundary rebind + `call_apply_lowered`'s handle shape; `apply` is a `list<u8>`
call. Gate: the existing `component_reducer_e2e` (currently drives a Rust fixture) passes over the bytes
boundary; a NEW e2e proves a Rust guest that only depends on `cadenza-ast` folds an event → effects with
NO value-heap runtime composed at all (the operator's headline). `HeapHandle` is gone (grep-proven).

**B3 — Cadenza-guest emission `apply(list<u8>)->list<u8>` (`v-rust-backend`/`v-compiler-ml`).** Emit the
decode/fold/encode body (§3c) from `rcdzc`: wrap the compiled fold in `value-decode(event)` /
`value-encode(result)`, exporting the bytes-shaped `apply`. Bake the event/result shape descriptors (same
machinery as `value-encode` descriptors). Gate: a real Cadenza reducer compiled by `rcdzc`, composed
against the runtime, drives through the kernel's bytes `apply` E2E — an event in as AST, effects out as
AST, matching the Rust guest's behavior on the same event. This closes the loop: BOTH guest kinds export
one boundary, and the kernel host binds ZERO heap ops.

(B0→B1 are independent and can land in parallel; B2 depends on B1 + B0; B3 depends on B0 + B2. Each is
independently green — B0/B1 don't touch the boundary, so trunk stays green until B2 flips it with fixtures
re-derived in the same commit.)

## 6. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — `component-abi.md` additive vs v6 increment (§4).** Default: treat the kernel↔reducer fold
  boundary as a NEW boundary v5 didn't cover → ADDITIVE (defines value-form-bytes for a boundary that had
  no representation). Escalate to an `ask` ONLY if the requirement gate flags it as changing v5's
  handle-exchange rule (then it's a v6 increment with the stated migration in §4). This is the one
  decision that could need the operator; it is scoped so the build (B0–B3) does not block on it — the
  code is identical either way; only the contract's version header differs.
- **D2 — one folded `event` document vs keeping `content-type` a WIT field.** Default: fold ALL THREE
  args into ONE `event` AST document (§3a) — maximally "one binary AST crosses," and future-proof (a new
  envelope field is a new AST child, never a WIT signature change). Alternative (keep `content-type` as a
  WIT enum + only payload/resumes as bytes) is REJECTED: it re-splits the boundary the operator wants
  unified and re-introduces a structural WIT shape the host must track.
- **D3 — does the Cadenza guest decode in-guest, or does the kernel pre-decode to a handle?** Default: the
  Cadenza guest calls `value-decode` ITSELF inside its composition (§2/§3c) — the kernel stays
  runtime-agnostic (§23) and ships only bytes. Alternative (kernel decodes bytes→handle and passes a
  handle) is REJECTED: it puts the heap knowledge back in the kernel host — the exact coupling being
  deleted.
- **D4 — shape-descriptor for the EVENT (the IN direction).** `value-encode` bakes a descriptor for the
  known static result type; `value-decode` needs the descriptor for the event type. Default: the compiler
  bakes BOTH (it knows the reducer's event type statically, same as it knows the result type) — symmetric
  with `value-encode`, no new mechanism. No open fork; recorded for the B3 implementer.

## 7. Watch-outs (for the implementing verticals)

- **Append-only is sacred (B0).** `value-decode` MUST be index 90 (next free) — never inserted mid-list.
  A reorder breaks every deployed program's baked import indices (`value-heap-runtime.md`). `xtask
  codegen --check` is the guard; the `REQUIRED_RUNTIME_HASH` bump is expected + correct.
- **Total decode, never trap (B0).** `value-decode` returns NULL on a shape mismatch (a compiler bug),
  never traps — mirroring `value-encode`'s "malformed descriptor → empty Bytes" and the runtime's
  total-read discipline (a null read is benign; the compiler only ever hands matching bytes). Do NOT make
  a decode failure a runtime trap.
- **`cadenza-ast` byte-stability is the structural gate (all increments).** The `cdzast\x00\x01`
  round-trip + the value-form corpus must stay green — they are the proof the wire is the canonical form.
  A fold-boundary document is a value-form document; it MUST decode with the same codec the log + output
  paths use (no bespoke second framing). Coordinate with `v-syntax` (owns `cadenza-ast`) before touching
  the codec — this design REQUIRES no codec change, only its use in a new place.
- **Delete, don't wrap (B2).** The operator's no-adapter-layers directive is explicit: `HeapHandle` and
  the handle-shaped `apply` are DELETED, not kept behind a feature flag or a compat path. The bytes
  boundary is the only boundary after B2.
- **Fixture re-derivation (B2).** Flipping `reducer.wit` re-derives every reducer fixture; rebuild +
  re-validate (or re-pin) the committed `.wasm` in the SAME commit so trunk never has a fixture that
  doesn't match the world (the CI fixture-regen-and-validate step catches drift).
- **`kv` is out of scope.** It already crosses `list<u8>` and never used a heap handle — leave it.
- **Do not touch the value-heap runtime's OTHER 89 ops.** This design adds ONE op and changes ONE kernel
  boundary; the runtime's internal ABI (Boundary A) is otherwise unchanged. The heap representation stays
  the runtime's private concern — that is what makes the kernel decoupling sound.

## 8. Verification (the gate that protects this)

- B0: `value-decode(value-encode(v, desc), desc)` structurally equals `v` for the `value-encode` corpus
  run backwards; `xtask codegen --check` green with the new op + bumped hash; existing runtime tests
  unaffected.
- B1: kernel-side event/effect AST build+parse round-trips in isolation; no boundary touched, gate
  neutral.
- B2: a Rust reducer depending ONLY on `cadenza-ast` (no value-heap runtime) folds an event → effects
  through the bytes `apply` E2E; `HeapHandle` absent (grep); existing e2e green over the new boundary;
  fixtures re-derived + validated.
- B3: a real `rcdzc`-compiled Cadenza reducer drives through the bytes `apply` E2E (event AST in, effect
  AST out), agreeing with the Rust guest on the same event.
- Throughout: `cargo test -p rcdzc --lib` 0 failed; `cargo xtask gate` additive-only (no `Todo→Fail`);
  `cargo xtask check` fmt+clippy+`codegen --check` clean; the `cadenza-ast` byte-stability corpus stays
  green (the structural proof the wire is the canonical form).

## 9. Note on perf / marshalling cost (the operator's question)

The operator asked about "the perf/marshalling cost of binary-AST-over-ABI vs typed Funcs." The honest
read: passing bytes is NOT free relative to threading a bare `u32` handle — a Cadenza guest now does a
`value-decode` walk on the way IN and a `value-encode` walk on the way OUT at the fold boundary, where
today the host threaded handles. BUT: (a) the OUT walk (`value-encode`) already happens today for any
compound result — this only adds the symmetric IN walk; (b) the fold boundary is crossed ONCE per event,
not in a hot inner loop — an agent-kernel fold is coarse-grained (an event → some effects), so a single
encode/decode per fold is negligible against the fold's own work (a model call, a shell exec); (c) the
cost BUYS the stability + the Rust-guest capability + the kernel decoupling, which is the operator's
stated priority ("a lot more stable … would actually work with a rust guest"). Where a value crosses
between two composed CADENZA components in a hot path, the v5 handle-exchange (no serialization) is
UNCHANGED — this design touches only the kernel↔reducer fold boundary, not component↔component
composition. So the marshalling cost lands exactly where it is cheapest to pay and is explicitly the
trade the operator chose.
