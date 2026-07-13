# DESIGN — the Map collection vertical (rcdzc)

Status: PLAN (2026-07-13). Worktree `.claude/worktrees/map-vertical`, branch `map-vertical`.

## Goal

Bring the **Map** collection all the way up in the native `rcdzc` compiler — from entirely
unbound today (`Map`, `Set`, `map`, `set` all resolve to CDZ0101) to passing every map corpus
case. The CHAMP runtime is already built and generated into `runtime_abi.rs` (WIT ops 37–45:
`map-empty`/`map-insert`/`map-lookup`/`map-remove`/`map-size` + the `map-iter*` cursor). This is a
**front-end-only** vertical: prelude → resolve → infer → core → lower → eval-fold → wasm backend →
render. No runtime work is needed for the base operation set.

Set is the sibling collection and shares the entire mechanism (CHAMP-minus-value-column; runtime
ops 46–59 exist). It is **out of scope** for this doc — do Map first, then a much smaller Set
increment reuses everything here. (Set corpus lives in `19-sets.sexp`, its own vertical.)

## The corpus (the oracle)

~36 map cases, almost all in `spec/semantics/05-compound-types.sexp`, plus one in
`07-type-system.sexp`. They cluster into six behavioral groups. Find them by substring with
`cargo xtask gate --case "<substr>"` (reads `actual:`), not by name.

### Group A — the `(map (k v) …)` LITERAL value, keys are VALUES

The defining rule (operator, confirmed): **a map key is an ordinary value expression resolved in
scope** — NOT a compile-time label like a record field. This is the whole difference from `record`.

- `a bound name in a map key is used by its value, not the literal name` — `(let ((a 5)) (= (map (a 1)) (Map.insert Map.empty 5 1)))` → `true`. `a` keys by the value `5`.
- `two distinct names bound to the same value key the same map entry` — `(let ((a 5)) (let ((b 5)) (Map.size (map (a 1) (b 2)))))` → `1` (later entry overwrites at key 5).
- `a name bound to a string keys a map by its value, not the literal name` — same at String key type → `true`.
- `distinct names bound to the same string key the same map entry` → `1`.
- `an unbound name in a map key is a scope error, not a coerced string` — `(map (undefined-key 1))` → **CDZ0101** (ordinary unbound-name; must NOT coerce the name to a String).
- `map equality is independent of insertion order` — `(= (map ("a" 1) ("b" 2)) (map ("b" 2) ("a" 1)))` → `true`.
- `a map with a computed key equals the same map with a constant key` — `(let ((j (+ 2 3))) (let ((k 5)) (= (map (j 1)) (map (k 1)))))` → `true`.

Note the corpus writes String-keyed literals as `(map ("a" 1))` and int-keyed as `(map (1 10))`,
and name-keyed as `(map (a 1))`. All three key positions are ordinary value expressions: a string
literal, an int literal, a `Ref`. `(+ 2 3)` in key position is a runtime-computed key.

### Group B — homogeneity / well-formedness rejections (construction-time)

- `a map with values of two different types is a type error` → CDZ0201 (Int + Bool values).
- `a map mixing integer and float values is a type error` → CDZ0201.
- `a map literal with keys of two different types is a type error` → CDZ0201 (Int key + Bool key, bound names, so independent of the coercion bug).
- `a map with record values of different field sets is a type error` → CDZ0201.
- `a map with tuple values of different arities is a type error` → CDZ0201.
- `a map with a duplicate key is a type error` → CDZ0201 (`(map (a 1) (a 2))` — for **constant** keys; a runtime-computed duplicate is a runtime overwrite, see the `size` cases).
- `a map entry with no value expression is rejected, not a crash` (07) — `(map ("a"))` → CDZ0201 (a map entry is a `(key value)` pair).
- `inserting a value of a different type into a map is a type error` → CDZ0201 (`Map.insert` value-homogeneity).
- `inserting a key of a different type into a map is a type error` → CDZ0201 (`Map.insert` key-homogeneity).

### Group C — cross-kind / cross-shape comparison rejections

