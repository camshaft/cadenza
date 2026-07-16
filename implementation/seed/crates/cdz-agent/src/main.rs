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
//!            [--ack] [--reply-to <dir>]
//!
//! `--ack` moves each driven message into `<inbox>/processed/` (the fleet's ack convention) so a re-run
//! doesn't re-process it — turning repeated runs into a durable, at-most-once drain. Without it the
//! driver is read-only (prints outcomes, leaves the inbox untouched — useful for a dry run).
//!
//! `--reply-to <dir>` closes the hive loop: after a message is driven, the model's completion is written
//! back as a reply message JSON (`{from,to,kind:"answer",subject,body,in_reply_to}`, the fleet's own
//! format) into `<dir>` — addressed `to` the source message's `from`, with `in_reply_to` naming the
//! source filename (the correlation `fleet audit` uses, so the driver's replies are auditable). So a peer
//! sends a task and gets the agent's answer back. A reply is written only for a message that actually
//! reached the model (a Cedar-DENIED run made no model call, so there is nothing to answer) whose sender
//! is known; a bare tick without `--reply-to` still just prints outcomes.

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
    reply_to: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut consumer = None;
    let mut inbox = None;
    let mut policy = None;
    let mut model = None;
    let mut limit = usize::MAX;
    let mut ack = false;
    let mut reply_to = None;
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
            "--reply-to" => reply_to = Some(PathBuf::from(next(&mut it, "--reply-to")?)),
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
        reply_to,
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
    msgs.sort_by(|a, b| a.name.cmp(&b.name));
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
    // With --reply-to, the model's answer is written back as a reply message into this dir. Create it once.
    if let Some(dir) = &args.reply_to {
        std::fs::create_dir_all(dir).map_err(|e| anyhow!("mkdir {}: {e}", dir.display()))?;
    }

    let mut processed = 0usize;
    for msg in msgs.into_iter().take(args.limit) {
        let result = drive_one(
            &consumer,
            &runtime,
            &policies,
            &msg.body,
            args.model.clone(),
        )?;
        println!("cdz-agent: {} -> {}", msg.name, result.report);
        // REPLY: close the hive loop by writing the model's answer back to the sender. Only when
        // --reply-to is given, the model actually answered (a denied/failed run has no completion), and
        // the source names a sender to address. A write failure is surfaced, not swallowed.
        if let Some(dir) = &args.reply_to {
            match (&result.completion, msg.from.is_empty()) {
                (Some(answer), false) => {
                    let path = write_reply(dir, &msg.name, &msg.from, answer)?;
                    println!("cdz-agent: replied to {} -> {}", msg.from, path.display());
                }
                (Some(_), true) => eprintln!(
                    "cdz-agent: {} has no `from`; not replying (nobody to address)",
                    msg.name
                ),
                (None, _) => eprintln!(
                    "cdz-agent: {} produced no model answer (denied/failed); not replying",
                    msg.name
                ),
            }
        }
        // ACK: move the message into processed/ so it is driven exactly once across runs. A rename
        // failure is surfaced (not swallowed) — a message that "processed" but wasn't acked would be
        // re-driven, which the operator should see rather than have silently happen. Ack AFTER the reply
        // so a failed reply-write leaves the message un-acked (re-drivable), never lost.
        if args.ack {
            let from = args.inbox.join(&msg.name);
            let to = processed_dir.join(&msg.name);
            std::fs::rename(&from, &to)
                .map_err(|e| anyhow!("ack (move) {} -> {}: {e}", from.display(), to.display()))?;
        }
        processed += 1;
    }
    println!("cdz-agent: processed {processed} message(s)");
    Ok(())
}

