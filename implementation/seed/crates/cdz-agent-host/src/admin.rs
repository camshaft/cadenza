//! The daemon's ADMIN CONTROL INTERFACE — the command layer (operator directive: "we need to have a way
//! to communicate with the daemon and send it commands as an admin").
//!
//! The daemon boots as a pure CONTROL PLANE: it comes up on its config with an EMPTY session registry, and
//! an admin then INSTALLS sessions into it + manages them at runtime — install a session, list what's
//! running, ask a session's status, stop one. This module is the transport-agnostic HEART of that: a
//! data-only [`AdminCommand`] an admin issues, an [`AdminResponse`] the daemon returns, and
//! [`AgentHost::apply_admin`] which applies one command against the live registry. It is DELIBERATELY
//! transport-free (no socket, no serialization) so the command semantics are hermetically testable; the
//! Unix-domain-socket listener that carries these frames is the next slice, and the Cedar `admin/*`
//! authorization of each command (deny-by-default, uniform with effect authz) is the slice after.
//!
//! **The install seam ([`SessionFactory`]).** Installing a session means building a [`HostedSession`] —
//! which needs a *reducer*, and building a reducer FROM a [`reducer_hash`](InstallSpec::reducer_hash)
//! means loading its wasm component (v-agent-harness's genesis-from-hash path, not yet wired here). So the
//! command layer does not build sessions itself: it delegates to a caller-supplied [`SessionFactory`]. The
//! real daemon supplies a factory that loads the reducer component + assembles the live executor set + the
//! policy-derived authorizer; a test supplies a stub factory returning a canned session. This is what lets
//! the command+registry layer land independent of the wasm-load path — the factory is the single seam to
//! coordinate with v-agent-harness on.

use crate::host::{AgentHost, HostedSession, SessionId};
use crate::status::{session_status_json, DEFAULT_STALL_AFTER_MS};
use cdz_kernel::hash::Hash;

/// What an admin asks the daemon to install: the new session's identity + which reducer it runs + an
/// optional initial goal. Data-only (it's the wire shape a control frame will carry) — it does NOT carry a
/// reducer or an authorizer, because those are built host-side: the [`SessionFactory`] loads the reducer
/// component by [`reducer_hash`](Self::reducer_hash) and derives the authorizer from the daemon's policy
/// (so what a session MAY do is a policy decision, not something the install command dictates — the same
/// deny-by-default posture the `admin/*` authorization slice builds on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSpec {
    /// The id to register the new session under (must be free — install does not silently replace).
    pub id: SessionId,
    /// The content hash of the reducer component to run — the factory loads its wasm by this hash.
    pub reducer_hash: Hash,
    /// An optional initial goal/prompt handed to the new session (the factory decides how to seed it).
    pub goal: Option<String>,
}

/// One admin command against the running daemon. Data-only + transport-agnostic: a control frame
/// deserializes into one of these, [`AgentHost::apply_admin`] applies it, and an [`AdminResponse`] is
/// serialized back. v0 command set (install + the three management ops the operator named).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminCommand {
    /// Install a new session into the registry from an [`InstallSpec`] (via the [`SessionFactory`]).
    InstallSession(InstallSpec),
    /// List the ids of every running session.
    ListSessions,
    /// Fetch one session's status snapshot (the [`session_status_json`] shape).
    SessionStatus { id: SessionId },
    /// Stop (remove) a running session, dropping it from the registry.
    StopSession { id: SessionId },
}

/// The daemon's reply to an [`AdminCommand`]. Data-only (a control frame serializes it back to the admin).
/// Every failure mode (unknown session, an already-taken id, a factory build error) is an
/// [`Error`](AdminResponse::Error) rather than a panic — an admin command must never crash the daemon.
///
/// Ids are carried as [`SessionId`] (not `String`), symmetric with [`AdminCommand`]/[`InstallSpec`]: an
/// in-process caller keeps type-safety + the `Arc<str>` cheap clone, and the String↔SessionId conversion
/// happens only at the transport boundary (`admin_wire`), not here (#1949 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminResponse {
    /// A session was installed under this id.
    Installed { id: SessionId },
    /// The ids of all running sessions (sorted, deterministic).
    Sessions { ids: Vec<SessionId> },
    /// A session's status as the [`session_status_json`] JSON string.
    Status { json: String },
    /// A session was stopped (removed) under this id.
    Stopped { id: SessionId },
    /// The command could not be applied — the message says why (unknown id, id already in use, a factory
    /// build failure). NOT a panic: a bad admin command is reported, not fatal.
    Error { message: String },
}

