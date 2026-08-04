//! `cdz-agent-daemon` — the real multi-session agent host process.
//!
//! Boots entirely from ONE TOML config file whose path is the SOLE cli argument (operator directive: "set
//! the entire thing up via config and not from cli args; the only arg I want is the path to the config").
//! The config ([`DaemonConfig`](cdz_agent_host::DaemonConfig)) selects the INFRASTRUCTURE — the durable-log backend, observability
//! (s2n-quic-dc-metrics), and effect-retry policy. Sessions are NOT listed in the config: the daemon boots
//! its infrastructure + an empty session registry, and sessions are INSTALLED at runtime (the install
//! mechanism — control interface vs a spawn/supervision path — is the next slice, pending the operator's
//! steer; this slice is the no-regret INFRA-BOOT half every answer needs).
//!
//! ```text
//! cargo run --bin cdz-agent-daemon --features live-net -- /etc/cdz/daemon.toml
//! ```
//!
//! The live wiring (`live_executor_set` → the real Bedrock/HTTP transports) is behind `live-net`; the
//! default build compiles a no-op main so the bin target always has a main + the hermetic gate never pulls
//! the network tree. Config parsing itself is always-on (it's infra-core), so a config-path smoke test runs
//! without live-net.

#[cfg(not(feature = "live-net"))]
fn main() {
    eprintln!(
        "cdz-agent-daemon: built without `live-net` — the real transports aren't compiled in. \
         Rebuild with `--features live-net` to run the daemon."
    );
    std::process::exit(2);
}

#[cfg(feature = "live-net")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use cdz_agent_host::{AgentHost, AsyncAgentHost, DaemonConfig};

    // The SOLE cli arg is the config path.
    let mut args = std::env::args().skip(1);
    let config_path = match (args.next(), args.next()) {
        (Some(p), None) => p,
        _ => {
            eprintln!("usage: cdz-agent-daemon <config.toml>   (exactly one arg: the config path)");
            return std::process::ExitCode::from(2);
        }
    };

    // Boot from the config. A bad config is fatal (no recovery) → exit non-zero with the reason.
    let config = match DaemonConfig::from_toml_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(1);
        }
    };
    eprintln!(
        "cdz-agent-daemon: booted from {config_path} — log={:?}, observability.enabled={}, \
         retries.max_attempts={}",
        config.log, config.observability.enabled, config.retries.max_attempts
    );

    // Assemble the real executor set (Clock + HTTP + Bedrock; env creds). A future slice wires the
    // config-selected durable-log backend + observability into the host here; for now the executor set is
    // the live spine every session shares.
    let _executors = match cdz_agent_host::live_executor_set().await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cdz-agent-daemon: could not build the live executor set: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    // Boot the async multi-session loop over an EMPTY registry — sessions are installed at runtime (the
    // install mechanism is the next slice). The daemon runs until shutdown (ctrl-c wiring lands with the
    // control interface); with no sessions + no held inbox sender the loop returns immediately today, which
    // is the honest current behavior: "the infra boots; nothing is installed yet."
    let host = AsyncAgentHost::new(AgentHost::new());
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    // Drop our inbox sender so the loop isn't held open by us; a real control interface will hold one to
    // feed install/inbound messages.
    drop(host.inbox());
    match host.run_with_wall_clock(shutdown_rx).await {
        Ok(_registry) => {
            eprintln!("cdz-agent-daemon: shut down cleanly.");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cdz-agent-daemon: host loop failed: {e:?}");
            std::process::ExitCode::from(1)
        }
    }
}
