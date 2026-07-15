# Vertical-ready: open sums + schema-typed payloads

**Design doc:** `implementation/design/DESIGN-open-sums-and-schema-typed-payloads-rcdzc.md` (landed on trunk).
**Subsystem:** `rcdzc` (primary: `resolve.rs` / `infer.rs` / `lower.rs`) + a small `cadenza-syntax`
grammar token (`.. r` row-var marker on `(type …)`) + a `spec/semantics/15-rows-and-open-sums.sexp`
corpus migration.
**Promotes:** the 4 `todo` cases in `spec/semantics/15-rows-and-open-sums.sexp:377`–`417`
(baseline `.gate-baseline:2621`–`2625`) → `pass`.

## First increment (OS1) — open-sum DECLARATION + open-tail exhaustiveness (cases 1–2)
Self-contained; no schema work.
1. Grammar: accept trailing `.. r` row-var marker on `(type …)` (`..` already lexes).
2. Resolve: a `(type … .. r)` sum carries an open-tail flag (`sums.rs:225 sum_record`); undeclared
   bare ctors still reject `CDZ0101` (no regression — this is the load-bearing constraint).
3. Exhaustiveness (`lower.rs:7348 build_tree`, reject `:7624`): open sum ⇒ require a `_` arm
   (present → exhaustive, case 1; absent → `CDZ0210`, case 2); a `_` over an open sum is never
   `CDZ0213`-redundant.
4. Migrate cases 1–2 inputs to declare the open sum; `gate --save`.

## Second increment (OS2) — schema decode, constant-fold path (cases 3–4)
Rides OS1. Register `payload-of` / `decode` / `Int64-schema` / `DecodeError` (prelude); fold a
constant payload+schema to a constant `Result`, modelled on `lower_ast_decode` (`lower.rs:2523`) +
`result_discs` (`:15844`). Mismatch → `(Err (DecodeError unit))`, NEVER a trap (§214). Runtime-payload
path is DEFERRED (OQ-4).

## Key resolved decisions (spec-forced — do not re-litigate)
- Open sums are DECLARED via a row variable; closed is the mandatory default (type-system.md §204/§208).
- Open-tail `_` arm mandatory + sufficient for exhaustiveness (§206).
- Schema decode reuses `Result` + value-interchange; only the constant-fold path is in scope.

Gate: `cargo test -p rcdzc --lib` + `cargo test -p cadenza-syntax` (round-trip) + `cargo xtask gate`
(4 cases todo→pass, diff FAIL SET — CDZ0101 pin + closed-sum cases must hold) + `cargo xtask check`.
NO `cargo xtask build` (does not touch cdz-runtime / frozen hash). See doc §5.
