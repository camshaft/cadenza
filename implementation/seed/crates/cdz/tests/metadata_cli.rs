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
    dir
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
