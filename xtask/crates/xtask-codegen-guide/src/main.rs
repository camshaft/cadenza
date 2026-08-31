//! xtask-codegen-guide — guide sexp→TSX codegen (cadenza-docs I5). Reads a chapter `.sexp` via the MAIN
//! parser (`cadenza_syntax_sexpr::read_all` → `cadenza_ast::ast::Arenas`, the binary-AST interchange),
//! walks the guide-doc heads, and emits the `@generated` TSX chapter module — replacing the node
//! `scripts/codegen-chapters.mjs`. Operator: one parser (Rust), no node parser; binary AST = interchange.
//!
//! Renders: chapter meta → H1 + Lede; ordered blocks h2/p/note (prose) + runnable/exercise/why (I5 example
//! blocks); inline text/em/c/br/strong/link/app-link. This IS the guide's codegen engine (the earlier node
//! prose core was retired when the whole guide flipped to .sexp); `check:codegen-sync` pins each committed
//! `.tsx` to this render, and example blocks emit extraction-compatible + DOM-correct TSX (fidelity =
//! `check:codegen` DOM vs pre-flip hand-written),
//! including multi-file `(files (file …) …)` runnables. Usage: `[--check] <chapter.sexp>`.
use cadenza_ast::ast::{Arenas, Struct, StructId};
use cadenza_syntax_core::spans::SpanTable;

// Guide shred (operator: shred in Rust from the binary AST). `wrap` = the wrapModule port; `shred` = the
// `--shred` mode (decode binary AST → walk (source) subtrees → wrap + render + emit the corpus cases).
mod homepage;
mod playground;
mod shred;
mod wrap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let migrate = args.iter().any(|a| a == "--migrate");
    let registry = args.iter().any(|a| a == "--registry");
    let playground_registry = args.iter().any(|a| a == "--playground-registry");
    let playground_bootstrap = args.iter().any(|a| a == "--playground-bootstrap");
    let homepage = args.iter().any(|a| a == "--homepage");

    // --shred <out-dir> <cdz-bin> <ordered .cdzb list>: the guide shred (binary-AST-in). Positional args:
    // out-dir, the cdz binary (for the sexpr→ml render), then the chapter binary-AST files in case order.
    if args.iter().any(|a| a == "--shred") {
        let pos: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .collect();
        if pos.len() < 3 {
            eprintln!(
                "usage: xtask-codegen-guide --shred <out-dir> <cdz-bin> <ch1.cdzb> [ch2.cdzb …]"
            );
            std::process::exit(2);
        }
        shred::run_shred(&pos[0], &pos[1], &pos[2..]);
        return;
    }

    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => {
            eprintln!(
                "usage: xtask-codegen-guide [--check] <chapter.sexp>  |  --registry [--check] <chapters.ts>"
            );
            std::process::exit(2);
        }
    };

    // --registry: derive the chapter registry (chapters.ts CHAPTERS[]) from the .sexp set + chapter-order.txt,
    // replacing the `// <generated:chapters>` region in place (or --check). The positional arg is chapters.ts,
    // NOT a chapter .sexp — so this branches before the single-chapter read below. (Ported from the retired
    // node codegen-registry.mjs — operator: no codegen in JavaScript, keep it in the xtask.)
    if registry {
        run_registry(&path, check);
        return;
    }

    // --playground-registry: regenerate (or --check) the EXAMPLES[] region of the playground's examples.ts
    // from the sibling examples.sexp source-of-truth (fork1a). The positional arg is examples.ts.
    if playground_registry {
        playground::run_playground_registry(&path, check);
        return;
    }

    // --playground-bootstrap: one-time fork1a migration — emit examples.sexp from the hand-authored
    // examples.ts (in Rust; the operator's directive is no JS tooling). Positional arg is examples.ts.
    if playground_bootstrap {
        playground::run_playground_bootstrap(&path);
        return;
    }

    // --homepage: regenerate (or --check) HomePageExamples.ts from the sibling HomePage.sexp (fork1b). The
    // positional arg is HomePage.sexp.
    if homepage {
        homepage::run_homepage_registry(&path, check);
        return;
    }

    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("xtask-codegen-guide: read {path}: {e}");
        std::process::exit(1);
    });
    // read_all_spanned (not read_all): the SpanTable lets us span-slice the ORIGINAL .sexp text for embedded
    // code (seq-213/214). We never slice the synthetic (do) root (it spans the whole input) — only real
    // (source …) form children, whose spans are accurate at any depth.
    let (a, spans) = cadenza_syntax_sexpr::read_all_spanned(&text).unwrap_or_else(|e| {
        eprintln!("xtask-codegen-guide: parse {path}: {e:?}");
        std::process::exit(1);
    });
    let chapter = locate_chapter(&a).unwrap_or_else(|| {
        eprintln!("xtask-codegen-guide: no (chapter …) form in {path}");
        std::process::exit(1);
    });

    // --migrate: rewrite eligible `(source "…")` / `(starter …)` / `(solution …)` STRING literals in place as
    // embedded AST forms (seq-213/214). Eligible = a sexpr-surface code string that re-parses cleanly; ML
    // sources (authored-in "ml") and non-parsing fragments are left as strings. Render output is unchanged by
    // construction (code_payload span-slices the embedded forms back to the same text), which the caller
    // verifies with --check. Idempotent: an already-embedded source has no string atom to migrate.
    if migrate {
        match migrate_sources(&text, &a, &spans, chapter) {
            Some((new_text, n)) => {
                std::fs::write(&path, &new_text).unwrap_or_else(|e| {
                    eprintln!("xtask-codegen-guide --migrate: write {path}: {e}");
                    std::process::exit(1);
                });
                println!("migrated {n} source(s) → embedded AST in {path}");
            }
            None => println!(
                "{path}: no eligible string source to embed (ml/unparsing/already-embedded)"
            ),
        }
        return;
    }

    let tsx = render_chapter(&a, chapter);

    if check {
        let sibling = std::path::Path::new(&path).with_extension("tsx");
        let committed = std::fs::read_to_string(&sibling).unwrap_or_else(|e| {
            eprintln!(
                "xtask-codegen-guide --check: read {}: {e}",
                sibling.display()
            );
            std::process::exit(1);
        });
        if tsx == committed {
            println!("✓ {} → {} byte-identical", path, sibling.display());
        } else {
            eprintln!(
                "✗ {} render DIFFERS from committed {}",
                path,
                sibling.display()
            );
            std::process::exit(1);
        }
    } else {
        print!("{tsx}");
    }
}

