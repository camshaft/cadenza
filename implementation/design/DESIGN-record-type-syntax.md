# Design — record TYPE field encoding: unify the dual arena, one canonical field surface

**Author:** design pass (fleet `design-record-type-syntax`).
**Audience:** the `vertical` agent(s) that build it — `cadenza-syntax` (parser + printer, both
surfaces + round-trip harness), `rcdzc` (the type-eval / decode readers), and a corpus migration
(`corpus-bugfix`).
**Status:** DESIGN. One operator decision is OPEN (the field-separator token, §7 D2) and is being
surfaced as an `ask`; everything else has a chosen default. The doc lands now with the default so a
vertical can start; a flip of D2 is a one-token change to the surface + printer (§2 flags where).
**Subsystem:** spans `cadenza-syntax` (parser normalization + printer + round-trip), `rcdzc`
(type-eval / decode-ty readers), corpus migration, and a touch of guide.

## 0. The problem, and the shape of the fix — READ FIRST

**The problem the operator hit.** A record TYPE declaration renders as
`Record(field(UInt8), fieldb(Int64))`. Each field `field(UInt8)` reads as a **function call**, not
as "field `field` has type `UInt8`". It looks broken — the same class of complaint that drove the
`Record.with` `price(9)`-looks-like-a-call fix ([[DESIGN-record-update-syntax]]). The operator wants
it to read cleanly, e.g. `Record(field = UInt8, fieldb = Int64)` (their stated preference), and
floated going further — using record/tuple **literal** syntax as the type surface, plus tuple
literals.

