# Design — choreographic protocols: define the interaction once, project per-actor at compile time

**Author:** design-choreography (interactive design agent, operator-directed 2026-07-17, relayed via
concierge→Slack). **Audience:** the operator (shaping this live); a future `vertical` owner
(area=`rcdzc` or `compiler-ml`); v-effects (comms-as-effects), v-metaprogramming (projection as a
compile-time `Ast→Ast` pass), v-inference (session/local types as rows), v-verification (machine-checked
projection soundness).
**Status:** 🟢 **ADOPTED — paradigm decided by the operator (2026-07-17): FULL CHOREOGRAPHIC PROGRAMMING
(option (b)).** The north star is **one global program** with located values + explicit communications;
the compiler **endpoint-projects it to GENERATE each actor's executable code** — the author writes NO
per-endpoint code, there is literally one artifact. This is the most ambitious / highest-differentiation
path (over pure-MPST-type-level (a) and the hybrid (c), which are retained only as *considered
alternatives*, §4.1). Written in the house style of `DESIGN-tagged-template-macros.md` and
`DESIGN-agent-runtime-vision.md` (vision + increments + seams + gate + open decisions with a chosen
default). **Queued for a vertical** (`corpus-bugfix`, area=`compiler-ml`) to build top-to-bottom. Everything
in §2 (seams), §3.1 (the projection algorithm), §5 (guarantees), §6.1 (the fleet flagship), and §7 (seams)
is final; the choreographic-programming surface is §4.

> **The operator's directive (verbatim intent):** "A way to model distributed systems where the protocol
> is defined in a single place but we're able to shred it up into one per actor at compile time. Research
> the existing work. Then make a proposal." Depth and ambition over speed.

---

## 0. The one-sentence vision

**A distributed protocol is written ONCE as a single global artifact; the compiler PROJECTS ("shreds")
it into one local program-and-type per actor at compile time, so every endpoint is correct-by-
construction — no unexpected messages, no deadlock — because all endpoints descend from one consistent
global source, and (uniquely for Cadenza) the projection itself can be machine-checked sound by the
verification kernel.**

The bet: projection is fundamentally an `Ast → Ast` transformation over a global-protocol description,
which is *exactly* Cadenza's metaprogramming sweet spot; communication is an *effect*, which is *exactly*
Cadenza's capability boundary; local/session types are *rows*, which the HM system already unifies; and
"the projection is sound" is a *theorem*, which the HOL-Light kernel can already discharge. No mainstream
host of session types has all four. And Cadenza already *runs* a live multiparty protocol — the fleet
(`send`/`merge-request`/`reject`/`assign`/`ask`→`answer`) — so the flagship example writes itself.

---

## 1. Research synthesis — the existing art (know it cold)

The field answers "define once, project per actor" in two paradigms. They differ in *what the single
artifact is* and *what projection produces*.

### 1.1 Multiparty session types (MPST) — the type-level camp
Honda, Yoshida, Carbone (POPL 2008; JACM 2016). The single artifact is a **global type** `G` describing
the whole interaction:

```
G  =  Buyer -> Seller : Title(String) .
      Seller -> Buyer : Quote(Int) .
      Buyer -> { Seller : Accept . Seller -> Shipper : Ship(Addr) . end
               , Seller : Reject . end }
```

**Endpoint projection** `G ↾ r` computes, for each role `r`, a **local type** (a session type) — e.g.
`G ↾ Seller = ?Title(String) . !Quote(Int) . &{ Accept: !Ship-req . end, Reject: end }` (receive Title,
send Quote, offer a choice). You still **write each endpoint's code by hand**; the type checker verifies
your code inhabits its projected local type. Well-typedness against projected locals *implies* the global
guarantees: **communication safety** (no unexpected message / type mismatch), **protocol fidelity** (the
runtime interaction matches `G`), and **deadlock-freedom** (for a single well-formed session).

Key machinery to borrow: the **projection operator** and its partiality (`G ↾ r` is undefined for
un-projectable `G`), and the **merge operator** `⊓` that combines the per-branch projections of a role
*not* involved in a choice (this is where "knowledge of choice" bites — §3).

