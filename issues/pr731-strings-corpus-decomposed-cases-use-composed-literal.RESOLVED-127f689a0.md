# PR#731 review comments — 13-strings.sexp "decomposed literal" cases actually use the COMPOSED literal

Mirrored from GitHub PR review comments (Copilot), ids `3620914439`, `3620914465`.
PR: https://github.com/camshaft/cadenza/pull/731 (merged; fix still belongs on trunk)
Location: `spec/semantics/13-strings.sexp:566` and `:577`

## Comments (verbatim)

- (id 3620914439, :566) "This case is described as exercising a decomposed literal (e + U+0301),
  but the test input uses the composed literal \"café\". As written it duplicates the earlier
  composed-literal indexing case and doesn't actually pin normalization on the addressing axis."
- (id 3620914465, :577) "This case is described as using a decomposed literal, but the scalar-len
  and String.at calls are on the composed literal \"café\", so it doesn't actually test the
  normalized-length boundary for decomposed input."

## Liaison verification (CONFIRMED on trunk)

Both cases' `doc` claim the literal is the DECOMPOSED "café" (c, a, f, e + U+0301 combining acute —
five raw scalars). But a byte dump of the actual source literal:

    $ sed -n '566p' 13-strings.sexp | grep -oP '"caf[^"]*"' | xxd
    00000000: 2263 6166 c3a9 22   "caf..""

is `63 61 66 c3 a9` = c, a, f, **U+00E9** (the COMPOSED single-scalar é) — NOT "e + U+0301". So the
input is already NFC-composed; the reader's NFC normalization is a no-op on it, and the cases do NOT
actually exercise decomposed→normalized behavior (case :566 also then duplicates the earlier
composed-literal indexing case). The value assertions (Some "é", sentinel 40) still pass, but the
tests don't pin what their docs claim.

Fix: put a genuinely DECOMPOSED literal in the source (c, a, f, e, then U+0301 COMBINING ACUTE ACCENT
= bytes `63 61 66 65 cc81`), so the reader's NFC step has real work and the case pins
decomposed→normalized indexing/length. Re-verify the value asserts (Some "é" at index 3; scalar-len 4
+ .at 4 = None → 40) hold on the decomposed input. Owner: corpus semantics.

NOTE for PM: a corpus `.sexp` edit needs the roundtrip gates
(`xtask roundtrip` + `cargo test -p cadenza-syntax --test corpus_roundtrip`), and the edit inserts a
raw combining codepoint — ensure the file stays valid UTF-8 and round-trips. Filing to corpus-bugfix.
