# Operating principles for the Lean oracle

The Lean oracle exists to be an **independent** check on the Cadenza language: a from-scratch model
whose disagreements with rcdzc surface real compiler bugs, spec ambiguities, and under-specified
corners. These principles keep it independent and keep it lean. They bind every increment.

## 1. Clean-room: build from the spec and the corpus, never from the Rust implementation

Implement the language semantics — name resolution, typechecking, const-folding (`reduce`), execution
(`execute`), traps, and diagnostics — **only** from:

- `spec/` — `constitution.md`, `overview.md`, `capabilities/*`, `semantics/*`, `contracts/*`;
- the corpus, `spec/semantics/*.sexp`, whose recorded `(output …)` / `(error …)` / `(trap …)` is the
  behavioral source of truth.

Do **not** read rcdzc's semantic passes (`implementation/seed/crates/rcdzc/*` — `eval.rs`, resolve,
typecheck, lower) to decide what the oracle should do. If the oracle just reproduces the Rust
implementation, the two share a single definition and the differential finds nothing — the
independence *is* the value. When the spec is silent, ambiguous, or appears wrong, do **not** guess
and do **not** crib the Rust answer: raise a concierge `ask` with the concrete question and options;
another agent resolves the spec; the oracle then implements from the resolved spec. (Design §5 trust
model: a confirmed disagreement is exactly one of rcdzc-bug | oracle-bug | spec-ambiguity.)

**The one nuance — serialization codecs.** The binary-AST format (`spec/contracts/ast-encoding.md`)
and the value form (`spec/contracts/deterministic-value-form.md`) are shared *transport* formats the
oracle must byte-match to read the harness's input at all; language bugs do not live in serialization.
Their concrete byte layout is currently defined only in the seed implementation, not pinned in a spec
(see §4) — so byte-matching the frozen format is treated as interop, not a semantic judgment, and
codec-matching must never bleed into how the semantics are decided.

## 2. Corpus-conformance is the quality signal — not a big Lean test suite

The primary measure of the oracle's quality and coverage is **running the corpus against it**
(`cargo xtask oracle-check`, increment L1.2): every realized case must agree with the recorded
expectation, and coverage grows by flipping cases from `Unsupported` to `Pass`. That harness is both
the coverage metric and the anti-redundancy discipline.

Do **not** grow a large suite of Lean unit tests that re-check example-by-example behavior the corpus
already exercises — the Rust side already suffers from heavy self-test/corpus overlap; do not repeat
it here. Before adding a Lean test, ask: *does a corpus case already cover this?* If yes, rely on
`oracle-check`. Reserve Lean-native tests for what the corpus **cannot** reasonably check — full
**proofs of correctness** (totality, determinism, stage-parity `reduce` ≡ `execute`, codec
round-trip / injectivity laws) — plus the minimal gate witnesses no corpus case can stand in for
(e.g. the frame round-trip smoke).

## 3. Two stages: `reduce` (const-fold) vs `execute` (run an input)

Model the two things the compiler actually does as **separate** stages, and hold the compiler to each:

- `reduce  : BinaryAstModule → Reduced` — compile-time const-evaluation of a program to its
  minimal/normal form (mirrors rcdzc's const-folding). Grades a bare `(input E)` corpus case.
- `execute : Reduced × Trial → Outcome` — runtime execution of a supplied input against the reduced
  program. Grades a `(call entry args)` trial (runtime args defeat const-folding).

Both are pure and deterministic in `(modules, args, hostResponses)`; host effects are modeled by
feeding the fixed `hostResponses` in call order and recording `hostCalls`. Separating the stages buys
a second independent check — **stage parity**: const-evaluating a closed program must equal executing
it, and a fold-vs-run divergence in rcdzc is a miscompile the oracle surfaces directly. Build the
separation in from the skeleton onward. (Design §1.1 + §6.)

## 4. Known spec finding — the AST-encoding contract vs the concrete format

`spec/contracts/ast-encoding.md` describes an abstract "symbol prelude" that is namespaced,
optionally versioned, and referenced by index. The concrete seed format (`cadenza-ast/src/codec.rs`,
`cdzast\x00\x01`) has **no prelude section and no namespace/version fields**: a construct head is just
a `Name` leaf in the leaf pool, referenced by an `Atom` child. The concrete byte layout is not pinned
in any spec at all — the contract defers it to "the declared-default location" (the Rust code). This
is a genuine spec-vs-implementation gap (and exactly the kind of disagreement this vertical exists to
surface). Raised to the concierge 2026-08-27; a spec agent should pin the concrete AST + value-form
byte formats in a contract and reconcile the prelude/namespace/version description. Until then the
codec follows §1's transport nuance.

---

See the merged design for the full picture:
`implementation/design/DESIGN-lean-differential-oracle.md`.
