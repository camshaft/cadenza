# DESIGN: Machine-checked verification — an LCF-style kernel baked into Cadenza

Status: **Increment 0 — design.** All four design forks CONFIRMED by the concierge (§6). Vertical
`v-verification`, subsystem `rcdzc`. Operator directive (2026-07-16, verbatim intent): *"We need to get
a vertical thinking about machine-checked verification. I really like the idea of baking something like
HOL-Light into the language."*

This doc answers the four core design questions from the charter, commits to an Increment-0 shape,
and — most importantly — settles the **soundness-boundary** question the whole feature rests on. **Key
result: the audit surfaced a reproduced, kernel-breaking `eval` forge hole (§3.4) — now FIXED (trunk
`e1506bd7c`) and verified, with regression pins landed (`25-verification.sexp`). Unforgeability holds
for an intra-package kernel (subject to §3.0).** The four design forks were confirmed by the concierge
(§6).

---

## 0. Executive summary — what I'm proposing and why it's cheap

**Increment 0 is candidate (a): an LCF-style kernel as an ORDINARY Cadenza LIBRARY**, not a compiler
feature. A module `hol` declares an **abstract** type `Thm` (a theorem) and `Term`/`Hty` (HOL terms
and types), exporting the type *handles* but **keeping the constructors private**. The only exported
functions that *return* a `Thm` are the primitive inference rules (`refl`, `trans`, `mk_comb`, `abs`,
`beta`, `assume`, `eq_mp`, `deduct_antisym`, plus `inst`/`inst_type` and the three HOL axioms). The
LCF design relies on Cadenza enforcing that **no code outside a module can construct or match an
abstract type's variants** (`CDZ0214`) and **cannot compare or strip one** (`CDZ0202`) — so that
everything above the kernel is untrusted code that can only obtain a `Thm` by calling a kernel rule.

**The key finding of this doc: Cadenza's opaque-type feature — already landed on `spec` and shipping —
is a ready-made LCF trust boundary, once one soundness hole the audit surfaced was closed.** The
abstract-data-type work
([DESIGN-opaque-types-abstract-constructor-rcdzc.md](DESIGN-opaque-types-abstract-constructor-rcdzc.md),
COMPLETE) was built for "a non-empty list / a validated email / a positive Money" — but an *unforgeable
theorem* is the same mechanism at maximum stakes, and running it at that stakes-level immediately
surfaced a soundness gap: `(eval (quote (Thm.Mk …)))` **forged** an abstract value, bypassing `CDZ0214`
(v-metaprogramming found it; I reproduced it on trunk — §3.4). That hole is now **FIXED** (trunk
`e1506bd7c`) and verified, with regression pins landed (`25-verification.sexp`). So the kernel needs
**no further compiler change** for its trust story; the rest is a library exercise plus the forge-hole
audit (§3). This is the vertical working as designed — a real stress test surfaced a real language gap,
which was fixed (REPORT/FIX, per the port ethos). (a) remains the right Increment 0 by a wide margin: it
delivers the trust foundation everything else needs, it is a flagship stress test of the opaque-type
soundness guarantee (it already paid off by catching + closing a kernel-breaking hole), and it is
buildable incrementally with the normal corpus/rcdzc/check gate.

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
memory). Verdict: **the boundary now holds for an intra-package kernel** — all seven forge vectors are
closed (the `eval` one was found OPEN and has since been fixed, §3.4), subject to ONE load-bearing
deployment constraint (§3.0). Findings, each with its spec/source citation:

> **✅ STATUS (2026-07-16): the eval forge hole (§3.4) that blocked unforgeability is FIXED (trunk
> `e1506bd7c`) and verified; regression pins landed in `25-verification.sexp` (Increment 1).** The
> audit process worked as intended: running opaque types at kernel stakes surfaced a real, reproduced,
> kernel-breaking soundness bug (`eval` reaching a module-private constructor); v-metaprogramming owned
> and landed the fix; I verified it and pinned it. Unforgeability now holds for the intra-package kernel.

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

### 3.4 The metaprogramming / reflection forge-hole — FOUND OPEN, now CLOSED ✅ (fixed `e1506bd7c`, 2026-07-16)

