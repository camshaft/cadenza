//! The mock control server's STATE MODEL (`DESIGN-http-outpost-conformance-harness.md` §3.3).
//!
//! A pure, synchronous model of everything the mock holds and everything the driver can inject or observe.
//! The two async faces wrap it: the CONTROL-PLANE ws layer (gateway-facing) calls [`MockState::on_connect`]
//! / [`MockState::record_control_up`] / [`MockState::reply_for`] / … as gateway traffic arrives, and the
//! ADMIN layer (driver-facing) calls [`MockState::set_config`] / [`MockState::push_root_router`] /
//! [`MockState::prime_reply`] / [`MockState::reset`] and the observation getters. Keeping the logic here —
//! sync + dependency-light — makes it exhaustively unit-testable without sockets.
//!
//! Programs are referred to BY NAME in the admin surface (the driver reads `set_root_router("router-hello")`
//! rather than a hash); [`MockState`] is seeded with the build manifest (`name -> ProgramHash bytes`) and
//! [`resolves`](MockState::resolve) a name-or-raw-hash to the bytes shipped in a [`ControlConfig`].

use bytes::Bytes;
use cdz_http_protocol::{ControlConfig, ControlDown, ControlUp};
use cdz_str::Str;
use std::collections::HashMap;

/// The driver ↔ mock admin protocol (binary-AST `AdminCommand`/`AdminReply` frames).
pub mod admin;
pub mod frame_ids;
/// The admin HTTP server (driver-facing) — binary-AST bodies over hyper.
pub mod server;
/// The control-plane ws server (gateway-facing) — ships config, captures ControlUp, replies ControlDown.
pub mod ws;

/// One entry of the mock's ordered observation log — the control-plane events, in arrival order, so a test
/// can assert on ORDERING (a config was served before a push, a control.send arrived after a request, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A gateway dialed the control link and opened a session.
    Connect { session: Bytes },
    /// The mock shipped its [`ControlConfig`] to a session on connect.
    ConfigServed { session: Bytes },
    /// The mock pushed a new root-router hash to connected sessions (a live-swap).
    RootRouterPushed { root_router: Bytes },
    /// A handler's `control.send` arrived up the link (see the captured [`ControlUp`] for the detail).
    ControlUpReceived { program: Bytes, correlation: Bytes },
    /// The mock sent a `ControlDown` to a session (a correlated reply or an unsolicited push).
    ControlDownSent { session: Bytes, correlation: Bytes },
    /// A gateway session closed.
    Disconnect { session: Bytes },
}

/// A connected gateway session the mock is aware of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub session: Bytes,
    /// Whether the mock has shipped its config to this session yet.
    pub config_served: bool,
}

/// How the mock replies to an incoming `control.send` ([`ControlUp`]) — primed by the driver so a test can
/// exercise the request/response leg. The reply's `correlation` is echoed from the matched `ControlUp` so it
/// routes back to the exact awaiting handler call (the ws layer builds the [`ControlDown`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimedReply {
    /// Match criteria; a `None` field matches anything. All specified fields must match.
    pub match_program: Option<Bytes>,
    pub match_path: Option<Str>,
    /// The reply payload to send back (correlation echoed from the matched up).
    pub reply: Bytes,
}

impl PrimedReply {
    /// Whether this primed reply applies to `up` — every specified criterion matches.
    #[must_use]
    pub fn matches(&self, up: &ControlUp) -> bool {
        self.match_program.as_ref().is_none_or(|p| p == &up.program)
            && self
                .match_path
                .as_ref()
                .is_none_or(|path| path == &up.request.path)
    }
}

