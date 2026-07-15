# Design — inline (tagged-immediate) Handles for small scalars

**Author:** runtime engineer. **Audience:** compiler engineer (owns `cdz-compiler/`) + future me.
**Status:** proposal / design-space survey — **nothing is landed**. This doc explores ways to let a
`Handle` carry a small value *inline* (as a tagged immediate) instead of always pointing at a heap
`Node`. It does not pick a single implementation blindly; it lays out four families, folds in an
adversarial critique of each (defects kept, not laundered), and lands on a phased recommendation.

This is a survey with a recommendation: it captures the *reasoning* behind each fork, not just the
winning shape.

---

## 1. TL;DR — the win, the two walls, the pick

**The win.** Today every `bool`, `unit`, small `Int`, and nullary constructor that must live *inside*
a heap compound (a tuple/list/map/set element, a sum payload, a record field) costs a full heap
`Node` + a pointer chase. If a `Handle` could carry such a value in its own bits, those allocations
and derefs vanish for the most common leaves in the language.

**Wall 1 — the u32 ABI budget.** A handle is exactly 32 bits at the frozen component boundary (WIT
`u32`). After reserving discriminant bits, `bool` / `unit` / nullary-ctor discriminant / a *small-int
window* fit comfortably; **full `i64` and `f64` do not** — carrying those inline requires *widening*
the handle ABI to 64 bits, a frozen-contract break with envelope re-derivation.

**Wall 2 — the canonical-form tripwire.** The runtime is tagless and enforces *one representation per
value*: equal values must be structurally byte-identical (`champ_eq`/`champ_hash`/`champ_key_cmp`/
`render`), load-bearing for map/set keys and cross-boundary determinism. If the value `3` could exist
as **either** an inline handle **or** a boxed `Node`, the two would not compare equal — silently
corrupting map/set membership and breaking the frozen byte output. Any inline scheme lives or dies on
**normalize-on-construct** (a value that *can* inline is *never* boxed) plus **representation-agnostic
readers**.

**The pick (see §7).** A **low-bit tagged MVP**: inline `unit`, `bool`, and a small-int window using
the 2 guaranteed-zero low bits of a 4-byte-aligned `Node` pointer, encoded at the `Handle` level so
native and wasm share it. Defer nullary-ctor inlining (a structural change to sums). Keep a documented
evolution path to a `u64` NaN-box **only if** profiling shows big-int/float boxing dominates — it is an
XL frozen-contract break that still cannot inline full `i64`, so it is not the default.

---

## 2. Current state (ground truth, with line anchors)

All references are to `cdz-runtime/src/lib.rs` unless noted.

**Handle representation.** `struct Handle(*mut Node)` (`:107`), derives `Clone/Copy/PartialEq/Eq/Debug`.
The one reserved value is `Handle::NULL = Handle(null_mut())` (`:110`) — a benign sentinel that every
total read maps to a default (reads never trap on null). `Node` (`:84`) is
`{ rc: u32, handles: Vec<Handle>, raw: Vec<u8> }` — a **tagless** cell; the Cadenza type is
compile-time knowledge, never stored.

**The u32 ABI boundary.** `to_u32`/`from_u32` exist *only* in the `#[cfg(target_arch="wasm32")]` block
(`:1548`, `:1552`): `to_u32 = self.0 as usize as u32`, `from_u32 = Handle(x as usize as *mut Node)` —
pure, lossless pointer↔u32 reinterpretation (a wasm32 linear-memory `Node` address *is* 32-bit). WIT
(`cdz-runtime/wit/runtime.wit`) types every handle as opaque `u32`; the program never dereferences it.
The import set/order is frozen and baked into the component envelope (`heap_envelope.rs:24 mod himport`:
`BOX_INT=0`, `GET_INT=1`, …, `DUP=17`, `DROP=18`). **An inline scheme that only reinterprets bit
patterns the program already treats as opaque needs zero envelope change.** Natively (tests only) a
`Handle` is a 64-bit pointer and `to_u32`/`from_u32` do not exist.

