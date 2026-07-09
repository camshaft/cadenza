## 22. 🟢 Seed gap 3l: emit a `compile : list<u8> → list<u8>` component, not only nullary `run` — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by direct probe.** The seed now lifts a def named `compile`
with one `Bytes`/`list<u8>` param → `Bytes` as `cadenza:compiler/compile : func(list<u8>) -> list<u8>`
(codegen selects it over `run` by shape — codegen.rs:1039, 8749). A new dev subcommand
`cadenza-seed compile-run <compiler.cdz> <input.cdz>` builds the compiler as a compile component and drives it
over the input's canonical AST bytes. **Probed end-to-end:** an identity `(def (compile b) b)` builds a VALID
3,059-byte component and returns the input's 32 canonical AST bytes unchanged (the list ABI round-trips through
linear memory: input → runtime `Bytes` handle → user `compile` → result bytes → retptr). The retarea must be
4-aligned; SINGLE export only (the compiler world has one export; a general `(export …)` surface is deferred).

**Two mechanical steps remain (neither a language/correctness gap):**
1. **Rewire `compiler.cdz`'s entry** from the current nullary `(def (main) …hardcoded target bytes…)` to
   `(def (compile b) (compile-bytes b))`. `compile-bytes` (the whole read→resolve→fold→lower→serialize→frame
   pipeline) already exists and takes a `Bytes`. Until this happens, `compile-run` on the real `compiler.cdz`
   fails `expected 0 argument(s), got 1` — the nullary `main` is lifted as `run`, and the host drives it with
   the 1-arg input a `compile` entry expects. (No forcing function yet: the interim harness still works on the
   nullary form.)
2. **Get the value-heap runtime component building again** (currently broken — CHAMP set ops mid-implementation)
   so `cadenza-seed component-check <compiler.cdz-as-compile-component> spec/semantics` can run the whole-corpus
   diff. This is an unrelated in-flight change, not part of 3l.

**When both land:** retire the interim `run_corpus.py` harness (it exists ONLY because 3l was open) and use
`component-check` — the exact clean differential already written.
Learning: `spec/learnings/2026-07-07-a-bytes-to-bytes-compile-entry-unblocks-the-real-differential-harness.md`.
(Original finding kept below.)

**Finding.** The real self-hosting check is running `compiler.cdz` over the whole corpus via
`component-check`, which feeds each case's canonical AST bytes to a component exporting
`cadenza:compiler/compile : func(list<u8>) -> result<list<u8>, list<diagnostic>>` (the `compiler.wit`
world) and diffs against native `cdz-rustc`. The host side already exists (`component-check`,
`run_compiler_component`, `compiler.wit`). But the **seed can only emit an entry as the nullary `run :
() -> output`** — a `main` that takes the input AST bytes and returns the output component bytes (the
`compile : Bytes → Bytes` seam that IS the self-hosted compiler) declines *"the entrypoint `main` must
take no parameters"* (reproducer: `(module m (def (main b) b))`). So `compiler.cdz`'s `main` must
hardcode one program's bytes, and the corpus differential can't be driven the clean way.

**Why it touches the seed.** This is the top-priority self-hosting *verification* infrastructure gap.
Without it, every emit-frontier feature (item 20) is verified by hand-patching bytes into `main` — an
interim harness (`run_corpus.py`) that, as measured, MIS-classifies (it reports ~147 "disagree" / 0
"mine-declines", counts drift between runs, and "mine" component sizes cluster at 88–102B while native
ranges 89–3332B — the patched bytes mostly never reach the compiler's decode path, so a degenerate stub
is scored as a disagreement). Trusting that table would be the modeled-subsystem trap; only its AGREE
set (real byte-identity) is reliable.

**Status.** 🔴 Seed work (SEED-GAPS gap 3l), top priority for verification. **No corpus case** (it is
compiler infrastructure, not a language behavior; and the interim harness's output is not an oracle to
pin). Fix: when `main` takes one `Bytes`/`list<u8>` parameter and returns `Bytes`/`list<u8>`, lift it as
the `cadenza:compiler/compile` export of the compiler world (or a dedicated `(def (compile ast) …)`
entry / flag), matching what `run_compiler_component` looks up (interface `cadenza:compiler/compile`,
then bare `compile`, then `run`). Once it lands, `component-check` runs `compiler.cdz` over the corpus
as the real differential gate, replacing the byte-patching harness. Learning:
`spec/learnings/2026-07-07-verifying-the-self-hosted-compiler-needs-a-compile-exporting-component.md`.
