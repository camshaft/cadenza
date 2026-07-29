# PR#896 review comment — 11-modules abstract-opacity doc: inaccurate (mk) example + "MUST forbids" grammar (corpus-bugfix)

Mirrored from GitHub PR#896 review comment (Copilot), id `3674682113`.
File: `spec/semantics/11-modules.sexp:1500` — corpus doc → corpus-bugfix. Blame `cd0b59486` "corpus:
complete abstract-opacity sweep — direct-eq compound + read-side lookup CDZ0202 (v-inference
2f2be099c/23fb89ea4)". (This is the corpus companion of the v-inference compound-key/direct-eq soundness
fixes surfaced via PR#890.)

## Comment (verbatim)

- (id 3674682113, 11-modules.sexp:1500) "In this docstring, the example `(= (mk) (mk))` is not valid for
  the local `temp` module (here `mk` takes an `Int64`), and the phrasing 'MUST forbids' is ungrammatical.
  Adjust the example to use `(mk k)` and rephrase to 'MUST forbid …' to keep the explanation accurate and
  readable."

## Liaison verification (both sub-issues confirmed on trunk fb75237da; NOT the jargon false-positive class)

Case "a built-in comparison on a COMPOUND containing an abstract type is rejected …" (:1495). Its local
module is `(def (mk (: c Int64)) (T (* c 10)))` — `mk` takes ONE `Int64` arg. The doc (:1499-1500):
1. "…like the bare `(= (mk) (mk))` (v-inference 2f2be099c)". `(mk)` is ZERO args — inaccurate for THIS
   module's `mk` (which needs an `Int64`). Should be `(mk k)` to match (the executed `(input …)` correctly
   uses `(mk k)`). Doc-example-only (the pin runs the correct `(mk k)`), but the example reads wrong.
2. "…what the opacity MUST forbids, regardless of the surrounding tuple." — "MUST forbids" is
   ungrammatical (a MUST + bare-verb spec phrasing); should be "MUST forbid".

Both doc-only, behavior-neutral (the case's `(input)` + `(error CDZ0202)` pin are correct). Fix: `(mk)` →
`(mk k)` and "MUST forbids" → "MUST forbid" in the docstring.

(Confirmed this is NOT the 11-modules Copilot jargon false-positive class — cf. PR#857 "performs" which I
DISMISSED. This is a concrete example-arity + a real grammar slip, both verifiable; route it.)

Owner: **corpus-bugfix** (`spec/semantics/11-modules.sexp` case doc; `cd0b59486`). Two doc edits.
