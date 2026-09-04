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
//! SCOPE: single-file runnables + exercises (canonical: sexpr-authored + the ml toggle) AND multi-file
//! `(files …)` runnables (lowered like the app's fileModel.ts `lowerToCompile` — one entry + preloaded
//! peers, sexpr-only, no toggle, matching the node shred). DEFERRED (emit meta.deferred, no program):
//! mode="test" runnables (they run via the @test-export driver — a v2 shred kind).

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

/// Normalize an authored `surface` value to the two the compiler declares (`"ml"` | `"sexpr"`), as a
/// `'static` literal — every guide surface is one of these, and a stray value degrades to sexpr (the
/// authored default) rather than emitting a bogus surface string into the manifest.
fn surface_lit(s: &str) -> &'static str {
    if s == "ml" { "ml" } else { "sexpr" }
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

/// One authored file of a multi-file `(files …)` runnable — a `(file (name ..)(source ..)(surface ..)[(entry
/// "true")])` sub-form, decoded to its parts.
struct MFile {
    name: String,
    source: String,
    surface: String,
    entry: bool,
}

/// Decode the `(file …)` children of a `(files …)` holder into [`MFile`]s (source = the `(source …)` form
/// children printed, like a single-file program). Skips a child that isn't a well-formed `(file …)`.
fn multifile_files(a: &Arenas, files_holder: StructId) -> Vec<MFile> {
    super::children(a, files_holder)
        .iter()
        .filter_map(|&f| {
            if a.head_name(f) != Some("file") {
                return None;
            }
            Some(MFile {
                name: super::named_attr(a, f, "name")?.to_string(),
                source: snippet_text(a, f, "source")?,
                surface: super::named_attr(a, f, "surface")
                    .unwrap_or("sexpr")
                    .to_string(),
                entry: super::named_attr(a, f, "entry") == Some("true"),
            })
        })
        .collect()
}