/// Builds a [`HostedSession`] from an [`InstallSpec`] — the seam between the admin command layer and the
/// reducer-load path. The real daemon's factory loads the reducer component by hash, assembles the live
/// executor set, and derives the authorizer from the configured policy; a test's factory returns a canned
/// session. Async (`?Send`, matching the crate's single-threaded host convention) because the real load is
/// I/O — reading a wasm component / resolving a hash through a store.
#[async_trait::async_trait(?Send)]
pub trait SessionFactory {
    /// Build the session an install spec describes, or an error string if it can't (unknown reducer hash,
    /// a policy that forbids the session, a malformed component). The error becomes an
    /// [`AdminResponse::Error`]; the registry is left untouched on failure.
    async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String>;
}

impl AgentHost {
    /// Apply one [`AdminCommand`] against this live registry, returning the [`AdminResponse`]. `factory`
    /// builds the session for an install; it is `Option` because ONLY [`InstallSession`](AdminCommand::InstallSession)
    /// needs it — a pure read/remove (list/status/stop) passes `None` and never threads an unused factory
    /// (#1949 review). An `InstallSession` with `None` is an [`Error`](AdminResponse::Error) ("no session
    /// factory available"), not a panic. `now_ms` is the admin's wall clock for the status-stall derivation
    /// (`None` skips the time-based stall check).
    ///
    /// Semantics:
    /// - **Install** — build via `factory`; register under the spec's id. REFUSES an id already in use
    ///   (returns [`Error`](AdminResponse::Error), registry untouched) — install is explicit, so a restart
    ///   is `StopSession` then `InstallSession`, never a silent replace. A factory build error (or a missing
    ///   factory) is likewise surfaced with the registry untouched.
    /// - **List** — the sorted session ids.
    /// - **Status** — the target's status JSON, or an error for an unknown id.
    /// - **Stop** — remove the target, or an error for an unknown id.
    ///
    /// Every not-found / conflict / build failure is an [`AdminResponse::Error`], never a panic — the
    /// daemon serves admin commands without ever crashing on a bad one.
    pub async fn apply_admin(
        &mut self,
        cmd: AdminCommand,
        factory: Option<&mut dyn SessionFactory>,
        now_ms: Option<u64>,
    ) -> AdminResponse {
        match cmd {
            AdminCommand::InstallSession(spec) => {
                // Install is the ONLY command that needs a factory — a caller doing a pure read/remove
                // passes None. No factory for an install is a clean error, not a panic.
                let Some(factory) = factory else {
                    return AdminResponse::Error {
                        message: format!(
                            "no session factory available to install {}",
                            spec.id.as_str()
                        ),
                    };
                };
                // Explicit install: refuse a taken id (restart = stop then install, not a silent replace —
                // spawn() itself would REPLACE, so we guard here before building/registering).
                if self.contains(&spec.id) {
                    return AdminResponse::Error {
                        message: format!("session already installed: {}", spec.id.as_str()),
                    };
                }
                match factory.build(&spec).await {
                    Ok(session) => {
                        let id = spec.id.clone();
                        self.spawn(id.clone(), session);
                        AdminResponse::Installed { id }
                    }
                    // Build failed → registry untouched, error surfaced.
                    Err(e) => AdminResponse::Error {
                        message: format!("install failed for {}: {e}", spec.id.as_str()),
                    },
                }
            }
            AdminCommand::ListSessions => AdminResponse::Sessions {
                ids: self.session_ids(),
            },
            AdminCommand::SessionStatus { id } => match self.get(&id) {
                Some(hosted) => AdminResponse::Status {
                    json: session_status_json(&id, hosted, now_ms, DEFAULT_STALL_AFTER_MS),
                },
                None => AdminResponse::Error {
                    message: format!("unknown session: {}", id.as_str()),
                },
            },
            AdminCommand::StopSession { id } => match self.remove(&id) {
                Some(_) => AdminResponse::Stopped { id },
                None => AdminResponse::Error {
                    message: format!("unknown session: {}", id.as_str()),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::event::Event;
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A trivial reducer for a canned installed session — never actually driven here (the admin tests
    /// exercise registry mutation, not folds).
    struct StubAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for StubAgent {
        async fn fold(&self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
            FoldOutput::none()
        }
    }

    /// A factory that builds a canned stub session for any spec — the test stand-in for the real
    /// wasm-loading factory. Records how many builds it was asked for so a test can assert the guard fired
    /// BEFORE the build on a collision.
    struct StubFactory {
        builds: usize,
    }
    #[async_trait::async_trait(?Send)]
    impl SessionFactory for StubFactory {
        async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
            self.builds += 1;
            Ok(HostedSession::genesis(
                spec.reducer_hash,
                Box::new(StubAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ))
        }
    }

    /// A factory that always fails — proves an install build error is surfaced with the registry untouched.
    struct FailingFactory;
    #[async_trait::async_trait(?Send)]
    impl SessionFactory for FailingFactory {
        async fn build(&mut self, _spec: &InstallSpec) -> Result<HostedSession, String> {
            Err("no such reducer component".into())
        }
    }

    fn spec(id: &str) -> InstallSpec {
        InstallSpec {
            id: SessionId::new(id),
            reducer_hash: Hash::of(id.as_bytes()),
            goal: Some("do the thing".into()),
        }
    }

    #[tokio::test]
    async fn install_registers_a_new_session_and_reports_installed() {
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        let resp = host
            .apply_admin(
                AdminCommand::InstallSession(spec("worker")),
                Some(&mut factory),
                None,
            )
            .await;
        assert_eq!(
            resp,
            AdminResponse::Installed {
                id: SessionId::new("worker")
            }
        );
        assert!(
            host.contains(&SessionId::new("worker")),
            "session registered"
        );
        assert_eq!(host.len(), 1);
        assert_eq!(factory.builds, 1, "the factory built exactly one session");
    }

    #[tokio::test]
    async fn an_install_with_no_factory_is_an_error_not_a_panic() {
        // Install is the only command needing a factory; apply_admin with None for an InstallSession is a
        // clean error (#1949 review — the other commands pass None and never touch a factory).
        let mut host = AgentHost::new();
        let resp = host
            .apply_admin(AdminCommand::InstallSession(spec("orphan")), None, None)
            .await;
        assert!(
            matches!(resp, AdminResponse::Error { message } if message.contains("no session factory")),
        );
        assert!(host.is_empty(), "nothing installed without a factory");
    }

    #[tokio::test]
    async fn install_refuses_an_id_already_in_use_without_building() {
        // Explicit install: a taken id is refused (restart = stop then install, not a silent replace). The
        // guard fires BEFORE the factory build — so the factory is NOT asked to build on a collision.
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        host.apply_admin(
            AdminCommand::InstallSession(spec("dup")),
            Some(&mut factory),
            None,
        )
        .await;
        assert_eq!(factory.builds, 1);

        let resp = host
            .apply_admin(
                AdminCommand::InstallSession(spec("dup")),
                Some(&mut factory),
                None,
            )
            .await;
        assert!(
            matches!(resp, AdminResponse::Error { message } if message.contains("already installed")),
        );
        assert_eq!(host.len(), 1, "registry unchanged on a collision");
        assert_eq!(
            factory.builds, 1,
            "the collision guard fired before a second build"
        );
    }

    #[tokio::test]
    async fn install_surfaces_a_factory_build_error_with_the_registry_untouched() {
        let mut host = AgentHost::new();
        let mut factory = FailingFactory;
        let resp = host
            .apply_admin(
                AdminCommand::InstallSession(spec("bad")),
                Some(&mut factory),
                None,
            )
            .await;
        assert!(
            matches!(resp, AdminResponse::Error { message } if message.contains("install failed") && message.contains("no such reducer component")),
        );
        assert!(host.is_empty(), "a failed build installs nothing");
    }

    #[tokio::test]
    async fn list_returns_the_sorted_session_ids() {
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        // Install out of order; list must come back sorted (deterministic).
        host.apply_admin(
            AdminCommand::InstallSession(spec("b")),
            Some(&mut factory),
            None,
        )
        .await;
        host.apply_admin(
            AdminCommand::InstallSession(spec("a")),
            Some(&mut factory),
            None,
        )
        .await;
        // A pure read passes None for the factory (never threads an unused one).
        let resp = host
            .apply_admin(AdminCommand::ListSessions, None, None)
            .await;
        assert_eq!(
            resp,
            AdminResponse::Sessions {
                ids: vec![SessionId::new("a"), SessionId::new("b")]
            }
        );
    }

    #[tokio::test]
    async fn status_returns_the_session_json_or_an_error_for_unknown() {
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        host.apply_admin(
            AdminCommand::InstallSession(spec("s1")),
            Some(&mut factory),
            None,
        )
        .await;

        // A pure read passes None for the factory.
        let ok = host
            .apply_admin(
                AdminCommand::SessionStatus {
                    id: SessionId::new("s1"),
                },
                None,
                Some(0),
            )
            .await;
        match ok {
            AdminResponse::Status { json } => {
                assert!(json.contains("\"session_id\":\"s1\""), "{json}");
                assert!(json.contains("\"errored\":false"), "{json}");
            }
            other => panic!("expected Status, got {other:?}"),
        }

        let missing = host
            .apply_admin(
                AdminCommand::SessionStatus {
                    id: SessionId::new("ghost"),
                },
                None,
                Some(0),
            )
            .await;
        assert!(
            matches!(missing, AdminResponse::Error { message } if message.contains("unknown session") && message.contains("ghost")),
        );
    }

    #[tokio::test]
    async fn stop_removes_a_session_or_errors_for_unknown() {
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        host.apply_admin(
            AdminCommand::InstallSession(spec("victim")),
            Some(&mut factory),
            None,
        )
        .await;
        assert_eq!(host.len(), 1);

        // A stop passes None for the factory.
        let stopped = host
            .apply_admin(
                AdminCommand::StopSession {
                    id: SessionId::new("victim"),
                },
                None,
                None,
            )
            .await;
        assert_eq!(
            stopped,
            AdminResponse::Stopped {
                id: SessionId::new("victim")
            }
        );
        assert!(host.is_empty(), "the session was removed");

        // Stopping an absent session is an error, not a panic.
        let again = host
            .apply_admin(
                AdminCommand::StopSession {
                    id: SessionId::new("victim"),
                },
                None,
                None,
            )
            .await;
        assert!(
            matches!(again, AdminResponse::Error { message } if message.contains("unknown session")),
        );
    }

    #[tokio::test]
    async fn stop_then_install_is_the_restart_path() {
        // The documented restart semantics: install refuses a live id, but stopping first frees it, so
        // stop-then-install re-installs cleanly (a fresh session under the same id).
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        host.apply_admin(
            AdminCommand::InstallSession(spec("r")),
            Some(&mut factory),
            None,
        )
        .await;
        host.apply_admin(
            AdminCommand::StopSession {
                id: SessionId::new("r"),
            },
            None,
            None,
        )
        .await;
        let reinstalled = host
            .apply_admin(
                AdminCommand::InstallSession(spec("r")),
                Some(&mut factory),
                None,
            )
            .await;
        assert_eq!(
            reinstalled,
            AdminResponse::Installed {
                id: SessionId::new("r")
            }
        );
        assert_eq!(host.len(), 1);
        assert_eq!(factory.builds, 2, "two real installs across the restart");
    }
}
