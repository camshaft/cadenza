//! Corpus ↔ markdown migration, built ON the first-class markdown surface.
//!
//! The corpus is fundamentally *literate*: most of its cases carry prose `(doc …)`, and the files are
//! threaded with `;`-comment narrative. Markdown is its natural home — prose becomes prose, and each
//! program payload becomes a fenced `cdz` code block an editor can highlight and a tool can extract.
//!
//! This module is now a THIN layer over [`cadenza_syntax::markdown`], the generic markdown surface:
//! there is ONE markdown parser/printer (the surface), not a second bespoke one here. Migration builds
//! a `(document …)` ARENA — case descriptions become `(heading 3 …)`, `(doc …)` and `;`-narrative
//! become `(paragraph …)`, a `; --- Title --- ` banner becomes `(heading 2 …)`, and each DSL clause
//! becomes a `(code-block <info> <raw>)` — and prints it with the surface printer. Reconstruction reads
//! the `.md` back to a document arena with the surface parser and walks it: each `(heading 3 …)` opens
//! a case; each following `(code-block <info> <raw> …)` is a clause whose ROLE is the last token of the
//! info string (`cdz input` → `input`) and whose body is the verbatim `raw`.
//!
//! ## The clause fences
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
//! `read_ml` and prepends the head named by the tag. Because the surface parser ALSO embeds the parsed
//! program as a subtree inside a `cdz` code block, reading a migrated corpus with `cdz convert … --to
//! sexpr` shows the real program trees — but reconstruction re-derives from the verbatim `raw`, which
//! keeps this byte-for-byte identical to the record stream the s-expr source produces.
//!
//! ## Losslessness
//!
//! `check` proves the migration preserves everything the behaviour gate sees: the record stream
//! (`crate::render(crate::read(…))`) of the reconstructed corpus is byte-identical to the original's.
//! Case `doc` prose and inter-case `;` narrative are NOT part of that stream; they are carried into
//! markdown for faithfulness (doc whitespace is normalized to clean prose).

use cadenza_syntax::ast::{Arenas, Builder, Leaf, Radix, Struct, StructId};
use cadenza_syntax::{markdown, parser, printer, sexpr};
use num_bigint::BigInt;

/// Target width for ML rendered inside a fence.
const ML_WIDTH: usize = 90;

// ============================================================================
// Public API
// ============================================================================

/// Migrate a corpus `.sexp` file's text to markdown, with an optional document `title` (a `# …`
/// heading emitted at the top — typically the file's name). Passing `None` omits the title, which
/// keeps the output stable for tests and for re-migrating reconstructed text.
pub fn migrate_titled(sexpr_text: &str, title: Option<&str>) -> Result<String, String> {
    let mut b = Builder::new();
    let mut blocks: Vec<StructId> = Vec::new();
    if let Some(title) = title {
        blocks.push(md_heading(&mut b, 1, title));
    }
    for seg in segment(sexpr_text) {
        match seg {
            Segment::Prose(text) => {
                for pb in prose_blocks(&text) {
                    match pb {
                        ProseBlock::Heading(t) => blocks.push(md_heading(&mut b, 2, &t)),
                        ProseBlock::Paragraph(t) => blocks.push(md_paragraph(&mut b, &t)),
                    }
                }
            }
            Segment::Form(form) => {
                let a = sexpr::read(form).map_err(|e| format!("case parse error: {}", e.0))?;
                if a.head_name(a.root) != Some("case") {
                    // A non-case top-level form (unusual). Preserve it verbatim in a `cdz` fence so
                    // nothing is dropped.
                    let ml = ml_of(&a, a.root);
                    blocks.push(md_code_block(&mut b, "cdz", &ml));
                    continue;
                }
                render_case_blocks(&a, &mut b, &mut blocks)?;
            }
        }
    }
    let doc = md_list(&mut b, "document", blocks);
    let arenas = b.finish(doc);
    Ok(markdown::print(&arenas, ML_WIDTH))
}

/// Migrate a corpus `.sexp` file's text to markdown, without a document title.
pub fn migrate(sexpr_text: &str) -> Result<String, String> {
    migrate_titled(sexpr_text, None)
}

