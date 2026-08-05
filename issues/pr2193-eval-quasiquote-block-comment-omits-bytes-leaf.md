# PR #2193 review — spec/semantics/12-metaprogramming.sexp (v-metaprogramming) — OPEN — corpus doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2193 (pin eval of a quasiquote with an active BYTES unquote).
Copilot 1 inline. Corpus-bugfix ZONE (`.sexp`) — the finding is a stale GROUP comment, not a case-semantics
issue.

## the block comment above the eval-splice cases says the idiom "composes with the FLOAT and STRING leaves", but this PR adds a BYTES leaf pin → the comment now under-enumerates the leaf family it documents (Copilot, 12-metaprogramming.sexp:236) — corpus doc-accuracy [VERIFIED, LOW]
> The block comment immediately above these cases says the eval-of-quasiquote idiom composes with FLOAT
> and STRING leaves, but this PR adds a BYTES leaf pin as well. Updating that comment keeps the surrounding
> documentation accurate for future readers.

VERIFIED against the file + diff. The group comment (12-metaprogramming.sexp:220-222) reads: "The
eval-of-quasiquote macro idiom composes with the FLOAT and STRING leaves, and `print` renders a … not only
integers/names. A float unquote lifts + reconstructs + folds like an integer one; a string …". This #2193
adds the CASE "eval of a quasiquote-built form with a byte-string unquote folds" (diff:45) whose OWN doc
says it is "The BYTES companion of the eval-splice idiom, closing the leaf-lift family for the `Ast.Bytes`"
(diff:46). So the new pin extends the leaf-lift family to BYTES, but the group comment still enumerates
only FLOAT + STRING — stale/under-enumerating. LOW/corpus-doc-accuracy (comment only; the case + its doc
are correct, and the pin itself is sound). Fix per Copilot: update the block comment (:220-222) to include
BYTES — e.g. "composes with the FLOAT, STRING, and BYTES leaves … a byte-string unquote lifts +
reconstructs + folds like the others". 

CORPUS DISCIPLINE NOTE for v-metaprogramming: cite the CASE-NAME ("eval of a quasiquote-built form with a
byte-string unquote folds"), not the line number, in any code-comment reference; and a corpus edit must
pass the ML round-trip, not just gate — but this is a comment-only tweak in the group header, so low-risk.
v-metaprogramming owns the metaprog corpus. PR OPEN → foldable pre-merge.
