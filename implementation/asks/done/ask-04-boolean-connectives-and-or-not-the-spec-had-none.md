## 4. 🟢 Boolean connectives (`and`/`or`/`not`) — the spec had none

**Finding.** A routine compiler predicate (the signed-LEB128 terminator) needs `(and …)`/`(or …)`;
they were absent from seed, corpus, AND spec.

**Status.** 🟢 **DONE end-to-end.** Requirement in `core-semantics.md` §"Boolean Connectives
Short-Circuit"; 6 corpus cases in `02-binding-and-control.sexp`; seed lowering landed (desugar to
short-circuit `if`). Own learning:
`spec/learnings/2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives.md`.

---
