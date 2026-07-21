# DESIGN — Rust backend runtime trait layer

**Status:** scoping (v-rust-backend, 2026-07-16). Supersedes the numeric-tower "build a Rust
bignum (A) vs accept-as-declines (B)" fork per the operator architecture direction relayed by
concierge (answer `766879`). **GREENLIT** (concierge `3236108`): numerics-first, type-alias-first
shape, proceed-now. **⏳ BLOCKED (as of tick-30)** on a crate-structure decision — see
"§ Cross-crate build blocker" below (escalated to concierge + v-runtime).

## § NEW OPERATOR DIRECTIVE (2026-07-20, relayed by concierge/pr-sync via Slack) — REVERSES the collections-are-fine-as-std stance

Verbatim: *"For the rust backend we really need to make the heap types generic. Using std collections
is fine for smoke tests but are going to cause real performance issues due to a lack of structural
sharing and cloning everywhere. Ideally the compilation target could define a target runtime module
interface and implement all of the operations on that. And then we could use the existing cadenza
runtime crate as one of those implementations, just running inside rust instead of wasm. I also noted
this before but I think it got lost - we really want to avoid emitting the env per output cause that
will make it difficult to integrate for an application - they would have to impl that trait per script
rather than once generically over all scripts."*

**Two asks:**
1. **Heap types GENERIC over a target-runtime-module INTERFACE (trait), not std collections.** This
   OVERRIDES my earlier Q-collections recommendation ("std is fine, defer a collection trait, numerics-
   first"). The operator's rationale is CONFIRMED-real in the tree: `List.push` today emits
   `xs.clone(); __v.push(x)` — an O(n) copy per push, zero structural sharing (verified 2026-07-20). So
   collections are now IN-SCOPE for the trait, and the cadenza-runtime crate (its CHAMP/RRB persistent
   structures, running NATIVELY in rust not wasm) is to be ONE impl of that interface. This is bigger
   than the numeric tower: it's a `TargetRuntime` module-interface abstraction over ALL heap types
   (List/Map/Set/String/Bytes + the numeric tower), the emitted program written against the trait, with
   ≥1 impl (cadenza-runtime native; std as a smoke-test default).
2. **Do NOT emit the env per-output — emit a trait an app impls ONCE, generically over all scripts.**
   ✅ **ALREADY SATISFIED (2026-07-20, my async slice + prior work):** `CdzEnv` lives in the shared
   `cdz-rt` rlib (NOT re-declared per module — pinned by a test `!rs.contains("pub trait CdzEnv")`), and
   every async fn is generic `<__CdzE: CdzEnv>`, so an app implements `CdzEnv` ONCE and it works across
   all functions/scripts. Sync programs emit no env at all. The operator's "got lost" concern predates
   the shared-crate move. Reported this status back to the concierge; the same "impl once, generic over
   all scripts" shape is the model ask (1)'s runtime trait should follow.

**Design pivot for ask (1):** the earlier doc below treats collections as "std default, trait buys only
swappability, defer." The directive makes STRUCTURAL SHARING the goal, so the cadenza-runtime persistent
collections become the POINT, not an optional extra. Open question flagged to concierge: whether ask (1)
is one `TargetRuntime` trait (collections + numerics together — matches "target runtime module interface"
+ the "impl once generically" shape of ask 2) or per-type traits; and confirmation that the emitted code
should be trait-GENERIC (param/associated-type threaded) rather than the type-alias `mod rt` (option (c)
below) — a type-alias picks ONE impl at compile time and does NOT let an app choose per-integration, so
it likely does NOT satisfy "the application decides." Leaning: one `TargetRuntime` associated-type bound
at the module boundary (option (b)), mirroring how `CdzEnv` is threaded once. Awaiting the nod before
building (it's a large, emit-wide change).

## § DECISION (concierge answer, 2026-07-20) — PROCEED on the lean: ONE combined trait, trait-generic

Concierge routed the operator's call: **build on the TRAIT-GENERIC shape now** — ONE combined
`TargetRuntime` trait covering collections + numeric tower, associated-type-bound ONCE per module exactly
like `CdzEnv` threads (NOT a build-time type-alias — that can't let an app choose). (a) ONE combined
trait, not per-type. Operator has a Slack backstop; if they override to per-type I adjust, but build
combined-trait now. (b) The CHAMP/RRB value-adapter is **v-runtime's lane** (they own the Handle API); **I
own the trait DEFINITION + threading + emit**, they make cadenza-runtime's persistent collections present a
value surface the trait binds. v-runtime asked (note, 2026-07-20) for my collection-method SIGNATURES so
their adapter targets the right shape; BigInt is turnkey (`cdz-runtime::Big`).

### The `TargetRuntime` trait — collection-method signatures (v0 LOCKED — v-runtime acked all, 2026-07-20)
Status: signatures AGREED with v-runtime (consume-by-value + GAT confirmed; see the RESOLVED block below).
Ready to build the trait + std impl + emit-threading the moment MR `c6f1f3f2` lands (per-commit cadence
blocks a 2nd MR until then). v-runtime builds the cdz-runtime adapter against these once my scaffold is on
trunk.

Grounded in the current Core emit arms (`expr.rs`) — each trait method replaces one inline std-collection
emit, preserving the SAME ownership/semantics (consume vs borrow, trap kinds). Collection value types are
ASSOCIATED TYPES so an impl chooses the representation (std `Vec`/`BTreeMap`… for smoke tests; a
structural-sharing `im::Vector`/CHAMP for cadenza-runtime). Element/key bounds: `Clone + Ord` (Ord for
Map/Set keys; List needs only `Clone`; a float element already declines upstream).

```rust
pub trait TargetRuntime {
    // ── associated collection value types (an impl picks the representation) ──
    type List<T: Clone>: Clone;
    type Map<K: Clone + Ord, V: Clone>: Clone;
    type Set<E: Clone + Ord>: Clone;
    type Str: Clone;      // Cadenza String (UNICODE-scalar semantics)
    type Bytes: Clone;    // raw byte sequence
    type BigInt: Clone;   // = cdz_runtime::Big for the native impl (turnkey)

    // ── List (from Core::List{New,Push,Concat,Update,At,Len}) ──
    fn list_new<T: Clone>(elems: Vec<T>) -> Self::List<T>;
    fn list_push<T: Clone>(l: Self::List<T>, e: T) -> Self::List<T>;      // CONSUMES l, returns new
    fn list_concat<T: Clone>(a: Self::List<T>, b: Self::List<T>) -> Self::List<T>;
    fn list_update<T: Clone>(l: Self::List<T>, i: usize, e: T) -> Self::List<T>; // OOB → panic!("unreachable")
    fn list_at<T: Clone>(l: &Self::List<T>, i: usize) -> Option<T>;       // BORROWS, total (OOB → None)
    fn list_len<T: Clone>(l: &Self::List<T>) -> usize;

    // ── Map (from Core::Map{New,Insert,Lookup,Remove,Size,ToList}) ──
    fn map_new<K: Clone + Ord, V: Clone>(entries: Vec<(K, V)>) -> Self::Map<K, V>;  // later dup key overwrites
    fn map_insert<K: Clone + Ord, V: Clone>(m: Self::Map<K, V>, k: K, v: V) -> Self::Map<K, V>;
    fn map_lookup<K: Clone + Ord, V: Clone>(m: &Self::Map<K, V>, k: &K) -> Option<V>;
    fn map_remove<K: Clone + Ord, V: Clone>(m: Self::Map<K, V>, k: &K) -> Self::Map<K, V>;
    fn map_size<K: Clone + Ord, V: Clone>(m: &Self::Map<K, V>) -> usize;
    fn map_to_list<K: Clone + Ord, V: Clone>(m: &Self::Map<K, V>) -> Self::List<(K, V)>; // canonical key order

    // ── Set (from Core::Set{Of,Insert,Contains,Remove,Len,ToList,Algebra}) ──
    fn set_of<E: Clone + Ord>(elems: Vec<E>) -> Self::Set<E>;             // dedup at construction
    fn set_insert<E: Clone + Ord>(s: Self::Set<E>, e: E) -> Self::Set<E>;
    fn set_contains<E: Clone + Ord>(s: &Self::Set<E>, e: &E) -> bool;     // total
    fn set_remove<E: Clone + Ord>(s: Self::Set<E>, e: &E) -> Self::Set<E>;
    fn set_len<E: Clone + Ord>(s: &Self::Set<E>) -> usize;
    fn set_to_list<E: Clone + Ord>(s: &Self::Set<E>) -> Self::List<E>;    // canonical element order
    fn set_union<E: Clone + Ord>(a: Self::Set<E>, b: Self::Set<E>) -> Self::Set<E>;
    fn set_intersection<E: Clone + Ord>(a: Self::Set<E>, b: Self::Set<E>) -> Self::Set<E>;
    fn set_difference<E: Clone + Ord>(a: Self::Set<E>, b: Self::Set<E>) -> Self::Set<E>;

    // ── String / Bytes (Core::Str{ScalarLen,At,Slice,ToBytes,FromBytes,Cmp}, Bytes{Of,Len,At,Concat,Slice}) ──
    //   (signatures TBD in v1 — String is UNICODE-scalar-indexed, Bytes is byte-indexed; both mostly fold
    //    at compile time today, so the runtime-operand surface is smaller. Draft after collections land.)

    // ── BigInt (turnkey — cdz_runtime::Big) ──
    fn bigint_from_i64(v: i64) -> Self::BigInt;
    fn bigint_to_i64(b: &Self::BigInt) -> Option<i64>;   // None → caller traps (out of range)
    fn bigint_add(a: &Self::BigInt, b: &Self::BigInt) -> Self::BigInt;   // + sub/mul/div/rem, cmp
    // … div/rem trap on zero divisor (matches wasm); cmp → Ordering.
}
```

✅ **RESOLVED by v-runtime (2026-07-20 ack):** (i) OWNERSHIP = **CONSUME-BY-VALUE** for mutating ops
(`list_push(l: List<T>, e) -> List<T>`), `&self` for reading ops — CONFIRMED correct: matches the wasm
CONSUME/BORROW ABI, and a persistent CHAMP/RRB `push` takes `self` + path-copies only the spine (O(log n))
while sharing the rest, so consume-by-value is the honest+cheap shape for BOTH the persistent impl and the
std impl. No signature change. (ii) GAT/MSRV = **SAFE** — the seed toolchain is pinned to **rustc 1.95.0**
(`rust-toolchain.toml`, load-bearing for frozen-hash reproducibility); GATs stable since 1.65, so
`type List<T: Clone>` is fine, NO monomorphized-fallback needed. (iii) unchanged: `R: TargetRuntime`
threads once per module exactly like `__CdzE: CdzEnv`; the two compose
(`async fn f<R: TargetRuntime, E: CdzEnv>(env: &mut E, …)`).

🚩 **DIFFERENTIAL-ORDER CONSTRAINT (v-runtime flag — bake into the std impl + a gate case):** Map/Set keys
are `Clone + Ord`, but cdz-runtime's CHAMP keys compare by the **tagless value-canonical byte form**
(`champ_key_cmp` = value-cmp), NOT a derived Rust `Ord`. For SCALAR keys the two coincide; for COMPOUND
keys (List/Rational/String/tuple keys) the **value-canonical order is the blessed total order** (it's what
the wasm backend + `map_to_list`/`set_to_list` canonical iteration order use). So the trait's STD impl must
NOT rely on `BTreeMap`'s derived-`Ord` for compound keys if that disagrees with value-cmp — the two
`TargetRuntime` impls (std + cdz-runtime) must present the SAME total order or the differential check
diverges on iteration order. ACTION when building: add a differential gate case with a COMPOUND map/set key
(e.g. a tuple or list key) to pin that both impls agree; if `BTreeMap`-derived-Ord on the emitted key type
already equals value-cmp order (it may, since the emitted key types are structural), no extra work — but
VERIFY, don't assume. This is why `map_to_list`/`set_to_list` are speced "canonical (sorted) order": that
order is value-cmp, the blessed one.

### Emit-threading plan (my lane)
Mirror the `CdzEnv` threading (`mod.rs` ENV_TYPE_PARAM/ENV_PARAM machinery): add an `R: TargetRuntime`
type param to every emitted fn signature, map `Ty::List(t)`→`R::List<t>` etc. in `types.rs`, and rewrite
each `Core::List*`/`Map*`/`Set*` emit arm from the inline `Vec`/`BTreeMap` expression to an `R::list_push(…)`
call. The gate harness supplies a concrete `impl TargetRuntime` (a std-backed one for the differential
oracle; cdz-runtime-backed once v-runtime's adapter lands). This is a BIG multi-slice change — do it behind
the existing gate (rust + rust-async baselines), one collection family per slice, differential-checked.

## § Cross-crate build blocker (discovered tick-30) — `Big` is not linkable as-is

The "reuse `cdz-runtime::Big` turnkey" plan hits three real obstacles, because `Big` lives inside a
**wasm-shaped** crate:
1. **`cdz-runtime` is `crate-type = ["cdylib"]`** (built for the wasm component) — NOT an `rlib`. The
   gate compiles each emitted `.rs` with **bare `rustc` + `--extern`** (see the async path's `cdz-rt`
   rlib, `xtask/src/main.rs:565` + `:1080`), which needs an `rlib`.
2. **`#![cfg_attr(not(test), no_std)]`** — a native host build of cdz-runtime is `no_std`.
3. **`mod bigint` is PRIVATE** and `Big` is not re-exported.

**But `bigint.rs` is fully self-contained**: 994 lines, deps are ONLY `alloc::vec::Vec` +
`core::cmp::Ordering` (no `Handle`/`Node`/heap/`champ`); `num_bigint` is a dev-dep oracle. Cleanly
extractable.

**PROPOSAL (mirrors the `cdz-rt` rlib pattern):** extract the bignum into a NEW small crate
**`cdz-num`** (`crate-type = ["rlib"]`, alloc-only, std under host). Both consume it, no duplication:
- `cdz-runtime` depends on `cdz-num` for `box_bigint`/`unbox_bigint`/`bigint-*` (wasm build pulls it
  as a normal dep — frozen hash shifts ONCE, same code, v-runtime controls).
- The rust backend emits `cdz_num::Big` ops; the gate links `libcdz_num.rlib` via `--extern`.
- Rational/Qty wrappers (`struct Rational { num: Big, den: Big }`, then Qty) later live in `cdz-num`.

**Fallback (lighter touch, less clean):** add `"rlib"` to cdz-runtime's crate-type + `pub use
bigint::Big` — but that links the WHOLE 22k-line runtime as the extern and drags `no_std`/`alloc`
wiring into the native gate build. Prefer extraction.

**Ownership question (open):** whether v-runtime extracts (their crate + frozen hash) with me
consuming, or I do it with their review. Escalated; do NOT build the crate until routed.

## Operator intent (verbatim)

> "for the rust layer we should really have all of the collections generic over a trait. And then
> the application decides on how they want to use it. We can provide a default impl of that trait
> using our cadenza-runtime crate. But then we can just punt a lot of decisions to the application
> if they don't want to use it. Or use our already heavily-optimized collections for free."

So: emitted Rust programs against a **trait (or trait set)** for the runtime types — collections
(List/Map/Set/String/Bytes) and, by the same principle, the numeric tower (BigInt/Rational/
Quantity). Ship a **default impl backed by cadenza-runtime**; let an application swap its own impl.
This dissolves the numeric-tower fork: abstract the numeric types behind the trait, reuse
cadenza-runtime's existing bignum (do **not** hand-roll a second one), and apps can substitute.

## What I found in the tree (grounds answers 3 & 4)

- **`cdz-runtime::bigint::Big`** (`crates/cdz-runtime/src/bigint.rs`) is ALREADY a clean Rust value
  type: `struct Big { neg: bool, mag: Vec<u32> }`, `#[derive(Clone, PartialEq, Eq, Debug)]`, with
  `add/sub/mul/divmod/gcd/neg/cmp`, `from_i64`/`to_i64_checked`, `to_decimal_string`, and two byte
  encodings. It's `#[allow(dead_code)]` — DCE'd from the wasm today, wired only when the runtime
  `bigint-*` ops land. **This is directly reusable as the default BigInt impl for the Rust backend**
  — a value type, no Handle/heap indirection. Rational/Quantity do NOT yet have a `Big`-equivalent
  clean type (Rational is a normalized `IntValue` pair inside the compiler; the runtime rational-*
  ops are byte/Handle-level). So: BigInt reuse is turnkey; Rational/Qty need a thin value wrapper
  built ON `Big` (e.g. `struct Rational { num: Big, den: Big }`), which is the "reuse, don't
  hand-roll" path — the hard part (the bignum) already exists.

- **The collections are NOT clean generic Rust types.** cdz-runtime's CHAMP/RRB/rope live behind an
  opaque tagless `Handle` into a global heap (`crates/cdz-runtime/src/lib.rs`, 22k lines); the ops
  are internal `fn`s over `Handle` exposed to wasm via the component ABI, not an ergonomic
  `Champ<K,V>` / `Rope` value surface. **The Rust backend already does BETTER for collections** than
  wrapping that: it maps List→`Vec<T>`, Map→`BTreeMap<K,V>`, Set→`BTreeSet<T>`, String→`String`,
  Bytes→`Vec<u8>` — native std types that are already `Eq`/`Ord`/clone-able and need no runtime dep.
  So for collections the trait's *default impl is std*, and cadenza-runtime is an OPTIONAL
  alternative impl (its persistent/structural-sharing CHAMP/RRB for apps that want them), NOT the
  baseline. This inverts the emphasis vs numerics: collections already work natively; the trait
  buys *swappability*, not *capability*. Numerics are where the trait unlocks NEW capability
  (the ~107 todos).

- **no_std / frozen-hash (answer 4).** cdz-runtime is `#![cfg_attr(not(test), no_std)]`; its wasm
  bytes are content-hashed (`REQUIRED_RUNTIME_HASH`). BUT the Rust backend emits native Rust
  compiled by rustc — it would link cdz-runtime as an ordinary **`std` Rust library dependency**
  (the crate is plain `std` under non-wasm builds), NOT consume the hashed wasm artifact. So the
  frozen-hash discipline does NOT constrain a Rust-backend default impl: linking `Big` as a Rust
  type touches no wasm bytes and cannot move `REQUIRED_RUNTIME_HASH`. An app's custom impl is
  likewise unconstrained by the hash. (The one care: keep the default-impl surface we depend on
  behind a path that stays `std`-buildable — `Big` already is.)

## Design questions (concierge asked me to flag any needing an operator call)

1. **Trait granularity.** Recommendation: **per-type traits** an app implements piecemeal
   (`RtBigInt`, `RtRational`, `RtQty`; collections likely need no trait — see below), NOT one
   god `Runtime` trait. Rationale: an app wanting only a custom bignum shouldn't have to supply
   collections; and collections already have a fine native default so they don't belong in the
   same knob. *Needs an operator nod on whether collections get a trait at all (see Q-collections).*

2. **How emitted code is parameterized.** Options: (a) generic type params threaded through every
   fn signature — viscous, infects the whole emit; (b) an associated-type `Runtime` bound once at
   the module boundary; (c) a `cfg`/feature-selected impl module (a type alias
   `type RtBigInt = cdz_runtime::Big;` the app can override via a feature). Recommendation: **(c)**
   for the first increment — a `mod rt` with `pub type BigInt = …;` etc., default-aliased to
   cadenza-runtime types, app-overridable by a cargo feature — because it keeps the EMITTED code
   monomorphic and readable (no generic-param infection of every signature), which matters for the
   differential-oracle role. Promote to (b) associated-type if apps need multiple impls in one
   binary. *This is the highest-leverage operator call: (c) is pragmatic and ships now; (b) is the
   "properly generic" shape the verbatim intent gestures at. I lean (c)-first, (b)-when-needed.*

3. **Does cadenza-runtime expose clean Rust types, or need an adapter?** Split answer (above):
   **BigInt = clean type today** (`Big`, turnkey). **Rational/Qty = thin value wrappers to build on
   `Big`** (small, reuses the bignum). **Collections = std is the default; cadenza-runtime's are
   Handle-based and would need a real adapter** — so DON'T adapter-wrap them for the default; expose
   them only as an optional alternative impl later.

4. **Frozen-hash / no_std discipline.** Answered above: does NOT constrain the Rust-backend default
   impl (native rustc link, not the hashed wasm). No operator call needed unless we later want the
   Rust backend to consume the actual wasm runtime (we don't — that's the wasm backend's job).

### Extra question I'm surfacing (Q-collections)
Given collections already map to native std types that work, is the collection-trait worth building
at all in the near term, or do we scope the FIRST increment to **numerics only** (where the trait
unlocks the ~107 todos) and leave a collection-trait as a documented future knob? My recommendation:
**numerics-first**. It's where the capability gap is, the reuse story (`Big`) is turnkey, and it
directly retires the largest decline family. Collections-swappability is real but lower-value and
higher-cost (Handle adapter), so defer.

## Proposed incremental build (pending operator answers on Q2 / Q-collections)

1. `mod rt` in the emitted module: `pub type BigInt = cdz_runtime::Big;` (feature-overridable).
   Wire `Core::ConstBigInt` / `BigIntOfI64` / `BigIntToI64` / `BigIntBinOp` / `BigIntCmp`
   (`expr.rs:1401-1407`, currently a clean decline) to emit `rt::BigInt` value ops. Add
   cdz-runtime as an rcdzc-emitted-program dependency (the gate harness's generated `Cargo.toml`).
2. `struct Rational { num: BigInt, den: BigInt }` on top, normalized-at-construction; wire
   `ConstRational`/`RationalOfInts`/`RationalBinOp`/`RationalCmp` (`expr.rs:1412-1416`).
3. Quantity on top of Rational (reference-normalized magnitude + unit), matching the wasm semantics.
Each step is a gated slice (rust gate todos → pass), differential-checked against the wasm oracle.

## Why this is the right shape
No duplicate bignum (reuse `Big`), makes the Rust target genuinely usable (not just a differential
oracle), and punts representation to the app. The numeric tower stops being "sound declines" and
becomes real coverage — the largest remaining rust-vs-wasm gap.

## § Prepared emit-side (tick-31, ready to drop in once `cdz-num` exists)

Concierge cleared prepping the emit-side during the crate-decision wait. This is the concrete,
grounded plan — verified against the real Core variants (`core.rs`) and `Big` API (`bigint.rs`).

### Type mapping (`types.rs`)
`Ty::BigInt => Some("cdz_num::Big".to_string())`. `Big` is `Clone + Eq` (not `Copy`) → it's a
non-Copy binding, so `needs_clone_on_read`/`ty_is_non_copy` must include `Ty::BigInt` (clone-on-read
like `Vec`/`String`). `Big` is NOT `Ord` today (only `PartialEq/Eq`) — so a `Set`/`Map` of `BigInt`
would need `Big: Ord`; either add `#[derive(PartialOrd, Ord)]` to `Big` (it has a total `cmp`
already — trivial) OR decline BigInt-keyed Set/Map via the existing `ty_is_ord` (which currently
falls through to `true` for non-float — must add a `Ty::BigInt => <Ord?>` arm). Decide when wiring.

### Emit arms (`expr.rs`, replacing the clean decline at ~1401-1416)
- `Core::BigIntOfI64 { value }` → `cdz_num::Big::from_i64(<emit value> as i64)`.
- `Core::BigIntToI64 { operand }` → the checked narrowing that TRAPS out of range (matches wasm):
  `match (<emit operand>).to_i64_checked() { Some(v) => v, None => panic!("BigInt.to-i64 out of range") }`.
- `Core::BigIntBinOp { op, lhs, rhs }` — `Big` methods BORROW both operands, return `Big`:
  - Add/Sub/Mul → `(<l>).add(&(<r>))` / `.sub` / `.mul`.
  - Div → `(<l>).divmod(&(<r>)).map(|qr| qr.0).unwrap_or_else(|| panic!("BigInt divide by zero")).0`
    — divmod returns `Option<(quot, rem)>`, `None` on zero divisor (TRAP, matching wasm `bigint-div`).
    (Cleaner: `.divmod(&r).expect("BigInt divide by zero").0`.)
  - Rem → `.divmod(&r).expect("BigInt remainder by zero").1`.
- `Core::BigIntCmp { op, lhs, rhs }` → three-way then the fixed `Prim` compare-with-zero, mirroring
  the wasm lowering (`cmp <ₛ 0` etc.): `((<l>).cmp(&(<r>)) as i32) <op> 0` for a `Prim` relational op,
  or `(<l>).cmp(&(<r>)) == core::cmp::Ordering::Equal` for `=`. Result is `bool`. NOTE: `lower` already
  wraps cmp in a fixed compare for most forms, so this arm may only see the raw three-way — confirm
  the Core shape at wire-time (the wasm arm at `select.rs:437`/`mod.rs:1134` is the guide).

Const `BigInt` (beyond-i64 literal) already folds to `Core::ConstInt` retyped BigInt in `lower` and
reaches a DIFFERENT arm — verify it emits a `Big` (likely `Big::from_sign_magnitude_bytes(&[...])`
for a beyond-i64 constant, `Big::from_i64` for an in-range one).

### Gate harness (`xtask/src/main.rs`) — link the rlib (mirror `cdz-rt`)
- In the tools build step, add `-p cdz-num` to the cargo build (near `cdz-rt`, ~line 586), and record
  `cdz_num_dir = bin.join("libcdz_num.rlib").exists().then(|| bin.clone())` (mirror `cdz_rt_dir`, :603).
- In `run_program_rust` (~:1080), ALWAYS (sync too, not just async) pass `-L dependency=<dir> --extern
  cdz_num=<dir>/libcdz_num.rlib` when the emitted source mentions BigInt. Simplest: always link it if
  present (harmless if unused). Then a BigInt-returning program compiles + runs.
- Result RENDER (`cdz_render_at`): a `Big` result renders via `.to_decimal_string()` → the bare
  integer text cdz-run emits for a BigInt (e.g. `42`, `-58`), and an `(Option BigInt)` etc. composes.

### First gated slice (once crate lands + float-set MR merged)
Wire `BigIntOfI64` + `BigIntBinOp` (Add/Sub/Mul/Div) + the `Big` render → unblocks the
06-numeric-model.sexp runtime-BigInt cases ("a runtime-computed BigInt result crosses…", the +/-/*//
escapes, the Option-BigInt-payload cases). Then `BigIntCmp` + `BigIntToI64`. Then Rational, then Qty.
