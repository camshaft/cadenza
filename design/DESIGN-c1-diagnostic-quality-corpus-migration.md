# C1 — migrating diagnostic-quality `#[test]`s to the corpus

**Status:** template published; migration STAGED (do it per-lane once seq-276 + the rational flag-days land — coordinate with v-corpus-harness before a fleet-wide push). Owner of the harness + grade: `v-corpus-harness`; owner of each test's *intent*: the lane that wrote it.

## Why

A compiler diagnostic's QUALITY — its stable `CDZ####` code, its message, and its structural
fix-it (a replace/insert/wrap/delete edit an agent can apply) — is behaviour, and behaviour belongs
in the executable-semantics corpus (operator: e2e→conformance, not in-crate `#[test]`). One source of
truth, graded the same way for every backend, instead of ~1376 rust asserts drifting from the corpus.
The corpus grader (`cdz-corpus-grade::grade_diag_quality`) already grades every facet a fix-it test
needs — this is a MIGRATION, not new machinery.

## What a corpus `(error …)` / `(warning …)` case can assert

Inside a trial's outcome clause (graded against the compiler's captured `KIND_DIAGNOSTICS` wire):

- `(error CODE)` / `(warning CODE)` — the refusal/warning must carry exactly `CODE`.
- `(message "substr")` — a load-bearing substring of the diagnostic prose (repeatable; ALL required).
- `(fix (kind K) (replacement "…") (verified | unverified))` — the structural repair:
  - `kind` ∈ `replace | insert | wrap | delete` (+ the spellings the wire emits, e.g. `insert-into`);
  - `replacement` — the surface payload (the substitute for `replace`, the appended form(s) for
    `insert`, the wrap text with a `…` hole for `wrap`); matched by a `ReplMatch` mode;
  - `verified` iff the compiler PROVED the fix, else `unverified` (an agent should confirm before applying).
- `(no-fix)` — the fault must carry NO repair (mutually exclusive with `(fix …)`).
- `(count N)` / `(once)` — the EXACT number of `(severity, code)` faults (`once` == `count 1`).
- `(no-other-errors)` — the emitted error-severity codes must be a SUBSET of the asserted `(error …)` codes.
- `(declines [CODE] msg?)` — the compiler must refuse; an optional leading `CDZxxxx` pins the decline's
  code (e.g. `CDZ0900`, the not-yet-built umbrella). A different/absent code grades `Todo`.

## Recipe: one `#[test]` → one corpus case

1. Find the test's INPUT program + the diagnostic it asserts (code, message, and — if it checks a fix —
   the `Fix` kind + replacement + verified flag).
2. Author a case in the matching `spec/semantics/NN-*.sexp`:
   ```
   (case "<the test's intent, as a title>"
     (input (do … the program …))
     (error CODE (message "the load-bearing phrase") (fix (kind replace) (replacement "…") (verified))))
   ```
   Input compound value literals MUST be native `#ctor` form (`#list`/`#record`/…) — run
   `cdz corpus nativize-check` (or the codemod). Pin only what the test MEANT to assert (code + the
   load-bearing message phrase + the fix facets it checked) — not incidental prose.
3. DELETE the migrated `#[test]` in the same change (the corpus case is now the source of truth).

## Worked examples (already in the corpus — the pattern is proven, 72 `(fix …)` + 18 `(no-fix)` cases)

- `replace`: `spec/semantics/11-modules.sexp` — `(error CDZ0102 (fix (kind replace) (replacement "x2") (unverified)))`
- `wrap`: `spec/semantics/10-bytes.sexp` — `(error CDZ0203 (fix (kind wrap) (replacement "(String.to-bytes …)") (unverified)))`
- `insert-into`: `spec/semantics/07-type-system.sexp` — `(fix (kind insert-into) (replacement "(_ (trap \"TODO\"))"))`
- `delete`: `spec/semantics/11-modules.sexp` — `(error CDZ0201 (message "declared more than once") (fix (kind delete)))`
- `no-fix` + `once`: see the `(no-fix) (once)` cases (e.g. the `CDZ0201` no-fix cases).

## Verify a migrated case

- `cargo xtask roundtrip <file>.sexp` — the case must survive the ML round-trip (parse↔print byte-stable).
- Gate it: the fix-it facets grade ONLY when the compiler's `KIND_DIAGNOSTICS` wire is captured
  (`diag_wire` `Some`); a case that pins a fix but the wire wasn't captured grades on code+message alone.
- Prefer the differential check for regressions: pure-`origin/main` `gate --check` vs branch `gate --check`,
  diffing the SORTED regressed-case SETS — the native absolute count is base-confound noise.

## Notes

- The `KIND_DIAGNOSTICS` wire is flipping text→binary-AST (seq-254; `cadenza_compile_abi::decode_diagnostics`).
  This is orthogonal to authoring: the assertion facets grade against the parsed faults regardless of wire
  encoding, so migrated cases are wire-flip-safe.
- STAGING (concierge): publish + a proof tranche now; hold the fleet-wide per-lane push until seq-276 and
  the rational flag-days land. Coordinate the tranche boundaries with v-corpus-harness (grade + baseline hygiene).

## Message-ABSENCE assertion — `(not "phrase")` (operator seq-29)

Complement of the positive `(message "phrase")` (contains) substring pin: `(not "phrase")` requires the
diagnostic to NOT contain `phrase`. Repeatable + AND-d, and composes with the positive pins on `(error …)`,
`(declines …)`, and `(warning …)`:

```
(declines CDZ0900 (message "not yet") (not "internal error"))
(error CDZ0201 (message "malformed record") (not "panic"))
```

Semantics (graded on the sexp `test-run.ast` path): after the code + every positive substring match, the
grade FAILS if the diagnostic contains ANY `(not …)` phrase. This lets a message-ABSENCE rust test (one that
only asserts `!diagnostic.contains("X")`) move into the corpus rather than staying a rust `#[test]` (the
motivating case: #6127). The flat direct-gate manifest ignores `(not …)` (rich prose pins are sexp-path-only).
