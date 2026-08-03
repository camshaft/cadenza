# PR #1504 review comment — flake.nix (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1504 (PR: "[v-nix] d011cd9ea").
Follow-on to my #1496 flake.nix notes.

## Comment uses literal "+" as a conjunction (reads like a diff marker) (Copilot, flake.nix:105) — doc nit
> The comment uses a literal "+" as a conjunction ("committed +"), which reads like a diff marker and
> is easy to misinterpret. Prefer "and" for clarity.

Trivial: replace the "+" conjunction with "and" so it doesn't read as a diff `+` line.
