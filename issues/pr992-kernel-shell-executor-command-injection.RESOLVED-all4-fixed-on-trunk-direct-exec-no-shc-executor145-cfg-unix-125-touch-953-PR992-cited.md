# PR#992 review comments — cdz-kernel ShellExecutor COMMAND INJECTION (⚠⚠ security) + non-Unix cfg + rm-rf test (v-agent-harness)

Mirrored from GitHub PR#992 review comments (Copilot), ids `3695328503` + `3695331941` (executor.rs:86,
command injection), `3695331944` (executor.rs:72, non-Unix), `3695331953` (loop_and_recovery.rs:611,
rm -rf test), `3695331961` (loop_and_recovery.rs:550/596, cfg(unix) tests). All
`implementation/seed/crates/cdz-kernel/*` → v-agent-harness. Blame `f8cf1c2f3` "feat(cdz-kernel): real
ShellExecutor behind `live-exec` — first executor that touches the world".

⚠⚠ SECURITY (command injection in the agent-runtime executor — a defensive/authorized-runtime hardening,
squarely in scope).

## Comment 1 (verbatim) — executor.rs:86, COMMAND INJECTION (ids 3695328503 + 3695331941, same site)

- (id 3695328503) "**Command Injection Vulnerability**: Using `sh -c` with `req.target` directly creates
  command injection risk. Even with upstream capability gating, shell metacharacters in `req.target`
  (`;`, `|`, `&&`, `$()`) allow arbitrary command execution. The comment claims 'capability-gated
  upstream' but shell injection bypasses prefix matching—`echo safe; rm -rf /` matches the prefix
  `echo ` yet executes destructive commands. Replace `sh -c` with direct command execution or strict
  shell escaping. For v0 … split the target into program and args, then execute directly without shell
  interpolation." [CWE-78]
- (id 3695331941) "`ShellExecutor` runs `req.target` via `sh -c`, but the kernel's SEC-F1
  `ResourcePredicate::Prefix` check is only a plain `starts_with` (effect.rs:93). A target like
  `echo ok; rm -rf …` still starts with `echo ` and would be authorized, letting arbitrary shell
  metacharacters slip past a naive allow-list."

### Liaison verification (CONFIRMED on trunk 18dba958f)

executor.rs:82-85: `Command::new("sh").arg("-c").arg(&req.target).output()`, comment "capability-gated
upstream (SEC-F1) — this executor trusts that authorization". The ONLY gate (effect.rs:93):
`ResourcePredicate::Prefix(p) => target.starts_with(p)`. So a grant `Prefix("echo ")` AUTHORIZES
`echo ok; rm -rf /tmp/x` — `starts_with("echo ")` is true, but `sh -c` runs the whole compound → the
`rm -rf` executes. The prefix allow-list is defeated by ANY shell metacharacter (`;`/`|`/`&&`/`$()`/
backtick). Real command injection (CWE-78) in the runtime that "touches the world". Fix (Copilot's, for
v0): split `req.target` into program + args and `Command::new(prog).args(rest)` — NO `sh -c`, so
metacharacters are literal, not shell-interpreted (a `;` becomes an argument, not a separator). A
production version wants a real arg parser / pre-parsed args. Highest priority.

## Comment 2 (verbatim) — executor.rs:72, non-Unix

- (id 3695331944) "This implementation hard-depends on a POSIX `sh`. If someone enables `live-exec` on a
  non-Unix target, the type will compile but the executor will always fail at runtime… safer to make the
  availability explicit with `cfg(unix)` so the feature can't be accidentally enabled on unsupported
  platforms."

## Comments 3-4 (verbatim) — tests

- (id 3695331953, loop_and_recovery.rs:611) "Even though this command is intended to be denied, keeping
  destructive commands like `rm -rf` in tests raises the blast radius if an authorization regression ever
  slips in (especially now that CI runs `--features live-exec`). Prefer a harmless command (e.g. `touch`)
  so an unexpected execution is observable but not destructive."
- (id 3695331961, loop_and_recovery.rs:550, +596) "These tests exercise `ShellExecutor`, which relies on
  `sh`. To avoid feature-enabled builds failing on non-Unix targets, gate the tests with `cfg(unix)` as
  well as `live-exec`."

### Liaison verification (confirmed on trunk 18dba958f)

loop_and_recovery.rs:609 literally uses `target: "rm -rf /tmp/should-never-run".into()` as a DENIED
target; CI now runs `--features live-exec` — if an authz regression ever authorized it, the real `rm -rf`
runs (destructive test-blast-radius). Use `touch /tmp/…` instead: an authz regression still shows as an
unexpected execution, but harmless. And the `sh`-dependent tests + executor should be `#[cfg(all(unix,
feature="live-exec"))]` so `live-exec` on non-Unix doesn't compile a runtime-always-fails executor.

Owner: **v-agent-harness** (`cdz-kernel`; `f8cf1c2f3`). PRIORITIZE the injection fix (split-args, no
`sh -c`); + `cfg(unix)` gate the executor/tests; + swap the test's `rm -rf` for `touch`. Gate = cdz-kernel's
own `cargo test`+clippy (incl `--features live-exec`), NOT `xtask check`.
