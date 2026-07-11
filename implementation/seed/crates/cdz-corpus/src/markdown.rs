//! Corpus ↔ markdown migration: turn a `spec/semantics/*.sexp` file into a literate markdown
//! document, and verify the migration is behaviour-preserving.
//!
//! The corpus is fundamentally *literate*: 918 of its cases carry prose `(doc …)`, and the files are
//! threaded with `;`-comment narrative. Markdown is its natural home — prose becomes prose, and each
//! program payload becomes a fenced `cdz` code block an editor can highlight and a tool can extract.
//!
//! ## The format
//!
//! One case is a `###` heading (its description), optional prose (its `doc`), then one **tagged code
//! fence per DSL clause**. The fence's role is the LAST token of its info string; a leading `cdz`
//! marks the ML-bearing blocks (for highlighting) and is ignored by dispatch:
//!
//! | clause                     | fence tag            | body                         |
//! |----------------------------|----------------------|------------------------------|
//! | `(input …)`                | `` ```cdz input ``   | the program, as ML           |
//! | `(output (: v T))`         | `` ```cdz output ``  | `v : T` (ML ascription)      |
//! | `(error CODE)`             | `` ```error ``       | `CODE`                       |
//! | `(trap "…")`               | `` ```trap ``        | the reason (an ML string)    |
//! | `(needs cap)`              | `` ```needs ``       | one capability per line      |
//! | `(compiler (error CODE))`  | `` ```compiler-error `` | `CODE`                    |
//! | `(call export a…)`         | `` ```cdz call ``    | export then args, one/line   |
//! | `(host-calls c…)`          | `` ```cdz host-calls `` | one call per line, ML     |
//! | `(host-responses r…)`      | `` ```cdz host-responses `` | one respond/line, ML  |
//!
//! Every fence body is the ML rendering of the clause's tail; reconstruction parses each body with
//! `read_ml` and prepends the head named by the tag. One render path, one parse path — so the whole
//! migration rests on the ML surface round-tripping (which the corpus round-trip gate proves).
//!
//! ## Losslessness
//!
//! `check` proves the migration preserves everything the behaviour gate sees: the record stream
//! (`crate::render(crate::read(…))`) of the reconstructed corpus is byte-identical to the original's.
//! Case `doc` prose and inter-case `;` narrative are NOT part of that stream; they are carried into
//! markdown for faithfulness (doc whitespace is normalized to clean prose).

use cadenza_syntax::ast::{Arenas, Builder, Leaf, Struct, StructId};
use cadenza_syntax::{parser, printer, sexpr};

/// Target width for ML rendered inside a fence.
const ML_WIDTH: usize = 90;

// ============================================================================
// Public API
// ============================================================================

/// Migrate a corpus `.sexp` file's text to markdown.
pub fn migrate(sexpr_text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut first = true;
    for seg in segment(sexpr_text) {
        match seg {
            Segment::Prose(text) => {
                let prose = prose_from_comments(&text);
                if !prose.is_empty() {
                    if !first {
                        out.push('\n');
                    }
                    out.push_str(&prose);
                    out.push('\n');
                    first = false;
                }
            }
            Segment::Form(form) => {
                let a = sexpr::read(form).map_err(|e| format!("case parse error: {}", e.0))?;
                if a.head_name(a.root) != Some("case") {
                    // A non-case top-level form (unusual). Preserve it verbatim in a `cdz` fence so
                    // nothing is dropped.
                    if !first {
                        out.push('\n');
                    }
                    out.push_str("```cdz\n");
                    out.push_str(&ml_of(&a, a.root));
                    out.push_str("\n```\n");
                    first = false;
                    continue;
                }
                if !first {
                    out.push('\n');
                }
                render_case(&a, &mut out)?;
                first = false;
            }
        }
    }
    Ok(out)
}

/// Verify a migration is behaviour-preserving: the reconstructed corpus produces a record stream
/// byte-identical to the original's. Returns the offending diff on mismatch.
pub fn check(sexpr_text: &str) -> Result<(), String> {
    let md = migrate(sexpr_text)?;
    let reconstructed = to_sexpr(&md)?;

    let original_records = crate::render(&crate::read(sexpr_text)?);
    let round_tripped = crate::render(&crate::read(&reconstructed)?);

    if original_records == round_tripped {
        Ok(())
    } else {
        Err(first_record_diff(&original_records, &round_tripped))
    }
}

