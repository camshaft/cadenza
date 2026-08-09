//! The daemon's ADMIN CONTROL INTERFACE — the command layer (operator directive: "we need to have a way
//! to communicate with the daemon and send it commands as an admin").
//!
//! The daemon boots as a pure CONTROL PLANE: it comes up on its config with an EMPTY session registry, and
//! an admin then INSTALLS sessions into it + manages them at runtime — install a session, list what's
//! running, ask a session's status, stop one. This module is the transport-agnostic HEART of that: a
//! data-only [`AdminCommand`] an admin issues, an [`AdminResponse`] the daemon returns, and
//! [`AgentHost::apply_admin`] which applies one command against the live registry. It is DELIBERATELY
//! transport-free (no socket, no serialization) so the command semantics are hermetically testable; the
//! Unix-domain-socket listener that carries these frames lives in [`admin_socket`](crate::admin_socket)
//! (behind the `admin` feature), and the Cedar `admin/*` authorization of each command (deny-by-default,
//! uniform with effect authz) is the [`AdminAuthorizer`] seam every command is gated through — both landed.
//!
//! **The install seam ([`SessionFactory`]).** Installing a session means building a [`HostedSession`] —
//! which needs a *reducer*, and building a reducer FROM a [`reducer_hash`](InstallSpec::reducer_hash)
//! means loading its wasm component (blob-get → lift → assemble — wired in the real
//! [`ComponentSessionFactory`](crate::ComponentSessionFactory), which also completes the genesis
//! authorizer-from-hash ceremony). So the
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

impl AdminCommand {
    /// The `admin/*` ACTION name this command authorizes as — the stable string an [`AdminAuthorizer`]
    /// (and, later, a Cedar policy) decides on. Deliberately id-free (the action is the VERB, not the
    /// target): a policy grants "may install sessions", not "may install session X". Kept in one place so
    /// the wire, the authorizer, and the audit log all name an admin action identically.
    pub fn action(&self) -> &'static str {
        match self {
            AdminCommand::InstallSession(_) => "admin/install-session",
            AdminCommand::ListSessions => "admin/list-sessions",
            AdminCommand::SessionStatus { .. } => "admin/session-status",
            AdminCommand::StopSession { .. } => "admin/stop-session",
        }
    }
}

/// Authorizes admin commands — the deny-by-default gate on the control interface (operator/concierge
/// directive: "Cedar admin/* authz", uniform with effect authz). Every command an admin submits is checked
/// here before [`apply_admin`](AgentHost::apply_admin) touches the registry; a denied command is an
/// [`AdminResponse::Error`], never applied.
///
/// The v0 shape mirrors the kernel's [`Authorize`](cdz_kernel::authz::Authorize) seam: the daemon holds a
/// `Box<dyn AdminAuthorizer>` and the real deployment swaps in a Cedar-policy-component impl (reusing the
/// [`ComponentAuthorizer`](cdz_kernel::wasm_host::ComponentAuthorizer) path — the following slice), while
/// tests and the always-on build use the concrete [`AllowList`]. Decides on the command's
/// [`action`](AdminCommand::action) string; `principal` is the admin identity the transport asserts (over a
/// local `0o600` socket that identity is implicitly the daemon's owner — the socket perms are the real
/// gate; this layer decides WHICH actions that principal may take).
///
/// **Async + `?Send`** (like the kernel's `Authorize`): the Cedar-component impl's evaluation `.await`s
/// (fuel-yielding) and holds a non-`Send` wasmtime store, so the trait is async NOW — while there's only
/// the synchronous [`AllowList`] impl — to avoid a breaking signature change when that impl lands (#1967
/// review).
#[async_trait::async_trait(?Send)]
pub trait AdminAuthorizer {
    /// May `principal` perform `command`? `Ok(())` permits; `Err(reason)` denies (the reason is surfaced in
    /// the error response + is audit-loggable). Total, must not panic — a broken decision denies. May
    /// `.await` a wasm policy evaluation internally (a synchronous impl like [`AllowList`] just returns).
    async fn authorize(&self, principal: &str, command: &AdminCommand) -> Result<(), String>;
}

