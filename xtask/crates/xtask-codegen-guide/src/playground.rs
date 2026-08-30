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

/// Find the `(example …)` form in a per-example doc — the root, or (when `read_all` wrapped a single top-level
/// form in a synthetic `(do …)`) its `(example …)` child. Mirrors `locate_chapter`.
fn locate_example(a: &Arenas) -> Option<StructId> {
    if a.head_name(a.root) == Some("example") {
        return Some(a.root);
    }
    if let Struct::List(items) = a.get(a.root) {
        return items
            .iter()
            .copied()
            .find(|&c| a.head_name(c) == Some("example"));
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

/// Read + validate ONE `(example …)` form. Returns an error string (not a panic) on a malformed/invalid
/// example so the codegen can fail loudly with a pointed message.
pub fn read_one_example(a: &Arenas, ex: StructId) -> Result<PlaygroundExample, String> {
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
    Ok(PlaygroundExample {
        id,
        name,
        theme,
        surface,
        source,
        expected,
        expect_error,
    })
}

/// Read + validate every `(example …)` in a `(playground …)` doc, in source order. Kept for the single-doc
/// form + the shred (a `(playground …)` .cdzb); the file-per-example source-of-truth is read by
/// [`read_playground_dir`]. Error string on a malformed example.
pub fn read_playground(a: &Arenas) -> Result<Vec<PlaygroundExample>, String> {
    let root = locate_playground(a).ok_or("no (playground …) form in the document")?;
    let mut out = Vec::new();
    for &ex in super::children(a, root) {
        if a.head_name(ex) != Some("example") {
            continue; // tolerate non-example children (comments/metadata) — walk only (example …)
        }
        out.push(read_one_example(a, ex)?);
    }
    Ok(out)
}

/// Read + validate every per-example `.sexp` in a directory (the file-per-example source-of-truth, seq-279),
/// in FILENAME order (files are `<NNNN>-<id>.sexp`, index-prefixed → the dropdown order sorts by name). Each
/// file is a bare `(example …)` form. Error string on a malformed file/example.
pub fn read_playground_dir(dir: &std::path::Path) -> Result<Vec<PlaygroundExample>, String> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("read dir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sexp"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no *.sexp examples in {}", dir.display()));
    }
    let mut out = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
        let a = cadenza_syntax_sexpr::read_all(&text)
            .map_err(|e| format!("parse {}: {e:?}", f.display()))?;
        let ex =
            locate_example(&a).ok_or_else(|| format!("{}: no (example …) form", f.display()))?;
        out.push(read_one_example(&a, ex)?);
    }
    Ok(out)
}

