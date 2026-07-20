# RESOLVED (corpus-bugfix triage, 2026-07-20, trunk 7f6f074e3)

The wildcard-exported prelude-shadowing-variant bug NO LONGER reproduces. `cdz compile <dir> --entry suite`
now COMPILES clean (no CDZ0214 "T's constructor List is withheld") AND runs correctly:
  sz(T.List([T.Foo(1)])) == 2  ->  main returns 0.
So an IMPORTING file's `T.List(...)` / `T.Foo(...)` now correctly resolves to the IMPORTED constructor,
not the prelude `List`. The prelude-name collision in the importer's constructor resolution was fixed
between the 2026-07-15 filing and now.

IMPACT NOW CLEARED: the compiler port's `Ast` (which naturally has `List`/`Bool`/`Str` variants) can be
constructed + matched from a SEPARATE file (a test suite / any consumer), no longer needing the
keep-construction-in-declaring-module or smart-constructor-function workaround.
