# The Cadenza ABI — handle-passing boundary + a host-arena runtime for zero-copy short-lived calls

**Status:** design — nothing landed. Written 2026-09-18 by the `design-cadenza-abi` fleet agent,
**interactively with the operator** (this doc records decisions pinned live, not autonomous guesses).
Line/file anchors are landmarks at trunk `d3368f1af8`. Audience: the `vertical` owner(s) who will
build this top-to-bottom (`rcdzc` backend + `cdz-runtime` + `cdz-platform` host), plus future me.

> **The operator's motivation (2026-09-18), verbatim on the essentials.** "The way cadenza is used in
> the hivemind platform is everything is really short lived. We make a single call and get a response
> back. Having to marshall all of this stuff and copy it across is just killing us in terms of
> performance. I want the host to be able to pass something like the bytevec without any copies at all.
> And the guest should be able to read it and do a kind of CoW for just the segment it wants to update
> … We need to make sure that a buggy program doesn't crash the host. Leaking doesn't matter a ton since
> we'll just free the scoped allocator after the function returns … track which type everything is. So
> even a UAF doesn't have the ability to confuse types, it would just result in weird guest behavior. If
> a guest tried to treat a handle as a different type it would result in a trap."

This design is **perf-first**: it removes the two copies that dominate a hivemind fold — the
component-model canonical marshalling of the event/result *structure*, and the copy of the *payload
bytes* across the boundary — while keeping a buggy guest from ever corrupting host memory.

---

## 0. TL;DR — two orthogonal axes, one interface name carries the signal

The design factors into **two independent decisions**, either landable alone, composing to the full win:

- **Axis 1 — the boundary lowering (the "cadenza ABI").** WIT stays the contract IDL, but it gets a
  **second lowering** alongside the wasm component-model canonical ABI: one that crosses values as
  **runtime handles** instead of laying records/lists out in linear memory. This is a compiler concern
  (`rcdzc`) and works against *any* runtime.
- **Axis 2 — the runtime memory model.** The value-heap runtime — already a composed, content-addressed
  wasm component — gains a **host-arena** build: it delegates allocation to a small set of unsafe host
  primitives over a **per-call arena + zero-copy shared-memory borrow of the host's input buffer**, and
  it tags every value's type so a buggy guest traps instead of corrupting. The guest is **agnostic** to
  which runtime it runs on; the host **substitutes** the trusted host-arena runtime at composition.

|                              | Canonical-ABI boundary          | **Cadenza-ABI boundary (Axis 1)**                  |
|------------------------------|---------------------------------|----------------------------------------------------|
| **Classic runtime**          | today                           | pass handles; runtime still self-heaps (no bulk zero-copy) |
| **Host-arena runtime (Axis 2)** | delegated heap; boundary still marshals | **full hivemind zero-copy hot path**          |

**The single signal that ties it together:** the guest imports/exports interfaces by
`namespace:package/iface@<version>`, and the component model checks that identity at composition. So
the **interface name** already indicates both *which contract shape* and *which ABI lowering*, and the
**version is a content-hash of the WIT shape** (`@…+<shape-hash>`) so that adding a field automatically
changes the name and composition rejects a skewed guest. No separate versioning mechanism is invented —
this is exactly how `cadenza:runtime/heap@0.0.0+<REQUIRED_RUNTIME_HASH>` already works.

---

## 1. Ground truth — the two boundaries and where the copies are (with anchors)

There are two boundaries; this design touches both, on different axes.

