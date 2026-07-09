# Design: genuine taglessness + fallibility policy

Status: **proposal / coordination** (runtime engineer → compiler engineer + operator).
No code or WIT edit lands until agreed. Scope: `cdz-runtime` only; the WIT is the shared contract.

Two questions on the table:

1. **Taglessness.** `enum Body` stores a Rust discriminant on every node — that *is* a per-object
   type tag. How do we store values without one, so the runtime tracks only what it irreducibly
   needs, and the compiled program passes the rest?
2. **Fallibility.** Which runtime functions should be fallible / trap? Especially maps — what
   happens when a looked-up key is absent? We do **not** want to trap on that.

**TL;DR:**
- The honest tagless node is `{ rc, handles[], raw_bytes[] }` — no discriminant. The only per-object
  metadata is `rc` + the **handle-scan count** (how many leading words are child handles). That
  count is *not* a type tag: `Int`, `Bool`, `Float`, `Bytes`, and `Str` all share the identical
  descriptor (0 handles). This is the Koka/Lean precise-RC layout. **No WIT change required.**
- **No runtime function needs to be fallible, and none needs to trap.** All fallibility is a
  *language-level* concern the compiler expresses with primitives that already exist: a `trap` it
  emits for the spec's total-or-trap ops (`List.at`/`Bytes.at`/`String.at` OOB), and an `Option`
  **sum** it emits for lookups that must not trap (map key-miss). The runtime stays **total**.
- The one policy call for the operator: on an **OOB index into a valid node** (a can't-happen
  compiler-invariant violation), should the runtime **trap (fail-fast)** or return a benign default?
  Recommendation: **trap**, matching "assume the compiler is correct and trap on tuple OOB." Scalar
  type-mismatch reads and null-handle reads stay benign (inherently total in this rep).

---

## Part 1 — Taglessness

### Why `enum Body` is a tag (conceding the point)

