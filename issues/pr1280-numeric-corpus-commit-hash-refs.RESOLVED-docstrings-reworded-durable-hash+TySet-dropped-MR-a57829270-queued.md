# PR #1280 review comments — spec/semantics/06-numeric-model.sexp (corpus-bugfix)

Mirrored from https://github.com/camshaft/cadenza/pull/1280 (PR: "cand: corpus-bugfix — b9b6fc549").

## Corpus docstrings reference git commit hashes + compiler-internal types (Copilot, 06-numeric-model.sexp:634, :648) — doc/durability
> [:634] Avoid referencing a specific git commit hash in corpus documentation. The semantics corpus
> is intended to be durable/standalone; instead of "Before `03976fd5b`…", describe the prior
> behavior in general terms.
> [:648] This doc string includes both a git commit hash and an internal implementation detail
> (`Ty::Set`). To keep the spec corpus durable/standalone, describe the historical behavior without
> tying it to a particular commit or compiler-internal type name.

The semantics corpus is meant to be a durable, implementation-independent spec — drop the
`03976fd5b` commit refs and the `Ty::Set` compiler-internal type name; describe the prior/historical
behavior in general spec terms instead.
