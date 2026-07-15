# Design — open sums (extensible variants) + schema-typed payloads

**Author:** design pass (fleet `design-open-sums-schema`).
**Audience:** the `vertical` agent that builds it — spans `rcdzc` (declaration surface, exhaustiveness
over an open sum, the decode primitive) with a `v-syntax` touch (row-variable spelling on `(type …)`)
and a corpus migration of the 4 TODO cases.
**Status:** DESIGN. Decision 1 (open-sum surface) is **spec-forced** — the spec text already pins it;
Decision 2 (schema decode) carries a chosen default with the open sub-questions flagged.
**Subsystem:** `rcdzc` primarily (`resolve.rs` / `infer.rs` / `lower.rs`), a small `cadenza-syntax`
grammar addition for the row-variable marker, and a `spec/semantics/15-rows-and-open-sums.sexp`
migration of the 4 open-sum/schema cases from `todo` to graded.

## 0. The problem — READ FIRST

`spec/semantics/15-rows-and-open-sums.sexp` carries **4 cases graded `todo` (declined)** — the tail of
the file, lines ~377–417:

1. *"a match on an open sum with an open-tail arm is exhaustive"* — `(match e ((Known _) …) (_ …))`
   over a value `(Known unit)`, expecting `"known"`.
2. *"a match on an open sum omitting the open-tail arm is rejected"* — the same match **without** the
   `_` arm, expecting `CDZ0210`.
3. *"an open sum's payload decodes against a schema to a typed result"* —
   `(decode Int64-schema (payload-of (Measured 7)))`, expecting `(Ok 7) : (Result Int64 DecodeError)`.
4. *"an open sum payload that does not match its schema yields a typed failure, not a trap"* —
   `(decode Int64-schema (payload-of (Labeled "x")))`, expecting `(Err (DecodeError unit))`.

These grade `todo` **correctly**: the file header states rows/open-sums are what "a later generation
realizes; the seed … does not realize row polymorphism or open sums." They are **not a trunk bug** — a
prior `fix` agent mapped the whole pipeline and confirmed the seed's decline is conformant. Promoting
them to `pass` needs surface **design**, which is this doc. Two independent surfaces are unpinned:

- **Decision 1 — the open-sum declaration + open-tail exhaustiveness** (cases 1–2). The cases use
  ctors `Known` / `Unknown` **bare and undeclared**. Today an undeclared capitalized head rejects
  `CDZ0101` (`resolve.rs`). What declares a sum *open*, and how does a match's `_` arm satisfy
  exhaustiveness over it?
- **Decision 2 — schema-typed payload decode** (cases 3–4). `decode` / `payload-of` / `Int64-schema` /
  `DecodeError` are four undesigned names — no prims, no schema value, no `DecodeError` type. Spec
  §210–214 pins *behavior* (a typed `Result`; a mismatch is an `Err`, never a trap) but not the surface.

## 1. Decision 1 — open sums are DECLARED, closed is the default (SPEC-FORCED)

**This is not a free choice — the spec text already decides it.** `type-system.md` §"A Sum Type May Be
Open, With A Mandatory Open-Tail Arm" (§202–208):

> §204 — "A program MUST be able to **declare an open sum** — a variant set that MAY carry a **row
> variable** standing for variants not named …"
> §208 — "A **closed sum MUST remain the default**: a sum declared without a row variable is closed,
> and the abstract syntax tree type MUST be a closed sum …"

So the two candidate directions the prior map floated resolve as follows:

- **(1a) anonymous polymorphic variants** — any undeclared capitalized head in value/pattern position
  becomes an open variant. **REJECTED, and it violates the spec.** §208 makes closed the *mandatory
  default* and requires open-ness to come from an explicit row variable; an implicit "any undeclared
  ctor is open" rule contradicts that MUST. It would also **regress the `CDZ0101` detection pin**
  (`07-type-system.sexp:47` `(: 5 Foo)` — a type-position uppercase-unbound name must stay `CDZ0101`)
  and mask typo'd constructor names as silently-open variants.
