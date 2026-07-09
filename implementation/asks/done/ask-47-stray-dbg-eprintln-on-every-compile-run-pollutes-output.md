## 47. 🟠 A stray `DBG` eprintln fires on EVERY `compile-run` — output pollution from the ask-41 artifact-detection WIP

**Finding.** Every `compile-run` invocation prints a debug line to stderr, regardless of the input program:
```
DBG compile param[0] is list; element variant = U8; input_is_artifact_list=false
```
It fires even for a trivial `(module m (def (main) 0))`. Source: `seed/crates/cadenza-seed/src/host.rs:596`
(and the `else` branch at :598) — an unconditional `eprintln!("DBG compile param[0] …")` inside
`run_compiler_component`, left in from the ask-41 artifact-list input detection WIP (the `input_is_artifact_list`
matcher that checks whether `compile`'s first param is a `list<record>`):
```rust
if let Some((_, Type::List(l))) = params.first() {
    …
    eprintln!("DBG compile param[0] is list; element variant = {variant}; input_is_artifact_list={…}");
} else {
    eprintln!("DBG compile param[0] is NOT a list; input_is_artifact_list={…}");
}
```

**Why it matters.** The line goes to **stderr**, so it does NOT corrupt the conformance harness (which reads
`r.stdout` only) or a grep that targets the `compile → Ok` stdout line — the gate is unaffected. But it IS (a)
noise on every invocation, (b) a correctness smell (debug scaffolding shipped in a gate-green build), and (c) a
real hazard for any consumer that reads combined `2>&1`: VERIFIED — extracting the emitted byte array from a
`compile-run 2>&1` capture pulls the `DBG …` text into the byte list and makes `wasm-tools validate` fail with
`unexpected character '\u{0}'`. It is clearly unintentional (a `DBG` prefix). Twin of ask-44 (a stray `eprintln`
in the seed's ctor-arm match codegen) — same class, different site, from the newer artifact-detection work.

**Repro.** `cadenza-seed compile-run <any compiler.cdz> <any program>` → the `DBG compile param[0] …` line on
stderr, always.

**Fix.** Delete the two `eprintln!("DBG …")` lines at `host.rs:596` / `:598` (or gate them behind a debug flag /
`log::trace!`). The `input_is_artifact_list` logic they wrap is real (ask-41 artifact-input detection) and
should stay; only the print is the leak.

**Status.** 🟠 Seed (host side) — trivial removal, but it ships debug noise in a gate-green build. Related:
ask-44 (the sibling's stray-eprintln in ctor-arm match codegen — same class), ask-41 (the artifact-detection
work this scaffolding came from).

**🟠 LOOP-VERIFIED 2026-07-07 (Run 90) — fires on EVERY compile-run, stderr, low-severity.** Confirmed: a
trivial `(def (main) 0)` via `compile-run` prints 1 DBG line on stderr (`DBG compile param[0] is list; element
variant = U8; input_is_artifact_list=false`, host.rs:596). 0 on stdout, 0 during `emit`/behavior-gate — so
bytes/gate uninffected (gate 570 green, WRONG=0). SECOND stray-DBG-print (after ask-44); both from in-flight
artifact-ABI work on the compile-run path. Remove/gate the eprintln. This recurrence validates the standing
"skim compile-run stderr" check (Run-81 learning).

**✅ FIXED + RE-PROBED 2026-07-07 (seed 15:50).** The `eprintln!("DBG …")` lines are gone from
`host.rs` (`grep 'DBG compile param' host.rs` → 0), and `compile-run` on any program prints 0 DBG lines on
stderr. The `2>&1` byte-extraction hazard is resolved: extracting the emitted byte array from a `compile-run
2>&1` capture now yields a component that `wasm-tools validate`s CLEAN (previously corrupted by the interleaved
DBG text → `unexpected character '\u{0}'`). The `input_is_artifact_list` detection logic it wrapped is retained.
Moved open → done.
