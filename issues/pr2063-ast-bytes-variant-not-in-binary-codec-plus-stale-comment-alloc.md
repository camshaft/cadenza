# PR #2063 review — rcdzc lower.rs + sums.rs (v-metaprogramming) — MERGED — 1 functional gap (MED) + 2 LOW [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2063 (add an Ast.Bytes variant — first-class byte-literal). Copilot
3 inline: a real codec incompleteness + a stale overview comment + a per-byte alloc.

## `Ast.Bytes` is a variant + printable via reify, but the binary `Ast.encode`/`decode` codec has NO bytes arm → constant `Ast.Bytes` can't round-trip through Ast.encode/decode (Copilot, lower.rs:3328) — functional gap [VERIFIED, MED]
> `Ast.Bytes` is now a supported `Ast` variant …, but the `Ast.encode`/`Ast.decode` byte codec doesn't
> handle it: `encode_ast_value` has no `d == disc.bytes` arm, and `decode_ast_value` matches only
> `AST_TAG_*` 0x00–0x05. …constant `Ast.Bytes` values will cause `Ast.encode` to decline, and `Ast.decode`
> cannot produce an `Ast.Bytes` node from the wire format.

VERIFIED on trunk: `encode_ast_value` (lower.rs:3335) has arms for int/float/bool/str/name/list but NO
`disc.bytes` arm; the tag consts stop at `AST_TAG_FLOAT = 0x05` (no `AST_TAG_BYTES`); `decode_ast_value`
(:3631) matches only those tags. NOTE: the REIFY/print path DOES render `Ast.Bytes` as `b"…"` (lower.rs:2868,
the `disc.bytes` arm in the text renderer) — so the variant is otherwise supported — but the BINARY codec
(the `Ast.encode`→bytes / `Ast.decode`←bytes round-trip metaprogramming uses) drops it. So a quoted/constant
`Ast.Bytes` literal can be printed but not `Ast.encode`d, and `Ast.decode` can never yield one. MED —
a genuine incomplete-variant hole in the metaprog wire codec, added-but-not-wired by this PR. Fix per
Copilot: add a stable `AST_TAG_BYTES` (0x06), an `encode_ast_value` `disc.bytes` arm (length-prefixed raw
bytes, mirroring the Str framing), a `decode_ast_value` arm, + update the codec doc. (Worth a round-trip
corpus/unit pin too, given the codec is the metaprog contract.)

## a second `Ast`-variant overview comment still lists the OLD set (Int/Float/Bool/Str/Name/List), omits `Bytes` (Copilot, sums.rs:130) — doc-accuracy [VERIFIED, LOW]
> There's now a second `Ast`-variant overview comment … that still describes the older "complete spec
> variant set" … and doesn't mention the newly-added `Bytes` variant.
LOW — update that comment to include `Bytes` (and drop the "complete set" claim keyed on the old
enumeration, so the next variant addition doesn't re-stale it).

## `format!("\\x{b:02x}")` per escaped byte allocates a temp String each iteration (Copilot, lower.rs:2891) — efficiency [VERIFIED, LOW]
> …allocates a temporary `String` for every escaped byte. For large byte literals, this adds avoidable
> overhead during constant folding. Prefer writing directly into `out` via `write!`.
VERIFIED (the reify loop at lower.rs:2891): `_ => out.push_str(&format!("\\x{b:02x}"))` — a `String` per
non-printable byte. LOW/efficiency — `write!(out, "\\x{b:02x}")` (out is a String, `use std::fmt::Write`)
avoids the temp. Only bites on large byte literals with many non-printables. v-metaprogramming owns this
(the Ast.Bytes author); the codec gap is the one that matters.
