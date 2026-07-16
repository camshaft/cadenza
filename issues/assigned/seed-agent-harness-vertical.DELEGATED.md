# Vertical charter: a native Cadenza agent harness / runtime (off Claude Code, Bedrock-direct)

**Operator directive (2026-07-16, verbatim intent):** "We also need to get another vertical to own
building an agent harness using Cadenza and getting away from using Claude Code and calling Bedrock
directly. And then the whole agent runtime would be self-modifying and have evolvable toolchains. I want
to use Cedar for permissions where we can grant agents to work on behalf of users and stuff. A lot of
the ideas I've captured in here: https://github.com/camshaft/hivemind"

## Your mandate
Own a NEW standing vertical: build an **agent harness / runtime written in Cadenza** that replaces
Claude Code as the agent loop. The end state: agents run on a Cadenza-authored runtime that calls
**Amazon Bedrock directly** (not via the Claude Code CLI), is **self-modifying**, has **evolvable
toolchains**, and uses **Cedar** for permissions (including on-behalf-of-a-user delegation). This is the
dogfood/flagship of the whole language — the fleet building itself IN Cadenza — and, like the
compiler-ml port, it's a REAL stress test: REPORT/FIX language gaps you hit, don't work around them.

## Context — read the operator's captured thinking FIRST
The operator has a detailed idea repo at **github.com/camshaft/hivemind** (a reference clone is at
`/tmp/hivemind-ref` this session; `gh repo clone camshaft/hivemind` otherwise). Read VISION.md,
ARCHITECTURE.md, DECISIONS.md. Hivemind is the BROADER vision — a hyperconnected, self-organizing,
self-improving pool of agents over an event-sourced log (memory + nervous system + desired-state
scheduler). It's currently a RUST implementation. YOUR vertical is narrower and foundational: the
**agent RUNTIME/HARNESS** that an individual agent runs on — the piece hivemind's agents would execute.
Hivemind is where this plugs in (the coordination substrate); your job is the agent loop itself, in
Cadenza. Understand the whole so your harness fits it, but don't try to build all of hivemind.

## The pieces (scope + sequence — this is design-first, huge, incremental)
1. **Bedrock-direct model calls.** The harness calls Bedrock's model API (Converse / InvokeModel)
   directly instead of shelling out to Claude Code. In Cadenza this means a peer/host binding to the
   Bedrock API (the cross-component-interop / `(bind …)` machinery v-peer-linking built, or a host
   capability). FIRST QUESTION: can Cadenza today make an authenticated HTTPS/AWS-SigV4 call to Bedrock
   — what host/peer surface exists, and what's missing? That gap analysis is likely Increment 0.
2. **The agent loop in Cadenza.** The read-inbox → build-context → call-model → parse-tool-calls →
   execute-tools → loop cycle, authored in Cadenza. Model tool-calls as Cadenza values (the
   metaprogramming/`Ast` + effects work is relevant — tool dispatch is an effect).
3. **Cedar permissions + on-behalf-of.** Use Cedar (the `cedar-policy` crate is ALREADY a dependency in
   this tree — grep it) to authorize what an agent may do, and to model DELEGATION: a user grants an
   agent authority to act on their behalf, scoped by Cedar policy. Every tool invocation / resource
   access is a Cedar authorization decision. Design the principal/action/resource/context model for
   "agent acting for user."
4. **Self-modifying runtime + evolvable toolchains.** The runtime can modify its own code/config and
   grow/replace its tools at runtime — agents evolve their own toolchain. This is the most speculative;
   it leans hard on Cadenza's metaprogramming (quote/eval/`Ast`) + the load-time expansion machinery.
   Design LAST, after the loop + Bedrock + Cedar basics work. The self-verification vertical
   (v-verification, HOL-Light-style) may be a natural partner here — a self-modifying agent that can
   PROVE properties of its own next state is the safe version of self-modification.

## How to work
- **Increment 0 = a DESIGN doc** (`design/agent-harness.md`): the end-state architecture, how it maps
  onto hivemind, the Bedrock-binding gap analysis (what Cadenza host/peer surface exists vs is needed),
  the Cedar principal/action/resource model for on-behalf-of, and the increment sequence. Route the real
  forks to the concierge (→ operator) — this is design-heavy and the operator has strong captured views.
- Coordinate EARLY + widely — this touches nearly everything: v-peer-linking / cross-component-interop
  (the Bedrock + tool bindings), v-effects (tool dispatch as effects; the compiler needs effects working
  — this is another forcing consumer), v-metaprogramming (self-modification via quote/eval), v-runtime
  (host capabilities), v-verification (provably-safe self-modification), v-cad/v-guide (a showcase
  surface eventually). Send notes when your design leans on their guarantees.
- This is the ULTIMATE dogfood: the fleet's own harness, in Cadenza. Language gaps you find are the
  point — file them (REPORT/FIX, not work-around), same ethos as the compiler-ml port.

## Not urgent, do it right — depth over speed
The operator said "get a vertical to OWN building" this — a standing, long-horizon charter, not a sprint.
A crisp Increment-0 design that nails the Bedrock-binding reality + the Cedar on-behalf-of model is worth
more than premature loop code. Strong owner: each tick advance the design or an increment; if idle,
deepen the hivemind alignment or the Bedrock/Cedar gap analysis. Note: Cedar is already vendored here
(cedar-policy in Cargo) — a real advantage; lean on it.