/// Reconstruct corpus `.sexp` text from a migrated markdown document — the inverse of [`migrate`] at
/// the level the record stream cares about (description + clauses; prose/`doc` are omitted, as they
/// are not part of the record stream). Public for round-trip testing.
pub fn to_sexpr(md_text: &str) -> Result<String, String> {
    let mut out = String::new();
    for case in parse_md(md_text)? {
        out.push_str(&reconstruct_case(&case)?);
        out.push('\n');
    }
    Ok(out)
}

// ============================================================================
// Rendering: sexpr case -> markdown
// ============================================================================

/// Render one `(case …)` arena to markdown, appending to `out`.
fn render_case(a: &Arenas, out: &mut String) -> Result<(), String> {
    let items = match a.get(a.root) {
        Struct::List(items) => items,
        _ => return Err("case is not a list".into()),
    };
    let description = items
        .get(1)
        .and_then(|&id| str_leaf(a, id))
        .ok_or("case has no description string")?;
    out.push_str("### ");
    out.push_str(&description);
    out.push('\n');

    for &clause in &items[2..] {
        match a.head_name(clause) {
            Some("doc") => {
                if let Some(text) = a
                    .as_form(clause, "doc")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(a, id))
                {
                    out.push('\n');
                    out.push_str(&normalize_prose(&text));
                    out.push('\n');
                }
            }
            Some(head) => {
                let (tag, body) = fence_for(a, clause, head)?;
                out.push_str("\n```");
                out.push_str(&tag);
                out.push('\n');
                out.push_str(&body);
                out.push_str("\n```\n");
            }
            None => return Err("case clause has no head".into()),
        }
    }
    Ok(())
}

