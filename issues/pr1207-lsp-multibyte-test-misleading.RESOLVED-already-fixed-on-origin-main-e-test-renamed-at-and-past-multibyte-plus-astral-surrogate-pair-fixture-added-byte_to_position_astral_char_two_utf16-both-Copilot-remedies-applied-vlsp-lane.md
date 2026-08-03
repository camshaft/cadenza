# PR #1207 review comment — cdz/src/lsp.rs (v-lsp)

Mirrored from https://github.com/camshaft/cadenza/pull/1207 (PR: "cand: v-lsp — d062a909d").

## "inside a multibyte char" LSP test is mislabeled — `é` is 1 UTF-16 unit (Copilot, lsp.rs:4037) — test/doc
> This test is described as exercising a cursor position "inside a multibyte char", but using `é`
> doesn't actually create an "inside" case in LSP terms because `é` is **one UTF-16 code unit**. The
> only time an editor can send a column that lands inside a single Unicode scalar is for non-BMP
> code points (UTF-16 surrogate pairs). As written, the test is still useful for "just after a
> multibyte UTF-8 char", but the name/comment are misleading and don't pin the surrogate-pair edge
> case.

The test is fine as a "just after a multibyte UTF-8 char" case, but the name/comment claim an
"inside a multibyte char" scenario that `é` can't produce in LSP's UTF-16 column model. Either rename
to reflect what it actually covers, or add a non-BMP (surrogate-pair) fixture to genuinely pin the
inside-a-scalar edge case.
