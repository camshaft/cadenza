# DESIGN: static allocation + reuse of constant literal data structures (allocate once, reuse forever)

Status: READY-TO-BUILD (2026-08-26, design-static-data). This is an **implementation-shaping** doc,
NOT a fresh design: the approach is the operator's own pre-existing mandate **§2d STATIC compound
values** in `DESIGN-value-heap-rcdzc.md:165-234` (operator mandate 2026-07-11). This doc confirms §2d
Option A is still the shape, records the operator's settled decisions for the FIRST vertical
(bytes-first), pins the precise gap + the dead scaffolding a builder activates, and hands off ownership.

Operator decisions settled (via concierge, 2026-08-26 — "go with recommendations there"):
1. **Scope of first vertical = bytes-only.** String next, then list/record, then map.
2. **Mechanism = hybrid** — data-segment for bytes+string; lazy-build-once-cached-global for compounds.
3. **Immortal rep = compile-time elision** — where the compiler PROVES a value static, emit NO
   dup/drop (mirrors the existing `IMM_UNIT` RC-noop). v-runtime owns the final rep pick + the sharp
   edge (a static value embedded in an OWNED struct later recursively dropped must NOT be freed).

## The problem (confirmed current behavior)

Every constant/literal data structure used as a *runtime value that escapes as a whole value* is
**reconstructed from scratch on every evaluation** — a fresh refcounted heap handle each time. No
hoisting, interning, or constant pool for in-body literals. Verified in the rcdzc wasm backend
(`implementation/seed/crates/rcdzc/src/backend/wasm/`) and runtime:

| literal kind  | emit arm (select.rs)        | per-evaluation runtime construction                                   |
|---------------|-----------------------------|-----------------------------------------------------------------------|
| **bytes**     | `Core::BytesOf` 7864–7895   | `bytes-alloc(len)` then a `bytes-set` per byte                        |
| string        | `Core::ConstStr` 7586–7596  | `bytes-alloc` + a `bytes-set` per UTF-8 byte (when it must be a handle)|
| list `[…]`    | `Core::ListNew` 7797–7818   | `arr-alloc` + boxed `arr-set` per elem + one `vec-of-arr`             |
| tuple         | 7746–7788                   | `arr-alloc` + boxed set per element                                   |
| map           | `Core::MapNew` 8631+        | `map-empty` + a consuming `map-insert` per entry                     |
| record        | `Core::Record` 7681–7739    | `arr-alloc` (record IS a tuple at runtime) + boxed set per field     |

`op_bytes_alloc` (`cdz-runtime/src/lib.rs:3527-3542`) allocates a fresh heap node per call. So a
function `fn f() = process(b"\x00\x01\x02")` called a million times allocates and fills the same 3-byte
buffer a million times.

### The narrow hoists that ALREADY work (do NOT redo these)
The gap is precisely a fully-constant literal that **escapes as a whole value**. Three constant paths
are already handled and are out of scope:
1. A whole-export nullary constant baked via `constant_value_form` (`lower.rs:14180-14188`) → a data
   segment at the component boundary.
2. Host-call constant string/bytes args materialized in a data segment (`layout.rs:102-111`).
3. A constant tuple that is ONLY projected (never escapes as a whole value) emits zero heap ops
   (`is_constant_compound` `lower.rs:14157-14169`).

### Perceus today has no immortal value
Standard retain/release (`OP_DUP` `select.rs:307`, `OP_DROP` frees at rc 0). The only refcount-free
value is the `IMM_UNIT` inline immediate (value `2`, `runtime_abi.rs:124`) — a compile-time sentinel,
not a heap constant. §2d's answer is to make static-ness a COMPILE-TIME fact and elide dup/drop, not a
runtime tag.

## The design = §2d Option A (build-once global), bytes-first

Per `DESIGN-value-heap-rcdzc.md:184-198` Option A (preferred): build each distinct static value ONCE in
a `start`/init region, store the handle in a module **global**, and `global.get` it at every use. Cost
moves from per-call to once-per-instance. This reuses the ordinary construction ops (no new runtime op),
and a shared static (the same literal written twice) is ONE global (interning / CSE over constants).

Per the operator's hybrid decision, for the leaf byte-payload kinds (bytes + string) the payload is a
flat contiguous byte run that maps to a wasm `(data …)` segment — the build-once init can `memory.init`
/ materialize from the segment rather than re-emit a `bytes-set` chain. Compounds (list/map/record),
whose runtime layout is boxed/hashed, use the general Option-A build-once (emit the existing builder
ONCE in the init region, cache the handle in a global).

### Reclamation = compile-time elision (§2d point 4, operator-approved)
Where the compiler PROVES a value is the build-once root of a static (`DESIGN-value-heap-rcdzc.md:206-219`):
the Perceus pass EMITS NO dup/drop for it. A build-once global is a persistent root that is never
reclaimed, and accessors borrow, so both calls are simply omitted. Compile-time knowledge → zero runtime
cost and NO ABI change. NOT a runtime static-tag (a sticky/saturating refcount would cost a per-call
branch AND grow the runtime ABI speculatively — rejected by §2b/§2d). This is exactly the
"immortal-under-refcounting" requirement the operator flagged.

