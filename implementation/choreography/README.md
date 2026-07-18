# The Cadenza choreographic-protocols library

Choreographic programming for Cadenza, written in Cadenza (ML surface): define a distributed protocol
**once** as a single global artifact, then **project** ("shred") it into one local program-and-type per
actor at compile time — so every endpoint is correct-by-construction (no unexpected messages, no
deadlock), because all endpoints descend from one consistent global source.

Extracted into its own **sidecar package** (operator ruling, 2026-07-18) so the compiler-ml package stays
laser-focused on the integer spine — choreography must not distract that initiative, and it should work
**today against the rust compiler (rcdzc / `cdz`)**, not wait on the self-hosted compiler to mature. Like
`iterators/`, `cad/`, `agent-harness/`, it is a self-contained sibling package the rust `cdz` builds. It
is prelude-only: it vendors a minimal homoiconic `Ast` (`src/ast.cdz`, `src/ast-eq.cdz`) rather than
depending on the compiler pipeline, so it has **no dependency on compiler-ml**.

Run the suite with `cdz test implementation/choreography` (over this directory's `Project.cdz`).

## The pipeline (a protocol, top to bottom)

A global protocol is a `Chor` value (`chor.cdz`). It flows through the passes, each its own module:

1. **`chor.cdz`** — the global-protocol AST (`Chor = Done | Comm(from,to,label) | Seq | Choice(chooser,brs)
   | Branch(label,cont) | Rec(var,body) | Var(var)`), a canonical round-trip through the vendored `Ast`
   (`to-ast`/`from-ast`, total), and a well-formedness checker (`wf`: declared distinct roles, branches
   only inside choices, bound rec-vars).
2. **`chor-project.cdz`** — the **projectability / knowledge-of-choice** analysis: can the protocol be
   shredded per-actor? At a `Choice(p, …)`, every non-chooser role that behaves differently across branches
   must be *told* which branch was taken (the strict MPST selection rule). `projectable` / `unprojectable-role`
   (names the offending role) — the reject path, the soundness core.
3. **`chor-local.cdz`** — **projection** to a per-role local (session) type (`project : Chor -> Role ->
   Local`), a continuation-passing fold: `Comm` → send/recv/skip, `Choice` → internal/external/collapse,
   `Rec` → recursive local type. Assumes projectability (guarded by `chor-project`).
4. **`chor-codegen.cdz`** — **code generation** from a local type to that actor's program `Ast`
   (`codegen : Local -> Ast`) in terms of `Comm` ops — the author writes no endpoint code; the compiler
   generates every actor.
5. **`chor-run.cdz`** — **execution** of the generated actors over a destination-aware mock `Comm` medium
   (`run` + `deliver`), proving the projected value flow end-to-end (a value emitted by one actor's
   generated code arrives at another's).
6. **`chor-safety.cdz`** — **deadlock-freedom by construction**: `deadlock-free` checks every send has its
   mirror recv in the recipient's projection (matched I/O — the only deadlock source for a parallel-free AST).
7. **`chor-fleet.cdz`** — the **flagship**: the fleet's own coordination (`FleetIntegrate` — Worker / PrSync
   / Concierge, `MergeRequest → choice{Merged | Reject | Blocked → Ask/Answer}`, guarded recursion) driven
   through every pass on a real protocol.
8. **`chor-consistency.cdz`** — cross-pass coverage pinning that project ↔ codegen ↔ run agree.
9. **`chor-diag.cdz`** — actionable projectability diagnostics (name the role + the selection-message fix).

## Sidecar architecture (the intended shape)

Today the library is a self-contained Cadenza package that the rust `cdz` compiles and tests. The sidecar
program shape — a CLI/tool that reads a global-protocol source, projects it, and emits each actor's
artifact (built via rcdzc) — layers on top of these modules; the projection/codegen/deadlock-free passes
above are the reusable core.
