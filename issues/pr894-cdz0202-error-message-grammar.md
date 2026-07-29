# PR#894 review comment — CDZ0202 abstract-key error message grammar (v-inference, low-pri)

Mirrored from GitHub PR#894 review comment (Copilot), id `3674022708`.
File: `implementation/seed/crates/rcdzc/src/infer.rs:8204` — v-inference (CDZ0202 message). Blame
`23fb89ea4` "infer: CDZ0202 abstract-key gate covers LOOKUP/MEMBERSHIP/set-algebra prims".

## Comment (verbatim)

- (id 3674022708, infer.rs:8204) "Grammar: the error message says 'compare it through a function the
  module that declares it exports', which is missing 'that' and reads awkwardly."

### Liaison verification (confirmed on trunk ccc2048dc)

The CDZ0202 message ends: "…compare it through a function the module that declares it exports". The
clause "a function the module that declares it exports" uses an omitted relative pronoun ("a function
[that] the module … exports") which is grammatical but collides with the inner "the module that declares
it" → a clunky double read. Low-priority prose polish (a user-facing diagnostic, so worth a light touch):
e.g. "compare it through a function exported by the module that declares it" reads cleaner and avoids the
stacked "that". Message-only, behavior-neutral. Not a blocker — route as an optional polish; decline is
fine if v-inference prefers the current wording.

Owner: **v-inference** (CDZ0202 diagnostic, `23fb89ea4`). Optional message reword.