- `comparing a map to a record is a type error` → CDZ0201 (map and record are distinct types).
- `comparing an empty map to an empty record is a type error` → CDZ0201.
- `member access on a map is a type error` → CDZ0201 (`(. m a)` on a map — a map is not a record).
- `a list of maps with different keys is homogeneous, not a type error` → **`true`**. Two maps with different key SETS are the SAME type `Map<K,V>` — key set is runtime data, not shape. This is the key contrast with records/tuples: it must NOT reuse the record/tuple shape-mismatch arm.

### Group D — different-keyset comparison is FALSE, not a rejection

- `two maps with different keys are unequal, not a type error` → `false`.
- `two maps of different sizes are unequal, not a type error` → `false`.
- `an empty map is unequal to a non-empty map, not a type error` → `false`.

These are the load-bearing correctness proof that Map's type identity is `Map<K,V>` (parametric in
K and V), decoupled from the key set. A wrong implementation that treats the key set like a record's
field set REJECTS these (the historic seed miscompile).

### Group E — the `Map.*` OPERATION surface (runtime maps)

- `inserting into the empty map then looking a key up yields the value` — `(Map.lookup (Map.insert Map.empty 1 10) 1)` → `(: (Some 10) (Option Int64))`.
- `looking up an absent key yields None` → `(: (None unit) (Option Int64))`.
- `matching a lookup from a computed-key map literal selects the present-value arm` — the computed-key `(map (j 1))` with `j=(+ 2 3)` looked up at 5 and matched → `1`. Proves a computed-key literal builds a PROPER runtime map (its lookup Option dispatches correctly).
- `inserting a key already present replaces its value, not the size` → `1`.
- `removing a key drops its association and the size` → `1`.
- `removing an absent key leaves the map unchanged` → `1`.
- `the value-yielding insert reports the value it replaced` — `Map.swap` → `(tuple (Option prior) new-map)`, project 0 → `(Some 10)`.
- `the value-yielding remove reports the value it dropped` — `Map.take` → `(tuple (Option removed) new-map)`, project 0 → `(Some 10)`.
- `a map operation applies to a map passed as a function parameter` — `(def (count mp) (Map.size mp))` on a 2-entry map → `2`.
- `a built map renders its entries in canonical key order` — `(Map.insert (Map.insert Map.empty 2 20) 1 10)` returned as the result → `(: (map (1 10) (2 20)) (Map Int64 Int64))`. Render is SORTED by canonical key form, independent of insert order.

### Group F — map PATTERNS (gated `(needs map-patterns)`, a later phase)

- `a map pattern matches a present key and binds its value` — `((map (1 v)) v)` → `10`.
- `a map pattern falls through when the key is absent` → `99`.
- `a map pattern binds the rest of the map after the named key` — `((map (1 v) .. rest) (Map.size rest))` → `1`.

## Runtime contract (already built — read `cdz-runtime/wit/runtime.wit:180-206`)

| WIT | op | ownership |
|-----|----|-----------|
| 37 | `map-empty() -> handle` | — the canonical empty map |
| 38 | `map-insert(m, key, val) -> m'` | consumes m, key, val |
| 39 | `map-lookup(m, key) -> val` | borrows m + key; **NULL** if absent |
| 40 | `map-remove(m, key) -> m'` | consumes m; borrows key |
| 41 | `map-size(m) -> u32` | O(1) |
| 42–45 | `map-iter`/`-next`/`-key`/`-val` | stateless cursor for render |

Keys/values cross as **plain handles**. The runtime hashes and compares keys by a tagless
structural walk (`champ_eq`/`champ_hash`) — no serialization, no upcall. This is sound because keys
are homogeneous (one key type per map, enforced at compile time) and every value form is canonical
by construction (Bytes rope is the only exception; a rope key would need `bytes-compact` first —
not exercised by the corpus). So `map-lookup`'s NULL-or-handle return maps directly onto the
built-in `Option` sum, exactly as `List.at`/`Core::ListAt` build `Some`/`None`
(`lower.rs:lower_list_at`, `select.rs` bounds-checked path).

