# PR#813 review comment — 13-strings.sexp fromroman/dec-go bounds a scalar-indexed walk with String.byte-len (recurrence of the PR#797 class)

Mirrored from GitHub PR review comment (Copilot), id `3634949436`.
PR: https://github.com/camshaft/cadenza/pull/813 (batch-staging; fix belongs on trunk)
Location: `spec/semantics/13-strings.sexp:429` (`fromroman` seeding `dec-go`)

## Comment (verbatim)

> `dec-go` iterates using `String.at` (scalar indexing), but `fromroman` bounds the loop with
> `String.byte-len`. This reintroduces the exact scalar-vs-byte loop-bound bug described earlier in
> this file: for any multibyte input, `i` will run past the last scalar and hit `(None _u)` early,
> producing incorrect results. Use `String.scalar-len` when driving a `String.at` walk.

## Liaison verification (CONFIRMED on trunk — same class as PR#797, new instance)

13-strings.sexp ~428: `(def (fromroman (: s String)) (dec-go s 0 (String.byte-len s) 0))`. `dec-go`
walks `i` with scalar-indexed `String.at s i` / `String.at s (+ i 1)` but the bound `len` comes from
`String.byte-len`. Same bug corpus-bugfix already fixed for the paren-scan + split cases on PR#797
(`641821e45`) — this ROMAN decoder case landed later (`33efcb93f`) and reintroduced it. Roman numerals
are ASCII so it PASSES today, but it's a latent multibyte bug and contradicts the scalar-walk idiom.

Fix: `(dec-go s 0 (String.scalar-len s) 0)`. (Roman input is ASCII so output is unchanged; no baseline
move.) `.sexp` edit needs `xtask roundtrip` + `cargo test -p cadenza-syntax --test corpus_roundtrip`.

Owner: **corpus-bugfix** (spec/semantics lane — NOT v-compiler-ml; see the routing memory). Routed as
an issue. NB for the PM: consider a quick grep for other `String.at`-walks bounded by `byte-len` across
the corpus while you're in there — this is the 3rd instance (paren-scan, split, now fromroman).
