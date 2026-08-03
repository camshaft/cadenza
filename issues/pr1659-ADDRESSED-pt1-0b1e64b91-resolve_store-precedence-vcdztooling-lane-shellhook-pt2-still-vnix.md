# PR #1659 review comments — flake.nix (v-nix) — OPEN

https://github.com/camshaft/cadenza/pull/1659 (R4 — devShell exports CDZ_STORE → the nix component store).

## 1. CDZ_STORE reaches cdz-run but NOT the `cdz` CLI (which shadows it with explicit runtime_cache_dir) (Copilot, flake.nix:481) — correctness [VERIFIED, cross-cutting w/ v-cdz-tooling]
> `CDZ_STORE` is only consulted by `cdz-run` when no explicit `runtime_cache_dir` is provided. The `cdz`
> CLI currently resolves the store as `args.store.unwrap_or_else(default_store)` (no `CDZ_STORE` env
> support), then passes `runtime_cache_dir: Some(store)` into `cdz-run` — so the env export doesn't take
> effect for `cdz` invocations.

VERIFIED against cdz/src/main.rs:3716 + :4451 — both do `let store = args.store.clone()
.unwrap_or_else(default_store)` with NO `CDZ_STORE` env consultation, then pass `runtime_cache_dir:
Some(store)` which SHADOWS cdz-run's env fallback. So R4's devShell `CDZ_STORE` export works for direct
`cdz-run` but is a no-op for the `cdz` CLI path — a gap in the R4 mechanism this PR is building. CROSS-
CUTTING: the fix is in the `cdz` CLI (v-cdz-tooling territory) — make `args.store` fall back to
`CDZ_STORE` before `default_store` (`args.store.or_else(|| env CDZ_STORE).unwrap_or_else(default_store)`).
v-nix + v-cdz-tooling should coordinate — the flake export is only half the wiring. MED.

## 2. shellHook unconditionally overwrites pre-existing CDZ_STORE + always prints (Copilot, flake.nix:488) — UX
> The `shellHook` unconditionally overwrites any pre-existing `CDZ_STORE` and always prints to stdout.
> This can be disruptive for `nix develop --command …` / non-interactive uses and prevents intentional
> per-session overrides.

Guard the export (`: "${CDZ_STORE:=<nix-store>}"` to respect a pre-set value) and gate the echo on an
interactive shell (or send it to stderr) so `nix develop --command` stays quiet + overridable. LOW/UX.
