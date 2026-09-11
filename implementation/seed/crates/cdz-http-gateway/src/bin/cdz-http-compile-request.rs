//! `cdz-http-compile-request` — build the binary-AST body a client POSTs to the `/compile` route.
//!
//! The COMPILE route handler (`cdz-reducer-guest/compile-route`) decodes its request body as a canonical
//! `CompileRoute` value (reducer-targets B10c artifact-list design) —
//! `Value.decode(r.body) : Option(CompileRoute)` where:
//!   `Payload       = | Inline(Bytes) | CasRef(Bytes)`
//!   `RouteArtifact = | RouteArtifact(Record(kind: String, name: String, payload: Payload))`
//!   `CompileRoute  = | Compile(Record(artifacts: List(RouteArtifact)))`
//! The handler resolves each payload to bytes (CasRef → `blobs.get(hash)`, Inline → bytes as-is) and hands
//! rcdzc the bytes-only `Artifact(kind, name, bytes)` list — no per-kind special-casing, no entry synthesis.
//! So the CLIENT assembles the complete kinded list: a `kind="ast"` artifact per module (payload `CasRef(<ast
//! hash from /parse>)`, big blobs stay by-reference) plus one `kind="entry"` artifact (payload `Inline(<entry
//! name in the binary-AST name wire>)`). Constructing that value by hand is the friction this tool removes: it
//! emits exactly the bytes `Value.decode` accepts, with the entry name encoded correctly (see below).
//!
//! ENTRY ENCODING (the exact thing that 422'd the naive path): rcdzc reads the `KIND_ENTRY` artifact via
//! `cadenza_compile_abi::decode_name` — the bytes must be the binary-AST NAME wire (a codec `Str` leaf via
//! `encode_name`), NOT raw UTF-8 (operator P0 seq-284: binary-AST everywhere, no raw-bytes-as-name). This tool
//! OWNS that encoding: `--entry <name>` emits `RouteArtifact{kind="entry", name="", payload=Inline(encode_name(name))}`,
//! so the client passes the entry by NAME and can't get the wire wrong.
//!
//! No wasm runtime is needed — the value is built structurally via the shared `cadenza-value` toolkit (the same
//! `record`/`list`/`ascribe`/`bare_ctor`/`finish` `Value.encode` uses). Light DEFAULT-features deploy tool
//! (no `host`/wasmtime), sibling of `cdz-http-programhash` in `cdz-http-gateway`.
//!
//! Usage:
//!   cdz-http-compile-request --ast <NAME>=<HASH> [--ast <NAME>=<HASH> …] --entry <NAME> [-o <FILE>]
//!
//! - `--ast <NAME>=<HASH>` (repeatable): a source module named `<NAME>` whose AST is content-addressed by
//!   `<HASH>`, emitted as a `kind="ast"` artifact with a `CasRef` payload. `<HASH>` is `@<path>` (raw
//!   `Hash::LEN`=33 bytes from a saved `/parse` response body) or a base62 `Hash` string (decoded to raw). A
//!   wrong-length / unparseable hash is a hard error (fails LOUDLY here, not as a silent store miss at compile).
//! - `--entry <NAME>`: the entrypoint module name (must be one of the `--ast` names). Emitted as a
//!   `kind="entry"` artifact with an `Inline` payload = `encode_name(<NAME>)`.
//! - `-o <FILE>` / `--out <FILE>`: write the body to `<FILE>` (default: stdout).

