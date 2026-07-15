# Design — a configurable, non-wasm-specific backend for rcdzc (Rust as the first new target)

**Author:** compiler engineer. **Audience:** whoever adds a second `rcdzc` backend.
**Status:** **DESIGN ONLY — nothing landed.** The pure-value spine is feasible against the code as it
stands today (2026-07-09); the effectful/host + async spine is forward design gated on the effects
workstream (task #148), which `rcdzc` has **not** started — `(meta …)` capabilities still *decline* at
`rcdzc/src/resolve.rs:972`. Where I name a line number it is a landmark at this commit, not a promise it
won't drift.

This is a how-to, not a mandate. It states one architectural move — **lift the backend out of the
pipeline behind a `Target` seam** — and then designs the Rust target against the real `rcdzc` structs,
with the honest decline boundaries called out at each step.

---

## 1. TL;DR — the win, the seam, and the one insight

**The win.** Today `rcdzc` emits exactly one thing: a WebAssembly component that imports the value-heap
runtime. If the backend is configurable, the *same* front end (decode → resolve → infer → lower → fold)
can emit **Rust source** as an alternative artifact. That opens the door the user asked for: author a
self-contained module in Cadenza, compile it to Rust, and link it into an existing Rust codebase (e.g.
Membrain) as an ordinary crate — no wasm host, no component boundary.

**The seam already exists.** The pipeline is already an artifacts-in / artifacts-out ABI keyed by a
`kind` string (`rcdzc/src/abi.rs`), and the boundary surface is already computed as a target-neutral
`Layout` of typed signatures **before** any wasm-specific work happens (`rcdzc/src/layout.rs`). The
wasm commitment is entirely downstream of one point in `compile_inner` — the call to
`select::select_module` (`rcdzc/src/pipeline.rs:70`). A backend is a function of `(MirModule, Layout)`.
Everything above that line is target-neutral; everything below it (`select` → `serialize` → `heap` →
`render`, ≈2270 lines) is one backend that happens to target wasm.

**The insight that makes it cheap.** `Mir` (`rcdzc/src/ir.rs:614`) is a **typed, structured expression
tree** — `If` / `Match` / `Let` / `Call` / `Apply` / `Arith` with a *solved `Ty`* on every type-bearing
node — that maps almost 1:1 onto Rust `if` / `match` / `let` / calls. The hard-to-retarget rung (the
flat stack-machine `Lir`, `ir.rs:706`) is produced *by* the wasm backend; a Rust backend never builds
it. So retargeting is **not** "reconstruct control flow from `If`/`Else`/`End` markers"; it is "print a
tree the compiler already solved." The `RT_FIXED_FUNCS` raw-wasm loop helpers (`heap.rs`, for
UTF-8 validation / `itoa` / `putu`) and the loop-requiring intrinsic declines (`Bytes.of`, `Set.of` over
non-literals) exist *only* because flat `Lir` can't express a loop — in Rust they become `str::from_utf8`,
`format!`, and ordinary `for` loops, and the declines lift.

---

## 2. Where the backend plugs in — the exact seam

`compile_inner` (`rcdzc/src/pipeline.rs:30`) today runs the front end, then hardcodes the wasm backend:

```
let hir    = resolve::resolve_program(node)?;   // Ast  → Hir
let typed  = infer::infer_module(hir)?;         // Hir  → typed-Hir
let mir    = lower::lower_module(typed);         // typed-Hir → Mir
// ... fold poison collection + erasure-fence checks (TARGET-NEUTRAL — keep) ...
let layout   = Layout::of(&mir)?;                // TARGET-NEUTRAL — keep
let entry_ret = mir.funcs[layout.order[0]].ret.clone();
let selected = select::select_module(mir, &layout)?;   // ← WASM STARTS HERE
if layout.imports_runtime {
    serialize::runtime_compound_component(&selected, &layout, &entry_ret)
} else {
    serialize::component(&selected, &layout)
}
```

