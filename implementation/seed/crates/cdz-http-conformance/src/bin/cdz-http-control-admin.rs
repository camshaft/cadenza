//! `cdz-http-control-admin` — an ad-hoc CLI to drive a LIVE mock control server (operator request, routed via
//! concierge). Outside the conformance harness you often want to poke a running `cdz-http-control-mock`
//! directly: set/push the root router, register a program, reset it, or inspect the control-ups it captured.
//!
//! It is a thin argv → [`AdminCommand`] → [`AdminClient`] → print-reply shell over the SAME binary-AST admin
//! client + frames the harness driver uses (one codec, no new protocol). Point it at the mock's admin address
//! (its `admin=<addr>` ready line):
//!
//!   cdz-http-control-admin <admin-addr> <command> [args]
//!     reset                              — clear the mock's programs / config / captured control-ups
//!     set-root-router  <name>            — set the root router to the (already-registered) program name
//!     push-root-router <name>            — live-swap the root router to the program name
//!     set-program      <name> <base62-hash>  — register a program name → its ProgramHash
//!     get-control-ups                    — print the captured ControlUps (handler → control provenance)
//!
//! Payload-carrying commands (prime-reply / push-down) take opaque bytes and ride a follow-on.

use bytes::Bytes;
use cdz_contract::Hash;
use cdz_http_conformance::AdminClient;
use cdz_http_control_mock::admin::{AdminCommand, AdminReply};
use cdz_http_protocol::decode_control_up;
use cdz_str::Str;
use std::net::SocketAddr;
use std::process::ExitCode;

const USAGE: &str = "usage: cdz-http-control-admin <admin-addr> <command> [args]\n  \
commands: reset | set-root-router <name> | push-root-router <name> | \
set-program <name> <base62-hash> | get-control-ups";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cdz-http-control-admin: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().ok_or(USAGE)?;
    let addr: SocketAddr = addr
        .parse()
        .map_err(|e| format!("bad admin address {addr:?}: {e}"))?;
    let cmd = args.next().ok_or(USAGE)?;
    let rest: Vec<String> = args.collect();

    let command = build_command(&cmd, &rest)?;
    let reply = AdminClient::new(addr).send(&command).await?;
    print_reply(&reply)
}

/// Map a CLI subcommand + its args to the corresponding [`AdminCommand`].
fn build_command(cmd: &str, args: &[String]) -> Result<AdminCommand, String> {
    let arg = |i: usize, what: &str| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("{cmd}: missing {what}\n{USAGE}"))
    };
    match cmd {
        "reset" => Ok(AdminCommand::Reset),
        "set-root-router" => Ok(AdminCommand::SetRootRouter {
            program: Str::from(arg(0, "<name>")?.as_str()),
        }),
        "push-root-router" => Ok(AdminCommand::PushRootRouter {
            program: Str::from(arg(0, "<name>")?.as_str()),
        }),
        "set-program" => {
            let name = arg(0, "<name>")?;
            let hash_text = arg(1, "<base62-hash>")?;
            let hash: Hash = hash_text
                .parse()
                .map_err(|e| format!("bad base62 hash {hash_text:?}: {e}"))?;
            Ok(AdminCommand::SetProgram {
                name: Str::from(name.as_str()),
                hash: Bytes::copy_from_slice(hash.as_bytes()),
            })
        }
        "get-control-ups" => Ok(AdminCommand::GetControlUps),
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    }
}

/// Print the mock's reply; `Ok`/`ControlUps` succeed, `Error` fails (exit 1).
fn print_reply(reply: &AdminReply) -> Result<(), String> {
    match reply {
        AdminReply::Ok => {
            println!("ok");
            Ok(())
        }
        AdminReply::Error { message } => Err(format!("mock rejected the command: {message}")),
        AdminReply::ControlUps { ups } => {
            println!("{} captured control-up(s):", ups.len());
            for (i, bytes) in ups.iter().enumerate() {
                match decode_control_up(bytes) {
                    Some(up) => println!(
                        "  [{i}] program={} session={} correlation={} request={} {} payload={}B",
                        hash_text(&up.program),
                        lossy(&up.session),
                        lossy(&up.correlation),
                        up.request.method,
                        up.request.path,
                        up.payload.len(),
                    ),
                    None => println!("  [{i}] <undecodable control-up, {}B>", bytes.len()),
                }
            }
            Ok(())
        }
    }
}

/// Render a 33-byte `ProgramHash` as its base62 text (what the CAS + the deploy tool print); fall back to a
/// byte count if it is not a well-formed hash.
fn hash_text(bytes: &[u8]) -> String {
    Hash::try_from(bytes).map_or_else(|_| format!("{}B", bytes.len()), |h| h.to_string())
}

/// A lossy-UTF8 view of an opaque token (session / correlation are usually ascii-ish, e.g. `conn-42`).
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