use cadenza_compile_abi::encode_name;
use cdz_http_protocol::value::{self, ValueBuilder};
use cdz_platform::Hash;
use std::io::Write;
use std::process::ExitCode;
use std::str::FromStr;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("cdz-http-compile-request: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// One `--ast NAME=HASH` pair, resolved to the module name + the raw `Hash::LEN` ast-hash bytes.
struct AstInput {
    name: String,
    ast_hash: Vec<u8>,
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut entry: Option<String> = None;
    let mut out: Option<String> = None;
    let mut asts: Vec<AstInput> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--entry" => {
                entry = Some(args.next().ok_or("--entry needs a module name")?);
            }
            "-o" | "--out" => {
                out = Some(args.next().ok_or("-o needs a path")?);
            }
            "--ast" => {
                let spec = args.next().ok_or("--ast needs NAME=HASH")?;
                asts.push(parse_ast(&spec)?);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unexpected argument: {other}\n\n{USAGE}")),
        }
    }

    let entry = entry.ok_or("missing --entry <NAME>")?;
    if asts.is_empty() {
        return Err("at least one --ast <NAME>=<HASH> is required".to_string());
    }
    if !asts.iter().any(|a| a.name == entry) {
        return Err(format!(
            "--entry {entry:?} is not among the --ast names ({})",
            asts.iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let body = encode_compile_route(&asts, &entry);

    match out {
        Some(path) => {
            std::fs::write(&path, &body).map_err(|e| format!("cannot write {path}: {e}"))?
        }
        None => std::io::stdout()
            .write_all(&body)
            .map_err(|e| format!("cannot write stdout: {e}"))?,
    }
    Ok(())
}

/// Parse `NAME=HASH` where HASH is `@<path>` (raw 33-byte file) or a base62 `Hash` string.
fn parse_ast(spec: &str) -> Result<AstInput, String> {
    let (name, hash) = spec
        .split_once('=')
        .ok_or_else(|| format!("--ast must be NAME=HASH, got {spec:?}"))?;
    if name.is_empty() {
        return Err(format!("--ast NAME is empty in {spec:?}"));
    }
    let ast_hash = resolve_hash(hash, name)?;
    Ok(AstInput {
        name: name.to_string(),
        ast_hash,
    })
}

/// Resolve a `<HASH>` token to the raw `Hash::LEN` bytes: `@<path>` reads raw bytes from a file (the PARSE
/// response body); anything else is parsed as a base62 `Hash` string. Either way the result is validated to be
/// exactly `Hash::LEN` bytes so a bad hash fails here, not as a silent store miss at `/compile` time.
fn resolve_hash(hash: &str, module: &str) -> Result<Vec<u8>, String> {
    let bytes = if let Some(path) = hash.strip_prefix('@') {
        std::fs::read(path)
            .map_err(|e| format!("module {module:?}: cannot read hash file {path}: {e}"))?
    } else {
        let parsed = Hash::from_str(hash).map_err(|_| {
            format!("module {module:?}: {hash:?} is neither @<file> nor a valid base62 hash")
        })?;
        parsed.as_bytes().to_vec()
    };
    if bytes.len() != Hash::LEN {
        return Err(format!(
            "module {module:?}: ast-hash is {} bytes, expected {} (a raw ProgramHash) — did you pass a base62 string as a raw file, or a truncated body?",
            bytes.len(),
            Hash::LEN
        ));
    }
    Ok(bytes)
}

/// Build the canonical `CompileRoute.Compile(Record(artifacts))` value form and serialize it to the binary-AST
/// bytes `Value.decode : Option(CompileRoute)` accepts. Each `RouteArtifact` newtype is ascribed (REQUIRED by
/// decode); `Payload` is a multi-ctor sum so `Inline`/`CasRef` keep their constructor; record fields are
/// canonicalized by `value::record`. The entry rides Inline with its bytes = `encode_name(entry)` (the codec
/// `Str`-leaf name wire rcdzc's `decode_name` reads); each AST rides `CasRef(raw-33-hash)`.
fn encode_compile_route(asts: &[AstInput], entry: &str) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let mut elems: Vec<value::ValueId> = asts
        .iter()
        .map(|a| {
            let hash = value::bytes_leaf(&mut b, &a.ast_hash);
            let payload = value::bare_ctor(&mut b, "CasRef", vec![hash]);
            route_artifact(&mut b, "ast", &a.name, payload)
        })
        .collect();
    let name_wire = value::bytes_leaf(&mut b, &encode_name(entry));
    let entry_payload = value::bare_ctor(&mut b, "Inline", vec![name_wire]);
    elems.push(route_artifact(&mut b, "entry", "", entry_payload));

    let artifacts = value::list_value(&mut b, elems);
    let rec = value::record(&mut b, vec![("artifacts", artifacts)]);
    // `Compile` ctor elided (nominal newtype) — the boundary ascribes the record as `CompileRoute`.
    value::finish(b, rec, "CompileRoute").to_vec()
}

/// `RouteArtifact.RouteArtifact(Record(kind, name, payload))` → `(: #record RouteArtifact)` (ctor elided).
fn route_artifact(
    b: &mut ValueBuilder,
    kind: &str,
    name: &str,
    payload: value::ValueId,
) -> value::ValueId {
    let kind = value::str_leaf(b, kind);
    let name = value::str_leaf(b, name);
    let rec = value::record(
        b,
        vec![("kind", kind), ("name", name), ("payload", payload)],
    );
    value::ascribe(b, rec, "RouteArtifact")
}

const USAGE: &str = "\
cdz-http-compile-request — build the /compile route's binary-AST request body (artifact-list form).

Usage:
  cdz-http-compile-request --ast <NAME>=<HASH> [--ast <NAME>=<HASH> …] --entry <NAME> [-o <FILE>]

  --ast   <NAME>=<HASH>   a source module <NAME> content-addressed by <HASH>; repeatable. Emitted as a
                          kind=\"ast\" CasRef artifact. <HASH> is @<path> (raw 33-byte ProgramHash from a
                          saved PARSE response body) or a base62 Hash string.
  --entry <NAME>          the entrypoint module name (must be one of the --ast names). Emitted as a
                          kind=\"entry\" Inline artifact whose bytes are encode_name(<NAME>).
  -o, --out <FILE>        write the body to <FILE> (default: stdout).
";
