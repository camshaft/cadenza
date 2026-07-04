# The assembler lives in Cadenza — even the instruction-to-bytes step is the compiler's own

*2026-07-03*

**What happened.** Hand-concatenating raw wasm bytes in the Cadenza compiler was becoming hard to
read, so a WAT-like structured instruction layer was introduced: the emitter builds a list of tagged
instruction forms (`(op-i64-const 42)`, `(op-if <kind> <then> <else>)`, `(op-call idx)`, …) and a fold
turns that structure into the binary encoding. The tempting shortcut was to emit a *textual* WAT string
from the compiler and assemble it to bytes with the host's `wat` crate (already cached, one call). That
was rejected: it would place a text-to-bytes translation in the seed/host and make the compiler emit a
partial artifact a separate tool finishes. Instead the assembler — `assemble-seq` / `assemble-instr`,
the only place raw wasm opcodes appear — is authored in Cadenza and run by the seed like the rest of the
compiler. The maintainability win (structured instructions instead of byte soup) is kept; the seam is
preserved. (An incidental finding while naming the instruction constructors: the reader treats a dotted
identifier `a.b` as member-access sugar `(. a b)`, so `i64.const` as a definition name would read as
member access on an unbound `i64`; the constructors use dot-free names like `op-i64-const`.)

**Why.** The bootstrap's whole value rests on one seam: the seed contributes only evaluation, and the
Cadenza-authored compiler contributes the entire translation to component bytes, so a derivation's bytes
are a function of the Cadenza compiler alone and self-hosting is a clean fixpoint. "Translation to
bytes" is easy to read narrowly as "the interesting codegen," letting a low-level assembly step drift
into a host tool for convenience. But the final instruction-encoding step *is* part of the translation;
if it lives in a host crate, the compiled artifact is no longer solely the compiler's output and the
fixpoint is broken. The existing requirement said the translation must be authored in Cadenza; this
made explicit that *every* lowering stage, including textual/structured-to-binary assembly, is included.

**The requirement it drove.** Tightened `spec/capabilities/self-hosting-and-bootstrap.md` §"Each
Generation Is Derived By The Previous" with a requirement that every stage lowering a program toward
component bytes — including assembling any textual or structured instruction form into its binary
encoding — MUST be authored in Cadenza rather than performed by a seed-language or external tool, so no
part of the translation escapes the Cadenza-authored compiler. Reinforces bootstrap.md §"The Compiler
Is Authored In Cadenza, Not In The Seed" ("the bytes … MUST be produced by evaluating the
Cadenza-authored compiler"; "the complete runnable component rather than a partial artifact that a
separate tool completes").
