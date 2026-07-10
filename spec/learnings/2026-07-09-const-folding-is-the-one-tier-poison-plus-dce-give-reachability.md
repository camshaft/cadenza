# Const folding is the one compile-time tier, and poison plus dead-code elimination give reachability for free

*2026-07-09*

**What happened.** `rcdzc`'s `fold.rs` is the single compile-time evaluator the spec calls "one tier"
([[2026-07-04-compile-time-evaluation-is-one-tier]]), built as a β-reduction engine over `Mir` rather than
an arithmetic peephole. It runs at module scope (so calls can reduce across function boundaries), is pure
and deterministic, and folds each node bottom-up into one of three shapes, **all expressed in `Mir`
itself** (no separate value type):

- **const** — an `Int`/`Bool`/`Unit`, or a `Tuple` of consts;
- **poison** (`Mir::Error(reject)`) — a constant operation that has no value: an overflow, a divide- or
  mod-by-zero, `Int64.min / -1`, an out-of-range shift;
- **dynamic** — everything else, left as runtime code with its children folded.

On this one tier, the constructs that a naive compiler special-cases instead **fold away**: a module is a
nullary function whose body is a tuple of its export values, so `(. m f)` reduces to the field and the
module vanishes; a function value applied to arguments (`Apply(FuncRef, args)`) reduces to a direct
`Call`, β-reducing constant arguments; an intrinsic applied to all-constant arguments reduces via that
op's own `fold_const`; a constructor applied to its payload reduces to a `Sum` value; a non-recursive
callee with all-constant arguments is inlined and folded (recursion is detected by transitive closure over
the call graph and never inlined). Because generic reduction and monomorphization are *the same reduction*
(a type is a value, an instantiation is an `Apply` — see
[[2026-07-04-generics-are-type-valued-parameters]]), they will ride on this tier as more fold arms rather
than as a new pass.

The trap and reachability story is the sharp part. A constant operation that would trap becomes **poison**,
and poison **propagates through strict positions like a value but is dropped in a branch proven
unreachable**: a constant `if` folds only its taken branch and discards the other, poison and all. After
folding, the pipeline walks each function collecting only *reached* poisons — descending into strict
positions (operands, product elements, call arguments, both a `let`'s value and its body, a sum payload, a
match scrutinee) but **deliberately not into `if` branches or match arm bodies**, because a poison there is
a *shielded* runtime trap that stays a runtime `unreachable`. A poison that survives to a reached position
**fails the build** (this is the numeric-only `CDZ0304` generalized to every provably-reached constant
trap), and *every* such poison is collected module-wide and reported, not just the first. Dead-code
elimination therefore falls out of the fold with **no separate reachability analysis** — reachability *is*
which poisons survive folding — and a second, coarser DCE at layout time emits only the functions reachable
from an export. The fold never manufactures a trap the source did not denote nor erases one it did (`MIN %
-1` folds to `0`, never the `MIN / -1` trap; an operation with a runtime operand keeps `select`'s runtime
guard), and its constant-trap conditions mirror `select`'s runtime-trapping sequences exactly.

**Why.** Building the tier as a general β-reducer instead of an arithmetic peephole is what lets sums,
generics, macros, and monomorphization "ride on it" later rather than each needing bespoke machinery — the
same avoid-four-drifting-subsystems argument the one-tier learning made, now realized as one function that
already reduces modules, functions, intrinsics, and constructors through a single substitution path. The
poison-plus-DCE design is the reproduction insight a fresh implementer will otherwise get wrong twice:
first by reaching for a separate dataflow/reachability pass (unnecessary — a value-shaped poison that the
fold already drops at an unreached branch *is* the reachability result), and second by mis-scoping the
trap policy. The policy has a precise seam: a trap the compiler can *prove is reached* is a compile-time
diagnostic ("that's where this language is really going to shine"), while a trap *shielded* by a
constant-false branch, or gated on an operand that stays dynamic, must remain a runtime trap — folding is
licensed to change *when* a computation runs, never *whether* a denoted trap can occur. Getting the descent
set wrong in either direction reintroduces exactly the bugs the language forbids: descend into `if`
branches and you manufacture a compile error for a trap the program shields; skip a strict position and you
ship a component that traps where the source is provably ill-valued.

This composes with, and is the positive dual of, the
[fold-preserves-checks miscompile][[2026-07-07-a-fold-that-eliminates-a-branch-must-not-eliminate-its-type-check]]:
const-folding a conditional to its taken branch is value-preserving but **not** rejection-preserving, so
the checks a dropped subterm would have triggered (its type-check, its scope-check, its trap) are separate
obligations from its evaluation. The reached-poison collection is how the *trap* obligation is honored
independently of the fold; the type-check and scope-check obligations on a dropped branch are honored
*before* folding, in `infer`. The unifying rule — **a meaning-preserving IR-to-IR rewrite preserves both a
subterm's value and its checks, dropping only its evaluation** — is the keystone that a fresh implementer
most needs stated, and it is still learning-only (`SPEC-BACKLOG` item 9).

**The requirement it drove.** Realizes `metaprogramming.md` §"Compile-Time Evaluation Is One Tier" (macro
expansion, generic reduction, monomorphization, and constant folding are the same mechanism) and §"…Is
Pure," and `numeric-model.md` §"A Constant Operation With No Value Is Rejected At Compile Time" (the
`CDZ0304` trap-preservation-both-directions rule). The reproduction content **not yet folded**, for the
architecture reference doc and a candidate `compiler-pipeline.md` requirement: (1) the general
**meaning-preserving-rewrite** rule — a Core→Core transform preserves value *and* checks (traps,
type-checks, scope-checks), eliminating only evaluation — of which the numeric `CDZ0304` line is one
instance; (2) **reachability via poison + DCE** — a reached provable trap fails the build while a shielded
one stays a runtime trap, and dead code (unreached poison, and functions unreachable from an export) is
eliminated as a consequence of the fold rather than by a separate analysis; and (3) the general
provable-trap policy *beyond* integer arithmetic (e.g. requiring the value of a provably-absent optional)
remains the open decision recorded as ask-09 and `SPEC-BACKLOG` item 9. That modules, function values,
intrinsics, and constructors *leave no runtime trace* once fully reduced is architecture, not language
semantics, and belongs in the prescriptive doc.
