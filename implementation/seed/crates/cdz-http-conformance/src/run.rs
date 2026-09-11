//! The run-loop's scenario-execution core (`DESIGN-http-outpost-conformance-harness.md` §3.2). Given a live
//! gateway, drive a [`RunSpec`](crate::spec::RunSpec)'s steps and judge them:
//!
//! - [`HarnessBins`] resolves the three SUT bin paths from the environment the nix harness rig sets (the
//!   driver spawns REAL processes — operator: nothing internal mocked).
//! - [`run_steps`] executes each step (an `http` request at the gateway, or a `control` injection at the mock)
//!   and records a per-step [`StepOutcome`].
//! - [`verdict`] folds the outcomes into a pass/fail with a diagnostic naming every failing step.
//!
//! The process orchestration + CAS seeding that stand up the gateway before this runs are the final assembly
//! (they need the nix rig's bin paths + compiled `programs/`); the step-driving + judgement here is the part
//! that is pure given a gateway address, so it is unit-tested against an in-process HTTP stub.

use crate::AdminClient;
use crate::gateway::GatewayClient;
use crate::spec::{ControlStep, Step};
use bytes::Bytes;
use cdz_http_control_mock::admin::{AdminCommand, AdminReply};
use cdz_str::Str;
use std::path::PathBuf;

/// The three SUT bin paths, resolved from the environment the nix harness rig sets before invoking the
/// driver. Each is the absolute path to a built server binary the driver spawns.
#[derive(Debug, Clone)]
pub struct HarnessBins {
    /// `cdz-cas-http` — the content-addressed store.
    pub cas: PathBuf,
    /// `cdz-http-control-mock` — the mock control server.
    pub mock: PathBuf,
    /// `cdz-http-gateway` — the stock gateway under test.
    pub gateway: PathBuf,
    /// `cdz-http-compile-request` — the deploy tool that builds the `/compile` route's artifact-list body from
    /// captured `/parse` ast-hashes. OPTIONAL: only the `/compile` scenarios invoke it, so a run that never
    /// builds a compile-request body (the majority) does not require it (a step that needs it errors clearly).
    pub compile_request: Option<PathBuf>,
}

/// The env var naming the CAS bin path.
pub const CAS_BIN_VAR: &str = "CDZ_CAS_HTTP_BIN";
/// The env var naming the mock control server bin path.
pub const MOCK_BIN_VAR: &str = "CDZ_HTTP_CONTROL_MOCK_BIN";
/// The env var naming the gateway bin path.
pub const GATEWAY_BIN_VAR: &str = "CDZ_HTTP_GATEWAY_BIN";
/// The env var naming the `cdz-http-compile-request` deploy-tool bin path (optional; see [`HarnessBins`]).
pub const COMPILE_REQUEST_BIN_VAR: &str = "CDZ_HTTP_COMPILE_REQUEST_BIN";

impl HarnessBins {
    /// Resolve the bin paths from [`CAS_BIN_VAR`] / [`MOCK_BIN_VAR`] / [`GATEWAY_BIN_VAR`] (required) and
    /// [`COMPILE_REQUEST_BIN_VAR`] (optional) in the process environment.
    ///
    /// # Errors
    /// Any of the three REQUIRED SUT variables is unset (the run cannot spawn a SUT it cannot locate).
    pub fn resolve_from_env() -> Result<Self, String> {
        Self::resolve(|var| std::env::var_os(var).map(PathBuf::from))
    }

    /// Resolve the bin paths from an arbitrary `lookup` (the pure core of [`resolve_from_env`], so tests need
    /// not mutate the process-global environment).
    ///
    /// # Errors
    /// `lookup` returns `None` for any of the three REQUIRED SUT variables.
    pub fn resolve(lookup: impl Fn(&str) -> Option<PathBuf>) -> Result<Self, String> {
        let req = |var: &str| {
            lookup(var).ok_or_else(|| {
                format!("{var} is not set (the nix harness rig provides the three SUT bin paths)")
            })
        };
        Ok(Self {
            cas: req(CAS_BIN_VAR)?,
            mock: req(MOCK_BIN_VAR)?,
            gateway: req(GATEWAY_BIN_VAR)?,
            compile_request: lookup(COMPILE_REQUEST_BIN_VAR),
        })
    }
}

/// The outcome of one executed step: its 1-based `index`, a short human `description` (e.g. `GET /`), and the
/// `result` — `Ok(())` if it passed its inline `Expect`, `Err(reason)` on a transport error or an assertion
/// miss (the reason names what diverged).
#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub index: usize,
    pub description: String,
    pub result: Result<(), String>,
}

