# PR #2143 review — .github/workflows/checks.yml (v-nix) — OPEN — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2143 (full-CI-in-nix — body-swap guide-examples to the hermetic
nix check; kills the binaryen gate-flake). Copilot 1 inline, doc-accuracy on the retained lockstep note.

## the guide-examples comment still says "keep in lockstep with pages.yml's build", but this PR swaps the job to the HERMETIC NIX flow while pages.yml stays on the non-nix pinned-toolchain flow → the "lockstep" note now describes an intentional DIVERGENCE and misleads (Copilot, checks.yml:492) — doc-accuracy [VERIFIED, LOW]
> The updated comment claims this job "mirrors pages.yml's build" and should be kept in lockstep with it,
> but pages.yml currently uses the non-nix toolchain flow and only runs a smaller subset of checks
> (check:examples, check:calculator, build). This is now misleading documentation and will cause future
> drift/confusion unless pages.yml is updated too or the comment is reworded to describe the intentional
> divergence (pre-merge gate vs deploy-time subset).

VERIFIED both sides. #2143 diff (checks.yml:17-30): the guide-examples body now "runs the HERMETIC nix
`guide-examples`" (`checks.aarch64-linux.guide-examples`) — yet KEEPS the note "Keep in lockstep with
pages.yml's build; if that gains a check, add it to the nix check too" (diff:30) and "mirrors pages.yml's
build" (diff:17). But pages.yml (verified on trunk) is the NON-nix flow: pinned toolchain via
rust-toolchain.toml + `cargo binstall wasm-pack`, then `cargo xtask build` / `cargo xtask guide-wasm` +
`npm run check:examples` / `check:calculator` (pages.yml:60-88). So post-#2143 the two are structurally
different engines (hermetic nix check vs pinned-toolchain cargo/npm) — "lockstep/mirrors" is now
misleading: they can't be kept identical, and the pages.yml deploy path is a smaller subset. LOW/doc-
accuracy (no behavior bug — the nix swap is the intended full-CI-in-nix cutover). Fix per Copilot: reword
the comment to describe the INTENTIONAL divergence (this = the hermetic nix PRE-MERGE gate; pages.yml =
the deploy-time non-nix subset), rather than implying they mirror each other. This is part of your own
full-CI-in-nix cutover arc, so the reword should state the target end-state (does pages.yml eventually
move to nix too, or stay the deploy-time subset by design?). PR OPEN → foldable pre-merge. v-nix owns the
CI/nix pipeline.