**Everything before `select::select_module` is target-neutral and stays put.** Specifically these are
NOT wasm-specific and every backend reuses them verbatim:

- **fold poison collection** (`pipeline.rs:40`) — a compile-time-provable trap fails the build (CDZ0304).
  Target-neutral: a `1/0` is a compile error regardless of backend.
- **erasure fence** (`pipeline.rs:54`) — a compile-time-only value (a type-value) must not reach the
  runtime boundary (CDZ0305). Target-neutral.
- **`Layout::of`** (`pipeline.rs:62`) — see §3. The one field that is wasm-specific (`imports_runtime`
  and the `abs`/`order` index assignment) is *ignored* by a non-wasm backend; the `exports`
  (`Vec<ExportPlan>`) are exactly what every backend needs.

The move: introduce a `Target` and branch on it *at* the current `select` call.

```rust
// abi.rs — a new artifact kind alongside KIND_COMPONENT
pub const KIND_RUST: &'static str = "rust";

// pipeline.rs — the backend becomes a parameter, defaulting to Wasm (byte-identical today)
pub enum Target { Wasm, Rust }

fn compile_inner(node: &Node, target: Target) -> Result<Artifact, Vec<Reject>> {
    // ... front end + fold + fence + Layout::of, all unchanged ...
    match target {
        Target::Wasm => {
            let selected = select::select_module(mir, &layout)?;
            let bytes = if layout.imports_runtime {
                serialize::runtime_compound_component(&selected, &layout, &entry_ret)?
            } else {
                serialize::component(&selected, &layout)?
            };
            Ok(Artifact::new(Artifact::KIND_COMPONENT, bytes))
        }
        Target::Rust => {
            let src = rustgen::emit_module(&mir, &layout)?;   // NEW backend, §4–§7
            Ok(Artifact::new(Artifact::KIND_RUST, src.into_bytes()))
        }
    }
}
```

Note the return type widens from "the component bytes" to "the artifact" — the `kind` already tells the
consumer what it got, which is exactly what the artifact ABI (`abi.rs:1`) was designed for. The `Target`
is selected by the build-tool interface (an input artifact, an env var like the existing
`CADENZA_COMPILER=v2` opt-in, or a flag); *how* it is selected is the toolchain's concern, not the
compiler's.

**This is the whole architectural change.** Everything else in this doc is the body of `rustgen`.

---

## 3. What the backend receives — `MirModule` + `Layout` are target-neutral already

The two inputs a backend consumes:

### `MirModule` (`rcdzc/src/ir.rs:599`)

```rust
pub struct MirModule { pub funcs: Vec<MirFunc>, pub exports: Vec<Export> }
pub struct MirFunc   { pub params: Vec<Ty>, pub ret: Ty, pub body: Mir }   // ir.rs:606
```

A function body is a **single `Mir` expression tree** (not a basic-block CFG); sequencing is nested
`Let`. This is the crucial property: it prints directly as a Rust expression.

### `Layout` (`rcdzc/src/layout.rs:44`)

```rust
pub struct Layout {
    pub exports: Vec<ExportPlan>,   // every (export …), by SIGNATURE — see below
    pub abs: Vec<u32>,              // wasm function index of user fn i — WASM-ONLY, ignore
    pub order: Vec<usize>,          // emission order — reused (see note)
    pub imports_runtime: bool,      // wasm envelope selector — WASM-ONLY, ignore
}
pub struct ExportPlan {             // layout.rs:30
    pub name: String,               // boundary name, VERBATIM (no main→run magic)
    pub func: usize,                // module index of the exported fn
    pub params: Vec<Ty>,            // the ABI inputs, as solved types
    pub ret: Ty,                    // the ABI output, as a solved type
}
```

`ExportPlan` **is the Rust function signature**, already computed and target-neutral. `params: Vec<Ty>`
and `ret: Ty` are exactly what §4 maps to Rust parameter/return types, and `name` is emitted verbatim
(the design's no-rename discipline, `layout.rs:31`, is a *gift* to a Rust backend: the exported Rust
`fn` has the name the author wrote, so it drops in against a caller that expects that name).

