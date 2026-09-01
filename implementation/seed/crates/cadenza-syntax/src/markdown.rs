//! The **markdown surface** — a literate document as a projection of the one canonical arena.
//!
//! Markdown is a first-class front-end syntax, exactly like the s-expression and ML surfaces: a
//! parser (`read`) turns CommonMark text into the shared [`Arenas`], and a printer (`print`) turns a
//! document arena back into CommonMark. It is not privileged (`spec/contracts/ast-encoding.md` §A
//! Textual Syntax Parses To And Prints From The Canonical Form) — a `.md` reads to the same binary
//! AST any surface does, so `cdz convert doc.md --to binary` yields a canonical arena.
//!
//! Unlike the code surfaces, a document is *data*, not a program: its nodes are markdown structure
//! (`document`/`heading`/`paragraph`/`code-block`/…), not language constructs, so the compiler never
//! sees one. But because Cadenza is homoiconic, a **fenced `cdz`/`ml`/`sexp` code block carries its
//! program as a real arena SUBTREE** embedded in the document — the code inside a doc *is* arena, not
//! an opaque string. Tooling extracts + compiles those subtrees (the corpus reader does exactly this).
//!
//! ## Node vocabulary (all ordinary `Name`-headed lists — no codec change, no version bump)
//!
//! Root: `(document <block>…)`. Blocks:
//! - `(heading <level:Int> <inline>…)` — level 1–6
//! - `(paragraph <inline>…)`
//! - `(code-block <info:Str> <raw:Str> [<subtree>])` — `info` is the full fence info string (e.g.
//!   `"cdz input"`, `""` for a bare fence); `raw` is the VERBATIM body (so code round-trips
//!   byte-exact). A block whose info's first token is `cdz`/`cadenza`/`ml`/`sexp`/`sexpr` and whose
//!   body parses cleanly ALSO carries the parsed program as a third child (the embedded subtree).
//! - `(block-quote <block>…)`, `(list <ordered|unordered> [<start:Int>] <item>…)`, `(item <block>…)`,
//!   `(thematic-break)`, `(html-block <Str>)`
//! - GFM tables: `(table (table-head <cell>…) (table-row <cell>…)…)`, `(table-cell <inline>…)`
//!
//! Inline: `(text <Str>)`, `(code <Str>)` (inline code), `(emph <inline>…)`, `(strong <inline>…)`,
//! `(strike <inline>…)`, `(link <dest:Str> <title:Str> <inline>…)`,
//! `(image <dest:Str> <title:Str> <inline>…)`, `(soft-break)`, `(hard-break)`, `(html <Str>)`.
//!
//! ## Round-trip
//!
//! CommonMark is not injective (many texts render one tree), so the guarantee is **arena-idempotence**
//! — `read(print(read(md)))` equals `read(md)` — the same contract the ML surface holds, not byte
//! identity of the source. Code-block bodies ARE byte-exact, because the printer emits the stored
//! `raw` verbatim rather than re-rendering the embedded subtree.

use crate::arena_read::{child_tail, int_leaf, list_items, str_leaf};
use crate::ast::{Arenas, Builder, Leaf, StructId};
use crate::span::Span;
use crate::spans::{FileId, SpanTable};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Parse CommonMark `src` into a `(document …)` arena (with GFM tables + strikethrough).
pub fn read(src: &str) -> Arenas {
    let mut b = Builder::new();
    let root = Md::new(&mut b, None).run(src);
    b.finish(root)
}

/// Parse CommonMark `src` into a `(document …)` arena, ALSO producing a [`SpanTable`] mapping each
/// structure occurrence to its source byte range — the same source-tracking substrate the code
/// surfaces produce. The arena is byte-identical to [`read`]'s; only the table is extra.
///
/// A NODE synthesized from a code block's embedded program subtree (built by `read_ml`/`sexpr::read`
/// on the fence BODY, whose spans are relative to that body, not the document) is given a best-effort
/// span covering the whole `(code-block …)` — the table stays total and 1:1, and a cursor inside a
/// fence resolves to the fence rather than to a byte offset in the wrong coordinate system.
pub fn read_spanned(src: &str) -> (Arenas, SpanTable) {
    let mut b = Builder::new();
    let mut md = Md::new(&mut b, Some(SpanTable::new(FileId::default())));
    let root = md.run(src);
    let spans = md.spans.take().expect("span tracking on");
    (b.finish(root), spans)
}

// ============================================================================
// Reader: CommonMark events -> document arena
// ============================================================================

/// The event-folding reader. pulldown-cmark yields a preorder `Start(tag)…End` stream; we keep an
/// explicit stack of open containers, each accumulating its built children, and close one on `End`.
struct Md<'b> {
    b: &'b mut Builder,
    /// One frame per open container (`document`, a block, or an inline span). The bottom frame is the
    /// document; `run` pops it last.
    stack: Vec<Frame>,
    /// Accumulated adjacent `Text` events, flushed as ONE `(text …)` node before any other event.
    /// pulldown-cmark splits a run of text at each backslash-escape (`Ast.\*` → three `Text` events),
    /// so coalescing keeps a logical text run one node — and makes the tree stable under the printer's
    /// escaping (which re-emits `*` as `\*`, re-split on the next read).
    pending_text: Option<(String, Span)>,
    /// When `Some`, every created occurrence pushes its span here in id order (kept 1:1 with the arena).
    spans: Option<SpanTable>,
}

/// An open container: the head name to emit when it closes, any leading fixed children (a heading's
/// level, a list's ordered/start, a code block's info+raw+subtree), and the child occurrences built
/// so far. `span` is the container's source range (from the `Start` event's offset).
struct Frame {
    head: &'static str,
    prefix: Vec<StructId>,
    children: Vec<StructId>,
    span: Span,
}