### 1.2 Choreographic programming — the program-level camp
Choral (Giallorenzo, Montesi, Peressotti — choreographies compiled to Java, one class per role); HasChor
(Shen, Kuper — a Haskell EDSL with *located values* and a freer-monad interpreter); the "choreographic
programming" paradigm (Montesi's thesis, *Choral: object-oriented choreographic programming*). The single
artifact is **one global PROGRAM** with:
- **located values** — `x@Buyer` is a value that physically lives at Buyer; and
- **explicit communications** — `Buyer.title ~> Seller` moves a value from Buyer to Seller (compiles to a
  matched `send` on Buyer + `recv` on Seller).

**Endpoint projection** here projects the *program itself*: it GENERATES each actor's executable code. You
write **no per-endpoint code at all** — there is literally one source artifact and the compiler emits
Buyer's program, Seller's program, Shipper's program. Deadlock-freedom is *structural*: because every
communication is a single `~>` that projects to a matched send/recv pair, sends and receives cannot
mismatch by construction. This is the more radical, higher-"wow" paradigm — and the one that best exploits
Cadenza's metaprogramming.

### 1.3 The pragmatic/industrial and model-checking neighbors
- **Scribble** (Yoshida et al.) — a *protocol description language* (MPST in practice) with a toolchain
  that projects to per-endpoint **finite-state machines** and generates **runtime monitors**. The lesson:
  even without full static endpoint typing, projecting to an FSM + monitoring at runtime catches
  violations. A good fallback tier if full static conformance is too strict initially.
- **Session-type libraries** — Rust (`ferrite`, `sesh`, `session_types`), Scala (`lchannels`, Effpi), F*,
  TypeScript. These encode local types in a host type system (linear/affine channels, duality). The lesson
  + the pain: without a *global* source they only get *binary* duality safety, not multiparty fidelity —
  which is exactly the gap "define once, project" closes.
- **TLA+ / the P language** — a *different angle*: you *model-check* a protocol's state space rather than
  *project* it to code. Complementary, not competing: projection gives you correct code; model-checking
  gives you liveness/safety over the whole state space. Cadenza's verification kernel could later offer
  the model-checking flavor too, but that is not this design.
- **Actyx machine-runner** — local-first / event-sourced state machines per role; relevant to the *runtime
  substrate* question (§Q5) because it maps a projected local behavior onto an append-only event log,
  which is precisely the agent-runtime log substrate.

### 1.4 What Cadenza should borrow vs do differently
| Concern | Borrow from | Cadenza's twist |
|---|---|---|
| The single global artifact | MPST global type / choreographic program | Written in Cadenza surface (s-expr/ML), a first-class `protocol`/`choreography` form; a **built-in AST**, so projection is an ordinary metaprogram. |
| Projection | MPST projection + merge; Choral program projection | A **compile-time `Ast → Ast` pass** on the one-tier evaluator that macros/generics/const-fold already share — not a bespoke compiler phase. |
| Communication | session-type channels | An **effect** (`Comm.send`/`Comm.recv`), so an actor's *capability row* = the messages it is projected to exchange — it *cannot* send a message projection didn't grant. |
| Local types | binary session-type duality | **Rows** in the existing HM system; a local type is a row of `send`/`recv` obligations, checked by unification. |
| Guarantees | MPST meta-theory (on paper) | **Machine-checked** by the HOL-Light kernel: projection soundness + deadlock-freedom as *proved theorems*, not trusted lemmas. |
| Un-projectable protocols | MPST rejects; Scribble monitors | A compile-time **reject with a precise diagnostic** naming the role that lacks knowledge of a choice (default; §3, Q4). |

---

## 2. Why Cadenza is an unusually strong host (the four seams)

