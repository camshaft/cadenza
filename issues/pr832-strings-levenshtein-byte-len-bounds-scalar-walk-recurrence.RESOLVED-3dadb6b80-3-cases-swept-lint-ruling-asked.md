# PR#832 review comment — 13-strings Levenshtein case bounds a scalar String.at walk with String.byte-len (recurrence #9)

Mirrored from GitHub PR review comment (Copilot), id `3636374738`.
PR: https://github.com/camshaft/cadenza/pull/832 (batch-staging; fix belongs on trunk)
Location: `spec/semantics/13-strings.sexp:370` (the EDIT DISTANCE / Levenshtein case, landed `b57b431f9`)

## Comment (verbatim)

> This case claims to roll the Levenshtein DP over string scalars and indexes characters via
> `String.at`, but it bounds `i`/`j` using `String.byte-len`. For any non-ASCII input, `byte-len` can
> exceed the scalar count, causing `String.at` to return None and `Option.expect` to trap (and it would
> compute incorrect distances if it didn't). Use `String.scalar-len` for `la`/`lb` to match scalar
> indexing.

## Liaison verification (CONFIRMED on trunk — same class as PR#797/#813, NEW instance)

`lev` (13-strings.sexp ~367-371): `(def la (String.byte-len a)) (def lb (String.byte-len b))` then
`rows`/`row-go` walk `i`/`j` with scalar-indexed `String.at a (- i 1)` under `Option.expect`. For a
multibyte input, `byte-len > scalar-len` → `String.at` returns None past the last scalar → `Option.expect
"ca"` TRAPS (worse than the earlier cases, which returned a sentinel — this one hard-traps). ASCII test
inputs (`"kitten"`/`"sitting"`/`"flaw"`/`"lawn"`) so it PASSES today.

This is the SAME byte-len-vs-scalar-len class corpus-bugfix already closed corpus-wide via
`641821e45` + `07d0e2196` + `7de4febfd` (8 sites). This EDIT DISTANCE case (`b57b431f9`) landed AFTER
that sweep and reintroduced the pattern — so the class isn't staying closed as new string cases are
authored.

Fix: `(def la (String.scalar-len a)) (def lb (String.scalar-len b))`. ASCII output unchanged, no
baseline move; `.sexp` edit → `xtask roundtrip` + `cargo test -p cadenza-syntax --test corpus_roundtrip`.

RECURRENCE FLAG for the PM: this is the ~9th instance of the same bug (5 in the 07d0e2196 sweep + split/
scan + hexdec + now Levenshtein). New string-walk cases keep seeding the loop bound from `byte-len`.
Worth a durable guard: a corpus-lint/CI check that flags a `String.byte-len` feeding a loop that indexes
via `String.at`/`String.slice`, OR a corpus-authoring convention note ("scalar-indexed walk ⇒
scalar-len bound"). Otherwise every new string DP/scan case re-lands it.

Owner: **corpus-bugfix** (spec/semantics lane — NOT v-compiler-ml). Routed as an issue.
