# DESIGN: Machine-checked verification — an LCF-style kernel baked into Cadenza

Status: **Increment 0 — design (nothing coded yet).** Vertical `v-verification`, subsystem `rcdzc`.
Operator directive (2026-07-16, verbatim intent): *"We need to get a vertical thinking about
machine-checked verification. I really like the idea of baking something like HOL-Light into the
language."*

This doc answers the four core design questions from the charter, commits to an Increment-0 shape,
and — most importantly — settles the **soundness-boundary** question the whole feature rests on. It
routes the genuine forks to the concierge (→ operator) at the end.

---

## 0. Executive summary — what I'm proposing and why it's cheap

**Increment 0 is candidate (a): an LCF-style kernel as an ORDINARY Cadenza LIBRARY**, not a compiler
feature. A module `hol` declares an **abstract** type `Thm` (a theorem) and `Term`/`Hty` (HOL terms
and types), exporting the type *handles* but **keeping the constructors private**. The only exported
functions that *return* a `Thm` are the primitive inference rules (`refl`, `trans`, `mk_comb`, `abs`,
`beta`, `assume`, `eq_mp`, `deduct_antisym`, plus `inst`/`inst_type` and the three HOL axioms). Because
Cadenza already enforces that **no code outside a module can construct or match an abstract type's
variants** (`CDZ0214`) and **cannot even compare or strip one** (`CDZ0202`), a `Thm` is *unforgeable*
by exactly the LCF discipline HOL-Light relies on: everything above the kernel is untrusted code that
can only obtain a `Thm` by calling a kernel rule.

**The key finding of this doc: Cadenza's opaque-type feature — already landed on `spec` and shipping —
is a ready-made LCF trust boundary.** The abstract-data-type work
([DESIGN-opaque-types-abstract-constructor-rcdzc.md](DESIGN-opaque-types-abstract-constructor-rcdzc.md),
COMPLETE) was built for "a non-empty list / a validated email / a positive Money" — but an *unforgeable
theorem* is the same mechanism at maximum stakes. The kernel needs **zero compiler change** for its
trust story; it is a library exercise plus a rigorous audit of the forge-holes (§3). That makes (a) the
right Increment 0 by a wide margin: it delivers the trust foundation everything else needs, it is a
flagship stress test of the opaque-type soundness guarantee (a real language-design win either way),
and it is buildable incrementally with the normal corpus/rcdzc/check gate.

Candidate (b) — verifying Cadenza *programs* — and (c) — a reflective proofs-as-`Ast` tie — are real,
larger, and **sequenced after** (a). See §5.

**We have prior proof this works.** A 2026-07-08 spike ([[hol-light-kernel-spike-2026-07-08]]) already
ported HOL-Light's `fusion.ml` kernel to Cadenza: `HolType`/`Term`/`Thm` as recursive sums with
explicit recursive `type-eq`/`term-eq`, and the `refl`/`assume`/`trans` rules — it compiled to a valid
wasm component and **proved+structurally-verified** `⊢ x = x` (REFL), `ASSUME`, and
`TRANS (REFL x) (REFL x) ⊢ x = x`. The spike confirmed the LCF discipline is expressible **pure, no
mutability** (the kernel's `ref` cells stratify into effect rows: PROOF = pure, CONSTRUCTION = Sig-read,
EXTENSION = Sig-write — so a fold-role module can be *certified axiom-free* by making `new_axiom` a
visible effect op, a checker that structurally cannot cheat). Its one flagged blocker was exactly
**constructor-level privacy** — which is now the landed opaque-types feature. So Increment 0 stands on a
demonstrated prototype; the new work is (1) wrapping it in the now-available opaque boundary and (2) the
soundness audit + gate coverage below.

---

## 1. What "baked into the language" MEANS here (charter Q1)

Three readings were on the table; here is the ruling and the sequence.

