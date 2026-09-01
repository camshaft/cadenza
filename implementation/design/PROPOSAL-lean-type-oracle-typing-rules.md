# Proposal — the Lean type-oracle's typing judgment (T1 core), for operator review

**Author:** `design-type-oracle`.
**Status:** MERGED (#7438, operator-approved "looks good", 2026-09-01). This revision is a follow-up amendment
folding in a later operator direction — run the oracle on ALL inputs so it also catches false-ACCEPTS (§1a) —
submitted as a fresh open PR for the operator to review and merge (the fleet does not merge it).
**Relationship to the merged design:** this REFINES, does not replace, the landed
[`DESIGN-lean-type-system-oracle.md`](DESIGN-lean-type-system-oracle.md) (#7421). That doc fixed the oracle's
*shape* — the verdict algebra, the differential classification, the `(typecheck …)` wire, the increment
ladder T0→T4, the seams, and the ownership (`v-lean-oracle`). Phase **T0.1 is already built** (`Oracle/Type.lean`:
`Ty`/`TypeVerdict`/`RcdzcVerdict`/`judgeTypecheck`, all-declining `infer`; #7433). What that design deliberately
left one-line ("T1.1 — HM inference over the pure total core") is the single most correctness-critical part of
the whole oracle: **the actual typing judgment.** An oracle whose type rules are wrong does not fail safe — it
manufactures false findings and wastes the fleet chasing phantom compiler bugs. So this is the piece that
warrants explicit sign-off before T1 lands, and this document is the reviewable specification of it.

Everything here is a from-scratch reading of the normative `spec/capabilities/type-system.md` (line citations
as `ts:N`) and shares zero code or intent with rcdzc — that independence is the entire point (design §0).

---

## 1. What T1 models, and what it deliberately does not

T1 is the **pure total core**: the fragment for which principal-type inference by unification (`ts:28-38`) is
straightforward and unambiguous, so the oracle's verdict is trustworthy enough to *positively disagree* with
rcdzc. Everything outside it returns `Unsupported` — a sound coverage gap that is *always* a `skip`, never a
finding (design §1.2; built into `judgeTypecheck`).

**In scope at T1:**

| Construct | Spec anchor |
|-----------|-------------|
| Scalar literals & their types — `Int`/width, `Bool`, `Unit`, `String`, `Char` | `ts:20-24`, numeric-model |
| `let` with **generalization** of free type variables | `ts:40-44` |
| `if` (both branches unified; `Never` unifies with anything) | `ts:76-84` |
| Curried `fn`/closure introduction + application (arrow intro/elim) | `ts:28-36` |
| Tuple construction & positional projection | `ts:70-74`, `ts:130-146` |
| **Closed** record construction & field access | `ts:70-74`, `ts:184-190` |
| **Closed** sum construction (`Some/None/Ok/Err/Ordering` + user closed sums) | `ts:70-74`, `ts:192-204` |
| `match` on a **closed** sum with **first-match** + closed-set exhaustiveness | `ts:206-208` |
| Ascription `(: e T)` as a **constraint**, unified — never an override | `ts:50-54` |
| The unresolved-escaping-variable rejection (a bare `None` result, etc.) | `ts:34` |

**Deliberately deferred (each ⇒ `Unsupported` at T1, lands in a later fragment slice — design §2 "Growing the
type-system fragment"):**

| Deferred construct | Why deferred | Spec anchor | Lands |
|--------------------|--------------|-------------|-------|
| **Open records / row polymorphism** | row unification + row variables; a naive closed check would false-reject a valid open-record use | `ts:86-128` | fragment slice |
| **Effect rows** | same row machinery; needs the effect vocabulary | `ts:148-152` | fragment slice |
| **Nominal / abstract types** (cross-boundary rules) | identity = fully-qualified name; module-scoped comparison rejects | `ts:154-204` | fragment slice |
| **First-class type-values / generics / dictionaries** | the bidirectional-checking boundary (`ts:56-60`): monomorphization by compile-time reduction, NOT unification — a fundamentally different judgment | `ts:232-268` | fragment slice |
| **Units of measure** | dimension algebra | units spec | fragment slice |
| **Open sums / open-tail match** | row variable over variants | `ts:210-216` | fragment slice |

The deferral discipline is what makes the oracle safe from day one: **the oracle only ever emits a positive
verdict (`WellTyped`/`IllTyped`) on a program every construct of which it fully models.** The moment `infer`
meets a deferred construct anywhere in the program, the whole program's verdict is `Unsupported`. This is the
same soundness contract the semantics oracle uses for `Unsupported`/`Diverges` and the `differential.rs`
`Declined⇒Agree` arm (design §1.2). §5 argues it rigorously.

---

## 1a. The differential runs on ALL inputs — bidirectional from the first increment (operator direction, 2026-09-01)

> Operator, verbatim: *"For the type check oracle we should make sure to run it on all inputs I think. Cause
> then we can also check that the compiler does not accept a poorly typed program as well."*

The T2 fuzzer differential (design §2) feeds the oracle **every** generated program together with rcdzc's
`cdz check` verdict — **both the accepted bucket and the rejected bucket**, not just rejects. This makes the
oracle bidirectional from its first fuzzer increment, at the *accept/reject boundary* — no principal-type
comparison (T4) is needed to catch either direction of a boundary-level disagreement:

| rcdzc `cdz check` | oracle `infer` | finding |
|-------------------|----------------|---------|
| reject(code)      | `WellTyped`    | **false-reject** — over-strict coded reject (the operator's first-priority direction) |
| decline (codeless)| `WellTyped`    | capability-gap — should-work-not-yet-built (backlog TODO) |
| **accept**        | **`IllTyped`** | **FALSE-ACCEPT / soundness hole** — rcdzc accepted a poorly-typed program (the operator's ask here) |
| accept            | `WellTyped`    | agree at the boundary (principal-type agreement compared at T4) |
| reject / decline  | `IllTyped`     | agree |
| any               | `Unsupported`  | skip — sound coverage gap |

`judgeTypecheck` **already implements every row** (built, #7433): its `.illTyped _, .accept` arm emits
`mismatch "false-accept: … (soundness hole)"`. So the false-accept direction needs no verdict-protocol change
and no new Lean — only that the fuzzer *feed the accepted bucket too*, which is a cheap change (the `cdz check`
verdict is already computed per program; the accept bucket streams the same `(typecheck <program> (accept))`
item). This is the design's §1.2 row `accept + IllTyped ⇒ FALSE-ACCEPT` promoted from "reached at T4" to
**caught at the boundary from T2**, superseding the design's earlier scheduling of the false-accept direction
solely at T4.

**Two false-accept tiers, so both land at their natural rung:**
- **Boundary false-accept (T2):** rcdzc *accepts*, the oracle *positively rejects* (`IllTyped`) a fully-modeled
  program → a soundness hole caught the moment T1's rules can reject. This is the operator's "compiler does not
  accept a poorly-typed program" check, and it is available as soon as T1 lands (no T4 dependency).
- **Principal-type false-accept (T4):** *both* accept but the inferred principal types disagree — rcdzc accepted
  the program at the *wrong type*. This subtler hole needs the T4 type-agreement comparison (design §2 Phase T4,
  OQ-C) and lands there.

**Soundness is unchanged (§5).** The accept bucket produces a finding ONLY when the oracle returns a *positive*
`IllTyped` on a program it *fully models*; an `Unsupported` over an accept is a `skip`, exactly as an
`Unsupported` over a reject is. So running on all inputs strictly *adds* the false-accept checks the moment
coverage permits and introduces **zero** new false-alarm risk — the same monotone-coverage guarantee (§5 bucket
4). Priority is unchanged: **false-reject remains the first shipped increment** (design §6, operator); the
accepted bucket rides the same T2 wiring at the same time because it costs nothing extra to feed it.

---

## 2. The type universe at T1

Extends the `Ty` already built in `Oracle/Type.lean` (int-width/bool/unit/string/char/fn/tuple/var) with the
closed structural forms T1 needs:

```
Ty =
  | int (width : Nat) (signed : Bool)     -- integer type indexed by signedness and bit width (width set admitted by the numeric-model default)
  | bool | unit | string | char
  | fn    (dom cod : Ty)                   -- curried; a multi-arg fn is nested arrows
  | tuple (elts : List Ty)                 -- positional, arity is part of the type (ts:130-146)
  | record (fields : List (String × Ty))  -- CLOSED; fields sorted by name and unique (no duplicates) for shape-equality (ts:70-74)
  | sum    (variants : List (String × Option Ty)) -- CLOSED; variants sorted by name and unique; None payload = nullary variant (ts:192-204)
  | never                                  -- the empty sum; unifies with any type (ts:76-84)
  | var    (id : Nat)                      -- a unification variable
```

**Type equality is structural** (`ts:70-74`, `ts:184-190`): two records are equal iff their `(name,type)` sets
coincide (hence the sorted-fields normal form); two tuples iff element types coincide positionally; two sums iff
their `(variant,payload)` sets coincide. `never` is the identity for unification (`ts:82`). A `Scheme` for
`let`-generalization is `∀ āvars. Ty` (§4).

> **T1 leaves `int` width as a bound-then-checked attribute, matching the semantics oracle's parametric-integer
> model** ([[cadenza-parametric-integer-model-and-ascribed-value-form]]): an unannotated integer literal gets a
> fresh width variable that ascription/use constrains; a width that stays free at an escape is the `ts:34`
> rejection. Numeric *range/overflow* checking (`CDZ0302`) is a fragment slice, not T1 — T1 handles the
> *type*-level `Int`-vs-other mismatch (`CDZ0301`), not out-of-range literals.

---

## 3. The typing judgment (the reviewable core)

`infer : Module → TypeVerdict` resolves the module to its `main` definition (as `Oracle/Eval.lean` already
does) and runs **Algorithm-W-style inference**: synthesize a type + a substitution, solving equality
constraints by unification (`ts:28-30`). Below, `Γ ⊢ e : τ` reads "under environment `Γ`, `e` synthesizes
`τ` (with the accumulated substitution applied)". Each rule cites the spec sentence it realizes; a rule's
failure column names the CDZ code the oracle emits (§4 is the full table).

| # | Construct | Rule (synthesis) | Failure ⇒ code | Spec |
|---|-----------|------------------|----------------|------|
| L | scalar literal | `int`-lit → `Int w` (`w` fresh); `#t/#f`→`Bool`; str→`String`; char→`Char`; `unit`→`Unit` | — | ts:20 |
| V | variable `x` | `x:σ ∈ Γ` ⇒ `τ = instantiate(σ)` (fresh vars for the ∀-bound) | `x∉Γ` ⇒ **CDZ0101** Unbound | ts:36 |
| A | ascription `(: e T)` | `Γ⊢e:τ`; `unify(τ, T)`; result `T` | `unify` fails ⇒ **CDZ0203** TypeMismatch | ts:50-54 |
| Fn | `(fn (x…) body)` | fresh `α…` for params; `Γ,x:α… ⊢ body:β`; result `α₁→…→β` | non-linear param binder ⇒ **CDZ0102** | ts:28-36 |
| App | `(f a)` | `Γ⊢f:τf`, `Γ⊢a:τa`; fresh `β`; `unify(τf, τa→β)`; result `β` | `τf` not an arrow / arg-type clash ⇒ **CDZ0203** (or **CDZ0201** if `f` is a non-callable head) | ts:36 |
| Let | `(let ((x e)…) body)` | `Γ⊢e:τ`; `σ = generalize(Γ,τ)` (§4); `Γ,x:σ ⊢ …` sequentially; result = body's type | duplicate binder ⇒ **CDZ0102** | ts:40-44 |
| If | `(if c t e)` | `unify(τc, Bool)`; `unify(τt, τe)`; result `τt` (with `never` absorbing) | `τc≠Bool` or `τt≠τe` ⇒ **CDZ0203** | ts:76-84 |
| Tup | `(tuple e…)` | each `Γ⊢eᵢ:τᵢ`; result `tuple [τ…]` | — | ts:130-146 |
| Prj | positional `.n` on a tuple | `Γ⊢e:tuple τ⃗`; `n < |τ⃗|` ⇒ `τ⃗[n]` | `n` out of arity ⇒ **CDZ0203** | ts:146 |
| Rec | closed record `(= f v)…` | each field `Γ⊢vᵢ:τᵢ`; result `record {fᵢ:τᵢ}` (dup field ⇒ error) | dup field ⇒ **CDZ0211** PresentField | ts:70-74,116-124 |
| Fld | `(. r f)` | `Γ⊢r:record ρ`; `f∈ρ` ⇒ `ρ.f` | `f∉ρ` ⇒ **CDZ0212** AbsentField | ts:104-108 |
| Con | closed sum ctor `(V payload?)` | resolve `V` to its declared sum `S`; unify payload with `V`'s declared type; result `S` | payload type clash ⇒ **CDZ0203**; ctor of an abstract type outside its module ⇒ **CDZ0214** | ts:192-204 |
| Mat | `(match s arm…)` | `Γ⊢s:sum σ`; each arm binds its variant's payload, all arm bodies unify; result the unified body type | arm variant ∉ `σ` ⇒ **CDZ0203**; missing variant ⇒ **CDZ0210** NonExhaustive; unreachable arm ⇒ **CDZ0213** | ts:206-208 |
| Div | a diverging expr (trap / `None`-forced) | result `never` (unifies with anything) | — | ts:82 |
| Esc | the program result `τ` contains a var no use constrains | reject | **CDZ0203** (type-determination) | ts:34 |

Notes that a reviewer should weigh:

- **First-match semantics for `match` (`ts:206`, first-match):** the oracle checks arms top-to-bottom; a later
  arm subsumed by an earlier one is `CDZ0213` RedundantArm (a T3 refinement — at T1 redundancy is *not* modeled
  and such a program stays whichever verdict the reachable arms give, never a false-reject).
- **Ascription is `constrain-not-contradict` (`ts:50-54`):** `(: e T)` unifies `T` with the inferred `τ`; it
  never *replaces* `τ`. A program whose annotation and inference agree is `WellTyped`; one where they cannot
  unify is `IllTyped CDZ0203`. This is the rule most likely to be subtly wrong in a hand-rolled checker — it is
  called out for explicit review.
- **`never` / the empty sum (`ts:76-84`):** a trap, or forcing an absent optional, has type `never`, which
  unifies with any expected type. This is why `(if c 1 (trap …))` is well-typed at `Int` — the oracle MUST
  treat `never` as bottom or it will false-reject every partial-branch program. Called out for review.
- **The escaping-free-variable rejection (`ts:34`):** a bare `None` as the program result (type `Option ?`) is
  `IllTyped CDZ0203` unless a use constrains the payload; a *consumed* such value type-checks. The oracle
  applies this only at the program's escape (the `main` result / a value crossing to the host), matching the
  spec's "the ambiguity bites only at an unannotated escape" (`ts:34`).

**Totality (design §6, OQ-F):** `infer` runs under a fuel/size budget; exceeding it ⇒ `Unsupported`, never a
hang and never a guessed verdict — matching the semantics oracle's `Diverges`-soundness rule. Unification
carries the **occurs check** (a var unified with a type containing it ⇒ the constraint is unsatisfiable ⇒ the
program is `IllTyped CDZ0203`, the infinite-type case), so the solver terminates.

---

## 4. The CDZ-code decision table (grounds T3)

Every rejection the oracle emits carries a specific `Code` (design §1.1; T3 matches it against rcdzc's). Each
row pairs the oracle's fault with the **spec sentence that mandates the rejection** and rcdzc's `Code` enum
name (`rcdzc/src/diag.rs`). At T1 only the accept/reject *direction* gates the additive baseline; the *code*
is advisory until T3 models a code family fully (design §7 OQ-D) — but the oracle emits the intended code from
T1 so the T3 flip is data, not new logic.

| Oracle fault | Code | rcdzc `Code` | Spec sentence mandating rejection |
|--------------|------|--------------|-----------------------------------|
| unbound name | CDZ0101 | `Unbound` | resolution precondition (name must be in scope) |
| non-linear / duplicate binder | CDZ0102 | `NonLinearBinder` | linear-binding rule |
| malformed / non-callable application head | CDZ0201 | `Malformed` | well-formedness precondition |
| **failed unification (the workhorse)** | CDZ0203 | `TypeMismatch` | `ts:38` (contradictory constraints), `ts:54` (annotation clash), `ts:146` (tuple index), `ts:34` (undetermined escape) |
| `Int`-vs-non-`Int` in an arithmetic position | CDZ0301 | `NumericMismatch` | numeric-model (mixed Int/Float etc.) |
| non-exhaustive match on a closed sum | CDZ0210 | `NonExhaustive` | `ts:206-208` |
| record field already present (combine/add) | CDZ0211 | `PresentField` | `ts:120-124` |
| record field absent (project/drop/update/access) | CDZ0212 | `AbsentField` | `ts:108,114,128`, `ts:104-108` |
| redundant match arm (first-match subsumed) | CDZ0213 | `RedundantArm` | `ts:206` (first-match) |
| ctor/strip of an abstract type outside its module | CDZ0214 | `AbstractCtor` | `ts:178-182` |
| nominal-vs-nominal / nominal-vs-structural comparison | CDZ0202 | `NominalMismatch` | `ts:164-170` (deferred to the nominal fragment slice) |

**The false-reject / capability-gap split hinges on rcdzc carrying a code (design §1.2):** a Lean-`WellTyped`
over an rcdzc **coded** reject is a *false-reject bug* (rcdzc wrongly emitted a diagnostic for a valid
program); over an rcdzc **codeless decline** it is a *capability gap* (should-work-not-yet-built) — routed as a
backlog/`(output V)` TODO, never a soundness alarm. `judgeTypecheck` already encodes exactly this (built,
#7433).

---

## 5. Why the oracle does not manufacture false findings (the soundness argument — the sign-off crux)

A finding costs fleet attention; a *false* finding costs it AND erodes trust in the oracle. The design's whole
safety rests on one invariant, argued here so the operator can accept or challenge it:

> **Positive-disagreement invariant.** The oracle emits a `mismatch` (a finding) only when it returns a
> *positive* verdict — `WellTyped τ` or `IllTyped code` — and that positive verdict is produced only for a
> program **every construct of which T1 fully models**. For any program touching a deferred construct (§1),
> `infer` returns `Unsupported`, and `judgeTypecheck`'s first arm makes that a `skip` unconditionally.

From this, each `mismatch` bucket is genuinely one of the four design-§1.2 outcomes and not noise:

1. **`WellTyped` vs rcdzc coded-reject (false-reject).** The oracle proved a full HM typing derivation over a
   fully-modeled program. If that derivation is *correct*, rcdzc's coded reject is the bug — the win. The only
   way this is a *false* alarm is an **oracle bug** (a wrong rule in §3) — which is exactly why §3 is the
   reviewable core, and why every confirmed finding is triaged (design §5 (a)-(d): rcdzc bug / capability gap /
   oracle bug / spec ambiguity) before it's filed against `v-inference`. Growing coverage never *invents* this
   alarm on an already-covered program; it only reveals it on newly-covered ones.
2. **`WellTyped` vs codeless decline (capability-gap).** Sound by construction — routed as a backlog TODO, not
   a bug. Even if the oracle's `WellTyped` were wrong, the consequence is a spurious backlog item, not a false
   bug report against the compiler.
3. **`IllTyped` vs accept (false-accept / soundness hole).** Caught at the accept/reject **boundary from T2**
   (§1a) — the fuzzer feeds rcdzc's *accepted* programs to the oracle, and a positive `IllTyped` over an accept
   is a soundness hole, available as soon as T1's rules can reject (no T4 dependency); T4 adds the subtler
   both-accept-but-principal-types-disagree tier. The oracle rejects only on a modeled fault with a concrete
   code (§4). A wrong reject here is again an oracle bug caught in triage, never silently filed.
4. **`Unsupported` — always `skip`.** Zero risk. Adding coverage strictly moves programs from bucket 4 into
   buckets 1-3, so **coverage growth is monotone: it can only ADD checks, never create a false alarm on a
   program that previously skipped correctly** (the same property `differential.rs`'s `Declined⇒Agree` relies
   on).

The three sub-claims a reviewer should specifically pressure-test:
- **`never` is bottom (`ts:82`)** — without this the oracle false-rejects every `(if … (trap))`. (§3 Div/If.)
- **Ascription constrains, never overrides (`ts:50-54`)** — without this the oracle either false-rejects valid
  annotations or false-accepts contradictory ones. (§3 A.)
- **Generalization does not escape its scope (`ts:44`)** — a `let`-bound var still constrained by an enclosing
  binding MUST NOT be generalized, or the oracle over-generalizes and false-accepts. `generalize(Γ,τ)`
  quantifies only vars free in `τ` but **not free in `Γ`** — the standard rule; called out because getting the
  `Γ`-freeness wrong is the classic Algorithm-W bug.

**Trust model (unchanged from design §5):** the spec is the arbiter; the recorded corpus expectation is the
tie-breaker where it exists; a spec ambiguity is a concierge `ask`, resolved in the spec, then encoded — never
guessed in Lean.

---

## 6. Gate & baseline (unchanged from design §4, restated for the reviewer)

- **Corpus conformance** — the new `oracleLeanTypeCheck` nix derivation (mirrors `oracleLeanCheck`): **0
  mismatch** across all realized cases; the `.type-oracle-baseline` diff is **additive-only** (a `skip→mismatch`
  flip never lands); realized-coverage count is reported and non-zero and drifts only upward.
- **Lean unit witnesses** — `#guard`/`lake test` over `infer` on representative well-typed accepts, ill-typed
  rejects with each modeled code, and `Unsupported` for each deferred construct; the `(typecheck …)` batch
  round-trip (already gated at T0.1).
- **`cargo xtask dev-gate`** for touched Rust (`cdz-smith`) — test + clippy + PINNED fmt.
- Does not touch `cdz-runtime`/`wit/runtime.wit` (frozen `REQUIRED_RUNTIME_HASH`); the semantics oracle
  (`oracleLeanCheck`/`oracleLeanAstRoundtrip`) stays green — this work only ADDS a judgment to `oracle-lean`.

---

## 7. Open decisions carried forward (defaults from design §7 hold; this depth adds two)

The design's OQ-A…OQ-F stand with their chosen defaults. Two decisions this typing-rule depth surfaces, each
with a chosen default the vertical may take without escalating:

- **OQ-G — width-variable model for integer literals.** *Default:* an unannotated int literal gets a fresh
  width var constrained by ascription/use; a still-free width at an escape is the `ts:34` rejection — reusing
  the semantics oracle's parametric-integer machinery ([[cadenza-parametric-integer-model-and-ascribed-value-form]])
  rather than a second width model. *Escalate only if* the two oracles would otherwise diverge on width.
- **OQ-H — how faithfully to model the minimal-conflict / two-site diagnostic (`ts:62-66`).** *Default:* the
  oracle emits the *code* and a direction string, NOT the minimal unsatisfiable constraint set or both source
  locations — code parity (T3) is the north star; minimal-conflict *quality* is rcdzc's diagnostic concern,
  routed to `v-diagnostics` as a weak finding, not something the oracle reproduces. *Escalate only if* the
  operator wants the oracle to also referee diagnostic minimality.

---

## 8. Reviewer checklist (the concrete sign-off points)

The operator's review reduces to six yes/no questions:

1. **Is the T1 modeled subset (§1) the right first cut** — pure total core in, rows/effects/nominal/generics/
   units out until their fragment slices?
2. **Are the typing rules (§3) a faithful, independent reading of `spec/capabilities/type-system.md`** — in
   particular `never`-as-bottom, ascription-constrains-never-overrides, and `let`-generalization-not-escaping?
3. **Is the CDZ-code decision table (§4) the right fault→code mapping**, and is the false-reject vs
   capability-gap split (coded-reject vs codeless-decline) the right triage line?
4. **Is the positive-disagreement soundness argument (§5) convincing** — does "the oracle only positively
   disagrees on a fully-modeled program, and coverage growth is monotone" hold up?
5. **Any construct that should move into or out of T1**, or any code mapping to change, before `v-lean-oracle`
   builds T1 on top of the landed T0.1?
6. **Is the all-inputs / bidirectional differential (§1a) right** — the T2 fuzzer feeds both rcdzc-accepted and
   rcdzc-rejected programs, catching boundary-level false-accepts from T2 (not deferring the whole false-accept
   direction to T4), while false-reject stays the first shipped increment?

On sign-off, `v-lean-oracle` builds T1 per §3-§4 (it owns `oracle-lean/` + `cdz-smith/src/lean.rs`; design §5).
This proposal changes nothing already merged and blocks nothing already building — T0.1's all-declining skeleton
is correct under it (every program `skip`s until a modeled rule fires).
