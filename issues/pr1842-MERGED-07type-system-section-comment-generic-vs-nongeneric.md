# PR #1842 review comment — spec/semantics/07-type-system.sexp (corpus-bugfix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1842 (MERGED).

## Section comment inaccurate after inserting the wrong-arity user-generic cases (Copilot, 07-type-system.sexp:295) — doc/accuracy
> This section comment is now inaccurate after inserting the wrong-arity user-generic cases: the preceding
> two cases are about over/under-applying GENERIC user type constructors, not over-applying a NON-generic
> type. Update the wording so readers don't confuse the two.
The inserted wrong-arity-generic cases changed what the section covers; the header still describes
non-generic over-application. Reword to reflect the generic-ctor arity cases now present. LOW/doc. Fold
into the next 07-type-system edit per the no-standalone-polish steer.