/// The `(tag, body)` for a clause: the fence's info string and its ML body.
fn fence_for(a: &Arenas, clause: StructId, head: &str) -> Result<(String, String), String> {
    let tail = clause_tail(a, clause);
    let (tag, single_form): (&str, bool) = match head {
        "input" => ("cdz input", true),
        "output" => ("cdz output", true),
        "error" => ("error", true),
        "trap" => ("trap", true),
        "needs" => ("needs", false),
        "call" => ("cdz call", false),
        "host-calls" => ("cdz host-calls", false),
        "host-responses" => ("cdz host-responses", false),
        "compiler" => {
            // `(compiler (error CODE))` -> tag `compiler-error`, body = CODE.
            let code = tail
                .first()
                .filter(|&&inner| a.head_name(inner) == Some("error"))
                .and_then(|&inner| a.as_form(inner, "error").and_then(|t| t.first().copied()))
                .ok_or("malformed (compiler (error …))")?;
            return Ok(("compiler-error".to_string(), ml_of(a, code)));
        }
        other => {
            // An unrecognized clause: keep it verbatim as a generic `cdz` fence tagged by its head,
            // rendering the whole clause form so nothing is lost.
            return Ok((format!("cdz {other}"), ml_of(a, clause)));
        }
    };
    let body = if single_form {
        // Exactly one tail child, rendered as one (possibly multi-line) ML form.
        match tail.first() {
            Some(&child) => ml_of(a, child),
            None => String::new(),
        }
    } else {
        // One ML form per line (small, single-line forms only).
        tail.iter()
            .map(|&child| ml_of(a, child))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok((tag.to_string(), body))
}

/// The tail (children after the head) of a `List` clause.
fn clause_tail(a: &Arenas, clause: StructId) -> Vec<StructId> {
    match a.get(clause) {
        Struct::List(items) => items[1..].to_vec(),
        _ => Vec::new(),
    }
}

/// Render the subtree at `id` as ML text: clone it into a fresh arena rooted there, then print.
fn ml_of(a: &Arenas, id: StructId) -> String {
    let mut b = Builder::new();
    let root = clone_into(a, id, &mut b);
    let arena = b.finish(root);
    printer::print(&arena, ML_WIDTH)
}

/// Deep-clone occurrence `id` from `a` into builder `b`.
fn clone_into(a: &Arenas, id: StructId, b: &mut Builder) -> StructId {
    match a.get(id) {
        Struct::Atom(l) => {
            let leaf = a.leaf(*l).clone();
            b.atom_leaf(leaf)
        }
        Struct::List(items) => {
            let children: Vec<StructId> = items.iter().map(|&c| clone_into(a, c, b)).collect();
            b.list(children)
        }
    }
}

// ============================================================================
// Parsing: markdown -> case model
// ============================================================================

/// A case parsed back from markdown, carrying only what the record stream needs: the description and
/// its clause fences (in order). Prose/`doc` are dropped — they are not part of the record stream.
struct MdCase {
    description: String,
    fences: Vec<MdFence>,
}

struct MdFence {
    /// The clause ROLE — the last token of the fence info string (`input`, `output`, …).
    role: String,
    /// The fence body text (may be multiple lines).
    body: String,
}

/// Parse a migrated markdown document into its cases.
fn parse_md(md: &str) -> Result<Vec<MdCase>, String> {
    let mut cases: Vec<MdCase> = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(desc) = line.strip_prefix("### ") {
            let mut fences = Vec::new();
            // Consume lines until the next heading, collecting fences.
            while let Some(&peek) = lines.peek() {
                if peek.starts_with("### ") {
                    break;
                }
                let line = lines.next().unwrap();
                if let Some(info) = fence_open(line) {
                    // A fence: gather its body until the closing ```.
                    let role = info.split_whitespace().last().unwrap_or("").to_string();
                    let mut body_lines = Vec::new();
                    let mut closed = false;
                    for inner in lines.by_ref() {
                        if inner.trim_end() == "```" {
                            closed = true;
                            break;
                        }
                        body_lines.push(inner);
                    }
                    if !closed {
                        return Err(format!("unterminated fence ```{info} in case {desc:?}"));
                    }
                    fences.push(MdFence {
                        role,
                        body: body_lines.join("\n"),
                    });
                }
                // Non-fence lines between the heading and fences are prose (doc) — ignored here.
            }
            cases.push(MdCase {
                description: desc.to_string(),
                fences,
            });
        }
        // Lines before the first heading are file-level prose — ignored for reconstruction.
    }
    Ok(cases)
}

/// If `line` opens a code fence (three backticks + optional info string), return the info string.
fn fence_open(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("```")?;
    // A closing fence is bare ```; an opening fence carries an info string.
    if rest.trim().is_empty() {
        None
    } else {
        Some(rest.trim())
    }
}

// ============================================================================
// Reconstruction: case model -> sexpr text
// ============================================================================

/// Rebuild one case's `(case "desc" <clause>…)` s-expression text from its markdown model.
fn reconstruct_case(case: &MdCase) -> Result<String, String> {
    let mut out = String::from("(case ");
    out.push_str(&sexpr_string(&case.description));
    for fence in &case.fences {
        out.push(' ');
        out.push_str(&reconstruct_clause(fence)?);
    }
    out.push(')');
    Ok(out)
}

/// Rebuild one clause's s-expression from a fence.
fn reconstruct_clause(fence: &MdFence) -> Result<String, String> {
    let body = fence.body.trim();
    match fence.role.as_str() {
        "input" | "output" | "error" | "trap" => {
            Ok(format!("({} {})", fence.role, sexpr_of_ml(body)?))
        }
        "compiler-error" => Ok(format!("(compiler (error {}))", sexpr_of_ml(body)?)),
        "needs" => {
            // One capability per non-empty line -> one (needs cap) clause each.
            let clauses: Result<Vec<String>, String> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| Ok(format!("(needs {})", sexpr_of_ml(l)?)))
                .collect();
            Ok(clauses?.join(" "))
        }
        "call" | "host-calls" | "host-responses" => {
            // One ML form per non-empty line -> the clause's children.
            let children: Result<Vec<String>, String> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(sexpr_of_ml)
                .collect();
            Ok(format!("({} {})", fence.role, children?.join(" ")))
        }
        other => {
            // A verbatim `cdz <head>` fence: the body is the whole clause form already.
            let _ = other;
            sexpr_of_ml(body)
        }
    }
}