**Canonical render = SORTED by key.** The runtime iterates in HASH order (its WIT note); the
compiler owns the canonical byte-form sort. The renderer must SORT entries by canonical key form,
not emit them in cursor order.

## The List template Map mirrors (verified file:line map)

Map is added the same way List is plumbed, with two structural differences: (1) **two type
parameters** `(Map K V)` not one; (2) **the literal's key is a value expression, not a label**.

| stage | file | List anchor | Map work |
|---|---|---|---|
| prelude | `prelude.rs:77,81,216-242,548-641` | `list`/`List`, `list_module`, `list_op_record`, type-lambdas | add `Map` module (2-param type-lambdas) + a `map` value-ctor alias |
| ty | `ty.rs:223,336,396,488,534` | `Ty::List(Box<Ty>)` + 4 method arms | `Ty::Map(Box<Ty>,Box<Ty>)` + `has_free_var`/`agrees_with`/`join`/`render_name` |
| resolved | `resolved.rs:172-202,339-345,520` | `Prim::List*`, `from_name`, `Resolved::List` | `Prim::MapNew/MapCtor/MapEmpty/MapInsert/MapLookup/MapRemove/MapSize/MapSwap/MapTake` + `Resolved::Map` |
| resolve | `resolve.rs:219,1968,1707` | `"list"` head, `resolve_list`, `decode_ty` | `"map"` head + `resolve_map` (keys = value occ!) + `(Map K V)` decode |
| infer | `infer.rs:147,851,1183,1637` | list literal type, apply type, homogeneity | map literal type (join keys, join vals), two-axis homogeneity |
| core | `core.rs:190-223` | `Core::List*` (`ListAt` carries `disc_some/none`) | `Core::MapNew/MapInsert/MapLookup(+disc)/MapRemove/MapSize/MapSwap/MapTake` |
| lower | `lower.rs:274,687-788,4342,5322,3279,3382,3146,4152,4271` | Resolved→Core, fold+emit, `op_name`, `const_value_ast`, `type_ast`, template, const-eq, canonical-check | mirror all |
| eval | `eval.rs:175,1406,1455,1484,1725` | type-ctor reduce, value-ctor build, `encode_ty` | mirror for `(Map K V)` + `map` head |
| wasm select | `select.rs:112-146,171-236,520-586,1966-2240` | `OP_VEC_*`, ownership, import-collect, emit | `OP_MAP_*` consts + import-collect + emit (ops in ABI) |
| wasm lir | `lir.rs:277,365` | `Ty::List → I32` / boundary `None` | `Ty::Map` arms |
| wasm mod | `backend/wasm/mod.rs:77,530` | list resource-escape gate | `Ty::Map` arm |
| rust backend | `backend/rust/expr.rs:713-737`, `types.rs:65` | grouped decline | add `Core::Map*` to the decline group (wasm-first) |
| layout | `layout.rs:360,499` | list reachability arms | `Core::Map*` reachability arms |
| unify | `unify.rs:162,290,444,536` | `Ty::List` in unify/occurs/rename/freshen | `Ty::Map` in all four |
| effects | `effects.rs:1251` | `ty_has_any` | `Ty::Map` arm |

## The central resolution decision: map keys are VALUES

`resolve_record` (`resolve.rs:1977`) reads each `(field value)` entry with `read_key` → a `Symbol`
LABEL, never resolving the key as a value. **Map must NOT do this.** A map entry `(k v)` is a pair
of two ordinary value occurrences; the key resolves in scope by the normal `Ref`/scope lookup, so:

- a bound name in key position resolves to its VALUE (`(let ((a 5)) (map (a 1)))` keys by `5`);
- a computed expression `(+ 2 3)` in key position is a runtime key;
- an UNBOUND name in key position is the ordinary CDZ0101 scope error (the resolver already emits
  this for any unbound `Ref` — we get the "not a coerced string" case for free by simply resolving
  the key as a value, never falling back to its spelling).

So:

```rust
Resolved::Map { entries: Arc<[(StructId, StructId)]> }  // (keyOcc, valOcc) pairs, both resolved on demand
```

