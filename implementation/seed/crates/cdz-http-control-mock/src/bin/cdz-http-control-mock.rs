//! `cdz-http-control-mock` — the conformance-harness mock control server binary. The driver spins it up as
//! one of the 3 SUT processes; it serves the ADMIN API (driver-facing, binary-AST) on `--admin-addr` and the
//! CONTROL-PLANE ws protocol (gateway-facing) on `--control-addr`, over one shared [`MockState`] + session
//! registry. The CAS coordinates it ships to a gateway (`--cas-url` / `--cas-credential`) are learned at
//! startup (the driver spun up the CAS on a known socket); programs are seeded by name via the admin API.
//!
//! Usage:
//!   cdz-http-control-mock --admin-addr 127.0.0.1:0 --control-addr 127.0.0.1:0 \
//!                         --cas-url http://127.0.0.1:PORT/ [--cas-credential TOKEN]

use bytes::Bytes;
use cdz_http_control_mock::MockState;
use cdz_http_control_mock::server::{AdminCtx, serve_admin};
use cdz_http_control_mock::ws::{new_sessions, serve_control};
use cdz_str::Str;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let state = Arc::new(Mutex::new(MockState::new(HashMap::new())));
    let sessions = new_sessions();

    let admin_listener = bind(&args.admin_addr, "admin").await;
    let control_listener = bind(&args.control_addr, "control").await;
    // Print the actual bound addresses (an ephemeral :0 resolves to a real port the driver reads).
    eprintln!(
        "cdz-http-control-mock: admin={} control={} cas-url={}",
        admin_listener.local_addr().unwrap(),
        control_listener.local_addr().unwrap(),
        args.cas_url,
    );

    let admin_ctx = AdminCtx {
        state: state.clone(),
        sessions: sessions.clone(),
        cas_url: Str::from(args.cas_url.as_str()),
        cas_credential: args.cas_credential,
    };

    tokio::join!(
        serve_admin(admin_listener, admin_ctx),
        serve_control(control_listener, state, sessions),
    );
}

async fn bind(addr: &str, which: &str) -> TcpListener {
    match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cdz-http-control-mock: cannot bind {which} address '{addr}': {e}");
            std::process::exit(1);
        }
    }
}

/// The parsed CLI arguments.
struct Args {
    admin_addr: String,
    control_addr: String,
    cas_url: String,
    cas_credential: Bytes,
}

impl Args {
    fn parse() -> Self {
        let mut admin_addr = None;
        let mut control_addr = None;
        let mut cas_url = None;
        let mut cas_credential = Bytes::new();
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let mut value = || {
                it.next().unwrap_or_else(|| {
                    eprintln!("cdz-http-control-mock: flag '{flag}' needs a value");
                    std::process::exit(2);
                })
            };
            match flag.as_str() {
                "--admin-addr" => admin_addr = Some(value()),
                "--control-addr" => control_addr = Some(value()),
                "--cas-url" => cas_url = Some(value()),
                "--cas-credential" => cas_credential = Bytes::from(value().into_bytes()),
                other => {
                    eprintln!("cdz-http-control-mock: unknown flag '{other}'");
                    std::process::exit(2);
                }
            }
        }
        let require = |v: Option<String>, name: &str| {
            v.unwrap_or_else(|| {
                eprintln!("cdz-http-control-mock: missing required --{name}");
                std::process::exit(2);
            })
        };
        Self {
            admin_addr: require(admin_addr, "admin-addr"),
            control_addr: require(control_addr, "control-addr"),
            cas_url: require(cas_url, "cas-url"),
            cas_credential,
        }
    }
}