**Why this is NEEDS-DESIGN, not a printer tweak (v-syntax's findings, built on — not re-derived):**

1. The current output is the faithful print of the arena `(Record (field UInt8) …)` — each field a
   NAME-headed 2-list `(field UInt8)` (head-application), printed by the generic call form as
   `field(UInt8)` (`cadenza-syntax/src/printer.rs:707`). Fixing the *rendering* alone is not enough;
   the field separator the operator wants (`=`) does not parse in type position at all.
2. `field = UInt8` does **not** parse today — there is no `=` handling in any type production
   (`Kind::Eq` is the binding separator only, `token.rs:39`); `Record(field = UInt8)` errors with a
   generic `expected \`,\`` (`parser.rs:571`), and `{field = UInt8}` errors `expected \`:\``
   (`parser.rs:3812`). Delivering `=` REQUIRES a parser grammar change.
3. A second surface already parses and round-trips: `Record(field: UInt8)` and brace `{field: UInt8}`
   both produce a **different** arena — the NAME(`:`)-headed 3-list `(: field UInt8)` ascription
   (`parser.rs:2249`, `parser.rs:3791`) — NOT `(field UInt8)`.
4. **The crux: the encoding is already DUAL and both shapes coexist in the corpus.** Head-application
   `(Record (field T))` ≈ 141 non-comment occurrences (dominant); ascription `(Record (: field T))`
   = 29 occurrences. Inference reads BOTH on the type-eval path (`rcdzc/src/eval.rs:299`,
   `eval.rs:3204`), but two readers are **pair-only** — `reduce_ctor` (`eval.rs:2758`) and the
   encode/decode round-trip `decode_ty` (`resolve.rs:5810`) — and the encoder `encode_ty` /
   `render_name` (`ty.rs:1729`, `ty.rs:1819`) emit **only** the `(field T)` pair. So `(field T)` is
   already the *encoder-canonical* arena; `(: field T)` is a secondary ML-lowering surface that
   inference tolerates.
5. The VALUE-side record literal already uses `=`: `{a = 1}` → `(record (a 1))`, a STRING-headed
   2-list `(name value)` pair (`parser.rs:3163`). So `field = T` for types would be consistent with
   the value-record `=` literal — which is the operator's instinct.

**The shape of the fix.** Two things, cleanly separable:

- **(Core, D1 — recommended, low-risk) Collapse the dual arena to ONE canonical field encoding:**
  the `(field T)` 2-list pair, mirroring the value-record `(name value)` pair. The `(: field T)`
  ascription arena is eliminated from record-type context — normalized away on parse and migrated
  out of the corpus. This is the real defect (two encodings for one concept, with asymmetric
  reader coverage); it is worth doing regardless of the surface token.
- **(Surface, D2 — OPEN operator decision) Pick ONE field-separator surface and print it as a clean
  infix** — `field = T` (operator's stated preference; matches value-record `=`) OR `field: T`
  (matches every other type annotation in Cadenza + the OCaml record model; already parses). Either
  fixes the `field(UInt8)`-looks-like-a-call rendering. This is the one fork worth the operator's
  eye (§7 D2) — surfaced as an `ask` with a recommendation; the doc defaults to the operator's
  stated `=` so a build can start.

Per the canonical-form discipline ([[garbage-render-means-not-canonical-fix-the-source]],
[[DESIGN-record-update-syntax]]): whichever surface is chosen, the *other* field spellings are
migrated + normalized/rejected — never kept as a second accepted spelling.

## 1. D1 — the canonical arena encoding: `(field T)` pair (recommended, design-internal)

Adopt the **NAME-headed 2-list `(field T)`** as the sole canonical record-type field encoding.
Eliminate the ascription triple `(: field T)` from record-type context.

Rationale (why `(field T)`, not `(: field T)`):

- It is the **dominant** corpus shape (~141 vs 29).
- It is what the encoder **already emits** — `encode_ty` / `render_name` (`ty.rs:1729`, `ty.rs:1819`)
  produce only the pair; `decode_ty` (`resolve.rs:5810`) and `reduce_ctor` (`eval.rs:2758`) read only
  the pair. Choosing the pair means the encode/decode round-trip and the ground reducer need **no**
  new colon arm; choosing the triple would force a colon arm into both (larger, riskier).
- It **mirrors the value record**: `(record (a 1))` (value) and `(Record (a Int64))` (type) then
  share the identical `(name payload)` field structure, differing only in head (`record` STRING vs
  `Record` NAME) and in what the payload denotes (a value vs a type). This structural parallel is
  exactly the "records as the type surface" instinct, realized in the arena.

What changes for the ascription surface: the parser stops producing `(: field T)` for record-type
fields and instead **normalizes to `(field T)`** at parse time (see RS1). The tolerant colon arms in
`rcdzc` (`eval.rs:299`, `eval.rs:3204`, `compile.rs:974`, `db.rs:5615`) become dead for
record-context fields once no producer emits the triple; leave them in place initially (harmless,
still exercised by any non-record ascription) and let a follow-up prune them once the corpus is
migrated and the round-trip proves the triple is gone (flagged OQ-3).

## 2. The increments (top-to-bottom, the way a vertical lands them)

> **D2 parameterization.** Below, `SEP` is the chosen field separator token (default `=` per the
> operator's stated preference; `:` if D2 flips). Every SEP-specific site is called out so a flip is
> a localized change, not a re-plan.

### RS1 — cadenza-syntax: parse the chosen field surface → canonical `(field T)`

The parser accepts the `SEP` surface for a record-type field and produces the canonical `(field T)`
pair (NOT the ascription triple).

- **Application form** `Record(field SEP T, …)`: today `type_arg` (`parser.rs:2249`) detects
  `Ident` + `Colon` and builds `(: label ty)`. Change it so a record-type field `label SEP ty`
  builds the 2-list `(label ty)` head-application instead.
  - If `SEP = =` (default): add an `Ident` + `Eq` arm to `type_arg` that consumes `=` and builds
    `(label ty)`. (`=` currently errors here, `parser.rs:571`.)
  - If `SEP = :`: keep the existing `Ident` + `Colon` detection but build `(label ty)` instead of
    `(: label ty)` — i.e. normalize the colon field to the pair.
- **Brace form** `{field SEP T, …}`: `type_brace_record` (`parser.rs:3791`) currently `expect`s
  `Colon` and builds `(: label ty)` under a `Record` NAME head. Change it to expect `SEP` and build
  `(label ty)`. (This is the "record literal as type surface" entry point — see RS4 / OQ-1 for
  whether it also PRINTS as a brace.)
- **Bare ascription unaffected:** the general Pratt `:` ascription operator on *expressions*
  (`e : T`, `PREC_ASCRIPTION`) and parameter binders `name: Type` (`param`, `parser.rs:3475`) are a
  DIFFERENT construct (value ascription / binder typing) and are OUT of scope — they keep `:`. Only
  record-type *field* positions are normalized. (If D2 = `:`, take care the field `:` and the
  ascription `:` stay distinguishable by position — field `:` is inside a `Record(...)` / `{...}`
  type head; this is why normalizing to the pair arena at parse time matters.)
- **Gate:** parser unit tests — `Record(field SEP UInt8)` and `{field SEP UInt8}` both read to
  `(Record (field UInt8))`; the OLD unchosen field spelling is rejected (or, for `:` under a `=`
  default, decide per OQ-2).

### RS2 — cadenza-syntax: print the canonical `(field T)` as `field SEP T`

The printer must render a record-type field as the clean infix `field SEP T`, never `field(T)`.

- Today a NAME-headed `Record(...)` type node prints via the generic call form
  (`printer.rs:707`–`709` → `plain_call`, `printer.rs:766`), so `(field UInt8)` recurses to
  `field(UInt8)`. Add recognition: when printing the args of a `Record` type head, a 2-list
  `(name ty)` field prints as `name SEP ty` (infix), not as an application.
  - Anchor the recognition where the record/map/list/tuple *value* literal re-sugar already keys off
    a known head (`printer.rs:342`–`356`, `print_record` at `printer.rs:2428`) — the model for
    "recognize a prim head and re-sugar its children".
  - `SEP = =` renders `field = UInt8`; `SEP = :` renders `field: UInt8`.
- **Round-trip gate (the milestone that pins this):** `corpus_roundtrip.rs` asserts
  `read_ml(print_ml(x)) structurally_eq x` for every corpus `(input …)`. After RS1+RS2, a record
  type round-trips through the new surface and lands back on the canonical `(field T)` arena; the
  printer NEVER emits `field(UInt8)` for a record-type field. Add a focused
  `assert_canonical_fixed_point` unit for `(Record (a Int64) (b Bool))`.

### RS3 — rcdzc: confirm the type-eval / decode path on the sole `(field T)` arena

Because `(field T)` is already the encoder-canonical form, this is mostly a *confirmation +
narrowing*, not a rewrite:

- `typeval_of_uncached` (`eval.rs:3204`) and `type_in_env` (`eval.rs:299`) already accept the pair;
  they keep working unchanged.
- The pair-only readers `reduce_ctor` (`eval.rs:2758`) and `decode_ty` (`resolve.rs:5810`) already
  match the canonical form — no colon arm needed (this is the payoff of choosing the pair).
- The tolerant colon arms (`eval.rs:299`/`3204`/`compile.rs:974`/`db.rs:5615`) become dead for
  record-context fields once RS1 stops producing the triple. **Do not remove them in this unit**
  (they may still see a triple from a not-yet-migrated corpus case mid-migration, and some may guard
  non-record ascription in payload positions — `db.rs:5615`'s comment cites a type-param collection
  case). Removal is OQ-3, a clean follow-up after the corpus is fully migrated and the round-trip
  proves the triple is extinct.
- **Gate:** `cargo test -p rcdzc --lib` — existing record type-eval units stay green; add one that a
  `(Record (a Int64))` type-evaluates and a value of that shape executes under wasmtime.

### RS4 — corpus + guide migration (the 29 ascription cases move; head-app cases are arena-stable)

- **Head-application cases (~141):** arena is UNCHANGED (`(field T)` stays `(field T)`), so their
  gate VERDICTS in `.gate-baseline{,-rust,-rust-async}` do not flip. Their *printed* ML surface
  changes (`field(T)` → `field SEP T`), which the round-trip test validates structurally — no
  baseline text edit needed (the baselines pin verdicts, not printed source).
- **Ascription cases (29):** their `.sexp` source arena `(: field T)` must migrate to `(field T)`.
  A mechanical codemod within `Record` heads: `(Record … (: F T) …)` → `(Record … (F T) …)`. Files:
  `05-compound-types.sexp` (7), `15-rows-and-open-sums.sexp` (4), `14-effects-and-handlers.sexp` (3),
  `03-equality-and-observation.sexp` (2), `07-type-system.sexp` (2), `06-numeric-model.sexp`
  (`:622`, `:633`), and the remainder. `corpus-bugfix` owns this (`.sexp` edits are the corpus-bugfix
  zone) and MUST run the ML round-trip, not just the gate ([[corpus-edit-must-run-ml-round-trip-not-just-gate]]).
- **Migration is ATOMIC with RS1 if the triple is REJECTED** (canonical discipline): the moment the
  parser stops accepting `(: field T)` in record context, un-migrated corpus cases red the
  round-trip. Two safe land orders (the vertical picks): (a) **normalize-not-reject first** — RS1
  normalizes both `:`-field and (if default) `=`-field to the pair while STILL accepting the old
  input during a transition, migrate the corpus, then flip to reject the old spelling in a follow-up;
  or (b) **one atomic unit** — RS1 reject + RS4 corpus migration in the same commit (mirrors
  [[DESIGN-record-update-syntax]]'s RW1⟂RW3 atomic landing). Default: (a) is gentler on the gate;
  (b) is cleaner. Flag OQ-2.
- **Guide:** any `implementation` guide chapter that shows a record TYPE (e.g. `RecordsTuples.tsx`)
  updates to `field SEP T`. Coordinate with the guide owner.

## 3. Seams / file anchors

| What | Where |
|---|---|
| Type-arg parser (labeled `:` detection → build `(field T)`) | `cadenza-syntax/src/parser.rs:2249`–`2272` |
| Brace record TYPE parser (`{field SEP T}`) | `cadenza-syntax/src/parser.rs:3791`–`3824` |
| `=` currently rejected in type-arg position | `parser.rs:571` (`sep_continue`) |
| `:` rejected in brace type | `parser.rs:3812` (`expect(Colon, …)`) |
| Value record literal (the `=` precedent) | `parser.rs:3163`–`3222` (`record_literal`) |
| Type-node print (generic call → the `field(T)` render to fix) | `printer.rs:707`–`709`; `plain_call` `printer.rs:766`–`784` |
| Value literal re-sugar dispatch (model for field re-sugar) | `printer.rs:342`–`356`; `print_record` `printer.rs:2428` |
| Round-trip fixed-point harness | `cadenza-syntax/tests/corpus_roundtrip.rs`; `assert_canonical_fixed_point` |
| rcdzc type-eval (both arms today; pair stays) | `rcdzc/src/eval.rs:299`–`304`, `eval.rs:3204`–`3208` |
| rcdzc pair-only readers (no change needed) | `reduce_ctor` `eval.rs:2758`; `decode_ty` `resolve.rs:5810` |
| Encoder (emits pair only — confirms canonical) | `rcdzc/src/ty.rs:1729`, `ty.rs:1819` |
| Tolerant colon arms (dead post-migration; prune in OQ-3) | `eval.rs:299`/`3204`, `compile.rs:974`, `db.rs:5615` |
| Corpus ascription cases to migrate (29) | `spec/semantics/{05,15,14,03,07,06}-*.sexp` |
| Gate baselines (verdicts; head-app unaffected) | `spec/semantics/.gate-baseline{,-rust,-rust-async}` |

## 4. The gate that protects it

1. `cargo test -p cadenza-syntax` — parser reads `Record(field SEP T)` and `{field SEP T}` to
   `(Record (field T))`; printer emits `field SEP T` and NEVER `field(T)`; `corpus_roundtrip`
   structural round-trip holds; `assert_canonical_fixed_point` on `(Record (a Int64) (b Bool))`.
2. `cargo test -p rcdzc --lib` — record type-eval units green; one wasmtime run where a value of a
   `(Record (a Int64))` type executes.
3. `cargo xtask gate` — diff the FAIL SET, ADDITIVE only. Head-app verdicts unchanged; the 29
   migrated ascription cases stay at their prior verdict (arena `(field T)` now, same meaning). No
   `Todo→Fail` miscompile.
4. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check`. **No `cargo xtask build`** —
   touches neither `cdz-runtime` nor its frozen hash (a parser/printer + corpus change).
5. Guide chapters showing record types actually round-trip / run.

## 5. Ownership / hand-off

One coordinated vertical, lead + coordinators:

- **Lead: `v-syntax`** — owns the parser normalization (RS1), the printer re-sugar (RS2), and the
  round-trip harness. This is the bulk of the work and the milestone gate.
- **`rcdzc` (RS3):** confirmation + a type-eval unit; small, `v-syntax` can carry it or a short rcdzc
  helper. The colon-arm prune (OQ-3) is a clean follow-up owned by `v-inference`.
- **Corpus migration (RS4):** `corpus-bugfix` (the `.sexp` zone), running the ML round-trip.
- **Guide:** the guide owner, or folded into the lead's unit.

Land order: prefer normalize-first (RS1 accepts old + new, produces canonical) → corpus migrate
(RS4) → printer to new surface (RS2) → flip to reject old spelling (follow-up). This keeps every
intermediate gate green (OQ-2 default (a)).

## 6. Resolved (design-internal defaults — not re-litigating unless the operator flips D2)

- **D1: canonical arena = `(field T)` 2-list pair** (recommended, low-risk). Ascription `(: field T)`
  eliminated from record-type context. Mirrors the value-record `(name value)` pair; the
  encode/decode round-trip and ground reducer need no colon arm.
- **The `field(UInt8)`-looks-like-a-call rendering is the concrete defect** both surface options fix.
- **One canonical field spelling** (canonical-form discipline) — the unchosen spelling is
  migrated + normalized/rejected, never a kept alternative.

## 7. Open decisions (chosen default — the vertical / operator can revisit)

- **D2 — the field-separator surface (OPERATOR decision; surfaced as an `ask`).** Default = `=`
  (operator's stated preference; matches value-record `{a = 1}`). Alternative = `:`.
  - **Case for `=`:** consistency with the value-record literal; realizes "a record type is spelled
    like a record value with types in the fields" (`{x = Int64}` parallels `{x = 1}`); the operator's
    instinct.
  - **Case for `:` (decision-relevant, possibly new to the operator):** it is consistent with EVERY
    OTHER type annotation in Cadenza — parameter binders `name: T`, expression ascription `e: T`, the
    existing brace type `{field: T}` — whereas `=` in a type position is novel and could misread as
    "defaults to" / "aliases to". And the *current* split — value `{a = 1}` with `=`, type `{a: T}`
    with `:` — is precisely the **OCaml record model** (OCaml: value `{ x = 1 }`, type `{ x : int }`),
    a principled, well-precedented distinction rather than an accident. `:` also already parses +
    round-trips, so it is less parser work.
  - The doc builds on `=` today; a flip to `:` changes only the `SEP` token at the RS1/RS2 anchors.
- **D3 — record/tuple LITERALS as the type surface + tuple literals (operator NOTE 2; scope fork).**
  - **Record type as brace literal.** RS1 already accepts `{field SEP T}`. OPEN: should it also
    *print* as a brace `{field SEP T}` (full "literal as type surface"), or keep printing the
    `Record(field SEP T)` application head? **Default: keep `Record(...)` head spelling in this
    unit** (smaller, and the app form is unambiguous); brace-printing is a clean Phase-2 follow-up
    once the field-separator settles. (OQ-1.)
  - **Tuple types.** `(A, B)` already PARSES as a tuple type (`type_paren`, `parser.rs:3760`) but
    canonicalizes to `Tuple(A, B)` on print. Making it print as `(A, B)` is the tuple analog of the
    brace-record-type question — **same Phase-2 default: keep `Tuple(...)` for now.**
  - **"Adding tuple literals."** Value tuple literals `(a, b)` → `(tuple …)` ALREADY EXIST
    (`parser.rs:1863`, `print_tuple` `printer.rs:2410`), as do tuple TYPES. **Flag to the operator:**
    what specifically is missing here? (Possibly: printing type tuples as `(A, B)`, per above; or a
    1-tuple / nesting nuance.) Captured in the `ask` so the operator can pin the intent; default is
    "nothing missing on the value side — this reduces to the Phase-2 tuple-type print question".
- **OQ-1 — brace-print the record type?** Default no (keep `Record(...)`). Phase-2.
- **OQ-2 — reject the old field spelling immediately, or normalize-then-reject?** Default:
  normalize-first, reject in a follow-up (gentler gate). §2 RS4.
- **OQ-3 — prune the now-dead tolerant colon arms in rcdzc.** Default: leave in this unit; prune in a
  `v-inference` follow-up once the corpus is migrated and the round-trip proves the triple is extinct
  (some arms may still guard non-record ascription — verify before removing).
