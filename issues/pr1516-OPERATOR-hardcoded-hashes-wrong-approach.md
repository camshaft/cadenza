# PR #1516 — OPERATOR DIRECTIVE (camshaft) — flake.nix (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1516 (PR: "[v-nix] 1772a8449").
⚠ This is a comment from the OPERATOR (camshaft), not a bot — a design directive on the v-nix
workstream that the vertical would otherwise never see (it's unattended).

## Operator, flake.nix:136 — "Why are we hard-coding hashes here? That's the wrong approach."

Context (from Copilot's overview of the same PR): the flake extends the N1 pipeline to build TWO
runtime components (RELEASE + a new DEBUG-COUNTERS built with `--features debug-counters`), both
enforced via FIXED-OUTPUT DERIVATIONS with hard-coded output hashes to prevent drift.

The operator is rejecting the hard-coded-hash approach for the runtime derivations. This connects to
my earlier #1496 note (the flake pinning REQUIRED_RUNTIME_HASH). The operator wants a different design
— NOT hand-maintained hashes in flake.nix. v-nix should rethink the runtime FOD approach per this
directive (e.g. derive the hash from the build / import-from-derivation / a generated lockfile rather
than hard-coding), and if the intended design isn't clear, raise it back through the concierge.

(Routing note: github-liaison normally ignores human PR comments, but an operator DESIGN DIRECTIVE on
an unattended vertical's PR is exactly the inbound bridge — relaying to v-nix + FYI concierge.)
