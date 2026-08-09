//! The compile-reuse PROVE-FIRST WITNESS at scale — the invariant the shared-import-graph compile-reuse
//! design rides on, checked over the REAL compiler-ml self-host closure via `cdz func-layout`.
//!
//! Two compiler-ml test files (`sread-eval-fns.cdz`, `sread-eval-ho.cdz`) share the SAME ~570-def import
//! closure and differ ONLY in their `@test` entries. If the shared subgraph emits identically regardless of
//! which file's @tests accompany it, then it can be compiled ONCE and reused across the ~8 heavy files
//! (~8×381s → ~1×381s + N×~3s). This test proves the reuse is SOUND at scale: every def shared by NAME
//! across the two layouts must report the SAME content-hash (a structural hash of the def's own AST subtree
//! — the cache KEY). Func-INDICES are expected to DIFFER (the @test region interleaves, shifting shared-def
//! slots), which is why the consensus design caches at the LOWERED-CORE tier keyed on content-hash and
//! re-runs layout + func-index assignment + emit per file — func-index is NOT part of the reused artifact.
//!
//! DRIVES the full compiler-ml front-end (monomorphize + layout) twice — ~18s in the debug profile — so it
//! is `#[ignore]`d to keep `cargo test --workspace` fast; run explicitly with `--ignored`. Skips cleanly
//! (not a failure) if the compiler-ml src tree is absent from the checkout.

use std::collections::HashMap;
use std::process::Command;

/// Walk up from the test crate's manifest dir to the repo's `implementation/compiler-ml/src` (the pattern
/// `run_ml_cli.rs` uses). Returns None if the tree isn't in this checkout.
fn compiler_ml_src() -> Option<std::path::PathBuf> {
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cand = root.join("implementation/compiler-ml/src");
        if cand.is_dir() {
            return Some(cand);
        }
        if !root.pop() {
            return None;
        }
    }
}

/// Run `cdz func-layout FILE` and parse the emitted-def rows into a `name -> (func_index, content_hash)`
/// map. Panics on a non-zero exit, a missing/malformed `defs-begin` marker, a malformed row, or a DUPLICATE
/// def name (the witness compares defs by name, so a name must be unique within one layout — a dup would
/// silently overwrite and compare the wrong def).
fn layout_of(file: &std::path::Path) -> HashMap<String, (String, String)> {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(["func-layout", file.to_str().unwrap()])
        .output()
        .expect("spawn cdz func-layout");
    assert!(
        out.status.success(),
        "func-layout {file:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // VALIDATE the marker rather than blindly skipping line 0: if the CLI output format ever changes (no
    // marker, or a row where the marker should be), silently treating line 0 as the marker would parse a real
    // def row as "the marker" and drop it — a misleading witness. Assert line 0 IS `defs-begin<TAB>N<TAB>-`.
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    let marker: Vec<&str> = first.split('\t').collect();
    assert!(
        marker.len() == 3
            && marker[0] == "defs-begin"
            && marker[1].parse::<u32>().is_ok()
            && marker[2] == "-",
        "first line must be the `defs-begin<TAB><import-base><TAB>-` marker, got {first:?}\nfull:\n{text}"
    );
    let mut map = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "row is idx<TAB>hash<TAB>name: {line:?}");
        // FAIL-FAST on a duplicate name: the witness compares defs by name across two files, so a name that
        // appears twice in ONE layout would silently overwrite (last-wins) and compare the WRONG def's
        // hash/index — a misleading green/red. A def name is expected unique within one layout; assert it.
        let prev = map.insert(
            cols[2].to_string(),
            (cols[0].to_string(), cols[1].to_string()),
        );
        assert!(
            prev.is_none(),
            "duplicate def name {:?} in {file:?}'s layout — the witness assumes names are unique within a \
             layout (a dup would silently overwrite and compare the wrong def)",
            cols[2]
        );
    }
    map
}

#[test]
#[ignore = "drives the full compiler-ml front-end twice (~18s); run with --ignored — the compile-reuse prove-first witness"]
fn shared_defs_of_two_test_files_hash_identically_the_compile_reuse_invariant() {
    let Some(src) = compiler_ml_src() else {
        // The compiler-ml src tree isn't in this checkout — nothing to witness; don't fail the harness.
        return;
    };
    let fns = layout_of(&src.join("sread-eval-fns.cdz"));
    let ho = layout_of(&src.join("sread-eval-ho.cdz"));
    assert!(
        fns.len() > 400 && ho.len() > 400,
        "both files lay out the full shared closure (expected ~570+ defs each): fns={}, ho={}",
        fns.len(),
        ho.len()
    );

    // Defs shared by NAME across the two layouts — the closure minus each file's own @test/helper defs.
    let shared: Vec<&String> = fns.keys().filter(|k| ho.contains_key(*k)).collect();
    assert!(
        shared.len() > 400,
        "the two files share the bulk of the ~570-def closure (got {} shared)",
        shared.len()
    );

    // THE INVARIANT: every shared def has the SAME content-hash in both files (the cache key is stable
    // regardless of which @tests accompany the shared subgraph). A single mismatch would mean the shared
    // emit is NOT reuse-safe — the reused lowering could silently run wrong code.
    let mut hash_mismatches = Vec::new();
    let mut idx_differ = 0usize;
    for name in &shared {
        let (i_fns, h_fns) = &fns[*name];
        let (i_ho, h_ho) = &ho[*name];
        if h_fns != h_ho {
            hash_mismatches.push(format!("{name}: {h_fns} (fns) != {h_ho} (ho)"));
        }
        if i_fns != i_ho {
            idx_differ += 1;
        }
    }
    assert!(
        hash_mismatches.is_empty(),
        "{} shared def(s) hash DIFFERENTLY across the two files — shared emit is NOT reuse-safe:\n{}",
        hash_mismatches.len(),
        hash_mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Positive documentation of the OTHER half of the design: func-indices SHIFT (the @test region
    // interleaves), which is exactly why the cache is keyed on content-hash at the Core tier and func-index
    // assignment is redone per file. Not a hard requirement — but if indices ever stopped shifting, the
    // design's "func-index is per-file, not cached" premise would deserve a re-look, so surface it.
    assert!(
        idx_differ > 0,
        "expected shared-def func-indices to shift across files (the @test region interleaves); none did — \
         re-examine whether func-index could be cached too"
    );
}
