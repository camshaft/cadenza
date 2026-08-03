# PR #1354 review comment — implementation/compiler-ml/src/emit-rec-db.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1354 (PR: "cand: v-compiler-ml — 0a79425ba").
Refinement of the #1343 comment-mechanism fix (same line :182).

## Comment implies a general wasm `funcidx==typeidx` rule; it's this assembler's encoding choice (Copilot, emit-rec-db.cdz:182) — doc
> The comment implies a general wasm rule that `funcidx i` uses `typeidx i`. In wasm, each function's
> type is the type index listed in the *function section*; it's only `typeidx == funcidx` here
> because this assembler emits `func-section-multi(range-list(k))` (i.e., type indices `[0..k]`) and
> `recursive-types` is constructed to match that ordering. Rewording this helps avoid misleading
> future readers about the wasm invariant versus this module's encoding choice.

Follow-on to #1343: the reworded comment now states `typeidx==funcidx` as if it were a wasm invariant,
but wasm sets a function's type via the FUNCTION SECTION's declared type index — the equality holds
here only because this assembler emits `func-section-multi(range-list(k))` (types `[0..k]`) and
`recursive-types` matches that order. Reword to attribute the equality to this module's encoding
choice, not a general wasm rule.
