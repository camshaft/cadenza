//! xtask-codegen-guide — guide sexp→TSX codegen (cadenza-docs I5). Reads a chapter `.sexp` via the MAIN
//! parser (`cadenza_syntax_sexpr::read_all` → `cadenza_ast::ast::Arenas`, the binary-AST interchange),
//! walks the guide-doc heads, and emits the `@generated` TSX chapter module — replacing the node
//! `scripts/codegen-chapters.mjs`. Operator: one parser (Rust), no node parser; binary AST = interchange.
//!
//! INCREMENT 2 (this): byte-exact TSX-render PARITY with chapterModel.ts's PROSE codegen (chapter meta →
//! H1 + Lede + h2/p/note blocks; inline text/em/c/br/strong/link/app-link), so `check:codegen-sync` passes
//! on the 2 flipped pilots. Example blocks (runnable/exercise/why) + link-split + registry land next.
//!
//! Usage: `xtask-codegen-guide <chapter.sexp>` prints the rendered TSX to stdout.
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
        // Parity gate: the rendered TSX must byte-match the sibling committed `.tsx` (same stem). Exit 1 on
        // drift — proves the emit-xtask reproduces the current codegen for the flipped chapters.
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
    let c = children(a, id);
    c.first().and_then(|&v| a.as_str(v))
}

// ---- render: chapter Arenas → @generated TSX string (byte-parity with chapterModel.ts renderChapter) ----

fn render_chapter(a: &Arenas, chapter: StructId) -> String {
    let mut title = "";
    let mut lede: Option<&[StructId]> = None;
    let mut blocks: Vec<(&'static str, &[StructId])> = Vec::new(); // (tag, inline-children)
    for &f in children(a, chapter) {
        match a.head_name(f) {
            Some("title") => title = attr_str(a, f).unwrap_or(""),
            Some("lede") => lede = Some(children(a, f)),
            // pillar/section/slug/blurb = registry metadata, NOT rendered into the .tsx.
            Some("slug") | Some("pillar") | Some("section") | Some("blurb") => {}
            Some("h2") => blocks.push(("H2", children(a, f))),
            Some("p") => blocks.push(("P", children(a, f))),
            Some("note") => blocks.push(("Note", children(a, f))),
            _ => {} // runnable/exercise/why: increment-3
        }
    }
    let slug = children(a, chapter)
        .iter()
        .find(|&&f| a.head_name(f) == Some("slug"))
        .and_then(|&f| attr_str(a, f))
        .unwrap_or("");

    // Import set — EXACTLY the heads used (tsc noUnusedLocals). H1 always; Lede/H2/P/Note per use; C if
    // inline code; Ch / AppLink if a chapter / app link is used. Sorted for determinism (BTreeSet).
    let mut prose: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    prose.insert("H1");
    if lede.is_some() {
        prose.insert("Lede");
    }
    for (tag, _) in &blocks {
        prose.insert(tag);
    }
    let mut uses_ch = false;
    let mut uses_app = false;
    if let Some(l) = lede {
        scan_inline(a, l, &mut prose, &mut uses_ch, &mut uses_app);
    }
    for (_, ch) in &blocks {
        scan_inline(a, ch, &mut prose, &mut uses_ch, &mut uses_app);
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
    out.push('\n');
    out.push_str(&format!("export default function {}() {{\n", pascal(slug)));
    out.push_str("  return (\n");
    out.push_str("    <article>\n");
    out.push_str(&format!("      <H1>{}</H1>\n", escape_text(title)));
    if let Some(l) = lede {
        out.push_str(&format!("      <Lede>{}</Lede>\n", render_inlines(a, l)));
    }
    for (tag, ch) in &blocks {
        out.push_str(&format!("      <{tag}>{}</{tag}>\n", render_inlines(a, ch)));
    }
    out.push_str("    </article>\n");
    out.push_str("  );\n");
    out.push_str("}\n");
    out
}

/// Learn the import needs of an inline sequence: `(c …)` → C; chapter link → Ch; app link → AppLink; recurse em/strong/link.
fn scan_inline(
    a: &Arenas,
    ins: &[StructId],
    prose: &mut std::collections::BTreeSet<&'static str>,
    uses_ch: &mut bool,
    uses_app: &mut bool,
) {
    for &i in ins {
        match a.head_name(i) {
            Some("c") => {
                prose.insert("C");
            }
            Some("link") => {
                *uses_ch = true;
                scan_inline(a, &children(a, i)[1..], prose, uses_ch, uses_app); // skip the (slug …) attr
            }
            Some("app-link") => {
                *uses_app = true;
                scan_inline(a, &children(a, i)[1..], prose, uses_ch, uses_app); // skip the (route …) attr
            }
            Some("em") | Some("strong") => scan_inline(a, children(a, i), prose, uses_ch, uses_app),
            _ => {}
        }
    }
}

fn render_inlines(a: &Arenas, ins: &[StructId]) -> String {
    ins.iter().map(|&i| render_inline(a, i)).collect()
}

fn render_inline(a: &Arenas, i: StructId) -> String {
    // A bare string atom = text.
    if let Some(t) = a.as_str(i)
        && matches!(a.get(i), Struct::Atom(_))
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
