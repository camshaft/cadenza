# The Cadenza compiler emits the whole component, not a core module a tool completes

*2026-07-03*

**What happened.** Having settled that the Cadenza-authored compiler emits component bytes (see
[the-seed-realizes-bytes](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)), the
build reached a fork: does the compiler emit only the **core module** (and a mechanical tool such as
`wasm-tools component new` adds the component-model envelope, as `options/execution-model/` then
pinned), or does it emit the **complete component binary** itself? Both keep the load-bearing codegen in
Cadenza and both are non-modeled. The operator chose the complete-component reading, tied to the north
star: the seed interprets the compiler, and ultimately the compiler compiles its own source — so if the
compiler emits complete components, its self-compiled successor emits complete components too, a clean
fixpoint with a derivation's byte output a function of the Cadenza compiler *alone* (no external tool in
the byte path).

**Why.** The spec had a latent tension: a new `spec/bootstrap.md` requirement said the component bytes
are "produced by evaluating the Cadenza compiler," while `options/execution-model/wasm-component-model.md`
described codegen as emitting a core module "wrapped (e.g. via `wasm-tools component new`) into a
component" — i.e. a tool completed the bytes. Read strictly, the two disagreed on whether the wrapping
tool is in the byte path. Left unresolved, re-derivation reproducibility and the self-hosting fixpoint
would depend on an external tool's determinism rather than on the Cadenza compiler, blurring "a
derivation is a function of the canonical source and the Cadenza-authored compiler."

**The requirement it drove.** `spec/bootstrap.md` §"The Compiler Is Authored In Cadenza, Not In The
Seed" — added: the bytes the Cadenza-authored compiler produces MUST be the complete runnable component
rather than a partial artifact a separate tool completes into the component, so that a derivation's byte
output is a function of the Cadenza-authored compiler alone. `options/execution-model/wasm-component-model.md`
§"Derivation produces a real component whose world matches the manifest" was rewritten to match: the
compiler emits the complete component binary as a `Bytes` value; a validator such as `wasm-tools` may be
used at the seed only as an out-of-band oracle that the emitted bytes are a well-formed component, never
as a step that produces or completes them.
