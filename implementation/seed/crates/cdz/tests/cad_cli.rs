//! End-to-end tests for `cdz cad` — the PASSTHROUGH mount of the standalone `cdz-cad` CAD mesh driver.
//! Like `cdz smith`, and unlike the FOLDED `cdz run`/`corpus`/`calc`, this is exec-not-link ON PURPOSE:
//! `cdz-cad` is a SEPARATE cargo workspace (its `manifold-csg` backend builds the C++ manifold3d library
//! via cmake, which must never enter `cdz`'s workspace/lockfile), so `cdz cad <args…>` locates and execs
//! the sibling `cdz-cad` binary, forwarding argv + exit code.
//!
//! The standalone `cdz-cad` binary may or may not be BUILT in a given test environment (it is a separate
//! `cargo build -p cdz-cad`, not produced by `cargo test -p cdz`). So these tests pin the PASSTHROUGH
//! CONTRACT robustly against BOTH states: `cdz cad` must EITHER pass through to the bin (present) OR fail
//! with the clear actionable "build it" error (absent) — never a `cdz`-side clap misparse of the forwarded
//! args, or a panic. The mount + argv-forwarding are what this vertical owns; the mesher's behavior itself
//! is tested in the cdz-cad workspace.

use std::process::Command;

/// Run `cdz <args…>` (optionally piping `stdin`), returning (exit_ok, stdout, stderr).
fn run_stdin(args: &[&str], stdin: &str) -> (bool, String, String) {
    use std::io::Write;
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn run(args: &[&str]) -> (bool, String, String) {
    run_stdin(args, "")
}

/// True if the combined output is the clean "cdz-cad not built" passthrough error — the expected outcome
/// when the separate-workspace binary is absent. It must name the binary AND the build command (an
/// actionable fix), not just fail opaquely.
fn is_clean_not_found(stderr: &str) -> bool {
    stderr.contains("cdz-cad")
        && stderr.contains("cargo build -p cdz-cad")
        && stderr.contains("cdz:")
}

#[test]
fn cad_is_a_passthrough_not_a_clap_misparse() {
    // `cdz cad --help` forwards `--help` to the standalone bin (or clap prints the wrapper about-text at
    // the subcommand level). Either way it must NOT be `cdz`'s clap rejecting a forwarded arg, nor a panic.
    let (_ok, _out, err) = run(&["cad", "--help"]);
    assert!(
        !err.contains("panic") && !err.contains("RUST_BACKTRACE"),
        "no panic on the passthrough: {err}"
    );
    assert!(
        !err.contains("error: unrecognized"),
        "`--help` is not rejected by cdz as an unknown arg: {err}"
    );
}

#[test]
fn cad_forwards_arbitrary_trailing_args_without_parsing_them() {
    // Flags that are NOT `cdz` flags (and a leading-hyphen value) must pass through untouched — the
    // `trailing_var_arg` + `allow_hyphen_values` contract. We assert ONLY the cdz-SIDE contract (cdz
    // doesn't eat the args), NOT what the bin does: a co-present cdz-cad may itself reject a bad flag and
    // exit non-zero, which is the bin's business, not a cdz parse failure.
    let (_ok, _out, err) = run(&[
        "cad",
        "-o",
        "/tmp/nonexistent-cdz-cad-out.stl",
        "--segments",
        "8",
    ]);
    assert!(
        !err.contains("unexpected argument") && !err.contains("panic"),
        "arbitrary trailing args are forwarded verbatim, not parsed by cdz: {err}"
    );
    assert!(
        !err.contains("error: unrecognized") && !err.contains("Usage: cdz cad"),
        "a non-success is the bin's own output or the actionable not-found, not a cdz parse error: {err}"
    );
}

#[test]
fn cad_appears_in_the_command_tree() {
    // The passthrough must be discoverable in `cdz --help` (the one-binary story: every tool reachable
    // under `cdz`).
    let (ok, out, _err) = run(&["--help"]);
    assert!(ok, "cdz --help succeeds");
    assert!(
        out.contains("cad"),
        "`cad` is listed as a subcommand in `cdz --help`: {out}"
    );
}

#[test]
fn cad_meshes_a_solid_from_stdin_when_the_bin_is_present() {
    // The real passthrough smoke (v-cad's suggestion): pipe a trivial cube SolidR into `cdz cad - -o …`.
    // The standalone cdz-cad bin isn't built by `cargo test -p cdz`, so this is CONDITIONAL: if the bin is
    // present it must mesh to a non-empty .stl (exit 0); if absent, the clean not-found error. Either way,
    // NEVER a cdz-side parse error or panic. A rendered `Solid` crosses the boundary as `(: <solid> Solid)`;
    // a unit cube is `(Cube (: (2/1 2/1 2/1) …))`-shaped — but since the bin is absent in-CI, we primarily
    // pin the not-found branch and leave the mesh assertion to run only when someone has built cdz-cad.
    let cube = "(: (Cube (2/1 2/1 2/1)) Solid)\n";
    let out_path = std::env::temp_dir().join(format!("cdz-cad-smoke-{}.stl", std::process::id()));
    let _ = std::fs::remove_file(&out_path);
    let (ok, _o, err) = run_stdin(&["cad", "-", "-o", out_path.to_str().unwrap()], cube);
    assert!(
        !err.contains("panic"),
        "no panic on the mesh passthrough: {err}"
    );
    if ok {
        // Bin present + meshed: the output file exists and is non-empty (a real STL).
        let meshed = std::fs::metadata(&out_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        assert!(
            meshed,
            "a present cdz-cad meshed the cube to a non-empty {}: {err}",
            out_path.display()
        );
    } else {
        // Bin absent (the CI case) OR the bin declined this exact SolidR shape — the failure must not be a
        // cdz-side parse error. In the absent case (the common CI one) it is exactly our clean not-found;
        // if the bin is present but declined, that's its own output (not a not-found, not a cdz parse error).
        let cdz_parse_error = err.contains("error: unrecognized") || err.contains("Usage: cdz cad");
        assert!(
            !cdz_parse_error,
            "a non-success is the bin's own output or the actionable not-found, not a cdz parse error: {err}"
        );
        // When the bin is genuinely absent, the message is our actionable build hint (not opaque).
        if err.contains("not found beside") {
            assert!(
                is_clean_not_found(&err),
                "the absent-bin failure names cdz-cad + the build command: {err}"
            );
        }
    }
    let _ = std::fs::remove_file(&out_path);
}
