//! `cdz-agent-daemon` — the real multi-session agent host process.
//!
//! Boots entirely from ONE TOML config file whose path is the SOLE cli argument (operator directive: "set
//! the entire thing up via config and not from cli args; the only arg I want is the path to the config").
//! The config ([`DaemonConfig`](cdz_agent_host::DaemonConfig)) selects the INFRASTRUCTURE — the durable-log
//! backend, observability (s2n-quic-dc-metrics), effect-retry policy, and the admin control interface.
//! Sessions are NOT listed in the config: the daemon boots its infrastructure + an EMPTY session registry,
//! and sessions are INSTALLED at runtime through the ADMIN CONTROL INTERFACE — when `[admin].enabled`, the
//! daemon binds a local Unix socket and serves admin commands (list / status / stop, and install once the
//! session factory is wired) concurrently with the host loop, ctrl-c ending both. That socket is the
//! operator's "communicate with the daemon + send it commands as admin" path.
//!
//! (`install-session` loads the reducer through a [`ComponentSessionFactory`](cdz_agent_host::ComponentSessionFactory):
//! it fetches the named reducer component from the `[blob]`-configured blob store, lifts it, and installs a
//! live session. An installed session gets a deny-all authorizer for v0 — it takes no world-effects until a
//! per-reducer capability policy is configured, a later slice.)
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