fn locate_chapter(a: &Arenas) -> Option<StructId> {
    if a.head_name(a.root) == Some("chapter") {
        return Some(a.root);
    }
    if let Struct::List(items) = a.get(a.root) {
        return items
            .iter()
            .copied()
            .find(|&c| a.head_name(c) == Some("chapter"));
    }
    None
}

/// Find the `(homepage …)` root form (the HomePage landing-page examples doc; mirrors `locate_chapter`).
fn locate_homepage(a: &Arenas) -> Option<StructId> {
    if a.head_name(a.root) == Some("homepage") {
        return Some(a.root);
    }
    if let Struct::List(items) = a.get(a.root) {
        return items
            .iter()
            .copied()
            .find(|&c| a.head_name(c) == Some("homepage"));
    }
    None
}

/// The children (`fields[1..]`) of a `(head …)` list node — the head is `fields[0]`.
fn children(a: &Arenas, id: StructId) -> &[StructId] {
    match a.get(id) {
        Struct::List(f) => &f[1..],
        Struct::Atom(_) => &[],
    }
}

/// The lone string value of a `(key "value")` attribute form, e.g. `(slug "x")` → `"x"`.
fn attr_str(a: &Arenas, id: StructId) -> Option<&str> {
    children(a, id).first().and_then(|&v| a.as_str(v))
}

/// The string of a NAMED sub-attribute `(name "value")` among a node's children, e.g. `(source "…")`.
fn named_attr<'a>(a: &'a Arenas, node: StructId, name: &str) -> Option<&'a str> {
    children(a, node)
        .iter()
        .find(|&&f| a.head_name(f) == Some(name))
        .and_then(|&f| attr_str(a, f))
}

/// The NAMED sub-form NODE `(name …)` among a node's children (the holder itself, not its value).
fn named_node(a: &Arenas, node: StructId, name: &str) -> Option<StructId> {
    children(a, node)
        .iter()
        .copied()
        .find(|&f| a.head_name(f) == Some(name))
}

/// The CODE text of a runnable/exercise `(name …)` sub-form, supporting BOTH shapes:
///   • `(name "…")` — a single STRING atom child → its (unescaped) string, exactly as `named_attr` returned
///     it. The pre-embed-AST form; still valid for snippets that don't parse as forms (deliberate syntax-error
///     examples) or during incremental migration.
///   • `(name <form>…)` — nested AST forms (seq-213/214): the code is authored as real s-expr sub-trees, not a
///     string. We recover the DISPLAYED source by SPAN-SLICING the original `.sexp` text from the first form's
///     start to the last form's end — preserving the author's EXACT formatting and comments (they ride in the
///     sliced bytes). The canonical sexpr printer flattens/reformats, so it is NOT used for display; and since
///     the converter writes embedded forms with continuation lines at column 0 (as the string form did), the
///     slice reproduces the pre-flip displayed code byte-for-byte.
/// `None` when the sub-form is absent (preserves each caller's "emit only if present" behavior).
fn code_payload(a: &Arenas, node: StructId, name: &str) -> Option<String> {
    let holder = named_node(a, node, name)?;
    let kids = children(a, holder);
    let &first = kids.first()?;
    if kids.len() == 1
        && matches!(a.get(first), Struct::Atom(_))
        && let Some(s) = a.as_str(first)
    {
        return Some(s.to_string());
    }
    let parts: Vec<String> = kids
        .iter()
        .map(|&k| cadenza_syntax_sexpr::print_pretty_from(a, k, cadenza_syntax_core::DEFAULT_WIDTH))
        .collect();
    Some(parts.join("\n\n"))
}

