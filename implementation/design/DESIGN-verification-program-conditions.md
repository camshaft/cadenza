# DESIGN: Program verification — pre/post-conditions on Cadenza programs, with proofs that feed the optimizer

Status: **Increment (b) DESIGN — ALL FORKS RESOLVED (operator 2026-07-16): `@requires`/`@ensures` GREENLIT;
`@requires`/`@ensures` parse SETTLED with v-syntax; opt-seam SETTLED as a FOUR-WAY division (v-core-opt +
v-wasm-opt + v-rust-backend + me — §7), opt-seam simplified to a Core-tier DISJUNCTION
(no new node; both backends already consult `arith_provably_in_range`, §3). b0 doc + b1 obligation corpus +
b2 (Cadenza match predicate + `CorePass` mechanism) DONE; b3 = land `discharged_no_overflow` (stub→body) that
v-core-opt's Slice-5 wrapper ORs into the predicate. NEXT: b3.** Follows the LCF kernel
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

## 2. The annotation surface — `@requires`/`@ensures` (fork #2, GREENLIT + parse SETTLED)

**Operator ruling (2026-07-16): `@requires`/`@ensures`** — *"I like requires/ensures. That's very
consistent with the other languages."* Sequencing per my recommendation: build the pipeline under
verify-blocks first (no surface change while the semantics bridge firms up), then surface as
`@requires`/`@ensures` on a `def`. Options considered:

- **2A. A `verify` block in the corpus (NO surface change) — the b1–b3 vehicle.** Conditions are written as
  kernel obligations directly, the way Inc-a cases already are. Zero compiler work; validates the whole
  pipeline (denotation + discharge + elision) before any syntax. **b1–b3 live here regardless of the
  eventual surface** — it de-risks the design.
