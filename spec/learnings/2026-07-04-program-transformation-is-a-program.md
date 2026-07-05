# Program transformation is a program: refactoring is a Cadenza component over the AST, not text patching

*2026-07-04*

**What happened.** The tooling that modifies a program is itself made **a program in the language**: an
agent refactors one program by writing (or invoking) a Cadenza program that transforms it. A
transformation consumes a program's canonical representation and produces a new one — it never patches
text. The structural interface's edit operations (`insert`/`replace`/`delete`/`move` in
`options/structural-interface/content-addressed-nodes.md`) are not a bespoke external tool; they are a
**library of Cadenza functions over the `Ast` sum type**, and a refactoring is an ordinary Cadenza
*component* that composes them.

**Why this falls out of commitments already made.**
- **A program is data.** The canonical stored form is the binary AST (`ast-encoding.md`), and the AST is
  an ordinary sum type deconstructible by `match` and constructible by quasiquote
  ([[2026-07-03-types-first-class-in-dynamic-seed]], [[2026-07-03-quasiquote-for-programmatic-ast-construction]]).
  So "transform a program" is just "map an `Ast` value to an `Ast` value."
- **It is the same seam as the compiler.** The compile seam is byte-to-byte —
  `compile : list<u8> -> list<u8>` (binary AST → component — [[2026-07-03-the-compile-seam-is-statically-typed]],
  [[2026-07-04-two-compilers-not-an-interpreter-and-a-compiler]]). A refactoring is the **same shape**:
  `list<u8> -> list<u8>` (binary AST → binary AST). No new kind of artifact — a transformation is a
  strict, deterministic, capability-gated, reproducibly-derived component like any other Cadenza
  program.
- **Its output is checkable by the discipline already required.** `agent-authoring.md` §"Structural
  Edits Preserve Well-Formedness Or Report" already requires a structural edit to either yield a
  well-formed program or a machine-readable rejection. A transformation *program* inherits exactly this:
  its output is a well-formed program or the transformation reports why — so an agent gets a *proof the
  refactor is well-formed*, not a hope that a text patch applied cleanly.

**Why this matters — it closes the flywheel and kills text patching.**
- **The tools that modify programs are programs**, so they are subject to the same static typing,
  determinism, capability-safety, and gates as any Cadenza artifact — the self-regeneration loop
  (overview §15) generalizes from "the compiler rebuilds the compiler" to "programs rewrite programs,"
  with only the seed outside the loop. A refactoring is auditable, reproducible, and re-runnable because
  it is a component, not an ad-hoc script.
- **Text patching is never the transformation mechanism.** Editing text and re-parsing risks whitespace
  ambiguity, partial parses, and re-parsing unrelated code — exactly what the structural interface
  exists to avoid (`agent-authoring.md` §"A Structural Interface Exists"). A transformation operates on
  the tree directly and touches nothing unrelated (`content-addressed-nodes.md`: an edit operates
  without re-parsing code unrelated to its target).
- **A verified fix IS a transformation program.** The compiler proposing a structural edit and
  applying-and-recompiling it to confirm it clears a diagnostic
  ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]) is precisely "a program
  transforming a program" — the fix mechanism and the refactoring mechanism are one thing.

**Prior art.** **Unison** stores programs as their AST in a codebase and manipulates them through a
codebase manager rather than by editing text files — the closest existing system, and content-addressed
like Cadenza ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]] notes the same
lineage for effects). Lisp's code-as-data macros are the ancestor; the difference is that a Cadenza
transformation runs at *program* time over *another* program's data, whereas a macro runs at *compile*
time over its *own* program's data ([[2026-07-04-compile-time-evaluation-is-one-tier]]) — same
substrate, different phase and subject.

**Consequences to hold.**
- **Determinism.** A transformation's output MUST be a deterministic function of its input program (it
  is an ordinary deterministic component), so the same refactor over the same program yields the same
  program on every run.
- **A transformation cannot manufacture authority.** Producing a program that reaches an undeclared
  capability is rejected when *that* program is compiled (Constitution IV) — the transformation cannot
  smuggle authority into its output any more than a macro can
  ([[2026-07-04-macro-phases-and-the-reader-stays-fixed]]).

**The requirements it drives.** `spec/capabilities/agent-authoring.md` gains a §"A Transformation Is A
Program Over The Canonical Representation": the structural read/rewrite interface is realized as Cadenza
functions over the `Ast` type, a program transformation is an ordinary Cadenza component whose input and
output are canonical representations (the same rep→rep seam as `compile`), and its output is subject to
the well-formed-or-machine-readable-rejection rule. `options/structural-interface/content-addressed-nodes.md`
is annotated that the edit operations are Cadenza functions composable into transformation programs, not
an external protocol. This reinforces Constitution X (structural manipulability) and the flywheel
(overview §15) without amending them — it states *how* the structural interface is realized. Composes
with [[2026-07-04-the-compiler-is-a-queryable-oracle]] (a transformation queries facts to decide its
edits) and [[2026-07-04-deterministic-replay-is-the-debugger]].