/// Everything the mock control server holds. Not thread-safe on its own — the async servers wrap it in an
/// `Arc<Mutex<MockState>>`. Seeded with the program manifest; the injected config + captured observations
/// are cleared by [`reset`](MockState::reset) between scenarios.
#[derive(Debug, Clone, Default)]
pub struct MockState {
    /// The build manifest: program name -> its `ProgramHash` bytes. Immutable across a run.
    programs: HashMap<Str, Bytes>,
    /// The `ControlConfig` shipped on connect — `None` until the driver sets it.
    config: Option<ControlConfig>,
    /// Captured `ControlUp` envelopes (what handlers sent up), oldest first.
    captured_up: Vec<ControlUp>,
    /// Primed replies to `control.send`, checked first-match-wins.
    primed_replies: Vec<PrimedReply>,
    /// The ordered event log.
    events: Vec<Event>,
    /// Connected gateway sessions.
    connections: Vec<Connection>,
}

impl MockState {
    /// A fresh mock seeded with the program manifest (`name -> ProgramHash bytes`).
    #[must_use]
    pub fn new(programs: HashMap<Str, Bytes>) -> Self {
        Self {
            programs,
            ..Self::default()
        }
    }

    /// Resolve a program reference: a manifest NAME (preferred) or, failing that, treat `reference` itself as
    /// the raw hash bytes. Returns the `ProgramHash` bytes to ship. (Raw-hash lets a test exercise an
    /// unknown/garbage hash; an unresolvable name returns `None`.)
    #[must_use]
    pub fn resolve(&self, reference: &str) -> Option<Bytes> {
        self.programs.get(reference).cloned()
    }

    // --- admin (driver → mock): inject -----------------------------------------------------------------

    /// Seed the program manifest: bind `name` to its `ProgramHash` bytes (survives [`reset`](Self::reset) —
    /// the manifest is the build input, not per-scenario state).
    pub fn add_program(&mut self, name: Str, hash: Bytes) {
        self.programs.insert(name, hash);
    }

    /// Set the [`ControlConfig`] the mock ships to a gateway on connect.
    pub fn set_config(&mut self, config: ControlConfig) {
        self.config = Some(config);
    }

    /// Live-swap: update the shipped config's `root_router` to `root_router` and log the push. Returns the
    /// sessions the ws layer should push the new hash to (all currently-connected). No-op-safe if no config
    /// is set yet (it stages the hash into a fresh config's `root_router` with empty cas fields).
    pub fn push_root_router(&mut self, root_router: Bytes) -> Vec<Bytes> {
        match &mut self.config {
            Some(c) => c.root_router = root_router.clone(),
            none => {
                *none = Some(ControlConfig {
                    cas_url: Str::new(),
                    cas_credential: Bytes::new(),
                    root_router: root_router.clone(),
                });
            }
        }
        self.events.push(Event::RootRouterPushed { root_router });
        self.connections.iter().map(|c| c.session.clone()).collect()
    }

    /// Prime how the mock replies to a matching `control.send`. First-match-wins in insertion order.
    pub fn prime_reply(&mut self, reply: PrimedReply) {
        self.primed_replies.push(reply);
    }

    /// Clear the injected config + all captured observations + primed replies + connections (per-scenario
    /// isolation). The program manifest is preserved (it is the immutable build input).
    pub fn reset(&mut self) {
        self.config = None;
        self.captured_up.clear();
        self.primed_replies.clear();
        self.events.clear();
        self.connections.clear();
    }

    // --- control-plane (gateway ↔ mock) ----------------------------------------------------------------

    /// Record a gateway connecting and return the [`ControlConfig`] to ship (if the driver has set one).
    /// Logs the connect + (when a config is shipped) the config-served event, and registers the session.
    pub fn on_connect(&mut self, session: Bytes) -> Option<ControlConfig> {
        self.events.push(Event::Connect {
            session: session.clone(),
        });
        let config_served = self.config.is_some();
        if config_served {
            self.events.push(Event::ConfigServed {
                session: session.clone(),
            });
        }
        self.connections.push(Connection {
            session,
            config_served,
        });
        self.config.clone()
    }

