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
//!       (source (do … (export main))) [(expected <value>)] [(expect-error "true")])
//!     …)
//!
//! A playground buffer is a WHOLE program compiled verbatim, so its sexpr source keeps its `(do …)` wrapper
//! (a bare multi-form sexpr file does not parse) — hence the source renders with `print_pretty_from` (canonical,
//! wrapper intact), NOT `print_pretty_program` (which strips the `(do)` for flush-left DISPLAY of a chapter
//! snippet). That is the key playground-vs-chapter rendering distinction.

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

/// The expected-result of an `(expected <value>)` form, rendered as text — a flat value form like
/// `(: #tuple(1 2) (Tuple Int64 Int64))` or a bare atom (`5`, `true`). Stored as an SEXPR value, NOT a code
/// string (operator seq-279 "no code in strings"); rendered via `print_from` (byte-stable round-trip, so the
/// emitted examples.ts `expected: "…"` string is unchanged from the hand-authored one). `None` when absent.
fn expected_value(a: &Arenas, example: StructId) -> Option<String> {
    let holder = super::named_node(a, example, "expected")?;
    let kids = super::children(a, holder);
    if kids.is_empty() {
        return None;
    }
    let parts: Vec<String> = kids
        .iter()
        .map(|&k| cadenza_syntax_sexpr::print_from(a, k))
        .collect();
    Some(parts.join(" "))
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
        let expected = expected_value(a, ex);
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

/// Emit the body of the `EXAMPLES: Example[] = [ … ];` array — the @GENERATED region of examples.ts — from
/// the read examples. Mirrors the chapters.ts `--registry` in-place array generation: only this array region
/// is generated; the `Example` interface + header stay hand-written. Each entry matches the authored format
/// (2-space item indent, 4-space fields; string fields JSON-quoted; `source` a backtick template literal
/// holding the canonical program verbatim; optional `expected`/`expectError` only when present). Per
/// operator seq-259 the `source` is the CANONICAL rendering (from `read_playground`), so a reformat vs the
/// old hand-authored text is expected + intended.
pub fn emit_examples_array(examples: &[PlaygroundExample]) -> String {
    let mut s = String::new();
    for ex in examples {
        s.push_str("  {\n");
        s.push_str(&format!("    id: {},\n", super::json_string(&ex.id)));
        s.push_str(&format!("    name: {},\n", super::json_string(&ex.name)));
        s.push_str(&format!("    theme: {},\n", super::json_string(&ex.theme)));
        s.push_str(&format!(
            "    surface: {},\n",
            super::json_string(&ex.surface)
        ));
        // A template literal — escape `\`, backtick, and `${` so a source containing them can't break out
        // (Cadenza sources don't today, so this is a no-op on real data → byte-matches the authored file).
        let src = ex
            .source
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        s.push_str(&format!("    source: `{src}`,\n"));
        if let Some(e) = &ex.expected {
            s.push_str(&format!("    expected: {},\n", super::json_string(e)));
        }
        if ex.expect_error {
            s.push_str("    expectError: true,\n");
        }
        s.push_str("  },\n");
    }
    s
}

// ---- fork1a one-time bootstrap: examples.ts → examples.sexp (Rust; operator directive: no JS tooling) ----

/// Parse a `key: "value",` TS field on a trimmed line, returning the UNESCAPED value (handling `\"`/`\\`).
/// `None` if the line doesn't lead with `key`. Reads to the first UNescaped `"` (so an `expected` value like
/// `"(: \"hi\" String)"` is captured whole).
fn ts_field_str(trimmed: &str, key: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(key)?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut escaped = false;
    for c in rest.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None // unterminated
}

/// The byte index of the first UNESCAPED backtick in `s` — the template-literal close. A `` \` `` (an escaped
/// backtick, used for a literal backtick inside the `source: `…`` template — e.g. a `` `List.prepend` `` mention
/// in a comment) is NOT the delimiter, so skip the escaped char. `None` if there is no closing backtick.
fn unescaped_backtick(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'`' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Unescape the template-literal escapes a source fragment can carry — only `` \` `` occurs in the guide's
/// examples (a literal backtick inside the template), so this is the sole unescape (matches what the node
/// bootstrap got for free by letting JS evaluate the template literal).
fn unescape_source(s: &str) -> String {
    s.replace("\\`", "`")
}

/// One entry accumulated while scanning examples.ts.
#[derive(Default)]
struct BootEntry {
    id: String,
    name: String,
    theme: String,
    surface: String,
    source: String,
    expected: Option<String>,
    expect_error: bool,
}

/// Extract the hand-authored `EXAMPLES[]` entries from examples.ts and emit the `(playground …)` doc — the
/// one-time fork1a migration input, IN RUST (operator: no JS tooling; the xtask owns codegen). A lightweight
/// LINE scan (the xtask has no regex crate) over the file's regular entry format: one-per-line `key: "value",`
/// fields, plus `source`, a backtick template that may span lines (closed by the next UNESCAPED backtick —
/// Cadenza sources mention backticks only inside `\``-escaped comments). Only scans inside the
/// `export const EXAMPLES … = [` array (skips the interface).
pub fn bootstrap_from_examples_ts(ts: &str) -> Result<String, String> {
    let mut entries: Vec<BootEntry> = Vec::new();
    let mut cur: Option<BootEntry> = None;
    let mut in_array = false;
    let mut in_source = false;
    let mut source = String::new();
    for line in ts.lines() {
        let t = line.trim();
        if !in_array {
            if t.starts_with("export const EXAMPLES") && t.contains('[') {
                in_array = true;
            }
            continue;
        }
        if in_source {
            // accumulate source lines until the UNESCAPED closing backtick (an escaped `\`` inside a comment
            // is part of the source, not the delimiter); unescape at the close.
            if let Some(bt) = unescaped_backtick(line) {
                source.push_str(&line[..bt]);
                if let Some(c) = cur.as_mut() {
                    c.source = unescape_source(&source);
                }
                source.clear();
                in_source = false;
            } else {
                source.push_str(line);
                source.push('\n');
            }
            continue;
        }
        match t {
            "{" => cur = Some(BootEntry::default()),
            "}," | "}" => {
                if let Some(e) = cur.take() {
                    entries.push(e);
                }
            }
            "];" => break,
            _ => {
                let Some(c) = cur.as_mut() else { continue };
                if let Some(v) = ts_field_str(t, "id:") {
                    c.id = v;
                } else if let Some(v) = ts_field_str(t, "name:") {
                    c.name = v;
                } else if let Some(v) = ts_field_str(t, "theme:") {
                    c.theme = v;
                } else if let Some(v) = ts_field_str(t, "surface:") {
                    c.surface = v;
                } else if let Some(v) = ts_field_str(t, "expected:") {
                    c.expected = Some(v);
                } else if t.starts_with("expectError:") && t.contains("true") {
                    c.expect_error = true;
                } else if let Some(after) = t.strip_prefix("source:") {
                    let after = after
                        .trim_start()
                        .strip_prefix('`')
                        .unwrap_or(after.trim_start());
                    if let Some(bt) = unescaped_backtick(after) {
                        c.source = unescape_source(&after[..bt]); // single-line source
                    } else {
                        source = format!("{after}\n");
                        in_source = true;
                    }
                }
            }
        }
    }
    if entries.is_empty() {
        return Err(
            "no EXAMPLES entries found in examples.ts (did the array/format change?)".into(),
        );
    }
    let mut doc = String::from("(playground\n");
    for e in &entries {
        if e.surface != "sexpr" {
            return Err(format!(
                "example {:?}: surface {:?} — the bootstrap embeds sexpr sources verbatim only",
                e.id, e.surface
            ));
        }
        let mut form = format!(
            "  (example\n    (id {})\n    (name {})\n    (theme {})\n    (surface {})\n    (source {})",
            super::json_string(&e.id),
            super::json_string(&e.name),
            super::json_string(&e.theme),
            super::json_string(&e.surface),
            e.source.trim(),
        );
        if let Some(exp) = &e.expected {
            // expected is an SEXPR VALUE, not a code-string (operator seq-279) — the value text is already
            // sexpr (a bare atom or an ascribed `(: …)` form), so splice it raw, not json_string-quoted.
            form.push_str(&format!("\n    (expected {exp})"));
        }
        if e.expect_error {
            form.push_str("\n    (expect-error \"true\")");
        }
        form.push(')');
        doc.push_str(&form);
        doc.push('\n');
    }
    doc.push_str(")\n");
    Ok(doc)
}

/// `--playground-bootstrap <examples.ts>`: emit the `(playground …)` doc (to the sibling examples.sexp) from
/// the hand-authored examples.ts — the one-time fork1a migration input. After the flip, examples.sexp is
/// authored directly + examples.ts is @generated (`--playground-registry`), so this is used once.
pub fn run_playground_bootstrap(examples_ts: &str) {
    let ts = std::fs::read_to_string(examples_ts)
        .unwrap_or_else(|e| die(&format!("read {examples_ts}: {e}")));
    let doc =
        bootstrap_from_examples_ts(&ts).unwrap_or_else(|e| die(&format!("{examples_ts}: {e}")));
    let out = std::path::Path::new(examples_ts).with_file_name("examples.sexp");
    std::fs::write(&out, &doc).unwrap_or_else(|e| die(&format!("write {}: {e}", out.display())));
    let n = doc.matches("  (example\n").count();
    println!(
        "✓ --playground-bootstrap: wrote {n} examples → {}",
        out.display()
    );
}

/// The generated-region markers inside the `EXAMPLES: Example[] = [ … ]` array of examples.ts (2-space
/// indented to match the array interior) — everything between them is regenerated; the array brackets, the
/// `Example` interface, and the header stay hand-written. Mirrors the chapters.ts `// <generated:chapters>`.
const BEGIN: &str = "  // <generated:examples>";
const END: &str = "  // </generated:examples>";

/// Replace the generated EXAMPLES region (between the markers) of examples.ts with `body`, returning the new
/// file text — or an error if the markers are absent. Pure (no I/O), mirrors the chapters.ts region swap.
pub fn regenerate_examples_region(ts_src: &str, body: &str) -> Result<String, String> {
    let begin_line = format!(
        "{BEGIN} — DO NOT EDIT; regenerated by `xtask-codegen-guide --playground-registry` (from examples.sexp)"
    );
    let block = format!("{begin_line}\n{body}{END}");
    match (ts_src.find(BEGIN), ts_src.find(END)) {
        (Some(bi), Some(ei)) if ei >= bi => Ok(format!(
            "{}{block}{}",
            &ts_src[..bi],
            &ts_src[ei + END.len()..]
        )),
        _ => Err(format!(
            "generated-region markers ({BEGIN} … {END}) not found in examples.ts"
        )),
    }
}

/// `--playground-registry [--check] <examples.ts>`: regenerate (or `--check`) the `EXAMPLES[]` region of
/// examples.ts from the sibling `examples.sexp` source-of-truth (read → validate → emit → replace the region).
pub fn run_playground_registry(examples_ts: &str, check: bool) {
    let sexp_path = std::path::Path::new(examples_ts).with_file_name("examples.sexp");
    let sexp = std::fs::read_to_string(&sexp_path)
        .unwrap_or_else(|e| die(&format!("read {}: {e}", sexp_path.display())));
    let a = cadenza_syntax_sexpr::read_all(&sexp)
        .unwrap_or_else(|e| die(&format!("parse {}: {e:?}", sexp_path.display())));
    let examples =
        read_playground(&a).unwrap_or_else(|e| die(&format!("{}: {e}", sexp_path.display())));
    let body = emit_examples_array(&examples);
    let src = std::fs::read_to_string(examples_ts)
        .unwrap_or_else(|e| die(&format!("read {examples_ts}: {e}")));
    let next = regenerate_examples_region(&src, &body)
        .unwrap_or_else(|e| die(&format!("{examples_ts}: {e}")));
    if check {
        if next != src {
            eprintln!(
                "✗ --playground-registry --check: {examples_ts} EXAMPLES[] is OUT OF SYNC with examples.sexp — regenerate + commit."
            );
            std::process::exit(1);
        }
        println!(
            "✓ --playground-registry --check: examples.ts EXAMPLES[] ({}) in sync",
            examples.len()
        );
    } else {
        std::fs::write(examples_ts, &next)
            .unwrap_or_else(|e| die(&format!("write {examples_ts}: {e}")));
        println!(
            "✓ --playground-registry: regenerated {} examples in {examples_ts}",
            examples.len()
        );
    }
}

fn die(msg: &str) -> ! {
    eprintln!("xtask-codegen-guide --playground-registry: {msg}");
    std::process::exit(1);
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
              (source (do (def (main) (+ 2 3)) (export main))) (expected 5)) \
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
              (source (do (def (main) 1) (export main))) (expected 1)))";
        assert!(
            read(ml_pin)
                .unwrap_err()
                .contains("requires (surface \"sexpr\")")
        );
    }

    #[test]
    fn emits_examples_array_in_authored_format() {
        let doc = "(playground \
            (example (id \"hello\") (name \"Hello\") (theme \"basics\") (surface \"sexpr\") \
              (source (do (def (main) (+ 2 3)) (export main))) (expected 5)) \
            (example (id \"neg\") (name \"Bad\") (theme \"numbers\") (surface \"sexpr\") \
              (source (do (def (main) (+ 1 \"x\")) (export main))) (expect-error \"true\")))";
        let exs = read(doc).unwrap();
        let ts = emit_examples_array(&exs);
        // graded entry: JSON-quoted fields, backtick source, expected present, no expectError
        assert!(ts.contains("  {\n    id: \"hello\",\n    name: \"Hello\",\n"));
        assert!(ts.contains("    theme: \"basics\",\n    surface: \"sexpr\",\n"));
        // source is a backtick template literal holding the CANONICAL program (print_pretty_from breaks a
        // `(do …)` block into 2-space-indented, blank-separated members — the formatter's canonical style).
        assert!(ts.contains("    source: `(do\n  (def (main) (+ 2 3))\n\n  (export main))`,\n"));
        assert!(ts.contains("    expected: \"5\",\n"));
        // negative entry: expectError present, expected absent
        assert!(ts.contains("    id: \"neg\",\n"));
        assert!(ts.contains("    expectError: true,\n"));
        // exactly two items closed
        assert_eq!(ts.matches("  },\n").count(), 2);
        // the graded entry carries no expectError line
        let hello = &ts[ts.find("id: \"hello\"").unwrap()..ts.find("id: \"neg\"").unwrap()];
        assert!(!hello.contains("expectError"));
    }

    #[test]
    fn regenerates_only_the_marked_region() {
        // a stand-in examples.ts: hand-written header/interface + array brackets, generated region between
        // the markers. Only the region is replaced; the surrounding file is byte-preserved.
        let ts = "export interface Example { id: string }\n\
                  export const EXAMPLES: Example[] = [\n\
                  \x20 // <generated:examples> — DO NOT EDIT; regenerated by `xtask-codegen-guide --playground-registry` (from examples.sexp)\n\
                  \x20 { id: \"old\" },\n\
                  \x20 // </generated:examples>\n\
                  ];\n";
        let body = "  {\n    id: \"new\",\n  },\n";
        let next = regenerate_examples_region(ts, body).unwrap();
        assert!(next.starts_with(
            "export interface Example { id: string }\nexport const EXAMPLES: Example[] = [\n"
        ));
        assert!(next.contains("id: \"new\""));
        assert!(!next.contains("id: \"old\""));
        assert!(next.trim_end().ends_with("];")); // brackets + trailing preserved
        // idempotent: re-running with the same body is a no-op
        assert_eq!(regenerate_examples_region(&next, body).unwrap(), next);
        // missing markers → error, not a silent whole-file clobber
        assert!(regenerate_examples_region("no markers here", body).is_err());
    }

    #[test]
    fn bootstrap_extracts_entries_from_examples_ts() {
        // a stand-in examples.ts: an interface (must be skipped) + the EXAMPLES array with single- and
        // multi-line sources, an escaped-quote expected, an escaped-backtick comment, and an expectError entry.
        let ts = "import type { Surface } from \"../compiler/client.ts\";\n\
                  export interface Example { id: string; surface: Surface }\n\
                  export const EXAMPLES: Example[] = [\n\
                  \x20 {\n\
                  \x20   id: \"one\",\n\
                  \x20   name: \"One\",\n\
                  \x20   theme: \"basics\",\n\
                  \x20   surface: \"sexpr\",\n\
                  \x20   source: `(do (def (main) 1) (export main))`,\n\
                  \x20   expected: \"(: \\\"hi\\\" String)\",\n\
                  \x20 },\n\
                  \x20 {\n\
                  \x20   id: \"two\",\n\
                  \x20   name: \"Two\",\n\
                  \x20   theme: \"numbers\",\n\
                  \x20   surface: \"sexpr\",\n\
                  \x20   source: `(do\n\
                  \x20 ; mentions \\`List.prepend\\` in a comment (escaped backtick, not the delimiter)\n\
                  \x20 (def (main) (+ 1 2))\n\
                  \x20 (export main))`,\n\
                  \x20   expectError: true,\n\
                  \x20 },\n\
                  ];\n";
        let doc = bootstrap_from_examples_ts(ts).unwrap();
        // the emitted doc round-trips through the reader → the entries we authored
        let a = cadenza_syntax_sexpr::read_all(&doc).unwrap();
        let exs = read_playground(&a).unwrap();
        assert_eq!(exs.len(), 2);
        assert_eq!(exs[0].id, "one");
        assert_eq!(exs[0].theme, "basics");
        // the escaped-quote expected survives extraction + re-emission (unescaped then sexpr-escaped)
        assert_eq!(exs[0].expected.as_deref(), Some("(: \"hi\" String)"));
        assert!(!exs[0].expect_error);
        assert_eq!(exs[1].id, "two");
        assert!(exs[1].expect_error);
        assert_eq!(exs[1].expected, None);
        // multi-line source captured, and the escaped `\`` inside the comment did NOT close the template early
        assert!(
            exs[1].source.contains("(def (main) (+ 1 2))")
                && exs[1].source.contains("(export main)")
        );
        // the interface's `id: string` / `surface: Surface` were NOT scanned as entries
        assert!(exs.iter().all(|e| e.id == "one" || e.id == "two"));
    }

    #[test]
    fn missing_required_field_errors() {
        let no_theme = "(playground (example (id \"x\") (name \"X\") (surface \"sexpr\") \
              (source (do (def (main) 1) (export main)))))";
        assert!(read(no_theme).unwrap_err().contains("missing (theme"));
    }
}