- **(1b) explicit open-sum declaration via a row-variable marker on `(type …)`** — **CHOSEN, because
  the spec mandates it.** No typo regression (an undeclared bare ctor still rejects `CDZ0101`); the
  open-tail arm becomes a *checked* totality rather than an unchecked fallthrough.

### 1.1 The surface: a row-variable marker on `(type …)`

A closed sum is declared today as `(type T (A Int64) (B Bool))`
(`05-compound-types.sexp`). An **open** sum adds a **trailing row-variable marker** naming the open
tail. The chosen spelling mirrors the list-rest marker `..` the language already uses
(`02-binding-and-control.sexp:2543`, `(list x .. rest)`), so open-ness reads the same everywhere:

```
(type T (Known Int64) (Unknown String) .. r)
```

- `.. r` in trailing position marks `T` as **open**, with `r` the row variable standing for the
  variants not named. `r` is a lowercase name (a type variable, like `a` in `(type T (Leaf a) …)`).
- **Closed stays the default:** a `(type …)` with no trailing `.. r` is closed exactly as today. The
  AST sum and every existing corpus sum are unchanged (§208's "AST MUST be a closed sum" holds for
  free — no `.. r`, so closed).
- A **value of an open sum** is constructed exactly as a closed one: `(Known 7)`, `(Unknown "x")`. The
  ctors must be **declared** in the `(type …)` — the open tail `r` covers *unknown* variants that
  arrive across a boundary, not undeclared local names. So `(Known unit)` in case 1 requires the case
  input to first `(type … (Known …) .. r)`; **the 4 case inputs must be rewritten** to declare the
  open sum (recorded-oracle change → this design is the sign-off).

> **OQ-1 — the marker token.** Default `.. r` (reuse the list-rest `..`, lowest grammar cost, reads
> consistently). Alternatives the vertical may weigh with v-syntax: a keyword head `(type T … (.. r))`
> or a dedicated `(open-type …)` form. `.. r` chosen for surface economy; cheap to revisit before RW1
> lands since it is only a grammar token.

### 1.2 Exhaustiveness over an open sum: the `_` arm is MANDATORY and SUFFICIENT

§206 — "A match on an open sum MUST carry an open-tail arm covering the variants not named, and a match
that omits it MUST be a compile-time rejection." This inverts the *closed*-sum rule at exactly one
seam, the exhaustiveness verdict in lowering:

- **Closed sum (today):** a match is exhaustive iff every declared variant is covered; a trailing `_`
  is *optional* (and if all variants are already covered, a `_` is redundant → `CDZ0213`).
- **Open sum (new):** a match is exhaustive iff it covers the named arms **and carries a `_` (open-tail)
  arm**. A match over an open sum **without** a `_` arm is **never** exhaustive — because the row
  variable `r` stands for variants the match cannot enumerate — so it rejects `CDZ0210` (case 2). A
  `_` arm over an open sum is **never** redundant (it is the only thing covering `r`), so the
  `CDZ0213` redundant-arm rule must **not** fire on it.