impl<'b> Md<'b> {
    fn new(b: &'b mut Builder, spans: Option<SpanTable>) -> Md<'b> {
        Md {
            b,
            stack: Vec::new(),
            pending_text: None,
            spans,
        }
    }

    /// Fold the whole event stream and return the `(document …)` root.
    fn run(&mut self, src: &str) -> StructId {
        let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
        // The document frame spans the whole source; it is the last frame popped.
        self.stack.push(Frame {
            head: "document",
            prefix: Vec::new(),
            children: Vec::new(),
            span: Span::new(0, src.len()),
        });
        for (event, range) in Parser::new_ext(src, opts).into_offset_iter() {
            let span = Span::new(range.start, range.end);
            self.event(event, span, src);
        }
        self.flush_text();
        let doc = self.stack.pop().expect("document frame");
        self.close_frame(doc)
    }

    fn event(&mut self, event: Event, span: Span, src: &str) {
        // A `Text` event accumulates into the pending run; anything else flushes it first, so a run
        // of text (which pulldown-cmark splits at each `\`-escape) becomes ONE `(text …)` node.
        if let Event::Text(t) = &event {
            self.push_text(t.as_ref(), span);
            return;
        }
        self.flush_text();
        match event {
            Event::Start(tag) => self.start(tag, span, src),
            Event::End(end) => self.end(end, src),

            Event::Text(_) => unreachable!("handled above"),
            Event::Code(c) => {
                let node = self.mk_list_of("code", &[Leaf::Str(c.into_string().into())], span);
                self.push_child(node);
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                // A raw-HTML line (block or inline) — carried verbatim as `(html "…")`. Block vs inline
                // is recoverable from context (an html node directly under `document` is a block); we
                // keep one head so the vocabulary stays small.
                let node = self.mk_list_of("html", &[Leaf::Str(h.into_string().into())], span);
                self.push_child(node);
            }
            Event::SoftBreak => {
                let node = self.mk_leaf_form("soft-break", span);
                self.push_child(node);
            }
            Event::HardBreak => {
                let node = self.mk_leaf_form("hard-break", span);
                self.push_child(node);
            }
            Event::Rule => {
                let node = self.mk_leaf_form("thematic-break", span);
                self.push_child(node);
            }
            // Constructs behind options we did not enable (math, footnotes, task lists, metadata,
            // definition lists) never arise; fold defensively as text so nothing is silently dropped.
            Event::InlineMath(t) | Event::DisplayMath(t) | Event::FootnoteReference(t) => {
                self.push_text(t.as_ref(), span);
            }
            Event::TaskListMarker(_) => {}
        }
    }

    /// Accumulate a `Text` fragment into the pending run, extending its span.
    fn push_text(&mut self, t: &str, span: Span) {
        match &mut self.pending_text {
            Some((buf, sp)) => {
                buf.push_str(t);
                *sp = sp.merge(span);
            }
            None => self.pending_text = Some((t.to_string(), span)),
        }
    }

    /// Emit the pending text run (if any) as one `(text …)` node in the current frame.
    fn flush_text(&mut self) {
        if let Some((buf, span)) = self.pending_text.take() {
            let node = self.mk_list_of("text", &[Leaf::Str(buf.into())], span);
            self.push_child(node);
        }
    }

    /// Open a container frame for a `Start(tag)`.
    fn start(&mut self, tag: Tag, span: Span, src: &str) {
        match tag {
            Tag::Paragraph => self.open("paragraph", Vec::new(), span),
            Tag::Heading { level, .. } => {
                let lvl = self.mk_int(heading_level(level) as i64, span);
                self.open("heading", vec![lvl], span);
            }
            Tag::BlockQuote(_) => self.open("block-quote", Vec::new(), span),
            Tag::CodeBlock(kind) => {
                // A code block's body arrives as `Text` events; we don't want them folded as inline
                // children, so we build the WHOLE `(code-block …)` here from the source slice and mark
                // the frame INERT (its text events are swallowed). `raw` is the verbatim fence body.
                let info = match &kind {
                    CodeBlockKind::Fenced(lang) => lang.as_ref().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let raw = fence_body(src, span, &kind);
                let node = self.code_block(&info, &raw, span);
                self.push_child(node);
                // Push an inert frame so the block's Text/End events are absorbed without effect.
                self.stack.push(Frame {
                    head: INERT,
                    prefix: Vec::new(),
                    children: Vec::new(),
                    span,
                });
            }
            Tag::HtmlBlock => {
                // Body arrives as Html events (handled as `(html …)` children); wrap them in an inert
                // frame so they attach to the document, not swallowed — actually we DO want them, so
                // this is a transparent frame whose children flush to the parent on close.
                self.open(TRANSPARENT, Vec::new(), span);
            }
            Tag::List(start) => {
                let kind = self.mk_name(
                    if start.is_some() {
                        "ordered"
                    } else {
                        "unordered"
                    },
                    span,
                );
                let mut prefix = vec![kind];
                if let Some(n) = start {
                    prefix.push(self.mk_int(n as i64, span));
                }
                self.open("list", prefix, span);
            }
            Tag::Item => self.open("item", Vec::new(), span),
            Tag::Table(_) => self.open("table", Vec::new(), span),
            Tag::TableHead => self.open("table-head", Vec::new(), span),
            Tag::TableRow => self.open("table-row", Vec::new(), span),
            Tag::TableCell => self.open("table-cell", Vec::new(), span),
            Tag::Emphasis => self.open("emph", Vec::new(), span),
            Tag::Strong => self.open("strong", Vec::new(), span),
            Tag::Strikethrough => self.open("strike", Vec::new(), span),
            Tag::Link {
                dest_url, title, ..
            } => {
                let dest = self.mk_str(dest_url.into_string(), span);
                let title = self.mk_str(title.into_string(), span);
                self.open("link", vec![dest, title], span);
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                let dest = self.mk_str(dest_url.into_string(), span);
                let title = self.mk_str(title.into_string(), span);
                self.open("image", vec![dest, title], span);
            }
            // Options we did not enable — never emitted. Open a transparent frame so if one ever is,
            // its children flow to the parent rather than corrupting the stack.
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => self.open(TRANSPARENT, Vec::new(), span),
        }
    }

    /// Close the innermost frame on an `End`.
    fn end(&mut self, _end: TagEnd, _src: &str) {
        let frame = self.stack.pop().expect("balanced end");
        if frame.head == INERT {
            // A code block: its `(code-block …)` was already emitted at Start; the frame only absorbed
            // the body's text events. Nothing to attach.
            return;
        }
        if frame.head == TRANSPARENT {
            // Flush the frame's children straight into its parent (an html-block wrapper, or a
            // disabled construct that still emitted children).
            for c in frame.children {
                self.push_child(c);
            }
            return;
        }
        let node = self.close_frame(frame);
        self.push_child(node);
    }

    /// Push `prefix` as a new open frame with the given head.
    fn open(&mut self, head: &'static str, prefix: Vec<StructId>, span: Span) {
        self.stack.push(Frame {
            head,
            prefix,
            children: Vec::new(),
            span,
        });
    }

    /// Build the `(head prefix… children…)` list for a closed frame.
    fn close_frame(&mut self, frame: Frame) -> StructId {
        let head = self.mk_name(frame.head, frame.span);
        let mut items = Vec::with_capacity(1 + frame.prefix.len() + frame.children.len());
        items.push(head);
        items.extend(frame.prefix);
        items.extend(frame.children);
        self.mk_list(items, frame.span)
    }

    /// Attach a built occurrence to the innermost open frame.
    fn push_child(&mut self, node: StructId) {
        self.stack
            .last_mut()
            .expect("an open frame")
            .children
            .push(node);
    }

    /// Build `(code-block <info> <raw> [<subtree>])`, embedding the parsed program subtree when the
    /// fence is a code fence and its body parses cleanly.
    fn code_block(&mut self, info: &str, raw: &str, span: Span) -> StructId {
        let head = self.mk_name("code-block", span);
        let info_leaf = self.mk_str(info.to_string(), span);
        let raw_leaf = self.mk_str(raw.to_string(), span);
        let mut items = vec![head, info_leaf, raw_leaf];
        if let Some(sub) = self.embed_program(info, raw, span) {
            items.push(sub);
        }
        self.mk_list(items, span)
    }

    /// If `info`'s role marks a Cadenza code block and `raw` parses cleanly, clone the parsed program
    /// into this arena and return its root (spanned over the whole fence). Otherwise `None` — a
    /// mistagged or malformed block stays raw-only, never embedding `<error>` placeholder nodes.
    fn embed_program(&mut self, info: &str, raw: &str, span: Span) -> Option<StructId> {
        match code_role(info) {
            CodeRole::Ml => {
                let parsed = crate::parser::read_ml(raw);
                parsed
                    .ok()
                    .then(|| self.clone_subtree(&parsed.arenas, parsed.arenas.root, span))
            }
            CodeRole::Sexpr => {
                // A corpus clause body may be several top-level forms (`(needs …)` etc.), so fall back
                // to the `(do …)`-wrapping multi-form reader when a single form leaves trailing input.
                let arenas = crate::sexpr::read(raw)
                    .or_else(|_| crate::sexpr::read_all(raw))
                    .ok()?;
                Some(self.clone_subtree(&arenas, arenas.root, span))
            }
            CodeRole::Other => None,
        }
    }

    /// Deep-clone the subtree rooted at `id` in `src` into this builder, giving every synthesized
    /// occurrence the same best-effort `span` (the fence's range — the source coordinates of the
    /// program's own spans do not map onto the document).
    ///
    /// Uses an EXPLICIT `Job{Visit|Emit}` stack (post-order), not native recursion — the standing rule
    /// that every arena-tree walk in the crate is iterative (an embedded fence body is depth-capped by
    /// its reader today, but keeping the walk iterative is consistent + defensive). The push order is
    /// IDENTICAL to the recursive form: a child's `mk_atom_leaf`/`mk_list` (and its span push) happens
    /// before its parent's `mk_list`, so the span table stays 1:1 and in structure order.
    fn clone_subtree(&mut self, src: &Arenas, root: StructId, span: Span) -> StructId {
        enum Job {
            Visit(StructId),
            // Emit a `List` node for `n` already-cloned children sitting atop `results`.
            Emit(usize),
        }
        let mut jobs: Vec<Job> = vec![Job::Visit(root)];
        let mut results: Vec<StructId> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(id) => match src.get(id) {
                    crate::ast::Struct::Atom(l) => {
                        let leaf = src.leaf(*l).clone();
                        results.push(self.mk_atom_leaf(leaf, span));
                    }
                    crate::ast::Struct::List(items) => {
                        jobs.push(Job::Emit(items.len()));
                        // Push children reversed so they pop (and thus clone + span-push) left-to-right →
                        // their new ids land on `results` in source order for the parent's `mk_list`.
                        for &c in items.iter().rev() {
                            jobs.push(Job::Visit(c));
                        }
                    }
                },
                Job::Emit(n) => {
                    let children = results.split_off(results.len() - n);
                    results.push(self.mk_list(children, span));
                }
            }
        }
        results
            .pop()
            .expect("clone_subtree leaves the root's new id")
    }