**This was the scariest vector — it was a real hole, and it is now fixed.** My initial audit reasoned
that `eval` re-resolves in the enclosing scope so a quoted private constructor would hit the same
visibility gate as source (→ `CDZ0214`). That reasoning was the RIGHT invariant but did NOT hold in the
implementation for a module-private constructor: v-metaprogramming found the counterexample and I
reproduced it against trunk — `eval` was reaching private constructors that the direct path rejected.
The hole and its closure (both verified by me with `cargo xtask gate` two-module probes; verdicts are
actual, not hypothesized):

| Probe (entry imports abstract `Color` + smart-ctor `mk`, NOT the variant ctors) | Before fix | After fix `e1506bd7c` |
|---|---|---|
| DIRECT `(Color.Green)` | `rejected [CDZ0214]` ✅ | `rejected [CDZ0214]` ✅ |
| `(eval (quote (Color.Green)))` | **`value (Green unit)`** 🚨 forged | `rejected [CDZ0214]` ✅ |
| `(eval (quote (match (mk) ((Color.Green) 99) …)))` | **`value 99`** 🚨 destructured | `rejected [CDZ0214]` ✅ |
| `(rank (eval (quote (Color.Red))))` | **`value 1`** 🚨 forged value flowed | `rejected [CDZ0214]` ✅ |
| `(eval (quote (mk)))` [public] | `value 2` ✅ | `value 2` ✅ (no over-reject) |

The bug was comprehensive (construct AND match; the forged value flowed freely where the abstract type
was expected — for a `Thm` a double break: mint a fake theorem AND read the sequent out of a real one),
which is why it was kernel-breaking. **The fix (`e1506bd7c`, "close the eval-forges-abstract-type-
private-ctor SOUNDNESS HOLE") makes an eval-reconstructed constructor reference re-resolve under the
SAME cross-file visibility gate as hand-written code** — so reflection is no longer a privileged door
onto a `Thm`, and eval of a *public* name still works (no over-rejection). Mechanism, as diagnosed by
v-metaprogramming: `quote (Color.Green)` reifies to a `.`-projection AST node; `eval_ast::reconstruct`
previously re-resolved it without re-applying the link-time `AbstractCtor` gate (`resolve.rs`
`withheld_ctor_reject`); the fix reinstates that gate on the reconstruction path.

**Consequence for candidate (c):** reflective proofs-as-`Ast` were an *active* trust hole while this was
open (any importer could `eval`-forge a `Thm` regardless of how proofs are written); with the fix, (c)
returns to being safe sugar — reflection routes through the same name-resolution boundary as source.

**Pins (Increment 1, LANDED):** `25-verification.sexp` cases 5–6 pin `(eval (quote (Thm.MkThm …)))` and
`(eval (quote (match … Proof variants …)))` as `CDZ0214`, so this trust-critical fix can never silently
regress. (A `hol"…"` tagged-template that emits a private-ctor splice is also rejected — it hits
`CDZ0101` first, per v-metaprogramming — pin when the DSL is built.)

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
| `eval`/`quote` reflection forgery | **Closed** ✅ (was open) | found forging private ctors (construct AND match); FIXED `e1506bd7c`; pinned in `25-verification.sexp` (§3.4) |
| `decode`/codec cast | **Closed** | decode total→`Err`; no `Thm` decoder exported |
| Raw-heap / unsafe cast | **Closed** | no such operation exists in the language |
| Host / cross-component handle | **Caveat** | intra-component-only deployment constraint for Inc 0 |

**Bottom line: the LCF discipline is soundly expressible in Cadenza for an intra-package kernel.** All
seven forge vectors are closed and the design (opaque `Thm`, intra-component, separate linked module) is
right. The `eval` hole (§3.4) was a genuine, reproduced, kernel-breaking soundness bug — found by
running opaque types at kernel stakes — and it has been fixed (`e1506bd7c`) and verified. The trust base
is therefore (the opaque-type boundary as specified, with the eval gate now closed) + (the ~200-line
kernel module) + (the intra-component deployment constraint). This is the vertical working as intended —
a real stress test surfaced a real language soundness gap, which was owned and fixed. Every row above
has (or will have, as the `Thm`-shaped ops are built) a *standing verification-corpus case* so a future
change cannot silently reopen it — that gate coverage IS the vertical's core protective deliverable (per
the role charter). **Increment 1 landed the first five soundness pins plus the two eval-forge
regression pins (`25-verification.sexp`).**

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
- **Increment 1 — the trust-boundary gate FIRST (spec-first, before any kernel code). ✅ LANDED
  (`spec/semantics/25-verification.sexp`, 7 cases, all pass).** The verification-corpus file (a new file,
  as the concierge confirmed) pins the §3 forge-vectors *specifically for an abstract `Thm`-shaped type*:
  construct-outside → CDZ0214; use-through-exported-rule+accessor → runs; compare-outside → CDZ0202;
  multi-variant match-outside → CDZ0214; **`eval`-a-quoted-private-ctor → CDZ0214**; **`eval`-a-quoted-
  private-*match* → CDZ0214**; re-declare-and-pass → CDZ0203 (distinct nominal type). These pin the
  boundary the kernel relies on, *independent of the kernel*, so the trust base is gate-protected before
  a line of `Thm` code exists. The two eval pins were the ones that had FAILED (forged) until the §3.4
  fix landed — pinning them now guards the trust-critical fix against regression. **Still tracked for a
  later slice:** the single-variant match diagnostic gap (CDZ0203 vs CDZ0214, queue repro
  `adv-single-variant-abstract-match-wrong-diag-cdz0203-not-cdz0214.sexp`) — pin the single-variant
  match as CDZ0214 when that fix lands.
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

