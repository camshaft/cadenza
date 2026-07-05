# The host is value-agnostic; the compiler owns the reader and printer

*2026-07-04*

**What happened.** A boundary decision: the host knows **nothing** about Cadenza value structure —
it shuttles opaque bytes/text. The knowledge of "what a Cadenza value looks like as text" lives in
the **compiler**, exposed as two surfaces that are the reader and printer the self-hosting spec
already requires:
- **reader**: text s-expression → canonical binary
- **printer**: canonical binary value → canonical text

**Why it came up.** Returning a compound value (a sum `(Some 42)`, a tuple `(tuple 1 true)`, bytes,
an AST) from a compiled component ran into a wall: the component-model value ABI would express these
as `variant`/`tuple`/`list` types, but (a) the component model requires **kebab-case** variant names
(so `Some`/`Sign.Pos` can't be component cases directly), and (b) a component value renders in the
host (wasmtime `Val`) as Rust debug output like `List([U8(1)])` — nowhere near Cadenza's canonical
`(Bytes.of (list 1 2 3))`. Making the gate agree would force the host to render every component-model
shape into Cadenza surface syntax: brittle, and it duplicates value-form knowledge the compiler
already has.

**The resolution (final shape).** Don't teach the host Cadenza, but DON'T collapse the boundary to a
string either — keep it strictly, statically typed. Main's result crosses the boundary as its
**proper component type**, exported as a **resource that owns a `display` method**:

    world runnable {
      resource value { display: func() -> string; }
      export run: func() -> value;
    }

The harness calls `run()` → gets an opaque resource handle → calls `.display()` on it → gets the
canonical text. The value never crosses destructured; the host handles only a handle and a string;
`display` lives WITH the value's type and is emitted by the compiler as part of exporting that type.
This is stricter than "everything is a string" (the boundary stays typed — constitution VII), and it
keeps the host formatting nothing. Rejected intermediate ideas: (a) teaching `render_val` every
component-model shape (host over-specialized); (b) `run : () -> string` returning printed text
(untyped boundary, violates strict typing). A free `display(value)` function was also rejected in
favor of a resource method so the value need not re-cross the boundary as an encoding.

**Consequences for cdz-rustc.**
- The existing `string_component` path (a `() -> string` component that writes constant bytes to
  linear memory and canon-lifts them) generalizes: a compound constant result is *printed* to its
  canonical text at compile time and returned as that string. No `variant`/`tuple` component types,
  no kebab-case problem.
- `host.rs` `render_val` stays trivial (it already renders `Val::String`); it gains no Cadenza value
  logic. `expected_render` in the gate compares against the corpus's canonical text.
- The printer is a compiler surface — eventually the Cadenza-authored compiler exposes it too, so the
  two compilers agree on canonical text as they agree on emitted bytes.

**The requirements it drove.**
- [host-interface-binding.md](../contracts/host-interface-binding.md) §"The Host Is Value-Agnostic"
  (3 reqs, additive to the frozen contract): host shuttles opaque bytes/text; render/read are
  compiler-exposed, not host operations; a load-verify-run root is unchanged by a new value form.
- [self-hosting-surface.md](../capabilities/self-hosting-surface.md) §"The Reader, Printer, And
  Display Are Compiler-Exposed Surfaces" (reqs): reader/printer/display are compiler surfaces; printer
  output is the canonical text form; a result's canonical text comes from the compiler-exposed display
  conversion a value-agnostic host invokes. The compiler itself reaches no host function.
