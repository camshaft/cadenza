//! `cdz-contract` — the contract tooling CLI (`design/cadenza-platform.md` §1).
//!
//! The command-line front end over the [`cdz_contract`] library: turn a directory of contract sources into a
//! name→contract-hash mapping (operator directive 2026-08-23 — the mapping is produced OUTSIDE the platform,
//! by nix invoking this, and fed to a run as data; the platform resolves only by hash). The library does the
//! hashing; this binary is the I/O shell — it walks a directory, parses each contract with the `cdz` binary,
//! and prints the mapping. The library stays dep-minimal (so it can become a wasm component); this binary is
//! the non-wasm surface, so its filesystem + subprocess use lives only here.
//!
//! ```text
//! cdz-contract hash <dir> [--cdz <path>] [--out <file>]
//! cdz-contract blob <file>
//! ```
//!
//! `hash` reads every `*.cdz` under `<dir>`, and for each contract module (one carrying the `@!contract` /
//! `@!input` / `@!output` pragmas) computes its contract-id, emitting a JSON object `{ "<name>": "<id>" }`
//! sorted by name, where the id is the base62 of the contract-id (§8, the one text form). A `.cdz` that is
//! a valid module but declares no contract is skipped; a source the `cdz` binary cannot parse is a hard
//! error. `--cdz` sets the parser binary (else `$CDZ`, else `cdz` on `PATH`); `--out` writes the mapping to a
//! file instead of stdout.
//!
//! `blob` reads one `<file>` and prints its raw content address — `Hash::of(HashTag::Blob, bytes)` rendered
//! base62 (§8), no trailing newline — the SAME string `cdz-run`'s and `xtask`'s `content_address` produce and
//! the store's `put()` keys by. It exists so the nix build (`flake.nix` `hashOf` / the component store / the
//! runtime-hash parity check) names an artifact by the exact platform content address, not a bare `b3sum`
//! hex that would disagree with the tagged base62 the compiler pins in a `+<hash>` import.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(msg)) => {
            eprintln!(
                "cdz-contract: {msg}\n\nusage: cdz-contract hash <dir> [--cdz <path>] [--out <file>]\n       cdz-contract blob <file>"
            );
            ExitCode::from(2)
        }
        Err(Error::Failed(msg)) => {
            eprintln!("cdz-contract: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// A CLI failure: a misuse (wrong arguments — exit 2) or a run failure (a source that would not parse, an I/O
/// error — exit 1). Kept separate so `main` maps each to the conventional exit code.
enum Error {
    Usage(String),
    Failed(String),
}

/// The parsed `hash` invocation.
struct HashArgs {
    dir: PathBuf,
    cdz: String,
    out: Option<PathBuf>,
}

fn run(args: &[String]) -> Result<(), Error> {
    match args.first().map(String::as_str) {
        Some("hash") => hash(parse_hash(&args[1..])?),
        Some("blob") => blob(&args[1..]),
        Some(other) => Err(Error::Usage(format!("unknown subcommand `{other}`"))),
        None => Err(Error::Usage("no subcommand given".into())),
    }
}

/// `blob <file>`: print the raw content address of `<file>` — `Hash::of(HashTag::Blob, bytes)` rendered
/// base62 (§8), no trailing newline. The one text form every content-address producer emits (this,
/// `cdz-run`/`xtask` `content_address`, the store's `put()`), so the nix build can name an artifact by the
/// exact platform address a `+<hash>` import pins. Exactly one positional argument; no options.
fn blob(args: &[String]) -> Result<(), Error> {
    let [path] = args else {
        return Err(Error::Usage(
            "blob takes exactly one <file> argument".into(),
        ));
    };
    let bytes = std::fs::read(path).map_err(|e| Error::Failed(format!("reading {path}: {e}")))?;
    print!(
        "{}",
        cdz_contract::Hash::of(cdz_contract::HashTag::Blob, &bytes)
    );
    Ok(())
}

/// Parse `hash`'s arguments: a single positional `<dir>`, plus optional `--cdz <path>` and `--out <file>`.
/// `--cdz` defaults to `$CDZ` then `cdz` (found on `PATH`).
fn parse_hash(args: &[String]) -> Result<HashArgs, Error> {
    let mut dir: Option<PathBuf> = None;
    let mut cdz: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--cdz" => {
                cdz = Some(
                    it.next()
                        .ok_or_else(|| Error::Usage("--cdz needs a path".into()))?
                        .clone(),
                );
            }
            "--out" => {
                out = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| Error::Usage("--out needs a file".into()))?,
                ));
            }
            flag if flag.starts_with('-') => {
                return Err(Error::Usage(format!("unknown option `{flag}`")));
            }
            positional if dir.is_none() => dir = Some(PathBuf::from(positional)),
            extra => return Err(Error::Usage(format!("unexpected argument `{extra}`"))),
        }
    }
    Ok(HashArgs {
        dir: dir.ok_or_else(|| Error::Usage("missing <dir>".into()))?,
        cdz: cdz
            .or_else(|| std::env::var("CDZ").ok())
            .unwrap_or_else(|| "cdz".into()),
        out,
    })
}