// ---- --migrate: rewrite eligible `(name "…")` code STRINGS as embedded AST forms `(name <forms>)` ----

/// Rewrite a chapter's runnable/exercise code STRING literals as embedded AST forms, in place. Returns
/// `(new_text, count)` or `None` if nothing was eligible. Eligibility (conservative — the render output must
/// stay byte-identical, verified by `--check`): a runnable's `(source …)` (+ each multi-file `(file (source
/// …))`) and an exercise's `(starter …)`/`(solution …)`, WHEN the sub-form is a single string atom whose
/// (unescaped) content re-parses cleanly as s-expr. ML sources (`authored-in "ml"` — not s-expr) and any
/// content that does not parse are left as strings. Replacements are applied right-to-left so earlier byte
/// offsets stay valid. The content is spliced RAW (unescaped, continuation lines at their original column 0,
/// exactly as the string held them) so `code_payload`'s span-slice reproduces the same displayed code.
fn migrate_sources(
    text: &str,
    a: &Arenas,
    spans: &SpanTable,
    chapter: StructId,
) -> Option<(String, usize)> {
    let mut repls: Vec<(usize, usize, String)> = Vec::new();
    for &f in children(a, chapter) {
        match a.head_name(f) {
            Some("runnable") => {
                if named_attr(a, f, "authored-in") == Some("ml") {
                    continue; // ML surface is not s-expr — never embed.
                }
                collect_embed(a, spans, f, "source", &mut repls);
                if let Some(files) = named_node(a, f, "files") {
                    for &file in children(a, files) {
                        if a.head_name(file) == Some("file") {
                            collect_embed(a, spans, file, "source", &mut repls);
                        }
                    }
                }
            }
            Some("exercise") => {
                collect_embed(a, spans, f, "starter", &mut repls);
                collect_embed(a, spans, f, "solution", &mut repls);
            }
            _ => {}
        }
    }
    if repls.is_empty() {
        return None;
    }
    let n = repls.len();
    repls.sort_by_key(|r| std::cmp::Reverse(r.0)); // apply right-to-left: later offsets first
    let mut out = text.to_string();
    for (s, e, content) in repls {
        out.replace_range(s..e, &content);
    }
    Some((out, n))
}

/// If `node`'s `(name …)` sub-form is a single STRING atom whose content re-parses as s-expr, queue a
/// replacement of that string LITERAL's span (quotes included) with the raw (unescaped) content.
fn collect_embed(
    a: &Arenas,
    spans: &SpanTable,
    node: StructId,
    name: &str,
    out: &mut Vec<(usize, usize, String)>,
) {
    let Some(holder) = named_node(a, node, name) else {
        return;
    };
    let kids = children(a, holder);
    if kids.len() != 1 {
        return; // already embedded (forms) or empty — nothing to migrate
    }
    let atom = kids[0];
    if !matches!(a.get(atom), Struct::Atom(_)) {
        return; // already a form, not a string
    }
    let Some(content) = a.as_str(atom) else {
        return; // not a string atom (a bare name/int) — leave it
    };
    // Only embed content that re-parses cleanly as s-expr (else it is ML / a fragment — keep the string).
    let Ok(parsed) = cadenza_syntax_sexpr::read_all(content) else {
        return;
    };
    // COLLISION GUARD: if the code is a single STRING-LITERAL expression (e.g. the runnable `"hello, world"`),
    // embedding it yields `(source "…")` — INDISTINGUISHABLE from the old string form, so `code_payload` would
    // take the string path and unescape away the quotes. Leave such sources as strings. (`read_all` wraps top
    // forms in a synthetic `(do …)`, so `children(root)` are the top forms.)
    let roots = children(&parsed, parsed.root);
    if roots.len() == 1
        && matches!(parsed.get(roots[0]), Struct::Atom(_))
        && parsed.as_str(roots[0]).is_some()
    {
        return;
    }
    let Some(sp) = spans.get(atom) else {
        return;
    };
    out.push((sp.start, sp.end, content.to_string()));
}

/// A block in document order — prose (h2/p/note) or an example (runnable/exercise/why).
enum Block<'a> {
    Prose(&'static str, &'a [StructId]), // (tag, inline children)
    Runnable(StructId),
    Exercise(StructId),
    Why(StructId),
    StatusLegend, // carve-out: zero-prop block
}

// ---- render: chapter Arenas → @generated TSX string ----

