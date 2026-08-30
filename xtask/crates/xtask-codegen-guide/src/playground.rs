//! Reader for a `(playground …)` guide document — the fork1a source-of-truth for the playground's Examples
//! dropdown (today the hand-authored `src/playground/examples.ts`). Operator seq-259: playground examples go
//! `.sexp` source-of-truth → `@generated examples.ts`, everything CANONICAL (embed the program as AST, render
//! via the canonical formatter — no hand formatting). This module is the reusable CORE both the codegen (emit
//! `examples.ts`) and the shred (per-example compile+run matrix) build on: it walks `(example …)` forms,
//! extracts the fields, and VALIDATES the closed enums (theme/surface) so a typo fails LOUDLY at codegen
//! (the playground analogue of chapter slug validation — v-guide-editor's explicit ask).
//!
//! Doc shape (round-trips losslessly through the main sexpr reader — de-risked 2026-08-30):
//!   (playground
//!     (example (id "..") (name "..") (theme "..") (surface "sexpr")
//!       (source (do … (export main))) [(expected "..")] [(expect-error "true")])
//!     …)
//!
//! A playground buffer is a WHOLE program compiled verbatim, so its sexpr source keeps its `(do …)` wrapper
//! (a bare multi-form sexpr file does not parse) — hence the source renders with `print_pretty_from` (canonical,
//! wrapper intact), NOT `print_pretty_program` (which strips the `(do)` for flush-left DISPLAY of a chapter
//! snippet). That is the key playground-vs-chapter rendering distinction.

// The fork1a foundation: consumed by the codegen (emit examples.ts) + shred (per-example matrix) increments
// next; until a CLI mode calls `read_playground`, its public API is exercised only by the unit tests below.
#![allow(dead_code)]

use cadenza_ast::ast::{Arenas, Struct, StructId};

/// The closed `theme` set the playground sidebar groups by (matches the `Example` union in examples.ts +
/// the `src/playground/examples.test.ts` structural lint). A theme outside this set is a codegen error.
const THEMES: &[&str] = &["basics", "algorithms", "data-and-collections", "numbers"];
/// The compiler's declared surfaces.
const SURFACES: &[&str] = &["ml", "sexpr"];

/// One playground example, extracted + validated from an `(example …)` form.
#[derive(Debug, PartialEq, Eq)]
pub struct PlaygroundExample {
    pub id: String,
    pub name: String,
    pub theme: String,
    pub surface: String,
    /// The program source, canonical-rendered (wrapper intact) — the exact text the dropdown loads + the
    /// reader edits.
    pub source: String,
    pub expected: Option<String>,
    pub expect_error: bool,
}

/// Find the `(playground …)` root form (mirrors `locate_chapter`).
fn locate_playground(a: &Arenas) -> Option<StructId> {
    if a.head_name(a.root) == Some("playground") {
        return Some(a.root);
    }
    if let Struct::List(items) = a.get(a.root) {
        return items
            .iter()
            .copied()
            .find(|&c| a.head_name(c) == Some("playground"));
    }
    None
}

/// Canonical-render the `(source …)` holder's program forms — each child pretty-printed (wrapper kept) and
/// blank-line joined. `None` when the holder is absent.
fn canonical_source(a: &Arenas, example: StructId) -> Option<String> {
    let holder = super::named_node(a, example, "source")?;
    let kids = super::children(a, holder);
    if kids.is_empty() {
        return None;
    }
    let parts: Vec<String> = kids
        .iter()
        .map(|&k| cadenza_syntax_sexpr::print_pretty_from(a, k, cadenza_syntax_core::DEFAULT_WIDTH))
        .collect();
    Some(parts.join("\n\n"))
}

