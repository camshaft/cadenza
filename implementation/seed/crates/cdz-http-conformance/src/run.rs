//! The run-loop's scenario-execution core (`DESIGN-http-outpost-conformance-harness.md` §3.2). Given a live
//! gateway, drive a [`RunSpec`](crate::spec::RunSpec)'s steps and judge them:
//!
//! - [`HarnessBins`] resolves the three SUT bin paths from the environment the nix harness rig sets (the
//!   driver spawns REAL processes — operator: nothing internal mocked).
//! - [`run_http_steps`] executes each `http` step against the gateway (via [`GatewayClient`]) and records a
//!   per-step [`StepOutcome`] (the request + its `Expect` result).
//! - [`verdict`] folds the outcomes into a pass/fail with a diagnostic naming every failing step.
//!
//! The process orchestration + CAS seeding that stand up the gateway before this runs are the final assembly
//! (they need the nix rig's bin paths + compiled `programs/`); the step-driving + judgement here is the part
//! that is pure given a gateway address, so it is unit-tested against an in-process HTTP stub.

use crate::gateway::GatewayClient;
use crate::spec::Step;
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
}

/// The env var naming the CAS bin path.
pub const CAS_BIN_VAR: &str = "CDZ_CAS_HTTP_BIN";
/// The env var naming the mock control server bin path.
pub const MOCK_BIN_VAR: &str = "CDZ_HTTP_CONTROL_MOCK_BIN";
/// The env var naming the gateway bin path.
pub const GATEWAY_BIN_VAR: &str = "CDZ_HTTP_GATEWAY_BIN";

impl HarnessBins {
    /// Resolve the bin paths from [`CAS_BIN_VAR`] / [`MOCK_BIN_VAR`] / [`GATEWAY_BIN_VAR`] in the process
    /// environment.
    ///
    /// # Errors
    /// Any of the three variables is unset (the run cannot spawn a SUT it cannot locate).
    pub fn resolve_from_env() -> Result<Self, String> {
        Self::resolve(|var| std::env::var_os(var).map(PathBuf::from))
    }

    /// Resolve the bin paths from an arbitrary `lookup` (the pure core of [`resolve_from_env`], so tests need
    /// not mutate the process-global environment).
    ///
    /// # Errors
    /// `lookup` returns `None` for any of the three variables.
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

/// Execute each of a run-spec's `http` steps against the live `gateway`, in order, collecting a per-step
/// [`StepOutcome`]. Every step is attempted (a failing step does not abort the run — the full outcome list
/// lets a report show all divergences at once). A transport error and an `Expect` miss both land as an
/// `Err` outcome; the caller distinguishes them by the message if needed.
pub async fn run_http_steps(gateway: &GatewayClient, steps: &[Step]) -> Vec<StepOutcome> {
    let mut outcomes = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let Step::Http { request, expect } = step;
        let description = format!("{} {}", request.method, request.path);
        let result = match gateway.send(request).await {
            Ok(resp) => expect.check(resp.status, &resp.body),
            Err(e) => Err(e),
        };
        outcomes.push(StepOutcome {
            index: i + 1,
            description,
            result,
        });
    }
    outcomes
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
        let outcomes = run_http_steps(&gateway, &steps).await;
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
        let outcomes = run_http_steps(&gateway, &steps).await;
        assert!(verdict(&outcomes).is_ok());
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

        // A missing var names itself in the error.
        let err =
            HarnessBins::resolve(|v| (v != CAS_BIN_VAR).then(|| PathBuf::from("/x"))).unwrap_err();
        assert!(err.contains(CAS_BIN_VAR), "got: {err}");
    }
}
