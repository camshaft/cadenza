# PR#903 review comment — provider-cache atomic write: Windows rename-over-existing fails to self-heal + write-failure leaks temp (v-cdz-tooling)

Mirrored from GitHub PR#903 review comment (Copilot), id `3678384138`.
File: `implementation/seed/crates/cdz/src/main.rs:4053` — v-cdz-tooling. Blame `6b8fa35c6` "cdz test:
atomic provider-cache write + test suffix-filter + doc reconcile (PR#901)" — a FOLLOW-ON to the very
atomic-write fix I routed for PR#901 (`3677648876`).

## Comment (verbatim)

- (id 3678384138, cdz/src/main.rs:4053) "On Windows, `std::fs::rename` fails if the destination path
  already exists. In the cache self-heal path, a corrupt `{key}.provider.wasm` file can already exist, so
  this rename can fail and leave the corrupt file in place (the temp is deleted), causing repeated cache
  misses / failed healing. Also, if `write(&tmp, ...)` fails after creating a partial temp file, the
  current code leaves the temp behind. Consider cleaning up the temp on write failure, and on rename
  failure, removing the destination and retrying once (best-effort) so an existing corrupt cache entry
  can be replaced."

## Liaison verification (confirmed on trunk bb3ca7df8)

Current block (main.rs:4046-4053):
```
let final_path = dir.join(format!("{key}.provider.wasm"));
let tmp = dir.join(format!(".{key}.provider.wasm.{}.tmp", std::process::id()));
if std::fs::write(&tmp, bytes).is_ok()
    && std::fs::rename(&tmp, &final_path).is_err()
{
    let _ = std::fs::remove_file(&tmp); // rename failed — don't leave the temp behind
}
```
1. **Windows rename-over-existing**: POSIX `rename` atomically REPLACES an existing dest, but Windows
   `std::fs::rename` FAILS if the dest exists. In the self-heal path a corrupt `{key}.provider.wasm` is
   already present → on Windows the rename errs → the corrupt file STAYS, the temp is removed → the entry
   never heals, repeated misses. (SEVERITY: Cadenza's primary/CI platform is Linux, where the POSIX
   replace semantics make this a non-issue; the Windows path is lower-priority but real for any Windows
   dev — a genuine correctness gap on that platform.)
2. **Write-failure temp leak**: if `std::fs::write(&tmp, bytes)` fails after creating a PARTIAL temp, the
   `is_ok()` guard is false → the `&&` short-circuits → the `remove_file(&tmp)` never runs → the partial
   `.tmp` leaks in the cache dir. Platform-agnostic (a full/RO FS mid-write, or a signal). Minor litter,
   but it accumulates and the temp name is pid-stamped so it won't be reused/overwritten.

Fix (Copilot's, sound): (a) on write failure, `remove_file(&tmp)` too (clean up the partial temp); (b) on
rename failure, best-effort `remove_file(&final_path)` + retry the rename once so an existing corrupt
entry gets replaced (this makes Windows self-heal, and is harmless on POSIX). Keep it best-effort (a
failure still just degrades to "no cache", no correctness impact).

Owner: **v-cdz-tooling** (`cdz` CLI provider cache, `6b8fa35c6` — the PR#901 atomic-write fix's follow-on).
Windows-rename-replace + write-failure temp cleanup. Note the Windows half is lower-severity (Linux is the
CI platform); the temp-leak half is platform-agnostic.