/// Execute each of a run-spec's steps in order, collecting a per-step [`StepOutcome`]: an `http` step makes a
/// request at the `gateway` + checks its `Expect`; a `control` step drives the `admin` channel (live root-router
/// swap / push-down). Every step is attempted (a failing step does not abort the run — the full outcome list
/// lets a report show all divergences at once). A transport error, an `Expect` miss, and a rejected control
/// command all land as an `Err` outcome.
pub async fn run_steps(
    gateway: &GatewayClient,
    admin: &AdminClient,
    cas: &crate::cas::CasClient,
    compile_request_bin: Option<&std::path::Path>,
    module_sources_dir: Option<&std::path::Path>,
    seed_captures: std::collections::HashMap<String, bytes::Bytes>,
    steps: &[Step],
) -> Vec<StepOutcome> {
    let mut outcomes = Vec::with_capacity(steps.len());
    // Named captures for a later step's `body_equals_capture` (cross-step comparison), a `compile-request`
    // body-source (a captured /parse ast-hash → a /compile artifact-list body), or a `wit-world` artifact.
    // Pre-seeded with driver-provided artifact hashes (e.g. the seeded `reducer-world` wit-world blob).
    let mut captures: std::collections::HashMap<String, bytes::Bytes> = seed_captures;
    for (i, step) in steps.iter().enumerate() {
        let (description, result) = match step {
            Step::Http { request, expect } => {
                let description = format!("{} {}", request.method, request.path);
                // Resolve the effective request body NOW: a `compile-request` is assembled (post-capture) via the
                // real deploy tool; a `body-source` is read from the staged module-source dir; else the inline
                // body is used as-is. On a build/read error, the step fails with that diagnostic.
                let effective = if let Some(spec) = &request.compile_request {
                    build_compile_request_body(compile_request_bin, spec, &captures).map(|body| {
                        let mut r = request.clone();
                        r.body = Some(body);
                        r
                    })
                } else if let Some(name) = &request.body_source {
                    read_staged_source(module_sources_dir, name).map(|body| {
                        let mut r = request.clone();
                        r.body = Some(body);
                        r
                    })
                } else if let Some(n) = request.body_fill {
                    // Generate a large filler body at send time (e.g. over the gateway's ceiling → 413) without
                    // a huge literal in the run-spec.
                    let mut r = request.clone();
                    r.body = Some(vec![b'a'; n as usize]);
                    Ok(r)
                } else {
                    Ok(request.clone())
                };
                let result = match effective {
                    Err(e) => Err(e),
                    Ok(request) => match run_http_step(gateway, cas, &request, expect).await {
                        Ok(body) => {
                            if let Some(name) = &expect.capture_body_as {
                                captures.insert(name.clone(), body.clone());
                            }
                            match &expect.body_equals_capture {
                                Some(name) => match captures.get(name) {
                                    Some(want) if *want == body => Ok(()),
                                    Some(_) => Err(format!(
                                        "body-equals-capture: response body differs from captured '{name}'"
                                    )),
                                    None => Err(format!(
                                        "body-equals-capture: no earlier step captured '{name}'"
                                    )),
                                },
                                None => Ok(()),
                            }
                        }
                        Err(e) => Err(e),
                    },
                };
                (description, result)
            }
            Step::Control(control) => {
                let (description, command) = control_command(control);
                let result = match admin.send(&command).await {
                    Ok(AdminReply::Ok) => Ok(()),
                    Ok(AdminReply::Error { message }) => {
                        Err(format!("control command rejected by the mock: {message}"))
                    }
                    Ok(other) => Err(format!("control command: unexpected reply {other:?}")),
                    Err(e) => Err(e),
                };
                (description, result)
            }
        };
        outcomes.push(StepOutcome {
            index: i + 1,
            description,
            result,
        });
    }
    outcomes
}

