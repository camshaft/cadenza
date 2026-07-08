# Debug Information — Choice: dwarf-in-wasm-custom-sections

> **The default choice for the `debug-information` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete interchange format the
> compiler emits source-level debug information in: DWARF carried in the WebAssembly module's standard
> `.debug_*` custom sections, per the "DWARF for WebAssembly" convention, consumed by off-the-shelf
> DWARF debuggers.
>
> Debug information is a non-executed metadata section the execution engine ignores, so emitting,
> stripping, or tuning it does not alter the bytes the runtime executes — it composes with the
> execution-model choice (`options/execution-model/`) without touching the component ABI.

## The format

The compiler emits **DWARF** debug information into the WebAssembly module as **custom sections**, the
mechanism the WebAssembly binary format defines for named, non-executed metadata the engine skips. This
is the same encoding LLVM/Clang emit for wasm, so the toolchain ecosystem's DWARF debuggers consume a
Cadenza artifact unchanged. The convention followed is **"DWARF for WebAssembly"**
(<https://yurydelendik.github.io/webassembly-dwarf/>), which fixes how DWARF's code-address model maps
onto a wasm module.

### Sections carried

| Section | Carries |
|---|---|
| `.debug_info` | the DWARF debugging information entries (DIEs): the tree of compilation units, subprograms (functions), variables, and types |
| `.debug_abbrev` | the abbreviation tables the `.debug_info` DIEs are encoded against |
| `.debug_line` | the line-number program: the map from a code offset to a `(file, line, column)` source position |
| `.debug_str` / `.debug_line_str` | the string table the other sections reference by offset |
| `.debug_ranges` / `.debug_rnglists` | the code-address ranges a DIE (a function, a lexical block) spans |
| `.debug_loc` / `.debug_loclists` | where a variable lives (which local/global/memory) over each range of the code |
| `name` | the WebAssembly `name` custom section — a lighter-weight symbol table naming the module, functions, and locals, independent of DWARF |

### The code-address model

DWARF was designed for native code addressed by a linear program counter; a wasm module has no such
address space. The "DWARF for WebAssembly" convention resolves this: a **code address in the DWARF is a
byte offset into the module's `Code` section**. The `.debug_line` program therefore maps a byte offset
of an instruction (measured from the start of the `Code` section) to the source position it derives
from, and `.debug_ranges`/`.debug_loclists` express their address ranges the same way. This is what
lets a debugger, stopped at a wasm instruction, name the source line — and it is why the mapping is
stable only against a fixed emission of the `Code` section, which the reproducible-derivation contract
already guarantees.

### The `name` section vs. full DWARF

The `name` custom section is the minimal symbol layer: it gives functions and locals their source names
so a stack trace reads `factorial` rather than `func[42]`, but it carries no line mapping, no types, and
no variable locations. Full DWARF is the complete stepping-debugger surface. The compiler emits the
`name` section whenever it emits DWARF (it is cheap and every DWARF consumer also reads it); a build that
wants only readable stack traces without the full DWARF payload MAY emit the `name` section alone — a
sub-choice recorded in the build's decision record.

## How the requirements are met

### Inert — the executed bytes are unchanged (debug-information.md §Emitting Debug Information Does Not Change Observable Behavior)

Custom sections are **not executed**: the WebAssembly specification requires an engine to ignore a
custom section for the purpose of validating and running a module (its contents are opaque bytes with a
name). The `Type`, `Function`, `Code`, `Memory`, `Import`, and `Export` sections — everything the engine
runs and everything that defines the component boundary — are byte-identical whether or not the
`.debug_*` and `name` sections are present. Adding DWARF therefore cannot change execution behavior, the
manifest (no import is added — DWARF adds no `Import` entry), or the entry signature. The running module
has no instruction that reads its own custom sections, so a component cannot observe its own debug
information (debug-information.md §A Running Component Cannot Observe Its Own Debug Information); the
sections are visible only to a tool inspecting the module from outside.

### Source spans and tree-relative paths, not build-host paths (README requirements 2 & 4)

- A `.debug_line` row's source position is the printer's rendering of the **source span over the
  canonical representation** the instruction derives from, so the location is stable under any textual
  syntax (debug-information.md §A Source Location Is A Span Over The Canonical Representation).
- The **`DW_AT_name`** of a compilation unit and any file entry in the `.debug_line` file table is the
  **tree-relative module path** fixed by the source-tree-encoding contract, never an absolute filesystem
  path (debug-information.md §A File Reference Is A Tree-Relative Module Path). This is the DWARF
  counterpart of `-ffile-prefix-map`: the emitter maps a source path to its tree-relative form rather
  than recording where the build happened to run.
- **`DW_AT_comp_dir`** (the compilation directory, conventionally an absolute path, the single largest
  reproducibility hazard in native DWARF) is normalized to a **fixed, source-independent value** (the
  empty string or a fixed sentinel root), so it carries no build-host path.

### Names and types stay out of the value representation (README requirement 3)

DWARF DIEs carry a binding's **source name** (`DW_AT_name` on a `DW_TAG_variable`) and its
**source-level type** (`DW_TAG_base_type` / `DW_TAG_structure_type` referenced by `DW_AT_type`) for the
debugger to present. These live in `.debug_info`, **beside** the code — never inside a value's runtime
representation, which stays the tag-free, name-free positional layout the value-heap runtime owns
(component-abi.md §The Runtime Does Not Name Or Render Values). The running module cannot reach a DIE, so
carrying names and types for the debugger does not reintroduce the runtime type reflection erasure
removes (debug-information.md §Debug Information May Carry Source-Level Names And Types).

### Reproducible and provenance-free (debug-information.md §Debug Information Is A Deterministic Function Of Source And Toolchain, §Debug Information Carries No Provenance)

- **`DW_AT_producer`** (the compiler-identity/version string) is normalized to a fixed value rather than
  the live toolchain's version banner, so it does not vary between builds of the same source. (Which
  toolchain produced the artifact is recorded by the reproducible-derivation contract's toolchain
  identity, not smuggled into a DWARF string.)
