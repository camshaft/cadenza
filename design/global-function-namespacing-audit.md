# Global / top-level function audit + namespacing proposal

Status: **RENAMES LANDED — only the schema-fn decision remains (2026-08-25).** Author:
`v-fn-namespacing`.

## Outcome (2026-08-25)

- ✅ **`compare → Ordering.of` — MERGED (#3296).** Prelude-native.
- ✅ **`print → Ast.print` / `read → Ast.read` — MERGED (#3304).** Completes the `Ast.*` self-hosting
  set (`Ast.module` + `Ast.print` + `Ast.read`).
- ✅ **Keep-bare set — closed:** `trap`, the operators, the `tuple`/`record`/`list`/`map` aliases, the
  SI/IEC unit prefixes stay bare (operator ruling).
- ⏳ **Schema fns (`decode`/`payload-of`/`Int64-schema`) — the one open item:** operator keep-vs-delete
  pending (investigation below found zero consumers; recommendation: delete).

### The mechanism pattern every prelude rename MUST follow (operator-blessed)

The augmentation that attaches a member to a built-in record **lives in the prelude, never in `db.rs`**
— only the prelude may hard-code a built-in name (operator: *"the db should not be hard-coding names …
the Ast.here convention is definitely wrong"*). Concretely: a sum's associated non-ctor members are
carried on `TypeDecl.associated`, set **at the decl** in `sums::prelude_decls`; `sum_record` appends
them (the `#3298` mechanism). Each member is an op-record (`(meta t)` = its scheme, `(meta apply)` =
`(intrinsic <prim>)`), reused verbatim from the old bare fn so the reduction is identical. Every rename
lands as ONE ATOMIC PR (remove old + add new + tests + every consumer incl. the guide); sweeps are
gate-driven (a name may also be an effect op / user def / prose — migrate only the true-prelude sites).

## Operator rulings (2026-08-25) — supersede the fork recommendations where they conflict

- **`compare → Ordering.of`: CONFIRMED.** (Landed, #3296.)
- **KEEP BARE (final, closed):** `trap`; the operators (`+`/`-`/`*`/`/`/`<`/`=`/…); the constructor
  aliases (`tuple`/`record`/`list`/`map`); the SI/IEC unit prefixes (`kilo`/`mega`/…). No change.
- **Schema functions (`decode`, `payload-of`, `Int64-schema`): ON HOLD — operator leaning DELETE, not
  namespace.** They were never explicitly greenlit. Investigation (below) found ZERO real consumers.
  The keep-vs-delete call is pending; do NOT namespace them under `Schema` unless the operator keeps
  them.
- **Transition: HARD-RENAME, and every rename PR is ATOMIC** (operator standing rule): in ONE PR —
  remove the old name, add the new, update the tests, and update EVERY consumer *including the guide*.
  No lingering alias, no split.

### Schema-function investigation (for the keep-vs-delete decision)

- **Provenance:** added in PR #410 (a broad "thirty-fifth batch … open-sums restore"), not an explicit
  greenlight. Spec-backed by `type-system.md §212 "An Open Sum's Payload May Be Schema-Typed"` (a
  normative MUST), `value-interchange.md "Decode Inverts Serialize And Refuses Otherwise"`, and the
  learning doc `2026-07-04-open-vocabulary-needs-open-sums-and-schema-typed-payloads.md`. This is the
  "OS2" surface.
- **Use case (as designed):** decode an open sum variant's payload against a run-time schema to a typed
  `Ok`/`Err` result instead of trapping, so a fold over an extensible vocabulary stays total.
- **Load-bearing? NO.** Zero real consumers — guide 0, self-hosted ML compiler 0, platform/reducer
  guests 0, runtime codec 0, other verticals 0 (all apparent matches were prose comments). The only
  references are self-test OS2 corpus cases (`15-rows-and-open-sums.sexp` + surface mentions in
  `10-bytes.sexp`, 22 occurrences) and the Rust prim impls (`lower.rs`/`resolved.rs`) + the
  `Schema`/`DecodeError` synthesized sums (`sums.rs`), which are themselves used only here.
- **Recommendation:** deletion is code-clean (no consumer breaks) but is a SPEC-RETRACTION decision
  (retract `type-system.md §212` etc.). Removal spans compiler-core (prelude + resolve + sums + lower +
  resolved + Prim/Core arms + corpus), wider than the prelude-namespacing lane — operator to decide
  owner. My lean: DELETE (contingent on the operator accepting the spec retraction).

## Why this exists

Operator ask (verbatim): *"Can we have someone do an audit of all of the global functions? I'm
really not a fan of top level functions. I know there's also a 'compare'. I would actually prefer to
have that be 'Ordering.of' instead. That's a lot more readable anyway."*

This audit enumerates **every** name the prelude installs at top level
(`implementation/seed/crates/rcdzc/src/prelude.rs`, `install()`), classifies each, and proposes a
namespaced home for the ones that read as free-standing functions. The bar is *readability*: a bare
`compare a b` becomes `Ordering.of a b`, which says what it produces.

## Operator's preferred mechanism (the target pattern)

The operator has stated the mechanism explicitly: *"I don't understand why the top level functions
were greenlit. What's so difficult about adding a record to the prelude with some associated
functions?"* So the target for every global function is: **a prelude RECORD whose fields are the
associated functions**, reached by ordinary member access (`Ordering.of`, `Schema.decode`, …) — never
a bare top-level binding. Every proposal below names *which prelude record* the function becomes a
member of. This is the same shape the prelude already uses for `List`/`Map`/`String`/… and the same
"built-in-record augmentation, shadow-safe" path v-inference is proving generalizes in #3286.

## The shape of the prelude (important framing)

The prelude is already "records everywhere": a built-in module is *just a record*, and an operation
is a field reached by member access. So the vast majority of built-in operations are **already
namespaced** — `List.len`, `Map.insert`, `Set.union`, `String.concat`, `Bytes.of`, `Char.to-int`,
`Symbol.of`, `Value.encode`, `BigInt.of`, `Rational.of`, `Qty.of`, `Type.of`, `Blake3.of`. None of
those are in scope for this audit; they are the *target namespaces*.

What remains genuinely **top-level** falls into five buckets. Only bucket **C** is a set of
free-standing functions; the audit's core recommendation is bucket C. Buckets B/D/E/F are surfaced
for a ruling because they are also bare names, even though they are operators/constructors/values
rather than functions.

---

## A. Type & module records — the namespaces themselves (KEEP; not audit subjects)

These bare names *are* modules / type-values; operations already hang off them by member access.

`Bool` `Unit` `BigInt` `Rational` `Int` `UInt` `->` `Tuple` `Record` `List` `Map` `Set` `Bytes`
`String` `Symbol` `Type` `Qty` `Blake3` `Char` `Value` `Float` `Int8` `Int16` `Int32` `Int64`
`UInt8` `UInt16` `UInt32` `UInt64` `Float32` `Float64` `Unit.*` `Unit./` `Unit.^`

No change proposed. (The `Unit.*` / `Unit./` / `Unit.^` group operators are already namespace-spelled;
the reader just keeps them as bare atoms because `*`/`/`/`^` are not alphabetic.)

---

## B. Arithmetic / relational operators — single symbols (KEEP; recommend no change)

`+` `-` `*` `/` `%` `<<` `>>` `&` `|` `^` (arithmetic/bitwise) and `<` `>` `<=` `>=` `=` (relational).

These are **spec-mandated single symbols**, not names: numeric-model.md §*An Arithmetic Operator
Requires Both Operands To Be One Numeric Type* requires one symbol per operation, dispatched on
operand type. Namespacing them (`Int64.add`?) would fight the spec and hurt readability. **Recommend
KEEP.** (Flagged only because the operator said "not a fan of top-level" — confirming these stay is a
one-line ruling.)

---

## C. Free-standing global FUNCTIONS — the audit's core (PROPOSED RENAMES)

These are the bare-name applyable operations that read as free functions. Grouped by proposed home.

| Current (top-level) | Signature | Proposed home | Confidence |
|---|---|---|---|
| `compare` | `∀a. a → a → Ordering` | **`Ordering.of`** | HIGH — operator's explicit request; `Ordering` is already a synthesized prelude sum, so `Ordering.of a b` is a natural member. |
| `print` | `Ast → String` | **`Ast.print`** | ALREADY IN FLIGHT — v-inference #3286. Do NOT duplicate; listed for completeness. |
| `read` | `String → Ast` | **`Ast.read`** | ALREADY IN FLIGHT — v-inference #3286. |
| `decode` | `∀t p. (Schema t) → p → (Result t DecodeError)` | **`Schema.decode`** | HIGH — `decode` on a `Schema` reads well; `Schema` is already a generic prelude sum. |
| `payload-of` | `∀v. v → v` (extract a variant's payload) | **`Schema.payload-of`** (fork below) | naming call only — v-inference confirms mechanically identical wherever it lands. |
| `Int64-schema` | `(Schema Int64)` (a compile-time witness VALUE, never applied) | **`Schema.int64`** (fork below) | naming call only — v-inference confirms it is the proven `Ast.here` value-member shape; no new typing. |
| `trap` | `∀a. String → a` (diverging primitive) | KEEP bare `trap` (fork below) | LOW to move — reads as a control keyword, not a function. |

### Recommended, high-confidence
- **`compare` → `Ordering.of`.** Attach an `of` field to the synthesized `Ordering` sum record whose
  op is the existing three-way `compare` intrinsic (`OpShape::Compare`). `Ordering.of a b : Ordering`.
- **`decode` → `Schema.decode`.** Move the `schema-decode` op onto a `Schema` module record. Reads as
  "decode against this schema"; keeps the OS2 payload-decode surface together with `Schema`.

### Per-name forks needing an operator ruling

**Coherence recommendation (the whole schema-decode surface under one namespace).** `decode`,
`payload-of`, and `Int64-schema` are *always used together* (extract a payload, get a width's schema
witness, decode against it). v-inference has confirmed all three are mechanically feasible under the
same built-in-record augmentation with no new resolution/typing design. So the cleanest scheme is to
**colocate the entire surface under `Schema`**: `Schema.decode` + `Schema.payload-of` +
`Schema.int64`. The forks below record the alternatives, but this grouping is the primary
recommendation.

**`payload-of`** — extracts a variant's payload for a schema check; deliberately opaque (its only
sanctioned consumer is `decode`). Purely a naming call (v-inference: resolves/types like any op-record
member wherever it lands). Options:
  - (a) `Schema.payload-of` — colocate with `decode`; reuses the existing `Schema` record, no new
    namespace. *Recommended (coherence).*
  - (b) `Sum.payload-of` — a fresh `Sum` module namespace (natural future home for other open-sum
    reflection). Requires adding a fresh module record.
  - (c) `Variant.payload-of` — `Variant` as the namespace instead of `Sum`.

**`Int64-schema`** — the compile-time schema *witness value* for `Int64` (one per decodable width;
only `Int64` realized today). The name `Int64-schema` is a hyphen-joined top-level name, the least
readable in the prelude. v-inference confirms it is the proven `Ast.here` value-member shape
(`(meta t) = (fn () (Schema Int64))`, `(meta apply) = (intrinsic schema-of)`) — no new typing. Options:
  - (a) `Schema.int64` — colocate under `Schema` alongside `decode`/`payload-of`, keeping the whole
    schema surface in one namespace. *Recommended (coherence).*
  - (b) `Int64.schema` — a `schema` field on the `Int64` width module; scales per width (`UInt32.schema`
    …). Viable if the operator prefers per-width homing over surface coherence.

**`trap`** — `∀a. String → a`, the diverging primitive (unconditional `unreachable`). It reads as a
control-flow keyword (like `if`/`let`), not a library function, and appears pervasively (`(trap "x")`
in any branch). Options:
  - (a) **KEEP bare `trap`.** *Recommended* — moving it to `Control.trap` / `Never.trap` hurts the very
    readability this effort is about, and it is spec-described as a primitive, not a module op.
  - (b) `Control.trap` — if the operator wants zero bare functions as a hard rule.

---

## D. Compound-value constructor aliases (lowercase, shadowable) — RULING NEEDED

`tuple` `record` `list` `map` — the shadowable *alias* spellings of the symbol constructors
(`(,)`/`{}`/list/map). Spec: core-semantics.md §*A Compound Value Has A Symbol Constructor And A
Shadowable Alias* mandates these aliases exist and obey lexical shadowing.

They are **constructors, not functions**, and the spec requires the bare alias. **Recommend KEEP** with
that rationale. (A capitalized `List.of` etc. already exists for the module-qualified constructor path;
the lowercase aliases are the ergonomic shorthand the spec pins.) Flagged because the operator may still
want an opinion given the "no top-level" sentiment.

---

## E. The `unit` value (KEEP)

`unit` — the unit VALUE, an alias for the empty tuple `()` (core-semantics.md §*Unit And The Empty Tuple
Are The Same Value*). A value, not a function. **KEEP.**

---

## F. SI / IEC unit prefixes (values) — RULING NEEDED (owner: v-quantity)

`kilo` `mega` `giga` `tera` `milli` `micro` `nano` `pico` `kibi` `mebi` `gibi` `tebi` — prefix VALUES
applied via `(Unit.prefix P u)`, part of the optional units-of-measure layer. They are bare names but
ergonomically meant to be written unqualified in unit expressions. Options:
  - (a) KEEP bare — matches how prefixes read in dimensional expressions.
  - (b) Namespace under `SI.kilo` / `Prefix.kilo` — consistent with "no bare names" but verbose at the
    use site.
This bucket is owned by the units-of-measure vertical (**v-quantity**); routing the ruling there.

---

## Mechanism (how a rename lands, single-writer safe)

The prelude + name-resolution is owned by **v-inference**; surface syntax by **v-syntax**. The mechanism
already exists and is proven by the in-flight `Ast.print`/`Ast.read` relocation (#3286): a member op is
attached to a (synthesized-sum or module) record and reached by the ordinary member-access-and-fold path
— no new resolution machinery.

**Surface confirmed (v-syntax):** no surface work and no new syntax for any rename —
`Namespace.member` is ordinary member access `(. Namespace member)`, and every member-key shape
round-trips on a built compiler: simple (`Ordering.of`), kebab/hyphenated (`Foo.payload-of` — the
`-`-glued-identifier lexer rule applies to member keys too), and even reserved-word keys
(`Foo.match` parses, key after `.` is read as a name regardless of keyword status). So no member name
is unspellable.

> **⚠ CORRECTED MECHANISM (operator, 2026-08-25).** The `Ast.here` / `Db::load_linked` augmentation
> pattern this section originally endorsed is **WRONG** — operator verbatim: *"the db should not be
> hard-coding names. only the prelude is allowed to do that"* and *"the Ast.here convention is
> definitely wrong. it was a mistake to let that through as well."* So **all built-in-record
> augmentation that names a built-in must live in `prelude.rs`, never in `db.rs`.** The already-landed
> `Ast.here` (#3286) needs remediation to the prelude too (v-inference owns this). The op-record SHAPE
> below is unchanged; only the LOCATION moves.

**Resolution mechanism (v-inference's lane).** Neither case is a name-check in the generic `sum_record`
(that would be "privileged by name" and break user-shadowing), and neither hard-codes a name in `db.rs`:
- **Target is a prelude SUM namespace** (`Ordering` = Less|Equal|Greater; `Schema`): the PRELUDE must
  declare the augmentation — e.g. a `prelude.rs` name→extra-member-fields table (`"Ordering"` → `of`
  op-record) — so the built-in NAME is hard-coded only in the prelude. Synthesis/`db.rs` then applies
  that prelude-provided table to the synthesized sum record without naming anything itself. The
  Capitalized-ctor / lowercase-member discriminator keeps a user's own `type Ordering` untouched
  (shadow-safe). (v-inference's design call on the exact mechanism.)
- **Target is a NON-sum namespace** (a fresh module like `Blake3`): add a module record + fields
  directly in `prelude::install` (`Blake3.of` is the reference) — already prelude-local, correct.

Each member is an **op-record**: `(meta t)` = its type scheme, `(meta apply)` = `(intrinsic <prim>)` →
a `Prim` variant. It resolves via ordinary member access `(. NS member)` and is shadow-correct by
construction. So `compare → Ordering.of` = augment `Ordering`'s built-in record with an `of` field
whose `(meta apply)` is the existing `Prim::Compare`; `decode → Schema.decode` similarly on `Schema`.
v-syntax nit (already satisfied): prefer **lowercase** member names on sum-type namespaces so the
ctor/member discriminator holds — all proposed members (`of`, `decode`, `payload-of`, `schema`) are
lowercase.

**All bucket-C renames are mechanical (v-inference confirmed — no new resolution/typing design):**
- **`Int64-schema` (value witness)** uses the same bare-VALUE member SHAPE as `Ast.here`: `(meta t) =
  (fn () T)` zero-param scheme (so `scheme_of` reads a monomorphic scheme for a *value*, not a
  type-value), `(meta apply) = (intrinsic prim)`. `Int64-schema` already has this exact shape
  (`schema_witness_type = (fn () (Schema Int64))` + `(intrinsic schema-of)`), so `Schema.int64` = add
  that op-record as an `int64` field on the `Schema` built-in record — via the prelude augmentation
  table above (NOT the `db.rs` location `Ast.here` used, which is being remediated). Shadow-safe, not a
  special case.
- **`payload-of`** resolves/types like any op-record member wherever it lands; `Schema.payload-of`
  reuses the existing `Schema` record (no new namespace), while `Sum.`/`Variant.` would add a fresh
  module record (also mechanical, like `Blake3`). Pure naming call — no design blocker either way.

For each blessed rename:
1. Attach the op as a field on the target namespace record (e.g. `of` on the `Ordering` sum record) —
   same `list_op_record` + `meta_field` shape every module field uses.
2. Remove the bare top-level `names.insert("compare", …)` entry.
3. Update the corpus (`spec/semantics/*.sexp`) and any prelude-facing tests + docs from `compare` to
   `Ordering.of`. Coordinate the transition window with v-inference (resolver) so no in-flight work
   references the old bare name.

**Transition question for the operator:** hard-rename (remove the bare name immediately) vs. a
deprecation window (both names resolve for a release). Recommend **hard-rename** — the fleet's corpus is
the only caller and can migrate in the same coordinated slice; a lingering alias re-introduces the very
bare name we are removing.

## Coordination status
- Pinged **v-inference** (owns prelude + resolve; mid-flight on Ast-namespacing #3286) and **v-syntax**
  (surface) to align on mechanism before any edit.
- Routing the **F. prefixes** ruling to **v-quantity** (units-of-measure owner).

## Open questions for the operator (one-line answers unblock phase 2)
1. `compare → Ordering.of`: confirmed. Any other bucket-C renames you want to veto or add?
2. `payload-of` home: `Schema.payload-of` (rec. — colocate) / `Sum.` / `Variant.`?
3. `Int64-schema` home: `Schema.int64` (rec. — colocate) / `Int64.schema`? (Together with Q2 + `Schema.decode`, the recommendation is the *whole schema surface under `Schema`*.)
4. `trap`: keep bare (rec.) or move to `Control.trap`?
5. Bucket B operators (`+`/`<`/…): keep bare (rec.)? (confirming the obvious)
6. Bucket D aliases (`tuple`/`record`/`list`/`map`): keep bare per spec (rec.)?
7. Bucket F prefixes: keep bare (rec.) or namespace under `SI.`/`Prefix.`?
8. Transition: hard-rename (rec.) or deprecation window?
