# DESIGN: Program verification — pre/post-conditions on Cadenza programs, with proofs that feed the optimizer

Status: **Increment (b) DESIGN — proposed, forks open to the operator.** Follows the LCF kernel
([DESIGN-verification-hol-kernel.md](DESIGN-verification-hol-kernel.md), CHARTER DELIVERED: a working,
unforgeable `Thm` on trunk). Vertical `v-verification`, subsystem `rcdzc`. Operator greenlight
(2026-07-16, via concierge, verbatim intent): *"Adding pre/post-conditions would be amazing … keep in
mind those conditions for optimization purposes — like if we can prove that an integer never overflows
then we should elide the checks entirely."*

This doc scopes the next major direction: from *proving standalone theorems* (Inc-a, done) to *stating
and discharging conditions about Cadenza programs* (Inc-b) — and, crucially, to making a discharged
condition a **first-class input the optimizer can consume** to remove now-redundant runtime guards. It
answers the four design questions the concierge flagged and routes each fork to the operator. No code
lands from this doc; it commits to a shape and an increment plan, design-first, exactly as Inc-a did.

---

## 0. Executive summary — what changes, and why the kernel makes it cheap

The Inc-a kernel proves theorems in a HOL logic: `⊢ p`. Program verification adds three things ON TOP of
that kernel, none of which weakens its trust story:

1. **An annotation surface** — a way to *state* a pre/post-condition in Cadenza source (`@requires` /
   `@ensures`-style, fork #2). This is untrusted: an annotation is a claim, not a proof.
2. **A semantics→logic bridge** — a *denotation* that turns a Cadenza function-plus-its-conditions into a
   HOL proof obligation (a `Term` the kernel can be asked to prove). This is the load-bearing design
   piece (fork #1).
3. **A proof→optimizer interface** — once the obligation is discharged (the kernel returns a `Thm` whose
   conclusion IS the obligation), that discharge becomes a fact the optimizer can query: *"is
   `no-overflow@node-N` discharged? then emit the unchecked add."* This is the novel bit the operator
   explicitly wants (fork #3); it's a v-verification × v-core-opt seam.

**Why the kernel makes this cheap and SOUND.** The LCF payoff is that *checking* a proof is trivial and
trusted while *finding* it is untrusted (Inc-a §6, the same answer given to v-agent-harness). Program
verification is that principle applied to the compiler: the optimizer's trusted question is a single
kernel-typed comparison — "does this `Thm`'s conclusion match the obligation this guard discharges?" — and
everything that PRODUCED the `Thm` (annotation elaboration, VC generation, tactic search) is untrusted and
cannot cause an unsound elision, because an elision only fires on a real kernel `Thm`. A miscompiled
optimizer that elides a check without a matching `Thm` is a bug the design structurally prevents: **no
`Thm`, no elision.** This is why the operator's "proofs feed optimization" directive is safe to build on
the kernel rather than on ad-hoc dataflow — the trust boundary is the same unforgeable `Thm`.

**Design-first, and REPORT/FIX.** Like Inc-a, this will surface language gaps (the annotation surface
almost certainly needs a real compiler feature; the denotation will stress the metaprogramming `Ast`
bridge). Those get reported and fixed, not worked around. The increment plan (§5) front-loads a
paper denotation + a hand-written obligation corpus BEFORE any compiler change, so the design is validated
against the real kernel before we ask for surface syntax.

---

## 1. The semantics→logic bridge — how a Cadenza program denotes into HOL (fork #1)

This is the big piece. A pre/post-condition is a claim about a *function's behavior*; the kernel proves
claims in *HOL*. Something must connect "the Cadenza function `f`" to "a HOL term whose truth is exactly
`f` satisfies its condition." Two candidate shapes:

### 1A. A shallow embedding (a denotation) — RECOMMENDED to start
Each Cadenza expression *denotes* a HOL term directly: an `Int64` is a HOL individual, `+` denotes HOL
addition (with the overflow side-condition explicit), `if` denotes a HOL conditional, a `match` denotes a
case split, a `let` a substitution. A function `(def (f x) body)` with `@requires P` / `@ensures Q`
denotes the obligation `⊢ ∀x. P(x) ⇒ Q(x, denote(body))`. The compiler generates this `Term`; the kernel
(plus tactics) discharges it.
- **Pro:** reuses the HOL logic the kernel already has verbatim; obligations are ordinary `Term`s; the
  discharge is an ordinary `Thm`. No new trusted code.
- **Con:** need a denotation clause per language construct; effects, heap, and non-termination need care
  (start with the pure total fragment — arithmetic, `if`/`match`/`let`, first-order — which is exactly the
  fragment where the overflow-elision payoff lives).

### 1B. A deep embedding / program logic (Hoare triples as HOL objects)
Reify Cadenza's operational semantics IN HOL (a `Stmt`/`Expr` datatype, an evaluation relation), and prove
Hoare triples `{P} c {Q}` as theorems about that reified syntax.
- **Pro:** models effects/heap/loops faithfully; the semantics is a first-class object you can prove
  meta-theorems about.
- **Con:** far larger; you must first prove the operational semantics sound, and every program construct
  needs a rule. Overkill for the overflow-elision win.

**Recommendation (fork #1): start 1A (shallow) on the pure total arithmetic fragment.** It's the minimum
that delivers the operator's headline example (proven no-overflow → elide the check) and it reuses the
kernel with zero new trusted surface. 1B is a later increment IF effect-ful/heap conditions are wanted.
The denotation is the design artifact of the FIRST code increment (b1, §5) — written on paper, validated
against a hand-authored obligation corpus, before any surface syntax.

**The connective already exists in the codebase.** The metaprogramming `Ast` sum (Inc-a references it;
`v-metaprogramming` SHIPPABLE-DONE) is a reflected Cadenza program. The denotation is a total function
`Ast → Term` — i.e. it lives in exactly the reflective bridge the language already has. This is the
natural home and a real stress test of that `Ast` (expect gaps → REPORT/FIX).

---

## 2. The annotation surface — what a condition looks like in source (fork #2, route to operator)

This is product taste, so it routes to the operator. Options, cheapest first:

- **2A. A `verify`/`theorem` block in the corpus (NO surface change).** Conditions are written as kernel
  obligations directly, the way Inc-a cases already are. Zero compiler work; validates the whole pipeline
  (denotation + discharge + elision-query) before asking for syntax. **This is where b1–b3 live regardless
  of the eventual surface** — it de-risks the design.
- **2B. `@requires` / `@ensures` annotations** on a `def`, checked at compile time (the operator's own
  phrasing). Ergonomic, familiar (Dafny/SPARK/JML). Needs: a parser/annotation feature (likely a
  `v-syntax` + `v-metaprogramming` seam — annotations are already a language concept, cf. `@property`,
  `@tag`, `@cite`), an elaboration that runs the denotation, and a diagnostic when the obligation can't be
  discharged ("CDZ-VERIFY: cannot prove `@ensures` … ").
- **2C. A refinement-type surface** (`Int64 where (> it 0)`). Most powerful, most invasive; defer.

**Recommendation (fork #2): build the pipeline under 2A first (no surface), then adopt 2B as the shipping
surface once b1–b3 prove the pipeline.** Route the *exact* `@requires`/`@ensures` spelling to the operator
— it's syntax taste, and it touches `v-syntax`'s territory. I'll draft a strawman for the operator to
react to rather than block.

---

## 3. The proof→optimization interface — how a discharge reaches the optimizer (fork #3, the novel bit)

The operator's headline: *proven no-overflow → elide the check entirely.* Design so a discharged
obligation is a fact the optimizer QUERIES, keyed to the IR node whose guard it licenses.

### The seam
- The compiler emits checked arithmetic (an overflow trap) at a Core/MIR node. Call the guard it emits
  `overflow-check@N` for node `N`.
- Verification produces, for some nodes, a `Thm` whose conclusion is exactly the obligation that guard
  exists to enforce — e.g. `⊢ ∀ inputs. P(inputs) ⇒ (a + b does not overflow Int64)` at node `N`.
- The optimizer, at the point it would emit `overflow-check@N`, asks a **proof oracle**: *is there a
  discharged `Thm` whose conclusion matches `no-overflow@N` under this node's context?* If yes → emit the
  unchecked op; if no → emit the check as today. **Default is always the check** — elision is opt-in on a
  present proof, never on absence of a disproof.

### Shape of the interface (design options)
- **3A. Proof-carrying IR annotation.** Verification attaches the discharged obligation (or a stable key
  into a proof table) to the Core/MIR node as an attribute. The optimizer reads the attribute — a local,
  O(1) query, no cross-module lookup at opt time. RECOMMENDED: it keeps the optimizer's trusted check
  tiny (match a conclusion) and localizes the seam.
- **3B. A side table the optimizer queries by node identity.** A `Map<NodeId, Thm>` the verification pass
  populates and the opt pass reads. Equivalent power; slightly looser coupling.
- **3C. The optimizer calls the kernel on demand.** The opt pass itself asks "prove `no-overflow@N`?" —
  rejected: it puts proof search on the compile-time hot path and inverts the trust story (search should
  be untrusted and precomputed).

**Recommendation (fork #3): 3A — proof-carrying annotation, with the optimizer's trusted action being a
single conclusion-match against a real `Thm`.** This is the design that makes an unsound elision
*impossible by construction*: the opt pass cannot elide without a `Thm` in hand, and it cannot forge one
(Inc-a unforgeability). The elision target is the overflow-trap emit (the codebase already has the guard;
`opt.rs` is the Core-IR opt framework where the query lands). **Coordinate with v-core-opt** — the
concierge has told them to expect proof-guided elision; I'll send them this design and co-own the seam
(they own `opt.rs`; I own the `Thm`-shaped obligation + the match predicate).

### What the optimizer must trust (and what it must NOT)
- **Trusted:** the kernel `Thm` type (unforgeable), and the *predicate* "this `Thm`'s conclusion is the
  obligation that `overflow-check@N` discharges." That predicate must itself be audited — a sloppy match
  (e.g. ignoring the node's precondition context, or matching a `Thm` proven under different bindings)
  would license an unsound elision. **This match predicate is the new trusted surface of Inc-b** and gets
  the same adversarial pinning the kernel boundary got (breaker: "supply a `Thm` that looks like it
  licenses `@N` but was proven under different assumptions — does the elision wrongly fire?").
- **Untrusted:** annotation elaboration, denotation, VC generation, tactic search. All can be buggy
  without unsoundness — a bug there yields "no `Thm`" or "wrong `Thm`," and either way the match fails and
  the check stays.

---

## 4. Own vertical, or under v-verification? (fork #4, route to operator)

Program verification + the opt seam is a large new surface (a denotation, an annotation feature, the
optimizer interface, a VC/tactic layer). But it is *continuous* with the kernel I built — it's the kernel's
first real consumer, and the trust argument is the same unforgeable-`Thm` argument.

**Recommendation (fork #4): keep it under v-verification through the design + b1–b3 (paper denotation,
obligation corpus, opt-seam prototype), and revisit spinning a co-scoped vertical when the annotation
SURFACE (2B) lands — that's when it grows a `v-syntax`/`v-core-opt` seam wide enough to want its own owner.**
Splitting now would fragment the trust story mid-design. Route the timing to the operator.

---

## 5. Increment plan (each code increment gated: corpus + `cargo test -p rcdzc` + `cargo xtask check`)

Front-load the design validation BEFORE any compiler change, exactly as Inc-a did:

- **b0 — THIS DOC.** Scope + forks. No code. ✅ (routing forks 1–4 to operator now.)
- **b1 — paper denotation + hand-authored obligation corpus (NO compiler change).** Write the shallow
  denotation (§1A) for the pure arithmetic fragment on paper; encode 6–10 obligations as kernel `verify`
  cases (§2A) in a NEW `spec/semantics/26-program-conditions.sexp`: e.g. "for `0 ≤ x ≤ 100`, `x + 1` does
  not overflow Int64" discharged by the kernel; and the dual "for unconstrained `x`, the no-overflow
  obligation is NOT provable" (the check must stay). Ships with its `.gate-baseline`. This validates the
  denotation + discharge end-to-end with zero risk.
- **b2 — the match predicate + a discharge→elision PROTOTYPE (behind a flag, corpus-only).** Implement the
  trusted "does this `Thm` license `no-overflow@N`" predicate; prototype the optimizer querying it on a
  toy node. Adversarially pinned (breaker) BEFORE it can gate anything real. Coordinate v-core-opt.
- **b3 — proof-guided elision on the real overflow guard (opt-in, proven cases only).** Wire b2 into
  `opt.rs`'s overflow-check emit: a node with a matching discharged `Thm` emits unchecked; all others
  unchanged. Differential-gate BOTH backends (wasm + rust) — a proven-safe add must compute the same value
  unchecked as checked, and an UNproven add must still trap. This is the operator's headline deliverable.
- **b4+ — the annotation SURFACE (2B, `@requires`/`@ensures`), pending operator syntax ruling; then wider
  obligations (bounds checks, exhaustiveness), then effect-ful conditions (1B) if wanted.**

Each increment is one gated unit; b1 is the next tick's work if the operator doesn't redirect the forks.

---

## 6. Forks — routed to the operator (2026-07-16); I proceed on the recommended defaults meanwhile

Per the contract (never wait on a human), I proceed on b1 under the recommended defaults and adjust if the
operator redirects:
1. **Semantics→logic bridge: shallow denotation (1A) on the pure arithmetic fragment first.** (Craft call;
   proceeding.)
2. **Annotation surface: pipeline under `verify`-blocks (2A) first; `@requires`/`@ensures` (2B) as the
   shipping surface.** ⟵ EXACT SYNTAX is product taste — routed to operator; I'll draft a strawman.
3. **Proof→opt interface: proof-carrying annotation (3A), optimizer's trusted action = a single
   conclusion-match against a real `Thm`.** ⟵ novel bit — routed to operator for confirmation; co-owned
   with v-core-opt.
4. **Ownership: stays under v-verification through b1–b3; revisit a co-scoped vertical at the 2B surface.**
   ⟵ routed to operator for timing.

## 7. Coordination

- **v-core-opt** — the proof-guided-elision seam (§3, fork #3). The concierge has told them to expect it.
  Send this design; co-own: they own `opt.rs` + the overflow-check emit; I own the `Thm`-shaped obligation
  + the trusted match predicate. Agree the node-keying (3A) and the "default is always the check" invariant.
- **v-metaprogramming** — the denotation `Ast → Term` (§1) lives on their reflected `Ast`. Expect gaps →
  REPORT/FIX. Confirm the `Ast` exposes every construct the arithmetic fragment needs.
- **v-syntax** — the annotation surface (2B) is their territory (annotations are a language concept, cf.
  `@property`/`@tag`/`@cite`). No action until the operator rules on fork #2.
- **breaker** — from b2 on, adversarial cases against the NEW trusted surface: the match predicate. "Supply
  a `Thm` proven under different assumptions that superficially matches `@N` — does the elision wrongly
  fire?" This is Inc-b's soundness boundary, the analogue of the §3 forge vectors.
- **v-agent-harness** — a downstream consumer (Inc-a §7): a self-modification carrying a `Thm` that it
  preserves an invariant is exactly a program-condition discharge. The b-track machinery is what states
  those conditions. Revisit when they reach their Inc-3.

---

## References
- [DESIGN-verification-hol-kernel.md](DESIGN-verification-hol-kernel.md) — the kernel this builds on
  (unforgeable `Thm`, the trust boundary, the LCF check-is-trivial/find-is-untrusted payoff).
- `spec/semantics/25-verification.sexp` — the kernel corpus (my 54 increment cases); Inc-b adds a new
  `26-program-conditions.sexp`.
- `implementation/seed/crates/rcdzc/src/opt.rs` — the Core-IR optimization framework (v-core-opt) where the
  proof-guided elision query lands; the overflow-trap emit is the first elision target.
- Cadenza `Ast` sum (metaprogramming, SHIPPABLE-DONE) — the reflected program the denotation walks.
- Prior art: Dafny / SPARK / Why3 (annotation surfaces + VC generation); F* / Liquid Haskell (refinement
  types, fork #2's 2C); CompCert / seL4 (proof-carrying compilation — the trusted-check-is-tiny discipline
  this §3 mirrors).
