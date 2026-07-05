# Command — ignite

**Purpose.** Perform generation 0: synthesize the seed toolchain from the
bootstrap subset, gate it against both gates, clear the ignition bar, and stand
up a compiler that can thereafter regenerate itself. This is the operator-driven
half of the human-versus-toolchain seam (`spec/bootstrap.md` §"The
Human-Versus-Toolchain Seam"); every later generation uses `regen`.

**Agent-agnostic.** Neutral prompt body. Run by an operator's session, because no
Cadenza toolchain yet exists to derive it.

## How ignite differs from regen

1. **Input scope.** `ignite` reads the ignition subset — `constitution.md`,
   `spec/contracts/**`, `spec/bootstrap.md`, and only the ignition-subset
   capability specs named in `.duvet/bootstrap.toml` — not the deferred capability
   specs that later generations realize.
2. **Driver.** `ignite` runs in the operator's session; `regen` is driven by the
   previous generation of the compiler.
3. **Gate scope.** `ignite` gates against the bootstrap subset
   (`.duvet/bootstrap.toml`); `regen` gates against the full config
   (`.duvet/config.toml`).
4. **Output disposition.** `ignite` ends by clearing the ignition bar — one real
   derived-and-run component; `regen` ends by proposing the derived generation for
   promotion.

## Procedure

1. Synthesize the seed toolchain source into `implementation/`, authored in the
   declared seed host language (`options/bootstrap-strategy/`) because no Cadenza
   toolchain yet exists to derive it: a reader for the binary AST and its
   self-contained symbol prelude (`spec/contracts/ast-encoding.md`,
   `options/ast-encoding/binary-sexpr.md`), the **static-typing floor** that rejects an
   ill-typed program at compile time (constitution §VII), realized incrementally,
   **compiled derivation** — the seed's derivation mode — that generates a component
   whose imports mirror its manifest and whose observable behavior agrees with the
   recorded corpus semantics (`spec/bootstrap.md` §"Compiled Output Agrees With The
   Recorded Semantics"), and the machine-readable diagnostics. No pre-existing Cadenza compiler is required at genesis
   (`spec/bootstrap.md` §"The Toolchain Builds The Next Generation, Not Itself At
   Genesis"), and there is no compiler-free root to stand up — Cadenza is itself the
   build tool, outside any minimal load-verify-run root
   (`build-tool-interface.md` §"The Tool Is Replaceable").

   **Two distinct artifacts, and do NOT collapse them (see
   `spec/learnings/2026-07-03-real-components-not-a-bespoke-module-model.md`,
   `spec/learnings/2026-07-03-bootstrap-targets-the-compiler-directly.md`):**
   (a) **the compiler's codegen** — the compiler generates a core module and wraps it
   into a **real component** (`options/execution-model/`) whose interface world
   declares exactly the program's granted capabilities, so "imports mirror the
   manifest exactly" holds natively (the world *is* the import set), with no
   per-program import surgery; and (b) **a minimal host** that provides only the
   capability functions a component imports, a separate artifact from the compiler and
   the interpreter, binding exactly the manifest's capabilities. Emitting a component
   that only *replays* an output precomputed at derivation time is a **modeled
   derivation** and MUST NOT be used (`spec/bootstrap.md` §"A Modeled Derivation Is Not
   An Ignition"); the guard for compiled derivation is **oracle agreement** — the
   generated component's observable behavior MUST match the recorded corpus semantics
   over the same input (`spec/bootstrap.md` §"Compiled Output Agrees With The Recorded
   Semantics"). The distinguishing check: deriving two different programs MUST
   reuse the *same* codegen and differ only in the compiled program logic, so behavior
   comes from the program, not from a transcript the derivation baked in.
   (An optional reference interpreter — embedding the interpreter over the AST — is an
   independent oracle only, never the runtime, `spec/bootstrap.md` §"A Reference
   Interpreter Is An Optional Independent Oracle"; where a generation offers it, the
   anti-transcript rule in `build-tool-interface.md` §"A Derived Component Computes Its
   Behavior When It Runs" and
   `spec/learnings/2026-07-02-decouple-interpreter-wasm-from-host.md` apply.)
2. Run `setup-gate` for that host language so the bootstrap gate's `[[source]]`
   half points at `implementation/`, and cite every frozen-contract and
   ignition-subset requirement the seed satisfies by quoting its sentence. Derive
   the seed toolchain itself to a content-addressed component and record its
   toolchain identity — the compiler-component hash plus the host-toolchain hash —
   so every component a later generation derives can record it as its producer
   (`spec/contracts/reproducible-derivation.md` §"Derivation Is A Function Of
   Source And Toolchain"; `options/toolchain/`).
3. Run both gates over the bootstrap subset. First the requirement gate:
   `duvet report --config-path .duvet/bootstrap.toml`. Iterate until every
   MUST/SHALL in the subset is covered with zero broken citations. Coverage MUST be
   honest per `conformance-gate.md` §"A Citation Discharges Its Own Requirement":
   every citation annotates the code that performs its behavior, and every cited
   test fails when that behavior is removed. A gate that passes on placeholder
   anchors or one-shared-exercise-per-file citations has NOT passed; regenerate the
   coverage as real, per-requirement tests. Then the behavior gate: derive and run every
   case in `spec/semantics/*.sexp` and confirm each reproduces its recorded output
   (`conformance-gate.md` §"The Behavior Gate"). Both MUST pass.
4. Perform the ignition run: derive a real Cadenza source program to a
   content-addressed component and run it to produce its output, so the seed
   toolchain has demonstrably built and run a Cadenza program and can thereafter
   produce its next generation via `regen`.

## Ignition check

Ignition is not the appearance of a derivation; it is one executed. After the
ignition run, confirm, per `spec/bootstrap.md` §"Ignition Demonstrates A Real
End-To-End Derivation" and §"A Modeled Derivation Is Not An Ignition":

- A real Cadenza source program was derived to a content-addressed component and
  that component was **actually run to produce its output** — not stood in for by
  emitting the artifacts a derivation would produce.
- **The component itself computed the output** — the derived component executes the
  program's compiled logic at run time; it is NOT a transcript of output the host
  precomputed. Confirm by deriving two different programs, observing that the same
  codegen is reused and only the compiled program logic differs, and that their
  behaviors differ purely from that program (see
  `spec/learnings/2026-07-03-real-components-not-a-bespoke-module-model.md`). Where a
  generation instead offers the optional interpreted-derivation mode, the equivalent
  guard is that the same interpreter component is reused and only the embedded AST
  differs (`spec/learnings/2026-07-02-decouple-interpreter-wasm-from-host.md`).
- The derived component's imports mirror its declared capability manifest, so the
  capability-binding is exercised rather than merely configured
  (`build-tool-interface.md` §"The Tool Produces A Component, A Manifest, And
  Diagnostics").
- Re-deriving the same source with the same toolchain produced a byte-identical
  component, so reproducibility is exercised rather than asserted
  (`reproducible-derivation.md` §"Derivation Is A Function Of Source And
  Toolchain").
- The derived component's observable behavior agrees with the recorded corpus
  semantics (the oracle) over the same input, and the whole derivation is
  reconstructable from its recorded steps (`spec/bootstrap.md` §"The Whole
  Regeneration Is Auditable").

Only an *executed* derivation is ignition: the seed toolchain can now build the
next generation of Cadenza. Emitting the artifacts a derivation would produce,
without a component that was actually derived and run, is a model, and MUST NOT be
reported as ignition.