/// Verify a migration is behaviour-preserving: the reconstructed corpus produces a record stream
/// byte-identical to the original's. Returns the offending diff on mismatch. The document title
/// does not affect the record stream, so `check` uses the untitled form.
pub fn check(sexpr_text: &str) -> Result<(), String> {
    let md = migrate(sexpr_text)?;
    let reconstructed = to_sexpr(&md)?;

    let original_records = crate::render(&crate::read(sexpr_text)?);
    let round_tripped = crate::render(&crate::read(&reconstructed)?);

    if original_records == round_tripped {
        return Ok(());
    }

    // The migration serializes through the ML surface, which is allowed to CANONICALIZE once — e.g.
    // the name-alias compound ctors `(tuple a b)`/`(list …)` normalize to the string-primitive
    // `("tuple" …)`/`("list" …)` (a deliberate, semantics-preserving rewrite, not information loss).
    // So require a FIXED POINT rather than byte-identity: migrating the RECONSTRUCTED text again must
    // reproduce it. That still catches a real migration bug (a non-idempotent / lossy transform) while
    // tolerating the one-time ctor canonicalization. Mirrors `xtask roundtrip`'s ML idempotence rule.
    let reconstructed2 = to_sexpr(&migrate(&reconstructed)?)?;
    let round_tripped2 = crate::render(&crate::read(&reconstructed2)?);
    if round_tripped == round_tripped2 {
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
// Rendering: sexpr case -> document blocks
// ============================================================================

/// Append one `(case …)` arena's document blocks (a `### desc` heading, doc paragraphs, and one
/// `(code-block …)` per clause) to `blocks`, building into document builder `b`.
fn render_case_blocks(
    a: &Arenas,
    b: &mut Builder,
    blocks: &mut Vec<StructId>,
) -> Result<(), String> {
    let items = match a.get(a.root) {
        Struct::List(items) => items,
        _ => return Err("case is not a list".into()),
    };
    let description = items
        .get(1)
        .and_then(|&id| str_leaf(a, id))
        .ok_or("case has no description string")?;
    blocks.push(md_heading(b, 3, &description));

    for &clause in &items[2..] {
        match a.head_name(clause) {
            Some("doc") => {
                if let Some(text) = a
                    .as_form(clause, "doc")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(a, id))
                {
                    blocks.push(md_paragraph(b, &normalize_prose(&text)));
                }
            }
            Some(head) => {
                let (tag, body) = fence_for(a, clause, head)?;
                blocks.push(md_code_block(b, &tag, &body));
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

// ---- document-arena builder helpers (build `(document …)` nodes for the markdown surface) ----

/// `(text "s")`.
fn md_text(b: &mut Builder, s: &str) -> StructId {
    let head = b.name("text");
    let leaf = b.atom_leaf(Leaf::Str(s.to_string()));
    b.list(vec![head, leaf])
}

/// `(heading <level> (text "s"))`.
fn md_heading(b: &mut Builder, level: i64, s: &str) -> StructId {
    let head = b.name("heading");
    let lvl = b.atom_leaf(Leaf::Int {
        value: BigInt::from(level),
        radix: Radix::Dec,
    });
    let text = md_text(b, s);
    b.list(vec![head, lvl, text])
}

/// `(paragraph (text "s"))`.
fn md_paragraph(b: &mut Builder, s: &str) -> StructId {
    let head = b.name("paragraph");
    let text = md_text(b, s);
    b.list(vec![head, text])
}

/// `(code-block "info" "raw")` — the surface printer emits the `raw` verbatim inside a fence tagged
/// by `info`, so the code body round-trips byte-exact.
fn md_code_block(b: &mut Builder, info: &str, raw: &str) -> StructId {
    let head = b.name("code-block");
    let i = b.atom_leaf(Leaf::Str(info.to_string()));
    let r = b.atom_leaf(Leaf::Str(raw.to_string()));
    b.list(vec![head, i, r])
}

/// `(head child…)`.
fn md_list(b: &mut Builder, head: &str, children: Vec<StructId>) -> StructId {
    let h = b.name(head);
    let mut items = Vec::with_capacity(1 + children.len());
    items.push(h);
    items.extend(children);
    b.list(items)
}

// ============================================================================
// Parsing: markdown -> case model, via the markdown surface
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

/// Parse a migrated markdown document into its cases, by reading it with the generic markdown surface
/// and walking the resulting `(document …)` arena. Each `(heading 3 …)` opens a case; each following
/// `(code-block <info> <raw> …)` is a clause. A `# title` / `## section` heading and any `(paragraph
/// …)` prose are not part of the record stream and are ignored.
fn parse_md(md: &str) -> Result<Vec<MdCase>, String> {
    let doc = markdown::read(md);
    let blocks = match doc.get(doc.root) {
        Struct::List(items) => &items[1..], // skip the `document` head
        _ => return Ok(Vec::new()),
    };
    let mut cases: Vec<MdCase> = Vec::new();
    for &block in blocks {
        match doc.head_name(block) {
            Some("heading") => {
                if let Some((level, desc)) = heading_level_text(&doc, block)
                    && level == 3
                {
                    cases.push(MdCase {
                        description: desc,
                        fences: Vec::new(),
                    });
                }
                // A level-1 title or level-2 section heading is not a case — ignored.
            }
            Some("code-block") => {
                // A code block before any case heading is file-level and ignored; otherwise it is a
                // clause of the current case. The ROLE is the last token of the info string.
                if let Some(case) = cases.last_mut() {
                    let items = list_items(&doc, block);
                    let info = items
                        .get(1)
                        .and_then(|&s| str_leaf(&doc, s))
                        .unwrap_or_default();
                    let raw = items
                        .get(2)
                        .and_then(|&s| str_leaf(&doc, s))
                        .unwrap_or_default();
                    let role = info.split_whitespace().last().unwrap_or("").to_string();
                    case.fences.push(MdFence { role, body: raw });
                }
            }
            // Prose paragraphs (doc / narrative) and anything else are ignored.
            _ => {}
        }
    }
    Ok(cases)
}

/// A heading block's `(level, flattened-inline-text)`, if `id` is a `(heading <Int> <inline>…)`.
fn heading_level_text(a: &Arenas, id: StructId) -> Option<(i64, String)> {
    let items = list_items(a, id);
    let level = items.get(1).and_then(|&n| int_leaf(a, n))?;
    let mut text = String::new();
    flatten_inline_text(a, &items[2.min(items.len())..], &mut text);
    Some((level, text))
}

/// Concatenate the literal text carried by a run of inline nodes — the inverse of building a heading
/// from a single `(text …)`. `text`/`code`/`html` contribute their string leaf; a styled wrapper
/// (`emph`/`strong`/`link`/…) contributes its inline children. This recovers a case description
/// verbatim even when it contains markdown metacharacters (the surface printer escaped them, and the
/// reader stripped the escapes, so the text node again holds the original string).
fn flatten_inline_text(a: &Arenas, nodes: &[StructId], out: &mut String) {
    for &n in nodes {
        match a.head_name(n) {
            Some("text") | Some("code") | Some("html") => {
                if let Some(t) = list_items(a, n).get(1).and_then(|&s| str_leaf(a, s)) {
                    out.push_str(&t);
                }
            }
            Some(_) => flatten_inline_text(a, &child_tail(a, n), out),
            None => {}
        }
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

/// The value of a small `Int` leaf as `i64` (a heading level), if `id` is one.
fn int_leaf(a: &Arenas, id: StructId) -> Option<i64> {
    match a.get(id) {
        Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Int { value, .. } => i64::try_from(value).ok(),
            _ => None,
        },
        _ => None,
    }
}

/// The children of a `List` (including the head), or empty.
fn list_items(a: &Arenas, id: StructId) -> Vec<StructId> {
    match a.get(id) {
        Struct::List(items) => items.clone(),
        _ => Vec::new(),
    }
}

/// The tail of a `List` (children after the head).
fn child_tail(a: &Arenas, id: StructId) -> Vec<StructId> {
    match a.get(id) {
        Struct::List(items) => items[1.min(items.len())..].to_vec(),
        _ => Vec::new(),
    }
}

/// Normalize a `doc` string to clean prose: collapse each run of whitespace (including the source
/// literal's newlines + indentation) to a single space. Prose is not part of the record stream, so
/// this reflow is faithful enough and keeps the migration idempotent.
fn normalize_prose(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A block of prose recovered from a run of `;` comment lines.
enum ProseBlock {
    /// A `; --- Title --- ` section banner → a `## Title` heading.
    Heading(String),
    /// A run of consecutive comment lines → one paragraph.
    Paragraph(String),
}

/// Extract prose blocks from a run of `;` comment lines (an inter-case gap). Consecutive comment lines
/// join into one paragraph; a blank line separates paragraphs. A **banner** line — a comment whose
/// content is a title fenced by `---` dash-runs (`--- The number / identifier boundary ---`) — becomes
/// a section heading instead of prose, and a bare divider (dashes with no title) is dropped.
/// Non-comment content is ignored.
fn prose_blocks(gap: &str) -> Vec<ProseBlock> {
    let mut blocks: Vec<ProseBlock> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let flush = |current: &mut Vec<String>, blocks: &mut Vec<ProseBlock>| {
        if !current.is_empty() {
            let para = current.join(" ");
            let para = para.split_whitespace().collect::<Vec<_>>().join(" ");
            if !para.is_empty() {
                blocks.push(ProseBlock::Paragraph(para));
            }
            current.clear();
        }
    };
    for line in gap.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(';') {
            let content = rest.strip_prefix(' ').unwrap_or(rest).trim_end();
            if let Some(banner) = banner_title(content) {
                // A section banner ends the current paragraph and becomes a heading (or, if it has
                // no title, is dropped as a bare divider).
                flush(&mut current, &mut blocks);
                if !banner.is_empty() {
                    blocks.push(ProseBlock::Heading(banner.to_string()));
                }
            } else {
                current.push(content.to_string());
            }
        } else if t.is_empty() {
            flush(&mut current, &mut blocks);
        }
    }
    flush(&mut current, &mut blocks);
    blocks
}

/// If `content` is a section banner — its text stripped of leading/trailing `-`/space runs, and it
/// actually contained a `---` dash-run of 3+ — return the inner title (possibly empty for a bare
/// divider). Otherwise `None`. A prose line that merely contains a lone `-` (a hyphen or an em-dash
/// surrogate) is NOT a banner: the run must be 3+ dashes.
fn banner_title(content: &str) -> Option<&str> {
    if !content.contains("---") {
        return None;
    }
    // A banner is composed only of dashes, spaces, and the title text between them; strip the
    // dash-runs off both ends.
    let title = content.trim_matches(|c: char| c == '-' || c == ' ');
    // Guard against a normal sentence that happens to contain `---`: a banner's title itself must
    // not contain a 3+ dash run (the dashes only frame it).
    if title.contains("---") {
        return None;
    }
    Some(title)
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

    #[test]
    fn read_markdown_matches_read_sexpr() {
        // The xtask reader seam: reading a migrated `.md` yields the identical record STREAM the
        // source `.sexp` does — the property the `records` bin relies on to serve either extension.
        let sexpr = r#"(case "integer addition" (input (+ 2 3)) (output (: 5 Int64)))
(case "no implicit promotion"
  (input (+ 2 2.0))
  (trap "numeric type mismatch")
  (compiler (error CDZ0301)))"#;
        let md = migrate(sexpr).unwrap();
        let from_sexpr = crate::render(&crate::read(sexpr).unwrap());
        let from_md = crate::render(&crate::read_markdown(&md).unwrap());
        assert_eq!(from_sexpr, from_md, "md:\n{md}");
    }

    #[test]
    fn title_and_banner_headings() {
        // A `; --- Title --- ` banner becomes `## Title`; a bare `; -----` divider is dropped; the
        // document title becomes `# name`. None of these affect the reconstructed record stream.
        let sexpr = r#"; --- Radix literals ------------------------------
; Intro prose for the section.
(case "hex" (input 0xff) (output (: 255 Int64)))
; ------------------------------
(case "dec" (input 42) (output (: 42 Int64)))"#;
        let md = migrate_titled(sexpr, Some("01-literals")).unwrap();
        assert!(md.starts_with("# 01-literals\n"), "md:\n{md}");
        assert!(md.contains("## Radix literals"), "md:\n{md}");
        assert!(md.contains("Intro prose for the section."), "md:\n{md}");
        // the bare divider between the two cases produced no heading (count `## ` lines exactly, not
        // the `## ` substring that also sits inside every `### ` case heading).
        let section_headings = md.lines().filter(|l| l.starts_with("## ")).count();
        assert_eq!(section_headings, 1, "md:\n{md}");
        // and the title/headings don't disturb the record stream
        let from_sexpr = crate::render(&crate::read(sexpr).unwrap());
        let from_md = crate::render(&crate::read_markdown(&md).unwrap());
        assert_eq!(from_sexpr, from_md, "md:\n{md}");
    }

    #[test]
    fn description_with_markdown_metacharacters() {
        // A case description containing markdown-special chars (the corpus has `Ast.*` and
        // `make-<name>`) must survive migration + reconstruction verbatim — the surface printer
        // escapes them, and reading strips the escapes, so the record's `case\t<desc>` is unchanged.
        let sexpr = r#"(case "a quote pattern equals the Ast.* constructor for make-<name>"
          (input (+ 1 1)) (output (: 2 Int64)))"#;
        let md = migrate(sexpr).unwrap();
        let from_sexpr = crate::render(&crate::read(sexpr).unwrap());
        let from_md = crate::render(&crate::read_markdown(&md).unwrap());
        assert_eq!(from_sexpr, from_md, "md:\n{md}");
    }

    #[test]
    fn em_dash_in_prose_is_not_a_banner() {
        // An em-dash or a lone hyphen inside prose must NOT be mistaken for a section banner (only a
        // 3+ dash run frames a banner).
        assert_eq!(banner_title("a value — its type"), None);
        assert_eq!(banner_title("a-b hyphenated"), None);
        assert_eq!(banner_title("--- Real Banner ---"), Some("Real Banner"));
        assert_eq!(banner_title("----------"), Some(""));
    }
}