/// A concrete deny-by-default [`AdminAuthorizer`]: an explicit allowlist of `(principal, action)` pairs.
/// The always-on / test authorizer until the Cedar-policy-component impl lands — and a perfectly serviceable
/// production gate for a small fixed admin set. Anything not explicitly allowed is denied (the fail-closed
/// default): an unknown principal, or a known principal attempting an un-granted action.
#[derive(Debug, Clone, Default)]
pub struct AllowList {
    /// `(principal, action)` pairs that are permitted. A `("*", action)` entry allows that action for ANY
    /// principal (the "single trusted local admin" shape — pair it with the socket's `0o600` owner-gate).
    allowed: std::collections::HashSet<(String, String)>,
}

impl AllowList {
    /// An empty allowlist — denies everything (the strictest fail-closed default). Grant with
    /// [`allow`](Self::allow) / [`allow_any_principal`](Self::allow_any_principal).
    pub fn deny_all() -> Self {
        AllowList::default()
    }

    /// Allow `principal` to perform `action` (an `admin/*` string, e.g. [`AdminCommand::action`]). Builder-
    /// style (chains).
    pub fn allow(mut self, principal: impl Into<String>, action: impl Into<String>) -> Self {
        self.allowed.insert((principal.into(), action.into()));
        self
    }

    /// Allow ANY principal to perform `action` — the "trusted local admin over the 0o600 socket" grant,
    /// where the socket's owner-only perms are the real identity gate and this layer just scopes actions.
    pub fn allow_any_principal(self, action: impl Into<String>) -> Self {
        self.allow("*", action)
    }

    /// Allow ANY principal to perform EVERY v0 admin action — the "full trusted local admin" preset (the
    /// daemon's default when the socket is the owner-gated control plane and no finer policy is configured).
    pub fn allow_all_for_local_admin() -> Self {
        let mut list = AllowList::deny_all();
        for action in [
            "admin/install-session",
            "admin/list-sessions",
            "admin/session-status",
            "admin/stop-session",
        ] {
            list = list.allow_any_principal(action);
        }
        list
    }
}

#[async_trait::async_trait(?Send)]
impl AdminAuthorizer for AllowList {
    async fn authorize(&self, principal: &str, command: &AdminCommand) -> Result<(), String> {
        let action = command.action();
        // A specific (principal, action) grant, OR a wildcard-principal ("*") grant for this action.
        // Compare the stored pairs' fields as &str (no per-check `.to_string()` allocation — #1967 review);
        // the allowed set is tiny (a handful of grants), so the linear scan is trivial.
        let permitted = self
            .allowed
            .iter()
            .any(|(p, a)| a == action && (p == principal || p == "*"));
        if permitted {
            Ok(())
        } else {
            Err(format!(
                "admin command denied: principal {principal:?} may not {action}"
            ))
        }
    }
}

