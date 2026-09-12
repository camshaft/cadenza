//! cdz-source-export — project the Cadenza reducer-runtime pieces into a self-contained,
//! vendorable Rust source tree (single source of truth, re-runnable, NOT a fork).
//!
//! I1 capability: project the LIGHT binary-AST value-codec tier — the crates `cadenza-ast`,
//! `cadenza-value`, `cadenza-ast-serde` — into a standalone cargo workspace under `--out`.
//!
//! Drift-proof by reading the SAME pinned inputs the flake builds from:
//!   * the crate SOURCES are copied verbatim out of the repo, and
//!   * the projected `Cargo.lock` is DERIVED by FILTERING the repo's pinned root `Cargo.lock` to the
//!     transitive dependency closure of those crates — no fresh resolution, so the emitted versions
//!     are exactly the ones the nix build pins (`seedCargoVendor = importCargoLock ./Cargo.lock`).
//!
//! Usage:
//!   cdz-source-export [--repo <repo-root>] --out <dir> [--tier codec|reducer]
//!
//! `--repo` defaults to the current directory. `--out` is created (must not already be a non-empty
//! tree we would clobber — it is emptied first). Tiers are kept SEPARATE so the heavy wasmtime deps
//! of the reducer tier never leak into the light codec path:
//!   * `codec`   — the LIGHT binary-AST value-codec tier (no wasmtime/tokio/network).
//!   * `reducer` — the HEAVY reducer-world tier: the wasmtime driving + WIT world + host-import traits.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The ROOT crates that define each projection tier — the tier's purpose surface. The full first-party
/// crate set to project (their transitive sibling path-dep closure) is DISCOVERED from these at run
/// time (see `discover_members`), so the projected member set auto-adapts if an upstream crate gains a
/// first-party dep — the projection stays drift-proof for the crate SET, not just the pinned versions.
fn tier_roots(tier: &str) -> Result<&'static [&'static str], String> {
    match tier {
        // LIGHT binary-AST value-codec tier: no wasmtime/tokio/network; `cadenza-ast`'s core is no_std.
        // All three are the consumer surface (cadenza-ast's codec + cadenza-value's forms + the serde bridge).
        "codec" => Ok(&["cadenza-ast", "cadenza-value", "cadenza-ast-serde"]),
        // HEAVY reducer-world tier: `cdz-platform` carries the wasmtime driving (src/host.rs —
        // ReducerHost/WasmReducer/WasmProgramStore, behind its `host` feature) + the reducer-world WIT
        // (wit/world.wit, copied verbatim as it lives inside the crate dir) + the swappable
        // BlobStore/KvStore/ReducerGraph host-import traits. Its first-party deps (cadenza-ast/cdz-contract/
        // cdz-str) are discovered. Build the projected tree with `--features cdz-platform/host`.
        "reducer" => Ok(&["cdz-platform"]),
        other => Err(format!(
            "unknown --tier {other:?} (known tiers: codec, reducer)"
        )),
    }
}

/// Where the first-party crate sources live under the repo root.
const SEED_CRATES: &str = "implementation/seed/crates";

/// Discover the transitive first-party crate set to project, starting from a tier's root crates: a BFS
/// over each crate's `Cargo.toml` sibling path-dependencies (`path = "../<name>"`). Returns the roots
/// first, then discovered deps, deduped — so the projected member set (and the lock-closure roots)
/// auto-adapts to an upstream first-party-dep change without editing this tool.
fn discover_members(repo: &Path, roots: &[&str]) -> Result<Vec<String>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: std::collections::VecDeque<String> =
        roots.iter().map(|s| s.to_string()).collect();
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        order.push(name.clone());
        let manifest = repo.join(SEED_CRATES).join(&name).join("Cargo.toml");
        let text = read(&manifest)?;
        for dep in sibling_path_deps(&text) {
            if !seen.contains(&dep) {
                queue.push_back(dep);
            }
        }
    }
    Ok(order)
}

