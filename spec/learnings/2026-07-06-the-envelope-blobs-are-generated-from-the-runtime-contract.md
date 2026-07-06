# The compiler's envelope byte-blobs are generated from the runtime contract, not pasted from ephemeral scripts

*2026-07-06*

**What happened.** The compiler emits a WebAssembly component by wrapping a per-program core module
in a FIXED component-model envelope. Those envelope bytes — the component HEAD/TAIL around the core
module, the import section the compiler puts *inside* its core module, the import indices, the core
signatures of the imported runtime functions, and the count of them — had all been **hand-derived and
pasted** into the compiler as opaque decimal arrays. The derivation ritual, each time the runtime
interface grew, was: author a reference component in WAT, assemble it with `wasm-tools`, split it at
the embedded core-module boundary with a throwaway Python script in `/tmp`, and copy the resulting
hex back into the source. Two envelope re-derivations
([the persistent vector](./2026-07-05-wiring-the-persistent-vector-re-derived-the-frozen-envelope.md)
and [the Bytes rope](./2026-07-06-wiring-the-rope-exercised-the-envelope-recipe-a-second-time-and-caught-a-no-op-compact.md))
had exercised that ritual and even declared it "mechanical" — but mechanical is not the same as
maintainable. The scripts lived in `/tmp` and evaporated between sessions; the arrays were unreadable;
and the *how* survived only in prose comments and a memory file, i.e. nowhere a program could check.

Worse, the interface those bytes encode — an ordered set of functions, each with a name and signature
— was duplicated by hand across **six** sites in the compiler and host that had to agree exactly or
the emitted component would be invalid: the import-index module, the core-signature table, the import
count, the three byte blobs, and the host's forwarding list. And the true source of the contract — the
runtime's own `wit/runtime.wit` — was a **seventh** copy that none of the six were derived from.
Appending one runtime operation meant six coordinated hand-edits against a copy that was not itself
tied to the contract, plus a fresh throwaway derivation.

The change makes the runtime **WIT the single source of truth**. A Rust generator, living in the
out-of-band `xtask` build tool, parses the WIT with `wit-parser`, takes the compiler's ordered
allow-list of the functions it lowers, builds each reference component with `wasm-encoder`,
self-validates it with `wasmparser`, and splits it at the core-module boundary (the `/tmp` splitter,
now a checked-in Rust function). It emits ordinary Rust source — the byte constants, the import
indices, the core signatures, the count, and the required-runtime hash — which the compiler and host
include as modules. The whole thing is one step inside the existing `build` command: build the runtime,
compute its content address, **generate the compiler's view of the contract from the WIT + that hash**,
then build the compiler against it. Appending a runtime op the compiler wants is now: add its name to
the allow-list, run `build`, re-verify the gates.

Three things fell out worth recording:

1. **The generator is generic over the contract; the compiler declares only its *selection*.** xtask
   knows nothing about `box-int` or `bytes-concat` — it reads whatever the WIT declares and lowers
   whatever names the allow-list selects, deriving every signature and index from the WIT. The one
   compiler-specific input is the ordered allow-list (which of the runtime's functions the envelope
   lowers, and in what order), because the compiler legitimately lowers a *subset*: it skips the
   `string`-typed ops (a heavier canon the envelope does not provide) and the not-yet-emitted
   reference-counting reuse ops. The generator errors if an allow-listed name is absent from the WIT
   or carries an unlowerable type, so the selection cannot silently drift from the contract.

2. **A re-baseline is fine when the invariant is validity, not byte-identity.** `wasm-encoder`
   produces a more compact HEAD than the `wasm-tools`-derived reference did (different type
   ordering/dedup), so the generated envelope bytes are *not* identical to the pasted ones. That is
   acceptable and was decided up front: the invariant is that each generated component **validates**
   (wasmparser, at generation time) and produces **correct output** (the four gates, after). The
   import section the compiler puts inside its own core module *did* come out byte-identical, because
   it is computed directly rather than split from a reference — a small confirmation that the two
   halves of the generator (reference-derived vs. directly-computed) agree where they overlap. And
   the self-hosting byte-identity property is untouched: the blobs are still baked constants, a
   deterministic function of source, so the compiler still reproduces itself exactly.

