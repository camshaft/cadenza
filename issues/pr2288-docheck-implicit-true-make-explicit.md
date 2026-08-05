# PR #2288 review — flake.nix (v-nix) — OPEN — defensiveness/style [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2288 (warm dev-deps in crane cargoArtifacts by letting doCheck
default to true — fixes test-ubuntu 16→23m regression; the crane-caching arc, ties to my #2286/#2282). Copilot
1 inline, comment id 3723655325 at flake.nix:363.

## `doCheck = true` is relied on IMPLICITLY (crane's default) — the fix REMOVES `doCheck = false` rather than setting `doCheck = true` explicitly; making it explicit would guard against a future crane default change / copy-paste-without-context (Copilot, flake.nix:363) — defensiveness [VERIFIED, LOW]
> `doCheck` is currently being relied on implicitly (via crane's default), while the surrounding comment
> explains that `doCheck` must remain enabled for caching dev-deps/test targets. Making this explicit
> (`doCheck = true;`) would prevent future regressions if crane defaults change or if this block is copied
> elsewhere without context.

VERIFIED the diff: the fix DELETES `doCheck = false;` and adds a 10-line comment explaining that crane's
DEFAULT is `doCheck = true` and it must stay true (doCheck=false makes buildDepsOnly skip the
`cargo test --no-run` dep-warm → clippy wins its subset but cargoTest recompiles the whole dev-dep/test
closure → the 16→23m regression). So the corrected behavior rides on crane's DEFAULT, not an explicit
assignment. Copilot's point: since the WHOLE FIX is "doCheck must be true," pinning it explicitly
(`doCheck = true;`) is more robust than relying on the upstream default — a future crane bump that flips the
default, or a copy of this block elsewhere, would silently regress again.

LOW / defensiveness-nit (behavior is correct as-is; the extensive comment already documents the intent — this
is belt-and-suspenders). Net: reasonable to set `doCheck = true;` explicitly so the invariant the comment
describes is enforced in code, not just prose; but it's optional polish, not a correctness issue. v-nix owns
flake.nix. PR OPEN → foldable. (Related arc: my #2262 chmod / #2273 --locked / #2279 DRY-root / #2282
strict-pattern / #2286 landed — this is the doCheck/warm-cache layer of the same crane framework.)