- **(a) LCF kernel as a library — Increment 0.** "Baked in" = the *trust primitive* HOL needs (an
  unforgeable proof type) is expressible using features the language already guarantees, with no
  `unsafe`, no privileged compiler hook, no bespoke `Thm` runtime. The soundness of a theorem reduces
  to the soundness of the opaque-type boundary + the ~200 lines of the kernel module — precisely the
  LCF "small trusted core" property. This is the most faithful rendering of "baking HOL-Light into the
  language," because HOL-Light's *own* trust story is exactly "an abstract `thm` type in an ML module
  whose only constructors are the inference rules." Cadenza's abstract types are Standard-ML-style
  abstract types; the port is close to mechanical at the trust layer.

- **(b) Verifying Cadenza programs (`@verify`/`@theorem`, refinement/pre-post types) — later.** This
  needs a semantics-to-logic bridge (a denotation of Cadenza terms into the HOL logic, or a separate
  program logic) and is a much bigger, possibly separate, design. It becomes *tractable* once (a)
  exists, because the kernel gives you the object logic to state and discharge the obligations in. Do
  not start here.

- **(c) Reflective tie via quote/`Ast` — an accelerator, not the foundation.** v-metaprogramming's
  `Ast` sum + `quote`/`eval` can, later, let tactics manipulate HOL terms as first-class syntax and let
  a proof be *checked* by re-running kernel calls the `Ast` denotes. Attractive for ergonomics (a
  `hol"..."` tagged-template DSL for terms, §4) but **must not be a trust dependency** — see §3.4, the
  reflection forge-hole. Treat (c) as sugar layered on (a), never as a second way to mint a `Thm`.

**Decision: Increment 0 = (a).** Sequence: (a) kernel → grow the logic → ergonomics → then scope (b)
as its own design that consumes (a).

---

## 2. The logic — which HOL fragment first (charter Q3)

HOL is classical higher-order logic: simply-typed λ-calculus (types built from type variables, a
function arrow, and declared type constants like `bool`, `ind`) over terms (variables, constants,
applications, λ-abstractions), with equality as the sole primitive logical constant and a handful of
primitive inference rules + three axioms.

**Target fragment, in order:**