`resolve_map` reads the `(map (k v) …)` tail, bounds-checking each entry is a 2-element list
(a 1-element `(map ("a"))` → CDZ0201 "a map entry is a (key value) pair", never a panic — Group B),
and stores both occurrences. It does NOT read the key as a `Symbol`. Both positions flow through
`infer`/`lower`/`eval` as value expressions.

`map` is a grammar string-head (dispatched at `resolve.rs:216`-style, before name dispatch), AND a
shadowable prelude alias, exactly as `list`/`tuple`/`record` are ("the strings are the symbols").

## Phasing

Each phase is a landable increment: build, gate (`--check` never regresses), `cargo test -p rcdzc`,
merge to `spec` via the guarded CAS. Order is chosen so each phase turns a coherent group of
`todo` → `pass` and nothing flips `todo → fail` (a miscompile).

### Phase M0 — `Ty::Map` + type surface + `(Map K V)` type expression

Add the `Ty::Map(Box<Ty>, Box<Ty>)` variant and every forced match arm (ty.rs 4 methods, unify.rs 4
arms, effects.rs, lir.rs, rust/types.rs, layout has no type arm). Add `(Map K V)` decode
(`resolve.rs:decode_ty`) + encode (`eval.rs:encode_ty`) + `render_name` `(Map K V)` +
`lower.rs:type_ast`. No values yet — this phase compiles green with no behavior change (the variant
is unreachable until M1). This de-risks the exhaustive-match churn in one isolated commit
(the `Ty::Float` playbook).

**Exit:** builds, all tests pass, gate unchanged. `(: x (Map Int64 Int64))` parses to the type.

### Phase M1 — the `Map` module + empty/insert/lookup/size on INLINE maps

Prelude `map_module` with `(meta apply) = (intrinsic Map)` (the 2-param type ctor) and op fields
`empty`/`insert`/`lookup`/`size`/`remove`, each a `list_op_record` with a 2-param `(fn (k v) …)`
type-lambda:

- `empty : ∀k v. (Map k v)` — a value, not a function (like `Map.empty`); reduces to `Core::MapNew{entries:[]}` or a `map-empty` call.
- `insert : ∀k v. (Map k v) → k → v → (Map k v)`
- `lookup : ∀k v. (Map k v) → k → (Option v)`
- `remove : ∀k v. (Map k v) → k → (Map k v)`
- `size : ∀k v. (Map k v) → Int64`

`Core::MapNew{entries}`, `MapInsert{map,key,val}`, `MapLookup{map,key,disc_some,disc_none}`,
`MapRemove{map,key}`, `MapSize{map}`. Lower emits the runtime ops: `map-empty`, `map-insert`,
`map-lookup` (NULL → build `None` at the Option discs, else `Some(val)`; mirror `lower_list_at` /
the `select.rs` bounds-checked ListAt path), `map-remove`, `map-size`. `op_name` entries. `lir.rs`
`Ty::Map → I32` handle + boundary `None`. Ownership: `insert`/`remove` CONSUME the map (FBIP);
`lookup`/`size` BORROW (mirror `select.rs:171-236`).

This targets the inline-operation cases of Group E: insert-then-lookup, absent→None, insert-replace,
remove, remove-absent. `Map.size`/`Map.lookup` on inline-built maps.

**Note on the parameter-map case** (`(def (count mp) (Map.size mp))`): List/Bytes/String ops on a
param operand already work in rcdzc (they read the runtime handle without the construction site). If
Map's ops lower against the operand's TYPE (`Ty::Map`) rather than a known construction shape — which
is the natural rcdzc lowering — the parameter case works from M1 with no extra effort. Verify with
the `a map operation applies to a map passed as a function parameter` case; if it declines, the op
dispatch is over-narrowly keyed on an inline shape and must key on `Ty::Map` instead.

**Exit:** Group E inline ops (minus swap/take/render) pass.

### Phase M2 — the `(map (k v) …)` LITERAL with value keys

