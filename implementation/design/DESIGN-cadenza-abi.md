# The Cadenza ABI — handle-passing boundary + host-replaceable value-runtime interfaces

**Status:** design — nothing landed. Written 2026-09-18/19 by the `design-cadenza-abi` fleet agent,
**interactively with the operator** across a long converging conversation (this doc records decisions
pinned live, not autonomous guesses). Anchors are landmarks at trunk `d3368f1af8`. Audience: the
`vertical` owner(s) who will build this (`rcdzc` backend + `cdz-runtime` + `cdz-platform` host + the
shared `etude-byterope` crate), and future me.

> **The operator's motivation (2026-09-18), verbatim on the essentials.** "The way cadenza is used in
> the hivemind platform is everything is really short lived. We make a single call and get a response
> back. Having to marshall all of this stuff and copy it across is just killing us in terms of
> performance. I want the host to be able to pass something like the bytevec without any copies at all.
> And the guest should be able to read it and do a kind of CoW for just the segment it wants to update
> … We need to make sure that a buggy program doesn't crash the host. Leaking doesn't matter a ton since
> we'll just free the scoped allocator after the function returns … track which type everything is. So
> even a UAF doesn't have the ability to confuse types, it would just result in weird guest behavior. If
> a guest tried to treat a handle as a different type it would result in a trap."

Perf-first: remove the two copies that dominate a hivemind fold — the component-model canonical
marshalling of the event/result *structure*, and the copy of the *payload bytes* — without ever letting
a buggy guest corrupt host memory, and without coupling the platform to the value representation.

---

## 0. TL;DR — two orthogonal axes; Axis 2 is host-replaceable component interfaces

- **Axis 1 — the cadenza ABI (a boundary lowering).** WIT stays the contract IDL, but gets a **second
  lowering** alongside the component-model canonical ABI: one that crosses compound values as **runtime
  handles** instead of laying records/lists out in linear memory. A compiler concern (`rcdzc`), works
  against any runtime. The shape is pinned by the **interface name whose version is a content-hash of
  the WIT shape**, so a field-add changes the name and composition rejects a skewed guest.

- **Axis 2 — host-replaceable value-runtime interfaces.** The value runtime decomposes into a small set
  of **hash-versioned component interfaces** — `cadenza:runtime/heap` (the structural value graph) and
  `cadenza:bytevec` (byte values). Each is **wasm by default** (a portable, content-addressed component)
  and **natively replaceable by the host** (the trust-whitelisted substitution): the host provides a
  native implementation of an interface it wants to accelerate, giving zero-copy + arena scoping, while
  a guest that runs on a host that *doesn't* replace it just uses the hashed wasm component and still
  works. The guest is **fully agnostic** to which implementation backs an interface. **`bytevec` is the
  first interface to make replaceable** (self-contained, biggest hivemind win); the `heap` interface is
  the harder, eventual second (§3.6).

|                              | Canonical-ABI boundary          | **Cadenza-ABI boundary (Axis 1)**                  |
|------------------------------|---------------------------------|----------------------------------------------------|
| **All-wasm interfaces**      | today                           | pass handles; runtime still self-contained wasm    |
| **Host-native `bytevec` (Axis 2, step 1)** | delegated bytes, boundary still marshals | payload zero-copy in/out                |
| **+ host-native `heap` (Axis 2, step 2)**  | —                               | **full zero-copy hot path**                        |

The unifying data structure is a **byterope** — an RRB relaxed-radix rope of byte chunks (§3.3) — which
is *the same shape* the runtime's list already is, and becomes the one type shared across the
`etude-byterope` crate, the runtime's bytes value, and the runtime list.

---

## 1. Ground truth (with anchors)

Two boundaries; this design touches both, on different axes.

