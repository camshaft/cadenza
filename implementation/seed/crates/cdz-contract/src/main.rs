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
//! cdz-contract hash <dir> [--cdz <path>] [--out <file>] [--lib <lib.cdz>]…
//! cdz-contract id <file.cdz> [--cdz <path>] [--lib <lib.cdz>]…
//! cdz-contract blob <file>
//! ```
//!
//! `hash` reads every `*.cdz` under `<dir>` and, for each, COMPILES + EXECUTES its `descriptor()` and reads
//! the contract's name + id from the folded descriptor record (operator 2026-08-27: the identity flows
//! through the guest's own `descriptor()` self-reflection — no `@!contract`/`@!input`/`@!output` pragmas). It
//! emits a JSON object `{ "<name>": "<id>" }` sorted by name, the id base62 (§8, the one text form). Each
//! `.cdz` must be a runnable contract (a non-contract is a hard error, not a silent skip). `--lib` supplies
//! the library module(s) a contract imports (the platform contracts `import … from "contract-id"`, whose lib
//! lives in `guests/`), compiled alongside each contract. `--cdz` sets the `cdz` binary (else `$CDZ`, else
//! `cdz` on `PATH`); `--out` writes the mapping to a file instead of stdout. (Executing a contract needs the
//! value-heap runtime resolvable by `cdz run` — the caller/build provides the store.)
//!
//! `blob` reads one `<file>` and prints its raw content address — `Hash::of(HashTag::Blob, bytes)` rendered
//! base62 (§8), no trailing newline — the SAME string `cdz-run`'s and `xtask`'s `content_address` produce and
//! the store's `put()` keys by. It exists so the nix build (`flake.nix` `hashOf` / the component store / the
//! runtime-hash parity check) names an artifact by the exact platform content address, not a bare `b3sum`
//! hex that would disagree with the tagged base62 the compiler pins in a `+<hash>` import.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(msg)) => {
            eprintln!(
                "cdz-contract: {msg}\n\nusage: cdz-contract hash <dir> [--cdz <path>] [--out <file>] [--lib <lib.cdz>]...\n       cdz-contract id <file.cdz> [--cdz <path>] [--lib <lib.cdz>]...\n       cdz-contract blob <file>"
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
    libs: Vec<PathBuf>,
}

fn run(args: &[String]) -> Result<(), Error> {
    match args.first().map(String::as_str) {
        Some("hash") => hash(parse_hash(&args[1..])?),
        Some("id") => id(parse_id(&args[1..])?),
        Some("blob") => blob(&args[1..]),
        Some(other) => Err(Error::Usage(format!("unknown subcommand `{other}`"))),
        None => Err(Error::Usage("no subcommand given".into())),
    }
}

/// The parsed `id` invocation: one contract source file, the `cdz` binary, and the library module(s) the
/// contract imports (`--lib`, repeatable — e.g. `guests/contract-id.cdz`), needed to compile + execute it.
struct IdArgs {
    file: PathBuf,
    cdz: String,
    libs: Vec<PathBuf>,
}

fn parse_id(args: &[String]) -> Result<IdArgs, Error> {
    let mut file: Option<PathBuf> = None;
    let mut cdz: Option<String> = None;
    let mut libs: Vec<PathBuf> = Vec::new();
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
            "--lib" => {
                libs.push(PathBuf::from(
                    it.next()
                        .ok_or_else(|| Error::Usage("--lib needs a path".into()))?,
                ));
            }
            flag if flag.starts_with('-') => {
                return Err(Error::Usage(format!("unknown option `{flag}`")));
            }
            positional if file.is_none() => file = Some(PathBuf::from(positional)),
            extra => return Err(Error::Usage(format!("unexpected argument `{extra}`"))),
        }
    }
    Ok(IdArgs {
        file: file.ok_or_else(|| Error::Usage("missing <file>".into()))?,
        cdz: cdz
            .or_else(|| std::env::var("CDZ").ok())
            .unwrap_or_else(|| "cdz".into()),
        libs,
    })
}

