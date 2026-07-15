# Design — record field-update surface: 3-operand `Record.with r #field v` (both surfaces)

**Author:** design pass (fleet `design-record-update-syntax`).
**Audience:** the `vertical` agent(s) that build it — `rcdzc` (special-form arity) + `v-syntax`
(both surface spellings) + a corpus/guide migration.
**Status:** DESIGN. Operator DECISION (2026-07-15) — direction chosen, not open for re-litigation.
**Subsystem:** spans `rcdzc` (special-form operand shape), `cadenza-syntax` (parser + printer, both
surfaces), corpus migration, guide (`RecordsTuples.tsx`).

> **This doc SUPERSEDES its own earlier revision.** The first cut of this file proposed an ML brace
> sugar `{ r with x = 1 }` desugaring to the *old 2-operand* `Record.with`. The operator instead
> DECIDED to change the operand SHAPE of `Record.with`/`Record.extend` on **both** surfaces (s-expr and
> ML), replacing the grouped `(field value)` pair with three positional operands
> `record, #field, value`. The brace-sugar direction is dropped. This revision is the sole canonical
> design; per the canonical-form discipline
> ([[garbage-render-means-not-canonical-fix-the-source]]) the OLD `(field value)`-pair spelling is
> **migrated and rejected**, never kept as a second accepted spelling.

## 0. The problem, and the decision — READ FIRST

**The problem the operator hit.** `Record.with`'s second operand today is a grouped `(field value)`
pair (`resolve.rs::record_op_pair`, `resolved.rs:325`). In the ML surface that pair renders as
`price(9)` — visually a **function call**, so `Record.with({ item = 1, price = 2 }, price(9))` reads
as if `price` is being *called*, not as a field update. It looks broken. (In s-expr,
`(Record.with (record (item 1) (price 2)) (price 9))`, the `(price 9)` is a field-pair special form —
correct, but the ML printer has no distinct spelling for it, so it collides with an application.)

**The decision (operator, via concierge, 2026-07-15).** `Record.with` (and its sibling
`Record.extend`) take **three positional operands** — a record, a **symbol literal** naming the field,
and the value — on **both** surfaces:

| | OLD (2-operand, grouped pair) | NEW (3-operand, symbol literal) |
|---|---|---|
| s-expr | `(Record.with (record (item 1) (price 2)) (price 9))` | `(Record.with (record (item 1) (price 2)) #price 9)` |
| ML | `Record.with({ item = 1, price = 2 }, price(9))` | `Record.with({ item = 1, price = 2 }, #price, 9)` |
| field access (unchanged) | `(. r price)` / `r.price` | `(. r price)` / `r.price` |

The `#price` operand is a **static symbol literal the compiler resolves at compile time** — this is
purely a change of *surface shape* (three positional operands instead of a grouped pair), **NOT** a
change to runtime semantics. The field name stays a static operand the compiler resolves; the row-ops
design is preserved intact (`spec/learnings/2026-07-05-record-and-tuple-reshaping-is-explicit-row-operations.md`;
[[2026-07-05-record-and-tuple-reshaping-is-explicit-row-operations]]). `Record.with` is still
without-then-merge on a **statically-known** field, possibly retyping it; presence/absence is still a
compile-time `CDZ0212`/`CDZ0211`, never a runtime lookup.