fn render_chapter(a: &Arenas, chapter: StructId) -> String {
    let mut title = "";
    let mut slug = "";
    let mut lede: Option<&[StructId]> = None;
    let mut blocks: Vec<Block> = Vec::new();
    for &f in children(a, chapter) {
        match a.head_name(f) {
            Some("title") => title = attr_str(a, f).unwrap_or(""),
            Some("slug") => slug = attr_str(a, f).unwrap_or(""),
            Some("lede") => lede = Some(children(a, f)),
            // registry metadata (slug handled above), not rendered into the .tsx.
            Some("pillar") | Some("section") | Some("blurb") | Some("nav-title") => {}
            Some("h2") => blocks.push(Block::Prose("H2", children(a, f))),
            Some("p") => blocks.push(Block::Prose("P", children(a, f))),
            Some("note") => blocks.push(Block::Prose("Note", children(a, f))),
            Some("runnable") => blocks.push(Block::Runnable(f)),
            Some("exercise") => blocks.push(Block::Exercise(f)),
            Some("why") => blocks.push(Block::Why(f)),
            Some("status-legend") => blocks.push(Block::StatusLegend),
            _ => {}
        }
    }

    // Import set — EXACTLY the heads used (tsc noUnusedLocals). Prose: H1 always; Lede/H2/P/Note per use; C
    // if inline code (incl inside example prose). Links: Ch/AppLink. Example components: Runnable/Exercise/Why.
    let mut prose: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    prose.insert("H1");
    if lede.is_some() {
        prose.insert("Lede");
    }
    let mut uses_ch = false;
    let mut uses_app = false;
    let mut uses_try_change = false;
    let (mut uses_runnable, mut uses_exercise, mut uses_why, mut uses_status_legend) =
        (false, false, false, false);
    if let Some(l) = lede {
        scan_inline(
            a,
            l,
            &mut prose,
            &mut uses_ch,
            &mut uses_app,
            &mut uses_try_change,
        );
    }
    for b in &blocks {
        match *b {
            Block::Prose(tag, ch) => {
                prose.insert(tag);
                scan_inline(
                    a,
                    ch,
                    &mut prose,
                    &mut uses_ch,
                    &mut uses_app,
                    &mut uses_try_change,
                );
            }
            Block::Runnable(_) => uses_runnable = true,
            Block::Exercise(n) => {
                uses_exercise = true;
                // prompt/hint inline prose may carry (c …)/links → contribute imports.
                if let Some(p) = named_child(a, n, "prompt") {
                    scan_inline(
                        a,
                        p,
                        &mut prose,
                        &mut uses_ch,
                        &mut uses_app,
                        &mut uses_try_change,
                    );
                }
                if let Some(h) = named_child(a, n, "hint") {
                    scan_inline(
                        a,
                        h,
                        &mut prose,
                        &mut uses_ch,
                        &mut uses_app,
                        &mut uses_try_change,
                    );
                }
            }
            Block::Why(n) => {
                uses_why = true;
                scan_inline(
                    a,
                    why_children(a, n),
                    &mut prose,
                    &mut uses_ch,
                    &mut uses_app,
                    &mut uses_try_change,
                );
            }
            Block::StatusLegend => uses_status_legend = true,
        }
    }

    let mut out = String::new();
    out.push_str("// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).\n");
    out.push_str(&format!(
        "import {{ {} }} from \"../../components/Prose.tsx\";\n",
        prose.iter().copied().collect::<Vec<_>>().join(", ")
    ));
    let mut link_imports: Vec<&str> = Vec::new();
    if uses_ch {
        link_imports.push("Ch");
    }
    if uses_app {
        link_imports.push("AppLink");
    }
    link_imports.sort_unstable();
    if !link_imports.is_empty() {
        out.push_str(&format!(
            "import {{ {} }} from \"../../components/ChapterLink.tsx\";\n",
            link_imports.join(", ")
        ));
    }
    if uses_runnable {
        out.push_str("import { Runnable } from \"../../components/Runnable.tsx\";\n");
    }
    if uses_exercise {
        out.push_str("import { Exercise } from \"../../components/Exercise.tsx\";\n");
    }
    if uses_why {
        out.push_str("import { Why } from \"../../components/Why.tsx\";\n");
    }
    if uses_status_legend {
        out.push_str("import { StatusLegend } from \"../../components/StatusIcon.tsx\";\n");
    }
    if uses_try_change {
        out.push_str("import { TryChange } from \"../../components/TryChange.tsx\";\n");
    }
    out.push('\n');
    out.push_str(&format!("export default function {}() {{\n", pascal(slug)));
    out.push_str("  return (\n");
    out.push_str("    <article>\n");
    out.push_str(&format!("      <H1>{}</H1>\n", escape_text(title)));
    if let Some(l) = lede {
        out.push_str(&format!("      <Lede>{}</Lede>\n", render_inlines(a, l)));
    }
    for b in &blocks {
        match *b {
            Block::Prose(tag, ch) => {
                out.push_str(&format!("      <{tag}>{}</{tag}>\n", render_inlines(a, ch)))
            }
            Block::Runnable(n) => out.push_str(&render_runnable(a, n)),
            Block::Exercise(n) => out.push_str(&render_exercise(a, n)),
            Block::Why(n) => out.push_str(&render_why(a, n)),
            Block::StatusLegend => out.push_str("      <StatusLegend />\n"),
        }
    }
    out.push_str("    </article>\n");
    out.push_str("  );\n");
    out.push_str("}\n");
    out
}

