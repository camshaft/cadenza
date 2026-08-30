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
    }));
    // Stamp the LIVE guard (needs the address, so after `into_raw`). Debug builds only.
    #[cfg(any(test, feature = "debug-counters"))]
    unsafe {
        (*ptr).guard = live_guard(ptr);
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

// ─── Scalar leaves: box a primitive, read it back (TOTAL — reinterprets bytes, never traps) ──────

fn op_box_int(v: i64) -> Handle {
    // Normalize-on-construct (P2), THE single source of truth for the fixnum boundary: a value that
    // fits the inline window is ALWAYS an immediate, never boxed, so inline-3 and boxed-3 cannot
    // coexist (canonical form). Only out-of-window ints keep a heap Node.
    if fixnum_fits(v) {
        return imm_int(v);
    }
    alloc_raw(Vec::new(), Raw::inline(&(v as u64).to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
fn op_get_int(h: Handle) -> i64 {
    if is_immediate(h) {
        return imm_as_int(h); // rep-agnostic decode; equals a boxed twin's `read_word`
    }
    with_node(h, 0, |n| read_word(&n.raw) as i64)
}
fn op_box_bool(v: bool) -> Handle {
    // Normalize-on-construct (P1b): a bool ALWAYS inlines, never boxes, so inline is the one
    // canonical representation. `imm_bool` carries the value in the tag bits — no heap Node.
    imm_bool(v)
}
fn op_get_bool(h: Handle) -> bool {
    if is_immediate(h) {
        return imm_as_bool(h); // rep-agnostic decode of an inline bool
    }
    with_node(h, false, |n| n.raw.first().is_some_and(|&b| b != 0))
}
fn is_whole_f64(f: f64) -> bool {
    let bits = f.to_bits();
    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    if biased_exp == 0 {
        return mantissa == 0;
    }
    let e = biased_exp - 1023;
    if e >= 52 {
        return true;
    }
    if e < 0 {
        return false;
    }
    let frac_bits = 52 - e as u32;
    (mantissa & ((1u64 << frac_bits) - 1)) == 0
}
fn op_box_float(v: f64) -> Handle {
    // Normalize-on-construct to the CANONICAL byte form (deterministic-value-form.md §A Value Has One
    // Canonical Byte Form): every NaN — of ANY bit pattern (a distinct literal NaN, or a runtime
    // arithmetic NaN like `0.0/0.0` whose payload/sign wasm need not fix) — collapses to the ONE
    // canonical quiet NaN `f64::NAN.to_bits()`, exactly the pattern the compiler's `ConstFloatNan`
    // emits. Without this, two NaN values with differing bits would be DISTINCT map/set keys and
    // structurally-unequal under `champ_hash`/`champ_eq` (which compare raw bytes), whereas the spec's
    // canonical form makes every NaN equal to every NaN. A NON-NaN value (incl. ±0.0 and ±inf) keeps
    // its bits verbatim, so `-0.0` stays DISTINCT from `0.0` (their canonical forms genuinely differ).
    // This is the float twin of `op_box_int`'s normalize-on-construct: `box-float` is the SOLE producer
    // of a float leaf, so canonicalizing here guarantees every stored float has one byte form — one
    // canonical encoding per value, equal values (every NaN) sharing identical bytes and unequal values
    // (±0.0) keeping distinct bytes, which `champ_hash`/`champ_eq` compare rawly:
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Each serializable value MUST have exactly one canonical byte encoding.
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Two values that are equal under the language's structural equality MUST have identical canonical byte encodings.
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Two values that are not equal under the language's structural equality MUST have distinct canonical byte encodings.
    let bits = if v.is_nan() {
        f64::NAN.to_bits()
    } else {
        v.to_bits()
    };
    alloc_raw(Vec::new(), Raw::inline(&bits.to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
fn op_get_float(h: Handle) -> f64 {
    if is_immediate(h) {
        return 0.0; // cross-kind totality: a float is never itself an immediate
    }
    with_node(h, 0.0, |n| f64::from_bits(read_word(&n.raw)))
}
/// Box a `Float32` in its NATURAL 4-byte form (distinct from `box-float`'s 8-byte Float64), so a
/// Float32's canonical byte form — and value-encode's shortest-decimal render — is the f32's, not a
/// promoted f64's. NaN-canonicalized on construction (the f32 twin of `op_box_float`): any NaN → the
/// one canonical quiet `f32::NAN.to_bits()`, so two NaN Float32s are the same map/set key. Non-NaN
/// (incl. ±0.0/±inf) keeps its bits, so `-0.0f32` stays distinct from `0.0f32`.
fn op_box_float32(v: f32) -> Handle {
    let bits = if v.is_nan() {
        f32::NAN.to_bits()
    } else {
        v.to_bits()
    };
    alloc_raw(Vec::new(), Raw::inline(&bits.to_le_bytes())) // 4-byte scalar: inline, no heap raw
}
fn op_get_float32(h: Handle) -> f32 {
    if is_immediate(h) {
        return 0.0; // cross-kind totality: a float is never itself an immediate
    }
    with_node(h, 0.0f32, |n| {
        // Read the low 4 bytes of the raw (zero-padded past the end — defensive, total).
        let mut buf = [0u8; 4];
        let k = n.raw.len().min(4);
        buf[..k].copy_from_slice(&n.raw[..k]);
        f32::from_bits(u32::from_le_bytes(buf))
    })
}

// ─── Arbitrary-precision integer (BigInt) — a sign-magnitude limb-array LEAF ─────────────────────
// A `BigInt` value is a raw-only heap leaf (zero handles), the `Bytes`-leaf shape: its `raw` holds the
// canonical sign-magnitude bytes of a `bigint::Big` (`to/from_sign_magnitude_bytes`). ALWAYS a heap leaf
// — never a fixnum immediate — because `BigInt` is a DISTINCT type from a fixed-width int: an immediate
// tag means "small int", and conflating the two would let a `BigInt` handle be misread as an `Int`. The
// arithmetic ops unbox both operands to `Big`, compute (the hand-written limb library), and re-box the
// normalized result. `op_dup`/`op_drop` need no change (a raw-only leaf is the cheapest node shape).

/// Box a `Big` as a BigInt heap leaf — its canonical sign-magnitude bytes in `raw`, zero handles.
fn box_bigint(b: &bigint::Big) -> Handle {
    // Fast path — a small BigInt (single/few limbs → ≤`INLINE_RAW_CAP` sign-magnitude bytes, the common
    // case) serializes DIRECTLY into an inline `Raw` with NO transient heap Vec (the `to_sign_magnitude_
    // bytes` + `Raw::from` path would allocate that Vec then free it once inlined — the transient-small-Vec
    // smell). A larger value falls back to the heap serialization. Byte-identical either way.
    let mut buf = [0u8; INLINE_RAW_CAP];
    if let Some(n) = b.to_sign_magnitude_bytes_into(&mut buf) {
        return alloc_raw(Vec::new(), Raw::inline(&buf[..n]));
    }
    alloc_raw(Vec::new(), Raw::from(b.to_sign_magnitude_bytes()))
}
/// Read a BigInt leaf back to a `Big`. Total: a null/mismatched node reads as zero (deterministic bits,
/// never a trap — the scalar-read discipline). A BigInt is never an immediate, so no immediate decode.
fn unbox_bigint(h: Handle) -> bigint::Big {
    with_node(h, bigint::Big::zero(), |n| {
        bigint::Big::from_sign_magnitude_bytes(&n.raw)
    })
}
/// `bigint-of-i64` — widen a fixed-width `i64` into a `BigInt` leaf (the `BigInt.of` target for a runtime
/// integer; a constant folds in the compiler and never calls this). Boxes the value DIRECTLY through the
/// i128 path (`box_bigint_i128`, which serializes to inline sign-magnitude bytes with NO `Big`) — an i64
/// trivially fits i128. This skips the transient `Big::from_i64` limb `Vec` the `box_bigint(&Big)` route
/// allocated-then-freed per call (the same transient-small-Vec smell `box_bigint`'s own inline fast path
/// avoids, reintroduced by the `Big` intermediate). Byte-identical leaf (both emit the canonical
/// `[sign][LE magnitude, trailing-zeros-stripped]` form — verified across the full i64 range incl. i64::MIN
/// + limb boundaries).
fn op_bigint_of_i64(v: i64) -> Handle {
    box_bigint_i128(v as i128)
}
/// `bigint-of-bytes` — build a BigInt leaf from a Bytes leaf holding the canonical sign-magnitude bytes
/// (`[sign][LE magnitude, trailing-zeros-stripped]`). The compiler emits this to materialize a CONSTANT
/// BigInt whose magnitude exceeds i64 range (too large for `bigint-of-i64`): it bakes the sign-magnitude
/// bytes as a Bytes leaf (`bytes-alloc`/`bytes-set`, like a constant string) then re-tags them here. The
/// input may be a rope (a concat/slice) in general, so FLATTEN it before reading `raw` — exactly as the
/// value-encode `Shape::Bytes` walker does. `from_sign_magnitude_bytes` re-normalizes (a malformed/empty
/// input decodes as zero — total), so `box_bigint` re-emits the canonical leaf form. CONSUMES `buf` (the
/// transient byte leaf is dropped after its content is read).
fn op_bigint_of_bytes(buf: Handle) -> Handle {
    bytes_flatten(buf);
    let big = with_node(buf, bigint::Big::zero(), |n| {
        bigint::Big::from_sign_magnitude_bytes(&n.raw)
    });
    let out = box_bigint(&big);
    op_drop(buf);
    out
}
/// `bigint-to-i64-checked` — the CHECKED narrowing back to `i64`: the value if it fits, else TRAP
/// (`options/numeric-model/explicit-checked.md` — `Int64.of` of an out-of-range BigInt traps). Reads the
/// leaf's sign-magnitude `raw` slice DIRECTLY (`Big::i64_checked_from_sign_magnitude_bytes`) — a narrowing
/// is READ-ONLY, so it needs NO `Big` (no limb `Vec`): allocation-free. A null node reads as zero.
fn op_bigint_to_i64_checked(h: Handle) -> i64 {
    let raw = unsafe { h.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    match bigint::Big::i64_checked_from_sign_magnitude_bytes(raw) {
        Some(v) => v,
        None => trap_bigint_narrow(),
    }
}
#[cold]
#[inline(never)]
fn trap_bigint_narrow() -> ! {
    panic!("cdz-runtime: BigInt value out of range for the target integer type")
}
/// Read a BigInt leaf's raw sign-magnitude bytes as an `i128`, or `None` if the value exceeds i128 range
/// (needs the full `Big` path). Borrows the node's `raw` slice DIRECTLY — no `Big`, no limb `Vec`. A
/// null/missing node reads as the empty slice = canonical zero. The small-operand arithmetic fast path.
#[inline]
fn bigint_as_i128(h: Handle) -> Option<i128> {
    let raw = unsafe { h.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    bigint::Big::i128_from_sign_magnitude_bytes(raw)
}
/// Box an `i128` result as a BigInt leaf directly from its sign-magnitude bytes — no intermediate `Big`.
/// An `i128`'s bytes are ≤17 (`[sign] + ≤16 magnitude`), which exceeds `INLINE_RAW_CAP` (12) only for a
/// value needing >11 magnitude bytes; such a value falls back to the heap `Raw`. Byte-identical to
/// `box_bigint(&Big::from_i128(v))`.
#[inline]
fn box_bigint_i128(v: i128) -> Handle {
    let mut buf = [0u8; 17]; // sign + 16 LE magnitude bytes (i128 max)
    let n = bigint::Big::i128_to_sign_magnitude_bytes_into(v, &mut buf)
        .expect("17-byte buf fits any i128");
    if n <= INLINE_RAW_CAP {
        alloc_raw(Vec::new(), Raw::inline(&buf[..n]))
    } else {
        alloc_raw(Vec::new(), Raw::from(buf[..n].to_vec()))
    }
}
/// `bigint-add`/`-sub`/`-mul` — the total (never-trapping) arithmetic. FAST PATH: when both operands fit
/// `i128` (the common case — a runtime BigInt is a BigInt by TYPE, its magnitude usually small) and the
/// native `checked_*` op does not overflow, compute + box the `i128` result with NO limb `Vec` on either
/// operand (was 2 unbox Vecs + a result Vec; now just the result node). SLOW PATH: an operand out of i128
/// range, or an overflowing result, falls back to the full `Big` path — byte-identical either way (both
/// produce the canonical sign-magnitude leaf; guarded by the `num-bigint` differential + the i128-boundary
/// differential test).
fn op_bigint_add(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_add(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).add(&unbox_bigint(b)))
}
fn op_bigint_sub(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_sub(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).sub(&unbox_bigint(b)))
}
fn op_bigint_mul(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_mul(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).mul(&unbox_bigint(b)))
}
/// `bigint-div` — TRUNCATING integer division (quotient toward zero); TRAPS on a zero divisor (an
/// unbounded range does not give `n/0` a value — numeric-model.md). Returns the quotient.
fn op_bigint_div(a: Handle, b: Handle) -> Handle {
    // FAST PATH (mirrors add/sub/mul): both operands fit i128 (the common case — a runtime BigInt is a
    // BigInt by TYPE, magnitude usually small). Rust's `/` truncates toward zero — EXACTLY `divmod`'s
    // quotient (differential-verified byte-identical across all sign combos + i128 extremes). `checked_div`
    // returns `None` for the two non-representable cases — a ZERO divisor AND the `i128::MIN / -1` overflow
    // — and BOTH then fall through to the `Big` path, which produces the identical result (or traps on
    // zero via `divmod`'s `None`). So no separate zero-guard is needed: the fallback preserves the trap.
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(q) = x.checked_div(y) {
            return box_bigint_i128(q);
        }
    }
    match unbox_bigint(a).divmod(&unbox_bigint(b)) {
        Some((q, _r)) => box_bigint(&q),
        None => trap_bigint_div_zero(),
    }
}
#[cold]
#[inline(never)]
fn trap_bigint_div_zero() -> ! {
    panic!("cdz-runtime: BigInt division by zero")
}
/// `bigint-rem` — the REMAINDER of truncating division (`%`): `a - (a / b) * b`, so its sign is the
/// DIVIDEND's (numeric-model.md — `%` takes the dividend's sign, the companion of `bigint-div`'s
/// truncate-toward-zero). TRAPS on a zero divisor (same as `bigint-div`). `divmod` returns `(q, r)` with
/// exactly this remainder, so this is the `r` half — the whole reason `divmod` computes both at once.
fn op_bigint_rem(a: Handle, b: Handle) -> Handle {
    // FAST PATH: Rust's `%` takes the DIVIDEND's sign — EXACTLY `divmod`'s remainder (differential-verified
    // byte-identical). Like `div`, `checked_rem` returns `None` on a zero divisor and on `i128::MIN % -1`
    // (defined as 0 but the paired division overflows), both falling through to the `Big` path (identical
    // result, or the zero-divisor trap) — so no separate zero-guard is needed.
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_rem(y) {
            return box_bigint_i128(r);
        }
    }
    match unbox_bigint(a).divmod(&unbox_bigint(b)) {
        Some((_q, r)) => box_bigint(&r),
        None => trap_bigint_div_zero(),
    }
}
/// `bigint-cmp` — three-way compare: `-1`/`0`/`1` for `a < b`/`a = b`/`a > b` (the primitive the
/// comparison operators `<`/`>`/`=`/… lower to + a fixed compare). Compares the operands' canonical
/// sign-magnitude `raw` slices DIRECTLY (`Big::cmp_sign_magnitude_bytes`) — a comparison is READ-ONLY, so
/// it needs NO `Big` (no limb `Vec`): allocation-FREE, unlike the arithmetic ops which must build a
/// result. A null/mismatched node reads as the empty slice = canonical zero.
fn op_bigint_cmp(a: Handle, b: Handle) -> i64 {
    let av = unsafe { a.node_ref() };
    let bv = unsafe { b.node_ref() };
    let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
    let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
    match bigint::Big::cmp_sign_magnitude_bytes(as_, bs) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

// ─── Exact rational (Rational) — a NORMALIZED pair of BigInt handles ─────────────────────────────
// A `Rational` value is a 2-HANDLE node `[numerator, denominator]`, each child a BigInt leaf, kept in
// canonical NORMALIZED form: lowest terms (gcd-reduced), the sign on the numerator, the denominator
// strictly positive (`> 0`). So two equal rationals are byte-identical (`2/4` and `1/2` share one node
// shape), and `champ_eq`/`champ_hash` over the two child leaves compare by value. The runtime reuses the
// `bigint::Big` limb arithmetic for the component math; `op_dup`/`op_drop` already recurse into the two
// child handles (a rational is an ordinary 2-handle node), so refcounting needs no special case. A
// runtime Rational is built by `rational-of` from two BigInt handles (which it consumes: it reads both,
// normalizes, and re-boxes the canonical pair, dropping the inputs); the constant fold in the compiler
// never calls these (it emits the folded `num/den` value form directly).

/// Read a Rational node's `(num, den)` components as `i64`s DIRECTLY from the child leaves' raw bytes —
/// `None` if EITHER exceeds i64 range or the node is malformed. No `Big`, no limb `Vec`. The small-operand
/// fast path for the READ-ONLY `rational-cmp`: a runtime Rational built from i64 params (the common R3b
/// case) has i64 components, and then two i64 components cross-multiply into an i128 that CANNOT overflow
/// (|i64| · |i64| < 2¹²⁷), so the compare is exact native arithmetic with zero allocation.
fn rational_components_as_i64(h: Handle) -> Option<(i64, i64)> {
    let n = unsafe { h.node_ref() }?;
    if n.handles.len() != 2 {
        return None;
    }
    let read = |slot: usize| -> Option<i64> {
        let ch = n.handles.get(slot).copied()?;
        let raw = unsafe { ch.node_ref() }.map_or(&[][..], |cn| cn.raw.as_slice());
        bigint::Big::i64_checked_from_sign_magnitude_bytes(raw)
    };
    Some((read(0)?, read(1)?))
}

/// Read a Rational node's two children as `(num, den)` `Big`s. Total: a null/mismatched node reads as
/// `0/1` (deterministic, never a trap — the scalar-read discipline). Borrows the child leaves; does NOT
/// consume the handles.
fn unbox_rational(h: Handle) -> (bigint::Big, bigint::Big) {
    match unsafe { h.node_ref() } {
        Some(n) if n.handles.len() == 2 => (
            unbox_bigint(n.handles.get(0).copied().unwrap_or(Handle::NULL)),
            unbox_bigint(n.handles.get(1).copied().unwrap_or(Handle::NULL)),
        ),
        _ => (bigint::Big::zero(), bigint::Big::from_i64(1)),
    }
}

/// Box a NORMALIZED `(num, den)` pair as a Rational node — a 2-handle node holding the two BigInt leaves.
/// REQUIRES `den` already normalized (lowest terms, strictly positive) by the caller.
fn box_rational_normalized(num: &bigint::Big, den: &bigint::Big) -> Handle {
    alloc_raw(
        Handles::inline_from(&[box_bigint(num), box_bigint(den)]),
        Raw::inline(&[]),
    )
}

/// Normalize + box an i128 `(num, den)` Rational NATIVELY (no `Big`) — the small-operand arithmetic fast
/// path's write half. `den != 0` (the caller ensures). Reduces to lowest terms via an i128 gcd, moves the
/// sign onto the numerator (den strictly positive), then boxes each component via `box_bigint_i128`. Returns
/// `None` (→ caller falls back to the full `Big` path) if either component is `i128::MIN` (whose `abs`
/// overflows i128 — a value that anyway only arises from operands far outside the i64 fast-path domain).
/// Byte-identical to `box_rational_normalized(normalize_rational(Big(num), Big(den)))`.
fn rational_from_i128_pair(mut num: i128, mut den: i128) -> Option<Handle> {
    if num == i128::MIN || den == i128::MIN {
        return None; // abs would overflow — bail to the Big path (unreachable from i64-domain operands)
    }
    if den < 0 {
        num = -num;
        den = -den;
    }
    // i128 gcd (Euclid) over the magnitudes; gcd(0, d) = d.
    let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    let g = a as i128; // g > 0 (den != 0)
    Some(box_rational_node(
        box_bigint_i128(num / g),
        box_bigint_i128(den / g),
    ))
}

/// Box two already-BigInt-handle children as a Rational node (the shared node-build for both the `Big` and
/// the i128 fast paths). CONSUMES the two handles into the node's `handles`.
fn box_rational_node(num: Handle, den: Handle) -> Handle {
    alloc_raw(Handles::inline_from(&[num, den]), Raw::inline(&[]))
}

/// Normalize `(num, den)` → lowest terms, denominator strictly positive, sign on the numerator. REQUIRES
/// `den != 0` (the caller — `rational-of` — traps on a zero denominator before this). `0/d` → `0/1`.
fn normalize_rational(num: &bigint::Big, den: &bigint::Big) -> (bigint::Big, bigint::Big) {
    let g = num.gcd(den); // non-negative; gcd(0, d) = |d|
    let (mut n, _) = num.divmod(&g).expect("gcd is nonzero when den != 0");
    let (mut d, _) = den.divmod(&g).expect("gcd is nonzero when den != 0");
    if d.neg {
        n = n.neg();
        d = d.neg();
    }
    (n, d)
}

/// `rational-of(num, den)` — CONSTRUCT a normalized rational from two BigInt handles. Normalizes (gcd-
/// reduce, sign on numerator, denom > 0). A ZERO denominator has no value → TRAPS. CONSUMES both operand
/// handles (reads then drops them — the caller transfers ownership in, matching the compiler's emit).
fn op_rational_of(num: Handle, den: Handle) -> Handle {
    let (n, d) = (unbox_bigint(num), unbox_bigint(den));
    op_drop(num);
    op_drop(den);
    if d.is_zero() {
        trap_rational_zero_denom();
    }
    let (nn, nd) = normalize_rational(&n, &d);
    box_rational_normalized(&nn, &nd)
}
#[cold]
#[inline(never)]
fn trap_rational_zero_denom() -> ! {
    panic!("cdz-runtime: rational with zero denominator")
}
/// `rational-num(r)` / `rational-den(r)` — the numerator / denominator as a fresh BigInt handle (a DUP of
/// the child leaf, so the rational stays intact — the child is borrowed, the returned handle owned). A
/// null/mismatched node yields the `0/1` components.
fn op_rational_num(r: Handle) -> Handle {
    let (n, _) = unbox_rational(r);
    box_bigint(&n)
}
fn op_rational_den(r: Handle) -> Handle {
    let (_, d) = unbox_rational(r);
    box_bigint(&d)
}
/// `rational-add`/`-sub`/`-mul`/`-div` — exact rational arithmetic over two normalized operands, re-
/// normalized: `a/b + c/d = (ad+cb)/(bd)`, `- = (ad-cb)/(bd)`, `* = (ac)/(bd)`, `÷ = (ad)/(bc)`. All BORROW
/// their operands (unbox reads the child leaves without consuming) and return a FRESH normalized rational.
/// `÷` by `0/1` gives a zero denominator → TRAPS (the rational analogue of ÷0). Never overflow (BigInt).
fn op_rational_add(a: Handle, b: Handle) -> Handle {
    // FAST PATH: all four components fit i64. A cross-product `an·bd`/`bn·ad` is i64·i64 → fits i128; the
    // numerator SUM can reach ±2¹²⁷ so use `checked_add` (overflow → the `Big` path). The denominator
    // `ad·bd` is i64·i64 → fits i128. Byte-identical result; ~23/op → the result node + 2 leaves only.
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(num) = (an as i128 * bd as i128).checked_add(bn as i128 * ad as i128) {
            if let Some(h) = rational_from_i128_pair(num, ad as i128 * bd as i128) {
                return h;
            }
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd).add(&bn.mul(&ad));
    let den = ad.mul(&bd);
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
fn op_rational_sub(a: Handle, b: Handle) -> Handle {
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(num) = (an as i128 * bd as i128).checked_sub(bn as i128 * ad as i128) {
            if let Some(h) = rational_from_i128_pair(num, ad as i128 * bd as i128) {
                return h;
            }
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd).sub(&bn.mul(&ad));
    let den = ad.mul(&bd);
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
fn op_rational_mul(a: Handle, b: Handle) -> Handle {
    // `an·bn / ad·bd` — both products are i64·i64 → fit i128, no overflow possible, so no `checked` guard.
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(h) = rational_from_i128_pair(an as i128 * bn as i128, ad as i128 * bd as i128) {
            return h;
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let (n, d) = normalize_rational(&an.mul(&bn), &ad.mul(&bd));
    box_rational_normalized(&n, &d)
}
fn op_rational_div(a: Handle, b: Handle) -> Handle {
    // `an·bd / ad·bn` — both products i64·i64 → fit i128. A zero result-denominator (÷ by 0/1) TRAPS,
    // exactly as the `Big` path does (checked BEFORE the fast-path box, so the trap fires either way).
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        let den = ad as i128 * bn as i128;
        if den == 0 {
            trap_rational_zero_denom();
        }
        if let Some(h) = rational_from_i128_pair(an as i128 * bd as i128, den) {
            return h;
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd);
    let den = ad.mul(&bn);
    if den.is_zero() {
        trap_rational_zero_denom();
    }
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
/// `rational-cmp(a, b)` — three-way `-1`/`0`/`1` for `a < b`/`a = b`/`a > b`. Both normalized with a
/// strictly-positive denominator, so `a/b <=> c/d` ⇔ `a·d <=> c·b` (cross-multiply, direction preserved).
/// Borrows both operands. FAST PATH: when all four components fit `i64` (the common case — a Rational
/// built from i64 params), the cross-products `an·bd` and `bn·ad` fit `i128` without overflow (i64·i64 <
/// 2¹²⁷), so the compare is exact NATIVE arithmetic with NO `Big`/limb `Vec` (was 6/op: 4 unbox Vecs + 2
/// mul Vecs → 0). A component out of i64 range falls back to the full `Big` cross-multiply — same result.
fn op_rational_cmp(a: Handle, b: Handle) -> i64 {
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        // an/ad <=> bn/bd ⇔ an·bd <=> bn·ad (both dens > 0). i64·i64 fits i128 exactly.
        let (lhs, rhs) = (an as i128 * bd as i128, bn as i128 * ad as i128);
        return match lhs.cmp(&rhs) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        };
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    match an.mul(&bd).cmp(&bn.mul(&ad)) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

// ─── Positional array — the ONE runtime shape for TUPLE, RECORD, and LIST ───────────────
// Elements live in `handles`. Access by an out-of-bounds index into a valid array TRAPS; a null
// handle is benign (returns NULL / no-op).

fn op_arr_alloc(len: u32) -> Handle {
    // Normalize-on-construct (P1b): the empty array (arity 0) IS the unit value, and unit ALWAYS
    // inlines — no heap Node. A non-empty array still allocates its slots.
    if len == 0 {
        return imm_unit();
    }
    // A small (≤cap) tuple/record builds its slots INLINE (no transient heap Vec that would be copied
    // into the inline arm and freed) — the node is then just its Box (the inline-handles WIN for the
    // dominant ≤2-arity products). Wider arrays keep the heap Vec.
    if (len as usize) <= INLINE_HANDLES_CAP {
        return alloc_raw(Handles::inline_nulls(len as usize), Raw::from(Vec::new()));
    }
    alloc(vec![Handle::NULL; len as usize], Vec::new())
}
/// Write an element handle and return the array handle (for convenient threading). OOB into a valid
/// array traps; null is a no-op.
fn op_arr_set(arr: Handle, index: u32, elem: Handle) -> Handle {
    if is_immediate(arr) {
        return arr; // an immediate array (inline unit) has no slots; elem is stored, not deref'd
    }
    match unsafe { arr.node_mut() } {
        None => {}
        Some(n) => match n.handles.get_mut(index as usize) {
            Some(slot) => *slot = elem,
            None => trap_oob(),
        },
    }
    arr
}
fn op_arr_get(arr: Handle, index: u32) -> Handle {
    if is_immediate(arr) {
        trap_oob(); // an immediate array (inline unit) has 0 slots — any index is OOB
    }
    match unsafe { arr.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get(index as usize) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_arr_len(arr: Handle) -> u32 {
    if is_immediate(arr) {
        return 0; // inline unit has 0 elements
    }
    with_node(arr, 0, |n| n.handles.len() as u32)
}

// ─── Sum: a discriminant (in `raw`) plus a payload handle (in `handles`) ────────────────
// `sum-payload` is TOTAL (no runtime index): a mismatched node with no handle yields NULL.

fn op_sum_new(disc: u32, payload: Handle) -> Handle {
    // Build BOTH the 4-byte disc raw AND the 1-element handles INLINE (no transient heap Vec for
    // either) — a sum node is then just the node Box, 1 alloc instead of 2 (was 3 before inline-raw).
    alloc_raw(
        Handles::inline_from(&[payload]),
        Raw::inline(&disc.to_le_bytes()),
    )
}
fn op_sum_disc(h: Handle) -> u32 {
    if is_immediate(h) {
        return 0; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, 0, |n| read_disc(&n.raw))
}
/// The discriminant of a sum value FOR A DESCRIPTOR-GUIDED WALK (compare + render) — decodes an ALL-NULLARY
/// sum that was boxed as an Int IMMEDIATE (SOUNDNESS #43). A nullary variant boxes via `box-int` (enum-disc
/// → OP_BOX_INT); a small disc (0/1/2…) fixnum_fits, so `op_box_int` returns an immediate carrying the disc
/// as its int value, NOT a heap sum node. `op_sum_disc` returns 0 for ANY immediate (its documented cross-
/// kind-totality contract, relied on by the render/decode/WIT callers + pinned tests), so the shape-guided
/// Sum arms (value_cmp_shaped + value-encode) MUST decode the disc from the immediate's value here instead —
/// else every nullary key/element reads disc 0 (wrong sort order in to-list; wrong variant in render). A
/// payload-carrying variant is a real heap node → `op_sum_disc` reads its stored disc. Kept SEPARATE from
/// `op_sum_disc` on purpose: only the descriptor-walk callers know the operand is a sum (so an immediate is
/// an enum-disc, not a cross-kind int); `op_sum_disc`'s blanket-0 stays correct for its other callers.
fn sum_disc_shaped(h: Handle) -> u32 {
    if is_immediate(h) {
        // A nullary-sum enum-disc is boxed via `box-int`, so the immediate is INT-tagged; `imm_as_int` is
        // only valid for an int-tagged immediate (a unit/bool immediate would arithmetic-shift to a garbage
        // disc). GUARD on the int tag (PR#889 Copilot, defensive): a non-int immediate under a Sum shape is a
        // MALFORMED descriptor/value pairing — return `u32::MAX` (out of any `variants` range) so the caller's
        // `variants.get(disc)?` DECLINES cleanly (the descriptor-walk contract) rather than garbage-decoding.
        match imm_kind(h) {
            ImmKind::Int => imm_as_int(h) as u32,
            _ => u32::MAX,
        }
    } else {
        op_sum_disc(h)
    }
}
fn op_sum_payload(h: Handle) -> Handle {
    if is_immediate(h) {
        return Handle::NULL; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, Handle::NULL, |n| {
        n.handles.first().copied().unwrap_or(Handle::NULL)
    })
}

// ─── Value-form encode (index 62): render a runtime value to its canonical binary AST document ──
//
// The type-directed renderer the compiler bakes into a program (`sum_form_template` / the fixed
// hole-templates) can render a value of FIXED shape, but a RUNTIME RECURSIVE sum (a linked list, a
// tree — `(type IL (Cons (Tuple Int64 IL)) Nil)`) has unbounded depth, so no fixed template exists and
// the escape declined. This op walks such a value to its canonical value form — the binary AST codec
// document (`codec.rs`: header · leaf pool · struct table · root) — guided by a SHAPE DESCRIPTOR the
// compiler bakes as bytes. The runtime stays NOMINAL-AGNOSTIC: every NAME (the `:` frame, a variant
// head, `tuple`, `unit`, the type name) comes from the descriptor, never invented here; the runtime
// owns only the document ASSEMBLY (leaf dedup, struct indices, byte layout) — the error-prone part that
// hand-emitted wasm would get wrong. See `DESIGN-recursive-sum-escape-walker.md` (approach C).
//
// Shape descriptor wire format (a compiler-baked constant, read by `decode_descriptor`):
//   [ table_len:LEB ]( Shape )*table_len   [ root:LEB ]
// A descriptor is a TABLE of shapes + a root index; a shape references another by INDEX (tag 11 Ref),
// so a self-referential type is FINITE — the recursive payload position is a `Ref` back to the sum's
// table entry, and the value walk follows it only as deep as the runtime value actually nests. Each
// shape is a tag byte + per-tag operands (all counts/lengths unsigned LEB128):
//     0 Int | 1 Bool | 2 Float | 3 Str | 4 Bytes | 5 Unit
//     6 Tuple  [ n ][ elem: idx ]*n                       — each element is a table INDEX
//     7 List   [ elem: idx ]
//     8 Record [ n ]( [ name_len ][ name_utf8 ] [ field: idx ] )*n
//     9 Sum    [ n ]( [ head_len ][ head_utf8 ] [ payload: idx ] )*n       (nullary payload → a Unit idx)
//    10 Named  [ name_len ][ name_utf8 ] [ inner: idx ]   — the `(: <value> <name>)` frame (root only)
//    11 Ref    [ idx ]                                    — an alias to another table entry (recursion)
//    12 Set    [ elem: idx ]                              — 13 Map [ key: idx ][ val: idx ] — 14 Float32
//    15 Framed <TypeNode> [ inner: idx ]   where TypeNode = [ head_len ][ head_utf8 ] [ n ]( TypeNode )*n
//              — the `(: <value> <type-node>)` frame: an arbitrary (possibly NESTED) type node written
//                RECURSIVELY, so a nested element type shows (e.g. `(List (List Int64))`, `(Map Int64
//                (Set Int64))`). A leaf node has n=0 (a bare name). Used for a runtime collection result.
// (Every child position is an INDEX into the table, not an inline shape — that is what lets a cycle
// close: entry k's Sum names entry k as a payload, a finite 1-entry loop the value walk unfolds.)

/// The canonical binary-AST codec tags — kept in lock-step with `rcdzc::codec` (the native encoder this
/// reproduces byte-for-byte). A drift is caught by the `encode_matches_codec` cross-check in the native
/// suite (a runtime document is decoded by `rcdzc::codec::decode` and compared to the source tree).
mod doc {
    pub const SCHEMA_HEADER: [u8; 8] = *b"cdzast\x00\x01";
    pub const KIND_INT_POS_DEC: u8 = 0;
    pub const KIND_FLOAT: u8 = 6;
    pub const KIND_STR: u8 = 7;
    pub const KIND_BOOL_FALSE: u8 = 8;
    pub const KIND_BOOL_TRUE: u8 = 9;
    pub const KIND_NAME: u8 = 10;
    pub const KIND_BYTES: u8 = 11;
    // A Unicode-scalar CHAR leaf — the scalar UTF-8-encoded (LEB len + those 1-4 bytes, `write_bytes`
    // framing like a string body), matching cadenza-ast codec's `KIND_CHAR`. Char = bool-analog: int at
    // runtime, no distinct rep; this wire kind is only the RENDER form (a `#\c` char literal on decode).
    pub const KIND_CHAR: u8 = 13;
    // M2 native-compound-data CTOR-HEAD kinds — payloadless single kind-bytes (like KIND_BOOL_*), matching
    // cadenza-ast codec's KIND_*_CTOR / KIND_FIELD_PAIR / KIND_MEMBER. The head-first ctor leaf is the LIST
    // HEAD atom (children follow); the codec has NO canon pass, so build-order IS the content-address form.
    pub const KIND_LIST_CTOR: u8 = 20;
    pub const KIND_TUPLE_CTOR: u8 = 21;
    pub const KIND_RECORD_CTOR: u8 = 22;
    pub const KIND_MAP_CTOR: u8 = 23;
    pub const KIND_SET_CTOR: u8 = 24;
    pub const KIND_FIELD_PAIR: u8 = 25;
    pub const KIND_MEMBER: u8 = 26;
    pub const TAG_ATOM: u8 = 0;
    pub const TAG_LIST: u8 = 1;
}

/// Append `value` as unsigned LEB128 — the codec's `write_u64`, byte-identical.
fn doc_leb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A shape descriptor node — a value position's shape. Child positions are TABLE INDICES (`u32`), so a
/// recursive type closes as a finite cycle. `Named` carries the outer type name for the
/// `(: <value> <Type>)` frame; `Ref` is an alias to another table entry.
enum Shape {
    Int,
    Bool,
    /// A Unicode-scalar Char leaf — an i32 code-point, SEMANTICS-identical to `Int` (compare / eq / hash by
    /// the code-point integer, exactly as `Bool` is by its 0/1 int); only the RENDER differs (a `KIND_CHAR`
    /// char literal, not a decimal `KIND_INT`). Char = bool-analog: int at runtime, no distinct rep, a
    /// render tag only (descriptor tag 19, mirroring `Bool`'s tag 1). A char value is stored as an immediate
    /// int (the code-point), so it is read with `op_get_int` and boxed with `op_box_int`.
    Char,
    Float,
    Str,
    Bytes,
    Unit,
    /// An arbitrary-precision integer leaf (a runtime `BigInt`, `box_bigint`'s sign-magnitude Raw leaf).
    /// Rendered via the SAME `KIND_INT` codec leaf as `Int` — the leaf is already arbitrary-width (sign +
    /// big-endian magnitude bytes, NOT i64-bounded), so a BigInt needs NO new wire kind, only its own
    /// SHAPE tag (so the walk reads the value via `unbox_bigint`, not `op_get_int` which caps at i64).
    BigInt,
    /// An exact-rational leaf (a runtime `Rational`, `box_rational_normalized`'s normalized 2-BigInt-handle
    /// node). Rendered as a single `num/den` NAME leaf — the walk reads both components via `unbox_rational`
    /// and formats each `Big` decimal in the runtime (the codec's Int leaf formats decimal on the HOST, but
    /// a rational is ONE name leaf, so the runtime does it), matching the constant form `(: 1/2 Rational)`.
    Rational,
    /// A Float32 leaf — read with `get-float32` (an `f32`) and rendered as the f32's SHORTEST decimal,
    /// distinct from `Float` (Float64). A Float32 is stored 4-byte (`box-float32`), so its canonical value
    /// form is the f32's, not a promoted f64's (`0.1f32` renders `0.1`, not `0.10000000149011612`).
    Float32,
    // Child-index/field lists are `Arc<[…]>` (not `Vec`) so the descriptor-guided walks (value_cmp_shaped
    // in the Set/Map render `sort_unstable_by` hot path, value_encode, value_eq_shaped) can CHEAPLY clone
    // them (a refcount bump, not an O(n) copy) to drop the `&desc.table` borrow before pushing to the work
    // stack. Shape is in-memory-only (built by decode_shape from the wire descriptor, never serialized), so
    // this retype is hash-neutral. Field names are `Rc<str>` (deduped-friendly, cheap-clone) for the same
    // reason. (operator-commissioned cheap-clone audit, v-core-opt 2026-08-10.)
    Tuple(Rc<[u32]>),
    List(u32),
    Record(Rc<[(Rc<str>, u32)]>),
    Sum(Rc<[(Rc<str>, u32)]>),
    Named(Rc<str>, u32),
    Ref(u32),
    /// A SET over one element shape — rendered `(Set.of (list e1 … en))` with the elements in CANONICAL
    /// key-VALUE order (collections-and-text.md §A Set's canonical form). The runtime iterates the CHAMP
    /// in hash order, so the walk SORTS by the element's canonical scalar value (matching the compiler's
    /// `const_key_order`), NOT by hash or raw bytes. Only a SCALAR element shape is orderable-and-encodable.
    Set(u32),
    /// A MAP from a key shape to a value shape — rendered `(map (k1 v1) … (kn vn))` with entries in
    /// CANONICAL KEY order (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order),
    /// NOT hash order. Only a SCALAR KEY shape is orderable-and-encodable; the VALUE may be any encodable
    /// shape (the walk recurses on it). `(key_shape, value_shape)` table indices.
    Map(u32, u32),
    /// A `(: <value> <type-node>)` frame — like `Named` but the TYPE is an arbitrary (possibly NESTED)
    /// type node, not a single name. Carries a recursive [`TypeNode`] so a nested collection renders its
    /// full parametric type — e.g. `(List (List Int64))`, `(Map Int64 (List Bool))` — matching the
    /// constant-value form. The `u32` is the inner value shape index.
    Framed(TypeNode, u32),
    /// A MULTI-payload sum variant's payload — a tuple handle at run time (`arr` of the boxed payloads)
    /// whose elements render FLATTENED as the variant's children: `(Cons h t)`, NOT `(Cons (tuple h t))`.
    /// Read exactly like a `Tuple` (each element via `arr-get`) but the enclosing `Sum` walk splices the
    /// elements directly under the variant head instead of emitting a `tuple` form. Only a `Sum` variant's
    /// payload references a `Spread`; a genuine tuple VALUE stays a `Tuple`.
    Spread(Rc<[u32]>),
}

/// A compile-time-baked TYPE node for a `Framed` frame: `head` + child type nodes. A LEAF type
/// (`Int64`/`Bool`/`String`/`Unit`/a nominal name) has no children and renders as the bare name atom; a
/// PARAMETRIC type (`(List e)`, `(Map k v)`, `(Tuple …)`, `(Set e)`) renders `list([head, child…])`, each
/// child rendered recursively. The whole thing is compile-time-known (the result type), so the runtime
/// only re-emits it — it never inspects the runtime value to build the type.
struct TypeNode {
    head: String,
    children: Vec<TypeNode>,
}

/// Decode a [`TypeNode`]: `[ head_len ][ head_utf8 ] [ n_children:LEB ]( TypeNode )*n`.
/// Max nesting of a Framed type node. A genuine type is shallow — `(Map Int64 (List Bool))` is depth 2,
/// and the compiler bakes only such well-formed nodes — so a cap far above any real type still declines a
/// MALFORMED descriptor whose TypeNode nests thousands deep before it overflows the native/wasm call
/// stack. WITHOUT this, `decode_type_node`'s recursion is bounded only by the byte length (each level is
/// just `[name_len=0][n_children=1]` = 2 bytes), so a ~200 KB descriptor crashes the guest — violating
/// value-encode's "never a trap" totality contract (a compiler-baked descriptor is always shallow, but
/// the escape op must DECLINE any input, not abort).
const TYPE_NODE_DEPTH_CAP: u32 = 256;

fn decode_type_node(d: &[u8], pos: &mut usize, depth: u32) -> Option<TypeNode> {
    if depth > TYPE_NODE_DEPTH_CAP {
        return None; // a malformed descriptor's runaway TypeNode nesting — decline, don't overflow
    }
    let head = desc_name(d, pos)?;
    let n = desc_leb(d, pos)?;
    // `reserve_cap`: clamp an untrusted child count to remaining bytes so a malformed TypeNode can't
    // `with_capacity`-abort (each child is ≥1 byte).
    let mut children = Vec::with_capacity(reserve_cap(n, d, *pos));
    for _ in 0..n {
        children.push(decode_type_node(d, pos, depth + 1)?);
    }
    Some(TypeNode { head, children })
}

/// The decoded descriptor: the shape table + the root index. A child index into `table` is followed by
/// the value walk (with a depth cap as a malformed-descriptor backstop).
struct Descriptor {
    table: Vec<Shape>,
    root: u32,
}

fn desc_leb(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *d.get(*pos)?;
        *pos += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn desc_name(d: &[u8], pos: &mut usize) -> Option<String> {
    let len = desc_leb(d, pos)? as usize;
    let bytes = d.get(*pos..*pos + len)?;
    *pos += len;
    core::str::from_utf8(bytes).ok().map(String::from)
}

/// A pre-reservation capacity for a count `n` decoded from UNTRUSTED descriptor bytes, CLAMPED to the
/// bytes remaining after `pos`. Every element decoded from a count consumes ≥1 byte, so a legitimate `n`
/// never exceeds `d.len() - pos`; clamping turns a bogus huge LEB (e.g. from a random/malformed
/// descriptor) into a small reservation the `?`-guarded loop then fails out of, instead of
/// `Vec::with_capacity(n)` trying to reserve gigabytes and ABORTING the guest (the value-encode escape's
/// "never a trap" totality contract). Costs nothing on well-formed input (the clamp never binds there).
#[inline]
fn reserve_cap(n: u64, d: &[u8], pos: usize) -> usize {
    (n as usize).min(d.len().saturating_sub(pos))
}

fn decode_shape(d: &[u8], pos: &mut usize) -> Option<Shape> {
    let tag = *d.get(*pos)?;
    *pos += 1;
    Some(match tag {
        0 => Shape::Int,
        1 => Shape::Bool,
        2 => Shape::Float,
        3 => Shape::Str,
        4 => Shape::Bytes,
        5 => Shape::Unit,
        6 => {
            let n = desc_leb(d, pos)?;
            let mut elems = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                elems.push(desc_leb(d, pos)? as u32);
            }
            Shape::Tuple(elems.into())
        }
        7 => Shape::List(desc_leb(d, pos)? as u32),
        8 => {
            let n = desc_leb(d, pos)?;
            let mut fields = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                let name: Rc<str> = desc_name(d, pos)?.into();
                fields.push((name, desc_leb(d, pos)? as u32));
            }
            Shape::Record(fields.into())
        }
        9 => {
            let n = desc_leb(d, pos)?;
            let mut variants = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                let head: Rc<str> = desc_name(d, pos)?.into();
                variants.push((head, desc_leb(d, pos)? as u32));
            }
            Shape::Sum(variants.into())
        }
        10 => {
            let name: Rc<str> = desc_name(d, pos)?.into();
            Shape::Named(name, desc_leb(d, pos)? as u32)
        }
        11 => Shape::Ref(desc_leb(d, pos)? as u32),
        12 => Shape::Set(desc_leb(d, pos)? as u32),
        13 => {
            let key = desc_leb(d, pos)? as u32;
            let val = desc_leb(d, pos)? as u32;
            Shape::Map(key, val)
        }
        14 => Shape::Float32,
        15 => {
            // Framed: <TypeNode> [ inner: idx ]  where TypeNode = [ head ][ n ]( TypeNode )*n (recursive).
            let type_node = decode_type_node(d, pos, 0)?;
            Shape::Framed(type_node, desc_leb(d, pos)? as u32)
        }
        16 => {
            // Spread: [ n ]( idx )*n — same wire shape as Tuple (tag 6), a distinct tag so the Sum walk
            // knows to splice the elements FLAT under the variant head rather than wrap them in `tuple`.
            let n = desc_leb(d, pos)?;
            let mut elems = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                elems.push(desc_leb(d, pos)? as u32);
            }
            Shape::Spread(elems.into())
        }
        17 => Shape::BigInt, // arbitrary-precision integer leaf (a runtime BigInt), rendered as KIND_INT
        18 => Shape::Rational, // exact-rational leaf (a 2-BigInt-handle node), rendered as a num/den name
        19 => Shape::Char, // Unicode-scalar Char leaf — int at runtime, rendered as a KIND_CHAR char literal
        _ => return None,
    })
}

fn decode_descriptor(d: &[u8]) -> Option<Descriptor> {
    let mut pos = 0usize;
    let n = desc_leb(d, &mut pos)?;
    // `reserve_cap`: a bogus huge table count from a malformed descriptor must not `with_capacity`-abort;
    // each shape is ≥1 byte so a real `n` ≤ remaining bytes, and the `?`-loop fails out of an overlong one.
    let mut table = Vec::with_capacity(reserve_cap(n, d, pos));
    for _ in 0..n {
        table.push(decode_shape(d, &mut pos)?);
    }
    let root = desc_leb(d, &mut pos)? as u32;
    if root as usize >= table.len() {
        return None;
    }
    Some(Descriptor { table, root })
}

runtime_local! {
    /// REUSED completed-struct stack for `encode_value`'s walk (the `out` results Vec) — the companion of
    /// `ENCODE_BUILDER`. `encode_value`'s `out` grew from zero every call (a fresh `Vec<u32>` per encode);
    /// caching it here + `clear()`ing per call retains capacity, so after the first walk it never
    /// reallocates. Safe: single-threaded, the walk is iterative + never re-enters `encode_value`, so the
    /// borrow never nests.
    static ENCODE_OUT: core::cell::RefCell<Vec<u32>> = core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED WORK stack for `encode_value`'s iterative walk — the companion of `ENCODE_OUT`. The `work`
    /// stack grows O(depth) (each container's assembler stays on the stack while its children are visited,
    /// so a Cons-list's depth is O(N)), so a fresh `Vec<EncodeWork>` per encode paid an O(log depth)
    /// grow-chain of reallocs EVERY call. Now that `EncodeWork` is `'static` (its formerly-borrowed key/
    /// name/type-node fields are re-derived from `desc` at process time), the stack caches here +
    /// `clear()`s per call, retaining capacity → grows ONCE to the high-water mark then refills allocation-
    /// FREE. Measured: value_encode of a 50-node list dropped from ~13/encode toward the output-Vec floor.
    /// Safe: single-threaded, iterative, never re-enters `encode_value` — the borrow never nests.
    static ENCODE_WORK: core::cell::RefCell<Vec<EncodeWork>> = core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED `DocBuilder` for `op_value_encode_form` — the value-form escape (op 62) is the hot
    /// host-boundary path (every collection/compound result crossing to the host runs one encode), and a
    /// fresh `DocBuilder::default()` per call grew its `leaves`/`structs`/`child_pool`/`name_index` pools
    /// FROM ZERO every time (~7 realloc doublings each for a modest value = the bulk of the residual
    /// ~43-alloc floor). Caching one builder thread-locally + `reset()`ing it (clear, capacity retained)
    /// makes the pool growth pay ONCE to the high-water mark, then every later encode refills allocation-
    /// FREE — the same alloc-elision as `HASH_SCRATCH`/`EQ_SCRATCH`. Safe: the runtime is single-threaded
    /// and `op_value_encode_form` never re-enters itself (the walk is iterative), so the borrow never
    /// nests. The document bytes are UNCHANGED — reuse only affects allocation, not the emitted output.
    static ENCODE_BUILDER: core::cell::RefCell<DocBuilder> =
        core::cell::RefCell::new(DocBuilder::new_const());
}

runtime_local! {
    /// SINGLE-ENTRY cache of the LAST decoded descriptor: `(descriptor bytes, decoded Descriptor)`.
    /// `decode_descriptor` allocates a `Vec<Shape>` table + a nested Vec per Tuple/Record/Sum/Spread shape
    /// + a `String` per Named/field/variant — a fixed per-call cost that was paid FRESH on every encode
    /// (measured 6 of the ~19 residual allocs for the IntList descriptor, ~31%). But an escape SITE always
    /// crosses the boundary with the SAME compiler-baked descriptor bytes (an escape in a loop re-encodes
    /// under one descriptor), so a 1-entry cache keyed by the byte slice hits ~every call after the first:
    /// on a hit the decode is skipped entirely (0 allocs); on a miss (first call, or a different escape
    /// site interleaved) it decodes + replaces the entry (1 alloc for the cloned key + the decode). The
    /// bytes are the cache key (a `Descriptor` decoded from identical bytes IS identical — the decode is a
    /// pure function of the bytes), so a hit is always correct. Safe: single-threaded; `op_value_encode_
    /// form` clones out / uses the cached `Descriptor` under one borrow and never re-enters itself.
    static DESCRIPTOR_CACHE: core::cell::RefCell<Option<(Vec<u8>, Descriptor)>> =
        core::cell::RefCell::new(None);
}

/// The document builder — a growing leaf pool + struct table, with leaf DEDUP (a repeated name/int
/// collapses to one pool entry, matching the canonical arenas the native encoder is handed). Each
/// `push_*` returns the entry's absolute index; `finish(root)` serializes to the codec document.
#[derive(Default)]
struct DocBuilder {
    leaves: Vec<DocLeaf>,
    structs: Vec<DocStruct>,
    /// Flat arena for every `List` struct's children: a `List` records a `(start, len)` RANGE into this
    /// one pool instead of owning a per-node `Vec<u32>`. Turns N per-compound-node small-Vec allocations
    /// into amortized growth of a single shared Vec (value-encode of a deep value was ~1.3 allocs/node,
    /// dominated by these per-node child Vecs). Children of DIFFERENT lists never interleave: each `list`
    /// call appends its children contiguously and the walk completes one struct before the next.
    child_pool: Vec<u32>,
    /// Name → leaf-index, so `name_leaf`'s dedup is O(log N) not a linear scan of ALL leaves. Without it,
    /// a value with K DISTINCT names (a WIDE record's fields, a many-variant sum's heads) makes the K-th
    /// `name_leaf` scan ~K prior leaves → O(K²) encode (measured: a 3200-field record took ~183 ms vs the
    /// linear ~9 ms). Repeated names (the `Cons`/`tuple` heads in a long list) were already O(1) — the
    /// scan short-circuits on the first match near the front — but DISTINCT names were the quadratic case.
    name_index: alloc::collections::BTreeMap<String, u32>,
}
enum DocLeaf {
    Name(String),
    /// A SCALAR (i64-bounded) integer leaf — stores the raw `i64`, NOT a heap magnitude `Vec`. The
    /// canonical `[sign][big-endian magnitude, leading-zeros-stripped]` wire form is derived directly
    /// into the pre-sized output at `finish` time (the magnitude is ≤8 stack bytes), so a scalar int
    /// leaf allocates NOTHING — this is the dominant leaf in every escaped value (each list/tuple/record
    /// int emitted one `Vec<u8>` before, ~50 of ~92 allocs for a 50-int list). Byte-identical to the old
    /// `Int(neg, be_mag)` form. An arbitrary-width BigInt (>i64) still uses `Int` (a real heap magnitude).
    IntScalar(i64),
    Int(bool, Vec<u8>), // (negative, big-endian magnitude) — BigInt / arbitrary width only
    Bool(bool),
    /// A Unicode-scalar Char leaf. Wire form is `KIND_CHAR` + the scalar UTF-8-encoded (LEB len + 1-4
    /// bytes), byte-identical to cadenza-ast codec's `Leaf::Char` framing.
    Char(char),
    // UTF-8 body / raw byte payload stored as `Raw` (inline for ≤INLINE_RAW_CAP bytes — the common short
    // string/key case — else heap), so a SHORT string/bytes leaf allocates NOTHING (a JSON-dictionary key
    // "k00", a small tag) instead of a per-leaf `Vec<u8>`. `Raw` owns its bytes (no lifetime coupling to the
    // source node), so the pooled leaf stays `'static`. `finish` reads `.as_slice()` — storage-transparent.
    Str(Raw),   // UTF-8 body verbatim (the runtime String's raw bytes)
    Bytes(Raw), // raw byte payload verbatim (the runtime Bytes value, rope flattened)
    Float {
        negative: bool,
        exponent: i64,
        significand: Vec<u8>,
    }, // exact decimal (from f64), big-endian mag
    /// A payloadless M2 ctor-head leaf — stores its `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER` byte
    /// (20-26). Wire form is that single kind byte (no body), like `Bool`. The head-first list-head atom for
    /// a native compound value; DEDUPED by `ctor_leaf` (matching cadenza-ast `Builder::leaf`'s general dedup).
    Ctor(u8),
}
enum DocStruct {
    Atom(u32),
    /// A list struct: its children are `child_pool[start .. start + len]` (a RANGE into the builder's
    /// shared arena, not an owned Vec).
    List {
        start: u32,
        len: u32,
    },
}

/// The canonical big-endian magnitude of a scalar `i64`, leading zeros stripped (empty for zero), into a
/// STACK buffer — the codec's `KIND_INT` magnitude for an i64-bounded value. Returns `(negative, &mag)`
/// borrowing `buf`. `unsigned_abs` handles `i64::MIN` without overflow (magnitude `80 00…00`). The
/// `DocLeaf::IntScalar` write path uses this to emit the same bytes the old heap-`Vec` form did, with NO
/// allocation — the write is `out.extend_from_slice(mag)` straight from the stack.
#[inline]
fn i64_be_magnitude(v: i64, buf: &mut [u8; 8]) -> (bool, &[u8]) {
    *buf = v.unsigned_abs().to_be_bytes();
    let start = buf.iter().position(|&b| b != 0).unwrap_or(buf.len());
    // Zero carries an EMPTY magnitude and is never negative on the wire (matches the old `int_leaf` +
    // `DocLeaf::Int`'s finish rule, and `bigint_leaf`'s canonical zero).
    let mag = &buf[start..];
    (v < 0 && !mag.is_empty(), mag)
}

impl DocBuilder {
    /// A const-constructible EMPTY builder — the initializer for the reused `ENCODE_BUILDER` thread-local
    /// (a `const {}` thread-local body needs a const init; `#[derive(Default)]`'s `default()` is not const).
    /// Every field's empty form is a const fn (`Vec::new`/`BTreeMap::new`).
    const fn new_const() -> DocBuilder {
        DocBuilder {
            leaves: Vec::new(),
            structs: Vec::new(),
            child_pool: Vec::new(),
            name_index: alloc::collections::BTreeMap::new(),
        }
    }
    /// Clear every pool for REUSE across encodes — retains each buffer's CAPACITY (`Vec::clear` /
    /// `BTreeMap::clear` free no backing store), so after the first encode grows them to the high-water
    /// mark, subsequent encodes refill without reallocating. Called by `op_value_encode_form` on the
    /// reused `ENCODE_BUILDER` before each walk. Dropping the old `DocLeaf` entries frees their owned
    /// Strings/byte Vecs (a name/str/bytes/float leaf) — only the SPINE Vecs' capacity is retained.
    fn reset(&mut self) {
        self.leaves.clear();
        self.structs.clear();
        self.child_pool.clear();
        self.name_index.clear();
    }
    fn name_leaf(&mut self, name: &str) -> u32 {
        // Dedup names to a single leaf. HYBRID, so the common encode pays ZERO extra allocation:
        //  • SMALL regime (few distinct names — the norm: `Cons`/`Nil`/`tuple`/`record`/`map`/`:`/keys):
        //    scan the existing `DocLeaf::Name` entries directly. Allocation-FREE (the name String lives
        //    only in the leaf, no duplicate map key) and fast — the scan short-circuits on the first match
        //    near the front, so a repeated head is O(1).
        //  • LARGE regime (many DISTINCT names — a wide record's fields, a many-variant sum): once the
        //    NAME leaf count crosses `NAME_INDEX_THRESHOLD` the linear scan would go O(N²) (a 3200-field
        //    record took 183 ms), so build `name_index` ONCE from the leaves seen so far and use the
        //    BTreeMap (O(log N)) thereafter (~15 ms). Byte-identical either way — a repeated name resolves
        //    to its FIRST-inserted index in both.
        const NAME_INDEX_THRESHOLD: u32 = 16;
        if self.name_index.is_empty() {
            let mut name_count = 0u32;
            for (i, l) in self.leaves.iter().enumerate() {
                if let DocLeaf::Name(n) = l {
                    if n == name {
                        return i as u32;
                    }
                    name_count += 1;
                }
            }
            let i = self.leaves.len() as u32;
            self.leaves.push(DocLeaf::Name(String::from(name)));
            if name_count + 1 > NAME_INDEX_THRESHOLD {
                // Crossed the threshold — index every name leaf ONCE; the map owns dedup from here.
                for (idx, l) in self.leaves.iter().enumerate() {
                    if let DocLeaf::Name(n) = l {
                        self.name_index.insert(n.clone(), idx as u32);
                    }
                }
            }
            return i;
        }
        // Large regime: the map owns the dedup (O(log N)).
        if let Some(&i) = self.name_index.get(name) {
            return i;
        }
        let i = self.leaves.len() as u32;
        self.leaves.push(DocLeaf::Name(String::from(name)));
        self.name_index.insert(String::from(name), i);
        i
    }
    fn int_leaf(&mut self, v: i64) -> u32 {
        // Store the raw `i64` — the canonical `[sign][big-endian magnitude, leading-zeros-stripped]` wire
        // form is derived directly into the output at `finish` (a ≤8-byte stack magnitude), so a scalar int
        // leaf allocates NO heap Vec. Byte-IDENTICAL to the old `Int(v<0, be_mag_stripped)` form.
        self.leaves.push(DocLeaf::IntScalar(v));
        (self.leaves.len() - 1) as u32
    }
    /// A BigInt leaf — the SAME `KIND_INT` codec leaf as `int_leaf`, but for an arbitrary-precision value.
    /// `Big::to_sign_magnitude_bytes` yields `[sign][LE magnitude…]` (trailing zeros stripped); the codec's
    /// `DocLeaf::Int` wants (negative, BIG-endian magnitude, leading zeros stripped), so drop the sign
    /// byte, reverse to big-endian, and trim leading zeros. Zero → empty magnitude (positive), matching
    /// `int_leaf`'s canonical zero. No i64 bound — the magnitude is however many bytes the value needs.
    fn bigint_leaf(&mut self, b: &bigint::Big) -> u32 {
        let sm = b.to_sign_magnitude_bytes(); // [sign][LE mag…]
        let neg = sm.first().copied().unwrap_or(0) != 0;
        let mut magnitude: Vec<u8> = sm.get(1..).unwrap_or(&[]).iter().rev().copied().collect();
        while magnitude.first() == Some(&0) {
            magnitude.remove(0);
        }
        // A zero magnitude is never negative on the wire (matches `int_leaf` + `DocLeaf::Int`'s finish rule).
        let neg = neg && !magnitude.is_empty();
        self.leaves.push(DocLeaf::Int(neg, magnitude));
        (self.leaves.len() - 1) as u32
    }
    fn bool_leaf(&mut self, b: bool) -> u32 {
        self.leaves.push(DocLeaf::Bool(b));
        (self.leaves.len() - 1) as u32
    }
    /// A Unicode-scalar Char leaf (`doc::KIND_CHAR`) — the render form of an int-repped char value. Not
    /// deduped (like `bool_leaf`/`int_leaf`): the decoder re-interns on read.
    fn char_leaf(&mut self, c: char) -> u32 {
        self.leaves.push(DocLeaf::Char(c));
        (self.leaves.len() - 1) as u32
    }
    /// An M2 ctor-head leaf (`doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER`, 20-26) — the payloadless
    /// head atom of a native compound value. DEDUPED to its FIRST-inserted id (matching cadenza-ast
    /// `Builder::leaf`'s general `leaf_index` dedup, which `const_value_ast` uses via `atom_leaf`), so a
    /// value with repeated ctors (a list-of-tuples) byte-matches `const_value_ast`. Only ≤7 distinct ctor
    /// kinds exist, so the linear scan is effectively O(1).
    fn ctor_leaf(&mut self, kind: u8) -> u32 {
        for (i, l) in self.leaves.iter().enumerate() {
            if let DocLeaf::Ctor(k) = l
                && *k == kind
            {
                return i as u32;
            }
        }
        self.leaves.push(DocLeaf::Ctor(kind));
        (self.leaves.len() - 1) as u32
    }
    /// A string leaf — the UTF-8 body verbatim (the codec's `KIND_STR`, `write_bytes` = LEB len + bytes,
    /// identical framing to a `Name` leaf but a distinct kind). Not deduped (like `int_leaf`/`bool_leaf`):
    /// the codec DECODER re-interns leaves on read, so a repeated string in the pool is harmless. Takes a
    /// BORROWED slice + stores it as `Raw` (inline for a short string — no heap alloc; the leaf owns its
    /// bytes so no lifetime coupling to the source node).
    fn str_leaf(&mut self, bytes: &[u8]) -> u32 {
        self.leaves.push(DocLeaf::Str(Raw::from_slice(bytes)));
        (self.leaves.len() - 1) as u32
    }
    /// A bytes leaf — the raw byte payload verbatim (the codec's `KIND_BYTES`, `write_bytes` = LEB len +
    /// bytes, same framing as a `Str`/`Name` leaf, distinct kind). Not deduped (like `str_leaf`). Borrowed
    /// slice → `Raw` (inline for a short payload — no heap alloc).
    fn bytes_leaf(&mut self, bytes: &[u8]) -> u32 {
        self.leaves.push(DocLeaf::Bytes(Raw::from_slice(bytes)));
        (self.leaves.len() - 1) as u32
    }
    /// A float leaf — the EXACT decimal `(-1)^neg · significand · 10^exponent` the codec's `KIND_FLOAT`
    /// stores. Converts the runtime `f64` to that decimal by a byte-for-byte PORT of the compiler's
    /// `Decimal::from_f64` (rcdzc `ast.rs`): `{:e}` shortest round-tripping text → sign, digit string,
    /// base-10 exponent, then a base-10→base-256 Horner conversion of the digits (no BigInt — plain
    /// `Vec<u8>`, so `no_std`-portable). `None` for a NON-FINITE float (`nan`/`inf`), matching
    /// `from_f64` (a non-finite float has no exact decimal / no written form) — the walker declines.
    fn float_leaf(&mut self, f: f64) -> Option<u32> {
        if !f.is_finite() {
            return None;
        }
        // A WHOLE float renders its FULL exact expansion (`{f:.0}`, matches scalar display_float + rust);
        // a non-whole keeps `{:e}` (shortest == written form; `{f:.0}` would round the fraction away).
        let text = if is_whole_f64(f) {
            format!("{f:.0}")
        } else {
            format!("{f:e}")
        };
        self.float_leaf_from_sci(&text)
    }
    /// A Float32 leaf — the f32's SHORTEST round-tripping decimal (via `{:e}` on the `f32`, NOT a
    /// promoted f64 whose shortest decimal differs — `0.1f32` → `"1e-1"` not `"1.0000000149…e-1"`). Same
    /// `KIND_FLOAT` encoding as `float_leaf`; declines a non-finite f32.
    fn float32_leaf(&mut self, f: f32) -> Option<u32> {
        if !f.is_finite() {
            return None;
        }
        self.float_leaf_from_sci(&format!("{f:e}"))
    }
    /// Build a `KIND_FLOAT` `DocLeaf::Float` from a `[-]D[.DDDD]eEXP` scientific-notation string (the
    /// `{:e}` form of an f32 or f64): parse sign / digit string / base-10 exponent, then a base-10→
    /// base-256 Horner conversion of the digits (no BigInt — `no_std`-portable). Shared by both float
    /// widths so the exact decimal is the value's OWN shortest form. `None` on a malformed string.
    fn float_leaf_from_sci(&mut self, sci: &str) -> Option<u32> {
        let (negative, rest) = match sci.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, sci),
        };
        let (mantissa, exp10): (&str, i64) = match rest.split_once('e') {
            Some((m, e)) => (m, e.parse().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, fr)) => (i, fr),
            None => (mantissa, ""),
        };
        let mut digits = String::from(int_part);
        digits.push_str(frac_part);
        let exponent = exp10 - frac_part.len() as i64; // fold fractional digits into the exponent
        // base-10 digits → little-endian base-256 magnitude (Horner: acc = acc*10 + d).
        let mut mag: Vec<u8> = Vec::new();
        for ch in digits.bytes() {
            if !ch.is_ascii_digit() {
                return None;
            }
            let mut carry = (ch - b'0') as u32;
            for byte in mag.iter_mut() {
                let v = (*byte as u32) * 10 + carry;
                *byte = (v & 0xff) as u8;
                carry = v >> 8;
            }
            while carry > 0 {
                mag.push((carry & 0xff) as u8);
                carry >>= 8;
            }
        }
        // strip most-significant zeros → big-endian minimal magnitude, empty iff zero.
        while mag.last() == Some(&0) {
            mag.pop();
        }
        mag.reverse();
        self.leaves.push(DocLeaf::Float {
            negative,
            exponent,
            significand: mag,
        });
        Some((self.leaves.len() - 1) as u32)
    }
    fn atom(&mut self, leaf: u32) -> u32 {
        self.structs.push(DocStruct::Atom(leaf));
        (self.structs.len() - 1) as u32
    }
    /// Record a `List` struct whose children are `children` — appended CONTIGUOUSLY to the shared
    /// `child_pool` (no per-node Vec). Takes a slice so callers pass a stack array (`&[a, b]`) or an
    /// existing slice (`&out[base..]`) without allocating a temporary Vec.
    fn list(&mut self, children: &[u32]) -> u32 {
        let start = self.child_pool.len() as u32;
        self.child_pool.extend_from_slice(children);
        self.structs.push(DocStruct::List {
            start,
            len: children.len() as u32,
        });
        (self.structs.len() - 1) as u32
    }
    /// Record a `List` whose first child is `head` and remaining children are `tail` — the assembler
    /// shape (`head` + the completed sub-results in `out[base..]`), building the range directly in the
    /// pool with NO temporary Vec.
    fn list_head_tail(&mut self, head: u32, tail: &[u32]) -> u32 {
        let start = self.child_pool.len() as u32;
        self.child_pool.push(head);
        self.child_pool.extend_from_slice(tail);
        self.structs.push(DocStruct::List {
            start,
            len: 1 + tail.len() as u32,
        });
        (self.structs.len() - 1) as u32
    }
    /// Render a [`TypeNode`] to a struct index (recursive): a LEAF type (no children) → the bare name
    /// atom; a PARAMETRIC type → `list([head-atom, child…])`, each child rendered recursively. Builds the
    /// `(: value <type>)` frame's type position for a `Framed` — handles arbitrary nesting like
    /// `(List (List Int64))` / `(Map Int64 (List Bool))`.
    fn render_type_node(&mut self, tn: &TypeNode) -> u32 {
        let head_leaf = self.name_leaf(&tn.head);
        let head_atom = self.atom(head_leaf);
        if tn.children.is_empty() {
            head_atom
        } else {
            let child_structs: Vec<u32> = tn
                .children
                .iter()
                .map(|c| self.render_type_node(c))
                .collect();
            self.list_head_tail(head_atom, &child_structs)
        }
    }
    fn finish(&self, root: u32) -> Vec<u8> {
        // Pre-size the output so serializing a large document doesn't realloc-churn (grow-once, the same
        // discipline as the leaf/struct/child pools). Cheap UPPER-BOUND estimate in one pass: header +
        // counts/root LEBs, per leaf a kind byte + a ≤10-byte length/exponent field + its payload bytes,
        // per struct a tag + ≤5-byte LEB, and ≤5 bytes per pooled child index. An over-estimate only wastes
        // a little transient capacity; it never truncates (the writers still push). MEASURED −8 reallocs/
        // encode on a 50-node list, −12 on a 1000-entry map.
        let leaf_bytes: usize = self
            .leaves
            .iter()
            .map(|l| match l {
                DocLeaf::IntScalar(_) => 11 + 8, // kind + ≤10-byte len LEB + ≤8 magnitude bytes
                DocLeaf::Int(_, mag) => 11 + mag.len(),
                DocLeaf::Bool(_) => 1,
                DocLeaf::Char(_) => 1 + 1 + 4, // kind + LEB len + ≤4 UTF-8 scalar bytes
                DocLeaf::Ctor(_) => 1,         // payloadless single kind byte
                DocLeaf::Name(n) => 11 + n.len(),
                DocLeaf::Str(b) | DocLeaf::Bytes(b) => 11 + b.len(),
                DocLeaf::Float { significand, .. } => 20 + significand.len(),
            })
            .sum();
        let est = doc::SCHEMA_HEADER.len()
            + 20 // the two count LEBs + root LEB, generous
            + leaf_bytes
            + self.structs.len() * 6 // tag + LEB per struct
            + self.child_pool.len() * 5; // ≤5-byte LEB per pooled List child
        let mut out = Vec::with_capacity(est);
        out.extend_from_slice(&doc::SCHEMA_HEADER);
        doc_leb(&mut out, self.leaves.len() as u64);
        for leaf in &self.leaves {
            match leaf {
                DocLeaf::IntScalar(v) => {
                    // Derive the canonical `[sign][BE magnitude]` on the STACK (no heap Vec) and write the
                    // SAME bytes the `Int` arm below writes for the equivalent value. Kinds (sign<<0 offset):
                    // pos-dec = 0, neg-dec = 3 (codec KIND_INT_*); zero → empty magnitude, positive kind.
                    let mut buf = [0u8; 8];
                    let (is_neg, mag) = i64_be_magnitude(*v, &mut buf);
                    out.push(if is_neg {
                        doc::KIND_INT_POS_DEC + 3
                    } else {
                        doc::KIND_INT_POS_DEC
                    });
                    doc_leb(&mut out, mag.len() as u64);
                    out.extend_from_slice(mag);
                }
                DocLeaf::Int(neg, mag) => {
                    // Zero carries an empty magnitude and the POSITIVE kind (never negative-zero).
                    let is_neg = *neg && !mag.is_empty();
                    // Kinds are (sign<<0 offset): pos-dec = 0, neg-dec = 3 (see codec KIND_INT_*).
                    out.push(if is_neg {
                        doc::KIND_INT_POS_DEC + 3
                    } else {
                        doc::KIND_INT_POS_DEC
                    });
                    doc_leb(&mut out, mag.len() as u64);
                    out.extend_from_slice(mag);
                }
                DocLeaf::Bool(b) => out.push(if *b {
                    doc::KIND_BOOL_TRUE
                } else {
                    doc::KIND_BOOL_FALSE
                }),
                DocLeaf::Char(c) => {
                    // KIND_CHAR + the scalar UTF-8-encoded (LEB len + 1-4 bytes) — the `write_bytes` framing
                    // (like a Str body), byte-identical to cadenza-ast codec's `Leaf::Char` encode.
                    out.push(doc::KIND_CHAR);
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    doc_leb(&mut out, s.len() as u64);
                    out.extend_from_slice(s.as_bytes());
                }
                DocLeaf::Ctor(k) => out.push(*k), // payloadless M2 ctor-head kind byte (20-26)
                DocLeaf::Name(n) => {
                    out.push(doc::KIND_NAME);
                    doc_leb(&mut out, n.len() as u64);
                    out.extend_from_slice(n.as_bytes());
                }
                DocLeaf::Str(bytes) => {
                    // KIND_STR + write_bytes (LEB len + UTF-8 body) — same framing as a Name, distinct kind.
                    out.push(doc::KIND_STR);
                    doc_leb(&mut out, bytes.len() as u64);
                    out.extend_from_slice(bytes.as_slice());
                }
                DocLeaf::Bytes(bytes) => {
                    // KIND_BYTES + write_bytes (LEB len + raw bytes) — same framing as Str/Name, distinct kind.
                    out.push(doc::KIND_BYTES);
                    doc_leb(&mut out, bytes.len() as u64);
                    out.extend_from_slice(bytes.as_slice());
                }
                DocLeaf::Float {
                    negative,
                    exponent,
                    significand,
                } => {
                    // KIND_FLOAT + negative(u8) + exponent(FIXED 8-byte big-endian i64, NOT LEB) +
                    // LEB significand length + big-endian magnitude bytes. Matches the codec's Float write.
                    out.push(doc::KIND_FLOAT);
                    out.push(*negative as u8);
                    out.extend_from_slice(&exponent.to_be_bytes());
                    doc_leb(&mut out, significand.len() as u64);
                    out.extend_from_slice(significand);
                }
            }
        }
        doc_leb(&mut out, self.structs.len() as u64);
        for s in &self.structs {
            match s {
                DocStruct::Atom(id) => {
                    out.push(doc::TAG_ATOM);
                    doc_leb(&mut out, *id as u64);
                }
                DocStruct::List { start, len } => {
                    out.push(doc::TAG_LIST);
                    doc_leb(&mut out, *len as u64);
                    let (s, l) = (*start as usize, *len as usize);
                    for &c in &self.child_pool[s..s + l] {
                        doc_leb(&mut out, c as u64);
                    }
                }
            }
        }
        doc_leb(&mut out, root as u64);
        out
    }
}

/// Follow a shape index through `Named`/`Ref` wrappers to the underlying shape (an erased newtype / a
/// table alias adds no runtime representation). Bounded by a small hop budget as a malformed-cycle
/// backstop. Returns the resolved `&Shape` (borrowed from the table), or `None` on a broken/cyclic index.
fn resolve_shape(desc: &Descriptor, mut shape_ix: u32) -> Option<&Shape> {
    for _ in 0..64 {
        match desc.table.get(shape_ix as usize)? {
            Shape::Ref(target) | Shape::Named(_, target) => shape_ix = *target,
            other => return Some(other),
        }
    }
    None
}

/// Collect a SET's elements into a Vec of (borrowed) element handles, SORTED into canonical key-VALUE
/// order under the element shape `elem_ix` (resolved through `Named`/`Ref`). The CHAMP iterates hash
/// order, so this re-sorts to the canonical render order. `None` (the encode declines) when the element
/// shape is not a canonically-orderable SCALAR — matching the compiler's `const_key_order`, which
/// declines a nested-compound element. The returned handles are BORROWED (the set still owns them); the
/// caller only reads them to encode, so no dup/drop is needed.
fn set_elements_canonical(desc: &Descriptor, set: Handle, elem_ix: u32) -> Option<Vec<Handle>> {
    // The element must offer a total order — a blessed scalar leaf OR an orderable COMPOUND (a tuple/list/
    // record/sum all of whose leaves are orderable). `value_cmp_shaped` supplies that order for BOTH cases
    // (it's the same descriptor-guided total order the runtime `<`/`Core::ValueCmp` walk and value-encode
    // use), so we probe orderability once and reuse the walk for the sort. A non-orderable element (a float/
    // bytes/set/map leaf) makes `value_cmp_shaped` return `None` → the encode declines (empty list), matching
    // the compiler, which only bakes a set descriptor over an orderable element.
    let mut elems: Vec<Handle> = Vec::new();
    let mut cur = op_set_iter(set);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break; // exhausted
        }
        elems.push(e);
        cur = op_set_iter_next(cur);
    }
    op_drop(cur); // release the final (exhausted) cursor
    // Probe orderability on a representative element (all elements share `elem_ix`'s shape). An empty set is
    // trivially orderable — nothing to sort. A `None` on a non-empty set means a non-orderable element shape.
    if let Some(&probe) = elems.first()
        && value_cmp_shaped(desc, probe, probe, elem_ix).is_none()
    {
        return None; // a non-orderable element shape — unrenderable, decline
    }
    // Sort into canonical VALUE order via the descriptor-guided total order. Set members are DISTINCT, so
    // stability is irrelevant → the in-place `sort_unstable_by` (no merge scratch-buffer allocation, better
    // constants) gives the same canonical order as a stable sort with one fewer heap allocation. A `None`
    // from `value_cmp_shaped` mid-sort (defensive — the orderability probe above already ruled it out) reads
    // as Equal, keeping the sort total (never a panic).
    elems.sort_unstable_by(|&x, &y| {
        value_cmp_shaped(desc, x, y, elem_ix).unwrap_or(core::cmp::Ordering::Equal)
    });
    Some(elems)
}

/// Collect a MAP's entries into a Vec of (borrowed) `(key, value)` handle pairs, SORTED into canonical
/// KEY-value order under the KEY shape `key_ix` (resolved through `Named`/`Ref`). The CHAMP iterates
/// hash order, so this re-sorts to the canonical render order (`collections-and-text.md §A Map Renders
/// As Its Entries In Canonical Key Order`). `None` (the encode declines) when the KEY shape is not a
/// canonically-orderable SCALAR — matching the compiler's `const_key_order`. The VALUE may be any
/// encodable shape (the walk recurses on it). Handles are BORROWED (the map owns them); no dup/drop.
fn map_entries_canonical(
    desc: &Descriptor,
    map: Handle,
    key_ix: u32,
) -> Option<Vec<(Handle, Handle)>> {
    // The KEY must offer a total order — a blessed scalar leaf OR an orderable COMPOUND (tuple/list/record/
    // sum of orderable leaves). `value_cmp_shaped` supplies that order for BOTH (the same total order the
    // runtime `<`/value-encode use), so we probe orderability once and reuse the walk for the sort. A
    // non-orderable key (a float/bytes/set/map leaf) makes it return `None` → the encode declines, matching
    // the compiler, which only bakes a map descriptor over an orderable key.
    let mut entries: Vec<(Handle, Handle)> = Vec::new();
    let mut cur = op_map_iter(map);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break; // exhausted
        }
        let v = op_map_iter_val(cur);
        entries.push((k, v));
        cur = op_map_iter_next(cur);
    }
    op_drop(cur); // release the final (exhausted) cursor
    // Probe orderability on a representative KEY (all keys share `key_ix`'s shape); an empty map is trivially
    // orderable. A `None` on a non-empty map means a non-orderable key shape.
    if let Some(&(probe, _)) = entries.first()
        && value_cmp_shaped(desc, probe, probe, key_ix).is_none()
    {
        return None; // a non-orderable key shape — unrenderable, decline
    }
    // Sort by canonical KEY order via the descriptor-guided total order. Map keys are DISTINCT → stability is
    // irrelevant, so `sort_unstable_by` (in-place, no merge scratch-buffer allocation) gives the same
    // canonical order with one fewer heap allocation than the stable `sort_by`. A defensive mid-sort `None`
    // (ruled out by the probe) reads as Equal, keeping the sort total.
    entries.sort_unstable_by(|&(ka, _), &(kb, _)| {
        value_cmp_shaped(desc, ka, kb, key_ix).unwrap_or(core::cmp::Ordering::Equal)
    });
    Some(entries)
}

/// `set-to-list(s, desc)` — enumerate a SET's elements as a runtime `List` (a persistent vec) in CANONICAL
/// element-value order (collections-and-text.md §A Set's canonical form: program iteration order == the
/// canonical byte-form order, NOT the CHAMP hash order the raw cursor walks). Reuses `set_elements_canonical`
/// (the same sorted collection value-encode uses to render `(Set.of (list …))`), so the observable order is
/// IDENTICAL to the value form — one source of truth for canonical order. BORROWS `s` and `desc` (an
/// inspector — the caller owns `s`'s release; `desc` is a compiler-baked constant): each element handle the
/// sorted walk returns is BORROWED (the set still owns it), so it is `dup`'d before being stored in the fresh
/// OWNED result vec (the vec now co-owns a reference; the set keeps its own). A malformed descriptor or a
/// non-scalar (unorderable) element shape yields the EMPTY vec — the defensive total matching value-encode's
/// never-trap contract (the compiler only bakes a well-formed `Set` descriptor here). The result is a normal
/// `List a` handle the front-end consumes exactly like any list.
fn op_set_to_list(set: Handle, desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return op_vec_empty();
    };
    // The root shape must resolve to a `Set(elem_ix)`; anything else is a malformed/mismatched descriptor.
    let elem_ix = match resolve_shape(&descriptor, descriptor.root) {
        Some(Shape::Set(e)) => *e,
        _ => return op_vec_empty(),
    };
    let Some(elems) = set_elements_canonical(&descriptor, set, elem_ix) else {
        return op_vec_empty(); // a non-scalar element shape is unorderable — decline to the empty list
    };
    // Build the arr of (dup'd) element handles in canonical order, then fold it into a persistent vec. The
    // CHAMP stores each element ALREADY BOXED (a scalar's box-* leaf / a compound's handle), so the element
    // handle is stored as-is — no re-box — exactly the representation a `List a` element carries.
    let arr = op_arr_alloc(elems.len() as u32);
    for (i, &e) in elems.iter().enumerate() {
        op_dup(e); // the set still owns `e`; the vec takes an independent reference
        op_arr_set(arr, i as u32, e);
    }
    op_vec_of_arr(arr) // consumes the arr, yields the List handle
}

/// `map-to-list(m, desc)` — enumerate a MAP's entries as a runtime `List (Tuple k v)` (a persistent vec of
/// 2-element tuple handles) in CANONICAL KEY order (collections-and-text.md §A Map Renders As Its Entries In
/// Canonical Key Order). Reuses `map_entries_canonical` (the sorted walk value-encode renders from), so the
/// observable order matches the value form exactly. BORROWS `m` and `desc`; each `(key, value)` handle the
/// walk returns is BORROWED (the map owns them), so both are `dup`'d before being stored into the fresh owned
/// entry tuple (an `arr-alloc(2)` — the runtime representation of `(Tuple k v)`, key at slot 0, value at slot
/// 1), and the tuple handles are collected into the result vec. A malformed descriptor or a non-scalar
/// (unorderable) KEY shape yields the EMPTY vec (the never-trap total). The result is a `List (Tuple k v)` the
/// front-end consumes like any list of pairs.
fn op_map_to_list(map: Handle, desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return op_vec_empty();
    };
    let key_ix = match resolve_shape(&descriptor, descriptor.root) {
        Some(Shape::Map(k, _v)) => *k,
        _ => return op_vec_empty(),
    };
    let Some(entries) = map_entries_canonical(&descriptor, map, key_ix) else {
        return op_vec_empty(); // a non-scalar key shape is unorderable — decline to the empty list
    };
    let arr = op_arr_alloc(entries.len() as u32);
    for (i, &(k, v)) in entries.iter().enumerate() {
        // A fresh 2-element tuple `[key, value]` — the `(Tuple k v)` representation. Each component is
        // BORROWED from the map, so `dup` it: the entry tuple co-owns a reference alongside the map. Both
        // components are stored ALREADY BOXED (the CHAMP holds boxed handles), matching a tuple's slots.
        let entry = op_arr_alloc(2);
        op_dup(k);
        op_arr_set(entry, 0, k);
        op_dup(v);
        op_arr_set(entry, 1, v);
        op_arr_set(arr, i as u32, entry); // the tuple handle is owned by `arr` (moved in, no dup)
    }
    op_vec_of_arr(arr) // consumes the arr, yields the List (Tuple k v) handle
}

/// The NON-PROGRESS cap on the value walk — bounds a MALFORMED descriptor whose `Ref`/`Named` chain
/// cycles WITHOUT ever consuming a heap node (e.g. `Ref → Ref`, or `Named → Ref → Named …`), which would
/// otherwise spin the iterative walk forever building nothing. It counts only CONSECUTIVE non-consuming
/// transitions (`Ref` and `Named` both keep the SAME value `h`); it RESETS to 0 on any descent into a
/// child node (Tuple/List/Record/Sum reach a DIFFERENT heap node via `arr-get`/`sum-payload`, so they
/// make progress and cannot cycle on a well-formed acyclic value). It is therefore NOT a value-DEPTH
/// limit: because the walk is ITERATIVE (an explicit heap work stack — see `encode_value`), a genuinely
/// deep value (a long list, a deep tree) is bounded only by heap, never by the ~4.5 k-frame native/wasm
/// call stack a recursive walker would overflow. A real descriptor's `Ref`/`Named` runs are O(1) between
/// consuming steps, so this cap never fires on a well-formed value however deep.
const ENCODE_REF_CYCLE_CAP: u32 = 100_000;

/// One unit of pending work on the iterative encode's explicit stack. Modelled directly on the recursive
/// walk it replaces (below, as `encode_value_recursive` in the tests) so the SEQUENCE of `DocBuilder`
/// leaf/struct pushes — and therefore the document bytes — is IDENTICAL. `'d` borrows the descriptor's
/// interned names (the head/field/type strings), so no name is cloned.
// `'static` (no borrow of the descriptor) so the `work` stack can be REUSED from a thread-local across
// encodes (grow-once, like `ENCODE_OUT`/`ENCODE_BUILDER`) instead of a fresh heap Vec per call — the
// `work` stack grows O(depth) for a deep value (each container's assembler stays on the stack during
// child descent), so a fresh Vec's grow-chain cost O(log depth) reallocs PER encode. The three formerly
// borrowed fields (a record field's key `&str`, a `Named`'s type name `&str`, a `Framed`'s `&TypeNode`)
// are re-derived from `desc` at PROCESS time via the OWNING shape's table index — the name leaf is still
// built at process time, so emission order (byte-exactness) is unchanged.
enum EncodeWork {
    /// Dispatch on the shape of value `h` at table entry `shape_ix`; leaf shapes emit + produce one
    /// result, container shapes emit their head eagerly then push children (in reverse) + an assembler.
    /// `refs` = consecutive non-consuming `Ref`/`Named` hops taken to reach here (reset on child descent).
    Visit { h: Handle, shape_ix: u32, refs: u32 },
    /// A record FIELD: emit the key leaf+atom (BEFORE the field value, matching the recursive per-field
    /// order), then queue the value visit and a `Pair` assembler. The key `&str` is re-derived at process
    /// time from `desc.table[rec_ix]` (the `Shape::Record`) at `field_ix` — no borrow held on the stack.
    VisitField {
        h: Handle,
        shape_ix: u32,
        rec_ix: u32,
        field_ix: u32,
    },
    /// Assemble `list([head_s, <the top `nkids` results in child order>])` — the tuple/list/record/sum body.
    List { head_s: u32, nkids: usize },
    /// Assemble the `(: value Type)` frame: pop the inner value, emit the type-name leaf+atom AFTER it
    /// (matching the recursive order), then `list([colon_s, value, tname_s])`. The name `&str` is
    /// re-derived at process time from `desc.table[named_ix]` (the `Shape::Named`).
    Named { colon_s: u32, named_ix: u32 },
    /// Assemble a `(: value <type-node>)` frame — like `Named` but the type is an arbitrary (possibly
    /// NESTED) type node, re-derived at process time from `desc.table[framed_ix]` (the `Shape::Framed`).
    /// Pop the inner value, `render_type_node` the type, then `list([colon_s, value, type_node])`.
    Framed { colon_s: u32, framed_ix: u32 },
    /// Assemble one record field: pop the field value, `list([eq, katom, fval])` where `eq` is the M2
    /// FieldPair ctor-head atom (pre-M2 it was the `=` name atom). `eq` and the key atom are built PRE-order
    /// (before the value visit) so the leaf/struct pool matches canon's pre-order first-encounter — see
    /// `VisitField`. Structure `(FieldPair name value)`.
    Pair { eq: u32, katom: u32 },
    /// A MAP entry (M2): build the FieldPair ctor-head atom PRE-order (before the k/v subtrees, for canon
    /// first-encounter — the FieldPair leaf dedups), then queue the value + key visits and a `MapPair`
    /// assembler. Mirrors `VisitField` (a map key is a VALUE, so it is Visited, not a pre-built name atom).
    VisitMapEntry {
        k: Handle,
        v: Handle,
        key_shape: u32,
        val_shape: u32,
    },
    /// Assemble one MAP entry `(FieldPair key value)`: the key result is directly below the value result on
    /// `out` (key Visited before value). Pop value then key, build `list([fp_s, key, value])`. `fp_s` is the
    /// FieldPair ctor-head atom built PRE-order in `VisitMapEntry`.
    MapPair { fp_s: u32 },
    /// Assemble `(map (k1 v1) … (kn vn))` — the canonical Map value form. Pops the top `nentries` pair
    /// results (already in canonical KEY order), under the pre-emitted `map` `head_s`.
    MapOf { head_s: u32, nentries: usize },
}

/// Walk the runtime value `root_h` under table entry `root_shape`, appending its value-form structs to
/// `b`; return the root struct index. A `Ref` follows the table (where a recursive value re-enters the
/// sum's shape). `None` on a malformed descriptor / out-of-range disc / unrenderable shape / a `Ref`/
/// `Named` cycle exceeding `ENCODE_REF_CYCLE_CAP`. BORROWS the value; caller drops the root afterward.
///
/// ITERATIVE (an explicit heap work stack, not native recursion) — a deep recursive value (a long linked
/// list, a deep tree: the very shapes this op exists to encode) would overflow the ~4.5 k-frame native /
/// wasm call stack of the recursive walker and ABORT the guest, rather than honour the op's decline
/// contract. Same discipline as `op_drop`'s iterative free cascade. The push order reproduces the
/// recursive walk's leaf/struct emission EXACTLY, so the document is byte-identical (guarded by
/// `value_encode_iterative_matches_recursive_reference`). `refs` counts only consecutive non-consuming
/// `Ref`/`Named` hops (reset on every child descent), so the cap bounds a malformed cycle WITHOUT
/// limiting a well-formed value's genuine depth.
fn encode_value(
    desc: &Descriptor,
    b: &mut DocBuilder,
    out: &mut Vec<u32>,
    work: &mut Vec<EncodeWork>,
    root_h: Handle,
    root_shape: u32,
) -> Option<u32> {
    // `out` (completed struct indices) and `work` (the pending-task stack) are both REUSED thread-local
    // buffers, passed in by the caller (cleared here, capacity retained across encodes). `EncodeWork` is
    // now `'static` (no descriptor borrow — the key/name/type-node are re-derived from `desc` at process
    // time), so the `work` stack reuses like `out`/the builder instead of a fresh Vec per encode.
    out.clear();
    work.clear();
    work.push(EncodeWork::Visit {
        h: root_h,
        shape_ix: root_shape,
        refs: 0,
    });
    while let Some(task) = work.pop() {
        match task {
            EncodeWork::Visit { h, shape_ix, refs } => {
                if refs > ENCODE_REF_CYCLE_CAP {
                    return None; // a Ref/Named chain that never consumes a node — malformed descriptor cycle
                }
                match desc.table.get(shape_ix as usize)? {
                    Shape::Ref(target) => {
                        // Non-consuming: same `h`, no heap node reached → count toward the cycle cap.
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: *target,
                            refs: refs + 1,
                        });
                    }
                    Shape::Int => {
                        let l = b.int_leaf(op_get_int(h));
                        out.push(b.atom(l));
                    }
                    Shape::BigInt => {
                        // Read the arbitrary-precision value via `unbox_bigint` (NOT `op_get_int`, which
                        // caps at i64) and render it as the SAME `KIND_INT` leaf — the leaf is already
                        // sign + arbitrary-width big-endian magnitude, so no new wire kind is needed.
                        let l = b.bigint_leaf(&unbox_bigint(h));
                        out.push(b.atom(l));
                    }
                    Shape::Rational => {
                        // Read the two BigInt components (`unbox_rational`) and render the single `num/den`
                        // NAME leaf — the constant-Rational value form. Each component is formatted decimal
                        // in the runtime (`Big::to_decimal_string`), since a rational is ONE name leaf (the
                        // codec's Int leaf would format on the host, but there is no "num/den" wire kind).
                        let (num, den) = unbox_rational(h);
                        let mut s = num.to_decimal_string();
                        s.push('/');
                        s.push_str(&den.to_decimal_string());
                        let l = b.name_leaf(&s);
                        out.push(b.atom(l));
                    }
                    Shape::Bool => {
                        let l = b.bool_leaf(op_get_bool(h));
                        out.push(b.atom(l));
                    }
                    Shape::Char => {
                        // A char value is an immediate int (the code-point) — read it with `op_get_int` and
                        // emit a `KIND_CHAR` leaf (rendered as a `#\c` char literal on decode), mirroring how
                        // `Bool` emits `KIND_BOOL_*`. A code-point that is not a Unicode scalar is a malformed
                        // Char value → decline the encode (like a non-finite Float).
                        let c = char::from_u32(op_get_int(h) as u32)?;
                        let l = b.char_leaf(c);
                        out.push(b.atom(l));
                    }
                    Shape::Unit => {
                        let l = b.name_leaf("unit");
                        out.push(b.atom(l));
                    }
                    Shape::Str => {
                        // A String value may be a ROPE (a `String.concat`/`String.at`-slice builds concat/
                        // slice nodes, NOT a flat leaf), so MATERIALIZE it to a leaf first (`bytes_flatten`,
                        // iterative so no deep-rope stack overflow; content-preserving so unobservable on a
                        // borrowed/shared value) before reading `raw` — exactly as `Shape::Bytes` does. A
                        // flat string leaf stores its UTF-8 bytes in `raw` and flatten is a no-op there.
                        // Without the flatten a runtime string (a concat/slice) rendered its raw HANDLE
                        // bytes (garbage), losing the content.
                        bytes_flatten(h);
                        // Build the leaf DIRECTLY from the flattened node's borrowed raw slice — `str_leaf`
                        // stores it as an inline `Raw` for a short string (no `to_vec`). `with_node` returns
                        // the leaf index while the borrow is live; a null/missing node reads as empty.
                        let l = with_node(h, None, |n| Some(b.str_leaf(n.raw.as_slice())))
                            .unwrap_or_else(|| b.str_leaf(&[]));
                        out.push(b.atom(l));
                    }
                    Shape::Bytes => {
                        // A Bytes value may be a ROPE (concat/slice nodes) — materialize it to a leaf
                        // (iterative `bytes_flatten`, so no deep-rope stack overflow; content-preserving so
                        // UNOBSERVABLE even on a borrowed/shared value), then read the leaf's raw and emit a
                        // KIND_BYTES leaf. A leaf is already flat (flatten is a no-op there).
                        bytes_flatten(h);
                        let l = with_node(h, None, |n| Some(b.bytes_leaf(n.raw.as_slice())))
                            .unwrap_or_else(|| b.bytes_leaf(&[]));
                        out.push(b.atom(l));
                    }
                    Shape::Float => {
                        // Convert the runtime f64 to the codec's EXACT decimal (KIND_FLOAT). A NON-FINITE
                        // float (nan/inf) has no exact-decimal form → `float_leaf` returns None → the whole
                        // encode declines (matches the compiler's `Decimal::from_f64` None; nan/inf cross by
                        // their own dedicated forms, not the value-encode walker).
                        let l = b.float_leaf(op_get_float(h))?;
                        out.push(b.atom(l));
                    }
                    Shape::Float32 => {
                        // Read the 4-byte Float32 and render the f32's OWN shortest decimal (not a promoted
                        // f64's). A non-finite f32 declines, like Float64.
                        let l = b.float32_leaf(op_get_float32(h))?;
                        out.push(b.atom(l));
                    }
                    Shape::Tuple(elems) => {
                        if elems.is_empty() {
                            // An EMPTY `(Tuple)`-typed value renders the HEADED empty tuple `(tuple)`, NOT
                            // `unit` (Ruling-B: `unit` and `(Tuple)` are DISTINCT types — 05-compound:9232-9239 —
                            // and a `(Tuple)`-typed value MUST render `(tuple)`, matching the rust
                            // cdz_render_expr path + the wasm const path). The physical handle is `imm_unit`
                            // (`op_arr_alloc(0)`) for BOTH a Unit and an empty-tuple value — they share one
                            // runtime handle, so the render MUST be driven by the SHAPE DESCRIPTOR, not the
                            // handle: `Shape::Unit` → `unit`, `Shape::Tuple([])` → `(tuple)`. Emit the same
                            // `tuple` head as the non-empty arm with ZERO children (`list_head_tail` over an
                            // empty slice yields the bare `(tuple)`). Paired with v-wasm-opt's shape_of change
                            // to emit `ShapeNode::Tuple([])` (not Unit) for an empty `Ty::Tuple` — BOTH needed.
                            let head = b.ctor_leaf(doc::KIND_TUPLE_CTOR);
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List { head_s, nkids: 0 });
                        } else {
                            // TOTALITY: the descriptor declares `elems.len()` fields; verify the actual node
                            // has at least that arity BEFORE any `op_arr_get` (which TRAPS on OOB / an
                            // immediate). A well-formed descriptor always matches, but a malformed one must
                            // DECLINE (`None`) per this op's contract, not trap the guest.
                            if (op_arr_len(h) as usize) < elems.len() {
                                return None;
                            }
                            let head = b.ctor_leaf(doc::KIND_TUPLE_CTOR);
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            // Push children in REVERSE so the LIFO stack visits them left→right; each
                            // completes to one `out` entry, in child order under the `List` assembler.
                            // A child is a DIFFERENT heap node (arr-get) → progress → reset `refs` to 0.
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        }
                    }
                    Shape::List(elem) => {
                        // A Cadenza `List` is an RRB `vec` (NOT a flat `arr` — a tuple/record is the arr),
                        // so read its length + elements with the `vec-*` ops. (`arr-len`/`arr-get` on a vec
                        // handle read the root node's arity, not the logical element count — the bug that
                        // rendered only the first element.)
                        let (elem, n) = (*elem, op_vec_len(h));
                        let head = b.ctor_leaf(doc::KIND_LIST_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: n as usize,
                        });
                        for i in (0..n).rev() {
                            work.push(EncodeWork::Visit {
                                h: op_vec_get(h, i),
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Record(fields) => {
                        // TOTALITY (as `Tuple`): a record is an arr of field values; verify the node's
                        // arity covers the descriptor's field count before any trapping `op_arr_get`.
                        if (op_arr_len(h) as usize) < fields.len() {
                            return None;
                        }
                        let head = b.ctor_leaf(doc::KIND_RECORD_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: fields.len(),
                        });
                        for (i, (_k, fs)) in fields.iter().enumerate().rev() {
                            work.push(EncodeWork::VisitField {
                                h: op_arr_get(h, i as u32),
                                shape_ix: *fs,
                                rec_ix: shape_ix, // the Record shape's own table index (re-derives the key)
                                field_ix: i as u32,
                            });
                        }
                    }
                    Shape::Sum(variants) => {
                        // `sum_disc_shaped` (not `op_sum_disc`): an all-nullary sum nested in a compound
                        // reaches render as an Int IMMEDIATE (box-int of a small disc → imm_int), and
                        // `op_sum_disc`→0 would render the FIRST variant for EVERY value (SOUNDNESS #43 render
                        // sibling of witness 4 — a runtime `(tuple (Tri.Hi unit) 5)` else renders `(Tri.Lo …)`).
                        let disc = sum_disc_shaped(h) as usize;
                        let (head, payload_shape) = variants.get(disc)?;
                        let head_leaf = b.name_leaf(head);
                        let head_s = b.atom(head_leaf);
                        let payload_shape = *payload_shape;
                        let payload_h = op_sum_payload(h);
                        // A MULTI-payload variant's payload is a `Spread`: the payload handle is the tuple
                        // arr of the boxed payloads, and the variant renders `(Variant p0 p1 …)` — the
                        // elements FLATTENED directly under the head, NOT wrapped in a `tuple` form. So
                        // splice the tuple's elements as the variant's children (one `arr-get` per element,
                        // like the `Tuple` walk) rather than visiting the single tuple shape.
                        if let Some(Shape::Spread(elems)) = desc.table.get(payload_shape as usize) {
                            let elems = elems.clone();
                            // TOTALITY (as `Tuple`): the payload arr must have ≥ the Spread's element count
                            // before any trapping `op_arr_get` — a malformed descriptor DECLINES, not traps.
                            if (op_arr_len(payload_h) as usize) < elems.len() {
                                return None;
                            }
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(payload_h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        } else {
                            // A nullary variant's payload shape is `Unit` → the bare `unit` atom (the
                            // `(Variant unit)` form); a single-payload variant reaches its payload via
                            // `sum-payload` — a DIFFERENT heap node → progress → reset `refs`.
                            work.push(EncodeWork::List { head_s, nkids: 1 });
                            work.push(EncodeWork::Visit {
                                h: payload_h,
                                shape_ix: payload_shape,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Named(_name, inner) => {
                        // The `(: <value> <Type>)` value-form frame — same `h`, no node consumed → count.
                        let inner = *inner;
                        let colon = b.name_leaf(":");
                        let colon_s = b.atom(colon);
                        work.push(EncodeWork::Named {
                            colon_s,
                            named_ix: shape_ix, // re-derives `name` from desc.table[named_ix] at process time
                        });
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: inner,
                            refs: refs + 1,
                        });
                    }
                    Shape::Framed(_type_node, inner) => {
                        // The `(: <value> <type-node>)` frame — an arbitrary (possibly nested) type node.
                        // Same `h`, no node consumed → count toward the ref cap.
                        let inner = *inner;
                        let colon = b.name_leaf(":");
                        let colon_s = b.atom(colon);
                        work.push(EncodeWork::Framed {
                            colon_s,
                            framed_ix: shape_ix, // re-derives the TypeNode from desc.table[framed_ix]
                        });
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: inner,
                            refs: refs + 1,
                        });
                    }
                    Shape::Set(elem) => {
                        // A Set renders `((. Set of) (list e1 … en))` with elements in CANONICAL key-VALUE
                        // order. The CHAMP iterates hash order, so collect + SORT by the element's canonical
                        // scalar value (matching the compiler's `const_key_order`). Only a SCALAR element is
                        // orderable/encodable; a non-scalar element shape declines (as `const_key_order` does).
                        // M2 head-first: a Set is a FLAT `(Ctor(Set) e1 … en)` — the Set ctor-leaf head atom +
                        // the sorted elements as direct children (NOT the pre-M2 `((. Set of) (list e…))`
                        // member-access-over-list form). Head interned PRE-order (canon first-encounter), then
                        // the elements visited in canonical order — reuse the plain `List` assembler.
                        let elem = *elem;
                        let sorted = set_elements_canonical(desc, h, elem)?;
                        let head = b.ctor_leaf(doc::KIND_SET_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: sorted.len(),
                        });
                        // Push in REVERSE so the LIFO stack encodes them in canonical order onto `out`. Each
                        // element is a DISTINCT heap node (a set member) → progress → reset `refs`.
                        for &e in sorted.iter().rev() {
                            work.push(EncodeWork::Visit {
                                h: e,
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Map(key, val) => {
                        // M2 head-first: a Map is `(Ctor(Map) (FieldPair k1 v1) … (FieldPair kn vn))` with
                        // entries in CANONICAL KEY order (CHAMP iterates hash order → collect + SORT by the
                        // key's canonical scalar value; only a SCALAR key is orderable/encodable, the value is
                        // any encodable shape). Map ctor head EAGER (pre-order); each entry is a FieldPair
                        // triple built by `VisitMapEntry` (which interns the FieldPair head PRE-order, before
                        // the k/v subtrees, for canon first-encounter — mirroring the record `VisitField`).
                        let (key, val) = (*key, *val);
                        let entries = map_entries_canonical(desc, h, key)?;
                        let map_head = b.ctor_leaf(doc::KIND_MAP_CTOR);
                        let head_s = b.atom(map_head);
                        work.push(EncodeWork::MapOf {
                            head_s,
                            nentries: entries.len(),
                        });
                        // Push entries in REVERSE (so `VisitMapEntry` pops in canonical order); each entry's
                        // handler builds its FieldPair head + visits key then value.
                        for &(k, v) in entries.iter().rev() {
                            work.push(EncodeWork::VisitMapEntry {
                                k,
                                v,
                                key_shape: key,
                                val_shape: val,
                            });
                        }
                    }
                    Shape::Spread(elems) => {
                        // A `Spread` is ONLY reached inline by the `Sum` walk (which splices its elements
                        // under the variant head). Visited DIRECTLY (a malformed descriptor that roots or
                        // nests a Spread outside a Sum variant), render it as an ordinary `tuple` — a safe
                        // fallback that never traps, matching the `Tuple` walk.
                        if elems.is_empty() {
                            let l = b.name_leaf("unit");
                            out.push(b.atom(l));
                        } else {
                            let elems = elems.clone();
                            let head = b.name_leaf("tuple");
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        }
                    }
                }
            }
            EncodeWork::VisitField {
                h,
                shape_ix,
                rec_ix,
                field_ix,
            } => {
                // Key leaf+atom emitted BEFORE the field value; the `Pair` assembler runs AFTER it. The
                // field value is a fresh child node (arr-get already applied) → a new walk, `refs` 0.
                // Re-derive the key from the owning `Shape::Record` at `field_ix` (no borrow on the stack).
                let key = match desc.table.get(rec_ix as usize) {
                    Some(Shape::Record(fields)) => fields.get(field_ix as usize).map(|(k, _)| &**k),
                    _ => None,
                }?;
                // CANON CONVERGENCE: emit the `=` head atom, THEN the key atom, BOTH before descending into
                // the field value — matching canon's pre-order first-encounter (a field triple's children are
                // `[=, name, value]`, so canon interns `=` first, then name, then the value subtree). The
                // pre-Phase-B code built `=` in the `Pair` assembler AFTER the value, which interned `=` LATE
                // and made value-encode non-canonical vs `codec::encode(canon(tree))`. See canon.rs `visit`.
                // M2: a record field is `(FieldPair name value)` — the FieldPair ctor-leaf head + the key
                // name atom + the value (was the `=` name head pre-M2). Structure unchanged (head + 2 kids).
                let eq_leaf = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let eq = b.atom(eq_leaf);
                let kname = b.name_leaf(key);
                let katom = b.atom(kname);
                work.push(EncodeWork::Pair { eq, katom });
                work.push(EncodeWork::Visit {
                    h,
                    shape_ix,
                    refs: 0,
                });
            }
            EncodeWork::List { head_s, nkids } => {
                let base = out.len().checked_sub(nkids)?;
                // Build the list's range directly in the pool: head + the completed children in `out[base..]`
                // (already in child order — see the reverse push), no temporary Vec.
                let s = b.list_head_tail(head_s, &out[base..]);
                out.truncate(base);
                out.push(s);
            }
            EncodeWork::Named { colon_s, named_ix } => {
                let value = out.pop()?;
                // Re-derive the type name from the owning `Shape::Named` (no borrow on the stack).
                let name = match desc.table.get(named_ix as usize) {
                    Some(Shape::Named(name, _)) => &**name,
                    _ => return None,
                };
                let tname = b.name_leaf(name);
                let tname_s = b.atom(tname);
                out.push(b.list(&[colon_s, value, tname_s]));
            }
            EncodeWork::Framed { colon_s, framed_ix } => {
                let value = out.pop()?;
                // Re-derive the TypeNode from the owning `Shape::Framed` (no borrow on the stack).
                let type_node = match desc.table.get(framed_ix as usize) {
                    Some(Shape::Framed(tn, _)) => tn,
                    _ => return None,
                };
                let type_s = b.render_type_node(type_node);
                out.push(b.list(&[colon_s, value, type_s]));
            }
            EncodeWork::Pair { eq, katom } => {
                // Record field value-output form is the `(= name value)` ascription (record-type Phase B
                // full-symmetry migration — literals, patterns, AND value-output all spell `(= name value)`;
                // operator-ruled 2026-08-09). The `=` and key atoms were built PRE-order (in `VisitField`,
                // before the value) so the leaf/struct pool matches canon first-encounter; here we only
                // assemble the list once the field value result is on `out`.
                let fval = out.pop()?;
                out.push(b.list(&[eq, katom, fval]));
            }
            EncodeWork::VisitMapEntry {
                k,
                v,
                key_shape,
                val_shape,
            } => {
                // M2 map entry `(FieldPair key value)`: intern the FieldPair ctor-head atom PRE-order (before
                // the k/v subtrees, so the leaf/struct pool matches canon first-encounter — the FieldPair leaf
                // dedups across entries), then visit key BEFORE value (key below value on `out`, as `MapPair`
                // relies on). Mirrors the record `VisitField`.
                let fp = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let fp_s = b.atom(fp);
                work.push(EncodeWork::MapPair { fp_s });
                work.push(EncodeWork::Visit {
                    h: v,
                    shape_ix: val_shape,
                    refs: 0,
                });
                work.push(EncodeWork::Visit {
                    h: k,
                    shape_ix: key_shape,
                    refs: 0,
                });
            }
            EncodeWork::MapPair { fp_s } => {
                // Key was Visited before value, so on `out` the value is on top, key directly below.
                let val = out.pop()?;
                let key = out.pop()?;
                out.push(b.list(&[fp_s, key, val]));
            }
            EncodeWork::MapOf { head_s, nentries } => {
                // The top `nentries` results are the `(FieldPair key value)` entries in canonical KEY order.
                let base = out.len().checked_sub(nentries)?;
                let s = b.list_head_tail(head_s, &out[base..]);
                out.truncate(base);
                out.push(s);
            }
        }
    }
    // A well-formed walk leaves exactly the one root struct index.
    match out.len() {
        1 => out.pop(),
        _ => None,
    }
}

/// Render the runtime value `h` to its canonical binary-AST value-form document, under the shape
/// descriptor `desc` (compiler-baked bytes; see the module note). `None` on a malformed descriptor or an
/// unrenderable shape (a not-yet-supported Float/Str/Bytes payload). Does NOT drop `h` — the caller
/// (the escape `encode`) owns the release point.
fn op_value_encode_form(h: Handle, desc: &[u8]) -> Option<Vec<u8>> {
    // Decode the descriptor via the single-entry cache: on a hit (the same escape site's bytes as last
    // call — the common loop case) the decode + its Vec/String allocs are skipped entirely. On a miss,
    // decode once and store `(bytes.to_vec(), descriptor)` as the new entry. The whole encode runs while
    // the cache cell is borrowed, so the cached `Descriptor` is used in place (no clone). `decode_
    // descriptor` is a pure function of the bytes, so a byte-equal hit yields the identical descriptor.
    DESCRIPTOR_CACHE.with(|dcell| {
        let mut slot = dcell.borrow_mut();
        // Refresh the entry on a miss (empty, or different bytes than cached).
        if slot.as_ref().map(|(bytes, _)| bytes.as_slice()) != Some(desc) {
            let decoded = decode_descriptor(desc)?;
            *slot = Some((desc.to_vec(), decoded));
        }
        let descriptor = &slot.as_ref()?.1;
        // Reuse the thread-local builder + `out` + `work` stacks — `reset()`/`clear()` empties them but
        // retains capacity, so the leaf/struct/child-pool + result-stack + work-stack growth is paid ONCE
        // (not per encode). The result bytes are identical either way; the reuse is a pure allocation
        // optimisation (see `ENCODE_BUILDER`/`ENCODE_OUT`/`ENCODE_WORK`). The cells are distinct → never alias.
        ENCODE_BUILDER.with(|bcell| {
            ENCODE_OUT.with(|ocell| {
                ENCODE_WORK.with(|wcell| {
                    let b = &mut *bcell.borrow_mut();
                    let out = &mut *ocell.borrow_mut();
                    let work = &mut *wcell.borrow_mut();
                    b.reset();
                    let root = encode_value(descriptor, b, out, work, h, descriptor.root)?;
                    Some(b.finish(root))
                })
            })
        })
    })
}

// ─── value-decode (heap idx 90): the inverse of value-encode ──────────────────────────────
// Parse a canonical `cadenza-ast` value-form document (the exact bytes `op_value_encode_form` /
// `DocBuilder::finish` produces) and, guided by the SAME shape descriptor `value-encode` reads,
// CONSTRUCT a fresh owned heap value. Descriptor-guided + name/tag-free (field names / variant tags come
// from the descriptor, matched against the document, never invented). TOTAL: any shape/format mismatch
// returns `Handle::NULL` (0) — NEVER traps (the decode analogue of `op_value_encode_form`'s
// malformed-descriptor → empty-Bytes decline). See runtime.wit idx 90.

/// A parsed document leaf — the read-side mirror of `DocLeaf` (see `DocBuilder`). Owns its bytes so the
/// walk can build heap values without holding a borrow on the source Vec.
enum ParsedLeaf {
    /// (negative, big-endian magnitude, leading-zeros-stripped) — covers both `IntScalar` and `Int` on the
    /// wire (they encode to the identical `KIND_INT` framing); the walk picks i64 vs BigInt by SHAPE.
    Int(bool, Vec<u8>),
    Bool(bool),
    /// A Unicode-scalar Char leaf (`KIND_CHAR`) — the code-point; `decode_value` boxes it as an int.
    Char(char),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Bytes(Vec<u8>),
    /// (negative, exponent, big-endian base-256 significand) — the `KIND_FLOAT` exact-decimal parts.
    Float(bool, i64, Vec<u8>),
    /// An M2 payloadless ctor-head leaf — its `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER` byte (20-26).
    /// The head atom of a native compound value's list; `doc_atom_ctor` reads its kind for the decode arms.
    Ctor(u8),
}

/// A parsed document struct — the read-side mirror of `DocStruct`. A `List`'s children are struct indices
/// (owned Vec here rather than a pooled range, since the reader has no shared child pool).
enum ParsedStruct {
    Atom(u32),      // → leaves[leaf_id]
    List(Vec<u32>), // → child struct indices
}

/// The parsed document: leaves + structs + the root struct index. `decode_value` walks it from `root`.
struct ParsedDoc {
    leaves: Vec<ParsedLeaf>,
    structs: Vec<ParsedStruct>,
    root: u32,
}

/// Read an unsigned LEB128 from `d` at `*pos`, advancing `*pos`. `None` on truncation or a >10-byte
/// (u64-overflowing) encoding — a malformed document declines, never panics.
fn doc_read_leb(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *d.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None; // overflow — malformed
        }
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Some(value)
}

/// Read `len` bytes from `d` at `*pos`, advancing `*pos`. `None` on truncation.
fn doc_read_bytes<'a>(d: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    let slice = d.get(*pos..end)?;
    *pos = end;
    Some(slice)
}

/// Parse a value-form document (the inverse of `DocBuilder::finish`) into a `ParsedDoc`. Total: any
/// malformed framing (bad header, truncation, unknown kind/tag, out-of-range index) returns `None`.
fn parse_doc(d: &[u8]) -> Option<ParsedDoc> {
    let mut pos = 0usize;
    // Header.
    let header = doc_read_bytes(d, &mut pos, doc::SCHEMA_HEADER.len())?;
    if header != doc::SCHEMA_HEADER {
        return None;
    }
    // Leaves.
    let leaf_count = doc_read_leb(d, &mut pos)? as usize;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        let kind = *d.get(pos)?;
        pos += 1;
        let leaf = match kind {
            // KIND_INT_POS_DEC (0) / neg (0+3=3): [maglen LEB][BE mag].
            doc::KIND_INT_POS_DEC | 3 => {
                let neg = kind == doc::KIND_INT_POS_DEC + 3;
                let maglen = doc_read_leb(d, &mut pos)? as usize;
                let mag = doc_read_bytes(d, &mut pos, maglen)?.to_vec();
                ParsedLeaf::Int(neg, mag)
            }
            doc::KIND_FLOAT => {
                let neg = *d.get(pos)? != 0;
                pos += 1;
                let eb = doc_read_bytes(d, &mut pos, 8)?;
                let mut ebuf = [0u8; 8];
                ebuf.copy_from_slice(eb);
                let exp = i64::from_be_bytes(ebuf);
                let siglen = doc_read_leb(d, &mut pos)? as usize;
                let sig = doc_read_bytes(d, &mut pos, siglen)?.to_vec();
                ParsedLeaf::Float(neg, exp, sig)
            }
            doc::KIND_STR => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Str(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            doc::KIND_BOOL_FALSE => ParsedLeaf::Bool(false),
            doc::KIND_BOOL_TRUE => ParsedLeaf::Bool(true),
            doc::KIND_CHAR => {
                // The scalar UTF-8-encoded (LEB len + 1-4 bytes), matching cadenza-ast codec's read_scalar:
                // read the body, parse as ONE Unicode scalar. A non-UTF-8 body or not-exactly-one-scalar
                // body is a malformed Char leaf → decline.
                let len = doc_read_leb(d, &mut pos)? as usize;
                let bytes = doc_read_bytes(d, &mut pos, len)?;
                let s = core::str::from_utf8(bytes).ok()?;
                let mut it = s.chars();
                let c = it.next()?;
                if it.next().is_some() {
                    return None; // more than one scalar in a char leaf — malformed
                }
                ParsedLeaf::Char(c)
            }
            doc::KIND_NAME => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Name(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            doc::KIND_BYTES => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Bytes(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            // M2 native-compound ctor-head kinds (20-26) — payloadless single kind byte (no body to read).
            doc::KIND_LIST_CTOR
            | doc::KIND_TUPLE_CTOR
            | doc::KIND_RECORD_CTOR
            | doc::KIND_MAP_CTOR
            | doc::KIND_SET_CTOR
            | doc::KIND_FIELD_PAIR
            | doc::KIND_MEMBER => ParsedLeaf::Ctor(kind),
            _ => return None, // unknown kind — malformed
        };
        leaves.push(leaf);
    }
    // Structs.
    let struct_count = doc_read_leb(d, &mut pos)? as usize;
    let mut structs = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = *d.get(pos)?;
        pos += 1;
        let s = match tag {
            doc::TAG_ATOM => {
                let id = doc_read_leb(d, &mut pos)? as u32;
                if id as usize >= leaves.len() {
                    return None; // dangling leaf index
                }
                ParsedStruct::Atom(id)
            }
            doc::TAG_LIST => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                let mut kids = Vec::with_capacity(len.min(1 << 16));
                for _ in 0..len {
                    kids.push(doc_read_leb(d, &mut pos)? as u32);
                }
                ParsedStruct::List(kids)
            }
            _ => return None, // unknown tag — malformed
        };
        structs.push(s);
    }
    let root = doc_read_leb(d, &mut pos)? as u32;
    if root as usize >= structs.len() {
        return None; // dangling root
    }
    Some(ParsedDoc {
        leaves,
        structs,
        root,
    })
}

/// The single Atom leaf of a struct index, or `None` if that struct is a List (a shape/document mismatch
/// where a leaf was expected). Also range-checks the struct index.
fn doc_atom_leaf<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a ParsedLeaf> {
    match doc.structs.get(struct_ix as usize)? {
        ParsedStruct::Atom(leaf_id) => doc.leaves.get(*leaf_id as usize),
        ParsedStruct::List(_) => None,
    }
}

/// The child struct indices of a List struct, or `None` if it is an Atom (mismatch). Range-checked.
fn doc_list_kids<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a [u32]> {
    match doc.structs.get(struct_ix as usize)? {
        ParsedStruct::List(kids) => Some(kids),
        ParsedStruct::Atom(_) => None,
    }
}

/// The NAME-leaf text of an atom struct (a head/tag/name position), as `&str`. `None` if not a Name leaf
/// or not valid UTF-8.
fn doc_atom_name<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a str> {
    match doc_atom_leaf(doc, struct_ix)? {
        ParsedLeaf::Name(bytes) => core::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// The M2 ctor-head KIND byte of an atom struct (a `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER`
/// head position), or `None` if the atom is not a `Ctor` leaf. The decode counterpart of `doc_atom_name`
/// for native-compound heads.
fn doc_atom_ctor(doc: &ParsedDoc, struct_ix: u32) -> Option<u8> {
    match doc_atom_leaf(doc, struct_ix)? {
        ParsedLeaf::Ctor(k) => Some(*k),
        _ => None,
    }
}

/// Max decode recursion depth — the same backstop class as `TYPE_NODE_DEPTH_CAP`/`ENCODE_REF_CYCLE_CAP`:
/// a compiler-baked value is shallow, but a malformed document/descriptor must DECLINE (return NULL), not
/// overflow the guest stack. Well above any real value nesting.
const DECODE_DEPTH_CAP: u32 = 512;

/// Reconstruct an `f64` from a `ParsedLeaf::Float`'s (neg, exp, big-endian base-256 significand) via the
/// exact decimal `[-]<sig>e<exp>` (base-256 → base-10 by repeated ÷10) parsed with Rust's correctly-rounded
/// `str::parse::<f64>` — the inverse of `float_leaf`. `None` if the decimal fails to parse or is non-finite.
fn float_from_parts(neg: bool, exp: i64, mag: &[u8]) -> Option<f64> {
    let s = float_decimal_string(neg, exp, mag);
    let f: f64 = s.parse().ok()?;
    if f.is_finite() { Some(f) } else { None }
}

/// The f32 twin of `float_from_parts` (parses the same decimal as `f32`).
fn float32_from_parts(neg: bool, exp: i64, mag: &[u8]) -> Option<f32> {
    let s = float_decimal_string(neg, exp, mag);
    let f: f32 = s.parse().ok()?;
    if f.is_finite() { Some(f) } else { None }
}

/// Build the `[-]<significand>e<exponent>` decimal string from a `KIND_FLOAT`'s parts: the significand is
/// the big-endian base-256 magnitude read as a base-10 integer (repeated ÷10, no width assumption).
fn float_decimal_string(neg: bool, exp: i64, mag: &[u8]) -> String {
    let mut limbs: Vec<u32> = mag.iter().map(|&b| b as u32).collect(); // most-significant first
    let mut digits_rev: Vec<u8> = Vec::new();
    while limbs.iter().any(|&l| l != 0) {
        let mut rem = 0u32;
        for l in limbs.iter_mut() {
            let cur = rem * 256 + *l;
            *l = cur / 10;
            rem = cur % 10;
        }
        digits_rev.push(b'0' + rem as u8);
        while limbs.first() == Some(&0) && limbs.len() > 1 {
            limbs.remove(0);
        }
    }
    let sig: String = if digits_rev.is_empty() {
        "0".into()
    } else {
        digits_rev.iter().rev().map(|&b| b as char).collect()
    };
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&sig);
    s.push('e');
    // i64 exponent as decimal (no_std-safe via itoa-free format through a small helper).
    s.push_str(&exp_to_string(exp));
    s
}

/// `i64` → decimal string without `format!`'s float machinery (kept explicit for the `no_std` wasm build).
fn exp_to_string(mut v: i64) -> String {
    if v == 0 {
        return "0".into();
    }
    let neg = v < 0;
    let mut digits: Vec<u8> = Vec::new();
    // Work in i128 to hold i64::MIN's magnitude without overflow.
    let mut n = (v as i128).unsigned_abs();
    let _ = &mut v;
    while n > 0 {
        digits.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.extend(digits.iter().rev().map(|&b| b as char));
    s
}

/// Build a big-endian magnitude + sign into the `[sign][little-endian magnitude]` form
/// `bigint::Big::from_sign_magnitude_bytes` expects: reverse the BE magnitude to LE and prepend the sign.
fn big_from_be_mag(neg: bool, be_mag: &[u8]) -> bigint::Big {
    let mut sm = Vec::with_capacity(be_mag.len() + 1);
    sm.push(neg as u8);
    sm.extend(be_mag.iter().rev().copied());
    bigint::Big::from_sign_magnitude_bytes(&sm)
}

/// The descriptor-guided construction walk: read the doc node at `struct_ix` as a value of shape
/// `shape_ix`, building a fresh OWNED heap handle. `Handle::NULL` on ANY mismatch (never traps). `depth`
/// bounds recursion (malformed-cycle backstop).
fn decode_value(
    desc: &Descriptor,
    doc: &ParsedDoc,
    struct_ix: u32,
    shape_ix: u32,
    depth: u32,
) -> Handle {
    decode_value_opt(desc, doc, struct_ix, shape_ix, depth).unwrap_or(Handle::NULL)
}

/// `decode_value`'s `Option` core (so `?` short-circuits a mismatch to `None` → `NULL`). Every arm that
/// builds a heap value on success returns `Some(handle)`; a shape/document mismatch returns `None`.
fn decode_value_opt(
    desc: &Descriptor,
    doc: &ParsedDoc,
    struct_ix: u32,
    shape_ix: u32,
    depth: u32,
) -> Option<Handle> {
    if depth > DECODE_DEPTH_CAP {
        return None;
    }
    match desc.table.get(shape_ix as usize)? {
        // Transparent wrappers: the value handle passes through unchanged. On the wire a Named/Ref adds no
        // struct level (encode reuses the same `h`), EXCEPT Named/Framed which wrap `(: value Type)`.
        Shape::Ref(target) => decode_value_opt(desc, doc, struct_ix, *target, depth + 1),
        Shape::Named(_, inner) | Shape::Framed(_, inner) => {
            // `(: <value> <Type>)` — a 3-element list; the value is element [1], decoded against `inner`.
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != 3 || doc_atom_name(doc, kids[0])? != ":" {
                return None;
            }
            decode_value_opt(desc, doc, kids[1], *inner, depth + 1)
        }
        Shape::Int => {
            let ParsedLeaf::Int(neg, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            // i64-bounded: rebuild via the BigInt magnitude then read as i64 (an >i64 magnitude here is a
            // malformed doc for an `Int` shape — decline).
            let big = big_from_be_mag(*neg, mag);
            let v = big.to_i64_checked()?;
            Some(op_box_int(v))
        }
        Shape::BigInt => {
            let ParsedLeaf::Int(neg, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(box_bigint(&big_from_be_mag(*neg, mag)))
        }
        Shape::Bool => {
            let ParsedLeaf::Bool(b) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_bool(*b))
        }
        Shape::Char => {
            // A char value IS an int (the code-point) at runtime — box it with `op_box_int`, exactly as a
            // Bool boxes its 0/1. The wire leaf is `KIND_CHAR` (a scalar); the semantics are int.
            let ParsedLeaf::Char(c) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_int(*c as i64))
        }
        Shape::Float => {
            let ParsedLeaf::Float(neg, exp, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_float(float_from_parts(*neg, *exp, mag)?))
        }
        Shape::Float32 => {
            let ParsedLeaf::Float(neg, exp, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_float32(float32_from_parts(*neg, *exp, mag)?))
        }
        Shape::Str => {
            let ParsedLeaf::Str(bytes) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            let s = String::from_utf8(bytes.clone()).ok()?;
            Some(op_str_new(s))
        }
        Shape::Bytes => {
            let ParsedLeaf::Bytes(bytes) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            let buf = op_bytes_alloc(bytes.len() as u32);
            for (i, &b) in bytes.iter().enumerate() {
                op_bytes_set(buf, i as u32, b as u32);
            }
            Some(buf)
        }
        Shape::Unit => {
            // Encodes as the `unit` NAME atom.
            if doc_atom_name(doc, struct_ix)? != "unit" {
                return None;
            }
            Some(imm_unit())
        }
        Shape::Rational => {
            // A single `num/den` NAME leaf.
            let name = doc_atom_name(doc, struct_ix)?;
            let (num_s, den_s) = name.split_once('/')?;
            let num = big_from_decimal(num_s)?;
            let den = big_from_decimal(den_s)?;
            Some(box_rational_normalized(&num, &den))
        }
        Shape::Tuple(elems) => {
            let elems = elems.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Tuple) e0 e1 …)` — the Tuple ctor-head atom + one child per element.
            if kids.is_empty()
                || doc_atom_ctor(doc, kids[0])? != doc::KIND_TUPLE_CTOR
                || kids.len() - 1 != elems.len()
            {
                return None;
            }
            build_arr(desc, doc, &kids[1..], &elems, depth)
        }
        Shape::Spread(elems) => {
            // A Spread is only reached as a Sum variant's payload; the Sum arm splices its children, so a
            // direct decode of a Spread shape reads the same as a Tuple's element list WITHOUT a head (the
            // caller passed exactly the element child indices). Guard arity and build the arr.
            let elems = elems.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != elems.len() {
                return None;
            }
            build_arr(desc, doc, kids, &elems, depth)
        }
        Shape::Record(fields) => {
            let fields = fields.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Record) (FieldPair name value) …)` — Record ctor-head atom + one FieldPair triple
            // per field. Fields are in descriptor (sorted) order.
            if kids.is_empty()
                || doc_atom_ctor(doc, kids[0])? != doc::KIND_RECORD_CTOR
                || kids.len() - 1 != fields.len()
            {
                return None;
            }
            let arr = op_arr_alloc(fields.len() as u32);
            for (i, (fname, fshape)) in fields.iter().enumerate() {
                let field = doc_list_kids(doc, kids[1 + i])?;
                // M2 field form `(FieldPair name value)` — a 3-element list with the FieldPair ctor head.
                // (The legacy `(name value)` 2-element pair is still accepted for back-compat.) The value
                // child is the last element; the name is matched against the descriptor's field.
                let (name_ix, value_ix) = match field.len() {
                    3 if doc_atom_ctor(doc, field[0]) == Some(doc::KIND_FIELD_PAIR) => {
                        (field[1], field[2]) // (FieldPair name value)
                    }
                    2 => (field[0], field[1]), // (name value) — legacy
                    _ => {
                        op_drop(arr);
                        return None;
                    }
                };
                if doc_atom_name(doc, name_ix)? != &**fname {
                    op_drop(arr);
                    return None;
                }
                let fval = decode_value_opt(desc, doc, value_ix, *fshape, depth + 1);
                match fval {
                    Some(h) => {
                        op_arr_set(arr, i as u32, h);
                    }
                    None => {
                        op_drop(arr);
                        return None;
                    }
                }
            }
            Some(arr)
        }
        Shape::List(elem) => {
            let elem = *elem;
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(List) e…)` — the List ctor-head atom + the elements.
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_LIST_CTOR {
                return None;
            }
            let mut v = op_vec_empty();
            for &ck in &kids[1..] {
                match decode_value_opt(desc, doc, ck, elem, depth + 1) {
                    Some(h) => {
                        v = op_vec_push(v, h);
                    }
                    None => {
                        op_drop(v);
                        return None;
                    }
                }
            }
            Some(v)
        }
        Shape::Sum(variants) => {
            let variants = variants.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // `(VariantName payload…)` — head atom is the variant name; match to its discriminant.
            if kids.is_empty() {
                return None;
            }
            let head = doc_atom_name(doc, kids[0])?;
            let (disc, (_, payload_shape)) = variants
                .iter()
                .enumerate()
                .find(|(_, (name, _))| &**name == head)?;
            let payload_shape = *payload_shape;
            // A MULTI-payload variant's payload is a `Spread`: its elements are the variant's children
            // (flattened) — build the payload arr from `kids[1..]` directly. A single/nullary payload
            // decodes the ONE payload node.
            match desc.table.get(payload_shape as usize) {
                Some(Shape::Spread(elems)) => {
                    let elems = elems.clone();
                    if kids.len() - 1 != elems.len() {
                        return None;
                    }
                    let payload = build_arr(desc, doc, &kids[1..], &elems, depth)?;
                    Some(op_sum_new(disc as u32, payload))
                }
                _ => {
                    // Single payload: exactly one child (a nullary variant's payload is `unit`).
                    if kids.len() != 2 {
                        return None;
                    }
                    let payload = decode_value_opt(desc, doc, kids[1], payload_shape, depth + 1)?;
                    Some(op_sum_new(disc as u32, payload))
                }
            }
        }
        Shape::Set(elem) => {
            let elem = *elem;
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Set) e…)` — the Set ctor-head atom + the elements directly (was the nested
            // `((. Set of) (list e…))` member-access-over-list form).
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_SET_CTOR {
                return None;
            }
            let mut s = op_set_empty();
            for &ck in &kids[1..] {
                match decode_value_opt(desc, doc, ck, elem, depth + 1) {
                    Some(h) => {
                        s = op_set_insert(s, h);
                    }
                    None => {
                        op_drop(s);
                        return None;
                    }
                }
            }
            Some(s)
        }
        Shape::Map(key, val) => {
            let (key, val) = (*key, *val);
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Map) (FieldPair k v) …)` — Map ctor-head atom + one FieldPair triple per entry.
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_MAP_CTOR {
                return None;
            }
            let mut m = op_map_empty();
            for &pair_ix in &kids[1..] {
                let pair = doc_list_kids(doc, pair_ix)?;
                // `(FieldPair key value)` — 3 elems: the FieldPair ctor head + key + value.
                if pair.len() != 3 || doc_atom_ctor(doc, pair[0]) != Some(doc::KIND_FIELD_PAIR) {
                    op_drop(m);
                    return None;
                }
                let kh = match decode_value_opt(desc, doc, pair[1], key, depth + 1) {
                    Some(h) => h,
                    None => {
                        op_drop(m);
                        return None;
                    }
                };
                let vh = match decode_value_opt(desc, doc, pair[2], val, depth + 1) {
                    Some(h) => h,
                    None => {
                        op_drop(kh);
                        op_drop(m);
                        return None;
                    }
                };
                m = op_map_insert(m, kh, vh);
            }
            Some(m)
        }
    }
}

/// Build a fresh `arr` (the runtime rep of a tuple/record/spread) from `kids` (one doc child per element)
/// decoded against `shapes` (parallel element shape indices). On any element mismatch, drops the
/// partially-built arr and returns `None`. Caller guarantees `kids.len() == shapes.len()`.
fn build_arr(
    desc: &Descriptor,
    doc: &ParsedDoc,
    kids: &[u32],
    shapes: &[u32],
    depth: u32,
) -> Option<Handle> {
    let arr = op_arr_alloc(shapes.len() as u32);
    for (i, (&ck, &sh)) in kids.iter().zip(shapes.iter()).enumerate() {
        match decode_value_opt(desc, doc, ck, sh, depth + 1) {
            Some(h) => {
                op_arr_set(arr, i as u32, h);
            }
            None => {
                op_drop(arr);
                return None;
            }
        }
    }
    Some(arr)
}

/// Parse a base-10 decimal string (optional leading `-`) into a `bigint::Big`. `None` on a non-digit
/// character. Used for the Rational `num/den` name-leaf components.
fn big_from_decimal(s: &str) -> Option<bigint::Big> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ten = bigint::Big::from_i64(10);
    let mut acc = bigint::Big::zero();
    for b in digits.bytes() {
        acc = acc.mul(&ten);
        acc = acc.add(&bigint::Big::from_i64((b - b'0') as i64));
    }
    if neg {
        acc = acc.neg();
    }
    Some(acc)
}

/// value-decode (heap idx 90): parse the value-form `doc_bytes` and, guided by `desc`, construct a fresh
/// owned heap value. `Handle::NULL` on a malformed document / descriptor mismatch (never traps).
//= spec/contracts/deterministic-value-form.md#the-canonical-byte-form-has-a-decode-that-inverts-it
//# Decoding the canonical byte encoding of a value against the type of that value MUST yield a value equal, under the language's structural equality, to the value that was encoded.
//= spec/contracts/deterministic-value-form.md#decoding-refuses-bytes-that-are-not-a-value-of-the-expected-type
//# Decoding a byte sequence that is not the canonical byte encoding of any value of the expected type MUST be refused rather than yield a value, so that a decode never misinterprets bytes as a value they do not encode.
fn op_value_decode(doc_bytes: &[u8], desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return Handle::NULL;
    };
    let Some(parsed) = parse_doc(doc_bytes) else {
        return Handle::NULL;
    };
    decode_value(&descriptor, &parsed, parsed.root, descriptor.root, 0)
}

// ─── Bytes: a packed immutable byte buffer (in `raw`) ───────────────────────────────────
// OOB into a valid buffer traps; null is benign.

// The shared IMMORTAL empty-BYTES singleton (lazily minted, census-excluded) — see op_bytes_alloc.
runtime_local! {
    static EMPTY_BYTES: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

fn op_bytes_alloc(len: u32) -> Handle {
    // len==0 → the shared IMMORTAL empty-BYTES singleton (the IMM_UNIT analog for bytes): an empty bytes
    // value is CONSTANT, so allocate it ONCE, immortal (census-excluded), reuse. SOUND: an empty bytes is
    // never mutated in place — bytes-set on it is OOB (traps, 0 slots), concat builds a fresh rope node,
    // and bytes_flatten is a no-op on an already-flat empty leaf. So the singleton is read-only.
    if len == 0 {
        return EMPTY_BYTES.with(|slot| {
            let mut e = slot.get();
            if e.0.is_null() {
                e = alloc_raw(
                    Vec::new(),
                    Raw::Inline {
                        len: 0,
                        buf: [0u8; INLINE_RAW_CAP],
                    },
                );
                op_mark_immortal(e);
                slot.set(e);
            }
            e
        });
    }
    // A ≤INLINE_RAW_CAP-byte buffer (a short string/section leaf — the common case when assembling a
    // rope from many small pieces) builds its zero-filled raw INLINE, skipping the transient `vec![0u8;
    // len]` that `alloc` would otherwise copy into the inline `Raw` and immediately free. That transient
    // Vec was pure malloc/free churn on the hot leaf-build path (dominant in a rope-assembly profile).
    if (len as usize) <= INLINE_RAW_CAP {
        return alloc_raw(
            Vec::new(),
            Raw::Inline {
                len: len as u8,
                buf: [0u8; INLINE_RAW_CAP],
            },
        );
    }
    alloc(Vec::new(), vec![0u8; len as usize])
}
/// Store a byte (the compiler guarantees `value` is 0–255) and return the buffer handle. OOB into a
/// valid buffer traps; null is a no-op.
fn op_bytes_set(buf: Handle, index: u32, value: u32) -> Handle {
    if is_immediate(buf) {
        return buf; // defensive (mirrors op_bytes_get/len): a bytes buffer is never an immediate;
        // return the handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { buf.node_mut() } {
        None => {}
        Some(n) => match n.raw.as_mut_slice().get_mut(index as usize) {
            Some(slot) => *slot = value as u8,
            None => trap_oob(),
        },
    }
    buf
}
/// `bytes-get` — the logical byte at `index`. A leaf reads `raw` directly (O(1)); a rope node
/// (slice/concat) is FLATTENED to a leaf in place on this first full-read, then read (see
/// `bytes_flatten` — this is what keeps the compiler's `0..len` emit loop O(n) instead of O(n²) on a
/// deep concat chain). OOB into a valid buffer traps; null is benign.
fn op_bytes_get(buf: Handle, index: u32) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    // Leaf fast path (and null-benign): today's behavior, unchanged.
    let is_leaf = match unsafe { buf.node_ref() } {
        None => return 0,
        Some(n) => n.handles.is_empty(),
    };
    if is_leaf {
        return match unsafe { buf.node_ref() } {
            None => 0,
            Some(n) => match n.raw.get(index as usize) {
                Some(&b) => b as u32,
                None => trap_oob(),
            },
        };
    }
    // Rope node: bounds-check against the logical length (so a stray OOB doesn't force a flatten),
    // then materialize once. After flatten, `buf` is a leaf; re-read it.
    if index >= op_bytes_len(buf) {
        trap_oob();
    }
    bytes_flatten(buf);
    with_node(buf, 0, |n| {
        n.raw.get(index as usize).map(|&b| b as u32).unwrap_or(0)
    })
}

/// `bytes-len` — the logical byte length. O(1): a leaf's is its physical `raw` length; a rope node
/// stores it (concat as `raw=[len]`, slice as `raw=[off, len]`) so length never walks the tree.
/// Reached only on a value the compiler statically types as Bytes (tagless dispatch), so a `handles`
/// length of 1 here always means a Bytes slice, never a vector header of the same physical shape.
fn op_bytes_len(buf: Handle) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    with_node(buf, 0, |n| match n.handles.len() {
        0 => n.raw.len() as u32,     // leaf: physical bytes
        1 => read_u32_at(&n.raw, 4), // slice [parent]: raw = [off, len] → len at offset 4
        _ => read_u32_at(&n.raw, 0), // concat [left, right]: raw = [len]
    })
}

// ─── Bytes rope: O(1) concat/slice over shared leaves, flatten-on-read ────────────────────
// A Bytes value is a rope of shared slices/concats bottoming out in leaves, so `bytes-concat` and
// `bytes-slice` are O(1) and copy no bytes until observed — killing the O(n²) copy cascade a compiler
// would otherwise hit assembling a module by concatenating sections (deferred materialization behind
// the observable bytes, value-heap-runtime.md §Deferred Materialization Is Permitted Behind The
// Observable Bytes). Same tagless trick as the persistent vector:
// no new `Node` field, children in `handles`, so the existing iterative `op_drop` reclaims a rope
// and a shared leaf survives until its last owner drops — zero new RC machinery. Ownership follows
// the `arr-set` convention: concat/slice/compact CONSUME their Bytes operands (stored in `handles`
// without dup), so the free cascade Just Works.

/// Materialize a rope node (slice/concat) into a leaf IN PLACE: fill `raw` with its logical bytes,
/// release the children it owned, clear `handles`. Content-preserving, so UNOBSERVABLE and safe even
/// when the node is shared (`rc > 1`) — every sharer sees identical bytes before and after (the
/// memory model's #Sharing Is Not Observable deferral clause). A leaf is left untouched (idempotent).
/// The fill is ITERATIVE (an explicit `(node, dst_off, src_start, count)` worklist) so a deep rope
/// cannot overflow the wasm call stack — same discipline as the free cascade.
///
/// A slice/concat rope IS a value combined or narrowed from existing Bytes whose materialization is
/// DEFERRED to this flatten: the deferral is unobservable (every reader sees identical logical bytes) and
/// a deterministic function of the source rope, so combining/narrowing need not eagerly copy.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# A value the compiler derives by combining or narrowing existing values MAY defer the work of materializing its contents until an operation observes them, provided the deferral is not observable and is a deterministic function of the source, so that combining and narrowing values need not eagerly copy their contents.
fn bytes_flatten(h: Handle) {
    let arity = with_node(h, 0usize, |n| n.handles.len());
    if arity == 0 {
        return; // already a leaf
    }
    let len = op_bytes_len(h) as usize;
    // SMALL fast path (the dominant per-char case: `String.at` → a 1-byte slice → compact): the flattened
    // bytes fit the inline `Raw` cap, so materialize them into a STACK buffer and install an inline `Raw`
    // — skipping BOTH transient heap Vecs the general path allocates (the `dst` output Vec AND the `work`
    // seed Vec), which for a ≤12-byte result are pure malloc/free churn (the Vec is copied into inline
    // storage then freed — the transient-small-Vec smell). Was 2 allocs/flatten; a small flatten now
    // allocates NOTHING. BYTE-IDENTICAL: `Raw::from` inlines a ≤cap Vec anyway, so the leaf is the same.
    if len <= INLINE_RAW_CAP {
        let mut buf = [0u8; INLINE_RAW_CAP];
        fill_rope_bytes(h, &mut buf[..len], len);
        let children = match unsafe { h.node_mut() } {
            Some(n) => {
                n.raw = Raw::Inline {
                    len: len as u8,
                    buf,
                };
                n.handles.take()
            }
            None => return,
        };
        for &c in children.iter() {
            op_drop(c);
        }
        return;
    }
    let mut dst = vec![0u8; len];
    fill_rope_bytes(h, &mut dst, len);
    // Convert `h` to a leaf: install the bytes and take its children out (so `h` no longer references
    // them), THEN release those references. Order matters — `h` is a leaf before the drops, so a
    // child freed here can never be reached through `h`.
    let children = match unsafe { h.node_mut() } {
        Some(n) => {
            n.raw = Raw::from(dst); // the flattened bytes (a wide rope leaf → Heap)
            n.handles.take() // the (now-orphaned) rope children (an owned `Handles`) to drop below
        }
        None => return,
    };
    for &c in children.iter() {
        op_drop(c);
    }
}

/// Copy the `len` logical bytes of rope `h` into `dst` (`dst.len() == len`). The iterative walk (an
/// explicit `(node, dst_off, src_start, count)` worklist) means a deep rope cannot overflow the wasm
/// call stack. READ-ONLY on every node (the caller mutates `h`'s node AFTER this returns), so briefly
/// holding a shared ref per node is sound even across a shared subgraph. Shared by both `bytes_flatten`
/// size paths so the copy logic lives once. The worklist is REUSED from `FLATTEN_SCRATCH` (thread-local:
/// clear + refill), so after the first flatten it never allocates again — the walk is allocation-FREE
/// steady-state, and combined with the caller's stack-buffer output for a ≤cap result, a small flatten
/// (the hot per-char `String.at`-compact) allocates NOTHING.
fn fill_rope_bytes(h: Handle, dst: &mut [u8], len: usize) {
    FLATTEN_SCRATCH.with(|cell| {
        let work = &mut *cell.borrow_mut();
        work.clear();
        work.push((h, 0, 0, len));
        while let Some((node, dst_off, src_start, count)) = work.pop() {
            if count == 0 {
                continue;
            }
            let n = match unsafe { node.node_ref() } {
                Some(n) => n,
                None => continue, // null child — benign
            };
            match n.handles.len() {
                0 => {
                    // leaf: copy the sub-range, trapping on any inconsistency (a compiler-bug rope).
                    match (
                        n.raw.get(src_start..src_start + count),
                        dst.get_mut(dst_off..dst_off + count),
                    ) {
                        (Some(src), Some(d)) => d.copy_from_slice(src),
                        _ => trap_oob(),
                    }
                }
                1 => {
                    // slice [parent], raw = [off, len]: logical byte j is parent's byte (off + j).
                    let parent = n.handles[0];
                    let off = read_u32_at(&n.raw, 0) as usize;
                    work.push((parent, dst_off, off + src_start, count));
                }
                _ => {
                    // concat [left, right], raw = [len]: [0, ll) from left, [ll, len) from right.
                    let left = n.handles[0];
                    let right = n.handles[1];
                    let ll = op_bytes_len(left) as usize;
                    if src_start < ll {
                        let lcount = count.min(ll - src_start);
                        work.push((left, dst_off, src_start, lcount));
                        let rcount = count - lcount;
                        if rcount > 0 {
                            work.push((right, dst_off + lcount, 0, rcount));
                        }
                    } else {
                        work.push((right, dst_off, src_start - ll, count));
                    }
                }
            }
        }
    });
}

/// `bytes-concat(a, b)` — a new Bytes = the bytes of `a` then `b`. O(1): allocates one concat node,
/// copies nothing. CONSUMES `a` and `b`. Empty operand is the identity (returns the other, dropping
/// the empty one to honor consume-semantics), matching the corpus identity law.
fn op_bytes_concat(a: Handle, b: Handle) -> Handle {
    let la = op_bytes_len(a);
    let lb = op_bytes_len(b);
    if la == 0 {
        op_drop(a);
        return b;
    }
    if lb == 0 {
        op_drop(b);
        return a;
    }
    // Logical length is u32 across the ABI (`bytes-len -> u32`); a > 4 GiB Bytes is unrepresentable
    // on wasm32, so an overflow here is a compiler-invariant violation → trap.
    let total = match la.checked_add(lb) {
        Some(t) => t,
        None => trap_oob(),
    };
    // Concat rope node: [left, right] handles + inline 4-byte `[len]` raw. Build the 2-element handles
    // INLINE (`inline_from`) rather than `vec![a, b]` — a concat is arity-2 = exactly INLINE_HANDLES_CAP,
    // so a heap Vec would be allocated then immediately re-inlined + freed by `From<Vec>` (the transient-
    // Vec smell). Direct inline construction = the node Box only, one fewer alloc per concat.
    alloc_raw(
        Handles::inline_from(&[a, b]),
        Raw::inline(&total.to_le_bytes()),
    )
}

/// `bytes-slice(buf, start, len)` — a new Bytes = `len` bytes of `buf` from `start`. O(1): one slice
/// node, no copy. Total-or-trap: `start + len > bytes-len(buf)` traps (checked in `u64`); `len == 0`
/// is the empty Bytes (never a trap, even at `start == len`). CONSUMES `buf`. A slice OF a slice is
/// collapsed into the grandparent (`slice(p, off1+start, len)`) to bound rope depth.
///
/// The slice SHARES the parent's storage — its representation holds the parent handle live (`op_dup`),
/// so the parent buffer is genuinely RETAINED (its rc reflects the slice), not hidden: the storage the
/// slice value retains is exactly the storage it holds live, and `op_drop` of the slice releases the
/// parent reference.
//= spec/capabilities/memory-and-resource-model.md#retained-storage-is-what-a-value-s-representation-holds-live
//# The storage a value retains MUST be the storage its representation actually holds live, so that a value that shares another value's storage keeps the shared storage retained rather than hidden.
fn op_bytes_slice(buf: Handle, start: u32, len: u32) -> Handle {
    let blen = op_bytes_len(buf);
    if (start as u64) + (len as u64) > (blen as u64) {
        trap_oob();
    }
    if len == 0 {
        op_drop(buf); // consume the operand; the empty result is independent
        return op_bytes_alloc(0);
    }
    // Collapse slice-of-slice: if `buf` is itself a slice into `parent` at `off1`, point the new
    // slice straight at `parent` (dup it — the new node owns a reference; dropping `buf` releases
    // buf's, net parent rc unchanged). Bounds slice-chain depth at 1.
    let collapse = with_node(buf, None, |n| {
        if n.handles.len() == 1 {
            Some((n.handles[0], read_u32_at(&n.raw, 0)))
        } else {
            None
        }
    });
    if let Some((parent, off1)) = collapse {
        op_dup(parent);
        op_drop(buf);
        // slice-of-slice: 1 handle + inline [off,len]. Build the handle INLINE (arity-1 ≤ cap) — a
        // `vec![parent]` would allocate a heap Vec then get re-inlined + freed by `From<Vec>`.
        return alloc_raw(
            Handles::inline_from(&[parent]),
            slice_raw(off1 + start, len),
        );
    }
    // slice node: 1 handle + inline [off,len]. Inline the single handle (no transient heap Vec).
    alloc_raw(Handles::inline_from(&[buf]), slice_raw(start, len))
}

/// The 8-byte `[off][len]` raw header of a bytes SLICE node, built INLINE (no transient heap Vec).
fn slice_raw(off: u32, len: u32) -> Raw {
    let mut buf = [0u8; INLINE_RAW_CAP];
    buf[0..4].copy_from_slice(&off.to_le_bytes());
    buf[4..8].copy_from_slice(&len.to_le_bytes());
    Raw::Inline { len: 8, buf }
}

/// `bytes-compact(buf)` — a Bytes equal to `buf` by content whose storage is INDEPENDENT of any
/// larger buffer `buf` was sliced from (memory-and-resource-model.md #Retained Storage: derive a
/// value that releases the parent's storage without changing the value). Falls out of the rope for
/// free: flattening `buf` in place materializes its own bytes and drops the parent it pinned, and a
/// leaf is already independent. CONSUMES and returns `buf` (now a leaf).
fn op_bytes_compact(buf: Handle) -> Handle {
    bytes_flatten(buf);
    buf
}

/// `str-nfc-normalize` (heap op 89, FINDING#23) — normalize a runtime String value to Unicode Normalization
/// Form C. A String is a UTF-8 byte leaf (possibly a rope): flatten `s`, read its logical bytes, normalize
/// them to NFC via the imported `cadenza:nfc/normalize` component (the runtime's DEPENDENCY — the heavy
/// Unicode tables live THERE, not in this runtime), and return a FRESH OWNED flat String leaf of the NFC
/// bytes. CONSUMES `s` (drops it; returns the fresh leaf) — the same spend-the-input contract as
/// `bytes-compact`/`str-to-bytes`. Idempotent (the imported `nfc` is). The NFC import is only linked in the
/// wasm component build; a native `cargo test` has no NFC component, so the call is gated to wasm and the
/// native build normalizes to a no-op passthrough (the native suite exercises the flatten/leaf plumbing, not
/// NFC content — NFC correctness is covered by cdz-nfc's own unit tests + the corpus witness).
#[cfg(target_arch = "wasm32")]
fn op_str_nfc(s: Handle) -> Handle {
    bytes_flatten(s);
    let bytes = unsafe { s.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    let normalized = bindings::cadenza::nfc::normalize::nfc(bytes);
    // FAST PATH (the common case — ASCII / already-NFC text): if normalization changed nothing, `s` is
    // ALREADY its own NFC form, so return the input handle unchanged rather than allocating a fresh leaf +
    // dropping `s`. This matches the op's stated contract ("same handle, near-free for already-NFC") and
    // avoids heap churn on the overwhelmingly common path (most runtime text is ASCII or pre-composed).
    // Only a genuinely decomposed input (rare) allocates the fresh normalized leaf.
    if normalized == bytes {
        return s;
    }
    let out = alloc(Vec::new(), normalized);
    op_drop(s);
    out
}

/// Native stand-in for `op_str_nfc` (no NFC component linked off-wasm): flatten + return `s` unchanged. The
/// native suite covers the flatten/leaf plumbing; NFC content correctness lives in cdz-nfc's unit tests.
#[cfg(not(target_arch = "wasm32"))]
fn op_str_nfc(s: Handle) -> Handle {
    bytes_flatten(s);
    s
}

// ─── String: a stored UTF-8 leaf (bytes in `raw`) ───────────────────────────────────────

// The shared IMMORTAL empty-STRING singleton (lazily minted, census-excluded) — see op_str_new.
runtime_local! {
    static EMPTY_STR: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

fn op_str_new(s: String) -> Handle {
    // Empty string → the shared IMMORTAL empty-STRING singleton (the IMM_UNIT analog for strings): an
    // empty string is CONSTANT, so allocate it ONCE, immortal (census-excluded), reuse. SOUND: an empty
    // string is never mutated in place — String.concat builds a fresh rope node, and bytes_flatten /
    // str-get are no-ops on an already-flat empty leaf. Read-only singleton.
    if s.is_empty() {
        return EMPTY_STR.with(|slot| {
            let mut e = slot.get();
            if e.0.is_null() {
                e = alloc(Vec::new(), Vec::new());
                op_mark_immortal(e);
                slot.set(e);
            }
            e
        });
    }
    alloc(Vec::new(), s.into_bytes())
}
fn op_str_get(h: Handle) -> String {
    if is_immediate(h) {
        return String::new(); // cross-kind totality: a string is never itself an immediate
    }
    // A runtime String IS a bytes rope: `String.concat`/`.at`-slice build concat/slice nodes (a String
    // shares the Bytes rope representation, `@b77b3ae0`), so `h` may be a rope whose `raw` holds the node's
    // HEADER bytes (a concat's `[len]`, a slice's `[off, len]`), NOT the content. MATERIALIZE it to a flat
    // leaf first (iterative `bytes_flatten`, so a deep rope can't overflow the stack; content-preserving,
    // so unobservable even on a shared value) — exactly as `op_bytes_get` and value-encode's `Shape::Str`
    // arm do. Without the flatten a rope String read back its raw handle/length bytes as UTF-8 (garbage).
    // A flat leaf is left untouched (flatten is a no-op there), so a plain `str-new` string is unaffected.
    bytes_flatten(h);
    with_node(h, String::new(), |n| {
        String::from_utf8_lossy(&n.raw).into_owned()
    })
}

/// `String.from-bytes` — the TOTAL UTF-8 decode `Bytes → (Option String)`: validate a RUNTIME byte
/// buffer as well-formed UTF-8 (strict: rejects invalid bytes, overlong encodings, AND surrogate code
/// points — the three spec failure modes), returning the buffer AS a String (Some) or NULL (None). A
/// String IS a byte leaf (`op_str_new` = `alloc(bytes)`, byte-identical to a Bytes leaf), so a VALID
/// buffer needs no conversion — it is already a valid String; the op is UTF-8 VALIDATION + a re-tag.
/// CONSUMES `buf`: on success `buf` flows out as the String (its ownership transfers to the result); on
/// failure the caller drops it. FLATTEN first (`buf` may be a rope — a `Bytes.concat`/`.slice` tree —
/// whose `raw` holds header bytes, NOT content; strict `from_utf8` must see the actual bytes), exactly as
/// `op_str_get`/`op_bytes_get`/value-encode's `Shape::Str` arm do. Returns `Handle::NULL` for invalid
/// UTF-8 so the compiler can build the `(Option String)` sum (`Some buf` / `None`), or wrap directly.
///
/// WIT-EXPORTED at index 85 (`str-from-bytes`) — the runtime half of the coordinated `String.from-bytes`
/// op, called by the `Guest::str_from_bytes` method. The compiler emits it (`Core::StrFromBytes`) when
/// `String.from-bytes` is applied to a RUNTIME byte sequence (a constant `Bytes.of` still folds in
/// lower.rs). The load-bearing logic (flatten + strict validate + consume/re-tag) lives here.
fn op_str_from_bytes(buf: Handle) -> Handle {
    if is_immediate(buf) {
        // The empty/inline-unit Bytes: no bytes → the empty string is valid UTF-8. `buf` (an immediate)
        // is itself a valid empty leaf-equivalent; return it (an immediate is a fine empty String).
        return buf;
    }
    bytes_flatten(buf);
    let valid = with_node(buf, false, |n| core::str::from_utf8(&n.raw).is_ok());
    if valid {
        buf // already a flat, valid-UTF-8 leaf — a String IS a byte leaf, no conversion
    } else {
        op_drop(buf); // ill-formed → None; release the consumed operand
        Handle::NULL
    }
}

/// `String.scalar-at` — the codepoint of the `scalar_index`-th UNICODE SCALAR of a String, or the
/// sentinel `NO_SCALAR` (`u32::MAX`) when the index is out of range. The SCALAR index is NOT the byte
/// index: a String is a UTF-8 byte-rope, so the Nth scalar can start at any byte offset (`"café"` has
/// byte-len 5 but scalar-len 4 — its scalar 3 `'é'` is a 2-byte encoding at byte offset 3). Returns the
/// scalar's Unicode codepoint as a `u32` (a `Char` at the language level is that codepoint immediate) —
/// UNLIKE `String.at`, whose `(Option String)` payload is a one-scalar SLICE ROPE that the physical
/// `champ_eq` mis-compares (the rope-eq bug the compiler-in-Cadenza lexer WORKS AROUND by lexing
/// `List Int64` char-codes). A `Char` codepoint is an ordinary integer, so comparing two of them is a
/// plain `i32.eq` — no rope, no content-eq hazard: this is the op a real text lexer wants.
///
/// FLATTEN first (`buf` may be a `Bytes.concat`/`.slice` rope whose `raw` holds header bytes, not
/// content) — iterative, so a deep rope can't overflow; content-preserving, so UNOBSERVABLE on a shared
/// value — exactly as `op_str_get`/`op_bytes_get`/`str-from-bytes` do. BORROWS `buf` (an indexed read,
/// no consume). Decodes the flat leaf as UTF-8 and takes the Nth `char`; a well-formed String always
/// decodes, but an ill-formed buffer (defensive) reads as `NO_SCALAR`, never a trap.
///
/// WIT-EXPORTED as `bytes-scalar-at` (the runtime half of the `str-scalar-at` op). Returns the codepoint
/// or `NO_SCALAR`=u32::MAX (out-of-range / ill-formed), so the compiler maps that sentinel to `None` when
/// building the `(Option Char)` sum. PENDING the compiler side: `String.scalar-at` on a RUNTIME string
/// still declines at lower.rs `lower_str_scalar_at` ("constant strings only"; the constant case folds to a
/// `Leaf::Char`) until a `Core::StrScalarAt` variant + backend emit (i32 codepoint → Char box, sentinel →
/// None) is wired — that is a compiler-variant addition (v-compiler-primitives/v-rust-backend), tracked to
/// flip corpus 13-strings:3218. The flatten + UTF-8 scalar walk is done and proven here. The SCALAR-indexed
/// String family (`scalar-len`/`scalar-at`/`slice`) all rest on this same UTF-8 walk.
///
/// COST: COST — O(scalar_index): reaching the i-th scalar walks the UTF-8 from the START (a String is not
/// scalar-indexable in O(1) — variable-width encoding). This is INHERENT to random access by scalar
/// index, NOT a defect. WARNING: CONSEQUENCE for the compiler agent: a LEXER that scans a string left-to-right
/// via repeated `scalar-at(s, 0)`, `scalar-at(s, 1)`, … is O(N²) (measured: ~67 ns/scalar at N=64 rising
/// to ~3300 ns/scalar at N=4096). A sequential scan wants a CURSOR (`scalar-next(buf, byte_off) ->
/// (codepoint, next_byte_off)`, advancing by the scalar's width) — that would be a SEPARATE coordinated
/// op (a different ABI: a pair return). `scalar-at` is the right primitive for RANDOM access; do NOT
/// build a left-to-right lexer on it. (The current compiler-in-Cadenza lexer sidesteps the whole area by
/// lexing `List Int64` char-codes — which is O(N) via `List` iteration, so the cursor gap is not yet
/// blocking; raise the cursor only when a real-String sequential scan is written.)
fn op_bytes_scalar_at(buf: Handle, scalar_index: u32) -> u32 {
    const NO_SCALAR: u32 = u32::MAX; // out-of-range / ill-formed sentinel (not a valid Unicode scalar)
    if is_immediate(buf) {
        return NO_SCALAR; // the empty/inline-unit Bytes has no scalars — any index is out of range
    }
    bytes_flatten(buf);
    with_node(buf, NO_SCALAR, |n| match core::str::from_utf8(&n.raw) {
        Ok(s) => s
            .chars()
            .nth(scalar_index as usize)
            .map(|c| c as u32)
            .unwrap_or(NO_SCALAR),
        Err(_) => NO_SCALAR, // ill-formed (defensive — a well-formed String always decodes)
    })
}

// NOTE: a prepared-but-unexported `op_bytes_eq_content` (a borrowing flatten-both + `champ_eq` content
// equality) lived here to unblock the `String.at`-content-equality miscompile. RETIRED `spec@<this>`:
// the compiler fixed that bug the OTHER way — COMPACT-AT-PRODUCER (compact the `bytes-slice` to a flat
// leaf in the `Core::StrAt` emit + compact rope operands before `value-eq`/CHAMP-key, backend/wasm/
// select.rs), which the existing consuming `bytes-compact` op already serves. So the borrowing content-eq
// had no remaining coordination path and was dead maintenance surface (unexported → DCE'd → hash-neutral
// either way); removed it + its test. The underlying primitives it composed — `bytes_flatten` +
// `champ_eq` — stay thoroughly covered by the collection fuzzers + the `compact_makes_a_*_canonical`
// contract tests. (`op_str_from_bytes` is now WIT-EXPORTED at index 85 — the string round-trip blocker is
// wired; `op_bytes_scalar_at` remains prepared-but-unexported: scalar-at the lexer's random-access read.)

// ─── Map: dynamic-key collection of (key, value) handle pairs, stored verbatim ──────────
// Pairs are flattened into `handles` as [k0, v0, k1, v1, …]; pair count = handles.len() / 2. OOB
// pair index into a valid map traps; null is benign.

fn op_map_alloc(len: u32) -> Handle {
    alloc(vec![Handle::NULL; (len as usize) * 2], Vec::new())
}
/// Write the (key, value) pair at `index` and return the map handle (for convenient threading). OOB
/// pair index into a valid map traps; null is a no-op.
fn op_map_set(m: Handle, index: u32, key: Handle, value: Handle) -> Handle {
    if is_immediate(m) {
        return m; // defensive (mirrors the map readers): a map is never an immediate; return the
        // handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { m.node_mut() } {
        None => {}
        Some(n) => {
            let base = (index as usize) * 2;
            if base + 1 < n.handles.len() {
                n.handles.set(base, key);
                n.handles.set(base + 1, value);
            } else {
                trap_oob();
            }
        }
    }
    m
}
fn op_map_key(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_map_val(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2 + 1) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_map_len(m: Handle) -> u32 {
    with_node(m, 0, |n| (n.handles.len() / 2) as u32)
}

// ─── Reference-count calling convention (Perceus) ───────────────────────────────────────
// Written as `Handle`-typed core ops so the whole RC discipline is developed and tested natively,
// against real node pointers, before the `u32` wasm boundary ever sees it. The compiler emits `drop`
// where a heap value is released (a dead heap binding, and the resource destructor), so a compound's
// storage is reclaimed; it does NOT yet emit `dup` (the current escape/return paths transfer ownership
// rather than share), so `dup`'s call sites arrive when a construct first shares a handle.

/// `dup` — a new reference to `h` is being retained: increment its refcount. Null is a no-op.
/// The reserved refcount sentinel of an IMMORTAL heap node (a build-once static held by a module global for
/// the whole instance). `dup`/`drop` are NO-OPS on it (it is never retained or freed), and it is excluded
/// from the live-objects census — an immortal is not a leak, exactly like the inline `IMM_UNIT`. `u32::MAX`
/// is safe as the sentinel: it is unreachable by real refcounting (it would require 4 billion live dups),
/// and being `!= 1` it makes every FBIP `rc == 1` in-place path conservatively path-copy, so a shared
/// immortal is never mutated. Set by `op_mark_immortal`; checked by `op_dup`/`op_drop`.
const IMMORTAL: u32 = u32::MAX;

/// `mark-immortal(handle)` (heap index 95) — convert a freshly-built heap node into an IMMORTAL one (see
/// [`IMMORTAL`]): its refcount becomes the sentinel so `dup`/`drop` no-op on it and it leaves the census.
/// The node was already counted at `alloc`, so converting it DECREMENTS the census (debug counter) to net it
/// to zero. Idempotent (a re-mark does not double-decrement). An immediate has no node and is returned
/// unchanged (already census-free + rc-noop). GENERAL over any heap node; returns the same handle.
fn op_mark_immortal(h: Handle) -> Handle {
    if is_immediate(h) {
        return h;
    }
    if let Some(node) = unsafe { h.node_mut() }
        && node.rc != IMMORTAL
    {
        node.rc = IMMORTAL;
        #[cfg(any(test, feature = "debug-counters"))]
        LIVE_NODES.with(|n| n.set(n.get() - 1));
    }
    h
}

/// `mark-immortal-deep(handle)` (heap index 96) — the TRANSITIVE [`op_mark_immortal`]: mark the root node
/// AND every node reachable through child handles IMMORTAL. For a build-once static whose value is a
/// MULTI-NODE heap structure with no compile-time per-node handle — a `>32` RRB list (interior + leaf
/// nodes) or a CHAMP map (interior nodes + `[k,v]` data entries). The walk is over `node.handles` — the
/// SAME child set `op_drop`'s free-cascade scans — so a map's key+value handles, a list's element handles,
/// and any nested compound payloads are ALL marked, not just the spine (else the payloads would stay
/// mortal and leak, or be freed under the immortal). ITERATIVE (an explicit worklist, no recursion) so a
/// deep RRB trie cannot overflow the wasm stack. IDEMPOTENT + DAG-safe: an already-IMMORTAL node is
/// skipped, so a shared node (persistent structures share) marks exactly once — no double census-decrement,
/// no cycle. An immediate (non-heap) handle owns no node and is skipped. Returns the same root handle.
fn op_mark_immortal_deep(root: Handle) -> Handle {
    // A LIFO worklist of handles yet to mark. Seeded with the root; a node's children are pushed as it is
    // marked. Handles are `Copy` (a pointer/immediate), so pushing a child READS it — the node stays live
    // and immortal (unlike `op_drop`, which takes the handles as it frees).
    let mut worklist: Vec<Handle> = Vec::new();
    worklist.push(root);
    while let Some(cur) = worklist.pop() {
        if is_immediate(cur) {
            continue; // an immediate owns no heap node — nothing to mark
        }
        if let Some(node) = unsafe { cur.node_mut() }
            && node.rc != IMMORTAL
        {
            node.rc = IMMORTAL;
            #[cfg(any(test, feature = "debug-counters"))]
            LIVE_NODES.with(|n| n.set(n.get() - 1));
            // Mark this node's children transitively. `handles` derefs to `[Handle]`, covering the inline
            // (≤2, e.g. a CHAMP `[k,v]` entry) and heap-spilled (a wide RRB/CHAMP node) cases uniformly.
            for &child in node.handles.iter() {
                worklist.push(child);
            }
        }
        // An already-IMMORTAL node: skip (its subtree was already marked on the path that first reached it).
    }
    root
}

fn op_dup(h: Handle) {
    if is_immediate(h) {
        return; // an immediate owns no heap — nothing to retain
    }
    if let Some(node) = unsafe { h.node_mut() } {
        // UAF/wild-handle guard (debug only): retaining a freed or fabricated cell is a bug.
        #[cfg(any(test, feature = "debug-counters"))]
        assert_node_live(h.0, node.guard, "dup");
        if node.rc != IMMORTAL {
            node.rc += 1; // an IMMORTAL node is never retained (dup is a no-op — the global owns it forever)
        }
    }
}

/// The refcount of `h` (0 for null). The FBIP fast paths read this to decide, PER NODE, whether the
/// node is uniquely owned (`rc == 1`, safe to mutate in place) or shared (`rc > 1`, must path-copy) —
/// the aliasing-safety rule: a shared node backs another persistent version and must stay byte-identical.
fn node_rc(h: Handle) -> u32 {
    if is_immediate(h) {
        // An immediate is not a Node. Return a non-1 sentinel (2) so every FBIP `rc == 1` in-place
        // path takes the conservative copy and NEVER tries to mutate the tagged bits as a Node.
        return 2;
    }
    with_node(h, 0, |n| n.rc)
}

/// `drop` — a reference to `h` is being released: decrement its refcount, and at zero free the node
/// and release the references it owned (which may cascade). The compiler emits the `drop` call at a
/// source-determined point (the value's last use), so reclamation is deterministic, not a background
/// collector's choice:
///
//= spec/capabilities/memory-and-resource-model.md#cleanup-is-source-determined
//# The point at which a value's storage is released MUST be a deterministic function of the source.
///
/// Fast paths, cheapest first: a **shared** node (`rc > 1`) is a bare decrement — no scan, no
/// reclamation. A **leaf** (empty `handles`) costs no worklist allocation: `mem::take` of an empty
/// `Vec` does not allocate, so the loop below simply doesn't run. Only a unique COMPOUND seeds a
/// worklist — and it reuses the freed node's OWN `handles` vector as that seed (it is already
/// allocated and the node is about to die), so no fresh allocation for the root level.
///
/// The cascade is ITERATIVE, over an explicit worklist — NOT recursive. A recursive free would grow
/// the wasm call stack proportionally to structure DEPTH and could overflow it on a deep unique
/// list/tree (the same host-stack limit that bounds deep recursion elsewhere). The worklist bounds
/// stack use to O(1) frames; total work is still O(n) in the freed subtree. `LIVE_NODES` (tests)
/// lets us assert the whole subtree is reclaimed and peak heap stays bounded across iterations.
fn op_drop(root: Handle) {
    if is_immediate(root) {
        return; // an immediate owns no heap — nothing to release
    }
    let node = match unsafe { root.node_mut() } {
        Some(n) => n,
        None => return, // null — benign
    };
    // UAF/wild-handle guard (debug only): dropping a freed cell is a double-free.
    #[cfg(any(test, feature = "debug-counters"))]
    assert_node_live(root.0, node.guard, "drop (double-free)");
    if node.rc == IMMORTAL {
        return; // an IMMORTAL node is never freed (a module global holds it) — drop is a no-op. MUST come
        // before the `rc > 1` decrement, else the sentinel would erode toward 1 and free the static.
    }
    if node.rc > 1 {
        node.rc -= 1; // shared: cheapest path, no reclamation
        return;
    }
    // rc == 1: last reference. Reclaim the node and cascade into its children.
    //
    // The worklist is allocated LAZILY: an inline node's ≤2 children are pushed straight onto the
    // (initially-empty) worklist, and a `Vec` is materialized only if/when a HEAP child is expanded (a
    // node with >2 children — which necessarily already owns a heap Vec, so the cascade is heap-bound
    // there regardless). This keeps the dominant case — dropping a small (≤2-child, often all-immediate)
    // compound like a tuple/sum/`[k,v]` — ALLOCATION-FREE, matching the pre-inline behavior where the
    // freed node's own handle Vec was reused as the worklist seed. `SmallVec`-style: the seed lives in a
    // fixed `[Handle; INLINE_HANDLES_CAP]` until a spill is unavoidable.
    let mut seed_buf = [Handle::NULL; INLINE_HANDLES_CAP];
    let mut seed_len = 0usize;
    // The worklist REUSES a dying heap node's own `Vec` as its backing rather than allocating a fresh
    // one (the pre-inline behavior — the freed node's handle Vec was going to be freed anyway, so using
    // it as scratch is a zero-alloc cascade). It stays empty (no alloc) until the FIRST heap node is
    // reached, whose Vec it adopts by move; inline nodes' ≤2 children fill `seed_buf` with no heap at all.
    let mut worklist: Vec<Handle> = Vec::new();
    // Seed from the root: an inline root fills the buffer; a heap root donates its Vec as the worklist.
    match &mut node.handles {
        Handles::Inline { buf, len } => {
            seed_buf[..*len as usize].copy_from_slice(&buf[..*len as usize]);
            seed_len = *len as usize;
        }
        Handles::Heap(v) => worklist = core::mem::take(v),
    }
    // Release the root. DEBUG: bump the generation ODD (= freed) and RETAIN the cell (release its
    // raw/handle backings to bound debug memory, but leak the shell so the address stays a detectable
    // freed cell for the UAF guards above). SHIPPED: deallocate as before — this arm is byte-for-byte the
    // original free, so the release runtime is unchanged.
    #[cfg(any(test, feature = "debug-counters"))]
    {
        node.guard = freed_guard(root.0);
        node.raw.clear();
        node.handles = Handles::default();
    }
    #[cfg(not(any(test, feature = "debug-counters")))]
    unsafe {
        drop(Box::from_raw(root.0));
    }
    #[cfg(any(test, feature = "debug-counters"))]
    LIVE_NODES.with(|n| n.set(n.get() - 1));

    loop {
        // Pop from the worklist first (deeper heap subtrees), then drain the inline seed.
        let cur = match worklist.pop() {
            Some(c) => c,
            None if seed_len > 0 => {
                seed_len -= 1;
                seed_buf[seed_len]
            }
            None => break,
        };
        if is_immediate(cur) {
            continue; // an inline child owns no heap — the hottest RC path (doc-named)
        }
        let n = match unsafe { cur.node_mut() } {
            Some(n) => n,
            None => continue, // null child slot — benign
        };
        // UAF/wild-handle guard (debug only): a freed child still referenced by a dying compound is a
        // double-free / dangling child.
        #[cfg(any(test, feature = "debug-counters"))]
        assert_node_live(cur.0, n.guard, "drop-cascade (dangling child)");
        if n.rc == IMMORTAL {
            continue; // an IMMORTAL child (e.g. a shared build-once static nested in a dying compound) is
            // never freed and its count is untouched — skip it, do not decrement toward freeing.
        }
        if n.rc > 1 {
            n.rc -= 1; // shared child survives; freed only when its last owner drops it
            continue;
        }
        // Move this node's children onto the pending set, then free it. An inline child-set with room
        // fills the seed buffer (no alloc). Otherwise: if the worklist is still empty, ADOPT this node's
        // own Vec as the worklist backing (a heap node owns one; reuse it — zero alloc, as the pre-inline
        // cascade did); if the worklist is already backed, append into it.
        match &mut n.handles {
            Handles::Inline { buf, len } if seed_len + *len as usize <= INLINE_HANDLES_CAP => {
                seed_buf[seed_len..seed_len + *len as usize].copy_from_slice(&buf[..*len as usize]);
                seed_len += *len as usize;
            }
            Handles::Heap(v) if worklist.is_empty() => {
                // Adopt the dying node's Vec (no alloc), then fold any pending inline-seed items in.
                worklist = core::mem::take(v);
                if seed_len > 0 {
                    worklist.extend_from_slice(&seed_buf[..seed_len]);
                    seed_len = 0;
                }
            }
            _ => {
                if seed_len > 0 {
                    worklist.extend_from_slice(&seed_buf[..seed_len]);
                    seed_len = 0;
                }
                n.handles.append_into(&mut worklist);
            }
        }
        // Release the child (see the root free above): DEBUG bumps the generation odd + retains for UAF
        // detection; SHIPPED deallocates byte-for-byte as before.
        #[cfg(any(test, feature = "debug-counters"))]
        {
            n.guard = freed_guard(cur.0);
            n.raw.clear();
            n.handles = Handles::default();
        }
        #[cfg(not(any(test, feature = "debug-counters")))]
        unsafe {
            drop(Box::from_raw(cur.0));
        }
        #[cfg(any(test, feature = "debug-counters"))]
        LIVE_NODES.with(|n| n.set(n.get() - 1));
    }
}

// ─── Reuse / FBIP (Perceus reset + reuse-aware constructors) ──────────────────────────────
// The in-place-update win: when a unique value is consumed and a value is rebuilt in the same
// breath (List.map, a functional record/cons rebuild), reuse the dying node's shell for the new
// one instead of free→malloc. Frame-limited by construction (research P3/P4): reuse fires ONLY on a
// UNIQUE node (`rc == 1`), so a reused cell is memory that was already live and about to die — peak
// heap cannot grow, and because no other reference observes the difference the reuse is invisible:
//
//= spec/capabilities/memory-and-resource-model.md#reuse-is-not-observable
//# When the compiler reuses a value's storage in place because no other reference to that value can observe the difference, that reuse MUST NOT change the program's observable behavior, so that reusing storage is a transparent optimization rather than a mutation of a value.
//
// The three ops form a two-step protocol the compiler emits:
//
//   token = reset(old);                        // old unique → emptied shell as a token; else null
//   new   = arr-alloc-reuse(len, token);       // token non-null → refit that shell; else fresh
//   …or   = sum-new-reuse(disc, payload, token);
//
// A reuse TOKEN is just a childless `rc == 1` node. It obeys the ordinary ownership ABI: it is
// CONSUMED by exactly one reuse-constructor, OR — if a control path doesn't rebuild — `drop`ped
// (which, on a childless unique node, frees exactly the shell). No separate "free token" op needed.
//
// Ordering the compiler must honor (the §4 dup-before-drop invariant, applied to reset): any child
// of `old` reused in the rebuild (e.g. recursing into a tree's subtrees, or reading a field into
// the new value) must be `dup`'d BEFORE `reset(old)`, because reset drops `old`'s references to its
// children. This is exactly the calling convention's existing rule; reset is a drop-for-its-shell.

/// `reset` — drop `node` for reuse. If UNIQUE (`rc == 1`): release the children it owns (a normal
/// cascading `drop` of each child reference — shared grandchildren survive), then RETAIN the emptied
/// shell (rc still 1, no children, no raw) and return it as a non-null reuse token. If SHARED
/// (`rc > 1`): decrement and return `NULL` — the other owners keep the node intact, so there is
/// nothing to reuse. Null in → null out. The returned token feeds a `*-reuse` constructor, or is
/// `drop`ped if unused (freeing the bare shell). Reuses the node's own handle/raw Vec backings, so a
/// same-arity refit performs no reallocation at all.
fn op_reset(node: Handle) -> Handle {
    if is_immediate(node) {
        return Handle::NULL; // an immediate owns no shell to reuse (covers the borrows below)
    }
    // Read rc through a short-lived borrow that ends before we recurse into children.
    let rc = match unsafe { node.node_ref() } {
        Some(n) => n.rc,
        None => return Handle::NULL, // null: nothing to reuse
    };
    if rc > 1 {
        if let Some(n) = unsafe { node.node_mut() } {
            n.rc -= 1; // shared: another owner keeps it intact; no reuse token
        }
        return Handle::NULL;
    }
    // Unique. Take the children out (ending the borrow before the drops), release each, then put
    // the now-empty backing Vec back so the shell keeps its allocation for the coming refit.
    let mut children = match unsafe { node.node_mut() } {
        Some(n) => core::mem::take(&mut n.handles),
        None => return Handle::NULL,
    };
    for &child in children.iter() {
        op_drop(child); // cascades; a child dup'd by the compiler before reset survives
    }
    children.clear(); // 0 elements, capacity retained
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles = children; // restore the (empty, capacity-bearing) backing
        n.raw.clear();
    }
    node // the retained shell, rc == 1, empty — a reuse token
}

/// `arr-alloc-reuse` — `arr-alloc(len)`, but reusing `token`'s shell when it is a non-null reuse
/// token from `reset`: refit it to `len` NULL slots (reusing its handle-Vec backing when capacity
/// allows — the common same-length case reallocates nothing) and return it, allocating NO new node.
/// A null token allocates fresh, so a `reset` that declined to yield a token is transparent.
fn op_arr_alloc_reuse(len: u32, token: Handle) -> Handle {
    if is_immediate(token) {
        return op_arr_alloc(len); // defensive: reset never yields an immediate token
    }
    // Normalize (P2 canonical form): an empty array IS unit, and unit ALWAYS inlines — no boxed
    // twin may exist. A `len == 0` refit would otherwise return a BOXED empty node, forking the rep.
    // The token came from `op_reset` (rc == 1, childless), so drop it to free the shell — not reused,
    // not leaked — and return the canonical inline unit. `op_arr_alloc(0)` === `imm_unit()`.
    if len == 0 {
        op_drop(token); // childless unique shell → frees exactly the token node
        return imm_unit();
    }
    match unsafe { token.node_mut() } {
        None => op_arr_alloc(len),
        Some(n) => {
            n.rc = 1;
            // Refit the handles to `len` NULL slots, matching what a FRESH `op_arr_alloc(len)` produces:
            // ≤cap → INLINE, wider → a heap Vec. A wide reset token carries a `Handles::Heap` whose Vec
            // `clear()`/`resize` KEEP (clear retains capacity; resize only spills inline→heap, never
            // re-inlines heap→inline) — so refitting it SMALL would leave a stray heap Vec where the fresh
            // node is inline: a retained allocation for the node's life AND a forked storage rep for one
            // logical value, invisible to `champ_eq`/`champ_hash` (they read via `as_slice`). This is the
            // handles-arm twin of the raw-arm divergence normalized below. Assign a fresh inline `Handles`
            // for a ≤cap refit (dropping any leftover heap Vec); reuse the token's Vec backing in place
            // only for a WIDE refit (the FBIP win — the common same-length refit reallocates nothing).
            if (len as usize) <= INLINE_HANDLES_CAP {
                n.handles = Handles::inline_nulls(len as usize);
            } else {
                n.handles.clear();
                n.handles.resize(len as usize, Handle::NULL);
            }
            // Reset to an EMPTY INLINE raw (an array node carries no raw). `raw.clear()` would keep a
            // heap buffer if the token came from a reset bytes/string leaf — an empty heap Vec retained
            // for the node's life, and a non-canonical rep vs the inline-empty raw a fresh `op_arr_alloc`
            // produces. Assigning the inline-empty raw drops that spill and matches the fresh node.
            n.raw = Raw::Inline {
                len: 0,
                buf: [0u8; INLINE_RAW_CAP],
            };
            token
        }
    }
}

/// `sum-new-reuse` — `sum-new(disc, payload)`, but reusing `token`'s shell when non-null: repurpose
/// it as the `(disc, payload)` node with no new allocation. Null token allocates fresh.
fn op_sum_new_reuse(disc: u32, payload: Handle, token: Handle) -> Handle {
    if is_immediate(token) {
        return op_sum_new(disc, payload); // defensive: reset never yields an immediate token
    }
    match unsafe { token.node_mut() } {
        None => op_sum_new(disc, payload),
        Some(n) => {
            n.rc = 1;
            // A sum node is ALWAYS arity 1 (a single payload), so a fresh `op_sum_new` gives INLINE
            // handles. A wide reset token carries a `Handles::Heap` whose Vec `clear()` + `push` KEEP the
            // heap arm (clear retains capacity; push on a Heap stays Heap) — leaving the reused sum node
            // carrying a stray heap Vec where the fresh node is inline: a retained allocation AND a forked
            // storage rep, invisible to `champ_eq`/`champ_hash` (the handles-arm twin of the raw-arm
            // divergence normalized below). Assign a fresh inline single-payload `Handles` directly,
            // dropping any leftover heap Vec — matching `op_sum_new` byte-for-byte.
            n.handles = Handles::inline_from(&[payload]);
            // Assign a fresh INLINE raw rather than clear()+extend_from_slice: if the token came from a
            // reset bytes/string leaf its raw was `Heap`, and clear() keeps a heap buffer (Vec::clear
            // semantics) — so extending 4 disc bytes into it would leave the reused sum node carrying a
            // HEAP raw where a fresh `op_sum_new` gives an INLINE one. That both retains a stray heap
            // allocation and forks the canonical storage rep for one logical value. A direct inline
            // assignment drops any heap spill and matches `op_sum_new` byte-for-byte.
            n.raw = Raw::inline(&disc.to_le_bytes());
            token
        }
    }
}

// ─── Persistent vector — a 32-way radix trie ──────────────────────────────────────────────
// A persistent (immutable, structurally-shared) growable sequence, laid out as a Bagwell/Clojure
// 32-way radix trie over the SAME tagless `Node`. No new node field and no change to the free
// cascade — exactly the bytes rope's trick: a vector's nodes are ordinary
// `Node`s whose children live in `handles`, so structural sharing is just `rc > 1` on a shared
// subtree and the existing iterative `op_drop` reclaims a whole trie transitively. Sharing a subtree
// is transparent: a shared node is immutable and byte-identical to a copied one, so whether a version
// shares its predecessor's storage or copies it never changes what any op observes — the persistent
// update path-copies the spine and `op_dup`s the retained children, so the two are indistinguishable.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# When the compiler represents a value by sharing another value's storage rather than by copying it, that sharing MUST NOT change the program's observable behavior, so that sharing storage is a transparent optimization rather than a distinction between two equal values.
// Tagless dispatch keeps this from colliding with tuples/lists: the compiler only ever calls `vec-*` on a value whose
// static type is a Vec and `arr-*` on a tuple/list, so `handles`/`raw` are interpreted as a vector
// only inside these ops (same argument as the rope, §3 of that doc).
//
// # Layout
// - **Header** (the `vec` handle): `raw = [count: u32, shift: u32]`, `handles = [root]` (empty for
//   `count == 0`). `shift` is the root-level radix shift, `BITS * (levels - 1)`; a leaf-only tree
//   (≤ 32 elements) has `shift == 0`.
// - **Trie node** (interior or leaf): `handles = [child/element handles]` (arity 1..=32), `raw = []`.
//   Interior vs leaf is NOT stored — it is determined by the descent depth (`shift`), never a tag.
//   Elements are dense (indices `0..count`), so the trie is "left-full": every branch is full except
//   the rightmost path, and index `i`'s path is its base-32 digits (`(i >> level) & MASK`).
//
// # Ownership (value-heap-runtime.md §Constructors Consume And Accessors Borrow)
// `vec-empty` produces a new owned vector. `vec-push`/`vec-update` are CONSTRUCTORS: they **consume**
// the input vector `v` and the element, and produce a new owned vector — the old version is untouched
// (persistence), so a caller keeping both versions `dup`s `v` before the call (§3.1). Path-copying
// `dup`s every shared subtree carried into the new version, then `op_drop`s the consumed `v`; this is
// correct whether `v` is unique (drop frees the old rightmost spine; shared subtrees survive via the
// dup) or shared (drop just decrements; both versions co-own the shared subtrees). `vec-get` BORROWS
// (rc unchanged; the vector still owns the element); `vec-len` returns a `u32` by value.
//
// # Complexity & bounds
// O(log₃₂ N) index / update / push, sharing all-but-one root→leaf path per update and all-but-the
// rightmost spine per push. Trie height is ≤ 7 for any `u32` count (32⁷ > 2³²), so the trie descent
// recurses at most 7 deep — bounded, unlike the free cascade (which stays iterative in `op_drop` for
// deep UNIQUE structures). FBIP reuse of a unique `v`'s spine (`vec_push_fbip`) and RRB relaxed nodes
// for O(log N) concat/split (`op_vec_concat`/`op_vec_split`, `vec_is_relaxed`) are implemented. One
// optimization remains DEFERRED, with an identical observable contract (so it needs no WIT change): a
// Clojure-style *tail* for amortized-O(1) push.

/// Radix branching bits: 32-way (2⁵) fan-out, the Bagwell/Clojure default.
const VEC_BITS: u32 = 5;
/// Radix digit mask: `(1 << VEC_BITS) - 1` — extracts one base-32 digit of an index.
const VEC_MASK: u32 = (1 << VEC_BITS) - 1;

/// Read a little-endian `u32` at byte `off` of `raw`, zero-padded past the end (total: a short raw
/// yields 0, never a panic — same discipline as `read_word`/`read_disc`).
///
/// FAST PATH: when the full 4-byte window is in bounds (the ALWAYS case for a real CHAMP header —
/// `champ_datamap`/`champ_nodemap`/`champ_size` on a 12-byte raw — and vec headers / relaxed size
/// tables), read the 4-byte subslice in ONE `from_le_bytes` with a single bounds check, instead of the
/// four per-byte `.get()` bounds checks the general loop does. This is on the hottest descent
/// (`champ_find_base_h` reads datamap+nodemap PER LEVEL) so the per-byte branches were measurable
/// (`Raw::as_slice`/`read_u32_at` ~3% of the profile). Byte-identical output. The zero-padded loop
/// stays for the defensive short/absent-raw tail (e.g. `champ_become_hdr`'s pre-resize probe).
#[inline]
fn read_u32_at(raw: &[u8], off: usize) -> u32 {
    if let Some(window) = raw.get(off..off + 4) {
        // SAFETY-FREE: `window` is exactly 4 bytes, so the array conversion cannot fail.
        return u32::from_le_bytes(window.try_into().unwrap());
    }
    let mut b = [0u8; 4];
    for k in 0..4 {
        if let Some(&byte) = raw.get(off + k) {
            b[k] = byte;
        }
    }
    u32::from_le_bytes(b)
}

// ─── Packed-bool vector leaves (memory-dense `List Bool`) ─────────────────────────────────────
// A `List Bool` LEAF stores its ≤32 boolean elements BIT-PACKED into a single `u32` instead of as up
// to 32 separate `imm_bool` handles in a heap `Vec` — ~6× denser (5 inline bytes vs a heap Vec of 32
// pointers) and one fewer allocation per leaf. This is a PURE-RUNTIME optimization: the compiler emits
// the identical `vec-*` ops for a `List Bool`, and the runtime auto-detects a bool element at leaf
// construction (`vec_leaf_of`/`op_vec_of_arr`) — no WIT op, no hint, no type channel.
//
// A leaf is PACKED iff `handles` is EMPTY and `raw` is exactly `[count: u8][bits: u32 LE]` = 5 bytes
// (`PACKED_BOOL_LEAF_RAW_LEN`), stored INLINE. Bit `i` of `bits` (LSB-first) is element `i`; bits at or
// above `count` are 0. The 5-byte length is the discriminant: within the vec subsystem a strict leaf
// carries its elements in `handles` (empty raw), a relaxed node's raw is `4*arity` bytes, a vec header's
// raw is 8, so no other trie node collides — and `vec_leaf_is_packed` is only ever asked of a genuine
// trie node (a leaf or interior), never of an element, so an unrelated 5-byte bytes/scalar VALUE that
// happens to be a list element is never misread as a packed leaf.
//
// WHY IT IS UNOBSERVABLE. A `List` is already non-byte-canonical (a concat-built vector has relaxed
// nodes where an `of-arr`-built one is strict, for the same logical value), so `ty_heap_walkable`
// returns `false` for `Ty::List` — a list is NEVER structurally `value-eq`'d nor used as a map/set key
// (never `champ_hash`/`champ_eq`'d). Packing adds a THIRD leaf shape beside strict/relaxed; a tree may
// freely MIX them (e.g. a packed leaf beside an unpacked one after a concat) and still read correctly,
// because every read funnels through `vec_arity`/`vec_child` (below) which decode a packed leaf on the
// fly into a count and `imm_bool` elements. A bool `imm_bool` is an `op_dup`/`op_drop`/`node_rc` no-op,
// so handing synthesized immediates back to callers that dup/drop them is ownership-trivial.

/// The `raw` length of a packed-bool leaf: `[count: u8]` + `[bits: u32 LE]`.
const PACKED_BOOL_LEAF_RAW_LEN: usize = 5;

/// Whether `h` is an inline boolean immediate (the element type a packed leaf holds). A non-immediate
/// (a heap node) or a non-bool immediate (unit/int) is not — so a non-`Bool` list never packs.
#[inline]
fn imm_is_bool(h: Handle) -> bool {
    is_immediate(h) && matches!(imm_kind(h), ImmKind::Bool)
}

/// Whether `node` is a packed-bool leaf (empty handles + a 5-byte `[count][bits]` raw). Total: a null
/// handle, an immediate, or any other node shape yields `false`.
#[inline]
fn vec_leaf_is_packed(node: Handle) -> bool {
    with_node(node, false, |n| {
        n.handles.is_empty() && n.raw.len() == PACKED_BOOL_LEAF_RAW_LEN
    })
}

/// The element count of a packed leaf (its `raw[0]`). Caller has verified `vec_leaf_is_packed`.
#[inline]
fn packed_leaf_count(node: Handle) -> usize {
    with_node(node, 0, |n| n.raw.first().copied().unwrap_or(0) as usize)
}

/// The `count` and `bits` of a packed leaf in one borrow. Caller has verified `vec_leaf_is_packed`.
#[inline]
fn packed_leaf_parts(node: Handle) -> (u8, u32) {
    with_node(node, (0, 0), |n| {
        (n.raw.first().copied().unwrap_or(0), read_u32_at(&n.raw, 1))
    })
}

/// Element `i` (`i < count ≤ 32`) of a packed leaf as an `imm_bool`. Caller has verified
/// `vec_leaf_is_packed`; `i` is a leaf slot (`idx & VEC_MASK`) so `i < 32` and the shift never overflows.
#[inline]
fn packed_leaf_get(node: Handle, i: usize) -> Handle {
    let (_, bits) = packed_leaf_parts(node);
    imm_bool((bits >> i) & 1 != 0)
}

/// Build the 5-byte `[count][bits]` raw of a packed leaf, inline (no heap).
#[inline]
fn packed_leaf_raw(count: u8, bits: u32) -> Raw {
    let mut buf = [0u8; PACKED_BOOL_LEAF_RAW_LEN];
    buf[0] = count;
    buf[1..5].copy_from_slice(&bits.to_le_bytes());
    Raw::inline(&buf)
}

/// A freshly-owned packed leaf (rc 1) of `count` bools whose values are `bits` (LSB-first).
#[inline]
fn packed_leaf_new(count: u8, bits: u32) -> Handle {
    alloc_raw(Handles::new(), packed_leaf_raw(count, bits))
}

/// Set (or clear) bit `i` of `bits` to `v`.
#[inline]
fn set_bit(bits: u32, i: usize, v: bool) -> u32 {
    (bits & !(1u32 << i)) | ((v as u32) << i)
}

/// Convert an rc==1 packed leaf IN PLACE back to a normal strict leaf (elements as `imm_bool` handles,
/// empty raw). The defensive escape hatch for the (well-typed-impossible) case of a NON-bool element
/// joining a `List Bool` leaf — a list is homogeneous, so a packed leaf only ever exists in a `List Bool`
/// whose every element is a bool immediate, and this never fires for well-typed code; it keeps the leaf
/// mutators TOTAL (deterministic, never a miscompile) if the compiler ever emitted a mixed list.
fn packed_leaf_unpack_inplace(node: Handle) {
    let (count, bits) = packed_leaf_parts(node);
    if let Some(n) = unsafe { node.node_mut() } {
        let mut hs = Handles::new();
        for i in 0..count as usize {
            hs.push(imm_bool((bits >> i) & 1 != 0));
        }
        n.handles = hs;
        n.raw = Raw::from(Vec::new());
    }
}

/// PATH-COPY append of element `e` to a packed leaf: a fresh packed leaf of `count + 1` bits with bit
/// `count` = `e`'s value. `e` (a bool immediate) is consumed with no drop (an immediate owns no heap).
/// The original leaf is untouched (the caller releases it), matching `vec_node_append`'s dup-siblings
/// contract (a packed leaf has no siblings to dup). Defensive strict-leaf fallback if `e` is not a bool
/// or the leaf is somehow full (well-typed-impossible for a `List Bool` — a leaf-level append has room).
fn packed_leaf_append(node: Handle, e: Handle) -> Handle {
    let (count, bits) = packed_leaf_parts(node);
    if imm_is_bool(e) && (count as usize) < 32 {
        return packed_leaf_new(count + 1, set_bit(bits, count as usize, imm_as_bool(e)));
    }
    let mut hs = Vec::with_capacity(count as usize + 1);
    for i in 0..count as usize {
        hs.push(imm_bool((bits >> i) & 1 != 0));
    }
    hs.push(e);
    alloc(hs, Vec::new())
}

/// PATH-COPY replace of element `sub` of a packed leaf with `e`: a fresh packed leaf with bit `sub` =
/// `e`'s value. `e` (bool imm) consumed with no drop; the replaced element (also a bool imm) needs none.
/// The original leaf is untouched. Defensive strict-leaf fallback if `e` is not a bool.
fn packed_leaf_replace(node: Handle, sub: usize, e: Handle) -> Handle {
    let (count, bits) = packed_leaf_parts(node);
    if imm_is_bool(e) {
        return packed_leaf_new(count, set_bit(bits, sub, imm_as_bool(e)));
    }
    let mut hs = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        hs.push(if i == sub {
            e
        } else {
            imm_bool((bits >> i) & 1 != 0)
        });
    }
    alloc(hs, Vec::new())
}

/// IN-PLACE append (FBIP, rc==1) of `e` to a packed leaf: bump `count` and set bit `count`, patching
/// the 5-byte raw. SAFETY: caller verified rc == 1. Defensive unpack-then-push if `e` is not a bool or
/// the leaf is full.
fn packed_leaf_push_inplace(node: Handle, e: Handle) {
    if imm_is_bool(e) {
        let (count, bits) = packed_leaf_parts(node);
        if (count as usize) < 32 {
            if let Some(n) = unsafe { node.node_mut() } {
                n.raw = packed_leaf_raw(count + 1, set_bit(bits, count as usize, imm_as_bool(e)));
            }
            return;
        }
    }
    packed_leaf_unpack_inplace(node);
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(e);
    }
}

/// IN-PLACE set (FBIP, rc==1) of element `sub` of a packed leaf to `e`: patch bit `sub` in the raw.
/// SAFETY: caller verified rc == 1. The caller separately releases the OLD element (a bool imm — a
/// drop no-op). Defensive unpack-then-set if `e` is not a bool.
fn packed_leaf_set_inplace(node: Handle, sub: usize, e: Handle) {
    if imm_is_bool(e) {
        let (count, bits) = packed_leaf_parts(node);
        if let Some(n) = unsafe { node.node_mut() } {
            n.raw = packed_leaf_raw(count, set_bit(bits, sub, imm_as_bool(e)));
        }
        return;
    }
    packed_leaf_unpack_inplace(node);
    if let Some(n) = unsafe { node.node_mut() } {
        if let Some(slot) = n.handles.get_mut(sub) {
            *slot = e;
        }
    }
}

/// Build a leaf from a slice of ≤32 element handles: PACKED when they are all bool immediates (the
/// `List Bool` case), else a normal strict leaf. Consumes the handles (moved into the leaf, or read as
/// bits — bool immediates own no heap so no drop is needed). Used by the `op_vec_of_arr` >32 chunking
/// path so a large `List Bool` literal packs every leaf, matching a push-built one.
fn vec_leaf_from_handles(hs: Vec<Handle>) -> Handle {
    if !hs.is_empty() && hs.len() <= 32 && hs.iter().all(|&e| imm_is_bool(e)) {
        let mut bits = 0u32;
        for (i, &e) in hs.iter().enumerate() {
            if imm_as_bool(e) {
                bits |= 1 << i;
            }
        }
        return packed_leaf_new(hs.len() as u8, bits);
    }
    alloc(hs, Vec::new())
}

/// If `arr` (an array node whose `handles` ARE its elements) holds 1..=32 elements that are ALL bool
/// immediates, return their packed bits (LSB = element 0); else `None`. Used to pack a `List Bool`
/// literal at `op_vec_of_arr` instead of reusing the arr node as a strict leaf.
fn arr_all_bool_bits(arr: Handle) -> Option<u32> {
    with_node(arr, None, |n| {
        let els = n.handles.as_slice();
        if els.is_empty() || els.len() > 32 || !els.iter().all(|&e| imm_is_bool(e)) {
            return None;
        }
        let mut bits = 0u32;
        for (i, &e) in els.iter().enumerate() {
            if imm_as_bool(e) {
                bits |= 1 << i;
            }
        }
        Some(bits)
    })
}

/// The count of a trie node's children (its arity). A null node has none (benign). A PACKED leaf reports
/// its bit count so every element-count reader (get/len/subtree-size/split/concat/invariant walks) sees
/// a packed leaf as a leaf of `count` elements with no other change.
fn vec_arity(node: Handle) -> usize {
    if vec_leaf_is_packed(node) {
        return packed_leaf_count(node);
    }
    with_node(node, 0, |n| n.handles.len())
}
/// The `i`-th child handle of a trie node, or NULL if absent (benign — the descent stays within a
/// valid tree by construction, so this never returns NULL in correct operation). A PACKED leaf decodes
/// bit `i` into an `imm_bool` on the fly, so every reader (leaf reads, dup-collect, split partition) sees
/// the same `imm_bool` elements it would from an unpacked leaf.
fn vec_child(node: Handle, i: usize) -> Handle {
    if vec_leaf_is_packed(node) {
        return packed_leaf_get(node, i);
    }
    with_node(node, Handle::NULL, |n| {
        n.handles.get(i).copied().unwrap_or(Handle::NULL)
    })
}

/// Build a vector header owning `root` (or childless when `root` is NULL, i.e. the empty vector).
fn vec_alloc_header(count: u32, shift: u32, root: Handle) -> Handle {
    // The 8-byte `[count][shift]` vector header, built INLINE (no transient heap Vec).
    let mut raw = [0u8; INLINE_RAW_CAP];
    raw[0..4].copy_from_slice(&count.to_le_bytes());
    raw[4..8].copy_from_slice(&shift.to_le_bytes());
    // A header holds its single root handle (or none, for the empty vector) — always ≤ 1 child, and it
    // NEVER grows past one (a root swap replaces slot 0 via `vec_set_child_inplace`; a level-grow builds a
    // fresh 2-child root, not a 2nd header handle). So carry the handle INLINE (`inline_from`) rather than
    // `vec![root]` — a header on EVERY vector construction otherwise pays a heap `Vec` alloc for one handle,
    // where the inline arm (cap 2) holds it in the `Node` itself. Reads/mutations are storage-transparent.
    let handles = if root == Handle::NULL {
        Handles::new()
    } else {
        Handles::inline_from(&[root])
    };
    alloc_raw(handles, Raw::Inline { len: 8, buf: raw })
}

/// Decode a header into `(count, shift, root)`. Borrows — no ownership change. A null/short header
/// yields the empty-vector triple.
fn vec_read_header(v: Handle) -> (u32, u32, Handle) {
    with_node(v, (0, 0, Handle::NULL), |n| {
        (
            read_u32_at(&n.raw, 0),
            read_u32_at(&n.raw, 4),
            n.handles.first().copied().unwrap_or(Handle::NULL),
        )
    })
}

/// A one-element leaf node holding `e` (consumed into it).
fn vec_leaf_of(e: Handle) -> Handle {
    if imm_is_bool(e) {
        // A `List Bool` leaf packs: element 0's value goes in bit 0. `e` is an immediate (nothing to
        // consume — no heap), so no drop is needed. Subsequent pushes grow this packed leaf in place.
        return packed_leaf_new(1, imm_as_bool(e) as u32);
    }
    // Born on the HEAP arm: an RRB leaf grows toward 32 elements via in-place `vec_push_child_inplace`,
    // so inlining it (≤2) would only pay a spill on the 3rd push with no lasting benefit (it ends up
    // heap regardless). `from_vec_heap` keeps the single-element Vec as the backing to grow into.
    alloc_raw(Handles::from_vec_heap(vec![e]), Raw::from(Vec::new()))
}

/// Append `child` (consumed) to a trie node, `dup`ing the existing children into the copy — the
/// container gains an owned reference to each carried-over subtree while the old node keeps its own
/// (the subtree is now shared). Used both for a leaf gaining an element and an interior gaining a
/// branch; the two are the same op over `handles`.
fn vec_node_append(node: Handle, child: Handle) -> Handle {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0) gains one element — grow the packed bits, no per-sibling dup
        // (its elements live in the raw, not in `handles`).
        return packed_leaf_append(node, child);
    }
    // Deref `node` ONCE and copy its children from the borrowed slice (was one `vec_child` deref +
    // null-check per sibling — up to 32 for a strict node; the compiler can't hoist them because
    // `op_dup` writes child memory, defeating alias analysis on the parent). `op_dup(c)` mutates a
    // CHILD node's rc, disjoint from the parent `&Node` we hold — sound (a well-formed RRB tree has
    // no cycles, so no child aliases its parent).
    let mut hs = with_node(node, Vec::new(), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len() + 1);
        for &c in children {
            op_dup(c);
            hs.push(c);
        }
        hs
    });
    hs.push(child);
    alloc(hs, Vec::new())
}

/// Copy `node`, replacing child `sub` with `new_child` (consumed) and `dup`ing every sibling into the
/// copy (shared). This is the path-copy step: one new node per level, all off-path subtrees shared.
/// Emits a STRICT copy (empty raw); use it only when `node` is strict, or when the replacement changes
/// the node's kind. For a relaxed node whose sizes are unchanged (e.g. an in-place element update),
/// use `vec_node_replace_keep_raw` so the size table survives the copy.
fn vec_node_replace(node: Handle, sub: usize, new_child: Handle) -> Handle {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0): replace bit `sub`. Its "siblings" are bits, not handles, so
        // there is nothing to dup and the old element (a bool imm) needs no drop.
        return packed_leaf_replace(node, sub, new_child);
    }
    // One deref of `node` + copy from the borrowed slice (was a `vec_child` deref per sibling). See
    // `vec_node_append` for the aliasing argument (op_dup mutates a disjoint CHILD node).
    let hs = with_node(node, Vec::new(), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len());
        for (j, &c) in children.iter().enumerate() {
            if j == sub {
                hs.push(new_child);
            } else {
                op_dup(c);
                hs.push(c);
            }
        }
        hs
    });
    alloc(hs, Vec::new())
}

/// Like `vec_node_replace`, but carries `node`'s `raw` (its relaxed size table) into the copy verbatim.
/// Correct only when the replacement does NOT change any child's element count — the case for
/// `vec-update`, which swaps one leaf element for another. Preserves the strict-vs-relaxed kind: a
/// strict node (empty raw) stays strict, a relaxed node keeps its table.
fn vec_node_replace_keep_raw(node: Handle, sub: usize, new_child: Handle) -> Handle {
    // One deref of `node` — copy the children from the borrowed slice AND read the raw size table in
    // the same borrow (was a `vec_child` deref per sibling PLUS a separate `with_node` for the raw).
    let (hs, raw) = with_node(node, (Vec::new(), Vec::new()), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len());
        for (j, &c) in children.iter().enumerate() {
            if j == sub {
                hs.push(new_child);
            } else {
                op_dup(c);
                hs.push(c);
            }
        }
        (hs, n.raw.to_vec())
    });
    alloc(hs, raw)
}

/// Push-append helper for a RELAXED node whose last child gained exactly one element: copy the node
/// replacing child `last` with `new_child` (consumed, siblings shared) and bump ONLY the final
/// cumulative-size entry by 1 (every preceding boundary is unchanged since only the last child grew).
fn vec_relaxed_grow_last(node: Handle, last: usize, new_child: Handle) -> Handle {
    let copy = vec_node_replace_keep_raw(node, last, new_child);
    // `copy` is a fresh sole owner (rc 1) carrying the old size table; add 1 to its final u32 entry.
    // Mutate in place via `as_mut` — the same discipline the reuse ops use for a just-allocated node.
    if let Some(n) = unsafe { copy.node_mut() } {
        let off = n.raw.len() - 4; // raw.len() == 4*arity ≥ 4 for a relaxed node
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
    copy
}

/// Push-append helper for a RELAXED node whose last child is full: copy the node appending `branch`
/// (consumed) as a new rightmost child covering exactly one new element, extending the size table with
/// `old_total + 1`.
fn vec_relaxed_append_branch(node: Handle, branch: Handle) -> Handle {
    let arity = vec_arity(node);
    let mut hs = Vec::with_capacity(arity + 1);
    for j in 0..arity {
        let c = vec_child(node, j);
        op_dup(c);
        hs.push(c);
    }
    hs.push(branch);
    let old_total = if arity == 0 {
        0
    } else {
        vec_relaxed_size_at(node, arity - 1)
    };
    let mut raw = with_node(node, Vec::new(), |n| n.raw.to_vec());
    raw.extend_from_slice(&(old_total + 1).to_le_bytes());
    alloc(hs, raw)
}

/// Build a fresh single-child spine from `level` down to `node` at level 0 (the new-branch case:
/// when a push starts a subtree that does not yet exist). Consumes `node`.
fn vec_new_path(level: u32, node: Handle) -> Handle {
    if level == 0 {
        node
    } else {
        // Arity-1 single-child spine node — build the handle INLINE (no transient `vec![child]` heap Vec
        // that `From<Vec>` would re-inline + free).
        alloc_raw(
            Handles::inline_from(&[vec_new_path(level - VEC_BITS, node)]),
            Raw::from(Vec::new()),
        )
    }
}

/// Insert element `e` at dense index `i` into the subtree rooted at `node` (borrowed), path-copying.
/// At a leaf (`level == 0`) `e` is appended; at an interior node the rightmost existing child is
/// path-copied (`sub < arity`) or a brand-new branch is appended (`sub == arity`). Returns a new
/// owned subtree; `e` is consumed.
fn vec_push_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
    if level == 0 {
        vec_node_append(node, e)
    } else if vec_is_relaxed(node) {
        // Relaxed interior: a push always lands in (or just after) the RIGHTMOST child — appending is
        // strictly a right-edge operation. The last child holds `1 << level` elements at most; if it is
        // not yet full, recurse into it (rebased index) and bump the final size entry; otherwise start a
        // fresh branch and extend the size table. All other boundaries are untouched.
        let arity = vec_arity(node);
        let last = arity - 1; // relaxed nodes always have arity ≥ 1 (no empty nodes)
        let last_size = vec_relaxed_child_size(node, last);
        // The last child is a subtree at `level - VEC_BITS`, so its full capacity is `1 << level`
        // (a leaf at level 0 holds `1 << VEC_BITS` = 32). Recurse while it still has room.
        if (last_size as u64) < (1u64 << level) {
            // Rightmost child has room. Rebase the index into it: subtract every preceding child's
            // size (its own base = the cumulative size up to `last-1`).
            let base = if last == 0 {
                0
            } else {
                vec_relaxed_size_at(node, last - 1)
            };
            let new_child = vec_push_into(vec_child(node, last), level - VEC_BITS, i - base, e);
            vec_relaxed_grow_last(node, last, new_child)
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_relaxed_append_branch(node, branch)
        }
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        if sub < vec_arity(node) {
            let new_child = vec_push_into(vec_child(node, sub), level - VEC_BITS, i, e);
            vec_node_replace(node, sub, new_child)
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_node_append(node, branch)
        }
    }
}

/// Replace the element at index `i` in the subtree rooted at `node` (borrowed) with `e` (consumed),
/// path-copying the one root→leaf path and sharing everything off it. Returns a new owned subtree.
fn vec_update_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
    if level == 0 {
        // Leaf: `i` is the (already rebased, for a relaxed descent) index; its low digit is the slot.
        vec_node_replace(node, (i & VEC_MASK) as usize, e)
    } else if vec_is_relaxed(node) {
        // Relaxed interior: locate the child via the size table and rebase the index; the update
        // leaves every child's element count unchanged, so the size table is carried through verbatim.
        let (sub, local_i) = vec_find_child_relaxed(node, i);
        let new_child = vec_update_into(vec_child(node, sub), level - VEC_BITS, local_i, e);
        vec_node_replace_keep_raw(node, sub, new_child)
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        let new_child = vec_update_into(vec_child(node, sub), level - VEC_BITS, i, e);
        vec_node_replace(node, sub, new_child)
    }
}

// ─── RRB relaxed-radix support (U1 foundation) ────────────────────────────────────────────
// A STRICT trie node (today's default) has `raw.is_empty()`: every child covers exactly `1 << level`
// elements, so a radix digit `(index >> level) & MASK` locates the child. A RELAXED node instead
// carries a per-child cumulative size table in `raw` (u32 LE, one entry per child), so
// `raw.len() == 4 * handles.len()` and `raw.len() > 0`. That table is `[size_0, size_0+size_1, …,
// total]`: entry `i` is the number of elements in children `0..=i`, and the last entry is the whole
// subtree total. Relaxed nodes are only produced by concat/split (U2/U3) at merge boundaries where a
// node's children are irregularly sized; U1 adds the representation and makes the read path honor it,
// but never CREATES one (so every existing all-strict test is unchanged).
//
// Discriminator safety — `raw.len() == 4 * handles.len() && raw.len() > 0` cannot misfire on:
//   • a vector HEADER: built only by `vec_alloc_header`, which always has `handles.len() ∈ {0,1}` and
//     `raw.len() == 8`; 8 == 4*1 would collide only at handles.len()==2, which a header never has.
//   • a STRICT trie node: `raw.is_empty()`, so `raw.len() > 0` is false.
//   • a LEAF: leaves are strict trie nodes (empty raw) here — same as above.
//   • a CHAMP map/set node or a bytes ROPE node: those helpers never call the vec helpers and vec ops
//     never touch them (no cross-op contamination), and their raw sizes (12 for CHAMP, 0/4/8 for rope)
//     don't match `4 * handles.len()` for the arities they carry.

/// True iff `node` is a RELAXED radix node — it carries a per-child cumulative size table in `raw`.
/// Strict nodes have empty raw and return false; a header/leaf/CHAMP/rope node returns false too (see
/// the discriminator-safety note above).
fn vec_is_relaxed(node: Handle) -> bool {
    with_node(node, false, |n| {
        let rlen = n.raw.len();
        let hlen = n.handles.len();
        rlen > 0 && rlen == 4 * hlen
    })
}

/// The `i`-th cumulative size (u32 LE at offset `4*i`) of a relaxed node's size table — the element
/// count of children `0..=i`. Short/absent raw yields 0 (benign, matching `read_u32_at`).
fn vec_relaxed_size_at(node: Handle, i: usize) -> u32 {
    with_node(node, 0, |n| read_u32_at(&n.raw, 4 * i))
}

/// The element count of child `i` alone in a relaxed node: `sizes[i] - sizes[i-1]` (with
/// `sizes[-1] := 0`). Used by U2/U3 rebalancing; kept here beside its reader.
#[cfg_attr(not(test), allow(dead_code))]
fn vec_relaxed_child_size(node: Handle, i: usize) -> u32 {
    let s = vec_relaxed_size_at(node, i);
    if i == 0 {
        s
    } else {
        s - vec_relaxed_size_at(node, i - 1)
    }
}

/// Locate the child of a RELAXED `node` that contains dense index `idx`, returning `(sub, local_idx)`
/// where `sub` is the child slot and `local_idx = idx - sizes[sub-1]` is the index rebased into that
/// child's subtree. Binary-searches the cumulative size table for the least `sub` with `idx <
/// sizes[sub]`. `idx` must be in-bounds (`idx < total`) — the caller's `index < count` guard ensures it.
fn vec_find_child_relaxed(node: Handle, idx: u32) -> (usize, u32) {
    let arity = vec_arity(node);
    let mut lo = 0usize;
    let mut hi = arity;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if idx < vec_relaxed_size_at(node, mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let base = if lo == 0 {
        0
    } else {
        vec_relaxed_size_at(node, lo - 1)
    };
    (lo, idx - base)
}

/// `vec-empty` — a new owned empty vector (rc 1). No root node until the first push.
// The shared IMMORTAL empty-vec singleton (lazily minted on first use), the `IMM_UNIT` analog for lists.
// `Handle::NULL` marks "not yet minted" (a real empty-vec is a heap node, never null), so the first
// `op_vec_empty` allocates + immortalizes it and every later call returns the SAME node.
runtime_local! {
    static EMPTY_VEC: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

fn op_vec_empty() -> Handle {
    // Return the shared IMMORTAL empty-vec. An empty list is CONSTANT (all empties are structurally equal,
    // no elements), so ONE immortal instance is semantically correct: it is EXCLUDED from the live-objects
    // census (rc = IMMORTAL; `op_mark_immortal` decrements the counter), so it NEVER shows as a leak — this
    // is the fix for the mixed-recursive List-fold terminal leak (the base `(list)` and every runtime-
    // generated empty, e.g. value-encode's defensive totals, now share one node instead of minting a fresh
    // MORTAL empty per call). It is also NEVER FBIP-mutated: every vec mutator gates on `node_rc == 1`, and
    // IMMORTAL != 1, so `vec-push`/`vec-set` on it take the persistent COPY path (verified) — read-only,
    // no shared-singleton corruption (the same discipline that protects `IMM_UNIT` and the immortal sums).
    EMPTY_VEC.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = vec_alloc_header(0, 0, Handle::NULL);
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// `vec-len` — the element count. Borrows; returns a `u32` by value.
fn op_vec_len(v: Handle) -> u32 {
    vec_read_header(v).0
}

/// `vec-get` — the element at `index` (BORROWED; the vector keeps ownership). An out-of-bounds index
/// TRAPS (fail-fast, like `arr-get`); the compiler emits the sign-aware bounds check on its side, so
/// reaching the trap is a compiler-invariant violation. After the `index < count` guard the trie
/// descent is in-bounds by construction.
fn op_vec_get(v: Handle, index: u32) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index >= count {
        trap_oob();
    }
    let mut node = root;
    let mut idx = index;
    let mut s = shift;
    while s > 0 {
        if vec_is_relaxed(node) {
            // Relaxed interior: scan the cumulative size table for the child holding `idx`, then
            // rebase `idx` into that child's subtree (subtract the preceding cumulative size).
            let (sub, local_idx) = vec_find_child_relaxed(node, idx);
            node = vec_child(node, sub);
            idx = local_idx;
        } else {
            // Strict interior: the radix digit is the child slot; `idx` stays absolute (the leaf-level
            // `idx & VEC_MASK` extracts the low digit) because a strict subtree is aligned to its level.
            node = vec_child(node, ((idx >> s) & VEC_MASK) as usize);
        }
        s -= VEC_BITS;
    }
    // Leaf level. After descending through any relaxed node, `idx` has been rebased so that its low
    // `VEC_BITS` are the slot within this leaf; for an all-strict descent `idx == index`, unchanged.
    vec_child(node, (idx & VEC_MASK) as usize)
}

// ─── FBIP (Functional But In-Place) rc==1 spine reuse for vec-push / vec-update (U4) ──────────
// When the touched spine is UNIQUELY owned we refit nodes in place instead of path-copying — zero
// heap traffic for the single-threaded push/update chains that dominate. Observationally IDENTICAL to
// the path-copy version; the win is fewer allocations.
//
// CRITICAL: ALIASING SAFETY (a violation silently corrupts a shared persistent version). `mine` means "this
// node is on a fully-unique path and is safe to mutate in place". It propagates STRICTLY DOWNWARD:
//   mine(root)  = header.rc == 1 && root.rc == 1
//   mine(child) = mine(parent)  && child.rc == 1
// Two facts make the per-node check mandatory: (1) `op_dup` on a header bumps ONLY the header's rc, so
// a header with rc>1 can still reach an rc==1 root — hence the header gate. (2) A child with rc==1 is
// still SHARED if any ancestor is shared (the other version reaches it THROUGH that ancestor) — hence
// once `mine` is false it stays false and we fall back to the exact existing path-copy for the whole
// remaining subtree. `mine=false` ⇒ delegate to `vec_push_into`/`vec_update_into` verbatim.

/// In-place set child slot `sub` of a node to `child` (no dup, no refcount bookkeeping — the caller
/// owns the transferred `child` and separately releases whatever was there). SAFETY contract: the
/// caller has verified this node is uniquely owned (rc == 1) before mutating it.
fn vec_set_child_inplace(node: Handle, sub: usize, child: Handle) {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0): patch bit `sub` in place. The old element (a bool imm) is
        // released by the caller — a drop no-op.
        packed_leaf_set_inplace(node, sub, child);
        return;
    }
    if let Some(n) = unsafe { node.node_mut() } {
        if let Some(slot) = n.handles.get_mut(sub) {
            *slot = child;
        }
    }
}

/// In-place append `child` to an rc==1 node's handles (no dup). SAFETY: caller verified rc == 1.
fn vec_push_child_inplace(node: Handle, child: Handle) {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0) gains one element: grow the packed bits in place. (The header
        // and interior nodes carry ≥1 handle or an 8-byte raw, so they never match here.)
        packed_leaf_push_inplace(node, child);
        return;
    }
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(child);
    }
}

/// In-place set an rc==1 vector HEADER's `[count][shift]` raw. SAFETY: caller verified rc == 1.
///
/// A header's raw is ALWAYS exactly 8 bytes (`vec_alloc_header` builds it inline), so PATCH the two
/// u32s in place via `as_mut_slice` rather than `clear()` + two capacity-checked `extend_from_slice`
/// calls. This runs on EVERY `op_vec_push`/`op_vec_update` on a unique vector (the hot FBIP path) and
/// showed up as `vec_set_header_inplace` + `Raw::extend_from_slice` in the vec profile. The defensive
/// branch (a header somehow not 8 bytes — never in correct operation) rebuilds via clear+extend.
fn vec_set_header_inplace(v: Handle, count: u32, shift: u32) {
    if let Some(n) = unsafe { v.node_mut() } {
        if n.raw.len() == 8 {
            let r = n.raw.as_mut_slice();
            r[0..4].copy_from_slice(&count.to_le_bytes());
            r[4..8].copy_from_slice(&shift.to_le_bytes());
        } else {
            n.raw.clear();
            n.raw.extend_from_slice(&count.to_le_bytes());
            n.raw.extend_from_slice(&shift.to_le_bytes());
        }
    }
}

/// In-place add 1 to the FINAL u32 entry of an rc==1 relaxed node's size table (a push into its last
/// child grew that child by one element). SAFETY: caller verified rc == 1 and the node is relaxed.
fn vec_bump_last_size_inplace(node: Handle) {
    if let Some(n) = unsafe { node.node_mut() } {
        let off = n.raw.len() - 4;
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
}

/// In-place append a new rightmost child `branch` (covering one new element) to an rc==1 relaxed node,
/// extending its size table with `old_total + 1`. SAFETY: caller verified rc == 1 and relaxed.
fn vec_relaxed_append_branch_inplace(node: Handle, branch: Handle) {
    let old_total = {
        let arity = vec_arity(node);
        if arity == 0 {
            0
        } else {
            vec_relaxed_size_at(node, arity - 1)
        }
    };
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(branch);
        n.raw.extend_from_slice(&(old_total + 1).to_le_bytes());
    }
}

/// FBIP variant of `vec_update_into`. When `mine`, mutate `node` in place and return the SAME handle;
/// otherwise delegate to the path-copying `vec_update_into` (returns a fresh node). Element counts are
/// unchanged by an update, so a relaxed node's size table is left untouched. Bounded-depth (≤7).
fn vec_update_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
    if !mine {
        return vec_update_into(node, level, i, e); // shared: exact existing path-copy
    }
    if level == 0 {
        // Leaf: swap the element slot in place, releasing the old element (matches path-copy's net RC).
        let slot = (i & VEC_MASK) as usize;
        let old = vec_child(node, slot);
        vec_set_child_inplace(node, slot, e);
        op_drop(old);
        return node;
    }
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, i)
    } else {
        (((i >> level) & VEC_MASK) as usize, i)
    };
    let child = vec_child(node, sub);
    let child_mine = node_rc(child) == 1;
    let new_child = vec_update_fbip(child, level - VEC_BITS, loc, e, child_mine);
    if !child_mine {
        // Child was shared → path-copied into `new_child`; swap it in, release the old shared child.
        vec_set_child_inplace(node, sub, new_child);
        op_drop(child);
    }
    // else: child mutated in place (new_child == child); node already points to it. Raw unchanged.
    node
}

/// FBIP variant of `vec_push_into`. When `mine`, mutate the rightmost spine in place and return the
/// SAME handle; otherwise delegate to the path-copying `vec_push_into`. Maintains relaxed size tables
/// in place (bump-last on descent, extend on a new branch). Bounded-depth (≤7).
fn vec_push_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
    if !mine {
        return vec_push_into(node, level, i, e); // shared: exact existing path-copy
    }
    if level == 0 {
        vec_push_child_inplace(node, e); // leaf gains the element
        return node;
    }
    if vec_is_relaxed(node) {
        let arity = vec_arity(node);
        let last = arity - 1;
        let last_size = vec_relaxed_child_size(node, last);
        if (last_size as u64) < (1u64 << level) {
            let base = if last == 0 {
                0
            } else {
                vec_relaxed_size_at(node, last - 1)
            };
            let child = vec_child(node, last);
            let child_mine = node_rc(child) == 1;
            let new_child = vec_push_fbip(child, level - VEC_BITS, i - base, e, child_mine);
            if !child_mine {
                vec_set_child_inplace(node, last, new_child);
                op_drop(child);
            }
            vec_bump_last_size_inplace(node); // the last child gained one element
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_relaxed_append_branch_inplace(node, branch);
        }
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        if sub < vec_arity(node) {
            let child = vec_child(node, sub);
            let child_mine = node_rc(child) == 1;
            let new_child = vec_push_fbip(child, level - VEC_BITS, i, e, child_mine);
            if !child_mine {
                vec_set_child_inplace(node, sub, new_child);
                op_drop(child);
            }
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_push_child_inplace(node, branch);
        }
    }
    node
}

/// `vec-push` — a new owned vector = `v` with `elem` appended at the end. CONSUMES `v` and `elem`.
/// FBIP fast path: when `v`'s header is uniquely owned it is REUSED as the result and the touched
/// rightmost spine is refit in place wherever each node is uniquely owned (rc==1); anywhere the spine
/// is shared it path-copies exactly as before (persistence preserved). When the header itself is shared
/// (another version holds `v`), the whole op is the original allocate-new-header path.
fn op_vec_push(v: Handle, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    let header_mine = node_rc(v) == 1;

    if !header_mine {
        // Header shared: original behavior — build a fresh version, leave `v` (and its version) intact.
        let (new_root, new_shift) = if count == 0 {
            (vec_leaf_of(elem), 0)
        } else if (count as u64) == (1u64 << (shift + VEC_BITS)) {
            op_dup(root);
            let branch = vec_new_path(shift, vec_leaf_of(elem));
            // Arity-2 new root — build handles INLINE (exactly INLINE_HANDLES_CAP, no transient Vec).
            (
                alloc_raw(Handles::inline_from(&[root, branch]), Raw::from(Vec::new())),
                shift + VEC_BITS,
            )
        } else {
            (vec_push_into(root, shift, count, elem), shift)
        };
        let hdr = vec_alloc_header(count + 1, new_shift, new_root);
        op_drop(v);
        return hdr;
    }

    // Header uniquely owned: reuse it as the result (no header alloc, no drop of `v`).
    if count == 0 {
        // First element: the empty header gains a one-element leaf; shift stays 0.
        vec_push_child_inplace(v, vec_leaf_of(elem));
        vec_set_header_inplace(v, 1, 0);
    } else if (count as u64) == (1u64 << (shift + VEC_BITS)) {
        // Root full → grow a level. Transfer the old root into the new root WITHOUT a dup: `v`'s single
        // owned reference to `root` relocates into `new_root` (whatever else references `root` is
        // untouched), so no dup/drop is needed regardless of `root`'s own rc.
        let branch = vec_new_path(shift, vec_leaf_of(elem));
        // Arity-2 new root — build handles INLINE (no transient `vec![root, branch]` heap Vec).
        let new_root = alloc_raw(Handles::inline_from(&[root, branch]), Raw::from(Vec::new()));
        vec_set_child_inplace(v, 0, new_root);
        vec_set_header_inplace(v, count + 1, shift + VEC_BITS);
    } else {
        let root_mine = node_rc(root) == 1;
        let new_root = vec_push_fbip(root, shift, count, elem, root_mine);
        if !root_mine {
            // Root was shared → path-copied; swap the copy into the reused header, release old root.
            vec_set_child_inplace(v, 0, new_root);
            op_drop(root);
        }
        vec_set_header_inplace(v, count + 1, shift);
    }
    v
}

/// PATH-COPY front-prepend of `elem` into the subtree `node` at `level`, returning the NEW owned subtree,
/// or `None` if `node` is FULL at the front (its front spine is full to the leaf AND `node` itself has 32
/// children — the caller then grows a level). BALANCED (log-depth): `elem` PACKS into the leftmost leaf
/// while it has room (< 32), and a fresh front subtree is added only when a level is genuinely full — the
/// front-growth mirror of `vec-push`'s tail-append, NOT a new single-element child per prepend (which built
/// a degenerate O(n)-deep tree, `List.prepend`'s original miscompile: value-correct but O(n) reads + a
/// `1<<level` overflow in `vec_subtree_size` past ~7 levels).
///
/// RECLAIM: `node` is BORROWED (not consumed) — the caller frees the old tree via `op_drop(v)`, whose
/// cascade reclaims exactly the OLD front spine this copied. Off-path siblings CARRY FORWARD, so they are
/// `dup`'d (rc bumped) into the new node → the cascade takes them rc2→1 (survive). The front child that the
/// recursion descends into is NOT dup'd (a fresh front replaces it; the cascade frees the old one). This is
/// the standard persistent path-copy, rc-correct for a uniquely-owned AND a shared `node`.
fn vec_prepend_into(node: Handle, level: u32, elem: Handle) -> Option<Handle> {
    let arity = vec_arity(node);
    let cap = VEC_MASK as usize + 1; // 32
    if level == 0 {
        // Leaf: pack `elem` at the front if there is room; a full leaf declines (caller adds a sibling).
        if arity >= cap {
            return None;
        }
        let mut kids = Vec::with_capacity(arity + 1);
        kids.push(elem);
        vec_collect_children_dup(node, &mut kids); // dup the old elements (they carry forward)
        return Some(vec_leaf_from_handles(kids));
    }
    let front = vec_child(node, 0);
    if let Some(new_front) = vec_prepend_into(front, level - VEC_BITS, elem) {
        // The front subtree absorbed `elem`. Path-copy: new child 0 = new_front (the old front is freed by
        // the caller's cascade — NOT dup'd here); children 1.. carry forward (dup'd).
        let mut kids = Vec::with_capacity(arity);
        kids.push(new_front);
        for i in 1..arity {
            let c = vec_child(node, i);
            op_dup(c);
            kids.push(c);
        }
        return Some(vec_relaxed_node(kids, level));
    }
    // Front subtree is FULL. If `node` has room, add a NEW front child — a fresh spine to a 1-element leaf
    // at this level's child depth (amortized rare, exactly `vec-push`'s full-tail case). ALL old children
    // (including the full old front child 0) carry forward → dup them.
    if arity >= cap {
        return None; // `node` full too — the caller grows a level above it
    }
    let mut kids = Vec::with_capacity(arity + 1);
    kids.push(vec_new_path(level - VEC_BITS, vec_leaf_of(elem)));
    vec_collect_children_dup(node, &mut kids); // dup all old children (they follow the new front child)
    Some(vec_relaxed_node(kids, level))
}

/// `vec-prepend` — a new owned vector = `elem` followed by the elements of `v`. CONSUMES `v` and `elem`
/// (a constructor, like `vec-push`); the FRONT-growth twin of `vec-push`'s tail-growth. `List.prepend`
/// lowers to this, REPLACING the old `concat(singleton, v)` path — which invoked the full RRB merge
/// (lifting the singleton to `v`'s level) per prepend and leaked the superseded front-spine. Front growth
/// PACKS into the leftmost leaf (`vec_prepend_into`), staying LOG-DEPTH — not a new single-element child
/// per prepend (which built a degenerate O(n)-deep tree). RECLAIM discipline: the path-copy `dup`s the
/// carried-forward (off-path) children and `op_drop(v)` frees the old front spine — rc-correct for BOTH a
/// uniquely-owned `v` (shell frees, shared children go rc2→1) AND a shared `v` (rc > 1 → decremented,
/// children stay shared, RRB persistence intact). SOUND on the immortal empty-vec base (count 0 → a fresh
/// one-element vector; `op_drop` of the shared immortal is a no-op).
fn op_vec_prepend(v: Handle, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    let cap = VEC_MASK as usize + 1; // 32 — max children per node
    if count == 0 {
        // Onto an empty vector (incl. the shared immortal empty-vec): a fresh one-element vector.
        let hdr = vec_alloc_header(1, 0, vec_leaf_of(elem));
        op_drop(v); // frees a mortal empty header; a no-op on the immortal empty singleton
        return hdr;
    }
    if shift == 0 {
        // Leaf root: the new leaf is `elem` then the old ≤32 elements (dup'd so they survive op_drop(v)).
        let mut kids = Vec::with_capacity(count as usize + 1);
        kids.push(elem);
        vec_collect_children_dup(root, &mut kids);
        let (new_root, new_shift) = if kids.len() <= cap {
            (vec_leaf_from_handles(kids), 0)
        } else {
            // 33 elements → two leaves under a 2-child relaxed parent (one level up).
            let k = kids.len().div_ceil(2);
            let right = kids.split_off(k);
            (
                vec_relaxed_node(
                    vec![vec_leaf_from_handles(kids), vec_leaf_from_handles(right)],
                    VEC_BITS,
                ),
                VEC_BITS,
            )
        };
        let hdr = vec_alloc_header(count + 1, new_shift, new_root);
        op_drop(v);
        return hdr;
    }
    // Interior root: PACK `elem` into the leftmost leaf via the balanced path-copy. On overflow (the whole
    // front spine full AND the root has 32 children) grow ONE level: a fresh single-element front subtree
    // beside the (dup'd) old root — the front-growth mirror of `vec-push`'s root-overflow, and the ONLY
    // place a single-element spine is minted (amortized, never per-prepend).
    let (new_root, new_shift) = match vec_prepend_into(root, shift, elem) {
        Some(new_root) => (new_root, shift),
        None => {
            let new_front = vec_new_path(shift, vec_leaf_of(elem)); // 1-element front subtree at the old root's level
            op_dup(root); // the old root carries forward as the new root's second child
            (
                vec_relaxed_node(vec![new_front, root], shift + VEC_BITS),
                shift + VEC_BITS,
            )
        }
    };
    let hdr = vec_alloc_header(count + 1, new_shift, new_root);
    op_drop(v);
    hdr
}

/// `vec-update` — a new owned vector = `v` with `index` set to `elem`. CONSUMES `v` and `elem`. OOB
/// index traps (like `vec-get`). FBIP fast path: when `v`'s header is uniquely owned it is reused and
/// the affected root→leaf path is refit in place wherever each node is uniquely owned; a shared node
/// path-copies exactly as before. A shared header takes the original allocate-new-header path.
fn op_vec_update(v: Handle, index: u32, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index >= count {
        trap_oob();
    }
    if node_rc(v) != 1 {
        // Header shared: original path-copy; `v` (the other version) stays byte-identical.
        let new_root = vec_update_into(root, shift, index, elem);
        let hdr = vec_alloc_header(count, shift, new_root);
        op_drop(v);
        return hdr;
    }
    // Header uniquely owned: reuse it. count/shift are unchanged by an update, so its raw is unchanged.
    let root_mine = node_rc(root) == 1;
    let new_root = vec_update_fbip(root, shift, index, elem, root_mine);
    if !root_mine {
        // Root was shared → path-copied; swap the copy into the reused header, release the old root.
        vec_set_child_inplace(v, 0, new_root);
        op_drop(root);
    }
    v
}

/// The element count of the subtree rooted at `node`, whose top level is `level`. A leaf (`level == 0`)
/// contributes its arity; a RELAXED node reads the last cumulative-size entry (O(1)); a STRICT interior
/// node has all-but-its-last child full (`1 << level` each) and recurses only into the last (partial)
/// child — so the recursion is bounded by the trie height (≤7), never fanning out. Used to build a
/// merged node's cumulative size table during concat.
fn vec_subtree_size(node: Handle, level: u32) -> u32 {
    if level == 0 {
        return vec_arity(node) as u32;
    }
    let arity = vec_arity(node);
    if arity == 0 {
        return 0;
    }
    if vec_is_relaxed(node) {
        return vec_relaxed_size_at(node, arity - 1);
    }
    // Strict interior: the first `arity-1` children are each full at this level; the last may be partial.
    let full = (arity as u32 - 1) * (1u32 << level);
    full + vec_subtree_size(vec_child(node, arity - 1), level - VEC_BITS)
}

/// Lift an OWNED subtree `node` (currently at `shift`) to `target` shift by wrapping it in STRICT
/// single-child interior nodes (one per level). Consumes `node` — its ownership transfers into the
/// innermost wrapper. The wrapper chain is a valid strict, left-aligned subtree (exactly what
/// `vec_new_path` builds), so radix descent through it always selects child 0 and reaches `node`.
/// `target >= shift`; the loop is bounded by their difference (≤ the 7-level trie depth per operand).
fn vec_grow_to_shift(mut node: Handle, mut shift: u32, target: u32) -> Handle {
    while shift < target {
        // Arity-1 strict single-child wrapper — inline the single handle (no transient `vec![node]`).
        node = alloc_raw(Handles::inline_from(&[node]), Raw::from(Vec::new()));
        shift += VEC_BITS;
    }
    node
}

/// Push each child of an OWNED node into `out`, `dup`ing it so it survives the node's later `op_drop`
/// (same discipline as `vec_node_append`). The node keeps its own references; `out` gains owned ones.
fn vec_collect_children_dup(node: Handle, out: &mut Vec<Handle>) {
    let arity = vec_arity(node);
    for j in 0..arity {
        let c = vec_child(node, j);
        op_dup(c);
        out.push(c);
    }
}

/// Build a RELAXED interior node at `level` (its children are subtrees at `level - VEC_BITS`) owning
/// `children` (consumed). Computes the cumulative size table `[Σsize(0..=i)]` from each child's actual
/// element count via `vec_subtree_size`, so the table is strictly increasing with last == subtree
/// total (the U1 invariants). Precondition: `children` non-empty, none zero-size (the callers only pass
/// real subtrees of non-empty operands).
fn vec_relaxed_node(children: Vec<Handle>, level: u32) -> Handle {
    let child_level = level - VEC_BITS;
    let mut raw = Vec::with_capacity(4 * children.len());
    let mut running = 0u32;
    for &c in &children {
        running += vec_subtree_size(c, child_level);
        raw.extend_from_slice(&running.to_le_bytes());
    }
    alloc(children, raw)
}

/// `vec-concat` — a new owned vector = the elements of `a` followed by the elements of `b`. CONSUMES
/// both `a` and `b` (a constructor, like `vec-push`); a caller keeping either version `dup`s it first.
///
/// Ownership: an empty operand is the identity — `concat(empty, b)` returns `b` unchanged (its rc
/// already covers the caller) and reclaims the consumed empty header; likewise `concat(a, empty)`.
///
/// Algorithm (CONSERVATIVE-CORRECT, not FULL RRB leaf-rebalance — see the U2 handoff): lift both roots
/// to a common level, then CONCATENATE their child lists at that level. If the combined child list fits
/// one node (≤ 32) the result is a single node at that level (NO height increase); otherwise the list
/// is split into two balanced groups → two nodes → a 2-child parent one level up. A leaf-level merge of
/// two small vectors yields one STRICT leaf (indistinguishable from a push-built one); every other
/// merged node is RELAXED with a correct cumulative size table (built from `vec_subtree_size`). This is
/// "conservative" because it does NOT rebalance the boundary LEAVES (two partial leaves can stay
/// adjacent — the result is not maximally dense), but height stays minimal-ish: it only grows a level
/// when a node would exceed 32 children, so repeated fold-concat does NOT build a degenerate deep tree,
/// and `vec-get`/`vec-len`/`vec-push`/`vec-update` all traverse the result correctly. Bounded-depth
/// only: `vec_grow_to_shift`/`vec_subtree_size` are bounded by the ≤7-level trie height; no fan-out
/// recursion.
fn op_vec_concat(a: Handle, b: Handle) -> Handle {
    let (count_a, shift_a, root_a) = vec_read_header(a);
    let (count_b, shift_b, root_b) = vec_read_header(b);
    if count_a == 0 {
        op_drop(a); // reclaim the empty header; `b` passes through unchanged
        return b;
    }
    if count_b == 0 {
        op_drop(b);
        return a;
    }
    let total = count_a + count_b;
    let cap = VEC_MASK as usize + 1; // 32 — max children per node

    // Lift both roots to a common level and gather their children (dup'd) into one list at that level.
    let max_shift = shift_a.max(shift_b);
    op_dup(root_a);
    op_dup(root_b);
    let grown_a = vec_grow_to_shift(root_a, shift_a, max_shift);
    let grown_b = vec_grow_to_shift(root_b, shift_b, max_shift);
    // Pre-size to the known maximum (each root has ≤`cap`=32 children after growth, so ≤64 total) — one
    // allocation, skipping the 1→2→…→64 realloc chain a growing `Vec` would do while gathering.
    let mut children: Vec<Handle> = Vec::with_capacity(2 * cap);
    vec_collect_children_dup(grown_a, &mut children);
    vec_collect_children_dup(grown_b, &mut children);
    op_drop(grown_a); // releases the grown wrappers + our root dups; the dup'd children survive in `children`
    op_drop(grown_b);

    // Build the merged root from `children` (subtrees at level `max_shift`, i.e. children of a node at
    // `max_shift + VEC_BITS`; when `max_shift == 0` they are ELEMENTS of a leaf).
    // `children` are the child subtrees of nodes at level `max_shift`, i.e. they live at level
    // `max_shift` themselves when `max_shift == 0` (elements) — more precisely, a node built from them
    // has its OWN level equal to `max_shift` and its children at `max_shift - VEC_BITS`.
    let m = children.len(); // ≤ 64 (each root has ≤32 children after growth)
    let (new_root, new_shift) = if m <= cap {
        if max_shift == 0 {
            // Leaf merge: elements fit one leaf. PACK it when all-bool (a `List Bool` concat stays
            // dense), else a STRICT leaf (uniform size-1, no table needed).
            (vec_leaf_from_handles(children), 0)
        } else {
            // One relaxed node (level `max_shift`) holds every gathered child; no height increase.
            (vec_relaxed_node(children, max_shift), max_shift)
        }
    } else {
        // Overflow: split into two balanced groups, wrap each at level `max_shift`, then a 2-child
        // relaxed parent at level `max_shift + VEC_BITS` (one level up).
        let k = m.div_ceil(2); // each group ≤ 32 since m ≤ 64
        let right = children.split_off(k);
        let left = children;
        let (g_left, g_right) = if max_shift == 0 {
            // Two leaves (level 0): packed when all-bool, else strict.
            (vec_leaf_from_handles(left), vec_leaf_from_handles(right))
        } else {
            (
                vec_relaxed_node(left, max_shift),
                vec_relaxed_node(right, max_shift),
            )
        };
        (
            vec_relaxed_node(vec![g_left, g_right], max_shift + VEC_BITS),
            max_shift + VEC_BITS,
        )
    };

    let hdr = vec_alloc_header(total, new_shift, new_root);
    op_drop(a);
    op_drop(b);
    hdr
}

/// `vec-of-arr` — build a persistent vector from an already-built flat `arr` (the tuple/record array
/// primitive) in ONE call, the lowering target for a `(list …)` literal. CONSUMES `arr`. The `arr` node
/// carries the elements exactly as a vector LEAF does (`handles` = elements, empty `raw`), so:
///   - 0 elements (arr is inline unit)   → the empty vector.
///   - ≤32 elements (fits one leaf)      → REUSE the `arr` node itself as the leaf-root by move (zero
///     extra allocation beyond the 8-byte header) — the common small-list-literal case.
///   - >32 elements                      → drain the elements into ≤32-element strict leaves and build a
///     strict left-full radix trie bottom-up in one pass (no per-element persistent rebuild).
/// The result is byte-identical to (and interchangeable with) a push-built vector of the same elements.
fn op_vec_of_arr(arr: Handle) -> Handle {
    let count = op_arr_len(arr);
    if count == 0 {
        op_drop(arr); // consume the (inline unit) arr; the empty vector is independent
        return op_vec_empty();
    }
    let cap = VEC_MASK as usize + 1; // 32 elements per leaf
    if (count as usize) <= cap {
        // A `List Bool` literal (≤32 all-bool elements) packs into ONE dense leaf instead of reusing the
        // arr node — ~6× denser. `arr`'s elements are bool immediates (nothing to drop individually);
        // release the now-superseded arr shell.
        if let Some(bits) = arr_all_bool_bits(arr) {
            let leaf = packed_leaf_new(count as u8, bits);
            op_drop(arr);
            return vec_alloc_header(count, 0, leaf);
        }
        // The arr node IS a valid strict single leaf (handles = elements, empty raw). Move it in as the
        // root — no copy, no per-element push. `shift == 0` for a leaf-only tree.
        return vec_alloc_header(count, 0, arr);
    }
    // >32: take the arr's element handles out (a move — no dup, they relocate into the leaves) and pack
    // them into ≤32-element leaves — packed when all-bool (a large `List Bool` literal), else strict.
    let mut elems = champ_take_handles(arr).into_vec();
    op_drop(arr); // the now-empty arr shell
    let mut leaves: Vec<Handle> = Vec::with_capacity(elems.len().div_ceil(cap));
    let mut rest = &mut elems[..];
    while !rest.is_empty() {
        let take = rest.len().min(cap);
        let (chunk, tail) = rest.split_at_mut(take);
        leaves.push(vec_leaf_from_handles(chunk.to_vec()));
        rest = tail;
    }
    // Build a strict, left-full radix trie bottom-up: each interior level groups ≤32 children until one
    // root remains. `shift` rises by VEC_BITS per level. (A vec built this way is dense/left-full, so it
    // stays STRICT — no relaxed size tables needed, unlike concat/split boundary nodes.)
    let mut level_nodes = leaves;
    let mut shift = 0u32;
    while level_nodes.len() > 1 {
        shift += VEC_BITS;
        let mut parents: Vec<Handle> = Vec::with_capacity(level_nodes.len().div_ceil(cap));
        let mut i = 0;
        while i < level_nodes.len() {
            let end = (i + cap).min(level_nodes.len());
            parents.push(alloc(level_nodes[i..end].to_vec(), Vec::new())); // strict interior node
            i = end;
        }
        level_nodes = parents;
    }
    vec_alloc_header(count, shift, level_nodes.pop().unwrap())
}

/// Split the subtree rooted at `node` (BORROWED, top level `level`) at LOCAL element index `idx`
/// (`0 < idx < subtree_size` at the top; deeper calls may hit the `idx==0`/`idx==size` boundaries).
/// Returns `(left, right)` as freshly OWNED subtrees at the SAME level as `node` — `Handle::NULL` for a
/// side that ends up empty. Carried-over whole children are `dup`ed (they survive the caller's later
/// `op_drop(v)`); the boundary child is split recursively. EVERY rebuilt boundary node is RELAXED with a
/// correct cumulative size table (a trim can leave an interior node non-dense — U2's gotcha — so it must
/// NOT stay strict); LEAVES stay strict (their elements are uniformly size-1, valid after any trim).
/// Bounded-depth: `level` drops by `VEC_BITS` each call, so recursion is ≤ the 7-level trie height.
fn vec_split_subtree(node: Handle, level: u32, idx: u32) -> (Handle, Handle) {
    if level == 0 {
        // Leaf: partition its elements. Each retained element is `dup`ed (the leaf still owns its own).
        let arity = vec_arity(node) as u32;
        let left = if idx == 0 {
            Handle::NULL
        } else {
            let mut hs = Vec::with_capacity(idx as usize);
            for j in 0..idx {
                let e = vec_child(node, j as usize);
                op_dup(e);
                hs.push(e);
            }
            vec_leaf_from_handles(hs) // packed when all-bool, else strict
        };
        let right = if idx >= arity {
            Handle::NULL
        } else {
            let mut hs = Vec::with_capacity((arity - idx) as usize);
            for j in idx..arity {
                let e = vec_child(node, j as usize);
                op_dup(e);
                hs.push(e);
            }
            vec_leaf_from_handles(hs) // packed when all-bool, else strict
        };
        return (left, right);
    }
    // Interior: find the child holding `idx` and the index within it.
    let arity = vec_arity(node);
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, idx)
    } else {
        // Strict, left-aligned, dense: child `sub` starts at `sub << level`.
        let sub = ((idx >> level) & VEC_MASK) as usize;
        (sub, idx & ((1u32 << level) - 1))
    };
    let (cl, cr) = vec_split_subtree(vec_child(node, sub), level - VEC_BITS, loc);
    // LEFT: whole children [0, sub) (dup'ed) then the split child's left part (if any).
    let mut left_children: Vec<Handle> = Vec::with_capacity(sub + 1);
    for j in 0..sub {
        let c = vec_child(node, j);
        op_dup(c);
        left_children.push(c);
    }
    if cl != Handle::NULL {
        left_children.push(cl);
    }
    // RIGHT: the split child's right part (if any) then whole children (sub, arity) (dup'ed).
    let mut right_children: Vec<Handle> = Vec::with_capacity(arity - sub);
    if cr != Handle::NULL {
        right_children.push(cr);
    }
    for j in (sub + 1)..arity {
        let c = vec_child(node, j);
        op_dup(c);
        right_children.push(c);
    }
    let left = if left_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(left_children, level) // rebuilt boundary node → relaxed with correct table
    };
    let right = if right_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(right_children, level)
    };
    (left, right)
}

/// The RIGHT-ONLY twin of `vec_split_subtree`: build ONLY the tail `[idx, …)` subtree, never the
/// discarded left prefix. `vec-drop` (the `(list p… .. rest)` REST binder, the HOT per-element fold step)
/// throws the left half away, so `vec_split_subtree`'s left leaf + `left_children` Vec + left relaxed node
/// + the dup-then-drop of every left element are PURE WASTE — roughly half the split's allocation. This
/// keeps only the kept side: whole children AFTER the boundary (dup'ed — they survive the caller's
/// `op_drop(v)`) plus the boundary child's own tail (recursively). The dropped children ([0,sub) and the
/// boundary child's head) are NOT dup'ed, so the caller's `op_drop(v)` reclaims them — identical ownership
/// to the split path, minus the transient left half. Returns the right subtree at `node`'s level (NULL if
/// the tail is empty). Same relaxed-rebuild + bounded-depth discipline as `vec_split_subtree`.
fn vec_take_tail(node: Handle, level: u32, idx: u32) -> Handle {
    if level == 0 {
        let arity = vec_arity(node) as u32;
        if idx >= arity {
            return Handle::NULL;
        }
        let mut hs = Vec::with_capacity((arity - idx) as usize);
        for j in idx..arity {
            let e = vec_child(node, j as usize);
            op_dup(e);
            hs.push(e);
        }
        return vec_leaf_from_handles(hs); // kept tail elements — packed when all-bool, else strict
    }
    let arity = vec_arity(node);
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, idx)
    } else {
        let sub = ((idx >> level) & VEC_MASK) as usize;
        (sub, idx & ((1u32 << level) - 1))
    };
    // Only the boundary child is split (its tail kept); its head + all children [0, sub) are left for the
    // caller's `op_drop(v)` to reclaim (never dup'ed here).
    let cr = vec_take_tail(vec_child(node, sub), level - VEC_BITS, loc);
    let mut right_children: Vec<Handle> = Vec::with_capacity(arity - sub);
    if cr != Handle::NULL {
        right_children.push(cr);
    }
    for j in (sub + 1)..arity {
        let c = vec_child(node, j);
        op_dup(c);
        right_children.push(c);
    }
    if right_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(right_children, level)
    }
}

/// `vec-drop(v, index)` — the tail `[index, len)`, CONSUMING `v`. The `(list p… .. rest)` REST-binder op
/// and the per-element step of a list fold, so it is HOT. Builds ONLY the kept tail spine (`vec_take_tail`)
/// — NOT the discarded left prefix a `split`+drop-left would materialize then free — then reclaims the
/// original tree (`op_drop(v)` frees every node the tail did not `dup`). Boundaries mirror `op_vec_split`:
/// `index == 0` returns `v` unchanged (whole tail); `index >= len` returns the canonical empty.
fn op_vec_drop_tail(v: Handle, index: u32) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index == 0 {
        return v; // whole vector is the tail — flow through unchanged (its rc already covers the caller)
    }
    if index >= count {
        op_drop(v);
        return op_vec_empty();
    }
    let rroot = vec_take_tail(root, shift, index);
    // rroot is non-NULL (0 < index < count) at level `shift` — split never raises height.
    let tail = vec_alloc_header(count - index, shift, rroot);
    op_drop(v); // reclaim the original tree; kept subtrees survive via the dups in vec_take_tail
    tail
}

/// `vec-split` — split `v` at element `index` into `(left, right)` where `left` is elements
/// `[0, index)` and `right` is elements `[index, len)`. CONSUMES `v`; returns TWO new owned vectors.
/// Boundaries: `index == 0` → `(empty, v)` (v flows through as the right output, unchanged); `index >=
/// len` → `(v, empty)`. Both outputs honor every relaxed-node invariant and are valid for downstream
/// get/len/push/update/concat. Bounded-depth (via `vec_split_subtree`). `index > len` is clamped to
/// `len` (total split — a benign no-op split, mirroring how the boundary is the identity).
fn op_vec_split(v: Handle, index: u32) -> (Handle, Handle) {
    let (count, shift, root) = vec_read_header(v);
    if index == 0 {
        // Left empty; `v` becomes the right output unchanged (no drop — its rc already covers the caller).
        return (op_vec_empty(), v);
    }
    if index >= count {
        // Right empty; `v` becomes the left output unchanged.
        return (v, op_vec_empty());
    }
    let (lroot, rroot) = vec_split_subtree(root, shift, index);
    // Both roots are non-NULL here (0 < index < count) and live at level `shift` — the header shift is
    // unchanged (split never increases height; a half may be sparse but descent stays correct).
    let left = vec_alloc_header(index, shift, lroot);
    let right = vec_alloc_header(count - index, shift, rroot);
    op_drop(v); // consume the input; carried subtrees survive via the dups, the boundary spine frees
    (left, right)
}

// ─── The WIT `Guest` impl: the ONLY place `u32` handles exist ───────────────────────────
// Present only in the wasm component build. Handles narrow to `u32` losslessly (wasm32 pointers are
// 32-bit); genuine runtime data (`arr-len` count, `sum-disc`, byte values) is already `u32` and
// passes straight through — the conversion is applied to handles alone.

#[cfg(target_arch = "wasm32")]
impl Handle {
    /// Narrow to the opaque public handle. Lossless on wasm32 (a `Node` address is 32-bit).
    ///
    /// The u32 IS a raw `Node` address into THIS runtime instance's heap, so it is meaningful only within
    /// the single run/instance that produced it — it never escapes as durable state the ABI transports;
    /// a host resuming a run reconstructs values through the runtime, not by carrying a handle across.
    //= spec/contracts/component-abi.md#a-runtime-value-crosses-as-an-opaque-handle
    //# A runtime handle MUST be meaningful only within the single run and runtime instance that produced it, so that a handle never escapes the run that produced it and a host that resumes a run by replaying it reconstructs the run's values through the runtime rather than by carrying a handle across the boundary (the handle is not durable state the ABI transports; whether and how a host replays is host policy — capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).
    fn to_u32(self) -> u32 {
        self.0 as usize as u32
    }
    /// Widen a public handle back to a node pointer. Inverse of `to_u32` on wasm32.
    fn from_u32(x: u32) -> Handle {
        Handle(x as usize as *mut Node)
    }
}

/// `hash-blake3(bytes)` (heap index 91) — the BLAKE3 digest of `input`'s Bytes-leaf contents, as a fresh
/// 32-byte Bytes leaf. A GENERIC content hash (`bytes -> digest`): no tag, no prefix, no notion of a
/// "contract" — userspace prepends any domain separation before calling (DESIGN-compiler-primitives.md D7).
/// This is the RUNTIME half of the compiler's `Blake3.of`; the compile-time fold calls the SAME `blake3`
/// crate over the same bytes, so both produce byte-identical digests (that design's §9 load-bearing
/// invariant). BORROWS `input` (reads it, never drops it — an inspector, like `op_value_encode_form`) and
/// returns a fresh owned leaf. Reads `input` LOGICALLY via the index accessors so a rope Bytes value
/// flattens correctly, exactly as `op_value_decode` reads its document. TOTAL: an empty input hashes to
/// blake3's defined empty-input digest; never traps.
fn op_hash_blake3(input: Handle) -> Handle {
    let n = op_bytes_len(input);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(input, i) as u8);
    }
    let digest = blake3::hash(&buf);
    alloc(Vec::new(), digest.as_bytes().to_vec())
}

/// The 7 `Ast` variant discs the compiler conveys to `ast-print`/`ast-read`. The compiler looks these up
/// BY NAME from the (prelude-defined) `Ast` sum decl, so the runtime NEVER hardcodes them — they ride in
/// the `discs` Bytes, LEB-encoded in this fixed slot order: [int, float, bool, str, name, bytes, list].
struct AstDiscs {
    int: u32,
    float: u32,
    boolv: u32,
    strv: u32,
    name: u32,
    bytes: u32,
    list: u32,
}

/// Decode the baked disc descriptor: 7 LEB128 varints in `[int,float,bool,str,name,bytes,list]` order.
/// `None` on a truncated/malformed descriptor (the compiler always bakes a well-formed one, so not-reached).
fn read_ast_discs(discs: Handle) -> Option<AstDiscs> {
    let n = op_bytes_len(discs);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(discs, i) as u8);
    }
    let mut pos = 0usize;
    let mut next = || -> Option<u32> {
        let mut val: u32 = 0;
        let mut shift = 0u32;
        loop {
            let b = *buf.get(pos)?;
            pos += 1;
            val |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Some(val)
    };
    Some(AstDiscs {
        int: next()?,
        float: next()?,
        boolv: next()?,
        strv: next()?,
        name: next()?,
        bytes: next()?,
        list: next()?,
    })
}

/// Escape a string's contents for a `"…"` Ast.Str literal — the closed set `\n \t \r \\ \"` — mirroring the
/// compiler's `push_escaped_str` (rcdzc lower.rs) so `Ast.print` is byte-identical to the compile-time fold.
fn push_escaped_ast_str(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
}

/// Render a runtime `Ast` heap value to canonical re-readable s-expression text — BYTE-IDENTICAL to the
/// compiler's `print_ast_value` (rcdzc lower.rs): Int→BigInt decimal, Float→Rust shortest round-trip
/// decimal (forced `.0`), Bool→true/false, Str→escaped `"…"`, Name→bare, Bytes→`b"…"` (printable / named /
/// `\xNN` lower-hex), List→`(e e …)` space-separated recursive. An `Ast` variant carries exactly one
/// payload (a real heap sum node → `op_sum_disc` reads its stored disc). An unknown disc renders nothing.
fn render_ast(h: Handle, d: &AstDiscs, out: &mut String) {
    let disc = op_sum_disc(h);
    let payload = op_sum_payload(h);
    if disc == d.int {
        out.push_str(&unbox_bigint(payload).to_decimal_string());
    } else if disc == d.float {
        // Match `float_text` (rcdzc): Rust's `{}` shortest round-trip, forced to carry `.`/`e` so it
        // re-lexes as a float (a bare `3` would re-read as Ast.Int). f64's Display is core (no_std-ok).
        let s = alloc::format!("{}", op_get_float(payload));
        out.push_str(&s);
        if !(s.contains('.') || s.contains('e') || s.contains('E')) {
            out.push_str(".0");
        }
    } else if disc == d.boolv {
        out.push_str(if op_get_bool(payload) {
            "true"
        } else {
            "false"
        });
    } else if disc == d.strv {
        out.push('"');
        push_escaped_ast_str(out, &op_str_get(payload));
        out.push('"');
    } else if disc == d.name {
        out.push_str(&op_str_get(payload));
    } else if disc == d.bytes {
        out.push_str("b\"");
        let n = op_bytes_len(payload);
        for i in 0..n {
            let b = op_bytes_get(payload, i) as u8;
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                b'\\' => out.push_str("\\\\"),
                b'"' => out.push_str("\\\""),
                0x20..=0x7e => out.push(b as char),
                _ => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    out.push('\\');
                    out.push('x');
                    out.push(HEX[(b >> 4) as usize] as char);
                    out.push(HEX[(b & 0xf) as usize] as char);
                }
            }
        }
        out.push('"');
    } else if disc == d.list {
        // `Ast.List`'s payload is a Cadenza `(list …)` — a persistent RRB VECTOR, read by `vec-len`/
        // `vec-get` (NOT the `arr-*` tuple/record accessors: an RRB root node's `handles` arity is its
        // branch/leaf count, not the element count, so `arr-len` misreads a multi-element list as 1).
        out.push('(');
        let n = op_vec_len(payload);
        for i in 0..n {
            if i > 0 {
                out.push(' ');
            }
            render_ast(op_vec_get(payload, i), d, out);
        }
        out.push(')');
    }
}

/// `ast-print(handle, discs)` (heap op 92) — the runtime half of the compiler's `Ast.print`: render a
/// RUNTIME `Ast` heap value to its canonical re-readable s-expression text (a fresh owned String leaf),
/// byte-identical to the compile-time `print_ast_value` fold. BORROWS `handle` + `discs` (the caller owns
/// their release, like `value-encode`); the disc→variant mapping is read from the compiler-baked `discs`.
fn op_ast_print(handle: Handle, discs: Handle) -> Handle {
    let mut out = String::new();
    if let Some(d) = read_ast_discs(discs) {
        render_ast(handle, &d, &mut out);
    }
    op_str_new(out)
}

/// The NINE Ast-variant discriminants `ast-encode`/`ast-decode` need — the print descriptor's seven plus
/// `char` + `symbol` (encode/decode must round-trip EVERY variant, whereas print renders seven). A distinct
/// descriptor from `AstDiscs`: the shipped `ast-print` op bakes seven in its own order, so its reader stays
/// as-is. Field order mirrors the compiler's `AstDiscs` struct (`lower.rs`) — `[int, float, bool, str, name,
/// list, bytes, char, symbol]` — the order the compiler bakes the descriptor in.
struct AstEncDiscs {
    int: u32,
    float: u32,
    boolv: u32,
    strv: u32,
    name: u32,
    list: u32,
    bytes: u32,
    chr: u32,
    symbol: u32,
    // M2 (OPTION B) — the 7 native-collection reflected-Ast ctors, appended after `symbol`. The reflected
    // `Ast` sum gained `ListCtor`/`TupleCtor`/`RecordCtor`/`MapCtor`/`SetCtor` (each `(List Ast)`) and
    // `FieldPair`/`Member` (each `(Tuple Ast Ast)`); a compound decoded from a ctor-leaf head reflects to
    // the DISTINCT ctor, not a name-headed list. Baked positionally in this order by the descriptor synth.
    list_ctor: u32,
    tuple_ctor: u32,
    record_ctor: u32,
    map_ctor: u32,
    set_ctor: u32,
    field_pair: u32,
    member: u32,
}

/// Decode the baked 16-disc descriptor: 16 LEB128 varints in
/// `[int,float,bool,str,name,list,bytes,char,symbol, list_ctor,tuple_ctor,record_ctor,map_ctor,set_ctor,field_pair,member]`
/// order (the 7 M2 native-collection ctors appended last). `None` on a truncated descriptor (the compiler
/// always bakes a well-formed one; a pre-M2 9-disc descriptor truncates → `None`, which is correct: a B
/// runtime requires a B descriptor).
fn read_ast_enc_discs(discs: Handle) -> Option<AstEncDiscs> {
    let n = op_bytes_len(discs);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(discs, i) as u8);
    }
    let mut pos = 0usize;
    let mut next = || -> Option<u32> {
        let mut val: u32 = 0;
        let mut shift = 0u32;
        loop {
            let b = *buf.get(pos)?;
            pos += 1;
            val |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Some(val)
    };
    Some(AstEncDiscs {
        int: next()?,
        float: next()?,
        boolv: next()?,
        strv: next()?,
        name: next()?,
        list: next()?,
        bytes: next()?,
        chr: next()?,
        symbol: next()?,
        list_ctor: next()?,
        tuple_ctor: next()?,
        record_ctor: next()?,
        map_ctor: next()?,
        set_ctor: next()?,
        field_pair: next()?,
        member: next()?,
    })
}

/// Bridge a runtime heap integer (`bigint::Big`) to the codec's `ast::IntValue{negative, magnitude:BE}`. The
/// heap leaf's sign-magnitude bytes are `[sign][LITTLE-endian magnitude, trailing-zeros-stripped]`; the codec
/// wants a BIG-endian magnitude with no leading zeros, so reverse the magnitude bytes (LE→BE; the LE form has
/// no trailing zeros, so the reversed BE form has no leading zeros — already canonical). Zero → `[0]` → empty
/// magnitude = `IntValue::zero`.
fn big_to_intvalue(b: &bigint::Big) -> crate::ast::IntValue {
    let sm = b.to_sign_magnitude_bytes();
    let negative = sm.first().copied() == Some(1);
    let mut magnitude: Vec<u8> = sm[1..].to_vec();
    magnitude.reverse();
    crate::ast::IntValue {
        negative,
        magnitude,
    }
}

/// Walk a runtime heap `Ast` value into the shared cadenza-ast `Builder` `b`, returning the built node's
/// `StructId` — the runtime twin of the compiler's `encode_ast_value` (rcdzc `lower.rs`), building the SAME
/// leaves/structs so `codec::encode` of the finished arena is BYTE-IDENTICAL to the compile-time fold. `None`
/// on an unknown disc (not reached for a well-typed Ast). A non-finite float has no finite `Decimal` and no
/// leaf yet (awaits the `KIND_FLOAT_{NAN,POS_INF,NEG_INF}` tags) — it declines here for now.
fn encode_ast_to_arenas(
    h: Handle,
    d: &AstEncDiscs,
    b: &mut crate::ast::Builder,
) -> Option<crate::ast::StructId> {
    let disc = op_sum_disc(h);
    let payload = op_sum_payload(h);
    if disc == d.int {
        Some(b.atom_leaf(crate::ast::Leaf::Int {
            value: big_to_intvalue(&unbox_bigint(payload)),
            radix: crate::ast::Radix::Dec,
        }))
    } else if disc == d.float {
        // A finite float encodes as the exact-decimal `Leaf::Float`; a NON-FINITE float has no finite
        // Decimal, so it rides its own payload-less leaf tag (17/18/19): NaN → FloatNan, +inf →
        // FloatInf{false}, -inf → FloatInf{true}. Byte-identical to the compiler's `encode_ast_value`
        // fold (both write the same shared codec tag), so `Ast.encode` of a non-finite Ast.Float agrees
        // compile-time and at runtime (the decode inverse is in `decode_arenas_to_ast`).
        let f = op_get_float(payload);
        let leaf = if f.is_nan() {
            crate::ast::Leaf::FloatNan
        } else if f.is_infinite() {
            crate::ast::Leaf::FloatInf { negative: f < 0.0 }
        } else {
            crate::ast::Leaf::Float(crate::ast::Decimal::from_f64(f)?)
        };
        Some(b.atom_leaf(leaf))
    } else if disc == d.boolv {
        Some(b.atom_leaf(crate::ast::Leaf::Bool(op_get_bool(payload))))
    } else if disc == d.strv {
        Some(b.atom_leaf(crate::ast::Leaf::Str(op_str_get(payload).into())))
    } else if disc == d.name {
        Some(b.atom_leaf(crate::ast::Leaf::Name(op_str_get(payload).into())))
    } else if disc == d.symbol {
        Some(b.atom_leaf(crate::ast::Leaf::Sym(op_str_get(payload).into())))
    } else if disc == d.chr {
        // A `Char` payload is a boxed i32 scalar code point (never a heap handle); a valid `Ast.Char` always
        // holds a real Unicode scalar, so `from_u32` succeeds.
        let c = char::from_u32(op_get_int(payload) as u32)?;
        Some(b.atom_leaf(crate::ast::Leaf::Char(c)))
    } else if disc == d.bytes {
        let n = op_bytes_len(payload);
        let mut raw = Vec::with_capacity(n as usize);
        for i in 0..n {
            raw.push(op_bytes_get(payload, i) as u8);
        }
        Some(b.atom_leaf(crate::ast::Leaf::Bytes(raw.into())))
    } else if disc == d.list {
        // A generic name-headed (or empty) list payload is a persistent RRB vector (`vec-*`, NOT `arr-*`);
        // each element is itself an Ast. Stays `Ast.List` (no ctor head) — the inverse of decode's fall-through.
        let n = op_vec_len(payload);
        let mut children = Vec::with_capacity(n as usize);
        for i in 0..n {
            children.push(encode_ast_to_arenas(op_vec_get(payload, i), d, b)?);
        }
        Some(b.list(children))
    } else if disc == d.list_ctor {
        // M2 (OPTION B): a reflected first-class compound-ctor value. Its payload is a `(List Ast)` RRB vector
        // of the reflected children (for Record/Map, those children are themselves `FieldPair` Ast values);
        // emit head-first via `Builder::compound`, whose head is the ctor LEAF KIND — byte-identical to the
        // compile-time `encode_ast_value` (both go through the shared cadenza-ast `Builder`) and the exact
        // inverse of `decode_arenas_to_ast`'s ctor-head arm.
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::List, &children))
    } else if disc == d.tuple_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Tuple, &children))
    } else if disc == d.record_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Record, &children))
    } else if disc == d.map_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Map, &children))
    } else if disc == d.set_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Set, &children))
    } else if disc == d.field_pair {
        // FieldPair / Member payload is a `(Tuple Ast Ast)` = an `arr` of exactly two reflected children
        // (key,value for FieldPair; obj,key for Member).
        let k = encode_ast_to_arenas(op_arr_get(payload, 0), d, b)?;
        let val = encode_ast_to_arenas(op_arr_get(payload, 1), d, b)?;
        Some(b.field_pair(k, val))
    } else if disc == d.member {
        let obj = encode_ast_to_arenas(op_arr_get(payload, 0), d, b)?;
        let key = encode_ast_to_arenas(op_arr_get(payload, 1), d, b)?;
        Some(b.member(obj, key))
    } else {
        None
    }
}

/// Encode the `(List Ast)` RRB-vector payload of a reflected compound-ctor value into the arena children
/// (each element recursively encoded), collected into a `Vec` so the mutable `Builder` borrow is released
/// before the caller's `Builder::compound` reborrows it. `None` propagates any child's encode failure.
fn encode_ast_ctor_children(
    payload: Handle,
    d: &AstEncDiscs,
    b: &mut crate::ast::Builder,
) -> Option<Vec<crate::ast::StructId>> {
    let n = op_vec_len(payload);
    let mut children = Vec::with_capacity(n as usize);
    for i in 0..n {
        children.push(encode_ast_to_arenas(op_vec_get(payload, i), d, b)?);
    }
    Some(children)
}

/// `ast-encode(handle, discs)` (heap op 93) — the runtime half of the compiler's `Ast.encode`: serialize a
/// RUNTIME `Ast` heap value to its canonical `cdzast` binary form (a fresh owned Bytes leaf), BYTE-IDENTICAL
/// to the compile-time `Ast.encode` fold (both run the shared `crate::codec::encode` over the same `Arenas`).
/// BORROWS `handle` + `discs`. An Ast that cannot be built (an unknown disc / a non-finite float pending its
/// tag) yields empty Bytes — not reached for a well-typed finite Ast.
fn op_ast_encode(handle: Handle, discs: Handle) -> Handle {
    let bytes = read_ast_enc_discs(discs)
        .and_then(|d| {
            let mut b = crate::ast::Builder::new();
            let root = encode_ast_to_arenas(handle, &d, &mut b)?;
            Some(crate::codec::encode(&b.finish(root)))
        })
        .unwrap_or_default();
    let buf = op_bytes_alloc(bytes.len() as u32);
    for (i, &v) in bytes.iter().enumerate() {
        op_bytes_set(buf, i as u32, v as u32);
    }
    buf
}

/// Inverse of [`big_to_intvalue`]: a codec `ast::IntValue{negative, magnitude:BE}` → a runtime heap
/// `bigint::Big`. `Big::from_sign_magnitude_bytes` wants `[sign][LITTLE-endian magnitude]`, so reverse the
/// big-endian magnitude back to little-endian and prepend the sign byte.
fn intvalue_to_big(iv: &crate::ast::IntValue) -> bigint::Big {
    let mut sm = Vec::with_capacity(1 + iv.magnitude.len());
    sm.push(iv.negative as u8);
    sm.extend(iv.magnitude.iter().rev().copied());
    bigint::Big::from_sign_magnitude_bytes(&sm)
}

/// Rebuild a heap `Ast` value from a node of a `codec::decode`d cadenza-ast `Arenas` — the runtime twin of
/// the compiler's `arenas_to_ast_value` (rcdzc `lower.rs`) and the inverse of [`encode_ast_to_arenas`].
/// Builds each node with `op_sum_new` at the descriptor's discs, boxing scalar payloads exactly as a
/// constructed `Ast` value does (bigint leaf / boxed float / boxed char scalar / RRB `vec-push` for a list).
/// `None` on an out-of-range id or a leaf with no `Ast` variant (`BadEscape`/`BadChar` markers — which a
/// well-formed `Ast.encode` never emits), so a malformed document decodes to the `Err` case, never a trap.
fn decode_arenas_to_ast(
    arenas: &crate::ast::Arenas,
    sid: crate::ast::StructId,
    d: &AstEncDiscs,
) -> Option<Handle> {
    match arenas.structure.get(sid.0 as usize)? {
        crate::ast::Struct::Atom(lid) => {
            let h = match arenas.leaves.get(lid.0 as usize)? {
                crate::ast::Leaf::Int { value, .. } => {
                    op_sum_new(d.int, box_bigint(&intvalue_to_big(value)))
                }
                crate::ast::Leaf::Float(dec) => {
                    op_sum_new(d.float, op_box_float(f64::from_bits(dec.to_f64_bits())))
                }
                // The non-finite float VALUES (codec tags 17/18/19) rebuild as an `Ast.Float` holding
                // the non-finite `f64` — the heap `Ast.Float` box carries any `f64`, so NaN / ±∞ are
                // ordinary boxed values (the inverse of `ast-encode` emitting the non-finite tag for a
                // non-finite `Ast.Float`).
                crate::ast::Leaf::FloatNan => op_sum_new(d.float, op_box_float(f64::NAN)),
                crate::ast::Leaf::FloatInf { negative } => op_sum_new(
                    d.float,
                    op_box_float(if *negative {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }),
                ),
                crate::ast::Leaf::Bool(b) => op_sum_new(d.boolv, op_box_bool(*b)),
                crate::ast::Leaf::Str(s) => op_sum_new(d.strv, op_str_new(s.to_string())),
                crate::ast::Leaf::Name(s) => op_sum_new(d.name, op_str_new(s.to_string())),
                crate::ast::Leaf::Sym(s) => op_sum_new(d.symbol, op_str_new(s.to_string())),
                crate::ast::Leaf::Char(c) => op_sum_new(d.chr, op_box_int(*c as i64)),
                crate::ast::Leaf::Bytes(v) => {
                    let buf = op_bytes_alloc(v.len() as u32);
                    for (i, &b) in v.iter().enumerate() {
                        op_bytes_set(buf, i as u32, b as u32);
                    }
                    op_sum_new(d.bytes, buf)
                }
                // M2 (OPTION B): a compound-ctor head leaf (`Leaf::Ctor`/`FieldPair`/`Member`, codec kinds
                // 20-26) is NEVER a bare atom — it only ever appears as the HEAD of a `Struct::List` (handled
                // in the List arm below, dispatched to the DISTINCT reflected ctor). Reached as a standalone
                // atom it is a malformed document; decode is TOTAL (op94 → NULL on bad bytes, never a trap),
                // so fail cleanly.
                crate::ast::Leaf::Ctor(_)
                | crate::ast::Leaf::FieldPair
                | crate::ast::Leaf::Member => return None,
                crate::ast::Leaf::BadEscape(_) | crate::ast::Leaf::BadChar(_) => return None,
                // A type-suffixed numeric literal (`100N`/`0.5R`) is decoded to a plain Int/Float by the
                // codec, so it never appears in a decoded document; a stray occurrence fails cleanly
                // (decode is TOTAL), like the marker leaves above.
                crate::ast::Leaf::Suffixed { .. } => return None,
            };
            Some(h)
        }
        crate::ast::Struct::List(children) => {
            // M2 (OPTION B): if the list HEAD is a compound-ctor leaf, reflect to the DISTINCT first-class
            // reflected Ast ctor (native collections — no string head), built from the REMAINING children; a
            // generic name-headed (or empty) list stays `Ast.List`.
            if let Some(&head_sid) = children.first()
                && let Some(crate::ast::Struct::Atom(lid)) =
                    arenas.structure.get(head_sid.0 as usize)
                && let Some(head_leaf) = arenas.leaves.get(lid.0 as usize)
            {
                match head_leaf {
                    // The 5 collections carry a `(List Ast)` of their reflected tail elements.
                    crate::ast::Leaf::Ctor(c) => {
                        let disc = match c {
                            crate::ast::CompoundCtor::List => d.list_ctor,
                            crate::ast::CompoundCtor::Tuple => d.tuple_ctor,
                            crate::ast::CompoundCtor::Record => d.record_ctor,
                            crate::ast::CompoundCtor::Map => d.map_ctor,
                            crate::ast::CompoundCtor::Set => d.set_ctor,
                        };
                        let mut v = op_vec_empty();
                        for &ch in &children[1..] {
                            v = op_vec_push(v, decode_arenas_to_ast(arenas, ch, d)?);
                        }
                        return Some(op_sum_new(disc, v));
                    }
                    // FieldPair/Member carry a `(Tuple Ast Ast)` = (key,value) / (obj,key): exactly 2 elems.
                    crate::ast::Leaf::FieldPair | crate::ast::Leaf::Member => {
                        if children.len() != 3 {
                            return None; // malformed: a pair head needs exactly two elements
                        }
                        let a = decode_arenas_to_ast(arenas, children[1], d)?;
                        let b = decode_arenas_to_ast(arenas, children[2], d)?;
                        let tup = op_arr_alloc(2);
                        op_arr_set(tup, 0, a);
                        op_arr_set(tup, 1, b);
                        let disc = if matches!(head_leaf, crate::ast::Leaf::FieldPair) {
                            d.field_pair
                        } else {
                            d.member
                        };
                        return Some(op_sum_new(disc, tup));
                    }
                    _ => {} // a name/other head → the generic `Ast.List` below
                }
            }
            let mut v = op_vec_empty();
            for &c in children.iter() {
                v = op_vec_push(v, decode_arenas_to_ast(arenas, c, d)?);
            }
            Some(op_sum_new(d.list, v))
        }
    }
}

/// `ast-decode(bytes-handle, discs)` (heap op 94) — the runtime half of the compiler's `Ast.decode`, the
/// TOTAL inverse of `ast-encode`: parse a `Bytes` leaf as one canonical `cdzast` document (via the shared
/// `crate::codec::decode`) and rebuild the heap `Ast` value. Returns the `Ast` handle on success, or
/// `Handle::NULL` on any parse failure (wrong header / malformed / trailing bytes / a non-`Ast` leaf) — the
/// compiler's `Core::AstDecode` emit wraps the result (`h != null → Ok(h)`, else `Err`), so decode is total
/// (a bad byte sequence is DATA, never a trap). BORROWS both operands.
fn op_ast_decode(bytes_handle: Handle, discs: Handle) -> Handle {
    let Some(d) = read_ast_enc_discs(discs) else {
        return Handle::NULL;
    };
    let n = op_bytes_len(bytes_handle);
    let mut raw = Vec::with_capacity(n as usize);
    for i in 0..n {
        raw.push(op_bytes_get(bytes_handle, i) as u8);
    }
    match crate::codec::decode(&raw) {
        Some(arenas) => decode_arenas_to_ast(&arenas, arenas.root, &d).unwrap_or(Handle::NULL),
        None => Handle::NULL,
    }
}

#[cfg(target_arch = "wasm32")]
struct Component;

#[cfg(target_arch = "wasm32")]
impl Guest for Component {
    fn box_int(v: i64) -> u32 {
        op_box_int(v).to_u32()
    }
    fn get_int(handle: u32) -> i64 {
        op_get_int(Handle::from_u32(handle))
    }
    fn box_bool(v: bool) -> u32 {
        op_box_bool(v).to_u32()
    }
    fn get_bool(handle: u32) -> bool {
        op_get_bool(Handle::from_u32(handle))
    }
    fn box_float(v: f64) -> u32 {
        op_box_float(v).to_u32()
    }
    fn get_float(handle: u32) -> f64 {
        op_get_float(Handle::from_u32(handle))
    }
    fn box_float32(v: f32) -> u32 {
        op_box_float32(v).to_u32()
    }
    fn get_float32(handle: u32) -> f32 {
        op_get_float32(Handle::from_u32(handle))
    }
    fn bigint_of_i64(v: i64) -> u32 {
        op_bigint_of_i64(v).to_u32()
    }
    fn bigint_of_bytes(buf: u32) -> u32 {
        op_bigint_of_bytes(Handle::from_u32(buf)).to_u32()
    }
    fn bigint_to_i64_checked(handle: u32) -> i64 {
        op_bigint_to_i64_checked(Handle::from_u32(handle))
    }
    fn bigint_add(a: u32, b: u32) -> u32 {
        op_bigint_add(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_sub(a: u32, b: u32) -> u32 {
        op_bigint_sub(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_mul(a: u32, b: u32) -> u32 {
        op_bigint_mul(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_div(a: u32, b: u32) -> u32 {
        op_bigint_div(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_cmp(a: u32, b: u32) -> i64 {
        op_bigint_cmp(Handle::from_u32(a), Handle::from_u32(b))
    }
    fn arr_alloc(len: u32) -> u32 {
        op_arr_alloc(len).to_u32()
    }
    fn arr_set(arr: u32, index: u32, elem: u32) -> u32 {
        op_arr_set(Handle::from_u32(arr), index, Handle::from_u32(elem)).to_u32()
    }
    fn arr_get(arr: u32, index: u32) -> u32 {
        op_arr_get(Handle::from_u32(arr), index).to_u32()
    }
    fn arr_len(arr: u32) -> u32 {
        op_arr_len(Handle::from_u32(arr))
    }
    fn sum_new(disc: u32, payload: u32) -> u32 {
        op_sum_new(disc, Handle::from_u32(payload)).to_u32()
    }
    fn sum_disc(handle: u32) -> u32 {
        op_sum_disc(Handle::from_u32(handle))
    }
    fn sum_payload(handle: u32) -> u32 {
        op_sum_payload(Handle::from_u32(handle)).to_u32()
    }
    fn bytes_alloc(len: u32) -> u32 {
        op_bytes_alloc(len).to_u32()
    }
    fn bytes_set(buf: u32, index: u32, value: u32) -> u32 {
        op_bytes_set(Handle::from_u32(buf), index, value).to_u32()
    }
    fn bytes_get(buf: u32, index: u32) -> u32 {
        op_bytes_get(Handle::from_u32(buf), index)
    }
    fn bytes_len(buf: u32) -> u32 {
        op_bytes_len(Handle::from_u32(buf))
    }
    fn bytes_scalar_at(buf: u32, scalar_index: u32) -> u32 {
        op_bytes_scalar_at(Handle::from_u32(buf), scalar_index)
    }
    fn str_new(s: String) -> u32 {
        op_str_new(s).to_u32()
    }
    fn str_get(handle: u32) -> String {
        op_str_get(Handle::from_u32(handle))
    }
    fn map_alloc(len: u32) -> u32 {
        op_map_alloc(len).to_u32()
    }
    fn map_set(m: u32, index: u32, key: u32, value: u32) -> u32 {
        op_map_set(
            Handle::from_u32(m),
            index,
            Handle::from_u32(key),
            Handle::from_u32(value),
        )
        .to_u32()
    }
    fn map_key(m: u32, index: u32) -> u32 {
        op_map_key(Handle::from_u32(m), index).to_u32()
    }
    fn map_val(m: u32, index: u32) -> u32 {
        op_map_val(Handle::from_u32(m), index).to_u32()
    }
    fn map_len(m: u32) -> u32 {
        op_map_len(Handle::from_u32(m))
    }

    // ── CHAMP persistent map (§37–45) ────────────────────────────────────────────────────
    fn map_empty() -> u32 {
        op_map_empty().to_u32()
    }
    fn map_insert(m: u32, key: u32, val: u32) -> u32 {
        op_map_insert(
            Handle::from_u32(m),
            Handle::from_u32(key),
            Handle::from_u32(val),
        )
        .to_u32()
    }
    fn map_lookup(m: u32, key: u32) -> u32 {
        op_map_lookup(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_remove(m: u32, key: u32) -> u32 {
        op_map_remove(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_size(m: u32) -> u32 {
        op_map_size(Handle::from_u32(m))
    }
    fn map_iter(m: u32) -> u32 {
        op_map_iter(Handle::from_u32(m)).to_u32()
    }
    fn map_iter_next(cur: u32) -> u32 {
        op_map_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_key(cur: u32) -> u32 {
        op_map_iter_key(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_val(cur: u32) -> u32 {
        op_map_iter_val(Handle::from_u32(cur)).to_u32()
    }

    // ── CHAMP persistent set (§46–53) ────────────────────────────────────────────────────
    fn set_empty() -> u32 {
        op_set_empty().to_u32()
    }
    fn set_insert(s: u32, elem: u32) -> u32 {
        op_set_insert(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_contains(s: u32, elem: u32) -> bool {
        op_set_contains(Handle::from_u32(s), Handle::from_u32(elem))
    }
    fn set_remove(s: u32, elem: u32) -> u32 {
        op_set_remove(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_size(s: u32) -> u32 {
        op_set_size(Handle::from_u32(s))
    }
    fn set_iter(s: u32) -> u32 {
        op_set_iter(Handle::from_u32(s)).to_u32()
    }
    fn set_iter_next(cur: u32) -> u32 {
        op_set_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn set_iter_elem(cur: u32) -> u32 {
        op_set_iter_elem(Handle::from_u32(cur)).to_u32()
    }
    fn set_union(a: u32, b: u32) -> u32 {
        op_set_union(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_intersection(a: u32, b: u32) -> u32 {
        op_set_intersection(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_difference(a: u32, b: u32) -> u32 {
        op_set_difference(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }

    fn dup(handle: u32) {
        op_dup(Handle::from_u32(handle))
    }
    fn drop(handle: u32) {
        op_drop(Handle::from_u32(handle))
    }
    fn reset(node: u32) -> u32 {
        op_reset(Handle::from_u32(node)).to_u32()
    }
    fn arr_alloc_reuse(len: u32, token: u32) -> u32 {
        op_arr_alloc_reuse(len, Handle::from_u32(token)).to_u32()
    }
    fn sum_new_reuse(disc: u32, payload: u32, token: u32) -> u32 {
        op_sum_new_reuse(disc, Handle::from_u32(payload), Handle::from_u32(token)).to_u32()
    }
    fn vec_empty() -> u32 {
        op_vec_empty().to_u32()
    }
    fn vec_len(v: u32) -> u32 {
        op_vec_len(Handle::from_u32(v))
    }
    fn vec_get(v: u32, index: u32) -> u32 {
        op_vec_get(Handle::from_u32(v), index).to_u32()
    }
    fn vec_push(v: u32, elem: u32) -> u32 {
        op_vec_push(Handle::from_u32(v), Handle::from_u32(elem)).to_u32()
    }
    fn vec_prepend(v: u32, elem: u32) -> u32 {
        op_vec_prepend(Handle::from_u32(v), Handle::from_u32(elem)).to_u32()
    }
    fn vec_update(v: u32, index: u32, elem: u32) -> u32 {
        op_vec_update(Handle::from_u32(v), index, Handle::from_u32(elem)).to_u32()
    }
    fn vec_concat(a: u32, b: u32) -> u32 {
        op_vec_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn vec_split(v: u32, index: u32) -> (u32, u32) {
        let (l, r) = op_vec_split(Handle::from_u32(v), index);
        (l.to_u32(), r.to_u32())
    }
    fn vec_drop(v: u32, index: u32) -> u32 {
        // The tail `[index, len)` — builds ONLY the kept spine (no discarded left prefix). Byte-identical
        // to `split`+drop-left (guarded by `vec_drop_tail_matches_split_drop_left`), ~half the allocation.
        op_vec_drop_tail(Handle::from_u32(v), index).to_u32()
    }
    fn bigint_rem(a: u32, b: u32) -> u32 {
        op_bigint_rem(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_of(num: u32, den: u32) -> u32 {
        op_rational_of(Handle::from_u32(num), Handle::from_u32(den)).to_u32()
    }
    fn rational_num(r: u32) -> u32 {
        op_rational_num(Handle::from_u32(r)).to_u32()
    }
    fn rational_den(r: u32) -> u32 {
        op_rational_den(Handle::from_u32(r)).to_u32()
    }
    fn rational_add(a: u32, b: u32) -> u32 {
        op_rational_add(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_sub(a: u32, b: u32) -> u32 {
        op_rational_sub(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_mul(a: u32, b: u32) -> u32 {
        op_rational_mul(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_div(a: u32, b: u32) -> u32 {
        op_rational_div(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_cmp(a: u32, b: u32) -> i64 {
        op_rational_cmp(Handle::from_u32(a), Handle::from_u32(b))
    }
    fn vec_of_arr(arr: u32) -> u32 {
        op_vec_of_arr(Handle::from_u32(arr)).to_u32()
    }
    // Structural value equality (index 61) — the deep heap walk behind `=` on two runtime compounds.
    // BORROWS both operands (an inspector, like `set-contains`): `champ_eq` reads without touching
    // either refcount, so the caller drops a temporary operand itself. This is the SAME tagless
    // structural comparison the map/set key path runs, exposed for the language's `=`.
    fn value_eq(a: u32, b: u32) -> bool {
        champ_eq(Handle::from_u32(a), Handle::from_u32(b))
    }
    // Value-form encode (index 62) — render a runtime value to its canonical binary-AST document,
    // guided by the compiler-baked shape descriptor `desc` (a Bytes handle). BORROWS both `v` and
    // `desc` (an inspector — the caller/escape owns the release of `v`; `desc` is a constant). Returns a
    // fresh owned Bytes. A malformed descriptor / unrenderable shape yields the empty Bytes (the
    // compiler only bakes a well-formed descriptor, so this is a defensive total, never a trap).
    fn value_encode(v: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let doc = op_value_encode_form(Handle::from_u32(v), &bytes).unwrap_or_default();
        alloc(Vec::new(), doc).to_u32()
    }
    // Value-form DECODE (index 90) — the exact inverse of value-encode: read the canonical value-form
    // `bytes` document + the SAME shape `desc` value-encode reads, and CONSTRUCT a fresh owned heap value.
    // BORROWS `bytes` + `desc` (both constants/inputs the caller owns); returns a fresh owned handle (or the
    // NULL handle `0` on a shape/format mismatch — never traps, mirroring value-encode's malformed-desc
    // decline). See `op_value_decode`.
    fn value_decode(bytes: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let dn = op_bytes_len(desc_h);
        let mut desc_bytes = Vec::with_capacity(dn as usize);
        for i in 0..dn {
            desc_bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let doc_h = Handle::from_u32(bytes);
        let bn = op_bytes_len(doc_h);
        let mut doc_bytes = Vec::with_capacity(bn as usize);
        for i in 0..bn {
            doc_bytes.push(op_bytes_get(doc_h, i) as u8);
        }
        op_value_decode(&doc_bytes, &desc_bytes).to_u32()
    }
    // BLAKE3 content hash (index 91) — the digest of `bytes`'s Bytes-leaf contents as a fresh 32-byte Bytes
    // leaf. A generic `bytes -> digest` primitive (no tag/prefix — userspace owns domain separation, DESIGN-
    // compiler-primitives.md D7); the runtime half of the compiler's `Blake3.of`, sharing the one `blake3`
    // crate with the compile-time fold so both agree bit-for-bit. BORROWS `bytes` (an inspector); returns a
    // fresh owned handle the caller drops. See `op_hash_blake3`.
    fn hash_blake3(bytes: u32) -> u32 {
        op_hash_blake3(Handle::from_u32(bytes)).to_u32()
    }
    // Ast render (index 92) — the runtime half of `Ast.print`: a runtime Ast heap value → its canonical
    // s-expr text (a fresh String leaf), byte-identical to the compiler's print_ast_value. BORROWS both;
    // `discs` conveys the Ast variant discs (baked by the compiler, by-name — never hardcoded). See
    // `op_ast_print`.
    fn ast_print(handle: u32, discs: u32) -> u32 {
        op_ast_print(Handle::from_u32(handle), Handle::from_u32(discs)).to_u32()
    }
    fn ast_encode(handle: u32, discs: u32) -> u32 {
        op_ast_encode(Handle::from_u32(handle), Handle::from_u32(discs)).to_u32()
    }
    fn ast_decode(bytes_handle: u32, discs: u32) -> u32 {
        op_ast_decode(Handle::from_u32(bytes_handle), Handle::from_u32(discs)).to_u32()
    }
    // mark-immortal (index 95) — convert a build-once static heap node to IMMORTAL (dup/drop no-op +
    // census-excluded). See `op_mark_immortal`.
    fn mark_immortal(handle: u32) -> u32 {
        op_mark_immortal(Handle::from_u32(handle)).to_u32()
    }
    // Mark-immortal-DEEP (index 96) — transitively mark a heap value AND every node reachable through its
    // child handles IMMORTAL (RRB list interior+leaf nodes, CHAMP map interior nodes + `[k,v]` entries, and
    // the k/v/element payloads they own). The deep analogue of `mark-immortal` for a build-once static whose
    // value is a multi-node structure (a `>32` list, a map) with no compile-time per-node handle. See
    // `op_mark_immortal_deep`.
    fn mark_immortal_deep(handle: u32) -> u32 {
        op_mark_immortal_deep(Handle::from_u32(handle)).to_u32()
    }
    // Value-form COMPARE (index 86) — the blessed three-way order over two runtime compound values of the
    // same type, guided by the compiler-baked shape `desc` (read exactly as `value-encode` reads it). BORROWS
    // `a`, `b` (an inspector — the caller owns their release) and `desc` (a constant). Returns -1/0/1
    // (Less/Equal/Greater) or the sentinel 2 when the type offers no total order or the descriptor is
    // malformed (the compiler declines ordering for a non-orderable type, so 2 is a defensive not-reached).
    fn value_cmp(a: u32, b: u32, desc: u32) -> i32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let Some(descriptor) = decode_descriptor(&bytes) else {
            return 2; // malformed descriptor — unordered sentinel
        };
        match value_cmp_shaped(
            &descriptor,
            Handle::from_u32(a),
            Handle::from_u32(b),
            descriptor.root,
        ) {
            Some(core::cmp::Ordering::Less) => -1,
            Some(core::cmp::Ordering::Equal) => 0,
            Some(core::cmp::Ordering::Greater) => 1,
            None => 2, // a non-orderable shape (float/bytes/set/map leaf) — unordered sentinel
        }
    }
    // Value-form structural EQUALITY (index 88) — the descriptor-guided companion of `value-eq` (index 61).
    // `value-eq` is the tagless `champ_eq` PHYSICAL-byte walk (sound for a canonical-by-construction value);
    // this walks the shape descriptor element-by-element, so it is exact for a LIST (an RRB vector that is
    // element- but not shape-canonical) and for a FLOAT/BYTES leaf a list carries (byte-canonical equality —
    // nan==nan, -0.0≠+0.0 — which `value-cmp` DECLINES since a float offers equality but no total order).
    // BORROWS `a`, `b` (an inspector — the caller owns their release) and `desc` (a constant). A malformed
    // descriptor / unrepresentable shape reads as `false` (defensive total — the compiler bakes a well-formed
    // descriptor, so this is a not-reached). Consistent with `value-cmp`: `value-eq-shaped == true` iff
    // `value-cmp == 0` for an orderable type.
    fn value_eq_shaped(a: u32, b: u32, desc: u32) -> bool {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let Some(descriptor) = decode_descriptor(&bytes) else {
            return false; // malformed descriptor — defensive not-equal (never reached)
        };
        crate::value_eq_shaped(
            &descriptor,
            Handle::from_u32(a),
            Handle::from_u32(b),
            descriptor.root,
        )
        .unwrap_or(false) // an unrepresentable shape reads as not-equal (defensive total)
    }
    // Value CANONICALIZE (index 87) — the blessed canonical form of a runtime value of the type `desc`
    // describes: a fresh OWNED value byte-identical for any two values EQUAL as values, whatever their
    // construction. Emitted at a Map/Set KEY site for a list-typed (or list-containing) key so the tagless
    // CHAMP byte-walk (`champ_hash`/`champ_eq`) places construction-equal list keys in the SAME slot
    // (collections-and-text.md §162 — a key's identity is construction-independent). BORROWS `a` (the
    // caller retains/releases it) and `desc` (a constant); returns a fresh owned handle the caller drops
    // after a borrowing key op, exactly like a `bytes-compact`ed rope key. On a malformed descriptor the
    // canonicalize declines and we return a DUP of the input (identity — degrades to the pre-fix byte-walk,
    // never a trap, never a leak): the op is total.
    fn value_canonicalize(a: u32, desc: u32) -> u32 {
        let a_h = Handle::from_u32(a);
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let out = match decode_descriptor(&bytes) {
            Some(descriptor) => value_canonicalize_shaped(&descriptor, a_h, descriptor.root),
            None => None,
        };
        match out {
            Some(h) => h.to_u32(),
            None => {
                op_dup(a_h); // decline → fresh owned identity (never trap/leak)
                a_h.to_u32()
            }
        }
    }
    // `set-to-list(s, desc)` (index 83) — a SET's elements as a `List a` in canonical element-value order,
    // and `map-to-list(m, desc)` (index 84) — a MAP's entries as a `List (Tuple k v)` in canonical KEY
    // order. Both BORROW their collection + the compiler-baked shape `desc` (a Bytes handle read the same
    // way `value-encode` reads it), reuse the sorted canonical walk value-encode renders from (so program
    // iteration order == the canonical byte form, collections-and-text.md:149), and return a fresh owned
    // `List` handle. A malformed descriptor / non-scalar unorderable key/element yields the empty list.
    fn set_to_list(s: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        op_set_to_list(Handle::from_u32(s), &bytes).to_u32()
    }
    fn map_to_list(m: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        op_map_to_list(Handle::from_u32(m), &bytes).to_u32()
    }
    fn bytes_concat(a: u32, b: u32) -> u32 {
        op_bytes_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bytes_slice(buf: u32, start: u32, len: u32) -> u32 {
        op_bytes_slice(Handle::from_u32(buf), start, len).to_u32()
    }
    fn bytes_compact(buf: u32) -> u32 {
        op_bytes_compact(Handle::from_u32(buf)).to_u32()
    }
    fn str_nfc_normalize(s: u32) -> u32 {
        op_str_nfc(Handle::from_u32(s)).to_u32()
    }
    fn str_from_bytes(buf: u32) -> u32 {
        op_str_from_bytes(Handle::from_u32(buf)).to_u32()
    }
    // Debug leak oracle (index 54). The number of live heap objects, or 0 when the counter is not
    // compiled in (default build). Not imported by the compiler; a leak-check harness asserts it is 0
    // after a run to verify the Perceus dup/drop discipline balances.
    fn live_objects() -> u32 {
        live_object_count()
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(Component with_types_in bindings);

// ─── CHAMP (Compressed Hash-Array Mapped Prefix trie) shared node core ───────────────────
// The tag-free foundation shared by the persistent map AND set: header/bitmap/slot helpers plus
// structural hash and eq. The map/set insert/lookup/remove/iter ops are built on top of these.
//
// Node layout (mirrors the vector radix trie's tag-free discipline, §DESIGN):
//   raw     = [datamap:u32 LE][nodemap:u32 LE][size:u32 LE]   (12 bytes)
//   handles = [entries in ascending datamap-bit order][subnodes in ascending nodemap-bit order]
// datamap bit i ⇒ slot i is an inline ENTRY; nodemap bit i ⇒ slot i is a SUBNODE. `size` is the
// total entry count in the subtree (O(1) size + an eq inequality fast-reject). There is NO runtime
// tag — the compiler's static type selects the map vs set op family and the entry stride (2 vs 1).
//
// The trie is 32-way, 5 bits per level (reusing VEC_BITS/VEC_MASK), 7 levels over a 32-bit hash.

/// CHAMP node header size in bytes: three little-endian u32s (datamap, nodemap, size).
const CHAMP_HEADER_SIZE: usize = 12;

/// 32-bit FNV-1a offset basis and prime, for the structural hash.
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

/// One FNV-1a byte step: xor then multiply. `wrapping_mul` because FNV is defined mod 2^32.
#[inline]
fn fnv_step(h: u32, b: u8) -> u32 {
    (h ^ b as u32).wrapping_mul(FNV_PRIME)
}

// ── Header read/write ───────────────────────────────────────────────────────────────────

/// Read the datamap word (u32 LE at offset 0) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_datamap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 0)
}
/// Read the nodemap word (u32 LE at offset 4) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_nodemap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 4)
}
/// Read the subtree size (u32 LE at offset 8) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_size(raw: &[u8]) -> u32 {
    read_u32_at(raw, 8)
}
/// Build a CHAMP raw header `[datamap][nodemap][size]` (12 bytes, all little-endian).
#[allow(dead_code)]
fn champ_header(datamap: u32, nodemap: u32, size: u32) -> Raw {
    // Build the 12-byte `[datamap][nodemap][size]` header directly as an INLINE `Raw` — no transient
    // heap Vec (CHAMP_HEADER_SIZE == INLINE_RAW_CAP, so it always inlines). This is the alloc-saving
    // win: every fresh CHAMP node (merge split, collision, path-copy rebuild, empty map/set) previously
    // allocated a 12-byte Vec here that `alloc` then copied inline and dropped.
    let mut buf = [0u8; INLINE_RAW_CAP];
    buf[0..4].copy_from_slice(&datamap.to_le_bytes());
    buf[4..8].copy_from_slice(&nodemap.to_le_bytes());
    buf[8..12].copy_from_slice(&size.to_le_bytes());
    Raw::Inline {
        len: CHAMP_HEADER_SIZE as u8,
        buf,
    }
}

// ── Bitmap / slot arithmetic ──────────────────────────────────────────────────────────────

/// Number of inline entries in this node: popcount of the datamap.
#[allow(dead_code)]
fn data_count(datamap: u32) -> u32 {
    datamap.count_ones()
}
/// Number of child subnodes in this node: popcount of the nodemap.
#[allow(dead_code)]
fn subnode_count(nodemap: u32) -> u32 {
    nodemap.count_ones()
}
/// Position of slot `i`'s entry within the entry region: count of set datamap bits below `i`.
/// `i` is a 5-bit slot (0..=31), so `1 << i` never overflows.
#[allow(dead_code)]
fn entry_index_for_slot(datamap: u32, i: u32) -> u32 {
    (datamap & ((1u32 << i) - 1)).count_ones()
}
/// Position of slot `i`'s subnode within the subnode region: count of set nodemap bits below `i`.
#[allow(dead_code)]
fn subnode_index_for_slot(nodemap: u32, i: u32) -> u32 {
    (nodemap & ((1u32 << i) - 1)).count_ones()
}
/// The 5-bit trie index selected by `level` (0-based): hash bits [5*level, 5*level+5).
#[allow(dead_code)]
fn level_index(hash: u32, level: u32) -> u32 {
    (hash >> (VEC_BITS * level)) & VEC_MASK
}

// ── Node-kind discrimination (tag-free) ─────────────────────────────────────────────────

/// True iff `node` is the canonical EMPTY node: both bitmaps 0 AND no handles. This is the root of
/// an empty map/set. Kept disambiguated from a collision node (which also has both bitmaps 0).
#[allow(dead_code)]
fn is_empty_node(node: Handle) -> bool {
    with_node(node, true, |n| {
        champ_datamap(&n.raw) == 0 && champ_nodemap(&n.raw) == 0 && n.handles.is_empty()
    })
}
/// True iff `node` is a COLLISION node: both bitmaps 0 AND at least one handle. Holds entries that
/// share a full 32-bit hash; only occurs at maximum depth and is linear-scanned by structural eq.
#[allow(dead_code)]
fn is_collision_node(node: Handle) -> bool {
    with_node(node, false, |n| {
        champ_datamap(&n.raw) == 0 && champ_nodemap(&n.raw) == 0 && !n.handles.is_empty()
    })
}

// ── Structural hash (FNV-1a), ITERATIVE ─────────────────────────────────────────────────

/// FNV-1a over `h`'s OWN canonical raw bytes (no children), starting from the offset basis — the
/// per-node "leaf hash" both the arity-0 fast path and the general walk's Combine step fold. Folds
/// the SAME little-endian bytes a boxed twin carries, so an inline value hashes equal to its boxed
/// twin (open-Q#8): an immediate contributes unit → no bytes, bool → one byte, int → 8 LE bytes; a
/// real node contributes its `n.raw`; a null handle contributes nothing (bare offset basis). Unlike
/// the old `imm_canonical_raw`-based fold this allocates NO intermediate `Vec`.
#[inline]
fn champ_node_raw_hash(h: Handle) -> u32 {
    if is_immediate(h) {
        let mut acc = FNV_OFFSET;
        match imm_kind(h) {
            ImmKind::Unit => {}
            ImmKind::Bool => acc = fnv_step(acc, imm_as_bool(h) as u8),
            ImmKind::Int => {
                for b in (imm_as_int(h) as u64).to_le_bytes() {
                    acc = fnv_step(acc, b);
                }
            }
        }
        acc
    } else {
        with_node(h, FNV_OFFSET, |n| {
            let mut acc = FNV_OFFSET;
            for &b in n.raw.iter() {
                acc = fnv_step(acc, b);
            }
            acc
        })
    }
}

/// One step of `champ_hash`'s iterative post-order walk (module-scoped so the reusable thread-local
/// worklist below can name it). `Visit` expands a node's children; `Combine` folds the node's own raw
/// with the `arity` child hashes now on the results stack.
enum HashTask {
    Visit(Handle),
    Combine(Handle, usize),
}

runtime_local! {
    /// REUSED scratch worklists for `champ_hash`'s general (nested-compound) walk. A nested KEY is
    /// hashed on every insert/lookup/remove, and each hash needs a `work` task stack + a `results`
    /// hash stack — freshly allocating them (even pre-sized) was 2 heap allocs PER HASH. Caching them
    /// thread-locally lets each hash `clear()` + reuse the buffers: they grow ONCE to the high-water
    /// mark, then every subsequent hash is allocation-FREE. Safe because the runtime is single-threaded
    /// and `champ_hash`'s walk is iterative + never re-enters `champ_hash` (so the borrow never nests).
    static HASH_SCRATCH: core::cell::RefCell<(Vec<HashTask>, Vec<u32>)> =
        core::cell::RefCell::new((Vec::new(), Vec::new()));
}

runtime_local! {
    /// REUSED scratch worklist for `champ_eq`'s general (NESTED-compound) equality walk — the same
    /// alloc-elision as `HASH_SCRATCH`. The shallow fast path (both compounds have only arity-0
    /// children) never touches this; a genuinely nested compound (e.g. a record-of-records or
    /// tuple-of-tuples compared by the language `=`, now that `value-eq` exposes `champ_eq`) fell
    /// through to a freshly-allocated `Vec<(Handle,Handle)>` per comparison. Reusing one buffer
    /// (clear + refill) makes a nested value-eq allocation-FREE steady-state. Safe: single-threaded,
    /// the walk is iterative and never re-enters `champ_eq` (`with_raw_arity` uses a stack buffer), so
    /// the borrow never nests.
    static EQ_SCRATCH: core::cell::RefCell<Vec<(Handle, Handle)>> =
        core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED scratch worklist for `fill_rope_bytes` (the `bytes_flatten` materialize walk) — same
    /// alloc-elision as `HASH_SCRATCH`/`EQ_SCRATCH`. Every `String.at`/`Bytes` read + `bytes-compact`
    /// flattens a rope, and the walk needs a `(node, dst_off, src_start, count)` task stack; freshly
    /// allocating it was 1 heap alloc PER FLATTEN (on TOP of the output buffer — which the small path now
    /// keeps on the stack). Caching it thread-locally lets each flatten `clear()` + reuse: grow once, then
    /// every subsequent flatten is allocation-FREE steady-state — the lexer's per-char `String.at`-compact
    /// (the hot text-scan loop) now allocates NOTHING for a ≤12-byte result. Safe: single-threaded, the
    /// walk is iterative and never re-enters `bytes_flatten` (it only reads nodes + calls the O(1)
    /// `op_bytes_len`), so the borrow never nests.
    static FLATTEN_SCRATCH: core::cell::RefCell<Vec<(Handle, usize, usize, usize)>> =
        core::cell::RefCell::new(Vec::new());
}

/// A deterministic structural hash of the whole subtree rooted at `root`: FNV-1a over each node's
/// raw bytes folded with its children's hashes. Because the rep is canonical, structurally-equal
/// subtrees hash equal; differing raw or structure (very likely) differs.
///
/// ITERATIVE, not recursive: a post-order walk over an explicit task worklist plus a results stack
/// (mirroring `op_drop`'s worklist discipline) keeps native/wasm stack use at O(1) frames regardless
/// of trie depth. Null handles fold as the empty (offset-basis) hash. Does NOT cache — v1 recomputes.
#[allow(dead_code)]
fn champ_hash(root: Handle) -> u32 {
    // Fast path — the hot map/set KEY case. An arity-0 node (an immediate, or a scalar/string/bytes
    // leaf) hashes to exactly FNV-1a over its own canonical raw bytes with NO child folds, which is
    // precisely what the general walk produces from a single Visit→Combine of a childless node. Take
    // it directly and allocate NEITHER the `work`/`results` worklists below NOR any canonical-raw Vec.
    if is_immediate(root) || with_node(root, 0usize, |n| n.handles.len()) == 0 {
        return champ_node_raw_hash(root);
    }
    // Shallow-compound fast path — a 1-level compound KEY (a small tuple/record whose children are all
    // arity-0 leaves/immediates; the common compound-key shape). Its hash is the node's own raw fold
    // followed by each child's `champ_node_raw_hash` (each child is arity-0, so its own hash needs no
    // recursion). The general walk folds children in the order they sit on `results`, which — because
    // children are pushed in index order then popped LIFO — is REVERSE index order; reproduce that here.
    // Allocates NOTHING (vs the two worklist Vecs below). Falls through to the iterative walk only for a
    // genuinely NESTED compound (a child that is itself a compound).
    if let Some(hash) = with_node(root, None, |n| {
        if n.handles
            .iter()
            .all(|&c| is_immediate(c) || with_node(c, 0usize, |cn| cn.handles.len()) == 0)
        {
            let mut acc = FNV_OFFSET;
            for &b in n.raw.iter() {
                acc = fnv_step(acc, b);
            }
            for &c in n.handles.iter().rev() {
                for b in champ_node_raw_hash(c).to_le_bytes() {
                    acc = fnv_step(acc, b);
                }
            }
            Some(acc)
        } else {
            None // a nested child — use the general iterative walk
        }
    }) {
        return hash;
    }
    // General nested-compound walk. Two-phase task: `Visit` expands a node's children; `Combine` folds
    // this node's raw + the child hashes now on `results`. Children are pushed Visit-first so their
    // Combine completes before their parent's — a standard single-stack iterative post-order. The two
    // scratch stacks are REUSED from a thread-local (see `HASH_SCRATCH`): each call clears them and
    // returns them empty-but-capacious, so after the first nested hash they never allocate again — the
    // walk is allocation-FREE steady-state (was 2 allocs/hash even when pre-sized).
    HASH_SCRATCH.with(|cell| {
        let (work, results) = &mut *cell.borrow_mut();
        work.clear();
        results.clear();
        work.push(HashTask::Visit(root));
        while let Some(task) = work.pop() {
            match task {
                HashTask::Visit(h) => {
                    if is_immediate(h) {
                        // Inline value: arity 0, no children to expand. Combine folds its canonical raw.
                        work.push(HashTask::Combine(h, 0));
                        continue;
                    }
                    let arity = with_node(h, 0usize, |n| n.handles.len());
                    work.push(HashTask::Combine(h, arity));
                    with_node(h, (), |n| {
                        for &c in n.handles.iter() {
                            work.push(HashTask::Visit(c));
                        }
                    });
                }
                HashTask::Combine(h, arity) => {
                    // Fold this node's own canonical raw bytes (the SAME LE bytes a boxed twin carries, so
                    // an inline value hashes equal to its boxed twin, open-Q#8), then the child hashes.
                    let mut s = champ_node_raw_hash(h);
                    // Consume the `arity` child hashes on top of `results` (deterministic order).
                    let start = results.len().saturating_sub(arity);
                    for &child_hash in &results[start..] {
                        for b in child_hash.to_le_bytes() {
                            s = fnv_step(s, b);
                        }
                    }
                    results.truncate(start);
                    results.push(s);
                }
            }
        }
        results.pop().unwrap_or(FNV_OFFSET)
    })
}

// ── Structural eq, ITERATIVE ────────────────────────────────────────────────────────────

/// Structural equality of two subtrees: equal raw bytes AND equal child count AND recursively-equal
/// children. Only needed on a hash collision. Compares raw byte-for-byte, so floats (-0.0 ≠ 0.0),
/// bytes, and strings are handled with no special-casing. Null-safe: two nulls are equal, one null
/// and one non-null differ.
///
/// ITERATIVE via an explicit pair worklist; identical pointers (structural sharing) short-circuit.
/// The worklist is LAZILY allocated — the root pair is processed directly, and the `Vec` is created
/// only when a COMPOUND node actually needs its children pushed. The dominant map/set case (scalar or
/// immediate keys, arity 0) therefore allocates NOTHING (this is on the hot `op_map_lookup` /
/// `set-contains` / insert-overwrite-probe path, once per hash-collision key comparison).
#[allow(dead_code)]
fn champ_eq(a: Handle, b: Handle) -> bool {
    // Shallow-compound fast path — the hot compound-KEY compare (a small tuple/record key on a slot
    // hit): two equal-arity compounds whose children are ALL arity-0 are equal iff their raw bytes
    // match and every child pair is `with_raw_arity`-equal, WITHOUT allocating the worklist Vec below.
    // Only fires when NEITHER side is immediate and both are real nodes (the general path handles
    // immediates/nulls); a nested child (arity > 0) falls through to the lazy worklist.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(result) = unsafe {
            match (a.node_ref(), b.node_ref()) {
                (Some(na), Some(nb)) => {
                    if *na.raw != *nb.raw || na.handles.len() != nb.handles.len() {
                        Some(false) // roots differ ⇒ not equal, no descent
                    } else if na.handles.iter().chain(nb.handles.iter()).all(|&c| {
                        is_immediate(c)
                            || c.node_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)
                    }) {
                        // Shallow: every child on both sides is arity-0 → compare pairwise inline.
                        let eq = na.handles.iter().zip(nb.handles.iter()).all(|(&cx, &cy)| {
                            cx == cy
                                || with_raw_arity(cx, |rx, ax| {
                                    with_raw_arity(cy, |ry, ay| rx == ry && ax == ay)
                                })
                        });
                        Some(eq)
                    } else {
                        None // a nested child — use the general worklist walk
                    }
                }
                _ => None, // a null side — general path
            }
        } {
            return result;
        }
    }
    // General NESTED-compound walk. The worklist is REUSED from a thread-local (`EQ_SCRATCH`): each
    // call clears it and returns it empty-but-capacious, so after the first nested compare it never
    // allocates again (was a fresh `Vec` per nested comparison). Safe: single-threaded + the walk is
    // iterative and never re-enters `champ_eq`, so the borrow never nests.
    EQ_SCRATCH.with(|cell| {
        let work = &mut *cell.borrow_mut();
        work.clear();
        let mut pair = Some((a, b));
        while let Some((x, y)) = pair {
            // Process (x, y); `descend` is set to the children to push when both are equal compounds.
            let mut descend: Option<(&Node, &Node)> = None;
            if x == y {
                // same pointer (incl. both NULL) ⇒ identical subtree, no descent needed
            } else if is_immediate(x) || is_immediate(y) {
                // An immediate's `.0` is NOT a Node pointer — compare by decoded value (arity 0, so
                // equality reduces to equal canonical raw bytes and equal arity), WITHOUT allocating.
                let equal =
                    with_raw_arity(x, |rx, ax| with_raw_arity(y, |ry, ay| rx == ry && ax == ay));
                if !equal {
                    return false;
                }
            } else {
                match (unsafe { x.node_ref() }, unsafe { y.node_ref() }) {
                    (None, None) => {}
                    (Some(nx), Some(ny)) => {
                        if *nx.raw != *ny.raw || nx.handles.len() != ny.handles.len() {
                            return false;
                        }
                        descend = Some((nx, ny)); // equal compound roots — push children below
                    }
                    _ => return false, // exactly one null ⇒ differ
                }
            }
            if let Some((nx, ny)) = descend {
                for i in 0..nx.handles.len() {
                    work.push((nx.handles[i], ny.handles[i]));
                }
            }
            pair = work.pop();
        }
        true
    })
}

/// A deterministic, insertion-INDEPENDENT total order over key subtrees, used to canonicalize the
/// storage order of collision-node entries (keys that share a full 32-bit hash cannot be ordered by
/// hash, so we tiebreak purely on structure). Compares, in order: `raw` bytes lexicographically,
/// then `handles.len()`, then children index-by-index. A null handle orders BEFORE any non-null.
///
/// CONSISTENT with `champ_eq`: `champ_key_cmp(a, b) == Equal` IFF `champ_eq(a, b) == true` (both are
/// "no structural difference anywhere"). ITERATIVE via an explicit stack — a DFS pre-order with
/// children pushed reversed so index 0 is compared (and fully descended) before index 1, which makes
/// the first difference found the lexicographically-first one. No unbounded recursion (wasm-safe).
/// The worklist is LAZILY allocated (see `champ_eq`): the root pair is processed directly and the
/// `Vec` is created only when a compound needs children pushed, so ordering two scalar/immediate keys
/// (the common collision-canonicalization case) allocates nothing.
#[allow(dead_code)]
fn champ_key_cmp(a: Handle, b: Handle) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    // Shallow-compound fast path (mirrors `champ_eq`): order two compounds whose children are ALL
    // arity-0 by raw bytes, then arity, then children in INDEX order (the general walk descends index 0
    // first), each via `with_raw_arity` — WITHOUT the worklist Vec. Consistent with the shallow
    // `champ_eq` path (both reduce to the same per-child (raw, arity) compare). Nested ⇒ general walk.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(ord) = unsafe {
            match (a.node_ref(), b.node_ref()) {
                (Some(na), Some(nb)) => {
                    let shallow = na.handles.iter().chain(nb.handles.iter()).all(|&c| {
                        is_immediate(c)
                            || c.node_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)
                    });
                    if !shallow {
                        None // a nested child — use the general worklist walk
                    } else {
                        let mut ord = na
                            .raw
                            .as_slice()
                            .cmp(nb.raw.as_slice())
                            .then(na.handles.len().cmp(&nb.handles.len()));
                        let mut i = 0;
                        while ord == Ordering::Equal && i < na.handles.len() {
                            let (cx, cy) = (na.handles[i], nb.handles[i]);
                            if cx != cy {
                                ord = with_raw_arity(cx, |rx, ax| {
                                    with_raw_arity(cy, |ry, ay| rx.cmp(ry).then(ax.cmp(&ay)))
                                });
                            }
                            i += 1;
                        }
                        Some(ord)
                    }
                }
                _ => None, // a null side — general path
            }
        } {
            return ord;
        }
    }
    let mut work: Option<Vec<(Handle, Handle)>> = None;
    let mut pair = Some((a, b));
    while let Some((x, y)) = pair {
        let mut descend: Option<(&Node, &Node)> = None;
        if x == y {
            // same pointer (incl. both NULL) ⇒ identical so far
        } else if is_immediate(x) || is_immediate(y) {
            // Order by canonical (raw bytes, arity) WITHOUT allocating — the SAME (raw, arity) ordering
            // `champ_eq` compares (slice `cmp` is byte-lexicographic like `Vec` `cmp`), keeping the two
            // consistent (`cmp == Equal` iff `champ_eq`). An immediate has arity 0, no children.
            let ord = with_raw_arity(x, |rx, ax| {
                with_raw_arity(y, |ry, ay| rx.cmp(ry).then(ax.cmp(&ay)))
            });
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            match (unsafe { x.node_ref() }, unsafe { y.node_ref() }) {
                (None, None) => {} // both null (unreachable given x==y, but total)
                (None, Some(_)) => return Ordering::Less, // null orders before non-null
                (Some(_), None) => return Ordering::Greater,
                (Some(nx), Some(ny)) => {
                    match nx.raw.as_slice().cmp(ny.raw.as_slice()) {
                        Ordering::Equal => {}
                        ord => return ord, // raw bytes lexicographically
                    }
                    match nx.handles.len().cmp(&ny.handles.len()) {
                        Ordering::Equal => {}
                        ord => return ord, // then arity
                    }
                    descend = Some((nx, ny)); // equal so far — descend children
                }
            }
        }
        if let Some((nx, ny)) = descend {
            // Descend children in index order: push reversed so index 0 pops (and fully descends) first.
            let w = work.get_or_insert_with(Vec::new);
            for i in (0..nx.handles.len()).rev() {
                w.push((nx.handles[i], ny.handles[i]));
            }
        }
        pair = work.as_mut().and_then(Vec::pop);
    }
    Ordering::Equal
}

/// A work item for [`value_cmp_shaped`]'s iterative three-way walk: either a PAIR of handles to compare
/// under a shape index, or a LENGTH tiebreak (evaluated only after a list's common-prefix elements all
/// compared equal — the shorter list is Less).
enum CmpTask {
    Pair { a: Handle, b: Handle, shape_ix: u32 },
    LenTie { la: u32, lb: u32 },
}

/// The BLESSED total-order three-way comparison of two heap values `a`, `b` of the SAME type, guided by the
/// compiler-baked shape `desc` (the same descriptor `value_encode` reads). Returns `Some(Less/Equal/Greater)`
/// for an orderable pair, or `None` when the shape does not offer a total order (a Float/Float32 leaf — the
/// spec's #319 partial-order carve-out; a Bytes leaf — the spec blesses no Bytes order; a Set/Map — ordering
/// not yet offered) OR the descriptor is malformed. The compiler DECLINES ordering for those at lower time
/// (it never emits a `value-cmp` call for a non-orderable type), so `None` here is a defensive not-reached.
///
/// LEXICOGRAPHIC per `core-semantics.md` #Compound Ordering Is Lexicographic + #58/#64: leaves by their
/// blessed order (Int NUMERIC/signed, BigInt/Rational by value, Bool false<true, Str content-lexicographic
/// over the UTF-8 bytes = scalar order), tuples/records by field in canonical (descriptor) order, sums by
/// discriminant then payload, lists element-wise with a proper prefix LESS than its extension. Iterative
/// (an explicit `CmpTask` stack — wasm-safe, no unbounded native recursion over deep data, like
/// `champ_key_cmp`); the first non-`Equal` leaf/length decides and the walk returns immediately.
/// Compare two values as a SCALAR LEAF, allocation-free — the zero-Vec fast path `value_cmp_shaped` takes for
/// a scalar-rooted compare (and the shape a canonically-scalar Set/Map key/element always has). Returns:
///   - `None` — the shape (resolved through Named/Ref) is NOT a scalar leaf (a Tuple/List/Record/Sum): the
///     caller falls through to the general iterative work-stack walk.
///   - `Some(Some(ord))` — an orderable scalar leaf compared to `ord` (Int NUMERIC/signed, Bool false<true,
///     Unit always equal, Str content-lexicographic over the flattened UTF-8 bytes, BigInt/Rational by value).
///   - `Some(None)` — a scalar-position leaf that offers NO total order (Float/Float32/Bytes): the caller
///     returns this `None` (the op maps it to the unordered sentinel), matching the work-stack walk's verdict.
fn compare_scalar_leaf(
    desc: &Descriptor,
    a: Handle,
    b: Handle,
    shape_ix: u32,
) -> Option<Option<core::cmp::Ordering>> {
    use core::cmp::Ordering;
    let shape = resolve_shape(desc, shape_ix)?;
    Some(match shape {
        Shape::Int => Some(op_get_int(a).cmp(&op_get_int(b))),
        // A Char is SEMANTICS-int — order by the code-point integer (identical to `Int`), never a char-
        // specific collation; only its RENDER differs.
        Shape::Char => Some(op_get_int(a).cmp(&op_get_int(b))),
        Shape::Bool => Some(op_get_bool(a).cmp(&op_get_bool(b))),
        Shape::Unit => Some(Ordering::Equal),
        Shape::BigInt => Some(op_bigint_cmp(a, b).cmp(&0)),
        Shape::Rational => Some(op_rational_cmp(a, b).cmp(&0)),
        Shape::Str => {
            // Content-lexicographic over the flattened UTF-8 bytes — a rope String is flattened first
            // (content-preserving/unobservable), then the borrowed leaf slices compare with NO `to_vec`
            // (the zero-alloc discipline `champ_eq` uses). This is the arm the string-keyed-map sort hits.
            bytes_flatten(a);
            bytes_flatten(b);
            let av = unsafe { a.node_ref() };
            let bv = unsafe { b.node_ref() };
            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
            Some(as_.cmp(bs))
        }
        // A FLOAT orders by its CANONICAL BIT PATTERN as an UNSIGNED integer — NOT numeric order. This is the
        // element-derived deterministic order collections-and-text.md #Set Iteration Is Deterministic requires
        // (agreeing with the canonical byte form, which totally orders floats): every NaN is collapsed to the
        // one quiet NaN by `op_box_float` on construction, and +0.0/-0.0 keep their distinct bits, so the bit
        // pattern is a TOTAL order (no unordered pairs). Read the canonical bits via the width's getter and
        // compare as `u64`/`u32` — exactly the Rust backend's `__CdzF64`/`__CdzF32` wrapper order (`self.0`,
        // where `self.0 = to_bits()`), so a float Set.to-list / Map.to-list key enumerates identically on both
        // backends. By this order a NEGATIVE float (sign bit set = high bit) sorts AFTER every positive, and
        // -0.0 (0x8000…) after +0.0 (0x0). This is the ORDERING used ONLY where an order is offered (to-list
        // enumeration, a float Set/Map key sort); float `<` remains the IEEE partial order (NaN unordered),
        // decided at compile time — the compiler routes numeric `<` away from this walk.
        Shape::Float => Some(op_get_float(a).to_bits().cmp(&op_get_float(b).to_bits())),
        Shape::Float32 => Some(
            op_get_float32(a)
                .to_bits()
                .cmp(&op_get_float32(b).to_bits()),
        ),
        // Bytes has a blessed TOTAL order (§order): content-lexicographic over its UNSIGNED byte values —
        // the SAME machinery as `Shape::Str` above (both are byte leaves; a Bytes value may be a rope, so
        // flatten both to a leaf first — content-preserving/unobservable — then compare the borrowed `raw`
        // slices with NO `to_vec`, the zero-alloc discipline `champ_eq` uses). Unlike a float this order is
        // total AND composes inside a compound; the compiler's `orderable_leaf_or_compound` admits Bytes in
        // lockstep with this arm.
        Shape::Bytes => {
            bytes_flatten(a);
            bytes_flatten(b);
            let av = unsafe { a.node_ref() };
            let bv = unsafe { b.node_ref() };
            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
            Some(as_.cmp(bs))
        }
        // Not a scalar leaf — a compound / set / map / framed root: the caller falls through to the walk.
        Shape::Tuple(_)
        | Shape::List(_)
        | Shape::Record(_)
        | Shape::Sum(_)
        | Shape::Spread(_)
        | Shape::Set(_)
        | Shape::Map(..)
        | Shape::Framed(..) => return None,
        // Named/Ref were already resolved by `resolve_shape`; a residual one is a malformed descriptor.
        Shape::Named(..) | Shape::Ref(_) => return None,
    })
}

fn value_cmp_shaped(
    desc: &Descriptor,
    a: Handle,
    b: Handle,
    root_shape: u32,
) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;
    // ZERO-ALLOC SCALAR FAST PATH. A SCALAR-leaf root (Int/Bool/Unit/Str/BigInt/Rational, resolved through
    // Named/Ref) — the overwhelmingly common `value-cmp` shape AND the ONLY shape a canonically-scalar Set/Map
    // KEY or element takes — compares in place with NO work-stack `Vec`. This is what the old dedicated
    // `canonical_scalar_order` did; folding it in keeps a String-keyed `Map.to-list`/`Set.to-list` sort
    // (`sort_unstable_by` → O(N·log N) comparisons) allocation-FREE, which a `vec![…]`-per-compare would blow
    // past (~2·N·log N transient Vecs, the alloc-bench regression this restores). A COMPOUND root falls
    // through to the general iterative work-stack walk below (its per-call Vec is amortized over the whole
    // structure, not per leaf). `compare_scalar_leaf` returns `None` for a non-scalar shape (→ fall through)
    // or a genuinely non-orderable scalar-position leaf (Float/Float32/Bytes → the unordered `None` the op
    // maps to the sentinel).
    if let Some(ord) = compare_scalar_leaf(desc, a, b, root_shape) {
        return ord;
    }
    let mut work: Vec<CmpTask> = vec![CmpTask::Pair {
        a,
        b,
        shape_ix: root_shape,
    }];
    while let Some(task) = work.pop() {
        match task {
            CmpTask::LenTie { la, lb } => match la.cmp(&lb) {
                Ordering::Equal => {}
                ord => return Some(ord), // a proper prefix is Less than its extension
            },
            CmpTask::Pair { a, b, shape_ix } => {
                // Resolve `Ref`/`Named` indirections to the underlying shape (bounded by `resolve_shape`).
                let shape = resolve_shape(desc, shape_ix)?;
                match shape {
                    Shape::Int => match op_get_int(a).cmp(&op_get_int(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    // Char = semantics-int: order by the code-point integer, exactly as `Int`.
                    Shape::Char => match op_get_int(a).cmp(&op_get_int(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    Shape::Bool => match op_get_bool(a).cmp(&op_get_bool(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    Shape::Unit => {} // the unit value is a singleton — always equal
                    Shape::BigInt => match op_bigint_cmp(a, b) {
                        0 => {}
                        n if n < 0 => return Some(Ordering::Less),
                        _ => return Some(Ordering::Greater),
                    },
                    Shape::Rational => match op_rational_cmp(a, b) {
                        0 => {}
                        n if n < 0 => return Some(Ordering::Less),
                        _ => return Some(Ordering::Greater),
                    },
                    // A String is content-lexicographic over its UTF-8 bytes (== Unicode-scalar order for
                    // well-formed UTF-8, #58); a Bytes value is content-lexicographic over its UNSIGNED byte
                    // values (§order). BOTH are `raw`-byte leaves comparing identically, so they share this
                    // arm. Either may be a ROPE — flatten both to a leaf first (iterative, content-preserving/
                    // unobservable) so `raw` holds the logical bytes, then compare the borrowed slices without
                    // allocating (the same zero-alloc discipline `champ_eq` uses). Bytes composes soundly here
                    // inside a compound (its order is total), unlike a float — the compiler admits Bytes in
                    // lockstep (`orderable_leaf_or_compound`).
                    Shape::Str | Shape::Bytes => {
                        bytes_flatten(a);
                        bytes_flatten(b);
                        let ord = {
                            let av = unsafe { a.node_ref() };
                            let bv = unsafe { b.node_ref() };
                            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
                            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
                            as_.cmp(bs)
                        };
                        match ord {
                            Ordering::Equal => {}
                            o => return Some(o),
                        }
                    }
                    // Non-orderable — the spec offers no total order (defensive; the compiler declines these
                    // before emitting a value-cmp call). Float/Float32 (#319 partial), Set/Map (not offered).
                    Shape::Float | Shape::Float32 | Shape::Set(_) | Shape::Map(..) => {
                        return None;
                    }
                    Shape::Tuple(elems) => {
                        let elems = elems.clone();
                        if (op_arr_len(a) as usize) < elems.len()
                            || (op_arr_len(b) as usize) < elems.len()
                        {
                            return None; // malformed vs the descriptor
                        }
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: es,
                            });
                        }
                    }
                    Shape::Record(fields) => {
                        // A record's runtime rep is a `tuple` arr in the descriptor's field order (the same
                        // canonical order equality/encode use); compare field values in that order.
                        let fields: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                        if (op_arr_len(a) as usize) < fields.len()
                            || (op_arr_len(b) as usize) < fields.len()
                        {
                            return None;
                        }
                        for (i, &fs) in fields.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: fs,
                            });
                        }
                    }
                    Shape::List(elem) => {
                        // Lexicographic: compare min(la,lb) elements; if all equal, the SHORTER list is Less
                        // (the LenTie task, pushed FIRST so it evaluates LAST — only after every prefix pair).
                        let elem = *elem;
                        let (la, lb) = (op_vec_len(a), op_vec_len(b));
                        let minl = la.min(lb);
                        work.push(CmpTask::LenTie { la, lb });
                        for i in (0..minl).rev() {
                            work.push(CmpTask::Pair {
                                a: op_vec_get(a, i),
                                b: op_vec_get(b, i),
                                shape_ix: elem,
                            });
                        }
                    }
                    Shape::Sum(variants) => {
                        // By discriminant first (the variant's index, as the canonical byte form encodes it),
                        // then by payload within the same variant. Different discriminants decide immediately.
                        //
                        // DISC of an ALL-NULLARY sum stored as an Int IMMEDIATE (SOUNDNESS #43 witness 4): a
                        // nullary variant boxes via `box-int` (enum-disc → OP_BOX_INT), and a small disc
                        // (0/1/2…) fixnum_fits, so `op_box_int` returns an IMMEDIATE int carrying the disc as
                        // its value — NOT a heap sum node. `op_sum_disc` returns 0 for ANY immediate ("a sum
                        // is never itself an immediate"), so WITHOUT this every nullary key read disc 0 → all
                        // Equal → the stable to-list sort kept insertion order → wrong enumeration ({Hi,Mid,Lo}
                        // heads Mid not Lo). So decode the disc from the immediate's int value here; a
                        // payload-carrying variant is a real heap node (`op_sum_disc` reads its stored disc,
                        // unchanged). Localized to the cmp/sort path — `op_sum_disc`'s immediate→0 contract
                        // (relied on by the render/decode callers) is untouched.
                        let variants = variants.clone();
                        let (da, db) = (sum_disc_shaped(a), sum_disc_shaped(b));
                        // DECLINE on an out-of-range disc BEFORE ordering (PR#891): `sum_disc_shaped` returns
                        // `u32::MAX` for a malformed non-int immediate under a Sum shape. The `Equal` arm below
                        // already declines via `variants.get(da)?`, but the DIFFERING-disc arm returns an
                        // Ordering directly — so a `u32::MAX` (or otherwise out-of-range) disc would order
                        // deterministically-but-wrong instead of declining. Validate BOTH discs are real
                        // variant indices first; either out of range ⇒ malformed ⇒ decline (None), matching
                        // the render path + the descriptor-walk reject-don't-miscompile contract.
                        if da as usize >= variants.len() || db as usize >= variants.len() {
                            return None;
                        }
                        match da.cmp(&db) {
                            Ordering::Equal => {
                                let (_, payload_shape) = variants.get(da as usize)?;
                                work.push(CmpTask::Pair {
                                    a: op_sum_payload(a),
                                    b: op_sum_payload(b),
                                    shape_ix: *payload_shape,
                                });
                            }
                            ord => return Some(ord),
                        }
                    }
                    Shape::Spread(elems) => {
                        // A multi-payload variant's payload is a `Spread` — a tuple arr of the boxed payloads;
                        // compare element-wise in order (reached only as a Sum payload's shape).
                        let elems = elems.clone();
                        if (op_arr_len(a) as usize) < elems.len()
                            || (op_arr_len(b) as usize) < elems.len()
                        {
                            return None;
                        }
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: es,
                            });
                        }
                    }
                    // A `Framed` frame (`(: value type-node)`) wraps an INNER value shape (like `Named` but
                    // with a full type node) — transparent for ordering: compare the inner value. Not
                    // followed by `resolve_shape` (which only chases `Ref`/`Named`), so descend it here.
                    Shape::Framed(_type_node, inner) => {
                        let inner = *inner;
                        work.push(CmpTask::Pair {
                            a,
                            b,
                            shape_ix: inner,
                        });
                    }
                    // `Ref`/`Named` were resolved by `resolve_shape` above and never reach here.
                    Shape::Ref(_) | Shape::Named(..) => return None,
                }
            }
        }
    }
    Some(Ordering::Equal)
}

/// A task for the iterative `value_eq_shaped` walk — a pair of handles to compare under a shape. Simpler
/// than `CmpTask`: equality has no length-tiebreak (a length mismatch is decided immediately, in-line).
enum EqTask {
    Pair { a: Handle, b: Handle, shape_ix: u32 },
}

/// STRUCTURAL EQUALITY of two heap values of the same type, guided by the compiler-baked shape `desc` —
/// the equality companion of `value_cmp_shaped`. Returns `Some(true/false)`, or `None` on a malformed
/// descriptor (defensive; the compiler bakes only a well-formed one). BORROWS both operands (an inspector,
/// like `value-eq`/`value-cmp`).
///
/// Unlike `value_cmp_shaped`, EVERY leaf is compared by EQUALITY — including the ones value-cmp DECLINES for
/// ordering: a Float/Float32/Bytes leaf compares by its CANONICAL BYTE FORM (spec core-semantics §313: float
/// equality is TOTAL — NaN canonicalized to one form, ±0 distinct — even though float ORDERING is only the
/// §319 IEEE partial order; and a Bytes leaf is byte-canonical at construction). This is WHY a `List<Float>`
/// (or any list-containing-float compound) `=` needs THIS walk, not value-cmp (which declines the float leaf)
/// nor the tagless `champ_eq` (unsound for the non-shape-canonical RRB list SPINE): the walk descends the
/// list ELEMENT-WISE (shape-independent, like value-cmp's List arm) while comparing each leaf by its
/// byte-canonical form. Iterative (an explicit `EqTask` stack — wasm-safe on deep data, like
/// `champ_key_cmp`/`value_cmp_shaped`); the FIRST inequality (or length/discriminant mismatch) short-circuits.
#[allow(dead_code)] // wired in BRICK 2 (the value-eq-shaped op export + List<Float> `=` emit routing)
fn value_eq_shaped(desc: &Descriptor, a: Handle, b: Handle, root_shape: u32) -> Option<bool> {
    // Compare two byte-canonical LEAVES (Float/Float32/Bytes/String/Symbol) by their raw bytes. A rope
    // String/Bytes is flattened first (content-preserving, unobservable) so `raw` holds the logical bytes —
    // exactly the `Shape::Str` discipline in value_cmp_shaped, extended to the float/bytes leaves that
    // equality (unlike ordering) admits. A float leaf's `raw` is its canonical byte form (`op_box_float`
    // normalizes NaN + preserves ±0's sign bit), so byte-equality IS the spec's canonical-byte-form rule.
    fn leaf_bytes_eq(a: Handle, b: Handle) -> bool {
        bytes_flatten(a);
        bytes_flatten(b);
        let av = unsafe { a.node_ref() };
        let bv = unsafe { b.node_ref() };
        let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
        let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
        as_ == bs
    }
    let mut work: Vec<EqTask> = vec![EqTask::Pair {
        a,
        b,
        shape_ix: root_shape,
    }];
    while let Some(EqTask::Pair { a, b, shape_ix }) = work.pop() {
        let shape = resolve_shape(desc, shape_ix)?;
        match shape {
            Shape::Int => {
                if op_get_int(a) != op_get_int(b) {
                    return Some(false);
                }
            }
            // Char = semantics-int: equal iff the code-point integers are equal, exactly as `Int`.
            Shape::Char => {
                if op_get_int(a) != op_get_int(b) {
                    return Some(false);
                }
            }
            Shape::Bool => {
                if op_get_bool(a) != op_get_bool(b) {
                    return Some(false);
                }
            }
            Shape::Unit => {} // singleton — always equal
            Shape::BigInt => {
                if op_bigint_cmp(a, b) != 0 {
                    return Some(false);
                }
            }
            Shape::Rational => {
                if op_rational_cmp(a, b) != 0 {
                    return Some(false);
                }
            }
            // Byte-canonical leaves — equality by canonical raw bytes (float eq TOTAL per §313; Bytes/String
            // byte-canonical). This is the KEY difference from value_cmp_shaped, which DECLINES these.
            Shape::Float | Shape::Float32 | Shape::Bytes | Shape::Str => {
                if !leaf_bytes_eq(a, b) {
                    return Some(false);
                }
            }
            // Set/Map are canonical-by-construction CHAMP handles — equal iff champ_eq (byte-identical), which
            // IS sound for them (order-independent canonical rep). No descriptor descent needed.
            Shape::Set(_) | Shape::Map(..) => {
                if !champ_eq(a, b) {
                    return Some(false);
                }
            }
            Shape::Tuple(elems) => {
                let elems = elems.clone();
                if (op_arr_len(a) as usize) < elems.len() || (op_arr_len(b) as usize) < elems.len()
                {
                    return None;
                }
                for (i, &es) in elems.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: es,
                    });
                }
            }
            Shape::Record(fields) => {
                let fields: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                if (op_arr_len(a) as usize) < fields.len()
                    || (op_arr_len(b) as usize) < fields.len()
                {
                    return None;
                }
                for (i, &fs) in fields.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: fs,
                    });
                }
            }
            Shape::List(elem) => {
                // Element-wise, SHAPE-INDEPENDENT (the whole point — the RRB spine is not byte-canonical):
                // equal iff same length AND every element equal. A length mismatch is decided immediately.
                let elem = *elem;
                let (la, lb) = (op_vec_len(a), op_vec_len(b));
                if la != lb {
                    return Some(false);
                }
                for i in 0..la {
                    work.push(EqTask::Pair {
                        a: op_vec_get(a, i),
                        b: op_vec_get(b, i),
                        shape_ix: elem,
                    });
                }
            }
            Shape::Sum(variants) => {
                let variants = variants.clone();
                let (da, db) = (op_sum_disc(a), op_sum_disc(b));
                if da != db {
                    return Some(false); // different variants ⇒ unequal
                }
                let (_, payload_shape) = variants.get(da as usize)?;
                work.push(EqTask::Pair {
                    a: op_sum_payload(a),
                    b: op_sum_payload(b),
                    shape_ix: *payload_shape,
                });
            }
            Shape::Spread(elems) => {
                let elems = elems.clone();
                if (op_arr_len(a) as usize) < elems.len() || (op_arr_len(b) as usize) < elems.len()
                {
                    return None;
                }
                for (i, &es) in elems.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: es,
                    });
                }
            }
            Shape::Framed(_type_node, inner) => {
                let inner = *inner;
                work.push(EqTask::Pair {
                    a,
                    b,
                    shape_ix: inner,
                });
            }
            Shape::Ref(_) | Shape::Named(..) => return None,
        }
    }
    Some(true)
}

/// A task for the iterative post-order rebuild in `value_canonicalize_shaped`. `Visit` expands a node
/// (pushing a matching `Build*` then its children); each `Build*` pops its now-canonical children off the
/// results stack and assembles the canonical parent. A single explicit stack → no native recursion over
/// deep data (wasm-safe, like `champ_key_cmp`/`encode_value`).
enum CanonTask {
    Visit {
        h: Handle,
        shape_ix: u32,
        refs: u32,
    },
    /// Pop `n` canonical elements (in child order) → a fresh `arr`, then `op_vec_of_arr` → the canonical
    /// STRICT left-full RRB vec (the unique push-shape). This is what makes a concat-built list key
    /// byte-identical to a push-built one.
    BuildList {
        n: usize,
    },
    /// Pop `n` canonical elements → a fresh `arr` (the runtime rep of a tuple/record/spread).
    BuildArr {
        n: usize,
    },
    /// Pop ONE canonical payload → `op_sum_new(disc, payload)`.
    BuildSum {
        disc: u32,
    },
}

/// Build a fresh `arr` from the LAST `n` handles on `results` (in child order), which are MOVED in (each
/// is a fresh owned canonical child — no dup, no drop). Returns the arr (owned). `n == 0` → the inline unit.
fn canon_build_arr(results: &mut Vec<Handle>, n: usize) -> Handle {
    let start = results.len() - n;
    let arr = op_arr_alloc(n as u32);
    for (i, h) in results.drain(start..).enumerate() {
        op_arr_set(arr, i as u32, h);
    }
    arr
}

/// Drop every partially-built canonical handle on `results` and return `None` — the cleanup path when a
/// malformed descriptor / arity mismatch aborts the walk, so a decline never LEAKS the work done so far.
fn canon_decline(results: &mut Vec<Handle>) -> Option<Handle> {
    for h in results.drain(..) {
        op_drop(h);
    }
    None
}

/// Produce the BLESSED CANONICAL form of a heap value `a` of the type described by `desc`/`root_shape`: a
/// fresh OWNED value, byte-identical for any two values that are EQUAL as values regardless of how each was
/// constructed. The load-bearing case is a **List** (RRB vector): a concat-built and a push-built list with
/// the same elements have DIFFERENT internal shapes (relaxed interior nodes vs a strict trie), so the
/// tagless `champ_hash`/`champ_eq` byte-walk places them in different CHAMP slots — a Map/Set with such a
/// list KEY false-MISSES (collections-and-text.md §162: a key's identity is construction-INDEPENDENT). This
/// rebuilds every list to its unique strict left-full shape (via `op_vec_of_arr`), recursing through any
/// list-CONTAINING compound (a `(tuple (list…) Int)` key), so the byte-walk becomes exact.
///
/// BORROWS `a` (returns a fresh owned tree; the caller/key-site releases `a` and later drops the returned
/// temporary — the model `value-encode` uses). Scalar leaves (Int/BigInt/Rational/Bool/Unit/Float/Float32)
/// are canonical by construction → dup as-is. Str/Bytes → `bytes_flatten` (a rope's canonical flat leaf,
/// shared). Set/Map → dup as-is: a CHAMP is canonical at its OWN level; a Set-of-lists / Map-with-list-
/// values (a list buried inside a collection that is itself a key) is a rarer RESIDUAL — the same nested
/// edge the String/Bytes key story leaves — deferred to a follow-on. TOTAL: a malformed descriptor or
/// arity mismatch declines to `None` (the caller falls back to the input as-is), never traps, never leaks.
/// Iterative (an explicit `CanonTask` stack) so a deeply-nested list does not overflow the guest stack.
fn value_canonicalize_shaped(desc: &Descriptor, a: Handle, root_shape: u32) -> Option<Handle> {
    let mut work: Vec<CanonTask> = vec![CanonTask::Visit {
        h: a,
        shape_ix: root_shape,
        refs: 0,
    }];
    let mut results: Vec<Handle> = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            CanonTask::BuildList { n } => {
                let arr = canon_build_arr(&mut results, n);
                results.push(op_vec_of_arr(arr));
            }
            CanonTask::BuildArr { n } => {
                let arr = canon_build_arr(&mut results, n);
                results.push(arr);
            }
            CanonTask::BuildSum { disc } => {
                let payload = results.pop()?;
                results.push(op_sum_new(disc, payload));
            }
            CanonTask::Visit { h, shape_ix, refs } => {
                if refs > ENCODE_REF_CYCLE_CAP {
                    return canon_decline(&mut results); // a Ref/Named chain that never reaches a node
                }
                match desc.table.get(shape_ix as usize) {
                    None => return canon_decline(&mut results),
                    // Indirections: same `h`, no node reached → count toward the cycle cap.
                    Some(Shape::Ref(target) | Shape::Named(_, target)) => {
                        work.push(CanonTask::Visit {
                            h,
                            shape_ix: *target,
                            refs: refs + 1,
                        });
                    }
                    // Scalar leaves are canonical BY CONSTRUCTION (`box_*` normalizes: BigInt sign-magnitude,
                    // Rational lowest-terms, Float NaN one byte form). Retain the borrowed handle as the fresh
                    // owned result — an immediate's `op_dup` is a no-op and owns no heap.
                    Some(
                        Shape::Int
                        | Shape::BigInt
                        | Shape::Rational
                        | Shape::Bool
                        | Shape::Char
                        | Shape::Unit
                        | Shape::Float
                        | Shape::Float32,
                    ) => {
                        op_dup(h);
                        results.push(h);
                    }
                    // A String/Bytes may be a ROPE → flatten to its canonical flat leaf (in place, content-
                    // preserving/unobservable even on a shared node), then retain. A flat leaf flattens no-op.
                    Some(Shape::Str | Shape::Bytes) => {
                        bytes_flatten(h);
                        op_dup(h);
                        results.push(h);
                    }
                    // A Set/Map handle is canonical at its OWN level (order-independent CHAMP). Retain as-is;
                    // a list buried inside it is the documented residual (see the fn doc).
                    Some(Shape::Set(_) | Shape::Map(..)) => {
                        op_dup(h);
                        results.push(h);
                    }
                    // THE load-bearing arm: rebuild the list to its canonical strict shape. Canonicalize each
                    // element first (recurse), then `BuildList` reassembles via `op_vec_of_arr`.
                    Some(Shape::List(elem)) => {
                        let elem = *elem;
                        let len = op_vec_len(h);
                        work.push(CanonTask::BuildList { n: len as usize });
                        for i in (0..len).rev() {
                            work.push(CanonTask::Visit {
                                h: op_vec_get(h, i),
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    // A tuple / record / multi-payload spread is an `arr` at run time; canonicalize each field
                    // in the descriptor's canonical order, then `BuildArr` reassembles the arr.
                    Some(Shape::Tuple(elems)) => {
                        let elems = elems.clone();
                        if (op_arr_len(h) as usize) < elems.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: elems.len() });
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: es,
                                refs: 0,
                            });
                        }
                    }
                    Some(Shape::Spread(elems)) => {
                        let elems = elems.clone();
                        if (op_arr_len(h) as usize) < elems.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: elems.len() });
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: es,
                                refs: 0,
                            });
                        }
                    }
                    Some(Shape::Record(fields)) => {
                        let field_ixs: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                        if (op_arr_len(h) as usize) < field_ixs.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: field_ixs.len() });
                        for (i, &fs) in field_ixs.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: fs,
                                refs: 0,
                            });
                        }
                    }
                    // A sum: canonicalize the payload under the ACTIVE variant's shape, then rebuild the shell.
                    Some(Shape::Sum(variants)) => {
                        let variants = variants.clone();
                        let disc = op_sum_disc(h);
                        let Some((_, payload_shape)) = variants.get(disc as usize) else {
                            return canon_decline(&mut results);
                        };
                        work.push(CanonTask::BuildSum { disc });
                        work.push(CanonTask::Visit {
                            h: op_sum_payload(h),
                            shape_ix: *payload_shape,
                            refs: 0,
                        });
                    }
                    // A `(: value type-node)` frame — transparent for the VALUE: canonicalize the inner value.
                    Some(Shape::Framed(_type_node, inner)) => {
                        work.push(CanonTask::Visit {
                            h,
                            shape_ix: *inner,
                            refs: refs + 1,
                        });
                    }
                }
            }
        }
    }
    // Exactly one fully-assembled canonical root remains (a well-formed walk); anything else is malformed.
    if results.len() == 1 {
        results.pop()
    } else {
        canon_decline(&mut results)
    }
}

// ─── CHAMP persistent MAP: empty / lookup / insert / size ───────────────────────────────
// Built on the U1 node core. Map stride = 2: an entry occupies two consecutive handles
// `[key, value]`. A node's handles are `[k0,v0,k1,v1,…]` (entries, ascending datamap-bit order)
// followed by `[sub0,sub1,…]` (subnodes, ascending nodemap-bit order). `size` (raw offset 8) is the
// total entry count of the subtree, giving O(1) `op_map_size` and a fast eq inequality gate.
//
// Descent is 5 bits/level over a 32-bit hash, so the trie is at most `CHAMP_LEVELS` deep. Keys that
// share their full 32-bit hash land in a COLLISION node (both bitmaps 0, handles nonempty) at the
// bottom and are linear-scanned by `champ_eq`. Because that depth is bounded by the hash WIDTH (not
// by data size), the insert split recursion is bounded to ≤7 frames — categorically unlike the
// free cascade, so recursion here is stack-safe. Lookup is an explicit iterative descent.
//
// A SHARED spine is path-copied (fully immutable/persistent): every node on the modified path is
// rebuilt and its retained children are `op_dup`'d, so structural sharing across versions is exact. A
// UNIQUELY-OWNED spine (`rc == 1`) instead refits in place via `champ_insert_fbip`'s `mine` gate — the
// FBIP fast path — since no other reference can observe the mutation.

/// Number of 5-bit trie levels over a 32-bit hash (levels 0..=6; level 6 consumes the top 2 bits).
/// At or beyond this depth the hash is exhausted and colliding keys share a collision node.
const CHAMP_LEVELS: u32 = 7;

/// Read a node's subtree size (raw offset 8). Borrows; a null/short node reads 0.
#[allow(dead_code)]
fn champ_size_of(node: Handle) -> u32 {
    with_node(node, 0, |n| champ_size(&n.raw))
}

/// The canonical empty map: both bitmaps 0, size 0, no handles (exactly `is_empty_node`). U3's
/// remove-to-empty MUST reproduce this representation so callers can recognise emptiness uniformly.
#[allow(dead_code)]
// The shared IMMORTAL empty-MAP singleton (the IMM_UNIT / empty-vec analog for maps) — lazily minted,
// rc=IMMORTAL (census-excluded), so an empty map allocates ONCE and is reused, never per-occurrence.
runtime_local! {
    static EMPTY_MAP: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

fn op_map_empty() -> Handle {
    // One shared immortal empty CHAMP node. An empty map is CONSTANT (no entries), so one immortal is
    // correct + census-EXCLUDED (never a leak) + free to share. SOUND: map insert gates on node_rc==1
    // (champ_insert_fbip's `mine`), and IMMORTAL != 1, so an insert onto the singleton takes the proven
    // COPY path — the shared empty is never mutated in place.
    EMPTY_MAP.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = alloc_raw(Vec::new(), champ_header(0, 0, 0));
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// O(1) entry count of the map. BORROWS `m` (no rc change).
#[allow(dead_code)]
fn op_map_size(m: Handle) -> u32 {
    champ_size_of(m)
}

/// Shared stride-aware membership descent: find `key`, returning `Some((deepest_node, base))` where
/// `base` is the entry's first-column index in that node's `handles`, or `None` on miss. BORROWS.
/// Iterative descent (a `while` loop, no recursion), mirroring `op_vec_get`. Map lookup reads the
/// value at `base+1`; set contains just checks presence.
#[allow(dead_code)]
fn champ_find_base(m: Handle, key: Handle, stride: usize) -> Option<(Handle, usize)> {
    champ_find_base_h(m, key, champ_hash(key), stride)
}

/// `champ_find_base` but with `key`'s hash PRECOMPUTED by the caller — so a caller that already needs
/// `champ_hash(key)` for a following insert (set-algebra ∩/∖: probe-then-insert) computes it ONCE
/// instead of paying a second full subtree hash walk (costly for string/compound keys). BORROWS.
#[allow(dead_code)]
fn champ_find_base_h(m: Handle, key: Handle, hash: u32, stride: usize) -> Option<(Handle, usize)> {
    enum Step {
        Hit(usize),
        Miss,
        Descend(Handle),
    }
    let mut node = m;
    let mut level = 0u32;
    loop {
        let step = with_node(node, Step::Miss, |n| {
            let datamap = champ_datamap(&n.raw);
            let nodemap = champ_nodemap(&n.raw);
            if datamap == 0 && nodemap == 0 {
                // Empty node ⇒ miss; collision node ⇒ linear scan of entries.
                let mut idx = 0;
                while idx < n.handles.len() {
                    if champ_eq(n.handles[idx], key) {
                        return Step::Hit(idx);
                    }
                    idx += stride;
                }
                return Step::Miss;
            }
            let i = level_index(hash, level);
            let bit = 1u32 << i;
            if datamap & bit != 0 {
                let base = stride * entry_index_for_slot(datamap, i) as usize;
                if champ_eq(n.handles[base], key) {
                    return Step::Hit(base);
                }
                return Step::Miss;
            }
            if nodemap & bit != 0 {
                let sidx = subnode_index_for_slot(nodemap, i) as usize;
                let sbase = stride * data_count(datamap) as usize;
                return Step::Descend(n.handles[sbase + sidx]);
            }
            Step::Miss
        });
        match step {
            Step::Hit(base) => return Some((node, base)),
            Step::Miss => return None,
            Step::Descend(child) => {
                node = child;
                level += 1;
            }
        }
    }
}

/// Look up `key`, returning its value handle or `Handle::NULL` on miss. BORROWS both `m` and `key`
/// (no rc change — the returned handle is borrowed too; the caller dups if it needs to retain it).
#[allow(dead_code)]
fn op_map_lookup(m: Handle, key: Handle) -> Handle {
    match champ_find_base(m, key, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base + 1),
        None => Handle::NULL,
    }
}

// ─── STRIDE-PARAMETERIZED insert core (shared by MAP stride 2 and SET stride 1) ─────────
// An "entry" is the incoming key/element columns for ONE insert; `entry.key()` (column 0) is the
// key/element compared by `champ_eq`. Map entries are `[key, value]` (len 2); set entries are `[elem]`
// (len 1). The generic OVERWRITE rule (key already present) keeps the STORED key and takes the incoming
// value columns `entry[1..]`, dropping the incoming duplicate `entry[0]`; the old stored value columns
// die with the consumed node. For a set (len 1) that degenerates to "keep stored, drop incoming
// element" — idempotent insert, size unchanged.
//
// `Entry` is an INLINE (stack) buffer of ≤2 handles, NOT a heap `Vec` — an insert is on the hot path
// (map/set insert + every set-algebra element), so allocating a Vec per insert was pure waste (the
// entry's handles are only ever MOVED into node storage, never shared). An entry holds Copy `Handle`s,
// so moving it touches no refcount — refcount-identical to the old `Vec<Handle>`. It is deliberately
// NOT `Copy`: the compiler's move-checking then still enforces that an entry (and each handle it owns)
// is consumed exactly once — the linearity discipline the old owned `Vec` gave us for free.

/// The incoming columns for one insert: `len` handles (1 for a set elem, 2 for a map `[k,v]`) in a
/// fixed inline `[Handle; 2]`. Move-only (no `Copy`/`Clone`) so single-consumption is compiler-checked.
struct Entry {
    cols: [Handle; 2],
    len: usize,
}
impl Entry {
    /// A set element (len 1).
    fn elem(e: Handle) -> Entry {
        Entry {
            cols: [e, Handle::NULL],
            len: 1,
        }
    }
    /// A map key/value pair (len 2).
    fn kv(k: Handle, v: Handle) -> Entry {
        Entry {
            cols: [k, v],
            len: 2,
        }
    }
    /// The key/element column (column 0), compared by `champ_eq`.
    fn key(&self) -> Handle {
        self.cols[0]
    }
    /// Column `i` (0 ≤ i < len).
    fn col(&self, i: usize) -> Handle {
        self.cols[i]
    }
    /// The number of columns (the insert stride: 1 for a set, 2 for a map).
    fn len(&self) -> usize {
        self.len
    }
    /// Consume the entry, appending its columns onto `out` — used where the columns become part of a
    /// freshly-built node handle vector (the rare fresh-single / split / collision-append paths).
    fn extend_into(self, out: &mut Vec<Handle>) {
        out.extend_from_slice(&self.cols[..self.len]);
    }
    /// Consume the entry into a `Handles` built INLINE — for the fresh-single-entry node whose `handles`
    /// IS exactly this entry. An entry has ≤2 columns (= INLINE_HANDLES_CAP), so this always fits the
    /// inline arm with NO heap Vec, unlike a `Vec`-based build that `From<Vec>` would then re-inline and
    /// free (a transient alloc on every fresh CHAMP node — the common map/set build path).
    fn into_handles(self) -> Handles {
        Handles::inline_from(&self.cols[..self.len])
    }
    /// The entry's columns as a slice — for a caller that copies them into node storage BY VALUE
    /// (handles are `Copy`, so this relocates them without dup/drop, exactly like moving the old Vec's
    /// elements). The caller is responsible for consuming the entry exactly once overall.
    fn cols(&self) -> &[Handle] {
        &self.cols[..self.len]
    }
}

/// Append `entry`'s columns onto `out` by value (handles are `Copy`). Used at the collision-node
/// splice site, which conditionally splices at one of two positions; the caller `drop`s the entry once
/// after the single splice actually runs, so this borrows rather than consumes.
fn entry_splice(out: &mut Vec<Handle>, entry: &Entry) {
    out.extend_from_slice(entry.cols());
}

/// Build an `Entry` from the `stride` columns of `handles` starting at `base` — the STORED entry a
/// SPLIT folds together with the newcomer. `dup` ⇒ retain a reference to each column (the copy path,
/// where the consumed node still owns its copy); `!dup` ⇒ relocate the columns (the FBIP path, where
/// the node's handle vector was already taken and these references move out).
fn stored_entry_from(handles: &[Handle], base: usize, stride: usize, dup: bool) -> Entry {
    let mut cols = [Handle::NULL, Handle::NULL];
    for t in 0..stride {
        let h = handles[base + t];
        if dup {
            op_dup(h);
        }
        cols[t] = h;
    }
    Entry { cols, len: stride }
}

/// The trie-slot indices of a cursor's descent path — a stack of `u32`, one per focused frame. The
/// depth is HARD-BOUNDED by the trie: at most `CHAMP_LEVELS` (7) normal levels plus one collision
/// frame at the hash floor, so the stack never exceeds `SLOTS_CAP` entries. Storing it inline in a
/// fixed `[u32; SLOTS_CAP]` (instead of a `Vec`) removes the per-advance-step slots allocation on the
/// hot iteration path — `slots.len() == frames.len()`, so the two grow/shrink in lockstep. Overflow is
/// a compiler-invariant violation (a rope deeper than the hash allows) and TRAPS, never silently wraps.
const SLOTS_CAP: usize = CHAMP_LEVELS as usize + 2; // 7 normal levels + collision frame + margin
struct Slots {
    buf: [u32; SLOTS_CAP],
    len: usize,
}
impl Slots {
    fn new() -> Slots {
        Slots {
            buf: [0; SLOTS_CAP],
            len: 0,
        }
    }
    fn len(&self) -> usize {
        self.len
    }
    fn push(&mut self, v: u32) {
        if self.len >= SLOTS_CAP {
            trap_oob(); // cursor deeper than the trie permits — a compiler-invariant violation
        }
        self.buf[self.len] = v;
        self.len += 1;
    }
    fn pop(&mut self) {
        // Mirrors `Vec::pop`'s use here (the return value is never read) — just shrink.
        if self.len > 0 {
            self.len -= 1;
        }
    }
    /// The slot values in push order — for encoding into a cursor's raw header.
    fn as_slice(&self) -> &[u32] {
        &self.buf[..self.len]
    }
}
impl core::ops::Index<usize> for Slots {
    type Output = u32;
    fn index(&self, i: usize) -> &u32 {
        &self.buf[..self.len][i]
    }
}
impl core::ops::IndexMut<usize> for Slots {
    fn index_mut(&mut self, i: usize) -> &mut u32 {
        &mut self.buf[..self.len][i]
    }
}

/// Build a node holding exactly two entries whose keys hash to `h1`/`h2`, splitting by the trie
/// index at `level` and recursing while they collide; at the hash floor they share a collision node.
/// CONSUMES both entries (their handles become the new node's). Bounded recursion (≤7).
#[allow(dead_code)]
fn merge_two_entries(e1: Entry, h1: u32, e2: Entry, h2: u32, level: u32) -> Handle {
    if level >= CHAMP_LEVELS {
        // Hash exhausted: a collision node holding both entries, stored in canonical KEY order
        // (insertion-independent) so equal collision sets are byte-identical regardless of order.
        let (first, second) = if champ_key_cmp(e1.key(), e2.key()) == core::cmp::Ordering::Greater {
            (e2, e1)
        } else {
            (e1, e2)
        };
        let mut hs = Vec::with_capacity(first.len() + second.len());
        first.extend_into(&mut hs);
        second.extend_into(&mut hs);
        return alloc_raw(hs, champ_header(0, 0, 2));
    }
    let i1 = level_index(h1, level);
    let i2 = level_index(h2, level);
    if i1 == i2 {
        // Same slot: nest one subnode a level deeper. Arity-1 — inline the single subnode handle (no
        // transient `vec![sub]` heap Vec that `From<Vec>` would re-inline + free).
        let sub = merge_two_entries(e1, h1, e2, h2, level + 1);
        alloc_raw(Handles::inline_from(&[sub]), champ_header(0, 1 << i1, 2))
    } else if i1 < i2 {
        // Two entries in different slots of a fresh 2-entry node, columns in ascending-slot order.
        // For a SET (stride 1) that is 2 handles = inline-eligible: `merge_entry_pair` builds them
        // INLINE (no transient heap Vec that `From<Vec>` would re-inline + free — this SPLIT fires on
        // every set-insert into an occupied slot). A MAP (stride 2) is 4 handles → heap, as before.
        alloc_raw(
            merge_entry_pair(&e1, &e2),
            champ_header((1 << i1) | (1 << i2), 0, 2),
        )
    } else {
        alloc_raw(
            merge_entry_pair(&e2, &e1),
            champ_header((1 << i2) | (1 << i1), 0, 2),
        )
    }
}

/// Build the `handles` for a fresh 2-entry CHAMP node from two entries in the given order, INLINE when
/// the total column count fits (a SET's two 1-column entries = 2 ≤ INLINE_HANDLES_CAP), else on the
/// heap (a MAP's two 2-column entries = 4). Consumes neither (handles are `Copy`); the caller has
/// already relocated ownership of the entries' columns into the returned node.
fn merge_entry_pair(first: &Entry, second: &Entry) -> Handles {
    let total = first.len() + second.len();
    if total <= INLINE_HANDLES_CAP {
        let mut buf = [Handle::NULL; INLINE_HANDLES_CAP];
        buf[..first.len()].copy_from_slice(first.cols());
        buf[first.len()..total].copy_from_slice(second.cols());
        Handles::Inline {
            buf,
            len: total as u8,
        }
    } else {
        let mut hs = Vec::with_capacity(total);
        hs.extend_from_slice(first.cols());
        hs.extend_from_slice(second.cols());
        Handles::Heap(hs)
    }
}

/// Insert `entry` into a collision node (both bitmaps 0, `handles` nonempty). CONSUMES `node` and
/// `entry`. Overwrite (key present) keeps the stored key + takes incoming value columns, dropping
/// the incoming duplicate key; otherwise the entry is appended. Path-copied. Returns
/// `(new_node, size_delta)` where `size_delta` is 0 (overwrite) or 1 (new key) — so the caller
/// propagates the size change WITHOUT a `champ_size_of` re-read of the child subtree.
#[allow(dead_code)]
fn collision_insert(
    node: Handle,
    handles: Vec<Handle>,
    entry: Entry,
    stride: usize,
) -> (Handle, u32) {
    let key = entry.key();
    let mut found = None;
    let mut idx = 0;
    while idx < handles.len() {
        if champ_eq(handles[idx], key) {
            found = Some(idx);
            break;
        }
        idx += stride;
    }
    match found {
        Some(j) => {
            let mut new_handles = Vec::with_capacity(handles.len());
            for (i2, &h) in handles.iter().enumerate() {
                if i2 > j && i2 < j + stride {
                    new_handles.push(entry.col(i2 - j)); // incoming value column
                } else {
                    op_dup(h);
                    new_handles.push(h);
                }
            }
            let entries = (handles.len() / stride) as u32;
            let new = alloc_raw(new_handles, champ_header(0, 0, entries));
            op_drop(entry.key()); // incoming duplicate key unused
            op_drop(node);
            (new, 0) // overwrite: size unchanged
        }
        None => {
            // New key: splice the incoming entry at the position that keeps entries sorted by KEY
            // (insertion-independent canonical order). Collision nodes are tiny, so a linear scan is
            // fine. Pure reordering of handles we already own — no extra dup/drop.
            let mut pos = handles.len(); // default: after all existing entries
            let mut idx = 0;
            while idx < handles.len() {
                if champ_key_cmp(key, handles[idx]) == core::cmp::Ordering::Less {
                    pos = idx;
                    break;
                }
                idx += stride;
            }
            let mut new_handles = Vec::with_capacity(handles.len() + stride);
            for (i2, &h) in handles.iter().enumerate() {
                if i2 == pos {
                    entry_splice(&mut new_handles, &entry); // owned incoming entry, in place
                }
                op_dup(h);
                new_handles.push(h);
            }
            if pos == handles.len() {
                entry_splice(&mut new_handles, &entry); // sorts last
            }
            let _ = entry; // consumed: its columns were copied into new_handles above (once, by `pos`)
            let entries = (handles.len() / stride + 1) as u32;
            let new = alloc_raw(new_handles, champ_header(0, 0, entries));
            op_drop(node);
            (new, 1) // new key: size + 1
        }
    }
}

/// The per-node recursive insert core. CONSUMES `node` and `entry` (`entry[0]` = key, len = stride);
/// returns `(new_node, size_delta)` where `size_delta` is 0 (an existing key was overwritten) or 1 (a
/// new key was added). Bounded recursion (≤ `CHAMP_LEVELS`). Always path-copies. Returning the delta
/// lets the DESCEND branch set the parent's size header WITHOUT two `champ_size_of` subtree re-reads.
#[allow(dead_code)]
fn champ_insert_node(
    node: Handle,
    entry: Entry,
    hash: u32,
    level: u32,
    stride: usize,
) -> (Handle, u32) {
    let key = entry.key();
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the SPLIT/EMPTY/collision branches that only ever READ it by index and
    // build a fresh, differently-sized result — a wasted Vec alloc + O(arity) copy on every path-copied
    // node. Now the OVERWRITE/DESCEND branches (which REUSE a full-length copy as their result) clone at
    // their own branch, and the growth branches read via a borrow. `arity` gates the empty/collision test.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

    // Empty node (fresh single entry) or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            let i = level_index(hash, level);
            let new = alloc_raw(entry.into_handles(), champ_header(1 << i, 0, 1)); // entry owned (inline handles, no transient Vec)
            op_drop(node);
            return (new, 1); // fresh single entry: a new key
        }
        // Collision node — needs an owned copy of the entries (the helper appends + rebuilds).
        let handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        return collision_insert(node, handles, entry, stride); // returns (node, delta)
    }

    let dcount = data_count(datamap) as usize;
    let scount = subnode_count(nodemap) as usize;
    let subbase = stride * dcount;
    let i = level_index(hash, level);
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        let base = stride * eidx;
        let stored_key = champ_handle_at(node, base);
        if champ_eq(stored_key, key) {
            // OVERWRITE: keep the stored key, swap in the incoming value columns; size/arity unchanged.
            // Clone the handle vector ONCE and use it AS the result (mutate in place) rather than reading
            // one vector and building a second — the new node needs a full-length handles Vec anyway, so
            // the clone IS that vector. Each KEPT handle is dup'd so `new` owns its own reference; the
            // OLD value columns (overwritten below) are released by op_drop(node).
            let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
            for (idx, slot) in new_handles.iter_mut().enumerate() {
                if idx > base && idx < base + stride {
                    *slot = entry.col(idx - base); // incoming value column (owned); old ptr still on node
                } else {
                    op_dup(*slot); // kept handle: new node needs its own reference
                }
            }
            let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size));
            op_drop(entry.key()); // incoming duplicate key unused
            op_drop(node); // frees the old value columns (and balances the dups) if node was unique
            return (new, 0); // overwrite: size unchanged
        }
        // SPLIT: turn the inline entry + newcomer into a subnode. Build the STORED entry (dup'd, since
        // the consumed node's copy also survives via `op_drop(node)` releasing only ITS references).
        // Read the node's handles by BORROW (`champ_handle_at`) — no upfront clone — since the result is
        // a fresh, differently-shaped Vec (one entry removed, one subnode added).
        let stored_entry = with_node(node, Entry::elem(Handle::NULL), |n| {
            stored_entry_from(&n.handles, base, stride, true)
        });
        let h1 = champ_hash(stored_key);
        let sub = merge_two_entries(stored_entry, h1, entry, hash, level + 1);
        let new_datamap = datamap & !bit;
        let new_nodemap = nodemap | bit;
        let new_sidx = subnode_index_for_slot(new_nodemap, i) as usize;
        // Build the fresh result vector directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() + 1);
            for e in 0..dcount {
                if e == eidx {
                    continue;
                }
                for t in 0..stride {
                    let h = n.handles[stride * e + t];
                    op_dup(h);
                    nh.push(h);
                }
            }
            // Splice the subnodes directly into `nh` with `sub` at `new_sidx` — no transient `subs` Vec
            // (dup each carried subnode; `sub` is already owned from `merge_two_entries`).
            for s in 0..new_sidx {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh.push(sub);
            for s in new_sidx..scount {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh
        });
        let new = alloc_raw(
            new_handles,
            champ_header(new_datamap, new_nodemap, size + 1),
        );
        op_drop(node);
        return (new, 1); // split adds the new key
    }

    if nodemap & bit != 0 {
        // DESCEND into the subnode. Arity is unchanged, so the result reuses a full-length copy of the
        // node's handles — clone ONCE here (at the branch that needs it) and mutate the one child slot.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx);
        op_dup(child);
        // The recursion RETURNS the size delta (0 overwrite / 1 new key) directly — no `champ_size_of`
        // re-read of the child subtree before and after (two 12-byte-header node reads per level, a
        // measurable ~5% of insert in profiling).
        let (new_child, delta) = champ_insert_node(child, entry, hash, level + 1, stride);
        // Swap the one child slot to `new_child` (the recursion already consumed the old `child` ref via
        // the op_dup above) and dup each KEPT handle so `new` owns its own references.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        for (idx, slot) in new_handles.iter_mut().enumerate() {
            if idx == subbase + sidx {
                *slot = new_child; // owned; old child ref was consumed by the recursion
            } else {
                op_dup(*slot); // kept handle: new node needs its own reference
            }
        }
        let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size + delta));
        op_drop(node);
        return (new, delta); // propagate the child's delta up
    }

    // EMPTY slot: place a new inline entry in canonical (ascending-bit) order. Fresh, larger result —
    // read the node's handles by BORROW (no upfront clone).
    let new_datamap = datamap | bit;
    let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
    let mut new_handles = with_node(node, Vec::new(), |n| {
        let mut nh: Vec<Handle> = Vec::with_capacity(n.handles.len() + stride);
        for e in 0..dcount {
            for t in 0..stride {
                let h = n.handles[stride * e + t];
                op_dup(h);
                nh.push(h);
            }
        }
        nh
    });
    for (off, h) in entry.cols().iter().enumerate() {
        new_handles.insert(stride * new_eidx + off, *h); // entry columns at the entry position
    }
    let _ = entry; // consumed: columns relocated into new_handles by value (handles are Copy)
    with_node(node, (), |n| {
        for s in 0..scount {
            let c = n.handles[subbase + s];
            op_dup(c);
            new_handles.push(c);
        }
    });
    let new = alloc_raw(new_handles, champ_header(new_datamap, nodemap, size + 1));
    op_drop(node);
    (new, 1) // empty slot filled: a new key
}

// ─── FBIP (Functional But In-Place) rc==1 shell reuse for CHAMP insert/remove (U5) ────────────
// When the touched CHAMP spine is UNIQUELY owned we REUSE each node's shell (mutate its handles/raw
// in place) instead of alloc-new + drop-old. Observationally IDENTICAL to the path-copy core, and
// canonical-shape-identical (the in-place builders mirror the copy path's ordering byte-for-byte).
//
// CRITICAL: ALIASING SAFETY (a violation silently corrupts a shared persistent map/set). `mine` = "this node
// is on a fully-unique path, rc==1, safe to reuse in place". It propagates STRICTLY monotone-false
// downward: the map/set handle IS the root node (no separate header like the vector), so the ops gate
// on `node_rc(m)==1`, then `child_mine = mine && node_rc(child)==1`. At the FIRST node with rc>1 we
// delegate to the proven copy path (`champ_insert_node`/`champ_remove_node`) for that node and its
// whole subtree; the consume-and-return contract means a delegated child consumes exactly the one
// reference passed and returns the handle to place — no pre-dup / post-drop bookkeeping at the boundary
// (the same pattern as `vec_push_fbip`). A subnode with rc==1 reached THROUGH a shared ancestor is
// still shared, which is why `mine` never turns back true once false.

/// In-place shell reuse for a uniquely-owned (`rc == 1`) CHAMP `node`: install `handles` and WRITE the
/// 3-u32 header (`[datamap][nodemap][size]`) into the node's EXISTING `raw` buffer in place, rather
/// than taking a freshly-allocated `champ_header` Vec — saving a 12-byte Vec allocation per rebuilt
/// spine node (the profiler's dominant FBIP-insert/remove cost). Carried children were moved into
/// `handles` WITHOUT a dup (each single owned reference relocates); the caller must have already
/// `op_drop`ed any REMOVED children. Dropping the old `handles` Vec changes no refcount (`Handle` is a
/// `Copy` pointer with no `Drop`), so a carried child in BOTH the old and new Vec keeps its rc intact.
/// A CHAMP node's `raw` is always exactly `CHAMP_HEADER_SIZE`, so the header write never grows/reallocs;
/// a defensively short/absent `raw` is resized to fit. SAFETY: caller verified `node_rc(node) == 1`.
fn champ_become_hdr(
    node: Handle,
    handles: impl Into<Handles>,
    datamap: u32,
    nodemap: u32,
    size: u32,
) -> Handle {
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles = handles.into();
        if n.raw.len() != CHAMP_HEADER_SIZE {
            n.raw.resize(CHAMP_HEADER_SIZE, 0); // defensive; a real CHAMP node is already 12 bytes
        }
        let r = n.raw.as_mut_slice();
        r[0..4].copy_from_slice(&datamap.to_le_bytes());
        r[4..8].copy_from_slice(&nodemap.to_le_bytes());
        r[8..12].copy_from_slice(&size.to_le_bytes());
    }
    node
}

/// `mem::take` a uniquely-owned (`rc == 1`) node's handle vector out (a pointer swap, no clone),
/// leaving it transiently empty. The caller reinstalls a handle vector via `champ_become_hdr` before
/// returning, so no other reference observes the empty state (single-threaded). Moving a `Handle` out
/// touches no refcount (`Copy`, no `Drop`). SAFETY: caller verified `node_rc(node) == 1`.
fn champ_take_handles(node: Handle) -> Handles {
    match unsafe { node.node_mut() } {
        Some(n) => n.handles.take(),
        None => Handles::new(),
    }
}

/// Write a single child slot AND patch the `size` header field of a uniquely-owned (`rc == 1`) CHAMP
/// node IN PLACE — the zero-allocation path for a remove whose subnode kept its arity (only one child
/// pointer changes and the subtree count drops by one; datamap/nodemap are unchanged). SAFETY: caller
/// verified `node_rc(node) == 1` and `slot < handles.len()`, `raw.len() == CHAMP_HEADER_SIZE`.
fn champ_set_child_and_size_inplace(node: Handle, slot: usize, child: Handle, size: u32) {
    if let Some(n) = unsafe { node.node_mut() } {
        if let Some(h) = n.handles.get_mut(slot) {
            *h = child;
        }
        if n.raw.len() >= CHAMP_HEADER_SIZE {
            n.raw.as_mut_slice()[8..12].copy_from_slice(&size.to_le_bytes());
        }
    }
}

/// FBIP variant of `champ_insert_node`. `mine` ⇒ reuse `node`'s shell in place; `!mine` ⇒ delegate to
/// the path-copying `champ_insert_node` verbatim. Bounded recursion (≤ `CHAMP_LEVELS`). CONSUMES
/// `node` and `entry` (`entry[0]` = key, len = `stride`). See the safety note above. Returns
/// `(new_node, size_delta)` — 0 (overwrite) or 1 (new key) — so the DESCEND branch propagates the size
/// change WITHOUT two `champ_size_of` subtree re-reads per level (a measurable insert cost, profiled).
fn champ_insert_fbip(
    node: Handle,
    entry: Entry,
    hash: u32,
    level: u32,
    stride: usize,
    mine: bool,
) -> (Handle, u32) {
    if !mine {
        return champ_insert_node(node, entry, hash, level, stride); // shared: proven copy path
    }
    let key = entry.key();
    // Read the header + arity WITHOUT cloning `handles` (see the take below).
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

    // Empty (fresh single entry) or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            let i = level_index(hash, level);
            return (
                champ_become_hdr(node, entry.into_handles(), 1 << i, 0, 1),
                1,
            ); // fresh: new key (inline handles, no transient Vec)
        }
        // Collision node (full 32-bit hash clash — rare): path-copy via the proven helper, which
        // `op_drop`s `node` and so needs its child references intact — clone rather than take here.
        let handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        return collision_insert(node, handles, entry, stride); // returns (node, delta)
    }

    // Normal (bitmap) node on a UNIQUE spine: TAKE its handle vector instead of cloning it. `node` is
    // `rc == 1` (the `mine` gate + monotone-false descent), so no other reference exists; the take is
    // a pointer swap (zero alloc, vs the clone's O(arity) copy on every spine node, every level). Every
    // path below rebuilds a fresh `new_handles` and `champ_become_hdr(node, …)` REINSTALLS it before this
    // function returns, so `node` is never observed in the transient empty state (single-threaded).
    // `mut` because the arity-preserving branches (OVERWRITE, DESCEND) mutate a slot in place and
    // reinstall this same vector rather than allocating a fresh one.
    let mut handles = match unsafe { node.node_mut() } {
        Some(n) => n.handles.take(),
        None => Handles::new(),
    };

    let dcount = data_count(datamap) as usize;
    let subbase = stride * dcount; // subnodes follow the `dcount` entries; the SPLIT/EMPTY paths now
    let i = level_index(hash, level); // rebuild in place, so `scount` is no longer needed here
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        let base = stride * eidx;
        let stored_key = handles[base];
        if champ_eq(stored_key, key) {
            // OVERWRITE: keep stored key, take the incoming value columns, drop the old ones. Size same,
            // arity unchanged — so mutate the taken `handles` IN PLACE (swap the value columns) and
            // reinstall it, rather than allocating a fresh `new_handles` (saves one Vec per spine node).
            for t in 1..stride {
                op_drop(handles[base + t]); // old value column, replaced
                handles.set(base + t, entry.col(t)); // incoming value column (owned)
            }
            op_drop(entry.key()); // incoming duplicate key unused
            return (champ_become_hdr(node, handles, datamap, nodemap, size), 0); // overwrite
        }
        // SPLIT: fold the inline entry + newcomer into a subnode. MOVE the stored entry (no dup — the
        // node's handle vec was already taken, so these references relocate) into the merge; carry the
        // other entries/subnodes without dup.
        let stored_entry = stored_entry_from(&handles, base, stride, false);
        let h1 = champ_hash(stored_key);
        let sub = merge_two_entries(stored_entry, h1, entry, hash, level + 1);
        let new_datamap = datamap & !bit;
        let new_nodemap = nodemap | bit;
        let new_sidx = subnode_index_for_slot(new_nodemap, i) as usize;
        // Transform the taken `handles` IN PLACE rather than building a fresh `new_handles` + `subs`:
        // (1) DRAIN the split entry's `stride` columns — their handles have already relocated into
        //     `sub` (via `stored_entry_from` + `merge_two_entries`), so DO NOT op_drop them; the drain
        //     just removes the now-duplicate pointers and shifts the tail (remaining entries+subnodes)
        //     left by `stride`. (2) INSERT `sub` at its canonical subnode position: after the drain the
        //     entry region holds `dcount-1` entries, so subnodes start at `stride*(dcount-1)`, and `sub`
        //     goes at `+ new_sidx`. The only allocation left is `sub` itself (intrinsic — a new node).
        handles.drain_range(base, stride);
        handles.insert(stride * (dcount - 1) + new_sidx, sub);
        return (
            champ_become_hdr(node, handles, new_datamap, new_nodemap, size + 1),
            1,
        ); // split: new key
    }

    if nodemap & bit != 0 {
        // DESCEND. Read the child's rc to decide `child_mine`; the recursion consumes the one reference
        // we pass and RETURNS `(new_child, delta)`. Taking the delta from the return replaces the two
        // `champ_size_of(child)` subtree re-reads (before + after) the old code did per level — a
        // measurable ~5% of insert in profiling, for a value the recursion already knows.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = handles[subbase + sidx];
        let child_mine = node_rc(child) == 1;
        let (new_child, delta) =
            champ_insert_fbip(child, entry, hash, level + 1, stride, child_mine);
        // Arity unchanged — swap the one child slot in the taken `handles` IN PLACE and reinstall,
        // rather than rebuilding a fresh Vec (saves one alloc per descended level, the common path).
        // The recursion CONSUMED `child` (the reference at this slot); writing `new_child` here is a
        // no-op when it reused the shell (`new_child == child`) and installs the fresh node otherwise.
        handles.set(subbase + sidx, new_child);
        return (
            champ_become_hdr(node, handles, datamap, nodemap, size + delta),
            delta,
        );
    }

    // EMPTY slot: place a new inline entry in canonical (ascending-bit) order. The entry region sits
    // BEFORE the subnodes (`subbase = stride*dcount`) and `new_eidx ≤ dcount`, so splicing the entry's
    // `stride` columns into the taken `handles` at `stride*new_eidx` lands them among the entries and
    // shifts the subnodes right — exactly the canonical layout. Reuse the taken vector (rc==1) rather
    // than building a fresh one: `Vec::insert` may grow it once, but there is no separate full-copy pass.
    let new_datamap = datamap | bit;
    let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
    for (off, h) in entry.cols().iter().enumerate() {
        handles.insert(stride * new_eidx + off, *h); // incoming entry column (owned), no dup
    }
    let _ = entry; // consumed: columns relocated into `handles` by value (handles are Copy)
    (
        champ_become_hdr(node, handles, new_datamap, nodemap, size + 1),
        1,
    ) // empty slot filled: new key
}

/// Insert `key => val`, returning the new map. CONSUMES `m`, `key`, `val`. Inserting an existing key
/// overwrites its value (size unchanged); a new key increments size. Persistent: to keep the old
/// map, `op_dup` it before inserting. FBIP: when `m` is uniquely owned (`rc == 1`) the touched spine
/// is refit in place; a shared map (`rc > 1`) path-copies (the old version stays byte-identical).
#[allow(dead_code)]
fn op_map_insert(m: Handle, key: Handle, val: Handle) -> Handle {
    let hash = champ_hash(key);
    let mine = node_rc(m) == 1;
    champ_insert_fbip(m, Entry::kv(key, val), hash, 0, MAP_STRIDE, mine).0 // discard the size delta
}

// ─── CHAMP persistent MAP: remove (the exact inverse of insert) ─────────────────────────
// Remove is the mirror of insert and MUST preserve canonicality so two maps with equal contents
// stay byte-identically shaped (equal `champ_eq`/`champ_hash`) regardless of insert/remove history.
// The load-bearing inverses of the split:
//   • Removing an inline entry clears its datamap bit and drops `[k,v]`.
//   • A subnode that reduces to a SINGLE inline entry (and no subnodes) is COLLAPSED back into the
//     parent as an inline entry (inverse of `merge_two_entries`). A single-entry collision node
//     collapses the same way. This cascades: after inlining a child, the parent may itself become a
//     single-entry collapse candidate for ITS parent — handled naturally by each level re-checking
//     its child result on the bounded post-order return path (≤ `CHAMP_LEVELS` frames, stack-safe).
//   • Removing the map's final entry yields EXACTLY the canonical empty (`op_map_empty()` shape).
// A subnode is only ever collapsed when it holds a single ENTRY — a subnode of ≥2 entries can't be
// inlined and stays put, matching what insert would have produced. Like insert, a uniquely-owned
// spine refits in place (`champ_remove_fbip`'s `mine` gate); a shared spine path-copies.
//
// NOTE: the ROOT is never collapsed to an inline entry — a one-entry map is a root node with one
// inline entry (datamap has one bit), which is exactly what insert produces. Collapse only lifts a
// child INTO its parent; the top-level `op_map_remove` never inlines the root into anything.

/// If `node` holds exactly one entry and no subnodes (a single inline entry, or a single-entry
/// collision node), return that entry's `stride` handles (BORROWED — no rc change) so the parent can
/// inline it. Otherwise `None`. Root nodes are never passed here.
#[allow(dead_code)]
fn collapse_candidate(node: Handle, stride: usize) -> Option<Entry> {
    with_node(node, None, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        if nm != 0 {
            return None; // has subnodes — cannot inline
        }
        let entries = if dm != 0 {
            data_count(dm) as usize
        } else {
            n.handles.len() / stride // collision (or empty) node
        };
        if entries == 1 && n.handles.len() >= stride {
            // The single entry's `stride` columns as an inline `Entry` (≤2 handles) — no heap Vec.
            let mut cols = [Handle::NULL, Handle::NULL];
            cols[..stride].copy_from_slice(&n.handles[0..stride]);
            Some(Entry { cols, len: stride })
        } else {
            None
        }
    })
}

/// The per-node recursive remove core. CONSUMES `node` (the input ref moves through to the returned
/// handle), BORROWS `key`. Returns `(new_node, removed)`; when `removed` is false the returned handle
/// is the unchanged input. Bounded recursion (≤ `CHAMP_LEVELS`). Always path-copies.
#[allow(dead_code)]
fn champ_remove_node(
    node: Handle,
    key: Handle,
    hash: u32,
    level: u32,
    stride: usize,
) -> (Handle, bool) {
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the common ABSENT-key early-returns (`(node, false)`) and on the
    // fresh-shorter-result branches (found-entry drop, collapse) that only READ it by index — a wasted
    // Vec alloc + O(arity) copy on every path-copied node. Branches that reuse a full-length copy as the
    // result (DESCEND non-collapse) clone at their own branch; the rest borrow-and-build / return early.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

    // Empty node or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            return (node, false); // empty — absent
        }
        // Collision node: linear scan of entries (BORROW — no upfront clone; a miss returns unchanged).
        let found = with_node(node, None, |n| {
            let mut idx = 0;
            while idx < n.handles.len() {
                if champ_eq(n.handles[idx], key) {
                    return Some(idx);
                }
                idx += stride;
            }
            None
        });
        let j = match found {
            Some(j) => j,
            None => return (node, false), // absent — unchanged
        };
        let entries_after = (arity / stride - 1) as u32;
        // Fresh shorter result — build directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() - stride);
            for (i2, &h) in n.handles.iter().enumerate() {
                if i2 >= j && i2 < j + stride {
                    continue; // removed entry columns: NOT dup'd, freed by op_drop(node) below
                }
                op_dup(h);
                nh.push(h);
            }
            nh
        });
        let new = alloc_raw(new_handles, champ_header(0, 0, entries_after));
        op_drop(node);
        return (new, true);
    }

    let dcount = data_count(datamap) as usize;
    let scount = subnode_count(nodemap) as usize;
    let subbase = stride * dcount;
    let i = level_index(hash, level);
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        // Inline entry at this slot: present only if the stored key equals `key`.
        let eidx = entry_index_for_slot(datamap, i) as usize;
        if !champ_eq(champ_handle_at(node, stride * eidx), key) {
            return (node, false); // different key occupies the slot — absent
        }
        let new_datamap = datamap & !bit;
        // Fresh shorter result (one entry dropped) — build directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() - stride);
            for e in 0..dcount {
                if e == eidx {
                    continue; // removed entry columns: not dup'd, freed by op_drop(node)
                }
                for t in 0..stride {
                    let h = n.handles[stride * e + t];
                    op_dup(h);
                    nh.push(h);
                }
            }
            for s in 0..scount {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh
        });
        let new = alloc_raw(new_handles, champ_header(new_datamap, nodemap, size - 1));
        op_drop(node);
        return (new, true);
    }

    if nodemap & bit != 0 {
        // Descend into the subnode.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx);
        op_dup(child);
        let (new_child, removed) = champ_remove_node(child, key, hash, level + 1, stride);
        if !removed {
            op_drop(new_child); // unchanged (== child): undo the dup
            return (node, false);
        }
        if let Some(centry) = collapse_candidate(new_child, stride) {
            // COLLAPSE: inline the child's single entry into this node at slot i. Fresh reshaped result
            // (one subnode → one entry) — build directly from a borrow of the node's handles.
            for &h in centry.cols() {
                op_dup(h);
            }
            op_drop(new_child); // frees the collapsed child wrapper; entry cols survive via our dups
            let new_datamap = datamap | bit;
            let new_nodemap = nodemap & !bit;
            let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
            let mut new_handles = with_node(node, Vec::new(), |n| {
                let mut nh = Vec::with_capacity(n.handles.len());
                for e in 0..dcount {
                    for t in 0..stride {
                        let h = n.handles[stride * e + t];
                        op_dup(h);
                        nh.push(h);
                    }
                }
                nh
            });
            for (off, &h) in centry.cols().iter().enumerate() {
                new_handles.insert(stride * new_eidx + off, h); // entry cols at the entry position
            }
            with_node(node, (), |n| {
                for s in 0..scount {
                    if s == sidx {
                        continue; // this subnode was collapsed away
                    }
                    let c = n.handles[subbase + s];
                    op_dup(c);
                    new_handles.push(c);
                }
            });
            let new = alloc_raw(
                new_handles,
                champ_header(new_datamap, new_nodemap, size - 1),
            );
            op_drop(node);
            return (new, true);
        }
        // Subnode still holds ≥2 entries: keep it, just swap in the rebuilt child. Arity unchanged, so
        // CLONE the handle vector ONCE and use it AS the result — mutate the one child slot, dup the
        // rest — rather than reading one vector and building a second.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        for (idx, slot) in new_handles.iter_mut().enumerate() {
            if idx == subbase + sidx {
                *slot = new_child; // owned; old child ref was consumed by the recursion
            } else {
                op_dup(*slot); // kept handle: new node needs its own reference
            }
        }
        let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size - 1));
        op_drop(node);
        return (new, true);
    }

    (node, false) // empty slot — absent
}

/// FBIP variant of `champ_remove_node`. `mine` ⇒ reuse `node`'s shell in place; `!mine` ⇒ delegate to
/// the path-copying `champ_remove_node` verbatim. CONSUMES `node`, BORROWS `key`. Returns
/// `(new_node, removed)`; when `removed` is false the returned handle is the unchanged input. Preserves
/// canonical shape (collapse cascade, remove-to-canonical-empty) exactly as the copy path. Bounded
/// recursion (≤ `CHAMP_LEVELS`).
fn champ_remove_fbip(
    node: Handle,
    key: Handle,
    hash: u32,
    level: u32,
    stride: usize,
    mine: bool,
) -> (Handle, bool) {
    if !mine {
        return champ_remove_node(node, key, hash, level, stride); // shared: proven copy path
    }
    // ALLOCATION-LAZY: read the header + INDIVIDUAL slots (borrows, no Vec) to decide absent-vs-present,
    // and only `mem::take` the handle vector in the branches that actually rebuild a node. Every ABSENT
    // path and the common non-collapse DESCEND then allocate NOTHING (the old code cloned the whole
    // handle vector at every level up front). `node` is rc==1 (caller-gated), so an in-place slot write
    // and a deferred take are both safe — no other reference observes the node.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

    // Empty node or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            return (node, false); // empty — absent
        }
        // Collision node: scan entry keys by BORROW (champ_handle_at), no clone, to find a match.
        let mut found = None;
        let mut idx = 0;
        while idx < arity {
            if champ_eq(champ_handle_at(node, idx), key) {
                found = Some(idx);
                break;
            }
            idx += stride;
        }
        let j = match found {
            Some(j) => j,
            None => return (node, false), // absent — unchanged, zero alloc
        };
        // Present: drain the removed entry's `stride` columns from the taken vector IN PLACE (reusing
        // its allocation), shifting the remaining collision entries left — collision entries stay in
        // canonical KEY order, so the drain preserves it.
        let mut handles = champ_take_handles(node);
        let entries_after = (arity / stride - 1) as u32;
        for t in 0..stride {
            op_drop(handles[j + t]); // removed entry columns: release the node's reference
        }
        handles.drain_range(j, stride); // shift the remaining collision entries left, in place
        return (champ_become_hdr(node, handles, 0, 0, entries_after), true);
    }

    let dcount = data_count(datamap) as usize;
    let subbase = stride * dcount; // subnodes follow the `dcount` entries; the datamap-found and
    let i = level_index(hash, level); // COLLAPSE paths now rebuild in place, so `scount` isn't needed
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        if !champ_eq(champ_handle_at(node, stride * eidx), key) {
            return (node, false); // different key occupies the slot — absent, zero alloc
        }
        // Present: remove the entry's `stride` columns from the taken vector IN PLACE (drain the range),
        // reusing its allocation rather than building a fresh `new_handles`. The removed columns sit at
        // `[base .. base+stride)` within the entry region (which precedes the subnodes), so draining them
        // shifts the remaining entries + all subnodes left by `stride` — exactly the canonical layout.
        let mut handles = champ_take_handles(node);
        let new_datamap = datamap & !bit;
        let base = stride * eidx;
        for t in 0..stride {
            op_drop(handles[base + t]); // removed entry columns: release the node's references
        }
        handles.drain_range(base, stride); // shift the remaining entries + subnodes left, in place
        return (
            champ_become_hdr(node, handles, new_datamap, nodemap, size - 1),
            true,
        );
    }

    if nodemap & bit != 0 {
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx); // borrow the child slot, no clone
        let child_mine = node_rc(child) == 1;
        let (new_child, removed) =
            champ_remove_fbip(child, key, hash, level + 1, stride, child_mine);
        if !removed {
            // Unchanged: `new_child == child` (in-place path) or an untouched shared handle (copy path
            // returns the input on absent). Either way the node's reference is intact; nothing to undo.
            return (node, false); // zero alloc
        }
        if let Some(centry) = collapse_candidate(new_child, stride) {
            // COLLAPSE: inline the child's single entry into this node at slot i. Dup the entry cols
            // (they must survive the child wrapper's drop), then free the now-empty child wrapper.
            for &h in centry.cols() {
                op_dup(h);
            }
            op_drop(new_child); // frees the collapsed child; entry cols survive via our dups
            // Transform the taken `handles` IN PLACE rather than rebuilding: the collapsed subnode's ONE
            // handle leaves (at `subbase + sidx`) and the entry's `stride` columns enter the entry region
            // (at `stride * new_eidx`). Both regions are contiguous and the entry region precedes the
            // subnodes, so: (1) remove the subnode handle, then (2) splice the entry columns at their
            // slot — the drain shifts the surviving subnodes, and the splice shifts them again to sit
            // after the now-larger entry region. Reuses the vec; only the `centry` dups above allocate
            // nothing (Entry is inline). Net arity change is `stride - 1`.
            let mut handles = champ_take_handles(node);
            let new_datamap = datamap | bit;
            let new_nodemap = nodemap & !bit;
            let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
            handles.drain_range(subbase + sidx, 1); // the collapsed subnode handle leaves (already dropped)
            for (off, &h) in centry.cols().iter().enumerate() {
                handles.insert(stride * new_eidx + off, h); // inlined entry columns (dup'd above)
            }
            return (
                champ_become_hdr(node, handles, new_datamap, new_nodemap, size - 1),
                true,
            );
        }
        // Subnode still holds ≥2 entries: keep it, swap in the rebuilt child (== child if in-place).
        // Arity unchanged, so DON'T take/rebuild — write the one child slot and patch the size field
        // directly into `node`'s live storage (rc==1). The recursion consumed the old child reference
        // at this slot; overwriting it with `new_child` is a no-op when the child was refit in place
        // (`new_child == child`) and installs the fresh node otherwise. ZERO allocation.
        champ_set_child_and_size_inplace(node, subbase + sidx, new_child, size - 1);
        return (node, true);
    }

    (node, false) // empty slot — absent
}

/// Remove `key`, returning the new map (the canonical empty if the last entry is removed).
/// CONSUMES `m` (moves through to the result), BORROWS `key`. Removing an absent key returns `m`
/// unchanged in value with no alloc, no leak, and no double-free. FBIP: a uniquely-owned map
/// (`rc == 1`) refits the touched spine in place; a shared map path-copies (old version byte-identical).
#[allow(dead_code)]
fn op_map_remove(m: Handle, key: Handle) -> Handle {
    let hash = champ_hash(key);
    let mine = node_rc(m) == 1;
    let (new, _removed) = champ_remove_fbip(m, key, hash, 0, MAP_STRIDE, mine);
    new
}

// ─── CHAMP cursor + in-order iteration (shared by map AND set) ───────────────────────────
// A CURSOR is itself a Node — so `op_drop` reclaims it with no special-casing — but it is an
// EPHEMERAL LINEAR value NEVER passed to hash/eq/lookup/insert/remove. Its layout is deliberately
// distinct from a map node:
//   handles = the descent-path frames, root→deepest: `[frame0=root, frame1, …, frameDeepest]`, each
//             an owned (dup'd) reference so the whole focused path stays live for the walk. Bounded
//             to ≤ CHAMP_LEVELS frames + at most one collision frame.
//   raw     = `[state:u32][slot0:u32][slot1:u32]…[slotN:u32]` (one slot per frame, so
//             `slots.len() == handles.len()`). `state` is CURSOR_LIVE or CURSOR_EXHAUSTED. For a
//             NORMAL frame, `slot_i` is the trie-SLOT index (0..31) the walk is at; for a COLLISION
//             frame (the deepest, when present) `slot_i` is the 0-based ENTRY INDEX within it.
//
// In-order walk (the canonical order proved in U3): at each node, visit trie slots 0..31 ASCENDING;
// a datamap bit ⇒ emit the inline entry at that slot; a nodemap bit ⇒ descend into that subnode and
// finish its whole 0..31 walk BEFORE continuing the parent's higher slots. Collision-node entries
// are emitted in stored order. Because a bit is in exactly one bitmap, the order is unambiguous and
// identical for equal maps. The walk is naturally bounded by depth (no unbounded recursion).
//
// STRIDE-PARAMETERIZED so the set (stride 1) reuses these verbatim: `champ_descend_leftmost`,
// `champ_advance`, and `champ_cursor_current` take `stride` (2 for map k/v, 1 for set elems).
// A uniquely-owned cursor (`node_rc == 1`) advances IN PLACE via `champ_cursor_next_fbip`; a shared
// cursor path-copies a fresh independent one (never writing through the consumed cursor's frames/raw),
// so forked cursors stay independent by construction.

const CURSOR_LIVE: u32 = 0;
const CURSOR_EXHAUSTED: u32 = 1;
/// Handles-per-entry: map stores `[k,v]` (2), set stores `[e]` (1).
const MAP_STRIDE: usize = 2;
/// A set is CHAMP minus the value column — a PRIMITIVE collection (not `Map<T,Unit>`), stride 1.
const SET_STRIDE: usize = 1;

/// The `i`-th subnode of a node under the given entry stride, or NULL (benign).
#[allow(dead_code)]
fn champ_subnode_at(node: Handle, slot: u32, stride: usize) -> Handle {
    with_node(node, Handle::NULL, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        let base = stride * data_count(dm) as usize;
        let sidx = subnode_index_for_slot(nm, slot) as usize;
        n.handles.get(base + sidx).copied().unwrap_or(Handle::NULL)
    })
}

/// A single handle from a node's `handles`, or NULL (benign).
#[allow(dead_code)]
fn champ_handle_at(node: Handle, idx: usize) -> Handle {
    with_node(node, Handle::NULL, |n| {
        n.handles.get(idx).copied().unwrap_or(Handle::NULL)
    })
}

/// From `node`, descend to the LEFTMOST (in-order first) entry, appending a `(node, slot)` frame at
/// each level. `frames`/`slots` receive BORROWED node pointers (the caller dups them for ownership).
/// `node` MUST be non-empty (callers exclude the empty root); subnodes are ≥2 entries by invariant,
/// so this always terminates at an inline entry or a collision frame.
#[allow(dead_code)]
fn champ_descend_leftmost(
    node: Handle,
    frames: &mut Vec<Handle>,
    slots: &mut Slots,
    stride: usize,
) {
    let mut cur = node;
    loop {
        let (dm, nm, is_coll) = with_node(cur, (0u32, 0u32, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            (dm, nm, dm == 0 && nm == 0 && !n.handles.is_empty())
        });
        if is_coll {
            frames.push(cur);
            slots.push(0); // entry index 0
            return;
        }
        let combined = dm | nm;
        if combined == 0 {
            // Defensive: an empty node (should not occur below the root). Record it as a leaf.
            frames.push(cur);
            slots.push(0);
            return;
        }
        let j = combined.trailing_zeros();
        frames.push(cur);
        slots.push(j);
        if dm & (1 << j) != 0 {
            return; // inline entry — deepest frame reached
        }
        cur = champ_subnode_at(cur, j, stride); // nodemap bit — descend
    }
}

/// Advance the cursor's OWN `(frames, slots)` in place to the in-order successor, applying the frame
/// refcount delta INLINE (`op_drop` each popped frame, `op_dup` each newly-descended frame; the kept
/// prefix is untouched) — the FBIP twin of `champ_advance` for a uniquely-owned cursor. This lets the
/// caller advance the taken frames directly, with NO `frames.clone()` and NO post-hoc common-prefix
/// diff (the whole point: kill the last per-step Vec allocation in `champ_cursor_next_fbip`). The net
/// delta is IDENTICAL to `champ_advance` + the external diff — a kept frame: 0; a popped frame: −1; a
/// descended frame: +1. Returns true on a new entry, false when exhausted (frames/slots emptied, every
/// remaining frame dropped). SAFETY: caller verified the cursor is `rc == 1` and owns these frames.
fn champ_advance_fbip(frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) -> bool {
    loop {
        let depth = frames.len();
        if depth == 0 {
            return false; // exhausted — all frames already dropped as they were popped
        }
        let node = frames[depth - 1];
        let cur = slots[depth - 1];
        let (dm, nm, ecount, is_coll) = with_node(node, (0u32, 0u32, 0usize, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            let is_coll = dm == 0 && nm == 0 && !n.handles.is_empty();
            (dm, nm, n.handles.len() / stride, is_coll)
        });
        if is_coll {
            if (cur as usize) + 1 < ecount {
                slots[depth - 1] = cur + 1; // next collision entry — same frame, no ref change
                return true;
            }
            op_drop(frames.pop().unwrap()); // exhausted this collision node — release its ref, resume at parent
            slots.pop();
            continue;
        }
        let combined = dm | nm;
        let above = if cur >= 31 {
            0
        } else {
            combined & !((1u32 << (cur + 1)) - 1)
        };
        if above == 0 {
            op_drop(frames.pop().unwrap()); // nothing left here — release this frame, resume at parent
            slots.pop();
            continue;
        }
        let j = above.trailing_zeros();
        slots[depth - 1] = j; // move within the SAME (kept) frame — no ref change
        if dm & (1 << j) != 0 {
            return true; // inline entry here — deepest
        }
        // nodemap bit: descend into the subnode and take its leftmost, DUP'ing each new frame.
        let child = champ_subnode_at(node, j, stride);
        champ_descend_leftmost_dup(child, frames, slots, stride);
        return true;
    }
}

/// `champ_descend_leftmost` but `op_dup`s each frame it pushes — the cursor takes ownership of every
/// newly-focused node on the descent. Used only by `champ_advance_fbip` (the inline-refcount advance).
fn champ_descend_leftmost_dup(
    node: Handle,
    frames: &mut Vec<Handle>,
    slots: &mut Slots,
    stride: usize,
) {
    let start = frames.len();
    champ_descend_leftmost(node, frames, slots, stride);
    for &f in &frames[start..] {
        op_dup(f); // the cursor now owns a reference to each descended frame
    }
}

/// Advance a working `(frames, slots)` walk state to the in-order successor. Returns true when it
/// lands on a new entry (state stays live), false when the walk is exhausted (frames/slots emptied).
/// Operates on the caller's COPIES (pointer values), so it never mutates any node. Bounded by depth.
#[allow(dead_code)]
fn champ_advance(frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) -> bool {
    loop {
        let depth = frames.len();
        if depth == 0 {
            return false; // exhausted
        }
        let node = frames[depth - 1];
        let cur = slots[depth - 1];
        let (dm, nm, ecount, is_coll) = with_node(node, (0u32, 0u32, 0usize, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            let is_coll = dm == 0 && nm == 0 && !n.handles.is_empty();
            (dm, nm, n.handles.len() / stride, is_coll)
        });
        if is_coll {
            if (cur as usize) + 1 < ecount {
                slots[depth - 1] = cur + 1; // next collision entry
                return true;
            }
            frames.pop();
            slots.pop();
            continue; // exhausted this collision node — resume at parent
        }
        // Normal node: lowest set bit of (datamap|nodemap) strictly above `cur`.
        let combined = dm | nm;
        let above = if cur >= 31 {
            0
        } else {
            combined & !((1u32 << (cur + 1)) - 1)
        };
        if above == 0 {
            frames.pop();
            slots.pop();
            continue; // nothing left here — resume at parent
        }
        let j = above.trailing_zeros();
        slots[depth - 1] = j;
        if dm & (1 << j) != 0 {
            return true; // inline entry here — deepest
        }
        // nodemap bit: descend into the subnode and take its leftmost.
        let child = champ_subnode_at(node, j, stride);
        champ_descend_leftmost(child, frames, slots, stride);
        return true;
    }
}

/// Build a cursor node owning the (already-dup'd) `frames` and encoding `slots` + `state` in raw.
#[allow(dead_code)]
fn champ_make_cursor(frames: Vec<Handle>, slots: Slots, state: u32) -> Handle {
    let mut raw = Vec::with_capacity(4 * (1 + slots.len()));
    raw.extend_from_slice(&state.to_le_bytes());
    for s in slots.as_slice() {
        raw.extend_from_slice(&s.to_le_bytes());
    }
    // Keep the frame stack on the HEAP arm (see `from_vec_heap`): cursor frames are push/popped as a Vec
    // by `champ_advance_fbip` and moved out by `champ_cursor_take`; inlining a shallow cursor would force
    // a Vec re-materialize every advance step (regresses iterate).
    alloc_raw(Handles::from_vec_heap(frames), Raw::from(raw))
}

/// Read a cursor into `(state, frames, slots)`. `frames` are BORROWED pointer copies (owned by the
/// cursor); `slots.len() == frames.len()`.
#[allow(dead_code)]
fn champ_cursor_read(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    with_node(cur, (CURSOR_EXHAUSTED, Vec::new(), Slots::new()), |n| {
        let state = read_u32_at(&n.raw, 0);
        let frames = n.handles.to_vec();
        let mut slots = Slots::new();
        for i in 0..frames.len() {
            slots.push(read_u32_at(&n.raw, 4 + 4 * i));
        }
        (state, frames, slots)
    })
}

/// Like `champ_cursor_read` but MOVES the frame handles out of the cursor (`mem::take`) instead of
/// cloning them — the alloc-free read for the FBIP advance, whose caller has verified the cursor is
/// UNIQUELY owned (`rc == 1`), so no other reference observes the transient empty `handles`. The
/// returned `frames` carry the cursor's frame references verbatim (a `Vec<Handle>` move touches no
/// refcount — `Handle` is `Copy`); the caller MUST reinstall a frame vector via `champ_become_cursor`
/// before returning (every path does). SAFETY: caller verified `node_rc(cur) == 1`.
fn champ_cursor_take(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    match unsafe { cur.node_mut() } {
        Some(n) => {
            let state = read_u32_at(&n.raw, 0);
            let frames = n.handles.take().into_vec(); // cursor frames need Vec semantics (heap arm)
            let mut slots = Slots::new();
            for i in 0..frames.len() {
                slots.push(read_u32_at(&n.raw, 4 + 4 * i));
            }
            (state, frames, slots)
        }
        None => (CURSOR_EXHAUSTED, Vec::new(), Slots::new()),
    }
}

/// The current entry's `(deepest_node, base_index_in_handles)` under `stride`, or None if exhausted.
/// The projection primitive SHARED by map (key = base, val = base+1) and set (elem = base). BORROWS,
/// and allocates NOTHING: it needs only the DEEPEST frame + its slot, so it reads the last handle and
/// the last raw u32 straight out of the cursor node — not `champ_cursor_read`, which would clone the
/// whole frames Vec and build the whole slots Vec just to index `[depth-1]` (this is called on EVERY
/// key/val/elem projection, so that was ~2 wasted Vec allocs per iteration step).
#[allow(dead_code)]
fn champ_cursor_current(cur: Handle, stride: usize) -> Option<(Handle, usize)> {
    // Read state, the deepest frame (last handle), and the deepest slot (last raw u32) in one borrow.
    let (node, slot) = with_node(cur, (Handle::NULL, None::<u32>), |n| {
        let state = read_u32_at(&n.raw, 0);
        if state != CURSOR_LIVE || n.handles.is_empty() {
            return (Handle::NULL, None);
        }
        let depth = n.handles.len();
        let node = n.handles[depth - 1];
        // slots live at raw[4 + 4*i]; the deepest is at i = depth-1. `slots.len() == frames.len()`.
        let slot = read_u32_at(&n.raw, 4 + 4 * (depth - 1));
        (node, Some(slot))
    });
    let slot = slot?;
    let base = with_node(node, 0usize, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        if dm == 0 && nm == 0 {
            stride * slot as usize // collision frame: slot IS the entry index
        } else {
            stride * entry_index_for_slot(dm, slot) as usize // normal frame
        }
    });
    Some((node, base))
}

/// A cursor over `m` positioned at the first entry in walk order (or exhausted if `m` is empty).
/// BORROWS `m` (dups the frames it captures; the cursor owns those refs).
#[allow(dead_code)]
fn op_map_iter(m: Handle) -> Handle {
    if is_empty_node(m) {
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut frames = Vec::new();
    let mut slots = Slots::new();
    champ_descend_leftmost(m, &mut frames, &mut slots, MAP_STRIDE);
    for &f in &frames {
        op_dup(f); // the cursor owns a reference to every focused node
    }
    champ_make_cursor(frames, slots, CURSOR_LIVE)
}

// ─── FBIP (Functional But In-Place) rc==1 cursor advance for map-iter-next / set-iter-next (U6) ──
// A cursor is a Node: `handles` = the descent-path frames (root→deepest, each a dup'd owned ref),
// `raw` = `[state:u32][slot0][slot1]…`. The copy path (`op_*_iter_next` below, kept as the rc>1
// fallback) clones the walk state, advances on the COPY, dups every resulting frame for a fresh
// cursor, then drops the consumed cursor. FBIP reuses the cursor SHELL in place when it is uniquely
// owned — the WIT's zero-steady-state-alloc promise for a non-forked walk.
//
// CRITICAL: ALIASING SAFETY: gate on `node_rc(cur) == 1`. A forked/peeked/teed cursor (rc>1) MUST take the
// copy path so the other owner's walk is undisturbed. The cursor is a leaf handle (no descent into
// it), so the check is a single rc read — no downward propagation like the trie ops.
//
// FRAME RC DISCIPLINE (the leak/double-free trap): `champ_advance` pops exhausted frames and pushes
// newly-descended ones on the COPY, touching NO refcounts. Old and new frame lists share the longest
// common PREFIX (identical node pointers — a CHAMP walk is a root→leaf path, and advance only trims a
// suffix then may descend a new suffter). So the exact ref delta is: retain the shared prefix
// (untouched), `op_drop` each POPPED old-tail frame, `op_dup` each newly-DESCENDED new-tail frame.
// This nets IDENTICALLY to the copy path (which dups all new frames then drops all old frames): a
// common-prefix frame there is +1 (dup) −1 (drop) = 0, a new frame +1, a popped frame −1 — same.

/// Refit an rc==1 cursor's shell in place: `handles` become `frames`, `raw` becomes `[state]slots…`.
/// No new allocation, no self-drop. Frame REFCOUNTS are the caller's responsibility (it applies the
/// dup/drop delta before calling); overwriting the old `handles` Vec changes no refcounts (`Handle` is
/// `Copy`, no `Drop`). SAFETY: caller verified `node_rc(cur) == 1`.
fn champ_become_cursor(cur: Handle, frames: Vec<Handle>, slots: Slots, state: u32) -> Handle {
    if let Some(n) = unsafe { cur.node_mut() } {
        // Reuse the cursor's EXISTING `raw` allocation (clear keeps its capacity) instead of allocating
        // a fresh Vec — the cursor is rc==1, and its raw already held a `[state]slots…` of comparable
        // size, so the re-extend rarely reallocates. Saves one Vec allocation per advance step.
        //
        // Pack `[state][slot0][slot1]…` into a single STACK buffer and write it with ONE
        // `extend_from_slice`, rather than a per-slot `extend_from_slice` loop (up to 9 calls, each
        // re-checking inline/heap capacity). This advance runs once PER WALKED ELEMENT and showed up as
        // `champ_become_cursor` + `Raw::extend_from_slice` ~5% of the set-algebra profile.
        let sl = slots.as_slice();
        let mut buf = [0u8; 4 * (SLOTS_CAP + 1)]; // state + up to SLOTS_CAP slots; never overflows
        buf[0..4].copy_from_slice(&state.to_le_bytes());
        for (i, &s) in sl.iter().enumerate() {
            buf[4 + 4 * i..8 + 4 * i].copy_from_slice(&s.to_le_bytes());
        }
        let total = 4 * (sl.len() + 1);
        n.raw.clear();
        n.raw.extend_from_slice(&buf[..total]);
        n.handles = Handles::from_vec_heap(frames); // keep the frame stack on the heap arm (see above)
    }
    cur
}

/// FBIP in-place advance of a UNIQUELY-OWNED (`rc == 1`) cursor. Reuses `champ_advance` verbatim for
/// the traversal (identical order + exhausted-signal to the copy path), then applies ONLY the frame-ref
/// delta and refits `cur`'s shell in place. Returns `cur`. Stride selects map (2) vs set (1).
fn champ_cursor_next_fbip(cur: Handle, stride: usize) -> Handle {
    // `cur` is rc==1 (caller-gated) ⇒ MOVE its frames/slots out (no clone) and advance them IN PLACE.
    // `champ_advance_fbip` applies the frame refcount delta inline (drop popped, dup descended, keep
    // the prefix), so there is NO `frames.clone()` and NO post-hoc common-prefix diff — the last
    // per-step Vec allocation is gone. We reinstall via `champ_become_cursor` on every return path
    // before any other reference could observe `cur` (single-threaded, rc==1).
    let (state, mut frames, mut slots) = champ_cursor_take(cur);
    if state != CURSOR_LIVE {
        // Already exhausted: release any frames it held (normally none) and stay exhausted in place.
        for &f in &frames {
            op_drop(f);
        }
        return champ_become_cursor(cur, Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let live = champ_advance_fbip(&mut frames, &mut slots, stride);
    // On exhaustion `champ_advance_fbip` has already dropped every frame it popped (frames now empty);
    // on a live step it has applied the exact drop/dup delta. Either way `(frames, slots)` is the new
    // cursor state with correct refcounts — just reinstall it.
    let new_state = if live { CURSOR_LIVE } else { CURSOR_EXHAUSTED };
    champ_become_cursor(cur, frames, slots, new_state)
}

/// Advance to the in-order successor. CONSUMES `cur`, returns the advanced cursor. FBIP: a uniquely
/// owned cursor (`rc == 1`) is refit in place (zero steady-state alloc); a forked/shared cursor
/// (`rc > 1`) path-copies into a fresh cursor so the other owner's walk is undisturbed. At the last
/// entry the cursor becomes exhausted. Order + exhausted-signal are identical on both paths.
#[allow(dead_code)]
fn op_map_iter_next(cur: Handle) -> Handle {
    if node_rc(cur) == 1 {
        return champ_cursor_next_fbip(cur, MAP_STRIDE);
    }
    // Shared (rc>1): copy path — build a fresh independent cursor, then release just THIS reference.
    // `champ_cursor_read` CLONED the frames (the shared cursor keeps its own), so `frames`/`slots`
    // here are throwaway locals — MOVE them into the working `wf`/`ws` rather than cloning again.
    let (state, frames, slots) = champ_cursor_read(cur);
    if state != CURSOR_LIVE {
        op_drop(cur);
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut wf = frames;
    let mut ws = slots;
    let live = champ_advance(&mut wf, &mut ws, MAP_STRIDE);
    let new = if live {
        for &f in &wf {
            op_dup(f); // new cursor's OWN refs (independent of the consumed cursor's)
        }
        champ_make_cursor(wf, ws, CURSOR_LIVE)
    } else {
        champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED)
    };
    op_drop(cur); // release the consumed cursor's frame refs
    new
}

/// The current key, or NULL when exhausted (NULL is the done-signal; a real key is never NULL).
/// BORROWS.
#[allow(dead_code)]
fn op_map_iter_key(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base),
        None => Handle::NULL,
    }
}

/// The current value (paired with `op_map_iter_key`; no per-step pair allocation). BORROWS.
#[allow(dead_code)]
fn op_map_iter_val(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base + 1),
        None => Handle::NULL,
    }
}

// ─── CHAMP persistent SET (CHAMP minus the value column, stride 1) ───────────────────────
// A set is a PRIMITIVE collection, NOT `Map<T, Unit>`: entries are ONE handle. Every op is a thin
// `SET_STRIDE` wrapper over the SAME shared trie core the map uses (`champ_insert_node`,
// `champ_remove_node`, `champ_find_base`, and the cursor walkers), so there is a single code path to
// trust. The node shape is identical to a map's (bitmaps + size + handles); only the stride at the
// use-site differs — and the compiler picks the op family statically, so a set node is only ever
// touched with stride 1.

/// The canonical empty set — byte-identical to the empty map (`alloc_raw(vec![], champ_header(0,0,0))`);
/// the collection kind is compile-time knowledge, not a runtime tag.
#[allow(dead_code)]
// The shared IMMORTAL empty-SET singleton (per-type, mirrors EMPTY_MAP). Separate from EMPTY_MAP for
// type-clarity + zero cross-type aliasing, though an empty set + empty map are structurally identical.
runtime_local! {
    static EMPTY_SET: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

fn op_set_empty() -> Handle {
    // One shared immortal empty CHAMP node (see op_map_empty). SOUND: set insert gates on node_rc==1
    // and IMMORTAL != 1, so an insert path-copies off the singleton — never mutated in place.
    EMPTY_SET.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = alloc_raw(Vec::new(), champ_header(0, 0, 0));
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// O(1) element count. BORROWS `s`.
#[allow(dead_code)]
fn op_set_size(s: Handle) -> u32 {
    champ_size_of(s)
}

/// Total membership predicate — NEVER traps. BORROWS both `s` and `elem`. The only bool-returning
/// CHAMP op: the shared descent, returning presence instead of a value handle.
#[allow(dead_code)]
fn op_set_contains(s: Handle, elem: Handle) -> bool {
    champ_find_base(s, elem, SET_STRIDE).is_some()
}

/// `op_set_contains` with `elem`'s hash PRECOMPUTED — for a caller (set ∩/∖) that will also insert the
/// same element and so hashes it once, using this for the membership probe and the same hash for the
/// insert instead of re-walking the element twice. BORROWS both.
fn set_contains_h(s: Handle, elem: Handle, hash: u32) -> bool {
    champ_find_base_h(s, elem, hash, SET_STRIDE).is_some()
}

/// Insert `elem`, returning the new set. CONSUMES `s`, `elem`. Idempotent: inserting an existing
/// element leaves size unchanged and drops the incoming duplicate (the shared OVERWRITE rule with
/// no value columns keeps the stored element and drops the newcomer). Persistent: `op_dup` `s` first
/// to keep it.
#[allow(dead_code)]
fn op_set_insert(s: Handle, elem: Handle) -> Handle {
    set_insert_h(s, elem, champ_hash(elem))
}

/// `op_set_insert` with `elem`'s hash PRECOMPUTED — lets the set-algebra ops reuse the one hash they
/// computed for the membership probe (or the walk) instead of re-hashing. CONSUMES `s` and `elem`.
fn set_insert_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    // SET_STRIDE (1) routes through the SAME FBIP core as the map — one careful change covers both.
    champ_insert_fbip(s, Entry::elem(elem), hash, 0, SET_STRIDE, mine).0 // discard the size delta
}

/// Remove `elem`, returning the new set (canonical empty if the last element is removed). CONSUMES
/// `s`, BORROWS `elem`. Absent element ⇒ no-op returning `s` unchanged, no leak. FBIP: uniquely-owned
/// set refits in place; a shared set path-copies (old version byte-identical).
#[allow(dead_code)]
fn op_set_remove(s: Handle, elem: Handle) -> Handle {
    set_remove_h(s, elem, champ_hash(elem))
}

/// `op_set_remove` with `elem`'s hash PRECOMPUTED — lets set-difference (remove-from-a form) hash each
/// `b`-element once for its removal instead of re-hashing. CONSUMES `s`, BORROWS `elem`.
fn set_remove_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    let (new, _removed) = champ_remove_fbip(s, elem, hash, 0, SET_STRIDE, mine);
    new
}

/// A cursor over `s` at the first element in walk order (exhausted if `s` is empty). BORROWS `s`
/// (dups the frames it captures). Same cursor representation as the map's.
#[allow(dead_code)]
fn op_set_iter(s: Handle) -> Handle {
    if is_empty_node(s) {
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut frames = Vec::new();
    let mut slots = Slots::new();
    champ_descend_leftmost(s, &mut frames, &mut slots, SET_STRIDE);
    for &f in &frames {
        op_dup(f);
    }
    champ_make_cursor(frames, slots, CURSOR_LIVE)
}

/// Advance to the in-order successor. CONSUMES `cur`, returns the advanced cursor. FBIP: a uniquely
/// owned cursor (`rc == 1`) is refit in place (zero steady-state alloc); a forked/shared cursor
/// (`rc > 1`) path-copies into a fresh cursor so the other owner's walk is undisturbed. Order +
/// exhausted-signal identical to the map path (shared `champ_cursor_next_fbip` / `champ_advance`).
#[allow(dead_code)]
fn op_set_iter_next(cur: Handle) -> Handle {
    if node_rc(cur) == 1 {
        return champ_cursor_next_fbip(cur, SET_STRIDE);
    }
    // Shared (rc>1): copy path. `champ_cursor_read` already cloned the frames, so move the throwaway
    // `frames`/`slots` locals into the working `wf`/`ws` instead of cloning them a second time.
    let (state, frames, slots) = champ_cursor_read(cur);
    if state != CURSOR_LIVE {
        op_drop(cur);
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut wf = frames;
    let mut ws = slots;
    let live = champ_advance(&mut wf, &mut ws, SET_STRIDE);
    let new = if live {
        for &f in &wf {
            op_dup(f);
        }
        champ_make_cursor(wf, ws, CURSOR_LIVE)
    } else {
        champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED)
    };
    op_drop(cur);
    new
}

/// The current element, or NULL when exhausted (NULL is the done-signal; a real element is never
/// NULL). BORROWS.
#[allow(dead_code)]
fn op_set_iter_elem(cur: Handle) -> Handle {
    match champ_cursor_current(cur, SET_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base),
        None => Handle::NULL,
    }
}

// ─── CHAMP SET ALGEBRA — union / intersection / difference (U7) ───────────────────────────
// CORRECTNESS-FIRST landing over the existing insert + contains + iterate machinery: walk one
// operand's elements with a cursor (BORROWS) and build the result with `op_set_insert` (a CONSTRUCTOR
// that CONSUMES its accumulator + element — and is FBIP-fast on a uniquely-owned accumulator, so a
// fresh empty or a consumed operand refits in place with no per-insert node churn). A recursive
// CHAMP node-merge (structural union of subtrees, sharing whole shared subnodes) would be the
// O(min) form — DEFERRED as an optimization; this is O(n·log) but correct and canonical.
//
// Canonicality: `op_set_insert` places every element in the canonical CHAMP position (sorted collision
// nodes, compacted layout), so a set built by folding inserts is byte-identical (`champ_eq`/
// `champ_hash`) regardless of insertion order — the result is canonical BY CONSTRUCTION.
//
// Element ownership: a cursor / `set-contains` BORROWS its element, so an element CARRIED into the
// result is `op_dup`ed before `op_set_insert` consumes it; an element NOT carried stays owned by its
// operand and dies when that operand is `op_drop`ed. Both operands are consumed (dropped) at the end.

/// `set-union` — a new owned set with every element of `a` OR `b`. CONSUMES both. Built by walking the
/// SMALLER operand's elements into the LARGER (the accumulator base): union is commutative and the CHAMP
/// result is canonical-by-construction (insertion-order-independent), so the base choice cannot change
/// the result — only how many inserts run. Walking the smaller therefore does `min(|a|,|b|)` inserts
/// instead of always `|b|`. The base's own elements are carried by reusing it as the accumulator (no
/// per-element dup). An empty operand is the identity (`union(empty,b)==b`, `union(a,empty)==a`).
#[allow(dead_code)]
fn op_set_union(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node (structural sharing / self-union): a ∪ a = a. The caller passed two references to
        // the one node; keep one as the result and release the other. O(1) vs the O(n·log) fold below.
        op_drop(b);
        return a;
    }
    if is_empty_node(a) {
        op_drop(a);
        return b;
    }
    if is_empty_node(b) {
        op_drop(b);
        return a;
    }
    // Insert the SMALLER set into the LARGER: fewer inserts, identical (canonical) result.
    let (base, walk) = if op_set_size(a) >= op_set_size(b) {
        (a, b)
    } else {
        (b, a)
    };
    let mut acc = base;
    let mut cur = op_set_iter(walk);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE and pass it to the insert (via `set_insert_h`) rather than `op_set_insert`,
        // which would re-`champ_hash(e)` internally — a full subtree re-walk per element for a
        // compound/string element (free for scalars via the arity-0 fast path). Mirrors the hash-once
        // discipline already used by ∩/∖ below.
        let h = champ_hash(e);
        op_dup(e); // carried into the result; cursor only BORROWS it
        acc = set_insert_h(acc, e, h);
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(walk); // walk's own references released; carried elements survive via the dups above
    acc
}

/// `set-intersection` — a new owned set with elements in `a` AND `b`. CONSUMES both. Builds a fresh
/// empty accumulator, walks the SMALLER operand, and inserts each element that `set-contains` in the
/// other. `intersection(x, empty) == empty` and `intersection(empty, x) == empty`.
#[allow(dead_code)]
fn op_set_intersection(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node: a ∩ a = a. Keep one reference, release the other. O(1). (Correct even for the
        // empty set: ∅ ∩ ∅ = ∅.)
        op_drop(b);
        return a;
    }
    if is_empty_node(a) || is_empty_node(b) {
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    // Walk the smaller operand, probe the larger — fewer contains-probes, same result.
    let (walk, probe) = if op_set_size(a) <= op_set_size(b) {
        (a, b)
    } else {
        (b, a)
    };
    let mut acc = op_set_empty();
    let mut cur = op_set_iter(walk);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE and reuse it for both the membership probe and the insert (the probe and the
        // insert would otherwise each re-walk `e`'s subtree — a redundant hash per kept element).
        let h = champ_hash(e);
        if set_contains_h(probe, e, h) {
            op_dup(e); // carried; cursor BORROWS
            acc = set_insert_h(acc, e, h);
        }
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(walk);
    op_drop(probe);
    acc
}

/// `set-difference` — a new owned set with elements in `a` but NOT in `b`. CONSUMES both. Builds a
/// fresh empty accumulator, walks `a`, and inserts each element NOT `set-contains` in `b`.
/// `difference(a, empty) == a` (all of a) and `difference(empty, b) == empty`.
#[allow(dead_code)]
fn op_set_difference(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node: a ∖ a = ∅. Release both references and return a fresh empty set. O(1).
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    if is_empty_node(a) {
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    if is_empty_node(b) {
        op_drop(b);
        return a; // nothing excluded — a unchanged
    }
    // FAST PATH — remove `b`'s elements FROM `a` in place, when that is a clear win: `a` is UNIQUELY
    // owned (rc==1, so each remove refits it in place, allocation-free) AND `|b| < |a|` (so `|b|`
    // removes beat the `|a|` inserts-into-a-fresh-set the general path would do). The `b`-cursor
    // iterates `b` while the removes target `a` (a DISTINCT tree) — no aliasing. Removing a `b`-element
    // absent from `a` is a no-op, so the result is exactly `a ∖ b`, canonical (remove preserves shape).
    // NOTE the guards: on a SHARED `a` every remove would path-copy (WORSE than the insert-fold), and
    // when `|b| ≥ |a|` the insert-fold does fewer ops — so both fall through to the general path below.
    if node_rc(a) == 1 && op_set_size(b) < op_set_size(a) {
        let mut acc = a;
        let mut cur = op_set_iter(b);
        loop {
            let e = op_set_iter_elem(cur);
            if e == Handle::NULL {
                break;
            }
            acc = set_remove_h(acc, e, champ_hash(e)); // hash `e` once; cursor + remove both BORROW it
            cur = op_set_iter_next(cur);
        }
        op_drop(cur);
        op_drop(b); // b's references released; nothing carried into the result
        return acc;
    }
    let mut acc = op_set_empty();
    let mut cur = op_set_iter(a);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE for both the membership probe and (on a kept element) the insert.
        let h = champ_hash(e);
        if !set_contains_h(b, e, h) {
            op_dup(e); // carried; cursor BORROWS
            acc = set_insert_h(acc, e, h);
        }
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(a);
    op_drop(b);
    acc
}

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
