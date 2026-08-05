# PR #2198 review — spec/semantics/14-effects-and-handlers.sexp (v-effects / corpus) — OPEN — corpus doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2198 (2-pin op-arg let-lift escalations — sibling-args order +
depth-3 chain; batch 56). Copilot 1 inline. Corpus-bugfix ZONE — a docstring nesting mis-description, not
a case-semantics issue.

## the depth-3 case docstring inverts the nesting: it says "the INNERMOST perform's argument is itself a perform", but in `(C.inn (B.mid (A.get)))` it's the OUTERMOST perform (`C.inn`) whose arg is a perform; `A.get`'s arg is `Unit`, so the innermost perform's arg is NOT a perform (Copilot, 14-effects-and-handlers.sexp:4262) — corpus doc-accuracy [VERIFIED, LOW]
> Docstring describes the nesting incorrectly: in `(C.inn (B.mid (A.get)))`, it's the *outermost* perform
> (`C.inn`) whose argument is a perform (`B.mid ...`), and `B.mid`'s argument is the outer handler's
> perform (`A.get`). `A.get`'s argument is `Unit`, so it isn't the "innermost perform's argument" that is
> a perform.

VERIFIED in the #2198 diff: the depth-3 case (diff:88-91) has body `(C.inn (B.mid (A.get)))` and the doc
says "the innermost perform's argument is itself a perform whose OWN argument is a perform of the outermost
op". But the structure is: `C.inn` (OUTERMOST) takes `(B.mid (A.get))` — a perform — as its arg; `B.mid`
takes `(A.get)` — a perform — as its arg; `A.get` (INNERMOST) takes `Unit`. So it's the OUTERMOST perform
whose arg is a perform, cascading inward; the INNERMOST (`A.get`) has a `Unit` arg, NOT a perform. The
docstring has the innermost/outermost roles inverted. LOW/corpus-doc-accuracy (the CASE semantics + the
7→107→214 evaluation are correct — A.get reads 7, B.mid adds 100, C.inn doubles; only the prose describing
WHICH perform nests is backwards). Fix: reword to "the OUTERMOST perform's argument is itself a perform,
cascading down to the innermost `A.get` (whose arg is `Unit`) — so the lift must fire at two nesting
levels". 

CORPUS DISCIPLINE: cite the CASE-NAME ("The depth face of the op-arg let-lift…"), not the line number, in
any code-comment reference; a corpus edit must pass the ML round-trip, not just gate — but this is a
docstring-only reword, low-risk. Owner = whoever authored batch 56 (v-effects / breaker corpus lane). PR
OPEN → foldable pre-merge.
