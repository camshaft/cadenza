//! Guide-example SHRED, in Rust from the binary AST (operator: shred in a Rust program via the binary AST;
//! no sexpr parser in the script; binary AST = universal exchange). Replaces the node shred-examples.mjs,
//! which parsed the @generated .tsx. The CALLER (guideShred nix derivation) parses each chapter .sexp →
//! binary AST (`cdz convert --from sexpr --to binary`); this xtask mode decodes that (`codec::decode`),
//! walks the (runnable)/(exercise) (source …) subtrees, wraps each into a compilable program, renders both
//! surfaces, and emits the per-case artifact dirs + manifest.json that guideShred/mkGuideBuild/mkGuideExec
//! consume — the SAME contract as shred-examples.mjs.
//!
//! Source text is recovered by PRINTING the (source …) form children (`print_from` — decoded binary AST has
//! no spans, and formatting is irrelevant for COMPILATION anyway; the exec grades the program, not its
//! layout). The ML surface is `cdz convert --from sexpr --to ml` of the wrapped sexpr (wrapping commutes
//! with surface conversion). Deterministic (case dirs = index+slug, no timestamps) for the CA nix cache.
//!
//! v1 SCOPE: single-file runnables + exercises (the ~all of them). DEFERRED (emit meta.deferred, no program):
//! multi-file `(files …)` runnables (need the lowerToCompile port — a follow-up) + mode="test" runnables.

use crate::wrap::{Surface, wrap_module};
use cadenza_ast::ast::Arenas;
use cadenza_ast::ast::StructId;
use std::process::Command;

/// Print the program a `(source …)`/`(solution …)` holder holds, as sexpr snippet text: its form children,
/// each canonical-printed (`print_from`) and newline-joined. A lone string atom (a string-literal program)
/// prints as the quoted string. Empty when the holder is absent.
fn snippet_text(a: &Arenas, node: StructId, name: &str) -> Option<String> {
    let holder = super::named_node(a, node, name)?;
    let kids = super::children(a, holder);
    if kids.is_empty() {
        return None;
    }
    let parts: Vec<String> = kids
        .iter()
        .map(|&k| cadenza_syntax_sexpr::print_from(a, k))
        .collect();
    Some(parts.join("\n"))
}

/// Render `sexpr_program` to ML via `cdz convert --from sexpr --to ml` (the caller puts `cdz` on PATH). This
/// is a downstream RENDER (the input to the script is binary AST; the ml surface is a projection), not the
/// input parser. Returns the ml text (trimmed) or an error string.
fn render_ml(cdz: &str, sexpr_program: &str) -> Result<String, String> {
    let out = Command::new(cdz)
        .args(["convert", "--from", "sexpr", "--to", "ml"])
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut ch| {
            use std::io::Write;
            ch.stdin
                .take()
                .unwrap()
                .write_all(sexpr_program.as_bytes())?;
            ch.wait_with_output()
        })
        .map_err(|e| format!("spawn {cdz} convert: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cdz convert sexpr→ml failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// A case's derived shred artifacts + metadata (kept minimal; JSON emitted by hand to match shred-examples).
struct Case {
    dir: String,
    kind: &'static str,
    graded: bool,
    expect_kind: &'static str,
    surfaces: Vec<&'static str>,
    deferred: bool,
    reason: Option<String>,
    program_sexpr: Option<String>,
    program_ml: Option<String>,
    expected: Option<String>,
    file_stem: String,
}

/// `slugify` matching shred-examples.mjs: strip path + extension, non-alnum runs → `-`, trim, lowercase.
fn slugify(stem: &str) -> String {
    let mut s = String::new();
    let mut prev_dash = false;
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            s.push('-');
            prev_dash = true;
        }
    }
    s.trim_matches('-').to_string()
}

