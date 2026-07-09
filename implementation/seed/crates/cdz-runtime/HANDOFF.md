# Runtime handoff — you own the whole value-heap runtime story

You are the **runtime engineer** for Cadenza's value-heap runtime. You own the runtime component
end to end: its efficient implementation now, reference counting (Phase D), and persistent
collections (Phase E). A separate engineer owns the **compiler** (`cdz-compiler/`) and drives this
runtime through a frozen interface. This document is everything you need to work independently and
concurrently, without ever colliding with the compiler engineer.

Read it fully before touching code. When something here disagrees with older comments in the repo,
this document wins for the runtime; ask (via the coordination channel) before crossing a boundary.

---

## 1. What the runtime is

Cadenza is a pure-functional language whose compiler emits WebAssembly **components**. Scalars
(int/bool/float) cross the component boundary directly, but **compound values** (tuples, records,
lists, sums, bytes, strings, maps) live in a shared **runtime component** that the emitted program
*imports* and the host *composes* in. The program threads **opaque `u32` handles** (indices into
the runtime's value store — the program never dereferences them) between constructors, and reads
values back through accessors.

Two design pillars, both load-bearing — do not erode them:

- **NAME-FREE.** A record is a positional product (a tuple; field names are compile-time indices).
  A sum is a `(discriminant, payload)`. The runtime holds **no** field or variant names.
- **TAG-FREE.** There is **no** per-object type tag. Cadenza has no type erasure — generics
  monomorphize, so the compiler knows the exact static type at every use site and never asks the
  runtime "what is this?" (`get-int` is only ever emitted where the static type says Int). The
  compiler bakes a *type-directed renderer* into each program that walks a value of KNOWN shape.

The runtime therefore stores raw payloads plus exactly two pieces of **genuine runtime data** —
an array's element count and a sum's variant discriminant — neither of which is a universal type
tag. Rendering, naming, normalization, ordering, and equality semantics are all the **compiler's**
job, not yours.

---

## 2. Files you own (and the ones you must NOT touch)

**You own, exclusively:**
- `implementation/seed/crates/cdz-runtime/src/lib.rs` — the entire runtime implementation.
- `implementation/seed/crates/cdz-runtime/Cargo.toml` — e.g. to add an allocator dependency.
- `implementation/seed/crates/cdz-runtime/src/*.rs` — any new modules you add under this crate.
- The generated `implementation/seed/crates/cdz-runtime/src/bindings.rs` is gitignored and
  regenerated from the WIT by `cargo component`; delete it if it goes stale and let it regenerate.

**You must NOT touch (these belong to the compiler engineer):**
- `implementation/seed/crates/cdz-compiler/` — the compiler.
- `implementation/seed/crates/cadenza-seed/` — the host + CLI.
- `implementation/seed/crates/cdz-compiler-component/` — the compiler-as-component.
- The corpus (`spec/semantics/*.sexp`) and anything under `spec/`.
- **`implementation/seed/crates/cdz-runtime/wit/runtime.wit` — FROZEN (see §3).**

`implementation/` is gitignored; **do not commit anything.** The operator handles commits.

---

## 3. The one hard boundary: the WIT is FROZEN

`cdz-runtime/wit/runtime.wit` defines the interface `cadenza:runtime/heap`. The compiler bakes each
import's **index** (0–25, in declaration order) into every emitted program's fixed component
envelope. Therefore:

> **You may NOT change the function set, order, names, or signatures in the WIT.**
> Not a rename, not a reorder, not an added param, not a removed op. Any of those silently
> mis-links every compiled program (the program calls import #7 expecting `arr-set`, etc.).

You change the *implementation behind* the interface freely — that's the whole point. Optimizing
the internal representation is entirely behind the opaque-handle boundary and changes **zero**
emitted program bytes. If you believe an op is genuinely missing for Phase D/E, do **not** edit the
WIT unilaterally — new ops are **append-only** and each one costs the compiler engineer a one-time
component-envelope re-derivation, so raise it on the coordination channel first and let them
append + re-derive in lockstep.

The 26 functions (indices 0–25), for reference — the WIT is authoritative:

```
0  box-int(s64)->u32       1  get-int(u32)->s64
2  box-bool(bool)->u32     3  get-bool(u32)->bool
4  box-float(f64)->u32     5  get-float(u32)->f64
6  arr-alloc(len:u32)->u32 7  arr-set(arr,index,elem)->u32(=arr)  8 arr-get(arr,index)->u32  9 arr-len(arr)->u32
10 sum-new(disc,payload)->u32  11 sum-disc(u32)->u32  12 sum-payload(u32)->u32
13 bytes-alloc(len)->u32   14 bytes-set(buf,index,value)->u32(=buf)  15 bytes-get(buf,index)->u32  16 bytes-len(buf)->u32
17 str-new(string)->u32    18 str-get(u32)->string
19 dup(u32)                20 drop(u32)
21 map-alloc(len)->u32     22 map-set(m,index,key,value)->u32(=m)  23 map-key(m,index)->u32  24 map-val(m,index)->u32  25 map-len(m)->u32
```

`arr-*` is the ONE positional shape backing tuple **and** record **and** list. A map is a distinct
type (dynamic keys) stored as positional (key,value) pairs, verbatim, no sort/dedup.

---

## 4. The second boundary: observable semantics are a contract

The compiler assumes each op behaves exactly as it does today. Any optimization MUST preserve:

- **Construct → read-back round-trips** for every value type (the native tests below encode this).
- **`arr-set`/`bytes-set`/`map-set` return the container handle** (used for threading).
- **Handles stay stable and opaque** for the lifetime the program uses them (until `drop` in
  Phase D). The program may read a handle many times and pass it to `dup`/`drop`.
- **`dup`/`drop` remain no-ops until Phase D.** When you implement RC, they may reclaim — but the
  compiler's call sites are already emitted expecting the RC calling convention (see §6).
- **`get-*` on a type mismatch returns a benign default, never panics/traps** — a mismatch is a
  compiler bug, not a runtime-checked condition. Keep this defensive behavior.
- **Strings stored verbatim** — no normalization or scalar-counting in the runtime.

The 16 native round-trip tests already in `lib.rs` (`#[cfg(test)] mod tests`) are the behavioral
contract. **Keep them green at every step, and do not weaken them.** Add tests as you add
machinery; never delete a round-trip assertion to make an optimization pass.

---

## 5. Where the implementation is now (your starting point)

`src/lib.rs` today:
- `enum Value { Int(i64), Bool(bool), Float(f64), Arr(Vec<u32>), Sum{disc:u32,payload:u32},
  Bytes(Vec<u8>), Str(String), Map(Vec<(u32,u32)>) }`.
- `thread_local! { static HEAP: RefCell<Vec<Value>> }` — an **index-table** heap: `intern` pushes
  and returns the index as the handle. **It only grows; it never reclaims.** This is deliberate
  interim debt (bump-only, Phase-C).
- Plain-Rust core ops (`op_box_int`, `op_arr_alloc`, `op_sum_new`, `op_bytes_set`, `op_str_new`,
  `op_map_set`, …) that native `cargo test` exercises directly.
- A `#[cfg(target_arch = "wasm32")]` `Guest` impl that thinly wraps the core (kebab→snake).
- Tests include a `render()` mirror driven by a static `Shape` descriptor, proving the accessors
  suffice to render **without** a tag. Preserve this — it's the design's proof.

The current build is verified: 16 tests pass; `cargo component build --release --target
wasm32-unknown-unknown` produces `target/wasm32-unknown-unknown/release/cdz_runtime.wasm`.

---

## 6. Your roadmap: optimize → RC (Phase D) → persistent collections (Phase E)

You own this entire arc. Land each phase independently; keep tests + the wasm build green
throughout.

### Phase C.opt — an efficient embedded allocator (do this first)
Replace the `Vec` index-table with a real allocator. The operator's explicit ask: *"the runtime
should ship its own embedded allocator … using a big Vec is going to be quite inefficient."*
- Options: a `no_std`-friendly Rust allocator (e.g. `dlmalloc`, `wee_alloc`, `talc`) as the global
  allocator, with `Value` nodes as real heap allocations addressed by pointer/offset handles; OR a
  hand-rolled size-classed free-list over a bump region (see Phase D — you'll want free lists
  anyway). Either is fine; the handle stays an opaque `u32`.
- The runtime crate builds `wasm32-unknown-unknown`, `panic=abort`, `no_std`-leaning,
  `opt-level="s"`, `lto=true` (see `Cargo.toml`). Keep it small and deterministic.
- **Constraint:** handles must remain opaque `u32` and semantics unchanged (§4). Do not leak
  representation into the interface.

### Phase D — Perceus precise reference counting (the reclamation story)
The design research (25 primary sources, adversarially verified) is summarized below; follow it.
- **Core result:** for an ACYCLIC IMMUTABLE value heap, precise RC (Perceus) is a COMPLETE
  reclamation discipline — **no tracing/cycle collector needed.** Cadenza has no mutable references
  at all (stricter than Koka), so the guarantee is unconditional **as long as no lazy
  self-reference / knot-tying is ever introduced.** Preserve acyclicity as an invariant.
- **Make `dup`/`drop` real:** `dup` = increment refcount; `drop` = decrement, and at zero,
  recursively drop children then free. The compiler already emits the calling convention (currently
  they no-op end-to-end; the compiler wires precise call sites in coordination with you).
- **Object header:** 1 count word + size-class info, size-class aligned so reuse pairing matches
  sizes; fields inline.
- **Reuse (FBIP):** reset/reuse + drop specialization — a dead pattern-matched object paired with a
  same-size allocation in a branch reuses the address when unique. The single predicate
  `count==1 along the whole root path` authorizes every in-place reuse (= Rust `Rc::make_mut`).
- ⚠ **Use DROP-GUIDED / FRAME-LIMITED reuse, NOT algorithm D; NO unrestricted borrow inference.**
  Naive reuse/borrow are not frame-limited → arbitrary peak-heap blowup (Frame-Limited Reuse,
  ICFP'22). This is THE constraint that makes the spec's "bounded/accountable allocation" hold.
- **Allocator:** NON-ATOMIC counts (single-threaded → avoids the atomic penalty) + per-size-class
  free lists over a bump region.
- ⚠ **Watch the O(n) synchronous free CASCADE** when a large unique structure's last ref drops —
  it's a real concern for the language's determinism-and-fuel accounting. Note/measure it.
- Borrowing ONLY for read-only positions, provably frame-limited (RC-free inspection).
- **Verify:** a recursion/loop that builds many compounds runs with **bounded peak heap** (add a
  probe/test asserting the allocation high-water mark is bounded across iterations).

### Phase E — persistent collections
Once collection *operations* arrive (later milestones), replace the flat `Arr`/`Map` reps:
- **CHAMP** for maps/sets (OOPSLA'15: 52–64% smaller than Scala's HAMT).
- **32-way radix / RRB-tree** for vectors/lists (RRB only if O(log N) concat/split is needed).
- Path-copying structural sharing needs **no special RC** — a shared subtree just carries count>1
  and is freed when the last version drops. This composes with Phase D for free.
- Finger trees only with lazy spine suspension (else amortized bounds degrade under sharing).
- This is likely gated behind the compiler adding collection ops; coordinate before starting.

Full research synthesis lives in the operator's memory note
`rc-heap-persistent-ds-sota-2026-07-05` (recs P1–P10); the above is the actionable digest.

---

## 7. How to build and verify (do ALL before reporting a phase done)

Prefix every shell command with `export PATH="$HOME/.cargo/bin:$PATH"` (cargo is not on the
default PATH here).

```sh
cd /Users/bythewc/Projects/camshaft/cadenza/implementation/seed/crates/cdz-runtime
export PATH="$HOME/.cargo/bin:$PATH"

# 1. Native tests (the behavioral contract) — must stay green.
cargo test --release 2>&1 | tail -30

# 2. The wasm COMPONENT builds against the frozen WIT (regenerates bindings, proves your Guest
#    impl still matches all 26 signatures). This is the artifact the host composes.
cargo component build --release --target wasm32-unknown-unknown 2>&1 | tail -30
#    Artifact: target/wasm32-unknown-unknown/release/cdz_runtime.wasm

# 3. Report the artifact size + hash so the compiler engineer can pin it.
shasum -a 256 target/wasm32-unknown-unknown/release/cdz_runtime.wasm
```

If `cargo component build` fails because the `Guest` trait signature doesn't match, fix your
`Guest` impl — **never** the WIT.

---

## 8. Coordination protocol (how we stay collision-free)

- **No source-file overlap.** You edit only files under `cdz-runtime/`; the compiler engineer edits
  only `cdz-compiler/` + `cadenza-seed/` + corpus. There is no file both of us write.
- **Shared artifact, not a source conflict:** we both cause a rebuild of
  `target/…/cdz_runtime.wasm`. The only care needed: don't be surprised if the file's timestamp
  moves under you. When you finish a phase, **post the new sha256** (step 7.3) so the compiler
  engineer re-pins the runtime for integration and re-runs the behavior gate.
- **The WIT is the treaty.** If Phase D/E genuinely needs a new op, propose it on the coordination
  channel; the compiler engineer appends it to the WIT and re-derives the envelope, then you
  implement it. Never edit the WIT yourself.
- **Report back** at each phase: (a) what changed in the representation, (b) native test count +
  result, (c) the component built + artifact path + sha256, (d) any performance numbers or the
  free-cascade / peak-heap measurements for Phase D, (e) anything that pushed against a boundary.

That's the whole story. Optimize hard, keep the interface still, keep the tests green.
