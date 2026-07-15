# Vertical-ready: ML-surface record-update syntax `{ r with x = 1 }`

**Design doc:** `implementation/design/DESIGN-record-update-syntax.md` (landed on trunk via pr-sync).
**Subsystem:** `cadenza-syntax` (the ML reader/printer — front-end only; NO rcdzc/runtime change).
**Scope:** front-end sugar over the already-shipped `Record.with` row op. Zero IR nodes, zero new
diagnostic codes (absent field = existing `CDZ0212`, inherited from `Record.with`).

**First increment — RU1 (Reader):** in `cadenza-syntax::parser::record_literal` (`parser.rs:1965`),
add a top-of-literal fork: speculatively parse a leading expression; if it stopped at the `with`
keyword, parse a `,`-separated `name = value` field list and desugar to a left-nested
`(Record.with (Record.with r (f1 v1)) (f2 v2))` chain; otherwise rewind to the existing `name = e`
field loop. `with` is already a contextual keyword that stops an expression (`token.rs::Keyword::With`,
`parser.rs:298/:320`), so the form is LL-unambiguous with the existing literal + shorthand.

**Then RU2 (Printer round-trip)** and **RU3 (corpus + spec witness)** — see the design doc §3.

**Gate:** `cargo test -p cadenza-syntax` (reader/printer/negative units) + `assert_canonical_fixed_point`
round-trip + `cargo xtask gate` (`(needs rows)` ML cases: positive update evals, `{ r with z=1 }` absent
→ `(error CDZ0212)`) + `cargo xtask check`. No `cargo xtask build` (runtime untouched).

**Suggested owner:** a `vertical` agent, area=`cadenza-syntax` (could fold into the existing
`v-syntax` vertical, which already owns the ML front-end round-trip harness).
