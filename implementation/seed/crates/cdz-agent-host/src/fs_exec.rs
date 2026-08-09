//! The `fs/*` executor — read/write/glob a file, the irreducible filesystem MECHANISM (GAP-3 toward the
//! self-hosting harness: an agent that BUILDS the language must read + edit source files). Behind `live-fs`.
//!
//! **THIN MECHANISM ONLY (operator standing-order: minimize host logic — the host is INEVOLVABLE).** This
//! executor does only the syscalls — read bytes, write bytes, list matching paths — and carries NO path
//! policy (no allow-list, no root confinement config). WHICH paths a session may read/write is EVOLVABLE
//! POLICY in the Cedar WASM policy on the log, NOT baked into this inevolvable host code. The kernel
//! authorizes each `fs/*` effect on its resolved `target` (the PATH) via the Cedar authorizer BEFORE dispatch
//! (SEC-F1), so a policy like `permit(principal, action == Action::"fs/write", resource) when {
//! resource.target like "implementation/**" }` is the path-scoping — swappable by shipping a new policy
//! component, never a host redeploy. This executor operates only on what Cedar already permitted.
//!
//! **Families (v-agent-harness kernel, `bb0851c57`):** `fs/read` (target=path → file bytes), `fs/write`
//! (target=path, payload=bytes → create/overwrite), `fs/glob` (target=glob/dir → newline-delimited matching
//! paths). `fs/edit` is deferred — read→modify-in-reducer→write covers the agent-edit loop.
//!
//! **Feature-gated (`live-fs`).** OFF by default so the hermetic gate never touches the real filesystem —
//! like `live-exec`/`live-net` gate the process/network executors.
//!
//! **Outcomes (supervision, §17):** a read/write/glob failure (missing path, permission, IO) is folded as a
//! classified `EffectOutcome::Err`, never a panic. Most fs errors are PERMANENT for a given path+op (a
//! missing file won't appear on a blind retry); the reducer decides what to do.

use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// A THIN filesystem executor: read/write/glob the (already-Cedar-authorized) path. Serves the `fs/*` family.
/// Holds NO state + NO policy — the path allow-list is the Cedar WASM policy's job (operator standing-order:
/// host = inevolvable mechanism; policy = evolvable wasm on the log). A unit struct: it makes no decisions.
pub struct FsExecutor;

impl FsExecutor {
    /// Construct the fs executor. No configuration — WHICH paths are reachable is decided by the Cedar policy
    /// that authorizes the `fs/*` effect's target before this executor is ever reached, not here.
    pub fn new() -> Self {
        FsExecutor
    }
}

impl Default for FsExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for FsExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // The target is the path. It is now opaque Arc<[u8]>; a path is UTF-8 here, so a non-UTF-8 target is
        // malformed → PERMANENT (fail-closed). The path was already authorized by Cedar (SEC-F1) before dispatch.
        let Ok(path) = req.target_str() else {
            return EffectOutcome::err("fs: target path is not valid UTF-8");
        };
        // Route on the fs/* family (seq-39 family-string source of truth). This executor does the raw syscall
        // for the one op Cedar permitted, nothing more.
        if req.content_type.matches_family(effect_ct::FS_READ) {
            match std::fs::read(path) {
                Ok(bytes) => EffectOutcome::Ok(Some(Payload::Inline(bytes.into()))),
                Err(e) => EffectOutcome::err(format!("fs/read {path}: {e}")),
            }
        } else if req.content_type.matches_family(effect_ct::FS_WRITE) {
            // The payload IS the file content. A blob-ref payload can't be resolved here (no blob-store
            // handle) and a payload-less write has no content — both structural PERMANENT.
            let content: &[u8] = match &req.payload {
                Some(Payload::Inline(bytes)) => bytes,
                Some(Payload::Blob(_)) => {
                    return EffectOutcome::err(
                        "fs/write: blob-ref payload unsupported — this executor has no blob-store access; inline the bytes",
                    );
                }
                None => {
                    return EffectOutcome::err(
                        "fs/write: a write requires a payload (the file bytes)",
                    );
                }
            };
            match std::fs::write(path, content) {
                // A write returns no data — an empty inline payload is the unit ack the reducer folds.
                Ok(()) => EffectOutcome::Ok(Some(Payload::Inline(Vec::new().into()))),
                Err(e) => EffectOutcome::err(format!("fs/write {path}: {e}")),
            }
        } else if req.content_type.matches_family(effect_ct::FS_GLOB) {
            glob_paths(path)
        } else {
            // A non-fs family is structural (this executor serves only fs/*) → PERMANENT.
            EffectOutcome::err(format!(
                "FsExecutor only handles the fs/* families, got {}",
                req.content_type.family
            ))
        }
    }

    /// Serves the `fs/*` family (the capability-manifest mechanism dimension when used bare as a `dyn
    /// Executor`). Uses the kernel's prefix test so all fs verbs route here.
    fn handles_family(&self, family: &str) -> bool {
        effect_ct::is_fs_family(family)
    }
}

