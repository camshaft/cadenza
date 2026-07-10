# The ABI is the export signature — emit names verbatim, never sniff the output shape

*2026-07-09*

**What happened.** `rcdzc`'s boundary is fixed by *explicit data*, never inferred from the program's body.
`layout.rs` builds a `Layout` holding one `ExportPlan` per exported definition, and an export's ABI **is
that function's signature** — its parameter types and return type, read generically at serialization. There
is no `main`/`compile` recognition, no `ExportAbi { Run | Compile | Resource }` classification, and no walk
of the body to guess whether it "looks like" a run entry or a compile entry. The boundary name is the
source name **verbatim**: the compiler never renames an export, and a consumer that needs a particular
entry resolves it by *signature* (a nullary export, a `list<u8> → list<u8>` export), not by a compiler-side
rename to a blessed name. The one genuine branch the layout draws — whether the module imports the value-
heap runtime — is likewise a *function of the solved types* (`imports_runtime` = does any reachable
function touch a compound), computed once and consulted once, not sniffed per node during emission.

The corollary that keeps this honest is `select`-owns-the-encoding: every pass above `serialize` reasons in
named types (`ValType::I64`, `BlockType`), and the raw wasm encoding byte lives in exactly one place
(`ValType::byte`, consumed only by the serializer). No pass hard-codes `0x7E`, and the SLEB128-vs-raw-byte
hazard (`i32.const`/`i64.const` take a *signed* LEB, so any operand ≥ 64 must be sign-extended, never
written as a raw byte) is contained to the one function that knows it.

**Why.** The predecessor compiler walked the entry body to *guess* its output shape and then renamed the
entry to a canonical `run`, and both were exactly the fragility a rebuild must not reproduce: shape-sniffing
means the ABI is re-derived from the body every compile, so it can disagree with what the body actually
returns (the coarse-kind failure at the boundary —
[[2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point]]),
and renaming means the byte output depends on a compiler convention the source never wrote, which no
consumer can predict without knowing the convention. The operator's ruling was categorical — renaming an
export is "an absolute no-no… completely counter-intuitive to just randomly rename a function" — and the
principle generalizes the whole no-name/no-shape-magic stance to the *output* side, not only entry
detection: **the boundary is a contract the source states explicitly, so encode it, never infer it.** An
`ExportAbi` enum was itself dropped as "sniffing one level up" — even a *classification* of the export is a
re-derivation, when the signature already *is* the classification. The reproduction value is that a fresh
implementer under a "just make the entry work" schedule will reach for exactly the two shortcuts this
forbids (find-the-nullary-and-drop-the-rest, rename-to-`run`), and both come back to bite: the first
silently loses every other export and fails outright on a compile-only module, and the second makes the
emitted bytes un-predictable from the source. The self-hosting byte gate is unforgiving here — a renamed or
mis-classified export is a byte difference against the reference for a reason that has nothing to do with
compiler correctness — which is how the shortcuts were caught.

**The requirement it drove.** Realizes `compiler-pipeline.md` §"Emission Serializes A Lowered
Representation" (emission "MUST NOT decide a type" — and, by extension, MUST NOT decide the ABI), the
build-tool interface's kinded-artifacts contract ([[2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result]]:
the component is one artifact among many, selected and named explicitly), and
`modules-and-namespaces.md` §Visibility (an export is visible by an explicit rule, and its boundary
identity is its declared name). The reproduction content **not yet folded**, for the architecture reference
doc: (1) **the ABI is the export's signature** — the compiler MUST NOT infer a boundary shape by inspecting
a function's body, and MUST NOT classify exports into blessed kinds when the signature already carries the
distinction; (2) **export names are verbatim** — the compiler MUST NOT rename an export, and a consumer
resolves an entry by signature, not by a compiler-assigned canonical name; and (3) **the encoding byte
lives in the serializer alone** — every pass above serialization reasons in named types, so no analysis
pass hard-codes an instruction or valtype byte. All three are compiler-construction discipline rather than
language semantics.
