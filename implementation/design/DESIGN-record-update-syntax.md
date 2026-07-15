# Design — ML-surface functional record-update syntax `{ r with x = 1, y = 2 }`

**Author:** design pass (fleet `design-record-update-syntax`).
**Audience:** the `vertical` agent that owns the ML front-end crate (`cadenza-syntax`), + future me.
**Status:** DESIGN. Not started. Front-end-only — the *semantics* it desugars to are already shipped.
**Subsystem:** `cadenza-syntax` (the ML reader/printer). No `rcdzc` change; no runtime change.

## 0. The one-sentence summary — READ FIRST

The record row-update *semantics* already exist and are already realized in `rcdzc`
(`Record.with`, defined in `options/record-tuple-operations/namespaced-row-operations.md`, resolved
by `resolved.rs`, inferred by `infer.rs`, coded `CDZ0212` on an absent field). What is missing is an
**ML-surface sugar** for the common case — "this record, but with these fields changed" — because
today the only way to write it in the ML surface is the namespaced special form
`Record.with(r, x = 1)` chained by hand. This feature adds **one grammar production** to the record
literal, `{ r with x = 1, y = 2 }`, that **desugars at read time** to nested `Record.with` and
**round-trips through the printer**. It adds **zero** IR nodes, zero resolver work, zero runtime
work, and zero new diagnostic codes — a field named in `with` that is absent from `r` is the existing
`Record.with` → `CDZ0212`, for free.

The through-line: *this is a reader/printer feature, exactly like the map `.. rest` spread and the
`#(…)` set literal — surface sugar over an already-pinned s-expr form.*

---

## 1. What it is

A new form inside the existing `{ … }` record braces:

```
{ r with x = 1, y = 2 }
```

reads "the record `r`, but with fields `x` and `y` replaced by the given values." It desugars, at
read time in `cadenza-syntax::parser`, to a left-nested chain of the already-pinned `Record.with`
special form:

```
{ r with x = 1, y = 2 }
  ==>  (Record.with (Record.with r (x 1)) (y 2))
```

The desugar is **left-to-right field order** (first-written field is the innermost `with`), which is
observationally irrelevant — each `with` names a distinct present field, so the order in which they
apply cannot change the result — but is fixed so the printer's inverse is deterministic.

### Semantics: update-only (chosen fork)

Every field named in the `with` clause **MUST already be present** in `r`. Naming an absent field is
a compile-time **`CDZ0212`** — inherited verbatim from `Record.with`, which the desugar targets. This
was the operator's decision (2026-07-15): the sugar means **change values**, never **grow shape**.
Adding a field stays the explicit `Record.extend`, and the extend/with split
(`options/record-tuple-operations/namespaced-row-operations.md` §"Record derived operations") is
preserved unchanged — the sugar does not blur it into a JS-spread add-or-replace.

Because it desugars to `Record.with`, a field's replacement value MAY be of a different type than the
field held (the result is a new closed record whose field `z` has whatever type the new value holds —
`type-system.md` §"A Field Is Added To Or Replaced In A Record By A Derived Operation"). No new rule.

### What it is NOT

- **Not add-or-replace.** `{ r with z = 1 }` where `z ∉ r` is `CDZ0212`, not a silent add. (Rejected
  fork; see §6.)
- **Not a map spread.** The map literal's `#{ 1 = v, .. rest }` spread (`parser.rs::map_literal`) is a
  *different* construct with *different* semantics (dynamic keys, last-writer-wins over a runtime map).
  Record update reuses neither its `..` token nor its semantics. The two stay visually distinct: a map
  is `#{…}`, a record is `{…}`, and record update is gated on the leading-expression-then-`with` shape.
- **Not a value-level mutation.** `r` is unchanged; a new record value is derived (immutable acyclic
  heap). This is `Record.with`'s contract, inherited.

---

## 2. Why the surface disambiguates cleanly (the load-bearing observation)

`with` is **already a contextual keyword** (`token.rs::Keyword::With`, used today by `handle … with`).
The lexer is keyword-free — `with` lexes as `Kind::Ident` — and the parser recognizes it by text and
position. Critically, `with` **already terminates an expression**: it is in the `at_expr_stop` /
`at_outer_close` keyword sets (`parser.rs:298`, `parser.rs:320`). That is exactly what makes the record
literal grammar unambiguous with **one token of lookahead after parsing a leading expression**:

- `{ x = 1, y = 2 }` — the existing record literal. First field is `x`, next token `=`.
- `{ x }` — the existing field-shorthand pun (`{ x = x }`). First field is `x`, next token `}` or `,`.
- `{ r with x = 1 }` — the new update form. Parse a leading expression `r`; because `with` stops the
  expression, the parser lands on `with` and switches to the update production.