**Tag budget from alignment.** `Node` is `#[repr(Rust)]` with no `#[repr(align)]`; its two `Vec`s make
it 3 pointers + a `u32`, so `align_of::<Node>() == 4` on wasm32 (≥8 native). Nodes are `Box::into_raw`
allocated via `alloc()` (`:142`) over the global allocator (talc on wasm, `allocator.rs:19`). talc
honors the `Layout` alignment (4) but gives no *stronger* guarantee — so **the low 2 bits are the
honest, portable, hard tag floor** (3 non-null tag values). A high-bit/MSB sentinel is *tempting*
(wasm linear memory is usually far below 2^31) but is **not a hard guarantee** (memory64-off wasm32 may
legally grow to 4GiB), whereas the low-bit alignment guarantee is enforced by the type.

**Where boxing happens.** Scalars are unboxed on the compiler's scalar path (Kind lattice,
`codegen.rs:96`: `Int64→i64`, `Bool→i32`, `Float64→f64`, `Unit→∅`). Boxing occurs *only* when a scalar
must become a child of a compound. Runtime producers: `op_box_int` (`:194`) → childless `Node`, `raw` =
8 LE bytes; `op_box_bool` (`:200`) → `raw=[v as u8]`; `op_box_float` (`:206`) → 8 LE bytes of f64 bits.
Readers: `op_get_int` (`:197`), `op_get_bool` (`:203`), `op_get_float` (`:209`). Unit = the empty tuple
`op_arr_alloc(0)`; a nullary sum variant is `op_sum_new(disc, unit)` (`:248`) — **two** nodes today.
The compiler gateway is `box_scalar` (`codegen.rs:8176`): `Int64→BOX_INT`, `Bool→BOX_BOOL`, and
**`Float64`/`Unit` currently `decline`** (`~:8194`) — so inlining `unit`/`bool` *extends* what the
compiler can box (pure capability gain). Emit sites that box: `codegen.rs:7979/8011/8048` (compound
element), `8292/8388` (ctor payload), and direct `BOX_INT` at `7091/7197` (`Bytes.at`, checked-arith).