- **Boundary A — the value-heap runtime interface** (`cdz-runtime/wit/runtime.wit`, ~101 ops). FROZEN,
  append-only, TAG-FREE, NAME-FREE (`runtime.wit:1–31`): the program threads opaque `u32` handles;
  the runtime stores positional products + a sum discriminant + lengths, never a universal type tag.
  Identity is `REQUIRED_RUNTIME_HASH` (`cadenza-compile-abi/src/runtime_hash.rs:17`). Allocator is talc
  over the component's own wasm linear memory (`allocator.rs`, flagged `:15-16` swappable). RC is
  Perceus `dup`/`drop` + FBIP reset/reuse (`rc.rs`); `IMMORTAL`/`mark-immortal` (ops 95/96) is the
  build-once/CoW-static primitive. Leak oracle `live-objects` (op 54, `cdz-run/src/grade.rs:189-215`).
  The **list is a 32-way radix trie + RRB relaxed nodes** (`vector.rs:3,41,44`): fanout 32, relaxed
  interior nodes carry a cumulative size table (`raw = 4·arity`), O(log₃₂) index/concat/split with
  path-copy sharing, plus a Clojure-style tail noted for amortized-O(1) push. The **bytes value is a
  rope** (`bytes_string.rs:147`) — binary concat/slice nodes, flatten-on-read.
- **Boundary B — the platform/reducer ABI** (`cdz-platform/wit/world.wit`). "Strongly typed with a
  carried payload" (`:8-14`): the envelope is typed WIT records; the one carried-as-bytes thing is
  `type payload = list<u8>` (`:50`). A **fresh instance folds each call** (`reducer.rs:19-22`); per-call
  cost is dominated by instance/linear-memory setup (**Pooling** allocator fix, `host.rs:799-846`:
  on-demand `mmap` capped fold concurrency ~8, +~26ms/req) and by **marshalling** — the payload copy +
  the `value-decode`/`value-encode` walk each fold.

The copies to remove: (1) the canonical-ABI lowering of the envelope structure (Axis 1), and (2) the
payload-bytes copy + decode (Axis 2 native `bytevec`). Also relevant: the **`etude` repo**
(`camshaft/etude`) where the `bytevec-extract` agent has extracted the current s2n-quic-dc `ByteVec`
into a crate — the intended home of the shared `etude-byterope` (§3.3).

---

## 2. Axis 1 — the cadenza ABI: a second lowering of WIT

### 2.1 WIT is the contract; the cadenza ABI is an alternate lowering of it

The component-model **canonical ABI** lowers a `record` to field offsets in linear memory, `list<T>` to
`(ptr,len)`, strings copied in — that lowering *is* the marshalling cost. WIT-the-definition and the
lowering are separable. The **cadenza ABI** keeps the identical WIT contract and lowers it a second way:
**every compound crosses as a runtime handle**, backed by an ordinary value in the composed runtime,
read/built through the runtime's accessor/constructor ops. A deterministic mapping (D1):

| WIT type                 | Cadenza-ABI lowering                                    |
|--------------------------|---------------------------------------------------------|
| `record { f0, f1, … }`   | positional runtime record; field `i` = `arr-get i`      |
| `list<T>`                | runtime RRB list; `vec-*` ops                           |
| `string`                 | a `bytevec` handle (UTF-8) — §3                          |
| `variant` / `enum`       | runtime sum `(disc, payload)`                            |
| scalar                   | boxed or inline scalar                                  |
| `list<u8>` **payload**   | a **`bytevec` handle** (borrowed zero-copy, §3)          |

The whole event — envelope + lists + payload — becomes **one runtime value tree** the host builds and
hands the guest as **one handle**; the guest folds and returns **one handle**. No canonical record/list
lowering. **The payload stays bytes** (a `bytevec`), never a typed value — it's contract-specific and
opaque, exactly as today; the win is that those bytes never copy across the boundary (§3).

### 2.2 The lowered signature erases the shape — the hash-version buys the check back

Under the canonical ABI the shape is *in* the signature (`fold(event: order) -> step`), so the component
model structurally type-checks it at composition. Under the cadenza ABI the lowered function is the
shape-erased `fold(handle) -> handle` — the structural check is gone from the signature. We recover it
through **interface identity**: the component model checks interface identity by **name + version**, so
the guest exports `cadenza-abi:orders/fold@…+<shape-hash>` where **the version is a content-hash of the
WIT shape**:

- The **name** (a distinct cadenza-ABI interface) is the ABI indicator — no side channel, no marker.
- The **`<shape-hash>` version** pins the shape. Field-add ⇒ new WIT ⇒ new hash ⇒ new name ⇒
  composition rejects an old guest. Automatic, unforgeable (a human can't forget to bump it). Same
  pattern as `cadenza:runtime/heap@0.0.0+<REQUIRED_RUNTIME_HASH>`. This is the "hash the type
  definition" idea of `DESIGN-binary-ast-abi.md`, with **WIT** as the schema language.

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
// what the guest imports/exports under the cadenza ABI — handles, hash-versioned
world reducer {
  import cadenza:runtime/heap@0.0.0+<runtime-hash>;   // structure  (host-replaceable, §3)
  import cadenza:bytevec@0.0.0+<bytevec-hash>;         // bytes      (host-replaceable, §3)
  export cadenza-abi:orders/fold@1.0.0+<shape-hash>;   // handle in / handle out; shape pinned by hash
}
```
Guest body reads the event via runtime ops **typed by the WIT** (indices/types from `order`):
```
let id       : u64      = rt.get_int(rt.field(event, 0));   // field 0 = u64
let items    : List     = rt.field(event, 1);               // field 1 = list<line-item>
let customer : ByteVec  = rt.field(event, 2);               // field 2 = string → bytevec handle
// … fold … build `step` as a runtime value, return its handle
```

---

## 3. Axis 2 — host-replaceable value-runtime interfaces

### 3.1 The runtime is a set of replaceable component interfaces; wasm default, native opt-in

The value runtime is **already** a composed, content-addressed wasm component. This design decomposes it
into a small set of **hash-versioned interfaces**, each of which the host may **replace with a native
implementation**:

- **`cadenza:runtime/heap`** — the *structural* value graph (records, sums, lists, CHAMP maps/sets;
  handles, RC, FBIP). Essentially today's runtime.
- **`cadenza:bytevec`** — *byte* values (strings, bytevecs, byte leaves). A `resource bytes` + byte ops.

For each interface: **default** = a portable wasm component (content-addressed, `@…+<hash>`); **optional
native** = the host provides the interface's functions in native code (zero-copy, arena-scoped, native
byte ops). The choice is the **trust-whitelisted substitution**: the host composes only runtime-component
hashes it trusts, and may satisfy a guest's `interface@<wasm-hash>` import with its trusted **native**
implementation instead. If the host does *not* replace it, the guest runs on the hashed **wasm**
component — portable, correct, self-contained, just not zero-copy.

Why this beats a monolithic "host substitutes the whole runtime": it's **granular** (accelerate one
interface at a time), gives **graceful degradation** (a Cadenza program runs anywhere; the fast path is a
host opt-in), keeps a **small trusted native surface per interface**, and lets each interface be
independently hash-versioned + verified. The guest is agnostic throughout — it imports "a bytevec / a
heap," never knowing whether wasm or native backs it. **This is the primary framing of Axis 2** (it
supersedes the earlier whole-runtime-substitution sketch).

The safety consequence is *simpler*: the untrusted guest is contained by the wasm sandbox and can never
reach host memory; the type tags / resource validation only make a guest bug a deterministic **trap**
rather than silent-wrong *within* the sandbox. Host-memory integrity rests on the wasm boundary + the
narrow, audited native interface surface — not on getting a large native runtime perfectly right.

### 3.2 `bytevec` — the first replaceable interface

A resource interface — one opaque `resource bytes` (a byterope, §3.3) + the byte operations:

```wit
package cadenza:bytevec;
interface bytevec {
  resource bytes {
    len:      func() -> u64;
    byte-at:  func(offset: u64) -> u8;
    slice:    func(offset: u64, len: u64) -> bytes;      // structural share, no copy
    concat:   func(other: borrow<bytes>) -> bytes;       // structural share, no copy
    cow-update: func(offset: u64, replacement: borrow<bytes>) -> bytes;  // per-chunk CoW
    eq:       func(other: borrow<bytes>) -> bool;
    hash:     func() -> u64;
    compare:  func(other: borrow<bytes>) -> s32;
    // utf8-validate / normalize / index-of / int↔bytes … as the language needs
  }
  from-list: func(bs: list<u8>) -> bytes;                // fallback constructor (non-zero-copy path)
}
```

- **Default (wasm):** a byterope implemented in wasm linear memory. Portable; used anywhere the host
  doesn't replace it. Byte ops are cross-component calls into the wasm bytevec (has call overhead — the
  correctness/portability baseline, not the fast path).
- **Native (host opt-in):** the host implements `bytes` natively. A `bytes` the host creates from the
  input payload is **borrowed zero-copy** from the host's own buffer; `slice`/`concat`/`cow-update` are
  native rope ops (no bytes moved for untouched chunks); `eq`/`hash`/`compare`/`utf8` run **natively —
  faster than wasm-lowered *and* copy-free**. The result `bytes` is a host resource the host reads
  directly. This is the operator's zero-copy payload, and the whole hivemind marshalling win.

The guest never materializes payload bytes in its own memory; it threads `bytes` handles and calls the
interface. Under the native impl **nothing is ever copied into the guest** — the operator's explicit
requirement. Any bytes the guest *produces* are host-owned too (allocated in a per-call host region,
freed wholesale on return — "leaking is fine").

### 3.3 The byterope — one RRB relaxed-radix rope, shared everywhere

A byte value is a **wide-fanout (32-way) RRB relaxed-radix rope** of chunks — *not* a binary tree and
*not* a flat buffer:

```rust
const B: usize = 32;                                 // the runtime's radix width
enum Node<L> {
    Leaf    { chunks: /* right-sized boxed slice */ [L] },
    Relaxed { children: Box<[Ref<Node<L>>]>,          // ONE right-sized alloc per node (not a fat inline array)
              cum_bytes: Box<[u32]>,                   // per-node cumulative byte-size table
              len: u32 },
}
enum Chunk { Owned(/* arena slice or Bytes */), Borrowed { src: ByteVecId, off: u32, len: u32 } }
```

- **Offset lookup is a size-table descent** — `partition_point` on `cum_bytes` to pick the child,
  O(log₃₂ n), one array scan per level, no linear walk, no flatten. (Your "list of spots.")
- **slice / concat** = rope split / join, structural sharing, zero bytes moved.
- **Per-chunk CoW**: `cow-update` splits the leaf at the range, replaces just the touched chunk with an
  `Owned` copy, rejoins; the O(log₃₂) spine is path-copied, every untouched chunk (Borrowed *or* Owned)
  is shared. A `Borrowed` chunk stays zero-copy until *it* is written.
- **Sharing needs a refcounted node ref** (`Ref` = `Arc<Node>` native / the runtime's Perceus-rc
  `Handle` in-runtime) — a `Box` (unique owner) can't share subtrees, which is the whole point of
  persistence/CoW. The **child array is one right-sized boxed slice per node** (matches the runtime's
  `handles`), not a fat inline `ArrayVec<_,32>`; each slot is a shared `Ref`.
- **RRB's tail buffer** gives amortized-O(1) append-at-tail, so the s2n-quic-dc network streaming
  pattern (push tail / consume head) doesn't regress. (Vertical: confirm the runtime's RRB tail is
  actually landed — `vector.rs:44-47` reads as "concat/split done, tail noted".)

**This is the same structure the runtime's list already is** (`vector.rs`), parameterized over
`(element, size-metric)`:

| Consumer                | element        | size metric   | node ref              |
|-------------------------|----------------|---------------|-----------------------|
| runtime `List`          | value handle   | element count | Perceus-rc `Handle`   |
| runtime bytes value     | byte chunk     | byte length   | Perceus-rc `Handle`   |
| `etude-byterope` (native/host + s2n-quic-dc) | byte chunk / `Bytes` | byte length | `Arc<Node>` / `Bytes` |

**One relaxed-radix core, three instantiations.** The **`etude-byterope` crate** (in `camshaft/etude`,
extending the extracted ByteVec) is the linchpin: because **both the wasm bytevec component's guts and
the native host impl are built from the same crate, they are observationally identical by
construction** — which is exactly what makes host substitution sound (§3.4). The current extracted
ByteVec is a flat `{head, VecDeque<Bytes>}` (chunk-index lookup, no offset random access, no borrowed
chunks); this refactors it to the byterope. **A PoC + benchmark comparing the two (streaming append/
consume, slice/concat, random offset, CoW-update) is in flight with the `bytevec-extract` agent** — if
the rope wins (or matches while adding zero-copy borrowed chunks + per-chunk CoW + O(log₃₂) lookup), it
becomes the one byte type everywhere, including extracting the runtime's own bytes/list onto the crate.

### 3.4 Safety — substitution soundness + the corruption firewall

- **Substitution is only sound if the wasm and native impls are observationally identical** (a
  bytes-keyed map must hash identically whichever backs `bytevec`, or membership + canonical form
  corrupt). The **shared `etude-byterope` crate guarantees this by construction** — there is one impl of
  `eq`/`hash`/`utf8`/canonical-form, compiled to both wasm and native. This is the load-bearing safety
  argument for the whole replaceable-interface scheme.
- **Guest bugs cannot corrupt host memory.** The untrusted guest is sandboxed; it holds only opaque
  handles/resources. For the `heap` interface a per-slot **KIND type-tag** (coarse: int/bytes/sum/array/
  champ/…; the compiler knows precise static types) **traps** on a mismatched accessor — type-tag-only,
  **no generation** (the per-call arena never frees mid-call, so a stale handle points at valid memory
  of *some* value → weird-but-bounded, and a type mismatch traps). For `bytevec` the host validates every
  `bytes` resource handle on every op (bad handle → trap). Nothing is mapped into the sandbox, so there
  is no shared region a buggy runtime could scribble; every host-memory touch is a checked call/op.

### 3.5 Provenance, RC, and CoW

- **Provenance** (`BORROWED` host input / static vs `OWNED` this-call) lives host-side under the native
  impl. CoW: mutate-in-place iff `OWNED && rc==1`, else copy — a `BORROWED` chunk always CoWs. For
  byterope this is per-chunk (§3.3).
- **RC is retained but not load-bearing for safety.** `dup`/`drop` drive the in-place-vs-CoW decision and
  keep the `live-objects` oracle meaningful; because a per-call region frees wholesale, an RC miscount
  merely leaks into the arena (benign). **Sub-decision:** in a host-native impl that *always* CoWs, per-
  node rc could be dropped entirely (arena reclaims), trading away the FBIP in-place fast path — decide
  per interface, not now.
- **CoW statics** reuse `DESIGN-static-data.md` build-once-global + `mark-immortal` (ops 95/96): statics
  built once at init into a persistent region (separate from the per-call arena), returned not
  reconstructed, immortal/BORROWED, excluded from the leak census; a mutation CoWs.

### 3.6 The `heap` (node allocator) interface — the harder, eventual second

Making `heap` host-replaceable is the same pattern but a much bigger equivalence obligation, so it comes
**after** bytevec proves the machinery: the heap contract is 101 ops entangled with RC / FBIP reuse /
CHAMP internals / the **canonical-form invariant** (equal values byte-identical, for map keys +
determinism). A native replacement must reproduce all of that observably-identically or substitution
silently corrupts. It is not conceptually different (the heap already *is* a component interface), and
the shared-crate equivalence trick (§3.4) extends to it — but the surface + the invariants make it the
second target, not the first.

### 3.7 The three coordinated knobs

| Layer                | Knob                                   | Effect                                                                 |
|----------------------|----------------------------------------|------------------------------------------------------------------------|
| Guest (compute)      | compiler `--abi cadenza`               | export the cadenza-ABI world; cross values as handles (Axis 1)         |
| Runtime interfaces   | per-interface **native impl + trust whitelist/substitution** | wasm default; host swaps in native `bytevec` (then `heap`) for the fast path |
| Value bytes          | the `etude-byterope` crate             | one byterope impl → wasm component guts **and** native host impl (equivalence by construction) |

---

## 4. Coexistence + relationship to prior designs

- **The frozen `cadenza:runtime/heap` is UNTOUCHED.** Native replacements are *alternate implementations
  of the same hash-versioned interface*, chosen by host substitution — no op reordered/removed. Both the
  all-wasm and host-native paths coexist indefinitely; `rcdzc` selects the boundary lowering (`--abi`),
  the host selects the implementations.
- **`cadenza:bytevec` is a NEW interface** carved out of the byte side of the heap (the strings/bytevecs
  that today live in `Node.raw` / the bytes rope).
- **`cdz-platform` gains a second guest world** (references/cadenza-ABI vs canonical); the host detects
  which by exported interface name (§2.2) and reads the result handle directly (no `value-encode`).
- **`DESIGN-binary-ast-abi.md`**: revises its "serialization negligible" bet for the in-process hot path
  (values cross as handles; the native `bytevec` makes the payload zero-copy). Binary-AST bytes remain
  the interop / Rust-guest / cross-machine path — coexist.
- **`DESIGN-static-data.md`** is the CoW-statics foundation (§3.5). **`DESIGN-inline-handle-tagging.md`**'s
  reader-decode-audit discipline applies to the heap KIND-tag accessors (§3.4).
- **`etude` / `ByteVec` extraction**: the shared byterope crate is the same work as extracting ByteVec to
  its own repo; both s2n-quic-dc and cadenza depend on it (preserve the Apache-2.0/Amazon header).

---

## 5. Increments (top-to-bottom, each its own commit + green; bytes-first)

Axis 1 lands first against the all-wasm runtime (isolated, kills boundary serialization); the shared
byterope + native `bytevec` land next (the payload zero-copy win); native `heap` is the eventual last.
Each ties to a measurable target (principle 3).

**A0 — cadenza-ABI lowering spec + WIT shape-hash convention.** The deterministic WIT→runtime-value
mapping (D1) + the interface-version shape-hash. *Gate:* mapping documented; hash over a WIT shape
defined; field-add-changes-hash unit test.

**A1 — cadenza-ABI lowering in `rcdzc`, all-wasm runtime.** `--abi cadenza` emits handle-in/out boundary
funcs per A0. *Gate:* emitted func passes handles (no canonical record/list lowering); differential vs
canonical on a corpus subset.

**A2 — platform references world + detect-by-export + serialization-free read-out.** *Gate:* reducer e2e
over the references world (all-wasm); bench shows boundary-serialization removed vs canonical. **Axis 1
done.**

**R0 — the shared `etude-byterope` crate: RRB byterope + PoC benchmark (with `bytevec-extract`).** Refactor
the extracted ByteVec to the byterope (§3.3), generic over `(element, node-ref)`. *Gate:* the benchmark
vs the flat ByteVec (streaming append/consume, slice/concat, random offset, CoW-update) — **decision
input**: does the rope win/match while adding the capabilities? (In flight.)

**R1 — `cadenza:bytevec` interface + wasm default component.** Define the resource interface (§3.2); build
the wasm byterope component from the crate; `rcdzc` emits string/bytes leaves as `bytevec` handles.
*Gate:* a guest constructs/reads/slices/concats bytes through the wasm component; corpus bytes/string
cases green on the wasm bytevec.

**R2 — native host `bytevec` + substitution + zero-copy payload.** Host implements `bytevec` natively from
the same crate; trust-whitelist substitution swaps it for the wasm hash; the host hands the payload as a
BORROWED byterope, reads the result directly. *Gate:* a guest runs on the native bytevec via substitution
with identical results (equivalence); the payload is never copied (assert via a host copy counter); a
bench quantifies the payload-marshal savings vs the canonical+serialize baseline.

**R3 — CoW statics on bytevec.** Persistent static region; build-once, returned; immortal/BORROWED; mutation
CoWs. *Gate:* a static evaluates once at init, same handle across calls, mutation CoWs, excluded from census.

**H0 (later) — native host `heap`.** Native implementation of the structural interface, behaviorally
identical to wasm (canonical form + RC + CHAMP), via the shared-crate equivalence trick. *Gate:* the
runtime value corpus passes identically on the native heap; full zero-copy hot path + headline bench vs
baseline. (Deferred behind R0–R3 per §3.6.)

(A* independent of R*; R2 depends on R0+R1; H0 depends on R2. Each independently green — the all-wasm path
stays intact until a guest is compiled/substituted onto a new path.)

---

## 6. Open decisions (chosen defaults; escalate only a genuine fork)

- **D1 — WIT→runtime-value mapping.** *Default:* mirror existing value shapes (positional record, RRB
  list, byterope bytes/string, boxed/inline scalars). Documented in A0.
- **D2 — how the host hands bytes in.** *Decided (2026-09-19):* NOT memory-mapping (fights wasmtime's
  single contiguous linear memory, breaks pooling, can't do multiple regions). Instead: **host owns all
  bytes; `bytes` are resource handles; native byte ops operate on host-owned byteropes; the guest never
  copies bytes in.** Multiple input regions/ByteVecs are just multiple host `bytes` resources.
- **D3 — heap KIND-tag granularity.** *Default:* coarse runtime KIND (corruption firewall); compiler
  keeps precise static types.
- **D4 — handle width.** *Default:* `u32`, no generation (type-tag-only).
- **D5 — cadenza-ABI op set.** *Default:* reuse existing heap op families (reuse `rcdzc` lowering +
  reclaim analysis); the runtime interface names stay stable (host-substitutable).
- **D6 — version string form.** *Decided:* a content-hash of the WIT shape (not arbitrary semver).
- **D7 — substitution soundness.** *Default:* guaranteed by the shared crate (one impl → wasm + native,
  identical by construction); the corpus-equivalence gate enforces it.
- **D8 — cross-component resource handles.** A record field holding a string is a `bytevec` resource
  owned by one interface, referenced from the `heap` interface. *Open:* the ownership/lifetime of a
  `bytes` handle stored inside a Node (who drops it, survival across storage). Resolve in R1.
- **D9 — always-CoW vs FBIP in-place** per native interface (§3.5). *Default:* keep RC + FBIP; revisit if
  the always-CoW simplification measures better under the wholesale-free arena.
- **D10 — byterope node ref + tail.** *Default:* `Arc<Node>` native / Perceus-`Handle` in-runtime;
  confirm the RRB tail buffer is landed (else append regresses).

---

## 7. Watch-outs

- **The shared crate is load-bearing for safety, not just DRY.** Substitution soundness *depends* on the
  wasm and native `bytevec` (later `heap`) being observationally identical; hand-maintaining two impls in
  lockstep is a footgun. One crate → two compile targets.
- **Handle-passing erases the shape from the lowered signature** — the hash-version in the interface name
  is the only thing pinning it; get D6 right.
- **The wasm-default path has cross-component call overhead** — it's the portable baseline, not the fast
  path; the native replacement is where the perf is. Benchmark both (R0/R2).
- **Don't regress the s2n-quic-dc streaming hot path** when refactoring ByteVec to a rope — RRB tail is
  the mechanism; that's exactly what R0's benchmark must catch.
- **Canonical form is sacred for `heap`** (H0): a native heap must reproduce byte-identical values or map
  keys + determinism corrupt. This is why heap is second.
- **KIND-tag trap is the heap corruption firewall — audit every reachable accessor** (the
  `DESIGN-inline-handle-tagging.md` §8 reader-decode discipline).
- **Preserve the Apache-2.0/Amazon license header** on anything derived from the s2n-quic-dc ByteVec.

---

## 8. Verification (the gate)

- A0: mapping + shape-hash + field-add-changes-hash test.
- A1: emitted func passes handles (no canonical lowering); differential vs canonical.
- A2: reducer e2e over references world (all-wasm); serialization-removed bench.
- R0: byterope-vs-flat-ByteVec benchmark (the decision input).
- R1: bytes/string corpus green on the wasm `bytevec` component.
- R2: substitution runs a guest on native `bytevec` with identical results (equivalence); payload never
  copied (counter); payload-marshal-savings bench.
- R3: static once-at-init, same handle across calls, mutation CoWs, census-excluded.
- H0: runtime value corpus identical on native `heap`; full zero-copy hot path + headline bench.
- Throughout: `dev-gate` green; `gate` additive-only (no `Todo→Fail`); `codegen --check` clean for any
  runtime-build/hash change.