`Layout::of` also runs **reachability DCE** (`layout.rs:92`): only functions reachable from an export
are emitted. A Rust backend reuses this directly — it emits `fn`s for `layout.order` (or just the
reachable set), skipping dead helpers, and needs nothing wasm-specific from it. Ignore `abs` and
`imports_runtime`.

> One target-neutral cleanup worth doing while here: `Layout` mixes the neutral surface (`exports`,
> reachability) with the wasm index assignment (`abs`, `imports_runtime`, the `RT_FUNC_BASE` offset).
> A backend split makes the neutral half (`exports` + reachable `order`) the shared product and pushes
> `abs`/`imports_runtime` into the wasm backend where they belong. Not required for a first spike — the
> Rust backend can simply not read those fields — but it clarifies the seam.

---

## 4. Types — `Ty` → Rust types, and the central choice the user raised

`Ty` (`rcdzc/src/ty.rs:235`) is fully structural and is threaded onto every Mir node. The complete map:

| `Ty` variant | wasm today (a heap `i32` handle) | **Rust-ergonomic** target | **ABI-0 target** (link the runtime) |
|---|---|---|---|
| `Int` | `i64` | `i64` | `i64` |
| `Bool` | `i32` | `bool` | `bool` |
| `Unit` | (no slot) | `()` | `()` |
| `Tuple(elems)` | `arr` handle | `(T0, T1, …)` | `Handle` |
| `Record(fields)` | `arr` handle | a generated `struct` w/ named fields | `Handle` |
| `List(T)` | RRB `vec` handle | `Vec<T>` (or a persistent-vector crate) | `Handle` |
| `Bytes` | `bytes-*` leaf | `Vec<u8>` / `bytes::Bytes` | `Handle` |
| `String` | `bytes-*` leaf | `String` | `Handle` |
| `Map(K,V)` | CHAMP handle | `BTreeMap<K,V>` (or persistent-map crate) | `Handle` |
| `Set(E)` | CHAMP handle | `BTreeSet<E>` (or persistent-set crate) | `Handle` |
| `Sum{def,args}` | `(disc, payload)` handle | a generated `enum` | `Handle` |
| `Fn(params,ret)` | (folds away; no rep) | `fn`/`impl Fn` — deferred, see §6 | same |
| `Param`/`Var`/`Type` | (no runtime rep) | erased before this point | erased |

**The user asked: rust-native types, or link the runtime for ABI-0? These are two distinct backend
*value strategies*, and the doc's recommendation is to support both, in this order.**

### Strategy A — "rust-ergonomic" (native types). *Recommended default.*

Each Cadenza compound becomes a real Rust type: `Tuple` → a Rust tuple, `Record` → a generated `struct`,
`Sum` → a generated `enum`, `List` → `Vec`, `Map` → `BTreeMap`, `Set` → `BTreeSet`, `Bytes` → `Vec<u8>`,
`String` → `String`. This is what makes the output *drop-in*: a Rust caller passes and receives ordinary
Rust values, pattern-matches the generated `enum`, iterates the `Vec` — no handles, no `unsafe`, no
runtime dependency at all. The `cdz-runtime` crate is **not linked** in this strategy; the value-heap
machinery (box/unbox, `dup`/`drop`, CHAMP) simply does not appear — Rust's own ownership/`Drop` and
`clone()` replace Perceus, and `Vec`/`BTreeMap` replace the persistent collections.

**Consequence to state honestly:** this changes the *sharing/persistence* semantics. Cadenza's
collections are persistent (immutable, structurally shared, O(log n) update returning a new version);
`Vec`/`BTreeMap` are not. For a **pure value→value** function that builds a result and returns it — the
whole feasibility spike, and functions like `VarU64` encode/decode or `Interval::subtract` — this is
invisible: the observable input/output is identical, only the internal allocation strategy differs. It
becomes observable only if a program *relies* on cheap persistence of a large collection across many
versions (holding many old versions live). A first backend should emit `Vec`/`BTreeMap` and **note in
generated-code comments** that persistence is not preserved; if a program needs it, swap in a persistent
crate (`im`, `rpds`) behind the same `Ty` → type map — a localized change in `rustgen`, not a redesign.

