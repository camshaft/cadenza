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
            // DynamoDB-backed durable session log (I2, behind `live-aws-storage`). Loads the ambient AWS
            // config ONCE here (boot path). A build WITHOUT the feature reports a clean "not compiled in"
            // boot error rather than silently degrading to in-memory (config validates regardless of feature).
            #[cfg(feature = "live-aws-storage")]
            LogConfig::Dynamo { table } => Some(Box::new(
                cdz_agent_host::DynamoLogSinkBuilder::new(table.clone()).await,
            )),
            #[cfg(not(feature = "live-aws-storage"))]
            LogConfig::Dynamo { .. } => {
                eprintln!(
                    "cdz-agent-daemon: [log] backend=dynamo requires the `live-aws-storage` feature — \
                     rebuild with `--features live-aws-storage` to use the DynamoDB log."
                );
                return std::process::ExitCode::from(1);
            }
            // memory (default) → in-memory session logs (no durable sink).
            LogConfig::Memory => None,
        };
        // A SECOND log-sink builder kept for BOOT-RECOVERY reads (I4b): the one above is MOVED into the factory
        // (it appends a live session's log), so recovery — which READS a persisted log back via
        // LogSinkBuilder::recover — needs its own owned builder. Config-derived + cheap (a Dynamo builder holds
        // a client, a file builder a path), so building a second is fine on the boot path. `None` for the
        // in-memory backend (nothing persisted → nothing to recover).
        let recovery_log_reader: Option<Box<dyn LogSinkBuilder>> = match &config.log {
            LogConfig::File { dir } => Some(Box::new(FileLogSinkBuilder::new(dir))),
            #[cfg(feature = "live-aws-storage")]
            LogConfig::Dynamo { table } => Some(Box::new(
                cdz_agent_host::DynamoLogSinkBuilder::new(table.clone()).await,
            )),
            #[cfg(not(feature = "live-aws-storage"))]
            LogConfig::Dynamo { .. } => None,
            LogConfig::Memory => None,
        };
        // The durable SESSION REGISTRY (I4b), per [session_registry] config — the index the host loop keeps
        // current (register on install, mark_terminated on terminate) so a restart can boot-recover durably
        // logged sessions WITHOUT an O(all-events) log scan. `dynamo` builds a DynamoSessionRegistry over
        // `table` (loading the ambient AWS config ONCE here, on the boot path, like the Dynamo log sink); a
        // build WITHOUT `live-aws-storage` reports a clean "not compiled in" boot error rather than silently
        // degrading (config validates regardless of feature). `memory` (default) = no durable registry.
        use cdz_agent_host::SessionRegistryConfig;
        #[cfg(feature = "live-aws-storage")]
        let session_registry = match &config.session_registry {
            SessionRegistryConfig::Dynamo { table } => {
                Some(cdz_agent_host::DynamoSessionRegistry::new(table.clone()).await)
            }
            SessionRegistryConfig::Memory => None,
        };
        #[cfg(not(feature = "live-aws-storage"))]
        if let SessionRegistryConfig::Dynamo { .. } = &config.session_registry {
            eprintln!(
                "cdz-agent-daemon: [session_registry] backend=dynamo requires the `live-aws-storage` \
                 feature — rebuild with `--features live-aws-storage` to use the durable session registry."
            );
            return std::process::ExitCode::from(1);
        }
        // Build the AgentHost FIRST so its metrics registry exists — the live executor set's per-effect
        // metrics register into the SAME registry (one registry the exporter reports over: host-boundary +
        // per-effect metrics together).
        //
        // §4c AWS-backends I4a — the canonical NAME STORE durability backend, per [name_store] config:
        // - `memory` (default): NO durability → the SHARE-LESS host (today's behavior, less-invasive — no
        //   canonical store, so a `store/*` folds per-session with no shared directory + no snapshot).
        // - `s3` (behind `live-aws-storage`): a DURABLE canonical store — RESTORED from the fixed S3 snapshot
        //   key on boot + snapshotted on each mutation. A build WITHOUT the feature reports a clean "not
        //   compiled in" boot error (config validates regardless of feature, like [blob]/[log]).
        use cdz_agent_host::NameStoreConfig;
        // Owned by value only — it's moved into the host loop (`AsyncAgentHost::new`) without mutation here;
        // boot-recovery spawns recovered sessions through `host.host_mut()` after the move, not this binding.
        let agent_host = match &config.name_store {
            NameStoreConfig::Memory => AgentHost::new(),
            #[cfg(feature = "live-aws-storage")]
            NameStoreConfig::S3 { bucket, prefix } => {
                let snapshot =
                    cdz_agent_host::S3NameStoreSnapshot::new(bucket.clone(), prefix.clone()).await;
                AgentHost::with_canonical_store_restored(Box::new(snapshot)).await
            }
            #[cfg(not(feature = "live-aws-storage"))]
            NameStoreConfig::S3 { .. } => {
                eprintln!(
                    "cdz-agent-daemon: [name_store] backend=s3 requires the `live-aws-storage` feature — \
                     rebuild with `--features live-aws-storage` to use the durable S3 name store."
                );
                return std::process::ExitCode::from(1);
            }
        };
        eprintln!(
            "cdz-agent-daemon: canonical name store = {:?} (durable={})",
            config.name_store,
            agent_host.canonical_store().is_some()
        );
        // Build the HOST LOOP FIRST (empty — no factory yet), so its own loop collaborators (the inbox, the
        // lifecycle channel, and — behind live-ws — the ws-control channel + node connection registry) EXIST
        // to wire into the executor set below. The factory that assembles those executors is injected AFTER
        // (`set_factory`), which resolves the executor-set construction chicken-egg: a LiveExecutorSet wired
        // with the loop's inbox/ws handles (for Emit / ws/send / ws/dial) can't be built until the host that
        // mints them exists (federation slice 3b-2). The host's `factory` field is read only inside `run`, so
        // a post-construction inject is sound. The admin authorizer is the trusted-local-admin allowlist (the
        // socket is owner-only 0o600, so every caller is the owner).
        let mut host = AsyncAgentHost::new(agent_host)
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
        // Grab the loop's own collaborators to wire into the executor set: the SAME lifecycle channel each
        // session's LifecycleExecutor records on (so its ops reach this loop), the inbox the Emit/userspace
        // executors forward into, and — behind live-ws — the ws-control channel + shared node connection
        // registry the ws/send + ws/dial executors route through.
        let lifecycle_tx = host.lifecycle_channel();
        #[cfg(feature = "live-ws")]
        let ws_inbox = host.inbox();
        #[cfg(feature = "live-ws")]
        let ws_conns = host.ws_conn_registry();
        #[cfg(feature = "live-ws")]
        let ws_control = host.ws_control_channel();
        // Build the LIVE executor transports ONCE, here at startup (this is where the AWS config / IMDS load
        // happens — on the boot path, NOT per install). Each install then CLONES these into a fresh executor
        // set cheaply, so a slow IMDS probe can't stall the host loop mid-run (#1987 review). The per-effect
        // metrics register into the host's registry (shared).
        let executors = match LiveExecutorSet::new(host.host().registry()).await {
            Ok(e) => e.with_lifecycle_channel(lifecycle_tx),
            Err(e) => {
                eprintln!("cdz-agent-daemon: could not build the live executor transports: {e}");
                return std::process::ExitCode::from(1);
            }
        };
        // THE OUTPOST federation (live-ws): wire the ws/send + ws/dial executors over the loop's shared node
        // registry + ws-control channel + inbox, so an installed REDUCER can send frames to a federated conn
        // and dial out to a hub (complementing the daemon's boot-bound transports). The build wires WS_SEND +
        // WS_DIAL from this into every installed session.
        #[cfg(feature = "live-ws")]
        let executors = executors.with_ws(ws_conns, ws_control, ws_inbox);
        // I3: when the reducer blob store is S3, wire the SAME bucket into the blob/put+blob/get executor (one
        // content-addressed store for docs + reducer components — corpus-bugfix PM ruling). A per-session
        // S3BlobStore over this bucket is built in LiveExecutorSet::build. Only the S3 blob backend gets a blob
        // executor; memory/dir blob backends serve no blob/* effects (the doc-publish path is an S3 deployment
        // concern). Gated on `live-aws-storage` (the S3 store's feature).
        #[cfg(feature = "live-aws-storage")]
        let executors = if let BlobConfig::S3 { bucket, prefix } = &config.blob {
            executors.with_blob_store(bucket.clone(), prefix.clone())
        } else {
            executors
        };
        // Wrap the configured backing store in the multi-tier CACHE ([blob_cache], operator directive):
        // in-memory S3-FIFO ([`CachingBlobStore`]) OVER an on-disk tier ([`DiskCacheTier`]) OVER the backing
        // store — mem over disk over S3. Content-addressed blobs are immutable, so hits are always correct.
        // Both tiers are budget-0-disable pass-throughs, so wrapping unconditionally is safe; the disk tier is
        // ACTIVE only when [blob_cache].disk_bytes > 0 AND disk_dir is set (a disk cache needs an explicit
        // path). Applied per-arm since each blob backend is a distinct concrete store type (the factory is
        // generic over it). `wrap_cache` composes both tiers onto any backing store uniformly.
        use cdz_agent_host::{CachingBlobStore, DiskCacheTier};
        let cache_mem_bytes = config.blob_cache.mem_bytes;
        // Resolve the disk tier: active (dir, bytes) only when BOTH a dir and a positive budget are set; else
        // a disabled (empty-dir, 0-budget) pass-through that touches no filesystem.
        let (disk_dir, disk_bytes): (std::path::PathBuf, u64) =
            match (&config.blob_cache.disk_dir, config.blob_cache.disk_bytes) {
                (Some(dir), bytes) if bytes > 0 => (dir.into(), bytes as u64),
                _ => (std::path::PathBuf::new(), 0),
            };
        // Compose both cache tiers onto ANY backing store: mem S3-FIFO over disk over the store. A closure
        // would monomorphize to one store type; a generic fn wraps each distinct blob-backend type uniformly.
        fn wrap_cache<B: cdz_kernel::blob::BlobStore>(
            store: B,
            disk_dir: std::path::PathBuf,
            disk_bytes: u64,
            mem_bytes: usize,
        ) -> CachingBlobStore<DiskCacheTier<B>> {
            CachingBlobStore::new(DiskCacheTier::new(store, disk_dir, disk_bytes), mem_bytes)
        }
        // Attach the optional log-sink builder to whichever blob-backed factory we build, then box it. (The
        // two arms are distinct concrete factory types, so the with_log_sink + Box happens per-arm; the
        // executor set is moved into the one arm that runs.)
        #[cfg_attr(not(feature = "live-aws-storage"), allow(unused_mut))]
        let mut factory: Box<dyn SessionFactory> = match &config.blob {
            BlobConfig::Memory => {
                let store = wrap_cache(
                    cdz_kernel::blob::MemBlobStore::new(),
                    disk_dir.clone(),
                    disk_bytes,
                    cache_mem_bytes,
                );
                let mut f = ComponentSessionFactory::new(store, executors, session_authz);
                if let Some(b) = log_sink {
                    f = f.with_log_sink(b);
                }
                Box::new(f)
            }
            BlobConfig::Dir { dir } => match cdz_kernel::blob::DiskBlobStore::open(dir) {
                Ok(store) => {
                    let store = wrap_cache(store, disk_dir.clone(), disk_bytes, cache_mem_bytes);
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
                let store = wrap_cache(store, disk_dir.clone(), disk_bytes, cache_mem_bytes);
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
            "cdz-agent-daemon: reducer blob store = {:?}, session log = {:?}, blob cache = {} MiB in-memory \
             S3-FIFO over {} on-disk (disk tier {}; 0 = tier disabled)",
            config.blob,
            config.log,
            cache_mem_bytes / (1024 * 1024),
            if disk_bytes > 0 {
                format!("{} MiB at {}", disk_bytes / (1024 * 1024), disk_dir.display())
            } else {
                "0 MiB".to_string()
            },
            if disk_bytes > 0 { "active" } else { "off" }
        );

        // §lifecycle I4b BOOT-RECOVERY (the culmination): after the canonical name store is restored (above)
        // and BEFORE the host loop runs, resurrect the sessions that were live at the last shutdown/crash.
        // Runs ONLY when a durable SESSION REGISTRY (the recoverable-session index) AND a durable LOG (what
        // recovery replays) are BOTH configured — a memory registry or memory log has nothing to recover, so
        // the daemon stays lossy-on-restart (today's behavior) with zero overhead. All the seams are on trunk:
        // the registry enumerates (list_all), the log reader reads a session's persisted log back
        // (LogSinkBuilder::recover), and the factory folds+rebuilds it (recover_and_build → the HostedSession +
        // a RecoveryReport). A session whose log ends Terminated is SKIPPED (not resurrected); a Corrupt
        // recovery is alarmed-and-skipped (that one session, not the rest); open effects are reported for
        // re-drive. Best-effort per session: one session's recovery failure is logged, never aborts the boot.
        #[cfg(feature = "live-aws-storage")]
        if let (Some(registry), Some(log_reader)) =
            (session_registry.as_ref(), recovery_log_reader.as_ref())
        {
            use cdz_agent_host::SessionId;
            use cdz_kernel::hash::Hash;
            match registry.list_all().await {
                Ok(records) => {
                    let mut recovered_count = 0usize;
                    for record in &records {
                        // The registry's status already marks terminated sessions; skip them without even
                        // reading the log (the cheap index-level skip the registry exists for).
                        if record.status == cdz_agent_host::SessionStatus::Terminated {
                            continue;
                        }
                        // The registry stores the session id as its canonical genesis-hash hex. Parse it back
                        // to the `Hash` id; a non-hex entry is registry corruption — log + skip that session
                        // (fail-closed), don't abort the whole boot.
                        let Some(id) = Hash::from_hex(&record.session_id).map(SessionId::new)
                        else {
                            tracing::error!(
                                target: "cdz_agent_host::recovery",
                                session_id = record.session_id.as_str(),
                                "boot-recovery: registry entry's session id is not canonical genesis-hash hex — skipping it"
                            );
                            continue;
                        };
                        // Read this session's persisted log back into a `Recovered`. `Ok(None)` = no durable
                        // log for it (nothing to recover — skip); `Err` = a read failure (log + skip, don't
                        // abort the whole boot for one session).
                        let recovered = match log_reader.recover(&id).await {
                            Ok(Some(r)) => r,
                            Ok(None) => continue,
                            Err(e) => {
                                tracing::error!(
                                    target: "cdz_agent_host::recovery",
                                    session_id = record.session_id.as_str(),
                                    error = %e,
                                    "boot-recovery: could not read a session's durable log — skipping it"
                                );
                                continue;
                            }
                        };
                        // Fold the log through the factory (reloads the reducer + recover_from). On failure
                        // (absent reducer bytes / a replay error), log + skip that session.
                        let (hosted, report) = match factory.recover_and_build(recovered).await {
                            Ok(pair) => pair,
                            Err(e) => {
                                tracing::error!(
                                    target: "cdz_agent_host::recovery",
                                    session_id = record.session_id.as_str(),
                                    error = %e,
                                    "boot-recovery: could not rebuild a session — skipping it"
                                );
                                continue;
                            }
                        };
                        // A log that ends in the Terminated marker is NOT resurrected (defense-in-depth over
                        // the registry status check above — the durable log tail is the source of truth).
                        if hosted.is_terminated() {
                            continue;
                        }
                        // A genuinely CORRUPT recovery is an alarm: recover the good prefix but flag it loud
                        // and skip THIS session (don't register a half-broken one), recovering the rest.
                        if report.is_corrupt() {
                            tracing::error!(
                                target: "cdz_agent_host::recovery",
                                session_id = record.session_id.as_str(),
                                "boot-recovery: a session's log recovered CORRUPT — skipping it (the other \
                                 sessions still recover)"
                            );
                            continue;
                        }
                        // The open effects the kernel reports are dispatched-but-unsettled at crash; a full
                        // re-drive (re-perform by idempotency key) is a following slice — for now they're
                        // surfaced so a deployment can see them (never silently dropped).
                        if !report.open_effects.is_empty() {
                            tracing::warn!(
                                target: "cdz_agent_host::recovery",
                                session_id = record.session_id.as_str(),
                                open_effects = report.open_effects.len(),
                                "boot-recovery: recovered a session with open effects (re-drive is a \
                                 following slice; they are reported, not dropped)"
                            );
                        }
                        host.host_mut().spawn(id, hosted);
                        recovered_count += 1;
                    }
                    eprintln!(
                        "cdz-agent-daemon: boot-recovery recovered {recovered_count} session(s) from {} \
                         registry record(s)",
                        records.len()
                    );
                }
                Err(e) => {
                    // The registry enumeration itself failed — log it, boot with an empty registry rather
                    // than aborting (a control-plane daemon can still serve fresh installs).
                    tracing::error!(
                        target: "cdz_agent_host::recovery",
                        error = %e,
                        "boot-recovery: could not enumerate the session registry — booting without recovery"
                    );
                }
            }
        }
        // Bind the recovery-read builder so it is consumed in every build (a memory-backend / no-feature build
        // never reads it). The recovery loop above borrowed it; drop it explicitly now.
        drop(recovery_log_reader);

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
            cloudwatch_forwarders,
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
            // CloudWatch targets are exportable with the metrics-export-cloudwatch feature, unexportable
            // without it (counted toward the diagnostic below). Same collect-then-push shape as otlp.
            #[cfg(feature = "metrics-export-cloudwatch")]
            let cloudwatch_targets: Vec<cdz_agent_host::CloudWatchTarget> =
                if config.observability.enabled {
                    config
                        .observability
                        .targets
                        .iter()
                        .filter_map(|t| match t {
                            MetricsTarget::CloudWatch { namespace } => {
                                Some(cdz_agent_host::CloudWatchTarget {
                                    namespace: namespace.clone(),
                                })
                            }
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
                        || (matches!(t, MetricsTarget::CloudWatch { .. })
                            && !cfg!(feature = "metrics-export-cloudwatch"))
                        || (matches!(t, MetricsTarget::Prometheus { .. })
                            && !cfg!(feature = "metrics-export-prometheus"))
                })
                .count();
            // `mut` only when a push backend (otlp/prometheus) registers into the reporter; a statsd-only build
            // never mutates it after construction.
            #[cfg_attr(
                not(any(
                    feature = "metrics-export-otlp",
                    feature = "metrics-export-cloudwatch",
                    feature = "metrics-export-prometheus"
                )),
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
            // Push each CloudWatch target into the SAME reporter (one drain fans out to statsd + otlp +
            // cloudwatch + prometheus together) and keep its (namespace, receiver) so we can spawn a forwarder
            // task per target below.
            #[cfg(feature = "metrics-export-cloudwatch")]
            let cloudwatch_forwarders: Vec<(
                String,
                tokio::sync::mpsc::Receiver<Vec<cdz_agent_host::CloudWatchDatum>>,
            )> = cloudwatch_targets
                .iter()
                .map(|t| (t.namespace.clone(), reporter.push_cloudwatch(t)))
                .collect();
            #[cfg(not(feature = "metrics-export-cloudwatch"))]
            let cloudwatch_forwarders: Vec<((), ())> = Vec::new();
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
                host.host().registry().clone(),
                reporter,
                std::time::Duration::from_secs(config.observability.report_interval_secs),
                otlp_forwarders,
                cloudwatch_forwarders,
                prometheus_servers,
            )
        };

        // Inject the fully-assembled factory into the host built above (federation slice 3b-2): the executor
        // set it carries was wired with THIS loop's inbox + ws handles, so an installed session's Emit /
        // ws/send / ws/dial reach this loop. `set_factory` is the seam that lets the host be built BEFORE the
        // factory (so its handles could be grabbed) yet still install sessions at runtime.
        host.set_factory(factory);
        // Wire the durable session registry (I4b) when [session_registry] backend=dynamo — the loop then keeps
        // it current (register on install, mark_terminated on terminate) for boot-recovery. `memory` leaves the
        // loop with no external index (lossy-on-restart), so nothing is chained.
        #[cfg(feature = "live-aws-storage")]
        let host = match session_registry {
            Some(registry) => host.with_session_registry(registry),
            None => host,
        };
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
        // THE OUTPOST federation transports (live-ws): bind an inbound peer WsListener from
        // [federation].listen and/or boot-dial each [federation].hubs URL, addressing every transport's
        // inbound ws/connect+frame+disconnect events to the configured outpost_session (the reducer that makes
        // all federation policy — host=plumbing). BIND-IF-CONFIGURED per the concierge ruling: transports come
        // up ONLY when federation is enabled AND an outpost_session is set (config validation already enforces
        // that pairing), so a config-less daemon binds nothing and a later admin-driven install can bind on the
        // same path. Each transport runs on its OWN `Send` task (it moves a stream + cloned channel handles,
        // never the loop's `!Send` registry — same split as the admin socket), routing Register/frames through
        // the loop's ws-control channel + inbox. They're torn down (via `fed_sd_tx`) when the loop returns.
        #[cfg(feature = "live-ws")]
        let fed_sd_txs: Vec<tokio::sync::oneshot::Sender<()>> = {
            use cdz_agent_host::{dial_hub, mint_conn_id, SessionId, WsListener};
            use cdz_kernel::hash::Hash;
            let mut fed_sd_txs = Vec::new();
            // Bind only when enabled AND an outpost_session is configured. `outpost_session` was validated as
            // canonical genesis-hash hex, so `from_hex` is Some; defend anyway (skip + warn rather than panic).
            let outpost = if config.federation.enabled {
                config
                    .federation
                    .outpost_session
                    .as_deref()
                    .and_then(|hex| Hash::from_hex(hex).map(SessionId::new))
            } else {
                None
            };
            if let Some(outpost) = outpost {
                // The inbound peer LISTENER (federation.listen), if configured.
                if let Some(bind) = config.federation.listen.clone() {
                    match WsListener::bind(&bind).await {
                        Ok(listener) => {
                            eprintln!("cdz-agent-daemon: federation ws listener on {bind}");
                            let (sd_tx, sd_rx) = tokio::sync::oneshot::channel::<()>();
                            fed_sd_txs.push(sd_tx);
                            tokio::spawn(listener.serve(
                                outpost,
                                host.inbox(),
                                host.ws_control_channel(),
                                sd_rx,
                            ));
                        }
                        Err(e) => {
                            eprintln!(
                                "cdz-agent-daemon: could not bind federation ws listener {bind}: {e} \
                                 — continuing without the inbound peer transport"
                            );
                        }
                    }
                }
                // The outbound HUB dials (federation.hubs): one boot-dial per URL. A dial that can't connect
                // surfaces a ws/disconnect to the reducer (dial_hub's own contract) rather than aborting boot.
                for hub in &config.federation.hubs {
                    eprintln!("cdz-agent-daemon: federation boot-dialing hub {hub}");
                    let (sd_tx, sd_rx) = tokio::sync::oneshot::channel::<()>();
                    fed_sd_txs.push(sd_tx);
                    let hub_url = hub.clone();
                    let inbox = host.inbox();
                    let control_tx = host.ws_control_channel();
                    tokio::spawn(async move {
                        if let Err(e) =
                            dial_hub(&hub_url, mint_conn_id(), outpost, inbox, control_tx, sd_rx)
                                .await
                        {
                            // A failed initial dial ends only THIS hub connection (unreachable/non-ws/bad URL);
                            // the daemon + other transports run on. NEVER log peer frame bytes — the io error
                            // is ours (the never-log-guest-strings seam).
                            eprintln!("cdz-agent-daemon: federation hub dial {hub_url} ended: {e}");
                        }
                    });
                }
            }
            fed_sd_txs
        };
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
        // Spawn one async CloudWatch forwarder per CloudWatch target. Each drains its bounded channel + calls
        // PutMetricData off the report path (the backend's report_end only enqueues). A forwarder self-
        // terminates when its channel closes (the reporter owning the senders is dropped as run_export_loop
        // returns at shutdown). Detached, like the OTLP forwarders (best-effort telemetry).
        #[cfg(feature = "metrics-export-cloudwatch")]
        for (namespace, rx) in cloudwatch_forwarders {
            eprintln!(
                "cdz-agent-daemon: metrics export → CloudWatch forwarder for namespace {namespace}"
            );
            tokio::spawn(cdz_agent_host::run_cloudwatch_forwarder(namespace, rx));
        }
        // Consume the placeholder in a build without the CloudWatch feature.
        #[cfg(all(feature = "metrics-export", not(feature = "metrics-export-cloudwatch")))]
        let _ = cloudwatch_forwarders;
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
        // Drive the host loop inside a LocalSet: a reducer's `ws/dial` effect ([`WsDialExecutor`]) spawns its
        // dial task with `tokio::task::spawn_local` (the dial moves `!Send`-adjacent per-connection state on
        // this single-threaded runtime), which is only valid within a LocalSet. The loop + all sessions are
        // already `!Send` (§15b), so running under a LocalSet matches their execution model — the same shape
        // the async_host tests use. The boot-bound transports + socket task are separate `Send` `tokio::spawn`s
        // and run on the runtime independently; only the loop itself needs the LocalSet (federation 3b-2).
        let loop_result = tokio::task::LocalSet::new()
            .run_until(host.run_with_wall_clock(loop_sd_rx))
            .await;
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
        // Shut down the federation ws transports (best-effort): fire each listener/dial's shutdown so it ends
        // its accept/pump loop. They're Send tasks holding only cloned channel handles, so — like the metrics
        // forwarders — we fire the signals and don't join (a lingering pump ends when its shutdown fires or the
        // loop's inbox/control channels close as the returned host drops).
        #[cfg(feature = "live-ws")]
        for tx in fed_sd_txs {
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
