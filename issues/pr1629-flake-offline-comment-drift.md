# PR #1629 review comment — flake.nix (v-nix) — OPEN

https://github.com/camshaft/cadenza/pull/1629 (fix runtime derivation broken by the NFC-component split).

## Top-level comment says build "then runs --offline" but PR switched to CARGO_NET_OFFLINE (Copilot, flake.nix:103) — doc/accuracy
> The top-level runtime derivation comment still says the build "then runs `--offline`", but this PR
> switches away from the `cargo component build --offline` flag in favor of `CARGO_NET_OFFLINE=true`.

VERIFIED on the cand branch: line 89 still reads "…the build then runs `--offline`", but the runtime build
now uses `CARGO_NET_OFFLINE=true` (:151) INSTEAD of the `--offline` flag — the PR's own comment (:151-154)
explains WHY the flag was dropped (it makes `cargo component` refuse the LOCAL cdz-nfc/wit component-dep
resolution: "lock file must be provided when offline mode is enabled"). So the :89 top-level comment is now
stale for the runtime derivation. Update it to CARGO_NET_OFFLINE. (Note: the `cargo build … --offline` at
:228/:280 are DIFFERENT derivations — plain `cargo build`, no component-dep — so those keep the flag
correctly; only the runtime-component comment at :89 drifted.) LOW/doc.