/// How long to keep re-issuing a `retry-until-match` request before giving up, and the pause between tries.
const RETRY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Run one `http` step: make the request + check its `Expect`. When `expect.retry_until_match` is set, RE-ISSUE
/// the request until the assertion holds or [`RETRY_TIMEOUT`] elapses (the non-linear primitive, for async
/// propagation like a live root-router swap the gateway applies only on a later request); otherwise one shot.
async fn run_http_step(
    gateway: &GatewayClient,
    cas: &crate::cas::CasClient,
    request: &crate::spec::HttpRequest,
    expect: &crate::spec::Expect,
) -> Result<bytes::Bytes, String> {
    let attempt = async || {
        let resp = gateway.send(request).await?;
        expect.check(resp.status, &resp.headers, &resp.body)?;
        // `resolves-in-cas`: the response body is a raw 33-byte content hash a handler published via blobs.put;
        // base62-encode it + assert the blob resolves in the CAS (the "publish persisted" round-trip).
        if expect.resolves_in_cas {
            let hash = crate::base62::encode(&resp.body).ok_or_else(|| {
                format!(
                    "resolves-in-cas: response body is not a 33-byte hash ({} bytes)",
                    resp.body.len()
                )
            })?;
            match cas.get(&hash).await {
                Ok(Some(bytes)) if !bytes.is_empty() => {
                    // Optionally assert the resolved blob starts with an expected prefix (e.g. the wasm magic
                    // for a /compile component) — proving it is the RIGHT kind of blob, not merely non-empty.
                    if let Some(prefix) = &expect.cas_body_starts_with
                        && !bytes.starts_with(prefix)
                    {
                        return Err(format!(
                            "resolves-in-cas: blob {hash} does not start with the expected {}-byte prefix \
                             (got first bytes {:02x?})",
                            prefix.len(),
                            &bytes[..bytes.len().min(prefix.len())]
                        ));
                    }
                }
                Ok(Some(_)) => {
                    return Err(format!(
                        "resolves-in-cas: hash {hash} resolved to an EMPTY blob"
                    ));
                }
                Ok(None) => {
                    return Err(format!(
                        "resolves-in-cas: hash {hash} NOT found in the CAS (the handler's publish did not persist)"
                    ));
                }
                Err(e) => return Err(format!("resolves-in-cas: CAS get for {hash}: {e}")),
            }
        }
        Ok(resp.body)
    };
    if !expect.retry_until_match {
        return attempt().await;
    }
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;
    let mut last = attempt().await;
    while last.is_err() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(RETRY_INTERVAL).await;
        last = attempt().await;
    }
    last.map_err(|e| format!("retry-until-match timed out after {RETRY_TIMEOUT:?}: {e}"))
}

