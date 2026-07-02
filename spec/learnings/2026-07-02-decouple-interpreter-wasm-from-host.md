# Interpreted derivation embeds the interpreter, not its transcript; the host is a separate concern

*2026-07-02*

**What happened.** The first cut of the seed's interpreted derivation took a shortcut: it ran the
reference interpreter *in the host (Rust) at derivation time*, computed the program's observable
behavior, and emitted a WebAssembly module that merely **replayed** the pre-computed events through
host imports. The component was content-addressed, its imports mirrored the manifest, it was executed,
and it re-derived byte-identically — yet the actual interpretation happened in the host, not in the
component. The emitted module was a *transcript*, a placeholder standing in for the interpreter rather
than containing it.

**Why it is wrong.** This is the "modeled derivation" the bootstrap spec exists to forbid
([bootstrap.md](../bootstrap.md) §"A Modeled Derivation Is Not An Ignition": an ignition demonstrated
only by emitting the events that would accompany a derivation, without a component that actually
performs it, is not conforming; a subsystem satisfiable without executing it must be exercised by the
end-to-end path, not stood in for by a placeholder). Two concerns were conflated: *what the derived
component is* (a real interpreter over the program) and *what computes the answer* (which had leaked
into the host). Because the leak still produced all the right shapes — a running component, mirrored
imports, a reproducible hash — every surface check passed while the load-bearing behavior (the
component interpreting Cadenza) never ran in the component. This is the same class of failure as
[a modeled subsystem passes a shape check](./2026-07-02-a-modeled-subsystem-passes-a-shape-check.md).

**The decoupling this drove.** Interpreted derivation has **two separate artifacts**, and the process
must build them as such:

1. **The interpreter, compiled to a component.** The reference interpreter — a function from a program
   AST to its observable behavior — is compiled to WebAssembly (at the seed stage, the Rust
   interpreter is compiled to wasm by the host toolchain; later, the Cadenza-authored interpreter is
   derived by the previous generation). Deriving a program is binding *this interpreter component*
   together with *the program's canonical AST as embedded data* into one content-addressed component
   (exactly `options/bootstrap-strategy/rust-seed-interpreted-first.md` §"Interpreter packaging": the
   interpreter component plus the program's canonical source, bound to one content-addressed
   component). When the derived component runs, it reads its embedded AST and interprets it — the
   semantics execute *in the component*, at run time.

2. **The host, providing the minimal function set.** The capabilities a component imports
   (`emit-event`, `read-projection`, `read-blob`, `invoke-tool`) are provided by a host that is a
   distinct concern from the interpreter. The host binds exactly the manifest's capabilities and no
   others (host-interface-binding.md §"Imports Mirror The Manifest Exactly"); the derived component's
   import set is trimmed to the manifest, so the two mirror. The interpreter does not embed the host,
   and the host does not embed the interpreter.

**The requirement/process it drove.** No frozen-contract change — the contracts already say derivation
embeds the interpreter over the source (build-tool-interface.md §"Derivation By Embedding The Reference
Interpreter") and that imports mirror the manifest. What changed is the **process**: `ignite.md` and
`build.md` now state explicitly that interpreted derivation embeds the interpreter *component* over the
program's AST (not the interpreter's precomputed output), and that the host providing the capability
functions is a separately-built minimal artifact. The check that distinguishes the two: an ignition is
conforming only if the *component* performs the interpretation — verifiable by deriving two different
programs and confirming the same interpreter component produces different behavior purely from its
different embedded AST, with no program-specific logic emitted by the derivation.