/// Read playground examples from a decoded doc — either a legacy `(playground …)` multi-example doc OR a
/// single per-example `(example …)` file (seq-279; the shred's per-example `.cdzb`). Returns all found.
pub fn read_playground_any(a: &Arenas) -> Result<Vec<PlaygroundExample>, String> {
    if locate_playground(a).is_some() {
        return read_playground(a);
    }
    if let Some(ex) = locate_example(a) {
        return Ok(vec![read_one_example(a, ex)?]);
    }
    Err("no (playground …) or (example …) form in the document".into())
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

/// Extract the hand-authored `EXAMPLES[]` entries from examples.ts → `(id, bare-(example …)-sexp)` per
/// example (source-order) — the one-time fork1a migration input, IN RUST (operator: no JS tooling; the xtask
/// owns codegen). A lightweight LINE scan (the xtask has no regex crate) over the file's regular entry format:
/// one-per-line `key: "value",` fields, plus `source`, a backtick template that may span lines (closed by the
/// next UNESCAPED backtick — Cadenza sources mention backticks only inside `\``-escaped comments). Only scans
/// inside the `export const EXAMPLES … = [` array (skips the interface).
pub fn bootstrap_from_examples_ts(ts: &str) -> Result<Vec<(String, String)>, String> {
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
    entries
        .iter()
        .map(|e| Ok((e.id.clone(), format_example_form(e)?)))
        .collect()
}

/// Format one bootstrapped entry as a standalone bare `(example …)` sexp file body (example at column 0,
/// attrs 2-space). source embedded VERBATIM (canonicalized on read); expected an SEXPR value (seq-279).
fn format_example_form(e: &BootEntry) -> Result<String, String> {
    if e.surface != "sexpr" {
        return Err(format!(
            "example {:?}: surface {:?} — the bootstrap embeds sexpr sources verbatim only",
            e.id, e.surface
        ));
    }
    let mut form = format!(
        "(example\n  (id {})\n  (name {})\n  (theme {})\n  (surface {})\n  (source {})",
        super::json_string(&e.id),
        super::json_string(&e.name),
        super::json_string(&e.theme),
        super::json_string(&e.surface),
        e.source.trim(),
    );
    if let Some(exp) = &e.expected {
        form.push_str(&format!("\n  (expected {exp})"));
    }
    if e.expect_error {
        form.push_str("\n  (expect-error \"true\")");
    }
    form.push_str(")\n");
    Ok(form)
}

/// `--playground-bootstrap <examples.ts>`: one-time fork1a migration — emit ONE `.sexp` per example into the
/// sibling `examples/` directory (`<NNNN>-<id>.sexp`, index-prefixed for dropdown order), from the
/// hand-authored examples.ts. Deletes the old single examples.sexp. After this, the per-example files are the
/// source-of-truth + examples.ts is @generated (`--playground-registry`).
pub fn run_playground_bootstrap(examples_ts: &str) {
    let ts = std::fs::read_to_string(examples_ts)
        .unwrap_or_else(|e| die(&format!("read {examples_ts}: {e}")));
    let forms =
        bootstrap_from_examples_ts(&ts).unwrap_or_else(|e| die(&format!("{examples_ts}: {e}")));
    let base = std::path::Path::new(examples_ts);
    let dir = base.with_file_name("examples");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&format!("mkdir {}: {e}", dir.display())));
    for (i, (id, form)) in forms.iter().enumerate() {
        let fname = format!("{:04}-{}.sexp", i + 1, id);
        std::fs::write(dir.join(&fname), form)
            .unwrap_or_else(|e| die(&format!("write {fname}: {e}")));
    }
    let _ = std::fs::remove_file(base.with_file_name("examples.sexp"));
    println!(
        "✓ --playground-bootstrap: wrote {} per-example files → {}",
        forms.len(),
        dir.display()
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
/// examples.ts from the sibling `examples/` directory (the per-example `.sexp` source-of-truth — read all →
/// validate → emit → replace the region).
pub fn run_playground_registry(examples_ts: &str, check: bool) {
    let dir = std::path::Path::new(examples_ts).with_file_name("examples");
    let examples =
        read_playground_dir(&dir).unwrap_or_else(|e| die(&format!("{}: {e}", dir.display())));
    let body = emit_examples_array(&examples);
    let src = std::fs::read_to_string(examples_ts)
        .unwrap_or_else(|e| die(&format!("read {examples_ts}: {e}")));
    let next = regenerate_examples_region(&src, &body)
        .unwrap_or_else(|e| die(&format!("{examples_ts}: {e}")));
    if check {
        if next != src {
            eprintln!(
                "✗ --playground-registry --check: {examples_ts} EXAMPLES[] is OUT OF SYNC with examples/ — regenerate + commit."
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
        let forms = bootstrap_from_examples_ts(ts).unwrap();
        assert_eq!(forms.len(), 2); // interface's `id`/`surface` NOT scanned as entries
        assert_eq!(forms[0].0, "one");
        assert_eq!(forms[1].0, "two");
        // each emitted form is a standalone bare (example …) that reads back to the authored example
        let read = |form: &str| {
            let a = cadenza_syntax_sexpr::read_all(form).unwrap();
            let ex = locate_example(&a).expect("emitted form is not an (example …)");
            read_one_example(&a, ex).unwrap()
        };
        let one = read(&forms[0].1);
        assert_eq!(one.id, "one");
        assert_eq!(one.theme, "basics");
        // the escaped-quote expected survives extraction + re-emission (unescaped then sexpr-escaped)
        assert_eq!(one.expected.as_deref(), Some("(: \"hi\" String)"));
        assert!(!one.expect_error);
        let two = read(&forms[1].1);
        assert_eq!(two.id, "two");
        assert!(two.expect_error);
        assert_eq!(two.expected, None);
        // multi-line source captured, and the escaped `\`` inside the comment did NOT close the template early
        assert!(
            two.source.contains("(def (main) (+ 1 2))") && two.source.contains("(export main)")
        );
    }

    #[test]
    fn missing_required_field_errors() {
        let no_theme = "(playground (example (id \"x\") (name \"X\") (surface \"sexpr\") \
              (source (do (def (main) 1) (export main)))))";
        assert!(read(no_theme).unwrap_err().contains("missing (theme"));
    }
}