**Why a symbol literal, not a runtime symbol (the load-bearing constraint).** The field name a row op
reshapes is deliberately **not a runtime value** — that is what makes these SPECIAL FORMS and keeps
the result shape statically fixed (the emitted component carries a concrete closed record shape, no
runtime field set; `type-system.md` §"A Record Row Is Reshaped Only Through An Explicit Operation
Yielding A New Value"). `#price` is read by the compiler as a **label** at resolve time — exactly as
the old pair's `read_key`-read name was — never demanded as a `Ty::Symbol` value that flows through
inference. The change is: *where* the label comes from (a positional `#sym` operand vs the head of a
grouped pair), not *whether* it is static. **The one thing to get right in review:** the third operand
is the value, whose type flows normally (`with` may retype the field); the `#field` operand is inert.
If a future need ever wants a genuinely *runtime* field name, that is a separate, larger semantic
change (a runtime record with a dynamic field set) — explicitly out of scope here and flagged so.

## 1. Scope: which operations change

The change touches exactly the row ops whose current operand is a **grouped `(name value)` pair** —
the two that render call-like:

- **`Record.with`** — `(Record.with r (z v))` → `(Record.with r #z v)`. **Changes.**
- **`Record.extend`** — `(Record.extend r (z v))` → `(Record.extend r #z v)`. **Changes**, for family
  uniformity (same call-like `z(v)` rendering problem; the operator note calls for the whole field-pair
  family to be uniform).

The other row ops are **unchanged** — their operands are already not `(name value)` pairs:

- `Record.project` / `Record.without` — second operand is a **label LIST** `(a c)`
  (`record_op_labels`), not a pair. It renders as a list, not a call. *No change* (unless the vertical
  finds the list-of-labels also reads poorly in ML — flagged as OQ-2, default leave alone).
- `Record.merge` — two **record VALUES**, no label. *No change.*
- `Record.pop` — a **bare label** `z` (`read_label`). Renders as a bare name, not a call. *No change.*
- `Tuple.cat` / `Tuple.split-at` / `Tuple.pop` — positional, no field names. *No change.*

## 2. The increments (top-to-bottom, the way a vertical lands them)

### RW1 — rcdzc: `Record.with` / `Record.extend` accept 3 operands with a `#symbol` field operand

The core semantic seam. Today both fold under `args.len() == 2` with a `record_op_pair` second
operand (`lower.rs:1401`, `infer.rs:3342`). The change:

1. **Read the field label from a `#symbol` operand.** Extend the label reader so a `#field` symbol
   literal is accepted as a static field label. Anchor: `read_key` (`resolve.rs:4046`) reads a bare
   name or `(meta NAME)` today; add an arm that reads a **symbol-literal node** (the `#"field"` /
   `#field` reader form, resolved at `resolve.rs:297` as a `SymbolConst`) into a plain `Symbol` **at
   resolve time, as a label** — NOT as a `Ty::Symbol` value. (Prefer extending `read_key` so every
   label site could accept `#sym` uniformly; if that widens too much, add a dedicated
   `read_symbol_label` used only by the 3-operand ops.)
2. **Accept the 3-operand shape in resolve/infer/lower.** Replace `record_op_pair` (label + value from
   one grouped node) with a **two-operand read**: `read_key`/`read_symbol_label` on `args[1]` (the
   `#field`) and the ordinary value occurrence `args[2]`. Sites:
   - `lower.rs:1401` — `Some(Prim::RecordExtend | Prim::RecordWith) if args.len() == 3` → insert
     `label ↦ value` into the constant `Core::Record` (reuse `lower_record_insert`, now taking the
     label + value from two operands instead of unpacking a pair).
   - `infer.rs:3342` — infer over `(r, #field, value)`: `#field` a static label, `value`'s type is
     `typeof(value)`; result is `r` with `field` retyped to `typeof(value)`; absent field → `CDZ0212`
     (with) / present field → `CDZ0211` (extend), unchanged.
   - `resolve.rs` — the resolved shape for these prims carries `(record, label, value-occurrence)`.
3. **Reject the OLD 2-operand form.** `(Record.with r (z v))` — the grouped-pair spelling — must no
   longer resolve. Decide the reject: a 2-operand `Record.with` is now an **arity error** (the special
   form wants 3 operands) — the cleanest signal, and it forces the migration rather than silently
   accepting a second spelling ([[garbage-render-means-not-canonical-fix-the-source]]). Confirm the
   arity check fires *before* any attempt to read `(z v)` as a pair (so the diagnostic says "3 operands"
   not "malformed pair").
4. **Gate:** a fold unit (`(Record.with (record (a 1)) #a 9)` folds to `(record (a 9))`), a wasmtime
   run where the value executes, a reject unit for the old 2-operand form (arity error), and the
   existing `CDZ0212`/`CDZ0211` presence/absence rejects re-expressed in 3-operand shape.

### RW2 — cadenza-syntax: parse + print both surfaces

**s-expr** (`sexpr.rs` reader is structural — a 3-element tail `(Record.with r #price 9)` already
parses as a list; the work is that the *printer* emits the new shape). **ML** (`parser.rs` /
`printer.rs`):

- **Parser:** `Record.with(r, #price, 9)` is an ordinary member-access application with three
  comma-separated arguments — the ML call surface already parses this once `#price` lexes as a symbol
  literal (the `#name` unquoted-symbol sugar + `#"name"` already exist,
  [[ml-unquoted-symbol-sugar]]). Confirm `#price` in argument position reads to the same symbol-literal
  arena node RW1 step 1 accepts. Likely **little or no parser change** — the value is that the 3-arg
  call spelling replaces the `price(9)` pair that never had a clean ML form.
- **Printer:** the ML printer must render the resolved `Record.with`/`Record.extend` as
  `Record.with(r, #field, value)` — three arguments, the middle a `#field` symbol literal — NOT the old
  `Record.with(r, field(value))`. Anchor: wherever member-access applications print (the name-head
  application dispatch in `printer.rs`; the record/map/list/tuple *literal* re-sugar at `printer.rs:306`
  is the model for recognizing a prim head). The old field-pair rendering path is removed.
- **Round-trip gate:** `assert_canonical_fixed_point` holds on the new shape — `read → print → read`
  identity for `(Record.with r #price 9)` and `Record.with(r, #price, 9)`; and the printer NEVER emits
  the old `(price 9)` / `price(9)` form.

### RW3 — corpus + guide migration (the OLD form is rejected, so every use moves)

Because the new form is the **sole canonical spelling**, every existing `Record.with`/`Record.extend`
use migrates in lockstep with RW1 (a split landing would red the gate):

- **Corpus** — `spec/semantics/15-rows-and-open-sums.sexp`: the `with`/`extend` cases (lines ~155, 164,
  172, 181, 190) migrate `(Record.with … (b 9))` → `(Record.with … #b 9)`, `(Record.extend … (b 2))` →
  `(Record.extend … #b 2)`. Add a **negative case for the old form** asserting an arity error, so the
  rejection is witnessed. `(needs rows)` tag unchanged.
- **Guide** — `guide/src/content/chapters/RecordsTuples.tsx` (~6 `Runnable`/`solution` sources at lines
  34, 41, 44, 48, 52, 58, 174, 179): migrate each `Record.with base (hp 99)` / `(price 9)` /
  `Record.extend … (z 3)` to the `#field value` form, and update the prose that says "give it a record
  and a …" to describe the `#field, value` shape. (Note the guide runs Cadenza in-browser via jco,
  [[browser-guide-jco-execution-path]] — the migrated sources must actually run.)