### Strategy B — "ABI-0" (link `cdz-runtime` directly). *For semantic fidelity / staged migration.*

Every compound stays a `Handle` and the backend emits calls to the runtime's `op_*` functions
(`op_arr_alloc`, `op_sum_new`, `op_map_insert`, …) exactly where the wasm backend emits the imports — but
as **ordinary Rust function calls against the linked crate**, not wasm `call $import`. This is the
"ABI difference is 0" option: the generated Rust computes bit-identically to the wasm build because it
runs the *same runtime code*, including Perceus refcounting and CHAMP/RRB persistence.

The runtime is **already built for this** (see `spec/architecture/value-heap-runtime.md` and the runtime
crate). Its core is `Handle`-typed (a real `*mut Node`, `cdz-runtime/src/lib.rs:107`), and the entire
wasm ABI (u32 narrowing via `to_u32`/`from_u32`, the talc allocator, the wit-bindgen exports) is
quarantined behind `#[cfg(target_arch = "wasm32")]`. On a native target none of it compiles — you get
the pure `Handle`/`Box`/`Vec` core, which the runtime's own `#[cfg(test)]` suite already drives directly.
So "link the runtime and call it from Rust" is a path the runtime *already exercises*. The gaps are
mechanical and small:

1. **Visibility.** The core `op_*` functions and `Handle` are private (`fn op_*`, crate-internal). A
   linking backend needs them `pub` (behind a `pub` façade module, so the wasm `Guest` impl stays the
   only *other* consumer). Non-behavioral.
2. **`crate-type`.** `cdz-runtime/Cargo.toml` is `["cdylib"]`; add `"rlib"` to link it as a normal
   dependency.
3. **Pointer/immediate width.** The inline-value tagging uses a 30-bit fixnum window tuned for wasm32's
   32-bit pointers (`lib.rs` `FIXNUM_MIN/MAX`). Native linking removes the `u32` narrowing entirely, but
   the immediate window is canonical-form-critical and must be decided once for a 64-bit host (widen it,
   or keep the narrow window and box more — a runtime-team decision, filed for RUNTIME-REQUESTS).

