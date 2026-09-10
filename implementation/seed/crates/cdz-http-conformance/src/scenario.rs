//! The scenario assembly (`DESIGN-http-outpost-conformance-harness.md` §3.2) — the end-to-end run-loop that
//! ties the driver's pieces together against REAL SUT processes (operator: nothing internal mocked, a stock
//! gateway tested end to end). [`run_scenario`] brings up the three servers on loopback, seeds + configures
//! them per the run-spec's `config`, drives the `http` steps, and returns the verdict.
//!
//! Order (fail fast, no leaked servers): resolve + hash every program the config names FIRST (a bad manifest
//! fails before anything is spawned); then CAS (writes on) → seed each program by hash → mock (shipping the
//! CAS url) → inject each program's `ProgramHash` + the root router over the admin channel → the stock
//! gateway (dialing the mock's control ws, booting from the `ControlConfig`) → drive the steps. The three
//! [`ServerProcess`](crate::process::ServerProcess) handles live until the function returns; dropping them
//! kills the processes.
//!
//! This is process-orchestration glue: its correctness is proven by the nix harness rig running real
//! scenarios (`http-conformance-<name>` checks), not by in-process `#[test]`s (per the e2e-to-conformance
//! directive). The one thing unit-testable without the bins — failing fast on an unresolvable program — is
//! covered below.

use crate::AdminClient;
use crate::cas::CasClient;
use crate::gateway::{GatewayClient, GatewayResponse};
use crate::programs::{ProgramManifest, ResolvedProgram};
use crate::run::{HarnessBins, run_steps, verdict};
use crate::servers::{spawn_cas, spawn_gateway, spawn_mock};
use crate::spec::{HttpRequest, RunSpec};
use cdz_http_control_mock::admin::{AdminCommand, AdminReply};
use cdz_str::Str;
use std::path::Path;
use std::time::Duration;

/// The throwaway write credential the harness gives its CAS so the driver can seed over the HTTP write path
/// (loopback only; the CAS is fresh per run).
const SEED_CREDENTIAL: &str = "harness-seed";
/// Bind every SUT to an ephemeral loopback port; each reports its real bound address on its ready line.
const LOOPBACK: &str = "127.0.0.1:0";

/// Run one conformance scenario end to end against real SUT processes and return its verdict.
///
/// `bins` are the three SUT binaries (from the nix rig); `spec` is the parsed run-spec; `programs` maps the
/// spec's program names to compiled `.wasm` paths.
///
/// # Errors
/// A program named in the config is unresolvable; a SUT fails to spawn / report ready; a blob fails to seed;
/// an admin injection is rejected by the mock; or the run's steps do not all pass (the [`verdict`]).
pub async fn run_scenario(
    bins: &HarnessBins,
    spec: &RunSpec,
    programs: &ProgramManifest,
    component_store: &Path,
) -> Result<(), String> {
    // 1. Resolve + hash every program the config names, BEFORE spawning anything — a bad manifest fails fast
    //    without leaving a server process running. Each carries the name to register it under.
    let resolved: Vec<(&str, ResolvedProgram)> = spec
        .config
        .programs
        .iter()
        .map(|p| Ok((p.name.as_str(), programs.resolve(&p.program)?)))
        .collect::<Result<_, String>>()?;

    // 2. CAS with writes enabled so the driver can publish by hash; a client bearing the seed credential.
    let cas = spawn_cas(&bins.cas, &cas_config(LOOPBACK, SEED_CREDENTIAL)).await?;
    let cas_url = format!("http://{}", cas.addr);
    let cas_client = CasClient::new(cas.addr).with_write_credential(SEED_CREDENTIAL);
    // 2a. Seed the dependency-closure component store (the value-heap runtime + nfc, each keyed by content
    //     hash) FIRST. A Cadenza guest is a component that IMPORTS the runtime, so the gateway's spawn
    //     (fetch + bind_dependencies + compose from the CAS) needs those dep components present — seeding the
    //     guest program alone leaves spawn unable to resolve the runtime (→ None → 502). Mirrors a real deploy.
    seed_component_store(&cas_client, component_store).await?;
    for (_, program) in &resolved {
        cas_client
            .put(&program.hash_text, program.bytes.clone())
            .await?;
    }

    // 3. Mock control server, shipping the seeded CAS's url to the gateway on connect.
    let mock = spawn_mock(&bins.mock, LOOPBACK, LOOPBACK, &cas_url, None).await?;
    let admin = AdminClient::new(mock.admin_addr);

    // 4. Inject: register each program name → its ProgramHash, then set the root router (its name resolves to
    //    the hash the mock ships in ControlConfig).
    for (name, program) in &resolved {
        expect_ok(
            admin
                .send(&AdminCommand::SetProgram {
                    name: Str::from(*name),
                    hash: program.hash_bytes.clone(),
                })
                .await?,
            "SetProgram",
        )?;
    }
    expect_ok(
        admin
            .send(&AdminCommand::SetRootRouter {
                program: Str::from(spec.config.root_router.as_str()),
            })
            .await?,
        "SetRootRouter",
    )?;

    // 5. The stock gateway, dialing the mock's control ws — it boots from the ControlConfig control ships.
    let gateway = spawn_gateway(&bins.gateway, LOOPBACK, &mock.control_addr.to_string()).await?;
    let gateway_client = GatewayClient::new(gateway.listen_addr);

    // 5a. WAIT UNTIL SETTLED. The gateway binds its socket + prints `listen=…` BEFORE control configures it
    //     (by design — it serves an explicit `503 waiting for control` in that gap, not a misleading 200), so
    //     the ready line proves the socket is up, NOT that a config is applied. A scenario's first request can
    //     race that gap. Poll until the gateway has SETTLED (no longer serving the unconfigured floor) so every
    //     scenario is race-free without a per-step retry — the operator: a settle gate, not a flaky sleep.
    wait_until_configured(&gateway_client).await?;

    // 6. Drive the run's http steps + judge. `cas`/`mock`/`gateway` are held alive across this (they drop —
    //    and their processes die — only when this function returns).
    let outcomes = run_steps(&gateway_client, &admin, &spec.requests).await;
    verdict(&outcomes)
}