`resolve_map` + `Resolved::Map{entries}` (keys as VALUE occurrences — the central decision above) +
the `"map"` grammar head. Infer: the literal's type is `Map<join(keys), join(vals)>`. Lower:
`Resolved::Map → Core::MapNew{entries}`; a literal lowers to `map-empty` + a `map-insert` per entry
(consuming), OR — when all keys+vals are constant — folds to a canonical constant map value (see M3
for equality/render). A computed-key entry `(+ 2 3)` just lowers its key sub-expression normally,
producing a proper runtime map (closes `matching a lookup from a computed-key map literal…`).

Malformed entry `(map ("a"))` → CDZ0201 in `resolve_map` (bounds-check before indexing the value —
Group B never-crash). Unbound key → CDZ0101 for free (key resolved as a value `Ref`).

**Exit:** the value-keys cases of Group A (bound-name-by-value, distinct-names-collide, string keys,
unbound→CDZ0101, computed-key-lookup-match).

### Phase M3 — equality + canonical render + homogeneity/comparison rejections

The correctness heart of the vertical.

**Equality.** Two maps compare via the runtime `value-eq` op (`Core::ValueEq`, the tagless
`champ_eq` walk) — a CHAMP map handle is canonical by construction, so structural byte-equality =
value equality, order-independent. `lower.rs:4079` already routes a compound `=` to `ValueEq` when
`compound_eq_heap_walkable`; extend that predicate to accept `Ty::Map` (a map's leaves being CHAMP
means it is ALREADY canonical — unlike an embedded RRB vector). This closes:
- `map equality is independent of insertion order` (canonical),
- `a map with a computed key equals the same map with a constant key` (runtime map = const map, both canonical CHAMP handles — the historic const-vs-runtime miscompile),
- the different-keyset FALSE cases of Group D (same type `Map<K,V>`, `value-eq` returns false),
- `a list of maps with different keys is homogeneous` (Group C — a `List (Map K V)` is homogeneous because all elements share `Map<K,V>`; the list-element homogeneity check must compare TYPE `Map K V`, NOT key set — do NOT route Map through the record/tuple shape-mismatch arm).

For the const-map folding path, structural equality of two constant `Core::MapNew` values compares
them as key→value SETS (canonicalize: dedup by key value, order-independent) — the map analogue of
`const_compound_eq`'s Record arm (`lower.rs`), NOT the positional Tuple/List arm.

**Render.** `const_value_ast`/the value-form renderer emits `(map (k1 v1) (k2 v2) …)` with entries
SORTED by canonical key form (Group E `a built map renders…`). Type surface `(Map K V)`.
The runtime-map render (a map returned as the result, not const-folded) walks the `map-iter*`
cursor and sorts — this needs a `Ty::Map` runtime value-form template
(`lower.rs:template_value_ast_flagged`, currently `_ => None` for List/Map). If the runtime template
is heavy, the corpus render case uses an inline-built const-foldable map, so the const render path
may suffice initially — measure which the case actually hits.

**Homogeneity + comparison rejections (Groups B, C).**
- Map literal: unify all key types to one, all value types to one, on CONSTRUCTION (when the map is built/returned), CDZ0201 on mismatch — the two-axis analogue of the list homogeneity check (`infer.rs:1183`). Covers value-type-mismatch, int/float values, key-type-mismatch, record-value shape, tuple-value arity.
- `Map.insert`: the produced map value satisfies the same rule — check inserted key against the map's key type and value against its value type (CDZ0201).
- Duplicate CONSTANT key `(map (a 1) (a 2))` → CDZ0201 (a compile-time duplicate is ambiguous; a RUNTIME duplicate, e.g. two names bound to 5, is a runtime overwrite → size 1, NOT a rejection — the distinction is const-key-set vs runtime-computed).
- map vs record, empty-map vs empty-record → CDZ0201 (distinct kinds; unify `Ty::Map` against `Ty::Record` fails).
- `(. m a)` on a map → CDZ0201 (member access requires a record; `Ty::Map` is not projectable).

**Exit:** Groups A, B, C, D, E all pass. This is the bulk of the vertical.

### Phase M4 — `Map.swap` / `Map.take` (value-yielding forms)

