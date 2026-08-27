//! Cadenza fleet ↔ Slack bridge — library crate.
//!
//! Internal fleet dev tooling: lets the operator drive the fleet **concierge** (and any agent) from Slack
//! instead of the tmux window. It is a SIDE-CAR to the concierge (see `DESIGN.md`): asks/backlog mirror
//! outbound to a Slack thread, and an operator's in-thread reply routes back to the asking agent as a
//! fleet `answer` — while the concierge's own tmux flow keeps working (dual-channel, first-answer-wins).
//!
//! The pure, transport-agnostic core (Rust):
//! - [`inbox`] — the fleet inbox protocol (byte-compatible `Message`/`deliver`/`drain`/`mark_processed`
//!   with `cargo xtask fleet`), the substrate the sidecar delivers `answer`s through.
//! - [`format`] — Slack ↔ fleet message shaping (parse an operator line into a send intent; render a
//!   fleet message as Slack mrkdwn).
//! - [`config`] — fail-soft credential/wiring resolution (env > gitignored `slack.toml` > defaults).
//! - [`sidecar`] — the outbound brain: decide which concierge `ask`/`backlog` to mirror, and the
//!   `thread_ts → asker` map (persisted to `slack-threads.json`) that routes an operator reply back.
//! - [`watchdog`] — the daemon's second job (operator decision): the `cargo xtask fleet watchdog` command
//!   spec + cadence decision, so the long-lived Slack host also heals stalled loops out-of-band.
//!
//! The Slack transport (slack-morphism, Socket Mode) binary wires these together in a later slice. Kept a
//! standalone crate (own workspace) so the async transport tree never enters the seed workspace's lockfile
//! or `cargo xtask check` — see `Cargo.toml`.

pub mod config;
pub mod format;
pub mod inbox;
pub mod sidecar;
pub mod watchdog;

pub use config::{Config, SlackTokens};
pub use format::{
    Intent, RELAY_QUEUE_WARN, RelayPlan, help_text, parse_operator_message, relay_plan,
    render_fleet_message, render_fleet_message_plain,
};
pub use inbox::{
    Drained, Message, deliver, drain, inbox_dir, is_valid_agent_name, mark_failed, mark_processed,
};
pub use sidecar::{MirroredAsk, ThreadMap, ToMirror, is_mirrored_kind, select_to_mirror};
pub use watchdog::{WatchdogSpec, due};