    // ---- span-recording arena helpers (mirror sexpr's `mk_*`; push one span per StructId) ----

    fn push_span(&mut self, span: Span) {
        if let Some(t) = self.spans.as_mut() {
            debug_assert_eq!(
                t.len() + 1,
                self.b.structure_len(),
                "markdown span table drifted from the arena"
            );
            t.push(span);
        }
    }

    fn mk_name(&mut self, name: &str, span: Span) -> StructId {
        let id = self.b.name(name);
        self.push_span(span);
        id
    }

    fn mk_str(&mut self, s: String, span: Span) -> StructId {
        self.mk_atom_leaf(Leaf::Str(s.into()), span)
    }

    fn mk_int(&mut self, n: i64, span: Span) -> StructId {
        self.mk_atom_leaf(
            Leaf::Int {
                value: crate::ast::IntValue::from_i64(n),
                radix: crate::ast::Radix::Dec,
            },
            span,
        )
    }

    fn mk_atom_leaf(&mut self, leaf: Leaf, span: Span) -> StructId {
        let id = self.b.atom_leaf(leaf);
        self.push_span(span);
        id
    }

    fn mk_list(&mut self, items: Vec<StructId>, span: Span) -> StructId {
        let id = self.b.list(items);
        self.push_span(span);
        id
    }

    /// A single-child `(head <leaf>)` form (e.g. `(text "…")`).
    fn mk_list_of(&mut self, head: &str, leaves: &[Leaf], span: Span) -> StructId {
        let h = self.mk_name(head, span);
        let mut items = vec![h];
        for leaf in leaves {
            items.push(self.mk_atom_leaf(leaf.clone(), span));
        }
        self.mk_list(items, span)
    }

    /// A childless `(head)` form (e.g. `(soft-break)`, `(thematic-break)`).
    fn mk_leaf_form(&mut self, head: &str, span: Span) -> StructId {
        let h = self.mk_name(head, span);
        self.mk_list(vec![h], span)
    }
}

/// Sentinel head for a frame whose text/end events are swallowed (a code block, whose node is built
/// eagerly at Start).
const INERT: &str = "\0inert";
/// Sentinel head for a frame whose children flush into its parent on close (an html-block wrapper, or
/// a disabled construct that still yields children).
const TRANSPARENT: &str = "\0transparent";

/// The 1-based heading level.
fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// What a fenced code block's info string means for embedding. The ROLE is the LAST whitespace token
/// of the info string, matching the corpus fence convention (`cdz input` → role `input`; a leading
/// `cdz` is a highlight hint). The LANGUAGE is the FIRST token.
enum CodeRole {
    /// An ML/Cadenza body — parse with `read_ml`.
    Ml,
    /// An s-expression body — parse with `sexpr::read`.
    Sexpr,
    /// Not a Cadenza code block (plain text, another language, a bare fence) — no embedding.
    Other,
}

/// Classify a fence info string. A block is Cadenza code when its FIRST token is a Cadenza language
/// tag (`cdz`/`cadenza`/`ml` → ML; `sexp`/`sexpr` → s-expr). The corpus emits `cdz input`,
/// `cdz output`, `error`, `needs`, etc.: the leading `cdz` marks the ML-bearing blocks, and a bare
/// role tag (`error`/`needs`/`compiler-error`) is corpus metadata that is NOT program text — so those
/// are `Other` here and the corpus layer reconstructs them from `raw`.
fn code_role(info: &str) -> CodeRole {
    match info.split_whitespace().next() {
        Some("cdz") | Some("cadenza") | Some("ml") => CodeRole::Ml,
        Some("sexp") | Some("sexpr") => CodeRole::Sexpr,
        _ => CodeRole::Other,
    }
}