1. **The equational core first** — the smallest set that proves the mechanism end-to-end:
   - Types: `Hty` = `TyVar name` | `TyApp const [Hty]` (so `bool`, `A -> B`, `ind` are all `TyApp`).
   - Terms: `Term` = `Var name Hty` | `Const name Hty` | `Comb Term Term` | `Abs Term Term`
     (the abstracted var is a `Var`). Well-typedness of a `Term` is checked *inside the kernel* at
     construction (a `mk_comb` that doesn't type-check yields a typed failure, never a `Thm`).
   - A theorem is a sequent `Thm { hyps : Set Term, concl : Term }` (HOL-Light's `(Γ ⊢ p)`).
   - Primitive rules for this core: **`REFL`** (`⊢ t = t`), **`TRANS`**, **`MK_COMB`**, **`ABS`**,
     **`BETA`** (`⊢ (λx. t) x = t`), **`ASSUME`** (`p ⊢ p`), **`EQ_MP`**, **`DEDUCT_ANTISYM_RULE`**,
     plus **`INST`** and **`INST_TYPE`** (term/type instantiation). This is *exactly* HOL-Light's
     `fusion.ml` primitive-rule set — porting it 1:1 keeps the audit surface identical to a system
     that has been trusted for 20 years.
2. **Then the logical constants + axioms** — define `T`, `∧`, `⇒`, `∀`, `∃`, `∨`, `F`, `¬`, `∃!` as
   HOL does (as *definitions*, via `new_basic_definition`, each yielding a defining theorem), and add
   the three axioms: **extensionality (ETA_AX)**, **choice (SELECT_AX / Hilbert ε)**, **infinity
   (INFINITY_AX)**. Definitions and axiom introduction are *also* kernel-gated (they mint `Thm`s /
   extend the constant table), so they live behind the same boundary.
3. **Then derived rules + a minimal tactic layer** — `SYM`, `MP`, `CONJ`, `SPEC`, `GEN`, … built as
   ordinary untrusted functions returning `Thm` *only by calling the primitives*. A first tactic
   combinator (`THEN`, `THENL`, `REPEAT`, a goalstack) is pure library code above the kernel.

**First provable theorem milestone (Increment 3):** `⊢ T` and `⊢ (λx. x) y = y` (a BETA + REFL
composition), then `⊢ p ⇒ p`. Small, but it exercises every core rule and proves the boundary holds
under real use.

---

## 3. THE SOUNDNESS BOUNDARY — can Cadenza actually guarantee `Thm` is unforgeable? (charter Q2)

This is make-or-break. If *any* path lets code outside the `hol` module fabricate a `Thm`, the kernel
is worthless. I audited every hole named in the charter against the **live spec** + the source (not
memory). Verdict: **the boundary is currently strong enough for an intra-package kernel**, subject to
ONE load-bearing deployment constraint (§3.0) and two documented caveats to pin adversarially with the
breaker. Findings, each with its spec/source citation:

### 3.0 THE LOAD-BEARING CONSTRAINT — the kernel MUST be a separate linked module ‼️

Every opacity check is gated on `db.is_linked_package()` (`resolve.rs:5009`, `db.rs:2870`). **In a
single-FILE program the namespace is flat and the entire opacity mechanism is a no-op — any type's
constructors are reachable, so a `Thm` would be totally forgeable.** This is not a bug; it is the
intra-package model (opacity is a per-file visibility overlay that only exists when there is more than
one file to draw a boundary between). **Consequence for the kernel:** `Thm`/`Term`/`Hty` and the
inference rules MUST live in their own file/module (`hol`), and any prover *client* MUST be a
*separate* file that `import`s the `hol` module. A kernel-and-client-in-one-file demo has no boundary
and proves nothing about unforgeability. Every verification-corpus soundness case (§5, Increment 1)
MUST therefore be a **multi-file package** (a `lib` module + an entry), exactly as the existing
`11-modules.sexp` abstract-export witnesses already are. This constraint is the first line of the
kernel's README and the first assertion in its gate.

### 3.1 The construction/match boundary — CLOSED ✅

`modules-and-namespaces.md` §"A Type's Handle And Its Constructors Are Independently Visible":
> *"A module that makes a type's handle visible without making a constructor visible MUST render that
> constructor unreachable outside the module — a construction or a match through that constructor in
> another module MUST be a compile-time rejection carrying the machine-readable code for a withheld
> constructor."*

So `(export Thm)` (handle only) makes `Thm.Mk` un-constructable and un-matchable outside `hol`
(**CDZ0214**). Witnessed live in `spec/semantics/11-modules.sexp` ("an abstract type's constructor is
not reachable outside its module" → `CDZ0214`; the companion "used through the module's exported
constructor" → runs). This is the LCF constructor gate, and it already ships.

### 3.2 The strip / equality escape hatches — CLOSED ✅

`type-system.md` §"An Abstract Type's Representation Is Not Observable Across Its Boundary" (lines
178–182):
> *"A built-in structural comparison whose operand is a value of an abstract type … MUST be rejected
> outside the declaring module"* (**CDZ0202**), and *"Stripping an abstract type's name tag to its
> underlying structural value MUST be rejected outside the declaring module."*

So an attacker cannot (i) observe a `Thm`'s hidden representation via `=`/`compare` to reverse-engineer
a forgery, nor (ii) `strip` a look-alike structural value *into* a `Thm`. Witnessed live in
`11-modules.sexp` ("a built-in comparison on an abstract type's value is rejected outside its module" →
`CDZ0202`). The strip half is currently *vacuous* (no strip operator exists yet in the surface) but the
MUST is on the books, so if one lands it is gated. **Pin:** add a verification-corpus case the day a
strip op appears, asserting `strip` on `Thm` outside `hol` is `CDZ0202`.

**⚠️ One open equality edge to verify (does not forge, but observes).** There is a known bug
([[nominal-vs-plain-record-comparison-not-rejected]]): a *nominal RECORD* compared to a same-shape
*plain record* wrongly returns `true` instead of `CDZ0202` — the nominal tag is dropped in that
ad-hoc-nominal value path. This is an *observation* leak, not a value forge, and it is in the
capitalized-head **record** path; a `Thm` declared as a proper `(type Thm (Mk …))` **sum** goes through
the sum-nominal path where nominal-vs-nominal comparison *is* correctly rejected. **Pin:** a
verification-corpus case asserting `=` between a `Thm` and a same-shape plain value outside `hol` is
`CDZ0202` — to prove the sum-`Thm` boundary is not subject to the record-path leak (and to catch it if a
future refactor routes `Thm` through the leaky path).

### 3.3 Re-declaration forgery ("forge by declaring my own `Thm`") — CLOSED ✅

A nominal/abstract type's identity is its *declaration*, not its shape
(`type-system.md` §Nominal). The opaque-types work's Increment O1 made **type + constructor resolution
file-scoped**: a sibling file's `(type Thm …)` is invisible unless imported, and two same-named
`(type Thm …)` in different files are *distinct types*, not interchangeable — the opaque-types doc
explicitly migrated a "forge-by-re-declare" corpus case to the import form. So an attacker declaring
their own `(type Thm (Mk …))` gets a *different* `Thm` that the `hol`-typed checker functions will not
accept. **Pin:** a corpus case — a second module re-declaring `Thm` and trying to pass its value where
`hol.Thm` is expected → type error, not acceptance.

### 3.4 The metaprogramming / reflection forge-hole — CLOSED ✅ (this was the scariest one)

Could `eval`/`quote` fabricate a `Thm` by *quoting* a `(Thm.Mk …)` form and evaluating it outside
`hol`? **No** — and the reason is decisive: `eval` **re-resolves its reconstructed source in the eval's
enclosing scope** (`spec/semantics/12-metaprogramming.sexp`, the eval-desugar cases: an unquote/eval
form "resolv[es] in the eval's enclosing scope"; an unbound name in a spliced form is the *ordinary*
unbound-name rejection **CDZ0101**, not silently quoted). Therefore `(eval (quote (Thm.Mk …)))`
evaluated in a module that did not import `Thm.Mk` hits the *same* resolve-time visibility gate as
hand-written code — it is `CDZ0214`/`CDZ0101`, never a forged value. Quote produces inert data; it is
`eval` that would run it, and `eval` gets no privileged scope. **This is why (c) can be sugar but never
a trust path:** reflection routes through the same name-resolution boundary as source. **Pins (with the
breaker):** (i) `(eval (quote (Thm.Mk <forged sequent>)))` outside `hol` → `CDZ0214`/`CDZ0101`;
(ii) a `hol"…"` tagged-template (if built) can only *call exported kernel rules*, and a template that
emits a `Thm.Mk` splice is rejected the same way.

### 3.5 The `decode` / codec forge-hole — CLOSED ✅ (by totality, and by not exporting a `Thm` decoder)

Two layers. First, the language's byte `decode` over *external* bytes is **total — it yields a typed
failure (`Err`), never a trap and never an unchecked value** (`type-system.md` §"A payload decode that
does not match its schema MUST yield a typed failure result rather than a trap"; the `Ast`/byte codec
totality is witnessed across `12-metaprogramming.sexp` decode cases and `10-bytes.sexp`). Second, and
more fundamentally: **the kernel simply does not export a `bytes → Thm` decoder.** A `Thm` has no
public deserialization surface; there is no generic "cast these bytes to type `T`" primitive in the
language. So codec is a non-path by construction. **Pin:** assert there is no exported `Thm`
constructor of any decode/cast shape (a structural test over the `hol` module's public surface).

### 3.6 The raw-heap / runtime-cast forge-hole — CLOSED ✅ (no such operation exists)

The charter asks "can a raw heap value be cast to `Thm`?" The language has **no** `unsafe`
reinterpret-cast, no `transmute`, no way to hand the runtime a heap word and assert its type — a value's
type is a compile-time fact and abstract types are erased to their structural rep *with no runtime
handle* (opaque-types doc §"compile-time only"). There is no surface through which untrusted code names
a heap layout and claims it is a `Thm`. Coordinate with **v-runtime** to confirm no debug/testing
intrinsic (a raw `heap.at`/`unsafe-coerce`) leaks one; none is in the spec today. **Pin:** a note to
v-runtime + a standing corpus assertion.

### 3.7 The host / effect boundary — CAVEAT ⚠️ (the one genuinely open edge)

Cross-*component* interop turns an opaque value into a WIT **resource handle**
(opaque-types doc §"Cross-component (future)"; `closures-across-host-boundary`). Within one package
(the intended kernel deployment) a `Thm` never crosses a component boundary, so this is a non-issue for
Increment 0. **But** if a `Thm` were ever exported across a component boundary to *untrusted host code*,
the host could in principle mint a handle. **Decision for Increment 0:** the kernel is a
**single-package, intra-component** artifact; `Thm` MUST NOT be exported across a component boundary.
This is a documented deployment constraint, not a hole in the intra-package story. Route to the operator
as a fork (§6) only if cross-component proof exchange is ever wanted — that would need the handle to be
minted *by the exporting (kernel) component*, which preserves soundness, but it is out of scope now.

### 3.8 Summary of the boundary verdict

| Forge vector | Status | Gate |
|---|---|---|
| Single-file program (no boundary at all) | **Constraint** | kernel MUST be a separate linked module (§3.0) |
| Construct / match `Thm` outside `hol` | **Closed** | CDZ0214 (shipping, witnessed) |
| Observe rep via `=`/`compare`; strip into `Thm` | **Closed** | CDZ0202 (compare shipping; strip MUST on-books, vacuous until a strip op lands) |
| Re-declare own `Thm` and pass it | **Closed** | file-scoped nominal identity (O1, shipping) |
| `eval`/`quote` reflection forgery | **Closed** | eval re-resolves in scope → CDZ0214/CDZ0101 |
| `decode`/codec cast | **Closed** | decode total→`Err`; no `Thm` decoder exported |
| Raw-heap / unsafe cast | **Closed** | no such operation exists in the language |
| Host / cross-component handle | **Caveat** | intra-component-only deployment constraint for Inc 0 |

**Bottom line: the LCF discipline is soundly expressible in Cadenza today.** The trust base is (the
opaque-type boundary as specified) + (the ~200-line kernel module) + (the intra-component deployment
constraint). Every closed row above deserves a *standing verification-corpus case* so a future language
change cannot silently reopen it — that gate coverage IS the vertical's core protective deliverable
(per the role charter), and is where several early increments go.

---

## 4. Surface + ergonomics (charter Q4) — designed AFTER the kernel works

Do not design ergonomics before the kernel exists; sketch only. Options, cheapest first:

- **Plain `Thm`-producing API (Increment 0–3).** Proofs are ordinary Cadenza expressions composing
  kernel + derived rules: `let th = mp (spec x all_p) p_holds`. No new syntax. This is enough to prove
  the mechanism and the first theorems.
- **Tactic combinators as a library (later).** `THEN`/`THENL`/`REPEAT`/`ORELSE` + a goalstack, all
  untrusted functions above the kernel — HOL-Light's `tactics.ml` ported. Pure library work.
- **A `hol"∀x. x = x"` tagged-template term DSL (later, leans on v-metaprogramming).** The
  tagged-template mechanism (SHIPPED: a `tag"…"` glued literal → a compile-time `List String -> List
  Ast -> Ast` macro) could parse HOL concrete syntax into `Term`-*building* calls. **Crucially this
  builds `Term`s (data), not `Thm`s** — so it stays outside the trust boundary (§3.4). Term parsing is
  ergonomics; theoremhood still only comes from kernel calls.
- **A `@theorem`/`@verify` annotation (Increment (b), separate design).** Compiler-checked program
  properties — the big one, deferred.

---

## 5. Increment plan (each code increment gated: corpus + `cargo test -p rcdzc` + `cargo xtask check`)

- **Increment 0 — THIS DOC.** The design, the (a)-first decision, and the soundness-boundary audit.
  Route the forks (§6). *(This tick.)*
- **Increment 1 — the trust-boundary gate FIRST (spec-first, before any kernel code).** Add a
  verification-corpus file (`spec/semantics/NN-verification.sexp` or fold into `11-modules.sexp`/
  `07-type-system.sexp`) pinning the §3 forge-vectors *specifically for an abstract `Thm`-shaped type*:
  construct-outside → CDZ0214; compare-outside → CDZ0202; `eval`-a-quoted-private-ctor → CDZ0214/CDZ0101;
  re-declare-and-pass → type error. These pin the boundary the kernel will rely on, *independent of the
  kernel*, so the trust base is gate-protected before a line of `Thm` code exists. Coordinate with
  v-inference / v-metaprogramming / v-runtime (§6 notes).
- **Increment 2 — the kernel skeleton.** The `hol` module: `Hty`, `Term`, `Thm` abstract types; term
  well-typedness checker (inside the module); `refl`, `assume`, `beta` (the three "leaf" rules that
  need no prior `Thm`). Export handles + these rules only. rcdzc unit test: a `refl` `Thm` is produced;
  a construction of `Thm.Mk` in a second module is CDZ0214.
- **Increment 3 — the full primitive rule set + first theorems.** `trans`, `mk_comb`, `abs`, `eq_mp`,
  `deduct_antisym`, `inst`, `inst_type`. Prove `⊢ (λx. x) y = y` and `⊢ p ⇒ p` end-to-end through the
  real runtime (a wasmtime run, per the gate). Adversarial "try to forge a `Thm`" pins WITH THE BREAKER.
- **Increment 4 — logical constants + the three axioms** (`new_basic_definition`, ETA/SELECT/INFINITY),
  kernel-gated. Derived rules (`SYM`, `MP`, `CONJ`, `SPEC`, `GEN`).
- **Increment 5 — a minimal tactic layer** (goalstack + `THEN`/`REPEAT`), pure library.
- **Increment 6+ — ergonomics** (`hol"…"` term DSL) and then scope Increment (b) (program verification)
  as its own design.

**Known language gaps the kernel will re-hit (from the spike — REPORT/FIX, don't work around, per the
port ethos).** The 2026-07-08 spike surfaced four seed gaps, now captured as corpus cases; expect to
meet them again in Increments 2–3 and treat each as a language finding to file, not paper over:
1. **Runtime compound `=` (a heap walk) is not emitted** — `type-eq`/`term-eq` on two *runtime* (non-
   constant) `Term`s declines; folds only when ≥1 operand is a literal. The spike's workaround was a
   hand-written recursive comparator. The kernel needs recursive structural equality on `Term`/`Hty`,
   so this is on the critical path — coordinate with v-runtime/v-inference (heap-`=` is a known
   unrealized op) and file a `Thm`-shaped repro if it still declines.
2. **Payload-bound compound shape is recovered only inside its match arm** — a `concl : Thm -> Term`
   accessor that returns a payload compound through a bare fn return can mis-reject; consume the payload
   inline in the arm (the transferable lesson `spec/learnings/2026-07-08-…-match-arm.md`). This shapes
   how the kernel's `dest_thm`/`dest_eq` accessors are written.
3. Nested ctor pattern inside a tuple slot of a payload declines; 4. a helper-returned payload compound
   + `tuple.N` wrongly rejects (a decline-don't-miscompile *violation* the spike recorded as a real
   finding). Re-verify both against current trunk when the kernel exercises them.

**Standing deliverable (every tick, per the role charter):** grow the verification-corpus so no peer's
change can silently reopen a §3 forge-vector. An un-gated soundness invariant is a bug in my own
coverage.

---

## 6. Forks routed to the concierge (→ operator)

These are genuine design decisions above my pay grade; I'll `ask` the concierge with these options and
keep working (kernel design does not block on them):

1. **Increment-0 confirmation.** Proposed: (a) LCF-kernel-as-library first, (b)/(c) later. Confirm or
   redirect. *(My strong recommendation: (a).)*
2. **HOL fragment scope + fidelity.** Port HOL-Light's `fusion.ml` primitive set 1:1 (max fidelity,
   max reuse of a 20-year-trusted design), or a trimmed intuitionistic/equational core first? Proposed:
   1:1 fusion, equational core first, axioms in Increment 4.
3. **Cross-component proof exchange (§3.7).** Is exchanging a `Thm` across a component boundary ever a
   goal? If yes it needs the kernel component to mint the resource handle (sound, but new work). Proposed:
   **no** for now — intra-component-only, documented constraint.
4. **Where the verification corpus lives.** A new `spec/semantics/NN-verification.sexp`, or fold the
   `Thm`-specific soundness pins into `11-modules.sexp`/`07-type-system.sexp`? Proposed: a new file
   (isolates a hot-file collision risk; signals the vertical's territory).

## 7. Coordination notes to send (this/next tick)

- **v-inference** — the kernel leans on nominal/abstract type identity being unforgeable (§3.3) and on
  no lowercase-type-var / inference path admitting a structural `Thm`. Confirm no inference edge treats
  a look-alike structural value as `Thm`.
- **v-metaprogramming** — confirm the §3.4 reflection analysis (eval re-resolves in enclosing scope,
  private ctor → CDZ0214/CDZ0101); flag if any eval/quote path could bind a private constructor.
- **v-runtime** — confirm no raw-heap / unsafe-coerce / debug intrinsic can mint an abstract value
  (§3.6).
- **v-guide** — once Increment 3 proves a first theorem, this is a flagship "look what Cadenza can do"
  story (like CAD); hand the guide the runnable first-theorem example.
- **breaker** — from Increment 3 on, standing "try to forge a `Thm`" adversarial cases against every §3
  vector.

---

## References
- [DESIGN-opaque-types-abstract-constructor-rcdzc.md](DESIGN-opaque-types-abstract-constructor-rcdzc.md)
  — the abstract-data-type feature this kernel's trust rests on (COMPLETE, shipping).
- `spec/capabilities/modules-and-namespaces.md` §A Type's Handle And Its Constructors Are Independently
  Visible; §Visibility Is Explicit.
- `spec/capabilities/type-system.md` §An Abstract Type's Representation Is Not Observable Across Its
  Boundary; §Nominal Is An Orthogonal Modifier; §The Abstract Syntax Tree Is An Ordinary Sum Type.
- `spec/semantics/11-modules.sexp` (abstract-export witnesses, CDZ0214/CDZ0202); `07-type-system.sexp`;
  `12-metaprogramming.sexp` (eval re-resolves in scope; decode totality); `10-bytes.sexp`.
- Source: `resolve.rs` (`withheld_ctor_reject` → CDZ0214, `is_linked_package` gate); `db.rs`
  (`is_abstract_type_at`, `file_scoped_type`/`file_scoped_variant_ctor`); `link.rs` (`CtorVis`).
- Memory: [[hol-light-kernel-spike-2026-07-08]] (the working prototype + surfaced gaps);
  [[opaque-types-workstream]] (the trust-boundary feature's landing log + the file-scoped-identity
  finding); [[nominal-vs-plain-record-comparison-not-rejected]] (the open equality-observation edge).
- HOL-Light `fusion.ml` (the primitive inference rules + axioms this fragment ports).