- **Boundary A — the value-heap runtime interface** (`cdz-runtime/wit/runtime.wit`, ~101 ops, indices
  0–100). A **FROZEN, append-only, TAG-FREE, NAME-FREE** contract (`runtime.wit:1–31`): the compiled
  program threads opaque `u32` handles; the runtime stores positional products + a sum discriminant +
  lengths, never a universal type tag; the compiler knows every static type. Identity is
  `REQUIRED_RUNTIME_HASH` (`cadenza-compile-abi/src/runtime_hash.rs:17`, `@generated` by `xtask codegen`).
  Allocator is talc over the component's own wasm linear memory (`cdz-runtime/src/allocator.rs`, flagged
  `:15-16` as swappable to "a bump/free-list of our own"). RC is Perceus `dup`/`drop` + FBIP
  reset/reuse (`cdz-runtime/src/rc.rs`); `IMMORTAL = u32::MAX` (`rc.rs:21`) + `mark-immortal`/
  `mark-immortal-deep` (ops 95/96, `rc.rs:28+`) are the existing build-once/CoW-static primitive. The
  leak oracle is `live-objects` (op 54) graded in `cdz-run/src/grade.rs:189-215`.
- **Boundary B — the platform/reducer ABI** (`cdz-platform/wit/world.wit`). "**Strongly typed with a
  carried payload**" (`world.wit:8-14`): the envelope (`message`/`response`/`notification`/`request`/
  `step`, `contract-id`, `origin`, `token`, `outcome`, `error`) is all typed WIT **records**; the one
  thing carried as bytes is `type payload = list<u8>` (`world.wit:50`), the contract's actual value,
  decoded by the guest against `contract-id`. A **fresh instance folds each call** (`reducer.rs:19-22`);
  per-call cost is dominated by instance/linear-memory setup (documented `host.rs:799-846`: on-demand
  `mmap` capped fold concurrency at ~8 and added ~26ms/req; the **Pooling** allocator fixes it) and by
  **marshalling** — the payload copy (`bytes-new`/`bytes-read`, `runtime.wit` 99/100; historically
  ~1.9µs/byte) plus the `value-decode`/`value-encode` walk (`runtime.wit` 90/62) the guest runs each fold.

**The copies this design removes:** (1) the canonical-ABI lowering of the envelope structure into linear
memory, and (2) the payload-bytes copy across the boundary and its decode. Axis 1 removes (1); Axis 2's
zero-copy ByteVec removes (2).

---

## 2. Axis 1 — the cadenza ABI: a second lowering of WIT

### 2.1 WIT is the contract; the cadenza ABI is an alternate lowering of it

Today WIT has exactly one lowering — the component-model **canonical ABI** (a `record` → field offsets
in linear memory; `list<T>` → a `(ptr,len)` into linear memory; `string` copied in). That lowering *is*
the marshalling cost. WIT-the-definition and canonical-ABI-the-lowering are separable. The **cadenza ABI**
keeps the identical WIT contract and lowers it a second way: **every compound crosses as a runtime
handle**, backed by an ordinary value in the composed runtime's heap, read/built through the runtime's
existing accessor/constructor ops. It is a *deterministic* mapping, exactly as the canonical ABI is —
just to handles instead of memory:

| WIT type                     | Cadenza-ABI lowering                                             |
|------------------------------|------------------------------------------------------------------|
| `record { f0, f1, … }`       | positional runtime record value; field `i` = `arr-get i`         |
| `list<T>`                    | runtime RRB list value; `vec-*` ops                              |
| `string`                     | runtime string (UTF-8 bytes leaf)                                |
| `variant` / `enum`           | runtime sum `(disc, payload)`                                    |
| scalar (`u64`/`bool`/…)      | boxed or inline scalar (existing `box-*`/`IMM_UNIT`)             |
| `list<u8>` **payload**       | a **ByteVec** value (§3), borrowed zero-copy from host memory    |