The disambiguation rule: **at the top of a `{`-literal, speculatively parse a leading expression; if
the token that stopped it is the `with` keyword, it is an update; otherwise rewind and parse the
existing `name = e` / shorthand field list.** In practice the leading expression of an update is
almost always a bare name (`r`), and a bare name followed by `with` cannot begin a field list (a field
is `name = …` or `name ,`/`name }`), so the two productions are LL-distinguishable. The rewind is
bounded (a single leading expression) and only taken when the first `{`-item is *not* immediately
`name =` / `name ,` / `name }` — the common literal case pays no rewind.

> **Anchor for the implementer:** `record_literal` is entered from `parser.rs:818`
> (`Kind::LBrace => self.bracketed_bars(Self::record_literal)`). The new fork lives at the top of
> `record_literal` (`parser.rs:1965`), before the existing field loop.

---

## 3. The increments (top-to-bottom, the way a vertical lands them)

### RU1 — Reader: parse `{ r with f = v, … }` → nested `Record.with`

The core increment. Edit `cadenza-syntax::parser::record_literal` (`parser.rs:1965`):

1. After consuming `{`, if the contents are non-empty, **peek for the update shape**. Save the cursor.
   Parse one expression at the record-field precedence (`PREC_SEQ + 1`). If the parser is now **at the
   `with` keyword**, this is an update; keep the parsed expression as `r`. Otherwise **rewind** to the
   saved cursor and fall through to the existing `name = e` field loop unchanged.
2. On the update path: consume `with`. Parse a **`,`-separated non-empty list of `name = value`
   fields** — the *same* per-field parse the literal already does (`binder` + `=` + `expr(PREC_SEQ+1)`),
   factored into a shared helper so field-shorthand and error-recovery behavior match the literal
   exactly. (Open question OQ-1: is field shorthand `{ r with x }` allowed? Default: **no** — an update
   value is always explicit `name = value`; a bare `name` in update position is a missing-`=` error.
   Rationale: shorthand puns "bind field to same-named scope value," which is a *literal*-construction
   convenience; in an update the intent is to state a new value, so requiring `=` keeps the read honest.
   Cheap to relax later.)
3. **Desugar** the parsed `(r, [(f1,v1), (f2,v2), …])` into a left-nested chain:
   `(Record.with (Record.with r (f1 v1)) (f2 v2)) …` — built with the arena `list` helper, the head of
   each `Record.with` being the member-access form `(. Record with)` exactly as the s-expr surface
   produces it. Reuse whatever the existing reader emits for a `Mod.op` member access so the desugared
   head is byte-identical to a hand-written `Record.with`.
4. **Gate:** a reader round-trip unit — `{ r with x = 1 }` reads to the expected arena; the arena
   matches a hand-written `(Record.with r (x 1))`. A `{ r with x = 1, y = 2 }` case pins the left-nest
   order. A negative parse case: `{ r with x }` (no `=`) reports the missing-`=` error, not a panic.

### RU2 — Printer: round-trip the sugar

Teach `cadenza-syntax::printer` to render a left-nested `Record.with` chain **whose innermost operand
is not itself a `Record.with`** back to `{ r with … }`. Anchor: the `list` dispatch at
`printer.rs:306` (where `record`/`map`/`list`/`tuple` literals are recognized and re-sugared) is the
model, but `Record.with` is a member-access application, not a string-headed ctor — so the re-sugar
hook goes where member-access applications are printed, recognizing the head `(. Record with)`.

