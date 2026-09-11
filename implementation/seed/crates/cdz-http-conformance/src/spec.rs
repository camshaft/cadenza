//! The run-spec PARSER (`DESIGN-http-outpost-conformance-harness.md` §3): decode a conformance run's
//! binary-AST value (a `*.ml` Cadenza record, `cdz rewrite`-resolved + encoded) into the [`RunSpec`] the
//! driver executes. Reads the record structure with the shared [`cdz_http_protocol::value`] toolkit — one
//! codec, no JSON. The interpreter (process-orchestration + request execution) sits on top of this.
//!
//! The value shape (see `runs/README.md`):
//!   { config = { root-router = "<name>", programs = [ { name, program }, … ] },
//!     requests = [ { http = { method, path, headers = [ { name, value } ]?, body = b"…"? },
//!                    expect = { status?, body?, body-contains?, headers = [ { name, value } ]? } }
//!                | { control = { push-root-router = "<name>" } }
//!                | { control = { push-down = { session = b"…"?, payload = b"…" } } }, … ] }
//! where an http `expect` may also carry `retry-until-match = true` (poll the request until it matches).
//! (prime-replies are added in a following slice.)

use cdz_http_protocol::value;

/// A whole conformance run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub config: Config,
    pub requests: Vec<Step>,
}

/// The SUT setup: which root router to ship + the programs to make resolvable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The root-router program reference (a manifest name, or a `cdz rewrite`-resolved hash).
    pub root_router: String,
    /// The programs to seed into the CAS (routers + handlers), each `{ name, program }`.
    pub programs: Vec<Program>,
}

/// One program the run makes resolvable: a `name` bound to a `program` reference (name or resolved path/hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub program: String,
}

/// One interaction in a run: an HTTP request at the gateway, or a control-plane injection at the mock control
/// server (a live root-router swap / an unsolicited push — the http-outpost's hot-reconfigure surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Make an HTTP request at the gateway + assert the response.
    Http {
        request: HttpRequest,
        expect: Expect,
    },
    /// Drive the mock control server (over its admin channel), interleaved between HTTP requests.
    Control(ControlStep),
}

/// A control-plane injection at the mock control server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlStep {
    /// Live-swap the root router to an (already-registered) program name (`push-root-router`).
    PushRootRouter(String),
    /// Push an unsolicited `ControlDown` to a session (`push-down`); `None` session ⇒ empty (broadcast).
    PushDown {
        session: Option<Vec<u8>>,
        payload: Vec<u8>,
    },
}

/// An HTTP request to make at the gateway. `headers` are `(name, value)` pairs; `body` is the request body
/// bytes (e.g. a `POST` payload). Both default to empty/none (a bare `GET` needs neither).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// The inline assertion on an HTTP response. A `None` field asserts nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expect {
    /// Assert the exact status code.
    pub status: Option<u16>,
    /// Assert the exact response body bytes.
    pub body: Option<Vec<u8>>,
    /// Assert the response body CONTAINS this substring.
    pub body_contains: Option<String>,
    /// Assert each of these response headers is present: a `(name, value)` must appear (header NAME matched
    /// case-insensitively per HTTP, value exact). Empty ⇒ asserts nothing about headers.
    pub headers: Vec<(String, String)>,
    /// Poll the request, re-issuing it until the assertion holds or a timeout elapses — the one non-linear
    /// primitive, for async propagation (e.g. a live root-router swap the gateway applies on a later request).
    pub retry_until_match: bool,
}

impl RunSpec {
    /// A one-line human summary of a parsed run (root router, program count, request count) — used by the
    /// driver's `--parse-only` mode to confirm a compiled run-spec decoded, without running it.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "root-router={:?}, {} program(s), {} request(s)",
            self.config.root_router,
            self.config.programs.len(),
            self.requests.len(),
        )
    }
}