**Recommendation.** Ship **Strategy A** first — it is what makes the output genuinely drop-in and it is
also *simpler* (no runtime dependency, no `pub` surface change, no fixnum decision, and the Perceus
`dup`/`drop` emission the wasm backend must do disappears because Rust's `Drop` handles it). Keep
**Strategy B** as a documented option for when a consumer needs bit-identical semantics or wants to stage
a migration where Cadenza and hand-written Rust share the exact same heap. The two strategies differ
*only* inside `rustgen`'s `Ty` → type map and its compound-construction emit; the front end, the seam,
and everything else are identical. A backend can even be parameterized on a `ValueStrategy` the same way
the pipeline is parameterized on `Target`.

### The generated type declarations

`Record` and `Sum` need nominal Rust declarations emitted once per distinct type. A `Sum{def,args}`'s
identity is `Arc::ptr_eq` on its `SumDef` (`ty.rs:287`), so the backend walks every `Ty` reachable in the
module, collects the distinct `SumDef`s and `Record` shapes, and emits:

- `Sum` → `enum <Name> { <Variant0>(payload…), <Variant1>, … }` from `SumDef.variants`
  (names + declaration-order discriminants live on `def`, `ty.rs:27`). `Option`/`Result` map to Rust's
  own `Option`/`Result` (they are ordinary prelude sums, but the std types are the ergonomic target).
- `Record` → `struct <Name> { <field>: <Ty>, … }`. The field list is kept sorted (`ty.rs:257`); the
  generated struct preserves the source field names.
- Parametric sums (`args` non-empty) → generic `enum <Name><T0, …>`, instantiated at use via the solved
  `args` on each `Ty::Sum`.

Naming: derive a deterministic Rust identifier from the `SumDef`/record shape (the sum's declared name;
a structural hash for anonymous records) so repeated references share one declaration.

---

## 5. Control flow and operators — the 1:1 print

Every `Mir` node has a direct Rust rendering. `emit_expr(&Mir, &Ctx) -> String` (or a `TokenStream` if
using `quote`/`prettyplease`, recommended — see §8) is a recursive tree-walk mirroring `select::emit`
(`rcdzc/src/select.rs:137`), but *printing Rust* instead of flattening to a stack:

| `Mir` (`ir.rs:616`) | Rust rendering |
|---|---|
| `Int(n)` / `Bool(b)` / `Unit` | `n_i64` / `true`/`false` / `()` |
| `Str(s)` | a string literal (Strategy A) / a `bytes-*` build (Strategy B) |
| `Local(id)` | the bound variable name for `id` (see α-naming below) |
| `Let{id,value,body}` | `{ let v{id} = <value>; <body> }` |
| `If{cond,then_,else_}` | `if <cond> { <then_> } else { <else_> }` |
| `Match{scrutinee,arms}` | `match <scrutinee> { <pat0> => <body0>, … }` |
| `Call{func,args}` | `<fn_name(func)>(<args>)` |
| `Arith`/`Bit`/`Shift`/`Cmp` | the Rust operator (**checked** — see below) |
| `Tuple`/`List`/`Map`/`Set` | a Rust literal/constructor (Strategy A) / runtime `op_*` calls (B) |
| `Proj{slot,operand}` | `<operand>.<slot>` (tuple) / `<operand>.<field>` (record) |
| `Sum{def,disc,payload}` | `<Enum>::<Variant>(<payload>)` (A) / `op_sum_new(disc, <payload>)` (B) |
| `Ctor`/`FuncRef`/`Intrinsic`/`Apply`/`Lambda` | see §6 (mostly fold away; declines where they don't) |
| `Trap(msg)` | `panic!(<msg>)` (or a generated abort — see checked arithmetic) |
| `TypeVal` | must not survive to here (erasure fence, `pipeline.rs:54`) |
| `Error(reject)` | a compile error was already emitted; unreachable in a successful compile |

**Match arms.** `rcdzc` has no `Pattern` enum — patterns are ordinary `Mir` trees (`ir.rs:381`): a
`Sum`/`Tuple` pattern, a literal, a `Local` binder, or `Wildcard`. The Rust backend prints them as Rust
patterns: `Sum{disc, payload: Local(id)}` → `<Enum>::<Variant>(v{id})`, `Wildcard` → `_`, a literal →
itself. This is *more* direct than the wasm backend, which lowers `match` to a `sum-disc` cascade of
`if`s (`select.rs:552`) — the Rust backend keeps it a real `match`, and gets exhaustiveness re-checked by
`rustc` as a bonus (Cadenza already checked it, CDZ0210, so this is belt-and-suspenders).

**Checked arithmetic — the one semantic subtlety.** Cadenza integer arithmetic **traps on overflow**,
and a compile-time-provable overflow already failed the build (CDZ0304, `pipeline.rs:40`). A runtime
overflow must trap, not wrap. So `Arith(Add, a, b)` emits **`a.checked_add(b).unwrap_or_else(|| <trap>)`**
(or `a + b` with `overflow-checks = true` guaranteed in the emitted crate's profile — but `checked_*` is
robust regardless of the consumer's profile and is the safer default). Likewise `BitOp::Div`/`Rem` emit
a divide-by-zero guard, and `Shift` masks/guards the shift amount, mirroring the ideal trapping sequences
the wasm backend hand-emits (`select.rs:990`, `:1045`). **Getting this right is the correctness heart of
the backend** — the mapping is trivial, the trap semantics are not, and the corpus's overflow/div0 cases
are the oracle. Note the `VarU64` spike specifically needs a **logical (zero-filling) right shift**
(`ShiftOp::Right` on an unsigned interpretation); emit `((x as u64) >> n) as i64` so a set bit 63 doesn't
sign-extend. (This also flags a real language question — is Cadenza's `>>` arithmetic or logical? —
worth pinning in the numeric model regardless of this backend.)

**α-naming.** The fold α-renames bound locals on every inline/β-reduce (the mandatory discipline noted in
the closures handoff), so `Local(id)`s are unique within a function after fold. Emit each as a fresh Rust
name `v{id}`; no scope-restore machinery needed (unlike `select`, which keys wasm slots by resolve-id).
This is *easier* in Rust than in wasm.

---

## 6. Functions, closures, and intrinsics

**Direct calls** are trivial: `Call{func,args}` → `fn_name(func)(args…)`, where `fn_name` comes from the
module's function table (the export name for exported fns, a generated `f{i}` for internal ones). No
index space, no `abs[]` — Rust resolves by name.

**Function values / closures.** `Fn` has no runtime representation yet (`ty.rs:246`): the fold resolves
every application to a direct `Call`, and a function value that would escape *declines*. The closures
design (ask-81, `open/P021-…`, summarized in `[[index-compiler-rewrite]]`) makes a `(fn …)` a transient
compile-time value that β-reduces at fold; Increment A (compile-time lambdas) closes the core with no
backend change. **For the Rust backend this is a gift:** if the fold has already reduced applications to
direct calls (Increment A), the backend never sees a surviving `Lambda`/`Apply` and needs no closure
support at all. A *surviving* runtime closure (Increment B) would map to a Rust `Box<dyn Fn>`/`impl Fn` —
but that is deferred in both backends identically, and until then it is the same honest decline. **The
Rust backend does not change the closure story; it inherits it.**

**Intrinsics** (`Int64.add`, `List.len`, `Bytes.of`, …) are prelude records of `Intrinsic` values lowered
at `select` today via `emit_intrinsic` (an id → wasm-instruction table). The Rust backend needs the
parallel table: id → Rust expression. Most are one-liners (`Int64.add` → checked add; `List.len` →
`.len()`; `List.concat` → `.extend`/`+`; `Bytes.at` → `.get(i).copied()` → `Option`). The intrinsics that
**decline in wasm because flat `Lir` can't loop** (`Bytes.of` over a non-literal list, `Set.of` over a
non-literal, `String.from-bytes`' UTF-8 loop emitted as a `RT_FIXED_FUNC`) become **ordinary Rust** and
the declines lift: `String.from-bytes` → `String::from_utf8(bytes).ok()`, `Bytes.of` → a `for` loop or
`.collect()`, `itoa`/`putu` → `format!`/`Display`. So the Rust backend is strictly *more* capable here
than the wasm backend at equal front-end maturity — a point worth making in the feasibility story.

---

## 7. Effects and the host boundary — the forward-looking half (async Rust + generated traits)

This is the part the user is most interested in ("use async Rust, generate traits for the host bindings")
and the part that must be **most honest about status: it is not buildable today, in either backend.**

**Current reality.** `rcdzc` has **no effects yet.** `(meta …)` — the capabilities/entry channel — is
parsed but *declines* (`resolve.rs:972`: "Realized with effects … a later phase, so DECLINE for now").
Effects are the NEXT major workstream (task #148) after first-class types (#150). The *seed* compiler
(`cdz-compiler`) has an effects lowering (Stages 0–3, `DESIGN-effects-lowering.md`) that targets wasm via
the component model + host suspend/replay; `rcdzc` has not yet built its equivalent. So this section
designs the Rust target for effects **as a requirement on the effects workstream**, not as something to
wire up now.

**The design, when effects land.** Cadenza's model is: `(effect …)` declares an interface
routing-agnostically; a `(host (Eff …) body)` delegation routes an effect to the host; the manifest is
the union of delegated capabilities, lowered to WIT for the wasm target. The Rust-target analogue is
clean and is exactly what the user proposed:

- **An effect declaration → a Rust `trait`.** Each `(effect E (op a → b) …)` generates
  `trait E { fn op(&self, a: A) -> B; … }` (or `async fn` — see below). The operations become trait
  methods with the mapped `Ty` signatures (§4). This is the direct analogue of "effect = WIT interface,
  op = function in it" (`DESIGN-effects-lowering.md` Stage 2), retargeted from WIT to a Rust trait.
- **A `(host …)` delegation → a trait bound the generated function takes as a parameter.** A Cadenza
  function that performs effect `E` compiles to a Rust `fn run<H: E>(host: &H, …)` (or a struct holding
  `H`), and each perform site becomes `host.op(args)`. The **host is dependency-injected**, which is
  precisely how Membrain's own gossip crate already threads its 30-method `Env` trait — so a
  Cadenza-generated module would compose with that pattern rather than fight it.
- **Async.** If the host trait methods are `async fn` (`trait E { async fn op(&self, …) -> B; }`), then a
  Cadenza function that performs an effect compiles to a Rust `async fn` that `.await`s the host call.
  This is the natural Rust encoding of Cadenza's suspend/replay effect semantics: **a perform is a
  suspension point; `.await` is Rust's suspension point.** Where the wasm target realizes suspend/replay
  by logging host calls and replaying (`host-calls-suspend-replay-from-host-log`), the Rust target
  realizes it by *being* an `async` function — the `Future` state machine `rustc` generates *is* the
  continuation. This is a strong structural match and is the reason the user's instinct ("use async
  Rust") is the right one.

**The gap that gates this — stated plainly.** The current effect lowering (both the seed's and any
`rcdzc` design) leans on **static, tail-resumptive** handlers (the handler is a compile-time constant,
resume is in tail position; `DESIGN-effects-lowering.md §1`). A tail-resumptive perform against a
host-delegated async method maps cleanly to `let x = host.op(args).await;`. The **general (non-tail)
one-shot continuation** — capture the rest of the computation as a value — does *not* have a trivial
async encoding and is a decline in the wasm target too (Tier 3, not landed). So the honest scope is:
**tail-resumptive host-delegated effects → async Rust traits is a clean, buildable design once effects
land in `rcdzc`; general delayed continuations are a research decline in both targets.** That is enough
for the effect shapes gossip actually needs (send/receive/timer/rng as host ops performed and awaited);
it is not a claim of full effpolymorphic continuations.

**Manifest.** The wasm target emits a WIT manifest (the capability union). The Rust target's "manifest"
is the *set of generated traits the consumer must implement* — emit it as a sidecar artifact
(`KIND_MANIFEST` or a doc comment listing the required trait impls) so the integrator knows the exact
host contract. Same union computation (`compute_manifest`), different rendering.

---

## 8. Implementation shape and the de-risking spike

**`rustgen` module layout** (parallels the wasm `select`+`serialize`+`heap`+`render`, but far smaller):

- `rustgen/types.rs` — walk the module's `Ty`s, emit `enum`/`struct` declarations (§4), build the
  `Ty` → Rust-type map for the chosen `ValueStrategy`.
- `rustgen/expr.rs` — `emit_expr(&Mir) -> TokenStream`, the tree-walk (§5), incl. the checked-arithmetic
  traps and the intrinsic table (§6).
- `rustgen/module.rs` — emit one `fn` per reachable function (signatures from `Layout::exports` /
  `MirFunc`), wire the crate scaffolding, drive the whole thing from `(MirModule, Layout)`.
- (later) `rustgen/effects.rs` — the trait generation of §7, gated on the effects workstream.

**Use `proc_macro2::TokenStream` + `prettyplease`** to build and format the output rather than
string-concatenation: it makes the emit code read like the tree it prints, gives free formatting, and
catches malformed output at construction. (`quote!` for the templates; `syn`/`prettyplease` to render.)
This mirrors the house preference for "emit code that reads like the structure" and avoids a
`render.rs`-style hand-rolled printer.

**The spike (recommended first increment), sequenced to prove the whole spine while touching the least:**

1. **Seam + scalar.** Add `Target::Rust`, the `KIND_RUST` artifact, and a `rustgen` that handles the
   scalar subset only (`Int`/`Bool`/`Unit`, `If`/`Let`/`Call`/`Arith`/`Cmp`, checked arithmetic). Compile
   `(def (main) (* 2 (+ 3 4)))` → a Rust `fn` returning `14_i64`; call it from a hand-written `main`.
   This validates the seam, the signature mapping from `ExportPlan`, and checked-arith trapping. Mirrors
   the wasm Phase 0 (`(def (main) 42)` → byte-identical) — same "smallest thing end-to-end" move.
2. **One compound (Strategy A).** Add `List`/`Bytes`/`Match`/`Sum` over native Rust types. Compile a
   function that takes and returns a compound, exercising the `Ty` → native-type map, generated `enum`s,
   and real `match`. This is the point where Strategy A's ergonomics (no handles) show.
3. **The real target — `VarU64`.** Reimplement Membrain's LEB128 varint (`core-wire/src/impl/varint.rs`,
   ~50 lines, encode/decode over bytes) in Cadenza, compile it to Rust, and **diff against Membrain's own
   byte-exact unit tests** (`round_trips_across_magnitudes`, `small_values_are_compact`,
   `rejects_truncated_input`, `rejects_overlong_encoding`) as the oracle. A byte-identical pass is the
   compelling feasibility result: a real production wire codec, authored in Cadenza, compiled to Rust,
   proven equivalent. (Watch the logical-vs-arithmetic right-shift, §5.) `Interval::subtract`
   (`component-replica/src/interval.rs`, Bytes + Option + List, eleven `subtract_*` tests) is the natural
   second target once the codec proves the pipeline.

This spike deliberately stays **inside the pure value language** — it needs no effects, no async, no
closures, no floats. It validates: the `Target` seam, `Mir` → Rust codegen, the `Ty` → type map (both
strategies if desired), checked-arithmetic trap fidelity, and boundary marshalling — the entire pure
spine — while the effectful/async/host-trait half (§7) waits on task #148. Do **not** aim a first spike at
gossip's `Client` (async, tokio, `#[derive(Wire)]`, the 30-method `Env`, Brazil packaging); aim it at a
pure, test-backed leaf function and let the compelling result argue for the larger investment.

---

## 9. Decline boundaries — what this design does NOT claim

Stated up front so no one reads more into it than is there:

- **No effects/host/async today.** §7 is forward design gated on task #148; `(meta …)` declines now.
- **No floats.** `Ty` has no `Float` (`ty.rs:235`) — anything statistical (the phi-accrual failure
  detector) is out of scope for *any* backend until the numeric model grows a float type.
- **No runtime closures** (Increment B) in either target — inherited decline, not a new one.
- **General (non-tail) continuations** — a research decline in both targets (§7).
- **Persistence semantics under Strategy A** — `Vec`/`BTreeMap` are not persistent; invisible for
  value→value functions, observable only for programs relying on cheap many-version sharing (§4). Strategy
  B (link the runtime) preserves it exactly.
- **Not a `Client`-level drop-in for Membrain gossip** — the realistic seam is a pure leaf function
  (varint, interval, reconciliation step), not the async/RPC/`Wire`/`Env`-welded `Client` crate.

The one load-bearing claim this doc *does* make: **the pure value→value spine — front end unchanged, a
`Target` seam at `pipeline.rs:70`, `Mir` → Rust via `prettyplease`, and either native types or the
already-Rust runtime linked directly — is feasible against the code as it stands today, and the varint
spike proves it.**
