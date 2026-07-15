# Host-boundary closure declines: confirm-against-spec, then FIX or PROMOTE (3 standing gate TODOs)

**File:** `spec/semantics/21-host-closures.sexp` — 3 cases grade TODO:
- "a closure returning Unit is declined — Unit has no machine representation"
- "a closure-typed closure ARG on the DIRECT-CALL path is declined — host would supply the closure"
- "a producer capturing a host-supplied COMPOUND parameter is declined — host→guest decode"

**FIRST decide per case (this is the highest-value step — do NOT assume they're bugs):** read the
component-abi / host-closure SPEC TEXT. Each todo describes a DECLINE. If the spec says the case
SHOULD work, it's a real gap → implement it (thread the missing ABI support). If the spec sanctions
the decline as a fundamental limitation (e.g. Unit genuinely has no machine representation at the
boundary, or the host legitimately supplies the closure), then the "todo" is a SOUND DECLINE that
should be PROMOTED to a pinned `(error …)`/decline case — not "fixed". Either way the case stops
being an open todo.

Area: rcdzc host-closure ABI (`backend/wasm`, ArgSlot). Coordinate with **v-effects** (owns the
host-boundary/closure surface — [[closures-across-host-boundary]]); if the fix is substantial ABI
work rather than a small thread-through, hand it to v-effects via a note instead.