3. **A generated file must not be rewritten when it hasn't changed.** Because generation runs on every
   `build`, naively writing the output would bump the files' timestamps and force the compiler to
   recompile even when the contract was untouched, defeating the incremental cache. Write-only-if-changed
   — read the existing file, return early if the bytes match — keeps a no-op `build` a genuine no-op.
   This makes the drift check implicit: run `build`; if a generated file changes, the contract moved,
   and the gate re-run catches any behavioral consequence.

The change also retired a dead wire: the build tool had been passing the runtime's content address to
the compiler build via an environment variable that nothing read. The generator now bakes that hash
into the generated source as a constant, so the compiler↔runtime pin is a checked-in, deterministic
function of the runtime source rather than an ephemeral build-env value — ready for when the emitted
component records its required runtime.

**Why.** The deeper principle is that **a derived artifact whose derivation is an ephemeral script is
a latent defect even when its bytes are correct.** Correctness you cannot re-derive is unmaintainable:
the next person (or the next you) faces an opaque array and a comment claiming how it was made, with no
way to check the claim or reproduce the bytes. The fix is not better comments; it is to make the
derivation itself a checked-in, re-runnable, self-validating program, and to give the thing it derives
from a single home. This is the same lesson the specification already learned at a larger scale —
one executable semantics rather than four drifted models, one canonical representation rather than
several front-ends — applied here to the runtime interface: one contract (the WIT), one generator, one
generated view, instead of seven hand-kept copies. It also honors, rather than adds to, the frozen ABI
contract's existing stance that "the compiler builds components itself; the tool is a dev-desk oracle":
`wasm-encoder`/`wit-parser`/`wasmparser` live only in the build tool, never in the shipped compiler's
dependency graph, so the emitted-bytes path stays a pure function of the compiler alone.

**The requirement it drove.** None new — this is engineering technique realizing existing requirements.
It is how `spec/contracts/component-abi.md`'s "the compiler builds components itself" and
`spec/contracts/reproducible-derivation.md`'s "the runtime is derived first, its content address
computed, and the compiler built against it" are made concrete: the build tool derives the runtime,
then derives the compiler's view of the runtime's contract from the runtime's own WIT, then builds the
compiler — one command, one source of truth, self-validating. Recorded so the next runtime-interface
change is understood as an edit to the WIT + a one-line allow-list entry + `build`, never a hand-paste;
and so the two prior envelope re-derivation learnings are read as the *problem* this retires, not a
recipe to repeat.

The same generator was then turned on the other hand-pasted magic-value tables, which sharpened the
principle in two ways. **The opcode table** (`codegen.rs`'s `mod op`) was generated the same way, but
its source of truth is not a contract we own — it is the WebAssembly spec's opcode numbers, and the
authoritative encoder of those is `wasm-encoder` itself. So each opcode byte is derived by *encoding a
`wasm_encoder::Instruction` and taking its leading byte*: we curate only *which* ops the compiler uses
and *what we name them*, never the numbers. And because two compiler implementations both hand-encode
wasm — the Rust seed and the Cadenza-authored compiler — the opcode table is emitted into BOTH (`op.rs`
and `op.cdz`), so a code generator that emits multiple languages lets one source of truth feed every
implementation. That is the general shape the effort reaches for: a magic-value table pulled into every
implementation from one derivation. (Reaching the opcodes from the spec's own instruction-index
appendix, by fetch or vendored snapshot, is a possible later refinement; `wasm-encoder` is the offline,
already-pinned, spec-tracking source in the meantime.) **The reference-with-display envelope**
(`RUNNABLE_ENVELOPE_TAIL`, the all-constant resource-ABI path — a `value` resource owning
`display()->string`, with a nested inner component) was also folded in, even though it is not
interface-driven and never changes: the point is not that it churns but that its derivation should be a
checked-in program, not an opaque array. Building it surfaced one real invariant — the reference's
embedded core module must be the FIRST section after the component preamble, because the compiler
splices its own core module there and appends the tail, so anything the generator emits before the
module lands in the wrong half of the split. Composes with
[wiring the persistent vector re-derived the frozen envelope](./2026-07-05-wiring-the-persistent-vector-re-derived-the-frozen-envelope.md)
and
[wiring the Bytes rope exercised the recipe a second time](./2026-07-06-wiring-the-rope-exercised-the-envelope-recipe-a-second-time-and-caught-a-no-op-compact.md).