/// The daemon's reply to an [`AdminCommand`]. Data-only (a control frame serializes it back to the admin).
/// Every failure mode (unknown session, an already-taken id, a factory build error) is an
/// [`Error`](AdminResponse::Error) rather than a panic — an admin command must never crash the daemon.
///
/// Ids are carried as [`SessionId`] (not `String`), symmetric with [`AdminCommand`]/[`InstallSpec`]: an
/// in-process caller keeps type-safety + the cheap `Copy` genesis `Hash`, and the hex↔SessionId conversion
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

    /// Build a SPAWNED CHILD session (§lifecycle I3): materialize the reducer for `reducer_hash` + the
    /// executor/authorizer set, then wrap them with PARENT-PROVENANCE via
    /// [`HostedSession::genesis_spawned_with_nonce`](crate::HostedSession::genesis_spawned_with_nonce) using
    /// the CALLER-supplied `spawn_nonce` (so the child's genesis-hash id matches the `lifecycle/spawn`
    /// executor's pre-computation — see [`LifecycleOp::Spawn`](crate::LifecycleOp)). Same load path as
    /// [`build`](Self::build) but `genesis_spawned_with_nonce` instead of the root `genesis`, and a
    /// deny-all child authorizer by default (the kernel's deny-by-default posture — a spawned child earns
    /// capabilities via an explicit later grant, not by inheriting the parent's).
    ///
    /// DEFAULT: not supported — a factory that only serves admin `install-session` (or a test stub) returns
    /// an error, so a `lifecycle/spawn` against it declines cleanly rather than the trait forcing every impl
    /// to grow a spawn path. The real [`ComponentSessionFactory`](crate::ComponentSessionFactory) overrides.
    async fn build_spawned(
        &mut self,
        reducer_hash: Hash,
        _parent_genesis: Hash,
        _spawn_nonce: Hash,
    ) -> Result<HostedSession, String> {
        Err(format!(
            "this SessionFactory does not support lifecycle/spawn (reducer {reducer_hash})"
        ))
    }

    /// Rebuild a [`HostedSession`] from a session's durably-read log for §lifecycle I4b boot-recovery — the
    /// recovery counterpart to [`build`](Self::build). Where `build` mints a fresh session from an
    /// [`InstallSpec`], this takes the [`Recovered`](cdz_kernel::log_store::Recovered) the daemon read back
    /// (via [`LogSinkBuilder::recover`](crate::factory::LogSinkBuilder::recover)), reads the reducer hash from
    /// the recovered GENESIS event (`events[0]`), reloads that reducer from the blob store, folds the log
    /// through [`Session::recover_from`](cdz_kernel::kernel::Session::recover_from) to reconstruct the session,
    /// and [`HostedSession::from_recovered`](crate::HostedSession::from_recovered) wraps it with a fresh
    /// executor set. Returns the session PLUS the [`RecoveryReport`](cdz_kernel::kernel::RecoveryReport) so the
    /// daemon can re-drive the session's `open_effects` and react to `report.is_corrupt()`.
    ///
    /// The reducer load is why this lives on the factory (only it holds the blob store), and why the daemon
    /// hands it the raw `Recovered` rather than a pre-built `Session`: `recover_from` needs the reducer, which
    /// the daemon can't materialize itself. A DENY-ALL authorizer is attached (like a spawned child): a
    /// recovered session re-earns caps via its replayed authorizer-install or a fresh grant, not by trusting a
    /// rebuilt-from-nothing policy. Reachable through the boxed [`SessionFactory`] the daemon holds.
    ///
    /// DEFAULT: not supported — a factory that only serves admin installs (or a test stub) returns an error,
    /// so boot-recovery against it declines cleanly rather than the trait forcing every impl to grow a
    /// recovery path. The real [`ComponentSessionFactory`](crate::ComponentSessionFactory) overrides.
    ///
    /// Errors (never a panic): the recovered log is empty / has no genesis at `events[0]`, the reducer bytes
    /// are absent from the blob store, the component doesn't lift, or the replay fails.
    async fn recover_and_build(
        &mut self,
        _recovered: cdz_kernel::log_store::Recovered,
    ) -> Result<(HostedSession, cdz_kernel::kernel::RecoveryReport), String> {
        Err("this SessionFactory does not support boot-recovery (recover_and_build)".to_string())
    }

    /// Fetch a content-addressed blob by hash from the factory's blob store — the seam the host LOOP uses to
    /// resolve a `control/signature` effect's TARGET component bytes (signature-query part-1): the loop
    /// surfaces the effect, resolves its target hash to bytes HERE, and hands them to
    /// [`HostedSession::settle_signature_query`](crate::HostedSession::settle_signature_query) to reflect +
    /// fold back. The factory owns the blob store (a [`HostedSession`] is blob-store-free), so the fetch lives
    /// here. `Ok(Some(bytes))` = present, `Ok(None)` = absent (a clean miss the loop settles as an Err arm),
    /// `Err` = a backend I/O failure.
    ///
    /// DEFAULT: `Ok(None)` — a factory with no blob store (a test stub) resolves every hash as absent, so the
    /// loop settles the signature query's Err arm cleanly. The real
    /// [`ComponentSessionFactory`](crate::ComponentSessionFactory) overrides with a real `blob.get`.
    async fn fetch_blob(&self, _hash: &Hash) -> Result<Option<bytes::Bytes>, String> {
        Ok(None)
    }
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
        factory: Option<&mut (dyn SessionFactory + '_)>,
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
                            spec.id.to_hex()
                        ),
                    };
                };
                // Explicit install: refuse a taken id (restart = stop then install, not a silent replace —
                // spawn() itself would REPLACE, so we guard here before building/registering).
                if self.contains(&spec.id) {
                    return AdminResponse::Error {
                        message: format!("session already installed: {}", spec.id.to_hex()),
                    };
                }
                match factory.build(&spec).await {
                    Ok(session) => {
                        let id = spec.id;
                        self.spawn(id, session);
                        AdminResponse::Installed { id }
                    }
                    // Build failed → registry untouched, error surfaced.
                    Err(e) => AdminResponse::Error {
                        message: format!("install failed for {}: {e}", spec.id.to_hex()),
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
                    message: format!("unknown session: {}", id.to_hex()),
                },
            },
            AdminCommand::StopSession { id } => match self.remove(&id) {
                Some(_) => AdminResponse::Stopped { id },
                None => AdminResponse::Error {
                    message: format!("unknown session: {}", id.to_hex()),
                },
            },
        }
    }

    /// AUTHORIZE then apply — the deny-by-default gate the daemon's control interface actually calls. Runs
    /// `authz.authorize(principal, &cmd)` FIRST; only on `Ok` does it delegate to
    /// [`apply_admin`](Self::apply_admin). A denied command returns an [`AdminResponse::Error`] carrying the
    /// deny reason and NEVER touches the registry — the fail-closed control-plane guard (uniform with the
    /// kernel's effect authz). This is the entry point the socket transport / loop uses; the bare
    /// `apply_admin` remains available for an already-authorized/trusted caller (e.g. an in-process test).
    pub async fn apply_admin_authorized(
        &mut self,
        cmd: AdminCommand,
        principal: &str,
        authz: &(dyn AdminAuthorizer + '_),
        factory: Option<&mut (dyn SessionFactory + '_)>,
        now_ms: Option<u64>,
    ) -> AdminResponse {
        if let Err(reason) = authz.authorize(principal, &cmd).await {
            return AdminResponse::Error { message: reason };
        }
        self.apply_admin(cmd, factory, now_ms).await
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
        async fn fold(&mut self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
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
            id: SessionId::new(Hash::of(id.as_bytes())),
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
                id: SessionId::new(Hash::of(b"worker"))
            }
        );
        assert!(
            host.contains(&SessionId::new(Hash::of(b"worker"))),
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
        // Listed sorted by SessionId (= genesis-hash byte order); build the expected the same way so the
        // assertion pins ordering without hardcoding hash bytes.
        let mut ids = vec![
            SessionId::new(Hash::of(b"a")),
            SessionId::new(Hash::of(b"b")),
        ];
        ids.sort();
        assert_eq!(resp, AdminResponse::Sessions { ids });
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
                    id: SessionId::new(Hash::of(b"s1")),
                },
                None,
                Some(0),
            )
            .await;
        match ok {
            AdminResponse::Status { json } => {
                assert!(
                    json.contains(&format!(
                        "\"session_id\":\"{}\"",
                        SessionId::new(Hash::of(b"s1")).to_hex()
                    )),
                    "{json}"
                );
                assert!(json.contains("\"errored\":false"), "{json}");
            }
            other => panic!("expected Status, got {other:?}"),
        }

        let missing = host
            .apply_admin(
                AdminCommand::SessionStatus {
                    id: SessionId::new(Hash::of(b"ghost")),
                },
                None,
                Some(0),
            )
            .await;
        assert!(
            matches!(missing, AdminResponse::Error { message } if message.contains("unknown session") && message.contains(&SessionId::new(Hash::of(b"ghost")).to_hex())),
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
                    id: SessionId::new(Hash::of(b"victim")),
                },
                None,
                None,
            )
            .await;
        assert_eq!(
            stopped,
            AdminResponse::Stopped {
                id: SessionId::new(Hash::of(b"victim"))
            }
        );
        assert!(host.is_empty(), "the session was removed");

        // Stopping an absent session is an error, not a panic.
        let again = host
            .apply_admin(
                AdminCommand::StopSession {
                    id: SessionId::new(Hash::of(b"victim")),
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
                id: SessionId::new(Hash::of(b"r")),
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
                id: SessionId::new(Hash::of(b"r"))
            }
        );
        assert_eq!(host.len(), 1);
        assert_eq!(factory.builds, 2, "two real installs across the restart");
    }

    // ── admin authorization (deny-by-default) ───────────────────────────────────────────────────────

    #[test]
    fn command_action_strings_are_stable() {
        assert_eq!(
            AdminCommand::InstallSession(spec("x")).action(),
            "admin/install-session"
        );
        assert_eq!(AdminCommand::ListSessions.action(), "admin/list-sessions");
        assert_eq!(
            AdminCommand::SessionStatus {
                id: SessionId::new(Hash::of(b"x"))
            }
            .action(),
            "admin/session-status"
        );
        assert_eq!(
            AdminCommand::StopSession {
                id: SessionId::new(Hash::of(b"x"))
            }
            .action(),
            "admin/stop-session"
        );
    }

    #[tokio::test]
    async fn allowlist_denies_by_default_and_permits_only_granted_actions() {
        let list = AllowList::deny_all()
            .allow("admin", "admin/list-sessions")
            .allow("admin", "admin/session-status");

        // Granted (specific principal + action).
        assert!(list
            .authorize("admin", &AdminCommand::ListSessions)
            .await
            .is_ok());
        // Un-granted action for a known principal → denied.
        let err = list
            .authorize("admin", &AdminCommand::InstallSession(spec("x")))
            .await
            .unwrap_err();
        assert!(err.contains("may not admin/install-session"), "{err}");
        // Unknown principal → denied even for a granted action.
        assert!(list
            .authorize("intruder", &AdminCommand::ListSessions)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn allowlist_wildcard_principal_grants_any_caller() {
        let list = AllowList::deny_all().allow_any_principal("admin/list-sessions");
        assert!(list
            .authorize("anyone", &AdminCommand::ListSessions)
            .await
            .is_ok());
        assert!(list
            .authorize("other", &AdminCommand::ListSessions)
            .await
            .is_ok());
        // Still deny-by-default for un-granted actions.
        assert!(list
            .authorize(
                "anyone",
                &AdminCommand::StopSession {
                    id: SessionId::new(Hash::of(b"x"))
                }
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn allow_all_for_local_admin_permits_every_v0_action() {
        let list = AllowList::allow_all_for_local_admin();
        for cmd in [
            AdminCommand::InstallSession(spec("x")),
            AdminCommand::ListSessions,
            AdminCommand::SessionStatus {
                id: SessionId::new(Hash::of(b"x")),
            },
            AdminCommand::StopSession {
                id: SessionId::new(Hash::of(b"x")),
            },
        ] {
            assert!(
                list.authorize("owner", &cmd).await.is_ok(),
                "{}",
                cmd.action()
            );
        }
    }

    #[tokio::test]
    async fn apply_admin_authorized_denies_without_touching_the_registry() {
        // The fail-closed gate: a denied command returns an Error and NEVER applies (nothing installed).
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        let authz = AllowList::deny_all(); // denies everything

        let resp = host
            .apply_admin_authorized(
                AdminCommand::InstallSession(spec("blocked")),
                "someone",
                &authz,
                Some(&mut factory),
                None,
            )
            .await;
        assert!(
            matches!(&resp, AdminResponse::Error { message } if message.contains("denied")),
            "a denied command is an error: {resp:?}"
        );
        assert!(host.is_empty(), "a denied install touches nothing");
        assert_eq!(factory.builds, 0, "the factory is never even consulted");
    }

    #[tokio::test]
    async fn apply_admin_authorized_applies_when_permitted() {
        let mut host = AgentHost::new();
        let mut factory = StubFactory { builds: 0 };
        let authz = AllowList::allow_all_for_local_admin();

        let resp = host
            .apply_admin_authorized(
                AdminCommand::InstallSession(spec("ok")),
                "owner",
                &authz,
                Some(&mut factory),
                None,
            )
            .await;
        assert_eq!(
            resp,
            AdminResponse::Installed {
                id: SessionId::new(Hash::of(b"ok"))
            }
        );
        assert!(host.contains(&SessionId::new(Hash::of(b"ok"))));
    }
}