/// Parse ML text to an arena and render it as a single-line s-expression.
fn sexpr_of_ml(ml: &str) -> Result<String, String> {
    let parsed = parser::read_ml(ml);
    if !parsed.ok() {
        return Err(format!(
            "ML parse error in fence body {ml:?}: {:?}",
            parsed.errors
        ));
    }
    Ok(sexpr::print(&parsed.arenas))
}

// ============================================================================
// Text helpers
// ============================================================================

/// The string a `Str` leaf carries, if `id` is one.
fn str_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Normalize a `doc` string to clean prose: collapse each run of whitespace (including the source
/// literal's newlines + indentation) to a single space. Prose is not part of the record stream, so
/// this reflow is faithful enough and keeps the migration idempotent.
fn normalize_prose(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract prose from a run of `;` comment lines (an inter-case gap). Consecutive comment lines join
/// into one paragraph; a blank line separates paragraphs. Non-comment content is ignored.
fn prose_from_comments(gap: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in gap.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(';') {
            current.push(
                rest.strip_prefix(' ')
                    .unwrap_or(rest)
                    .trim_end()
                    .to_string(),
            );
        } else if t.is_empty() && !current.is_empty() {
            paragraphs.push(current.join(" "));
            current.clear();
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join(" "));
    }
    paragraphs
        .into_iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render a Rust string as an s-expression string literal (quotes + escapes).
fn sexpr_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// ============================================================================
// Segmentation: split a corpus file into top-level forms and the gaps between them
// ============================================================================

/// A slice of a corpus file: a top-level `(…)` form, or the gap text (comments/blanks) around it.
enum Segment<'a> {
    Prose(String),
    Form(&'a str),
}

/// Split `src` into an alternating sequence of gaps (`Prose`) and top-level forms (`Form`), scanning
/// with paren-depth tracking that respects `"strings"` and `; comments`.
fn segment(src: &str) -> Vec<Segment<'_>> {
    let bytes = src.as_bytes();
    let mut segs = Vec::new();
    let mut i = 0;
    let mut gap_start = 0;
    while i < bytes.len() {
        match bytes[i] {
            b';' => {
                // line comment — part of the gap
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => {
                // A top-level form begins. Emit the preceding gap.
                if gap_start < i {
                    segs.push(Segment::Prose(src[gap_start..i].to_string()));
                }
                let start = i;
                i = skip_form(bytes, i);
                segs.push(Segment::Form(&src[start..i]));
                gap_start = i;
            }
            _ => i += 1,
        }
    }
    if gap_start < bytes.len() {
        segs.push(Segment::Prose(src[gap_start..].to_string()));
    }
    segs
}

/// Given `bytes[start] == b'('`, return the index just past the matching `)`, respecting nested
/// parens, `"strings"` (with `\` escapes), and `; comments`.
fn skip_form(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    i // unterminated — return end (the sexpr reader will report the error)
}

/// Build a human-readable one-line diff of the first differing record between two record streams.
fn first_record_diff(a: &str, b: &str) -> String {
    let a_recs: Vec<&str> = a.split("---\n").collect();
    let b_recs: Vec<&str> = b.split("---\n").collect();
    for (i, (ra, rb)) in a_recs.iter().zip(&b_recs).enumerate() {
        if ra != rb {
            return format!(
                "record {i} differs:\n  original:      {}\n  reconstructed: {}",
                ra.replace('\n', " | "),
                rb.replace('\n', " | ")
            );
        }
    }
    if a_recs.len() != b_recs.len() {
        return format!(
            "record count differs: original {} vs reconstructed {}",
            a_recs.len(),
            b_recs.len()
        );
    }
    "record streams differ (no single record isolated)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A migrated file must round-trip: its reconstructed record stream matches the original's.
    fn assert_preserves(sexpr: &str) {
        check(sexpr).unwrap_or_else(|e| panic!("migration changed the record stream:\n{e}"));
    }

    #[test]
    fn simple_output_case() {
        let sexpr = r#"(case "integer addition" (input (+ 2 3)) (output (: 5 Int64)))"#;
        let md = migrate(sexpr).unwrap();
        assert!(md.contains("### integer addition"), "md:\n{md}");
        assert!(md.contains("```cdz input"), "md:\n{md}");
        assert!(md.contains("2 + 3"), "md:\n{md}");
        assert!(md.contains("```cdz output"), "md:\n{md}");
        assert!(md.contains("5 : Int64"), "md:\n{md}");
        assert_preserves(sexpr);
    }

    #[test]
    fn doc_becomes_prose() {
        let sexpr = r#"(case "documented"
          (doc "Notes for humans;
                spanning two lines.")
          (input (let ((x 10)) x))
          (output (: 10 Int64)))"#;
        let md = migrate(sexpr).unwrap();
        // The doc's source newlines/indentation collapse to clean single-spaced prose.
        assert!(
            md.contains("Notes for humans; spanning two lines."),
            "md:\n{md}"
        );
        assert_preserves(sexpr);
    }

    #[test]
    fn trap_and_compiler_error() {
        let sexpr = r#"(case "no implicit promotion"
          (input (+ 2 2.0))
          (trap "numeric type mismatch")
          (compiler (error CDZ0301)))"#;
        let md = migrate(sexpr).unwrap();
        assert!(md.contains("```trap"), "md:\n{md}");
        assert!(md.contains("numeric type mismatch"), "md:\n{md}");
        assert!(md.contains("```compiler-error"), "md:\n{md}");
        assert!(md.contains("CDZ0301"), "md:\n{md}");
        assert_preserves(sexpr);
    }

    #[test]
    fn needs_and_module_input() {
        let sexpr = r#"(case "a named higher-order function"
          (needs collections)
          (input (module m (def (ap g v) (g v)) (def (main) (ap (fn (x) (* x 2)) 7))))
          (output (: 14 Int64)))"#;
        let md = migrate(sexpr).unwrap();
        assert!(md.contains("```needs"), "md:\n{md}");
        assert!(md.contains("collections"), "md:\n{md}");
        assert!(md.contains("module m {"), "md:\n{md}");
        assert_preserves(sexpr);
    }

    #[test]
    fn call_and_host_clauses() {
        let sexpr = r#"(case "a deterministic host response"
          (needs effects)
          (input (module m (effect ask (op ask (-> Unit Int64))) (def (main) (host (ask) (+ 1 (ask.ask))))))
          (host-responses (respond ask.ask (: 41 Int64)))
          (host-calls (call ask.ask))
          (output (: 42 Int64)))"#;
        let md = migrate(sexpr).unwrap();
        assert!(md.contains("```cdz host-responses"), "md:\n{md}");
        assert!(md.contains("```cdz host-calls"), "md:\n{md}");
        assert_preserves(sexpr);
    }

    #[test]
    fn compound_output_uses_ascription() {
        let sexpr =
            r#"(case "a tuple" (input (tuple 1 2)) (output (: (tuple 1 2) (Tuple Int64 Int64))))"#;
        let md = migrate(sexpr).unwrap();
        assert!(md.contains("(1, 2) : Tuple(Int64, Int64)"), "md:\n{md}");
        assert_preserves(sexpr);
    }

    #[test]
    fn migration_is_idempotent_over_markdown() {
        // Migrating, then reconstructing + re-migrating, yields the same markdown.
        let sexpr = r#"(case "integer addition" (input (+ 2 3)) (output (: 5 Int64)))"#;
        let md1 = migrate(sexpr).unwrap();
        let recon = to_sexpr(&md1).unwrap();
        let md2 = migrate(&recon).unwrap();
        // md2 omits the doc-less case's prose (none here), so it matches md1 structurally.
        assert_eq!(
            md1, md2,
            "not idempotent:\n---md1---\n{md1}\n---md2---\n{md2}"
        );
    }

    #[test]
    fn call_clause_with_args() {
        let sexpr = r#"(case "a parameterized entry"
          (input (module m (def (main (: x Int64)) (+ x 1)) (export main)))
          (call main (: 41 Int64))
          (output (: 42 Int64)))"#;
        assert_preserves(sexpr);
    }
}
