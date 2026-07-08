# Fuel is host-owned runtime policy, not a compiler-emitted measure

*2026-07-06*

**What happened.** A design pass on resource exhaustion asked whether a running program could *recover*
from running out of fuel — the host refueling it, yielding it to other work, or aborting — rather than
only halting terminally. The first framing modeled this as a new **boundary effect**: fuel exhaustion
becomes a suspension point, resumed by the same host-owned-replay mechanism a host call uses. That
framing was rejected on its own merits: a data host call is a *coarse* effect (a handful of statically
known I/O points in a program), but fuel can run out at **any** loop back-edge or call, so making
exhaustion an effect turns the whole program into a fine-grained state machine of tiny resumable units —
the opposite of the coarse effect surface the language has. It also let the program observe *how much
fuel it had left*, which is a determinism hazard (see below).

The pass then asked the sharper question: does the runtime already do this natively, so we build no
state machine at all? It does. The seed runs finished components under the embeddable `wasmtime` crate
(`cadenza-seed`, pinned to wasmtime 37 with `runtime`/`cranelift`/`component-model`). Wasmtime offers
exactly the three host actions, all host-side, all on the same fiber, over emitted wasm that is
**unchanged**:

- **`Config::consume_fuel(true)`** makes Cranelift inject the fuel counting *at JIT time* — every loop
  back-edge and call entry — so the compiler emits nothing. This is the deterministic per-call/per-loop
  counter a prior note said we would otherwise have to hand-emit to fix bounded-deep recursion trapping
  at the native stack limit.
- **`Config::async_support(true)` + `Store::fuel_async_yield_interval` + `TypedFunc::call_async`** is a
  native fiber yield: at the interval the fiber suspends to the host's async executor and resumes the
  *same stack* — zero recompute, no teardown, no program-side state. This is "yield a long-running
  program to other processes." Dropping the `call_async` future instead **cancels and unwinds the
  fiber** — that is "abort." Refuel is `set_fuel(n)` on the resumed store.
- **`Config::epoch_interruption(true)`** with a deadline callback (`UpdateDeadline::Continue` / `Yield`
  / trap) is a cheaper scheduling interrupt, but it fires on **wall-clock**, so it is fit only for the
  *unobservable* yield, never for the abort/complete decision.

**Why.** Two properties make delegating fuel entirely to the host the right move rather than a downgrade:

- **The compiler never owned fuel to begin with.** Under `consume_fuel`, the runtime instruments plain
  emitted wasm loops and calls. The "compiler MUST emit the measure" reading was a mechanism the seed
  never implemented and does not need — the obligation is satisfiable by *delegation* to a runtime that
  guarantees interruptibility. Structured wasm control flow means the only unbounded constructs are loop
  back-edges and calls, and the runtime instruments exactly those, so coverage is total and automatic.

- **Exhaustion is a resource terminal like out-of-memory, not observable behavior.** Once the program
  cannot read its remaining fuel — which is *mandatory*, not a preference: if it could, output would
  depend on how the host sliced the budget, and grant scheduling is a host runtime choice that must not
  be observable — the consequence of fuel is binary and coarse. A run that *completes* produces
  byte-identical output regardless of budget size or grant schedule (two schedules that both sum to
  enough are indistinguishable); a run that *aborts* produced no value whose determinism is at stake.
  So "whether a program is permitted to complete versus interrupted for resources" is a host resource
  policy, exactly as OOM is, and it is *not* a deterministic function of the program's input — while the
  determinism that matters (the value and host-call sequence of a completing run) is untouched.

The distinction that killed the effect framing is the same one that makes this safe: fuel accounting is
**cumulative-from-entry**, which is why (a) it cannot be a coarse effect — its natural granularity is
every operation — and (b) replay-from-entry re-consumes it, so the durable-continuation replay path that
works for rare data host calls is quadratic for fuel. The resolution is that fuel is not in the program's
state machine at all: it is the runtime's, realized by fuel + async-fiber yield for the live case (zero
recompute) and teardown for the durable case (accepts recompute for crash-survival, which we do not
need for the yield use case). "Durable yield" collapses to "abort and replay the data log," which is
cheap in mechanism and simply not required here.

The one obligation that **relocates rather than vanishes**: Core Principle V's bounded-termination
guarantee cannot be dropped, or a runaway loop under a fuel-less runtime hangs uninterruptibly and the
"yield or abort a long-running program" goal is impossible. So the obligation moves from *"the compiler
MUST emit the measure"* to *"the execution environment MUST be able to interrupt any run at a bounded
resource point,"* which wasmtime satisfies by configuration. The compiler does nothing; the language
keeps only the guarantee that some deterministic (operation-count-class, not wall-clock) measure exists
and that no construct escapes it, and hands ownership of the budget, the abort/yield/refuel decision,
and the counter implementation to the host.

A consequence for the conformance gate: the gate host MUST meter with deterministic operation-count fuel
under a generous budget, never wall-clock — two independent compilers emit different operation counts for
the same source, so a tight or wall-clock budget would make the differential gate flaky (this repo has
the exact failure mode on record, where saturating CPU produced false "hangs"). Deterministic fuel for
the gate; the *language* still permits a wall-clock policy for real hosts, because abort is not a
conformance property.

Two ideas were explicitly set aside as out of scope. A `Fuel.remaining` operation (letting a program
checkpoint proactively) is rejected outright — it makes grant scheduling observable and breaks the
determinism above. A *scoped/metered sub-computation* handler (running untrusted macro expansion under a
sub-budget) is inconsistent with "the program has no idea about fuel," so it is a separate,
deliberately-observable opt-in capability, not this ambient mechanism.

**The requirement it drove.** This relaxes the resource-accounting obligation from the compiler to the
execution environment, so it touches a Core Principle and a frozen contract and is landed by a
coordinated act with explicit human approval, not by this learning alone:

- **constitution.md Core Principle V ("Bounded Termination By A Deterministic Measure")** — reworded so
  the *execution environment*, not the compiler, must keep every unbounded construct accountable against
  a deterministic measure and interruptible at a bounded point; recorded as a constitution amendment. It
  does not downgrade the never-downgradable determinism floor, because the observable behavior of a
  *completing* run stays a deterministic function of input and capability responses; exhaustion becomes a
  host resource terminal outside that function, as OOM already is.
- **spec/contracts/determinism-and-fuel.md §Resource Accounting** — the compiler-emission obligations are
  replaced by an execution-environment-conformance obligation (interruptible at a bounded, deterministic,
  non-wall-clock measure the host budgets); the "Deterministic Emission" section (float rounding,
  no-uninitialized-reads) is unrelated to fuel and is untouched. This alters no emitted byte, because
  emitted components never carried fuel-decrements.
- **spec/capabilities/core-semantics.md §Evaluation Is Bounded, §Recursion Is Accountable Against The
  Resource Measure, §A Program Terminates In Exactly One Terminal Condition** — reworded to attribute the
  measure and the bounded-halt to the execution environment and to scope terminal-condition determinism
  to completing runs, exhaustion being a host-budgeted terminal.
- **spec/glossary.md** — *Resource measure* and *Fuel* reframed as host-owned and program-unobservable.

It builds directly on the resumption-strategy split already committed in the effects model
([[2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log]], `options/effects-model/`): resuming a
run live versus tearing it down is a host choice off the byte path, and fuel yield is the same choice
applied to the runtime's own interrupt rather than to a declared effect. It also supersedes the
open fix direction recorded for bounded-deep recursion trapping at the wasm native stack limit — the
deterministic per-call counter that fix wanted is what `consume_fuel` provides, so the fix is a runtime
configuration, not a compiler feature.
