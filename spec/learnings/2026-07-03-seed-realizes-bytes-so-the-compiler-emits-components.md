# The seed realizes a byte-sequence form so the Cadenza compiler emits component bytes

*2026-07-03*

**What happened.** Midway through synthesizing the seed toolchain in an attended build, the build was
about to author the component-emitting codegen **in the seed's foreign language (Rust)** — a Rust
function translating a program's AST to WebAssembly component bytes. The operator halted it: the *only*
role Rust should play is the seed **reference interpreter (the oracle)**; the compiler — including its
codegen — is authored in Cadenza and run by that interpreter, so that the first Cadenza artifact really
is the compiler and self-hosting is a clean fixpoint. Under that model a contradiction surfaced:
`spec/bootstrap.md` required the toolchain to *generate a component* (emit wasm bytes), while
`options/realized-capability-set/seed-ignition-set.md` **deferred all byte primitives** — so a Cadenza
program run by the seed had no value form with which to construct wasm bytes, and nothing normative
forbade putting the codegen in the seed instead.

**Why.** The specification under-determined the seed↔compiler seam. It said *who* authors the
translation loosely ("the compiler") and *what* the toolchain produces ("a component"), but never pinned
that the translation is authored in Cadenza rather than the seed, nor that the seed must realize a
value form capable of holding component bytes. With the seam unstated, the cheapest path was to write
the codegen in Rust — which would have made the seed the compiler and left the "first Cadenza artifact
is the compiler" line of sight unrealized, exactly the class of gap that `no-line-of-sight-to-self-hosting`
warns against.

**The requirement it drove.** `spec/bootstrap.md` §"The Compiler Is Authored In Cadenza, Not In The
Seed" — three requirements: the bytes of a derived component MUST be produced by evaluating the
Cadenza-authored compiler over the program's canonical representation; the seed reference interpreter
MUST NOT contain a translation of a program to component bytes; and the compiler MUST be able to
construct component bytes as an ordinary value of a byte-sequence value form the seed realizes, pinned
at the declared-default location. Paired in `spec/capabilities/self-hosting-and-bootstrap.md` §"Each
Generation Is Derived By The Previous" (the translation is authored in Cadenza, not the seed; the seed
MUST realize a byte-sequence value form). The declared default is the `Bytes` form added to
`options/realized-capability-set/seed-ignition-set.md`, witnessed by `spec/semantics/10-bytes.sexp`.