### The sharp edge — v-runtime owns this (the one genuinely-open point)
Compile-time dup/drop elision is clean while the static value is only ever a **borrowed operand** or the
**build-once root** (returned, passed as a borrowed arg, projected). The sharp edge: a static value
STORED as an OWNED CHILD of a runtime-built reclaimable structure (e.g. a static `b"…"` inserted into a
runtime list/map). When that parent is dropped, the runtime's recursive drop would decrement the static
child's refcount and free it → use-after-free on the next evaluation.

For the **bytes-only first vertical** this is narrow (a bytes literal most commonly escapes as a
whole value, a return, or a borrowed arg — the clean case), but the embedding case must be RULED before
generalizing to compounds. Candidate resolutions for v-runtime to pick:
- (a) **Borrowed-only proof.** Only route a static to the build-once path where the compiler proves it
  never flows into an owned reclaimable structure; otherwise fall back to per-eval construction. Purely
  compile-time, no ABI change, but conservative (declines the embedding case).
- (b) **Recursive-drop skips a static child.** The runtime recognizes a static/immortal handle (a bit /
  a reserved rep) so recursive drop of an owned parent does NOT free a static child. This is a runtime
  distinction (mild rep touch) — the price of admitting the embedding case — and revisits §2d point 4's
  "no runtime tag" specifically for the embedded-child sub-case.
- (c) **Dup-on-store.** Emit a dup when a static is stored into an owned structure so the parent's drop
  decrements a count the build-once global still holds a ref above — keeps it alive, at the cost of the
  static no longer being strictly refcount-free when embedded.
**Default for the bytes-first vertical:** (a) — borrowed-only, decline the embedding case — proves the
mechanism with zero ABI risk; v-runtime rules on (b)/(c) before compounds generalize.

## Dead scaffolding to ACTIVATE (already present, zero producers today)
The build-once-global machinery is stubbed but never driven — activating it IS the core of the build:
- `Lir::GlobalGet(u32)` (`backend/wasm/lir.rs:146`), `Lir::GlobalSet(u32)` (`lir.rs:149`) — defined +
  serializable (`serialize.rs:236-240`), but **ZERO producers** of `GlobalSet`/`GlobalGet`.
- Core-wasm opcodes `GLOBAL_GET = 0x23`, `GLOBAL_SET = 0x24` (`wasm_abi.rs:59-60`).
- `CORE_SEC_GLOBAL = 0x06` (`wasm_abi.rs:166`, comment: "a build-once static compound's handle global")
  and `CORE_SEC_START = 0x08` (`wasm_abi.rs:170`, comment: "the init function that builds each static
  compound once") — section ids documented for exactly this, but **no GLOBAL or START section is ever
  written**.
- Gating note `lower.rs:14086-14087`: the build-once-GLOBAL path "activates with the first escape path
  — the renderer."

## Increments (top-to-bottom, bytes-first)
1. **Constancy detection for bytes.** Flag a `Core::BytesOf` whose every element is a constant byte AND
   which escapes as a whole value (not projected-only) as a static-once candidate; prove borrowed-only
   (default (a)). Emit unchanged; detect + log. Gate: byte-neutral.
2. **Emit the GLOBAL + START sections.** Teach the renderer/serializer to write a `CORE_SEC_GLOBAL`
   section (one global per distinct static bytes) and a `CORE_SEC_START` init function that builds each
   once; produce `Lir::GlobalSet` in the init and `Lir::GlobalGet` at each use site.
3. **Route constant-bytes uses to `global.get`** + elide dup/drop for the proven-static handle (§2d pt4).
   For bytes, materialize the payload from a `(data …)` segment in the init (hybrid mechanism).
4. **Interning** identical constant bytes → one global (CSE over the canonical value form).
5. **String** (same flat-byte-payload shape as bytes).
6. **List / tuple / record** (general Option-A build-once) — gated on v-runtime's embedding ruling.
7. **Map** (last — resolve freezing a hashed/ordered structure).

## Gate
- Byte-neutrality on the corpus (`gate --opt-sweep --target wasm`, 0-divergence): reuse must not change
  any observable output.
- A targeted corpus case that evaluates a constant bytes literal MANY times (a loop / repeated call) and
  asserts the value each time — run under `cdz-run` on wasm so a real value executes — proving reuse
  neither corrupts nor frees the shared allocation (the use-after-free this design prevents), and that
  the per-call `bytes-alloc` is GONE (assert the function body only reads the global, per §2d:232-234).
- A heap-count / leak probe: zero net alloc growth across N evaluations (no leak) and the static handle
  never freed (no UAF).
- Self-host (`cdz test implementation/compiler-ml`) stays green.

## Ownership recommendation (for the concierge's hand-off)
This is implementing a pre-existing design with dead scaffolding already present, spanning emit +
runtime-rep. Recommend a **NEW dedicated vertical `v-static-data`** owning it top-to-bottom (detection →
GLOBAL/START emit → dup/drop elision → the bytes corpus gate), coordinating the two seams rather than
splitting ownership: the Perceus-elision + embedding-ruling seam with **v-runtime**, and the
build-once-emit / GLOBAL-START-section seam with **v-wasm-opt/v-core-opt** (sharing-aware-emit owners).
A dedicated owner is cleaner than a split because increments 1-3 are one tight emit+rep loop that would
otherwise thrash across two owners.