    /// Capture a handler's `control.send` [`ControlUp`] (with its handler id + correlation + request context)
    /// and log it. Returns the correlation-matched reply [`ControlDown`] the ws layer should send back, if a
    /// primed reply matches (first-match-wins); `None` means no reply (the send was fire-and-forget for this
    /// scenario).
    pub fn record_control_up(&mut self, up: ControlUp) -> Option<ControlDown> {
        self.events.push(Event::ControlUpReceived {
            program: up.program.clone(),
            correlation: up.correlation.clone(),
        });
        let reply = self.reply_for(&up).map(|payload| ControlDown {
            session: up.session.clone(),
            correlation: up.correlation.clone(),
            payload,
        });
        self.captured_up.push(up);
        if let Some(down) = &reply {
            self.events.push(Event::ControlDownSent {
                session: down.session.clone(),
                correlation: down.correlation.clone(),
            });
        }
        reply
    }

    /// The reply payload primed for `up`, if any (first matching [`PrimedReply`]).
    #[must_use]
    pub fn reply_for(&self, up: &ControlUp) -> Option<Bytes> {
        self.primed_replies
            .iter()
            .find(|r| r.matches(up))
            .map(|r| r.reply.clone())
    }

    /// Record that the mock sent an unsolicited [`ControlDown`] push (a correlated reply is logged by
    /// [`record_control_up`](MockState::record_control_up) instead).
    pub fn record_control_down(&mut self, down: &ControlDown) {
        self.events.push(Event::ControlDownSent {
            session: down.session.clone(),
            correlation: down.correlation.clone(),
        });
    }

    /// Record a gateway session closing.
    pub fn on_disconnect(&mut self, session: Bytes) {
        self.connections.retain(|c| c.session != session);
        self.events.push(Event::Disconnect { session });
    }

    // --- admin (mock → driver): observe ----------------------------------------------------------------

    /// The captured `ControlUp` envelopes, oldest first.
    #[must_use]
    pub fn captured_up(&self) -> &[ControlUp] {
        &self.captured_up
    }