`swap : (Map k v) → k → v → (Tuple (Option v) (Map k v))` and
`take : (Map k v) → k → (Tuple (Option v) (Map k v))`. The runtime has no single value-yielding op,
so lower each as: prior = `map-lookup` (→ Option), new-map = `map-insert`/`map-remove`, paired into a
`Core::Tuple`. Ownership care: `map-lookup` borrows, so a `dup` may be needed before the consuming
insert/remove — model as `Core::MapSwap`/`MapTake` that select.rs expands to the borrow-lookup +
consume-update + tuple build, or compose in lower from the existing primitives.

**Exit:** the two value-yielding Group E cases pass.

### Phase M5 — map PATTERNS (gated `(needs map-patterns)`, separate phase)

`(map (k p) … .. rest)` is a KEY-DIRECTED LOOKUP pattern, not a structural match. Lower a map-pattern
arm to: for each `(k p)`, `map-lookup m k` → if `None` the arm fails (fall-through); if `Some v`,
match `v` against sub-pattern `p`. `.. rest` binds `Map.remove`-of-the-named-keys. A map pattern
NEVER contributes to exhaustiveness (unbounded key set) — the match needs a catch-all. This is a new
pattern kind in the resolve/pattern layer and the Maranget matcher; scope it after M0–M4 land.

**Exit:** Group F passes.

## Risks / traps (from memory)

- **Do NOT route Map through the record/tuple shape-mismatch arm.** A map's key set is runtime data,
  not shape. The list-of-maps-homogeneous and different-keyset-false cases are the proof; they FAIL
  if Map reuses `shapes_incompatible`. (`[[map-different-keyset-comparison-wrongly-rejected]]`,
  `[[map-record-type-confusion]]`.)
- **Keys are values, not labels.** Never call `read_key` on a map key; resolve it as a value
  occurrence. This is the operator's explicit instruction and closes the unbound-key CDZ0101 and
  computed-key cases for free. (`[[unbound-map-key-coerced-to-string-break]]`.)
- **const-map equality/render is SET-based (order-independent), not positional.** Mirror the Record
  const-eq arm, not the Tuple/List arm.
- **A new `Ty`/`Core`/`Prim` variant needs a RUST-backend arm too** (`backend/rust/expr.rs`) — a
  grouped decline is fine (wasm-first).
- **wasm opcodes/section-ids are generated** — the `map-*` runtime ops are ALREADY in
  `runtime_abi.rs` (generated from the frozen WIT); do NOT hand-edit. Just reference the op names in
  `select.rs` via `Lir::CallImport("map-insert")` etc.
- **No keys outside the prelude** — `Map`/`map`/`Map.empty`/… are prelude entries + a grammar head;
  NO `if name=="Map"` special-casing in infer/lower. (`[[no-keys-outside-the-prelude]]`.)
- **Gate measurement**: build the runtime FIRST (`cargo xtask build`); a stale store makes heap
  cases false-fail. Diff the FAIL set (`gate --check`), not the P count.
- **CAS landing** on `spec` (checked out in main): edit only in the worktree, gate there, land via
  guarded `git update-ref` with the ancestor + `HEAD~1==$SPEC` assertions
  (`[[spec-ref-cas-races-recompute-expected-sha]]`). Re-verify reachability after landing.

## Related memory

`[[map-operation-surface-spec]]` (the spec surface), `[[champ-map-set-design-2026-07-06]]` (the
runtime seam), `[[map-value-keys-and-two-symmetric-arity-checks]]`,
`[[map-literal-key-homogeneity-check]]`, `[[map-insert-skips-homogeneity-check-break]]`,
`[[computed-key-map-literal-lookup-match-misdispatch]]`,
`[[map-equality-const-vs-runtime-construction-miscompile]]`,
`[[map-operation-on-parameter-map-not-lowered-todo]]`, `[[list-vec-merge-to-one-sequence-type]]`
(the List precedent this mirrors). Note: several of these describe the PRE-rewrite seed; in rcdzc
the whole vertical is greenfield (Map is entirely unbound today).