/// Read + validate every `(example …)` in a `(playground …)` doc, in source order. Returns an error string
/// (not a panic) on a malformed/invalid example so the codegen can fail loudly with a pointed message.
pub fn read_playground(a: &Arenas) -> Result<Vec<PlaygroundExample>, String> {
    let root = locate_playground(a).ok_or("no (playground …) form in the document")?;
    let mut out = Vec::new();
    for &ex in super::children(a, root) {
        if a.head_name(ex) != Some("example") {
            continue; // tolerate non-example children (comments/metadata) — walk only (example …)
        }
        let id = super::named_attr(a, ex, "id")
            .ok_or("an (example …) is missing (id \"…\")")?
            .to_string();
        let name = super::named_attr(a, ex, "name")
            .ok_or_else(|| format!("example {id}: missing (name \"…\")"))?
            .to_string();
        let theme = super::named_attr(a, ex, "theme")
            .ok_or_else(|| format!("example {id}: missing (theme \"…\")"))?
            .to_string();
        if !THEMES.contains(&theme.as_str()) {
            return Err(format!(
                "example {id}: unknown theme {theme:?} — allowed: {THEMES:?} (extend the Example union + this set together)"
            ));
        }
        let surface = super::named_attr(a, ex, "surface")
            .ok_or_else(|| format!("example {id}: missing (surface \"…\")"))?
            .to_string();
        if !SURFACES.contains(&surface.as_str()) {
            return Err(format!(
                "example {id}: unknown surface {surface:?} — allowed: {SURFACES:?}"
            ));
        }
        let source =
            canonical_source(a, ex).ok_or_else(|| format!("example {id}: missing (source …)"))?;
        let expected = super::named_attr(a, ex, "expected").map(str::to_string);
        let expect_error = super::named_attr(a, ex, "expect-error") == Some("true");
        // A pinned `expected` value must be sexpr-authored (compared on the sexpr pass) — mirrors the
        // examples.test.ts lint. Playground examples are sexpr, so this is a codegen guard against drift.
        if expected.is_some() && surface != "sexpr" {
            return Err(format!(
                "example {id}: an (expected …) pin requires (surface \"sexpr\") — it is compared on the sexpr pass"
            ));
        }
        out.push(PlaygroundExample {
            id,
            name,
            theme,
            surface,
            source,
            expected,
            expect_error,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Vec<PlaygroundExample>, String> {
        let a = cadenza_syntax_sexpr::read_all(text).unwrap();
        read_playground(&a)
    }

    #[test]
    fn reads_and_validates_examples() {
        let doc = "(playground \
            (example (id \"hello\") (name \"Hello\") (theme \"basics\") (surface \"sexpr\") \
              (source (do (def (main) (+ 2 3)) (export main))) (expected \"5\")) \
            (example (id \"neg\") (name \"See the squiggle\") (theme \"numbers\") (surface \"sexpr\") \
              (source (do (def (main) (+ 1 \"x\")) (export main))) (expect-error \"true\")))";
        let exs = read(doc).unwrap();
        assert_eq!(exs.len(), 2);
        assert_eq!(exs[0].id, "hello");
        assert_eq!(exs[0].name, "Hello");
        assert_eq!(exs[0].theme, "basics");
        assert_eq!(exs[0].surface, "sexpr");
        assert_eq!(exs[0].expected.as_deref(), Some("5"));
        assert!(!exs[0].expect_error);
        // canonical source keeps the (do) wrapper (playground buffer is a whole program)
        assert!(exs[0].source.starts_with("(do"));
        assert!(exs[0].source.contains("(def (main) (+ 2 3))"));
        assert!(exs[0].source.contains("(export main)"));
        assert_eq!(exs[1].id, "neg");
        assert!(exs[1].expect_error);
        assert_eq!(exs[1].expected, None);
    }

    #[test]
    fn rejects_unknown_theme() {
        let doc = "(playground (example (id \"x\") (name \"X\") (theme \"algorithm\") (surface \"sexpr\") \
              (source (do (def (main) 1) (export main)))))";
        let err = read(doc).unwrap_err();
        assert!(
            err.contains("unknown theme") && err.contains("algorithm"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_unknown_surface_and_ml_pin() {
        let bad_surface = "(playground (example (id \"x\") (name \"X\") (theme \"basics\") (surface \"sexp\") \
              (source (do (def (main) 1) (export main)))))";
        assert!(read(bad_surface).unwrap_err().contains("unknown surface"));
        // an (expected …) pin on a non-sexpr example is rejected
        let ml_pin = "(playground (example (id \"x\") (name \"X\") (theme \"basics\") (surface \"ml\") \
              (source (do (def (main) 1) (export main))) (expected \"1\")))";
        assert!(
            read(ml_pin)
                .unwrap_err()
                .contains("requires (surface \"sexpr\")")
        );
    }

    #[test]
    fn missing_required_field_errors() {
        let no_theme = "(playground (example (id \"x\") (name \"X\") (surface \"sexpr\") \
              (source (do (def (main) 1) (export main)))))";
        assert!(read(no_theme).unwrap_err().contains("missing (theme"));
    }
}