/// The inline children of a named `(name <inline>…)` sub-form (e.g. exercise `(prompt …)`), skipping the name.
fn named_child<'a>(a: &'a Arenas, node: StructId, name: &str) -> Option<&'a [StructId]> {
    children(a, node)
        .iter()
        .find(|&&f| a.head_name(f) == Some(name))
        .map(|&f| children(a, f))
}

/// A `(why (tenet "…") <inline>…)`'s prose children (everything after the leading `(tenet …)` attr).
fn why_children(a: &Arenas, node: StructId) -> &[StructId] {
    let ch = children(a, node);
    if ch.first().map(|&f| a.head_name(f)) == Some(Some("tenet")) {
        &ch[1..]
    } else {
        ch
    }
}

// ---- example blocks (I5) — extraction-compatible + DOM-correct; no @generated byte-parity reference ----

fn render_runnable(a: &Arenas, node: StructId) -> String {
    let mut out = String::from("      <Runnable\n");
    // MULTI-FILE: (files (file (name …)(source …)(surface …)(entry "true")) …) → files={[ {…}, … ]}.
    if let Some(files) = children(a, node)
        .iter()
        .find(|&&f| a.head_name(f) == Some("files"))
    {
        out.push_str("        files={[\n");
        for &file in children(a, *files) {
            if a.head_name(file) != Some("file") {
                continue;
            }
            out.push_str("          {\n");
            out.push_str(&format!(
                "            name: {},\n",
                json_string(named_attr(a, file, "name").unwrap_or(""))
            ));
            out.push_str(&format!(
                "            source: `{}`,\n",
                tmpl(&code_payload(a, file, "source").unwrap_or_default())
            ));
            out.push_str(&format!(
                "            surface: {},\n",
                json_string(named_attr(a, file, "surface").unwrap_or("sexpr"))
            ));
            if named_attr(a, file, "entry") == Some("true") {
                out.push_str("            entry: true,\n");
            }
            out.push_str("          },\n");
        }
        out.push_str("        ]}\n");
    } else {
        let src = tmpl(&code_payload(a, node, "source").unwrap_or_default());
        out.push_str(&format!("        source={{`{src}`}}\n"));
    }
    // Optional scalar props (BOTH single-source + multi-file), fixed order. authored-in → authoredIn; wrap "false" → wrap={false}.
    for (sexp_key, tsx_key) in [
        ("expected", "expected"),
        ("expect", "expect"),
        ("id", "id"),
        ("title", "title"),
        ("mode", "mode"),
        ("authored-in", "authoredIn"),
    ] {
        if let Some(v) = named_attr(a, node, sexp_key) {
            out.push_str(&format!("        {tsx_key}={}\n", jsx_attr(v)));
        }
    }
    if named_attr(a, node, "wrap") == Some("false") {
        out.push_str("        wrap={false}\n");
    }
    out.push_str("      />\n");
    out
}

fn render_exercise(a: &Arenas, node: StructId) -> String {
    let mut out = String::from("      <Exercise\n");
    if let Some(id) = named_attr(a, node, "id") {
        out.push_str(&format!("        id={}\n", jsx_attr(id)));
    }
    if let Some(p) = named_child(a, node, "prompt") {
        out.push_str(&format!(
            "        prompt={{<>{}</>}}\n",
            render_inlines(a, p)
        ));
    }
    if let Some(s) = code_payload(a, node, "starter") {
        out.push_str(&format!("        starter={{`{}`}}\n", tmpl(&s)));
    }
    if let Some(s) = code_payload(a, node, "solution") {
        out.push_str(&format!("        solution={{`{}`}}\n", tmpl(&s)));
    }
    if let Some(e) = named_attr(a, node, "expected") {
        out.push_str(&format!("        expected={}\n", jsx_attr(e)));
    }
    if let Some(h) = named_child(a, node, "hint") {
        out.push_str(&format!("        hint={{<>{}</>}}\n", render_inlines(a, h)));
    }
    out.push_str("      />\n");
    out
}

fn render_why(a: &Arenas, node: StructId) -> String {
    let tenet = named_attr(a, node, "tenet").unwrap_or("");
    format!(
        "      <Why tenet={}>{}</Why>\n",
        jsx_attr(tenet),
        render_inlines(a, why_children(a, node))
    )
}

