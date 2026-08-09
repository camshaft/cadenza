//! End-to-end tests for `cdz metadata [dir]` — print the resolved project manifest as JSON (the `cargo
//! metadata` analogue). Resolves the same `Project.cdz` as `cdz build`/`cdz test` and emits one object
//! carrying the manifest's raw fields PLUS their glob-expanded, `exclude`-filtered resolved file sets.
//! These drive the built binary and assert the JSON is well-formed and the resolution is correct.

use std::process::Command;

/// Run `cdz <args…>` from `cwd`, returning (exit_ok, stdout, stderr).
fn run_in(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// A project with globbed modules/tests and an exclude, so a test can assert glob expansion + filtering.
/// Returns the project dir.
fn temp_project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-metadata-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).expect("mkdir");
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"demo\"\ndef entry = \"app.cdz\"\ndef modules = [\"util.cdz\", \"lib/*.cdz\"]\n\
         def tests = [\"*_test.cdz\"]\ndef exclude = [\"lib/skip.cdz\"]\ndef opt-level = \"O2\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def main() -> Int64 = 0\nexport { main }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("util.cdz"),
        "def inc(n: Int64) -> Int64 = n + 1\nexport { inc }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/helper.cdz"),
        "def h() -> Int64 = 1\nexport { h }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/skip.cdz"),
        "def s() -> Int64 = 2\nexport { s }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app_test.cdz"),
        "def t() -> Int64 = 3\nexport { t }\n",
    )
    .unwrap();
    // Canonicalize so every test sees the SAME resolved path the no-arg upward search reports. On macOS
    // `std::env::temp_dir()` is `/var/folders/...`, a symlink to `/private/var/folders/...`; the no-arg
    // metadata search canonicalizes through that symlink while an explicit absolute-path arg does not, so
    // without this the two forms differ only by the `/var`→`/private/var` prefix and the
    // byte-identical-JSON assertion fails on macOS (green on Linux, where temp_dir has no such symlink).
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

/// Minimal JSON string-value lookup for a top-level `"key":"value"` — avoids a serde dep in the test.
/// Returns the value of the first `"key":"..."` string member (no escapes in our fixtures).
fn json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":\"");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// The raw text of a top-level `"key":[...]` array member (the bracketed substring), for asserting on a
/// specific array rather than the whole document.
fn json_array<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":[");
    let start = json.find(&needle)? + needle.len() - 1; // keep the `[`
    let rest = &json[start..];
    let end = rest.find(']')?;
    Some(&rest[..=end])
}

