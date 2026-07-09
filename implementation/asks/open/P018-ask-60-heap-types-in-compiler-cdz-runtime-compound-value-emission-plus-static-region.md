## 60. 🟢 (compiler.cdz — OPERATOR DIRECTION, the next big thing) Implement HEAP TYPES: runtime-compound VALUE emission (records/tuples/lists/maps/sets/strings/sum values as RESULTS) — plus a design for STATIC heap types (a two-tier heap)

**STAGE-2 (runtime-element TUPLE) — DECODED + Python-verified, but the TEMPLATE APPROACH REJECTED as
over-fitting; the real work is BACKEND INTEGRATION. 2026-07-08.** Fully decoded `(def (f n) (tuple <elems>))
(def (main) (f K))` and PROVED byte-identical reassembly in Python for `(tuple n 1)`/`(n 2)`/`(n n)`/`(n 1 2)`
from fixed blobs + THREE generators: `main` = `i64.const K; call 45(=f); end`; `f` = `i32.const N; call
6(arr-alloc); local.set 1; per slot {local.get 1; i32.const i; <elem = local.get 0 | i64.const k>; call
0(box-int); call 7(arr-set); drop}; local.get 1`; and an ARITY-PARAMETERIZED display walk (head `(tuple`,
n per-element `arr-get`+`get-int`+render blocks, tail `)`). Reuses the shared heap envelope (hl-prefix/hl-suffix).
⛔ **BUT: this only works for the EXACT 2-function shape.** A 3-def program shifts the core (more functions →
different type/func sections, `call <idx>` moves — verified: 2-def core 1352 B vs 3-def 1408 B). Hardcoding
`call 45` + a fixed 2-function layout is MEMORIZING one program's byte offsets, not real construction codegen —
it would light up exactly 1 corpus case (`(module m (def (f n) (tuple n 1)) (def (main) (f 3)))`) while being a
brittle contortion that misrepresents capability. Per the discipline (don't contort to fit), I did NOT ship it;
reverted to Stage-1. **The honest Stage 2 = teach the compiler's REAL backend to emit tuple construction as a
Core/Instr construct** integrated with the existing multi-function `lower`/`mod-fenv` machinery: a `(tuple …)`
with a runtime element lowers to `arr-alloc`/`box-int`/`arr-set` opcodes (new `Instr`s or heap-op calls), the
GENERAL envelope gains the heap-op imports (not a per-shape blob), and the type-directed display is emitted per
result type. That is a genuine architectural addition (heap-construction in the IR + a heap-importing general
envelope), the correct next investment — decoded recipe above is the target the backend must reproduce.
CONTRAST with Stage 1 (heap int-list): that shape is a SELF-CONTAINED nullary-main construct whose envelope is
always the SAME single-function layout, so a template is legitimate there (and it correctly DECLINES — 101-B stub,
not a miscompile — when a `List.push` appears in a multi-function program). Stage 1 stays; Stage 2 waits for the
backend path.

**✅ STAGE-1 LANDED 2026-07-08 — first HEAP-IMPORTING component emission (runtime int-list).** compiler.cdz
now emits BYTE-IDENTICAL heap components for a runtime int-LIST — a nullary `main` that is a `List.push`-chain of
const ints over `(list)` (e.g. `(List.push (list) 5)`, `(List.push (List.push (list) 7) 8)`). Verified
byte-identical to native for `[5]`,`[7,8]`,`[1,2,3,4]`,`[100,-3,0]`,`[42]`. This is the FIRST time compiler.cdz
emits a component that IMPORTS the value-heap runtime and EXPORTS `run` returning an opaque handle — the whole
heap-emission pipeline (heap-op imports + construction + the baked-in heap-walk display) now works end-to-end.
- Detection (`main-heap-list-off`/`node-is-list-push?`/`heap-list-chain-ok?`): main's body is a `List.push`
  chain `[apply, (. List push), acc, elem]` bottoming out in `(list)`, every `elem` a const int (major 0/1).
  A bare `(list …)` literal still const-folds (the const tier) — only a push-CHAIN takes the heap path.
- Emission (`heap-list-component`): SIX fixed transcribed blobs (`hl-prefix` 1983 B, `hl-corehead` 951 B,
  `hl-coremid` 55 B, `hl-codepre` 117 B, `hl-codesuf` 172 B, `hl-suffix` 564 B) + one GENERATED `run` body:
  `[0 locals][call 24=vec-empty] per elem [i64.const e (sleb); call 0=box-int; call 27=vec-push] [0x0B end]`.
- 0 hard / 0 error held; 37 agree unchanged (the push-chain cases aren't in the corpus — this is FOUNDATIONAL
  plumbing verified by direct byte-comparison, not a corpus win). ⏭ STAGE 2 = runtime-element TUPLE (`(def (f n)
  (tuple n 1))`) — the construction primitive is decoded (`arr-alloc`/`box-int`/`arr-set`), needs param threading
  + two-function codegen + the tuple-display walk; that opens the corpus frontier.

**STAGE-1 RECIPE VALIDATED IN PYTHON 2026-07-08 (byte-identical reassembly proven).** Decomposed a
runtime-heap component into transcribable fixed blobs + one generated body:
`component = PREFIX(1983 B fixed) + coresec(id=1 + uleb(size) + CORE) + SUFFIX(564 B fixed)`, where
`CORE = COREHEAD(951 B fixed: magic+type+import) + COREMID(func+mem+global+export, fixed for a given
function-count) + CODESEC`. For a NULLARY-main int-LIST (`(List.push (list) 5)` → 3859 B): CODESEC = 6
functions, FIVE fixed (int-renderer + list-display walk, program-independent) + ONE generated `run` body:
`[0][call 24=vec-empty] then per element [i64.const <e>][call 0=box-int][call 27=vec-push] then [0x0b end]`.
Verified byte-identical to native for `[5]`,`[7,8]`,`[1,2,3,4]`,`[100,-3,0]` (negatives via sleb). The
TUPLE construction primitive is also decoded — `(def (f n) (tuple n 1))` func = `i32.const N; call 6=arr-alloc;
local.set A; {local.get A; i32.const i; <elem>; call 0=box-int; call 7=arr-set; drop} per slot; local.get A`
where `<elem>` = `local.get <param>` or `i64.const <k>`; display is a per-shape heap walk (`arr-get`+`get-int`).
Blobs saved (transcription-ready). ⏭ LANDING the int-list slice into compiler.cdz next (establishes the whole
heap-emission pipeline — envelope + construction + vec ops — that tuple/sum/record reuse). NOTE: the pure-int-list
result unlocks 0 corpus cases directly (they're all `(list …)` literals that const-fold, or Bytes/tuple/sum); it
is FOUNDATIONAL plumbing. The corpus payoff arrives with the runtime-element TUPLE (needs param threading +
two-function codegen), the natural stage-2.

**TIER-2 (RUNTIME-ELEMENT HEAP OBJECTS) — RECON 2026-07-08 (c), operator: "focus on heap objects as much as
possible".** Decoded native's runtime-element component (e.g. `(def (f n) (tuple n 1)) (def (main) (f 3))` →
3902 B) to scope the build. Findings that define the target:
- A "runtime element" is a PARAMETER or non-const call result. Native CONST-FOLDS constant elements even inside a
  compound (`(tuple (+ 1 2) 1)` → the 673-B const `(tuple 3 1)` component), so tier-2 only fires for a genuinely
  runtime element — which needs the full heap path.
- SHAPE IS DIFFERENT from the const tier: a runtime-heap component EXPORTS `run` (returns an opaque u32 heap
  HANDLE) and IMPORTS the whole heap-op set (box-int/get-int/arr-alloc/arr-set/arr-get/sum-new/… each present ×4
  across type/import/alias/export). The const tier's resource-with-display (`make`/`display`) shape does NOT apply.
- The core-module HEAD is a FIXED BLOB: type section (258 B) + import section (684 B) + func/mem/global decls are
  byte-identical across different runtime-element programs (shared for the first 1013 B of the core module) — so
  like `compound-corehead`, it can be transcribed once. Two programs share a 1984-B prefix and 633-B suffix; the
  variable MIDDLE (≈1285 B) is the GENERATED construction + display code.
- CONSTRUCTION recipe (from the heap WIT): a runtime tuple `(tuple n 1)` = `arr-alloc 2` → for each element,
  `box-int <elem>` then `arr-set <arr> <i> <boxed>` → the arr handle is the value; a sum `(Some x)` = `sum-new
  <disc> <box x>`; a list = `vec-empty` + `vec-push`. DISPLAY is a type-directed heap walk (`arr-get`+`get-int`
  → `int-to-decimal`, wrapped in `(tuple … )`), reusing the render string-building already built for the const tier.

**⚠ SCOPE: this is a MULTI-STAGE build, not a one-cycle drop — a new component envelope (heap-import head +
component wrapper) + construction codegen in `lower` + a heap-walk `display`. Staged plan (each stage byte-verified
against native, 0-hard/0-error held, decline-don't-miscompile):**
1. Transcribe + emit the FIXED heap-import envelope (head blob + component wrapper) for the simplest heap-returning
   `run` — prove compiler.cdz can emit a valid heap-importing component at all.
2. Construction codegen for a single runtime-element tuple (`arr-alloc`/`box-int`/`arr-set`), byte-match native.
3. The heap-walk display for that tuple.
4. Generalize to sum/list/record/nested; then the runtime-element frontier (26 corpus cases) opens.
Recon done this cycle; stage 1 is the next implementation step. NOT started emitting yet (correctly — a rushed
partial envelope would risk invalid bytes; the discipline is byte-verify each stage).

**PROGRESS 2026-07-08 (b) — CONST STRING-LITERAL tier landed.** compiler.cdz now emits BYTE-IDENTICAL compound
components for a CONST string literal value — `"hello"`, `"café"`, `""`, and escaped forms (`"a\"b"`, `"a\\b"`,
`"tab\there"`, `"line\nbreak"`). Verified byte-identical to native on both const-string corpus cases (`"hello"`,
`"café"` in 01-literals) + 8 hand cases; value-harness 34→37 agree, 0 hard/0 error. A string is a CBOR TEXT node
(major 3) storing raw UTF-8; it displays as `"` + escaped-contents + `"`. Reuses the existing `compound-component`
assembler (a string result is the same resource-with-display recipe as a tuple — `"hi"` vs `"ab"` differ in exactly
2 display-char bytes). New helpers: `node-is-string?` (major 3), `render-string`/`render-string-bytes` (the CLOSED
escape set — `\`→`\\`, `"`→`\"`, LF→`\n`, CR→`\r`, TAB→`\t`, else verbatim incl. multi-byte UTF-8), and
`str-all-renderable?`/`str-byte-renderable?` — a C0 control byte OTHER than \n\r\t (which native displays as
`\u{hex}`) makes the string NOT render-ok → DECLINE (we do NOT reproduce native's `\u{}` form — under-decline,
never mis-render; also sidesteps the known `\u{}` round-trip gap). Wired into `render-ast`/`body-is-compound-head`/
`render-ok?`. NOTHING for the seed agent — purely additive compiler.cdz work.

**PROGRESS 2026-07-08 — CONST SUM-CONSTRUCTOR tier landed (const-tier now covers tuple/list/record AND sum-ctor values).**
compiler.cdz now emits BYTE-IDENTICAL compound components for CONST constructor values: `(Some 42)`, `(None unit)`,
`(Some (Some 5))`, `(Ok (Some 3))`, and ctors nested in / around tuples & records (`(tuple (Some 1) (Ok 2))`,
`(record (a (Some 1)) (b (Ok 2)))`, `(Some (tuple 7 1))`). Verified byte-identical to native on all 3 const-ctor
corpus cases (`(Some 42)`, `(Some (Some 5))`, `(Ok (Some 3))`) + ~12 hand cases. How it works — a constructor renders
structurally EXACTLY like a tuple (`(Head elem…)`), so it reuses the existing renderer/assembler; the whole addition
is recognition + a safety gate:
- **Discriminator = CAPITALIZATION.** A capitalized (A–Z first byte) bare application head is a nominal constructor
  (`head-is-ctor?`); `tuple`/`list`/`record` and all operators are lowercase. A DOTTED `A.B x` becomes the
  `apply`/`.` shape (head "apply"), so it never matches — non-dotted ctors only this tier.
- **`unit` payload** renders as the literal `"unit"` (`node-is-unit?`); `(None unit)` = `(Head unit)`.
- **EMIT-SAFETY (strict subset of native — decline-don't-miscompile):** a UNIT payload is safe for ANY capitalized
  head; a NON-unit payload only for a verified WHITELIST `{Some,Ok,Err,Left,Right,Just}` at arity 1 (an unknown
  capitalized head with a non-unit payload might be a nullary ctor native DECLINES — e.g. `(None 5)`/`(Zero 5)` —
  so we under-decline). `ctor-emit-safe?` + `head-in-ctor-whitelist?`.
- **ctor-in-`list` DECLINED:** native requires a homogeneous element TYPE across a list (`(list (Some 1) (Some (tuple
  2 3)))` DECLINES on native; `(list (Some 1) (None unit))` renders — it's payload-type unification, not head equality),
  which this tier does not perform, so any `list` containing a ctor element is a safe under-decline (`list-has-ctor-elem?`).
  Ctors in tuple/record are fine (heterogeneous by nature). This is the honest frontier — value-harness stays 0 hard/0 error,
  decline 86→88 (the ctor-in-list under-declines).
- ⏭ STILL TIER-2 (runtime-element compounds — a param/call element like `(def (f n) (Some n))`): needs real heap
  construction + heap-walk renderer, not const rendering. Unchanged by this.

**Operator's direction (2026-07-07):** *"Move to start implementing heap types now. I know it's big but it's
definitely the next thing. The other interesting thing would be to have a way to have STATIC heap types in this
impl — not sure if that would be possible — we'd need to somehow have different memory regions."*

**Why now.** The byte gate has been 0-disagree for many cycles; the ~404 remaining declines are dominated by ONE
capability compiler.cdz lacks — emitting a runtime COMPOUND VALUE as a result. ask-57's map: **05-compound-types
= 139 declines (~1/3 of all)**, and strings/bytes/list/equality all hit the same wall (any op returning a compound
needs it). It's the M2 acceptance target ([[m2-acceptance-target-runtime-compound]]). Timing is right: the sibling
just landed the **CHAMP heap runtime natively** ([[champ-runtime-implemented-native]] — all 17 ops, 116 tests), so
the runtime component compiler.cdz would import already exists.

### The target ABI (verified by decoding what the seed emits)
A compound-returning program is NOT the scalar `run : () → i64` envelope compiler.cdz emits today. It is a
**resource-with-display component** ([[resource-display-component-abi]]):
- exports `make` (build the value → a handle/u32), `display` (`self → string`, the type-directed renderer),
  `cabi_realloc`, `memory`;
- imports the heap runtime interface — for a runtime-element compound this is the rich CHAMP/RC surface
  (`box-int`/`get-int`/`box-bool`/`box-float`/`tuple-alloc`/`arr-alloc`/`champ-*`/…); for a const compound it's
  minimal (`intr.new`);
- the runtime is a SEPARATE wasm COMPONENT the host composes (M2 shared-runtime decision
  [[m2-shared-runtime-component-decision]] — needs the "composed non-effect import" governance amendment so a
  runtime import is not counted as a host effect / capability).
- The heap is genuinely TAGLESS: `Node { rc, handles[], raw[] }`, no discriminant — Int/Bool/Float/Bytes/Str
  share one descriptor; a 2-tuple, a 2-list, a 2-record are byte-identical. **The COMPILER holds the exact static
  type and only emits `get-int` where the type says Int** (no type erasure). This invariant is load-bearing.

### Staging (compiler.cdz half — decline-don't-miscompile at each step)
1. **Constant compounds first** (`(tuple 1 2)`, `(record (a 1) (b 2))`, `(list 1 2 3)`): the seed const-folds the
   DISPLAY to a fixed string, so `make`/`display` are near-trivial (write literal bytes; `make` = one `intr.new`).
   This is the smallest slice that emits the WHOLE resource-ABI envelope end-to-end — the right first landing (it
   forces the new component shape without the heap-walk renderer). Composes with the projection-fold already built.
2. **Runtime-element compounds** (`(tuple a 2)`, `(f 3) → (record …)`, list/map/set built at runtime): real heap
   construction via the runtime intrinsics + a type-directed renderer that WALKS the heap (`Shape::Rec` for
   recursive/unbounded — read the DECLARATION, cut back-edges to a named ref, never inline; see
   [[recursive-sum-value-renderer]] for how native does it). This is the bulk.
3. Each tier: land const → runtime-scalar-element → runtime-compound-element, gate-green (0 disagree, value 0
   hard) at every step; a shape compiler.cdz can't yet build DECLINES.

### STATIC heap types — the two-tier heap (operator's "different memory regions" idea — ANALYSIS: possible, natural)
The idea: a value of STATICALLY-KNOWN, finite, non-recursive shape (a fixed tuple, a fixed-field record, a
fixed-width int) need not pay the dynamic-RC-heap cost (`Node{rc,handles,raw}` alloc + refcount + tagless
descriptor + heap-walk display). It can live FLAT in a dedicated STATIC region. Findings:
- **The compiler already holds the exact static type** (the tagless-heap invariant above) — so a statically-shaped
  value has all its layout at compile time; nothing forces it onto the dynamic heap.
- **The component model already gives separate memory regions.** The dynamic RC heap lives in the RUNTIME
  component's memory; the emitted program has its OWN linear memory (verified: 3 memories in a composed compound
  component). A static value can live in the PROGRAM's own memory — a bump-allocated STATIC ARENA (a reserved
  range below the scratch region), distinct from the runtime heap BY COMPOSITION. So "different memory regions" is
  achievable WITHOUT the wasm multi-memory proposal — it's a static arena in the program's single memory vs. the
  runtime's heap, not two core memories in one module.
- **The tier split writes itself from the type:** static-shaped (fixed tuple/record, fixed-width int, bounded
  non-recursive sum) → FLAT static layout, no rc, unrolled fixed renderer; statically-unbounded/runtime-shaped
  (list/map/set of runtime length, recursive sum, runtime-chosen variant) → dynamic RC heap. This is exactly the
  `Shape::Rec` vs fixed-`Shape` split the renderer already draws. And it stays sound: shape not statically known ⇒
  dynamic heap, always.
- **Open questions for the compiler agent / a design pass:** (a) does the ABI need to distinguish a static-region
  handle from a heap handle at the boundary, or is `display` uniform? (b) how does a static value that ESCAPES
  into a dynamic structure get promoted (copied to the RC heap)? (c) is the static arena worth it before the
  dynamic path even exists — probably NO: build the dynamic path first (tier 2 above), then add the static region
  as an OPTIMIZATION once there's a working baseline to measure against. Premature static regions would complicate
  the first landing.

**Recommendation on sequencing:** implement the DYNAMIC heap path first (const compounds → runtime compounds,
staged), get compound coverage green, THEN design the static two-tier region as an optimization. The static-region
idea is sound and the memory separation is free (composition), but it's a refinement of a working heap path, not
the way to start.

### 🔬 CONCRETE GROUNDWORK done this cycle (decoded the seed's const-compound emission — de-risks tier 1)
- A const `(tuple 1 2)` component is **673 bytes**. Diffing `(tuple 1 2)` vs `(tuple 3 4)`: they differ in
  **exactly 2 bytes** (the display chars `1`↔`3` @204, `2`↔`4` @218). So for a FIXED compound shape the entire
  envelope is a **fixed blob + the rendered display-string content spliced in** — the same fixed-blob+splice
  structure compiler.cdz's scalar `run` envelope (`wrap-component`) already uses.
- `(tuple 10 200)` is **695 bytes** — the envelope SIZE grows with the display-string length, so the splice is
  **length-parameterized** (the `display` func's per-byte `i32.store8` sequence + the string ptr/len constants
  scale with the rendered string). Not a single fixed blob: a fixed frame around a variable-length display body.
- The `display` core func (func 3 in the WAT) literally writes the rendered string byte-by-byte
  (`i32.const <off>; i32.const <char>; i32.store8` per char), then stores a `[ptr,len]` return descriptor and
  returns 0. `make` (func 2) = `i32.const 0; call intr.new`. `cabi_realloc` (func 1) = a bump allocator. Around
  the core module: a `(resource (rep i32))` type, `canon resource.new`, `canon lift (memory)(realloc)`, and a
  nested sub-component with the resource import/export — the component-model resource machinery compiler.cdz has
  NOT emitted before.

**∴ tier-1 (const compound) = TWO net-new pieces:** (1) a **value RENDERER** (compiler.cdz has NONE — needs
integer→decimal-string + the structural walk `(tuple `,elems,` `,`)` / `(record (k v)…)` / `(list …)`), and (2) a
**length-parameterized resource-ABI envelope** (the 673-byte frame with the rendered string + its length fields
spliced by rendered-length). Both are real; neither is a contortion; but together they are NOT a single-cycle
landing and can't be validated incrementally (it's all-or-nothing to a VALID component). Recommend a FOCUSED
multi-step pass: (a) build+unit-test the renderer (a pure `Core-const → string bytes` fn, testable via a scalar
program that returns its length), (b) transcribe the fixed envelope frame as a blob with a documented splice point,
(c) wire render→splice→emit, gate-green. The runtime-element tier (2) then reuses the renderer's structure but
drives it over heap-walk intrinsics instead of a compile-time-known value.

**Status.** 🔴 OPERATOR-DIRECTED, the next big feature. Large multi-part subsystem (resource-ABI emission + heap
intrinsic calls + type-directed renderer), seed-scaffolded (CHAMP runtime + resource-display ABI exist native-side;
recursive-sum renderer landed). compiler.cdz starts from the const-compound resource-ABI envelope. Related:
[[m2-acceptance-target-runtime-compound]], [[m2-shared-runtime-component-decision]], [[resource-display-component-abi]],
[[champ-runtime-implemented-native]], [[recursive-sum-value-renderer]], ask-57 (the coverage map this unlocks),
ask-58 (builtin-modules — many builtin RESULTS are compounds, so they ride this too).
