//! `cdz-http-compile-request` — build the binary-AST body a client POSTs to the `/compile` route.
//!
//! The COMPILE route handler (`cdz-reducer-guest/compile-route`) decodes its request body as a canonical
//! `CompileRoute` value (reducer-targets B10c artifact-list design) —
//! `Value.decode(r.body) : Option(CompileRoute)` where:
//!   `Payload       = | Inline(Bytes) | CasRef(Bytes)`
//!   `RouteArtifact = | RouteArtifact(Record(kind: String, name: String, payload: Payload))`
//!   `CompileRoute  = | Compile(Record(artifacts: List(RouteArtifact)))`
//! The handler resolves each payload to bytes (CasRef → `blobs.get(hash)`, Inline → bytes as-is) and hands
//! rcdzc the bytes-only `Artifact(kind, name, bytes)` list — no per-kind special-casing. So the CLIENT
//! assembles the COMPLETE kinded artifact list rcdzc needs: `kind="ast"` per source module, a `kind="entry"`
//! naming the entrypoint, and — to compile a reducer-world GUEST (a router / handler) — a `kind="wit-world"`
//! artifact carrying the world binary-AST that TYPES the guest's `on-message` boundary and links its
//! `run`/`blobs` imports. Any other kind rcdzc understands (spans → wasm-debug/dwarf, …) can be passed too.
//!
//! Payload choice: large blobs ride `CasRef` (a 33-byte hash the handler `blobs.get`s — `--ast`/`--wit-world`/
//! `--artifact`), so the request stays small; the entry name rides `Inline` (below).
//!
//! ENTRY ENCODING (the thing that 422'd the naive path): rcdzc reads the `KIND_ENTRY` artifact via
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
//!   cdz-http-compile-request --ast <NAME>=<HASH> [--ast …] --entry <NAME>
//!                            [--wit-world <NAME>=<HASH>] [--artifact <KIND>:<NAME>=<HASH>] [-o <FILE>]
//!
//! - `--ast <NAME>=<HASH>` (repeatable): a source module, emitted as a `kind="ast"` `CasRef` artifact.
//! - `--wit-world <NAME>=<HASH>`: the WIT world (e.g. `reducer-world`) that types a guest boundary — a
//!   `kind="wit-world"` `CasRef` artifact. Needed to `/compile` a reducer-world guest (router / handler).
//! - `--artifact <KIND>:<NAME>=<HASH>` (repeatable): a generic `kind=<KIND>` `CasRef` artifact — any other kind
//!   rcdzc understands (spans, …), so the tool needn't grow a flag per kind.
//! - `--entry <NAME>`: the entrypoint module name (must be one of the `--ast` names). Emitted as a
//!   `kind="entry"` `Inline` artifact whose bytes are `encode_name(<NAME>)`.
//! - `<HASH>` (for every CasRef flag) is `@<path>` (raw `Hash::LEN`=33 bytes from a saved `/parse` or CAS-PUT
//!   response body) or a base62 `Hash` string. A wrong-length / unparseable hash is a hard error.
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

/// One content-addressed artifact: a `kind`, a `name`, and the raw `Hash::LEN` bytes it derefs from the CAS.
/// Emitted as a `RouteArtifact{kind, name, payload=CasRef(hash)}`.
#[derive(Debug)]
struct CasArtifact {
    kind: String,
    name: String,
    hash: Vec<u8>,
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut entry: Option<String> = None;
    let mut out: Option<String> = None;
    let mut arts: Vec<CasArtifact> = Vec::new();

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
                arts.push(parse_kinded("ast", &spec)?);
            }
            "--wit-world" => {
                let spec = args.next().ok_or("--wit-world needs NAME=HASH")?;
                arts.push(parse_kinded("wit-world", &spec)?);
            }
            "--artifact" => {
                let spec = args.next().ok_or("--artifact needs KIND:NAME=HASH")?;
                arts.push(parse_artifact(&spec)?);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unexpected argument: {other}\n\n{USAGE}")),
        }
    }

    let entry = entry.ok_or("missing --entry <NAME>")?;
    let ast_names: Vec<&str> = arts
        .iter()
        .filter(|a| a.kind == "ast")
        .map(|a| a.name.as_str())
        .collect();
    if ast_names.is_empty() {
        return Err("at least one --ast <NAME>=<HASH> is required".to_string());
    }
    if !ast_names.contains(&entry.as_str()) {
        return Err(format!(
            "--entry {entry:?} is not among the --ast names ({})",
            ast_names.join(", ")
        ));
    }

    let body = encode_compile_route(&arts, &entry);

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

/// Parse `NAME=HASH` into a `CasArtifact` of the given fixed `kind` (the `--ast` / `--wit-world` shape).
fn parse_kinded(kind: &str, spec: &str) -> Result<CasArtifact, String> {
    let (name, hash) = spec
        .split_once('=')
        .ok_or_else(|| format!("--{kind} must be NAME=HASH, got {spec:?}"))?;
    if name.is_empty() {
        return Err(format!("--{kind} NAME is empty in {spec:?}"));
    }
    Ok(CasArtifact {
        kind: kind.to_string(),
        name: name.to_string(),
        hash: resolve_hash(hash, name)?,
    })
}