/// Lower a multi-file set to `(entry index, preloaded-peer indices in model order)` — a faithful port of
/// fileModel.ts `lowerToCompile`: exactly one `entry:true` file (the genesis compiled as the program), every
/// other file a preloaded peer; non-empty, unique names. Returns the same reason strings on a malformed set.
fn lower_multifile(files: &[MFile]) -> Result<(usize, Vec<usize>), String> {
    if files.is_empty() {
        return Err("empty file set — add at least an entry file.".into());
    }
    if files.iter().any(|f| f.name.is_empty()) {
        return Err("every file needs a non-empty `name` (the import link target).".into());
    }
    for (i, f) in files.iter().enumerate() {
        if files[..i].iter().any(|g| g.name == f.name) {
            return Err(format!(
                "duplicate file name(s): {} — each file needs a unique name (imports resolve by name).",
                f.name
            ));
        }
    }
    let entries: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.entry)
        .map(|(i, _)| i)
        .collect();
    match entries.as_slice() {
        [] => {
            Err("no entry file — mark exactly one file `entry: true` (the genesis program).".into())
        }
        [e] => Ok((*e, (0..files.len()).filter(|&i| i != *e).collect())),
        _ => Err(format!(
            "multiple entry files ({}) — exactly one file may be the entry.",
            entries
                .iter()
                .map(|&i| files[i].name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
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
    /// The program files to write, `(filename, contents)` — `program.<surface>` plus, for a multi-file case,
    /// each preloaded `module-<name>.<surface>` peer. Empty for a deferred case.
    files: Vec<(String, String)>,
    /// Multi-file preloaded peers `(name, surface)` in model order (empty for a single-file case) — the flake
    /// converts each to `module-<name>.<surface>` + passes `--entry`.
    peers: Vec<(String, String)>,
    entry_name: Option<String>,
    multi_file: bool,
    authored_surface: Option<&'static str>,
    expected: Option<String>,
    /// The source file this case came from, for `meta.file` + the per-file aggregate grouping — a chapter's
    /// `src/content/chapters/<Stem>.tsx` or the playground's `src/playground/examples.ts`.
    file: String,
    /// The playground example's `id` (playground cases only) — `meta.playgroundId`, mirrors the node shred.
    playground_id: Option<String>,
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

/// Derive one runnable/exercise case from its node. `cdz` renders the ml surface. mode="test" runnables are
/// marked deferred (no program) — they need the @test-export driver. `stem` is the dir-slug source; `file`
/// is the `meta.file` path (a chapter's `src/content/chapters/<Stem>.tsx`, or HomePage's component path).
#[allow(clippy::too_many_arguments)] // the case-derivation context (arena, node, kind, stem, file, idx,
// cdz, prose-value assertions) is inherent; bundling would obscure more than it clarifies.
fn derive_case(
    a: &Arenas,
    node: StructId,
    kind: &'static str,
    stem: &str,
    file: &str,
    idx: usize,
    cdz: &str,
    results: &std::collections::HashMap<String, String>,
) -> Result<Case, String> {
    let dir = format!("{idx:04}-{}", slugify(stem));
    let file = file.to_string();

    // mode="test" runnables: deferred (they run via the @test-export driver, a v2 shred kind).
    if super::named_attr(a, node, "mode") == Some("test") {
        return Ok(Case {
            dir,
            kind: "test-mode",
            graded: false,
            expect_kind: "value",
            surfaces: vec![],
            deferred: true,
            reason: Some(
                "mode=test runnable runs via the @test-export driver (v2 shred kind)".into(),
            ),
            files: vec![],
            peers: vec![],
            entry_name: None,
            multi_file: false,
            authored_surface: None,
            expected: None,
            file,
            playground_id: None,
        });
    }

    // Prose-value gate (operator greenlit 2026-09-03): if this runnable is NAMED `(id "slug")` and the
    // chapter's prose carries a `(result (of "slug") <value>)` assertion, GRADE the runnable against that
    // asserted value — reusing the existing `expected=` path (no new manifest field). An inline `(expected
    // …)` takes precedence when both are present (q6 coexist: the pin the author wrote on the runnable wins);
    // otherwise the prose assertion supplies `expected`, so a value-shifting change reds guideExamplesShredded.
    let expected = super::named_attr(a, node, "expected")
        .map(str::to_string)
        .or_else(|| super::named_attr(a, node, "id").and_then(|id| results.get(id).cloned()));
    let expect_kind = if super::named_attr(a, node, "expect") == Some("error") {
        "error"
    } else {
        "value"
    };

    // MULTI-FILE `(files …)`: lower the set like fileModel.ts `lowerToCompile` — entry → program.<surface>,
    // preloaded peers → module-<name>.<surface>. Full modules (imports/exports), so NOT wrapped, and authored
    // in one surface with no toggle (matches the node shred: surfaces=[from]).
    if let Some(files_holder) = super::named_node(a, node, "files") {
        let mfiles = multifile_files(a, files_holder);
        let (entry_idx, peer_idxs) =
            lower_multifile(&mfiles).map_err(|e| format!("{dir}: multi-file won't lower — {e}"))?;
        let from = surface_lit(&mfiles[entry_idx].surface);
        let mut files = vec![(format!("program.{from}"), mfiles[entry_idx].source.clone())];
        let mut peers = Vec::new();
        for &pi in &peer_idxs {
            let s = surface_lit(&mfiles[pi].surface);
            files.push((
                format!("module-{}.{s}", mfiles[pi].name),
                mfiles[pi].source.clone(),
            ));
            peers.push((mfiles[pi].name.clone(), s.to_string()));
        }
        return Ok(Case {
            dir,
            kind: "multi-file",
            graded: expected.is_some(),
            expect_kind,
            surfaces: vec![from],
            deferred: false,
            reason: None,
            files,
            peers,
            entry_name: Some("main".into()),
            multi_file: true,
            authored_surface: Some(from),
            expected,
            file,
            playground_id: None,
        });
    }

    // SINGLE-FILE: runnable → (source); exercise → (solution) (the gradeable correct program). Wrap the
    // sexpr snippet + render the ml toggle.
    let src_name = if kind == "exercise" {
        "solution"
    } else {
        "source"
    };
    let snippet = snippet_text(a, node, src_name)
        .ok_or_else(|| format!("{dir}: no ({src_name} …) program"))?;
    let program_sexpr = wrap_module(&snippet, Surface::Sexpr);
    let program_ml = render_ml(cdz, &program_sexpr)?;

    Ok(Case {
        dir,
        kind,
        graded: expected.is_some(),
        expect_kind,
        surfaces: vec!["sexpr", "ml"],
        deferred: false,
        reason: None,
        files: vec![
            ("program.sexpr".to_string(), program_sexpr),
            ("program.ml".to_string(), program_ml),
        ],
        peers: vec![],
        entry_name: None,
        multi_file: false,
        authored_surface: Some("sexpr"),
        expected,
        file,
        playground_id: None,
    })
}

/// Derive a shred case from a playground example — a WHOLE-program sexpr module (NOT wrapped) rendered in
/// BOTH surfaces (the reader toggles it), graded by `expected` / `expect-error`. Mirrors the node shred's
/// playground path (kind="playground", playgroundId, surfaces=[authored, other]). dir slug = `NNNN-examples`.
fn derive_playground_case(
    pe: &crate::playground::PlaygroundExample,
    idx: usize,
    cdz: &str,
) -> Result<Case, String> {
    let from = surface_lit(&pe.surface);
    if from != "sexpr" {
        return Err(format!(
            "playground {}: surface {:?} — the shred renders sexpr-authored playground programs only",
            pe.id, pe.surface
        ));
    }
    let other = "ml";
    // source is a full `(do …)` module; render the toggle surface via cdz convert (wrapping-free).
    let ml_src = render_ml(cdz, &pe.source)?;
    Ok(Case {
        dir: format!("{idx:04}-examples"),
        kind: "playground",
        graded: pe.expected.is_some(),
        expect_kind: if pe.expect_error { "error" } else { "value" },
        surfaces: vec![from, other],
        deferred: false,
        reason: None,
        files: vec![
            ("program.sexpr".to_string(), pe.source.clone()),
            ("program.ml".to_string(), ml_src),
        ],
        peers: vec![],
        entry_name: None,
        multi_file: false,
        authored_surface: Some(from),
        expected: pe.expected.clone(),
        file: "src/playground/examples.ts".to_string(),
        playground_id: Some(pe.id.clone()),
    })
}

fn json_str(s: &str) -> String {
    super::json_string(s)
}

/// Render a case's `peers` array as JSON `[{ "name": .., "surface": .. }, …]` (empty `[]` for single-file).
fn peers_json(peers: &[(String, String)]) -> String {
    let items: Vec<String> = peers
        .iter()
        .map(|(n, s)| {
            format!(
                "{{ \"name\": {}, \"surface\": {} }}",
                json_str(n),
                json_str(s)
            )
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// Collect a chapter/doc's inline `(result (of "slug") <value>)` prose assertions → `slug -> asserted-value
/// text`. Walks ALL descendants (the tags live inside prose `(p …)`/`(note …)` blocks the case-derivation
/// loop doesn't otherwise visit). The value is printed canonically (`print_from`) so a bare scalar `6` →
/// `"6"`, matching a runnable's rendered scalar run output. The prose-value gate linkage (operator greenlit
/// 2026-09-03): `derive_case` injects this as the runnable id'd `slug`'s `expected`.
fn collect_result_assertions(
    a: &Arenas,
    root: StructId,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if a.head_name(n) == Some("result")
            && let (Some(slug), Some(v)) =
                (super::result_of_slug(a, n), super::result_value_node(a, n))
        {
            out.insert(slug.to_string(), cadenza_syntax_sexpr::print_from(a, v));
        }
        for &c in super::children(a, n) {
            stack.push(c);
        }
    }
    out
}

/// SPAN-SLICE a chapter's `(result (of "slug") <value>)` assertions VERBATIM from its AUTHORED `.sexp` —
/// `slug -> the value's exact source text`. This preserves a value's PER-TYPE member surface, which the
/// spanless [`collect_result_assertions`] cannot: the decoded binary AST is spanless, so its `print_from`
/// renders every `Leaf::Member` STRUCTURAL (`(. Qty of)`), but the RUNTIME value-render sugars Qty/Unit keys
/// to bare dotted names (`Qty.of`), so a Qty/Unit `(result …)` pin would diverge from the runtime `got` and
/// RED the gate (the #7804 playground bug class, here in the chapter path). Since `Qty.of` and `(. Ast List)`
/// parse to the SAME `Leaf::Member`, NO re-render printer can reproduce the per-type surface — only the
/// author's own source text does. Mirrors `playground::expected_value`'s verbatim slice. For every OTHER type
/// (scalars, tuple/list/record/Some/Rational/BigInt/String, and structural-authored Ast) the sliced text
/// equals what `print_from` produced, so this is a no-op for the already-gated pins and a fix only for the
/// sugared-member types. Returns `None` when the sibling `.sexp` is absent (a native/test invocation), so the
/// caller falls back to the spanless collection.
fn collect_result_assertions_spanned(
    sexp_path: &std::path::Path,
) -> Option<std::collections::HashMap<String, String>> {
    let text = std::fs::read_to_string(sexp_path).ok()?;
    let (a, spans) = cadenza_syntax_sexpr::read_all_spanned(&text).ok()?;
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![a.root];
    while let Some(n) = stack.pop() {
        if a.head_name(n) == Some("result")
            && let (Some(slug), Some(v)) = (
                super::result_of_slug(&a, n),
                super::result_value_node(&a, n),
            )
            && let Some(sp) = spans.get(v)
        {
            out.insert(slug.to_string(), text[sp.start..sp.end].to_string());
        }
        for &c in super::children(&a, n) {
            stack.push(c);
        }
    }
    Some(out)
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
        // A .cdzb is either a chapter doc (runnable/exercise) or the playground doc (examples).
        if let Some(chapter) = super::locate_chapter(&a) {
            // Prose-value gate: gather the chapter's (result (of "slug") …) assertions once, so each named
            // runnable can be graded against its in-text asserted value (derive_case injects it as `expected`).
            // Prefer the VERBATIM span-slice from the sibling authored `.sexp` (preserves a Qty/Unit value's
            // sugared member surface, matching the runtime; the spanless print_from would render it structural
            // and RED the gate — #7804 class). The sibling sits at the shred's cwd (guideShred runs from the
            // guide src root; the `.cdzb` stem is exactly its `src/content/chapters/<stem>.sexp` basename).
            // Fall back to the spanless collection when the sibling is absent (a native/test invocation).
            let sibling = std::path::Path::new("src/content/chapters").join(format!("{stem}.sexp"));
            let results = collect_result_assertions_spanned(&sibling)
                .unwrap_or_else(|| collect_result_assertions(&a, chapter));
            for &f in super::children(&a, chapter) {
                let kind = match a.head_name(f) {
                    Some("runnable") => "runnable",
                    Some("exercise") => "exercise",
                    _ => continue,
                };
                idx += 1;
                let file = format!("src/content/chapters/{stem}.tsx");
                let case = derive_case(&a, f, kind, &stem, &file, idx, cdz, &results)
                    .unwrap_or_else(|e| die(&format!("shred {path} #{idx}: {e}")));
                write_case(out_dir, &case);
                cases.push(case);
            }
        } else if let Some(homepage) = super::locate_homepage(&a) {
            // HomePage landing page: its `(runnable …)` are chapter-style runnables (bare expr, both
            // surfaces), attributed to the component file. (fork1b — the last of the 60-case gap.)
            let results = collect_result_assertions(&a, homepage);
            for &f in super::children(&a, homepage) {
                if a.head_name(f) != Some("runnable") {
                    continue;
                }
                idx += 1;
                let case = derive_case(
                    &a,
                    f,
                    "runnable",
                    "HomePage",
                    "src/components/HomePage.tsx",
                    idx,
                    cdz,
                    &results,
                )
                .unwrap_or_else(|e| die(&format!("shred {path} #{idx}: {e}")));
                write_case(out_dir, &case);
                cases.push(case);
            }
        } else {
            // Playground: a per-example `(example …)` .cdzb (seq-279 file-per-example) or a legacy
            // `(playground …)` doc. Each example is a whole-program module rendered in both surfaces.
            //
            // For a per-example `.cdzb`, RE-READ the sibling authored `.sexp` (spanned) so the `(expected …)`
            // pin is span-sliced VERBATIM — the decoded binary AST is SPANLESS, so a spanless read would
            // re-render a Qty pin STRUCTURAL `(. Qty of)` while the runtime `got` renders it FLAT `Qty.of`
            // (#7616), RED-ing localGate (guide-editor 2026-09-02). The `.sexp` sits at the shred's cwd: the
            // guideShred derivation runs from the guide src root and each playground `.cdzb` stem is exactly
            // its `src/playground/examples/<stem>.sexp` basename. Fall back to the spanless binary read for a
            // legacy `(playground …)` doc or when the sibling is absent (a native/test invocation) — that path
            // only re-renders structural, which is still correct for a structural-authored (e.g. Ast) pin.
            let sibling =
                std::path::Path::new("src/playground/examples").join(format!("{stem}.sexp"));
            let examples = if sibling.is_file() {
                vec![
                    crate::playground::read_one_example_file(&sibling)
                        .unwrap_or_else(|e| die(&format!("shred {}: {e}", sibling.display()))),
                ]
            } else {
                crate::playground::read_playground_any(&a)
                    .unwrap_or_else(|e| die(&format!("{path}: {e}")))
            };
            for pe in &examples {
                idx += 1;
                let case = derive_playground_case(pe, idx, cdz)
                    .unwrap_or_else(|e| die(&format!("shred {path} #{idx}: {e}")));
                write_case(out_dir, &case);
                cases.push(case);
            }
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
    for (name, body) in &c.files {
        w(name, body);
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
        json_str(&c.file),
        json_str(c.kind),
        c.graded,
        json_str(c.expect_kind),
        surfaces,
    );
    if let Some(s) = c.authored_surface {
        meta.push_str(&format!(",\n  \"authoredSurface\": {}", json_str(s)));
    }
    if let Some(pid) = &c.playground_id {
        meta.push_str(&format!(",\n  \"playgroundId\": {}", json_str(pid)));
    }
    if c.multi_file {
        meta.push_str(",\n  \"multiFile\": true");
        if let Some(e) = &c.entry_name {
            meta.push_str(&format!(",\n  \"entryName\": {}", json_str(e)));
        }
        meta.push_str(&format!(",\n  \"peers\": {}", peers_json(&c.peers)));
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
            let mut e = format!(
                "    {{ \"dir\": {}, \"file\": {}, \"kind\": {}, \"graded\": {}, \"expectKind\": {}, \"surfaces\": [{}], \"deferred\": {}",
                json_str(&c.dir),
                json_str(&c.file),
                json_str(c.kind),
                c.graded,
                json_str(c.expect_kind),
                surfaces,
                c.deferred,
            );
            if let Some(pid) = &c.playground_id {
                e.push_str(&format!(", \"playgroundId\": {}", json_str(pid)));
            }
            if c.multi_file {
                if let Some(en) = &c.entry_name {
                    e.push_str(&format!(", \"entryName\": {}", json_str(en)));
                }
                e.push_str(&format!(", \"peers\": {}", peers_json(&c.peers)));
            }
            e.push_str(" }");
            e
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

    /// The chapter-path Qty fix (#7804 class): the span-slice preserves a Qty/Unit value's SUGARED member
    /// surface (`Qty.of`, matching the runtime value-render), where the spanless `print_from` structuralizes
    /// it (`(. Qty of)`) and would RED the gate. Guards the verbatim-slice against a future "simplify back to
    /// print_from" regression, exactly like `playground::read_one_example_file`'s baked-in guard.
    #[test]
    fn spanned_result_preserves_sugared_qty_member_surface() {
        let text = r##"(chapter (slug "x") (title "T") (pillar "p") (section "s") (blurb "b") (lede "l")
  (p "len " (result (of "q") (: (Qty.of 5.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))) ":")
  (runnable (id "q") (source (Qty.of 5.0 (Unit.of #"meter")))))"##;
        let dir = std::env::temp_dir().join(format!("cdz-shred-spantest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("QtyChap.sexp");
        std::fs::write(&p, text).unwrap();
        // SPAN-SLICE preserves the authored sugared surface verbatim.
        let spanned = collect_result_assertions_spanned(&p).unwrap();
        assert_eq!(
            spanned.get("q").map(String::as_str),
            Some(r##"(: (Qty.of 5.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))"##)
        );
        // CONTRAST: the spanless print_from STRUCTURALIZES the members — the divergence the slice fixes.
        let a = cadenza_syntax_sexpr::read_all(text).unwrap();
        let ch = super::super::locate_chapter(&a).unwrap();
        let spanless = collect_result_assertions(&a, ch);
        assert!(
            spanless.get("q").is_some_and(|s| s.contains("(. Qty of)")),
            "spanless print_from should structuralize the Qty member: {:?}",
            spanless.get("q")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn playground_case_rejects_ml_surface() {
        // the cdz-free guard: an ml-authored playground example is rejected before any render (the sexpr
        // path is validated end-to-end — it renders the ml toggle via `cdz convert`). All 59 real examples
        // are sexpr, so this only guards a future ml-authored one from silently mis-shredding.
        let pe = crate::playground::PlaygroundExample {
            id: "x".into(),
            name: "X".into(),
            theme: "basics".into(),
            surface: "ml".into(),
            source: "def main() = 1".into(),
            expected: None,
            expect_error: false,
        };
        match derive_playground_case(&pe, 1, "cdz-unused") {
            Err(e) => assert!(
                e.contains("sexpr-authored playground programs only"),
                "got: {e}"
            ),
            Ok(_) => panic!("expected an ml-surface playground example to be rejected"),
        }
    }

    #[test]
    fn lower_multifile_validates_and_orders() {
        let mk = |name: &str, entry: bool| MFile {
            name: name.into(),
            source: format!("(do (export {name}))"),
            surface: "sexpr".into(),
            entry,
        };
        // one entry, peers in model order (non-entry, stable)
        let files = vec![mk("events", false), mk("reducer", true)];
        assert_eq!(lower_multifile(&files).unwrap(), (1, vec![0]));
        // no entry
        assert!(
            lower_multifile(&[mk("a", false)])
                .unwrap_err()
                .contains("no entry file")
        );
        // multiple entries
        assert!(
            lower_multifile(&[mk("a", true), mk("b", true)])
                .unwrap_err()
                .contains("multiple entry files")
        );
        // duplicate names
        assert!(
            lower_multifile(&[mk("a", true), mk("a", false)])
                .unwrap_err()
                .contains("duplicate file name")
        );
        // empty set
        assert!(lower_multifile(&[]).unwrap_err().contains("empty file set"));
    }

    #[test]
    fn multifile_case_emits_program_and_peers() {
        // the PlatformExecution shape: events (peer) + reducer (entry), both sexpr.
        let text = "(chapter (slug \"x\") (runnable (files \
            (file (name \"events\") (source (do (export turn))) (surface \"sexpr\")) \
            (file (name \"reducer\") (source (do (export main))) (surface \"sexpr\") (entry \"true\")))))";
        let a = cadenza_syntax_sexpr::read_all(text).unwrap();
        let ch = super::super::locate_chapter(&a).unwrap();
        let runnable = super::super::children(&a, ch)
            .iter()
            .copied()
            .find(|&f| a.head_name(f) == Some("runnable"))
            .unwrap();
        // cdz is not invoked for multi-file (sexpr-only, no ml render), so a dummy path is fine.
        let case = derive_case(
            &a,
            runnable,
            "runnable",
            "PlatformExecution",
            "src/content/chapters/PlatformExecution.tsx",
            7,
            "cdz",
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert_eq!(case.kind, "multi-file");
        assert!(case.multi_file && !case.deferred);
        assert_eq!(case.surfaces, vec!["sexpr"]);
        assert_eq!(case.entry_name.as_deref(), Some("main"));
        assert_eq!(
            case.peers,
            vec![("events".to_string(), "sexpr".to_string())]
        );
        // entry program + one preloaded peer module, entry compiled as program.sexpr.
        assert_eq!(
            case.files,
            vec![
                (
                    "program.sexpr".to_string(),
                    "(do (export main))".to_string()
                ),
                (
                    "module-events.sexpr".to_string(),
                    "(do (export turn))".to_string()
                ),
            ]
        );
    }
}