The runtime representation is unchanged: an open sum is a tagged value exactly like a closed sum (the
learning §"Open sums compose with monomorphization/erasure" — "the runtime representation is a tagged
value, and the open tail is just the tags a given match does not name"). The `_` arm compiles to the
same default arm a closed-sum wildcard does. **Open-ness is purely a compile-time typing/exhaustiveness
property** — no ABI or heap change.

## 2. Decision 2 — schema-typed payload decode (`decode` / `payload-of` / `«T»-schema` / `DecodeError`)

Cases 3–4 pin the *behavior* (§210–214): a variant's payload decodes against a run-time-resolved schema
to a typed `Result`; a mismatch is an `Err`, never a trap. Four names are undesigned. The design **reuses
the existing value-interchange surface** (`value-interchange.md`) rather than inventing a parallel one —
the learning is explicit that schema decode "is the reader/printer at a data boundary" and "the result
is a `Result`, not a trap," which is precisely what value-interchange's *decode-inverts-encode /
header-mismatch-yields-absence* rule already specifies.

### 2.1 The four names, grounded

| name | what it is | grounding |
|---|---|---|
| `payload-of` | extracts an open sum variant's payload as an **opaque, schema-checkable value** (the analogue of the "opaque bytes interpreted only against the declaration" in the learning §16) | a prim reading the variant's single payload slot |
| `«T»-schema` (`Int64-schema`) | the **schema value for type `T`** — the run-time type witness `decode` is directed by (value-interchange §"decode is directed by a known type", §83) | a per-type schema constant, the type-identity value; **OQ-2** below |
| `decode` | `∀t. Schema t → Payload → (Result t DecodeError)` — decode the payload against the schema, yielding `Ok`/`Err` | value-interchange §"Decode Inverts Serialize And Refuses Otherwise" + §63 (header mismatch → absence, no payload decode) |
| `DecodeError` | the failure type in the `Err` arm — a **1-variant sum** `(DecodeError unit)` (matches case 4's `(Err (DecodeError unit))`) | a prelude sum, the value-interchange "absence of a value" on mismatch, reified as a typed error |

### 2.2 Why this reuses `Result` + the value-heap constant path

- **`Result` already exists** (the prelude sum, used by `try`, `lower_ast_decode`'s `result_discs` at
  `lower.rs:15844`). `decode` builds an `Ok`/`Err` exactly as `lower_ast_decode` does
  (`lower.rs:2523`) — that function is the **direct precedent**: it takes bytes and a target type and
  builds a `Result` by discriminant, folding to a constant when the input is constant.
- **All 4 cases' payloads are CONSTANT** — `(Measured 7)`, `(Labeled "x")` — so the decode **folds to a
  constant `Result` at compile time** (verified in the prior pipeline map). The runtime value-heap path
  (a genuinely runtime payload) is **out of scope for these 4 cases** and deferred (OQ-4) — the vertical
  builds only the constant-fold path to promote the corpus, and flags the runtime path as a follow-up
  increment, so the slice stays small.
- **A schema mismatch is a typed `Err`, not a trap** (§214) — `decode` on an `Int64-schema` against a
  `String` payload (case 4) returns `(Err (DecodeError unit))` by construction; it never emits an
  `unreachable`/trap. This is the value-interchange §63 rule (mismatch → absence) reified as
  `Result.Err`.

### 2.3 Open sub-questions on Decision 2 (chosen default, flag to the vertical)

- **OQ-2 — the schema value `Int64-schema`.** Default: a **per-type schema constant** in the prelude,
  the type's run-time identity witness (the "schema-identity function over the canonical form of a
  type" — value-interchange §85). Simplest realization for the constant-fold cases: `«T»-schema`
  resolves to a compile-time type witness the fold reads to pick the decode target; no runtime schema
  registry needed for constant payloads. Alternative (larger): a first-class `Schema` type with a
  runtime registry keyed by `(kind, version)` (the full learning story). **Default = compile-time
  witness only**, enough for the 4 cases; the runtime registry is the deferred general form.
- **OQ-3 — `payload-of`'s type.** The payload's static type is not known until the variant is matched
  (learning §18). For the constant cases it is inferable from the constant ctor. Default: `payload-of`
  yields an **opaque payload value** whose only consumer is `decode` (which re-types it via the
  schema); it is not a general projection. The vertical confirms `payload-of` need not be a
  general-purpose prim for these cases.
- **OQ-4 — runtime-payload decode.** Deferred. The 4 cases are all constant-fold. A runtime payload
  (decode on the value heap) is a **second increment** the vertical files as a follow-up, not part of
  the corpus-promoting slice.
- **OQ-5 — is `DecodeError` richer than `(DecodeError unit)`?** Case 4 pins exactly `(DecodeError unit)`
  — a nullary-payload error. Default: `DecodeError` is a **1-variant sum with a `unit` payload**,
  matching the recorded oracle. A richer error (carrying the expected/actual schema) is a follow-up; do
  not over-build past the pinned oracle.

## 3. The increments (top-to-bottom, the way a vertical lands them)

### OS1 — rcdzc + syntax: the open-sum DECLARATION and exhaustiveness (cases 1–2)

The self-contained first slice; promotes 2 of the 4 cases and needs **no** schema work.

1. **Grammar (`cadenza-syntax`):** accept a trailing `.. r` row-variable marker on `(type …)`. The
   `..` token already lexes (list-rest); the parser adds a trailing-`.. name` arm to the `(type …)`
   production. Round-trip: `read → print → read` identity for `(type T (Known Int64) .. r)`.
2. **Resolve (`resolve.rs`):** a `(type …)` with a trailing `.. r` resolves to a sum carrying an
   **open-tail flag** (a `bool` / row-var name on the sum's declaration record — the sum is an ordinary
   record, `sums.rs:225 sum_record`). Undeclared bare ctors still reject `CDZ0101` (no regression).
3. **Exhaustiveness (`lower.rs`):** at the verdict seam `build_tree` (`lower.rs:7348`, reject at
   `:7624 non_exhaustive_sum_reject`): if the scrutinee sum is **open**, require a `_` (open-tail) arm —
   present → exhaustive (case 1); absent → `CDZ0210` (case 2). Suppress the `CDZ0213` redundant-arm
   verdict for a `_` over an open sum (it is never redundant).
4. **Corpus migration:** rewrite cases 1–2's inputs to **declare** the open sum
   (`(type T (Known Int64) .. r)` … `(Known unit)` / `(Unknown unit)`), then grade them (`gate --save`).
5. **Gate:** a fold unit + a wasmtime run for case 1 (`"known"` executes); a reject unit for case 2
   (`CDZ0210` by code); the `07-type-system.sexp:47` `CDZ0101` pin still holds.

### OS2 — rcdzc: schema-typed payload decode, constant-fold path (cases 3–4)

Rides OS1 (an open sum exists to carry a payload).

1. **Prelude names:** register `payload-of`, `decode`, `«T»-schema` (at least `Int64-schema`), and the
   `DecodeError` sum (`prelude.rs`, alongside `Option`/`Result` at `prelude.rs:793`).
2. **Lower (`lower.rs`):** `decode` folds a **constant** payload + schema into a constant `Result`,
   modelled on `lower_ast_decode` (`lower.rs:2523`) + `result_discs` (`lower.rs:15844`): schema matches
   the payload's type → `(Ok v)`; mismatch → `(Err (DecodeError unit))`, **never a trap** (§214).
3. **Infer (`infer.rs`):** `decode : Schema t → Payload → (Result t DecodeError)`; case 3 infers
   `(Result Int64 DecodeError)`.
4. **Corpus migration:** cases 3–4 grade to their pinned `(Ok 7)` / `(Err (DecodeError unit))` oracles
   (their inputs already declare no sum — they may need the same `(type …)` open-sum preamble as OS1 to
   introduce `Measured`/`Labeled`; the vertical confirms and rewrites, `gate --save`).
5. **Gate:** a fold unit for `decode` (Ok + Err arms fold to constants); a wasmtime run where `(Ok 7)`
   executes; **assert no trap** on the mismatch case (case 4 returns `Err`, does not `unreachable`).

## 4. Seams / file anchors

| What | Where |
|---|---|
| Open-tail row-var grammar on `(type …)` | `cadenza-syntax` parser (`(type …)` production); `..` token already lexes |
| Sum declaration as a record (add open-tail flag) | `rcdzc/src/sums.rs:225` (`sum_record`), `:428` (`variant_ctor`) |
| Ctor resolve / `CDZ0101` for undeclared heads | `rcdzc/src/resolve.rs` (undeclared capitalized head → `CDZ0101`) |
| Exhaustiveness verdict (require `_` when open) | `rcdzc/src/lower.rs:7348` (`build_tree`), reject `:7624` (`non_exhaustive_sum_reject`) |
| Non-exhaustive surfaced for check | `rcdzc/src/lower.rs:2791` (`match_nonexhaustive_fault`) ← `infer.rs:9135` |
| `CDZ0210` / `CDZ0213` codes | `rcdzc/src/diag.rs:326` / `:329` |
| Result-building precedent for `decode` | `rcdzc/src/lower.rs:2523` (`lower_ast_decode`), `:15844` (`result_discs`) |
| Prelude sum registration (model for `DecodeError`) | `rcdzc/src/prelude.rs:793` (`Option`), `:79` (generic type ctors) |
| Value-interchange behavior (decode/mismatch→absence) | `spec/capabilities/value-interchange.md` §57, §63, §83, §85 |
| The 4 cases to migrate | `spec/semantics/15-rows-and-open-sums.sexp:377`–`417` |
| Baseline entries (todo → graded) | `spec/semantics/.gate-baseline:2621`–`2625` |

## 5. The gate that protects it

1. `cargo test -p rcdzc --lib` — OS1: open-sum exhaustive fold + wasmtime run (case 1), `CDZ0210`
   reject (case 2). OS2: `decode` Ok/Err folds + wasmtime run (case 3), no-trap on mismatch (case 4).
2. `cargo test -p cadenza-syntax` — `(type T … .. r)` round-trips (read→print→read identity).
3. `cargo xtask gate` — the 4 cases flip `todo → pass`; **diff the FAIL SET** — the `07-type-system`
   `CDZ0101` pin and every closed-sum exhaustiveness case must stay put (a `Todo→Fail` on any closed-sum
   case is a miscompile, not a landing). `gate --save` the 4 flipped cases.
4. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check`. **No `cargo xtask build`** —
   this touches `rcdzc` resolve/infer/lower + a grammar token + prelude names, **not** `cdz-runtime`
   or its frozen hash.

## 6. Ownership / hand-off

One `vertical` agent (area = `rcdzc`), landing OS1 then OS2 (OS2 rides OS1). The grammar token (OS1
step 1) is a small `cadenza-syntax` touch the same vertical carries (coordinate with `v-syntax` if the
`..` marker collides with the list-rest lexing — expected clean, since `(type …)` is a distinct
production). The corpus migration lands **in the same unit** as each rcdzc slice (a split landing would
red the gate — the moment the open-sum declaration resolves, the case inputs must already declare the
sum).

## 7. Resolved vs. open

**Resolved (spec-forced, do NOT re-litigate):**
- **Open sums are DECLARED via an explicit row variable; closed is the mandatory default** (§204/§208).
  Anonymous-variant (1a) is rejected — it violates §208 and regresses the `CDZ0101` pin.
- **The open-tail `_` arm is mandatory and sufficient** for exhaustiveness over an open sum; a match
  omitting it is `CDZ0210` (§206). A `_` over an open sum is never `CDZ0213`-redundant.
- **Schema decode reuses `Result` + the value-interchange surface** (typed result, mismatch → `Err`
  never a trap, §210–214) — modelled on `lower_ast_decode`.
- **Only the constant-fold path is in scope** for the 4 cases; the runtime-payload path is deferred.

**Open (chosen default, cheap to revisit):**
- **OQ-1** — row-var marker token (`.. r` default).
- **OQ-2** — schema value realization (compile-time type witness default; runtime registry deferred).
- **OQ-3** — `payload-of` typing (opaque payload consumed only by `decode`).
- **OQ-4** — runtime-payload decode (deferred to a second increment).
- **OQ-5** — `DecodeError` shape (`(DecodeError unit)` per the pinned oracle; richer error deferred).
