# Design — explicit field nodes: record TYPE fields as `(: name T)`, record VALUE fields as `(= name v)`

**Author:** design pass (fleet `design-record-type-syntax`).
**Audience:** the `vertical` agent(s) that build it — `cadenza-syntax` (parser + printer + round-trip),
`rcdzc` (type-eval / decode / encode readers + value-record field read), and a corpus migration
(`corpus-bugfix`).
**Status:** DESIGN — REVISED per operator review on PR #2794 (2026-08-08). All forks are now DECIDED
by the operator; no open decisions remain (§6). The earlier revision recommended the OPPOSITE canonical
form and is superseded (§0.1).
**Subsystem:** spans `cadenza-syntax` (parser normalization + printer + round-trip), `rcdzc` (type-eval /
encode / decode + value-record field read), corpus migration, and a touch of guide.

## 0. The principle — READ FIRST

**Operator principle (PR #2794 review + follow-up, verbatim):**
> "I would prefer to have fewer exceptions in the syntax. Rewriting and inserting nodes is the wrong
> direction." … "We also have that problem on the record literals … we are going to have to change
> those as well to explicitly include a `=` node … It is just much more explicit and less magical."

The unifying rule: **the field-separator token survives into the AST as an explicit node; the parser
does NOT silently rewrite it into a bare pair.** Two symmetric consequences:

| | Surface | Canonical arena (explicit node) | Today (the magical form being removed) |
|---|---|---|---|
| **record TYPE field** | `field: T` / `{x: Int64}` | `(: field T)` ascription (`:`-headed 3-list) | dual — also `(field T)` pair from `Record(field(T))` |
| **record VALUE field** | `{a = 1}` | `(= a 1)` (`=`-headed 3-list) | `(a 1)` bare pair (the `=` is dropped) |

Both make the same move: keep the surface token (`:` on types, `=` on values) as the head of the
field node, so the arena is explicit rather than an implicit `(name payload)` pair. On the type side
this ALSO means the record-type field becomes the SAME `(: name T)` ascription node already used by
parameter binders (`name: T`) and expression ascription (`e: T`) — one node instead of a bespoke pair,
i.e. fewer syntax exceptions.

### 0.1 What changed from the first revision (superseded)
The first cut of this doc recommended the OPPOSITE for the type side: adopt the `(field T)` head-app
pair as canonical and eliminate the ascription. The operator **rejected** that ("rewriting and
inserting nodes is the wrong direction") and DECIDED:
- **Field-separator surface = COLON** (`field: T`), for consistency with every other type annotation
  in Cadenza (param binders, `e: T`) and the OCaml record model. (This was open decision D2; now closed.)
- **Canonical type-field arena = `(: field T)` ASCRIPTION**, migrate the `(field T)` pair cases TO it.
  Because the colon surface ALREADY parses straight to `(: field T)` (`parser.rs:2249`, `parser.rs:3791`),
  this needs **no parse-time node rewriting** — the parser output is already canonical. (This was
  decision D1; flipped and now closed.)
- **Value-record literal gets the symmetric explicit `=` node** — `{a = 1}` → `(= a 1)`, not `(a 1)`.
  (New scope from the operator; §5 below.)

The tradeoff this flip accepts, stated honestly: choosing ascription-canonical shifts work INTO `rcdzc`
(the pair-only readers `decode_ty`/`reduce_ctor` and the `encode_ty`/`render_name` renderer move from
the pair to the ascription form — §3), whereas the pair choice would have left them untouched. The
operator weighed this and chose the ascription anyway, prioritizing (a) no parser node surgery and
(b) one shared ascription node over minimizing the rcdzc delta. Not re-litigated.

## 1. The canonical encodings (DECIDED)

- **Record TYPE field: `(: name T)`** — the `:`-headed 3-list ascription. Identical node to param
  binders and `e: T`. This is what the colon surface (`field: T`, `{x: T}`) already produces.
- **Record VALUE field: `(= name value)`** — the `=`-headed 3-list. Symmetric to the type side; the
  `=` the author writes survives into the arena.
- The old **implicit** forms are migrated + rejected (canonical-form discipline,
  [[garbage-render-means-not-canonical-fix-the-source]], [[DESIGN-record-update-syntax]]): the type
  `(field T)` head-application pair and the value `(name value)` bare pair are no longer produced or
  accepted once migration completes.

## 2. The increments (top-to-bottom, the way a vertical lands them)

The two sides (TYPE `(: name T)`, VALUE `(= name v)`) are INDEPENDENT landables — different parsers,
different arenas, different corpus pins, wildly different blast radius (type ~170 occ, value ~838
occ / 17 files). They ship as SEPARATE units. Type side first (smaller; surface already parses).

### Phase A — record TYPE fields canonical `(: name T)` (smaller; colon already parses)

**RT1 — cadenza-syntax: normalize the `(field T)` head-application to `(: field T)`.**
The colon surface already parses to `(: field T)` (`type_arg` `parser.rs:2249`; `type_brace_record`
`parser.rs:3791`) — no change needed there. The only surface that produces the pair `(field T)` is the
application spelling `Record(field(UInt8))` (the `field(UInt8)` arg parsed as an application via
`type_postfix`). Decide (OQ-A): (a) **reject** `Record(field(T))` — a record-type field must be written
`field: T`, so the call-like arg is a parse/resolve error steering to the colon surface; or (b) keep
accepting it but **normalize** its arena to `(: field T)`. Default: **(a) reject** — it is the cleanest
"fewer exceptions" outcome and the head-app spelling has no reason to survive once colon is canonical.
(Normalizing (b) is itself a node rewrite, which the operator's principle disfavors — another reason to
reject.) NOTE: this is NOT parser node surgery on the ascription output — the ascription path is
untouched; this is only about what to do with the now-obsolete call-like spelling.

**RT2 — cadenza-syntax: printer emits `field: T` for record-type fields.**
Today a `Record(...)` type node prints via the generic call form (`printer.rs:707`), rendering
`(field UInt8)` as `field(UInt8)`. After migration the field nodes are `(: field T)`, which already
print through the infix `:` path (`printer.rs:490`, `infix` `printer.rs:852`) as `field: UInt8`.
Confirm the `Record(...)` head prints its `(: f T)` children as `field: T` (infix), and that NO field
prints as `field(T)`. Round-trip gate (`corpus_roundtrip.rs`) pins this.

**RT3 — rcdzc: make the ascription `(: name T)` the primary type-field reader on ALL paths.**
- Type-eval already accepts ascription: `type_in_env` (`eval.rs:299`), `typeval_of_uncached`
  (`eval.rs:3204`), `push_payload_type_positions` (`compile.rs:974`), `collect_type_params`
  (`db.rs:5615`). These stay; the ascription arm becomes the sole live path (their `(name T)` pair arm
  becomes dead once the corpus is migrated — prune later, OQ-C).
- The pair-only readers gain the ascription form: `reduce_ctor` (`eval.rs:2758`) and `decode_ty`
  (`resolve.rs:5810`) currently match `List(children) if len == 2`; they must read the `(: name T)`
  triple. `decode_ty` is the dual of `encode_ty`, so:
- **Encoder flips to render ascription:** `Ty::Record` in `render_name` (`ty.rs:1728`) and the
  `encode_ty` path (`ty.rs:1819`) currently emit `(Record (name T))`; change to `(Record (: name T))`
  so the type renderer spells the type the way the (colon) surface accepts it (the renderer's own
  comment `ty.rs:1722`–`1727` mandates surface-matching). This is the load-bearing rcdzc change — every
  encode/decode/diagnostic render moves in lockstep, so encode↔decode round-trips on the ascription
  form.
- **Gate:** `cargo test -p rcdzc --lib` — a `(Record (: a Int64))` type-evaluates; a value of that type
  executes under wasmtime; encode→decode round-trips on the ascription form; a diagnostic renders
  `(Record (: a Int64))`.

**RT4 — corpus migration: ~141 head-app cases `(Record (a T))` → `(Record (: a T))`.**
`corpus-bugfix` zone (`spec/semantics/*.sexp` + `.gate-baseline*`). Mechanical codemod within `Record`
type heads: `(Record … (F T) …)` → `(Record … (: F T) …)`. The ~29 already-ascription cases are already
canonical (no change). MUST run the ML round-trip, not just the gate
([[corpus-edit-must-run-ml-round-trip-not-just-gate]]). Because encode now emits ascription, gate
baselines that carry a rendered `(Record (a T))` in a diagnostic/description regenerate to `(: a T)` —
regenerate baselines with `cargo xtask gate --save` and diff the VERDICT set (additive only, no
`Todo→Fail`). Land RT1+RT2+RT3+RT4 as one coordinated unit (a split reds the round-trip the moment the
head-app spelling is rejected). Corpus-bugfix has claimed RT4 and executes it after RT1–RT3 land.

### Phase B — record VALUE fields canonical `(= name value)` (LARGE; ~838 occ / 17 files)

**RV1 — cadenza-syntax: value-record field arena becomes `(= name value)`.**
`record_literal` (`parser.rs:3163`) reads `name = value` and today builds the bare pair
`(name value)` (`parser.rs:3197`), dropping the `=`. Change it to build the `=`-headed triple
`(= name value)`. Shorthand pun `{x}` (`parser.rs:3181`) → decide the punned node (default `(= x x)`
for uniformity — every field is `=`-headed). The `record` STRING head is unchanged.

**RV2 — cadenza-syntax: printer emits `{ name = value }` from `(= name value)`.**
`print_record` (`printer.rs:2428`) currently renders `(name value)` pairs as `name = value`; point it
at the `(= name value)` triple instead (read the value from the third child). Pun re-sugar
(`is_field_pun` `printer.rs:2522`) updated for the new node. The printed SURFACE `{a = 1}` is
UNCHANGED — only the arena gains the explicit `=`. Round-trip pins it.

**RV3 — rcdzc: read value-record fields from `(= name value)`.**
`read_record_fields` (`resolve.rs:5989`) and its consumers (`infer.rs:1261`, `infer.rs:1563`) read the
`(name value)` pair today; move them to read `(= name value)`. Any value-record construction/lowering
that emits the pair emits the triple. **Gate:** fold + wasmtime unit for `{a = 1}` evaluating to a
record value; existing value-record inference units green.

**RV4 — corpus migration: ~838 `(record (a v) …)` → `(record (= a v) …)` across 17 files.**
`corpus-bugfix` zone. Mechanical codemod within `record` VALUE heads (STRING-headed `record`, distinct
from the NAME-headed `Record` type — the codemod must key off the head to avoid touching type nodes).
This is the big one; script it. MUST run the ML round-trip. Regenerate `.gate-baseline*` and diff the
VERDICT set (additive only). Land RV1–RV4 as one coordinated unit, SEPARATE from Phase A.

## 3. Seams / file anchors

| What | Where |
|---|---|
| **TYPE side** | |
| Colon type-arg → `(: field T)` (already correct) | `cadenza-syntax/src/parser.rs:2249`–`2272` |
| Brace type `{field: T}` → `(: field T)` (already correct) | `parser.rs:3791`–`3824` |
| Head-app `Record(field(T))` spelling to reject/normalize (RT1) | `type_postfix` `parser.rs:3624`; `type_arg` `parser.rs:2270` |
| Type printer: `(: f T)` prints `field: T` (infix, already) | `printer.rs:490`–`494`, `infix` `printer.rs:852` |
| Type-eval ascription readers (ascription becomes sole path) | `rcdzc/src/eval.rs:299`, `eval.rs:3204`, `compile.rs:974`, `db.rs:5615` |
| Pair-only readers to add ascription (RT3) | `reduce_ctor` `eval.rs:2758`; `decode_ty` `resolve.rs:5810` |
| **Encoder flips pair→ascription (RT3, load-bearing)** | `render_name` `ty.rs:1728`; `encode_ty` `ty.rs:1819` (comment `ty.rs:1722`–`1727`) |
| **VALUE side** | |
| Value-record parser: build `(= name value)` (RV1) | `record_literal` `parser.rs:3163`–`3222` (field build `:3197`, pun `:3181`) |
| Value-record printer: emit `{name = value}` from triple (RV2) | `print_record` `printer.rs:2428`–`2439`; pun `is_field_pun` `printer.rs:2522` |
| Value-record field read (RV3) | `read_record_fields` `resolve.rs:5989`; consumers `infer.rs:1261`, `infer.rs:1563` |
| **Shared** | |
| Round-trip fixed-point harness | `cadenza-syntax/tests/corpus_roundtrip.rs`; `assert_canonical_fixed_point` |
| Type corpus (~141 head-app + 29 ascription) | `spec/semantics/*.sexp` |
| Value corpus (~838 `(record …)` / 17 files) | `spec/semantics/*.sexp` |
| Gate baselines (regenerate after encoder flip / migration) | `spec/semantics/.gate-baseline{,-rust,-rust-async}` |

## 4. The gate that protects it

Per phase (each phase is its own gated unit):
1. `cargo test -p cadenza-syntax` — parser produces the canonical node; printer emits the canonical
   surface and never the old implicit form; `corpus_roundtrip` structural round-trip; a focused
   `assert_canonical_fixed_point`.
2. `cargo test -p rcdzc --lib` — type-eval / value-record fold + a wasmtime run; encode↔decode
   round-trip on the ascription form (Phase A).
3. `cargo xtask gate` — regenerate `.gate-baseline*` (the encoder flip + migration change rendered
   descriptions), diff the VERDICT set, ADDITIVE only, no `Todo→Fail` miscompile.
4. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check`. **No `cargo xtask build`** —
   neither phase touches `cdz-runtime` or its frozen hash.
5. Guide chapters showing records round-trip / run.

## 5. Ownership / hand-off

- **Lead: `v-syntax`** — parser + printer + round-trip on both phases (RT1/RT2, RV1/RV2), plus the
  rcdzc reads it can carry (RT3, RV3 are localized).
- **Corpus migration: `corpus-bugfix`** — RT4 (~141 type cases, claimed) and RV4 (~838 value cases);
  runs the ML round-trip + regenerates baselines. RT4 executes after RT1–RT3 land; RV4 after RV1–RV3.
- **rcdzc RT3 (encoder flip + pair-reader ascription arms):** `v-syntax` or a short rcdzc helper /
  `v-inference` — it is the one non-trivial rcdzc change. The dead type-eval pair arms prune later (OQ-C).
- **Guide:** the guide owner, or folded into the lead's unit.

Land order: **Phase A first** (RT1+RT2+RT3+RT4 atomic) — smaller, and the colon surface already parses.
**Phase B second** (RV1+RV2+RV3+RV4 atomic) — the large value-corpus migration, independent.

## 6. Resolved (operator DECISIONS, PR #2794 review 2026-08-08) — do NOT re-litigate

- **Field-separator surface = COLON on types** (`field: T`), for cross-annotation consistency + OCaml
  model. (Was D2.)
- **Canonical type-field arena = `(: field T)` ascription** — the same node as param binders / `e: T`;
  colon already parses to it (no parse-time node rewrite). Migrate the `(field T)` head-app cases to it.
  (Was D1; flipped from the first revision.)
- **Canonical value-field arena = `(= name value)`** — the `=` survives into the arena, symmetric to
  the type-side `:`. `{a = 1}` → `(= a 1)`, not `(a 1)`. (New scope.)
- **Principle: fewer syntax exceptions; the surface token is an explicit AST node; no implicit
  parser rewriting.** The rcdzc encode/decode delta (pair→ascription) is an accepted tradeoff.
- Two independent landables: Phase A (type, ~170 occ) then Phase B (value, ~838 occ / 17 files).

## 7. Open (implementation-local; the vertical picks, cheap to revisit)

- **OQ-A — the obsolete head-app spelling `Record(field(T))`.** Default: **reject** (a record-type
  field must be `field: T`); alternative: normalize its arena to `(: field T)`. Reject is the
  "fewer exceptions / no rewrite" outcome. §2 RT1.
- **OQ-B — value-record shorthand pun `{x}`.** Default: `(= x x)` (every field `=`-headed, uniform).
  §2 RV1.
- **OQ-C — prune the now-dead `(name T)` pair arms in the rcdzc type-eval readers** (`eval.rs:299`/
  `3204`, `compile.rs:974`, `db.rs:5615`) once the type corpus is fully migrated and the round-trip
  proves the pair is extinct. Default: leave in the landing units; prune in a `v-inference` follow-up
  (verify none guard a non-record pair position before removing).
- **Tuple literals (operator NOTE-2, earlier):** value tuple literals `(a, b)` → `(tuple …)` and tuple
  TYPES already exist; only their PRINT canonicalizes to `Tuple(...)`. No implicit-node problem there
  (positional, no dropped separator), so it is OUT of this doc's explicit-node scope. If the operator
  wants type tuples to print as `(A, B)`, that is a separate small print-only follow-up.
