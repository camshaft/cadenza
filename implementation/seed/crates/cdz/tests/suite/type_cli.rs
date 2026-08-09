//! End-to-end tests for `cdz type NAME FILE` — the by-name type query.
//!
//! Focus: the EXIT CODE distinguishes an UNRESOLVABLE name from a real one. `TypeOf` is a total sidecar
//! query — it renders a defined line even for an unknown name ("no such definition `X` — did you mean …"
//! / "… — closest matches: …"). `cdz type` maps that verdict to a non-zero exit (a typo isn't a type), so
//! a script can tell "you misspelled it" from a real type. A real name prints its type and succeeds.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// A module with two definitions, so the unknown-name query has near/far candidates. Returns the file.
fn temp_module(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-type-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("m.cdz");
    std::fs::write(
        &f,
        "def add(a: Int64, b: Int64) -> Int64 = a + b\ndef other() -> Int64 = 0\nexport { add, other }\n",
    )
    .unwrap();
    f
}

#[test]
fn type_of_a_real_definition_prints_the_type_and_succeeds() {
    let f = temp_module("real");
    let (ok, out, err) = run(&["type", "add", f.to_str().unwrap()]);
    assert!(ok, "cdz type of a real def should succeed: {err}");
    assert!(out.contains("Int64"), "prints the rendered type: {out}");
    assert!(
        !out.contains("no such definition"),
        "a real def is not a no-such-definition verdict: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn type_of_a_far_unknown_name_fails_with_closest_matches() {
    // A name unlike any def gets a "closest matches" hint — and exits NON-ZERO (it's a typo, not a type).
    let f = temp_module("far");
    let (ok, out, _err) = run(&["type", "zzz_nomatch", f.to_str().unwrap()]);
    assert!(!ok, "an unresolvable name must FAIL (non-zero exit): {out}");
    assert!(
        out.contains("no such definition `zzz_nomatch`") && out.contains("closest matches"),
        "reports the unresolvable name with candidates: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn type_of_a_near_typo_fails_with_did_you_mean() {
    // A near-miss of a real name gets "did you mean `add`?" and also fails.
    let f = temp_module("near");
    let (ok, out, _err) = run(&["type", "ad", f.to_str().unwrap()]);
    assert!(!ok, "a near-miss (unresolvable) must fail: {out}");
    assert!(
        out.contains("no such definition `ad`") && out.contains("did you mean `add`"),
        "suggests the near name: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn a_directory_in_the_file_slot_gives_a_clean_diagnostic_not_a_raw_os_error() {
    // A DIRECTORY passed where a program FILE is expected must give a clean, actionable message — not the
    // raw `Is a directory (os error 21)` a `read_to_string` leaks. This pins the shared program loader's
    // pre-check, which fixes it for EVERY by-name/by-offset query command at once (type/doc/uses/def/
    // scope/type-at/doc-at/exports/symbols/highlight/instantiations all funnel through it).
    let dir = std::env::temp_dir().join(format!("cdz-type-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (ok, out, err) = run(&["type", "foo", dir.to_str().unwrap()]);
    assert!(!ok, "a directory in the file slot must fail: {out}{err}");
    assert!(
        err.contains("is a directory") && err.contains("single program file"),
        "clean directory diagnostic naming what to pass, not a raw errno: {err}"
    );
    assert!(
        !err.contains("os error"),
        "the raw OS error is not surfaced: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bare_stdin_marker_gives_a_clean_diagnostic_not_a_raw_os_error() {
    // A bare `-` (the stdin marker) passed to a command that does NOT read stdin — the by-name/by-offset
    // query commands take a NAMED file (its extension picks the surface) — must give a clean message
    // pointing at the stdin-capable commands, not the raw `reading -: No such file or directory (os error
    // 2)` that `read_to_string("-")` leaks (it looks for a file literally named `-`). Pinned via the shared
    // loader (so it holds for every query command at once).
    let (ok, out, err) = run(&["type", "foo", "-"]);
    assert!(!ok, "a bare `-` must fail here: {out}{err}");
    assert!(
        err.contains("stdin") && err.contains("not supported by this command"),
        "clean stdin diagnostic pointing at the stdin-capable commands: {err}"
    );
    assert!(
        !err.contains("os error"),
        "the raw OS error is not surfaced: {err}"
    );
    // The pointer must stay CURRENT: the verdict runners (`run-ml`/`run-rust`/`run-emitted`) now accept `-`
    // as the stdin marker too, so the "commands that read stdin" list names them — else it silently drifts
    // stale (a user is told fmt/convert/compile/run when run-ml would also work).
    assert!(
        err.contains("run-ml") && err.contains("cdz fmt"),
        "the stdin-capable list includes both the pipe commands and the verdict runners: {err}"
    );
}
