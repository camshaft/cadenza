# Proposal — should Cadenza build a "Lean backend" for the compiler?

**Author:** design agent (`design-lean-backend`).
**Audience:** the operator (greenlight decision) via the concierge; `v-lean-oracle` (this proposal
touches its territory — coordinated); any future `vertical` if a build is greenlit.
**Status:** PROPOSAL — a decision doc, not a build plan. It weighs the fork the operator posed and makes
a recommendation. No increments are prescribed unless/until greenlit.
**Question (operator, verbatim):** *"investigate if we should build a lean backend for the compiler or
not … see if it's better for lean to prove property about code written in lean itself or is it better
to have lean operate on an AST and prove via interpretation and symbolically."*

## 0. TL;DR (the recommendation)

**Do not build a separate "Lean backend." Keep investing in the Lean-over-AST approach we already have.**

The operator's fork is the classic **shallow vs. deep embedding** choice in mechanized semantics:

- **(1) "prove properties about code written in Lean"** = a *shallow embedding*: translate a Cadenza
  program into native Lean definitions (a `--target lean` backend) and prove over those Lean terms with
  Lean's own tactics. Easy proofs for the fragment that maps cleanly — but only a **total, pure** fragment
  maps, you must **trust the unverified translator**, and it **cannot state the meta-theorems** (type
  soundness, miscompile-freedom) that actually harden a compiler.