- **Codemod** — a mechanical rewrite `(Record.with R (F V)) → (Record.with R #F V)` (and `extend`) over
  the corpus is a candidate for the `cdz` codemod tool; the vertical decides whether to script it or
  hand-migrate the ~small count.
- **Learning / decision docs** — `options/record-tuple-operations/namespaced-row-operations.md` and the
  2026-07-05 learning describe the `(z v)` pair operand; update them to the `#z v` 3-operand shape.
  Update the `type-system.md` §"A Field Is Added To Or Replaced In A Record" wording if it pins the
  operand shape (it pins semantics, likely no change — confirm).

## 3. Seams / file anchors

| What | Where |
|---|---|
| `Record.with` / `Record.extend` prim docs (operand shape) | `rcdzc/src/resolved.rs:319`–`329` |
| Fold sites (change `args.len() == 2` → `== 3`, drop `record_op_pair`) | `rcdzc/src/lower.rs:1401`; `lower_record_insert` at `lower.rs:16796` |
| Infer sites | `rcdzc/src/infer.rs:3342` (pair read at `:3345`) |
| Label reader to extend for `#symbol` | `rcdzc/src/resolve.rs:4046` (`read_key`); pair reader at `:4035` (`record_op_pair`) removed for these ops |
| Symbol-literal resolve (`#"x"` → `SymbolConst`) | `rcdzc/src/resolve.rs:297` |
| `CDZ0211`/`CDZ0212` codes (unchanged) | `rcdzc/src/diag.rs:327`–`328` |
| ML symbol-literal sugar `#name` (already exists) | [[ml-unquoted-symbol-sugar]]; `cadenza-syntax` lexer/parser |
| ML application printer (emit `Record.with(r, #f, v)`) | `cadenza-syntax/src/printer.rs` (name-head application dispatch; model at `:306`) |
| Round-trip fixed-point harness | `assert_canonical_fixed_point` (v-syntax) |
| Corpus cases to migrate | `spec/semantics/15-rows-and-open-sums.sexp` (~lines 155, 164, 172, 181, 190) |
| Guide cases to migrate | `guide/src/content/chapters/RecordsTuples.tsx` (~lines 34, 41, 44, 48, 52, 58, 174, 179) |
| Decision/learning to update | `options/record-tuple-operations/namespaced-row-operations.md`; `spec/learnings/2026-07-05-record-and-tuple-reshaping-is-explicit-row-operations.md` |