/// Parse the generic `--artifact KIND:NAME=HASH` into a `CasArtifact` of an arbitrary `kind`.
fn parse_artifact(spec: &str) -> Result<CasArtifact, String> {
    let (kinded, hash) = spec
        .split_once('=')
        .ok_or_else(|| format!("--artifact must be KIND:NAME=HASH, got {spec:?}"))?;
    let (kind, name) = kinded
        .split_once(':')
        .ok_or_else(|| format!("--artifact must be KIND:NAME=HASH, got {spec:?}"))?;
    if kind.is_empty() || name.is_empty() {
        return Err(format!(
            "--artifact KIND and NAME must be non-empty in {spec:?}"
        ));
    }
    Ok(CasArtifact {
        kind: kind.to_string(),
        name: name.to_string(),
        hash: resolve_hash(hash, name)?,
    })
}

/// Resolve a `<HASH>` token to the raw `Hash::LEN` bytes: `@<path>` reads raw bytes from a file (a saved PARSE
/// or CAS-PUT response body); anything else is parsed as a base62 `Hash` string. Validated to be exactly
/// `Hash::LEN` bytes so a bad hash fails here, not as a silent store miss at `/compile` time.
fn resolve_hash(hash: &str, artifact: &str) -> Result<Vec<u8>, String> {
    let bytes = if let Some(path) = hash.strip_prefix('@') {
        std::fs::read(path)
            .map_err(|e| format!("artifact {artifact:?}: cannot read hash file {path}: {e}"))?
    } else {
        let parsed = Hash::from_str(hash).map_err(|_| {
            format!("artifact {artifact:?}: {hash:?} is neither @<file> nor a valid base62 hash")
        })?;
        parsed.as_bytes().to_vec()
    };
    if bytes.len() != Hash::LEN {
        return Err(format!(
            "artifact {artifact:?}: ast-hash is {} bytes, expected {} (a raw ProgramHash) — did you pass a base62 string as a raw file, or a truncated body?",
            bytes.len(),
            Hash::LEN
        ));
    }
    Ok(bytes)
}

/// Build the canonical `CompileRoute.Compile(Record(artifacts))` value form and serialize it to the binary-AST
/// bytes `Value.decode : Option(CompileRoute)` accepts. Each `RouteArtifact` newtype is ascription-free (decodes by shape via frame-tolerant Value.decode); `Payload` is a multi-ctor sum so `Inline`/`CasRef` keep their constructor; record fields are
/// canonicalized by `value::record`. Every CAS artifact rides `CasRef(raw-33-hash)`; the entry rides
/// `Inline(encode_name(entry))` (the codec `Str`-leaf name wire rcdzc's `decode_name` reads).
fn encode_compile_route(arts: &[CasArtifact], entry: &str) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let mut elems: Vec<value::ValueId> = arts
        .iter()
        .map(|a| {
            let hash = value::bytes_leaf(&mut b, &a.hash);
            let payload = value::bare_ctor(&mut b, "CasRef", vec![hash]);
            route_artifact(&mut b, &a.kind, &a.name, payload)
        })
        .collect();
    let name_wire = value::bytes_leaf(&mut b, &encode_name(entry));
    let entry_payload = value::bare_ctor(&mut b, "Inline", vec![name_wire]);
    elems.push(route_artifact(&mut b, "entry", "", entry_payload));

    let artifacts = value::list_value(&mut b, elems);
    let rec = value::record(&mut b, vec![("artifacts", artifacts)]);
    // `Compile` ctor elided (nominal newtype); ascription-free encode (finish_value) — `Value.decode` is
    // frame-tolerant (v-value-codec #8790), so the bare record decodes against `CompileRoute` by shape.
    value::finish_value(b, rec).to_vec()
}

/// `RouteArtifact.RouteArtifact(Record(kind, name, payload))` — ctor elided, ascription-free (decodes by shape).
fn route_artifact(
    b: &mut ValueBuilder,
    kind: &str,
    name: &str,
    payload: value::ValueId,
) -> value::ValueId {
    let kind = value::str_leaf(b, kind);
    let name = value::str_leaf(b, name);
    // Ascription-free: the bare record decodes against `RouteArtifact` by shape (frame-tolerant Value.decode).
    value::record(
        b,
        vec![("kind", kind), ("name", name), ("payload", payload)],
    )
}

const USAGE: &str = "\
cdz-http-compile-request — build the /compile route's binary-AST request body (artifact-list form).