/// Derive one runnable/exercise case from its node. `cdz` renders the ml surface. Multi-file + test-mode
/// runnables are marked deferred (no program) in v1.
fn derive_case(
    a: &Arenas,
    node: StructId,
    kind: &'static str,
    stem: &str,
    idx: usize,
    cdz: &str,
) -> Result<Case, String> {
    let dir = format!("{idx:04}-{}", slugify(stem));
    let file_stem = stem.to_string();

    // Multi-file / test-mode runnables: deferred in v1 (need the lowerToCompile port).
    let is_multifile = super::named_node(a, node, "files").is_some();
    let is_test = super::named_attr(a, node, "mode") == Some("test");
    if is_multifile || is_test {
        return Ok(Case {
            dir,
            kind: if is_multifile {
                "multi-file"
            } else {
                "test-mode"
            },
            graded: false,
            expect_kind: "value",
            surfaces: vec![],
            deferred: true,
            reason: Some(if is_multifile {
                "multi-file (files …) runnable — lowerToCompile port pending (v2 shred kind)".into()
            } else {
                "mode=test runnable runs via the @test-export driver (v2 shred kind)".into()
            }),
            program_sexpr: None,
            program_ml: None,
            expected: None,
            file_stem,
        });
    }

    // The program: runnable → (source); exercise → (solution) (the gradeable correct program).
    let src_name = if kind == "exercise" {
        "solution"
    } else {
        "source"
    };
    let snippet = snippet_text(a, node, src_name)
        .ok_or_else(|| format!("{dir}: no ({src_name} …) program"))?;
    let program_sexpr = wrap_module(&snippet, Surface::Sexpr);
    let program_ml_snippet = render_ml(cdz, &program_sexpr)?;

    let expected = super::named_attr(a, node, "expected").map(str::to_string);
    let expect_kind = if super::named_attr(a, node, "expect") == Some("error") {
        "error"
    } else {
        "value"
    };

    Ok(Case {
        dir,
        kind,
        graded: expected.is_some(),
        expect_kind,
        surfaces: vec!["sexpr", "ml"],
        deferred: false,
        reason: None,
        program_sexpr: Some(program_sexpr),
        program_ml: Some(program_ml_snippet),
        expected,
        file_stem,
    })
}

fn json_str(s: &str) -> String {
    super::json_string(s)
}

/// `--shred <out-dir> <cdz-bin> <ordered .cdzb list>`: decode each chapter binary AST, shred its
/// runnable/exercise cases into `<out-dir>/<NNNN>-<slug>/`, and write manifest.json. Case order = the .cdzb
/// argument order (the caller passes chapters in a stable order).
pub fn run_shred(out_dir: &str, cdz: &str, cdzb_paths: &[String]) {
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir).unwrap_or_else(|e| die(&format!("mkdir {out_dir}: {e}")));

    let mut cases: Vec<Case> = Vec::new();
    let mut idx = 0usize;
    for path in cdzb_paths {
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("chapter")
            .to_string();
        let bytes = std::fs::read(path).unwrap_or_else(|e| die(&format!("read {path}: {e}")));
        let a = cadenza_ast::codec::decode(&bytes)
            .unwrap_or_else(|| die(&format!("decode {path}: invalid binary AST")));
        let chapter =
            super::locate_chapter(&a).unwrap_or_else(|| die(&format!("{path}: no (chapter …)")));
        for &f in super::children(&a, chapter) {
            let kind = match a.head_name(f) {
                Some("runnable") => "runnable",
                Some("exercise") => "exercise",
                _ => continue,
            };
            idx += 1;
            let case = derive_case(&a, f, kind, &stem, idx, cdz)
                .unwrap_or_else(|e| die(&format!("shred {path} #{idx}: {e}")));
            write_case(out_dir, &case);
            cases.push(case);
        }
    }

    write_manifest(out_dir, &cases);
    let emitted = cases.iter().filter(|c| !c.deferred).count();
    let deferred = cases.len() - emitted;
    println!(
        "shred: {} cases across {} chapters → {out_dir} ({emitted} emitted, {deferred} deferred)",
        cases.len(),
        cdzb_paths.len()
    );
}

