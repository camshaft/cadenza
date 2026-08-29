//! xtask-codegen-guide — guide sexp→TSX codegen (cadenza-docs I5). Reads a chapter `.sexp` via the MAIN
//! parser (`cadenza_syntax_sexpr::read_all` → `cadenza_ast::ast::Arenas`, the binary-AST interchange),
//! walks the guide-doc heads, and emits the `@generated` TSX chapter module — replacing the node
//! `scripts/codegen-chapters.mjs`. Operator: one parser (Rust), no node parser; binary AST = interchange.
//!
//! Renders: chapter meta → H1 + Lede; ordered blocks h2/p/note (byte-parity with chapterModel.ts) +
//! runnable/exercise/why (I5 example blocks); inline text/em/c/br/strong/link/app-link. The PROSE subset is
//! byte-identical to chapterModel.ts (so `check:codegen-sync` holds on the pilots); example blocks emit
//! extraction-compatible + DOM-correct TSX (fidelity = `check:codegen` DOM vs pre-flip hand-written),
//! including multi-file `(files (file …) …)` runnables. Usage: `[--check] <chapter.sexp>`.
use cadenza_ast::ast::{Arenas, Struct, StructId};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: xtask-codegen-guide [--check] <chapter.sexp>");
            std::process::exit(2);
        }
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("xtask-codegen-guide: read {path}: {e}");
        std::process::exit(1);
    });
    let a = cadenza_syntax_sexpr::read_all(&text).unwrap_or_else(|e| {
        eprintln!("xtask-codegen-guide: parse {path}: {e:?}");
        std::process::exit(1);
    });
    let chapter = locate_chapter(&a).unwrap_or_else(|| {
        eprintln!("xtask-codegen-guide: no (chapter …) form in {path}");
        std::process::exit(1);
    });
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
            Some("pillar") | Some("section") | Some("blurb") => {} // registry metadata, not in .tsx
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
    out.push_str("// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (chapterModel.ts).\n");
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
                tmpl(named_attr(a, file, "source").unwrap_or(""))
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
        let src = tmpl(named_attr(a, node, "source").unwrap_or(""));
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
    if let Some(s) = named_attr(a, node, "starter") {
        out.push_str(&format!("        starter={{`{}`}}\n", tmpl(s)));
    }
    if let Some(s) = named_attr(a, node, "solution") {
        out.push_str(&format!("        solution={{`{}`}}\n", tmpl(s)));
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

/// JSX text escape — matches chapterModel.ts escapeText: wrap in `{"…"}` (JS-string-escaped) when the text
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
