//! `cdz-agent-daemon` — the real multi-session agent host process.
//!
//! Boots entirely from ONE TOML config file whose path is the SOLE cli argument (operator directive: "set
//! the entire thing up via config and not from cli args; the only arg I want is the path to the config").
//! The config ([`DaemonConfig`](cdz_agent_host::DaemonConfig)) selects the INFRASTRUCTURE — the durable-log
//! backend, observability (s2n-quic-dc-metrics), effect-retry policy, and the admin control interface.
//! Sessions are NOT listed in the config: the daemon boots its infrastructure + an EMPTY session registry,
//! and sessions are INSTALLED at runtime through the ADMIN CONTROL INTERFACE — when `[admin].enabled`, the
//! daemon binds a local Unix socket and serves admin commands (install / list / status / stop) concurrently
//! with the host loop, ctrl-c ending both. That socket is the operator's "communicate with the daemon +
//! send it commands as admin" path.
//!
//! ```text
//! cargo run --bin cdz-agent-daemon --features admin,live-net -- /etc/cdz/daemon.toml
//! ```
//!
//! The live wiring (`live_executor_set` → the real Bedrock/HTTP transports) is behind `live-net`, and the
//! admin socket behind `admin`; the default build compiles a no-op main so the bin target always has a main
//! (and the hermetic gate never pulls the network tree). Config parsing itself is always-on (it's
//! infra-core), so a config-path smoke test runs without either feature.

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
    use cdz_agent_host::DaemonConfig;
    // AgentHost/AsyncAgentHost are used only by the admin control-interface block below (the sole consumer
    // of the host loop today); import them there so a `live-net`-without-`admin` build has no unused import.
    #[cfg(feature = "admin")]
    use cdz_agent_host::{AgentHost, AsyncAgentHost};

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

    // The admin CONTROL INTERFACE is what makes this a live control plane: when `[admin].enabled` (and the
    // `admin` feature is compiled), boot the async multi-session loop over an EMPTY registry, bind the Unix
    // socket, and serve it CONCURRENTLY with the loop — ctrl-c drives a graceful shutdown of both. Sessions
    // are INSTALLED at runtime through that socket. Without an active control interface the loop would have
    // no producer to serve, so there's nothing to run.
    //
    // The admin authorizer is the trusted-local-admin allowlist: the socket is owner-only (0o600), so every
    // caller is the daemon's owner, and the allowlist grants that principal the v0 admin actions (a finer
    // Cedar policy is a later slice).
    #[cfg(feature = "admin")]
    if config.admin.enabled {
        let Some(socket_path) = config.admin.socket.clone() else {
            // validate() guarantees a socket path when enabled; defend anyway rather than panic.
            eprintln!("cdz-agent-daemon: [admin] enabled but no socket path — aborting");
            return std::process::ExitCode::from(1);
        };
        let socket = match cdz_agent_host::AdminSocket::bind(&socket_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cdz-agent-daemon: could not bind admin socket {socket_path}: {e}");
                return std::process::ExitCode::from(1);
            }
        };
        eprintln!("cdz-agent-daemon: admin control interface listening on {socket_path}");

        let host = AsyncAgentHost::new(AgentHost::new()).with_admin_authz(Box::new(
            cdz_agent_host::AllowList::allow_all_for_local_admin(),
        ));
        // Drop our inbox sender so an idle inbox doesn't hold the loop open; the admin socket's AdminChannel
        // is what keeps the loop alive as a control plane.
        drop(host.inbox());
        let admin_channel = host.admin_channel();

        let (loop_sd_tx, loop_sd_rx) = tokio::sync::oneshot::channel::<()>();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel::<()>();
        // ctrl-c → shut down BOTH the host loop and the socket server.
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("cdz-agent-daemon: ctrl-c received — shutting down");
                let _ = loop_sd_tx.send(());
                let _ = sock_sd_tx.send(());
            }
        });
        // Run the loop + the socket server together: the loop owns the !Send registry, the socket task is
        // the Send producer feeding it. Both end on ctrl-c.
        let (loop_result, ()) = tokio::join!(
            host.run_with_wall_clock(loop_sd_rx),
            socket.serve(admin_channel, sock_sd_rx)
        );
        return match loop_result {
            Ok(_registry) => {
                eprintln!("cdz-agent-daemon: shut down cleanly.");
                std::process::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("cdz-agent-daemon: host loop failed: {e:?}");
                std::process::ExitCode::from(1)
            }
        };
    }

    #[cfg(not(feature = "admin"))]
    if config.admin.enabled {
        eprintln!(
            "cdz-agent-daemon: [admin] enabled in config but this build lacks the `admin` feature — \
             no control interface. Rebuild with `--features admin` (or `admin,live-net`)."
        );
    }

    // No active control interface (admin disabled, or the `admin` feature isn't compiled). The host loop
    // would have no producer to serve, so there's nothing to run — report + exit cleanly.
    eprintln!("cdz-agent-daemon: no active control interface — nothing to serve; exiting.");
    std::process::ExitCode::SUCCESS
}