    /// The ordered event log.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The currently-connected gateway sessions.
    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// The config the mock currently ships on connect (if set).
    #[must_use]
    pub fn config(&self) -> Option<&ControlConfig> {
        self.config.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(tag: &str) -> Bytes {
        // a distinct 33-byte program-hash-shaped value for tests.
        let mut v = format!("cdz.prog.{tag}").into_bytes();
        v.resize(33, b'.');
        Bytes::from(v)
    }

    fn manifest() -> HashMap<Str, Bytes> {
        HashMap::from([
            (Str::from("router-hello"), hash("router-hello")),
            (Str::from("router-echo"), hash("router-echo")),
        ])
    }

    fn up(program: Bytes, path: &str, correlation: &str, payload: &str) -> ControlUp {
        ControlUp {
            program,
            session: Bytes::from_static(b"sess-1"),
            correlation: Bytes::copy_from_slice(correlation.as_bytes()),
            payload: Bytes::copy_from_slice(payload.as_bytes()),
            request: cdz_http_protocol::RequestContext {
                method: Str::from("POST"),
                path: Str::from(path),
                headers: vec![],
            },
        }
    }

    #[test]
    fn resolves_a_name_to_its_hash_and_misses_an_unknown() {
        let s = MockState::new(manifest());
        assert_eq!(s.resolve("router-hello"), Some(hash("router-hello")));
        assert_eq!(s.resolve("nope"), None);
    }

    #[test]
    fn on_connect_ships_the_set_config_and_logs_connect_then_config_served() {
        let mut s = MockState::new(manifest());
        // No config set → connect logs Connect only, ships nothing, session not config-served.
        assert!(s.on_connect(Bytes::from_static(b"a")).is_none());
        assert_eq!(
            s.events(),
            &[Event::Connect {
                session: Bytes::from_static(b"a")
            }]
        );
        assert!(!s.connections()[0].config_served);

        // Set a config → the next connect ships it and logs ConfigServed after Connect.
        s.reset();
        s.set_config(ControlConfig {
            cas_url: Str::from("http://cas"),
            cas_credential: Bytes::new(),
            root_router: hash("router-hello"),
        });
        let shipped = s
            .on_connect(Bytes::from_static(b"b"))
            .expect("config shipped");
        assert_eq!(shipped.root_router, hash("router-hello"));
        assert_eq!(
            s.events(),
            &[
                Event::Connect {
                    session: Bytes::from_static(b"b")
                },
                Event::ConfigServed {
                    session: Bytes::from_static(b"b")
                },
            ]
        );
        assert!(s.connections()[0].config_served);
    }

    #[test]
    fn push_root_router_updates_config_and_targets_connected_sessions() {
        let mut s = MockState::new(manifest());
        s.set_config(ControlConfig {
            cas_url: Str::from("http://cas"),
            cas_credential: Bytes::new(),
            root_router: hash("router-hello"),
        });
        s.on_connect(Bytes::from_static(b"s1"));
        s.on_connect(Bytes::from_static(b"s2"));
        let targets = s.push_root_router(hash("router-echo"));
        // Both connected sessions are targeted, and the shipped config now carries the new hash.
        assert_eq!(
            targets,
            vec![Bytes::from_static(b"s1"), Bytes::from_static(b"s2")]
        );
        assert_eq!(s.config().unwrap().root_router, hash("router-echo"));
        assert!(s.events().contains(&Event::RootRouterPushed {
            root_router: hash("router-echo")
        }));
    }

    #[test]
    fn control_up_is_captured_and_a_primed_reply_correlates_back() {
        let mut s = MockState::new(manifest());
        // Prime a reply matched on path.
        s.prime_reply(PrimedReply {
            match_program: None,
            match_path: Some(Str::from("/emit")),
            reply: Bytes::from_static(b"PONG"),
        });
        let down = s
            .record_control_up(up(hash("handler-emitter"), "/emit", "corr-1", "ping"))
            .expect("a matching primed reply produces a ControlDown");
        // The reply echoes the correlation + session so it routes back to the exact call.
        assert_eq!(down.correlation, Bytes::from_static(b"corr-1"));
        assert_eq!(down.session, Bytes::from_static(b"sess-1"));
        assert_eq!(down.payload, Bytes::from_static(b"PONG"));
        // The up is captured with its full provenance/context.
        assert_eq!(s.captured_up().len(), 1);
        assert_eq!(s.captured_up()[0].program, hash("handler-emitter"));
        assert_eq!(s.captured_up()[0].request.path, "/emit");
    }

    #[test]
    fn a_control_up_with_no_matching_reply_is_captured_without_a_down() {
        let mut s = MockState::new(manifest());
        s.prime_reply(PrimedReply {
            match_program: None,
            match_path: Some(Str::from("/other")),
            reply: Bytes::from_static(b"X"),
        });
        assert!(
            s.record_control_up(up(hash("h"), "/emit", "c", "p"))
                .is_none()
        );
        assert_eq!(s.captured_up().len(), 1);
        // No ControlDownSent event, since nothing matched.
        assert!(
            !s.events()
                .iter()
                .any(|e| matches!(e, Event::ControlDownSent { .. }))
        );
    }

    #[test]
    fn reset_clears_observations_but_keeps_the_manifest() {
        let mut s = MockState::new(manifest());
        s.set_config(ControlConfig {
            cas_url: Str::from("http://cas"),
            cas_credential: Bytes::new(),
            root_router: hash("router-hello"),
        });
        s.on_connect(Bytes::from_static(b"s1"));
        s.record_control_up(up(hash("h"), "/x", "c", "p"));
        s.reset();
        assert!(s.config().is_none());
        assert!(s.captured_up().is_empty());
        assert!(s.events().is_empty());
        assert!(s.connections().is_empty());
        // The manifest survives a reset.
        assert_eq!(s.resolve("router-hello"), Some(hash("router-hello")));
    }

    #[test]
    fn disconnect_drops_the_session_and_logs_it() {
        let mut s = MockState::new(manifest());
        s.on_connect(Bytes::from_static(b"s1"));
        s.on_connect(Bytes::from_static(b"s2"));
        s.on_disconnect(Bytes::from_static(b"s1"));
        assert_eq!(s.connections().len(), 1);
        assert_eq!(s.connections()[0].session, Bytes::from_static(b"s2"));
        assert!(s.events().contains(&Event::Disconnect {
            session: Bytes::from_static(b"s1")
        }));
    }
}
