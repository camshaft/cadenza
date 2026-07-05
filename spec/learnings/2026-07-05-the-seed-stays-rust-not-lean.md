# The seed stays Rust, not Lean: the seed's implementation language is orthogonal to Cadenza's verification aims

*2026-07-05*

**What happened.** Because Cadenza aims to be a machine-checked language, we weighed writing the seed
compiler (`cdz-rustc`) in **Lean 4** instead of **Rust** — Lean being a prover with a self-hosted
compiler, so the seed's own lowering could in principle be formally verified. We kept Rust. Three
facts decided it:

- **The seed is disposable and off the critical path.** The seed compiler is a foreign-language
  artifact whose only jobs are to lower Cadenza source to a component and to compile the first
  Cadenza-authored compiler; it is gitignored, and once `compiler.cdz` self-hosts the seed leaves the
  loop entirely (self-hosting-and-bootstrap.md §"The Seed Compiler Is The One Step Outside The Loop").
  Cadenza's own verification aims are realized in **Cadenza's** design (the verification layers), not
  inherited from whatever language the seed is written in — so Lean would only let us verify a
  throwaway.

- **Machine-checking in Lean is opt-in and expensive to cash in.** Unverified Lean is an ordinary
  functional language (with a termination checker one escapes via `partial def`); the distinguishing
  value — proving lowering preserves semantics or the type-checker is sound — requires explicit proofs
  at the CompCert/CakeML cost class. Paying that on a disposable artifact buys nothing the architecture
  needs, because independence of the behavioral judgment already comes from **two compilers agreeing
  against the corpus** (constitution XIV), not from a single trusted verified compiler.

- **Rust's ecosystem is this problem's ecosystem; Lean's is not.** The seed already relies on mature
  wasm-component emission, an embeddable component-model runtime, and canonical byte encoding; Lean has
  no first-class wasm/WASI target and no comparable component/bytes/runtime libraries. The
  `component-check` (the seed built to wasm, byte-identical to native) — the one thing that would break
  without a good wasm story — is exactly where Lean is weakest.

**Why.** The architecture deliberately does **not** rest on a trusted verified compiler at its root: the
corpus is the oracle and independence is two implementations agreeing, so the seed only has to *work*
and *cross-check*, not be proven. That makes the seed's implementation language a replaceable
engineering choice governed by ecosystem fit, not a load-bearing part of the trust story — and Rust
wins on fit. Lean's value (formal proof) is real but belongs to Cadenza's own verification layers and,
optionally, to a future *third* independent oracle or a formal semantics model — additive, not the
seed.

**The requirement it drove.** No normative requirement changed: this confirms the existing default at
`options/bootstrap-strategy/rust-seed-interpreted-first.md` (Seed host language: Rust) against a
considered alternative, and records the rationale so the choice is not re-litigated. A Lean realization
remains admissible only as an *optional* independent oracle or semantics model, never as the seed on the
bootstrap path, consistent with self-hosting-and-bootstrap.md §"A Reference Interpreter Is An Optional
Independent Oracle" (the same "extra cross-check, not the root" role).
