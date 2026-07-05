# The compiler is a queryable oracle: ask it any fact about a program, never instrument to learn it

*2026-07-04*

**What happened.** The compiler is made a **queryable service over the program-as-data**: an agent asks
the compiler for any static fact about a program — the type of any node, what a name resolves to, the
inferred capability manifest and effect row, the constraints inference solved, the documentation
attached to a definition — and gets a **total, deterministic, machine-readable** answer *without
modifying the program*. The agent never inserts a print, a probe, or a throwaway annotation to discover
a type; it queries the compiler that already knows.

**Why this is a generalization of what the spec already requires.**
- **The compiler already computes and exposes the facts.** `agent-authoring.md` §"Machine-Readable
  Output" already requires the compiler expose its diagnostics, **the types it inferred**, and **the
  capability manifest it produced** in machine-readable form. The gap is that these are framed as
  *outputs of a compile*, not as a **query surface** an agent drives with a question ("what is the type
  of *this* node?") and gets a scoped answer.
- **Tooling is already the one compiler, incremental=batch, total over incomplete source.**
  `tooling-and-lsp.md` already requires that a type/definition/diagnostic the tooling reports *agree
  with the compiler and the executable semantics* (not a second implementation), that an incremental
  result *equal* a full compilation's, and that a query over source that does not fully parse *return a
  defined partial result rather than fail*. That is exactly a query oracle's contract — it just needs to
  be named as a first-class, program-callable surface rather than an editor convenience.
- **The structural interface already addresses any node deterministically.** `content-addressed-nodes.md`
  gives every node a path address and a content-derived id, and requires every query result be a
  deterministic function of the representation. So "the type of *this* node" has a well-defined,
  reproducible referent to ask about.

**Why the framing inverts the prior art.** A Language Server (LSP) exists to serve a *human's editor* —
hover, completion, go-to-def rendered for a person. Cadenza's query surface serves **a program** (the
agent): it is callable from Cadenza, returns canonical values (not prose tooltips), and — consistent
with [[2026-07-04-program-transformation-is-a-program]] — a query is itself an ordinary function the
compiler exposes over the `Ast`. The agent's write→query→fix loop
([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]) runs entirely against this
surface: it proposes a program, queries the facts it needs, and transforms accordingly, with no human
and no instrumentation in the loop.

**The obligations that make it trustworthy (mostly already latent).**
- **Totality.** A query over any node — including one in incompletely-parsing or ill-typed source — MUST
  return a defined result (a type, an *error type* ⊥ where inference failed —
  [[2026-07-04-diagnosis-is-complete-and-cascade-aware]], or an explicit "unknown here" answer), never
  an opaque failure or a crash (`tooling-and-lsp.md` §"Queries Over Incomplete Source Are Total"). The
  agent must be able to ask about a program it is *in the middle of fixing*.
- **Agreement.** A queried fact MUST equal what a full compilation determines — the query oracle is a
  view onto the one compiler and the one executable semantics, never a second analysis that could drift
  ([[2026-07-02-parallel-semantics-drifted]], Constitution IX).
- **Determinism.** Every query answer MUST be a deterministic function of the source, so the agent's
  loop is reproducible (Constitution II).
- **Erasure boundary.** Types and effects are queryable **statically**; they are still erased from the
  component ([[2026-07-04-static-typing-is-mandatory-post-pivot]]). The oracle answers *about* the
  program at compile time; it does not add runtime reflection.

**The requirements it drives.** `spec/capabilities/tooling-and-lsp.md` (or a renamed
`spec/capabilities/tooling-and-queries.md`) gains a §"The Compiler Exposes A Query Surface": an agent
MAY query the static type of any addressed node, the resolution of any name, the inferred manifest and
effect row, the solved constraints, and the documentation of a definition; each answer is machine-
readable, total, deterministic, and agrees with a full compilation. `spec/capabilities/agent-authoring.md`
§"Machine-Readable Output" is extended from "the compiler exposes its outputs" to "the compiler answers
scoped queries about any node." The query surface is itself Cadenza-callable, consistent with
[[2026-07-04-program-transformation-is-a-program]]. Composes with
[[2026-07-04-deterministic-replay-is-the-debugger]] (runtime facts by replay; static facts by query).
