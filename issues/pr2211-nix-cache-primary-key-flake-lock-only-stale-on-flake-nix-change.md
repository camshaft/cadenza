# PR #2211 review — .github/workflows/checks.yml (v-nix) — OPEN — cache-correctness [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2211 (PILOT the /nix/store cache — cache-nix-action, save+purge
on main only; the re-cut of my #2209 with the purge-scope fix). Copilot 1 inline — a cache-key staleness
gap (third layer on this cache config).

## `primary-key` is `hashFiles('flake.lock')` ONLY → a `flake.nix`/nix-expr change that alters the closure WITHOUT a lockfile change produces the SAME key; GHA caches are immutable per key, so the /nix/store cache goes STALE on `main` and never refreshes (Copilot, checks.yml:49) — cache-correctness [VERIFIED, LOW-MED]
> `primary-key` is derived only from `flake.lock`. Because GitHub Action caches are immutable per key,
> changes to `flake.nix` (or other Nix expressions) that don't update the lockfile won't produce a new
> cache key, so the /nix/store cache can become effectively stale and never get refreshed on `main` even
> though the closure changed. Consider including `flake.nix` in the hash so cache updates when the
> evaluated flake outputs change without a lockfile change (you can still keep
> `restore-prefixes-first-match` for warm starts).

VERIFIED in the #2211 diff: `primary-key: nix-${runner.os}-${runner.arch}-${hashFiles('flake.lock')}`
(diff:16). Keyed ONLY on `flake.lock`. GHA cache semantics: a key, once saved, is IMMUTABLE — a later run
with the same key RESTORES the old cache, never overwrites. So a `flake.nix` edit that changes the
evaluated closure (a new/edited `mkDerivation`, build input, dep expression, `gc-max-store-size`, etc.)
WITHOUT bumping `flake.lock` yields the SAME `primary-key` → `main` keeps restoring the pre-change
/nix/store cache indefinitely, even though what it should cache changed. LOW-MED/cache-correctness (a
stale-closure cache → wrong/missing store paths restored → builds fall back to rebuild at best, or use
stale artifacts at worst; and since this is the PILOT template for a 7-job rollout, the staleness would
propagate). Fix per Copilot: fold `flake.nix` into the primary-key hash — `hashFiles('flake.lock',
'flake.nix')` (or the whole nix expr set if there are more `.nix` files in the closure) — keeping
`restore-prefixes-first-match` (diff:17) for warm partial restores. That way a closure-changing flake.nix
edit rotates the key + saves a fresh cache on main. v-nix owns CI. PR OPEN → foldable pre-merge.
(This is the THIRD finding on this cache config — after the #2209 purge-scope MED [fixed here] and the
trunk/main comment LOW — the cache-key derivation is the remaining correctness gap. Worth nailing before
the 7-job rollout carries the template.)