The whole event — envelope + lists + payload — becomes **one runtime value tree** the host builds and
hands the guest as **one handle**; the guest folds and returns **one handle**. Zero canonical record/list
lowering. (D1 pins the full mapping table; the default mirrors the runtime's existing value shapes.)

### 2.2 The lowered signature erases the shape — the hash-version buys the check back

Under the canonical ABI the shape is *in the signature* — `fold(event: order) -> step` — so the
component model **structurally type-checks it at composition**; add a field and an old guest won't link.
Under the cadenza ABI the lowered function is the shape-erased `fold(handle) -> handle` (an `i32` in/out,
a valid component function) — so the structural check is *gone from the signature*.

**We recover it through the interface identity, not a bolted-on mechanism.** The component model checks
interface identity by **name + version**, not only structural signature. So the guest exports
`cadenza-abi:orders/fold@…+<shape-hash>`, where the **version is a content-hash of the WIT shape**:

- The **name** (a distinct cadenza-ABI interface) tells the host it's the handle-passing lowering —
  this *is* the ABI indicator (Axis 1's signal); no side channel, no marker section.
- The **`<shape-hash>` version** pins the exact contract shape. A field-add changes the WIT ⇒ changes
  the hash ⇒ changes the name ⇒ **composition rejects an old guest.** Automatic and unforgeable
  (a human can't forget to bump it, unlike a hand-managed semver). Same pattern as
  `cadenza:runtime/heap@0.0.0+<REQUIRED_RUNTIME_HASH>`.

The host, holding the shape-hash, knows the exact WIT shape (a content-addressed artifact addressed by
that hash), so it *constructs* the event value and *reads* the returned value by that shape. Both sides
are pinned to one WIT; the composition check catches any skew. This is the same "hash the type
definition" idea as `DESIGN-binary-ast-abi.md`'s schema-hash, with **WIT** as the schema language.

### 2.3 Illustration

```wit
// orders.wit — THE contract (canonical WIT). Its content-hash is <shape-hash>.
package example:orders;
interface order-placed {
  record order     { id: u64, items: list<line-item>, customer: string }
  record line-item { sku: string, qty: u32 }
  fold: func(event: order) -> step;
}
```
```wit
// what the guest actually imports/exports under the cadenza ABI — handles, hash-versioned
world reducer {
  import cadenza:runtime/heap@0.0.0+<runtime-hash>;   // the value runtime (host-substitutable, §Axis 2)
  export cadenza-abi:orders/fold@1.0.0+<shape-hash>;  // handle in / handle out; shape pinned by hash
}
```
Guest body reads the event via runtime ops **typed by the WIT** (indices/types from `order`, not guessed):
```
let id       : u64  = rt.get_int(rt.field(event, 0));   // field 0 = u64
let items    : List = rt.field(event, 1);               // field 1 = list<line-item>
let customer : Str  = rt.field(event, 2);               // field 2 = string
// … fold … build `step` as a runtime value, return its handle
```

---

## 3. Axis 2 — the host-arena runtime: zero-copy, arena-scoped, type-safe

### 3.1 The runtime stays a privileged wasm component; the platform stays decoupled

The runtime is **already** a composed, content-addressed wasm component. It **stays one** — it does not
move into native host code. Instead it gains a build variant that delegates *memory* to a minimal set of
unsafe host primitives. This preserves the codebase's core invariant (§23 "the kernel/platform is
runtime-agnostic"): the platform never hard-codes the value representation, and the runtime remains an
independently **upgradable** content-addressed artifact.

The safety consequence is *simpler*, not harder: the untrusted **guest is contained by the wasm sandbox**
and can never reach host memory; the type tags only make a guest bug a deterministic **trap** rather than
silent-wrong *within* the sandbox. Host-memory integrity rests on the wasm boundary + a tiny audited
unsafe primitive surface, not on getting a large native runtime perfectly right.

```
  Guest reducer (compute, untrusted)          — cadenza-ABI world, opaque handles
      │  imports  cadenza:runtime/heap@…+<hash>
      ▼
  Runtime component (privileged, trusted, content-addressed, upgradable)
      │  owns: handle table, type tags, provenance, RC, CHAMP/rope, ByteVec, CoW-statics logic
      │  imports  cadenza:runtime/host-arena         — the NEW minimal unsafe surface
      ▼
  Host / platform (provides only generic primitives; knows nothing about value representation)
```

### 3.2 The three coordinated build knobs — one per layer

| Layer                | Knob                                   | Effect                                                                 |
|----------------------|----------------------------------------|------------------------------------------------------------------------|
| Guest (compute)      | compiler `--abi cadenza`               | export the cadenza-ABI world; cross values as handles (Axis 1)         |
| Runtime (privileged) | `cdz-runtime` feature `host-arena`     | delegate heap mgmt to host-arena primitives; type-tags + provenance; **distinct content hash** |
| Host (platform)      | detect-by-export + arena/shared-mem plumbing + trust whitelist | recognize a cadenza-ABI guest; map arena+RO input; substitute + read result directly |

### 3.3 The minimal unsafe host-primitive interface (`cadenza:runtime/host-arena`)

Kept small and stable so the runtime evolves freely above it. The host provides (all bounds-checked):

- **A per-call host-owned arena**, mapped **RW** into the runtime component's address space:
  `arena-alloc(len) -> offset`, `arena-reset()` (wholesale reclaim between calls). Value storage —
  Node/table, records, lists, owned ByteVec segments — lives here. Pooled + reused across calls
  (mirrors the existing Pooling instance allocator win, `host.rs:799-846`).
- **A zero-copy borrow of the host input buffer**, mapped **read-only** into the same address space:
  the runtime reads the input directly at real offsets — *truly* zero-copy, not accessor-copied (the
  operator's explicit choice). The input's bytes never enter a wasm-copied buffer.
- **Result extraction**: after the fold the host reads the result value directly out of the shared arena
  (structure) and reads any **borrowed** segments from its *own* input buffer — no wasm crossing for
  unchanged bytes.

**Containment:** the *only* host memory mapped into the runtime is the RW per-call arena + the RO input
region — both scratch/input, never the host's general heap. So even a buggy *runtime* is confined to
scratch (freed per call) and cannot write the input (RO) or reach the host heap.

### 3.4 Handles, type tags, provenance — the corruption firewall (operator: "type-tag only")

- **Handle = opaque `u32` = index into the runtime's per-call handle table** (not a raw pointer). Each
  slot carries a **type tag** (a coarse runtime KIND — int/bytes/string/sum/array/champ-map/champ-set/
  rational/bigint; the compiler still knows precise static types), **provenance**, and the Perceus `rc`.
- **Type-tag-only safety, no generation** (the operator's cheaper choice). Every typed accessor checks
  the KIND tag and **TRAPS on mismatch** — so a guest treating a handle as the wrong type traps, never
  corrupts. The arena **never frees mid-call**, so a stale (UAF-style) handle always points at valid
  memory of *some* value → at worst weird-but-bounded guest behavior, and a type mismatch traps. No
  generation counter is needed because there is no mid-call free to create a dangling slot.

### 3.5 Provenance + RC drive CoW (operator: keep lightweight RC)

- **Provenance:** `BORROWED` (a host-input segment or a static — immutable, must CoW) vs `OWNED`
  (allocated in this call's arena — may mutate in place).
- **RC is retained but is *not* load-bearing for safety.** `dup`/`drop` still drive the
  in-place-vs-copy decision and keep the `live-objects` leak oracle meaningful. The **CoW basis** is:
  *mutate in place iff `provenance == OWNED && rc == 1`; otherwise copy-on-write.* A `BORROWED` value
  always CoWs. Because the arena reclaims wholesale, an RC bug merely **leaks into the arena** (benign,
  freed on return) — safety comes from the arena + tags + sandbox, so RC correctness is an optimization/
  oracle concern, not a corruption risk. `drop` is effectively a bookkeeping no-op here.

### 3.6 The ByteVec — zero-copy payload with segment-level CoW

The payload stays **bytes** — specifically a **ByteVec** for hivemind — *not* a typed value. The point
is that those host bytes cross **without a copy**. A ByteVec is a rope/piece-table of segments, each
`BORROWED` (referencing the RO host input at an offset) or `OWNED` (an arena copy); it builds on the
existing bytes-rope ops (`runtime.wit` 34–36). The guest reads slices directly; to **update a segment**
it CoWs *just that segment* into an `OWNED` copy and builds a new rope that **shares the untouched
`BORROWED` segments** back with the input. On read-out the host walks the result rope and reads the
borrowed segments from its own buffer — so **end-to-end copy volume = only the bytes the guest actually
touched or produced**, never the whole payload.

> **Project item — extract `ByteVec` to a standalone GitHub repo** so both the platform and cadenza
> depend on one shared crate (same pattern as the fleet-tooling extraction). Filed to the concierge as a
> backlog lead; it is a dependency of the ByteVec increments below but does not block the design.

### 3.7 CoW statics — statics computed once, returned not reconstructed

A guest's static value is built **once at init** into a **persistent static region** (separate from the
per-call arena, surviving across calls), marked **immortal/`BORROWED`**, and *returned* on use rather
than reconstructed each call — the host tracks statics separately from per-run values (the operator's
"track static values separately from the ones created per run"). Any "mutation" of a static CoWs into the
per-call arena. This reuses `DESIGN-static-data.md`'s build-once-global + reclaim-elision and the
existing `mark-immortal`/`mark-immortal-deep` ops (95/96); statics are excluded from the `live-objects`
census (an immortal is not a leak).

### 3.8 Host-side substitution + the trust whitelist (guest stays agnostic)

The guest links the **ordinary** runtime interface and never requests unsafe APIs — it is fully agnostic
to whether it runs on the classic or the host-arena runtime, because the two are **behaviorally
identical at the `heap` interface** (same ops, same value semantics; only the allocator + safety-tags
differ). The host **substitutes**: "any guest importing classic runtime hash `X`, I satisfy with trusted
host-arena runtime `Y`." Because the host-arena runtime is *privileged* (its unsafe imports touch host
memory), the host must only ever compose runtime components whose content hash it **trusts** — so the
**trust whitelist doubles as the substitution map** (`X → Y`). It is also the **upgrade path**: build a
new host-arena runtime → add its hash → rebind guests → old and new coexist (rolling). A non-whitelisted
hash is refused composition.

---

## 4. What changes, what is untouched (coexistence)

- **The frozen `cadenza:runtime/heap` (Boundary A) is UNTOUCHED.** No op reordered/removed. The
  host-arena runtime is a **separate build** (feature `host-arena`) with its **own content hash**; it
  implements the same op semantics with a different allocator + tags. Both coexist indefinitely (operator
  decision). `rcdzc` selects the boundary lowering per compile (`--abi cadenza`); the host selects the
  runtime by substitution.
- **`cadenza:runtime/host-arena` is a NEW interface** — the minimal unsafe primitive surface (§3.3),
  imported only by the host-arena runtime, provided only by a trusting host.
- **`cdz-platform` gains a second guest world** — the references/cadenza-ABI world alongside the existing
  canonical one; the host detects which by the exported interface name (§2.2), and reads the result
  **handle directly** (no `value-encode` serialization on the hot path).
- **Relationship to `DESIGN-binary-ast-abi.md`.** That design serializes `cadenza-ast` bytes at the
  fold boundary and *bet* per-fold serialization is negligible. This ABI **revises that bet for the
  in-process hot path**: values cross as handles over the shared arena, no serialization. The binary-AST
  bytes path **remains** the interop / Rust-guest / cross-machine path — the two coexist, each where it
  is cheapest.

---

## 5. Increments (top-to-bottom, each its own commit + green; Axis-1 first, then Axis-2, then compose)

The two axes are independent, so **Axis 1 lands first against the existing classic runtime** (small,
isolated, removes boundary serialization, testable today), then Axis 2 (the runtime + host lift), then
they compose. Each increment ties to a measurable target (principle 3).

**A0 — the cadenza-ABI lowering spec + WIT shape-hash convention (`v` owner).** Write the deterministic
WIT→runtime-value mapping table (D1) and define the interface-version shape-hash. The shared spec both
the guest binding generator and the host value-builder implement from, so they cannot drift. *Gate:* the
mapping documented; a hash over a WIT shape defined; a unit test that a field-add changes the hash.
(Probe: the shared spec before any code.)

**A1 — cadenza-ABI lowering in `rcdzc`, against the CLASSIC runtime.** `--abi cadenza` emits handle-in/
handle-out boundary functions, reading/building boundary values via the existing classic runtime ops per
A0. No host-arena. *Gate:* a compiled guest crosses a structured event as a handle (no canonical record/
list lowering — grep/inspect the emitted func) and matches the canonical-ABI result on the same input
(differential over a corpus subset). Proves handle-passing in isolation.

**A2 — platform references world + detect-by-export + serialization-free read-out (classic runtime).**
`cdz-platform` gains the cadenza-ABI guest world; the host detects it by exported interface name, builds
the event value + reads the result handle via runtime accessors (no `value-encode`). *Gate:* reducer e2e
drives a cadenza-ABI guest end-to-end on the classic runtime; a bench shows the boundary-serialization
cost removed vs canonical. **Axis 1 fully landed.**

**B0 — host-arena primitives + shared-memory arena plumbing (host side).** Define
`cadenza:runtime/host-arena`; implement in `cdz-platform` via wasmtime custom/shared memory + pooling
(map RW arena + RO input; reset; result handoff). *Gate:* a trivial privileged component writes/reads the
arena + reads the RO input; host resets/reuses across calls; a bounds violation traps; the pooling floor
(`host.rs:799-846`) is preserved. (Probe: the shared-arena plumbing + containment before value logic.)

**B1 — the host-arena runtime build (`cdz-runtime` feature `host-arena`).** Swap the allocator to draw
from the host arena; add the typed handle table + KIND-tag trap + provenance; RO-input borrow path; keep
RC + `live-objects`. Distinct content hash. *Gate:* the host-arena runtime passes the runtime's own value
corpus **behaviorally identically to classic** (the substitutability requirement); a type-mismatched read
traps; `OWNED && rc==1` mutates in place else CoW.

**B2 — trust whitelist + host-side substitution.** Host composes only whitelisted runtime hashes; the
substitution map swaps `classic → host-arena` for a guest. *Gate:* a guest linked against the classic
runtime runs on the host-arena runtime via substitution with an unchanged result; a non-whitelisted hash
is refused.

**B3 — zero-copy ByteVec hand-in + segment CoW.** Host hands the input as a `BORROWED` ByteVec over the
RO region; segment CoW producing ropes that share borrows; result borrowed segments extracted from the
host's own buffer. Depends on the ByteVec crate (§3.6). *Gate:* a guest reads a host ByteVec with the
whole payload never copied (assert via a host-side copy counter), updates one segment, and the result
rope shares the untouched borrow; end-to-end copy volume = only touched/produced bytes.

**B4 — CoW statics.** Persistent static region (init-time, separate from the per-call arena); build-once,
returned not reconstructed; immortal/`BORROWED`; mutation CoWs. *Gate:* a guest with a static evaluates it
once at init and returns the *same* static handle across calls; a mutation CoWs into the arena;
`live-objects` excludes statics.

**C0 — compose: the full hivemind hot path (Axis 1 × Axis 2) + the headline bench.** cadenza-ABI guest +
host-arena runtime + substitution + zero-copy ByteVec, end-to-end through the pure `run` path
(`run.rs:120`). *Gate:* the **headline bench** quantifies per-call marshal + payload-copy + allocation
savings vs the canonical + classic + serialize baseline (the measured target); differential correctness
vs baseline on a corpus subset.

(A0/A1/A2 are independent of B*; B0/B1 are independent of A*; C0 depends on A2 + B1..B4. Each increment is
independently green — the classic path stays intact until a guest is explicitly compiled/substituted onto
the new path.)

---

## 6. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — the WIT→runtime-value lowering mapping.** *Default:* mirror the runtime's existing value shapes
  (positional record, RRB list, CHAMP map/set, rope bytes/string ByteVec, boxed/inline scalars),
  documented in A0. No new runtime representations.
- **D2 — the shared-memory mechanism.** *Default:* a host-backed custom linear memory (wasmtime
  `MemoryCreator`) for the RW arena + a read-only mapping for the input; whether that is multi-memory vs
  a single memory with a reserved RO region is a B0 implementer call. **This is the trickiest plumbing —
  flagged for the B0 owner.**
- **D3 — type-tag granularity.** *Default:* a coarse runtime KIND tag (enough for the corruption
  firewall); the compiler retains precise static types. Not a full per-type tag.
- **D4 — handle width.** *Default:* `u32` index into the per-call table, **no generation** (type-tag-only,
  operator decision).
- **D5 — the cadenza-ABI op set.** *Default:* reuse the existing heap op families for reading/building
  boundary values (so `rcdzc` lowering + reclaim analysis are reused); the runtime interface the guest
  imports stays `heap` (host-substitutable). The ABI difference is the **boundary lowering** (Axis 1),
  not a new guest-facing op set. `cadenza:runtime/host-arena` is the only genuinely new interface.
- **D6 — the version string form.** *Decided (operator, 2026-09-18):* a **content-hash of the WIT shape**
  in the interface version, like `REQUIRED_RUNTIME_HASH` — not an arbitrary semver.
- **D7 — substitution soundness.** *Default:* the whitelist entry asserts behavioral equivalence, and the
  B1 gate enforces it (the runtime's shared value corpus passes identically for classic + host-arena).
  Trust is operator-curated.

---

## 7. Watch-outs (for the implementing verticals)

- **Handle-passing erases the shape from the lowered signature.** The hash-version in the interface name
  is the *only* thing pinning the shape at composition — get D6 right or a field-add slips past undetected.
- **The host-arena runtime MUST stay behaviorally identical to classic at the `heap` interface**, or
  substitution is unsound. The corpus-equivalence gate (B1) is the guard; do not let the tags/allocator
  change any observable value semantics.
- **Only the RW arena + RO input are mapped into the runtime** — never the host's general heap. This is
  the containment property; a leak here is a host-memory-safety hole.
- **The KIND-tag trap is the corruption firewall — audit every reachable accessor.** A single unguarded
  deref that can see a mis-tagged handle is a hole (same discipline as `DESIGN-inline-handle-tagging.md`'s
  reader-decode audit; that doc's §8 checklist applies to the tagged host-arena handles too).
- **RC is an optimization + oracle here, not safety.** Keep `dup`/`drop` emission for the CoW decision and
  `live-objects`, but a miscount only leaks into the arena — don't gate host-memory safety on it.
- **`ByteVec` extraction is a cross-repo dependency** for B3 — coordinate the split; both platform and
  cadenza depend on the extracted crate.
- **The frozen runtime + append-only rule is untouched** — the host-arena runtime is a *separate build/
  hash*, not an edit to `cadenza:runtime/heap`. Do not reorder or mutate the existing 101 ops.

---

## 8. Verification (the gate that protects this)

- A0: mapping documented + shape-hash defined + field-add-changes-hash unit test.
- A1: emitted cadenza-ABI func passes handles (no canonical record/list lowering); differential vs
  canonical on a corpus subset (`cargo test -p rcdzc --lib`; scoped `corpus-gate-coarse-*`).
- A2: reducer e2e on the classic runtime over the references world; boundary-serialization-removed bench.
- B0: shared-arena write/read/reset/reuse + bounds-trap + pooling-floor preserved.
- B1: host-arena runtime passes the value corpus behaviorally identically to classic; type-mismatch traps;
  in-place-vs-CoW correct.
- B2: substitution runs a classic-linked guest on host-arena unchanged; non-whitelisted refused.
- B3: whole-payload-never-copied assertion + segment-CoW rope-sharing; touched-only copy volume.
- B4: static evaluated once, same handle across calls, mutation CoWs, statics excluded from census.
- C0: headline savings bench vs baseline + differential correctness.
- Throughout: `cargo xtask dev-gate` green; `cargo xtask gate` additive-only (no `Todo→Fail`);
  `codegen --check` clean for any runtime-build change (the host-arena runtime has its own generated hash).
