//! Cadenza fleet ↔ Slack bridge — library crate.
//!
//! Internal fleet dev tooling: lets the operator drive the fleet **concierge** (and any agent) from Slack
//! instead of the tmux window. It is a SIDE-CAR to the concierge (see `DESIGN.md`): asks/backlog mirror
//! outbound to a Slack thread, and an operator's in-thread reply routes back to the asking agent as a
//! fleet `answer` — while the concierge's own tmux flow keeps working (dual-channel, first-answer-wins).
//!
//! This slice ports the pure, transport-agnostic core to Rust:
//! - [`inbox`] — the fleet inbox protocol (byte-compatible `Message`/`deliver`/`drain`/`mark_processed`
//!   with `cargo xtask fleet`), the substrate the sidecar delivers `answer`s through.
//! - [`format`] — Slack ↔ fleet message shaping (parse an operator line into a send intent; render a
//!   fleet message as Slack mrkdwn).
//!
//! The Slack transport (slack-morphism, Socket Mode) and the sidecar watch/route loop land in later
//! slices atop this core. Kept a standalone crate (own workspace) so the async transport tree never
//! enters the seed workspace's lockfile or `cargo xtask check` — see `Cargo.toml`.

pub mod config;
pub mod format;
pub mod inbox;

pub use config::{Config, SlackTokens};
pub use format::{Intent, help_text, parse_operator_message, render_fleet_message};
pub use inbox::{Drained, Message, deliver, drain, inbox_dir, mark_processed};