`enum Body { Int, Bool, Float, Arr, Sum, Bytes, Str, Map }` stores a Rust discriminant in every
node. That discriminant answers "what *kind* am I?" — which is exactly a per-object type tag. A
truly tagless node cannot distinguish an `Int` from a `Bool`; the *compiler* must, because it holds
the static type at every use site. The handoff blessed the enum ("the one distinction the runtime
needs"); that was a rationalization. It is a tag.

### The honest tagless node

```
Node (one heap allocation; its address is the Handle):
  rc        : u32          non-atomic, born 1
  n_handles : u32          # of leading child-handle words  ── the "scan count"
  n_raw     : u32          # of raw payload bytes
  handles   : [handle; n_handles]   child handles (the free cascade scans exactly these)
  raw       : [u8; n_raw]           scalar bits / sum disc / byte buffer / UTF-8 string bytes
```

Native representation (keeps the Handle-typed core testable, no wasm needed):

```rust
struct Node {
    rc: u32,
    handles: Vec<Handle>,  // n_handles = handles.len()
    raw: Vec<u8>,          // n_raw     = raw.len()
}
```

**No enum. No discriminant.** Every typed entry point funnels into this one shape:

| constructor         | handles                  | raw                         |
|---------------------|--------------------------|-----------------------------|
| `box-int v`         | `[]`                     | `v.to_le_bytes()` (8)       |
| `box-bool b`        | `[]`                     | `[b as u8]` (or 8, padded)  |
| `box-float f`       | `[]`                     | `f.to_bits().to_le_bytes()` |
| `arr-alloc n`       | `[NULL; n]`              | `[]`                        |
| `sum-new disc p`    | `[p]`                    | `disc.to_le_bytes()` (4)    |
| `bytes-alloc n`     | `[]`                     | `[0; n]`                    |
| `str-new s`         | `[]`                     | `s.into_bytes()`            |
| `map-alloc n`       | `[NULL; 2n]`             | `[]`                        |

Accessors read back out of the same shape — `arr-len = handles.len()`, `bytes-len = raw.len()`,
`sum-disc = u32::from_le_bytes(raw[0..4])`, `sum-payload = handles[0]`, `map-key i = handles[2i]`,
`map-val i = handles[2i+1]`, `map-len = handles.len()/2`, `get-int = i64::from_le_bytes(raw[0..8])`.

### Why the scan count is not a type tag

The load-bearing property: **two values with the same physical layout share the identical
descriptor, regardless of Cadenza type.**

- `Int`, `Bool`, `Float`, `Bytes`, `Str` → all `n_handles = 0`. The runtime cannot tell them apart
  — and never needs to, because the compiler only ever emits `get-int` where the type is `Int`.
- A 2-tuple, a 2-element list, and a 2-field record → all `n_handles = 2`, byte-identical. This is
  exactly the "same node, different render" property the tests already prove.
- `Sum` → `n_handles = 1` (the payload) + a raw disc word.
- `Map` → `n_handles = 2·len`.

`n_handles` is *genuine layout data the free cascade needs* (which words to recurse into on
reclamation), not an identity. It is precisely what Koka's and Lean 4's precise-RC runtimes store.
`arr-len` and `sum-disc` — the two "genuine runtime data" values the WIT already blesses — are
recovered from `handles.len()` and the raw disc word; they were never tags either.

### "What if every call site passed explicit layout?" — the floor, proven

A natural push: don't store even `n_handles` — have the compiler pass layout as a parameter at
every call, since it knows the static type everywhere. How far does this go?

**Far, for construction and single-level access.** Layout-as-parameter *already exists in disguise*:
`get-int` vs `arr-get` is the compiler passing "treat this as Int" vs "as array" via which function
it calls. The generic API (Level 2b) makes it explicit — `alloc(n_handles, n_raw)` / `set-handle` /
`get-word` — and the runtime stores no kind. This part fully works.

**It hits a wall at the free cascade.** `drop(node)` recurses into children, and for each child must
know *its* out-degree to recurse further. A node's **type** is static → pushable to the call site.
But a `list`/`map` node's **length** is runtime data. Decisive example: dropping a
`List<List<Int>>`, the drop site knows the type but NOT that inner-list #2 has length 7 — that is
discoverable only by inspecting inner-list #2, which the runtime reaches *transitively*, with no
caller in scope to supply it. **A variable-length node must therefore store its own length**, and
that length is exactly `n_handles`.

**Storing literally nothing** requires moving the cascade into emitted per-type `drop_T` glue
(Model B below). That reintroduces deep-spine stack overflow (or forces the compiler to re-emit a
heap worklist — the runtime's cascade, but bigger/slower in wasm), plus per-type code bloat. The
only alternative to a bare count is storing a static *shape-descriptor pointer* — 4 bytes just like
the count, but a pointer into type info is *more* tag-like than a bare integer, not less.

**Conclusion — the irreducible per-node state is `rc + n_handles + n_raw`.** `rc` (RC needs it),
`n_handles` (a runtime-driven cascade needs each node's out-degree; a nested collection's length
can't be handed in), `n_raw` (byte/string length). Zero discriminant, zero type identity — `Int`,
`Bool`, and a 5-element list are distinguished by *content and count*, never a stored kind. Pushing
type layout to call sites is exactly what taglessness means; the length of a variable node is stored
regardless. The scan count is not a compromise on the ask — it is the ask's floor.

### Rejected alternative — compiler-emitted recursive drop glue

The "purest" compiler-owns-layout idea is to store *only* `rc` and have the compiler emit a
per-type `drop_T` that walks children and calls a dumb `free`. **Rejected**, because:

- It is recursive in the emitted code → the free of a deep unique spine grows the wasm call stack by
  structure depth and overflows it — reintroducing the exact hazard the iterative worklist cascade
  was built to kill (see `deep_unique_structure_frees_without_stack_overflow`, 200k depth).
- It emits one drop function per monomorphized type — significant code bloat and re-introduced
  per-type dispatch.

Storing `n_handles` and keeping the **iterative** worklist cascade in the runtime is strictly
better: O(1) stack, O(n) work, zero emitted glue. The scan count is the minimum that buys this.

### Two levels — and the recommendation

**Level 2a — tagless internal rep, typed API unchanged (RECOMMENDED, no WIT change).**
Replace `enum Body` with `{ rc, handles, raw }`. The 26 typed WIT functions stay as *typed entry
points* — they carry no stored tag; `box-int` and `bytes-alloc` produce physically-identical-shaped
nodes differing only in content. This fully delivers the ask (no type tag stored; runtime tracks
only `rc` + scan/raw counts; compiler owns all type knowledge) with **zero blast radius** on the
frozen contract, the paused compiler, or the emitted envelope. Ships now.

**Level 2b — generic layout API, touches the frozen WIT (designed below, NOT recommended now).**
Collapse the 26 typed funcs into ~8 generic ones so the compiler passes layout explicitly at the
boundary:

```wit
interface heap {
  alloc: func(n-handles: u32, n-raw: u32) -> u32;   // one constructor
  set-handle: func(node: u32, slot: u32, h: u32) -> u32;
  get-handle: func(node: u32, slot: u32) -> u32;
  n-handles: func(node: u32) -> u32;                // = arr-len / 2·map-len
  set-word: func(node: u32, byte-off: u32, v: u64) -> u32;   // int/float/disc bits
  get-word: func(node: u32, byte-off: u32) -> u64;
  set-byte: func(node: u32, i: u32, v: u32) -> u32;          // bytes buffers
  get-byte: func(node: u32, i: u32) -> u32;
  n-raw:  func(node: u32) -> u32;
  dup:  func(node: u32);
  drop: func(node: u32);
  // strings: keep str-new/str-get as the ONE typed exception (see cost below)
}
```

Honest cost of 2b, which is why I do **not** recommend taking it now:
- Floats/ints become raw words → the compiler must bit-pun (`f64.reinterpret_i64`), extra codegen.
- Strings lose the component-model `string` marshaling (free UTF-8 across the boundary) unless we
  keep `str-new`/`str-get` as a typed exception — so it isn't even fully generic.
- Every himport index shifts; the fixed component envelope must be re-derived
  (`spec/learnings/2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md` warns each
  such change is a one-time-but-real cost); the compiler's entire constructor codegen changes.
- The purity gain over 2a is essentially aesthetic: 2a already stores no tag. The typed entry points
  in 2a are functions, not stored metadata.

**Recommendation: implement 2a now; hold 2b as a documented option we take only if we later want the
smaller import surface for its own sake.**

---

## Part 2 — Fallibility

### The key realization: a tagless runtime *cannot* do a fallible lookup

Map key lookup requires **type-directed key equality** — Int keys compare as ints, String keys
byte-wise, tuple keys structurally. A tagless runtime has no tag and therefore *cannot* compare two
key handles. So map lookup **must** be compiler-emitted:

```
lookup(m, needle):
  for i in 0 .. map-len(m):
    if key_eq(map-key(m, i), needle):   # key_eq = the compiler's type-directed equality
      return Some(map-val(m, i))
  return None
```

`Some`/`None` is a **Cadenza sum**, built with the existing `sum-new`. So the answer to "what
happens when the key doesn't exist, and I don't want to trap" is: **lookup returns an `Option`**,
constructed by the compiler out of primitives that already exist. The runtime needs **no
map-lookup primitive and no fallible anything**. This is the language expressing failure in its own
type system, which is exactly right — and it matches the corpus, where maps have no by-key lookup
op yet and absent-key behavior is deliberately unpinned.

### Which operations trap, and who emits the trap

The spec's runtime-value-indexed accesses are **total-or-trap** (`collections-and-text.md:48`;
`10-bytes.sexp`; `13-strings.sexp`): `List.at` / `Bytes.at` / `String.at` MUST trap on out-of-bounds
(including negative) with a defined kind. The clean division:

- **The compiler emits the bounds check and the `trap`.** For `(List.at xs i)` it emits
  `if i < 0 || i >= arr-len(xs) then trap("list index out of bounds") else arr-get(xs, i)`. The
  compiler already produces the spec's trap kinds on the const-fold path (`ConstTrap`); this is the
  runtime-valued mirror. The signed→unsigned hazard the corpus warns about is handled here, before
  the runtime ever sees the index.
- **The runtime accessor is only ever called in bounds.** Tuple/record access is by static index
  (compile-time-checked — a type error, never a runtime OOB). The render walk indexes `0..arr-len`.
  So no runtime accessor is *reached* out of bounds in correct operation.

Net: the runtime stays **total and trap-free** in normal operation. Fallibility is entirely
compiler-side, split cleanly by intent — `trap` for total-or-trap ops, `Option`/`Result` sums for
"failure is a value" lookups.

### The one policy decision: OOB index into a *valid* node

An OOB index into a valid node can only occur if the compiler has a bug (it violated the invariant
above). Two choices for that can't-happen path:

- **(A) Trap (fail-fast).** A compiler bug aborts loudly and shows up as an unmistakable divergence
  in the differential gate, instead of a silent wrong value that might accidentally match.
- **(B) Benign default (current behavior).** Return `NULL`/0; never crash, but a compiler bug leaks
  a silent wrong output.

**Recommendation: (A) trap on OOB index.** This matches your "assume the compiler is correct and
trap on tuple OOB" — for accesses the compiler guarantees, a violation is a bug we *want* to catch,
and the gate catches traps far better than silent zeros. This is a defensive backstop that never
fires in correct operation, so it does not weaken any real path.

Kept benign regardless (inherently total in the tagless rep, nothing to check):
- **Scalar type-mismatch reads** — `get-int` reinterprets `raw[0..8]`; on a mismatched node it
  yields deterministic garbage, never a trap. (There is no tag to compare, so this is total by
  construction.)
- **Null-handle reads** — the benign-default sentinel; reading it returns 0/`NULL`/"" so a stray
  null never faults linear memory.

This flips one line of the handoff ("get-* returns a benign default, never traps") for the *index*
accessors only. It needs the operator's OK — which "trap on tuple OOB" appears to grant.

---

## Part 3 — drop cost tiers and reuse (compiler-supplied drop hints)

The question: can the compiler pass drop hints so the runtime optimizes reclamation — a cheap-drop
vs expensive-drop signal, or a few drop calls of varying cost the compiler picks between? Yes; it is
what Lean 4 / Koka do. But it splits into a free win the runtime takes itself and a win that
genuinely needs the hint.

### The biggest cheap-drop case is free (no hint, no WIT change)

A leaf drop (Int/Bool/Float/Bytes/Str) needs no cascade — `dec rc; if 0 free`. The runtime already
knows it is a leaf: `n_handles == 0`, the length it stores anyway. So `op_drop` fast-paths it with
no compiler cooperation:

```rust
fn op_drop(h) {
    if rc > 1 { rc -= 1; return }              // shared: no scan
    if node.handles.is_empty() { free(h); return }  // leaf: free, NO worklist
    // …only here seed the worklist for the general cascade
}
```

Today's `op_drop` heap-allocates a `worklist: Vec` and calls `owned_children` even to free one Int —
that overhead dies here. **Part of the 2a rewrite; the compiler does nothing for this.**

### What a hint adds on top: depth-boundedness (modest, measure first)

The one thing the runtime cannot infer from a single node is **depth-boundedness**. `(tuple Int
Int)` is statically bounded (3 frees); `List<Int>` is runtime-unbounded in its spine — the *only*
reason the heap worklist exists (stack safety, the 200k-depth test). The compiler knows statically:
a type is unbounded iff it reaches a `List` or recursive sum anywhere in its structure. So two
appended calls (indices 26+, existing indices untouched — the WIT's "append only" rule, one envelope
re-derivation):

- `drop(node)` — general iterative worklist; emitted only when the static type reaches a
  list/recursive sum.
- `drop-bounded(node)` — statically depth-bounded types: recurse on a fixed small stack, never touch
  the heap worklist, cannot overflow (depth is a compile-time constant).

Payoff is modest once the leaf fast path exists **and** the general worklist uses a small *inline*
buffer that only spills to heap when genuinely deep — then `drop-bounded` only saves the inline
setup for shallow structures. **Measure before spending the envelope re-derivation.** Multiple
functions, not a hint param, so `drop`'s frozen signature never changes.

### The drop hint actually worth an API touch: reuse (FBIP) — Phase D.2

The high-value compiler-supplied info is not cheap-vs-expensive; it is **reuse**. A drop of an
`rc==1` node immediately followed by a same-size allocation (map/list rebuild, record update) should
hand the memory straight to the allocation instead of free→malloc. This is Lean/Koka's core
performance story (in-place `map`/`filter`). It genuinely needs compiler cooperation and a
deliberate **appended** API — a `drop-for-reuse` that yields the slot as a reuse token feeding the
next `alloc` — because the runtime cannot know the *next allocation's size class*. This is the
frame-limited reuse already flagged as Phase D.2, and it is where drop-hints pay for themselves far
more than a bounded/deep split does.

### Recommendation

1. **Now (2a rewrite):** leaf + shared fast paths + inline worklist buffer. Free; grabs most of the
   cheap-drop win from the length already stored.
2. **Deliberate next WIT touch (appended, measured):** the **reuse** API — where compiler drop-hints
   are load-bearing — ahead of `drop-bounded`, which we add only if measurement shows the inline
   buffer is insufficient.

## WIT impact

- **Level 2a + the fallibility policy: no WIT change.** Taglessness is an internal rep change;
  fallibility lives compiler-side using existing primitives. The 26 frozen functions and their
  indices are untouched.
- **Level 2b (only if the operator wants the smaller import surface): full reshape**, one-time
  envelope re-derivation. Spec above.

## Coordination asks for the compiler engineer

1. **Bounds checks + traps are yours.** For `List.at`/`Bytes.at`/`String.at`, emit the
   sign-aware bounds check and the spec-kinded `trap`, then call the (in-bounds) accessor. The
   runtime will not defensively handle OOB — it will trap (policy A above) as a backstop.
2. **Map lookup is yours, as an `Option`.** When map key-lookup is specified, emit the
   type-directed `key_eq` iteration and return a `sum-new`-built `Option`. No runtime primitive is
   needed or wanted.
3. **Reconcile the himport index desync before wiring `dup`/`drop`.** The compiler's `himport`
   table (`codegen.rs`: dup=17, drop=18, map-*=19..23, `RT_N_IMPORTS=24`, str-* omitted) is out of
   sync with the frozen WIT (str-new=17, str-get=18, dup=19, drop=20, map-*=21..25, 26 funcs).
   `himport::{DUP,DROP}` currently have zero call sites, so live reclamation is safe today; but the
   indices must line up before the Perceus call sites are emitted or `drop` will mis-link.

## Test impact (Level 2a)

- All 24 tests stay, unweakened, except `mismatched_read_returns_default_never_traps`: in the
  tagless rep a mismatched scalar read *reinterprets bytes* rather than returning a type-specific
  default, so its assertions change from "== 0 / false / 0.0" to "is total, does not trap." This
  tracks the tagless rep faithfully; it is not a weakening (the read is still total).
- Add: OOB index into a valid node traps (policy A); null-handle read stays benign.
- The "same bytes, different render" and RC/free-cascade tests carry over unchanged — the node is
  still `rc` + child handles + raw bytes, just without a discriminant.
