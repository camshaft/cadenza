# PR #1623 review comments — cdz-run/src/cli.rs + lib.rs (v-cdz-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1623 (remove RunOpts.nfc — NFC self-resolves from store).

## 1. --store <dir> + --runtime <path> won't affect NFC resolution (Copilot, cli.rs:210) — behavior
> `resolve_nfc_from_store` falls back to `CDZ_STORE` when `runtime_cache_dir` is `None` (e.g. when
> `--runtime` is used). As written, passing `--store <dir>` with `--runtime <path>` won't affect NFC
> resolution.

A CLI-flag-interaction edge: with `--runtime`, `runtime_cache_dir` is None → NFC falls back to `CDZ_STORE`/
default, IGNORING an explicit `--store <dir>`. Confirm whether that combination is intended; if `--store`
should scope NFC resolution too, thread it. MED (silent wrong-store for an NFC load — though the
hash-verify, if added per #1590, would catch a mismatch). Recommend v-cdz-tooling verify intent.

## 2. Comment references `opts.store`, but RunOpts has no `store` field (Copilot, lib.rs:1673) — doc [VERIFIED]
> This comment mentions `opts.store`, but `RunOpts` has no `store` field.

VERIFIED: RunOpts fields are export/args/runtime/runtime_cache_dir/host_responses — no `store`. The comment
at lib.rs:1668 ("The store is `opts.store` if …") is a dangling ref next to the new self-resolution logic.
Fix the comment to the actual source (runtime_cache_dir / CDZ_STORE / default). LOW/doc.
