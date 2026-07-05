# Macro phases: definition, invocation, and expansion — and the reader stays fixed

*2026-07-04*

**What happened.** The metaprogramming spec said what a macro *is* (an `Ast`→`Ast`, now `Expr[T]`,
hygienic transformation — [[2026-07-04-macros-are-typed-and-hygienic]]) but never said **how one is
defined, how the compiler tells a macro call from a function call, or when expansion happens relative
to the rest of the tier.** Two decisions close that, plus one deliberate exclusion:

- **A macro binding is distinguished at definition, not by a call-site heuristic.** A macro is
  introduced by a *macro-binding* form, so `(m x)` is a macro use exactly when `m` resolves to a macro
  binding in scope — never by inspecting the spelling or shape of the call. (Consistent with killing
  the earlier dotted-atom spelling heuristic — [[2026-07-03-one-accessor-modules-are-records]].)
- **A minimal two-phase model.** There is a **runtime phase** (phase 0) and a **compile-time phase**
  (phase 1). A macro body, a generic's type-level computation, and a constant all evaluate at phase 1;
  the emitted component is phase 0. This is the smallest sound phase separation — Racket's tower and
  Template Haskell's stage restriction are the rigorous references; Cadenza takes the two-level floor,
  not the full tower, and MAY generalize later.
- **Expansion runs to a fixpoint, before type-checking.** The compiler expands macro uses repeatedly
  until none remain, and only the fully-expanded tree is type-checked and capability-checked
  (`metaprogramming.md` §"Expansion Precedes And Feeds The Core Guarantees"). Expansion is bounded by
  the resource measure so it halts at a defined point (`metaprogramming.md` §"Expansion Terminates").
- **Deliberate exclusion: no reader macros.** Syntax is extended by macros over the canonical `Ast`,
  **never** by user-defined *lexical/reader* extension.

**Why.**
- **Self-hosting forces the phase model to be pinned.** The first Cadenza artifact is a compiler
  ([[2026-07-03-bootstrap-targets-the-compiler-directly]]), which will want compile-time abstractions
  over its own source. "Which definitions are available when a macro runs?" then decides whether that
  is even well-founded. The answer: a macro at phase 1 may use bindings available at phase 1 — its own
  module's definitions and the compile-time-available definitions of modules it imports — resolved at
  the *macro definition's* scope (the module side of hygiene). Cross-module macro use therefore requires
  the macro's module to be compile-time-available to the user, which the module system's deterministic
  initialization order already supports (`modules-and-namespaces.md` §"Initialization Order Is
  Deterministic").
- **Distinguishing macros at definition keeps resolution deterministic.** A call-site heuristic
  ("is this name special?") is exactly the kind of spelling-derived meaning the language has already
  rejected. Binding-based dispatch means the reader/parser stays meaning-free and the compile-time tier
  decides expansion from *bindings*, deterministically (Constitution II/IX).
- **No reader macros is a principled contrast with the LISP inspiration.** LISP allows user-defined
  *reader* syntax (reader macros mutate the lexer). Cadenza deliberately does **not**, because it would
  re-privilege text and drag the reader into the trusted derivation path — against the frozen
  `ast-encoding.md` §"Parsing And Printing Are Not In The Compiler's Trusted Path" and the whole
  homoiconic-decoupled-display decision (`options/code-shape/`,
  [[2026-07-02-multiple-frontends-diluted-one-surface]]). Cadenza takes LISP's homoiconicity and
  code-as-data macros but **not** its reader extensibility: syntax grows at the `Ast` level, where every
  textual syntax converges and none is privileged. Extending *display* syntax is a
  printer/parser concern outside the trusted path, not a program-level macro.

**Consequences.**
- **Expansion is a phase-1 sub-activity of the one compile-time tier**
  ([[2026-07-04-compile-time-evaluation-is-one-tier]]); it shares the tier's purity (empty effect row —
  [[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]) and boundedness, so the phase
  model does not add a second evaluation mechanism.
- **The order within the tier is fixed:** expand-to-fixpoint → then the fully-expanded tree is subject
  to inference/type-checking, generic reduction/monomorphization, and capability checking. A macro
  cannot observe types (they are not yet assigned during expansion) — a deliberate, simplifying
  limitation of the two-phase floor, revisitable only by a later, explicitly-designed generalization.
- **Determinism of expansion order** must be a function of source (which macro use expands first cannot
  depend on iteration order), consistent with Constitution II.

**The requirements it drives.** `spec/capabilities/metaprogramming.md` gains §"A Macro Is Bound And
Invoked By Binding" (macro dispatch is by binding resolution, not a call-site heuristic), §"Macro
Expansion Is A Compile-Time Phase" (two-phase model; a macro body evaluates at phase 1; which
definitions are available at phase 1), and §"Expansion Runs To A Fixpoint Before Type-Checking"
(order within the tier; expansion halts by the resource measure). A new §"Syntax Is Extended By Macros,
Not By The Reader" states the deliberate exclusion of reader macros and points at `ast-encoding.md`
§"Parsing And Printing Are Not In The Compiler's Trusted Path." `spec/capabilities/modules-and-namespaces.md`
is annotated that compile-time (phase-1) availability of an imported module's macros follows the same
explicit-import and deterministic-initialization rules. Composes with
[[2026-07-04-compile-time-evaluation-is-one-tier]] and [[2026-07-04-macros-are-typed-and-hygienic]].