## 6. Forks — ALL CONFIRMED by the concierge (2026-07-16; operator may override on wake)

I `ask`ed the concierge; all four recommendations were confirmed (as sound engineering defaults —
craft calls, not product-taste; the operator can redirect #1/#2 on wake and the concierge will relay):

1. **Increment 0 = (a) LCF-kernel-as-library first, (b)/(c) later.** ✅ CONFIRMED — the trust foundation,
   and it stress-tests opaque types (doubly so given the eval hole surfaced immediately).
2. **HOL fragment: port HOL-Light `fusion.ml` primitive set 1:1, equational core first, axioms in
   Increment 4.** ✅ CONFIRMED — reuse the 20-year-trusted kernel design; don't invent a logic.
3. **Cross-component `Thm` exchange (§3.7): NO — intra-component-only for now.** ✅ CONFIRMED — keeps the
   trust boundary tight; the resource-handle minting is real work with no current consumer.
4. **Verification corpus: a new `spec/semantics/NN-verification.sexp`.** ✅ CONFIRMED — a distinct concern
   deserves its own corpus file.

## 7. Coordination (status 2026-07-16)

- **v-metaprogramming** — ⚡ found the §3.4 eval-forge hole and **LANDED the fix** (`e1506bd7c`,
  concierge-assigned): eval-reconstructed ctor references now re-resolve under the same visibility gate
  as source. I verified it and landed the two eval regression pins in `25-verification.sexp`. ✅ Done.
- **v-inference** — note SENT: confirm no infer/unify edge admits a look-alike structural value as `Thm`
  (§3.3), and that the sum-nominal path is not subject to the record-path equality leak (§3.2). Awaiting.
- **v-runtime** — note SENT: confirm no raw-heap / unsafe-coerce / debug intrinsic can mint an abstract
  value (§3.6). Awaiting.
- **v-agent-harness** — a concrete DOWNSTREAM CONSUMER: their Cadenza-native self-modifying agent wants
  each self-modification to carry a *proof* it preserves a stated invariant (e.g. "the new Cedar policy
  still forbids `tool:delete-prod`"), and proposes a Cadenza-native Cedar evaluator as a provable target
  (Inc-3/4, no action now). **Open interface question they asked:** what proof surface does the kernel
  expose — a checkable `Thm` term, or a tactic script over an `Ast`? *Proposed answer (to shape with
  them later):* a `Thm` value IS the proof term — a self-mod attaches the `Thm` that its post-state
  satisfies the invariant predicate, and the harness's trusted check is just "does this `Thm`'s
  conclusion match the required invariant?" (a single kernel-typed comparison, no tactic replay at check
  time). This is the LCF payoff — checking a proof is trivial and trusted; *finding* it (tactics) is
  untrusted. Revisit when they reach Inc-3.
- **v-guide** — once Increment 3 proves a first theorem, this is a flagship "look what Cadenza can do"
  story (like CAD); hand the guide the runnable first-theorem example.
- **breaker** — from Increment 1 on, standing "try to forge a `Thm`" adversarial cases against every §3
  vector (the eval vector especially, once fixed).

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
