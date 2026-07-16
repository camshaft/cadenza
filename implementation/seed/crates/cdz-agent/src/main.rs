//! `cdz-agent` — the inbox-loop DRIVER: a real hive-driving binary that runs the native Cadenza agent
//! harness over a fleet inbox (hardening + dogfood, concierge-greenlit 2026-07-16).
//!
//! For each JSON message in an inbox directory (`.claude/fleet/inbox/<agent>/*.json`, the format the
//! fleet already uses), it drives a compiled Cadenza agent-loop consumer through the AUTHORIZED loop
//! ([`cdz_agent::run_hive_agent_loop`]): the loop READS the message body via `Inbox.next`, calls the
//! model on it (`Model.converse`, mock by default / Bedrock behind `--features bedrock`), and every tool
//! dispatch is gated by a Cedar policy (`Cedar.authorize`). The message body reaches the model through
//! the inbox closure this binary supplies — so the agent actually acts on the message it is driving.
//!
//! Usage:
//!   cdz-agent --consumer <loop.wasm> --inbox <dir> [--policy <cedar-file>] [--model <id>] [--limit N]
//!            [--ack]
//!
//! `--ack` moves each driven message into `<inbox>/processed/` (the fleet's ack convention) so a re-run
//! doesn't re-process it — turning repeated runs into a durable, at-most-once drain. Without it the
//! driver is read-only (prints outcomes, leaves the inbox untouched — useful for a dry run). Writing a
//! REPLY back to a sender is a further follow-on; the end-to-end path (real message → the pure-Cadenza
//! authorized loop → a model, driven once) runs.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(e) = run() {
        eprintln!("cdz-agent: {e}");
        std::process::exit(1);
    }
}

/// Parsed CLI args (a tiny hand-rolled parser — no clap dep in this leaf crate).
struct Args {
    consumer: PathBuf,
    inbox: PathBuf,
    policy: Option<PathBuf>,
    #[cfg_attr(not(feature = "bedrock"), allow(dead_code))]
    model: Option<String>,
    limit: usize,
    ack: bool,
}

fn parse_args() -> Result<Args> {
    let mut consumer = None;
    let mut inbox = None;
    let mut policy = None;
    let mut model = None;
    let mut limit = usize::MAX;
    let mut ack = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--consumer" => consumer = Some(PathBuf::from(next(&mut it, "--consumer")?)),
            "--inbox" => inbox = Some(PathBuf::from(next(&mut it, "--inbox")?)),
            "--policy" => policy = Some(PathBuf::from(next(&mut it, "--policy")?)),
            "--model" => model = Some(next(&mut it, "--model")?),
            "--limit" => {
                limit = next(&mut it, "--limit")?
                    .parse()
                    .map_err(|e| anyhow!("--limit not a number: {e}"))?
            }
            "--ack" => ack = true,
            other => {
                return Err(anyhow!(
                    "unknown arg `{other}` (see the usage in the module docs)"
                ))
            }
        }
    }
    Ok(Args {
        consumer: consumer.ok_or_else(|| anyhow!("--consumer <loop.wasm> is required"))?,
        inbox: inbox.ok_or_else(|| anyhow!("--inbox <dir> is required"))?,
        policy,
        model,
        limit,
        ack,
    })
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next().ok_or_else(|| anyhow!("{flag} needs a value"))
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let consumer = std::fs::read(&args.consumer)
        .map_err(|e| anyhow!("read consumer {}: {e}", args.consumer.display()))?;

    // Resolve the value-heap runtime the consumer requires, by content address, from the store (walk up
    // ancestors to find `target/cadenza-store/<hash>.wasm`).
    let req = cdz_run::required_runtime(&consumer)?.ok_or_else(|| {
        anyhow!("the consumer imports no value-heap runtime (not an agent loop?)")
    })?;
    let runtime = find_runtime(&req.hash).ok_or_else(|| {
        anyhow!(
            "runtime {} not found in any ancestor `target/cadenza-store` (run `cargo xtask build`)",
            req.hash
        )
    })?;

    // The Cedar policy gating tool dispatch: from `--policy`, else a permit-all default (the mock demo).
    // A permit-all default is explicit + logged, so it is never a silent open door.
    let policies = match &args.policy {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| anyhow!("read policy {}: {e}", p.display()))?
        }
        None => {
            eprintln!("cdz-agent: no --policy given; using a PERMIT-ALL default (demo only)");
            "permit(principal, action, resource);".to_string()
        }
    };

    // Collect the inbox messages (oldest-first by filename, the fleet's delivery-sequence order).
    let mut msgs = read_inbox(&args.inbox)?;
    msgs.sort_by(|a, b| a.0.cmp(&b.0));
    if msgs.is_empty() {
        println!("cdz-agent: inbox {} is empty", args.inbox.display());
        return Ok(());
    }

    // With --ack, a driven message is MOVED into `<inbox>/processed/` so a re-run doesn't re-process it
    // (the fleet's own ack convention). Create the dir once, up front, if acking.
    let processed_dir = args.inbox.join("processed");
    if args.ack {
        std::fs::create_dir_all(&processed_dir)
            .map_err(|e| anyhow!("mkdir {}: {e}", processed_dir.display()))?;
    }

    let mut processed = 0usize;
    for (name, body) in msgs.into_iter().take(args.limit) {
        let outcome = drive_one(&consumer, &runtime, &policies, &body, args.model.clone())?;
        println!("cdz-agent: {name} -> {outcome}");
        // ACK: move the message into processed/ so it is driven exactly once across runs. A rename
        // failure is surfaced (not swallowed) — a message that "processed" but wasn't acked would be
        // re-driven, which the operator should see rather than have silently happen.
        if args.ack {
            let from = args.inbox.join(&name);
            let to = processed_dir.join(&name);
            std::fs::rename(&from, &to)
                .map_err(|e| anyhow!("ack (move) {} -> {}: {e}", from.display(), to.display()))?;
        }
        processed += 1;
    }
    println!("cdz-agent: processed {processed} message(s)");
    Ok(())
}