- **Detection:** an application whose head is `(. Record with)` and whose second argument is a
  `(field value)` pair. Walk the left spine collecting `(field value)` pairs until the innermost
  operand is a non-`Record.with` expression `r`; render `{ <r> with f1 = v1, f2 = v2, … }` with the
  pairs in spine order (innermost = first written, the RU1 desugar's inverse).
- **Fidelity guard:** only re-sugar when the reader would parse the result back to the identical arena
  — i.e. the innermost operand `r` must print as something a `with`-update's leading expression accepts
  (any expression is fine; `with` stops it). A `Record.with` a user wrote *explicitly* in s-expr also
  re-sugars to `{ … with … }` — that is intended and correct (they denote the same value), consistent
  with the printer's existing policy that `("record" …)` and `(record …)` both sugar to `{…}`.
- **Gate:** the corpus round-trip fixed-point test (`assert_canonical_fixed_point`, the syntax
  vertical's harness) must hold: `read → print → read` is identity on every `{ r with … }` case, and
  `print` of a hand-written `(Record.with r (x 1))` yields `{ r with x = 1 }` which reads back
  identically.

### RU3 — Corpus + spec witness

- Add ML-surface cases to the row corpus. `spec/semantics/15-rows-and-open-sums.sexp` holds the s-expr
  `Record.with` cases and carries `(needs rows)`; the ML-surface twin belongs wherever the vertical
  keeps ML-surface round-trip cases (the syntax vertical's all-surface coverage — commit `b4e811d2c`
  added "corpus-wide binary + all-surface round-trip coverage"; follow that shape). Cases:
  a positive update, a two-field update (order pin), and a negative `{ r with z = 1 }` where `z ∉ r`
  asserting `(error CDZ0212)`.
- Note the sugar in `options/record-tuple-operations/namespaced-row-operations.md` under a new
  "**Surface sugar**" subsection: `{ r with f = v, … }` is the ML surface for a chain of `Record.with`,
  desugaring at read time, update-only (absent field = `CDZ0212`), with `extend` staying explicit. This
  is a *surface* note, not a semantics change — the s-expr `Record.with` remains the canonical form.

---

## 4. Seams / file anchors

| What | Where |
|---|---|
| Record-literal reader (add the update fork at top) | `implementation/seed/crates/cadenza-syntax/src/parser.rs:1965` (`record_literal`) |
| `{`-dispatch into `record_literal` | `parser.rs:818` |
| Per-field parse to factor into a shared helper | `parser.rs:1977`–`1995` (binder + `=` + `expr`) |
| `with` is already a keyword & already stops an expr | `token.rs::Keyword::With`; `parser.rs:298`, `:320` |
| Printer literal re-sugar model (record/map/etc.) | `implementation/seed/crates/cadenza-syntax/src/printer.rs:306` |
| Member-access application print (where the `Record.with` re-sugar hooks) | `printer.rs` name-head/application dispatch |
| Round-trip fixed-point harness | `assert_canonical_fixed_point` (syntax vertical) |
| The semantics being desugared to (NO edit — reference) | `rcdzc/src/resolved.rs:328` (`Record.with`), `infer.rs:3326`; `diag.rs:328` (`CDZ0212`) |
| Decision doc to annotate | `options/record-tuple-operations/namespaced-row-operations.md` |

---

## 5. The gate that protects it

Standard fleet gate, with the syntax-vertical additions:
1. `cargo test -p cadenza-syntax` — new reader unit (`{ r with x = 1 }` → expected arena), printer
   round-trip unit, negative missing-`=` parse unit.
2. `assert_canonical_fixed_point` over the new corpus cases — `read→print→read` identity, and
   `print(hand-written Record.with) == { … with … }` reading back identically.
3. `cargo xtask gate` — the `(needs rows)` ML cases: positive update evaluates, negative
   `{ r with z=1 }` (absent) is `(error CDZ0212)`. Diff the FAIL SET, additive only.
4. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check`.

No runtime rebuild (`cargo xtask build`) is needed — this touches neither `cdz-runtime` nor its frozen
hash.

---

## 6. Resolved forks (operator, 2026-07-15)

- **Surface = `{ r with x = 1, y = 2 }`** (OCaml/F#-style `with`), NOT `{ ..r, x = 1 }` (JS spread) and
  NOT "no sugar / `Record.with` only." Chosen because `with` is already a keyword that already stops an
  expression, so the form is unambiguous with the existing `{ name = e }` literal and `{ name }`
  shorthand at one token of lookahead, and because `..`-spread connotes add-or-replace, which the spec
  deliberately forbids for records.
- **Update-only** (desugar to `Record.with` only), NOT update-or-add (per-field `with`/`extend`). An
  absent field is `CDZ0212`, not a silent `extend`. Preserves the pinned extend/with split — the sugar
  means "change values," growing the shape stays the explicit `Record.extend`. A convenience
  add-or-replace MAY be offered later only as an elaboration that provably rewrites to `with`/`extend`
  without changing emitted bytes (the same posture the decision doc already reserves).

## 7. Open decisions (with a chosen default — flag to the vertical, cheap to revisit)

- **OQ-1 — field shorthand in an update?** `{ r with x }` meaning `{ r with x = x }` (bind field to
  the same-named scope value). **Default: NO** — an update field is always explicit `name = value`; a
  bare `name` is a missing-`=` error. Rationale in RU1(2). Relaxable later without a grammar conflict.
- **OQ-2 — empty update `{ r with }`?** **Default: reject** as a parse error (a `with` with no fields
  is almost certainly a mistake; `r` alone already means "unchanged"). The vertical MAY instead accept
  it as `r` unchanged (the empty chain), matching `Record.without r ()` = `r`; low stakes either way.
- **OQ-3 — nested/chained leading expression.** `{ f(x) with a = 1 }` (leading expr is a call, not a
  bare name). The design says the leading operand is *any* expression (`with` stops it), so this Just
  Works; the only cost is the speculative-parse rewind is taken for any first-item that is not
  `name =`/`name ,`/`name }`. Confirm the rewind is bounded and cheap (it parses one expression). No
  default needed — it falls out of §2.
