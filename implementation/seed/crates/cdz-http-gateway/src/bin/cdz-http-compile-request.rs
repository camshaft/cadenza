//! `cdz-http-compile-request` — build the binary-AST body a client POSTs to the multi-module `/compile` route.
//!
//! The COMPILE route handler (`cdz-reducer-guest/compile-route`) decodes its request body as a canonical
//! `CompileRoute` value — `Value.decode(r.body) : Option(CompileRoute)` where
//! `CompileRoute = | Compile(Record(modules: List(Module), entry: String))` and
//! `Module = | Module(Record(name: String, astHash: Bytes))`. A client first parses each source module via the
//! PARSE route (source -> AST published to the CAS -> the AST's raw `ProgramHash` bytes in the response body),
//! then POSTs the `{module-name -> ast-hash}` map + an entrypoint here. Constructing that binary-AST VALUE by
//! hand is the friction this tool removes: it emits exactly the bytes `Value.decode` accepts.
//!
//! There is deliberately NO `cdz` subcommand for this and no wasm runtime is needed — the value is built
//! structurally via the shared `cadenza-value` toolkit (the SAME `record`/`list`/`ascribe`/`finish` the
//! compiler's `Value.encode` uses), so this runs anywhere with no cranelift/JIT. It lives beside
//! `cdz-http-programhash` as a light DEFAULT-features deploy tool (no `host`/wasmtime).
//!
//! CANONICAL VALUE FORM (verified end-to-end — decodes as `CompileRoute`): each single-ctor newtype
//! (`Compile`, `Module`) is ERASED — the ctor is elided and the payload ascribed with the type name. The
//! per-element `(: #record … Module)` ascription is REQUIRED: `Value.decode` REJECTS a bare `#record` element
//! (that is why the `cdz run` value-RENDERER form — which elides it — is NOT decode-acceptable and must not be
//! hand-copied). `cadenza-value`'s `ascribe` puts it back, so this tool is correct by construction.
//!
//! Usage:
//!   cdz-http-compile-request --entry <NAME> --module <NAME>=<HASH> [--module <NAME>=<HASH> …] [-o <FILE>]
//!
//! - `--module <NAME>=<HASH>` (repeatable): a module named `<NAME>` whose AST is content-addressed by `<HASH>`.
//!   `<HASH>` is EITHER `@<path>` — read the raw `Hash::LEN` (33) bytes from `<path>` (the PARSE response body
//!   saved to a file, e.g. `curl … /parse -o main.hash`) — OR a base62 `Hash` string (what `Hash` Display /
//!   `cdz-http-programhash --base62` emit; decoded back to the raw bytes). A raw-bytes file that is not exactly
//!   33 bytes, or an unparseable base62 string, is a hard error (so a wrong hash fails LOUDLY here, not as a
//!   silent per-module store miss at `/compile` time).
//! - `--entry <NAME>`: the entrypoint module name (must be one of the `--module` names; rcdzc compiles it).
//! - `-o <FILE>` / `--out <FILE>`: write the body to `<FILE>` (default: stdout).

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

/// One `--module NAME=HASH` pair, resolved to the module name + the raw `Hash::LEN` ast-hash bytes.
struct Module {
    name: String,
    ast_hash: Vec<u8>,
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut entry: Option<String> = None;
    let mut out: Option<String> = None;
    let mut modules: Vec<Module> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--entry" => {
                entry = Some(args.next().ok_or("--entry needs a module name")?);
            }
            "-o" | "--out" => {
                out = Some(args.next().ok_or("-o needs a path")?);
            }
            "--module" => {
                let spec = args.next().ok_or("--module needs NAME=HASH")?;
                modules.push(parse_module(&spec)?);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unexpected argument: {other}\n\n{USAGE}")),
        }
    }

    let entry = entry.ok_or("missing --entry <NAME>")?;
    if modules.is_empty() {
        return Err("at least one --module <NAME>=<HASH> is required".to_string());
    }
    if !modules.iter().any(|m| m.name == entry) {
        return Err(format!(
            "--entry {entry:?} is not among the --module names ({})",
            modules
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let body = encode_compile_route(&modules, &entry);

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
fn parse_module(spec: &str) -> Result<Module, String> {
    let (name, hash) = spec
        .split_once('=')
        .ok_or_else(|| format!("--module must be NAME=HASH, got {spec:?}"))?;
    if name.is_empty() {
        return Err(format!("--module NAME is empty in {spec:?}"));
    }
    let ast_hash = resolve_hash(hash, name)?;
    Ok(Module {
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

/// Build the canonical `CompileRoute.Compile(Record(modules, entry))` value form and serialize it to the
/// binary-AST bytes `Value.decode : Option(CompileRoute)` accepts. Ctors erased (nominal newtypes); each module
/// element ascribed `(: #record … Module)` (REQUIRED by decode); record fields canonicalized by `value::record`.
fn encode_compile_route(modules: &[Module], entry: &str) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let elems: Vec<_> = modules
        .iter()
        .map(|m| {
            let name = value::str_leaf(&mut b, &m.name);
            let ast_hash = value::bytes_leaf(&mut b, &m.ast_hash);
            let rec = value::record(&mut b, vec![("name", name), ("astHash", ast_hash)]);
            value::ascribe(&mut b, rec, "Module")
        })
        .collect();
    let modules_list = value::list_value(&mut b, elems);
    let entry_str = value::str_leaf(&mut b, entry);
    let rec = value::record(
        &mut b,
        vec![("modules", modules_list), ("entry", entry_str)],
    );
    // `Compile` ctor elided (nominal newtype) — the boundary ascribes the record as `CompileRoute`.
    value::finish(b, rec, "CompileRoute").to_vec()
}

const USAGE: &str = "\
cdz-http-compile-request — build the /compile route's binary-AST request body.

Usage:
  cdz-http-compile-request --entry <NAME> --module <NAME>=<HASH> [--module <NAME>=<HASH> …] [-o <FILE>]

  --module <NAME>=<HASH>   a module named <NAME> content-addressed by <HASH>; repeatable.
                           <HASH> is @<path> (raw 33-byte ProgramHash from a saved PARSE response body)
                           or a base62 Hash string (as `cdz-http-programhash --base62` / Hash Display emit).
  --entry  <NAME>          the entrypoint module name (must be one of the --module names).
  -o, --out <FILE>         write the body to <FILE> (default: stdout).
";