/// Build a `/compile` route request body from a [`CompileRequestSpec`] by invoking the REAL
/// `cdz-http-compile-request` deploy tool — the single source of truth for the `CompileRoute` artifact-list
/// value shape (the harness never re-encodes it). Each `asts` entry's raw 33-byte ast-hash is taken from an
/// earlier step's capture and written to a temp file passed as `--ast <name>=@<file>`; `--entry` names the
/// entrypoint module. Returns the encoded body bytes.
///
/// # Errors
/// The compile-request bin path was not provided ([`COMPILE_REQUEST_BIN_VAR`] unset); a named capture is
/// missing; a temp file cannot be written; or the tool exits non-zero (its stderr diagnostic is surfaced).
fn build_compile_request_body(
    bin: Option<&std::path::Path>,
    spec: &crate::spec::CompileRequestSpec,
    captures: &std::collections::HashMap<String, bytes::Bytes>,
) -> Result<Vec<u8>, String> {
    let bin = bin.ok_or_else(|| {
        format!(
            "compile-request: {COMPILE_REQUEST_BIN_VAR} is not set (the nix rig provides the \
             cdz-http-compile-request deploy tool)"
        )
    })?;
    // A unique scratch dir for this build's raw-hash files (the sandbox is ephemeral; cleaned best-effort).
    let dir = std::env::temp_dir().join(format!(
        "cdz-compile-req-{}-{}",
        std::process::id(),
        COMPILE_REQ_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "compile-request: creating scratch dir {}: {e}",
            dir.display()
        )
    })?;

    // Resolve one {name, from_capture} artifact to a `--<flag> name=@<hashfile>` pair: look the raw 33-byte hash
    // up in the captures, write it to a temp file, and append the flag. Used for both `--ast` and `--wit-world`.
    let write_hash_arg = |cmd: &mut std::process::Command,
                          flag: &str,
                          art: &crate::spec::CompileAst|
     -> Result<(), String> {
        let hash = captures.get(&art.from_capture).ok_or_else(|| {
                format!(
                    "compile-request: {flag} {:?} references capture {:?}, but no earlier step captured it",
                    art.name, art.from_capture
                )
            })?;
        let hash_file = dir.join(format!(
            "{}-{}.hash",
            flag.trim_start_matches('-'),
            art.name
        ));
        std::fs::write(&hash_file, hash).map_err(|e| {
            format!(
                "compile-request: writing {flag} hash for {:?} to {}: {e}",
                art.name,
                hash_file.display()
            )
        })?;
        cmd.arg(flag)
            .arg(format!("{}=@{}", art.name, hash_file.display()));
        Ok(())
    };

    let mut cmd = std::process::Command::new(bin);
    for ast in &spec.asts {
        write_hash_arg(&mut cmd, "--ast", ast)?;
    }
    if let Some(ww) = &spec.wit_world {
        write_hash_arg(&mut cmd, "--wit-world", ww)?;
    }
    let out_file = dir.join("body.bin");
    cmd.arg("--entry").arg(&spec.entry).arg("-o").arg(&out_file);

    let output = cmd
        .output()
        .map_err(|e| format!("compile-request: running {}: {e}", bin.display()))?;
    if !output.status.success() {
        return Err(format!(
            "compile-request: {} exited {} — {}",
            bin.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let body = std::fs::read(&out_file).map_err(|e| {
        format!(
            "compile-request: reading built body {}: {e}",
            out_file.display()
        )
    })?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(body)
}

/// A per-process counter making each [`build_compile_request_body`] scratch dir unique.
static COMPILE_REQ_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Read a staged module SOURCE (`<dir>/<name>.cdz`) as an HTTP request body — the `body-source` mechanism, so a
/// scenario can POST a real in-tree module source (a lib/contract of a reducer-world guest's compile closure)
/// to `/parse` without embedding + drifting its text in the run-spec.
///
/// # Errors
/// The staged-sources dir was not provided ([`MODULE_SOURCES_DIR_VAR`] unset), or the `<name>.cdz` file is
/// missing / unreadable.
fn read_staged_source(dir: Option<&std::path::Path>, name: &str) -> Result<Vec<u8>, String> {
    let dir = dir.ok_or_else(|| {
        format!(
            "body-source: {MODULE_SOURCES_DIR_VAR} is not set (the nix rig stages the module sources for this \
             scenario)"
        )
    })?;
    let path = dir.join(format!("{name}.cdz"));
    std::fs::read(&path).map_err(|e| {
        format!(
            "body-source: reading staged module {name:?} at {}: {e}",
            path.display()
        )
    })
}

/// The env var naming the dir of staged module SOURCES (`<name>.cdz`) a scenario's `body-source` reads.
pub const MODULE_SOURCES_DIR_VAR: &str = "CDZ_HARNESS_MODULE_SOURCES_DIR";

/// Map a [`ControlStep`] to its human description + the [`AdminCommand`] that drives it at the mock.
fn control_command(control: &ControlStep) -> (String, AdminCommand) {
    match control {
        ControlStep::PushRootRouter(name) => (
            format!("control push-root-router {name}"),
            AdminCommand::PushRootRouter {
                program: Str::from(name.as_str()),
            },
        ),
        ControlStep::PushDown { session, payload } => (
            format!("control push-down ({} byte payload)", payload.len()),
            AdminCommand::PushDown {
                session: Bytes::copy_from_slice(session.as_deref().unwrap_or(&[])),
                payload: Bytes::copy_from_slice(payload),
            },
        ),
        ControlStep::PrimeReply { match_path, reply } => (
            format!(
                "control prime-reply (match-path {}, {} byte reply)",
                match_path.as_deref().unwrap_or("*"),
                reply.len()
            ),
            AdminCommand::PrimeReply {
                match_path: match_path.as_deref().map(Str::from),
                reply: Bytes::copy_from_slice(reply),
            },
        ),
    }
}

/// The run's verdict: `Ok(())` iff every step passed, else `Err` with a `;`-joined summary of each failing
/// step (its index, description, and reason) — the scenario's failure diagnostic.
///
/// # Errors
/// One or more steps failed their assertion or errored in transport.
pub fn verdict(outcomes: &[StepOutcome]) -> Result<(), String> {
    let failures: Vec<String> = outcomes
        .iter()
        .filter_map(|o| {
            o.result
                .as_ref()
                .err()
                .map(|e| format!("step {} ({}): {e}", o.index, o.description))
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A CAS client the http-only tests never contact (no resolves-in-cas), like the bogus admin.
    fn bogus_cas() -> crate::cas::CasClient {
        crate::cas::CasClient::new("127.0.0.1:1".parse().unwrap())
    }
    use crate::spec::{Expect, HttpRequest, Step};
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A looping HTTP/1.1 stub: for EVERY connection, drain the request headers and reply with a fixed
    /// `status`/`body`. Unlike the one-shot stub in `gateway::tests`, this serves a whole run's worth of
    /// steps (reqwest sends `Connection: close`, so each step is a fresh connection). Runs until dropped.
    async fn looping_http_stub(status_line: &'static str, body: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let resp = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.write_all(body).await;
                    let _ = sock.flush().await;
                });
            }
        });
        addr
    }

    fn http_step(path: &str, expect: Expect) -> Step {
        Step::Http {
            request: HttpRequest {
                method: "GET".into(),
                path: path.into(),
                ..Default::default()
            },
            expect,
        }
    }

    #[tokio::test]
    async fn drives_every_step_and_reports_per_step_outcomes() {
        // The stub answers 200 "ok" to everything; step 1 expects that (pass), step 2 expects 404 (fail).
        let addr = looping_http_stub("200 OK", b"ok").await;
        let gateway = GatewayClient::new(addr);
        // Control steps go to the admin; these are http-only, so a never-contacted bogus admin is fine.
        let admin = AdminClient::new("127.0.0.1:1".parse().unwrap());
        let steps = vec![
            http_step(
                "/",
                Expect {
                    status: Some(200),
                    body: Some(b"ok".to_vec()),
                    ..Expect::default()
                },
            ),
            http_step(
                "/nope",
                Expect {
                    status: Some(404),
                    ..Expect::default()
                },
            ),
        ];
        let outcomes = run_steps(
            &gateway,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &steps,
        )
        .await;
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].result.is_ok(), "step 1 should pass");
        assert!(outcomes[1].result.is_err(), "step 2 should fail");
        assert_eq!(outcomes[1].description, "GET /nope");

        // The verdict fails, naming the failing step (not the passing one).
        let v = verdict(&outcomes).unwrap_err();
        assert!(v.contains("step 2 (GET /nope)"), "got: {v}");
        assert!(!v.contains("step 1"), "passing step should not appear: {v}");
    }

    #[tokio::test]
    async fn an_all_passing_run_has_an_ok_verdict() {
        let addr = looping_http_stub("200 OK", b"hello").await;
        let gateway = GatewayClient::new(addr);
        // Control steps go to the admin; these are http-only, so a never-contacted bogus admin is fine.
        let admin = AdminClient::new("127.0.0.1:1".parse().unwrap());
        let steps = vec![
            http_step(
                "/a",
                Expect {
                    status: Some(200),
                    ..Expect::default()
                },
            ),
            http_step(
                "/b",
                Expect {
                    body_contains: Some("ell".into()),
                    ..Expect::default()
                },
            ),
        ];
        let outcomes = run_steps(
            &gateway,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &steps,
        )
        .await;
        assert!(verdict(&outcomes).is_ok());
    }

    /// A stub that serves `503` for its first `fail_n` connections, then `200` + `body` — to exercise
    /// retry-until-match polling past a transient state (like an async live-swap not yet applied).
    async fn flipping_http_stub(fail_n: usize, body: &'static [u8]) -> SocketAddr {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let n = count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        let r = sock.read(&mut buf).await.unwrap_or(0);
                        if r == 0 || buf[..r].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let (status, b): (&str, &[u8]) = if n < fail_n {
                        ("503 Service Unavailable", b"")
                    } else {
                        ("200 OK", body)
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        b.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.write_all(b).await;
                    let _ = sock.flush().await;
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn retry_until_match_polls_past_a_transient_failure() {
        let admin = AdminClient::new("127.0.0.1:1".parse().unwrap());
        // Retry: the first 2 requests get 503, then 200 "ready" — the retry step polls until it matches.
        let addr = flipping_http_stub(2, b"ready").await;
        let gateway = GatewayClient::new(addr);
        let retry = vec![Step::Http {
            request: HttpRequest {
                method: "GET".into(),
                path: "/".into(),
                ..Default::default()
            },
            expect: Expect {
                status: Some(200),
                body: Some(b"ready".to_vec()),
                retry_until_match: true,
                ..Default::default()
            },
        }];
        let outcomes = run_steps(
            &gateway,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &retry,
        )
        .await;
        assert!(
            outcomes[0].result.is_ok(),
            "retry should poll past the 503s: {:?}",
            outcomes[0].result
        );

        // Without retry, the same first-503 stub fails on the single shot.
        let addr2 = flipping_http_stub(2, b"ready").await;
        let gateway2 = GatewayClient::new(addr2);
        let once = vec![Step::Http {
            request: HttpRequest {
                method: "GET".into(),
                path: "/".into(),
                ..Default::default()
            },
            expect: Expect {
                status: Some(200),
                ..Default::default()
            },
        }];
        let outcomes = run_steps(
            &gateway2,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &once,
        )
        .await;
        assert!(
            outcomes[0].result.is_err(),
            "no retry → the first 503 is reported"
        );
    }

    #[tokio::test]
    async fn a_control_step_drives_the_mock_admin() {
        use bytes::Bytes;
        use cdz_http_control_mock::MockState;
        use cdz_http_control_mock::admin::AdminCommand;
        use cdz_http_control_mock::server::{AdminCtx, serve_admin};
        use cdz_http_control_mock::ws::new_sessions;
        use cdz_str::Str;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        // Boot the mock's admin server in-process (the driver spawns the bin in production).
        let state = Arc::new(Mutex::new(MockState::new(HashMap::new())));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let admin_addr = listener.local_addr().unwrap();
        tokio::spawn(serve_admin(
            listener,
            AdminCtx {
                state,
                sessions: new_sessions(),
                cas_url: Str::from("http://127.0.0.1:9/cas"),
                cas_credential: Bytes::new(),
                codec: cdz_http_protocol::FrameCodec::new(
                    Bytes::from_static(b"c"),
                    Bytes::from_static(b"u"),
                    Bytes::from_static(b"d"),
                ),
            },
        ));
        let admin = AdminClient::new(admin_addr);
        // Register a program so push-root-router resolves.
        admin
            .send(&AdminCommand::SetProgram {
                name: Str::from("router-b"),
                hash: Bytes::from_static(b"a-33-byte-program-hash-goes-here."),
            })
            .await
            .unwrap();

        // A gateway is required by the signature but never contacted (no http steps here).
        let gateway = GatewayClient::new("127.0.0.1:1".parse().unwrap());

        // A push-root-router control step drives the admin + succeeds.
        let outcomes = run_steps(
            &gateway,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &[Step::Control(ControlStep::PushRootRouter(
                "router-b".into(),
            ))],
        )
        .await;
        assert_eq!(outcomes.len(), 1);
        assert!(
            outcomes[0].result.is_ok(),
            "push-root-router should succeed: {:?}",
            outcomes[0].result
        );
        assert!(
            outcomes[0]
                .description
                .contains("push-root-router router-b")
        );

        // Pushing an UNREGISTERED root router surfaces the mock's rejection as an Err outcome.
        let bad = run_steps(
            &gateway,
            &admin,
            &bogus_cas(),
            None,
            None,
            std::collections::HashMap::new(),
            &[Step::Control(ControlStep::PushRootRouter("nope".into()))],
        )
        .await;
        assert!(
            bad[0].result.is_err(),
            "unregistered program must be rejected"
        );
    }

    #[test]
    fn bins_resolve_from_a_lookup_or_error_clearly() {
        use std::collections::HashMap;
        // All three present → resolved (no global-env mutation; the lookup is injected).
        let full: HashMap<&str, &str> = [
            (CAS_BIN_VAR, "/bin/cas"),
            (MOCK_BIN_VAR, "/bin/mock"),
            (GATEWAY_BIN_VAR, "/bin/gw"),
        ]
        .into_iter()
        .collect();
        let bins =
            HarnessBins::resolve(|v| full.get(v).map(|s| PathBuf::from(*s))).expect("all set");
        assert_eq!(bins.cas, PathBuf::from("/bin/cas"));
        assert_eq!(bins.mock, PathBuf::from("/bin/mock"));
        assert_eq!(bins.gateway, PathBuf::from("/bin/gw"));
        // The compile-request bin is OPTIONAL: absent here (not in `full`) → None, and resolve still succeeds.
        assert_eq!(bins.compile_request, None);

        // When present, the optional compile-request bin resolves too.
        let with_cr = HarnessBins::resolve(|v| {
            full.get(v)
                .map(|s| PathBuf::from(*s))
                .or_else(|| (v == COMPILE_REQUEST_BIN_VAR).then(|| PathBuf::from("/bin/cr")))
        })
        .expect("all set");
        assert_eq!(with_cr.compile_request, Some(PathBuf::from("/bin/cr")));

        // A missing REQUIRED var names itself in the error (the optional one never blocks resolution).
        let err =
            HarnessBins::resolve(|v| (v != CAS_BIN_VAR).then(|| PathBuf::from("/x"))).unwrap_err();
        assert!(err.contains(CAS_BIN_VAR), "got: {err}");
    }
}