## 4. The gate that protects it

1. `cargo test -p rcdzc --lib` — fold unit + wasmtime run for `Record.with r #f v`; reject unit for the
   old 2-operand form; `CDZ0212`/`CDZ0211` presence/absence rejects in 3-operand shape.
2. `cargo test -p cadenza-syntax` — parser reads `Record.with(r, #price, 9)`; printer emits it and
   never the old form; `assert_canonical_fixed_point` round-trip.
3. `cargo xtask gate` — the migrated `(needs rows)` cases evaluate; the old-form negative case rejects.
   Diff the FAIL SET; additive only (the migration flips old-form cases from pass to the new spelling —
   ensure no `Todo→Fail` miscompile).
4. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check`. **No `cargo xtask build`** —
   this touches neither `cdz-runtime` nor its frozen hash (a resolve/infer/lower + surface change only).
5. Guide: the migrated `RecordsTuples.tsx` `Runnable` sources actually run in-browser (jco path).

## 5. Ownership / hand-off

This spans two subsystems and a migration — one coordinated vertical, or a lead + coordinator:

- **Lead: `v-syntax`** (owns `cadenza-syntax` — the parser/printer for both surfaces + the
  round-trip harness). Owns RW2 and drives RW3's corpus round-trip.
- **Coordinating `rcdzc` change: RW1** — the special-form arity + `#symbol` label read + reject of the
  old form. Small and localized (the anchors in §3); v-syntax can carry it, or a short-lived rcdzc
  helper vertical, coordinating so RW1+RW2+RW3 land **together** (a split landing reds the gate — the
  old form is rejected the moment RW1 lands, so the corpus/guide must migrate in the same unit).
- **Guide migration (RW3):** whoever owns the guide chapter, or folded into the lead's unit.

Land order within the single unit: RW1 (reject old + accept new) ⟂ RW3 corpus migration must be
**atomic** (same commit); RW2 printer must emit the new form in the same unit (else the round-trip
gate reds). Practically: one branch, one merge-request, gated whole.

## 6. Resolved (operator DECISION, 2026-07-15) — do NOT re-litigate

- **3 positional operands `(record, #field, value)`**, not the grouped `(field value)` pair. Both
  surfaces change (s-expr AND ML). Chosen to fix the `price(9)`-looks-like-a-call rendering.
- **`#field` is a STATIC symbol literal the compiler resolves** — purely a surface-shape change, NOT a
  runtime symbol. Row-ops design (static field name, statically-fixed result shape) preserved.
- **The OLD 2-operand form is migrated + rejected**, not kept as a second spelling (canonical-form
  discipline). Every corpus + guide use moves in the same landing.
- **`Record.extend` gets the same treatment** for family uniformity. `project`/`without` (label list),
  `merge` (record values), `pop` (bare label), and the `Tuple.*` positional ops are unaffected.

## 7. Open decisions (chosen default — flag to the vertical, cheap to revisit)

- **OQ-1 — reject shape for the old form.** Default: **arity error** (special form wants 3 operands).
  Alternative: a dedicated migration diagnostic ("`Record.with` now takes `r #field value`"). Default
  is simplest; the dedicated message is friendlier if the churn warrants it. The vertical picks.
- **OQ-2 — do `project`/`without`'s label-LIST operands read poorly in ML too?** They render as a list
  `(a c)`, not a call, so they don't hit the `price(9)` problem. **Default: leave them alone** (out of
  this decision's scope). If the operator later wants list-of-`#symbol` uniformity, that's a follow-up.
- **OQ-3 — `#field` label namespace.** `read_key` also reads `(meta NAME)` namespaced labels. Confirm a
  `#field` operand only ever names a plain field label (records have no meta-namespaced fields in the
  value surface); default: plain labels only, reject a namespaced `#(meta x)` operand here.
- **OQ-4 — does the ML parser need ANY change,** or does `Record.with(r, #price, 9)` already parse as a
  3-arg member-access call once `#price` lexes as a symbol literal? Spike expected: **no parser change**
  (the `#name` sugar + N-arg call surface both exist); the work is the printer + the rcdzc operand read.
  Confirm in RW2 step 1.