/// Template-literal escape for a code payload emitted into `` {`…`} `` — so a `\`, backtick, or `${` in the
/// (cooked) source round-trips: JS template cooking of the emitted `` `…` `` yields the original code back.
/// Backslash first (so the `\` this adds for backtick/`${` isn't re-escaped).
fn tmpl(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${")
}

/// A JSX string-valued attribute: `="value"` when the value is plain, else a TEMPLATE literal `` ={`…`} ``
/// (template-escaped) when it holds a `"`/`{`/`}`/`<`/`>` a bare `="…"` couldn't carry. A template (not
/// `={"…"}`) so it matches the hand-written form AND the check:examples extractor's `name={`…`}` grab
/// (which does NOT recognize `{"…"}`) — e.g. a Runnable `expected` of `(: "…" String)`.
fn jsx_attr(v: &str) -> String {
    if v.contains(['"', '{', '}', '<', '>']) {
        format!("{{`{}`}}", tmpl(v))
    } else {
        format!("\"{v}\"")
    }
}

/// Learn the import needs of an inline sequence: `(c …)` → C; chapter link → Ch; app link → AppLink; recurse em/strong/link.
fn scan_inline(
    a: &Arenas,
    ins: &[StructId],
    prose: &mut std::collections::BTreeSet<&'static str>,
    uses_ch: &mut bool,
    uses_app: &mut bool,
    uses_try_change: &mut bool,
) {
    for &i in ins {
        match a.head_name(i) {
            Some("c") => {
                prose.insert("C");
            }
            Some("cdz") => {
                // <Cadenza> is re-exported from Prose.tsx, so it rides the same prose import line as C.
                prose.insert("Cadenza");
            }
            Some("link") => {
                *uses_ch = true;
                scan_inline(
                    a,
                    &children(a, i)[1..],
                    prose,
                    uses_ch,
                    uses_app,
                    uses_try_change,
                );
            }
            Some("app-link") => {
                *uses_app = true;
                scan_inline(
                    a,
                    &children(a, i)[1..],
                    prose,
                    uses_ch,
                    uses_app,
                    uses_try_change,
                );
            }
            Some("try-change") => {
                *uses_try_change = true;
                // children after the 3 (example)(find)(replace) attrs are the inline label.
                scan_inline(
                    a,
                    &children(a, i)[3..],
                    prose,
                    uses_ch,
                    uses_app,
                    uses_try_change,
                );
            }
            Some("em") | Some("strong") => {
                scan_inline(a, children(a, i), prose, uses_ch, uses_app, uses_try_change)
            }
            _ => {}
        }
    }
}

fn render_inlines(a: &Arenas, ins: &[StructId]) -> String {
    ins.iter().map(|&i| render_inline(a, i)).collect()
}

fn render_inline(a: &Arenas, i: StructId) -> String {
    if matches!(a.get(i), Struct::Atom(_))
        && let Some(t) = a.as_str(i)
    {
        return escape_text(t);
    }
    match a.head_name(i) {
        Some("em") => format!("<em>{}</em>", render_inlines(a, children(a, i))),
        Some("strong") => format!("<strong>{}</strong>", render_inlines(a, children(a, i))),
        Some("c") => format!("<C>{}</C>", escape_text(attr_str(a, i).unwrap_or(""))),
        // (cdz "<sexpr>") — a SURFACE-AWARE inline Cadenza span (vs (c …) which stays literal). The body
        // is authored s-expr; <Cadenza> shows it verbatim in the s-expr surface and re-renders it in the
        // conventional (ml) surface at runtime (the codegen only has the s-expr printer). Re-exported from
        // Prose.tsx, so it rides the existing prose import line (prose.insert("Cadenza") in scan_inline).
        Some("cdz") => format!(
            "<Cadenza>{}</Cadenza>",
            escape_text(attr_str(a, i).unwrap_or(""))
        ),
        Some("br") => "<br />".to_string(),
        Some("link") => {
            let slug = children(a, i)
                .first()
                .and_then(|&s| attr_str(a, s))
                .unwrap_or("");
            format!(
                "<Ch to=\"/{slug}\">{}</Ch>",
                render_inlines(a, &children(a, i)[1..])
            )
        }
        Some("app-link") => {
            let route = children(a, i)
                .first()
                .and_then(|&r| attr_str(a, r))
                .unwrap_or("");
            format!(
                "<AppLink to=\"{route}\">{}</AppLink>",
                render_inlines(a, &children(a, i)[1..])
            )
        }
        Some("try-change") => {
            // (try-change (example ..)(find ..)(replace ..) <inline>…). find/replace may hold </>; valid in a JSX string attr.
            let ex = named_attr(a, i, "example").unwrap_or("");
            let find = named_attr(a, i, "find").unwrap_or("");
            let rep = named_attr(a, i, "replace").unwrap_or("");
            format!(
                "<TryChange example=\"{ex}\" find=\"{find}\" replace=\"{rep}\">{}</TryChange>",
                render_inlines(a, &children(a, i)[3..])
            )
        }
        _ => String::new(),
    }
}

/// JSX text escape: wrap in `{"…"}` (JS-string-escaped) when the text
/// has a JSX-significant char `{}<>` OR whitespace JSX would collapse (2+ consecutive spaces, tab, newline).
fn escape_text(text: &str) -> String {
    let has_jsx = text.contains(['{', '}', '<', '>']);
    let has_collapsible = text.contains("  ") || text.contains('\t') || text.contains('\n');
    if has_jsx || has_collapsible {
        format!("{{{}}}", json_string(text))
    } else {
        text.to_string()
    }
}

/// JS `JSON.stringify` of a string: double-quoted, with JSON escapes (matches V8 output for the wrap).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn pascal(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

// ---- --registry: derive chapters.ts CHAPTERS[] from chapter-order.txt + each chapter's .sexp ----

/// One `CHAPTERS[]` entry, derived from a chapter's `.sexp` (title = `nav-title` ?? `title`; pillar emitted
/// only when non-`"language"`; exercises = count of `(exercise …)` blocks, emitted only when > 0; Component
/// is a lazy import of the file `<stem>.tsx`). Formatting matches the hand registry exactly (2/4-space indent,
/// `JSON.stringify`-equivalent string quoting via `json_string`).
fn derive_entry(a: &Arenas, chapter: StructId, stem: &str) -> String {
    let slug = named_attr(a, chapter, "slug").unwrap_or("");
    let title = named_attr(a, chapter, "nav-title")
        .or_else(|| named_attr(a, chapter, "title"))
        .unwrap_or("");
    let blurb = named_attr(a, chapter, "blurb").unwrap_or("");
    let section = named_attr(a, chapter, "section").unwrap_or("");
    let pillar = named_attr(a, chapter, "pillar").filter(|&p| p != "language");
    let exercises = children(a, chapter)
        .iter()
        .filter(|&&f| a.head_name(f) == Some("exercise"))
        .count();

    let mut out = String::from("  {\n");
    out.push_str(&format!("    slug: {},\n", json_string(slug)));
    out.push_str(&format!("    title: {},\n", json_string(title)));
    out.push_str(&format!("    blurb: {},\n", json_string(blurb)));
    if let Some(p) = pillar {
        out.push_str(&format!("    pillar: {},\n", json_string(p)));
    }
    out.push_str(&format!("    section: {},\n", json_string(section)));
    if exercises > 0 {
        out.push_str(&format!("    exercises: {exercises},\n"));
    }
    out.push_str(&format!(
        "    Component: lazy(() => import({})),\n",
        json_string(&format!("./chapters/{stem}.tsx"))
    ));
    out.push_str("  },");
    out
}

/// Regenerate (or `--check`) the `CHAPTERS[]` array region of `chapters.ts` from `chapter-order.txt` (the
/// reading-order stem list, one per line, `#`/blank ignored) + each `<stem>.sexp`. Replaces only the region
/// between the `// <generated:chapters>` and `// </generated:chapters>` markers; the hand-written rest of the
/// file is left untouched.
fn run_registry(chapters_ts: &str, check: bool) {
    let ts_path = std::path::Path::new(chapters_ts);
    let content_dir = ts_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let chapters_dir = content_dir.join("chapters");
    let order_path = content_dir.join("chapter-order.txt");

    let order_text = read_or_die(&order_path.to_string_lossy());
    let stems: Vec<&str> = order_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let entries: Vec<String> = stems
        .iter()
        .map(|stem| {
            let sexp = chapters_dir.join(format!("{stem}.sexp"));
            let t = read_or_die(&sexp.to_string_lossy());
            let a = cadenza_syntax_sexpr::read_all(&t).unwrap_or_else(|e| {
                eprintln!(
                    "xtask-codegen-guide --registry: parse {}: {e:?}",
                    sexp.display()
                );
                std::process::exit(1);
            });
            let ch = locate_chapter(&a).unwrap_or_else(|| {
                eprintln!(
                    "xtask-codegen-guide --registry: no (chapter …) in {}",
                    sexp.display()
                );
                std::process::exit(1);
            });
            derive_entry(&a, ch, stem)
        })
        .collect();

    // Marker ANCHORS (stable prefixes); the BEGIN line carries a DO-NOT-EDIT note that we (re)write in full.
    const BEGIN: &str = "  // <generated:chapters>";
    const END: &str = "  // </generated:chapters>";
    let begin_line = format!(
        "{BEGIN} — DO NOT EDIT; regenerated by `xtask-codegen-guide --registry` (from chapter-order.txt + each chapter's .sexp)"
    );
    let block = format!("{begin_line}\n{}\n{END}", entries.join("\n"));

    let src = read_or_die(chapters_ts);
    let (bi, ei) = match (src.find(BEGIN), src.find(END)) {
        (Some(b), Some(e)) if e >= b => (b, e),
        _ => {
            eprintln!(
                "xtask-codegen-guide --registry: generated-region markers not found in {chapters_ts}"
            );
            std::process::exit(1);
        }
    };
    let next = format!("{}{}{}", &src[..bi], block, &src[ei + END.len()..]);

    if check {
        if next != src {
            eprintln!(
                "✗ xtask-codegen-guide --registry --check: {chapters_ts} CHAPTERS[] is OUT OF SYNC with chapter-order.txt + the .sexp — regenerate and commit."
            );
            std::process::exit(1);
        }
        println!(
            "✓ xtask-codegen-guide --registry --check: chapters.ts CHAPTERS[] ({}) in sync",
            stems.len()
        );
    } else {
        std::fs::write(chapters_ts, &next).unwrap_or_else(|e| {
            eprintln!("xtask-codegen-guide --registry: write {chapters_ts}: {e}");
            std::process::exit(1);
        });
        println!(
            "✓ xtask-codegen-guide --registry: regenerated {} chapter entries in {chapters_ts}",
            stems.len()
        );
    }
}

fn read_or_die(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("xtask-codegen-guide: read {path}: {e}");
        std::process::exit(1);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Arenas, SpanTable) {
        cadenza_syntax_sexpr::read_all_spanned(text).expect("parse")
    }
    fn first(a: &Arenas, chapter: StructId, head: &str) -> StructId {
        children(a, chapter)
            .iter()
            .copied()
            .find(|&f| a.head_name(f) == Some(head))
            .unwrap_or_else(|| panic!("no ({head} …)"))
    }

    #[test]
    fn string_source_payload_is_the_string() {
        let text = "(chapter (slug \"x\") (runnable (source \"(def (main) 5)\")))";
        let a = parse(text).0;
        let r = first(&a, locate_chapter(&a).unwrap(), "runnable");
        assert_eq!(
            code_payload(&a, r, "source").as_deref(),
            Some("(def (main) 5)")
        );
    }

    #[test]
    fn embed_source_is_canonical_pretty() {
        // Embedded forms are CANONICAL-printed (operator ruling; not the author's layout): each top form
        // print_pretty'd, blank-line separated — matching `cdz convert --to sexpr` at DEFAULT_WIDTH.
        let text = "(chapter (slug \"x\")\n  (runnable (source (def (main) (f 5))\n(@ (requires (>= x 0))\n  (def (f (: x Int64)) (+ x 1))))))\n";
        let a = parse(text).0;
        let r = first(&a, locate_chapter(&a).unwrap(), "runnable");
        assert_eq!(
            code_payload(&a, r, "source").unwrap(),
            "(def (main) (f 5))\n\n(@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))"
        );
    }

    #[test]
    fn migrate_embeds_normal_source_but_keeps_string_literal_and_skips_ml() {
        // A lone string-literal runnable (`"hi"`) must stay a STRING (embedding collides with the string
        // form and would lose the quotes); an ML runnable must stay a STRING (not s-expr); a normal sexpr
        // runnable IS embedded as forms.
        let text = "(chapter (slug \"x\")\n  (runnable (source \"\\\"hi\\\"\"))\n  (runnable (source \"def main() = 5\") (authored-in \"ml\"))\n  (runnable (source \"(def (main) 5)\")))\n";
        let (a, spans) = parse(text);
        let ch = locate_chapter(&a).unwrap();
        let (new_text, n) = migrate_sources(text, &a, &spans, ch).expect("one embedded");
        assert_eq!(n, 1, "only the plain sexpr runnable is embedded");
        assert!(
            new_text.contains("(source \"\\\"hi\\\"\")"),
            "string-literal code stays a string"
        );
        assert!(
            new_text.contains("(source \"def main() = 5\")"),
            "ml source stays a string"
        );
        assert!(
            new_text.contains("(source (def (main) 5))"),
            "plain sexpr source is embedded as forms"
        );
        // Idempotent: a second pass finds nothing to embed.
        let (a2, spans2) = parse(&new_text);
        let ch2 = locate_chapter(&a2).unwrap();
        assert!(migrate_sources(&new_text, &a2, &spans2, ch2).is_none());
    }

    #[test]
    fn embed_source_renders_canonical_in_runnable() {
        // An embed-form source renders the CANONICAL code into the Runnable source={`…`} template.
        let embed_text = "(chapter (slug \"x\") (runnable (source (def (main) (f 5)))))";
        let a = parse(embed_text).0;
        let r = first(&a, locate_chapter(&a).unwrap(), "runnable");
        assert!(
            render_runnable(&a, r).contains("source={`(def (main) (f 5))`}"),
            "runnable renders the canonical embedded source"
        );
    }
}
