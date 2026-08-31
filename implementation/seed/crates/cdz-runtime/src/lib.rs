//! The Cadenza value-heap runtime (M2), compiled to a wasm **component** exporting the
//! `cadenza:runtime/heap` interface. The emitted program component imports this interface and the
//! host composes the two; the program threads OPAQUE `u32` handles between the constructors and
//! reads values back through the accessors, never dereferencing them itself.
//!
//! # Model — a genuinely tagless, per-node heap
//! A value is an immutable, reference-counted node in an acyclic heap. Each node is its OWN
//! allocation (`Box<Node>`) — there is no central value table; a handle is just the node's address.
//! A node stores exactly three things:
//!
//! ```text
//! Node { rc, handles: [Handle], raw: [u8] }
//! ```
//!
//! - `rc` — the Perceus refcount (RC needs it).
//! - `handles` — the child handles. Its LENGTH is the "scan count": how many words the free cascade
//!   must recurse into. This is genuine runtime layout data, NOT a type tag — a variable-length
//!   node (list/map) can only learn its own length by carrying it, because a runtime-driven `drop`
//!   reaches a nested node transitively with no caller in scope to hand the length in.
//! - `raw` — packed scalar bits / a sum discriminant / a byte buffer / UTF-8 string bytes.
//!
//! There is **no discriminant / kind field**. `Int`, `Bool`, `Float`, `Bytes`, and `Str` all share
//! the identical descriptor (`handles` empty); a 2-tuple, a 2-element list, and a 2-field record are
//! byte-identical (`handles` length 2). The runtime cannot tell those apart — and never needs to,
//! because Cadenza has no type erasure: the COMPILER holds the exact static type at every use site
//! and only ever emits `get-int` where the type says Int. `rc + handles.len() + raw.len()` is the
//! irreducible per-node state; storing type identity would be a tag, and we store none:
//!
//= spec/contracts/component-abi.md#the-runtime-does-not-name-or-render-values
//# The value-heap runtime MUST NOT hold a value's TYPE as a per-value tag, so that — because the language has no type erasure and the compiler therefore knows a value's static type at every use site — the runtime stores only structure and data (a product's elements, a sum's variant discriminant, a leaf's payload) and never a type identity a reader would dispatch on.
//!
//! The runtime OWNS every value's storage — each node is its own `Box<Node>` allocation with its own
//! `rc`, laid out and reclaimed entirely here; the program holds only opaque `u32` handles it threads
//! between the constructors and accessors and never dereferences, so the node layout is the runtime's
//! private concern and may change without re-deriving any program. The typed WIT functions (`arr-alloc`
//! + `arr-set` to construct a compound, `arr-get`/`sum-payload` to read a component by position) are the
//! only interface — a program builds and takes apart values through them, never by reaching into memory:
//= spec/contracts/component-abi.md#the-runtime-owns-the-value-heap-and-its-representation
//# The value-heap runtime MUST own the entire storage of a program's runtime values — their allocation, their in-memory layout, their reference-count discipline, and their reclamation — so that a program component holds no value storage of its own and the representation of every compound value is the runtime's private concern.
//= spec/contracts/component-abi.md#the-runtime-owns-the-value-heap-and-its-representation
//# The internal representation a value has within the runtime MUST NOT be observable across the runtime boundary, so that the runtime may change how it lays out, shares, counts, or reclaims a value without altering any program's observable behavior or requiring a program to be re-derived.
//= spec/contracts/component-abi.md#the-runtime-owns-the-value-heap-and-its-representation
//# The runtime MUST expose the operations that construct a compound value from its parts and that read a component out of a compound value by position, so that a program builds and takes apart its values entirely through the interface and never by reaching into the runtime's memory.
//!
//! Because the heap is acyclic (values immutable, recursion via code not heap back-edges) a
//! reference-count discipline is a COMPLETE reclamation strategy — no tracing/cycle collector:
//!
//= spec/capabilities/memory-and-resource-model.md#the-value-heap-is-acyclic
//# The heap of values a program forms at runtime MUST be acyclic, because a value's contents are fixed when it is created and no operation mutates an existing value to refer to a value created later.
//!
//! # Three decouplings (all load-bearing — keep them)
//! - **Tagless.** No per-object type tag (see above). The typed WIT functions (`box-int`,
//!   `arr-alloc`, …) are typed ENTRY POINTS, not stored metadata: `box-int` and `bytes-alloc`
//!   produce physically same-shaped nodes differing only in content.
//! - **Core is `Handle`-typed, not `u32`-typed.** Every operation — including RC and the persistent
//!   collections — is written and tested against the internal `Handle` (a node
//!   pointer). The `u32` public handle is a lossless narrowing that exists ONLY at the WIT `Guest`
//!   boundary (`Handle::to_u32`/`from_u32`, wasm32 only — pointers are 32-bit there). RC and
//!   CHAMP/RRB are thus developed and unit-tested natively without touching wasm or `u32`.
//! - **Core is allocator-agnostic.** The heap core only ever uses `Box`/`Vec`, i.e. the global
//!   allocator. The embedded wasm allocator (talc) lives behind `#[global_allocator]` in the
//!   isolated `allocator` module, so swapping allocators changes zero core code.
//!
//! # Totality and traps
//! The runtime is total in correct operation and never needs a fallible primitive: map key-lookup
//! and the total-or-trap `.at` bounds checks are compiler-emitted (lookup returns a compiler-built
//! `Option` sum; `.at` emits the sign-aware bounds check + spec-kinded trap). Two runtime behaviors:
//! - **Scalar reads and null-handle reads are TOTAL** — never trap. A `get-*` on a mismatched node
//!   reinterprets bytes (deterministic garbage, a compiler bug, but no trap); a null handle yields a
//!   benign default so a stray null never faults linear memory.
//! - **An out-of-bounds index into a VALID node TRAPS** (operator decision: fail-fast). This can
//!   only happen if the compiler violated an invariant it guarantees (static tuple/record arity;
//!   compiler-emitted list/bytes/string bounds checks; the renderer indexing only `0..len`). We trap
//!   rather than return a silent wrong value the differential gate might miss.
//!
//! # Name-free rendering
//! The runtime does NOT name or render values: a record is a positional product (field names are
//! compile-time indices) and a sum is a `(discriminant, payload)`. Rendering to canonical
//! s-expression text is type-directed code the COMPILER emits into the program, walking a value of
//! KNOWN shape through these accessors and baking every name/keyword as a constant
//! (component-abi.md §The Runtime Does Not Name Or Render Values). The `render` mirror in the tests
//! drives exactly that walk off a static `Shape`, proving the accessors suffice WITHOUT a tag.
//!
//! Authored in Rust for maintainability; to be re-authored in Cadenza at M8/M9 self-hosting.

// `no_std` in the SHIPPED build, plain `std` under `cargo test`.
//
// WHY no_std: the runtime's wasm bytes — and thus its content hash (`REQUIRED_RUNTIME_HASH`, pinned
// by every compiled program) — must be byte-identical on every builder. The build rebuilds the std
// library from source (`build-std`, see `.cargo/config.toml`) so the panic machinery can be compiled
// out; but building the FULL `std` from source lays its functions out in a host-architecture-
// dependent ORDER (x86_64 vs aarch64 emit the same code in a different sequence), reintroducing
// nondeterminism. `core` + `alloc` do NOT have that problem, so the runtime is `no_std` and links
// only those two — the crate never needed `std` (it allocates through `Box`/`Vec` = `alloc`, and its
// one global allocator is talc on wasm; see `allocator`).
//
// Gated `not(test)` so the NATIVE test suite keeps `std` (HashMap/BTreeMap reference oracles,
// `catch_unwind`, `Instant`) and stays trivially runnable with a plain `cargo test` — exactly as the
// `allocator` module is `cfg(target_arch = "wasm32")`. The shipped/hashed artifact is the wasm build,
// which is `not(test)`, so this is the form whose determinism matters.
#![cfg_attr(not(test), no_std)]