impl Expect {
    /// Check an HTTP response against this assertion. `Ok(())` if every present field holds; `Err(reason)`
    /// with a human-readable diagnostic on the first field that fails (so a failing scenario names WHAT
    /// diverged). A `None` field asserts nothing, so an empty [`Expect`] always passes.
    ///
    /// # Errors
    /// The status, exact body, `body-contains` substring, or a required response `header` assertion does not
    /// hold.
    pub fn check(
        &self,
        status: u16,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<(), String> {
        if let Some(want) = self.status
            && status != want
        {
            return Err(format!("status: expected {want}, got {status}"));
        }
        if let Some(want) = &self.body
            && body != want.as_slice()
        {
            return Err(format!(
                "body: expected {} bytes ({}), got {} bytes ({})",
                want.len(),
                preview(want),
                body.len(),
                preview(body),
            ));
        }
        if let Some(sub) = &self.body_contains {
            let hay = String::from_utf8_lossy(body);
            if !hay.contains(sub.as_str()) {
                return Err(format!(
                    "body-contains: {sub:?} not found in body ({})",
                    preview(body)
                ));
            }
        }
        for (name, value) in &self.headers {
            // HTTP header names are case-insensitive; the response headers arrive lower-cased.
            let want_name = name.to_ascii_lowercase();
            let found = headers.iter().any(|(n, v)| *n == want_name && v == value);
            if !found {
                return Err(format!(
                    "header: expected {name:?}: {value:?} not found (response headers: {headers:?})"
                ));
            }
        }
        Ok(())
    }
}

/// A short, escaped preview of a body for a failure diagnostic (bodies can be large / binary): the first
/// 64 bytes, lossy-decoded, with a trailing ellipsis when truncated.
fn preview(bytes: &[u8]) -> String {
    const MAX: usize = 64;
    let shown = &bytes[..bytes.len().min(MAX)];
    let text = String::from_utf8_lossy(shown);
    if bytes.len() > MAX {
        format!("{text:?}…")
    } else {
        format!("{text:?}")
    }
}

/// Decode a run-spec's binary-AST bytes into a [`RunSpec`], or `None` if malformed / missing a required field.
#[must_use]
pub fn parse_run_spec(bytes: &[u8]) -> Option<RunSpec> {
    let arenas = value::decode(bytes)?;
    let root = arenas.root;
    let config = parse_config(&arenas, value::record_field(&arenas, root, "config")?)?;
    let requests = value::read_list(&arenas, value::record_field(&arenas, root, "requests")?)?
        .iter()
        .map(|&s| parse_step(&arenas, s))
        .collect::<Option<Vec<_>>>()?;
    Some(RunSpec { config, requests })
}

fn parse_config(arenas: &value::Arenas, id: value::ValueId) -> Option<Config> {
    let root_router = value::read_str(arenas, value::record_field(arenas, id, "root-router")?)?;
    let programs = value::read_list(arenas, value::record_field(arenas, id, "programs")?)?
        .iter()
        .map(|&p| {
            Some(Program {
                name: value::read_str(arenas, value::record_field(arenas, p, "name")?)?,
                program: value::read_str(arenas, value::record_field(arenas, p, "program")?)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Config {
        root_router,
        programs,
    })
}

fn parse_step(arenas: &value::Arenas, id: value::ValueId) -> Option<Step> {
    // A step is either an `http` request (+ optional `expect`) or a `control` injection.
    if let Some(control) = value::record_field(arenas, id, "control") {
        return Some(Step::Control(parse_control(arenas, control)?));
    }
    let http = value::record_field(arenas, id, "http")?;
    let headers = match value::record_field(arenas, http, "headers") {
        Some(hs) => value::read_list(arenas, hs)?
            .iter()
            .map(|&h| {
                Some((
                    value::read_str(arenas, value::record_field(arenas, h, "name")?)?,
                    value::read_str(arenas, value::record_field(arenas, h, "value")?)?,
                ))
            })
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    let body = match value::record_field(arenas, http, "body") {
        Some(b) => Some(value::read_bytes(arenas, b)?.to_vec()),
        None => None,
    };
    let request = HttpRequest {
        method: value::read_str(arenas, value::record_field(arenas, http, "method")?)?,
        path: value::read_str(arenas, value::record_field(arenas, http, "path")?)?,
        headers,
        body,
    };
    let expect = match value::record_field(arenas, id, "expect") {
        Some(e) => parse_expect(arenas, e)?,
        None => Expect::default(),
    };
    Some(Step::Http { request, expect })
}

/// Parse a `control = { … }` injection: `{ push-root-router = "<name>" }` or
/// `{ push-down = { session = b"…"?, payload = b"…" } }`.
fn parse_control(arenas: &value::Arenas, id: value::ValueId) -> Option<ControlStep> {
    if let Some(name) = value::record_field(arenas, id, "push-root-router") {
        return Some(ControlStep::PushRootRouter(value::read_str(arenas, name)?));
    }
    let pd = value::record_field(arenas, id, "push-down")?;
    let session = match value::record_field(arenas, pd, "session") {
        Some(s) => Some(value::read_bytes(arenas, s)?.to_vec()),
        None => None,
    };
    let payload = value::read_bytes(arenas, value::record_field(arenas, pd, "payload")?)?.to_vec();
    Some(ControlStep::PushDown { session, payload })
}

fn parse_expect(arenas: &value::Arenas, id: value::ValueId) -> Option<Expect> {
    // Every field is optional; a present field must parse (a malformed present field → None).
    let status = match value::record_field(arenas, id, "status") {
        Some(s) => Some(u16::try_from(value::read_uint(arenas, s)?).ok()?),
        None => None,
    };
    let body = match value::record_field(arenas, id, "body") {
        Some(b) => Some(value::read_bytes(arenas, b)?.to_vec()),
        None => None,
    };
    let body_contains = match value::record_field(arenas, id, "body-contains") {
        Some(bc) => Some(value::read_str(arenas, bc)?),
        None => None,
    };
    // `headers = [ { name, value }, … ]` — each response header the assertion requires present.
    let headers = match value::record_field(arenas, id, "headers") {
        Some(hs) => value::read_list(arenas, hs)?
            .iter()
            .map(|&h| {
                Some((
                    value::read_str(arenas, value::record_field(arenas, h, "name")?)?,
                    value::read_str(arenas, value::record_field(arenas, h, "value")?)?,
                ))
            })
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    let retry_until_match = match value::record_field(arenas, id, "retry-until-match") {
        Some(r) => value::read_bool(arenas, r)?,
        None => false,
    };
    Some(Expect {
        status,
        body,
        body_contains,
        headers,
        retry_until_match,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_http_protocol::value::{
        ValueBuilder, bool_leaf, bytes_leaf, finish, list_value, record, str_leaf, uint_leaf,
    };

    /// Build the binary-AST for a run-spec matching `runs/route-to-handler.ml`'s first request.
    fn sample_run_spec_bytes() -> bytes::Bytes {
        let mut b = ValueBuilder::new();
        // config = { root-router = "router", programs = [ { name = "router", program = "router" } ] }
        let rr = str_leaf(&mut b, "router");
        let p_name = str_leaf(&mut b, "router");
        let p_prog = str_leaf(&mut b, "router");
        let prog = record(&mut b, vec![("name", p_name), ("program", p_prog)]);
        let programs = list_value(&mut b, vec![prog]);
        let config = record(&mut b, vec![("programs", programs), ("root-router", rr)]);
        // requests = [ { http = { method = "GET", path = "/" },
        //               expect = { status = 200, body = b"hello" } } ]
        let m = str_leaf(&mut b, "GET");
        let path = str_leaf(&mut b, "/");
        let http = record(&mut b, vec![("method", m), ("path", path)]);
        let st = uint_leaf(&mut b, 200);
        let body = bytes_leaf(&mut b, b"hello");
        let expect = record(&mut b, vec![("body", body), ("status", st)]);
        let step = record(&mut b, vec![("expect", expect), ("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        finish(b, root, "RunSpec")
    }

    #[test]
    fn parses_a_config_and_an_http_step_with_expect() {
        let spec = parse_run_spec(&sample_run_spec_bytes()).expect("run-spec parses");
        assert_eq!(spec.config.root_router, "router");
        assert_eq!(spec.config.programs.len(), 1);
        assert_eq!(spec.config.programs[0].name, "router");
        assert_eq!(spec.requests.len(), 1);
        let Step::Http { request, expect } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/");
        assert_eq!(expect.status, Some(200));
        assert_eq!(expect.body.as_deref(), Some(&b"hello"[..]));
        assert_eq!(expect.body_contains, None);
    }

    #[test]
    fn parses_http_request_headers_and_body() {
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        // http = { method="POST", path="/echo", headers=[{name="x-a",value="1"}], body=b"hi" }
        let m = str_leaf(&mut b, "POST");
        let path = str_leaf(&mut b, "/echo");
        let hn = str_leaf(&mut b, "x-a");
        let hv = str_leaf(&mut b, "1");
        let hdr = record(&mut b, vec![("name", hn), ("value", hv)]);
        let headers = list_value(&mut b, vec![hdr]);
        let body = bytes_leaf(&mut b, b"hi");
        let http = record(
            &mut b,
            vec![
                ("body", body),
                ("headers", headers),
                ("method", m),
                ("path", path),
            ],
        );
        let step = record(&mut b, vec![("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        let Step::Http { request, .. } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/echo");
        assert_eq!(request.headers, vec![("x-a".to_string(), "1".to_string())]);
        assert_eq!(request.body.as_deref(), Some(&b"hi"[..]));
    }

    #[test]
    fn a_step_with_no_expect_defaults_to_asserting_nothing() {
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let m = str_leaf(&mut b, "GET");
        let path = str_leaf(&mut b, "/health");
        let http = record(&mut b, vec![("method", m), ("path", path)]);
        let step = record(&mut b, vec![("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        let Step::Http { expect, .. } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert_eq!(expect, &Expect::default());
    }

    #[test]
    fn summary_reports_router_program_and_request_counts() {
        let spec = parse_run_spec(&sample_run_spec_bytes()).unwrap();
        let s = spec.summary();
        assert!(s.contains("root-router=\"router\""), "got: {s}");
        assert!(s.contains("1 program(s)"), "got: {s}");
        assert!(s.contains("1 request(s)"), "got: {s}");
    }

    #[test]
    fn expect_checks_each_field_and_reports_the_first_miss() {
        let e = Expect {
            status: Some(200),
            body: Some(b"hello".to_vec()),
            body_contains: Some("ell".into()),
            ..Default::default()
        };
        assert!(e.check(200, &[], b"hello").is_ok());
        // Status mismatch is named.
        assert!(e.check(404, &[], b"hello").unwrap_err().contains("status"));
        // Exact-body mismatch is named.
        assert!(e.check(200, &[], b"HELLO").unwrap_err().contains("body"));
        // body-contains miss is named.
        let contains = Expect {
            status: None,
            body: None,
            body_contains: Some("wasm".into()),
            ..Default::default()
        };
        assert!(
            contains
                .check(200, &[], b"plain")
                .unwrap_err()
                .contains("body-contains")
        );
        assert!(contains.check(200, &[], b"a wasm handler").is_ok());
        // An empty Expect asserts nothing.
        assert!(Expect::default().check(500, &[], b"anything").is_ok());
    }

    #[test]
    fn expect_checks_required_response_headers_case_insensitively() {
        let e = Expect {
            headers: vec![("content-type".into(), "text/plain".into())],
            ..Default::default()
        };
        // Present (response headers arrive lower-cased) → ok.
        let resp = [("content-type".to_string(), "text/plain".to_string())];
        assert!(e.check(200, &resp, b"").is_ok());
        // Header NAME match is case-insensitive (the assertion may spell it any case).
        let mixed = Expect {
            headers: vec![("Content-Type".into(), "text/plain".into())],
            ..Default::default()
        };
        assert!(mixed.check(200, &resp, b"").is_ok());
        // Missing header → named error.
        let err = e.check(200, &[], b"").unwrap_err();
        assert!(
            err.contains("header") && err.contains("content-type"),
            "got: {err}"
        );
        // Value must match exactly.
        let wrong_val = [("content-type".to_string(), "application/json".to_string())];
        assert!(e.check(200, &wrong_val, b"").is_err());
    }

    #[test]
    fn parses_expect_headers() {
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let m = str_leaf(&mut b, "GET");
        let path = str_leaf(&mut b, "/");
        let http = record(&mut b, vec![("method", m), ("path", path)]);
        // expect = { status = 200, headers = [ { name = "content-type", value = "text/plain" } ] }
        let st = uint_leaf(&mut b, 200);
        let hn = str_leaf(&mut b, "content-type");
        let hv = str_leaf(&mut b, "text/plain");
        let hdr = record(&mut b, vec![("name", hn), ("value", hv)]);
        let hlist = list_value(&mut b, vec![hdr]);
        let expect = record(&mut b, vec![("headers", hlist), ("status", st)]);
        let step = record(&mut b, vec![("expect", expect), ("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        let Step::Http { expect, .. } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert_eq!(
            expect.headers,
            vec![("content-type".to_string(), "text/plain".to_string())]
        );
    }

    #[test]
    fn parses_retry_until_match_bool() {
        // `retry-until-match = true` encodes as a `Leaf::Bool` (NOT a `Leaf::Name`) — the shape a
        // `cdz convert`-compiled run-spec actually carries. This asserts the parser reads that leaf (the
        // earlier as_name-based reader silently mis-parsed a real bool, failing the whole run-spec decode).
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let m = str_leaf(&mut b, "GET");
        let path = str_leaf(&mut b, "/");
        let http = record(&mut b, vec![("method", m), ("path", path)]);
        let st = uint_leaf(&mut b, 200);
        let retry = bool_leaf(&mut b, true);
        let expect = record(&mut b, vec![("retry-until-match", retry), ("status", st)]);
        let step = record(&mut b, vec![("expect", expect), ("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        let Step::Http { expect, .. } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert!(
            expect.retry_until_match,
            "retry-until-match bool should parse as true"
        );

        // A `false` bool parses to false (distinct from an absent field, which also defaults false).
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let m = str_leaf(&mut b, "GET");
        let path = str_leaf(&mut b, "/");
        let http = record(&mut b, vec![("method", m), ("path", path)]);
        let retry = bool_leaf(&mut b, false);
        let expect = record(&mut b, vec![("retry-until-match", retry)]);
        let step = record(&mut b, vec![("expect", expect), ("http", http)]);
        let requests = list_value(&mut b, vec![step]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        let Step::Http { expect, .. } = &spec.requests[0] else {
            panic!("expected an http step");
        };
        assert!(!expect.retry_until_match);
    }

    #[test]
    fn parses_control_steps() {
        // requests = [ { control = { push-root-router = "r2" } },
        //              { control = { push-down = { session = b"s", payload = b"p" } } } ]
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let name = str_leaf(&mut b, "r2");
        let prr = record(&mut b, vec![("push-root-router", name)]);
        let step1 = record(&mut b, vec![("control", prr)]);
        let sess = bytes_leaf(&mut b, b"s");
        let payload = bytes_leaf(&mut b, b"p");
        let pd_inner = record(&mut b, vec![("payload", payload), ("session", sess)]);
        let pd = record(&mut b, vec![("push-down", pd_inner)]);
        let step2 = record(&mut b, vec![("control", pd)]);
        let requests = list_value(&mut b, vec![step1, step2]);
        let root = record(&mut b, vec![("config", config), ("requests", requests)]);
        let spec = parse_run_spec(&finish(b, root, "RunSpec")).expect("parses");
        assert_eq!(spec.requests.len(), 2);
        assert_eq!(
            spec.requests[0],
            Step::Control(ControlStep::PushRootRouter("r2".into()))
        );
        assert_eq!(
            spec.requests[1],
            Step::Control(ControlStep::PushDown {
                session: Some(b"s".to_vec()),
                payload: b"p".to_vec(),
            })
        );
    }

    #[test]
    fn malformed_or_incomplete_is_none() {
        assert!(parse_run_spec(b"garbage").is_none());
        // Missing `requests` → None.
        let mut b = ValueBuilder::new();
        let rr = str_leaf(&mut b, "r");
        let empty = list_value(&mut b, vec![]);
        let config = record(&mut b, vec![("programs", empty), ("root-router", rr)]);
        let root = record(&mut b, vec![("config", config)]);
        assert!(parse_run_spec(&finish(b, root, "RunSpec")).is_none());
    }
}