/// `fs/glob` mechanism: list the paths matching `pattern`, newline-delimited, sorted for determinism. v0 uses
/// a minimal glob — a trailing `/*` lists a directory's immediate entries; otherwise the pattern is treated
/// as a literal path (present → itself, absent → empty). A richer glob (recursive `**`) is a later refinement
/// (or a `glob`-crate dep) if a reducer needs it; the result shape (newline-delimited bytes) stays the same.
/// Returns the paths as `\n`-joined bytes — the reducer splits them (a canonical path-list codec is a later
/// option if the newline framing proves insufficient).
#[cfg(feature = "live-fs")]
fn glob_paths(pattern: &str) -> EffectOutcome {
    let entries: Vec<String> = if let Some(dir) = pattern.strip_suffix("/*") {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .collect();
                v.sort();
                v
            }
            Err(e) => return EffectOutcome::err(format!("fs/glob {pattern}: {e}")),
        }
    } else if std::path::Path::new(pattern).exists() {
        vec![pattern.to_string()]
    } else {
        Vec::new()
    };
    EffectOutcome::Ok(Some(Payload::Inline(
        entries.join("\n").into_bytes().into(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{EffectKind, Timeliness};
    use cdz_kernel::event::Retryability;

    fn fs_req(family: &'static str, path: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            family,
            path,
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    // A unique tmp dir per test (the crate's proven-fresh helper pattern; here inline for the fs tests).
    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cdz-fsexec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn write_then_read_round_trips_the_bytes() {
        let dir = tmpdir("rw");
        let path = dir.join("hello.txt");
        let ps = path.to_string_lossy().into_owned();
        let mut exec = FsExecutor::new();
        // write
        let out = exec
            .perform(
                EffectId(0),
                &fs_req(effect_ct::FS_WRITE, &ps, Some(b"cargo test")),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(out, EffectOutcome::Ok(_)), "write ok, got {out:?}");
        // read back
        let out = exec
            .perform(
                EffectId(0),
                &fs_req(effect_ct::FS_READ, &ps, None),
                Hash::of(b"k"),
            )
            .await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(&bytes[..], b"cargo test", "read returns the written bytes");
            }
            other => panic!("expected Ok(bytes), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reading_a_missing_path_is_a_permanent_error() {
        let mut exec = FsExecutor::new();
        let out = exec
            .perform(
                EffectId(0),
                &fs_req(effect_ct::FS_READ, "/no/such/cdz-fsexec/path", None),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("fs/read")),
            "a missing path read is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_payloadless_write_is_a_permanent_error() {
        let dir = tmpdir("nopay");
        let ps = dir.join("x").to_string_lossy().into_owned();
        let mut exec = FsExecutor::new();
        let out = exec
            .perform(
                EffectId(0),
                &fs_req(effect_ct::FS_WRITE, &ps, None),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, .. } if message.contains("requires a payload")),
            "a payload-less write is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn glob_lists_directory_entries_sorted() {
        let dir = tmpdir("glob");
        std::fs::write(dir.join("b.rs"), b"").unwrap();
        std::fs::write(dir.join("a.rs"), b"").unwrap();
        let pat = format!("{}/*", dir.to_string_lossy());
        let mut exec = FsExecutor::new();
        let out = exec
            .perform(
                EffectId(0),
                &fs_req(effect_ct::FS_GLOB, &pat, None),
                Hash::of(b"k"),
            )
            .await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                let listing = String::from_utf8_lossy(&bytes);
                let lines: Vec<&str> = listing.lines().collect();
                assert_eq!(lines.len(), 2, "two entries: {listing:?}");
                assert!(
                    lines[0].ends_with("a.rs") && lines[1].ends_with("b.rs"),
                    "sorted: {listing:?}"
                );
            }
            other => panic!("expected Ok(newline-delimited paths), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_fs_family_is_a_permanent_error() {
        let mut exec = FsExecutor::new();
        // A Shell-kind request routed here (wrong family) → PERMANENT.
        let req = EffectRequest::new(EffectKind::Shell, "echo hi", None, Timeliness::Interactive);
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("only handles")),
            "a non-fs family is PERMANENT, got {out:?}"
        );
        assert!(exec.handles_family(effect_ct::FS_READ) && !exec.handles_family(effect_ct::SHELL));
    }
}