/// `cdz-contract id <file.cdz> [--lib <lib.cdz>]…` — print the base62 contract-id of the contract in `<file>`,
/// by COMPILING + EXECUTING its `descriptor()` and reading the id from the folded descriptor record (operator
/// 2026-08-27: "the codegen should call the compiled module, get the descriptor" — no `@!contract` pragmas).
/// The id is byte-identical to (1) the platform router's routing keys and (2) the self-reflecting guest's own
/// `descriptor().id` fold — the guest execution IS the single source of truth. `--lib` supplies the
/// `contract-id` library module the contract imports (it lives in `guests/`, not beside the contract). Exit 1
/// if `<file>` is not a runnable contract (does not compile, run, or export a descriptor record).
fn id(args: IdArgs) -> Result<(), Error> {
    let (_name, id) = compile_run_descriptor(&args.cdz, &args.file, &args.libs)?;
    println!("{id}");
    Ok(())
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

/// Parse `hash`'s arguments: a single positional `<dir>`, plus optional `--cdz <path>`, `--out <file>`, and
/// `--lib <lib.cdz>` (repeatable — the library module(s) the contracts import, compiled alongside each).
/// `--cdz` defaults to `$CDZ` then `cdz` (found on `PATH`).
fn parse_hash(args: &[String]) -> Result<HashArgs, Error> {
    let mut dir: Option<PathBuf> = None;
    let mut cdz: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut libs: Vec<PathBuf> = Vec::new();
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
            "--lib" => {
                libs.push(PathBuf::from(
                    it.next()
                        .ok_or_else(|| Error::Usage("--lib needs a path".into()))?,
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
        libs,
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
        // Each `.cdz` under a contracts dir is a contract: compile + execute its `descriptor()` and read its
        // name + id from the folded record (no `@!contract` pragma). A `.cdz` that is not a runnable contract
        // is a hard error (it names no contract for the mapping) rather than being silently skipped, so a
        // broken contract surfaces here instead of as a later "not in the mapping" failure.
        let (name, id) = compile_run_descriptor(&args.cdz, src, &args.libs)?;
        if let Some(prev) = mapping.insert(name.clone(), id.to_string()) {
            return Err(Error::Failed(format!(
                "two contracts declare the name `{name}` (one is {prev}); names must be unique"
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

/// A contract's declared **name** and its [`contract-id`](cdz_contract::contract_id), obtained by COMPILING
/// and EXECUTING the contract's `descriptor()` (operator 2026-08-27: "the codegen should call the compiled
/// module, get the descriptor" — no `@!contract`/`@!input`/`@!output` pragmas, no Rust re-derivation). The
/// contract is compiled together with the `contract-id` library module(s) it imports (`libs` — the platform
/// contracts `import { contract-descriptor } from "contract-id"`, whose lib lives in `guests/`, so the caller
/// supplies it with `--lib`) into a component exporting `descriptor`, then run with `cdz run --format
/// binary-ast` — which emits the descriptor record as the canonical binary AST (the universal `cadenza-ast`
/// exchange form). The two `cdz` invocations are piped IN MEMORY (`compile … -o -` → `run - --format
/// binary-ast`), so no temp file is written and the crate stays dep-minimal. The emitted bytes are decoded
/// (`cadenza_ast::codec::decode`) and the id + name read out ([`cdz_contract::id_name_from_descriptor`]). A
/// spawn failure, a non-zero exit from either `cdz`, an undecodable doc, or a value that is not a descriptor
/// record is a hard error naming the source (a `.cdz` under the hashed dir must be a runnable contract).
fn compile_run_descriptor(
    cdz: &str,
    src: &Path,
    libs: &[PathBuf],
) -> Result<(String, cdz_contract::Hash), Error> {
    let src_str = src.to_str().ok_or_else(|| {
        Error::Failed(format!(
            "contract path {} is not valid UTF-8",
            src.display()
        ))
    })?;
    // `--entry <stem>` names which input file's `(export …)` forms the component boundary — the contract, not
    // the library. A source file's entry name is its stem (`deliver.cdz` → `deliver`).
    let stem = src.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
        Error::Failed(format!(
            "contract path {} has no usable stem",
            src.display()
        ))
    })?;

    // 1) Compile the contract + its imported lib(s) to a component on stdout (`-o -`).
    let mut compile_args: Vec<&str> = vec!["compile", src_str];
    let lib_strs: Vec<&str> = libs
        .iter()
        .map(|l| {
            l.to_str().ok_or_else(|| {
                Error::Failed(format!("lib path {} is not valid UTF-8", l.display()))
            })
        })
        .collect::<Result<_, _>>()?;
    compile_args.extend(lib_strs.iter().copied());
    compile_args.extend(["--entry", stem, "-o", "-"]);
    let compiled = Command::new(cdz)
        .args(&compile_args)
        .output()
        .map_err(|e| Error::Failed(format!("running `{cdz} compile {src_str} …`: {e}")))?;
    if !compiled.status.success() {
        return Err(Error::Failed(format!(
            "`{cdz} compile {src_str} …` failed: {}",
            String::from_utf8_lossy(&compiled.stderr).trim()
        )));
    }

    // 2) Run the component (from stdin, `-`), emitting the descriptor record as canonical binary AST.
    let mut child = Command::new(cdz)
        .args(["run", "-", "--format", "binary-ast"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Failed(format!("running `{cdz} run - --format binary-ast`: {e}")))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(&compiled.stdout)
        .map_err(|e| Error::Failed(format!("feeding the component to `{cdz} run`: {e}")))?;
    let ran = child
        .wait_with_output()
        .map_err(|e| Error::Failed(format!("waiting on `{cdz} run`: {e}")))?;
    if !ran.status.success() {
        return Err(Error::Failed(format!(
            "`{cdz} run - --format binary-ast` failed for {src_str}: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        )));
    }

    // 3) Decode the descriptor value form and read (name, contract-id) out of it.
    let value = cadenza_ast::codec::decode(&ran.stdout).ok_or_else(|| {
        Error::Failed(format!(
            "descriptor() of {src_str} did not emit a decodable value form"
        ))
    })?;
    cdz_contract::id_name_from_descriptor(&value).ok_or_else(|| {
        Error::Failed(format!(
            "the descriptor() of {src_str} is not a contract descriptor record (id + name)"
        ))
    })
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
