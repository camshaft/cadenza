# PR#772 review comment — cdz run-rust `deps_dir = lib_dir.join("deps")` becomes `deps/deps` when the bin lives in deps/

Mirrored from GitHub PR review comment (Copilot), id `3628370863`.
PR: https://github.com/camshaft/cadenza/pull/772 (batch-staging; fix belongs on trunk)
Location: `implementation/seed/crates/cdz/src/main.rs:1635`

## Comment (verbatim)

> `deps_dir` is computed as `lib_dir.join("deps")`, but when the `cdz` binary itself lives in
> `target/<profile>/deps/` (common under `cargo test`), `lib_dir` is already the `deps` directory. In
> that case this becomes `.../deps/deps` and the hashed `lib<crate>-<hash>.rlib` artifacts in
> `.../deps/` are not found, reintroducing the missing-`cdz_num` link failure this change is trying to
> fix.

## Liaison verification (CONFIRMED on trunk/staging)

main.rs:1631 `let deps_dir = lib_dir.join("deps");`. The comment just above (lines ~1628-1630) itself
states the bin "sits in `target/<profile>/deps/` OR `target/<profile>/`". When `lib_dir` (=
`exe.parent()`) is ALREADY `.../deps/`, `lib_dir.join("deps")` = `.../deps/deps`, which does not exist
→ `deps_dir.is_dir()` is false → the `-L dependency=.../deps` search dir is NOT added → the hashed
`libcdz_num-<hash>.rlib` in `.../deps/` isn't found. Under `cargo test` (where the plain-named rlib is
often absent and only the hashed one exists in `deps/`), this reintroduces the exact
`E0433 cannot find crate cdz_num` link failure that `bb7f13c6d` ("find the cdz_rt/cdz_num rlib as the
plain OR hashed name") is meant to fix.

Fix: search BOTH `lib_dir` AND `lib_dir/deps` as dependency dirs, guarded by `is_dir()`, AND/OR detect
when `lib_dir` already ends in `deps` and don't double-append. Simplest robust form: add `-L
dependency=<lib_dir>` (already done) and, only if `lib_dir` does not already end in `deps`, also add
`lib_dir/deps`; PLUS always also consider `lib_dir` itself as the hashed-rlib dir when it ends in
`deps`. A regression test running the `cdz run-rust` path from a `deps/`-located bin would pin it.

Owner: v-cdz-tooling (`cdz/src/main.rs`; the run-rust rlib-resolution fix `bb7f13c6d`). Routed as a
note flagged CORRECTNESS (re-opens the CI link failure under the cargo-test layout).