/// Drive ONE message through the full hive-agent loop: the loop reads the message body via `Inbox.next`
/// (so the body ACTUALLY reaches the model), calls the model on it (`Model.converse`), and every tool
/// dispatch is Cedar-gated by `policies`. The consumer must import all three ops (`cadenza:inbox/api`,
/// `cadenza:model/api`, `cadenza:cedar/api`).
fn drive_one(
    consumer: &[u8],
    runtime: &[u8],
    policies: &str,
    msg_body: &str,
    _model: Option<String>,
) -> Result<String> {
    let opts = cdz_run::RunOpts {
        export: Some("main".to_string()),
        args: Vec::new(),
        runtime: Some(runtime.to_vec()),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    let authorize = cdz_agent::cedar_authorizer(
        policies.to_string(),
        r#"Agent::"agent:cdz-agent""#.to_string(),
        r#"Tool::"any""#.to_string(),
        "{}".to_string(),
    );
    // The inbox closure hands the loop THIS message's body — so the model call operates on the real
    // message (the body reaches the model via `Inbox.next` → `Model.converse`, not a fixed prompt).
    let body = msg_body.to_string();
    let next_message = move || body.clone();

    // Observe model FAILURES: a backend encodes a failed call as a `MODEL_ERROR_PREFIX` completion (the
    // converse contract is `String -> String`, so it can't return a Result). Wrap the backend to raise a
    // shared flag whenever it returns that marker, so the loop's outcome is reported as a FAILURE rather
    // than a normal completion — a Bedrock error never silently becomes an answer the agent acts on.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let model_errored = Arc::new(AtomicBool::new(false));
    let observe = {
        let flag = Arc::clone(&model_errored);
        move |completion: String| {
            if cdz_agent::is_model_error(&completion) {
                flag.store(true, Ordering::SeqCst);
                eprintln!("cdz-agent: model call failed: {completion}");
            }
            completion
        }
    };

    // The model backend: mock by default; the real Bedrock backend when built with `--features bedrock`
    // + an optional `--model` (defaulting to a region-prefixed inference-profile id). Wrapped in `observe`.
    let outcome = {
        #[cfg(feature = "bedrock")]
        {
            let model =
                _model.unwrap_or_else(|| "us.anthropic.claude-haiku-4-5-20251001-v1:0".to_string());
            let backend = cdz_agent::bedrock_converse(model, 1024);
            let converse = move |p: String| observe(backend(p));
            cdz_agent::run_hive_agent_loop(consumer, &opts, next_message, converse, authorize)?
        }
        #[cfg(not(feature = "bedrock"))]
        {
            let converse = move |p: String| observe(cdz_agent::mock_converse(p));
            cdz_agent::run_hive_agent_loop(consumer, &opts, next_message, converse, authorize)?
        }
    };
    // A model failure during the run is a FAILURE outcome, even if the loop returned a value (the marker
    // completion would otherwise render as a normal `value …`).
    if model_errored.load(Ordering::SeqCst) {
        return Ok("model-error (see stderr)".to_string());
    }
    Ok(match outcome {
        cdz_run::Outcome::Value(s) => format!("value {s}"),
        cdz_run::Outcome::Trap(t) => format!("trap {t}"),
    })
}

/// Read every `*.json` message directly in `dir` (not recursing into `processed/`), returning
/// `(filename, body-field)` pairs. The `body` field is extracted with a minimal JSON string reader (no
/// serde dep); a message with no `body` yields an empty string.
fn read_inbox(dir: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| anyhow!("read inbox dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow!("read message {}: {e}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        out.push((name, json_string_field(&text, "body").unwrap_or_default()));
    }
    Ok(out)
}

/// Extract the string value of top-level JSON field `key` from `body` — a minimal reader (no serde):
/// find `"<key>":"`, then read the quoted value honoring backslash escapes, iterating by CHAR so
/// multi-byte UTF-8 survives (the same discipline as the Bedrock decoder). Returns None if absent.
fn json_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = body.find(&needle)? + needle.len();
    let mut chars = body[start..].chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => return Some(out),
            },
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

/// Walk up from the current dir to find `target/cadenza-store/<hash>.wasm` for the required `hash`.
fn find_runtime(hash: &str) -> Option<Vec<u8>> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir
            .join("target/cadenza-store")
            .join(format!("{hash}.wasm"));
        if let Ok(bytes) = std::fs::read(&candidate) {
            return Some(bytes);
        }
        if !dir.pop() {
            return None;
        }
    }
}