// `alloc` is always available (the heap core is built on `Box`/`Vec`); bring it in explicitly since
// there is no `std` prelude re-export under `no_std`.
extern crate alloc;

// Pull the allocation types + macros the core uses pervasively into scope, so the ~260 `Vec`/`Box`/
// `vec!` sites read the same under `no_std` as they did under `std`'s prelude.
use alloc::boxed::Box;
use alloc::string::String;
// `ToString` for `&Rc<str> -> String` in the `ast-decode` leaf rebuild (no_std has no std prelude).
use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
// `format!` is used by `DocBuilder::float_leaf` (the f64 → exact-decimal conversion for a `KIND_FLOAT`
// value-form leaf reads the shortest round-tripping text via `{:e}`) and by test/reference-oracle code.
use alloc::format;

// The runtime's embedded allocator (talc), isolated so it is swappable in one file. Only present in
// the wasm build; native `cargo test` uses the system allocator.
#[cfg(target_arch = "wasm32")]
mod allocator;

// The generated component bindings + the WIT `Guest` impl exist only in the wasm component build.
// The heap core below is plain Rust over `Handle`, so native `cargo test` exercises it directly.
#[cfg(target_arch = "wasm32")]
#[allow(warnings)]
mod bindings;

#[cfg(target_arch = "wasm32")]
use bindings::exports::cadenza::runtime::heap::Guest;

// Arbitrary-precision signed integers (DESIGN-bigint-and-rational-rcdzc.md §5). A pure `no_std` limb
// library, not yet wired to any WIT op — `#[allow(dead_code)]` so it is DCE'd from the shipped wasm
// (hash-neutral) until the `bigint-*` ops land. Independently unit-tested (differential vs `num-bigint`,
// a dev-dependency) as the safety net for the hand-written arithmetic.
#[allow(dead_code)]
mod bigint;

// The shared canonical cadenza-AST binary codec — the SAME `ast`/`leb128`/`codec` source files rcdzc
// compiles, pulled in with `#[path]` (`copy-don't-depend` shared SOURCE, NOT a crate dependency: a
// crate dep would enter cross-crate LTO and perturb the frozen runtime wasm hash — the #459 lesson).
// `#[allow(dead_code)]` so the whole codec is DCE'd from the shipped wasm (hash-neutral) until the
// `ast-encode`/`ast-decode` heap ops call it; those ops walk a heap `Ast` value into `ast::Arenas` and
// `codec::encode` it BYTE-IDENTICALLY to the compiler's compile-time `Ast.encode` const fold (and the
// inverse for decode), so one serializer source guarantees the runtime and const forms agree.
// Source-included from the SINGLE `cadenza-ast` crate (formerly rcdzc's copies, now deleted —
// consolidated to one source of truth). `#[path]` source-include, NOT a crate dependency, is
// DELIBERATE (the #459 lesson, see above): a crate dep would enter cross-crate LTO and perturb the
// frozen runtime wasm hash, which must be a function of THIS crate's own compilation only. cadenza-ast's
// `ast.rs`/`codec.rs`/`leb128.rs` are the `no_std`+alloc CORE (canon/fxhash/std behind `cfg(feature =
// "std")`), so they include standalone here exactly as rcdzc's copies did, byte-identically.
// Re-exported from the `cadenza-ast` CRATE (default-features = false → its no_std+alloc codec core),
// replacing the former `#[path]`-into-a-sibling-crate's-src source-include (operator seq-273). The
// `crate::ast::` / `crate::codec::` / `crate::leb128::` call sites below are unchanged. `#[allow(unused_imports)]`
// because not every re-exported module is named directly (codec/ast reach leb128 through cadenza-ast's
// own internal `crate::` paths, resolved inside cadenza-ast now — a cleaner boundary than the include).
#[allow(unused_imports)]
pub(crate) use cadenza_ast::{ast, codec, leb128};

/// A single-threaded stand-in for `std::thread_local!`, so the two scratch/counter cells work under
/// `no_std` (the shipped wasm build) without pulling in `std`. A component instance is
/// single-threaded, so a plain `static` behind an `UnsafeCell` is sound: there is no other thread to
/// race, and neither cell's `.with(...)` closure re-enters the same cell (documented at each use).
/// The `.with(|&T| ...)` shape mirrors `thread_local!`, so the call sites are unchanged.
///
/// Under `test` this is still a real `std::thread_local!` — the native suite is multi-threaded (the
/// test harness), so it must NOT share one static across threads.
#[cfg(not(test))]
struct SingleThreadCell<T>(core::cell::UnsafeCell<T>);
// SAFETY: only ever used in the single-threaded wasm runtime; see the type doc.
#[cfg(not(test))]
unsafe impl<T> Sync for SingleThreadCell<T> {}
#[cfg(not(test))]
impl<T> SingleThreadCell<T> {
    const fn new(v: T) -> Self {
        SingleThreadCell(core::cell::UnsafeCell::new(v))
    }
    fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        // SAFETY: single-threaded; the closure does not re-enter this cell (see call sites).
        f(unsafe { &*self.0.get() })
    }
}

