# Instruction selection emits against a fixed runtime — fixed helpers for control flow, scratch-or-decline for guards, and one home for the encoding

*2026-07-09*

**What happened.** Building `rcdzc`'s bottom rungs (`Mir → select → Lir → serialize → bytes`) settled a
set of decisions about how a lowered IR becomes *valid* WebAssembly component bytes — the part a
clean-room restart struggles with most, because a flat instruction rung and a byte-exact target punish the
obvious shortcuts. Four decisions held:

1. **A flat instruction rung, and control flow it can't express is a fixed helper.** The `Lir` ISA has
   `If`/`Else`/`End`/`Unreachable` but no `block`/`loop`/`br`. A runtime operation that needs a real loop —
   UTF-8 validation, integer-to-decimal — is *not* handled by adding loop variants to `Lir`. It is emitted
   as a **fixed compiler-emitted helper** written in raw structured control flow, baked into the runtime
   component, and `select` emits only a `Call` plus a two-way branch. The set of helpers is a closed,
   program-independent part of the runtime (the `putu`/`itoa`/`utf8-valid` precedent). The one design note
   on the validator is itself reproduction-critical: its loops are *guarded* so they run to a clean finish
   with no multi-level `br` — the depth bookkeeping that makes a hand-emitted validator wrong.

2. **Scratch-locals-or-decline for guarded ops.** Checked arithmetic and shifts need scratch locals
   reserved past params and lets; the guard is emitted inline as a *bounded* sequence. But the reservation
   must be **exactly what the client needs** — when shifts reused checked-arithmetic's uniform three-slot
   reservation, right-shift over-declared one unused local and fell out of byte-identity (value-correct but
   byte-different). And an operation whose correct emission would need a *loop* the flat rung can't express
   (e.g. `Bytes.of`/`Set.of` of a runtime, non-literal list) must **decline**, never emit a plausible
   sequence.

3. **The component envelope is derived from the runtime contract, and its indices are computed.** The
   fixed component-model wrapping around a program's core module — imports, instantiation, export lifting —
   is generated from the runtime interface's declaration (the WIT), not hand-maintained. Every index it
   depends on (the number of runtime imports, the base position of the first program-defined function) is
   *derived* from that interface. This paid off exactly as a drift-avoidance measure would predict: the
   prose comments still say "42 imports + 3 helpers, base 45" while the *generated* constants are 53 imports
   + 4 helpers, base 57 — the numbers drifted, the mechanism did not, because nothing reads a literal.

4. **The encoding lives in exactly one pass, and a signed constant uses signed LEB.** Every pass above
   `serialize` reasons in named `ValType`/`BlockType`; the raw encoding byte lives only in `ValType::byte`.
   The one hazard that bites hand-emitted bytes: `i32.const`/`i64.const` take a *signed* LEB, so a raw byte
   ≥ 64 has its high bit set and sign-extends negative — `(Bytes.of (list 65 66 67))` once rendered
   `b"\x41\x42\x43"` instead of `b"ABC"` because a printable-range guard emitted raw `I32_CONST, 126`
   (= −2). The rule: never hand-emit a constant ≥ 64 as a raw byte; always route through signed LEB.

Emission is validated **byte-identical to an independent reference encoder** at a covering set of small
cases (a scalar component vs. the prior compiler; the N-export envelope vs. a `wasm-encoder` oracle at
N=1,2). That oracle is what *licenses* hand-encoding the envelope — carrying no external encoder in the
emitter's own byte path, which is what lets it port 1:1 to the Cadenza self-host.

**Why.** These are the decisions where "just make it emit" produces a compiler that is subtly wrong rather
than obviously broken, so a restart that doesn't know them pays for them in miscompiles. Adding loop
variants to the flat rung to handle one byte-loop would grow the ISA, the validator-scratch juggling, and
every downstream pass, for a construct that a fixed helper handles with a plain call — and the helper is
generated once and shared, where open-coded control flow is a fresh chance to get the branch depth wrong.
Reserving scratch as the *union* of clients' needs is the reuse that costs byte-identity on the client that
needs less, so the last mile to byte-fidelity is per-client tailoring, not shared machinery. Deriving the
envelope and its indices from the runtime contract is the same keep-it-concrete/no-drift instinct the ladder
applies upward, applied to the bottom: a literal index is a second source of truth that drifts from the
interface, and the stale "42/3/45" comments are the proof it does. The signed-LEB hazard is the canonical
"the encoding is subtle, so it lives in one audited place" case — every hand-emit site that reaches past the
serializer re-exposes it. And byte-identity to an independent encoder is what turns "we hand-wrote the
component envelope" from a trust into a checked fact, which is the precondition for the self-host emitting
its own bytes with no encoder dependency.

**The requirement it drove.** New normative section in
[reference-compiler.md §Instruction Selection Emits Against A Fixed Runtime And Envelope](../architecture/reference-compiler.md):
a value's machine representation follows its solved type at selection; a consuming operation retains a
shared operand first (the conservative-Perceus rule); control flow the flat rung can't express is a fixed
helper, not a new instruction; a guarded operation reserves bounded scratch or declines; the component
envelope is derived from the runtime contract with computed (never literal) indices; emission is validated
byte-identical to an independent encoder and a signed constant uses signed LEB. Realizes
[compiler-pipeline.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md)
and the byte-identity/reproducibility of [constitution §II](../../constitution.md); composes with
[value-heap-runtime.md](../architecture/value-heap-runtime.md) (the runtime it emits against). The concrete
helper set, the scratch counts, and the envelope byte-layout are declared-default/internal, not normative.