/// Extract the sibling-crate names from a `Cargo.toml`'s `path = "../<name>"` dependency entries. Only
/// `../`-prefixed paths (the seed's first-party path-dep convention) count as first-party crates, so a
/// `[[bin]]`/`[lib]` `path = "src/..."` target is naturally excluded (it is not `../`-prefixed).
fn sibling_path_deps(manifest: &str) -> Vec<String> {
    let needle = "path = \"../";
    let mut deps = Vec::new();
    let mut rest = manifest;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        match after.find('"') {
            Some(end) => {
                // `after[..end]` is the path with the leading `../` already consumed by the needle; the
                // crate name is its final component (handles `../x` and `../../a/x` alike).
                if let Some(name) = after[..end].rsplit('/').next() {
                    if !name.is_empty() {
                        deps.push(name.to_string());
                    }
                }
                rest = &after[end..];
            }
            None => break,
        }
    }
    deps
}

fn main() -> ExitCode {
    match run() {
        Ok((tier, out)) => {
            eprintln!(
                "cdz-source-export: projected {tier} tier -> {}",
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cdz-source-export: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(String, PathBuf), String> {
    let args = Args::parse(std::env::args().skip(1))?;
    let repo = args.repo;
    let out = args.out;

    // Discover the full first-party crate set (tier roots + their transitive sibling path-deps).
    let members = discover_members(&repo, tier_roots(&args.tier)?)?;
    let crates: Vec<&str> = members.iter().map(String::as_str).collect();

    // 1. Filter the pinned root Cargo.lock to the tier crates' transitive closure.
    let lock_path = repo.join("Cargo.lock");
    let lock_text = read(&lock_path)?;
    let filtered =
        filter_lock(&lock_text, &crates).map_err(|e| format!("{}: {e}", lock_path.display()))?;

    // 2. Assemble a fresh output tree (crate sources copied verbatim, incl. any wit/ dir inside them).
    reset_dir(&out)?;
    let crates_dir = out.join("crates");
    mkdir(&crates_dir)?;
    for crate_name in &crates {
        let src = repo.join(SEED_CRATES).join(crate_name);
        if !src.is_dir() {
            return Err(format!("missing tier crate source: {}", src.display()));
        }
        copy_crate(&src, &crates_dir.join(crate_name))?;
    }

    // 3. Write the projected workspace manifest, the filtered lock, and the refresh doc.
    write(&out.join("Cargo.toml"), &workspace_manifest(&crates))?;
    write(&out.join("Cargo.lock"), &filtered)?;
    write(&out.join("REFRESH.md"), &refresh_doc(&args.tier, &crates))?;

    Ok((args.tier, out))
}

// ---------------------------------------------------------------------------------------------
// Cargo.lock closure filtering (pure — unit-tested below)
// ---------------------------------------------------------------------------------------------

/// A reference to a dependency by name and (when the name is ambiguous across versions) version —
/// exactly the two forms a `Cargo.lock` `dependencies` entry takes: `"name"` or `"name version"`.
#[derive(Clone)]
struct Dep {
    name: String,
    version: Option<String>,
}

/// A single `[[package]]` block, kept as its verbatim source text plus its parsed identity/deps.
struct Block {
    name: String,
    version: String,
    deps: Vec<Dep>,
    /// The exact block text, from its `[[package]]` line up to (not including) the next block — so
    /// concatenating retained blocks reproduces the input byte-for-byte for those packages.
    text: String,
}

/// Split a `Cargo.lock` into its preamble (everything before the first `[[package]]`) and its blocks.
fn parse_lock(lock: &str) -> Result<(String, Vec<Block>), String> {
    let marker = "[[package]]";
    let first = match lock.find(marker) {
        Some(i) => i,
        None => return Err("no [[package]] entries found".into()),
    };
    // The preamble runs up to the newline that precedes the first marker line.
    let pre_end = lock[..first].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let preamble = lock[..pre_end].to_string();

    let body = &lock[pre_end..];
    // Block boundaries: each line that is exactly `[[package]]` starts a new block.
    let mut starts = Vec::new();
    let mut idx = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == marker {
            starts.push(idx);
        }
        idx += line.len();
    }
    let mut blocks = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(body.len());
        let text = body[start..end].to_string();
        let (name, version, deps) = parse_block(&text)?;
        blocks.push(Block {
            name,
            version,
            deps,
            text,
        });
    }
    Ok((preamble, blocks))
}

/// Parse a single block's `name`, `version`, and its dependency references (`name` [+ `version`]).
fn parse_block(text: &str) -> Result<(String, String, Vec<Dep>), String> {
    let mut name = None;
    let mut version = None;
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if in_deps {
            if t.starts_with(']') {
                in_deps = false;
                continue;
            }
            if let Some(dep) = dep_ref(t) {
                deps.push(dep);
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("name = ") {
            name = Some(unquote(rest));
        } else if let Some(rest) = t.strip_prefix("version = ") {
            version = Some(unquote(rest));
        } else if t.starts_with("dependencies = [") {
            in_deps = true;
            // A dependency may sit on the same line for a single-entry inline array (rare in
            // Cargo.lock, which pretty-prints, but handle it): `dependencies = ["foo"]`.
            if let Some(inner) = t
                .strip_prefix("dependencies = [")
                .and_then(|s| s.strip_suffix(']'))
            {
                in_deps = false;
                for part in inner.split(',') {
                    if let Some(dep) = dep_ref(part.trim()) {
                        deps.push(dep);
                    }
                }
            }
        }
    }
    match (name, version) {
        (Some(n), Some(v)) => Ok((n, v, deps)),
        (None, _) => Err("a [[package]] block has no name".into()),
        (Some(n), None) => Err(format!("package {n:?} has no version")),
    }
}

/// A dependency array entry is a quoted string `"name"` / `"name version"` / `"name version (src)"`.
/// The name is the first token; the version (present only when the name is ambiguous across multiple
/// locked versions) is the second — used to resolve to the EXACT block, so the closure is minimal.
fn dep_ref(entry: &str) -> Option<Dep> {
    let s = entry.trim().trim_end_matches(',');
    if !s.starts_with('"') {
        return None;
    }
    let inner = s.trim_matches('"');
    let mut toks = inner.split_whitespace();
    let name = toks.next()?.to_string();
    let version = toks.next().map(|w| w.to_string());
    Some(Dep { name, version })
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// The identity of a locked package: `(name, version)`.
type Id = (String, String);

/// Compute the transitive dependency closure of `roots` over the lock graph, and re-emit a valid
/// `Cargo.lock` containing the preamble plus exactly the closure's blocks in their original order.
/// Resolution is VERSION-AWARE so duplicate-version crates (e.g. three `syn`s in the root lock) do
/// not over-include unreachable versions — the emitted lock is minimal and safe for `--locked`.
fn filter_lock(lock: &str, roots: &[&str]) -> Result<String, String> {
    let (preamble, blocks) = parse_lock(lock)?;

    // name -> the versions locked under it (to resolve a name-only dep/root to its unique block).
    let mut versions: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for b in &blocks {
        versions
            .entry(b.name.as_str())
            .or_default()
            .push(b.version.as_str());
    }
    // (name, version) -> its dependency refs.
    let mut graph: std::collections::BTreeMap<Id, &Vec<Dep>> = std::collections::BTreeMap::new();
    for b in &blocks {
        graph.insert((b.name.clone(), b.version.clone()), &b.deps);
    }

    // Resolve a `Dep` (or a root name with `version = None`) to a concrete locked `Id`.
    let resolve = |name: &str, want: Option<&str>| -> Result<Id, String> {
        match want {
            Some(v) => Ok((name.to_string(), v.to_string())),
            None => match versions.get(name).map(|vs| vs.as_slice()) {
                None | Some([]) => Err(format!("dependency {name:?} not present in Cargo.lock")),
                Some([only]) => Ok((name.to_string(), only.to_string())),
                Some(_many) => Err(format!(
                    "dependency {name:?} is ambiguous (locked under multiple versions) but referenced without a version"
                )),
            },
        }
    };

    // BFS over concrete (name, version) ids.
    let mut reachable: BTreeSet<Id> = BTreeSet::new();
    let mut stack: Vec<Id> = Vec::new();
    for r in roots {
        stack.push(
            resolve(r, None).map_err(|_| format!("root crate {r:?} not present in Cargo.lock"))?,
        );
    }
    while let Some(id) = stack.pop() {
        if !reachable.insert(id.clone()) {
            continue;
        }
        let deps = graph.get(&id).ok_or_else(|| {
            format!(
                "locked package {:?} {:?} referenced but not defined",
                id.0, id.1
            )
        })?;
        for d in deps.iter() {
            let dep_id = resolve(&d.name, d.version.as_deref())?;
            if !reachable.contains(&dep_id) {
                stack.push(dep_id);
            }
        }
    }

    let mut out = preamble;
    for b in &blocks {
        if reachable.contains(&(b.name.clone(), b.version.clone())) {
            out.push_str(&b.text);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Emitted artifacts
// ---------------------------------------------------------------------------------------------

fn workspace_manifest(crates: &[&str]) -> String {
    let members = crates
        .iter()
        .map(|c| format!("    \"crates/{c}\",\n"))
        .collect::<String>();
    format!(
        "# GENERATED by cdz-source-export — do not edit by hand.\n\
         # A faithful, refreshable PROJECTION of Cadenza reducer-runtime crates for downstream vendoring.\n\
         # Re-run the projection to refresh (see REFRESH.md); the source of truth is the Cadenza flake.\n\
         [workspace]\n\
         resolver = \"3\"\n\
         members = [\n{members}]\n"
    )
}

fn refresh_doc(tier: &str, crates: &[&str]) -> String {
    let members = crates
        .iter()
        .map(|c| format!("- `crates/{c}` — projected verbatim from the Cadenza repo.\n"))
        .collect::<String>();
    // The reducer tier's driving is behind cdz-platform's `host` feature; the codec tier is plain.
    let build_note = if tier == "reducer" {
        "\n## Building\n\n\
         The wasmtime driving lives behind `cdz-platform`'s `host` feature — build it with:\n\n\
         ```sh\n\
         cargo build -p cdz-platform --features host\n\
         ```\n\n\
         (`ReducerHost` / `WasmReducer` / `WasmProgramStore` in `cdz-platform::host`; the reducer-world\n\
         WIT is `crates/cdz-platform/wit/world.wit`; plug your own backends into the `BlobStore` /\n\
         `KvStore` / `ReducerGraph` traits.) This tier pulls wasmtime 37 + cranelift — kept a SEPARATE\n\
         projection from the light codec tier so those deps never leak into it.\n"
    } else {
        ""
    };
    format!(
        "# Cadenza {tier} projection\n\n\
         This tree is a **projection** of Cadenza reducer-runtime crates ({tier} tier), emitted by the\n\
         `cdz-source-export` tool from the Cadenza repo. It is a single source of truth: DO NOT edit\n\
         the vendored crate sources here — change them upstream in Cadenza and re-run the projection so\n\
         this copy stays faithful.\n\n\
         ## What is here\n\n\
         {members}\
         - `Cargo.lock` — the transitive dependency closure of the above, FILTERED (version-aware) from\n\
           the Cadenza repo's pinned root lock (same versions the nix build pins → drift-proof).\n\
         {build_note}\n\
         ## Refresh\n\n\
         From a checkout of the Cadenza repo:\n\n\
         ```sh\n\
         cargo run -p cdz-source-export -- --repo <cadenza-repo> --out <this-tree> --tier {tier}\n\
         ```\n\n\
         Or, drift-proof from the flake outputs: `nix build .#{tier}-source-export` and copy the result.\n"
    )
}

// ---------------------------------------------------------------------------------------------
// CLI + filesystem helpers
// ---------------------------------------------------------------------------------------------

struct Args {
    repo: PathBuf,
    out: PathBuf,
    tier: String,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut repo = None;
        let mut out = None;
        let mut tier = "codec".to_string();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--repo" => repo = Some(PathBuf::from(next(&mut it, "--repo")?)),
                "--out" => out = Some(PathBuf::from(next(&mut it, "--out")?)),
                "--tier" => tier = next(&mut it, "--tier")?,
                "-h" | "--help" => {
                    return Err(
                        "usage: cdz-source-export [--repo <dir>] --out <dir> [--tier codec]".into(),
                    )
                }
                other => return Err(format!("unexpected argument {other:?}")),
            }
        }
        Ok(Args {
            repo: repo.unwrap_or_else(|| PathBuf::from(".")),
            out: out.ok_or("missing required --out <dir>")?,
            tier,
        })
    }
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} expects a value"))
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))
}

fn write(p: &Path, s: &str) -> Result<(), String> {
    std::fs::write(p, s).map_err(|e| format!("write {}: {e}", p.display()))
}

fn mkdir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("mkdir {}: {e}", p.display()))
}