/// How long to wait for a freshly-spawned gateway to APPLY its initial control config before driving steps,
/// and the poll interval. Generous: control-config application (dial the control ws → receive `ControlConfig`
/// → fetch + compose the root router from the CAS → atomically swap in the drive context) happens after the
/// `listen=…` line under concurrent load.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(15);
const CONFIGURE_INTERVAL: Duration = Duration::from_millis(50);

/// Poll the gateway until it has SETTLED — control has applied a config and it no longer serves the
/// unconfigured `503 waiting for control` floor (see [`is_unconfigured`]). Returns once the first non-floor
/// response arrives (the gateway is driving a real root router), or errors if it never settles within
/// [`CONFIGURE_TIMEOUT`] (a genuinely stuck gateway fails clearly, not as a mystery first-step 503).
///
/// A discarded probe `GET /` is invisible to the scenario: the gateway drives a FRESH mailbox per request over
/// a stateless fold, so the probe leaves no state the subsequent steps observe.
///
/// # Errors
/// The gateway keeps serving the unconfigured floor (or stays unreachable) past [`CONFIGURE_TIMEOUT`].
async fn wait_until_configured(gateway: &GatewayClient) -> Result<(), String> {
    let probe = HttpRequest {
        method: "GET".into(),
        path: "/".into(),
        ..Default::default()
    };
    let deadline = tokio::time::Instant::now() + CONFIGURE_TIMEOUT;
    loop {
        // A settled gateway answers the probe with anything but the unconfigured floor; a transport error /
        // the floor itself means "still booting" — keep polling until the deadline.
        if let Ok(resp) = gateway.send(&probe).await
            && !is_unconfigured(&resp)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "gateway never settled: still serving the unconfigured `503 waiting for control` floor after \
                 {CONFIGURE_TIMEOUT:?} (control did not apply a config)"
            ));
        }
        tokio::time::sleep(CONFIGURE_INTERVAL).await;
    }
}

/// Whether a response is the gateway's UNCONFIGURED FLOOR — a `503` with the specific `waiting for control`
/// body the gateway serves before control configures it (boot.rs). This is the unambiguous "not yet settled"
/// signal: a `503` a *configured* router itself returns carries a different body, so keying on this exact
/// floor (status + body) won't mistake a real post-config `503` for the boot gap.
fn is_unconfigured(resp: &GatewayResponse) -> bool {
    resp.status == 503 && String::from_utf8_lossy(&resp.body).contains("waiting for control")
}