/// Hash every contract under `dir` and emit the name→hash JSON mapping. Deterministic: files are visited in
/// sorted path order and the mapping is emitted sorted by contract name, so the output is byte-stable across
/// runs (reproducible for a content-addressed build).
fn hash(args: HashArgs) -> Result<(), Error> {
    let mut sources = Vec::new();
    collect_cdz(&args.dir, &mut sources)?;
    sources.sort();

    // name → base62 id. A BTreeMap keeps the emission sorted by name and rejects a duplicate name
    // (two contracts declaring the same name is an authoring error, not a silent last-wins).
    let mut mapping: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for src in &sources {
        let ast = convert(&args.cdz, src)?;
        let arenas = cadenza_ast::codec::decode(&ast).ok_or_else(|| {
            Error::Failed(format!(
                "`cdz convert` of {} produced an undecodable AST",
                src.display()
            ))
        })?;
        // A valid module that declares no contract (no `@!contract` pragma) is not an error — a directory may
        // hold non-contract `.cdz`; it is simply not in the mapping.
        let Some((name, id)) = cdz_contract::contract_from_module(&arenas) else {
            continue;
        };
        if let Some(prev) = mapping.insert(name.to_string(), id.to_string()) {
            return Err(Error::Failed(format!(
                "two contracts declare the name `{name}` (one is {}); names must be unique",
                prev
            )));
        }
    }

    let json = render_json(&mapping);
    match &args.out {
        Some(path) => std::fs::write(path, json)
            .map_err(|e| Error::Failed(format!("writing {}: {e}", path.display())))?,
        None => print!("{json}"),
    }
    Ok(())
}

/// Recursively collect every `*.cdz` file under `dir` into `out`.
fn collect_cdz(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Error::Failed(format!("reading directory {}: {e}", dir.display())))?;
    for entry in entries {
        let path = entry
            .map_err(|e| Error::Failed(format!("reading an entry in {}: {e}", dir.display())))?
            .path();
        if path.is_dir() {
            collect_cdz(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "cdz") {
            out.push(path);
        }
    }
    Ok(())
}

/// Parse a contract source to its canonical binary AST via the `cdz` binary (`cdz convert <src> --to
/// binary`, the same invocation `xtask codegen` uses), returning the AST bytes. Delegating the parse to the
/// pinned binary keeps this crate off the compiler (the first cut; a compiler-as-library parse is the later
/// wasm-component path). A non-zero exit or a spawn failure is a hard error naming the source.
fn convert(cdz: &str, src: &Path) -> Result<Vec<u8>, Error> {
    let src_str = src.to_str().ok_or_else(|| {
        Error::Failed(format!(
            "contract path {} is not valid UTF-8",
            src.display()
        ))
    })?;
    let output = Command::new(cdz)
        .args(["convert", src_str, "--to", "binary"])
        .output()
        .map_err(|e| {
            Error::Failed(format!(
                "running `{cdz} convert {src_str} --to binary`: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Failed(format!(
            "`{cdz} convert {src_str} --to binary` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

/// Render the name→id mapping as a JSON object, one entry per line, sorted by name (the `BTreeMap`'s order).
/// Hand-rolled to keep the crate dep-minimal: an id is base62 (`[0-9A-Za-z]`, no escaping), and a name
/// only needs `"` and `\` escaped.
fn render_json(mapping: &std::collections::BTreeMap<String, String>) -> String {
    if mapping.is_empty() {
        return "{}\n".to_string();
    }
    let mut s = String::from("{\n");
    for (i, (name, id)) in mapping.iter().enumerate() {
        let comma = if i + 1 == mapping.len() { "" } else { "," };
        s.push_str(&format!("  \"{}\": \"{id}\"{comma}\n", escape(name)));
    }
    s.push_str("}\n");
    s
}

/// Escape a JSON string's `"` and `\` (a contract name is otherwise plain text).
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{Error, blob, escape, render_json};
    use std::collections::BTreeMap;

    #[test]
    fn blob_requires_exactly_one_file_argument() {
        // No file / two files is a usage error (exit 2), not a run failure — the arg shape is fixed.
        assert!(matches!(blob(&[]), Err(Error::Usage(_))));
        assert!(matches!(
            blob(&["a".to_string(), "b".to_string()]),
            Err(Error::Usage(_))
        ));
        // A path that does not exist is a run failure (I/O), naming the file.
        assert!(matches!(
            blob(&["/no/such/blob/file".to_string()]),
            Err(Error::Failed(_))
        ));
    }

    #[test]
    fn json_is_sorted_by_name_and_well_formed() {
        // Insertion order differs from sorted order; the BTreeMap emits sorted, so the output is stable.
        let mut m = BTreeMap::new();
        m.insert("cdz-platform.timer".to_string(), "AAAA".to_string());
        m.insert("cdz-platform.deliver".to_string(), "BBBB".to_string());
        assert_eq!(
            render_json(&m),
            "{\n  \"cdz-platform.deliver\": \"BBBB\",\n  \"cdz-platform.timer\": \"AAAA\"\n}\n"
        );
    }

    #[test]
    fn empty_mapping_is_an_empty_object() {
        assert_eq!(render_json(&BTreeMap::new()), "{}\n");
    }

    #[test]
    fn a_name_with_quotes_or_backslashes_is_escaped() {
        assert_eq!(escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }
}