/// Write a reply message back to the sender: a fleet-format JSON
/// (`{from,to,kind:"answer",subject,body,in_reply_to}`) naming this driver as `from`, the source
/// message's sender as `to`, and the model's completion as the `body`. `in_reply_to` names the SOURCE
/// message's filename — the same correlation `fleet ack`/`fleet audit` use to prove a request got
/// exactly one reply, so the driver's replies are first-class auditable (not "unverifiable"). The
/// reply's own filename is derived deterministically from the source name (`reply-<source>`) so runs are
/// reproducible and a re-drive overwrites its own prior reply rather than piling up duplicates (the
/// toolchain forbids wall-clock, and reusing the hub's durable delivery-seq would couple this leaf
/// binary to the hub's counter file). Returns the written path.
fn write_reply(dir: &Path, source_name: &str, to: &str, answer: &str) -> Result<PathBuf> {
    let subject = format!("re: {source_name}");
    // Assemble the JSON by hand (no serde dep in this leaf binary), escaping every string field so a
    // completion containing quotes/newlines/control chars can't break the message or inject fields.
    // `in_reply_to` carries the source filename verbatim (also escaped, though fleet names are tame).
    let json = format!(
        r#"{{"from":"cdz-agent","to":"{}","kind":"answer","subject":"{}","body":"{}","in_reply_to":"{}"}}"#,
        cdz_agent::json_escape(to),
        cdz_agent::json_escape(&subject),
        cdz_agent::json_escape(answer),
        cdz_agent::json_escape(source_name)
    );
    let path = dir.join(format!("reply-{source_name}"));
    std::fs::write(&path, json).map_err(|e| anyhow!("write reply {}: {e}", path.display()))?;
    Ok(path)
}

/// The result of driving one message: a human-readable `report` (printed per-message) and, when the
/// model was actually called and returned a real (non-error) completion, that `completion` — the text a
/// `--reply-to` reply carries back to the sender. `completion` is None for a Cedar-DENIED run (no model
/// call was made) or a model failure (nothing trustworthy to answer with).
struct DriveResult {
    report: String,
    completion: Option<String>,
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
) -> Result<DriveResult> {
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

    // Observe model calls: (1) raise a shared flag on a FAILURE — a backend encodes a failed call as a
    // `MODEL_ERROR_PREFIX` completion (the converse contract is `String -> String`, so it can't return a
    // Result), so the loop's outcome is reported as a FAILURE rather than a normal completion; a Bedrock
    // error never silently becomes an answer the agent acts on. (2) Capture the LAST real completion, so
    // a `--reply-to` reply can carry the model's actual answer back to the sender.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let model_errored = Arc::new(AtomicBool::new(false));
    let last_completion: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let observe = {
        let flag = Arc::clone(&model_errored);
        let last = Arc::clone(&last_completion);
        move |completion: String| {
            if cdz_agent::is_model_error(&completion) {
                flag.store(true, Ordering::SeqCst);
                eprintln!("cdz-agent: model call failed: {completion}");
            } else {
                *last.lock().unwrap() = Some(completion.clone());
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
    // completion would otherwise render as a normal `value …`). No completion is trustworthy to reply with.
    if model_errored.load(Ordering::SeqCst) {
        return Ok(DriveResult {
            report: "model-error (see stderr)".to_string(),
            completion: None,
        });
    }
    // The reply body is the model's actual completion — but only if the model was CALLED (a Cedar-denied
    // run never performs `Model.converse`, so `last_completion` stays None → nothing to reply with).
    let completion = last_completion.lock().unwrap().take();
    let report = match outcome {
        cdz_run::Outcome::Value(s) => format!("value {s}"),
        cdz_run::Outcome::Trap(t) => format!("trap {t}"),
    };
    Ok(DriveResult { report, completion })
}

/// One inbox message the driver acts on: the source filename, the `body` (what reaches the model), and
/// the `from` sender (so a reply can be addressed back). Both fields default to empty when absent.
struct Msg {
    name: String,
    body: String,
    from: String,
}

/// Read every `*.json` message directly in `dir` (not recursing into `processed/`), returning one [`Msg`]
/// each. The `body`/`from` fields are extracted with a minimal JSON string reader (no serde dep); a
/// message missing either yields an empty string for it.
fn read_inbox(dir: &Path) -> Result<Vec<Msg>> {
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
        out.push(Msg {
            name,
            body: json_string_field(&text, "body").unwrap_or_default(),
            from: json_string_field(&text, "from").unwrap_or_default(),
        });
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