/// Install the `tracing` subscriber from `[tracing]` config. `Ok(())` with no subscriber when disabled (the
/// facade stays a no-op); otherwise build an `EnvFilter` from `filter`, a fmt layer per `format`, and a
/// writer per `output` (opening the file for a `file` output), then `.init()` the global subscriber. `Err`
/// on a bad filter directive or an un-openable file — a fatal boot misconfig the caller surfaces + exits on.
#[cfg(feature = "live-net")]
fn init_tracing(cfg: &cdz_agent_host::TracingConfig) -> Result<(), String> {
    use cdz_agent_host::{TracingFormat, TracingOutput};
    use tracing_subscriber::fmt::writer::BoxMakeWriter;
    use tracing_subscriber::EnvFilter;

    if !cfg.enabled {
        return Ok(());
    }

    // The RUST_LOG-grammar directive → EnvFilter. A bad directive is a fatal config error (not silently
    // dropped to a default), so a typo'd filter is loud at boot.
    let filter = EnvFilter::try_new(&cfg.filter)
        .map_err(|e| format!("bad [tracing] filter {:?}: {e}", cfg.filter))?;

    // Collapse the WRITER dimension to one boxed type so the format match below is the only branching. A
    // `file` output opens (create+append) the path; stderr/stdout use the process streams.
    let writer: BoxMakeWriter = match &cfg.output {
        TracingOutput::Stderr => BoxMakeWriter::new(std::io::stderr),
        TracingOutput::Stdout => BoxMakeWriter::new(std::io::stdout),
        TracingOutput::File { path } => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("could not open [tracing] output file {path}: {e}"))?;
            BoxMakeWriter::new(std::sync::Mutex::new(file))
        }
    };

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer);
    // The FORMAT dimension: each returns a distinct builder type, so `.init()` per-arm (can't unify the
    // value). `try_init` so a double-init (shouldn't happen at boot) is an error, not a panic.
    match cfg.format {
        TracingFormat::Compact => builder.compact().try_init(),
        TracingFormat::Pretty => builder.pretty().try_init(),
        TracingFormat::Json => builder.json().try_init(),
    }
    .map_err(|e| format!("could not install tracing subscriber: {e}"))
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

    // Install the tracing subscriber from `[tracing]` BEFORE anything else logs — so the boot line + every
    // later host span/event is captured through it. `[tracing].enabled=false` (the default) installs no
    // subscriber (the facade's spans stay the near-zero-cost no-op). A subscriber build failure (a bad filter
    // directive, an un-openable file) is fatal like a bad config.
    if let Err(e) = init_tracing(&config.tracing) {
        eprintln!("cdz-agent-daemon: could not initialize tracing: {e}");
        return std::process::ExitCode::from(1);
    }

    eprintln!(
        "cdz-agent-daemon: booted from {config_path} — log={:?}, observability.enabled={}, \
         retries.max_attempts={}, tracing.enabled={}",
        config.log,
        config.observability.enabled,
        config.retries.max_attempts,
        config.tracing.enabled
    );

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

        // Build the reducer BLOB STORE the install factory loads from, per [blob] config (memory or a disk
        // dir). Boxed as `dyn SessionFactory` so the two concrete blob-store types share one factory value.
        use cdz_agent_host::{
            AllowList, BlobConfig, ComponentSessionFactory, LiveExecutorSet, SessionFactory,
        };
        // Each installed session's own authorizer: DENY-ALL for v0 (fail-closed — an installed session takes
        // no world-effects until a policy grants them; the per-reducer capability policy is a later slice).
        let session_authz = || -> Box<dyn cdz_kernel::authz::Authorize> {
            Box::new(cdz_kernel::authz::Authorizer::deny_all())
        };
        // A per-session durable-log sink builder when [log].backend = file (each session's log →
        // <dir>/<id>.log); None for the in-memory log backend. Applied to the factory below.
        use cdz_agent_host::{FileLogSinkBuilder, LogConfig, LogSinkBuilder};
        let log_sink: Option<Box<dyn LogSinkBuilder>> = match &config.log {
            LogConfig::File { dir } => Some(Box::new(FileLogSinkBuilder::new(dir))),
            // memory (default) → in-memory session logs; dynamo → not yet a session-log backend (later slice).
            _ => None,
        };
        // Build the AgentHost FIRST so its metrics registry exists — the live executor set's per-effect
        // metrics register into the SAME registry (one registry the exporter reports over: host-boundary +
        // per-effect metrics together).
        let agent_host = AgentHost::new();
        // Build the LIVE executor transports ONCE, here at startup (this is where the AWS config / IMDS load
        // happens — on the boot path, NOT per install). Each install then CLONES these into a fresh executor
        // set cheaply, so a slow IMDS probe can't stall the host loop mid-run (#1987 review). The per-effect
        // metrics register into the host's registry (shared).
        // §lifecycle: mint the loop's lifecycle channel HERE, BEFORE the factory, so the SAME channel is wired
        // into (a) each session's LifecycleExecutor (via LiveExecutorSet::with_lifecycle_channel below) and
        // (b) the host loop (via with_factory_and_lifecycle) — a session's lifecycle op then lands on the loop
        // the daemon runs. (The construction chicken-egg: the factory needs the sender, but the host — which
        // otherwise mints the channel — is built last.)
        let (lifecycle_tx, lifecycle_rx) = AsyncAgentHost::new_lifecycle_channel();
        let executors = match LiveExecutorSet::new(agent_host.registry()).await {
            Ok(e) => e.with_lifecycle_channel(lifecycle_tx.clone()),
            Err(e) => {
                eprintln!("cdz-agent-daemon: could not build the live executor transports: {e}");
                return std::process::ExitCode::from(1);
            }
        };
        // Attach the optional log-sink builder to whichever blob-backed factory we build, then box it. (The
        // two arms are distinct concrete factory types, so the with_log_sink + Box happens per-arm; the
        // executor set is moved into the one arm that runs.)
        let factory: Box<dyn SessionFactory> = match &config.blob {
            BlobConfig::Memory => {
                let mut f = ComponentSessionFactory::new(
                    cdz_kernel::blob::MemBlobStore::new(),
                    executors,
                    session_authz,
                );
                if let Some(b) = log_sink {
                    f = f.with_log_sink(b);
                }
                Box::new(f)
            }
            BlobConfig::Dir { dir } => match cdz_kernel::blob::DiskBlobStore::open(dir) {
                Ok(store) => {
                    let mut f = ComponentSessionFactory::new(store, executors, session_authz);
                    if let Some(b) = log_sink {
                        f = f.with_log_sink(b);
                    }
                    Box::new(f)
                }
                Err(e) => {
                    eprintln!("cdz-agent-daemon: could not open [blob] dir {dir}: {e}");
                    return std::process::ExitCode::from(1);
                }
            },
            // S3-backed store: the AWS-native reducer blob store, behind `live-aws-storage`. A build WITHOUT
            // that feature reports a clean "not compiled in" boot error rather than silently degrading (the
            // config validates regardless of feature, so `backend = "s3"` parses; only the RUNTIME backend is
            // gated). `S3BlobStore::new` loads SDK-default creds from the environment (async, at boot).
            #[cfg(feature = "live-aws-storage")]
            BlobConfig::S3 { bucket, prefix } => {
                let store = cdz_agent_host::S3BlobStore::new(bucket.clone(), prefix.clone()).await;
                let mut f = ComponentSessionFactory::new(store, executors, session_authz);
                if let Some(b) = log_sink {
                    f = f.with_log_sink(b);
                }
                Box::new(f)
            }
            #[cfg(not(feature = "live-aws-storage"))]
            BlobConfig::S3 { .. } => {
                eprintln!(
                    "cdz-agent-daemon: [blob] backend=s3 requires the `live-aws-storage` feature — \
                     rebuild with `--features live-aws-storage` to use the S3 blob store."
                );
                return std::process::ExitCode::from(1);
            }
        };
        eprintln!(
            "cdz-agent-daemon: reducer blob store = {:?}, session log = {:?}",
            config.blob, config.log
        );

        // Metrics EXPORT: when the `metrics-export` feature is compiled AND `[observability]` is enabled,
        // clone the host's metrics registry (the Arc-backed handle shares the same storage the host loop
        // increments) BEFORE `agent_host` moves into the loop, and build a BACKEND-AGNOSTIC MetricsReporter
        // with every configured export backend PUSHED INTO it — so a single periodic drain fans out to all of
        // them at once (reporting is destructive; a per-backend report would starve later backends — operator
        // review on #2124). The periodic loop is shut down alongside the host loop (its own oneshot, fired
        // after the loop returns) and does one final report on the way out. OTLP/prometheus backends push in
        // here too once built (statsd first; a following slice).
        #[cfg(feature = "metrics-export")]
        let (
            export_registry,
            export_reporter,
            export_interval,
            otlp_forwarders,
            prometheus_servers,
        ) = {
            use cdz_agent_host::{MetricsReporter, MetricsTarget, StatsdTarget};
            // Collect the statsd targets. OTLP targets are exportable only when the `metrics-export-otlp`
            // feature is compiled; otherwise they're counted as unexportable so an enabled config with only
            // such targets warns instead of silently no-op'ing.
            let statsd_targets: Vec<StatsdTarget> = if config.observability.enabled {
                config
                    .observability
                    .targets
                    .iter()
                    .filter_map(|t| match t {
                        MetricsTarget::Statsd { endpoint, prefix } => Some(StatsdTarget {
                            endpoint: endpoint.clone(),
                            prefix: prefix.clone(),
                        }),
                        // Non-statsd targets (otlp/prometheus) are collected separately below.
                        _ => None,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // OTLP targets are exportable with the feature, unexportable without it. Counted (not per-loop
            // mutated) so the warning below can name how many targets this build can't handle.
            #[cfg(feature = "metrics-export-otlp")]
            let otlp_targets: Vec<cdz_agent_host::OtlpTarget> = if config.observability.enabled {
                config
                    .observability
                    .targets
                    .iter()
                    .filter_map(|t| match t {
                        MetricsTarget::Otlp {
                            endpoint,
                            scope_name,
                            scope_version,
                        } => Some(cdz_agent_host::OtlpTarget {
                            endpoint: endpoint.clone(),
                            scope_name: scope_name.clone(),
                            scope_version: scope_version.clone(),
                        }),
                        // Non-otlp targets (statsd/prometheus) are collected in their own passes.
                        _ => None,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // Prometheus (PULL) targets are exportable with the metrics-export-prometheus feature. Collected
            // like otlp: with the feature they become backends + scrape servers; without it they count toward
            // the unexportable diagnostic below.
            #[cfg(feature = "metrics-export-prometheus")]
            let prometheus_targets: Vec<cdz_agent_host::PrometheusTarget> =
                if config.observability.enabled {
                    config
                        .observability
                        .targets
                        .iter()
                        .filter_map(|t| match t {
                            MetricsTarget::Prometheus { bind, prefix, .. } => {
                                // allow_non_loopback is a validation-time gate (already enforced in
                                // DaemonConfig::validate); the runtime target only needs bind + prefix.
                                Some(cdz_agent_host::PrometheusTarget {
                                    bind: bind.clone(),
                                    prefix: prefix.clone(),
                                })
                            }
                            _ => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            // Count targets this build can't export (for the enabled-but-nothing-exportable diagnostic): an
            // OTLP target without metrics-export-otlp, or a prometheus target without metrics-export-prometheus.
            let unexportable = config
                .observability
                .targets
                .iter()
                .filter(|t| {
                    (matches!(t, MetricsTarget::Otlp { .. })
                        && !cfg!(feature = "metrics-export-otlp"))
                        || (matches!(t, MetricsTarget::Prometheus { .. })
                            && !cfg!(feature = "metrics-export-prometheus"))
                })
                .count();
            // `mut` only when a push backend (otlp/prometheus) registers into the reporter; a statsd-only build
            // never mutates it after construction.
            #[cfg_attr(
                not(any(feature = "metrics-export-otlp", feature = "metrics-export-prometheus")),
                allow(unused_mut)
            )]
            let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&statsd_targets);
            for f in &failed {
                eprintln!("cdz-agent-daemon: metrics export target unavailable at boot: {f}");
            }
            // Push each OTLP target into the SAME reporter (one drain fans out to statsd + otlp + prometheus
            // together) and keep its (endpoint, receiver) so we can spawn a forwarder task per target below.
            #[cfg(feature = "metrics-export-otlp")]
            let otlp_forwarders: Vec<(String, tokio::sync::mpsc::Receiver<Vec<u8>>)> = otlp_targets
                .iter()
                .map(|t| (t.endpoint.clone(), reporter.push_otlp(t)))
                .collect();
            #[cfg(not(feature = "metrics-export-otlp"))]
            let otlp_forwarders: Vec<((), ())> = Vec::new();
            // Push each prometheus target's backend into the reporter + keep (bind, handle) so we can spawn a
            // scrape server per target below (the handle reads the exposition text the backend re-renders).
            #[cfg(feature = "metrics-export-prometheus")]
            let prometheus_servers: Vec<(
                String,
                s2n_quic_dc_metrics::backend::PrometheusHandle,
            )> = prometheus_targets
                .iter()
                .map(|t| (t.bind.clone(), reporter.push_prometheus(t)))
                .collect();
            #[cfg(not(feature = "metrics-export-prometheus"))]
            let prometheus_servers: Vec<((), ())> = Vec::new();
            // Enabled but NO export backend got set up → warn (never a silent skip). Distinguish the causes so
            // the message is accurate (#2137 review): (a) zero targets configured, (b) targets present but
            // unexportable in THIS build (non-statsd targets without the feature compiled), (c) targets present
            // and exportable-in-principle but all failed to initialize (e.g. every statsd endpoint unresolvable).
            if config.observability.enabled && reporter.backend_count() == 0 {
                if config.observability.targets.is_empty() {
                    eprintln!(
                        "cdz-agent-daemon: [observability] enabled but no targets configured \
                         — nothing will be exported."
                    );
                } else if unexportable > 0 {
                    eprintln!(
                        "cdz-agent-daemon: [observability] enabled but no export backend in this build \
                         — nothing will be exported ({unexportable} target(s) need an export feature not \
                         compiled here, e.g. metrics-export-otlp / metrics-export-prometheus)."
                    );
                } else {
                    eprintln!(
                        "cdz-agent-daemon: [observability] enabled but no export backend could be set up \
                         — nothing will be exported (all configured targets failed to initialize)."
                    );
                }
            }
            (
                agent_host.registry().clone(),
                reporter,
                std::time::Duration::from_secs(config.observability.report_interval_secs),
                otlp_forwarders,
                prometheus_servers,
            )
        };

        // Use the SAME lifecycle channel the session executors were wired with (not the internally-minted one)
        // so a session's lifecycle op reaches this loop.
        let host = AsyncAgentHost::with_factory_and_lifecycle(
            agent_host,
            factory,
            lifecycle_tx,
            lifecycle_rx,
        )
        .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
        // Drop our inbox sender so an idle inbox doesn't hold the loop open; the admin socket's AdminChannel
        // is what keeps the loop alive as a control plane.
        drop(host.inbox());
        let admin_channel = host.admin_channel();

        let (loop_sd_tx, loop_sd_rx) = tokio::sync::oneshot::channel::<()>();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel::<()>();
        // ctrl-c → shut down the host LOOP (only). The loop's return then drives the socket teardown below,
        // so ctrl-c needs to fire just the loop signal; keeping sock_sd_tx here (not moved into this task)
        // lets the single "loop returned → tear down socket" path handle BOTH ctrl-c and a loop-Err exit.
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("cdz-agent-daemon: ctrl-c received — shutting down");
                let _ = loop_sd_tx.send(());
            }
        });
        // Serve the socket on its OWN task (the loop owns the !Send registry, the socket task is the Send
        // producer feeding it). Then drive the host loop as the primary future — CRUCIALLY, when the loop
        // RETURNS (clean shutdown OR a kernel Err), tear the socket task down explicitly. A prior version
        // `join!`ed both, which DEADLOCKED on a loop-Err: the socket only exits when sock_sd_tx fires (ctrl-c
        // only), so a loop error left join! blocked on the socket forever, swallowing the error (#1977
        // review). Now the loop's return drives socket teardown, and the loop's Err reaches the exit code.
        let socket_task = tokio::spawn(socket.serve(admin_channel, sock_sd_rx));
        // Spawn one async OTLP forwarder per OTLP target. Each drains its bounded channel + POSTs off the
        // report path (the sink's send only enqueues). A forwarder self-terminates when its channel closes —
        // which happens when the reporter (owning the sinks) is dropped as run_export_loop returns at
        // shutdown. Detached: they hold only a Receiver, so we don't need to join them (best-effort telemetry).
        #[cfg(feature = "metrics-export-otlp")]
        for (endpoint, rx) in otlp_forwarders {
            eprintln!("cdz-agent-daemon: metrics export → OTLP forwarder for {endpoint}");
            tokio::spawn(cdz_agent_host::run_otlp_forwarder(endpoint, rx));
        }
        // Bind the non-otlp placeholder so the value is consumed in a build without the OTLP feature.
        #[cfg(all(feature = "metrics-export", not(feature = "metrics-export-otlp")))]
        let _ = otlp_forwarders;
        // Spawn one prometheus scrape server per prometheus target. Each binds its configured address (a
        // loopback bind is recommended; a non-loopback bind is warned by the server) and serves GET /metrics
        // from its handle until shut down. Collect their shutdown senders so they're torn down after the host
        // loop returns (before the socket), like the export task.
        #[cfg(feature = "metrics-export-prometheus")]
        let mut prometheus_shutdowns: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();
        #[cfg(feature = "metrics-export-prometheus")]
        for (bind, handle) in prometheus_servers {
            eprintln!("cdz-agent-daemon: metrics export → prometheus scrape server on {bind}");
            let (sd_tx, sd_rx) = tokio::sync::oneshot::channel::<()>();
            prometheus_shutdowns.push(sd_tx);
            tokio::spawn(async move {
                if let Err(e) =
                    cdz_agent_host::run_prometheus_scrape_server(bind, handle, sd_rx).await
                {
                    eprintln!("cdz-agent-daemon: prometheus scrape server failed to start: {e}");
                }
            });
        }
        // Consume the placeholder in a build without the prometheus feature.
        #[cfg(all(feature = "metrics-export", not(feature = "metrics-export-prometheus")))]
        let _ = prometheus_servers;
        // Spawn the periodic metrics-export task (if built + at least one backend was registered). It reads
        // the cloned registry the host loop increments; its own shutdown oneshot fires after the loop returns
        // so it does a final report on the way out. Skipped (no task) when no backends registered.
        #[cfg(feature = "metrics-export")]
        let (export_task, export_sd_tx) = if export_reporter.backend_count() == 0 {
            (None, None)
        } else {
            eprintln!(
                "cdz-agent-daemon: metrics export → {} backend(s) every {}s",
                export_reporter.backend_count(),
                export_interval.as_secs()
            );
            let (export_sd_tx, export_sd_rx) = tokio::sync::oneshot::channel::<()>();
            let task = tokio::spawn(cdz_agent_host::run_export_loop(
                export_registry,
                export_reporter,
                export_interval,
                export_sd_rx,
            ));
            (Some(task), Some(export_sd_tx))
        };
        let loop_result = host.run_with_wall_clock(loop_sd_rx).await;
        // Tear the export task down (final-flush-and-return) before the socket, so its last metrics ship.
        #[cfg(feature = "metrics-export")]
        if let (Some(task), Some(tx)) = (export_task, export_sd_tx) {
            let _ = tx.send(());
            if let Err(e) = task.await {
                eprintln!("cdz-agent-daemon: metrics export task failed: {e:?}");
            }
        }
        // Shut down the prometheus scrape servers (best-effort; they're detached, we just fire their signals).
        #[cfg(feature = "metrics-export-prometheus")]
        for tx in prometheus_shutdowns {
            let _ = tx.send(());
        }
        // The loop is done — tear down the socket server and await its task so the socket file's Drop
        // cleanup runs before we exit. A JoinError here means the socket server task PANICKED; the exit code
        // is still driven by loop_result (correct — the loop is the primary), but log the panic so a dead
        // socket side isn't silently swallowed (#1984 review).
        let _ = sock_sd_tx.send(());
        if let Err(e) = socket_task.await {
            eprintln!("cdz-agent-daemon: admin socket task failed: {e:?}");
        }
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