- **2B. `@requires` / `@ensures` annotations on a `def` — the SHIPPING surface (b4).** Ergonomic, familiar
  (Eiffel/Dafny/SPARK/JML), the operator's stated consistency bar. **Parse SETTLED with v-syntax
  (2026-07-16):** the surface fits the EXISTING glued-annotation form with NO new parser primitive —
  `@requires(pred) @ensures(pred) def f x = body` parses today to
  `(@ (requires <pred>) (@ (ensures <pred>) (def (f x) body)))` via the existing `@` head rule;
  `@requires`/`@ensures` are ordinary annotation NAMES (like `@param`/`@cite`/`@test`), needing no
  reservation. What's mine: the elaboration that runs the denotation and the diagnostic when an obligation
  can't be discharged ("CDZ-VERIFY: cannot prove `@ensures` …").
  - **Result binding = `it`** (v-syntax's call, on consistency): `@ensures(> it 0)`. `it` is NOT a new
    parsed form — it is a plain name the front-end blesses nothing extra for; MY elaboration binds it to
    the function's result. If the refinement-type `where`-sugar (`Int64 where (> it 0)`, 2C) later lands,
    `it` is the same implicit-subject binder — one convention across refinements and post-conditions.
  - **Pre-state (`old()`): deferred to the effect-ful increment (1B), agreed with v-syntax.** The pure
    fragment's args are immutable, so `@ensures` names params directly — no `old()` needed. A pre-state
    operator gets co-designed when 1B needs it.
- **2C. A refinement-type surface** (`Int64 where (> it 0)`). Most powerful, most invasive; defer — but
  note it shares the `it` binder with 2B, so the two stay consistent when it lands.

### 2.1 The b4 elaboration algorithm — from `@requires`/`@ensures` to a kernel obligation
The surface parses to `(@ (requires P) (@ (ensures Q) (def (f x…) body)))`. Elaboration (untrusted — a bug
here yields "no proof / wrong proof", never an unsound elision) turns that into obligation `Term`s the
kernel discharges, reusing the b1 denotation (`Ast → Term`, §1A). For a pure-arithmetic `f`:

1. **Denote the annotation predicates.** `denote(P)` and `denote(Q)` map the Cadenza predicate `Ast` to a
   HOL `Term` — the SAME `Ast → Term` the b1/b2 obligations use (`>`/`<=`/`+` → `le`/`add`/… head-symbols,
   params → `Var`, literals → `Num`). `it` in `Q` denotes the function-RESULT term `denote(body)`; a param
   `x` denotes `Var x` in both `P` and `Q`.
2. **Form the two obligations.**
   - `@requires P` is a PRECONDITION the caller must establish — at each call site, `denote(P[args])` is an
     obligation the caller discharges (or an assumption the callee's body may use). In the callee, `P`
     enters as a HYPOTHESIS via `assume (denote P)` — exactly the b1 `assume (le x 100)` step.
   - `@ensures Q` is the POSTCONDITION obligation on the body: `⊢ (denote P) ⇒ (denote Q[it := denote body])`
     — discharged in the `hol`/`bounds` kernel from the `P` hypothesis, closing to `Q`. This is the b1
     chain generalized: the specific `no-overflow@Id` obligation b1–b3 use is the case where `Q` is the
     implicit `add-in-range` side-condition of a checked `+` in `body`.
3. **The overflow side-condition is an IMPLICIT `@ensures`.** Every checked `x + k` in `body` carries an
   implicit obligation `LE (add x k) MAXINT` at its node `Id`. b3 already discharges + consumes THIS one
   (the operator's headline). An explicit `@ensures` is the same machinery with an author-written `Q`
   instead of the compiler-synthesized range side-condition — so b4 REUSES b1–b3 wholesale; it adds the
   *surface* (`@ensures(Q)` → `denote(Q)`) and the *diagnostic* ("CDZ-VERIFY: cannot prove `@ensures` …"
   when the kernel returns no `Thm`), not new kernel or oracle machinery.
4. **Discharge + report — a THREE-TIER outcome (v-property-testing seam, 2026-07-17).** Run the obligation
   through the kernel (compile-time eval, as b2/b3). The SAME denoted `Q` drives all three outcomes — one
   postcondition, no author restatement:
   - **PROVEN** — the kernel discharges a `Thm` whose conclusion matches the obligation → the annotation
     holds (and, for the implicit overflow one, feeds the b3 elision oracle).
   - **TESTED** — if `Q` is not statically provable, auto-synthesize a PROPERTY TEST from it.
     **IMPLEMENTED (v-property-testing, MR `9f1b981b1`):** gated on the interim `@test @ensures` STACK
     (a bare `@ensures` is untouched, so the PROVEN path is unaffected), `proptest_gen` rewrites
     `(@ test (@ (ensures Q) (def SIG BODY)))` → the body becomes `(let ((it BODY)) (if Q unit (trap)))`, so
     `Q` runs as the oracle over F1-generated inputs (scalar/tuple/record/set/map/leaf) and a false `Q`
     shrinks to a minimal counterexample. The `it := BODY` binding matches this doc's `denote(Q[it:=body])`,
     so ONE postcondition serves both consumers (my kernel denotes it to a `Term`; their harness lowers it as
     code) — no author restatement.
   - **CDZ-VERIFY** — only if neither proven nor testable (a param type the generators don't cover), or the
     mode an author opts into for "must be statically proven".
   Seam ownership: **the interim gate is the `@test @ensures` stack** (no pragma, no spec change — fully
   v-property-testing's lane; they own TESTED, I own PROVEN). The `ensures-mode` pragma that drives the
   automatic proven→tested→CDZ-VERIFY SEQUENCING is a spec module-directive (`PRAGMA_REGISTRY` +
   modules-and-namespaces.md §Fixed Set + validator) that **I coordinate at b4c** when the sequencing lands.

**Why this is a small b4, not a new increment.** b1 built the denotation + discharge; b2 the match predicate;
b3 the oracle→optimizer wiring. b4 is: (a) the `@ensures`/`@requires` elaboration that emits `denote(Q)` /
`assume(denote P)` — a front-end pass over the already-settled parse; (b) the CDZ-VERIFY diagnostic. The
kernel, the denotation, the match predicate, and the elision seam are all already built. b4 corpus pins:
a `def` with a provable `@ensures` type-checks + discharges; an unprovable-but-testable `@ensures` yields a
synthesized property test (TESTED tier); a truly-uncheckable one gets CDZ-VERIFY; a `@requires` bound flows
into the body's overflow discharge (linking the surface to the b3 elision — a proven `@requires x<=100`
elides an `x+1` guard in the body).

**b3/b4 ORDER (corrected 2026-07-17, agreed with v-core-opt).** b4 must PRECEDE b3: `discharged_no_overflow`
(b3's body) has nothing to compile-time-eval until a Core node CARRIES a discharge program, and that
per-node attachment IS the b4 elaboration. There is no `@requires`/`@ensures`→Core-node channel today
(`arith_provably_in_range` reads only `value_range`, pure interval analysis). So the built order is: Slice-5
wrapper (DONE, stub=`false`, behavior-neutral) → **b4 elaboration (attaches the obligation+precondition to
the arith node's `StructId`)** → **b3 fills `discharged_no_overflow` to eval that per-node obligation
(fail-closed on trap)**. v-core-opt's seam is order-independent; the stub correctly stays `false` until b4.

---

## 3. The proof→optimization interface — how a discharge reaches the optimizer (fork #3, the novel bit)

The operator's headline: *proven no-overflow → elide the check entirely.* Design so a discharged
obligation is a fact the optimizer QUERIES, keyed to the node whose guard it licenses.

**Keying + tier (SETTLED with v-core-opt, 2026-07-16).** Key the obligation by the **stable `StructId`**
(the same `Id` space `lower::core_of` and the poison/escape VISITED walks use), NOT a MIR/Lir node, and run
the elision as a **`CorePass` at the CORE tier**. Two reasons, both from v-core-opt: (1) eliding high in the
pipeline means BOTH backends (wasm + rust) inherit ONE elision, rather than each backend re-eliding at its
own emit — this also matches the operator's explicit "higher is better" optimization steer; (2) a MIR/Lir-
keyed annotation would be backend-specific and would not survive node regeneration. So the obligation is
`no-overflow@<StructId>` and the elision is a Core pass that, for each checked-arith Core node, queries the
proof oracle by that `Id` and, on a discharge, calls `db.install_core_override(N, <unchecked Core>)` (the
override map is EMPTY in the default pipeline → byte-identical no-proof emit; no-proof is literally the
no-override path).

**✅ b3 MECHANISM SIMPLIFIED — a Core-tier DISJUNCTION, no new node (v-core-opt SEQUENCING CORRECTION,
2026-07-17).** An earlier draft assumed the elision needed a new Core-level unchecked `Arith`
representation (v-core-opt's "Slice-2a"). That is **SUPERSEDED and cancelled** — `Core::Arith` stays
`{op,lhs,rhs}`. The reason: the overflow-guard elision MECHANISM already exists at the Core tier —
`lower::arith_provably_in_range` (`lower.rs`, i128 interval arithmetic, sound-by-endpoint): a checked add
emits its guard UNLESS that predicate returns true. So eliding = making the predicate return true for a
node; no new node/flag is needed. And **both backends already consult the predicate** — wasm (`select.rs`)
always did, and rust's `emit_arith` now does too (v-core-opt Slice-2 `ca0f82a6b`, landed; emits
`wrapping_add` when true). **This RETIRES the earlier "wasm-only / rust needs a symmetric consult" caveat.**
So b3 is a **disjunction**:

> `provably_no_overflow(db, op, lhs, rhs, ty, id) = arith_provably_in_range(db, op, lhs, rhs, ty)  OR  discharged_no_overflow(db, id)`

v-core-opt owns a thin Core wrapper (their Slice-5) that ORs my oracle into the existing predicate; both
backends inherit it with ZERO emit change (they already gate on the predicate). I own `discharged_no_overflow`
— which lands first as a `false`-returning STUB (disjunction identical to today → behavior-neutral, green),
then b3 fills its body (compile-time-eval the discharge program, return the `licenses` boolean).
Unforgeability still gates it: no licensing `Thm` → the oracle returns `false` → the guard stays.

### The seam
- The compiler emits checked arithmetic (an overflow trap) at a Core node with stable `Id`, UNLESS
  `arith_provably_in_range` already proves it safe. Call the guard `overflow-check@Id`.
- Verification produces, for some `Id`s, a `Thm` whose conclusion is exactly the obligation that guard
  exists to enforce — e.g. `⊢ ∀ inputs. P(inputs) ⇒ (a + b does not overflow Int64)` at `Id`.
- `discharged_no_overflow(db, id)` returns true iff such a licensing `Thm` exists (compile-time-eval of the
  Cadenza `licenses` predicate). The Core wrapper ORs it into `arith_provably_in_range`, so a licensed node
  sheds its guard on BOTH backends; an unlicensed node keeps it. **Default is always the check** — elision
  is opt-in on a present proof, never on absence of a disproof.

### Shape of the interface — 3A, refined with v-core-opt's mechanism
- **3A. Proof-carrying annotation as a PURE ORACLE QUERY (SETTLED).** The discharge does NOT thread through
  passes as mutable state. Instead the elision `CorePass` calls the oracle (`Id → Option<Thm-handle>`, a
  pure function that leaks no kernel internals) and, on `Some`, installs an unchecked-op **override** via
  v-core-opt's core-override layer. This cleanly separates the two trusted/untrusted concerns: **I own the
  oracle + the match predicate** (does this `Thm` license `no-overflow@Id`); **v-core-opt owns the override
  mechanism** (`core_of` consults an override table a `PassManager` populates — their "slice 1", landing
  first). The obligation is stated in the corpus keyed by a placeholder Core-`Id` so it already speaks the
  `Id` language before the mechanism exists.
- **3B. A side table the opt pass reads** (`Map<Id, Thm>`) — subsumed by 3A's oracle query; equivalent
  power, kept as a note.
- **3C. The optimizer calls the kernel on demand** — rejected: puts proof search on the compile-time hot
  path and inverts the trust story (search is untrusted and precomputed; the oracle only looks up a
  already-discharged result).

**Recommendation (fork #3): 3A as refined — a pure oracle query keyed by stable `StructId`, the elision a
`CorePass`, the optimizer's trusted action a single conclusion-match against a real `Thm`.** This makes an
unsound elision *impossible by construction*: the pass cannot install an unchecked override without a `Thm`
in hand, and it cannot forge one (Inc-a unforgeability). The elision target is the overflow-trap emit; the
mechanism is v-core-opt's core-override layer. **Seam SETTLED with v-core-opt** (their
`DESIGN-tiered-optimization-levels-rcdzc.md` §9.2 carries the identical default=check invariant); we sync
at my b2 (match predicate + toy prototype) ↔ their slice-2 (real passes), by which point their slice-1
override seam exists so the prototype is a real `CorePass`.

### How the discharge reaches the EXISTING elision predicate (v-wasm-opt, 2026-07-16 — with a rust caveat)
The overflow guard is already elided where a range analysis proves safety: `lower::arith_provably_in_range`
(defined at the Core tier). The oracle does NOT introduce a new elision node — it feeds that predicate as a
**disjunction**:

> `provably_no_overflow(id) = arith_provably_in_range(op, lhs, rhs, ty)  OR  oracle(id).is_some()`

so range analysis and proof COMPOSE (either suffices), and the discharged-`Thm` case reuses the tested,
corpus-armored elision path. The disjunction only ever ADDS elisions (a proof licenses what range analysis
could not); it never removes a check range analysis would keep — the default-is-check invariant is
preserved. The one new trust obligation is that `oracle(id) = Some` implies the same range-safety
`arith_provably_in_range` would certify — which is exactly what the match predicate checks.

**⚠️ RUST CAVEAT (v-wasm-opt correction, 2026-07-16): the elision is WASM-ONLY today.**
`arith_provably_in_range` is Core-defined, but only the **wasm** backend consults it (`select.rs`); the
**rust** backend's `emit_arith` (`backend/rust/expr.rs`) emits `checked_add(…).unwrap_or_else(panic)`
UNCONDITIONALLY — no elision path. So a discharged proof will not elide on rust until **v-rust-backend adds
the symmetric consult** (a 4th touch on the seam, flagged to v-core-opt). Two consequences for my plan:
(1) the fork-#3 mechanism is unchanged, but the seam is now **four-way** (v-core-opt: Core unchecked node +
override pass; v-wasm-opt: wasm emit predicate; v-rust-backend: rust emit consult; me: oracle + match
predicate); (2) **my b2/b3 differential gate MUST include a rust case**, not only wasm — otherwise a
proof-driven elision passes the gate while silently doing nothing (or diverging) on rust. v-wasm-opt will
co-verify both-backend byte-identity once my oracle (b2) + v-core-opt's pass exist.

### How a Cadenza `Thm` reaches the Rust optimizer — COMPILE-TIME EVAL (b2 architecture, decided 2026-07-16)
A real question b2 surfaces: the kernel `Thm` is a Cadenza *library value*, but the optimizer (`opt.rs`) is
*Rust*. The oracle must NOT reimplement the kernel in Rust — that would be a second, untrusted kernel
defeating the LCF story. Resolution: **reuse the compiler's existing compile-time `eval`** (12-metaprogramming
§Compile-Time Evaluation Is One Tier; `eval_ast::desugar_eval` — `(eval <ast>)` reconstructs the source form
and folds it through the ordinary path, already tested to execute hand-built `Ast` at compile time). The
discharge program is an ordinary Cadenza expression: it calls the kernel rules to build the obligation `Thm`
and returns a boolean — "does `concl(proof)` structurally equal `no-overflow@Id` AND are its `hyps` ⊆ the
node's stated precondition?" That expression is **evaluated at compile time**; the optimizer consumes only
its boolean/`Option` result. So:
- The **oracle** (`StructId → Option<discharged-Thm-handle>`) is, concretely, "compile-time-eval the
  discharge program for this node; `Some` iff it returns the match-true." No `Thm` representation crosses
  into Rust — only the already-checked boolean does. The kernel stays the sole `Thm` authority.
- The **match predicate** (conclusion-structural-equality + hyp-subset) is written IN CADENZA, in the same
  trusted kernel module, and pinned in the corpus (b1 already exercises the discharge; b2 adds the match
  predicate + the "wrong-assumption `Thm` is rejected" pin). The Rust side trusts only "compile-time eval
  returned true," which it cannot forge (eval runs the real kernel).

### What the optimizer must trust (and what it must NOT)
- **Trusted:** the kernel `Thm` type (unforgeable), the compile-time-eval tier (already trusted for macros),
  and the *predicate* "this `Thm`'s conclusion is the obligation that `overflow-check@Id` discharges." That
  predicate must itself be audited — a sloppy match (e.g. ignoring the node's precondition context, or
  matching a `Thm` proven under different bindings) would license an unsound elision. **This match predicate
  is the new trusted surface of Inc-b** and gets the same adversarial pinning the kernel boundary got
  (breaker: "supply a `Thm` that looks like it licenses `@Id` but was proven under different assumptions —
  does the elision wrongly fire?").
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
  obligation is NOT provable" (the check must stay). Obligations keyed by a placeholder Core-`Id` so the
  corpus already speaks v-core-opt's `Id` language. Ships with its `.gate-baseline`. Validates the
  denotation + discharge end-to-end with zero risk.
- **b2 — the match predicate (IN CADENZA) + the compile-time-eval oracle + a discharge→elision PROTOTYPE.**
  Write the trusted "does this `Thm` license `no-overflow@Id`" predicate as a Cadenza function in the kernel
  module (conclusion structural-equality + hyps ⊆ node precondition), and pin it in the corpus — including
  the soundness pin that a `Thm` proven under DIFFERENT assumptions is REJECTED. The oracle
  (`StructId → Option<Thm-handle>`) is compile-time-eval of the discharge program (§3 "compile-time eval");
  the Rust side consumes only its boolean. Prototype the Core elision pass installing an unchecked override
  on the true result — v-core-opt's slice-1 override seam (`db.install_core_override`) is LANDED, so this is
  a real `CorePass`. Adversarially pinned (breaker) BEFORE it gates anything real. Sync ↔ v-core-opt slice-2.
- **b2 — the match predicate (IN CADENZA) + the compile-time-eval oracle + a discharge→elision PROTOTYPE.**
  ✅ DONE. Corpus half landed (`licenses` predicate + wrong-assumptions soundness pin,
  `26-program-conditions.sexp`); mechanism half (`ProofElisionPass` `CorePass` prototype, `opt.rs` tests)
  pending. (Entry kept above; marked done here.)
- **b3 — proof-guided elision on the real overflow guard, via the Core-tier DISJUNCTION (SIMPLIFIED §3).**
  NO new Core node (Slice-2a cancelled). I land `discharged_no_overflow(db, id) -> bool` — first as a
  `false`-returning STUB (behavior-neutral) so v-core-opt's thin Core wrapper (Slice-5) can OR it into
  `arith_provably_in_range` and land green; then b3 fills the body (compile-time-eval the discharge program,
  return the `licenses` boolean). Both backends already consult the predicate (rust via Slice-2 `ca0f82a6b`),
  so a licensed node sheds its guard on BOTH with zero emit change. **Differential gate covers wasm AND rust**
  (the rust caveat is retired — the rust case now PASSES; reference v-core-opt's Slice-3 narrow-masked-add
  pin): a proven-safe add computes the same value guard-free as checked on both backends, and an UNproven add
  still traps on both. This is the operator's headline deliverable.
- **b4+ — the annotation SURFACE (2B, `@requires`/`@ensures`) — GREENLIT + parse SETTLED (§2); then wider
  obligations (bounds checks, exhaustiveness), then effect-ful conditions (1B) if wanted.**

Each increment is one gated unit; b3 (the disjunction wiring) is the next unit now that b2 is done.

---

## 6. Forks — ALL RESOLVED (operator 2026-07-16; v-core-opt + v-syntax seams settled)

1. **Semantics→logic bridge: shallow denotation (1A) on the pure arithmetic fragment first.** ✅ operator
   default confirmed — proceeding at b1.
2. **Annotation surface: `@requires`/`@ensures`.** ✅ operator GREENLIT ("very consistent with the other
   languages"); verify-blocks (2A) for b1–b3, `@requires`/`@ensures` (2B) as the shipping surface at b4.
   **Parse SETTLED with v-syntax** — no new primitive, result binding `it`, `old()` deferred to 1B (§2).
3. **Proof→opt interface: pure oracle query `Id → Option<Thm-handle>` keyed by stable Core-`Id`, elision a
   `CorePass` at the Core tier (both backends inherit), optimizer's trusted action = a single
   conclusion-match against a real `Thm`.** ✅ operator confirmed the "prove no-overflow → elide" shape;
   SEAM SETTLED with v-core-opt (§3) — prototype at b2.
4. **Ownership: stays under v-verification through b1–b3; revisit a co-scoped vertical at the 2B surface.**
   ✅ operator confirmed.

## 7. Coordination — the opt seam (SIMPLIFIED to a disjunction, 2026-07-17)

- **v-core-opt** — owns the existing Core-tier elision predicate `arith_provably_in_range` and a thin Core
  **wrapper (their Slice-5)** that ORs my oracle into it:
  `provably_no_overflow = arith_provably_in_range(…) OR discharged_no_overflow(db, id)`. **Slice-2a (the new
  Core unchecked node) is CANCELLED** — no node needed; eliding = making the predicate return true.
  **Slice-2 (`ca0f82a6b`, LANDED)** made rust's `emit_arith` consult the predicate too, so both backends
  already inherit it. Slice-3 adds the narrow-masked-add both-backend pin. Default=check invariant matches
  their `DESIGN-tiered-optimization-levels-rcdzc.md` §9.2.
- **v-wasm-opt** — the wasm emit already consults `arith_provably_in_range` (`select.rs`); co-verified the
  both-backend byte-identity triple on trunk. Nothing further needed for b3.
- **v-rust-backend** — **RETIRED as a separate touch**: the rust consult that was the "4th touch / wasm-only"
  caveat is DONE (v-core-opt's Slice-2 `ca0f82a6b`). rust's `emit_arith` now emits `wrapping_add` when the
  predicate is true. My b3 differential gate keeps the rust case, now as a PASSING positive.
- **me (v-verification)** — own `discharged_no_overflow(db, id) -> bool` (lands first as a `false` STUB so
  v-core-opt's wrapper is behavior-neutral, then b3 fills the body = compile-time-eval the `licenses`
  predicate) + the **trusted match predicate** (`licenses`, in Cadenza). This is Inc-b's only new trusted
  surface.
- **v-metaprogramming** — the denotation `Ast → Term` (§1) lives on their reflected `Ast`. Expect gaps →
  REPORT/FIX. Confirm the `Ast` exposes every construct the arithmetic fragment needs.
- **v-syntax** — the `@requires`/`@ensures` parse (2B) is **SETTLED** (2026-07-16): fits the existing
  glued-annotation `@name(args)` form with NO new primitive; result binding `it` (a plain name my
  elaboration binds, front-end blesses nothing extra); `old()` pre-state deferred to 1B. Nothing more from
  them until b4, when the parse is already ready (they're registered names). I ping them at b4.
- **breaker** — from b2 on, adversarial cases against the NEW trusted surface: the match predicate. "Supply
  a `Thm` proven under different assumptions that superficially matches `@Id` — does the elision wrongly
  fire?" This is Inc-b's soundness boundary, the analogue of the §3 forge vectors.
- **v-agent-harness** — a downstream consumer (Inc-a §7): a self-modification carrying a `Thm` that it
  preserves an invariant is exactly a program-condition discharge. The b-track machinery is what states
  those conditions. Revisit when they reach their Inc-3.

---

## 8. THE CAPSTONE — `@trap_free`: prove a function NEVER crashes (operator directive, 2026-07-17)

**Operator directive:** *"a way to prove that a function is completely free of traps … incredibly powerful
for building super reliable systems that are completely crash free."* This is the capstone of the whole
Inc-b arc: where `@ensures` proves ONE stated postcondition and proof-guided elision proves ONE no-overflow
obligation, `@trap_free` proves that **EVERY trap source in the function body is unreachable on every input
satisfying `@requires`** — so the function is statically guaranteed never to crash.

### 8.1 Surface (fork T1, route to operator)
A `@trap_free` (or `@total`) annotation on a `def`, in the `@requires`/`@ensures` family (same glued-
annotation parse, no new primitive):
`@trap_free @requires(P) def f x = body`. It takes NO predicate argument — it is a whole-body promise. Fork
T1 (naming): `@trap_free` (precise — "no trap") vs `@total` (familiar from type theory, but "total" also
implies termination, which this does NOT prove — a `@trap_free` function may still loop forever). **My lean:
`@trap_free`** — it names exactly the guarantee (no trap ≠ terminates); `@total` overclaims. Route to operator.

### 8.2 The obligation — every trap source proven unreachable
`@trap_free f` denotes to the CONJUNCTION of a per-trap-source obligation over `body`, each under the
`@requires` precondition. The trap sources (from the runtime trap set + `is_trap_free`):
| Trap source | Per-node obligation |
|---|---|
| integer **overflow** (`+`/`-`/`*` checked arith) | `no-overflow@Id` — the b1–b3 obligation, ALREADY built |
| integer **divide/mod by zero** (`/`,`%`) | `divisor ≠ 0` at the node (`GT`/`≠` obligation, new order rule) |
| **out-of-bounds** index (`List.at`/`Bytes.at`/…) | `0 ≤ i < len` at the node (a bounds obligation) |
| **partial match / unreachable arm** | the match is EXHAUSTIVE on the scrutinee's reachable values (the exhaustiveness checker already proves this for total matches; the obligation is "no `Unreachable` node reachable") |
| explicit **`trap()`** / effect-abort | the `trap` node is unreachable (its guarding condition is provably false) |

So `@trap_free` is the **whole-function generalization of proof-guided elision**: elision proves ONE
overflow guard unreachable to drop it; `@trap_free` proves ALL trap sources unreachable to certify the
function. It REUSES the b1–b3 machinery per trap source (the discharge, the `licenses`-style match), adding
the divide-by-zero and bounds obligation shapes (new order/range rules in the arithmetic base, same
checked-schema discipline).

### 8.3 Discharge + guarantee (fork T2, route to operator)
The kernel discharges each per-source obligation (compile-time eval, as b4c); the function is `@trap_free`
iff EVERY trap source is proven unreachable. **Fork T2 — the failure mode when a source can't be proven:**
(a) REJECT (compile error "cannot prove `@trap_free`: <the un-proven trap source> may trap") — the
annotation is a PROMISE, so an unproven promise is an error; OR (b) warn + fall back to runtime checks. **My
lean: REJECT** — `@trap_free` is a guarantee the author asserts; silently keeping runtime checks would break
the "proven crash-free" contract the operator wants (and the author would think they had a guarantee they
don't). Route to operator. **Bonus (the elision payoff):** a proven `@trap_free` function's guards are ALL
provably dead → the optimizer elides EVERY one (the b3 disjunction, applied per source) — so proven-crash-
free code is ALSO faster (no runtime checks). That is the operator's "proofs feed optimization" at whole-
function scale.

### 8.4 Dependency + increment plan
`@trap_free` **depends on the kernel-location fork** (§3-adjacent, still pending operator): compile-time
discharge needs the kernel available, same as b4c. It also reuses proof-guided elision's per-trap-source
reasoning. Increment plan (after the kernel-location ruling):
- **t1 — corpus:** hand-author the per-trap-source obligations (divide-by-zero, bounds, exhaustiveness,
  explicit-trap) as `bounds`-kernel discharges, like the b1 `+`/`-`/`*` cases — validate each source's
  obligation discharges (and its NEGATIVE: an un-bounded divisor is NOT provably non-zero → stays trappable).
- **t2 — the `@trap_free` conjunction:** a def is trap-free iff every trap-source obligation in its body
  discharges; corpus-pin the whole-function certificate + the reject when one source can't be proven.
- **t3 — surface + optimizer:** the `@trap_free` annotation (fork T1) + REJECT diagnostic (fork T2) +
  elide-all-proven-guards (the whole-function elision payoff).

### 8.5 Forks routed to operator
- **T1 (surface naming):** `@trap_free` (my lean — names the exact guarantee) vs `@total` (overclaims
  termination). 
- **T2 (failure mode):** REJECT an unprovable `@trap_free` (my lean — it's a promise) vs warn+runtime-fallback.
- **(carried) kernel-location fork** — `@trap_free` needs it resolved too (compile-time discharge).

---

## 9. Kernel-location = (A) BUNDLED PRELUDE — RULED by operator (2026-07-17), the implementation plan

**Operator ruling:** *"I'm fine to put the verification kernel in the prelude for now."* So the trusted HOL
kernel ships as a compiler-bundled **prelude module**, always in scope; the compiler compile-time-evals the
`licenses`/discharge check against the REAL kernel (no Rust re-implementation — one trusted copy, preserving
the unforgeable-LCF guarantee). This one ruling unblocks BOTH b3 proof-guided-elision AND the `@trap_free`
capstone (same compile-time-discharge dependency). "for now" = pragmatic current choice, not necessarily
forever.

**The mechanism (no precedent — the prelude is Rust-built today; `db.prelude` maps a builtin name → an arena
occurrence, `db.rs:1021`).** The kernel is a MODULE (types + many defs), not a single builtin, and it must
NOT be re-implemented in Rust (that would be the second kernel the LCF design forbids). So embed the kernel
SOURCE and load it AS A LINKED PACKAGE MEMBER:

**REFINED a1 architecture (2026-07-17) — link the kernel as a package member, NOT prelude-install.** An
initial plan was "install the kernel into the `db.prelude` map + carve out the opacity gate." Investigation
found two problems with that: (i) `prelude::install` builds Rust-constructed nodes in the MAIN arena, but
`parse` yields a FRESH arena → prelude-install needs arena-merge surgery; (ii) worse, opacity is gated on
`is_linked_package()` = `file_scope.is_some()` (db.rs:3076), and prelude nodes are "in no file" → a
prelude-installed `Thm` would be FORGEABLE (the soundness blocker §A1). BOTH problems vanish if the kernel is
loaded the way a MULTI-FILE LINKED PACKAGE already is: `Db::load_linked(merged_arena, Linkage)` (db.rs:1734,
the real compile entry at compile.rs:129) takes a merged multi-file arena + a `Linkage` (per-file demux +
scopes) and SETS `file_scope` — so a linked member's `Thm` opacity works NATURALLY, no carve-out.
- **a1 — link the bundled kernel as an extra package "file."** `include_str!` the trusted kernel source
  (the `bounds`/`hol` module, promoted from the corpus to a canonical compiler-bundled asset), and at the
  compile entry PREPEND it as an additional file in the merged arena + an extra `FileSpan`/`FileScope` in the
  `Linkage` (exports its rules + `Thm` handle). It is then a genuine linked module: `is_linked_package` true,
  `Thm` opacity fires (CDZ0214) with NO carve-out and NO prelude-arena surgery. The kernel is a
  compiler-prepended package member, always present.
- **a2 — the kernel's exports are in scope** via the normal linked-package import/export surface (its
  `FileScope` exports `licenses`/rules/`Thm`); the synthesized discharge program imports them like any
  cross-file reference. No new resolution path.
- **a3 — compile-time-eval the discharge.** With the kernel linked + in scope, the b4c oracle synthesizes a
  discharge program (`(licenses <proof> <obligation> <pre>)`) and compile-time-evals it (`eval_ast`, §3) to a
  boolean; b3's `discharged_no_overflow` returns it (fail-closed on an eval trap). The payoff.
- **FORGE-VECTOR pin (with a1):** a Rust `#[test]` that a bundled-kernel `Thm.MkThm(...)` outside the kernel
  file is CDZ0214 (opacity holds through the linked-member path) — the regression guard for §A1.

**Sequencing:** a1 (link the kernel as a package member) is the foundation — the first real slice of the
unblocked arc. Then a2 (exports in scope — mostly free via linkage) → a3 (compile-time-eval wiring = b4c) →
b3 (the oracle) → the
`@trap_free` t2/t3. The t1 per-trap-source obligation corpus (div0 done; OOB/exhaustiveness/explicit-trap
next) proceeds in parallel — it's kernel-location-independent and feeds a3/b3's discharge targets.

---

## 10. DATA TYPE INVARIANTS — `@invariant` (operator directive, 2026-07-17)

**Operator directive:** *"data type invariants … annotations on data types that must be held. Just like
requires/ensures these would be included as part of verification as well as optimizations."* This is the
DATA-level member of the verification-annotation family — `@requires`/`@ensures` are FUNCTION pre/post-
conditions, `@trap_free` is a whole-function crash-free proof, and `@invariant` is a property EVERY VALUE of
a type maintains — all discharged by the same HOL kernel, all feeding both verification and optimization.

### 10.1 Surface (fork I1, route to operator)
`@invariant(<predicate over the value>)` on a type/record/sum declaration, in the `@`-family:
`@invariant(> (len it) 0) (type NonEmptyList …)` / `@invariant(and (>= it 0) (<= it 100)) (type Percent Int64)`.
The value is bound to `it` (same result-binder convention as `@ensures` — one implicit-subject name across
the family, v-syntax's ruling). Fork I1 (naming/placement): `@invariant` on the type decl (my lean — names
the concept, fits the family) vs a refinement-type surface (`Int64 where (…)`, the 2C option) — the two share
the `it` binder so they stay consistent; `@invariant` first, refinement-types later. Route to operator.

### 10.2 The obligation — ESTABLISH + PRESERVE (the classic invariant proof shape)
An invariant `I` on type `T` is really a pair of obligations, both reusing the `@requires`/`@ensures`
machinery:
- **ESTABLISH:** every CONSTRUCTOR of `T` must prove its result satisfies `I` — i.e. `I` is an implicit
  `@ensures(I[it := constructed value])` on each constructor. A constructor that can't establish `I` is a
  compile error (or, per the failure-mode fork, must carry a `@requires` strong enough to).
- **PRESERVE:** every operation returning `T` must prove it maintains `I` — `I` is an implicit `@ensures(I)`
  on the result, AND (the dual) a consumer may ASSUME `I` on any `T` input (an implicit `@requires(I)`
  granted for free, since every `T` value provably holds `I`). So an invariant is simultaneously a proof
  OBLIGATION on producers and a proof GIFT to consumers — that gift is what powers the optimization.
This is exactly `@ensures`-on-every-constructor + `@requires`-you-get-free-on-every-consumer, so b4c's
denotation + b3's discharge machinery apply unchanged; `@invariant` adds the surface + the establish/preserve
obligation generation, not new kernel machinery.

### 10.3 Optimization payoff — data-level proof-guided elision
A held invariant is a proven fact the optimizer consumes, exactly like a proven no-overflow (§3): a
`NonEmptyList` value provably has `len > 0` → the optimizer ELIDES every empty-guard/`head`-bounds-check on
it; a `Percent` provably in `[0,100]` → range-guards on it are dead. This is the DATA-level analogue of b3:
where b3 elides a guard a *proof at a node* discharges, `@invariant` elides a guard the *type's invariant*
discharges — and because the invariant holds for EVERY value, the elision applies everywhere the typed value
flows (a stronger, whole-type version of the per-node elision). Same `provably_no_overflow`-style disjunction
seam, keyed on the value's type-invariant instead of a per-node `Thm`.

### 10.4 Failure mode (fork I2, route to operator) + dependency
**I2 — a constructor that can't establish `I`:** REJECT (compile error "constructor cannot establish
invariant `I`") — my lean, consistent with `@ensures`/`@trap_free` (an invariant is a promise). A `@requires`
on the constructor that makes `I` establishable is the escape hatch. Route to operator.
**Dependency:** `@invariant` shares the compile-time-discharge path — the kernel-in-prelude (A1) + the b4c
oracle + b3's elision seam. It lands AFTER the a1/a3/b3 foundation (it reuses all of it); the corpus
obligation shapes (establish/preserve as `@ensures`-style discharges) can be pinned in parallel now, like the
`@trap_free` t1 sources.

### 10.5 Forks routed to operator
- **I1 (surface):** `@invariant(pred)` on the type decl (my lean) vs a refinement-type `where`-surface
  (defer; shares the `it` binder).
- **I2 (failure mode):** REJECT a constructor that can't establish the invariant (my lean) vs warn.
- **(shared) dependencies:** the (A1) kernel-in-prelude + b4c/b3 discharge+elision path `@invariant` reuses.

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
