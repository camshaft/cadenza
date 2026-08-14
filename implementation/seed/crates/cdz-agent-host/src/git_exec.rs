//! The `git/*` executor — run a typed git operation and fold its outcome (GAP-6 increment #2). A
//! [`ShellExecutor`](crate::shell::ShellExecutor) TYPED SIBLING: git is a subprocess mechanism, so this is
//! the same irreducible spawn-mechanism, specialized to the `git` program with a fixed verb set. The typed
//! `git/*` family (vs the generic `shell`) buys authz-GRANULARITY — a policy can grant `git/status` but not
//! `git/push` — and gate-ability, without the host deciding anything.
//!
//! **THIN MECHANISM ONLY (operator standing-order: minimize host logic — the host is INEVOLVABLE).** This
//! executor carries NO policy: WHICH worktree/repo/remote/ref a session may touch is EVOLVABLE Cedar policy
//! on the effect's resolved `target` (the worktree/repo), authorized BEFORE dispatch (SEC-F1) — not baked
//! here. It runs only what Cedar already permitted.
//!
//! **Injection-safe (CWE-78, like [`ShellExecutor`]).** `git` is exec'd DIRECTLY —
//! `Command::new("git").args(...)` — NEVER via `sh -c`, and the verb is one of a fixed CLOSED set (matched on
//! the kernel family consts, not a blind strip-prefix) while the reducer-supplied argument rides as a SINGLE
//! `args()` element. So shell metacharacters + extra tokens in the argument are literal, never interpreted,
//! and a `git/<unknown>` can never become an arbitrary git subcommand.
//!
//! **Feature-gated at the WIRING (`live-git`).** The module is always compiled (it adds no dependency — just
//! `std::process`), like [`shell`](crate::shell); the deployed daemon WIRES a `GitExecutor` per session only
//! under `live-git`, so the hermetic gate never spawns `git`.

use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// A THIN host git executor: run the (already-Cedar-authorized) git op + fold its outcome. Serves the 8 v0
/// `git/*` verbs (status/diff/add/commit/rev-parse/checkout/fetch/push). Unit struct — holds no state because
/// it makes no policy decision.
pub struct GitExecutor;

impl GitExecutor {
    /// Construct the git executor. No configuration — WHICH git ops run where is the Cedar policy's call,
    /// decided before this executor is reached, not here.
    pub fn new() -> Self {
        GitExecutor
    }
}

