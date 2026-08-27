# Design — cadenza-ast: cheap-clone leaves, zero-copy decode, and extension-trait consolidation

**Author:** v-syntax (owns cadenza-ast), co-owned with v-core-opt (owns rcdzc). **Audience:** anyone
touching the AST leaf/arena representation, the binary codec decode path, or the rcdzc AST copy.
**Status:** DESIGN — direction settled with the operator (seq378 extension-traits; seq380
decode-takes-Bytes + per-variant leaf repr). Increment 1 is in flight; increments 2/3 + consolidation
are scoped here.

Origin: the operator-commissioned cheaply-clonable audit (v-core-opt finding) — the whole `Leaf`
(String included) is `.clone()`d per-node on every arena→arena copy (codec encode/graft, `canon`,
`Tree::from_arena`/rewrite-fixpoint, `cdz-corpus::clone_into`, whole-`Arenas` `Clone`). `Leaf::Name` is
every identifier/head/segment — a name used 500× was 500 `String` allocs.

---

## 0. The three threads, and how they compose

Three operator-blessed changes to the AST representation, deliberately layered so each lands on its own:

1. **Cheap-clone leaves** (increment 1, IN FLIGHT) — the string-payload `Leaf` variants
   `Str`/`Sym`/`Name`/`BadChar` become `Arc<str>` (was `String`). Every per-node leaf clone is a
   refcount bump; interned names share one alloc.
2. **Zero-copy decode** (increment 2, reshaped by seq380) — the codec `decode` entrypoint takes a
   `bytes::Bytes`, and the leaf payloads that are byte-slices of the input decode as **zero-copy
   sub-slices** (`Bytes::slice` = refcount bump) instead of `String::from_utf8(bytes.to_vec())` per leaf.
3. **Extension-trait consolidation** (increment 3 / structural) — collapse rcdzc's diverged `ast.rs`
   copy onto a shared cadenza-ast structural core, with rcdzc extending only the leaf-VALUE type.

**Key invariant across all three: NO wire/header change, NO hash bump.** The frozen `cdzast\x00\x01`
codec reads/writes identical bytes; only the in-memory *representation* of a decoded leaf changes. This
makes every increment a free-to-land-anytime class, not a batched hash-bump.

---

## 1. The per-variant leaf representation (the seq380 nuance)

The one-size answer is wrong: **zero-copy-slice and `Arc<str>`-interning pull opposite ways.**

- An `Arc<str>` leaf: cheap to clone (refcount), and **deduplicated** via the arena's `name_index` — 500
  occurrences of `x` share ONE allocation. Best for a **repeated, interned** payload.
- A `Bytes`-slice leaf: zero-copy on decode (a sub-slice of the input, refcount bump, no alloc/copy), but
  **not deduped** (500 occurrences = 500 slices) and it **holds the whole input `Bytes` alive** as long
  as any leaf survives. Best for a **usually-unique literal** payload where dedup doesn't pay and the
  input is transient.

So the representation is chosen PER VARIANT by usage:

| Leaf variant | Repr | Rationale |
|---|---|---|
| `Name` | `Arc<str>` (interned) | every id/head/segment, HIGHLY repeated — interning dedup wins, and `Arc<str>` is cheap-clone. |
| `Sym` | `Arc<str>` (interned) | symbol content, interned like a name. |
| `BadChar` | `Arc<str>` | a rare marker — `Arc<str>` for uniformity, no zero-copy need. |
| `Str` | zero-copy (`Bytes`-backed, validated UTF-8) | literal payload, usually unique — zero-copy decode beats interning. |
| `Bytes` | zero-copy (`bytes::Bytes`) | raw byte literal, usually unique — the canonical zero-copy case. |

Increment 1 (in flight) converts Name/Sym/Str/BadChar → `Arc<str>` as the *cheap-clone-now* step; the
seq380 reshape then moves **Str + Bytes** to the zero-copy `Bytes`-backed repr in increment 2 (Str's
`Arc<str>` from increment 1 becomes a validated-UTF8 `Bytes` newtype). Name/Sym/BadChar stay `Arc<str>`.

---

## 2. Increment 2 — decode-takes-`Bytes`, zero-copy Str/Bytes

### The change

- **`codec::decode`/`decode_detailed`** take `bytes::Bytes` (not `&[u8]`/`Vec<u8>`).
  A caller with a `Vec<u8>` gets one `Bytes::from(vec)` at the boundary (the LAST copy); every leaf after
  is a slice of it.
