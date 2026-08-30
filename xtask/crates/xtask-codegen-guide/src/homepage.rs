//! HomePage landing-page examples (fork1b): the `<Runnable>`(s) on the bespoke HomePage move out of the .tsx
//! into `guide/src/content/HomePage.sexp` — `(homepage (runnable (title …) (source <expr>)) …)` — and a
//! @generated `HomePageExamples.ts` constant is codegen'd for HomePage.tsx to import + render (its
//! hand-written layout stays). Operator seq-259/344/284: no code-in-.tsx; the runnable source is a .sexp AST.
//! The shred covers the runnable via `run_shred`'s `(homepage …)` branch (chapter-style, both surfaces).

use cadenza_ast::ast::{Arenas, StructId};

/// One HomePage runnable — its title + canonical-rendered source (a bare Cadenza expr like `(+ 2 3)`).
#[derive(Debug)]
pub struct HomeRunnable {
    pub title: String,
    pub source: String,
}

/// Render the `(source …)` holder's form children — `print_from` (a bare value/expr for HomePage). `None` if
/// the holder is absent.
fn source_text(a: &Arenas, runnable: StructId) -> Option<String> {
    let holder = super::named_node(a, runnable, "source")?;
    let kids = super::children(a, holder);
    if kids.is_empty() {
        return None;
    }
    Some(
        kids.iter()
            .map(|&k| cadenza_syntax_sexpr::print_from(a, k))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Read the `(runnable (title …) (source …))` forms of a `(homepage …)` doc, in source order. Error string on
/// a malformed doc so the codegen fails loudly.
pub fn read_homepage(a: &Arenas) -> Result<Vec<HomeRunnable>, String> {
    let root = super::locate_homepage(a).ok_or("no (homepage …) form in the document")?;
    let mut out = Vec::new();
    for &r in super::children(a, root) {
        if a.head_name(r) != Some("runnable") {
            continue;
        }
        let title = super::named_attr(a, r, "title")
            .ok_or("a (runnable …) is missing (title \"…\")")?
            .to_string();
        let source =
            source_text(a, r).ok_or_else(|| format!("runnable {title:?}: missing (source …)"))?;
        out.push(HomeRunnable { title, source });
    }
    if out.is_empty() {
        return Err("(homepage …) has no (runnable …)".into());
    }
    Ok(out)
}

/// Emit the @generated `HomePageExamples.ts` — a `HOMEPAGE_RUNNABLES` constant HomePage.tsx imports.
pub fn emit_homepage_examples_ts(runnables: &[HomeRunnable]) -> String {
    let mut s = String::from(
        "/// @generated from src/content/HomePage.sexp by `xtask-codegen-guide --homepage`. Do NOT hand-edit —\n\
         /// edit HomePage.sexp + regenerate. HomePage.tsx imports HOMEPAGE_RUNNABLES and renders them (its\n\
         /// hand-written layout stays). Each `source` is a Cadenza program (sexpr), the text a <Runnable>\n\
         /// `source` prop receives.\n\n\
         export const HOMEPAGE_RUNNABLES = [\n",
    );
    for r in runnables {
        // template literal — escape `\`, backtick, `${` (a no-op on real Cadenza source).
        let src = r
            .source
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        s.push_str(&format!(
            "  {{ title: {}, source: `{src}` }},\n",
            super::json_string(&r.title)
        ));
    }
    s.push_str("] as const;\n");
    s
}

fn die(msg: &str) -> ! {
    eprintln!("xtask-codegen-guide --homepage: {msg}");
    std::process::exit(1);
}

/// `--homepage [--check] <HomePage.sexp>`: regenerate (or `--check`) the sibling `HomePageExamples.ts` from
/// `HomePage.sexp`. The whole file is @generated (unlike examples.ts's array region — this is a fresh file).
pub fn run_homepage_registry(homepage_sexp: &str, check: bool) {
    let sexp = std::fs::read_to_string(homepage_sexp)
        .unwrap_or_else(|e| die(&format!("read {homepage_sexp}: {e}")));
    let a = cadenza_syntax_sexpr::read_all(&sexp)
        .unwrap_or_else(|e| die(&format!("parse {homepage_sexp}: {e:?}")));
    let runnables = read_homepage(&a).unwrap_or_else(|e| die(&format!("{homepage_sexp}: {e}")));
    let ts = emit_homepage_examples_ts(&runnables);
    let out = std::path::Path::new(homepage_sexp).with_file_name("HomePageExamples.ts");
    if check {
        let committed = std::fs::read_to_string(&out)
            .unwrap_or_else(|e| die(&format!("read {}: {e}", out.display())));
        if committed != ts {
            eprintln!(
                "✗ --homepage --check: {} is OUT OF SYNC with HomePage.sexp — regenerate + commit.",
                out.display()
            );
            std::process::exit(1);
        }
        println!(
            "✓ --homepage --check: HomePageExamples.ts ({} runnable) in sync",
            runnables.len()
        );
    } else {
        std::fs::write(&out, &ts).unwrap_or_else(|e| die(&format!("write {}: {e}", out.display())));
        println!(
            "✓ --homepage: regenerated {} runnable(s) → {}",
            runnables.len(),
            out.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_emits_homepage() {
        let doc = "(homepage (runnable (title \"Try it: edit and Run\") (source (+ 2 3))))";
        let a = cadenza_syntax_sexpr::read_all(doc).unwrap();
        let rs = read_homepage(&a).unwrap();
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].title, "Try it: edit and Run");
        assert_eq!(rs[0].source, "(+ 2 3)");
        let ts = emit_homepage_examples_ts(&rs);
        assert!(ts.contains("export const HOMEPAGE_RUNNABLES = ["));
        assert!(ts.contains("{ title: \"Try it: edit and Run\", source: `(+ 2 3)` },"));
        assert!(ts.trim_end().ends_with("] as const;"));
    }

    #[test]
    fn errors_on_missing_title_or_source() {
        let a = cadenza_syntax_sexpr::read_all("(homepage (runnable (source (+ 1 1))))").unwrap();
        assert!(read_homepage(&a).unwrap_err().contains("missing (title"));
        let b = cadenza_syntax_sexpr::read_all("(homepage (runnable (title \"x\")))").unwrap();
        assert!(read_homepage(&b).unwrap_err().contains("missing (source"));
    }
}
