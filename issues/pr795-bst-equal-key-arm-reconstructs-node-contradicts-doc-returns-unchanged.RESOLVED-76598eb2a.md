# PR#795 review comment — BST corpus case: equal-key arm reconstructs a Node but the doc says "returns the node UNCHANGED"

Mirrored from GitHub PR review comment (Copilot), id `3632696884`.
PR: https://github.com/camshaft/cadenza/pull/795 (batch-staging; fix belongs on trunk)
Location: `spec/semantics/05-compound-types.sexp:5264` (case "a BST built by comparison-driven inserts…")

## Comment (verbatim)

> In the BST `insert` function, the equal-key branch is described in the case doc as returning the
> node unchanged, but the implementation reconstructs a new `(Node (tuple l k r))`. Even if the
> semantics don't expose pointer identity, returning `t` here better matches the test's stated intent
> (dedup without path-copy/allocation on equality) and avoids confusing readers.

## Liaison verification (CONFIRMED on trunk — doc/impl mismatch, behavior-neutral)

- Case doc (05-compound-types.sexp ~5245-5247): "…with the EQUAL-key arm returning the node UNCHANGED
  (BST dedup)."
- `insert` body equal-key branch (the innermost else, ~5264): `(Node (tuple l k r))` — it RECONSTRUCTS
  a fresh Node from the destructured `l k r` rather than returning the already-matched `t`.

So the code contradicts its own doc's stated "returns unchanged / dedup without path-copy on equality"
intent. The output (13589 / dedup behavior) is unchanged either way — value-equal — so this is
behavior-neutral; but the case is meant to PIN the persistent-structure "no path-copy on equality"
discipline, and as written it path-copies on equality (the opposite of what it documents).

Fix: change the equal-key arm from `(Node (tuple l k r))` to `t` (the matched BST binding is in scope
in the `((Node p) …)` arm — return `t`). Re-verify output stays 13589 (it will; dedup semantics
identical). A `.sexp` edit needs `xtask roundtrip` + `cargo test -p cadenza-syntax --test
corpus_roundtrip`; the case TITLE is unchanged so the gate baselines don't move.

Owner: v-compiler-ml (semantics corpus; the BST case landed `9b30aed25`). Routed as a note. Minor but
it's a self-contradicting pin (the doc claims the exact property the code doesn't exhibit).
