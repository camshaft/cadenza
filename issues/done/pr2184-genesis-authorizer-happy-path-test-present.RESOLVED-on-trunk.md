# PR #2184 review — cdz-agent-host/src/factory.rs (v-agent-harness-host) — OPEN — 1 test-coverage (LOW-MED) + 1 efficiency (LOW) [VERIFIED]

https://github.com/camshaft/cadenza/pull/2184 (genesis authorizer-install glue — resolve
bootstrap/authorizer-hash + reload the real policy; closes the genesis loop). Copilot 2 inline.

## `install_genesis_authorizer` has tests for the error/empty cases but NO happy-path test proving a valid 32-byte hash present in the blob store installs successfully (`Ok(true)`) + changes the authz surface (Copilot, factory.rs:794) — test-coverage [VERIFIED, LOW-MED]
> The new tests cover the error/empty cases, but there's no happy-path test proving that a valid 32-byte
> hash present in the blob store actually results in a successful install (and that the session's
> capabilities/authz surface changes as expected). Consider an env-gated test (like the existing
> `CEDAR_POLICY_COMPONENT`-gated e2es) that stores a real policy component in `MemBlobStore`, seeds
> genesis with its hash bytes, calls `install_genesis_authorizer`, and asserts `Ok(true)` plus an
> observable post-condition (e.g., capability manifest differs from deny-all).

VERIFIED in the #2184 diff: the 3 new tests are `install_genesis_authorizer_root_only_is_ok_false`
(diff:113), `_bad_length_hash_is_a_clean_err` (diff:134), `_absent_blob_is_a_clean_err` (diff:152) — all
error/empty paths. The SUCCESS path (`Ok(true)`, diff:60 — a valid 32-byte hash whose component IS in the
store → real authorizer installed) has NO test. LOW-MED/test-coverage: this PR "closes the genesis loop",
and the happy path (install succeeds + the session's authz flips from deny-all to the real policy) is the
one that matters — an untested success path could regress silently (e.g. install returns Ok(true) but
doesn't actually swap the authorizer). Fix per Copilot: an env-gated e2e (mirror the
`CEDAR_POLICY_COMPONENT`-gated tests) — store a real policy component in `MemBlobStore`, seed genesis with
its hash, call `install_genesis_authorizer`, assert `Ok(true)` + an observable post-condition (capability
manifest ≠ deny-all). (The error-path coverage is good; this adds the missing success half.)

## `install_genesis_authorizer` allocates an intermediate `Vec` (`b.to_vec()`) just to end the KV borrow, then `try_into()`s a `[u8; 32]` — can copy directly from the borrowed slice into `[u8; 32]` (Copilot, factory.rs:350) — efficiency [VERIFIED, LOW]
> This allocates an intermediate Vec just to end the KV borrow; since the authorizer hash must be exactly
> 32 bytes, you can copy directly into a `[u8; 32]` via `try_into()` and avoid the extra allocation/copy.
VERIFIED in the diff: `let hash_bytes: Vec<u8> = match session…{ Some(b) => b.to_vec() … }` (diff:34-39)
then `let arr: [u8; 32] = hash_bytes.as_slice().try_into()…` (diff:42). The `to_vec()` heap-allocs just to
release the KV borrow before the `try_into`. Since the target is a fixed `[u8; 32]`, you can `try_into()`
the BORROWED slice directly into the array (copies 32 bytes onto the stack, no heap alloc) inside the match
arm, then the borrow ends. LOW/efficiency (once-per-genesis, so negligible — but free + cleaner). Fix per
Copilot: `let arr: [u8; 32] = b.try_into().map_err(|_| …)?;` in the `Some(b)` arm. v-agent-harness-host
owns cdz-agent-host. PR OPEN → both foldable. The happy-path test is the one worth adding.
