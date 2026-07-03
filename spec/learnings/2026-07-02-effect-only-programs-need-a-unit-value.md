# An effect-only program had no normal-termination value; pin a Unit value

*2026-07-02*

**What happened.** During the attended `/build` run, the Phase-0 `analyze` pass failed the behavioral-
witnessing check: four executable-semantics cases — the event-emitting programs in
`spec/semantics/03-equality-and-observation.sexp` and `spec/semantics/04-capabilities.sexp` — carried
only an `(events …)` observation and **no primary result clause**, violating the corpus rule that "a
case with no definite primary result is not a case" (`spec/semantics/README.md`). The root cause was
not a corpus typo: there was **no value form for the normal-termination result of a program whose
`main` only emits events**. `emit-event` is typed `result<_, host-error>` in the host interface, but
nothing in `spec/contracts/deterministic-value-form.md`, the `options/type-mapping/` table, or the
`options/code-shape/` core symbol set pinned a unit/empty value, so an author could not write
`(output (: <value> <Type>))` for such a program, and the seed interpreter's
`Terminal = Normal(Value)` had no `Value` to carry for it.

This left three seed-realized behavioral requirements without a well-formed witness the seed can run to
a complete recorded observable behavior: core-semantics.md §"Emitted Events Are Ordered And Part Of
Observable Behavior", and the positive-direction behavior of capabilities-and-effects.md §"Capabilities
Are Declared Up Front" and §"The Program Manifest Is The Union Of Its Modules". A broken interpreter
that emitted the right events but then *trapped* would still have "reproduced the recorded output,"
because the terminal condition was unpinned.

**Why.** Observable behavior is defined as *terminal condition + normal-termination value + event
sequence* (core-semantics.md §"Observable Behavior Is A Defined Projection Of A Run"), and the value
form was pinned for every primitive and aggregate **except** the value an effect-only program returns.
The corpus exercised that shape before the value form existed to describe it — exactly the kind of gap
the behavior gate is meant to surface. This is an attended-mode halt: the missing value form is a
frozen-contract byte-level pin, so it was resolved deliberately rather than invented.

**The resolution and why it is additive, not a downgrade.** A **Unit value** was pinned rather than a
fourth "void" terminal condition. A Unit value satisfies the frozen three-terminal-condition
enumeration (core-semantics.md §"A Program Terminates In Exactly One Terminal Condition") and the
observable-behavior projection **unchanged** — an effect-only program terminates normally with a
value, the unit value. Both `deterministic-value-form.md` §"Additive Evolution" and `component-abi.md`
§"Additive Evolution" explicitly permit "defining a canonical byte form / boundary representation for a
value that previously had none" as an **additive** change, so this carries no version increment and
touches no Governance Floor. The rejected alternative — a `void` terminal with no value, or an optional
normal-termination value — would have *weakened* an existing frozen requirement, which the change
process forbids.

**The requirement it drove.**
- `spec/contracts/deterministic-value-form.md` §"The Unit Value Has A Canonical Byte Form" — the unit
  value has exactly one canonical byte encoding, distinct from every other value (additive).
- `spec/capabilities/core-semantics.md` §"An Effect-Only Expression Yields The Unit Value" — an
  expression evaluated only for its emitted event yields the unit value; a program that terminates
  normally without producing a value other than through its events produces the unit value.
- Declared defaults: `options/type-mapping/component-model-types.md` adds the Unit boundary row (the
  empty result payload); `options/code-shape/homoiconic-decoupled-display.md` adds the `unit` core
  symbol and the `Unit` built-in type name.
- The four corpus cases now carry `(output (: unit Unit))` alongside their `(events …)` observation, so
  each pins a definite terminal condition.
