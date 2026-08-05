# PR #2142 review — reducer_b3.cdz (v-harness-bootstrap) — OPEN — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2142 (B3 reducer — FIX the kv counter to actually increment,
Rust-guest byte parity + pin it; the fix-forward for MY #2138 parity finding). Copilot 1 inline,
doc-accuracy on the helper-type comment.

## helper-type comment lists `Bytes.of : List(Int64) -> Bytes`, but the actual prelude signature is `List(UInt8) -> Bytes` → the byte-parity doc mis-states the exact element type it exists to mirror (Copilot, reducer_b3.cdz:47) — doc-accuracy [VERIFIED CORRECT, LOW]
> The helper-type comment lists `Bytes.of : List(Int64) -> Bytes`, but `Bytes.of` actually takes
> `List(UInt8)` (see seed prelude docs). This matters because it's documenting the exact byte-level
> parity behavior you're trying to mirror.

VERIFIED against prelude SOURCE (not just the prelude doc): `bytes_of_type` (rcdzc/src/prelude.rs:1830-
1845) builds the arg type as `list_u8 = (List (UInt 8))` — i.e. `Bytes.of : List(UInt8) -> Bytes`. The
#2142 diff comment (reducer_b3.cdz:13/47) says `Bytes.of : List(Int64) -> Bytes` — WRONG element type.
Copilot is correct. LOW/doc-accuracy — the CODE is fine: `Bytes.of([UInt8.wrap(prev-byte + 1)])` and the
pins `Bytes.of([5])`/`Bytes.of([UInt8.wrap(255)])` all feed byte-range values, and `UInt8.wrap` narrows.
Only the type COMMENT mis-labels the element type — but it matters here precisely because this comment
documents the byte-level parity the fix exists to demonstrate. Fix: change the comment to
`Bytes.of : List(UInt8) -> Bytes`.

⚠️ PROVENANCE NOTE for v-harness-bootstrap: the prelude's OWN doc comments are ALSO stale here —
prelude.rs:946 ("This increment realizes `of : (List Int64) → Bytes`") and :957 ("`of : (List Int64) →
Bytes`") say Int64 while the adjacent CODE (`bytes_of_type`, same file) builds `(List (UInt 8))`. So the
reducer comment inherited the mistake from the prelude doc. Worth a heads-up to whoever owns the prelude
module doc (v-inference/prelude) to fix prelude.rs:946/957 too — otherwise the next reader re-copies
`List(Int64)`. (I'll route that separately as a LOW.) PR OPEN → foldable pre-merge. v-harness-bootstrap
owns the reducer fixtures.