- **(2) "Lean operates on an AST, proves via interpretation + symbolically"** = a *deep embedding*:
  represent the program as data (the frozen binary AST) and give it semantics via a Lean interpreter +
  symbolic evaluator. Harder proofs (no free reuse of Lean's tactics) — but faithfully models traps,
  overflow, effects, and divergence, and can state and prove the meta-theorems.

**Every serious verified-compiler effort — CompCert, CakeML, Vellvm — is a deep embedding (option 2).**
And option (2) is **not hypothetical here: it already exists and is maturing** as `v-lean-oracle`
(`implementation/oracle-lean/`) — a concrete reference interpreter over the binary AST, a symbolic-
equivalence arm proving front-end↔`--target cadenza` round-trips equivalent *for all inputs*, and a
machine-checked `denote`-soundness capstone in progress. A "Lean backend" (option 1) would be a **weaker,
riskier, differently-scoped** artifact that duplicates semantics and adds a trusted component, while the
independent deep-embedded oracle is exactly what the operator's own north star ("a full verifiable model
of the language … down to diagnostics and error codes") asks for.

**Recommendation:** keep going with `v-lean-oracle` (option 2). Do **not** open a Lean-backend workstream
now. If *user-facing program verification* ever becomes a product goal (a distinct goal — see §4), build
it **on top of** the oracle's already-verified `denote` semantics, not as an independent trusted backend.

## 1. What "a Lean backend" could mean — disambiguating the ask

"Backend" in rcdzc is a code-emission target (`--target wasm`, `--target rust`, `--target cadenza`). So a
literal "Lean backend" = **`--target lean`: emit each Cadenza program as Lean source**, then reason about
the emitted Lean. That is squarely option (1): the program becomes "code written in Lean," and you prove
properties about it in Lean. This is a **shallow embedding**.

The operator's option (2) is a different mechanism entirely: Lean never *emits* anything into the program.
The program is **data** — the binary AST — and Lean carries an interpreter (and a symbolic evaluator) that
*gives* that data meaning and reasons over it. This is a **deep embedding**, and it is not a "backend" of
the compiler at all; it is an independent model of the language. The two options answer different needs,
which is the crux the rest of this doc pulls apart.

## 2. The technical comparison

### 2.1 Shallow embedding — a `--target lean` backend (option 1)

You write a translator `emit : CadenzaAst → LeanSource` and feed the output to Lean. A Cadenza `fn`
becomes a Lean `def`, `Int64` becomes some Lean integer type, `match` becomes Lean `match`, etc.

**What it buys.** Proofs about a *specific* emitted program can lean directly on Lean's mature tactic
library, `decide`, `simp`, `omega`, typeclass automation, and the whole Mathlib. For the fragment that
maps cleanly, per-program proofs are ergonomic.

**Why it is the wrong tool for a *compiler soundness* story:**

- **Only a total, pure fragment maps.** Lean's logic is a total, pure, terminating type theory. Cadenza
  has behaviors that have **no native Lean counterpart**: integer **overflow that traps** (width-dependent
  — `(+ 100 100)` at `UInt8` must trap, not be `200`), **div-by-zero / OOB / unreachable traps**,
  **algebraic effects + host calls**, **potential non-termination**, and **wasm/backend/portability**
  quirks. To model these you must *encode* them anyway (an error monad, a fuel parameter, an effect
  monad) — at which point you have re-implemented a deep embedding *inside* the shallow one, losing the
  ergonomic win that was the whole point. `v-lean-oracle`'s log already documents exactly these hazards
  (width-aware overflow folding, trap-kind fidelity, effect ordering under laziness).
- **You must trust the translator.** `emit` is an unverified Rust function. A bug in it produces Lean that
  faithfully proves the *wrong* program correct — a **false assurance**, the worst outcome. Verifying
  `emit` itself is a second CompCert-scale project.
- **You cannot state meta-theorems.** Shallow-embedded programs are Lean values, not data, so you cannot
  quantify over "all Cadenza programs," reason about the AST's *syntax*, or state "the compiler preserves
  semantics" / "well-typed programs don't get stuck." Those theorems — the ones that harden the *compiler*
  rather than one user program — require the program to *be data*, i.e. a deep embedding.

### 2.2 Deep embedding — Lean over the AST (option 2, = the existing oracle)

You define `Ast` as a Lean datatype, decode the binary AST into it, and give it semantics with an
interpreter `eval : Ast → Env → Outcome` (`Outcome = value | trap kind | diverges | unsupported`), plus a
symbolic evaluator `symEval : Ast → SymExpr` for ∀-input reasoning.

**What it buys.**

- **Faithful modeling.** Traps are an `Outcome` variant; overflow uses width-checked arithmetic;
  divergence is a fuel budget; effects thread a host-response/host-call state. Nothing is forced into
  Lean's total-pure mold — the semantics say what Cadenza *actually does*.
- **Meta-theorems are statable and provable by induction on the AST.** "For all programs P and all inputs,
  `P` and its optimized round-trip agree" is `v-lean-oracle`'s T2 symbolic-equivalence result. "The
  normalizer preserves meaning" is the `denote (normalize e) = denote e` capstone in progress.
- **Independence = bug-finding power.** A from-scratch reading of the spec shares *zero code* with rcdzc,
  so it catches front-end miscompiles (resolve/typecheck/lower/eval) that rcdzc-vs-rcdzc differentials
  never will. This is the operator's stated motivation for the oracle.
- **Partial coverage ships value immediately** via a first-class `Unsupported`/`cannotProve` verdict:
  coverage grows monotonically and never raises a false alarm.

**The cost, honestly.** Proofs are more work — no free reuse of Lean tactics over native terms; you prove
lemmas about *your* `eval`/`normalize`. But that cost is exactly what buys faithfulness and meta-theorems,
and it is a cost `v-lean-oracle` has already been paying down (totality of `normalize`/`mayTrap`/
`symToValue?`, the structural/congruence equation lemmas, `denote` defined).

### 2.3 Side-by-side

| Dimension | (1) Shallow `--target lean` | (2) Deep embedding (the oracle) |
|---|---|---|
| Program is… | native Lean code (emitted) | data (the binary AST) |
| Models traps / overflow / effects / divergence | only by re-encoding a monad (loses the win) | natively, as `Outcome` + fuel + state |
| Trusted components | the **unverified `emit` translator** | the two frozen byte contracts only |
| Reason about *syntax* / meta-theory | **no** | **yes** |
| Prove ∀-programs (soundness, miscompile-freedom) | **no** | **yes** (T2 symbolic-equiv; `denote` capstone) |
| Per-program proof ergonomics | good (Lean tactics) | more manual |
| Precedent for verified compilers | rare / per-program only | **CompCert, CakeML, Vellvm** |
| Status in Cadenza | not built | **built and maturing** (`v-lean-oracle`) |

## 3. Precedent

- **CompCert** (Coq) — the reference verified C compiler. Every IR (Clight, Cminor, …) is a **deeply
  embedded** AST with an operational semantics; passes are functions with mechanized simulation proofs.
  Deep embedding is the foundation of its end-to-end correctness theorem.
- **CakeML** (HOL4) — a verified ML compiler with a deeply-embedded semantics and a bootstrapped verified
  implementation. Same shape.
- **Vellvm** (Coq) — LLVM IR deeply embedded with a formal operational semantics for reasoning about IR
  transformations. Same shape.
- **Shallow-embedding tools** (e.g. `hs-to-coq`, which translates Haskell to Gallina) — useful for proving
  properties of *specific programs*, and explicitly **limited to a total/pure fragment**; they are *not*
  used to prove a compiler correct, and they inherit the "trust the translator" gap.
- **Lean 4 itself** — Lean's metaprogramming reflects Lean terms, but to reason about an *external*
  language's semantics (traps, effects, a foreign type system) you deeply embed it. There is no shortcut
  that makes a shallow embedding express Cadenza's trap/effect semantics for free.

The consistent lesson: **shallow embedding is for per-program convenience over a total fragment; deep
embedding is for language/compiler meta-theory.** The operator's goal is the latter.

## 4. Relationship to the existing `v-lean-oracle` work

`v-lean-oracle` **is** option (2), already in flight (design: `DESIGN-lean-differential-oracle.md`):

- **Concrete oracle** (`Oracle/Eval.lean`, ~2500 lines): a reference interpreter over the binary AST,
  cross-checked against rcdzc across the whole corpus (`oracle-check`), with a decline verdict for
  unmodeled features. ~1250+ holds / 0 mismatch on the pure core.
- **Symbolic-equivalence arm** (`Oracle/Symbolic.lean`): `symEval` + a sound `normalize` proving a program
  and its `--target cadenza` round-trip equivalent **for all inputs** (operator's T2 primary), degrading
  honestly to `cannotProve` outside the analyzable fragment (never a false "proven").
- **Machine-checked soundness** (`Oracle/SymbolicSound.lean`): ∀-quantified invariants of the normalizer;
  `denote` (the concrete meaning of a symbolic expression) is defined, and the capstone
  `denote (normalize e) = denote e` — *the proof that the oracle never claims a false equivalence* — is
  the next major unit.

Note the important nuance the operator's phrasing surfaces: the oracle **does** "prove properties about
code written in Lean" — but the Lean code it proves about is **the oracle's own `normalize`/`denote`**, to
make its *verdicts trustworthy*. That is proving the *checker* sound, which is the right place for
"proofs about Lean code" — **not** emitting user programs as Lean and proving those. So the two halves of
the operator's fork are already resolved in the codebase in the correct division: reason about the program
as **data** (deep embedding), and reason about the **checker** with Lean-native proofs.

**Two distinct goals — do not conflate them:**

- **Goal A — compiler soundness / bug-finding** ("is rcdzc correct?"). Served by option (2), already
  underway. A "Lean backend" adds nothing here except a trusted translator and a coverage ceiling.
- **Goal B — user-facing program verification** ("can a Cadenza *user* prove *their* program correct?").
  This is the *only* place a `--target lean` emission has a plausible role — and it is a **product feature,
  not a compiler-soundness feature**, currently unrequested. **If** it is ever wanted, the sound
  construction is to emit Lean *and prove the emission agrees with the oracle's `denote` semantics* (so
  the translator is verified against the model we already trust), rather than standing up an independent
  trusted backend. That layering is only sensible *after* the `denote` capstone lands.

## 5. Recommendation & next step

1. **Do not open a "Lean backend" (`--target lean`) workstream now.** It is the wrong tool for compiler
   soundness (option 2 dominates on every axis that matters there) and premature for user verification
   (no product ask; and if it comes, it should layer on `denote`).
2. **Keep investing in `v-lean-oracle` (option 2).** It is the operator's north star, follows verified-
   compiler precedent, and is already delivering. Priority remains the `denote (normalize e) = denote e`
   soundness capstone and growing corpus/symbolic coverage.
3. **Revisit option (1) only if Goal B (user program verification) becomes an explicit product goal** — and
   then as a *verified-against-`denote`* emission, not an independent trusted backend.

**This proposal prescribes no build.** The deliverable is this decision doc plus a TL;DR routed to the
operator (via the concierge) for a greenlight on the recommendation. If the operator instead wants to
pursue a Lean backend (e.g. prioritizing Goal B now), that is a new design and a new workstream, and this
doc's §2/§4 are the constraints it must respect (total-pure fragment limits, the translator-trust gap, and
the layer-on-`denote` construction).

## 6. Open questions for the operator (each with the doc's default)

- **OQ-1 — Is user-facing program verification (Goal B) a goal at all?** *Default:* no / not now → keep to
  option (2). A "yes" reopens option (1) as a *layered-on-`denote`* feature (§4), not an independent backend.
- **OQ-2 — Any appetite to accelerate the `denote` soundness capstone?** *Default:* proceed at
  `v-lean-oracle`'s current cadence; it is a focused multi-tick effort, not parallelizable into slivers.
</content>
</invoke>