/// Empty `out` if it exists, then create it fresh — so a re-run is a clean projection, not a merge.
fn reset_dir(out: &Path) -> Result<(), String> {
    if out.exists() {
        std::fs::remove_dir_all(out).map_err(|e| format!("clear {}: {e}", out.display()))?;
    }
    mkdir(out)
}

/// Copy a crate's source dir verbatim, skipping build/VCS artifacts that must never be projected.
fn copy_crate(src: &Path, dst: &Path) -> Result<(), String> {
    mkdir(dst)?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read_dir {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("read_dir {}: {e}", src.display()))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Never project build output or VCS state; a crate's own leaf lock is irrelevant here (the
        // projected workspace has one filtered lock at its root).
        if name_str == "target" || name_str == ".git" || name_str == "Cargo.lock" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let ty = entry
            .file_type()
            .map_err(|e| format!("stat {}: {e}", from.display()))?;
        if ty.is_dir() {
            copy_crate(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "\
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 3

[[package]]
name = \"autocfg\"
version = \"1.5.1\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
checksum = \"deadbeef\"

[[package]]
name = \"cadenza-ast\"
version = \"0.1.0\"
dependencies = [
 \"num-bigint\",
 \"unicode-normalization\",
]

[[package]]
name = \"cadenza-value\"
version = \"0.0.0\"
dependencies = [
 \"bytes\",
 \"cadenza-ast\",
]

[[package]]
name = \"cadenza-ast-serde\"
version = \"0.1.0\"
dependencies = [
 \"cadenza-ast\",
 \"serde\",
]

[[package]]
name = \"num-bigint\"
version = \"0.4.8\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
dependencies = [
 \"num-integer\",
]

[[package]]
name = \"num-integer\"
version = \"0.1.46\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"unicode-normalization\"
version = \"0.1.25\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"bytes\"
version = \"1.10.0\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"serde\"
version = \"1.0.229\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"unrelated-heavy-crate\"
version = \"9.9.9\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
dependencies = [
 \"wasmtime\",
]

[[package]]
name = \"wasmtime\"
version = \"37.0.0\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
";

    fn names(lock: &str) -> Vec<String> {
        parse_lock(lock)
            .unwrap()
            .1
            .into_iter()
            .map(|b| b.name)
            .collect()
    }

    #[test]
    fn closure_keeps_only_reachable_packages() {
        let out = filter_lock(LOCK, tier_roots("codec").unwrap()).unwrap();
        let kept: BTreeSet<String> = names(&out).into_iter().collect();
        let expected: BTreeSet<String> = [
            "cadenza-ast",
            "cadenza-value",
            "cadenza-ast-serde",
            "num-bigint",
            "num-integer",
            "unicode-normalization",
            "bytes",
            "serde",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(kept, expected);
        // The heavy, unreachable crates must be pruned.
        assert!(!kept.contains("wasmtime"));
        assert!(!kept.contains("unrelated-heavy-crate"));
    }

    #[test]
    fn preamble_and_retained_blocks_are_verbatim() {
        let out = filter_lock(LOCK, tier_roots("codec").unwrap()).unwrap();
        // Header preamble preserved exactly.
        assert!(out.starts_with(
            "# This file is automatically @generated by Cargo.\n\
             # It is not intended for manual editing.\nversion = 3\n"
        ));
        // A retained block appears byte-identical to its source form.
        assert!(out.contains(
            "[[package]]\nname = \"num-integer\"\nversion = \"0.1.46\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
        ));
    }

    #[test]
    fn full_closure_round_trips_identically() {
        // If every package is a root, the filtered lock must equal the input exactly.
        let all: Vec<&str> = names(LOCK)
            .iter()
            .map(|s| Box::leak(s.clone().into_boxed_str()) as &str)
            .collect();
        let out = filter_lock(LOCK, &all).unwrap();
        assert_eq!(out, LOCK);
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(filter_lock(LOCK, &["does-not-exist"]).is_err());
    }

    #[test]
    fn dep_ref_extracts_name_and_version() {
        let d = dep_ref("\"foo\",").unwrap();
        assert_eq!((d.name.as_str(), d.version), ("foo", None));
        let d = dep_ref("\"foo 1.2.3\",").unwrap();
        assert_eq!(
            (d.name.as_str(), d.version.as_deref()),
            ("foo", Some("1.2.3"))
        );
        let d = dep_ref("\"foo 1.2.3 (registry+https://x)\"").unwrap();
        assert_eq!(
            (d.name.as_str(), d.version.as_deref()),
            ("foo", Some("1.2.3"))
        );
        assert!(dep_ref("not-a-quote").is_none());
    }

    // Two `syn` versions: `user-a` pins syn 2, `user-b` pins syn 1. A closure rooted at `user-a`
    // must keep ONLY syn 2 (version-aware) — the name-only approach would wrongly retain both.
    const LOCK_MULTI: &str = "\
version = 3

[[package]]
name = \"user-a\"
version = \"0.1.0\"
dependencies = [
 \"syn 2.0.0\",
]

[[package]]
name = \"user-b\"
version = \"0.1.0\"
dependencies = [
 \"syn 1.0.0\",
]

[[package]]
name = \"syn\"
version = \"1.0.0\"
source = \"registry+https://x\"

[[package]]
name = \"syn\"
version = \"2.0.0\"
source = \"registry+https://x\"
";

    #[test]
    fn version_aware_closure_prunes_wrong_version() {
        let out = filter_lock(LOCK_MULTI, &["user-a"]).unwrap();
        assert!(out.contains("name = \"syn\"\nversion = \"2.0.0\""));
        assert!(!out.contains("version = \"1.0.0\""));
        assert!(!out.contains("name = \"user-b\""));
    }

    #[test]
    fn name_only_ref_to_ambiguous_crate_errors() {
        // A root named `syn` cannot resolve when two `syn` versions are locked.
        assert!(filter_lock(LOCK_MULTI, &["syn"]).is_err());
    }

    #[test]
    fn tier_roots_maps_known_tiers_and_rejects_unknown() {
        assert_eq!(
            tier_roots("codec").unwrap(),
            &["cadenza-ast", "cadenza-value", "cadenza-ast-serde"]
        );
        // The reducer tier roots at cdz-platform; its first-party deps are discovered from the manifest.
        assert_eq!(tier_roots("reducer").unwrap(), &["cdz-platform"]);
        assert!(tier_roots("bogus").is_err());
    }

    #[test]
    fn sibling_path_deps_extracts_first_party_only() {
        let manifest = "\
[package]
name = \"cdz-platform\"

[[bin]]
path = \"src/bin/cdz-platform-itest.rs\"

[dependencies]
cadenza-ast = { path = \"../cadenza-ast\" }
cdz-contract = { path = \"../cdz-contract\" }
cdz-str = { path = \"../cdz-str\" }
num-bigint = \"0.4\"
wasmtime = { version = \"37\", optional = true }
";
        // Only the `../`-prefixed sibling deps are first-party; the [[bin]] `src/...` path is excluded,
        // and external (crates.io) deps have no `path`.
        assert_eq!(
            sibling_path_deps(manifest),
            vec![
                "cadenza-ast".to_string(),
                "cdz-contract".to_string(),
                "cdz-str".to_string()
            ]
        );
        // A crate with no first-party path-deps discovers nothing.
        assert!(sibling_path_deps("[dependencies]\nbytes = \"1\"\n").is_empty());
    }
}