#[test]
fn metadata_reports_the_manifest_fields_and_resolved_files_as_json() {
    let dir = temp_project("full");
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "cdz metadata failed: {err}");
    // Raw manifest fields.
    assert_eq!(json_str(&out, "name"), Some("demo"), "name field: {out}");
    assert_eq!(
        json_str(&out, "entry"),
        Some("app.cdz"),
        "entry field: {out}"
    );
    assert_eq!(
        json_str(&out, "opt_level"),
        Some("O2"),
        "opt_level field: {out}"
    );
    // The entry resolves to its concrete file.
    assert!(
        json_str(&out, "entry_file").is_some_and(|f| f.ends_with("app.cdz")),
        "entry_file resolves to app.cdz: {out}"
    );
    // A `.cdz` entry has the `ml` surface.
    assert_eq!(
        json_str(&out, "surface"),
        Some("ml"),
        "a .cdz entry reports the ml surface: {out}"
    );
    // Glob expansion: `lib/*.cdz` pulls in helper.cdz; `exclude` drops lib/skip.cdz. `skip.cdz` still
    // appears in the `exclude` PATTERN list (echoed back), so assert it's absent from the RESOLVED
    // `module_files` array specifically, not the whole document.
    assert!(
        out.contains("helper.cdz"),
        "module_files include the glob-matched lib/helper.cdz: {out}"
    );
    let module_files = json_array(&out, "module_files").expect("module_files present");
    assert!(
        module_files.contains("helper.cdz") && !module_files.contains("skip.cdz"),
        "the excluded lib/skip.cdz is filtered out of module_files: {module_files}"
    );
    // The test glob resolves to app_test.cdz.
    assert!(
        out.contains("app_test.cdz"),
        "test_files include the glob-matched app_test.cdz: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_no_arg_and_manifest_path_arg_agree() {
    // No-arg (upward search) and an explicit ABSOLUTE `Project.cdz` path describe the SAME project with
    // the same absolute file paths — byte-identical JSON — mirroring how `cdz build`/`cdz test` resolve
    // the manifest. (A relative `.`/`Project.cdz` arg yields relative paths, which is a legitimate
    // difference in path FORM, not content — so this asserts on the absolute forms, which must match.)
    let dir = temp_project("resolve");
    let abs_manifest = dir.join("Project.cdz");
    let (m_ok, m_out, me) = run_in(&dir, &["metadata", abs_manifest.to_str().unwrap()]);
    assert!(m_ok, "metadata <abs manifest> failed: {me}");
    let (n_ok, n_out, ne) = run_in(&dir, &["metadata"]);
    assert!(n_ok, "no-arg metadata failed: {ne}");
    assert_eq!(
        m_out, n_out,
        "an absolute manifest-path arg and the no-arg upward search produce the same JSON"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_a_named_manifest_that_does_not_exist_errors() {
    // Consistency with `cdz build`/`check`/`test`: naming a non-existent `Project.cdz` errors "no such
    // file" rather than resolving to the parent dir.
    let dir = temp_project("missing");
    let missing = dir.join("subdir").join("Project.cdz");
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    let (ok, _o, err) = run_in(&dir, &["metadata", missing.to_str().unwrap()]);
    assert!(!ok, "a named non-existent manifest must error");
    assert!(
        err.contains("no such file"),
        "error names the missing file: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_artifacts_is_empty_before_build_and_lists_outputs_after() {
    // The `artifacts` field reports the build OUTPUTS present in the manifest dir (the same set `cdz clean`
    // removes) — so a tool can tell whether a project is built without running a build. Empty before a
    // build; after `cdz build`, it lists the produced `.wasm` + `link-map.txt`.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-artifacts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"app.cdz\"\n").unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def go() -> Int64 = 0\nexport { go }\n",
    )
    .unwrap();
    // Before building: no artifacts.
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "metadata failed: {err}");
    let arts = json_array(&out, "artifacts").expect("artifacts present");
    assert_eq!(arts, "[]", "an un-built project has no artifacts: {arts}");
    // After building: the component + link-map appear.
    let (bok, _bo, be) = run_in(&dir, &["build"]);
    assert!(bok, "build failed: {be}");
    let (ok2, out2, err2) = run_in(&dir, &["metadata", "."]);
    assert!(ok2, "metadata after build failed: {err2}");
    let arts2 = json_array(&out2, "artifacts").expect("artifacts present");
    assert!(
        arts2.contains(".wasm") && arts2.contains("link-map.txt"),
        "artifacts lists the build outputs after a build: {arts2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_artifacts_excludes_user_authored_rs_and_wasm_files() {
    // The read-only twin of the `cdz clean` data-loss guarantee: `artifacts` reports EXACTLY what `cdz
    // clean` would remove (via the shared `project_artifact_files`), so a user's hand-authored `helper.rs`
    // or checked-in `asset.wasm` must NOT be listed — only the project's own `<export>.wasm` + `link-map`.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-userfiles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"app.cdz\"\n").unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def go() -> Int64 = 0\nexport { go }\n",
    )
    .unwrap();
    std::fs::write(dir.join("helper.rs"), "fn h() {}\n").unwrap();
    std::fs::write(dir.join("asset.wasm"), b"asset").unwrap();
    let (bok, _bo, be) = run_in(&dir, &["build"]);
    assert!(bok, "build failed: {be}");
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "metadata failed: {err}");
    let arts = json_array(&out, "artifacts").expect("artifacts present");
    assert!(
        arts.contains("go.wasm") && arts.contains("link-map.txt"),
        "artifacts lists the project's own outputs: {arts}"
    );
    assert!(
        !arts.contains("helper.rs") && !arts.contains("asset.wasm"),
        "a user-authored helper.rs / asset.wasm is NOT reported as an artifact: {arts}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_reports_the_sexpr_surface_for_an_s_expression_entry() {
    // A `.sexp` entry reports the `sexpr` surface — so a consumer picks the s-expression parser.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-sexpr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"main.sexp\"\n").unwrap();
    std::fs::write(dir.join("main.sexp"), "(do (def (main) 0) (export main))\n").unwrap();
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "sexpr-entry metadata failed: {err}");
    assert_eq!(
        json_str(&out, "surface"),
        Some("sexpr"),
        "a .sexp entry reports the sexpr surface: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_omits_absent_fields_as_null() {
    // A minimal manifest (only `entry`) still produces well-formed JSON — absent fields are `null`, and
    // the empty pattern lists are `[]`.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-min-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"main.cdz\"\n").unwrap();
    std::fs::write(
        dir.join("main.cdz"),
        "def main() -> Int64 = 0\nexport { main }\n",
    )
    .unwrap();
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "minimal-manifest metadata failed: {err}");
    assert!(out.contains("\"name\":null"), "absent name is null: {out}");
    assert!(
        out.contains("\"opt_level\":null"),
        "absent opt_level is null: {out}"
    );
    assert!(out.contains("\"modules\":[]"), "empty modules is []: {out}");
    assert!(out.contains("\"tests\":[]"), "empty tests is []: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_resolved_files_report_only_existing_files() {
    // HARDENING: the RESOLVED file fields (entry_file / module_files) must report files that actually
    // exist — `expand_manifest_globs` passes a non-glob literal through verbatim (so `cdz build` can error
    // "reading X: No such file"), but metadata must not CLAIM a missing declared file is present. A
    // missing entry → entry_file null (like a zero-match glob); a missing module is omitted.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-existing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def entry = \"ghost.cdz\"\ndef modules = [\"real.cdz\", \"missing.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("real.cdz"), "def r() -> Int64 = 1\nexport { r }\n").unwrap();
    // ghost.cdz + missing.cdz deliberately absent.
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "metadata succeeds (read-only report): {err}");
    // The declared PATTERNS are still echoed.
    assert!(
        out.contains("ghost.cdz") && out.contains("missing.cdz"),
        "declared patterns echoed: {out}"
    );
    // But the RESOLVED sets only include existing files.
    assert!(
        out.contains("\"entry_file\":null"),
        "a missing entry → entry_file null: {out}"
    );
    assert!(out.contains("\"surface\":null"), "and surface null: {out}");
    let module_files = json_array(&out, "module_files").expect("module_files present");
    assert!(
        module_files.contains("real.cdz") && !module_files.contains("missing.cdz"),
        "module_files lists the existing module, not the missing one: {module_files}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metadata_surfaces_manifest_warnings_so_null_malformed_is_distinguishable_from_null_absent() {
    // `cdz metadata` resolves via `load_manifest` (it never builds), so the stderr warnings the project
    // COMMANDS emit via `resolve_project_manifest` (malformed `name`/`opt-level`, duplicate keys) never fire
    // here. A machine consumer reading ONLY `cdz metadata` would then see `"name":null` with no way to tell
    // a MALFORMED value (wrong type, silently dropped) from an ABSENT one. The `warnings` array is the
    // machine-readable twin: populated when a field was silently dropped, EMPTY for a clean manifest — so a
    // tool learns WHY a field is null.
    // Case 1: malformed `name` + malformed `opt-level` + a duplicate key → three warnings.
    let dir = std::env::temp_dir().join(format!("cdz-metadata-warn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = 42\ndef entry = \"a.cdz\"\ndef entry = \"b.cdz\"\ndef opt-level = 99\n",
    )
    .unwrap();
    let (ok, out, err) = run_in(&dir, &["metadata", "."]);
    assert!(ok, "metadata succeeds (read-only report): {err}");
    assert!(
        out.contains("\"name\":null"),
        "malformed name → null: {out}"
    );
    let warnings = json_array(&out, "warnings").expect("warnings array present");
    assert!(
        warnings.contains("`name` is not a string"),
        "the malformed name is reported as a warning: {warnings}"
    );
    assert!(
        warnings.contains("`opt-level` is not a string"),
        "the malformed opt-level is reported: {warnings}"
    );
    assert!(
        warnings.contains("more than once"),
        "the duplicate `entry` key is reported: {warnings}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Case 2: an ABSENT name → `"name":null` too, but NO warning — this is the distinction the array buys.
    let dir2 = std::env::temp_dir().join(format!("cdz-metadata-absent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir2);
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(dir2.join("Project.cdz"), "def entry = \"a.cdz\"\n").unwrap();
    let (ok2, out2, _e2) = run_in(&dir2, &["metadata", "."]);
    assert!(ok2, "metadata succeeds on an absent-name manifest");
    assert!(
        out2.contains("\"name\":null"),
        "absent name → null too: {out2}"
    );
    assert_eq!(
        json_array(&out2, "warnings").expect("warnings array present"),
        "[]",
        "an ABSENT (not malformed) name emits NO warning — null-absent is distinguishable from null-malformed: {out2}"
    );
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn metadata_reports_the_dependency_graph_in_the_deps_array() {
    // The metadata JSON documents a `"deps"` array = the project's `def deps` path-dependencies as their
    // raw manifest refs ("so a consumer sees ... the project graph"), the machine-readable twin of `cdz
    // tree`. Pin BOTH sides of the contract: a declared dep appears in the `deps` array, and a standalone
    // project (no `def deps`) yields `[]` (not a missing member). Asserts on the `deps` array specifically
    // (via `json_array`), not a whole-document substring, so a stray "../lib" elsewhere can't mask a
    // regression. Was uncovered — the shared `temp_project` fixture declares no `def deps`, so neither the
    // populated nor the empty case had an assertion.
    let root = std::env::temp_dir().join(format!("cdz-metadata-deps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // A sibling `lib` project (need not build — metadata never compiles) …
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(
        root.join("lib/Project.cdz"),
        "def name = \"lib\"\ndef entry = \"lib.cdz\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("lib/lib.cdz"),
        "def h() -> Int64 = 1\nexport { h }\n",
    )
    .unwrap();
    // … and an `app` that declares it as a path dependency.
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(
        root.join("app/Project.cdz"),
        "def name = \"app\"\ndef entry = \"app.cdz\"\ndef deps = [\"../lib\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/app.cdz"),
        "def main() -> Int64 = 0\nexport { main }\n",
    )
    .unwrap();

    let (ok, out, err) = run_in(&root.join("app"), &["metadata", "."]);
    assert!(ok, "cdz metadata failed: {err}");
    let deps = json_array(&out, "deps").expect("deps array present");
    assert!(
        deps.contains("../lib"),
        "the declared `def deps` path appears in the deps array: {deps}"
    );

    // A standalone project (the sibling `lib` itself declares no deps) reports `[]`, not a missing member.
    let (ok2, out2, err2) = run_in(&root.join("lib"), &["metadata", "."]);
    assert!(ok2, "cdz metadata on a depless project failed: {err2}");
    assert_eq!(
        json_array(&out2, "deps").expect("deps array present"),
        "[]",
        "a standalone project reports an empty deps array: {out2}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