1. **Metaprogramming = projection.** `spec/capabilities/metaprogramming.md` gives a built-in `Ast` sum +
   quote/quasiquote/`eval`, all on **one compile-time evaluator** (`fold.rs`) shared with generics and
   const-folding (§*Compile-Time Evaluation Is One Tier*, line 72; §*A Macro Is An Ordinary Compile-Time
   Function Over The AST*, line 74). Projection `G ↾ r` is a Cadenza function `project : Ast -> Role -> Ast`
   evaluated at compile time and spliced — the same mechanism `DESIGN-tagged-template-macros.md` uses for
   embedded DSLs. **The protocol compiler is a Cadenza program**, not new Rust in `rcdzc`. Two spec facts
   make this *more* than plausible:
   - **Quasiquote patterns destructure an AST** (metaprogramming.md line 44–50): `project` can pattern-match
     the global-protocol tree with `` `(,a ~> ,b : ,msg . ,rest)`` arms and reconstruct each role's tree —
     projection is written as *structural recursion over the protocol AST*, the most natural Cadenza idiom.
   - **A typed quote carries the type of the expression it builds** (line 118): an emitted endpoint that is
     ill-typed is rejected **at the projection function**, not downstream at its splice site. So "the
     projected endpoint is well-typed" is enforced by the very mechanism that produces it — projection
     *cannot* emit a type-incoherent actor.
2. **Effects = communication + capability.** `spec/capabilities/capabilities-and-effects.md`: an effect is
   a routing-agnostic contract; an entrypoint delegates it to the host; an unhandled/undelegated effect is
   a compile error. Model `Comm` as an effect and a projected actor's program is a handler/perform over
   `Comm` — the host binds the wire (transport-agnostic, like every other host call). The capability floor
   *becomes* protocol fidelity: an actor physically cannot perform a `send` its projection didn't emit.
3. **Rows = local/session types.** `spec/capabilities/type-system.md` §*Records Are Rows, Open By Default
   Under Inference* (line 86) + §*The Effect Row Is A Row Over The Same Machinery* (line 148): records AND
   effect rows are the *same* open-row machinery, unified by `infer`/`unify`. A projected local type is a
   row of communication obligations (the `send`/`recv` operations a role performs); conformance checking is
   row unification, reusing the existing engine. Line 92 ("the row variable MUST be resolved to a closed
   set before a value crosses a component boundary") is a *gift* here: it forces each actor's message set to
   be **closed and concrete** at the boundary — i.e. an actor's exact protocol alphabet is pinned at
   compile time, which is precisely protocol fidelity at the type level.
4. **The kernel = proof.** `DESIGN-verification-hol-kernel.md` + program-conditions: the HOL-Light LCF
   kernel can state and check "for all well-formed `G` and roles `r`, `project(G, r)` refines `G`" and
   "the parallel composition of all projections is deadlock-free." This is the differentiator: session-type
   soundness is usually a *paper* theorem; here it is a *checked* one.

**This is not speculative — the load-bearing bet is already demonstrated.** The design's central technical
claim is "projection is a recursive fold over a typed sum-AST, written in Cadenza." `implementation/
compiler-ml/` *already runs dozens of exactly-this-shaped folds in Cadenza, with passing `@test`s*:
`constprop.cdz`, `inline.cdz`, `cfold.cdz`, `infer.cdz`, `tycheck.cdz`, `dominators.cdz`, `closure.cdz`,
`anf.cdz` — a whole compiler's worth of recursive traversals over a `type Ex`/`type Term` sum, pattern-
matching each constructor and rebuilding a transformed tree. `project` and `⊓` are the **same shape** over a
protocol AST (§3.1). So the mechanism this design needs is not a new capability to prove out — it is the
mechanism the self-host workstream exercises every day. That collapses the primary implementation risk.

---

## 3. The hard problem: projectability & knowledge of choice

Every system in the literature fights this, and Cadenza must pick a stance (Q4). When one actor makes an
**internal choice** (branch L vs R), every actor whose subsequent behavior *differs* between L and R must
learn which branch was taken — otherwise its projection is ambiguous (two incompatible local behaviors
"merge" to nothing). Example: Buyer chooses Accept/Reject; Shipper only acts on Accept. If nobody tells
Shipper, Shipper's projection can't know whether to expect a `Ship` message.

Two stances:
- **(default) Reject + diagnose.** If `G ↾ r` is undefined because role `r` lacks knowledge of a choice,
  the compiler **rejects `G`** with a diagnostic naming `r` and the choice, and suggests the fix (add a
  selection/notification message `chooser -> r : Label`). Safe, explicit, teachable — matches Cadenza's
  "no silent magic" ethos and the fleet's own explicit `assign`/`reject` messages.
- **(alt) Auto-insert notifications.** The compiler silently inserts selection messages so every protocol
  projects. Convenient, but it *changes the wire behavior* the human wrote — a form of ambient magic
  Cadenza generally rejects.

**Default: reject + diagnose**, with the diagnostic doing the teaching. (This is the MPST-with-explicit-
selection discipline, which is also what makes projection *total* on the accepted subset.)

Also to handle in projection: **recursion/loops** (`rec X . G` projects to a recursive local type;
requires the loop to be *guarded* — at least one communication per iteration — and all roles to agree on
continue-vs-exit, another knowledge-of-choice site) and **parallel/interleaving** (independent
sub-protocols; projection distributes, but shared roles must not deadlock across the interleaving).

### 3.1 The projectability algorithm Inc 1 actually implements (paradigm-independent)
This is the crux the whole design rests on, so it must be concrete, not prose. Projection `G ↾ r` is
defined by structural recursion on `G`; the only interesting case is **choice**, and the only partiality
is the **merge** `⊓` on the branches for a role that is *not* the chooser. Given `G = p → { lᵢ : Gᵢ }`
(role `p` internally selects label `lᵢ` then continues as `Gᵢ`):

- **`p ↾ (p → {lᵢ:Gᵢ}) = ⊕{ lᵢ : (Gᵢ ↾ p) }`** — the chooser gets an **internal choice** (it decides), a
  row of `send`-obligations, one per label.
- **`q ↾ (p → {lᵢ:Gᵢ}) = &{ lᵢ : (Gᵢ ↾ q) }`** when the *first action* of every `Gᵢ ↾ q` is a `recv`
  **from `p`** of the distinct label `lᵢ` — role `q` gets an **external choice** (it is told), a row of
  `recv`-branches. This is "knowledge of choice": `q` learns the branch from `p`'s selection message.
- **`q ↾ (p → {lᵢ:Gᵢ}) = ⊓ᵢ (Gᵢ ↾ q)`** otherwise — `q` is *not* directly told, so its per-branch
  projections must **MERGE**. Merge is defined only when the branches are *behaviourally reconcilable*:
    - identical local types merge to themselves;
    - two external choices `&{…}` / `&{…}` merge by **unioning their branch rows** (a role that receives
      `A` in one branch and `B` in another can offer `&{A:…, B:…}` — it will learn which at recv time);
    - **anything else is `⊓`-undefined ⇒ `G` is UN-PROJECTABLE for `q`.**
  When `⊓` is undefined, Inc 1 **rejects** with the §4-default diagnostic: *"role `q` cannot tell branch
  `l₁` from `l₂` of the choice at `p` (line N); add a selection message `p → q : <label>` in each branch."*

This is exactly the classical MPST projection-with-merge (Honda–Yoshida–Carbone), restated on Cadenza's
AST. Two Cadenza-native leverage points make it cheap to build and trustworthy:
- **It's a fold over the protocol `Ast` written in Cadenza** (§2.1): `project` and `⊓` are ordinary
  compile-time functions pattern-matching the tree with quasiquote patterns — no new Rust phase.
- **`⊓` on external choices = row union** (§2.3): merging `&{…}` branches is literally the row-combine
  operation the type system already has (type-system.md line 118, "combine two records into one whose field
  set is the union"). So the hardest operator in the literature is a *reuse*, not a new mechanism.

Merge is also where **recursion** is checked: `rec X . G ↾ r` requires every role's unfolding to reach a
communication before recurring (guardedness) and the loop's exit-vs-continue to itself be a knowledge-of-
choice site (someone must signal "loop again" / "done"), handled by the same `⊓` rule.

---

## 4. The surface: full choreographic programming (ADOPTED)

**The operator's ruling (2026-07-17): go full choreographic programming.** The single artifact is **one
global program**: it names the roles, threads **located values** (`x@Role` — a value that physically lives
at `Role`), and moves data with **explicit communications** (`a ~> b`). The compiler **endpoint-projects**
this one program to **generate each actor's executable code**. The author writes **no per-endpoint code** —
there is literally one source, and Buyer's program, Seller's program, and Shipper's program are all
*emitted* from it. This is the most ambitious, highest-differentiation shape and the one the metaprogramming
machinery exploits best.

```
// The ONE global source: the PROGRAM, written once.
//   x@R          a located value: `x` lives at role R.
//   e@R ~> S     communicate: evaluate `e` at R, send it, bind the result located at S.
//   Label@R ~> S a selection: R tells S which branch was taken (knowledge of choice — §3).
choreography Purchase(title@Buyer: String) =
  let title'@Seller = title ~> Seller             // Buyer sends the title; it arrives located at Seller
  let quote@Buyer   = price(title')@Seller ~> Buyer  // Seller computes a quote locally, sends it to Buyer
  if accept?(quote)@Buyer then                    // an INTERNAL CHOICE, decided at Buyer
    Accept@Buyer ~> Seller                          // Buyer NOTIFIES Seller of the branch (required — §3)
    let addr@Shipper = addr(title')@Seller ~> Shipper
    ship(addr)@Shipper
  else
    Reject@Buyer ~> Seller
```

**What the compiler generates.** Projection walks this one program per role, keeping only that role's
actions and turning each `~>` into a matched `Comm.send`/`Comm.recv` (§2.1 — a recursive `Ast → Ast` fold;
§3.1 — the projection algorithm). The three emitted actors:

```
// GENERATED — the author wrote none of this.
def Buyer(title: String) =
  Comm.send(Seller, title)
  let quote = Comm.recv(Seller)
  if accept?(quote) then Comm.send(Seller, Accept) else Comm.send(Seller, Reject)

def Seller() =
  let title' = Comm.recv(Buyer)
  Comm.send(Buyer, price(title'))
  match Comm.recv(Buyer) with
    | Accept => Comm.send(Shipper, addr(title'))
    | Reject => unit

def Shipper() =
  match Comm.recv(Seller) with               // Shipper only hears from Seller on the Accept branch;
    | Ship(addr) => ship(addr)               // reachable ONLY because Seller forwards after being told
```

**Why this is correct-by-construction:**
- **Matched pairs.** Every `~>` projects to exactly one `send` at the source and one `recv` at the target —
  sends and receives cannot mismatch, the structural root of deadlock-freedom (§5, Q3-ii).
- **Knowledge of choice is a compile error, not a footgun.** The `if …@Buyer` makes Buyer's continuation
  differ by branch; any role whose behavior differs (here Seller) must be *told*. Omitting the
  `Accept@Buyer ~> Seller` / `Reject@Buyer ~> Seller` selections makes the `⊓` merge in §3.1 undefined, and
  the compiler **rejects** with a diagnostic naming Seller and the choice (Q4). The author cannot silently
  ship an un-projectable program.
- **Located types are inferred.** `title'@Seller` etc. carry a *location* alongside their ordinary type;
  a value may only be used at the role it lives at (using `title'` at Buyer after it moved to Seller is a
  type error), and `~>` is the sole way to change location. This is a light extension of the existing type
  system, not a parallel one.

### 4.1 Considered alternatives (not adopted)
Two other paradigms were designed and sketched; the operator chose (b) over both. Kept as a record.
- **(a) MPST type-level** — the single artifact is a *global type*; projection yields a per-role *local
  (session) type*, and the author still **writes each endpoint by hand**, checked against its projected
  type. Most incremental (leans on inference/rows), but you still write N endpoints — no code generation,
  lower "wow." Closest to Scribble / session-type libraries, but multiparty and global-sourced.
- **(c) hybrid** — a global declaration projects, per actor, BOTH a checked local type AND a fill-in
  handler *scaffold* (generated send/recv spine, holes for local logic). Subsumes (a)+(b) on one projection
  engine and was the pre-ruling default; the operator preferred the fully-generated (b) end state directly.

The machinery (§2, §3.1) is shared across all three, so the design investment in projection/merge/rows is
paradigm-independent; only the *surface* and the *degree of code generation* differ. (b) takes generation
all the way: one artifact, every actor emitted.

### 4.2 The projection target: one self-contained deployable artifact per actor (operator clarifications)
Two operator clarifications after the Q1 ruling sharpen *what projection produces*:
- **Per-actor artifacts are fully self-contained and independently deployable.** Projecting the one
  choreography yields, per role, not just a per-role *function* but a **separate, totally self-contained
  compiled artifact** — each actor is its own independent deployable unit (a wasm component / binary), with
  only the `Comm` effect as its boundary to the others. The single global source fans out to N standalone
  programs that never share a runtime; they interact *only* through projected messages. This is the natural
  end of "shred it up into one per actor" — the shredded pieces are deployables, not just code fragments.
  Design consequence: projection's output per role is a *complete program* (entrypoint + its `Comm`
  delegation manifest, per capabilities-and-effects.md), and each actor's manifest is exactly its projected
  message alphabet — nothing more is grantable. Inc 3's gate should therefore compile each projected actor
  as its *own* component and run them against a shared mock `Comm` transport, not link them into one module.
- **The compiler runs as a wasm build (agent-harness/kernel context).** The agent-runtime uses a wasm build
  of the Cadenza compiler; since projection is a compile-time metaprogram (§2.1), "generate every actor" is
  a capability the *wasm-hosted* compiler exposes — i.e. an agent can project a choreography into per-actor
  artifacts at runtime through the compiler-as-tool. This aligns choreographic-protocols with the
  agent-runtime vision (`DESIGN-agent-runtime-vision.md`): the fleet flagship (§6.1) is not just an
  illustration but a path to the runtime projecting its own coordination.

---

## 5. The other forks (Q2–Q5) — defaults chosen, operator may override

- **Q2 — lean on effects for the runtime?** *Default: yes, fully.* `Comm.send`/`Comm.recv` are effect ops;
  a projected actor is a program that performs `Comm`; the host binds the wire (any transport). An actor's
  capability row = its projected message set → capability-floor *is* fidelity.
- **Q3 — how strong a guarantee, how hard?** *Default tiering:* Inc-early = **(i) projectable-or-reject**;
  mid = **(ii) deadlock-freedom by construction** (structural for (b), well-formedness-checked for (a));
  north-star = **(iii) HOL-kernel machine-checked projection soundness + fidelity**. Commit to (iii) as the
  vision, land (i)→(ii)→(iii) as increments.
- **Q4 — knowledge of choice:** *Default:* **reject + diagnose** (§3), never auto-insert.
- **Q5 — runtime substrate:** *Default:* **transport-agnostic** (host binds `Comm`, like effect
  delegation), with the **fleet / agent-runtime event log as the flagship concrete binding** — model the
  fleet's own coordination (`send`/`merge-request`/`reject`/`assign`/`ask`→`answer`) as the showcase
  choreography. Ties directly into `DESIGN-agent-runtime-vision.md`.

---

## 6. Increment plan (top-to-bottom, the way a vertical lands it)

For the adopted (b) choreographic-programming surface: Inc 3 GENERATES each actor's executable code from
the one global program. Each increment is independently gated green.

- **Inc 0 — the choreography AST + parser.** A `choreography` surface form (located values `x@R`, comms
  `e@R ~> S`, selections `Label@R ~> S`, `let`, `if`-at-role, `rec`) parsed into a built-in-`Ast`-shaped
  value. Gate: round-trip parse/print of the Purchase choreography; a well-formedness checker rejects
  malformed input (e.g. a comm whose source/target roles are undeclared).
- **Inc 1 — well-formedness + projectability check.** The §3 knowledge-of-choice analysis; reject a
  choreography whose `if`-at-a-role fails to notify a differing role, with a role-naming diagnostic. Gate:
  a corpus of projectable / un-projectable choreographies with expected accept / `(error CDZ…)`.
- **Inc 2 — projection to per-role programs (the core).** `project : Ast -> Role -> Ast` as a compile-time
  fold: keep only `Role`'s actions, turn each `~>` into a matched `Comm.send`/`Comm.recv`, project `if`-at-
  `Role` to a local branch and `if`-at-another to an external `match` on the selection. Gate: the projected
  `Ast` of each role of Purchase matches a golden.
- **Inc 3 — code generation + end-to-end execution.** Splice the projected per-role `Ast`s as real top-level
  defs (the author wrote none). Gate: the three generated actors compile and, composed, execute the
  choreography end-to-end over a mock in-memory `Comm` handler — a `title` value provably flows Buyer→Seller
  and a `quote` flows back, and the Accept branch reaches Shipper.
- **Inc 4 — deadlock-freedom by construction.** The structural matched-send/recv argument (every `~>`
  projects to a paired send/recv); a corpus of would-be-deadlocking choreographies rejected. Gate: no
  accepted choreography deadlocks in the mock runtime.
- **Inc 5 — the fleet as flagship choreography.** Model the fleet coordination protocol; project the
  role loops. Ties to agent-runtime. Gate: the projected roles round-trip a `merge-request`→`merged`.
  *Worked sketch (§6.1) — the motivating example that makes the whole design concrete.*
- **Inc 6 (north star) — machine-checked projection soundness.** State + discharge in the HOL-Light kernel
  that projection refines the global protocol and the composition is deadlock-free. Gate: kernel-checked
  proof object in the verification suite.

### 6.1 Worked example — the fleet's own coordination as a choreography (paradigm-neutral form)
The flagship. The fleet already *runs* this protocol daily; writing it as ONE global source and projecting
the role loops is the demonstration that the design models real distributed systems, not toys. Here is the
**global protocol** in the abstract form (independent of the Q1 surface — only the concrete syntax changes
between paradigm (a)/(b)/(c); the interaction is the same):

```
protocol FleetIntegrate =                       // roles: Worker, PrSync, Concierge
  rec Session .
    Worker  -> PrSync    : MergeRequest(Ref, GateSummary)
    choice at PrSync                             // PrSync decides after gating — the choice site
      | Merged  => PrSync -> Worker : Merged(Sha) .           Session      // loop: worker sends next MR
      | Reject  => PrSync -> Worker : Reject(Reason) .        Session      // loop: worker fixes + resends
      | Blocked => PrSync -> Concierge : Ask(Question)                     // escalate to the human
                   Concierge -> PrSync : Answer(Decision) .   Session
```

What this one artifact demonstrates about the design, end to end:
- **The choice at `PrSync` is projectable** precisely because every branch's *first action toward each
  other role is a distinct labelled message* (`Merged`/`Reject` to Worker; `Ask` to Concierge) — so
  Worker's projection is `&{Merged: …, Reject: …}` (it is *told* which) and Concierge's is `&{Ask: …}`.
  The §3.1 merge succeeds by row-union; no role is left guessing. This is the design's central claim shown
  on a real protocol.
- **Guarded recursion:** every unfolding of `Session` performs at least one communication before recurring
  (the `MergeRequest`), so `rec Session` projects to a well-defined recursive local type per role — the
  loop each fleet agent actually runs.
- **Projected `Worker`** is exactly today's hand-written loop ("send a `merge-request`; on `Reject` fix and
  resend; on `Merged` send the next") — but now *generated from* (paradigm (b)) or *checked against*
  (paradigm (a)) the global protocol, so a worker that tried to send an *unexpected* message, or ignore a
  `Reject`, would not compile. **The fleet contract's prose invariants become type errors.**
- **Capability = fidelity (§2.2):** `Worker`'s projected `Comm` row grants send-`MergeRequest` /
  recv-`{Merged,Reject}` and *nothing else* — a worker literally cannot perform `PrSync`'s `Merged`
  send. The single-writer invariant the fleet enforces socially is enforced by the *type* of the projection.
- **Inc 6 payoff:** "no reachable state of `FleetIntegrate`'s projection composition deadlocks" is a
  theorem the HOL kernel discharges — a machine-checked proof that the fleet's coordination cannot wedge.

The gate for Inc 5 is this protocol accepted, its three roles projected, and a `MergeRequest`→`Merged`
round-trip executed over a mock in-memory `Comm` handler (a `Ref` value provably flows Worker→PrSync and a
`Sha` flows back).

---

## 7. Seams / file anchors (provisional)
- Global-protocol AST + parser: a `protocol`/`choreography` reader rule → built-in `Ast`
  (`cadenza-syntax`; the reader stays non-extensible per metaprogramming.md — one fixed rule producing a
  canonical node, interpreted by a binding-dispatched compile-time function, exactly like tagged-template
  macros).
- Projection: a compile-time Cadenza function on the one-tier evaluator (`rcdzc/src/fold.rs` is the tier;
  the projection *logic* is Cadenza code, likely in `implementation/compiler-ml/` or a new
  `implementation/choreography/` package once bootstrapped).
- Comm effect: `(effect Comm | send : … | recv : …)`, host-bound like other capabilities
  (`rcdzc/src/link.rs` binding surface; `capabilities-and-effects.md`).
- Local-type rows: `rcdzc/src/infer.rs` / `unify.rs` (rows already unify).
- Proofs: `implementation/verification/` (HOL-Light kernel module).

## 8. Open decisions (chosen defaults, operator may override)
1. Q1 paradigm — **DECIDED: (b) full choreographic programming** (operator ruling 2026-07-17; §4). 2. Effects for runtime — **yes** (Q2). 3. Guarantee tiering
— **(i)→(ii)→(iii)** (Q3). 4. Knowledge of choice — **reject+diagnose** (Q4). 5. Substrate —
**transport-agnostic, fleet as flagship** (Q5). 6. Which subsystem owns it — **`compiler-ml`/new
`choreography` package** (projection is Cadenza code), with `rcdzc` seams for the `Comm` effect + reader
rule.

## 9. Gate (what protects this)
A `choreography`/`protocol` corpus under the existing gate: projectable protocols accept + their projected
endpoints execute end-to-end over a mock `Comm` handler (a value provably flows across all roles);
un-projectable protocols reject with the expected `(error CDZ…)`; deadlocking protocols reject; and (north
star) a kernel-checked projection-soundness proof in the verification suite. Additive-only to the baseline,
per the fleet gate discipline.