/// Build the CAS's binary-AST `ServerConfig` document: bind `listen` (`127.0.0.1:0` for an ephemeral port)
/// and enable the write path with `write_credential` (so the driver can seed by hash). No store tier ⇒ the
/// CAS defaults to a single in-memory store (fresh per run). A `#record` with `cdz_cas_http::config`'s field
/// names; the CAS decodes it ascription-tolerantly, so the value toolkit's root ascription is fine.
fn cas_config(listen: &str, write_credential: &str) -> bytes::Bytes {
    use cdz_http_protocol::value;
    let mut b = value::ValueBuilder::new();
    let listen = value::str_leaf(&mut b, listen);
    let write = value::str_leaf(&mut b, write_credential);
    let rec = value::record(
        &mut b,
        vec![("listen", listen), ("write-credential", write)],
    );
    value::finish(b, rec, "ServerConfig")
}

/// Seed every component in the content-addressed store `dir` (a dir of `<base62-hash>.wasm` — the value-heap
/// runtime + its nfc dependency) into the CAS, keyed by its filename hash. The CAS validates the body's digest
/// against the key on PUT, so a mismatched filename would be rejected.
///
/// # Errors
/// The store dir cannot be read, a component file cannot be read, or a PUT is rejected.
async fn seed_component_store(cas: &CasClient, dir: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("reading component store {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("component store entry: {e}"))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        let hash = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("component store: bad filename {}", path.display()))?;
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("reading component {}: {e}", path.display()))?;
        cas.put(hash, bytes::Bytes::from(bytes)).await?;
    }
    Ok(())
}

/// `Ok(())` when the admin reply is `Ok`, else an error naming the command + the mock's message.
fn expect_ok(reply: AdminReply, command: &str) -> Result<(), String> {
    match reply {
        AdminReply::Ok => Ok(()),
        AdminReply::Error { message } => Err(format!("{command} rejected by the mock: {message}")),
        other => Err(format!("{command}: unexpected admin reply {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Config, Program, RunSpec};
    use bytes::Bytes;
    use std::path::PathBuf;

    #[test]
    fn is_unconfigured_matches_only_the_boot_floor() {
        // The exact floor the gateway serves before control configures it → "not yet settled".
        let floor = GatewayResponse {
            status: 503,
            body: Bytes::from_static(
                b"cdz-http-gateway: waiting for control (no program configured yet)\n",
            ),
        };
        assert!(is_unconfigured(&floor), "the boot floor is unconfigured");
        // A configured router's own 503 (different body) is SETTLED — not the boot floor.
        let router_503 = GatewayResponse {
            status: 503,
            body: Bytes::from_static(b"upstream busy"),
        };
        assert!(
            !is_unconfigured(&router_503),
            "a router 503 is not the floor"
        );
        // Any normal response is settled.
        let ok = GatewayResponse {
            status: 200,
            body: Bytes::from_static(b"hello from a wasm handler"),
        };
        assert!(!is_unconfigured(&ok));
    }

    #[tokio::test]
    async fn an_unresolvable_program_fails_before_spawning_any_server() {
        // The config names a program the manifest doesn't have → run_scenario errors at the resolve step,
        // before it touches the (deliberately bogus) bin paths. So this exercises the fail-fast ordering
        // without needing the real SUT binaries.
        let bins = HarnessBins {
            cas: PathBuf::from("/nonexistent/cas"),
            mock: PathBuf::from("/nonexistent/mock"),
            gateway: PathBuf::from("/nonexistent/gateway"),
        };
        let spec = RunSpec {
            config: Config {
                root_router: "missing".into(),
                programs: vec![Program {
                    name: "missing".into(),
                    program: "missing".into(),
                }],
            },
            requests: vec![],
        };
        let err = run_scenario(
            &bins,
            &spec,
            &ProgramManifest::new(),
            Path::new("/nonexistent/component-store"),
        )
        .await
        .expect_err("unresolvable program must error before spawning");
        assert!(
            err.contains("not in the harness manifest"),
            "expected a manifest-resolution error, got: {err}"
        );
    }
}