- **`leb128::Reader`** carries the `Bytes` (or a cursor + a handle to it) so `take(len)` can return a
  `Bytes::slice(pos..pos+len)` — a refcount bump, not a copy.
- **`read_string`** (today `String::from_utf8(bytes.to_vec())`) → validate the slice is UTF-8
  (`std::str::from_utf8(&slice).is_ok()`, the SAME check, still rejecting `BadText` on invalid — decode
  stays total + the one-canonical-byte-form bijection holds), then wrap the slice unchecked as the
  validated-UTF8 `Bytes`-str newtype. NO `.to_vec()`.
- **`read_raw_bytes`** (today `Vec<u8>`) → return the `Bytes` slice directly.

### The validated-UTF8 `Bytes`-str newtype

`Str` needs to be a `str` (UTF-8) but backed by a zero-copy `Bytes`. Define a small newtype in
cadenza-ast:

```rust
/// A UTF-8 string backed by a (possibly shared, zero-copy) `bytes::Bytes`. Constructed only after a
/// `std::str::from_utf8` check, so `as_str()` is sound. `Clone` is a refcount bump. Encodes byte-
/// identically to the string content (the codec writes the bytes; representation is invisible to it).
#[derive(Clone)]
pub struct ByteStr(bytes::Bytes); // invariant: .0 is valid UTF-8
impl ByteStr {
    pub fn from_utf8(b: bytes::Bytes) -> Result<ByteStr, ()> {
        std::str::from_utf8(&b).map(|_| ()).map_err(|_| ())?; Ok(ByteStr(b))
    }
    pub fn as_str(&self) -> &str { unsafe { std::str::from_utf8_unchecked(&self.0) } }
}
```

`Leaf::Str(ByteStr)`, `Leaf::Bytes(bytes::Bytes)`. `PartialEq/Eq/Hash` for `ByteStr` delegate to
`as_str()` (so leaf dedup + `structurally_eq` are unchanged). `Debug` prints the str.

### Why no hash bump

`codec::encode` already writes the string/byte CONTENT (`write_bytes(len, &content)`); it reads `.as_str()`
/ `&bytes[..]` — identical for `String` vs `ByteStr` vs `Bytes`. The byte-stability guard
(`v1_canonical_bytes_are_unchanged`) confirms it; the round-trip + never-panic fuzzes confirm decode
totality. Verified for increment 1 (`Arc<str>`); the same holds for the `Bytes`-backed reprs.

### The dep

Adds `bytes` to cadenza-ast (dep-floor-zero today). Justified (concierge seq380): `bytes` is already
fleet-wide (cdz-kernel/cdz-agent-host), and the bottom syntax crate taking it for alloc-elimination on
the hottest decode path is a real win, not bloat.

---

## 3. Increment 3 / structural — extension-trait consolidation (seq378)

### The problem