/// Extract a fenced/indented code block's VERBATIM body from the source, given the block's full span
/// (which includes the fence lines). For a fenced block we drop the opening fence line and the closing
/// fence line; for an indented block we strip the 4-space indent. Returns the body WITHOUT a trailing
/// newline (the printer re-adds fence framing), so `read`→`print` re-frames identically.
fn fence_body(src: &str, span: Span, kind: &CodeBlockKind) -> String {
    let slice = &src[span.start..span.end.min(src.len())];
    match kind {
        CodeBlockKind::Fenced(_) => {
            let mut lines: Vec<&str> = slice.split('\n').collect();
            // Drop the opening fence line (```lang or ~~~lang).
            if !lines.is_empty() {
                lines.remove(0);
            }
            // Drop a trailing empty element from a final newline, then the closing fence line. Capture the
            // closing fence's own leading indent FIRST: when the block sits inside a list item (or block
            // quote), the span starts AT the opening fence — so the opening line has no container indent
            // in the slice, but every CONTINUATION line (body + closing fence) still carries the
            // container's indentation. That indent is NOT part of the code; leaving it in the stored `raw`
            // makes the printer re-indent it on top of the list indent (`  code` → `    code`), so the
            // re-read sees deeper-indented code — a different tree (arena-idempotency break).
            if lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
            let container_indent = lines
                .last()
                .filter(|l| is_fence_line(l))
                .map(|l| l.len() - l.trim_start_matches(' ').len())
                .unwrap_or(0);
            if lines.last().is_some_and(|l| is_fence_line(l)) {
                lines.pop();
            }
            // Strip the container indent (at most) from each body line, so `raw` holds the code as it
            // reads AFTER the container strips its indent — the same bytes a top-level fence would store.
            lines
                .iter()
                .map(|l| strip_up_to_spaces(l, container_indent))
                .collect::<Vec<_>>()
                .join("\n")
        }
        CodeBlockKind::Indented => slice
            .lines()
            .map(|l| l.strip_prefix("    ").unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string(),
    }
}

/// Strip up to `n` leading SPACE characters from `line` (a blank/shorter line loses only what it has) —
/// used to remove a code block's container indent without touching the code's own deeper indentation.
fn strip_up_to_spaces(line: &str, n: usize) -> &str {
    let strippable = line.len() - line.trim_start_matches(' ').len();
    &line[strippable.min(n)..]
}

/// Render an inline-code span per CommonMark: the backtick delimiter is a run ONE longer than the
/// longest backtick run inside the content (so the content's backticks can't close the span early),
/// and when the content begins or ends with a backtick (or is entirely backticks) a single space is
/// added on each side (stripped on re-parse). Without this, a body like `` `(+ ,a ,b)` `` — which the
/// spec README contains — reprints with the wrong delimiter and re-reads to a different tree.
fn render_inline_code(content: &str) -> String {
    let max_run = content
        .split(|c| c != '`')
        .map(|run| run.len())
        .max()
        .unwrap_or(0);
    let ticks = "`".repeat(max_run + 1);
    let needs_pad = content.starts_with('`')
        || content.ends_with('`')
        || content.chars().all(|c| c == '`') && !content.is_empty();
    let mut out = String::new();
    out.push_str(&ticks);
    if needs_pad {
        out.push(' ');
    }
    out.push_str(content);
    if needs_pad {
        out.push(' ');
    }
    out.push_str(&ticks);
    out
}

/// Whether a line is a code-fence delimiter (a run of 3+ backticks or tildes, optionally indented).
fn is_fence_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

// ============================================================================
// Printer: document arena -> CommonMark text
// ============================================================================

/// Render a `(document …)` arena as CommonMark. `width` is accepted for surface-layer uniformity; the
/// markdown printer wraps only where the source structure requires (prose reflows to one line per
/// paragraph, which re-reads identically). A NON-document root (e.g. a bare program handed to `cdz
/// convert prog.cdz --to md`) is wrapped in a single ```cdz fence over its ML rendering, so `--to md`
/// stays total and meaningful.
pub fn print(arenas: &Arenas, width: usize) -> String {
    let mut out = String::new();
    if arenas.head_name(arenas.root) == Some("document") {
        let blocks = child_tail(arenas, arenas.root);
        print_blocks(arenas, &blocks, &mut out);
    } else {
        // Fallback: wrap a program arena as one cdz fence.
        out.push_str("```cdz\n");
        out.push_str(&crate::printer::print(arenas, width));
        out.push_str("\n```\n");
    }
    out
}

/// Print a sequence of block nodes, separating each from the next with a blank line. A run of INLINE
/// nodes appearing directly among blocks — how pulldown-cmark represents a TIGHT list item's content
/// (text is placed under `(item …)` with no wrapping `(paragraph …)`) — is coalesced into one line.
fn print_blocks(a: &Arenas, blocks: &[StructId], out: &mut String) {
    let mut i = 0;
    let mut first = true;
    let mut prev_inline = false;
    while i < blocks.len() {
        // A nested list directly following a tight item's inline text must HUG it — no blank-line
        // separator. CommonMark reads a blank line before the sublist as making the item LOOSE (it then
        // wraps the leading text in a `(paragraph …)`), so emitting one here reshapes `(item (text …)
        // (list …))` into a loose item on the re-read — a different tree, breaking arena-idempotency
        // (`- item\n  - nested\n` round-tripped through a spurious `- item\n  \n  - nested\n`). Only this
        // inline-run → list transition is suppressed; genuine block/block separations still get the line.
        let cur_is_list = a.head_name(blocks[i]) == Some("list");
        if !(first || prev_inline && cur_is_list) {
            out.push('\n');
        }
        first = false;
        if is_inline_head(a.head_name(blocks[i])) {
            // Gather the maximal run of inline siblings and print them as one line.
            let start = i;
            while i < blocks.len() && is_inline_head(a.head_name(blocks[i])) {
                i += 1;
            }
            print_inlines(a, &blocks[start..i], out);
            out.push('\n');
            prev_inline = true;
        } else {
            print_block(a, blocks[i], out);
            i += 1;
            prev_inline = false;
        }
    }
}

/// Whether a head names an INLINE node (vs a block). `html` is deliberately excluded: at block level
/// it is a block (`print_block` re-adds its newline), and inline HTML only occurs inside a paragraph
/// (handled by that paragraph's `print_inlines`).
fn is_inline_head(head: Option<&str>) -> bool {
    matches!(
        head,
        Some(
            "text"
                | "code"
                | "emph"
                | "strong"
                | "strike"
                | "link"
                | "image"
                | "soft-break"
                | "hard-break"
        )
    )
}

fn print_block(a: &Arenas, id: StructId, out: &mut String) {
    match a.head_name(id) {
        Some("heading") => {
            let items = list_items(a, id);
            let level = items
                .get(1)
                .and_then(|&n| int_leaf(a, n))
                .unwrap_or(1)
                .clamp(1, 6);
            for _ in 0..level {
                out.push('#');
            }
            out.push(' ');
            print_inlines(a, &items[2.min(items.len())..], out);
            out.push('\n');
        }
        Some("paragraph") => {
            print_inlines(a, &child_tail(a, id), out);
            out.push('\n');
        }
        Some("code-block") => {
            let items = list_items(a, id);
            let info = items
                .get(1)
                .and_then(|&s| str_leaf(a, s))
                .unwrap_or_default();
            let raw = items
                .get(2)
                .and_then(|&s| str_leaf(a, s))
                .unwrap_or_default();
            out.push_str("```");
            out.push_str(&info);
            out.push('\n');
            out.push_str(&raw);
            if !raw.is_empty() {
                out.push('\n');
            }
            out.push_str("```\n");
        }
        Some("block-quote") => {
            let mut inner = String::new();
            print_blocks(a, &child_tail(a, id), &mut inner);
            for line in inner.trim_end_matches('\n').split('\n') {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
        }
        Some("list") => print_list(a, id, out),
        Some("thematic-break") => out.push_str("---\n"),
        Some("html") | Some("html-block") => {
            let items = list_items(a, id);
            if let Some(h) = items.get(1).and_then(|&s| str_leaf(a, s)) {
                out.push_str(&h);
                if !h.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        Some("table") => print_table(a, id, out),
        _ => {
            // An unknown block — render its inline children as a paragraph so nothing is dropped.
            print_inlines(a, &child_tail(a, id), out);
            out.push('\n');
        }
    }
}

fn print_list(a: &Arenas, id: StructId, out: &mut String) {
    let items = list_items(a, id);
    // (list <ordered|unordered> [start] <item>…)
    let ordered = items.get(1).and_then(|&k| a.as_name(k)) == Some("ordered");
    let mut n = 1i64;
    let mut rest = &items[2.min(items.len())..];
    if ordered
        && let Some(&start) = rest.first()
        && let Some(s) = int_leaf(a, start)
    {
        n = s;
        rest = &rest[1..];
    }
    // LOOSE vs TIGHT. A tight item holds its text as bare inline (`(item (text …) …)`); a loose one
    // wraps it in a paragraph (`(item (paragraph …))`) — pulldown-cmark's representation, and the whole
    // list is loose if ANY item is. A loose list MUST be printed with a blank line between items, else
    // the reprint reads back TIGHT (each item's paragraph collapses to bare inline) — a different tree,
    // breaking arena-idempotency (`- a\n\n- b\n` → reprinted tight `- a\n- b\n` → loses the paragraphs).
    let loose = rest.iter().any(|&item| {
        child_tail(a, item)
            .iter()
            .any(|&c| a.head_name(c) == Some("paragraph"))
    });
    for (idx, &item) in rest.iter().enumerate() {
        if loose && idx > 0 {
            out.push('\n'); // blank line separating loose items
        }
        let marker = if ordered {
            let m = format!("{n}. ");
            n += 1;
            m
        } else {
            "- ".to_string()
        };
        let mut inner = String::new();
        print_blocks(a, &child_tail(a, item), &mut inner);
        let indent = " ".repeat(marker.len());
        for (li, line) in inner.trim_end_matches('\n').split('\n').enumerate() {
            if li == 0 {
                out.push_str(&marker);
            } else if line.is_empty() {
                // Don't emit trailing indent on a blank continuation line (keeps output clean and avoids
                // reintroducing the tight-item spurious-blank shape a nested block already separates with).
                out.push('\n');
                continue;
            } else {
                out.push_str(&indent);
            }
            out.push_str(line);
            out.push('\n');
        }
    }
}

fn print_table(a: &Arenas, id: StructId, out: &mut String) {
    let rows = child_tail(a, id);
    // Render one row as `| cell | cell |`, returning the column count.
    let render_row = |a: &Arenas, row: StructId, out: &mut String| -> usize {
        let cells = child_tail(a, row);
        out.push('|');
        for &cell in &cells {
            out.push(' ');
            let mut c = String::new();
            print_inlines(a, &child_tail(a, cell), &mut c);
            out.push_str(c.trim());
            out.push_str(" |");
        }
        out.push('\n');
        cells.len()
    };
    for &row in &rows {
        match a.head_name(row) {
            Some("table-head") => {
                let cols = render_row(a, row, out);
                // The GFM delimiter row, one `---` per column.
                out.push('|');
                for _ in 0..cols {
                    out.push_str(" --- |");
                }
                out.push('\n');
            }
            Some("table-row") => {
                render_row(a, row, out);
            }
            _ => {}
        }
    }
}

/// Append `text` as inline markdown, backslash-escaping the ASCII-punctuation metacharacters that
/// would otherwise re-parse as structure (emphasis `*`/`_`, code `` ` ``, links `[`/`]`, escapes `\`,
/// images/autolinks `<`/`>`, strikethrough/gfm `~`). A `(text …)` node is LITERAL text, so it must
/// re-read verbatim — without this, a description like `Ast.*` or `make-<name>` (both present in the
/// corpus) would round-trip to a different tree. Escaping the closed punctuation set is always safe
/// (CommonMark drops a backslash before any ASCII punctuation), and the reader strips it, so the tree
/// is preserved.
fn escape_text_into(text: &str, out: &mut String) {
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '#' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
}

/// Print a sequence of inline nodes.
fn print_inlines(a: &Arenas, inlines: &[StructId], out: &mut String) {
    for &node in inlines {
        print_inline(a, node, out);
    }
}

/// Print a `link` / `image` inline as `<prefix>text](dest[ "title"])`. The two forms differ only in the
/// opening `prefix` (`[` for a link, `![` for an image); destination (item 1), title (item 2), and the
/// inline text (items 3..) are read identically.
fn print_link_like(a: &Arenas, id: StructId, prefix: &str, out: &mut String) {
    let items = list_items(a, id);
    let dest = items
        .get(1)
        .and_then(|&s| str_leaf(a, s))
        .unwrap_or_default();
    let title = items
        .get(2)
        .and_then(|&s| str_leaf(a, s))
        .unwrap_or_default();
    out.push_str(prefix);
    print_inlines(a, &items[3.min(items.len())..], out);
    out.push_str("](");
    out.push_str(&dest);
    if !title.is_empty() {
        out.push_str(" \"");
        out.push_str(&title);
        out.push('"');
    }
    out.push(')');
}

fn print_inline(a: &Arenas, id: StructId, out: &mut String) {
    match a.head_name(id) {
        Some("text") => {
            if let Some(t) = list_items(a, id).get(1).and_then(|&s| str_leaf(a, s)) {
                escape_text_into(&t, out);
            }
        }
        Some("code") => {
            if let Some(t) = list_items(a, id).get(1).and_then(|&s| str_leaf(a, s)) {
                out.push_str(&render_inline_code(&t));
            }
        }
        Some("emph") => {
            out.push('*');
            print_inlines(a, &child_tail(a, id), out);
            out.push('*');
        }
        Some("strong") => {
            out.push_str("**");
            print_inlines(a, &child_tail(a, id), out);
            out.push_str("**");
        }
        Some("strike") => {
            out.push_str("~~");
            print_inlines(a, &child_tail(a, id), out);
            out.push_str("~~");
        }
        Some("link") => print_link_like(a, id, "[", out),
        Some("image") => print_link_like(a, id, "![", out),
        Some("html") => {
            if let Some(t) = list_items(a, id).get(1).and_then(|&s| str_leaf(a, s)) {
                out.push_str(&t);
            }
        }
        Some("soft-break") => out.push('\n'),
        Some("hard-break") => out.push_str("\\\n"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core surface contract: parse → print → parse is a fixed point (arena-idempotent), the same
    /// guarantee the ML surface holds. CommonMark is not injective, so byte identity of the source is
    /// NOT required — but the TREE is stable.
    fn assert_idempotent(md: &str) {
        let a1 = read(md);
        let printed = print(&a1, 100);
        let a2 = read(&printed);
        assert!(
            a1.structurally_eq(&a2),
            "not arena-idempotent\n--- source ---\n{md}\n--- reprinted ---\n{printed}"
        );
    }

    #[test]
    fn headings_and_paragraphs() {
        assert_idempotent("# Title\n\nA paragraph of prose.\n\n## Section\n\nMore text.\n");
    }

    #[test]
    fn inline_styles() {
        assert_idempotent("Some *emph* and **strong** and `code` and ~~strike~~ here.\n");
    }

    #[test]
    fn links_and_images() {
        assert_idempotent("See [the site](https://example.com \"Title\") and ![alt](img.png).\n");
    }

    #[test]
    fn lists_ordered_and_unordered() {
        assert_idempotent("- one\n- two\n- three\n");
        assert_idempotent("1. first\n2. second\n3. third\n");
    }

    #[test]
    fn block_quote_and_rule() {
        assert_idempotent("> a quoted line\n> and another\n\n---\n\nafter the break\n");
    }

    #[test]
    fn gfm_table() {
        assert_idempotent("| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n");
    }

    #[test]
    fn plain_code_block_is_raw_only() {
        let md = "```python\nprint('hi')\n```\n";
        let a = read(md);
        // Find the code-block node; it must be raw-only (info + raw, no subtree).
        let cb = (0..a.structure.len() as u32)
            .map(StructId)
            .find(|&id| a.head_name(id) == Some("code-block"))
            .expect("a code-block node");
        let items = list_items(&a, cb);
        assert_eq!(
            items.len(),
            3,
            "plain code block: head + info + raw, no subtree"
        );
        assert_eq!(str_leaf(&a, items[1]).as_deref(), Some("python"));
        assert_eq!(str_leaf(&a, items[2]).as_deref(), Some("print('hi')"));
        assert_idempotent(md);
    }

    #[test]
    fn cdz_code_block_embeds_program_subtree() {
        // A ```cdz fence carries its program as a real arena subtree; that subtree must be the SAME
        // tree `read_ml` produces for the fence body.
        let md = "```cdz\n2 + 3\n```\n";
        let a = read(md);
        let cb = (0..a.structure.len() as u32)
            .map(StructId)
            .find(|&id| a.head_name(id) == Some("code-block"))
            .expect("a code-block node");
        let items = list_items(&a, cb);
        assert_eq!(
            items.len(),
            4,
            "cdz code block: head + info + raw + subtree"
        );
        // The embedded subtree equals read_ml("2 + 3")'s tree → `(+ 2 3)`.
        let sub = items[3];
        let expected = crate::parser::read_ml("2 + 3").arenas;
        // Compare structurally by lifting the subtree into its own arena.
        let mut b = Builder::new();
        let root = super::Md::new(&mut b, None).clone_subtree(&a, sub, Span::new(0, 0));
        let lifted = b.finish(root);
        assert!(
            lifted.structurally_eq(&expected),
            "embedded subtree should equal read_ml body"
        );
        assert_idempotent(md);
    }

    /// A random valid s-expr program body (bounded by `depth`) to embed in a fence — atoms, infix,
    /// call, `let`, `if`, nested. No compound-ctor spellings (they'd need surface-specific handling);
    /// the point is exercising the embed→standalone-parse agreement over the ordinary program space.
    fn gen_fence_body(rng: &mut Rng, depth: usize) -> String {
        let atoms = ["a", "b", "x", "y", "f", "g", "1", "42", "true"];
        if depth == 0 || rng.below(3) == 0 {
            return atoms[rng.below(atoms.len())].to_string();
        }
        let sub = |rng: &mut Rng| gen_fence_body(rng, depth - 1);
        match rng.below(5) {
            0 => format!("(+ {} {})", sub(rng), sub(rng)),
            1 => format!("(f {} {})", sub(rng), sub(rng)),
            2 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
            3 => format!("(let ((x {})) {})", sub(rng), sub(rng)),
            _ => format!("(g {} {} {})", sub(rng), sub(rng), sub(rng)),
        }
    }

    #[test]
    fn embedded_fence_subtree_equals_the_standalone_parse_over_generated_programs() {
        // The "code inside a doc IS arena" contract, swept: a ```sexp fence's embedded subtree must be
        // the SAME tree `sexpr::read` produces for the fence body standalone — over random programs, not
        // one hand case (`cdz_code_block_embeds_program_subtree`). Pins the CodeRole::Sexpr embed path
        // (`embed_program` → `clone_subtree`) against an independent standalone parse across the program
        // space; a clone-subtree or fence-body-extraction regression surfaces as a structural mismatch.
        let mut rng = Rng(0x00fe_4ce5_ab77_ee01);
        for _ in 0..2000 {
            let depth = 1 + rng.below(4);
            let body = gen_fence_body(&mut rng, depth);
            let md = format!("```sexp\n{body}\n```\n");
            let a = read(&md);
            let cb = (0..a.structure.len() as u32)
                .map(StructId)
                .find(|&id| a.head_name(id) == Some("code-block"))
                .expect("a code-block node");
            let items = list_items(&a, cb);
            assert_eq!(
                items.len(),
                4,
                "a clean sexp fence embeds a subtree (head + info + raw + subtree) for {body:?}"
            );
            // Lift the embedded subtree into its own arena and compare to the standalone parse.
            let mut b = Builder::new();
            let root = super::Md::new(&mut b, None).clone_subtree(&a, items[3], Span::new(0, 0));
            let lifted = b.finish(root);
            let standalone = crate::sexpr::read(&body).expect("body parses standalone");
            assert!(
                lifted.structurally_eq(&standalone),
                "embedded subtree != standalone parse for {body:?}"
            );
        }
    }

    #[test]
    fn clone_subtree_is_iterative_not_recursive_on_a_deep_source() {
        // `clone_subtree` (the embed-program deep-copy) is iterative. Build a deep source arena DIRECTLY
        // (bypassing the reader's MAX_NESTING_DEPTH cap) and clone it into a fresh Md builder — a
        // native-recursive clone would overflow the stack. `Arenas` is FLAT (no recursive drop), so this
        // needs no big-stack thread; only clone_subtree's own recursion was the concern, now an explicit
        // stack. Assert the clone completes, is structurally equal to the source, and — since span
        // tracking is ON — its span table stays 1:1 with the arena (one span pushed per emitted node, in
        // id order, so the table is TOTAL: `get(id)` is `Some` for every structure id and `len` matches).
        let depth = 60_000usize;
        let mut sb = Builder::new();
        let mut cur = sb.name("x");
        for _ in 0..depth {
            cur = sb.list(vec![cur]);
        }
        let src = sb.finish(cur);

        // Extract the span table before `b.finish` reclaims the builder (the `Md` borrows it mutably).
        let mut b = Builder::new();
        let (root, spans) = {
            let mut md = super::Md::new(&mut b, Some(SpanTable::new(FileId(0))));
            let root = md.clone_subtree(&src, src.root, Span::new(0, 0)); // must NOT overflow
            let spans = md.spans.take().expect("span tracking is on");
            (root, spans)
        };
        let cloned = b.finish(root);
        assert!(
            cloned.structurally_eq(&src),
            "deep clone preserves the tree"
        );
        // The span table is 1:1 with the cloned arena: exactly one entry per node...
        assert_eq!(
            spans.len(),
            cloned.structure.len(),
            "span table has one entry per cloned node"
        );
        // ...and TOTAL — every structure id resolves to a span (push order preserved, no gaps).
        assert!(
            (0..cloned.structure.len() as u32).all(|i| spans.get(StructId(i)).is_some()),
            "span table is total over every cloned node id"
        );
    }

    #[test]
    fn a_cdz_fence_with_a_malformed_body_falls_back_to_raw_only() {
        // Graceful degradation: a ```cdz fence whose body does NOT parse cleanly must NOT embed a
        // broken/error subtree (only a clean body earns the 4th subtree child) and must NOT fail the
        // whole document parse — the doc is data, so bad code in a fence is just kept as raw text (a
        // README with a work-in-progress snippet still reads as a document). The `raw` is preserved
        // verbatim so it round-trips byte-exact.
        let md = "```cdz\ndef f() =\n```\n"; // `def f() =` has no body → a parse error
        let a = read(md);
        let cb = (0..a.structure.len() as u32)
            .map(StructId)
            .find(|&id| a.head_name(id) == Some("code-block"))
            .expect("a code-block node");
        let items = list_items(&a, cb);
        assert_eq!(
            items.len(),
            3,
            "a malformed cdz fence is raw-only (head + info + raw, NO subtree)"
        );
        assert_eq!(str_leaf(&a, items[1]).as_deref(), Some("cdz"));
        assert_eq!(
            str_leaf(&a, items[2]).as_deref(),
            Some("def f() ="),
            "raw body kept verbatim"
        );
        // Arena-idempotent, and the raw body survives byte-exact through a print→read (fences are the
        // one part of a markdown doc that IS byte-exact — the printer emits the stored `raw`).
        assert_idempotent(md);
        assert!(
            print(&a, 100).contains("def f() ="),
            "malformed body printed verbatim"
        );
    }

    #[test]
    fn nested_list_and_tight_items() {
        // A tight list (inline text directly under `item`) and a nested list must both round-trip.
        assert_idempotent("- one\n- two\n\n  nested paragraph\n- three\n");
        assert_idempotent("1. alpha\n2. beta\n");
    }

    #[test]
    fn tight_item_with_a_nested_list_is_arena_idempotent() {
        // The COMBINED case the faces above cover only separately: a TIGHT item (`(item (text …) …)`,
        // no wrapping paragraph) whose content is immediately followed by a NESTED list. The printer used
        // to emit a blank-line separator between the item's inline text and the sublist
        // (`- item\n  \n  - nested\n`); CommonMark reads that blank line as making the item LOOSE, wrapping
        // "item" in a paragraph, so the re-read tree differed — an arena-idempotency violation (it only
        // reconverged on the 2nd round). print_blocks now suppresses the separator on the inline-run → list
        // transition so the sublist hugs the tight text.
        assert_idempotent("- item\n  - nested\n");
        // Ordered outer + nested, and a two-level nest, exercise the same hug at deeper indents.
        assert_idempotent("1. item\n   - nested\n");
        assert_idempotent("- a\n  - b\n    - c\n");
        // A tight item with text, a nested list, THEN more tight siblings still round-trips.
        assert_idempotent("- one\n  - inner\n- two\n");
    }

    #[test]
    fn loose_lists_stay_loose_and_tight_lists_stay_tight() {
        // A LOOSE list — a blank line between items, so pulldown-cmark wraps each item's content in a
        // `(paragraph …)` — must reprint WITH the blank lines, else it reads back TIGHT (bare inline under
        // `item`), a different tree. The printer used to never separate items, so every loose list
        // collapsed to tight on reprint (`- a\n\n- b\n` → `- a\n- b\n`), an arena-idempotency violation.
        // `print_list` now detects looseness (any item holds a paragraph) and emits the item separators.
        assert_idempotent("- a\n\n- b\n"); // loose unordered
        assert_idempotent("1. a\n\n2. b\n"); // loose ordered
        assert_idempotent("- one\n\n- two\n\n  extra paragraph\n- three\n"); // loose w/ a multi-block item
        // And a tight list must NOT gain blank lines (stays tight on reprint).
        assert_idempotent("- a\n- b\n");
        assert_idempotent("1. a\n2. b\n");
        // A tight item carrying a nested list is still tight (no spurious item separators either).
        assert_idempotent("- x\n  - sub\n- y\n");
    }

    #[test]
    fn code_block_inside_a_list_item_round_trips() {
        // A fenced code block INSIDE a list item: the span pulldown-cmark gives starts at the opening
        // fence, so its continuation lines (body + closing fence) still carry the list-container indent.
        // fence_body used to store that indent in `raw` (`  code`), and the printer then re-indented it
        // under the list (`    code`), so the re-read saw deeper-indented code — a different tree. Now the
        // container indent (read off the closing fence) is stripped, so `raw` holds the code as it reads
        // after the container strips its indent. The code's OWN deeper indentation is preserved.
        assert_idempotent("- a\n\n  ```\n  code\n  ```\n- b\n");
        assert_idempotent("- x\n\n  ```py\n  a = 1\n    b = 2\n  ```\n"); // internal indent kept
        assert_idempotent("- item\n\n  ```\n  line1\n\n  line2\n  ```\n"); // blank line inside body
        // A top-level fence (no container indent) is unaffected.
        assert_idempotent("```\ncode\n```\n");
        assert_idempotent("```cdz\nlet x = 1 in x\n```\n");
    }

    #[test]
    fn mixed_document_round_trips() {
        // A document mixing every block kind + an embedded cdz fence is arena-idempotent.
        let md = "# Title\n\nIntro *prose* with a [link](http://x).\n\n## Section\n\n\
                  - a\n- b\n\n> quoted\n\n```cdz\nlet x = 1 in x\n```\n\n\
                  | h1 | h2 |\n| --- | --- |\n| 1 | 2 |\n\n---\n\nEnd.\n";
        assert_idempotent(md);
    }

    #[test]
    fn literal_text_metacharacters_round_trip() {
        // Prose containing markdown metacharacters (the corpus has `Ast.*` and `make-<name>` in case
        // descriptions) is LITERAL — it must re-read verbatim, not as emphasis / a tag / a link.
        assert_idempotent("A quote pattern equals the Ast.* constructor pattern.\n");
        assert_idempotent("Normalizes each make-<name> export.\n");
        assert_idempotent("Underscores a_b_c and stars a*b*c and brackets a[b]c stay literal.\n");
        // The escaped text really reads back as the ORIGINAL string, not a styled tree.
        let a = read("stars a*b*c here\n");
        let printed = print(&a, 100);
        let a2 = read(&printed);
        assert!(a.structurally_eq(&a2));
    }

    #[test]
    fn inline_code_with_backticks_round_trips() {
        // An inline-code span whose body CONTAINS backticks needs a longer delimiter (CommonMark rule);
        // the spec README's `(+ ,a ,b)` metaprogramming example exercises exactly this.
        assert_idempotent("Consider the quote ``(+ ,a ,b)`` in a pattern.\n");
        assert_idempotent("A bare `x + 1` span and a `` `tick` `` span together.\n");
        // The helper's fencing rule directly.
        assert_eq!(render_inline_code("x + 1"), "`x + 1`");
        assert_eq!(render_inline_code("has ` tick"), "``has ` tick``");
        assert_eq!(render_inline_code("`leading"), "`` `leading ``");
    }

    #[test]
    fn real_spec_readme_is_arena_idempotent() {
        // The strongest real-world check: the actual hand-written spec/semantics/README.md (headings,
        // prose, links, code fences, tables) parses → prints → parses to the same arena.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../spec/semantics/README.md");
        let Ok(src) = std::fs::read_to_string(&path) else {
            // The file may be absent in some checkouts; the inline tests cover the surface.
            return;
        };
        assert_idempotent(&src);
    }

    #[test]
    fn non_document_root_falls_back_to_cdz_fence() {
        // `cdz convert prog.cdz --to md` hands a bare program arena to print; it wraps as a cdz fence.
        let prog = crate::sexpr::read("(+ 1 2)").unwrap();
        let md = print(&prog, 100);
        assert!(md.contains("```cdz"), "fallback fence:\n{md}");
        // And that fenced doc re-reads with the program embedded as a subtree.
        let doc = read(&md);
        let has_codeblock = (0..doc.structure.len() as u32)
            .map(StructId)
            .any(|id| doc.head_name(id) == Some("code-block"));
        assert!(has_codeblock);
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Generate a random CommonMark document from a grammar of blocks — headings, paragraphs (with
    /// inline emph/strong/code/strike), bullet + ordered lists, blockquotes, fenced code, GFM tables,
    /// thematic breaks — separated by blank lines. CommonMark is NOT injective (it normalizes marker
    /// styles, whitespace, …), so the contract is arena-IDEMPOTENCE, not byte-exactness: the generator
    /// need not emit already-canonical markdown, only well-formed blocks the reader accepts.
    fn gen_md(rng: &mut Rng) -> String {
        let words = ["alpha", "beta", "gamma", "text", "a word", "more"];
        let word = |rng: &mut Rng| words[rng.below(words.len())].to_string();
        // A line of inline content mixing plain text and the inline styles.
        let inline = |rng: &mut Rng| -> String {
            let mut s = String::new();
            for i in 0..(1 + rng.below(4)) {
                if i > 0 {
                    s.push(' ');
                }
                match rng.below(6) {
                    0 => s.push_str(&format!("*{}*", word(rng))),
                    1 => s.push_str(&format!("**{}**", word(rng))),
                    2 => s.push_str(&format!("`{}`", word(rng))),
                    3 => s.push_str(&format!("~~{}~~", word(rng))),
                    _ => s.push_str(&word(rng)),
                }
            }
            s
        };
        let mut out = String::new();
        // Track the previous block kind: CommonMark MERGES two adjacent same-marker lists across a
        // blank line (and normalizes loose/tight), which is a legitimate non-injectivity — the printer
        // emits no separator to keep them distinct, so `read(print(read))` differs from `read` for that
        // shape. That's a known markdown-surface round-trip GAP (filed as a backlog note), not what this
        // idempotence sweep tests — so the generator never emits a list directly after a list.
        let mut prev_was_list = false;
        for _ in 0..(1 + rng.below(6)) {
            let mut choice = rng.below(8);
            if prev_was_list && (choice == 2 || choice == 3) {
                choice = 1; // demote a list-after-list to a paragraph (avoid the adjacent-list merge)
            }
            prev_was_list = choice == 2 || choice == 3;
            match choice {
                0 => out.push_str(&format!(
                    "{} {}\n",
                    "#".repeat(1 + rng.below(6)),
                    inline(rng)
                )),
                1 => out.push_str(&format!("{}\n", inline(rng))), // paragraph
                2 => {
                    // bullet list
                    for _ in 0..(1 + rng.below(3)) {
                        out.push_str(&format!("- {}\n", inline(rng)));
                    }
                }
                3 => {
                    // ordered list
                    for i in 0..(1 + rng.below(3)) {
                        out.push_str(&format!("{}. {}\n", i + 1, inline(rng)));
                    }
                }
                4 => out.push_str(&format!("> {}\n", inline(rng))), // blockquote
                5 => out.push_str(&format!("```\n{}\n{}\n```\n", word(rng), word(rng))), // fenced code
                6 => {
                    // GFM table (2 columns, 1..=2 body rows)
                    out.push_str("| A | B |\n| --- | --- |\n");
                    for _ in 0..(1 + rng.below(2)) {
                        out.push_str(&format!("| {} | {} |\n", word(rng), word(rng)));
                    }
                }
                _ => out.push_str("---\n"), // thematic break
            }
            out.push('\n'); // blank line between blocks
        }
        out
    }

    #[test]
    fn markdown_surface_is_idempotent_over_generated_documents() {
        // The surface contract (arena-idempotence: read(print(read(md))) == read(md)) swept over random
        // CommonMark, complementing the hand-picked cases. A generator explores block + inline-style
        // COMBINATIONS and nestings the fixed tests don't, so a printer/parser asymmetry no hand-written
        // case hits is caught. Fixed seeds → reproducible; a failure prints source + reprint.
        let seeds: [u64; 3] = [
            0x0bad_c0de_dead_beef,
            0x5eed_1234_5678_9abc,
            0xfeed_face_cafe_babe,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..800 {
                assert_idempotent(&gen_md(&mut rng));
                total += 1;
            }
        }
        assert!(total >= 2000, "swept a meaningful space, got {total}");
    }

    #[test]
    fn markdown_survives_the_binary_codec_over_generated_and_embedded_program_documents() {
        // json/toml/cedar each pin a to-binary round-trip; markdown did NOT — yet it is the riskiest
        // surface for the codec because an embedded ```cdz/```ml/```sexp fence body is a REAL arena
        // subtree (not opaque text), so a codec defect around nested program trees would surface here
        // first. Assert: read(md) → codec::encode → codec::decode reproduces a structurally-equal arena,
        // AND printing the decoded arena re-reads to the same tree (the full paper-trail json_to_binary
        // checks). Swept over the generator (plain markdown) plus explicit embedded-program fences the
        // generator doesn't emit.
        fn assert_binary_round_trip(md: &str) {
            let a1 = read(md); // infallible
            let bin = crate::codec::encode(&a1);
            let a2 = crate::codec::decode(&bin)
                .unwrap_or_else(|| panic!("decode returned None for {md:?}"));
            assert!(
                a1.structurally_eq(&a2),
                "arena not preserved through the binary codec for {md:?}"
            );
            // Printing the decoded arena re-reads to the same tree.
            let printed = print(&a2, 100);
            let a3 = read(&printed);
            assert!(
                a1.structurally_eq(&a3),
                "decoded→print→read diverged for {md:?}\n--- reprint ---\n{printed}"
            );
        }
        // Generated plain-markdown documents.
        let mut rng = Rng(0xb1a5_5eed_face_ce01);
        for _ in 0..2000 {
            assert_binary_round_trip(&gen_md(&mut rng));
        }
        // Embedded-program fences (cdz/ml/sexp) — the fence body is a nested arena subtree, the part the
        // generator never exercises and the part most likely to trip the codec.
        for md in [
            "```cdz\nlet x = 1 in x\n```\n",
            "text\n\n```ml\ndef f() = 42\n```\n\nmore\n",
            "```sexp\n(def (main) (+ 1 2))\n```\n",
            "# H\n\n```cdz\nmatch xs with\n| [] -> 0\n| x :: _ -> x\n```\n\n> after\n",
            "```cdz\n(\"list\" 1 2 3)\n```\n",
        ] {
            assert_binary_round_trip(md);
        }
    }

    /// `read`/`read_spanned` are INFALLIBLE (CommonMark always parses to SOME document), so there is no
    /// error path to catch a defect — the invariant is that they never PANIC and always produce a
    /// well-formed document with a TOTAL span table: a non-empty arena, root id in range, `spans` exactly
    /// 1:1 with the structure vector, and every reachable child id in range (fully traversable). A broken
    /// span table would silently corrupt a span-based structural edit over a markdown document, with no
    /// error to signal it — so assert it on arbitrary input, not just the fixed valid cases.
    fn assert_markdown_read_invariants(src: &str) {
        let plain = read(src); // must not panic
        let (a, spans) = read_spanned(src); // must not panic
        // The two entry points agree structurally (the spanned read is the plain read + a table).
        assert!(
            plain.structurally_eq(&a),
            "read and read_spanned disagree for {src:?}"
        );
        let n = a.structure.len();
        assert!(n > 0, "a document arena is never empty for {src:?}");
        assert!((a.root.0 as usize) < n, "root id in range for {src:?}");
        assert_eq!(
            spans.len(),
            n,
            "span table is total (1:1 with structure) for {src:?}"
        );
        // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, on UTF-8 char
        // boundaries. Totality only says a span EXISTS per node; this says `&src[sp.start..sp.end]` (an
        // LSP hover / diagnostic underline / span-based structural edit over the document) can be taken
        // WITHOUT panicking. Markdown synthesizes spans for container nodes AND for a code-block's embedded
        // program subtree (a best-effort span covering the whole `(code-block …)`, in the DOCUMENT's
        // coordinate system) — so a span escaping the source or landing off a char boundary is a real risk
        // on multibyte / fenced input. Completes the span-geometry sweep across all read_spanned surfaces.
        for id in (0..n as u32).map(StructId) {
            let sp = spans.get(id).expect("total span table");
            assert!(
                sp.start <= sp.end
                    && sp.end <= src.len()
                    && src.is_char_boundary(sp.start)
                    && src.is_char_boundary(sp.end),
                "span {sp:?} for node {id:?} is not a valid slice of {src:?}"
            );
        }
        fn walk(a: &Arenas, id: StructId) {
            if let crate::ast::Struct::List(kids) = a.get(id) {
                for &c in kids {
                    assert!(
                        (c.0 as usize) < a.structure.len(),
                        "child id {} in range",
                        c.0
                    );
                    walk(a, c);
                }
            }
        }
        walk(&a, a.root);
    }

    #[test]
    fn markdown_read_never_panics_on_arbitrary_input() {
        // Sweep random markdown-ish strings (structural chars + text + unicode) plus odd fragments; each
        // must produce a well-formed document with a total span table (see `assert_markdown_read_invariants`).
        let alphabet: Vec<char> = "#*_`~->|[]()!\n \t.0123456789abcλ".chars().collect();
        let mut rng = Rng(0x9abc_5678_1234_5eed);
        for len in 0..=40usize {
            for _ in 0..60 {
                let s: String = (0..len)
                    .map(|_| alphabet[rng.below(alphabet.len())])
                    .collect();
                assert_markdown_read_invariants(&s);
            }
        }
        for s in [
            "#", "```", "> ", "- [", "| a |", "![", "[x](", "~~~", "\t\t",
        ] {
            assert_markdown_read_invariants(s);
        }
        // Adversarial `cdz`/`ml`/`sexp` fences — the embedded-program path is the riskiest for span
        // alignment (the fence body is a real arena subtree whose spans are relative to the body): a
        // truncated, empty, or malformed-program fence must still keep the outer table total.
        for s in [
            "```cdz\n",
            "```cdz\n(def",
            "```cdz\n\n```\n",
            "```ml\ndef f() =\n```",
            "```sexp\n((((\n```",
            "```cdz\n)))\n```\ntext after\n",
            "~~~cdz\n@\n~~~",
        ] {
            assert_markdown_read_invariants(s);
        }
    }
}