Usage:
  cdz-http-compile-request --ast <NAME>=<HASH> [--ast …] --entry <NAME>
                           [--wit-world <NAME>=<HASH>] [--artifact <KIND>:<NAME>=<HASH>] [-o <FILE>]

  --ast      <NAME>=<HASH>        a source module <NAME>; kind=\"ast\" CasRef artifact; repeatable.
  --wit-world <NAME>=<HASH>       a WIT world (e.g. reducer-world) that types a guest boundary; kind=\"wit-world\"
                                  CasRef artifact. Needed to /compile a reducer-world guest (router / handler).
  --artifact <KIND>:<NAME>=<HASH> a generic kind=<KIND> CasRef artifact (spans, …); repeatable.
  --entry    <NAME>               the entrypoint module name (must be one of the --ast names). kind=\"entry\"
                                  Inline artifact whose bytes are encode_name(<NAME>).
  <HASH>                          @<path> (raw 33-byte ProgramHash from a saved PARSE/CAS-PUT body) or a base62
                                  Hash string.
  -o, --out <FILE>                write the body to <FILE> (default: stdout).
";

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_http_protocol::value;

    fn hash_of(byte: u8) -> Vec<u8> {
        vec![byte; Hash::LEN]
    }

    struct Decoded {
        kind: String,
        name: String,
        ctor: String,
        bytes: Vec<u8>,
    }

    fn decode_artifacts(bytes: &[u8]) -> Vec<Decoded> {
        let arenas = value::decode(bytes).expect("output decodes as a binary-AST value");
        // Root is `(: #record CompileRoute)` (Compile ctor elided) — unascribe, read `artifacts`.
        let root = value::unascribe(&arenas, arenas.root);
        let artifacts = value::record_field(&arenas, root, "artifacts")
            .expect("a CompileRoute has an `artifacts` field");
        let elems = value::read_list(&arenas, artifacts).expect("`artifacts` is a list");
        elems
            .iter()
            .map(|&e| {
                let rec = value::unascribe(&arenas, e);
                let field = |f: &str| value::record_field(&arenas, rec, f).unwrap();
                let payload = field("payload");
                Decoded {
                    kind: value::read_str(&arenas, field("kind")).unwrap(),
                    name: value::read_str(&arenas, field("name")).unwrap(),
                    ctor: value::read_ctor(&arenas, payload).unwrap().to_string(),
                    bytes: value::read_bytes(
                        &arenas,
                        value::ctor_payload(&arenas, payload).unwrap()[0],
                    )
                    .unwrap()
                    .to_vec(),
                }
            })
            .collect()
    }

    /// The client-encoding contract is subtle (field order, the per-element `RouteArtifact` ascription decode
    /// requires, the `Payload` constructors, and the entry's binary-AST name wire — the things that drove the
    /// multi-module 422). Lock it: an `--ast`, a `--wit-world`, and the entry all round-trip to the expected
    /// kinded artifacts, with CAS artifacts as `CasRef(raw-hash)` and the entry as `Inline(encode_name(entry))`.
    #[test]
    fn encode_compile_route_round_trips_ast_witworld_and_entry() {
        let arts = vec![
            CasArtifact {
                kind: "ast".to_string(),
                name: "router".to_string(),
                hash: hash_of(1),
            },
            CasArtifact {
                kind: "wit-world".to_string(),
                name: "reducer-world".to_string(),
                hash: hash_of(2),
            },
        ];
        let bytes = encode_compile_route(&arts, "router");
        let seen = decode_artifacts(&bytes);
        assert_eq!(seen.len(), 3, "one ast + one wit-world + one entry");

        // The source module → a CasRef `kind="ast"` artifact carrying its raw hash verbatim.
        assert!(seen.iter().any(|d| d.kind == "ast"
            && d.name == "router"
            && d.ctor == "CasRef"
            && d.bytes == hash_of(1)));
        // The WIT world → a CasRef `kind="wit-world"` artifact (types the guest boundary at rcdzc).
        assert!(seen.iter().any(|d| d.kind == "wit-world"
            && d.name == "reducer-world"
            && d.ctor == "CasRef"
            && d.bytes == hash_of(2)));
        // The entry → an Inline artifact whose bytes are the binary-AST name wire (encode_name), which rcdzc
        // reads with decode_name — the exact encoding whose raw-UTF-8 form 422'd the multi-module path.
        let entry = seen
            .iter()
            .find(|d| d.kind == "entry")
            .expect("an entry artifact");
        assert_eq!(entry.ctor, "Inline", "entry rides Inline");
        assert_eq!(
            entry.bytes,
            encode_name("router"),
            "entry bytes are encode_name(entry)"
        );
        assert_eq!(
            cadenza_compile_abi::decode_name(&entry.bytes).as_deref(),
            Some("router"),
            "and they decode back through the name wire"
        );
    }

    /// The generic `--artifact KIND:NAME=HASH` requires the `KIND:NAME` colon (checked before hash validity).
    #[test]
    fn generic_artifact_flag_requires_kind_colon_name() {
        // No colon in the KIND:NAME part → a clear parse error, independent of hash validity.
        let err = parse_artifact("router=deadbeef").unwrap_err();
        assert!(err.contains("KIND:NAME"), "got: {err}");
    }

    /// A wrong-length raw hash fails LOUDLY at parse time (not as a silent per-module store miss at /compile).
    #[test]
    fn a_missing_raw_hash_file_is_rejected() {
        let err = resolve_hash("@/nonexistent/path/to/hash", "router").unwrap_err();
        assert!(err.contains("cannot read hash file"), "got: {err}");
    }
}
