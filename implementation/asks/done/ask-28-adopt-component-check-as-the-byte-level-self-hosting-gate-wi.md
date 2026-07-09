## 28. 🟢 Adopt `component-check` as the byte-level self-hosting gate — WIRING DONE (`--emit-component` landed); gate now RUNS, but its `disagree` count needs a decline discriminator (→ #29)

**🟢 WIRING DONE 2026-07-07.** The seed gained `compile-run <compiler.cdz> --emit-component <path>`, which
persists the Cadenza-authored `cadenza:compiler/compile` component (verified: `compiler.cdz` → 27 KB component).
`component-check <that> spec/semantics` now RUNS the whole-corpus byte diff: **58 agree, 496 disagree, 204
skip**. The gate is live. ⚠️ But the raw `disagree` count is misleading — see #29: 158 of the 496 are the
byte-identical 88-byte `func 0 → unreachable` DECLINE stub, not miscompiles. The `agree` count (58, byte-identical
to native) is the trustworthy signal; the `disagree` count needs the decline discriminator before it means
anything. Original scoping below.

**Finding.** With gap 3n fixed (#27), the self-hosting `compile-run` loop works for arbitrary programs and
`compiler.cdz` is byte-identical to native on the programs where byte-identity is expected. The byte-level GATE
— `cadenza-seed component-check <component.wasm> spec/semantics`, which already does the whole-corpus
native-vs-component byte diff — is now unblocked in principle, but cannot yet be pointed at the Cadenza-authored
compiler: `component-check` reads a compiler component from a fixed crate path (`crates/cdz-compiler-component/…
cdz_compiler_component.wasm` — the RUST cdz-rustc-as-component), and `compile-run` builds the *compiler.cdz*
compile-component in memory but never writes it to disk.

**The wiring step (seed, small).** Add a subcommand (or a `compile-run --emit-component <path>` flag) that
PERSISTS the compiler.cdz-built `cadenza:compiler/compile` component to disk. Then `component-check <that>
spec/semantics` grades the whole corpus at the byte level — the real differential self-hosting gate, replacing
the interim value-first `emit`-based harness. This is pure plumbing (the component already builds and validates;
`compile-run` proves it runs), not a compiler or language capability.

**Remaining after that (separate, later).** Corpus REJECTION cases need the diagnostics ABI — the `compile`
export returning `result<list<u8>, list<diagnostic>>` and a way to construct diagnostics — since compiler.cdz's
only failure channel today is a TRAP (`KError → unreachable`), no CDZ code. So `component-check` grades SUCCESS
cases (byte-identical / value) the moment the component is persistable; rejection cases wait on the diagnostics
gap (already noted in `compiler.cdz`'s entry comment and gap notes).
Learning: `spec/learnings/2026-07-07-gap-3n-fixed-the-self-hosting-loop-is-operational-and-the-byte-gate-is-one-step-away.md`.

---
