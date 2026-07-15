# Debug information for the wasm output — DWARF + `name` sections, injected by the backend

**Status:** design/scoping only — nothing landed. Written 2026-07-11 on branch `spec` (gate 135/0,
205 tests, after the value-heap H1 landing). Realizes the already-normative capability
`spec/capabilities/debug-information.md` and its pinned format
`options/debug-information/dwarf-in-wasm-custom-sections.md` (the "DWARF for WebAssembly" convention,
<https://yurydelendik.github.io/webassembly-dwarf/>). Line numbers are landmarks at `9f76c1b`.

The developer-facing goal: a Cadenza `.wasm` component that **just works in GDB/LLDB and the Chrome
debugger** — stops at the actual Cadenza source line, steps through it, and inspects values in a way
that makes sense to a Cadenza developer. This doc scopes what "just works" reachably means (scalar
stepping is near; compound-value inspection is a real fork — §3) and lays out the increments.

> **The operator's ruling (2026-07-11), verbatim intent.** "I think the span table should be another
> artifact and the compiler would be responsible for injecting it." — i.e. the span table crosses the
> front-end→compiler boundary as **its own kinded input artifact** (keyed by the same `StructId`, so
> the binary AST stays span-free exactly as `ast.rs` intends), and the **backend injects** the DWARF /
> `name` custom sections. This is a third path, cleaner than either "assemble DWARF in the driver" or
> "extend the codec to smuggle spans into the AST": spans never re-enter the span-free `Db` columns,
> and debug emission stays a first-class compiler responsibility, hand-rolled in the byte path so it
> ports 1:1 to the eventual Cadenza self-host (no new host encoder dependency).

> **The operator's second ruling (2026-07-11).** "I want to make it optional as well, and this ties
> into the whole sidecar API — enabling debug info would be some call/query in the sidecar that drives
> it." So *enablement is a sidecar directive, not a `--debug` build flag.* This converges debug-info
> with `DESIGN-query-engine.md` (the compiler is already a query engine: `engine(target: Ast, sidecar:
> Program, output: Format) → Bytes`, the sidecar being the driving program that today says "lower to a
> component"). Enabling debug info is that same directive carrying one more request. Optionality is
> already normative at the *capability* level (debug-information.md §This Capability Is Optional + a
> declared default, defaulted *off* by seed-ignition-set.md); this ruling fixes the *enablement
> mechanism*. Full treatment in §9.

---

## 1. Why this is reachable — three facts, and three walls

### The three facts that make it feasible

1. **The node *identity* already survives to the last instruction.** Every rcdzc IR is a *column*
   over the AST's own `StructId`, not a fresh id space: `Core` nodes reference children by `StructId`
   (`core.rs:51`), and `select::emit` (`backend/wasm/select.rs:106`) walks those `StructId`s to emit
   flat `Lir`. So at the moment an instruction is chosen, the source node it derives from is in scope
   — the one thing DWARF's line program fundamentally needs. (It is *discarded* the instant a `Core`
   node becomes `Lir` today — that is the gap D1 closes; see §2.1.)

2. **The span byte-ranges already exist, keyed by that same identity.** `cadenza-syntax`'s `SpanTable`
   (`spans.rs:21`) is a total map `StructId → Span{start,end}` (byte offsets), plus a `FileId`. The
   parser records one span per occurrence in id order, so `spans[id]` is exactly that occurrence's
   source range — repeated leaves get distinct ids and distinct spans. This is the same substrate
   diagnostics key off. Program node ids are `0..user_node_count` (`db.rs:208`), a clean boundary
   below the appended prelude (`is_user_node`, `db.rs:289`).

3. **Custom sections are inert *by construction*, and hand-rolled emission makes stripping trivial.**
   Appending `.debug_*` / `name` (section id 0) sections does not move a single byte of the
   Type/Import/Function/Export/Code sections the engine runs. So two capability MUSTs hold
   automatically, not by discipline: "executed bytes byte-identical with/without debug"
   (debug-information.md §Emitting Debug Information Does Not Change Observable Behavior) and "strip
   recovers the byte-identical undecorated artifact" (§Stripping Debug Information Recovers The
   Undecorated Artifact). We append and recompute exactly one length prefix (the embedded core
   module's, `envelope.rs:269`). No optimizer/re-serializer is ever involved — which is *why* the spec
   forbids stripping via re-serialization (it renumbers `Code`).

### The three walls to design around

1. **The compiler is deliberately span-free, and the codec drops spans.** `rcdzc` decodes a span-free
   binary AST (`codec::decode`, `compile.rs:40`); the `SpanTable` lives only in `cadenza-syntax` and
   never crosses the byte bridge. Diagnostics already live with this — the compiler emits only a node
   *index*, the consumer resolves it (`abi.rs:51`). **The operator's ruling resolves this without
   softening it:** the span table crosses as a *sibling artifact*, not folded into the AST — so the
   `Db` columns stay span-free, and only the backend's new debug-emission path reads the span sidecar.

2. **Component-model DWARF debugging is younger than core-module DWARF.** Our artifact is a component
   embedding a core module (`envelope.rs`); DWARF lives in the *embedded core module's* `.debug_*`
   sections, and code addresses are byte offsets into *that* core module's `Code` section. GDB/LLDB via
   `wasmtime -D debug-info` and the Chrome "C/C++ DevTools Support (DWARF)" extension are proven for
   **bare core modules**; seeing through the component wrapper is the open empirical question (§6).
   Clean fallback: the debug workflow can consume a **bare core module** build (identical DWARF + Code;
   the component envelope is only instantiation glue).

3. **Heap-backed compound values are opaque handles — the wall for "inspect *any* value."** Scalars
   (ints, bool) live in wasm locals as i32/i64, so a `.debug_loc` → local-slot entry gives a real
   `print n`. But tuples/records/sums/lists/strings are **u32 handles into the runtime's tagless
   CHAMP/RRB heap** — type-erased, name-free, positional, owned by the runtime. A stock DWARF debugger
   sees a `u32`, not a record, and cannot walk that heap. Making compounds inspectable is a genuine
   fork (§3), and the spec already anticipates it: `.debug_*` serves *stepping*; deterministic *replay*
   serves *value inspection* (`tooling-and-lsp.md` §Deterministic Replay Is The Debugger).

---

## 2. The increments

Sequenced cheapest-first, each inert, each independently landable behind an off-by-default build
choice with a decision-record entry (debug-information.md §Whether To Emit Debug Information Is A
User-Facing Choice). All byte emission is hand-rolled in `backend/wasm/` via the existing
`encode::section` choke point (`encode.rs:48`), so it ports to the self-host.

### 2.1 The enabling primitive — the span sidecar + the `offset → StructId` table

Two halves, one for each direction of the new plumbing.

**(a) The span table as a kinded input artifact (the operator's ruling).** Today `compile()` takes a
set of kinded input `Artifact`s and picks the `ast` one (`abi.rs:32` `KIND_AST = "ast"`;
`compile.rs:35`). Add a **`KIND_SPANS = "spans"`** input carrying, in a hand-rolled byte layout that
ports to self-host: the `SpanTable` (a `Vec<(start,end)>` positionally indexed by `StructId`, so it
aligns 1:1 with the decoded arena) **plus the tree-relative module path** for the `FileId`
(source-tree-encoding.md §"MUST include each module's tree-relative path" — this is the path the
DWARF file entry records, never an absolute filesystem path). The front-end (`parser.rs` /
`sexpr.rs`, which build the `SpanTable`) emits it; the driver passes it alongside the `ast` artifact.
When absent (the common no-debug build), the backend simply cannot emit debug sections and the
artifact is exactly today's bytes.

> Note the front-end reader currently records **byte-offset** spans, not `(line,col)`. The line/col a
> `.debug_line` row needs is derived at emit time by a one-pass newline index over the module's source
> text. Whether that text also rides in the spans artifact or is re-read by the driver is a small
> sub-decision for D2 (the reproducibility rules in §4 apply either way).

**(b) The `offset → StructId` line table (the gap in the byte path).** `Lir` (`lir.rs:52`) carries no
source id and `SelectedFunc` (`select.rs:32`) carries none. Two touch-points:
  - `select::emit` (`select.rs:106`) is the *last scope holding the `StructId`* per instruction. Have
    it record, per emitted instruction (or per contiguous run from one node), the originating
    `StructId` — e.g. a parallel `Vec<StructId>` beside `SelectedFunc.code`, or a `Vec<(usize lir_ix,
    StructId)>`. This is additive; it does not change the emitted `code` vector.
  - `serialize::instr` / `code_entry` (`serialize.rs:50` / `:139`) is where `Lir` → bytes, so it is
    where the **running code-section byte offset** of each instruction is known. Thread a callback (or
    return a `Vec<(u32 code_offset, StructId)>` + the code section's base offset) so the backend ends
    with `code_offset → StructId` for the whole module.

Compose (a)+(b) at emit time: `code_offset → StructId → span → (tree-relative file, line, col)`. That
tuple stream *is* the DWARF line program's input. **D1 lands this side-table as an inert internal
value** (and optionally as a debug-only kinded *output* artifact for inspection), with **no DWARF yet**
— proving the primitive and the strip round-trip before any encoding work.

### 2.2 D0 — the `name` custom section (readable frames, DWARF-independent)

Emit the wasm `name` custom section (id 0, name `"name"`): the module-name + function-name
subsections from `db.defs` / `layout.exports` (`layout.rs:29` already holds export names verbatim;
internal reachable callees are anonymous today — give them their source def name), optionally the
local-name subsection. Tiny, needs only (a)'s def-name access, and immediately turns `func[42]` into
`factorial` in every trace and profile. **Highest value-per-line; land it first, even before D1's
line table.** The format doc explicitly blesses `name`-alone as a sub-choice for "readable stack
traces without the full DWARF payload."

### 2.3 D2 — `.debug_line` + minimal `.debug_info` (source stepping — the payoff)

The core of "step through the actual source." Using the §2.1 tuple stream, emit the DWARF line-number
program and the minimum DIE tree a debugger will accept:
  - `.debug_line` — the line program mapping each code offset to `(file, line, col)`; the file table
    entry is the tree-relative module path.
  - `.debug_abbrev` + `.debug_info` — one `DW_TAG_compile_unit` DIE (with normalized `DW_AT_comp_dir`,
    `DW_AT_producer`, `DW_AT_name`; see §4) and one `DW_TAG_subprogram` per function with its
    `DW_AT_low_pc`/`DW_AT_high_pc` as Code-section offset ranges.
  - `.debug_str` / `.debug_line_str` — the string table the above reference by offset.

All appended to the **embedded core module** when the sidecar directive asks for debug (§9); custom sections ride inside the core-module blob
transparently — the standard place tools expect wasm DWARF), then fix the component's core-module
length prefix (`envelope.rs:269`). After D2, GDB/LLDB (via `wasmtime -D debug-info`) and the Chrome
DWARF extension stop at Cadenza source lines. Hand-rolling the DWARF LEB/line-program bytes is the
bulk of the work but is exactly the byte discipline the backend already lives by; the `wasm-encoder`
oracle does **not** cover DWARF, so the oracle here is **round-trip through `llvm-dwarfdump` /
`wasm-tools` + real consumption** (§6), not byte-identity to an encoder.

### 2.4 D3 — variable locations + base types (scalar value inspection)

Extend §2.1(a)'s function metadata with per-function `local slot → (source name, Ty)` — recoverable in
`select_function` (`select.rs:50`), where `slot_of: HashMap<StructId,u32>` still maps a binder
occurrence to its slot (`let`-bindings are keyed by their initializer `StructId`, so their source names
resolve too). Emit `DW_TAG_variable` DIEs with `DW_AT_location` (the local slot, in the wasm loclist
form) and `DW_AT_type` (`DW_TAG_base_type` for each integer width / bool). Now `print n` works for
scalar params, `let`-bindings, and match binders — real value inspection for the scalar language we
have today.

### 2.5 D4 — compound value inspection (the fork; see §3)

Deferred behind the value heap and a deliberate choice. Do **not** try to make DWARF describe the
tagless heap.

**Cross-cutting, folded into D2:** sidecar mode (emit a debug-only `.wasm` of just the custom sections
+ an `external_debug_info` custom section in the runnable pointing to it — debug-information.md §May Be
Embedded Or Emitted As A Sidecar); the strip round-trip test (below); and the enablement path (a
sidecar directive over the kinded-artifact ABI — §9 — not a build flag).

---

## 3. The compound-value fork (what "inspect values sensibly" can mean)

For the developer's literal ask — inspect values the way a *Cadenza* developer thinks — scalars are
solved by D3, but compounds hit wall #3. Three ways to cross it, in order of spec-alignment:

- **Replay-based value inspection (spec's designated path, recommended).** `tooling-and-lsp.md`
  §Deterministic Replay Is The Debugger already assigns *value inspection* to deterministic replay,
  reserving `.debug_*` for *stepping*. A compound value renders through the runtime's own render ops
  (the boundary already renders values for `cdz-run`), not through DWARF walking the heap. This keeps
  type erasure intact and matches the spec's division of labor.
- **Debugger pretty-printers (pragmatic add-on).** Ship GDB/LLDB Python pretty-printers (and/or a
  Chrome formatter) that, given a handle `u32`, call runtime render/inspect ops to display the
  structured value. Off-the-shelf-debugger-native, but bespoke per debugger and lives outside the
  artifact.
- **DWARF `DW_TAG_structure_type` over the heap (rejected).** Teaching DWARF the heap layout would
  re-encode the positional/tagged structure the value heap deliberately keeps runtime-private —
  fighting erasure and the "runtime does not name or render values" invariant. Not this.

Recommended framing for the roadmap: **DWARF delivers frames + stepping + scalar locals; compounds are
inspected via replay (optionally sugared by pretty-printers).** This is honest about the wall and
leans on machinery the spec already blesses.

---

## 4. Reproducibility — the three DWARF hazards, pre-normalized

Debug info MUST be a deterministic function of source + toolchain and carry no provenance
(debug-information.md §Debug Information Is A Deterministic Function…, §Carries No Provenance). The
three native-DWARF reproducibility hazards, and our fixed values:
  - **`DW_AT_comp_dir`** → the empty string / a fixed sentinel root (never the build directory).
  - **`DW_AT_name`** (compile unit + file table) → the **tree-relative module path** from the spans
    artifact (§2.1(a)), the DWARF counterpart of `-ffile-prefix-map`, never an absolute path.
  - **`DW_AT_producer`** → a fixed string, not the live toolchain version banner (which toolchain
    produced it is the reproducible-derivation contract's job, not a DWARF string).
  - No wall-clock time (DWARF mandates none; add none). DIE order, the line program, and the string
    table are emitted in **source-determined order** — the same order the backend emits the `Code`
    section it describes — so two derivations byte-match.

## 5. The strip round-trip is the reproducibility anchor (a gate-worthy test)

The undecorated artifact is the content-addressed, re-derivable form; a debug-carrying artifact is that
module *plus* a strippable section. So the load-bearing test, addable to the gate:

> emit(source, **debug=off**) == strip(emit(source, **debug=on**))  — byte-for-byte,

where `strip` is a **section remover** (`wasm-tools strip`; `--all` also drops `name`), never a
re-serializing optimizer (which can renumber `Code` and break the guarantee). This simultaneously
proves inertness (#3), strippability, and reproducibility, and it is cheap to run. It replaces
byte-identity-to-an-encoder as the backend's correctness oracle *for the debug sections* (the
`wasm-encoder` oracle covers the executed sections and does not model DWARF).

## 6. Verification spikes to run *before* committing to D2

The component-vs-core question (wall #2) is the biggest unknown and should be settled empirically first
(installed toolchain: **wasmtime 46.0.1, wasm-tools 1.242.0, cargo-component 0.21.1**):

1. Hand-build a *minimal* valid DWARF (a one-CU, one-subprogram, few-row `.debug_line`) — via `gimli`
   in a throwaway harness, or by hand — splice it into a current rcdzc component's embedded core
   module, and check: does **`wasmtime -D debug-info`** let GDB/LLDB stop at a source line **through the
   component wrapper**, or only for a bare core module?
2. Does the **Chrome "C/C++ DevTools Support (DWARF)"** extension consume our embedded core-module
   DWARF through the component?
3. Does the hand-built DWARF round-trip cleanly through `llvm-dwarfdump` and `wasm-tools`?
4. If the component wrapper blocks either debugger: confirm the **bare-core-module** debug build is a
   clean supported fallback (identical Code + DWARF; drop the envelope for the debug artifact).

The `gimli` spike is a *dev-desk oracle only* (like `wasm-encoder`/`wasmtime` today — dev-dependency,
never in the compile path); the shipped emitter is hand-rolled bytes per the operator's ruling. The
spike's job is to de-risk the format and the consumption path before we write the hand emitter.

---

## 7. File-by-file touch map (landmarks at `9f76c1b`)

| Concern | File:line | Change |
|---|---|---|
| Span sidecar as input | `abi.rs:32` (`KIND_AST`), `compile.rs:35` | add `KIND_SPANS`; decode it beside the AST; pass to the backend when present |
| Span byte layout | `cadenza-syntax` (`spans.rs`, `parser.rs`) + a new `rcdzc` codec | hand-rolled encode/decode of `Vec<(start,end)>` + tree-relative path |
| Node id per instruction | `backend/wasm/select.rs:106` (`emit`), `:32` (`SelectedFunc`) | record originating `StructId` per emitted `Lir` run (additive) |
| Code-offset per instruction | `backend/wasm/serialize.rs:50` (`instr`), `:139` (`code_entry`), `:178` (`core_module`) | thread running byte offset → build `Vec<(offset, StructId)>` |
| `name` + `.debug_*` sections | `backend/wasm/serialize.rs:178` via `encode::section` (`encode.rs:48`) | append custom sections to the core module |
| Fix component length prefix | `backend/wasm/envelope.rs:269` (`core_module_section`) | recompute embedded-core length after appending sections |
| Function/local names + types | `backend/wasm/select.rs:50` (`slot_of`), `db.defs`, `layout.rs:29` | collect `def → name`, `slot → (name, Ty)` |
| Enablement = sidecar directive (§9) | `compile.rs:32` (kinded inputs/targets), `backend/mod.rs:40` (`emit`) | recognize the "emit debug" request + `KIND_SPANS` input; NO `debug: bool` special path (the request rides the ABI) |
| Strip round-trip test | `backend/wasm/*` tests, `tests.rs` | `emit(off) == strip(emit(on))` |

**Caveat carried from the value-heap docs:** every backend `#[cfg(test)]` byte-oracle asserts
byte-identity to `wasm-encoder` for the *executed* sections; debug sections must be appended *after*
those so the oracle tests still pass unchanged (inertness proven by construction), and the debug
sections get their own round-trip/consumption tests rather than an encoder byte-oracle.

---

## 8. Open decisions (for the next session)

1. **First-landing scope** — recommend **D0 + D1** as one thin inert increment (readable `name`
   frames + the offset↔StructId side-table + the strip round-trip test), then the §6 spike, then D2.
2. **Source text location for line/col** — does the module's source text ride in the spans artifact,
   or does the driver re-read it? (Reproducibility §4 holds either way; the artifact-carried form is
   more self-contained.)
3. **Debug artifact shape** — does the engine emit a debug-*carrying* component (embedded, one
   artifact) or the lean component **plus** a separate `dwarf` output artifact (sidecar, two
   artifacts)? Both are valid per debug-information.md §May Be Embedded Or Emitted As A Sidecar; the
   sidecar directive (§9) can carry which. Informed by the §6 component-vs-core result.
4. **Compound inspection** — confirm the §3 recommendation (replay for values, DWARF for stepping)
   before the value heap makes it live.

---

## 9. Optionality + enablement — the sidecar directive (the operator's second ruling)

Optionality is **already normative at the capability level** and needs no new spec work: debug info is
a MUST-optional capability with a declared default (debug-information.md §This Capability Is Optional,
§Whether To Emit Debug Information Is A User-Facing Choice), and the seed's realized set defaults it
**off** ("the seed clears ignition emitting the undecorated artifact only" — seed-ignition-set.md).
What this ruling fixes is the **enablement *mechanism*: it is a sidecar directive, not a `--debug`
flag threaded into `compile()`.**

### The convergence with the query engine

`DESIGN-query-engine.md` reframes the compiler as `engine(target: Ast, sidecar: Program, output:
Format) → Bytes`, where the **sidecar is the driving program** telling the engine what to produce.
"Lower to a component" is the degenerate sidecar. **"Lower to a component *and emit its debug info*"
is that same directive carrying one more request** — enabling debug info is a thing the sidecar
*asks for*, not a build-tool flag. All of it crosses the *same* `ast_bytes → result_bytes` ABI
(`rcdzc/src/abi.rs`), so this adds no new invocation surface.

### The three artifacts, unified by the ABI

The §2.1 plumbing already made debug info a matter of kinded artifacts; the sidecar model just names
who *requests* it:

| Role | Artifact / signal | Crossing |
|---|---|---|
| **enablement** (replaces the flag) | the sidecar directive "emit debug for this component" | the sidecar `Program` input |
| **data** the backend injects | `KIND_SPANS` input (span table + tree-relative module path, §2.1a) | a kinded **input** artifact |
| **result** | the debug-carrying component, or a separate `dwarf` **output** artifact | a kinded **output** artifact (Amendment 0.8.0: "a debug-information sidecar … is another artifact of the same shape") |

Optionality then falls out **for free and by construction**: a sidecar that does not issue the
directive gets the undecorated artifact (the byte-identical reproducibility anchor of §5); one that
does gets the debug output. No `debug: bool` special path, no second `Target`.

### The realized default records itself

debug-information.md §A build MUST record in its decision record whether it emitted debug information.
In this model that record *is* the sidecar directive that ran — the enablement is itself the audit
trail, not a separate logged flag.

### ⚠ Trap — two unrelated things both called "debug"

The query engine's `output: Format` already has a value literally named **`debug`**
(`convert.rs` — a *readable debug rendering of the AST tree*, §2 of that doc). That is **NOT** wasm
debug info. Enablement is **not** "select the `debug` output format"; it is the sidecar directing
*which kinded output artifacts* (the `component` and/or a `dwarf` artifact) the engine produces, with
`KIND_SPANS` supplied as input. Do not wire the AST-debug `Format` to DWARF emission — they are
orthogonal axes (how a tree is *rendered* vs. whether the *component* carries debug sections).

### Staging against the query engine's rungs

The query engine is **design-only**, gated on the generics/recursion (self-host) work
(`DESIGN-query-engine.md` §7); its Rung 2 is an interim Rust driver. Debug info need not wait for the
self-hosted rung: the enablement directive can ride the **existing kinded-artifact `compile()` entry**
(a recognized input/output kind + a boolean-or-enum request field the driver sets) as the Rung-2-shaped
realization, and *become* a first-class sidecar query verbatim when the query engine lands — same ABI,
same artifacts, the request just moves from a driver argument into the sidecar program. So D0–D3 are
**not blocked** on the query engine; they land against the artifact seam today and inherit the sidecar
surface for free later.