**RC chokepoints (the design's structural strength).** RC funnels through four functions:
- `op_dup` (`:539`) — one deref (`h.0.as_mut()`).
- `op_drop` (`:566`) — **two** deref sites: root (`:567`) and worklist child (`:583`).
- `node_rc` (`:548`) — via `with_node`, returns 0 for null.
- `op_reset` (`:626`) — FBIP reuse-token minting; 3 borrow points.

`with_node` (`:156`) is the one *central* total deref, but the index accessors hand-roll their own
(`op_arr_get :232`, `op_arr_set :222`, `op_bytes_get :282`) and the collection walkers (vec spine
`~1142–1536`, CHAMP map/set `~2157–2799`, set algebra `~3302–3390`) do ~90 more. **RC balance itself is
gated by exactly the four functions**, but *reads* touch many more sites.

**FBIP/Perceus.** The `rc==1` predicate via `node_rc` authorizes unique-in-place vs shared path-copy
(call sites `1151/1184/1199/1220/1252/1273/1281/2461/2500/2786/3130/3208/3219/3246`). `op_reset` mints a
reuse token consumed by `op_arr_alloc_reuse` (`:659`) / `op_sum_new_reuse` (`:674`).

**Canonical form.** `champ_hash` (`:1873`) folds each node's `raw` + child hashes (FNV-1a);
`champ_eq` (`:1926`) compares `raw` + `handles.len()` recursively, with a pointer short-circuit
(`if x==y continue`, `:1929`); `champ_key_cmp` (`:1958`) total-orders by raw-lex → arity → children;
`render` (`:3451`) reads leaves via `op_get_*`. **None of these read `rc`.** The frozen contract
(`spec/contracts/deterministic-value-form.md`): each serializable value has *exactly one* canonical
byte encoding.

**Native vs wasm asymmetry (the sharpest practical hazard).** The whole heap core + all
`champ_eq`/`hash`/`render`/RC/FBIP tests run *natively* over a 64-bit `*mut Node`, with no
`to_u32`/`from_u32` in scope. Any inline discriminator defined only at the u32 seam is **not exercised
by the native suite**. To keep tests honest, inline encoding must live at the `Handle` level in *both*
builds — which the low-bit scheme can do (alignment ≥4 holds on both) and the MSB scheme cannot (native
pointers are unbounded).

---

## 3. The two hard constraints, stated up front

### 3.1 The u32 ABI budget

| Value | Bits needed | Fits inline in u32? |
|---|---|---|
| `unit` | 0 | Yes (trivially) |
| `bool` | 1 | Yes |
| nullary-ctor discriminant | small index | Yes (28–30 bits is ample) |
| small `Int` (fixnum window) | ≤ ~30 | Yes, a *subset* of `Int64` only |
| full `Int64` | 64 | **No** — needs a u64 handle |
| `f64` | 64 | **No** — needs a u64 handle |
| bytes / string / any compound | multi-word | Never — always heap |

Reserve 1–3 discriminant bits and ~29–30 payload bits remain. A **fixed** small-int window (always
inline, never boxed) is the clean rule; a range-conditional choice reintroduces the §3.2 hazard at the
boundary unless readers are representation-agnostic. Full `i64`/`f64` inline is *only* reachable by
widening the ABI to `u64` — every `-> u32`/`(handle: u32)` signature changes, the envelope re-derives,
and the value-interchange schema hash changes (a versioned break + migration).

### 3.2 The canonical-form tripwire (inline-3 vs boxed-3)

Because the runtime is tagless, `champ_eq`/`champ_hash`/`champ_key_cmp` read **structure**, not
identity. If `3` could exist as both an inline handle *and* a boxed `Node`:
- `champ_hash(inline-3)` would fold the empty/absent hash while `champ_hash(boxed-3)` folds 8 LE bytes
  → **different buckets**.
- `champ_eq(inline-3, boxed-3)` sees pointer-vs-Node (or `Some`/`None`) → **not equal**.

That silently corrupts map/set membership and breaks byte-identical output. **The only safe basis** is:

1. **Normalize-on-construct.** A value that *can* be inlined is *always* inlined and *never* boxed —
   the inline form *is* the canonical form. This is feasible because `op_box_int`/`op_box_bool`/the
   unit/nullary constructors are the *sole* leaf producers (the other ~40 `alloc` sites build headers,
   size tables, byte buffers, and compound shells — never a bare value leaf).
2. **Representation-agnostic readers.** Every reader (`op_get_*`, `champ_hash`/`eq`/`key_cmp`, `render`)
   must *decode* an inline handle into the *same* logical bytes a `Node.raw` would have held — belt-and-
   suspenders, so even a stray boxed twin from an un-migrated path or an older serialized value still
   compares equal.

**A critical correction the recon surfaced:** `with_node` has signature `f: impl FnOnce(&Node) -> T`
(`:156`). You **cannot fabricate a `&Node` for an immediate**, so `with_node` *cannot* be the decode
chokepoint — taught naively, it would just return its `default` (making `op_get_int(inline-3)` yield
`0`). Decode must therefore be a **bespoke `is_immediate` arm in each reader, placed *before* any
`as_ref`/`with_node`**. Calling `.as_ref()`/`.as_mut()` on a non-null immediate pointer is
**language-level UB**, not a benign wild read — the guard-before-deref is a hard correctness rule at
*every* reachable site.

---

## 4. The design space

Four families. Each subsection is design-level; implementation line numbers stay in §2. The
adversarial critique is folded in honestly.

### 4.1 Low-bit tagged Handle (fixnum window) — *family: lowbit-u32*

**Encoding.** Discriminant = `h & 0b11` (real pointers are 4-aligned → low 2 bits zero; `NULL=0` is
tag `00`). `00` = pointer/NULL; `01` = 30-bit signed fixnum (`(h as i32) >> 2`); `10` = atom (sub-kind
in bits[3:2]: unit / bool-value-in-bit[4] / nullary-disc in bits[31:4]); `11` = reserved.
`is_immediate = (h & 0b11) != 0`. Negative fixnums round-trip via arithmetic shift and match
`op_box_int(-1)`'s bytes. Must live on `Handle.0`'s low bits, not only at the u32 seam.

**Inlines.** `unit`, `bool`, nullary-ctor disc, small signed int in ~`±2^29`. Stays boxed: `Int64`
outside the window, `f64`, all compounds.

**ABI stance.** Fits u32 **as-is**. Zero WIT / himport / envelope change; `to_u32`/`from_u32` preserve
the low bits.

**RC/FBIP.** Guard the 4 RC ops (7 deref sites) with `is_immediate` *before* the deref: `dup`=no-op,
`drop`=no-op at both sites (the worklist child at `:583` must be filtered — `as_mut` on `(v<<2)|1`
returns `Some` then reads garbage = UB), `node_rc`=2 (forces conservative FBIP path-copy),
`reset`=NULL. Verified: the FBIP in-place helpers store/read children opaquely and route releases
through the guarded ops, so no spine path wild-derefs an inline child once the 4 ops are guarded.
LIVE_NODES strictly improves.

**Canonical form.** Normalize-on-construct holds (`op_box_int` is the sole int-leaf producer). Readers
need **bespoke** decode arms (see §3.2 correction) — *not* centralized in `with_node`. The two-rep
hazard closes; `champ_eq`'s `x==y` short-circuit is a real speed win for bit-identical immediates.

**Compiler coordination.** Runtime stays correct even for a naive compiler (`op_box_int` normalizes
regardless). Point `box_scalar`'s Unit arm at the inline constant (capability gain). *Optional* peephole:
emit an in-window literal as `(v<<2)|1` directly — but this is a **correctness-sensitive** path (must
emit *exactly* the tagged bits, or it forks the representation).

**Pros.** Zero ABI change; **identical encoding on wasm and native** (rides alignment ≥4 on both) so
the native suite exercises the shipped rep; saves a Node + chase per small int/bool/unit; RC free for
immediates; extends `box_scalar` to accept `unit`.

**Cons / risks (honest).** The "with_node centralizes decode" story is **wrong** — decode is duplicated
across ~10 functions (`op_get_int/bool/float`, `op_arr_len`, `op_sum_disc/payload`, `champ_*`, `render`
leaf arms). The totality contract widens the surface: cross-kind reads (an int handle fed to
`op_get_float`/`op_arr_len`/`op_sum_disc`, exercised by the totality test) each need a guard or they
wild-deref. **Test-side impact is a correctness change, not a rebaseline:** `rc_of` does unconditional
`*h.0` and `node_born_with_refcount_one` asserts `rc_of(op_box_int(5))==1` — both crash on an immediate
and must be rewritten, along with dozens of `rc==1`/node-count asserts. Only a 30-bit window inlines.
Nullary-ctor inlining changes sum structure (`sum_disc`/`sum_payload`/`render` must synthesize disc +
unit child) — **worth deferring**. **Corrected blast radius: L** (not the M the naive framing suggests),
XL if nullary-ctor is included.

### 4.2 u64 NaN-box unified Handle — *family: widen-u64-nanbox*

**Encoding.** Widen `Handle` to `u64` in both builds; NaN-box on IEEE-754 double.
`BOX_PREFIX = 0xFFF8_0000_0000_0000`; `is_boxed = (h & PREFIX) == PREFIX`. **Inverted discriminator
(a trap):** `!is_boxed` means a *genuine f64 value* (do **not** deref), not a pointer; the pointer case
is tag `000` *within* `is_boxed`. `box_float` **must canonicalize every NaN** to `0x7FF8…` (sign 0) or a
negative qNaN aliases `BOX_PREFIX` → wild deref. Tags in bits[50:48]: PTR / FIXNUM(48-bit) / BOOL / UNIT
/ NULLARY.

**Inlines.** Full `f64`, a `±2^47` fixnum window, `bool`, `unit`, nullary disc. Still **not** full
`i64` (ints beyond `±2^47` box).

**ABI stance.** Does **not** fit u32 — requires widening every handle-typed WIT signature to `u64`,
re-deriving the envelope (hash changes), bumping the value-interchange schema hash, and migrating
persisted bytes. **XL frozen-contract break.**

**RC/FBIP.** Same 4-op guard shape as §4.1 plus the ~30 hand-rolled deref sites (some in hot loops) each
gaining a per-visit `is_boxed` branch. Reuse trio falls through on immediates.

**Canonical form.** Two omissions the critique caught: (1) **nullary is not runtime-decidable** — the
tagless `op_sum_new` cannot tell a nullary variant from `Variant(unit)`, so a boxed `None` (2-node, has
a unit *child*) vs inline `NULLARY` (no child) hits `champ_eq`'s arity check → not equal → key
corruption; inlining nullary needs **compiler** special-casing at every construction site. (2) The
proposal's optional `-0.0 → +0.0` canonicalization **contradicts** existing `champ_eq` behavior
(`:1921` treats `-0.0 ≠ 0.0`) and must **not** be taken.

**Compiler coordination.** Regenerate the ABI/envelope; `box_scalar` gains Float64+Unit; **compiler
must special-case nullary at every site**; any compile-time constant materialized as a data-segment Node
must obey the inline rule or a boxed twin leaks.

**Pros.** Full `f64` inline (an *optimization* — float-in-compound already works via heap `op_box_float`);
~48-bit fixnum window; one uniform representation.

**Cons / risks (honest).** XL break for what is mostly an optimization; **still no full `i64`**;
`Handle` doubles 4→8 bytes → **every `Node.handles` child grows on wasm32**, plausibly *erasing* the
alloc savings on child-heavy CHAMP/RRB structures. The inverted discriminator invites wild-derefing
every float; missed NaN canonicalization → heap corruption; **native pointers are not guaranteed
≤2^48** (needs a fallible box that traps — violating the never-trap discipline — or a custom low-address
native allocator). **Not worth it** as a default.

### 4.3 Slot-packed self-describing containers — *family: slot-directed*

**Encoding.** `Handle`/`Node` unchanged; scalar *slots* become self-describing bytes inside the owning
node's `raw` behind a FORM+descriptor prefix. Type-directed: a scalar is packed only when its **static
slot type** is a concrete scalar.

**Inlines.** `unit`, `bool`, **full `Int64` / `IntN` / full `f64` / char** — no 30-bit cap, because
storage is raw bytes, not a 32-bit handle. Stays boxed: sum/Any/record/tuple/list/map/set/bytes/string.

**ABI stance.** Safe at u32 (containers still cross as one pointer; packed scalars ride in `raw`, never
as handles — full `i64`/`f64` survive with no truncation). Cost: **append-only** new WIT imports
(packed construct + typed slot getters), one envelope re-derivation, stable-binary republish.

**RC/FBIP.** **Zero RC change** — a packed scalar is never a `Handle` and never in `handles`, so it can
never reach an RC op (copied by value with `raw`). This is a genuine advantage. FBIP: a packed leaf is
childless → an ideal reuse token; rc==1 slot update becomes a raw byte write. New obligation: reuse
constructors must rewrite FORM/descriptor.

**Canonical form.** The homogeneity argument holds *within* one CHAMP collection, **but** "representation
= pure function of static type" does **not** hold globally in four places the critique proved: (1)
**monomorph/Any boundary** — `3` packs in `list<i64>` but boxes in `list<Any>`; a migrating value
meeting as a key breaks `champ_eq`/`champ_hash`; (2) the **value-interchange decode path** must rebuild
the packed form (else a boxed twin); (3) **RRB concat** of a packed leaf and a boxed leaf of equal
contents is not `champ_eq` to a fully-packed list; (4) `op_sum_payload` returns `handles.first()` — a
packed payload has no handle → `op_get_int(NULL)` silently reads 0. The belt-and-suspenders net is
**largely illusory** here (a boxed child lives in `handles`, a packed scalar in `raw` — different
structural positions).

**Compiler coordination.** Heavy: packed store/load at every container family, stop emitting
`op_sum_payload` for packed payloads, **thread per-slot concrete type through monomorphization**,
migrate the decode/host-import emitters, and rewrite CHAMP key/value emission (see below).

**Pros.** RC untouched; ABI round-trip safe; **wasm/native identical** (raw:Vec<u8> in both builds) so
the native corpus exercises the shipped rep; inlines full `i64`/`f64`/`IntN`/char for concrete
homogeneous collections; extends `box_scalar` to Float64+Unit.

**Cons / risks (honest).** **CHAMP-key packing is oversold** — there is *no* entry node; keys/values are
popcount-indexed handles inside the bitmap node, so packing scalar keys **rewrites the core CHAMP index
invariant** (or you keep CHAMP keys boxed and forfeit the advertised `map<i64,i64>`-flat win). Nodes
become self-describing (descriptor in `raw`) — a departure from pure-tagless minimalism, variable-offset
slot access. **Corrected blast radius: XL** (CHAMP-index rewrite + decode-path migration + monomorph
descriptor threading), and every value with a scalar element changes its canonical bytes → a versioned
deterministic-value-form amendment + whole-corpus golden re-baseline.

### 4.4 Immortal singleton pool (interned boxed leaves) — *family: interning*

**Encoding.** **No handle/ABI change.** Pre-allocate one shared, never-freed `Node` per cheap common
value (unit, true/false, a small-int window e.g. `[-128,255]`, nullary disc `0..=63`). `op_box_*`
returns the shared `Node`. Mortal/immortal lives in the existing `rc: u32` via `IMMORTAL = u32::MAX`,
read only in the RC chokepoints.

**Inlines.** Nothing — this **interns** (dedups) boxed forms. The `Handle` stays a pointer; the pointer
chase remains.

**ABI stance.** Zero change — fits the frozen u32 with no re-derivation (only *which* Node address
`op_box_*` returns changes, and that is opaque).

**RC/FBIP.** Guard `op_dup`/`op_drop` (both decrement sites) with `rc != IMMORTAL`. `node_rc` needs no
guard (naturally returns `u32::MAX`). **Correction the critique caught:** `op_reset`'s `rc > 1` branch
**decrements** the sentinel (`n.rc -= 1` at `:634`) — so `op_reset` needs its *own*
`if rc == IMMORTAL { return NULL }` early-return. Correct guard count is **4** (dup + drop×2 + reset),
not the "5 incl. node_rc" the proposal claimed.

**Canonical form.** **Verified safe with no reader changes** — a pooled value is a real `Node`
byte-identical to what `op_box_*` produces; the readers never touch `rc`. No inline-vs-boxed hazard;
normalize is a churn win, not a correctness requirement.

**Compiler coordination.** Runtime-only for correctness. Optional: drop the Unit `box_scalar` decline.

**Pros.** Zero ABI change; canonical form free; wasm/native bit-identical; small blast radius; cuts
allocation churn for the hottest values.

**Cons / risks (honest).** **Off-goal** — it does not achieve the doc's objective at all: the Handle
stays a pointer, no value is carried without a Node, `get_int` still dereferences. It saves *allocation*,
not *indirection*. The `op_reset` sentinel-decrement bug must be fixed or the immortal flag is corrupted
after the first reset of any pooled value. Pooling **masks** compiler Perceus dup/drop imbalance bugs on
pooled values (no-op dup/drop absorb them) — a testing blind spot. Reserving `rc==u32::MAX` shrinks the
max legal refcount by one. **Best framing: a low-risk *complement*, not a substitute.**

---

## 5. Comparison matrix

Scoring: ✅ good / clean, ⚠️ caveated, ❌ bad / blocked. Blast radius S < M < L < XL.

| Axis | 4.1 Low-bit tag | 4.2 u64 NaN-box | 4.3 Slot-packed | 4.4 Singleton pool |
|---|---|---|---|---|
| **Fits u32 ABI** | ✅ as-is, zero change | ❌ requires u64 widen + envelope re-derive | ⚠️ append-only imports, one re-derive | ✅ zero change |
| **Types inlinable** | unit, bool, nullary, ±2^29 int | + full f64, ±2^47 int (still not full i64) | ✅ full i64/f64/IntN/char (concrete slots) | none (interns boxed) |
| **RC / FBIP impact** | ⚠️ 4 ops guarded (7 sites), + read guards | ⚠️ 4 ops + ~30 sites, per-child branch in hot loops | ✅ **zero RC change** | ✅ 4 guards, tiny |
| **Canonical-form risk** | ⚠️ closes under normalize + bespoke decode | ⚠️ nullary not runtime-decidable; NaN/-0.0 traps | ❌ 4 coexistence leaks (monomorph/decode/RRB/payload) | ✅ **free, no reader change** |
| **Compiler coordination** | ⚠️ optional peephole (correctness-sensitive); Unit arm | heavy: ABI regen + nullary special-case | heavy: monomorph type-threading + CHAMP-index rewrite | ✅ runtime-only |
| **Blast radius** | **L** (XL w/ nullary) | **XL** | **XL** | **S/M** |
| **Perf upside** | Node+chase saved for small int/bool/unit | + float chase saved, but Handle doubles → children grow | full-width scalars flat in raw (biggest density win *where applicable*) | alloc churn only, **no chase saved** |
| **wasm/native parity** | ✅ identical (alignment ≥4 both) | ⚠️ native ptr may exceed 2^48 | ✅ identical (raw both) | ✅ identical |
| **Achieves the goal?** | ✅ yes (partial value set) | ✅ yes (widest, at XL cost) | ✅ yes (full scalars, container-scoped) | ❌ no (dedup, not inline) |

---

## 6. Cross-cutting truths

- **`with_node` cannot be the decode chokepoint** for any handle-tag scheme (§3.2) — it takes
  `FnOnce(&Node)` and cannot fabricate a `&Node`. Every reader needs a bespoke `is_immediate` arm
  *before* the deref. This single misconception is the most likely implementation trap.
- **`.as_ref()`/`.as_mut()` on a non-null immediate is UB**, not a benign wild read — guard-before-deref
  is mandatory at *every* reachable site, including the ~90 collection-walker derefs (most operate on
  internal trie nodes that are never inline, but that must be *proven per-site*, not assumed).
- **Native tests validate a different representation** unless inline encoding lives at the `Handle`
  level in both builds. Low-bit (§4.1) and slot-packing (§4.3) satisfy this; MSB and u64-NaN-box carry a
  wasm-only or native-allocator-constrained hazard.
- **The value-interchange / serializer path is an easily-omitted canonical surface.** Any path that
  copies `node.raw` directly (rather than reading via `op_get_*`) serializes an inline int as empty
  bytes → breaks deterministic-value-form and the schema-hash envelope. Must be audited.

---

## 7. Recommendation

**Land the low-bit tagged MVP (§4.1), scoped tightly; keep §4.4 as a cheap complement; defer §4.2 and
§4.3 behind profiling.**

**Why §4.1 as the MVP.** It is the only family that (a) achieves the actual goal — carrying a value
without a Node — while (b) requiring **zero ABI/WIT/envelope change** and (c) using **the one hard tag
guarantee** (4-byte `Node` alignment) rather than the unenforced sub-2GB MSB assumption, and (d) sharing
one encoding across wasm and native so the existing test corpus exercises the shipped representation.
The MSB variant buys 28 payload bits over the low-bit 30 in the reserved-tag layout — a *worse* trade
that adds an unenforced memory-bound invariant; prefer low-bit.

**MVP phasing (each phase independently shippable and gate-green):**
- **Phase 1 — `unit` + `bool`.** Pure capability gain (`box_scalar` declines Unit today), *zero*
  dual-representation risk. But it is **not** "runtime-only, small": the moment a `bool` leaf inlines,
  `champ_hash`/`eq`/`key_cmp`, `render`, and every reachable child deref must become inline-aware **in
  the same change**, or the first `bool`-keyed map corrupts and reachable derefs are UB. Realistic size:
  **M**.
- **Phase 2 — small-int window.** Add the `±2^29` fixnum with normalize-on-construct in `op_box_int`
  (range-branch: inline in-window, else alloc) and representation-agnostic decode arms. Defend the
  boundary at exactly `±2^29` (a value that sometimes inlines, sometimes boxes, must still `champ_eq` its
  twin via rep-agnostic decode). Fix the totality-read surface (`op_get_float`/`op_arr_len`/
  `op_sum_disc`/`op_sum_payload` on a cross-kind immediate). Rewrite the native `rc_of` / node-count /
  `rc==1` assertions to be immediate-aware. **L.**
- **Defer — nullary-ctor inlining.** It changes sum structure (`sum_disc`/`sum_payload`/`render` must
  synthesize disc + unit child; a boxed nullary twin must `canon_view`-equal the inline form). Adds real
  surface and pushes toward XL. Land only after Phases 1–2 are proven.

**§4.4 as a complement, not a substitute.** If profiling shows allocation *churn* (not chase) dominates
before the MVP lands — or for values the tag cannot carry (e.g. large ints that recur) — the immortal
pool is an S/M, canonical-form-free win that composes cleanly with §4.1. Fix the `op_reset`
sentinel-decrement first.

**Evolution path to §4.2 (u64 NaN-box) — only if profiling justifies it.** If, after the MVP, profiling
shows big-int (`>2^29`) or `f64` boxing is a top allocation/chase cost, revisit the u64 widening. It is
an XL frozen-contract break (envelope + schema-hash re-derivation + migration) that *still* cannot inline
full `i64` and doubles child-handle size — so it must clear a real, measured bar. The reserved low-bit
tags (`0b11`) and representation-agnostic readers built in the MVP make this a *widening*, not a
*redefinition*, of the immediates already shipped.

**§4.3 (slot-packing) is the right answer for a different problem** — dense homogeneous
`list<i64>`/`tuple`/`record` where full-width scalars must be flat. It is orthogonal to the handle-tag
MVP and could be layered later, but its CHAMP-index rewrite and monomorph type-threading make it XL and
out of scope for a first inline-handle landing.

---

## 8. Open questions / validate before committing

1. **Reader-decode audit.** Enumerate *every* `as_ref`/`as_mut`/`with_node` reachable by an inline
   handle (RC ops, `op_arr_get/set`, `champ_hash/eq/key_cmp`, `render` leaf arms, and the ~90 collection
   walkers) and confirm each either cannot see an immediate (internal trie node) or guards
   `is_immediate` *before* the deref. A single miss is UB.
2. **Serializer path.** Does any value-interchange / Ast serialization path copy `node.raw` directly
   rather than reading via `op_get_*`? If so it must be routed through the decode arm, or inline ints
   serialize as empty bytes and break the schema hash. (Highest-severity, easily missed.)
3. **Native test rewrite scope.** `rc_of` (`:3523`) does unconditional `*h.0`;
   `node_born_with_refcount_one` (`:3785`) asserts `rc_of(op_box_int(5))==1`. Triage the full native
   suite for `rc==1` / exact-node-count assertions that assume every leaf is boxed — this is a
   correctness rewrite, not a rebaseline.
4. **Fixnum window width.** 30 bits (1 tag bit) vs 28 bits (3-bit tag, room for more atom sub-kinds).
   Pick before Phase 2 — it is baked into canonical form via the range-branch (though rep-agnostic
   readers make widening it later non-breaking).
5. **Compiler peephole go/no-go.** Is eliding `dup`/`drop` and emitting `(v<<2)|1` directly worth the
   correctness risk (must emit *exactly* the tagged bits), or is the runtime no-op sufficient for v1?
6. **`node_rc(immediate)` sentinel value.** Confirm it returns a value `≠ 1` (e.g. `2`) at *every* FBIP
   call site — an accidental `1` would let an FBIP path attempt in-place mutation of a non-Node.
7. **Totality contract.** Confirm the cross-kind reads exercised by the totality test
   (`op_get_float`/`op_arr_len`/`op_sum_disc` on an int immediate) are each guarded — the proposal's
   original list only covered `op_get_int`/`op_get_bool`.
8. **Interaction with the stable binary.** A stored value from an older stable binary may contain a
   boxed small-int; confirm representation-agnostic decode makes it `champ_eq` its inline twin (this is
   the whole point of belt-and-suspenders — validate it with an explicit test).
