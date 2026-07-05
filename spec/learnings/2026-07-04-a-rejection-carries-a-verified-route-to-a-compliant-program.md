# A rejection carries a verified route to a compliant program, not just a reason

*2026-07-04*

**What happened.** The language's top-priority purpose is sharpened from "diagnostics are
machine-*actionable*" to its actual goal: **empower an agent to produce a safe program with no human
feedback.** Cadenza already guarantees *safety by construction* — determinism, capability-safety,
bounded termination, static typing are the floor (overview §1), so a program that compiles cannot be
unsafe in those dimensions. The missing half is the **convergence of the unattended fix loop**: today a
rejection tells the agent what is *wrong* (a stable `code`, a `span`, the `rule` — `coded-span-record.md`)
but not what to *do*, so the loop is a slow, guess-the-fix, one-error-at-a-time climb. The commitment:
**a rejection MUST carry a machine-applicable route to a compliant program**, and — the part no existing
compiler can match — **that route is verified, not merely suggested.**

**Why Cadenza can be strictly stronger than a suggestion.** Rust's structured suggestions
(`rustc` JSON: `suggested_replacement` + an `applicability` marker) are *text-span replacements the tool
promises are probably right*. Two properties already in the tree let Cadenza do better:
- **A fix is a structural AST edit, not a text patch.** The canonical form is the AST and text is a
  projection ([[2026-07-03-one-accessor-modules-are-records]], `ast-encoding.md`), so a proposed fix is
  an *unambiguous* structural edit — no reparsing, no whitespace guessing.
- **The compiler can validate its own suggestion.** `agent-authoring.md` §"Structural Edits Preserve
  Well-Formedness Or Report" already requires a structural edit to either yield a well-formed program or
  report why. So the compiler can **apply its own proposed fix and recompile** — "machine-applicable"
  becomes a *verified property* ("this edit recompiles clean and clears this diagnostic"), a guarantee
  Rust structurally cannot make.

**Verified where determinable, else marked (the operator's call).** Many of Cadenza's rejection codes
have a **mechanically derivable** fix, because the compiler already computes the information the fix
needs:
- **`CDZ0401` undeclared capability** — `capabilities-and-effects.md` requires the compiler to
  *determine* required capabilities from the operations reached, so the exact manifest delta is *known*,
  not guessed. Under the effects model this is "add the escaping effect to the manifest row"
  ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]).
- **`CDZ0210` non-exhaustive match** — the uncovered variants are known from the scrutinee's variant
  set, so "add these arms" is derivable.
- **`CDZ0301` numeric mismatch** — both operand types are known, so the explicit conversion to insert is
  derivable ([[2026-07-04-traits-are-dictionaries-scoped-not-coherent]] notes `+`-resolution shares this).
- **`CDZ0202` nominal boundary** — the required tag-strip is known
  ([[2026-07-04-nominal-is-orthogonal-tag-over-structural-types]]).
For these the compiler **applies + recompiles** the fix and marks it **VERIFIED**. Where the repair is a
genuine heuristic (which of several edits the author *meant*), the fix carries a Rust-style
**applicability marker** (maybe-incorrect / has-placeholders) instead, so the agent gets maximum help
where repair is determinable and honest uncertainty where it is not — and can branch on the marker.

**The determinism constraint.** A proposed fix (and whether it verified) MUST be a deterministic
function of the source, like every other compiler output (Constitution II; `diagnostics.md`
§Determinism), so the fix a fix-application produces does not vary between builds. The
verify-by-recompile step is itself pure compile-time work, bounded by the resource measure.

**This is a constitution-level commitment, so it amends XI.** Principle XI today requires only
*code + span + rule* — branchable, but weaker than the language's stated reason to exist. The autonomy
loop is not a capability detail; it is the invariant the whole architecture serves. So XI is
**strengthened** (the operator approved amending it): a diagnostic MUST carry a machine-applicable route
to a compliant program, verified where the repair is determinable. This *adds* obligations and weakens
no governance floor (determinism and capability-safety are untouched), so it needs no human-approval
floor beyond the operator's decision here; it is recorded with rationale per the Amendment Discipline as
**Amendment 0.5.0**.

**The requirements it drives.** `constitution.md` §XI ("Diagnostics Are Machine-Actionable") gains a
requirement that every diagnostic reporting a rejection carry a machine-applicable fix — a structural
edit toward a compliant program — and that a fix whose application the compiler has confirmed recompiles
clean and clears the diagnostic be marked verified (Amendment 0.5.0, with the ratified/amended line and
an amendment note). `spec/capabilities/diagnostics.md` gains §"A Diagnostic Carries A Fix" (a rejection
diagnostic carries a proposed structural edit; a determinable fix is verified by application-and-
recompile and marked so; a heuristic fix carries an applicability marker). `spec/capabilities/agent-authoring.md`
§"Machine-Readable Output" is extended so the fix and its verified/applicability status are
machine-readable. `options/diagnostics-schema/coded-span-record.md` gains `fix` and `fix_status` fields
on the diagnostic record. Composes with [[2026-07-04-diagnosis-is-complete-and-cascade-aware]] (report
*all* problems, each with its route) and [[2026-07-04-type-errors-report-the-minimal-conflict]] (the
route for a type error names both conflicting sites).