/// Declare a single-threaded thread-local-shaped static: a real `thread_local!` under `test`, a
/// `SingleThreadCell` in the shipped `no_std` build. Both expose `NAME.with(|v| …)`.
macro_rules! runtime_local {
    ($(#[$m:meta])* static $name:ident : $ty:ty = $init:expr;) => {
        #[cfg(test)]
        std::thread_local! { $(#[$m])* static $name: $ty = const { $init }; }
        #[cfg(not(test))]
        $(#[$m])* static $name: SingleThreadCell<$ty> = SingleThreadCell::new($init);
    };
    ($(#[$m:meta])* pub(crate) static $name:ident : $ty:ty = $init:expr;) => {
        #[cfg(test)]
        std::thread_local! { $(#[$m])* pub(crate) static $name: $ty = const { $init }; }
        #[cfg(not(test))]
        $(#[$m])* pub(crate) static $name: SingleThreadCell<$ty> = SingleThreadCell::new($init);
    };
}

/// A heap node: a Perceus refcount header, the child handles, and the packed raw payload. Each node
/// is an independent allocation; a `Handle` is its address. There is NO kind/discriminant field —
/// the node's Cadenza type is compile-time knowledge the compiler holds, never stored here.
struct Node {
    /// Perceus refcount — NON-ATOMIC (a component instance is single-threaded, so we avoid the
    /// 1.5–2× atomic penalty). Born at 1 (the constructor's returned reference); `dup` increments,
    /// `drop` decrements, and at 0 the node is freed. The heap is acyclic (immutable values,
    /// recursion via code not heap back-edges) so this is a COMPLETE reclamation discipline.
    rc: u32,
    /// The child handles this node owns. Its length is the free cascade's scan count. Empty for
    /// scalars/bytes/strings; the elements for an array (tuple/record/list); `[payload]` for a sum;
    /// `[k0, v0, k1, v1, …]` for a map. This is the ONE positional shape backing every compound —
    /// the tuple-vs-list-vs-record distinction and a map's key ordering are compile-time / language
    /// knowledge the runtime does not hold. Stored as `Handles` (a newtype over `Vec<Handle>` today),
    /// which `Deref`s to `&[Handle]` so reads are storage-transparent — the intended home for the
    /// inline-or-spill representation (a `[Handle; 2]` inline arm for the overwhelmingly common ≤2-child
    /// nodes: tuples, sums, `[k,v]` pairs) once the method surface below has fully replaced direct `Vec`
    /// access at the mutation sites.
    handles: Handles,
    /// Packed raw payload: a scalar's little-endian bits, a sum's little-endian discriminant, a
    /// byte buffer, or a string's UTF-8 bytes. Empty for pure-compound nodes (array/map). Read back
    /// by reinterpretation — the compiler's static type says how to read it. Stored as `Raw`, which
    /// inlines the common ≤`INLINE_RAW_CAP`-byte payload (scalars, sum discs, CHAMP headers, vec
    /// headers — the overwhelming majority) with NO heap Vec, spilling to the heap only for longer
    /// bytes/strings. This is storage-transparent: `Raw` derefs to `&[u8]`, so the tagless byte-hash
    /// (`champ_hash`/`champ_eq`/`champ_key_cmp`) and every reader see the identical bytes regardless.
    raw: Raw,
    /// DEBUG-only liveness GUARD for use-after-free + wild-handle detection (native tests /
    /// `debug-counters`) — kept SEPARATE from `rc` so the refcount stays a pure count. It holds one of
    /// three ADDRESS-DERIVED sentinels (see [`live_guard`] / [`freed_guard`]), so the check is 3-way:
    ///   * `== live_guard(ptr)`  → a live cell (set at `alloc`);
    ///   * `== freed_guard(ptr)` → a FREED cell (set on free, which RETAINS the cell rather than
    ///     deallocating, so the address is never recycled and a stale handle always lands here) →
    ///     use-after-free;
    ///   * anything else → the handle is NOT a live node (fabricated / uninitialized / wild pointer).
    /// Deriving the sentinel from the node's own address means a garbage handle would need the memory it
    /// points at to equal `f(that_address)` to pass — astronomically unlikely — so we catch fabricated
    /// handles too, not just freed ones. ABSENT from the shipped build — the field does not exist, so the
    /// release `Node` layout (and `REQUIRED_RUNTIME_HASH`) is byte-unchanged; only `DEBUG_RUNTIME_HASH`
    /// moves. This is the operator's UAF safety net for the leak-reclaim work ("UAF is much worse than
    /// leaks" — an unsound reclaim drop fails the corpus/heap run loudly rather than shipping silently).
    #[cfg(any(test, feature = "debug-counters"))]
    guard: u32,
    /// DEBUG-only monotonic identity for rc-trace (leak attribution): unique PER ALLOC, so a reused
    /// cell address gets a fresh id and "alloc'd but never freed" is unambiguous. ABSENT from the
    /// shipped build — like `guard`, this keeps the release `Node` layout + `REQUIRED_RUNTIME_HASH`
    /// byte-unchanged; only `DEBUG_RUNTIME_HASH` moves.
    #[cfg(any(test, feature = "debug-counters"))]
    node_id: u32,
}

/// The ADDRESS-DERIVED "this cell is a LIVE node" sentinel (debug builds only). Mixing the node's own
/// address into the magic makes each live cell's guard unique, so a fabricated/wild handle whose target
/// memory holds some arbitrary value is overwhelmingly unlikely to match — that is how we distinguish a
/// real node from a plausible-looking garbage pointer. See [`Node::guard`].
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn live_guard(ptr: *const Node) -> u32 {
    (ptr as usize as u32).wrapping_mul(0x9E37_79B1) ^ 0x1122_3344
}

/// The ADDRESS-DERIVED "this cell was FREED" sentinel (debug builds only) — same address mixing, a
/// DISTINCT salt, so a freed cell reads neither `live_guard` nor a generic value. See [`Node::guard`].
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn freed_guard(ptr: *const Node) -> u32 {
    (ptr as usize as u32).wrapping_mul(0x9E37_79B1) ^ 0x5566_7788
}

/// Assert (debug builds only) that `ptr` is a LIVE node before it is accessed, with a 3-way diagnostic:
/// a `freed_guard` match is a use-after-free; any other non-live value is a fabricated/wild handle.
/// `ctx` names the access site. No-op + `cfg`-absent in the shipped build.
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn assert_node_live(ptr: *const Node, guard: u32, ctx: &str) {
    if guard == live_guard(ptr) {
        return;
    }
    if guard == freed_guard(ptr) {
        panic!("use-after-free: {ctx} touched a freed heap node");
    }
    panic!(
        "invalid heap handle: {ctx} touched a pointer that is not a live node (fabricated/uninitialized)"
    );
}

/// The inline capacity of a `Raw`'s payload. Sized to `CHAMP_HEADER_SIZE` (12) — the largest raw a hot
/// node carries (a CHAMP node's `[datamap][nodemap][size]`); a scalar is ≤8, a sum disc 4, a vec
/// header 8, so all of those inline too. Bytes/strings longer than this spill to the heap.
const INLINE_RAW_CAP: usize = 12;

/// A node's raw payload: inline for the common ≤`INLINE_RAW_CAP`-byte case (no heap allocation),
/// heap-backed for longer bytes/strings. Reads go through `Deref<Target = [u8]>` so it is a drop-in
/// for `&[u8]` everywhere the old `Vec<u8>` was borrowed — the byte-hash, comparisons, and every
/// `read_*`/`champ_*` accessor are storage-transparent. Writes use the explicit methods below, which
/// mirror the `Vec` surface the runtime used (`clear`/`extend_from_slice`/`resize` + in-place patches
/// via `as_mut_slice`), transparently promoting inline→heap if a write would exceed the inline cap.
enum Raw {
    Inline { len: u8, buf: [u8; INLINE_RAW_CAP] },
    Heap(Vec<u8>),
}

impl Raw {
    /// Build an INLINE `Raw` directly from `bytes` (≤`INLINE_RAW_CAP`) — no heap Vec. For the small
    /// scalar/disc/header constructors that would otherwise allocate a transient `Vec` just for `alloc`
    /// to copy inline and drop. Caller guarantees `bytes.len() <= INLINE_RAW_CAP` (scalars are ≤8, a
    /// sum disc 4, a header 12); a longer slice would truncate, so it's only used where that holds.
    fn inline(bytes: &[u8]) -> Raw {
        let mut buf = [0u8; INLINE_RAW_CAP];
        buf[..bytes.len()].copy_from_slice(bytes);
        Raw::Inline {
            len: bytes.len() as u8,
            buf,
        }
    }
    /// Build a `Raw` from a BORROWED slice of ANY length: inline when it fits the cap (no heap alloc — the
    /// common short-string/short-key case), else copy to a heap `Vec`. Unlike `From<Vec<u8>>` this needs no
    /// caller-side `Vec` for the short case, so a value-encode `Str`/`Bytes` leaf built from a node's raw
    /// slice allocates NOTHING when short (was a `to_vec` per leaf).
    fn from_slice(bytes: &[u8]) -> Raw {
        if bytes.len() <= INLINE_RAW_CAP {
            Raw::inline(bytes)
        } else {
            Raw::Heap(bytes.to_vec())
        }
    }
    fn as_slice(&self) -> &[u8] {
        match self {
            Raw::Inline { len, buf } => &buf[..*len as usize],
            Raw::Heap(v) => v,
        }
    }
    /// Empty the payload. Mirrors `Vec::clear` — a HEAP buffer is emptied but KEEPS its capacity (so a
    /// clear-then-refill, e.g. the per-step cursor `raw` rebuild in `champ_become_cursor`, reuses the
    /// allocation instead of reallocating every time); an inline buffer just resets its length. (We do
    /// NOT re-inline a heap buffer on clear: a node that once spilled is refilled to a similar size, so
    /// keeping the heap capacity is the right call — re-inlining would reallocate on the next refill.)
    fn clear(&mut self) {
        match self {
            Raw::Inline { len, .. } => *len = 0,
            Raw::Heap(v) => v.clear(),
        }
    }
    /// Append `bytes`, promoting inline→heap if the total would exceed the inline cap. Mirrors
    /// `Vec::extend_from_slice`.
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        match self {
            Raw::Inline { len, buf } => {
                let cur = *len as usize;
                if cur + bytes.len() <= INLINE_RAW_CAP {
                    buf[cur..cur + bytes.len()].copy_from_slice(bytes);
                    *len = (cur + bytes.len()) as u8;
                } else {
                    // Spill: materialize the current inline bytes + the new ones into a heap Vec.
                    let mut v = Vec::with_capacity(cur + bytes.len());
                    v.extend_from_slice(&buf[..cur]);
                    v.extend_from_slice(bytes);
                    *self = Raw::Heap(v);
                }
            }
            Raw::Heap(v) => v.extend_from_slice(bytes),
        }
    }
    /// Resize to `new_len`, filling new bytes with `fill` (only ever grows a short/absent header to
    /// `CHAMP_HEADER_SIZE` in practice). Mirrors `Vec::resize`.
    fn resize(&mut self, new_len: usize, fill: u8) {
        if new_len <= INLINE_RAW_CAP {
            // Fits inline: rebuild an inline buffer of `new_len` from the current bytes (truncate or
            // pad with `fill`), releasing any heap spill.
            let cur = self.as_slice();
            let keep = cur.len().min(new_len);
            let mut buf = [fill; INLINE_RAW_CAP];
            buf[..keep].copy_from_slice(&cur[..keep]);
            *self = Raw::Inline {
                len: new_len as u8,
                buf,
            };
        } else {
            let mut v = self.as_slice().to_vec();
            v.resize(new_len, fill);
            *self = Raw::Heap(v);
        }
    }
    /// A mutable slice of the payload, for the in-place `raw[a..b].copy_from_slice(...)` header patches.
    /// The length is unchanged (the patches only overwrite existing bytes), so no inline↔heap flip.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Raw::Inline { len, buf } => &mut buf[..*len as usize],
            Raw::Heap(v) => v,
        }
    }
    fn len(&self) -> usize {
        match self {
            Raw::Inline { len, .. } => *len as usize,
            Raw::Heap(v) => v.len(),
        }
    }
}

impl From<Vec<u8>> for Raw {
    /// Build a `Raw` from a freshly-constructed byte vector (the `alloc` boundary): inline it when it
    /// fits the cap (the common case — the Vec is then dropped, unallocated-away), else keep the heap
    /// buffer verbatim (no copy).
    fn from(v: Vec<u8>) -> Raw {
        if v.len() <= INLINE_RAW_CAP {
            let mut buf = [0u8; INLINE_RAW_CAP];
            buf[..v.len()].copy_from_slice(&v);
            Raw::Inline {
                len: v.len() as u8,
                buf,
            }
        } else {
            Raw::Heap(v)
        }
    }
}

impl Clone for Raw {
    fn clone(&self) -> Raw {
        // `.raw.clone()` sites want an owned copy of the bytes — re-derive via `From` so a small heap
        // buffer clones back to inline (and a large one stays heap). Keeps clones inline when possible.
        Raw::from(self.as_slice().to_vec())
    }
}

impl core::ops::Deref for Raw {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Inline capacity of the `Handles::Inline` arm — the ≤2-child nodes that dominate (tuples/records of
/// small arity, sums `[payload]`, CHAMP `[k,v]` data entries) carry their children INLINE in the `Node`
/// with no separate heap `Vec`; wider nodes spill to `Heap`.
const INLINE_HANDLES_CAP: usize = 2;

/// A node's child-handle vector, inline-or-spill: ≤`INLINE_HANDLES_CAP` children live INLINE in the
/// `Node` (no heap `Vec`), more spill to a heap `Vec`. Mirrors `Raw`: `Deref<Target=[Handle]>` makes all
/// READS storage-transparent; every MUTATION goes through an explicit method that grows inline→heap on
/// demand. `take()` returns an OWNED `Handles` (a move — NEVER materializes a Vec for an inline node, or
/// it would re-add a heap alloc to the 0-alloc FBIP paths — the iter-23 trap).
enum Handles {
    Inline {
        buf: [Handle; INLINE_HANDLES_CAP],
        len: u8,
    },
    Heap(Vec<Handle>),
}

impl Default for Handles {
    #[inline]
    fn default() -> Handles {
        Handles::new()
    }
}

impl Handles {
    #[inline]
    fn new() -> Handles {
        Handles::Inline {
            buf: [Handle::NULL; INLINE_HANDLES_CAP],
            len: 0,
        }
    }
    /// Build a ≤`INLINE_HANDLES_CAP`-element `Handles` INLINE from a slice — no heap Vec. The direct
    /// construction path for the small terminal nodes (sum `[payload]`, tuple/`[k,v]` of arity ≤2) so
    /// they carry their children inline with only the `Node` box allocated (the inline-handles WIN).
    /// Caller guarantees `hs.len() <= INLINE_HANDLES_CAP`.
    #[inline]
    fn inline_from(hs: &[Handle]) -> Handles {
        let mut buf = [Handle::NULL; INLINE_HANDLES_CAP];
        buf[..hs.len()].copy_from_slice(hs);
        Handles::Inline {
            buf,
            len: hs.len() as u8,
        }
    }
    /// An inline `Handles` of `len` NULL slots (≤ cap) — `op_arr_alloc`'s direct path for a small
    /// tuple/record before its slots are filled by `op_arr_set`. No heap Vec.
    #[inline]
    fn inline_nulls(len: usize) -> Handles {
        Handles::Inline {
            buf: [Handle::NULL; INLINE_HANDLES_CAP],
            len: len as u8,
        }
    }
    #[inline]
    fn as_slice(&self) -> &[Handle] {
        match self {
            Handles::Inline { buf, len } => &buf[..*len as usize],
            Handles::Heap(v) => v,
        }
    }
    #[inline]
    fn as_mut_slice(&mut self) -> &mut [Handle] {
        match self {
            Handles::Inline { buf, len } => &mut buf[..*len as usize],
            Handles::Heap(v) => v,
        }
    }
    #[inline]
    fn len(&self) -> usize {
        match self {
            Handles::Inline { len, .. } => *len as usize,
            Handles::Heap(v) => v.len(),
        }
    }
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Promote an inline arm to a heap `Vec` (copying the current elements). The spill point.
    #[inline]
    fn spill(&mut self) -> &mut Vec<Handle> {
        if let Handles::Inline { buf, len } = self {
            let v = buf[..*len as usize].to_vec();
            *self = Handles::Heap(v);
        }
        match self {
            Handles::Heap(v) => v,
            Handles::Inline { .. } => unreachable!("just spilled"),
        }
    }
    /// Append one handle, spilling inline→heap if it would exceed the inline capacity.
    #[inline]
    fn push(&mut self, h: Handle) {
        match self {
            Handles::Inline { buf, len } if (*len as usize) < INLINE_HANDLES_CAP => {
                buf[*len as usize] = h;
                *len += 1;
            }
            Handles::Heap(v) => v.push(h),
            Handles::Inline { .. } => self.spill().push(h),
        }
    }
    /// Set slot `i` in place (length-preserving; caller-checked in bounds).
    #[inline]
    fn set(&mut self, i: usize, h: Handle) {
        self.as_mut_slice()[i] = h;
    }
    /// A mutable ref to slot `i`, or `None` if OOB (length-preserving).
    #[inline]
    fn get_mut(&mut self, i: usize) -> Option<&mut Handle> {
        self.as_mut_slice().get_mut(i)
    }
    /// Empty it, keeping the arm (Inline → len 0; Heap → keep the buffer, capacity retained).
    #[inline]
    fn clear(&mut self) {
        match self {
            Handles::Inline { len, .. } => *len = 0,
            Handles::Heap(v) => v.clear(),
        }
    }
    /// Resize to `len`, filling new slots with `fill`; spills inline→heap past the inline cap.
    #[inline]
    fn resize(&mut self, len: usize, fill: Handle) {
        if len <= INLINE_HANDLES_CAP {
            if let Handles::Inline { buf, len: cur } = self {
                let old = *cur as usize;
                for slot in buf.iter_mut().take(len).skip(old) {
                    *slot = fill;
                }
                *cur = len as u8;
                return;
            }
        }
        self.spill().resize(len, fill);
    }
    /// Insert `h` at `i`, shifting the tail right; spills inline→heap past the inline cap.
    #[inline]
    fn insert(&mut self, i: usize, h: Handle) {
        match self {
            Handles::Inline { buf, len } if (*len as usize) < INLINE_HANDLES_CAP => {
                let mut j = *len as usize;
                while j > i {
                    buf[j] = buf[j - 1];
                    j -= 1;
                }
                buf[i] = h;
                *len += 1;
            }
            Handles::Heap(v) => v.insert(i, h),
            Handles::Inline { .. } => self.spill().insert(i, h),
        }
    }
    /// Remove `count` slots starting at `start`, shifting the tail left (length-preserving arm). The
    /// removed handles are DISCARDED (the caller has already relocated/dropped their references).
    #[inline]
    fn drain_range(&mut self, start: usize, count: usize) {
        match self {
            Handles::Inline { buf, len } => {
                let n = *len as usize;
                for j in start..n - count {
                    buf[j] = buf[j + count];
                }
                *len -= count as u8;
            }
            Handles::Heap(v) => {
                v.drain(start..start + count);
            }
        }
    }
    /// Take the handles out, leaving an empty (inline) `Handles`; returns the OWNED `Handles` — a MOVE,
    /// never a heap alloc even for an inline node. The FBIP rebuild mutates the returned value via these
    /// methods and reinstalls it via `champ_become_hdr`.
    #[inline]
    fn take(&mut self) -> Handles {
        core::mem::take(self)
    }
    /// A fresh owned `Vec` copy of the handles — the path-COPY path clones the source node's children
    /// into a working buffer it then mutates/reinstalls. (`Handle` is `Copy`, so this copies pointers,
    /// touching no refcount — the caller applies the dup/drop discipline.)
    #[inline]
    fn to_vec(&self) -> Vec<Handle> {
        self.as_slice().to_vec()
    }
    /// Consume into an owned `Vec` — a MOVE for `Heap`, a materialize for `Inline`. Only for consumers
    /// that need `Vec` push/pop semantics (the CURSOR frame stack, kept on the heap arm — see
    /// `from_vec_heap`), so this is a no-copy move in practice.
    #[inline]
    fn into_vec(self) -> Vec<Handle> {
        match self {
            Handles::Heap(v) => v,
            Handles::Inline { buf, len } => buf[..len as usize].to_vec(),
        }
    }
    /// Move all handles out onto `out` (leaving self empty). Used by the free cascade.
    #[inline]
    fn append_into(&mut self, out: &mut Vec<Handle>) {
        match self {
            Handles::Inline { buf, len } => {
                out.extend_from_slice(&buf[..*len as usize]);
                *len = 0;
            }
            Handles::Heap(v) => out.append(v),
        }
    }
    /// Wrap a `Vec` WITHOUT inlining — keep the `Heap` arm even for ≤2 elements. For the CURSOR frame
    /// stack only: `champ_advance_fbip` push/pops it as a `Vec` (depth reaches the trie height) and
    /// `champ_cursor_take` moves it out via `into_vec`; inlining a shallow cursor would force a Vec
    /// re-materialize every advance step (regresses map_iterate 3→553). NOT for value nodes (those go
    /// through `From` so ≤2-child nodes inline — the win).
    #[inline]
    fn from_vec_heap(v: Vec<Handle>) -> Handles {
        Handles::Heap(v)
    }
}

impl From<Vec<Handle>> for Handles {
    /// Build from a freshly-constructed handle vector (the `alloc`/reinstall boundary): INLINE it when it
    /// fits (the common ≤2-child node — the Vec is dropped, unallocated-away), else keep the heap buffer
    /// verbatim (no copy). Same inline-on-construct discipline as `Raw::from`.
    #[inline]
    fn from(v: Vec<Handle>) -> Handles {
        if v.len() <= INLINE_HANDLES_CAP {
            let mut buf = [Handle::NULL; INLINE_HANDLES_CAP];
            buf[..v.len()].copy_from_slice(&v);
            Handles::Inline {
                buf,
                len: v.len() as u8,
            }
        } else {
            Handles::Heap(v)
        }
    }
}

impl core::ops::Deref for Handles {
    type Target = [Handle];
    #[inline]
    fn deref(&self) -> &[Handle] {
        self.as_slice()
    }
}

/// The internal handle: the address of a `Node`. The core is written entirely in terms of this, so
/// RC and persistent collections are testable natively. `NULL` is the benign-default sentinel a
/// total read yields for a missing node (a mismatch is a compiler bug; the runtime must never trap
/// on a null read).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Handle(*mut Node);

impl Handle {
    const NULL: Handle = Handle(core::ptr::null_mut());

    /// Guarded shared deref (mirrors `<*mut Node>::as_ref`): `None` for a null handle, else the node —
    /// but in a debug build (`test` / `debug-counters`) it FIRST asserts the cell is a LIVE node, so a
    /// read through a freed or fabricated handle traps loudly AT THE GETTER instead of returning wild
    /// bytes. This is the every-access-site companion to the `with_node` / `with_raw_arity` chokepoints:
    /// routing the direct derefs through it closes read-after-free coverage on the direct getters too
    /// (the operator's "UAF is much worse than leaks" safety net, extended past the two chokepoints).
    /// In the SHIPPED build the guard is `cfg`-absent, so this is EXACTLY `self.0.as_ref()` — zero cost,
    /// and the release `Node` layout / codegen (`REQUIRED_RUNTIME_HASH`) is byte-unchanged; only the
    /// debug-counters hash moves.
    #[inline(always)]
    unsafe fn node_ref<'a>(self) -> Option<&'a Node> {
        let n = unsafe { self.0.as_ref() };
        #[cfg(any(test, feature = "debug-counters"))]
        if let Some(node) = n {
            assert_node_live(self.0, node.guard, "node_ref");
        }
        n
    }

    /// Guarded mutable deref — the `as_mut` twin of [`Handle::node_ref`]. Reads the guard through a
    /// short-lived shared ref FIRST (so no borrow overlaps the returned `&mut`), then hands out the
    /// mutable node. Same debug-only LIVE-guard, same zero-cost `self.0.as_mut()` in the shipped build.
    #[inline(always)]
    unsafe fn node_mut<'a>(self) -> Option<&'a mut Node> {
        #[cfg(any(test, feature = "debug-counters"))]
        if let Some(node) = unsafe { self.0.as_ref() } {
            assert_node_live(self.0, node.guard, "node_mut");
        }
        unsafe { self.0.as_mut() }
    }
}

// ─── Low-bit tagged immediates (inline handles) — DESIGN-inline-handle-tagging.md §4.1 ────
// A `Handle` whose low 2 bits are non-zero carries its VALUE inline (no heap Node). Real nodes
// come only from `alloc` (`Box::into_raw`), which is ≥4-aligned, so a real pointer and NULL both
// have low bits `00`. This single discriminant (`is_immediate`) is shared by BOTH the native and
// the wasm32 build (alignment ≥4 holds on a native pointer and a wasm32 `u32` alike), which is why
// the native suite exercises the shipped representation and `to_u32`/`from_u32` need no change.
//
// LIVE: `op_box_int` returns an immediate for an in-window fixnum, `op_box_bool` always inlines, and
// `op_arr_alloc(0)` returns the inline unit — so an `is_immediate` handle really flows through the ops,
// and every deref-site guard is exercised. The guards stay INERT only for cross-kind ops where an
// immediate can never legitimately appear (an immediate is never a bytes/map/sum value).
//
// Encoding (tag = low 2 bits of `Handle.0`):
//   00  pointer or NULL (unchanged)
//   01  fixnum: 30-bit signed int in bits[31:2]; value = (h.0 as i32) >> 2 (arith shift)
//   10  atom:   subkind in bits[3:2]; 00 = unit, 01 = bool (value in bit[4]); 10/11 reserved
//   11  reserved (future widening)

/// The single classifier: a handle is immediate iff its low 2 bits are non-zero. NULL is tag `00`,
/// so it is NOT immediate (it keeps its benign `with_node`/RC-noop defaults).
#[inline]
fn is_immediate(h: Handle) -> bool {
    (h.0 as usize) & 0b11 != 0
}

/// The inline `unit` (empty tuple): atom subkind `00`, tag `10` ⇒ bits `0b0010`.
#[inline]
#[allow(dead_code)]
fn imm_unit() -> Handle {
    Handle(0b0010usize as *mut Node)
}

/// The ABI IMMEDIATE encodings, emitted into a `cdz-abi` wasm CUSTOM SECTION so the COMPILER can learn
/// them without hand-coding an ABI constant. Today it carries one value — the inline-unit handle bits
/// (`imm_unit`): `op_arr_alloc(0)` returns exactly this (a compile-time-known handle, no heap node), so
/// the compiler can push it as a constant instead of emitting a runtime `arr-alloc(0)` CALL for every
/// unit payload (a nullary sum variant, an empty tuple/record/list).
///
/// `cargo xtask codegen` reads this section (by name, statically — no execution) out of the RAW runtime
/// build and emits its value into `runtime_abi.rs` as `IMM_UNIT`, BEFORE `canonicalize_runtime`'s
/// `wasm-tools strip -a` removes all custom sections (so the section costs zero bytes in the shipped/
/// hashed runtime, and the const the compiler pushes is DERIVED from the runtime, guarded by the content
/// hash — never a hand-transcribed number). The bytes are the little-endian `u32` of `imm_unit()`'s bit
/// pattern; a change to the encoding re-derives through codegen on the next run. No `#[used]` is needed:
/// `link_section` on a `static` places it in the `cdz-abi` section, which survives the `wasm32` build (a
/// unit test also references it), and codegen's `read_abi_imm_unit` PANICS loudly if the section is ever
/// absent — so a lost section fails codegen/CI visibly rather than silently. `#[allow(dead_code)]` only
/// silences the release-build lint (the sole in-crate reference is `#[cfg(test)]`); deliberately NOT
/// `#[used]`, because `#[used]` perturbs the STRIPPED runtime bytes and would force a fleet-wide
/// `REQUIRED_RUNTIME_HASH` bump for zero behavioral gain.
#[allow(dead_code)]
#[unsafe(link_section = "cdz-abi")]
static CDZ_ABI_IMM_UNIT: [u8; 4] = (0b0010u32).to_le_bytes();

/// An inline boolean: atom subkind `01`, tag `10`, value in bit[4] ⇒ false = `0b0110`, true = `0b10110`.
#[inline]
#[allow(dead_code)]
fn imm_bool(b: bool) -> Handle {
    Handle((0b0110usize | ((b as usize) << 4)) as *mut Node)
}

/// Fixnum window (canonical-form-critical): a value inlines iff it fits the 30-bit signed range.
const FIXNUM_MIN: i64 = -(1 << 29);
const FIXNUM_MAX: i64 = (1 << 29) - 1;

/// Whether `v` fits the inline fixnum window `[-2^29, 2^29 - 1]`. THE single source of truth for the
/// boundary (used in exactly one place, `op_box_int`, in Phase 2) so a value never sometimes-inlines.
#[inline]
fn fixnum_fits(v: i64) -> bool {
    (FIXNUM_MIN..=FIXNUM_MAX).contains(&v)
}

/// An inline fixnum. Caller MUST have checked `fixnum_fits(v)`; the `(v << 2)` shift would otherwise
/// drop high bits. Tag `01` in the low 2 bits, value sign-extended into bits[31:2].
#[inline]
fn imm_int(v: i64) -> Handle {
    Handle((((v as i32) << 2) | 0b01) as usize as *mut Node)
}

/// Decode an inline fixnum (only valid when `is_immediate(h)` and tag is `01`). Arithmetic shift
/// sign-extends the 30-bit payload back to a full `i64`.
#[inline]
fn imm_as_int(h: Handle) -> i64 {
    ((h.0 as usize as i32) >> 2) as i64
}

/// Decode an inline boolean (only valid when `is_immediate(h)` and it is a bool atom).
#[inline]
#[allow(dead_code)]
fn imm_as_bool(h: Handle) -> bool {
    (h.0 as usize >> 4) & 1 != 0
}

/// The kind of an immediate. Only valid when `is_immediate(h)`.
#[allow(dead_code)]
enum ImmKind {
    Unit,
    Bool,
    Int,
}

/// Classify an immediate. Only valid when `is_immediate(h)`.
#[inline]
#[allow(dead_code)]
fn imm_kind(h: Handle) -> ImmKind {
    match (h.0 as usize) & 0b11 {
        0b01 => ImmKind::Int,
        _ /* 0b10 atom */ => {
            if (h.0 as usize >> 2) & 0b11 == 0 {
                ImmKind::Unit
            } else {
                ImmKind::Bool
            }
        }
    }
}

/// The raw bytes a BOXED twin of this immediate would carry (the same bytes `op_box_int`/
/// `op_box_bool`/`op_arr_alloc(0)` write), so an inline value hashes / compares EQUAL to its boxed
/// twin (open-Q#8) and orders consistently. An immediate always has arity 0 (no child handles).
/// Only valid when `is_immediate(h)`.
#[allow(dead_code)]
fn imm_canonical_raw(h: Handle) -> Vec<u8> {
    match imm_kind(h) {
        ImmKind::Unit => Vec::new(),
        ImmKind::Bool => vec![imm_as_bool(h) as u8],
        ImmKind::Int => (imm_as_int(h) as u64).to_le_bytes().to_vec(),
    }
}

/// `(canonical raw bytes, child arity)` for either an immediate (canonical raw, arity 0) or a real
/// node (its `raw`, `handles.len()`); NULL folds as `(empty, 0)`. Used by the CHAMP guards to compare
/// an inline value structurally with its boxed twin.
#[allow(dead_code)]
fn node_raw_arity(h: Handle) -> (Vec<u8>, usize) {
    if is_immediate(h) {
        (imm_canonical_raw(h), 0)
    } else {
        with_node(h, (Vec::new(), 0usize), |n| {
            (n.raw.to_vec(), n.handles.len())
        })
    }
}

/// Call `f` with `h`'s canonical raw bytes (BORROWED) and child arity, allocating NOTHING — the
/// alloc-free twin of `node_raw_arity` for the hot CHAMP eq/cmp immediate branches. A real node lends
/// its `raw` slice directly (no clone); an immediate materializes its ≤8 canonical little-endian
/// bytes — the SAME bytes a boxed twin carries (open-Q#8), so eq/cmp treat inline and boxed alike —
/// into a stack buffer and lends a subslice; NULL folds as `(&[], 0)`. Extracted so the two callers
/// stop cloning a `Vec` per comparison to inspect ≤8 bytes (mirrors the `champ_hash` scalar fast path).
#[inline]
fn with_raw_arity<T>(h: Handle, f: impl FnOnce(&[u8], usize) -> T) -> T {
    if is_immediate(h) {
        let mut buf = [0u8; 8];
        let len = match imm_kind(h) {
            ImmKind::Unit => 0usize,
            ImmKind::Bool => {
                buf[0] = imm_as_bool(h) as u8;
                1
            }
            ImmKind::Int => {
                buf = (imm_as_int(h) as u64).to_le_bytes();
                8
            }
        };
        f(&buf[..len], 0)
    } else {
        match unsafe { h.node_ref() } {
            Some(node) => {
                #[cfg(any(test, feature = "debug-counters"))]
                assert_node_live(h.0, node.guard, "raw/arity read");
                f(&node.raw, node.handles.len())
            }
            None => f(&[], 0), // NULL folds as (empty, 0)
        }
    }
}

// Count of nodes currently allocated and not yet freed. Compiled under the native test suite AND the
// `debug-counters` wasm feature: the native suite asserts exact reclamation and BOUNDED PEAK HEAP
// across iterations (the leak / peak-heap acceptance probe), and a wasm leak-check harness reads it via the
// `live-objects` export after a run to prove the compiler's Perceus dup/drop discipline balances
// (assert 0). In the DEFAULT (shipped) build neither the counter nor its updates exist — the runtime
// is zero-cost and byte-stable, and `live-objects` returns 0.
#[cfg(any(test, feature = "debug-counters"))]
runtime_local! {
    static LIVE_NODES: core::cell::Cell<i64> = core::cell::Cell::new(0);
}

// ─── rc-trace (leak-attribution diagnostic — debug-counters only) ────────────────────────────────
// A per-node ALLOC/DUP/DROP event log for DEFINITIVE leak attribution (which handle never reached rc0,
// and whether the missing op is a direct drop, a cascade edge, or an extra dup — the three modes static
// WAT op-counts cannot separate, since cascade-children muddy the count). Endorsed by v-memory-safety +
// v-corpus-harness as the attribution complement to `--guarded-all`. OFF by default (zero append cost)
// even in the debug build until `rc_trace_enable(true)`: cdz-run's `--rc-trace` (via the debug-only
// `debug-trace` WIT export) flips it before a run; native tests flip it directly. Buffer keeps the FIRST
// `RC_TRACE_CAP` events and sets `RC_TRACE_TRUNCATED` on overflow (a "truncated at N" marker, never a
// silent ring-wrap that loses the early allocs a leak needs). Release build: this whole region is cfg'd
// OUT → `REQUIRED_RUNTIME_HASH` (058B5h) byte-unchanged; only `DEBUG_RUNTIME_HASH` moves (v-nix re-bake).
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_ALLOC: u8 = 0;
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_DUP: u8 = 1;
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_DROP: u8 = 2;
/// A node LEFT THE CENSUS as IMMORTAL (`mark-immortal`/`mark-immortal-deep`): it is reclaimed-via-immortal
/// (a build-once static held for the instance's life), NOT a leak and NOT a freed drop. The leak summary
/// must treat a node with this event as census-exited (excluded from "ALLOC with no freed DROP") — else
/// every immortal constant reads as a false leak (the dqe17 false-positive that motivated this).
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_MARK_IMMORTAL: u8 = 3;

// Structural node tag — the runtime is TAGLESS (see `Node`: no stored Cadenza type), so this is the
// node SHAPE, not the semantic type. `Leaf` (0 handles, raw-bearing: scalar/Bytes/String/BigInt — not
// distinguishable from each other), `Sum` (1 handle + a non-empty raw disc), `Compound` (≥1 handle,
// empty raw: tuple/record/list/map — not distinguishable from each other). Separates an Ast.Int SUM from
// a BigInt LEAF (v-mem's key case); a finer semantic type is the emit's compile-time knowledge, keyed by
// node#, not the runtime's to give.
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TAG_LEAF: u8 = 0;
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TAG_SUM: u8 = 1;
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TAG_COMPOUND: u8 = 2;

/// `cascade_parent` sentinel for "no parent" (a direct/root drop, or a non-freeing event).
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_NO_PARENT: u32 = u32::MAX;

/// A fixed rc-trace event record (v-memory-safety's schema). `cascade_parent == RC_TRACE_NO_PARENT`
/// means None (a direct drop or a non-DROP event). For v1 a cascade child's `cascade_parent` is the
/// ROOT drop's node# that initiated the cascade (== the immediate parent at depth 2, e.g. an Ast.Int
/// sum's BigInt child); deeper immediate-parent linkage is a follow-up (the flat free-worklist would
/// need to carry per-entry parent ids under a debug-only cfg-split).
#[cfg(any(test, feature = "debug-counters"))]
#[derive(Clone, Copy)]
pub(crate) struct RcTraceEvent {
    pub op: u8,
    pub node: u32,
    pub tag: u8,
    pub rc_before: u32,
    pub rc_after: u32,
    pub freed: bool,
    pub cascade_parent: u32,
}

/// Buffer capacity — keeps the FIRST this-many events, then marks truncated. 64Ki events covers a
/// typical corpus case; a leak needs the early allocs, so keep-first beats a lossy ring-wrap.
#[cfg(any(test, feature = "debug-counters"))]
pub(crate) const RC_TRACE_CAP: usize = 1 << 16;

#[cfg(any(test, feature = "debug-counters"))]
runtime_local! {
    static NEXT_NODE_ID: core::cell::Cell<u32> = core::cell::Cell::new(0);
}
#[cfg(any(test, feature = "debug-counters"))]
runtime_local! {
    static RC_TRACE_ENABLED: core::cell::Cell<bool> = core::cell::Cell::new(false);
}
#[cfg(any(test, feature = "debug-counters"))]
runtime_local! {
    static RC_TRACE_TRUNCATED: core::cell::Cell<bool> = core::cell::Cell::new(false);
}
#[cfg(any(test, feature = "debug-counters"))]
runtime_local! {
    static RC_TRACE: core::cell::RefCell<alloc::vec::Vec<RcTraceEvent>> =
        core::cell::RefCell::new(alloc::vec::Vec::new());
}

/// Mint the next monotonic node id (unique PER ALLOC — a Handle slot reused after free gets a NEW id,
/// so "alloc'd but never freed" attribution is unambiguous).
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn rc_next_node_id() -> u32 {
    NEXT_NODE_ID.with(|c| {
        let v = c.get();
        c.set(v.wrapping_add(1));
        v
    })
}

/// The structural (shape) tag of a node — tagless runtime, so shape only (see `RC_TAG_*`).
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn rc_struct_tag(node: &Node) -> u8 {
    if node.handles.is_empty() {
        RC_TAG_LEAF
    } else if node.handles.len() == 1 && node.raw.len() != 0 {
        RC_TAG_SUM
    } else {
        RC_TAG_COMPOUND
    }
}

/// Append one rc-trace event (no-op unless recording is enabled; drops on overflow with a marker).
#[cfg(any(test, feature = "debug-counters"))]
#[inline]
fn rc_trace_push(
    op: u8,
    node: u32,
    tag: u8,
    rc_before: u32,
    rc_after: u32,
    freed: bool,
    cascade_parent: u32,
) {
    if !RC_TRACE_ENABLED.with(|e| e.get()) {
        return;
    }
    RC_TRACE.with(|t| {
        let mut buf = t.borrow_mut();
        if buf.len() >= RC_TRACE_CAP {
            RC_TRACE_TRUNCATED.with(|x| x.set(true));
            return;
        }
        buf.push(RcTraceEvent {
            op,
            node,
            tag,
            rc_before,
            rc_after,
            freed,
            cascade_parent,
        });
    });
}

/// Enable/disable rc-trace recording (OFF by default → zero append cost). Enabling CLEARS the buffer +
/// the truncation marker so each traced run starts fresh. cdz-run's `--rc-trace` calls this via the
/// `debug-trace` WIT export; native tests call it directly.
#[cfg(any(test, feature = "debug-counters"))]
#[allow(dead_code)]
pub(crate) fn rc_trace_enable(on: bool) {
    RC_TRACE_ENABLED.with(|e| e.set(on));
    if on {
        RC_TRACE.with(|t| t.borrow_mut().clear());
        RC_TRACE_TRUNCATED.with(|x| x.set(false));
    }
}

/// Snapshot the recorded events (for a native test / the drain export to consume). Second tuple field
/// is the truncation marker (true = the run produced more than `RC_TRACE_CAP` events, buffer holds the
/// first `RC_TRACE_CAP`).
#[cfg(any(test, feature = "debug-counters"))]
#[allow(dead_code)]
pub(crate) fn rc_trace_snapshot() -> (alloc::vec::Vec<RcTraceEvent>, bool) {
    (
        RC_TRACE.with(|t| t.borrow().clone()),
        RC_TRACE_TRUNCATED.with(|x| x.get()),
    )
}

/// The truncation marker alone (true = the run produced more than `RC_TRACE_CAP` events) — a cheap
/// read for the `rc-trace-truncated` export that avoids cloning the whole event buffer.
#[cfg(any(test, feature = "debug-counters"))]
#[allow(dead_code)]
pub(crate) fn rc_trace_truncated_flag() -> bool {
    RC_TRACE_TRUNCATED.with(|x| x.get())
}

/// Serialize the recorded events to the flat `list<u8>` wire the `rc-trace-drain` export returns: one
/// 20-byte little-endian record per event — `[op:u8, tag:u8, freed:u8, _pad:u8, node:u32, rc_before:u32,
/// rc_after:u32, cascade_parent:u32]` (`cascade_parent == RC_TRACE_NO_PARENT` = none). The consumer
/// (cdz-run `--rc-trace` / v-corpus-harness) decodes this fixed layout.
#[cfg(any(test, feature = "debug-counters"))]
#[allow(dead_code)]
pub(crate) fn rc_trace_drain_bytes() -> alloc::vec::Vec<u8> {
    let events = RC_TRACE.with(|t| t.borrow().clone());
    let mut out = alloc::vec::Vec::with_capacity(events.len() * 20);
    for e in &events {
        out.push(e.op);
        out.push(e.tag);
        out.push(e.freed as u8);
        out.push(0u8); // _pad — keeps the u32 fields 4-byte aligned within the record
        out.extend_from_slice(&e.node.to_le_bytes());
        out.extend_from_slice(&e.rc_before.to_le_bytes());
        out.extend_from_slice(&e.rc_after.to_le_bytes());
        out.extend_from_slice(&e.cascade_parent.to_le_bytes());
    }
    out
}

/// The live heap-object count, or 0 when the counter is not compiled in (the default build). The
/// `live-objects` export returns this; a leak-check harness asserts it is 0 after a run to verify the
/// compiler's dup/drop discipline leaves nothing behind.
#[allow(dead_code)]
fn live_object_count() -> u32 {
    #[cfg(any(test, feature = "debug-counters"))]
    {
        LIVE_NODES.with(|n| n.get()).max(0) as u32
    }
    #[cfg(not(any(test, feature = "debug-counters")))]
    {
        0
    }
}

/// Allocate a node (refcount 1) from `handles` + `raw` and return its handle. Uses the global
/// allocator (talc on wasm) — the core never names it, so a size-classed free-list could be swapped
/// in here alone.
fn alloc(handles: Vec<Handle>, raw: Vec<u8>) -> Handle {
    // Convert the byte vector to a `Raw` (inline ≤INLINE_RAW_CAP, the common case) then delegate. Note:
    // a caller that ALREADY holds the bytes as a `Vec` (byte buffers, string leaves) still allocated
    // that Vec; the alloc-saving win is for callers that build a small header directly as inline `Raw`
    // via `alloc_raw` (see `champ_header`), never materializing a transient Vec.
    alloc_raw(handles, Raw::from(raw))
}

/// `alloc` but taking a ready `Raw` — lets a caller that builds a small header INLINE (e.g.
/// `champ_header`) skip the transient `Vec` allocation entirely. `handles` is taken as a `Vec` (the
/// construction sites build `vec![…]`) and moved into the node's `Handles` field.
fn alloc_raw(handles: impl Into<Handles>, raw: Raw) -> Handle {
    #[cfg(any(test, feature = "debug-counters"))]
    LIVE_NODES.with(|n| n.set(n.get() + 1));
    let ptr = Box::into_raw(Box::new(Node {
        rc: 1,
        handles: handles.into(),
        raw,
        #[cfg(any(test, feature = "debug-counters"))]
        guard: 0, // provisional; stamped with the address-derived live sentinel just below
        #[cfg(any(test, feature = "debug-counters"))]
        node_id: 0, // provisional; assigned the monotonic rc-trace id just below
    }));
    // Stamp the LIVE guard (needs the address, so after `into_raw`) + mint the rc-trace node id and
    // record the ALLOC event (rc 0→1). Debug builds only.
    #[cfg(any(test, feature = "debug-counters"))]
    unsafe {
        (*ptr).guard = live_guard(ptr);
        let id = rc_next_node_id();
        (*ptr).node_id = id;
        rc_trace_push(
            RC_TRACE_ALLOC,
            id,
            rc_struct_tag(&*ptr),
            0,
            1,
            false,
            RC_TRACE_NO_PARENT,
        );
    }
    Handle(ptr)
}

/// Borrow a node to read from it TOTALLY; a null handle yields `default`. Centralizes the one unsafe
/// deref and its null check for the reads that are total by construction (scalars, lengths, sum
/// disc/payload, string). The index accessors do NOT use this — they distinguish a benign null from
/// an out-of-bounds index into a valid node (which traps), so they inline their own check.
fn with_node<T>(h: Handle, default: T, f: impl FnOnce(&Node) -> T) -> T {
    match unsafe { h.node_ref() } {
        Some(node) => {
            // UAF/wild-handle guard (debug only): reading through a freed or fabricated handle is a bug.
            // `with_node` is the central node reader (`node_rc`, the accessors), so this catches most reads.
            #[cfg(any(test, feature = "debug-counters"))]
            assert_node_live(h.0, node.guard, "read");
            f(node)
        }
        None => default,
    }
}

/// Fail-fast on an out-of-bounds index into a VALID node. In correct operation this never fires:
/// tuple/record access is by static index, list/bytes/string `.at` get their bounds check emitted by
/// the compiler, and the renderer only ever indexes `0..len`. Reaching it means a compiler-invariant
/// violation — we trap (a panic, which `panic = "abort"` lowers to a wasm trap) rather than return a
/// silent wrong value the differential gate might miss. (Operator decision: trap on OOB.)
#[cold]
#[inline(never)]
fn trap_oob() -> ! {
    panic!("cdz-runtime: index out of bounds into a valid node")
}

/// Read up to 8 little-endian bytes of `raw` as a `u64` (zero-padded if shorter). Keeps scalar reads
/// TOTAL even on a mismatched/short node — a compiler bug yields deterministic bits, never a trap.
fn read_word(raw: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = raw.len().min(8);
    buf[..n].copy_from_slice(&raw[..n]);
    u64::from_le_bytes(buf)
}

/// Read up to 4 little-endian bytes of `raw` as a `u32` (zero-padded if shorter) — the sum
/// discriminant, total on a short/mismatched node.
fn read_disc(raw: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = raw.len().min(4);
    buf[..n].copy_from_slice(&raw[..n]);
    u32::from_le_bytes(buf)
}

// ─── Submodules ────────────────────────────────────────────────────────────────────────────────

mod array_sum;
mod bytes_string;
mod champ;
mod guest;
mod map;
mod rc;
mod scalars;
mod value_codec;
mod vector;

// Re-export public items needed across modules or by tests
pub(crate) use array_sum::*;
pub(crate) use bytes_string::*;
pub(crate) use champ::*;
pub(crate) use guest::*;
pub(crate) use map::*;
pub(crate) use rc::*;
pub(crate) use scalars::*;
pub(crate) use value_codec::*;
pub(crate) use vector::*;

// ─── Native tests: exercise the Handle-typed heap+accessor core ─────────────────────────

// TEMP PROFILER (removed after measurement): a process-wide counting allocator for the native test
// build only, so a probe can measure GROSS heap allocations (including transient Vecs freed at once —
// the target of the clone-elimination work) per runtime operation. Counting only; correctness intact.
#[cfg(test)]
struct CountingAlloc;
#[cfg(test)]
static ALLOC_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[cfg(test)]
#[global_allocator]
static PROFILER_ALLOC: CountingAlloc = CountingAlloc;

#[cfg(test)]
mod tests;