- No wall-clock time is embedded (DWARF has no mandatory timestamp; none is added).
- DIE order, the `.debug_line` program, and the string table are emitted in a **source-determined
  order** — the same order the compiler emits the `Code` section it describes — so two derivations of the
  same source with the same toolchain and the same emit-debug-info choice produce **byte-identical**
  `.debug_*` sections, hence a byte-identical debug-carrying artifact.

### Separable — embedded or sidecar, and strippable (debug-information.md §Debug Information May Be Embedded Or Emitted As A Sidecar, §Stripping Debug Information Recovers The Undecorated Artifact)

- **Embedded (default):** the `.debug_*` and `name` sections travel inside the module.
- **Sidecar:** the DWARF is emitted into a **separate `.wasm` file** carrying only the custom sections,
  and the runnable module carries an **`external_debug_info`** custom section recording the reference
  that links it to that sidecar (the convention's mechanism), so a tool holding the runnable module
  locates its debug information (debug-information.md §"The compiler MUST emit a reference, reachable from
  the runnable artifact, that identifies the separately emitted debug artifact describing it"). Placing
  the reference in the runnable is this format's realization of that requirement — the requirement fixes
  that the reference exists and is reachable from the runnable, not which artifact physically holds it. A build ships the
  runnable artifact lean and the sidecar alongside it.
- **Stripping** removes the debug-metadata custom sections (`.debug_*`, `name`, `external_debug_info`)
  with a **section-surgery** tool that rewrites nothing else — `wasm-tools strip` (its `--all` removes
  the `name` section too, which its default preserves), `llvm-strip`, or an equivalent section remover —
  yielding the **byte-identical** module the same source derives with debug information excluded. A
  strip MUST be performed by a section remover rather than by an optimizer that re-serializes the
  module (a re-encoding pass can renumber or re-lay-out the `Code` section and so is not guaranteed to
  preserve the executed bytes), so that the recovered undecorated module is byte-for-byte the one the
  source derives without debug information. The stripped module is the **content-addressed,
  independently re-derivable** artifact the reproducible-derivation contract anchors on; a
  debug-carrying artifact is that module plus a strippable metadata section and has its own, distinct
  content address.

## Tooling that consumes it

- **The default engine's DWARF support** — a component-model runtime run with its DWARF-debug flag (e.g.
  wasmtime's `-D debug-info`) maps the embedded DWARF to a native debugger (GDB/LLDB), so a wasm trap or
  breakpoint stops at a Cadenza source line.
- **Browser DevTools** — Chromium DevTools with the **"C/C++ DevTools Support (DWARF)"** extension reads
  the embedded DWARF and steps through the original source.
- **LLDB** — reads the wasm DWARF directly.
- **`wasm-tools` / `wasm-objdump`** — inspect and strip the sections; `wasm-objdump -x` lists the custom
  sections and `wasm-tools strip` removes them.

## Component Model note

A WebAssembly **component** wraps one or more **core modules**; the DWARF lives in the `.debug_*` custom
sections of the core module(s) the component contains, exactly as for a bare core module, and the
component's own custom sections may additionally carry component-level `name` metadata. Debugging
support for the raw core module is the mature path; component-level debugging is evolving in the
ecosystem, so a build that targets the component model MAY emit DWARF at the core-module level (always
available) and treat richer component-level debug metadata as an additive refinement of this choice as
the tooling lands — without any change to the capability's requirements, which are stated over "the
artifact" and are format-agnostic.

## Why this choice

- **Zero-cost when off, and inert when on.** DWARF is a custom section the engine never executes, so a
  build that does not ask for it pays nothing and a build that does changes no executed byte — the
  capability's inertness requirement is satisfied by construction rather than by discipline.
- **The ecosystem already reads it.** DWARF-in-wasm is what LLVM emits and what browser DevTools,
  wasmtime, and LLDB consume, so "an external debugger relates the artifact to its source without bespoke
  Cadenza support" is met by using the format those debuggers already speak.
- **Reproducibility is a known, solved problem.** The three DWARF fields that break reproducible builds —
  `DW_AT_comp_dir`, file-path `DW_AT_name`, and `DW_AT_producer` — are exactly the ones this choice
  normalizes, the same normalization reproducible-build toolchains apply, so the metadata section inherits
  the reproducibility the runnable form already has.