fn write_case(out_dir: &str, c: &Case) {
    let dir = std::path::Path::new(out_dir).join(&c.dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&format!("mkdir {}: {e}", dir.display())));
    let w = |name: &str, body: &str| {
        std::fs::write(dir.join(name), body).unwrap_or_else(|e| die(&format!("write {name}: {e}")));
    };
    if let Some(p) = &c.program_sexpr {
        w("program.sexpr", p);
    }
    if let Some(p) = &c.program_ml {
        w("program.ml", p);
    }
    if let Some(e) = &c.expected {
        w("expected", e);
    }
    if !c.deferred {
        w("expect-kind", c.expect_kind);
    }
    // meta.json — schema mirrors shred-examples.mjs (file kept as ".tsx" so the nix per-file aggregate's
    // `removeSuffix ".tsx"` grouping is unchanged).
    let surfaces = c
        .surfaces
        .iter()
        .map(|s| json_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    let mut meta = format!(
        "{{\n  \"file\": {},\n  \"kind\": {},\n  \"graded\": {},\n  \"expectKind\": {},\n  \"surfaces\": [{}]",
        json_str(&format!("src/content/chapters/{}.tsx", c.file_stem)),
        json_str(c.kind),
        c.graded,
        json_str(c.expect_kind),
        surfaces,
    );
    if !c.deferred {
        meta.push_str(&format!(",\n  \"authoredSurface\": {}", json_str("sexpr")));
    }
    if c.deferred {
        meta.push_str(",\n  \"deferred\": true");
        if let Some(r) = &c.reason {
            meta.push_str(&format!(",\n  \"reason\": {}", json_str(r)));
        }
    }
    meta.push_str("\n}\n");
    w("meta.json", &meta);
}

fn write_manifest(out_dir: &str, cases: &[Case]) {
    let emitted = cases.iter().filter(|c| !c.deferred).count();
    let deferred = cases.len() - emitted;
    let entries: Vec<String> = cases
        .iter()
        .map(|c| {
            let surfaces = c
                .surfaces
                .iter()
                .map(|s| json_str(s))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "    {{ \"dir\": {}, \"file\": {}, \"kind\": {}, \"graded\": {}, \"expectKind\": {}, \"surfaces\": [{}], \"deferred\": {} }}",
                json_str(&c.dir),
                json_str(&format!("src/content/chapters/{}.tsx", c.file_stem)),
                json_str(c.kind),
                c.graded,
                json_str(c.expect_kind),
                surfaces,
                c.deferred,
            )
        })
        .collect();
    let manifest = format!(
        "{{\n  \"count\": {},\n  \"emitted\": {},\n  \"deferred\": {},\n  \"blockedDirs\": [],\n  \"cases\": [\n{}\n  ]\n}}\n",
        cases.len(),
        emitted,
        deferred,
        entries.join(",\n"),
    );
    std::fs::write(
        std::path::Path::new(out_dir).join("manifest.json"),
        manifest,
    )
    .unwrap_or_else(|e| die(&format!("write manifest.json: {e}")));
}

fn die(msg: &str) -> ! {
    eprintln!("xtask-codegen-guide --shred: {msg}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_node() {
        // mirrors shred-examples.mjs slugify (strip path/ext done by the caller; here on the file stem)
        assert_eq!(slugify("Basics"), "basics");
        assert_eq!(slugify("AdHocPolymorphism"), "adhocpolymorphism");
        assert_eq!(slugify("Records-Tuples"), "records-tuples");
        assert_eq!(slugify("Platform  Overview!!"), "platform-overview");
    }

    #[test]
    fn snippet_prints_source_forms() {
        // decode-free: read_all gives the same arena shape the shred decodes from binary.
        let text = "(chapter (slug \"x\") (runnable (source (def (main) (f 5)))))";
        let a = cadenza_syntax_sexpr::read_all(text).unwrap();
        let ch = super::super::locate_chapter(&a).unwrap();
        let runnable = super::super::children(&a, ch)
            .iter()
            .copied()
            .find(|&f| a.head_name(f) == Some("runnable"))
            .unwrap();
        assert_eq!(
            snippet_text(&a, runnable, "source").as_deref(),
            Some("(def (main) (f 5))")
        );
        // wrapped into a compilable program
        assert_eq!(
            wrap_module(
                &snippet_text(&a, runnable, "source").unwrap(),
                Surface::Sexpr
            ),
            "(do (def (main) (f 5)) (export main))"
        );
    }
}
