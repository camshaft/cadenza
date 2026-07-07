# Verifying the self-hosted compiler needs a `compile`-exporting component — the interim byte-patching harness mis-measures

*2026-07-07*

**What happened.** The spike reached the point of wanting to run `compiler.cdz` over the *whole corpus*
— the differential check that is the real self-hosting verification: feed each case's canonical AST
bytes to the Cadenza-authored compiler and diff its output component against native `cdz-rustc`,
byte-for-byte. The host already has exactly this: `component-check <component.wasm> <corpus-dir>` +
`run_compiler_component`, which drive a component exporting `cadenza:compiler/compile : func(list<u8>)
-> result<list<u8>, list<diagnostic>>` (the `compiler.wit` world) against every case. But it is
**blocked by seed gap 3l**: the seed can only emit an entry as the **nullary `run : () -> output`** — a
`main` that takes the input AST bytes and returns the output component bytes (the `compile : Bytes →
Bytes` seam that *is* the self-hosted compiler) declines *"the entrypoint `main` must take no
parameters."* So `compiler.cdz`'s `main` is forced to hardcode one program's bytes.

To work around that, the spike wrote an **interim harness** (`run_corpus.py`) that drives the same
comparison without the `compile` ABI: for each case, dump the program's AST bytes via the seed, *patch
them into `compiler.cdz`'s `main`* (via its `compile-bytes` reader), `emit` → MINE, `emit` the program
directly → NATIVE, and classify AGREE / MINE-DECLINES / NATIVE-DECLINES / DISAGREE. Its docstring names
the intent correctly: **DISAGREE is the real-bug signal; MINE-DECLINES is the coverage frontier.**

The harness reported ~147 DISAGREE and **0 MINE-DECLINES**, with counts drifting between runs (147 then
149; 27 then 25 AGREE). My first reading of this was **wrong and is corrected here** (the honest trail,
not smoothed over): I inferred the "mine" sizes clustering at 88–102 B meant the byte-patching *wasn't
reaching the decode path* — a harness artifact. A next-cycle probe showed the opposite: the bytes DO
reach the reader, which **miscompiles** them. A CBOR float `0xfb` (major 7, info 27) hits `read-node`'s
major-7 branch, which assumes a boolean (`0xF5`/`0xF4` = arg 21/20), so `arg 27 ≠ 21` decodes to
`NBool 0` → the program's `run()` returns **`false`** (verified). Strings/records/tuples have no reader
node, so they fall through to `NInt`/`NPrim`-of-`"?"` and emit an i64 stub. So the ~88 B "stub" is not
a *degenerate non-decode* — it is a *wrong decode*: a valid component computing the wrong thing. **The
147 DISAGREE are (mostly) real miscompiles, and `0 MINE-DECLINES` is the true, alarming signal** — the
compiler *never declines* an unsupported construct; it always emits something. (This corrects the claim
above; see [[2026-07-07-the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining]]
for the reject-don't-miscompile violation and its fix.) The harness's *count instability* and the need
for a clean `compile` component (below) still stand; what changed is that its DISAGREE axis is
signal, not noise.

**Why.** The lesson is a specific instance of the modeled-subsystem trap
([[2026-07-02-a-modeled-subsystem-passes-a-shape-check]]): **a verification harness is only as
trustworthy as its own validation, and a workaround that routes around the real ABI can report
confident numbers that measure the workaround, not the system.** The byte-patching harness produces a
classification table that *looks* like a differential result but whose "disagree" axis is dominated by
"the patched bytes never drove the compiler." The clean path — a `compile`-exporting component fed
through `component-check` — is trustworthy precisely because it exercises the *actual* `Bytes → Bytes`
seam the compiler will ship as; the interim path substitutes byte-patching for that seam and thereby
substitutes an artifact for the measurement. So the right response to "147 disagree" is not to chase
147 bugs (that is the trap — hand-investigating a mismeasured table) but to **fix the ABI gap so the
real harness can run**, and meanwhile trust only the harness's AGREE set (which *is* a real
byte-identity result) and treat its DISAGREE/DECLINE split as unreliable. This also connects to the
recurring "probe the real thing, not a proxy" rule that has run through the whole spike (probe the
rebuilt seed not the handoff doc; reduce the failing program not a clean analogue; a const case is not
evidence the runtime path works) — here: a byte-patching proxy is not evidence of what the `compile`
component would do.

**The requirement it drove.** No corpus case — this is compiler-verification *infrastructure*, not a
language behavior, and the harness's own output is not a reliable oracle to pin (pinning a mismeasured
table would encode the artifact). The durable output is **SPEC-BACKLOG item 22**: seed gap 3l — emit an
entry whose signature is `list<u8> → list<u8>` exported as `cadenza:compiler/compile` (the compiler
world), not the nullary `run`, so `component-check` can run `compiler.cdz` over the corpus as the real
differential gate. The host side (`component-check`, `run_compiler_component`, the `compiler.wit` world)
already exists; only the seed's ability to *emit that export* is missing — likely: when `main` takes one
`Bytes`/`list<u8>` parameter and returns `Bytes`/`list<u8>`, lift it as the `compile` export instead of
`run`. It is top-priority self-hosting *verification* infrastructure: until it lands, every emit-frontier
feature (backlog item 20) must be checked by hand-patching bytes, which the interim harness shows is
both laborious and mismeasuring. The learning also records the caution for whoever reads the interim
harness's output: **trust its AGREE set, not its disagree count, until gap 3l lets the clean
`component-check` replace it.**