impl Default for GitExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the git argv (the args AFTER the `git` program) for a `git/<verb>` effect: `-C <worktree>` (the
/// resolved target, so git runs in the authorized repo) followed by the verb + its typed argument (the inline
/// payload, for the verbs that take one). Matches the KNOWN family consts — NOT a blind `strip_prefix` — so
/// the git subcommand stays a fixed closed set. Returns a PERMANENT-error message (`Err`) on a malformed
/// request: a non-UTF-8 / empty target, a missing-or-blob-ref payload for a verb that needs one, or a family
/// outside the 8 verbs.
fn git_argv(req: &EffectRequest) -> Result<Vec<String>, String> {
    let worktree = req
        .target_str()
        .map_err(|_| "git: target (worktree) is not valid UTF-8".to_string())?
        .to_string();
    if worktree.is_empty() {
        return Err("git: empty target (expected the worktree/repo path)".to_string());
    }
    // The verb's typed argument (add/commit/rev-parse/checkout/push): the inline payload as UTF-8, a SINGLE
    // argument. A blob-ref payload is unsupported (no blob-store handle here, like EmitExecutor); a missing
    // payload for a verb that needs one is malformed.
    let payload_arg = || -> Result<String, String> {
        match &req.payload {
            Some(Payload::Inline(b)) => std::str::from_utf8(b)
                .map(|s| s.to_string())
                .map_err(|_| "git: payload argument is not valid UTF-8".to_string()),
            Some(Payload::Blob(_)) => {
                Err("git: a blob-ref payload is unsupported — inline the argument".to_string())
            }
            None => Err("git: this git verb requires a payload argument".to_string()),
        }
    };
    let family = req.content_type.family.as_ref();
    let verb_args: Vec<String> = if family == effect_ct::GIT_STATUS {
        // Machine-readable status (deterministic output the reducer folds).
        vec!["status".into(), "--porcelain".into()]
    } else if family == effect_ct::GIT_DIFF {
        vec!["diff".into()]
    } else if family == effect_ct::GIT_ADD {
        // `--` terminates options so the pathspec can never be read as a flag.
        vec!["add".into(), "--".into(), payload_arg()?]
    } else if family == effect_ct::GIT_COMMIT {
        vec!["commit".into(), "-m".into(), payload_arg()?]
    } else if family == effect_ct::GIT_REV_PARSE {
        vec!["rev-parse".into(), payload_arg()?]
    } else if family == effect_ct::GIT_CHECKOUT {
        vec!["checkout".into(), payload_arg()?]
    } else if family == effect_ct::GIT_FETCH {
        vec!["fetch".into()]
    } else if family == effect_ct::GIT_PUSH {
        vec!["push".into(), payload_arg()?]
    } else {
        return Err(format!(
            "GitExecutor only handles the {} family verbs, got {family}",
            effect_ct::GIT_PREFIX
        ));
    };
    let mut argv = vec!["-C".to_string(), worktree];
    argv.extend(verb_args);
    Ok(argv)
}

/// stderr markers of a TRANSIENT network failure (fetch/push) → the outcome is RETRYABLE (a supervisor may
/// re-drive once connectivity returns). Everything else — a merge conflict, a bad ref, a dirty tree, any
/// other nonzero exit — is PERMANENT (a real git outcome the reducer folds; a retry re-fails identically).
/// Lowercased substring match (git error text is not a stable contract, so this is a best-effort classifier,
/// like the model executor's throttle/5xx classification — a mismatch degrades to PERMANENT, the safe default).
fn is_transient_git_network_error(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    [
        "could not resolve host",
        "connection refused",
        "connection timed out",
        "network is unreachable",
        "failed to connect",
        "temporary failure in name resolution",
        "operation timed out",
        "could not read from remote repository",
    ]
    .iter()
    .any(|m| s.contains(m))
}

#[async_trait::async_trait(?Send)]
impl Executor for GitExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // Structural family guard (like shell/fs): a non-git family reaching here is a mis-route (a
        // CompositeExecutor routes by family) → PERMANENT (§17: observable Err, never a panic).
        if !effect_ct::is_git_family(req.content_type.family.as_ref()) {
            return EffectOutcome::err(format!(
                "GitExecutor only handles the {} family, got {}",
                effect_ct::GIT_PREFIX,
                req.content_type.family
            ));
        }
        let argv = match git_argv(req) {
            Ok(a) => a,
            Err(e) => return EffectOutcome::err(e),
        };
        // Direct exec — no `sh -c` (CWE-78). The op was already Cedar-authorized on its target before dispatch.
        match std::process::Command::new("git").args(&argv).output() {
            Ok(out) if out.status.success() => {
                EffectOutcome::Ok(Some(Payload::Inline(out.stdout.into())))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let msg = format!(
                    "git exit {}: {}",
                    out.status.code().unwrap_or(-1),
                    stderr.trim()
                );
                // fetch/push transient network failures are RETRYABLE; every other nonzero exit is the
                // reducer's to fold → PERMANENT.
                let fam = req.content_type.family.as_ref();
                if (fam == effect_ct::GIT_FETCH || fam == effect_ct::GIT_PUSH)
                    && is_transient_git_network_error(&stderr)
                {
                    EffectOutcome::err_retryable(msg)
                } else {
                    EffectOutcome::err(msg)
                }
            }
            // git missing on PATH / fork error — a host/environment fault → PERMANENT (won't appear on retry).
            Err(e) => EffectOutcome::err(format!("git spawn failed: {e}")),
        }
    }

    /// Serves the `git/*` verbs (the capability-manifest mechanism dimension when used bare; a
    /// `CompositeExecutor`'s own map answers otherwise). Prefix-scoped like `fs/*`; an unknown `git/<verb>` is
    /// claimed here and refused PERMANENT by `git_argv` rather than falling through.
    fn handles_family(&self, family: &str) -> bool {
        effect_ct::is_git_family(family)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event::Retryability;

    fn req(family: &str, target: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            family.to_string(),
            target,
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[test]
    fn git_argv_maps_each_verb_to_a_direct_c_worktree_invocation() {
        // The core mapping: `-C <worktree>` + the verb + its typed arg. Tested directly (no git spawn) so it
        // is hermetic + deterministic.
        assert_eq!(
            git_argv(&req(effect_ct::GIT_STATUS, "/repo", None)).unwrap(),
            vec!["-C", "/repo", "status", "--porcelain"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_DIFF, "/repo", None)).unwrap(),
            vec!["-C", "/repo", "diff"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_ADD, "/repo", Some(b"src/x.rs"))).unwrap(),
            vec!["-C", "/repo", "add", "--", "src/x.rs"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_COMMIT, "/repo", Some(b"a message"))).unwrap(),
            vec!["-C", "/repo", "commit", "-m", "a message"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_REV_PARSE, "/repo", Some(b"HEAD"))).unwrap(),
            vec!["-C", "/repo", "rev-parse", "HEAD"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_CHECKOUT, "/repo", Some(b"main"))).unwrap(),
            vec!["-C", "/repo", "checkout", "main"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_FETCH, "/repo", None)).unwrap(),
            vec!["-C", "/repo", "fetch"]
        );
        assert_eq!(
            git_argv(&req(effect_ct::GIT_PUSH, "/repo", Some(b"origin"))).unwrap(),
            vec!["-C", "/repo", "push", "origin"]
        );
    }

    #[test]
    fn git_argv_rejects_malformed_requests() {
        // Empty target -> Err; a payload-verb with no payload -> Err; a blob-ref payload -> Err; an unknown
        // git verb -> Err. (These become PERMANENT EffectOutcome::err in `perform`, before any spawn.)
        assert!(git_argv(&req(effect_ct::GIT_STATUS, "", None)).is_err());
        assert!(git_argv(&req(effect_ct::GIT_COMMIT, "/repo", None)).is_err());
        let blob = EffectRequest::new_with_family(
            effect_ct::GIT_ADD.to_string(),
            "/repo",
            Some(Payload::Blob(Hash::of(b"x"))),
            Timeliness::Interactive,
        );
        assert!(git_argv(&blob).is_err());
        assert!(git_argv(&req("git/rebase", "/repo", Some(b"x"))).is_err());
    }

    #[tokio::test]
    async fn a_non_git_family_is_a_permanent_error() {
        // Structural self-guard (mirrors shell/fs): a non-git family reaching this executor is a mis-route ->
        // PERMANENT, never a panic, and never spawns git.
        let mut exec = GitExecutor::new();
        match exec
            .perform(
                EffectId(0),
                &req(effect_ct::HTTP, "/repo", None),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err {
                message,
                retryability: Retryability::Permanent,
            } => assert!(message.contains("only handles"), "{message}"),
            other => panic!("a non-git family must be a permanent Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_payloadless_commit_is_a_permanent_error_before_spawn() {
        // A git/commit with no message payload is malformed -> PERMANENT, decided by git_argv BEFORE any
        // spawn (so this is hermetic — it never invokes git).
        let mut exec = GitExecutor::new();
        match exec
            .perform(
                EffectId(0),
                &req(effect_ct::GIT_COMMIT, "/repo", None),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err {
                message,
                retryability: Retryability::Permanent,
            } => assert!(message.contains("requires a payload"), "{message}"),
            other => panic!("a payloadless commit must be a permanent Err, got {other:?}"),
        }
    }

    #[test]
    fn transient_network_stderr_is_classified_retryable_others_permanent() {
        assert!(is_transient_git_network_error(
            "fatal: unable to access: Could not resolve host: github.com"
        ));
        assert!(is_transient_git_network_error(
            "ssh: connect to host ... Connection timed out"
        ));
        assert!(!is_transient_git_network_error(
            "error: Your local changes would be overwritten by merge"
        ));
        assert!(!is_transient_git_network_error(
            "fatal: bad revision 'nope'"
        ));
    }

    #[test]
    fn handles_family_covers_the_git_verbs_and_nothing_else() {
        let exec = GitExecutor::new();
        assert!(exec.handles_family(effect_ct::GIT_STATUS));
        assert!(exec.handles_family(effect_ct::GIT_PUSH));
        assert!(!exec.handles_family(effect_ct::SHELL));
        assert!(!exec.handles_family(effect_ct::HTTP));
    }
}