Only `rcdzc` carries a diverged `ast.rs` (1437 vs cadenza-ast's ~1205 lines). It is NOT stale debt: it
is **eval-bearing** — its `Leaf::Int`/`Float` hold a custom `IntValue` (add/sub/mul/divmod/gcd/wrap_to/
fits_width) + `Decimal` f64 methods, i.e. the compiler's constant-fold/eval arithmetic, where cadenza-ast
(pure syntax/codec) holds `num_bigint::BigInt` + a plain `Decimal` (NO arithmetic, arbitrary-precision
for faithful round-trip). Same structural `Struct`/arena; same 10 leaf variants (rcdzc drops `Suffixed`,
decoding suffixes straight to Int/Float). (cdz-kernel already DEPENDS on cadenza-ast; cdz-runtime/cdz-wasm
have no ast.rs — so rcdzc is the only consolidation target.)

### The approach (operator: "extension traits is probably the way to go")

A FULL merge onto one `Leaf` is wrong layering (cadenza-ast's pure-syntax leaf would gain compiler eval
arithmetic). Instead:

- **cadenza-ast owns the STRUCTURAL core**: `Struct`/`StructId`/`Builder`/the arena/the codec, generic
  (or trait-bounded) over the leaf-value type where the value representation differs.
- **rcdzc extends only the leaf-VALUE** via a trait/newtype: its `IntValue`/eval `Decimal` implement a
  cadenza-ast `LeafValue` trait (or rcdzc wraps cadenza-ast's structural arena with its own leaf pool).
  The eval arithmetic stays rcdzc-local; the structure + codec are shared.
- **Payoff**: a STRUCTURAL change (e.g. `Struct::List(Vec<StructId>) → Arc<[StructId]>`, the last
  cheap-clone slice) lands ONCE in cadenza-ast instead of being hand-mirrored into rcdzc's copy.

### The seam — DECIDED (b), with v-core-opt (rcdzc owner), 2026-08-10

Two candidates were weighed: (a) cadenza-ast's arena generic `Arena<L: LeafValue>` (rcdzc instantiates
with its eval-leaf, cadenza-syntax with the syntax-leaf) — most sharing, biggest refactor; (b) cadenza-ast
exposes the structural arena + codec as-is over its OWN `Leaf`, and rcdzc keeps a THIN LOCAL leaf but
reuses the `Struct`/codec via a conversion at the boundary — less sharing, smaller change.

**DECIDED: (b).** v-core-opt (rcdzc owner) strongly prefers it, and it is the correct layering:

1. rcdzc's `Leaf` is EVAL-BEARING — `Leaf::Int` carries an `IntValue` (arbitrary-precision bignum) that
   the const-fold path does real arithmetic on (`add`/`mul`/`wrap`/`divmod`); cadenza-ast's `Leaf` is a
   pure syntactic carrier. A generic-leaf merge (a) would drag eval semantics into the syntax crate (or
   force the trait to abstract over arithmetic) — the wrong layer, coupling the front-end to the
   compiler's numeric model.
2. The operator's no-adapter/full-collapse directive applies only where two things are genuinely ONE. A
   syntactic leaf and an eval-bearing leaf are DISTINCT responsibilities — so (b) shares the STRUCTURAL
   core (`Struct`/codec/arena) where they truly coincide and keeps each `Leaf` local where they don't =
   correct layering, not an adapter.
3. (b) is the smaller, lower-risk refactor and still delivers the win: `Struct::List → Arc<[StructId]>`
   (the last cheap-clone slice) lands ONCE in the shared structural core; rcdzc inherits it via the
   boundary conversion instead of re-implementing it in its diverged copy — the "lands once not twice"
   outcome.

**Ownership split:** the shared structural core (cadenza-ast: `Struct`/`StructId`/`Builder`/arena/codec,
+ the `Struct::List → Arc<[_]>` cheap-clone slice) is v-syntax's; each crate keeps its own `Leaf`
(rcdzc's eval-bearing, cadenza-ast's syntactic) behind the boundary; v-core-opt owns the rcdzc side of
the boundary conversion. (v-core-opt is separately mid-flight on rcdzc Core-IR cheap-clone slices —
compiler-internal, hash-neutral, landing freely; the ast.rs structural-core work is this joint arc.)

---

## 4. Increment order + land discipline

1. **Increment 1** (Name/Sym/Str/BadChar → `Arc<str>`) — IN FLIGHT, held for an atomic cross-lane land
   with cdz-kernel's use-site patch (v-ah standing delegation), sequenced AFTER v-ah's B2 lands (B2
   rewrites the overlapping `wasm_host.rs`). No wire change.
2. **Increment 2** (decode-takes-`Bytes`; Str→`ByteStr`, Bytes→`bytes::Bytes` zero-copy; `leb128::Reader`
   over `Bytes`) — after increment 1 lands. Adds the `bytes` dep. No wire change. Same cross-lane
   lockstep pattern with cdz-kernel (Str/Bytes use-sites) under the standing delegation.
3. **Increment 3 / consolidation** (seam (b) DECIDED, §3; `Struct::List → Arc<[StructId]>` lands once in
   the shared structural core) — v-syntax builds the structural-core share + the `Arc<[_]>` slice;
   v-core-opt owns rcdzc's boundary conversion. Can proceed in parallel with increments 1/2 (the
   `Struct::List` cheap-clone slice is independent of the leaf-payload reprs), but is a larger arc.

**Cross-lane rule (learned):** any cadenza-ast leaf/arena TYPE change breaks consumers that hold the type
by value — cdz-kernel (depends on cadenza-ast) + rcdzc (its own copy, until consolidated). cdz-kernel is
its OWN workspace (build it from `crates/cdz-kernel`, not the seed workspace, to reproduce a gate RED).
Each increment lands as ONE atomic MR folding the cdz-kernel use-site patch under v-ah's standing
delegation.
