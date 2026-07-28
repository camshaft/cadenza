# PR#747 review comment — decode_ast_value `4 + mag_len` can overflow usize on untrusted length prefix

Mirrored from GitHub PR review comment (Copilot), id `3623988432`.
PR: https://github.com/camshaft/cadenza/pull/747 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/lower.rs:3344`

## Comment (verbatim)

> `decode_ast_value` computes `4 + mag_len` to slice the magnitude. Since `mag_len` comes from an
> untrusted length prefix, `4 + mag_len` can overflow `usize`, which would wrap in release and
> potentially slice the wrong range (or panic in debug builds with overflow checks). Use
> `checked_add` to compute the end offset safely before slicing.

## Liaison verification (CONFIRMED on trunk)

lower.rs ~3342-3344:
```
let mag_len = u32::from_le_bytes(len_field.try_into().ok()?) as usize;
let magnitude = after_sign.get(4..4 + mag_len)?.to_vec();
```
`mag_len` is a `u32` read from the wire (untrusted), cast to `usize`. `4 + mag_len`:
- On a 32-bit target (wasm32 — and rcdzc self-hosts to wasm), `usize` is 32-bit, so `4 + 0xFFFFFFFF`
  overflows → wraps to `3` in release (a `4..3` range → `.get()` returns `None`, so probably still
  SAFE by luck) or PANICS in a debug/overflow-checks build.
- On 64-bit it can't overflow from a u32, so the hazard is 32-bit-target + debug-overflow-checks
  specific — but the codec is meant to be TOTAL on untrusted input (the surrounding comment: "a
  truncated sign/length/magnitude yields `None` … never a panic").

Fix (per Copilot): `let end = 4usize.checked_add(mag_len)?; let magnitude = after_sign.get(4..end)?.to_vec();`
— returns `None` on overflow, preserving the never-panic contract. Small robustness fix.

Owner: v-metaprogramming (this is the `Ast.Int` codec — non-lossy length-prefixed sign+magnitude,
landed `910c45261` "Ast.Int codec … operator directive PART 1a"; quote/eval/Ast is v-metaprogramming's
lane). Routed as a note.
