# Decision — Debug Information Format

**The decision.** The concrete interchange format the compiler emits source-level debug information in,
how that information is carried in a derived artifact (embedded or as a sidecar), how it is stripped
back to the undecorated artifact, and the external tooling that consumes it. The constitution and the
debug-information capability fix the *behavior* — debug information relates an execution position to
the source span it derives from, is inert (it changes neither the executed bytes nor the manifest),
carries no provenance, is a deterministic function of source and toolchain, cannot be read by the
running component, and is strippable back to the byte-identical undecorated artifact
(debug-information.md; [Core Principle II](../../constitution.md),
[Core Principle VI](../../constitution.md), [Core Principle VII](../../constitution.md)). They do not
fix the *format*: which interchange encoding the debug information is written in, which sections of the
artifact carry it, and which debuggers read it. That format is the choice this decision pins.

**Why the language wants it.** A stepping debugger, a crash backtrace that names source constructs, and
a profiler that attributes time to source all need a map from a position in the running artifact back
to the source it derives from. Cadenza's runtime-fact story is already deterministic replay
(tooling-and-lsp.md §Deterministic Replay Is The Debugger): an agent observes any runtime fact by
replaying a run from its recorded inputs. Debug information is the complement a *human* debugger and an
*external* tool need — the symbol-level metadata that lets an existing off-the-shelf debugger relate the
artifact to its source without bespoke Cadenza support. Emitting it in a standard interchange format
means the ecosystem's debuggers work on a Cadenza artifact for free, rather than requiring every tool
to learn a Cadenza-specific format.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Debug information MUST be inert: the bytes the runtime executes are byte-identical whether or not it
  is emitted, it adds no host operation to the manifest, and the running component cannot read it
  (debug-information.md §Emitting Debug Information Does Not Change Observable Behavior, §A Running
  Component Cannot Observe Its Own Debug Information). A choice therefore carries debug information in a
  region of the artifact the execution engine does not run — a non-executed metadata section — so that
  adding or removing it cannot change execution behavior.
- A recorded source location MUST be a source span over the canonical representation, and a file
  reference MUST be the tree-relative module path fixed by the source-tree-encoding contract rather than
  an absolute filesystem path (debug-information.md §A Source Location Is A Span Over The Canonical
  Representation, §A File Reference Is A Tree-Relative Module Path). A textual location a debugger shows
  is the printer's rendering of that span, not a second authority.
- A source-level name or type carried for the debugger MUST NOT be reachable by the running component
  (debug-information.md §Debug Information May Carry Source-Level Names And Types), so carrying names and
  types for an external tool does not reintroduce the runtime type reflection erasure removes: the
  metadata sits beside the code, never inside the value representation.
- Debug information MUST be a deterministic function of source and toolchain, its entry order MUST be
  source-determined, and it MUST carry no wall-clock time, absolute path, build-host identifier, or
  producer string (debug-information.md §Debug Information Is A Deterministic Function Of Source And
  Toolchain, §Debug Information Carries No Provenance). This is the reproducible-derivation discipline
  (reproducible-derivation.md §Provenance Is Stripped Or Normalized) applied to the metadata section:
  the format's provenance fields are normalized to a fixed value, and any embedded path is mapped to its
  tree-relative form, so two derivations of the same source with the same toolchain and the same
  emit-debug-info choice produce a byte-identical artifact.
- Stripping the debug information MUST yield the byte-identical artifact the same source derives with
  debug information excluded (debug-information.md §Stripping Debug Information Recovers The Undecorated
  Artifact). The stripped artifact is the reproducibility anchor: it is content-addressed and
  independently re-derivable, and the debug-carrying artifact is that same artifact plus a strippable
  metadata section. A debug-carrying artifact and its stripped form are distinct content addresses;
  the stripped form is the one a verifier re-derives.
- The format MUST be an interchange debug-information format an external debugger already consumes, not
  a Cadenza-private form, and MUST be pinned here so two builds emit the same format
  (debug-information.md §Debug Information Uses An Interchange Format).
- Whether to emit debug information MUST be a user-facing build choice carrying a declared default, and
  the capability MUST be optional (debug-information.md §Whether To Emit Debug Information Is A
  User-Facing Choice, §This Capability Is Optional).

**Why this is an isolated decision.** Debug information is a non-executed metadata section over the
existing runnable form: the executed bytes, the entry signature, the calling convention, the boundary
layout, and the manifest are all unchanged, so it touches no frozen contract and needs no ABI version
increment (component-abi.md pins the *executed* boundary; a metadata section the engine ignores is
outside it). It adds no new value form, no node kind, no diagnostic code, and no trap. Its only
reproducibility obligation — determinism and no provenance — is the discipline the compiler already
applies to the component it emits, extended to the metadata section. Changing the concrete format is an
edit to the choice file here plus the emitter's metadata pass; nothing a program means depends on it. It
is realized by a later generation, not the seed (`options/realized-capability-set/`): the seed clears
ignition without emitting debug information, and until a generation realizes this capability its
requirements are not load-bearing for that generation (conformance-gate.md §A Generation Is Judged
Against The Capabilities It Realizes).

## Choices

- [`dwarf-in-wasm-custom-sections`](./dwarf-in-wasm-custom-sections.md) — DWARF debug information carried
  in the standard `.debug_*` custom sections of the WebAssembly module (per the "DWARF for WebAssembly"
  convention), with a lightweight `name` custom section for symbol names; consumed by the default
  engine's DWARF support, browser DevTools, and LLDB; provenance normalized (`DW_AT_comp_dir` fixed,
  `DW_AT_name` mapped to the tree-relative module path, `DW_AT_producer` normalized); emittable embedded
  or as a sidecar module referenced by an `external_debug_info` section; stripped with the standard
  custom-section strip. **The default.**

DEFAULT: dwarf-in-wasm-custom-sections
