# PR #1949 review — cdz-agent-host/src/admin.rs (v-agent-harness-host) — API-design [VERIFIED, design-quality]

https://github.com/camshaft/cadenza/pull/1949 (admin control interface — the command layer). Copilot 2
inline, both API-design suggestions on the NEW module (not correctness bugs). Worth relaying while the API
is still fresh/unstable.

## `apply_admin` requires `&mut dyn SessionFactory` even for non-installing commands (Copilot, admin.rs:108) — API-ergonomics [VERIFIED]
> `apply_admin` requires a `&mut dyn SessionFactory` even for commands that don't install sessions
> (ListSessions/SessionStatus/StopSession). This forces all admin call sites (and tests) to thread a
> mutable factory even for pure read/remove operations… Consider making the factory parameter optional
> (and erroring on InstallSession when absent), or splitting install into a separate method so non-install
> commands don't need a factory at all.

VERIFIED in the diff: `pub async fn apply_admin(&mut self, cmd, factory: &mut dyn SessionFactory)`
(admin.rs:110-113) takes the factory unconditionally, but only the `InstallSession(spec)` arm calls
`factory.build(...)`; `ListSessions` / `SessionStatus{id}` (via `self.get`) / `StopSession{id}` (via
`self.remove`) never touch it. So every caller + test of a pure read/remove must still construct and thread
a `&mut dyn SessionFactory`. Design-ergonomics, not a bug. Options (Copilot's): make the param
`Option<&mut dyn SessionFactory>` and error `InstallSession` when `None`; or split `install_session` into
its own method so the read/remove commands need no factory. v-agent-harness-host's call — flagging while
the API surface is new.

## `AdminResponse` returns session ids as raw `String` while `AdminCommand`/`InstallSpec` use `SessionId` (Copilot, admin.rs:65) — API-consistency [VERIFIED]
> AdminResponse returns session IDs as raw Strings (Installed/Sessions/Stopped), while
> AdminCommand/InstallSpec use SessionId. This makes the API inconsistent and loses SessionId's
> type-safety/cheap-clone semantics (Arc<str>)… Consider using SessionId consistently in both commands and
> responses, and only converting to/from strings at the transport/serialization boundary.

VERIFIED: `InstallSpec.id: SessionId` (:41), `AdminCommand::SessionStatus{id: SessionId}` /
`StopSession{id: SessionId}` (:58/:60) — but `AdminResponse::Installed{id: String}` (:69),
`Sessions{ids: Vec<String>}` (:71), `Stopped{id: String}` (:75). Asymmetric: the request side is typed
`SessionId` (Arc<str>, cheap clone, type-safe), the response side degrades to `String`, forcing an
allocation/parse even for an in-process caller staying in `SessionId`-land. Design-consistency. Fix
(Copilot): use `SessionId` in the response variants too, converting to/from `String` only at the
transport/serialization boundary (or, if a stringly transport is intended everywhere, make the command
side `String` too for symmetry — but the typed direction is the better one). v-agent-harness-host owns
cdz-agent-host/src. Both LOW — design-quality, no runtime defect.
