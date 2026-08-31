//! The **TOML surface** — a source-faithful config document as a projection of the one canonical arena.
//!
//! TOML is a first-class front-end syntax, exactly like the s-expression, ML, markdown, and JSON
//! surfaces: a parser (`read`) turns TOML text into the shared [`Arenas`], and a printer (`print`)
//! turns a TOML arena back into TOML text. It is not privileged (`spec/contracts/ast-encoding.md` §A
//! Textual Syntax Parses To And Prints From The Canonical Form) — a `.toml` reads to the same binary
//! AST any surface does, so `cdz convert config.toml --to binary` yields a canonical arena. (Note:
//! `--to json` does NOT translate TOML data into JSON data — each surface prints only its OWN node
//! vocabulary, so a TOML document handed to the JSON printer takes the JSON fallback. Cross-FORMAT
//! data conversion between two faithful-but-distinct vocabularies would be a separate transform.)
//!
//! Like a markdown/JSON document, a TOML value is *data*, not a program: its nodes are TOML structure
//! (`toml-document`/`toml-table`/`toml-kv`/scalars), not language constructs, so the compiler never
//! sees one. The surface is **source-faithful via the decor-in-arena model**: comments, blank lines,
//! whitespace, and each scalar's RAW spelling are all stored as `Str`-leaf nodes IN the arena, so the
//! arena stays the single, rewritable representation — a rewrite of any node reflects in the printed
//! output. Every byte of "decor" (toml_edit's word for the inter-token whitespace + comments) lives in
//! a node.
//!
//! ## Parser + printer
//!
//! We use the `toml_edit` crate — the format-preserving TOML library (the one Cargo uses to edit
//! `Cargo.toml`) — as a REFERENCE-only convenience, exactly like `pulldown-cmark` for markdown; the
//! eventual Cadenza rewrite hand-rolls it. The READER parses to a `toml_edit::DocumentMut` (which
//! *despans* on parse, so every `display_repr()`/decor `as_str()` returns exact source text) and walks
//! it into arena nodes. The PRINTER does the inverse: it **reconstructs a `DocumentMut`** from the
//! arena nodes (restoring each scalar's raw repr by re-parsing it, and replaying every decor string and
//! table position) and calls its `Display` — so byte-exact reconstruction (headers, document order,
//! synthesized newlines) is done by toml_edit's own proven encoder rather than reimplemented here.
//!
//! ## Numbers, datetimes, inf/nan
//!
//! Each scalar stores its RAW source text, so the printed bytes never come from the parsed `i64`/`f64`
//! — `0xDEADBEEF`, `1_000`, `9_223_372_036_854_775_807`, `inf`, `nan`, and RFC-3339 datetimes all
//! survive verbatim regardless of what the parsed value rounded to. A datetime and a non-finite float
//! have no arena `Leaf` (the `Float` leaf is finite-only by language decision), so they are raw-text
//! nodes only; this needs zero new leaf types and zero codec change.
//!
//! ## Round-trip
//!
//! The guarantee is **byte-exact** for an unmutated document — `print(read(src)) == src` — stronger
//! than the arena-idempotence the markdown/JSON surfaces hold. After any node rewrite it degrades to
//! arena-idempotence (`read(print(x))` is a fixed point).

use cadenza_syntax_core::arena_read::{bool_leaf, child_tail, int_leaf, list_items, str_leaf};
use cadenza_syntax_core::ast::{Arenas, Builder, Leaf, Radix, StructId};
use cadenza_syntax_core::span::Span;
use cadenza_syntax_core::spans::{FileId, SpanTable};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Key, Table, Value};

/// A TOML parse failure, with a human-readable message (mirrors `sexpr::ReadError`). Where a position
/// is known it ends in `at byte N`, so a caller holding the source can map it to `line:col`.
#[derive(Debug)]
pub struct ReadError(pub String);

/// Parse TOML `src` into a decor-in-arena value document, or a [`ReadError`] on malformed input.
/// Fallible (unlike CommonMark, TOML can fail) — a bad document is a clean error, never patched up.
pub fn read(src: &str) -> Result<Arenas, ReadError> {
    let doc = parse_doc(src)?;
    let mut b = Builder::new();
    let root = Toml::new(&mut b, None).document(&doc);
    Ok(b.finish(root))
}

/// Parse TOML `src` into a value document, ALSO producing a [`SpanTable`] 1:1 with the arena. Because
/// a `DocumentMut` is despanned on parse, per-node source ranges are not available; every node gets a
/// best-effort `Span::new(0, 0)` placeholder (matching how the markdown surface spans synthesized
/// subtree nodes) — the table stays total and in id order, which is what the query/rewrite path needs
/// (a `.toml` is data, never handed to the compiler).
pub fn read_spanned(src: &str) -> Result<(Arenas, SpanTable), ReadError> {
    let doc = parse_doc(src)?;
    let mut b = Builder::new();
    let mut t = Toml::new(&mut b, Some(SpanTable::new(FileId::default())));
    let root = t.document(&doc);
    let spans = t.spans.take().expect("span tracking on");
    Ok((b.finish(root), spans))
}

fn parse_doc(src: &str) -> Result<DocumentMut, ReadError> {
    src.parse::<DocumentMut>().map_err(|e| {
        // toml_edit gives a byte-range for the error; surface `at byte N` so the caller (convert.rs)
        // can remap it to `line:col` via `locate_byte_in_message`.
        match e.span() {
            Some(range) => ReadError(format!("{} at byte {}", err_head(&e), range.start)),
            None => ReadError(err_head(&e)),
        }
    })
}

/// The first line of a toml_edit error message (its Display is multi-line with a source excerpt; we
/// keep the headline and append our own `at byte N`).
fn err_head(e: &toml_edit::TomlError) -> String {
    let msg = e.message();
    if msg.trim().is_empty() {
        // toml_edit emits an EMPTY message when its VALUE parser reaches end-of-input or only trailing
        // whitespace after `key =` (a missing value: `a = `, `a =`, `x = \t`) — every OTHER malformation
        // (invalid key/table/string/array/datetime) carries a specific headline. Without this, the lifted
        // `toml{ … }` region error was a bare `at byte N` with no cause (double space, no reason) — found
        // by v-parser-corpus. Substitute the accurate cause for the whole empty-message class.
        return "expected a value".to_string();
    }
    msg.to_string()
}

// ============================================================================
// Reader: toml_edit DocumentMut -> decor-in-arena
// ============================================================================

/// The document walker. Recurses `DocumentMut` → arena via `Builder`, mirroring the `mk_*`/`push_span`
/// discipline of the sexpr/json/markdown readers (one span pushed per created `StructId`, in id order).
/// Manual recursion (not the `Visit` trait) because we need the key path + table position + dotted
/// structure that toml_edit's own `visit_nested_tables` traversal uses.
struct Toml<'b> {
    b: &'b mut Builder,
    spans: Option<SpanTable>,
}

impl<'b> Toml<'b> {
    fn new(b: &'b mut Builder, spans: Option<SpanTable>) -> Toml<'b> {
        Toml { b, spans }
    }

    /// Build `(toml-document <entry>… <trailing:Str>)`.
    fn document(&mut self, doc: &DocumentMut) -> StructId {
        let head = self.mk_name("toml-document");
        let mut items = vec![head];
        self.table_entries(doc.as_table(), &mut items);
        // The document's final whitespace/comments.
        let trailing = doc.trailing().as_str().unwrap_or("").to_string();
        items.push(self.mk_str(trailing));
        self.mk_list(items)
    }

    /// Append a table's entries to `items`, mirroring toml_edit's own encoder split:
    /// - `get_values()` yields the directly-visible key-values, flattening a DOTTED key (`a.b.c = 1`
    ///   is stored as nested `is_dotted()` tables) into one `(toml-kv <multi-segment key-path> …)`;
    /// - `iter()` yields the sub-tables — a NON-dotted `Item::Table` is a real `[header]`
    ///   (`toml-table`), an `ArrayOfTables` is `[[header]]` (`toml-array-table`); a dotted table and a
    ///   direct value are already covered by `get_values()`, so they are skipped here.
    ///
    /// Document order need not be preserved in the arena: the printer rebuilds a `DocumentMut` and its
    /// `Display` re-derives order (root kvs first, then tables sorted by stored `position()`).
    fn table_entries(&mut self, table: &Table, items: &mut Vec<StructId>) {
        for (path, value) in table.get_values() {
            let key_node = self.key_path_multi(&path);
            let kv = self.kv(key_node, value);
            items.push(kv);
        }
        for (key_str, item) in table.iter() {
            let key = table
                .key(key_str)
                .cloned()
                .unwrap_or_else(|| Key::new(key_str));
            match item {
                Item::Table(sub) if !sub.is_dotted() => {
                    let node = self.std_table(&key, sub);
                    items.push(node);
                }
                Item::ArrayOfTables(aot) => {
                    for sub in aot.iter() {
                        let node = self.array_table(&key, sub);
                        items.push(node);
                    }
                }
                // A direct value and a dotted intermediate table are handled by `get_values()`.
                _ => {}
            }
        }
    }

    /// `(toml-table <prefix:Str> <pos:Int> <implicit:Bool> <header:key-path> <suffix:Str> <entry>…)`.
    fn std_table(&mut self, key: &Key, table: &Table) -> StructId {
        let head = self.mk_name("toml-table");
        let (pre, suf) = decor_strs(table.decor());
        let prefix = self.mk_str(pre);
        let pos = self.mk_int(table.position().unwrap_or(0) as i64);
        let implicit = self.mk_bool(table.is_implicit());
        let header = self.key_path(key);
        let suffix = self.mk_str(suf);
        let mut items = vec![head, prefix, pos, implicit, header, suffix];
        self.table_entries(table, &mut items);
        self.mk_list(items)
    }

    /// `(toml-array-table <prefix:Str> <pos:Int> <header:key-path> <suffix:Str> <entry>…)` — one node
    /// per `[[key]]` element (the caller loops the array).
    fn array_table(&mut self, key: &Key, table: &Table) -> StructId {
        let head = self.mk_name("toml-array-table");
        let (pre, suf) = decor_strs(table.decor());
        let prefix = self.mk_str(pre);
        let pos = self.mk_int(table.position().unwrap_or(0) as i64);
        let header = self.key_path(key);
        let suffix = self.mk_str(suf);
        let mut items = vec![head, prefix, pos, header, suffix];
        self.table_entries(table, &mut items);
        self.mk_list(items)
    }

    /// `(toml-kv <key-path> <value>)` — the key path carries its own decor; the value carries its
    /// leading/trailing decor. A dotted leaf key (`a.b.c = 1`) becomes a multi-segment key path.
    fn kv(&mut self, key_path: StructId, value: &Value) -> StructId {
        let head = self.mk_name("toml-kv");
        let v = self.value(value);
        self.mk_list(vec![head, key_path, v])
    }

    /// A single-segment key path (a `[header]` key or a plain leaf key).
    fn key_path(&mut self, key: &Key) -> StructId {
        let head = self.mk_name("toml-key-path");
        let seg = self.key_segment(key);
        self.mk_list(vec![head, seg])
    }

    /// A multi-segment key path — the flattened form of a dotted key `a.b.c` (from `get_values`), each
    /// segment carrying its own raw spelling + dotted decor.
    fn key_path_multi(&mut self, keys: &[&Key]) -> StructId {
        let head = self.mk_name("toml-key-path");
        let mut items = vec![head];
        for k in keys {
            let seg = self.key_segment(k);
            items.push(seg);
        }
        self.mk_list(items)
    }

    /// `(toml-key <raw:Str> <leaf-pre:Str> <leaf-suf:Str> <dot-pre:Str> <dot-suf:Str>)` — the key's raw
    /// spelling (quotes preserved) plus its leaf decor (around the key in a kv) and dotted decor
    /// (around the dots in a dotted key).
    fn key_segment(&mut self, key: &Key) -> StructId {
        let head = self.mk_name("toml-key");
        let raw = self.mk_str(key.display_repr().into_owned());
        let (lpre, lsuf) = decor_strs(key.leaf_decor());
        let (dpre, dsuf) = decor_strs(key.dotted_decor());
        let leaf_pre = self.mk_str(lpre);
        let leaf_suf = self.mk_str(lsuf);
        let dot_pre = self.mk_str(dpre);
        let dot_suf = self.mk_str(dsuf);
        self.mk_list(vec![head, raw, leaf_pre, leaf_suf, dot_pre, dot_suf])
    }

    /// Build a value node. Scalars store `(toml-<kind> <prefix:Str> <suffix:Str> <raw:Str> [<value>])`;
    /// composites recurse.
    fn value(&mut self, v: &Value) -> StructId {
        let (pre, suf) = decor_strs(v.decor());
        match v {
            Value::String(f) => {
                // A string always carries its decoded logical value as a convenience leaf.
                let val = Leaf::Str(f.value().clone().into());
                self.scalar_with("toml-string", &pre, &suf, &f.display_repr(), Some(val))
            }
            Value::Integer(f) => {
                let val = Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(*f.value()),
                    radix: Radix::Dec,
                };
                self.scalar_with("toml-integer", &pre, &suf, &f.display_repr(), Some(val))
            }
            Value::Float(f) => {
                // A non-finite float (inf/nan) has no `Float` leaf — raw-text only.
                let raw = f.display_repr();
                let val = cadenza_syntax_core::literal::parse_float(&raw).map(Leaf::Float);
                self.scalar_with("toml-float", &pre, &suf, &raw, val)
            }
            Value::Boolean(f) => {
                let val = Leaf::Bool(*f.value());
                self.scalar_with("toml-boolean", &pre, &suf, &f.display_repr(), Some(val))
            }
            Value::Datetime(f) => {
                // No datetime leaf — the raw RFC-3339 text is the sole faithful form.
                self.scalar_with("toml-datetime", &pre, &suf, &f.display_repr(), None)
            }
            Value::Array(a) => self.array(a, &pre, &suf),
            Value::InlineTable(t) => self.inline_table(t, &pre, &suf),
        }
    }

    /// `(toml-<kind> <prefix:Str> <suffix:Str> <raw:Str> [<value-leaf>])`.
    fn scalar_with(
        &mut self,
        head: &str,
        pre: &str,
        suf: &str,
        raw: &str,
        value: Option<Leaf>,
    ) -> StructId {
        let h = self.mk_name(head);
        let prefix = self.mk_str(pre.to_string());
        let suffix = self.mk_str(suf.to_string());
        let raw_leaf = self.mk_str(raw.to_string());
        let mut items = vec![h, prefix, suffix, raw_leaf];
        if let Some(leaf) = value {
            let v = self.mk_atom_leaf(leaf);
            items.push(v);
        }
        self.mk_list(items)
    }

    /// `(toml-array <prefix:Str> <suffix:Str> <trailing:Str> <trailing-comma:Bool> <elem>…)` where each
    /// `elem` is `(toml-elem <prefix:Str> <suffix:Str> <value>)`.
    fn array(&mut self, a: &Array, pre: &str, suf: &str) -> StructId {
        let head = self.mk_name("toml-array");
        let prefix = self.mk_str(pre.to_string());
        let suffix = self.mk_str(suf.to_string());
        let trailing = self.mk_str(a.trailing().as_str().unwrap_or("").to_string());
        let tcomma = self.mk_bool(a.trailing_comma());
        let mut items = vec![head, prefix, suffix, trailing, tcomma];
        for elem in a.iter() {
            let (epre, esuf) = decor_strs(elem.decor());
            let ehead = self.mk_name("toml-elem");
            let ep = self.mk_str(epre);
            let es = self.mk_str(esuf);
            let ev = self.value(elem);
            let e = self.mk_list(vec![ehead, ep, es, ev]);
            items.push(e);
        }
        self.mk_list(items)
    }

    /// `(toml-inline-table <prefix:Str> <suffix:Str> <preamble:Str> <dotted:Bool> <inline-kv>…)` where
    /// each `inline-kv` is `(toml-inline-kv <key-path> <value>)`.
    fn inline_table(&mut self, t: &InlineTable, pre: &str, suf: &str) -> StructId {
        let head = self.mk_name("toml-inline-table");
        let prefix = self.mk_str(pre.to_string());
        let suffix = self.mk_str(suf.to_string());
        let preamble = self.mk_str(t.preamble().as_str().unwrap_or("").to_string());
        let dotted = self.mk_bool(t.is_dotted());
        let mut items = vec![head, prefix, suffix, preamble, dotted];
        for (key, v) in t.iter() {
            // Recover the Key (with decor) via get_key_value; the iterator gives a decoded &str.
            let khead = self.mk_name("toml-inline-kv");
            let kp = match t.get_key_value(key) {
                Some((k, _)) => {
                    let head = self.mk_name("toml-key-path");
                    let seg = self.key_segment(k);
                    self.mk_list(vec![head, seg])
                }
                None => {
                    let head = self.mk_name("toml-key-path");
                    let seg = self.key_segment(&Key::new(key));
                    self.mk_list(vec![head, seg])
                }
            };
            let vv = self.value(v);
            let ikv = self.mk_list(vec![khead, kp, vv]);
            items.push(ikv);
        }
        self.mk_list(items)
    }

    // ---- span-recording arena helpers (mirror sexpr/json/markdown `mk_*`; one span per StructId) ----

    fn push_span(&mut self) {
        if let Some(t) = self.spans.as_mut() {
            debug_assert_eq!(
                t.len() + 1,
                self.b.structure_len(),
                "toml span table drifted from the arena"
            );
            // A despanned DocumentMut has no per-node source range; a placeholder keeps the table 1:1.
            t.push(Span::new(0, 0));
        }
    }

    fn mk_name(&mut self, name: &str) -> StructId {
        let id = self.b.name(name);
        self.push_span();
        id
    }

    fn mk_str(&mut self, s: String) -> StructId {
        self.mk_atom_leaf(Leaf::Str(s.into()))
    }

    fn mk_bool(&mut self, v: bool) -> StructId {
        self.mk_atom_leaf(Leaf::Bool(v))
    }

    fn mk_int(&mut self, n: i64) -> StructId {
        self.mk_atom_leaf(Leaf::Int {
            value: cadenza_syntax_core::ast::IntValue::from_i64(n),
            radix: Radix::Dec,
        })
    }

    fn mk_atom_leaf(&mut self, leaf: Leaf) -> StructId {
        let id = self.b.atom_leaf(leaf);
        self.push_span();
        id
    }

    fn mk_list(&mut self, items: Vec<StructId>) -> StructId {
        let id = self.b.list(items);
        self.push_span();
        id
    }
}

/// A `Decor`'s (prefix, suffix) as owned strings (empty when absent). Works because a parsed
/// `DocumentMut` is despanned, so `RawString::as_str()` returns the materialized text.
fn decor_strs(decor: &toml_edit::Decor) -> (String, String) {
    let pre = decor
        .prefix()
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let suf = decor
        .suffix()
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    (pre, suf)
}

// ============================================================================
// Printer: decor-in-arena -> TOML text (byte-exact, via rebuilding a DocumentMut)
// ============================================================================

/// Render a decor-in-arena TOML document back to text. Reconstructs a `toml_edit::DocumentMut` from the
/// arena nodes (restoring raw reprs by re-parsing them, replaying every decor string + table position)
/// and calls its `Display`, so byte-exact reconstruction (header derivation, document order,
/// synthesized structural newlines) is toml_edit's job, not ours. `width` is accepted for surface-layer
/// uniformity and ignored (TOML layout is byte-reproduced). A NON-TOML root (a bare program handed to
/// `--to toml`) becomes a single `program = "<ml text>"` key — a valid TOML doc that survives a
/// round-trip — mirroring the JSON surface's string fallback.
/// `ml_print` renders an arbitrary arena as ML text — INJECTED (not called directly) so this crate stays
/// BELOW the ML surface: the facade re-exports it, so a dependency on the ML printer would cycle. Only
/// the non-TOML fallback path invokes it; a genuine `(toml-document …)` root never touches it. The facade
/// (and the ML printer, when embedding a `toml{…}` sub-document) pass `cadenza_syntax::printer::print`.
pub fn print(arenas: &Arenas, width: usize, ml_print: fn(&Arenas, usize) -> String) -> String {
    if arenas.head_name(arenas.root) == Some("toml-document") {
        match build_document(arenas) {
            Some(doc) => doc.to_string(),
            None => fallback(arenas, width, ml_print),
        }
    } else {
        fallback(arenas, width, ml_print)
    }
}

/// The non-TOML-root fallback: carry the program's ML text as a single TOML string key.
fn fallback(arenas: &Arenas, width: usize, ml_print: fn(&Arenas, usize) -> String) -> String {
    let ml = ml_print(arenas, width);
    let mut doc = DocumentMut::new();
    doc["program"] = toml_edit::value(ml);
    doc.to_string()
}

/// Reconstruct a `DocumentMut` from the arena. `None` if the tree is malformed (a defensive guard;
/// a tree produced by `read` is always well-formed).
fn build_document(a: &Arenas) -> Option<DocumentMut> {
    let mut doc = DocumentMut::new();
    let items = child_tail(a, a.root);
    // Last child is the document trailing string; the rest are entries.
    let (entries, trailing) = match items.split_last() {
        Some((last, rest)) => (rest, str_leaf(a, *last).unwrap_or_default()),
        None => (&items[..], String::new()),
    };
    let table = doc.as_table_mut();
    build_table_entries(a, entries, table)?;
    doc.set_trailing(trailing);
    Some(doc)
}

/// Populate `table` from a slice of entry nodes (kvs, sub-tables, arrays-of-tables).
fn build_table_entries(a: &Arenas, entries: &[StructId], table: &mut Table) -> Option<()> {
    for &entry in entries {
        match a.head_name(entry) {
            Some("toml-kv") => {
                let items = list_items(a, entry);
                let keys = build_key_segments(a, *items.get(1)?)?;
                let value = build_value(a, *items.get(2)?)?;
                insert_dotted_kv(table, &keys, value)?;
            }
            Some("toml-table") => {
                // (toml-table prefix pos implicit header suffix entry…)
                let items = list_items(a, entry);
                let (pre, suf) = (
                    str_leaf(a, *items.get(1)?).unwrap_or_default(),
                    str_leaf(a, *items.get(5)?).unwrap_or_default(),
                );
                let pos = int_leaf(a, *items.get(2)?).unwrap_or(0);
                let implicit = bool_leaf(a, *items.get(3)?).unwrap_or(false);
                let header = build_header_key(a, *items.get(4)?)?;
                let mut sub = Table::new();
                sub.set_position(pos as usize);
                sub.set_implicit(implicit);
                *sub.decor_mut() = mk_decor(&pre, &suf);
                build_table_entries(a, &items[6.min(items.len())..], &mut sub)?;
                table.insert_formatted(&header, Item::Table(sub));
            }
            Some("toml-array-table") => {
                // (toml-array-table prefix pos header suffix entry…)
                let items = list_items(a, entry);
                let (pre, suf) = (
                    str_leaf(a, *items.get(1)?).unwrap_or_default(),
                    str_leaf(a, *items.get(4)?).unwrap_or_default(),
                );
                let pos = int_leaf(a, *items.get(2)?).unwrap_or(0);
                let header = build_header_key(a, *items.get(3)?)?;
                let mut sub = Table::new();
                sub.set_position(pos as usize);
                *sub.decor_mut() = mk_decor(&pre, &suf);
                build_table_entries(a, &items[5.min(items.len())..], &mut sub)?;
                // Append to (creating if needed) the array-of-tables at `header`.
                let key_str = header.get().to_string();
                let aot_item = table
                    .entry(&key_str)
                    .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
                aot_item.as_array_of_tables_mut()?.push(sub);
            }
            _ => return None, // unexpected entry head
        }
    }
    Some(())
}

/// Reconstruct all key segments (each with raw spelling + leaf/dotted decor) from a `(toml-key-path …)`
/// node. A `[header]` key is one segment; a dotted key-value (`a.b.c`) is several.
fn build_key_segments(a: &Arenas, path: StructId) -> Option<Vec<Key>> {
    let segs = child_tail(a, path);
    if segs.is_empty() {
        return None;
    }
    segs.iter().map(|&s| build_key_segment(a, s)).collect()
}

/// Reconstruct a single header `Key` from a one-segment `(toml-key-path …)` node.
fn build_header_key(a: &Arenas, path: StructId) -> Option<Key> {
    build_key_segments(a, path)?.into_iter().next()
}

/// Insert `value` at a (possibly dotted) key path into `table`. A single segment inserts directly; a
/// dotted path (`a.b.c`) creates nested `is_dotted()` tables so toml_edit's encoder renders it inline
/// (`a.b.c = 1`) rather than as `[a.b]` headers — the inverse of `Table::get_values`'s flattening.
fn insert_dotted_kv(table: &mut Table, keys: &[Key], value: Value) -> Option<()> {
    match keys {
        [] => None,
        [only] => {
            table.insert_formatted(only, Item::Value(value));
            Some(())
        }
        [first, rest @ ..] => {
            // Descend/create a dotted intermediate table for `first`, then recurse.
            let entry = table.entry_format(first).or_insert(Item::Table({
                let mut t = Table::new();
                t.set_dotted(true);
                t
            }));
            let sub = entry.as_table_mut()?;
            sub.set_dotted(true);
            insert_dotted_kv(sub, rest, value)
        }
    }
}

/// The inline-table analogue of [`insert_dotted_kv`]: a dotted key inside `{ … }` (`{ a.b = 1 }`)
/// nests dotted inline tables so the encoder renders it inline.
fn insert_inline_dotted_kv(table: &mut InlineTable, keys: &[Key], value: Value) -> Option<()> {
    match keys {
        [] => None,
        [only] => {
            table.insert_formatted(only, value);
            Some(())
        }
        [first, rest @ ..] => {
            let entry = table.entry_format(first).or_insert(Value::InlineTable({
                let mut t = InlineTable::new();
                t.set_dotted(true);
                t
            }));
            let sub = entry.as_inline_table_mut()?;
            sub.set_dotted(true);
            insert_inline_dotted_kv(sub, rest, value)
        }
    }
}

/// Reconstruct one `Key` from a `(toml-key raw leaf-pre leaf-suf dot-pre dot-suf)` node.
fn build_key_segment(a: &Arenas, seg: StructId) -> Option<Key> {
    let items = list_items(a, seg);
    let raw = str_leaf(a, *items.get(1)?)?;
    let lpre = str_leaf(a, *items.get(2)?).unwrap_or_default();
    let lsuf = str_leaf(a, *items.get(3)?).unwrap_or_default();
    let dpre = str_leaf(a, *items.get(4)?).unwrap_or_default();
    let dsuf = str_leaf(a, *items.get(5)?).unwrap_or_default();
    // Parse the raw spelling into a Key (preserves quote style), then replay its decor.
    let mut keys = Key::parse(&raw).ok()?;
    let mut key = keys.pop()?;
    key = key
        .with_leaf_decor(mk_decor(&lpre, &lsuf))
        .with_dotted_decor(mk_decor(&dpre, &dsuf));
    Some(key)
}

/// Reconstruct a `Value` from a value node, replaying its decor + raw repr.
fn build_value(a: &Arenas, id: StructId) -> Option<Value> {
    match a.head_name(id) {
        Some("toml-string")
        | Some("toml-integer")
        | Some("toml-float")
        | Some("toml-boolean")
        | Some("toml-datetime") => {
            let items = list_items(a, id);
            let pre = str_leaf(a, *items.get(1)?).unwrap_or_default();
            let suf = str_leaf(a, *items.get(2)?).unwrap_or_default();
            let raw = str_leaf(a, *items.get(3)?)?;
            // Re-parse the raw scalar text to recover its exact Repr (the public byte-exact path;
            // set_repr_unchecked is private). Then replay the value's decor.
            let mut v = raw.parse::<Value>().ok()?;
            *v.decor_mut() = mk_decor(&pre, &suf);
            Some(v)
        }
        Some("toml-array") => build_array(a, id),
        Some("toml-inline-table") => build_inline_table(a, id),
        _ => None,
    }
}

/// Reconstruct a `Value::Array` from `(toml-array prefix suffix trailing trailing-comma elem…)`.
fn build_array(a: &Arenas, id: StructId) -> Option<Value> {
    let items = list_items(a, id);
    let pre = str_leaf(a, *items.get(1)?).unwrap_or_default();
    let suf = str_leaf(a, *items.get(2)?).unwrap_or_default();
    let trailing = str_leaf(a, *items.get(3)?).unwrap_or_default();
    let tcomma = bool_leaf(a, *items.get(4)?).unwrap_or(false);
    let mut arr = Array::new();
    for &elem in &items[5.min(items.len())..] {
        // (toml-elem prefix suffix value)
        let ei = list_items(a, elem);
        let epre = str_leaf(a, *ei.get(1)?).unwrap_or_default();
        let esuf = str_leaf(a, *ei.get(2)?).unwrap_or_default();
        let mut ev = build_value(a, *ei.get(3)?)?;
        *ev.decor_mut() = mk_decor(&epre, &esuf);
        arr.push_formatted(ev);
    }
    arr.set_trailing(trailing);
    arr.set_trailing_comma(tcomma);
    let mut v = Value::Array(arr);
    *v.decor_mut() = mk_decor(&pre, &suf);
    Some(v)
}

/// Reconstruct a `Value::InlineTable` from
/// `(toml-inline-table prefix suffix preamble dotted inline-kv…)`.
fn build_inline_table(a: &Arenas, id: StructId) -> Option<Value> {
    let items = list_items(a, id);
    let pre = str_leaf(a, *items.get(1)?).unwrap_or_default();
    let suf = str_leaf(a, *items.get(2)?).unwrap_or_default();
    let preamble = str_leaf(a, *items.get(3)?).unwrap_or_default();
    let dotted = bool_leaf(a, *items.get(4)?).unwrap_or(false);
    let mut t = InlineTable::new();
    t.set_preamble(preamble);
    t.set_dotted(dotted);
    for &ikv in &items[5.min(items.len())..] {
        // (toml-inline-kv key-path value)
        let ki = list_items(a, ikv);
        let keys = build_key_segments(a, *ki.get(1)?)?;
        let value = build_value(a, *ki.get(2)?)?;
        insert_inline_dotted_kv(&mut t, &keys, value)?;
    }
    let mut v = Value::InlineTable(t);
    *v.decor_mut() = mk_decor(&pre, &suf);
    Some(v)
}

/// A `Decor` from prefix/suffix strings.
fn mk_decor(prefix: &str, suffix: &str) -> toml_edit::Decor {
    toml_edit::Decor::new(prefix, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strong contract: parse → print is byte-identical for an unmutated document.
    fn assert_byte_exact(src: &str) {
        let a = read(src).expect("valid TOML");
        let printed = print(&a, 100, |_, _| String::new());
        assert_eq!(
            printed, src,
            "not byte-exact\n--- src ---\n{src}\n--- printed ---\n{printed}"
        );
    }

    /// The fallback law: read → print → read is an arena fixed point (after any normalization).
    fn assert_idempotent(src: &str) {
        let a1 = read(src).expect("valid TOML");
        let printed = print(&a1, 100, |_, _| String::new());
        let a2 = read(&printed).expect("reprinted TOML parses");
        assert!(
            a1.structurally_eq(&a2),
            "not arena-idempotent\n--- src ---\n{src}\n--- printed ---\n{printed}"
        );
    }

    #[test]
    fn byte_exact_scalars() {
        assert_byte_exact("s = \"hello\"\n");
        assert_byte_exact("i = 42\n");
        assert_byte_exact("neg = -7\n");
        assert_byte_exact("f = 3.14\n");
        assert_byte_exact("b = true\n");
        assert_byte_exact("b2 = false\n");
    }

    #[test]
    fn byte_exact_lossy_number_forms_survive() {
        // The whole point of storing the RAW repr: these must print verbatim, not as the parsed value.
        assert_byte_exact("hex = 0xDEADBEEF\n");
        assert_byte_exact("oct = 0o755\n");
        assert_byte_exact("bin = 0b1010\n");
        assert_byte_exact("sep = 1_000_000\n");
        assert_byte_exact("big = 9_223_372_036_854_775_807\n");
        assert_byte_exact("exp = 6.28e-2\n");
        assert_byte_exact("pos = 5e+22\n");
    }

    #[test]
    fn byte_exact_inf_nan_datetime() {
        // No Float leaf for inf/nan and no datetime leaf — raw-text nodes must still round-trip.
        assert_byte_exact("a = inf\n");
        assert_byte_exact("b = -inf\n");
        assert_byte_exact("c = nan\n");
        assert_byte_exact("odt = 1979-05-27T07:32:00Z\n");
        assert_byte_exact("ldt = 1979-05-27T07:32:00\n");
        assert_byte_exact("ld = 1979-05-27\n");
        assert_byte_exact("lt = 07:32:00\n");
    }

    #[test]
    fn byte_exact_string_flavors() {
        assert_byte_exact("basic = \"a\\nb\"\n");
        assert_byte_exact("literal = 'C:\\path\\no\\escape'\n");
        assert_byte_exact("multi = \"\"\"\nline one\nline two\n\"\"\"\n");
        assert_byte_exact("multi_lit = '''\nraw\ntext\n'''\n");
    }

    #[test]
    fn byte_exact_comments_and_blanks() {
        assert_byte_exact("# leading comment\nkey = \"value\"\n");
        assert_byte_exact("key = \"value\" # trailing comment\n");
        assert_byte_exact("a = 1\n\n\nb = 2\n");
        assert_byte_exact("# c1\n# c2\n[server]\nhost = \"x\"\n");
    }

    #[test]
    fn byte_exact_dotted_and_quoted_keys() {
        assert_byte_exact("a.b.c = 1\n");
        assert_byte_exact("\"quoted key\" = 2\n");
        assert_byte_exact("'literal.key' = 3\n");
        assert_byte_exact("physical.color = \"orange\"\n");
    }

    #[test]
    fn byte_exact_tables_and_aot() {
        assert_byte_exact("[server]\nhost = \"localhost\"\nport = 8080\n");
        assert_byte_exact("[a.b.c]\nx = 1\n");
        assert_byte_exact("[[products]]\nname = \"a\"\n\n[[products]]\nname = \"b\"\n");
    }

    #[test]
    fn byte_exact_arrays_and_inline_tables() {
        assert_byte_exact("ports = [8000, 8001]\n");
        assert_byte_exact("nested = [[1, 2], [3]]\n");
        assert_byte_exact("trailing = [1, 2,]\n");
        assert_byte_exact("point = { x = 1, y = 2 }\n");
        assert_byte_exact("empty = {}\n");
        assert_byte_exact("empty_arr = []\n");
    }

    #[test]
    fn byte_exact_realistic_document() {
        let src = "# a realistic config\ntitle = \"demo\"\n\n[server]\nhost = \"127.0.0.1\"  # bind\nports = [8000, 8001]\nopts = { debug = true, level = 3 }\n\n[[peer]]\nname = \"alpha\"\n\n[[peer]]\nname = \"beta\"\n";
        assert_byte_exact(src);
    }

    #[test]
    fn arena_idempotent_across_kinds() {
        assert_idempotent(
            "# c\n[t]\na = 0xFF\nb = [1, 2, 3]\nc = { d = inf }\ndt = 1979-05-27T07:32:00Z\n",
        );
    }

    #[test]
    fn errors_are_refused() {
        for bad in [
            "[unterminated",
            "a = ",
            "a = = 1",
            "= 1",
            "[a]\n[a]\n",     // duplicate table
            "a = 1\na = 2\n", // duplicate key
            "d = 2024-13-40", // bad datetime
            "a = [1, 2",      // unterminated array
            "a = \"open",     // unterminated string
        ] {
            assert!(
                read(bad).is_err(),
                "expected a parse error for {bad:?}, got Ok"
            );
        }
    }

    #[test]
    fn a_missing_value_after_eq_reports_a_cause_not_an_empty_headline() {
        // toml_edit emits an EMPTY message for the missing-value class (a `key =` whose value position
        // hits end-of-input or only whitespace). Left unhandled it lifted into the embedded `toml{ … }`
        // region as a bare `... region:  at byte N` (double space, no reason) — the gap v-parser-corpus
        // routed here. `err_head` now substitutes an accurate cause, and every message still ends in the
        // `at byte N` the caller remaps to line:col.
        for src in ["a = ", "a =", "x = \t", "[t]\nb =", "y =  "] {
            let msg = read(src).unwrap_err().0;
            assert!(
                msg.starts_with("expected a value"),
                "missing-value {src:?} must report a cause, got {msg:?}"
            );
            assert!(
                msg.contains("at byte"),
                "message must keep the byte anchor, got {msg:?}"
            );
        }
        // A value that IS present but malformed keeps toml_edit's own specific headline (NOT the
        // missing-value fallback): a bare `@`, a newline, or a `#` comment in value position is an
        // "invalid string" (toml_edit tries the string parser), an unterminated array is "invalid array".
        for (src, head) in [
            ("a = @", "invalid string"),
            ("a = [1, 2", "invalid array"),
            ("[unterminated", "invalid table header"),
            ("= 1", "invalid key"),
        ] {
            let msg = read(src).unwrap_err().0;
            assert!(
                msg.starts_with(head),
                "{src:?} should keep its specific headline {head:?}, got {msg:?}"
            );
        }
    }

    #[test]
    fn rewrite_reflects_in_output() {
        // Editing a scalar's raw Str leaf in the arena changes the printed output — proving the arena
        // is the single rewritable representation, not a frozen blob.
        let src = "host = \"localhost\"\n";
        let mut a = read(src).unwrap();
        // Find the toml-string node's raw leaf and rewrite it.
        let mut changed = false;
        for id in (0..a.structure.len() as u32).map(StructId) {
            if a.head_name(id) == Some("toml-string") {
                let items = list_items(&a, id);
                // items[3] is the raw leaf; rebuild the arena with that leaf's string changed.
                if let cadenza_syntax_core::ast::Struct::Atom(l) = *a.get(items[3])
                    && let Leaf::Str(_) = a.leaf(l)
                {
                    a.leaves[l.0 as usize] = Leaf::Str("\"0.0.0.0\"".into());
                    changed = true;
                }
            }
        }
        assert!(changed, "found and rewrote the raw string leaf");
        let printed = print(&a, 100, |_, _| String::new());
        assert_eq!(
            printed, "host = \"0.0.0.0\"\n",
            "the rewrite reflects in output"
        );
    }

    #[test]
    fn toml_to_binary_round_trips() {
        let src = "[a]\nx = [1, 2, { b = true }]\ny = \"z\"\n";
        let a1 = read(src).unwrap();
        let bin = cadenza_ast::codec::encode(&a1);
        let a2 = cadenza_ast::codec::decode(&bin).expect("decodes");
        assert!(a1.structurally_eq(&a2));
        // And printing the decoded arena reproduces the source (toml-document root; ml_print unused).
        assert_eq!(print(&a2, 100, |_, _| String::new()), src);
    }

    // NOTE: `non_toml_root_falls_back_to_program_key` moved to `cadenza-syntax`'s in-crate
    // `surface_tests` — it exercises the ML-printer fallback (a non-TOML root → `program = "<ml>"` key),
    // which needs the ML printer + the sexpr reader, neither of which this below-the-surface crate may
    // depend on.

    #[test]
    fn span_table_is_total_and_ordered() {
        let (a, spans) = read_spanned("[t]\na = 1\nb = [2, 3]\n").unwrap();
        assert_eq!(spans.len(), a.structure.len());
        for id in (0..a.structure.len() as u32).map(StructId) {
            assert!(spans.get(id).is_some(), "node {id:?} has a span");
        }
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

    /// Generate a random WELL-FORMED TOML document whose bytes `toml_edit` reproduces EXACTLY — the
    /// decor-in-arena model must preserve every byte. Emits blank lines, `# comments`, `[table]`
    /// headers, and `key = value` lines with scalars in their canonical printed spelling (so re-print
    /// is byte-identical): decimal ints, `a.b` floats, `true`/`false`, basic strings, and flat arrays.
    /// Deliberately CONSERVATIVE (no exotic number spellings whose Repr could re-normalize) — the goal
    /// is to stress the DECOR + document-order + table-nesting reconstruction over many shapes, exactly.
    fn gen_toml(rng: &mut Rng) -> String {
        let scalar = |rng: &mut Rng| -> String {
            match rng.below(6) {
                0 => rng.below(100000).to_string(),
                1 => format!("-{}", rng.below(1000)),
                2 => format!("{}.{}", rng.below(1000), rng.below(1000)),
                3 => "true".to_string(),
                4 => "false".to_string(),
                _ => format!("\"str{}\"", rng.below(1000)),
            }
        };
        let mut out = String::new();
        // TOML forbids duplicate keys (in a table) and duplicate table headers, so keys/table names are
        // made UNIQUE with a monotonic counter — the generator emits only VALID TOML (a duplicate would
        // be a generator bug, correctly rejected by `read`, not a surface bug).
        let mut n = 0usize;
        let mut uniq = || {
            n += 1;
            n
        };
        // arrays sometimes; simple `key = value` otherwise. `toml_edit` prints `[1, 2, 3]` with the
        // single-space-after-comma decor a fresh parse carries, so generate that exact form.
        let kv = |rng: &mut Rng, out: &mut String, id: usize| {
            if rng.below(4) == 0 {
                let m = rng.below(4);
                let elems: Vec<String> = (0..m).map(|_| scalar(rng)).collect();
                out.push_str(&format!("k{id} = [{}]\n", elems.join(", ")));
            } else {
                out.push_str(&format!("k{id} = {}\n", scalar(rng)));
            }
        };
        let decor = |rng: &mut Rng, out: &mut String| match rng.below(4) {
            0 => out.push('\n'),                                           // blank line
            1 => out.push_str(&format!("# comment {}\n", rng.below(100))), // comment line
            _ => {}                                                        // nothing
        };
        for _ in 0..(1 + rng.below(3)) {
            decor(rng, &mut out);
            let id = uniq();
            kv(rng, &mut out, id);
        }
        // Then 0..=2 tables (unique headers), each with its own unique-keyed lines.
        for _ in 0..rng.below(3) {
            decor(rng, &mut out);
            out.push_str(&format!("[t{}]\n", uniq()));
            for _ in 0..(1 + rng.below(3)) {
                decor(rng, &mut out);
                let id = uniq();
                kv(rng, &mut out, id);
            }
        }
        out
    }

    #[test]
    fn toml_surface_is_byte_exact_over_generated_documents() {
        // The strong byte-exact contract (`print(read(src)) == src`) swept over random well-formed TOML,
        // complementing the hand-picked cases. The generator explores decor (blanks/comments) + document
        // order + table-nesting COMBINATIONS the fixed tests don't, so a decor-reconstruction asymmetry
        // that no hand-written case hits is caught. Every generated doc is in `toml_edit`'s canonical
        // printed spelling, so byte-exact is the right assertion; fixed seeds → reproducible.
        let seeds: [u64; 3] = [
            0x0bad_c0de_dead_beef,
            0x5eed_1234_5678_9abc,
            0xfeed_face_cafe_babe,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..800 {
                let src = gen_toml(&mut rng);
                // A generated doc must PARSE (a generator bug otherwise) and re-print byte-exactly.
                assert_byte_exact(&src);
                total += 1;
            }
        }
        assert!(total >= 2000, "swept a meaningful space, got {total}");
    }

    #[test]
    fn a_generated_toml_document_survives_the_binary_codec_byte_exact() {
        // TOML stores its DECOR (comments, blank lines, each scalar's raw spelling, key/table order) in
        // the arena so an unmutated doc round-trips BYTE-EXACT. The binary codec is the canonical STORED
        // form, so it must preserve that decor-in-arena faithfully — `cdz convert doc.toml --to binary`
        // then back must reproduce the source byte-for-byte, not just structurally. `toml_to_binary_round
        // _trips` pins ONE hand doc; this sweeps it: for random well-formed TOML, read → encode → decode
        // is structurally identical AND printing the DECODED arena reproduces the source exactly. A codec
        // that dropped or reordered a decor leaf would corrupt a stored config doc — the case the hand
        // doc can't reach. Also asserts encode is deterministic (the bijection).
        let seeds: [u64; 3] = [
            0x7031_c0de_0bad_f00d,
            0x5eed_beef_1234_abcd,
            0xca7f_00d5_dead_10ff,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..800 {
                let src = gen_toml(&mut rng);
                let a1 = read(&src).expect("generated TOML parses");
                let bin = cadenza_ast::codec::encode(&a1);
                let a2 = cadenza_ast::codec::decode(&bin)
                    .expect("a TOML arena decodes from its own encoding");
                assert!(
                    a1.structurally_eq(&a2),
                    "TOML arena survives the binary round-trip for:\n{src}"
                );
                assert_eq!(
                    bin,
                    cadenza_ast::codec::encode(&a2),
                    "binary encode is deterministic for:\n{src}"
                );
                // The STRONG contract: the decoded arena re-prints byte-exact to the source.
                assert_eq!(
                    print(&a2, 100, |_, _| String::new()),
                    src,
                    "TOML is byte-exact through the binary codec for:\n{src}"
                );
                total += 1;
            }
        }
        assert!(total >= 2000, "swept a meaningful codec space, got {total}");
    }

    #[test]
    fn toml_read_never_panics_on_arbitrary_input() {
        // `read` operates on UNTRUSTED config text; it must return a diagnostic, never panic. Sweep
        // random TOML-ish strings (structural chars + key/value bytes) plus truncated fragments. On a
        // SUCCESSFUL read the arena must be well-formed with a TOTAL span table — see
        // `assert_toml_read_invariants`.
        let alphabet: Vec<char> = "[]{}=.,\"'#\n \t01abtruefalse-_".chars().collect();
        let mut rng = Rng(0x2468_ace0_1357_9bdf);
        for len in 0..=32usize {
            for _ in 0..80 {
                let s: String = (0..len)
                    .map(|_| alphabet[rng.below(alphabet.len())])
                    .collect();
                assert_toml_read_invariants(&s);
            }
        }
        for s in [
            "[", "[t", "a =", "a = \"", "a = [", "# c", "[[", "a = 0x", "a = ''",
        ] {
            assert_toml_read_invariants(s);
        }
    }

    /// `read` must not panic on arbitrary input, and on a SUCCESSFUL read the arena is well-formed with
    /// a TOTAL span table: `read`/`read_spanned` agree structurally, the arena is non-empty with root in
    /// range, `spans` is exactly 1:1 with the structure vector, and every reachable child id is in range.
    /// A clean `ReadError` on malformed input is fine. Mirrors the ML/s-expr/markdown/json reader fuzzes.
    fn assert_toml_read_invariants(src: &str) {
        let plain = read(src); // must not panic
        let Ok((a, spans)) = read_spanned(src) else {
            assert!(plain.is_err(), "read_spanned Err but read Ok for {src:?}");
            return;
        };
        assert!(
            plain.is_ok_and(|p| p.structurally_eq(&a)),
            "read and read_spanned disagree for {src:?}"
        );
        let n = a.structure.len();
        assert!(
            n > 0 && (a.root.0 as usize) < n,
            "root id in range for {src:?}"
        );
        assert_eq!(spans.len(), n, "span table is total for {src:?}");
        // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, on UTF-8 char
        // boundaries — even on malformed input. Totality only says a span EXISTS per node; this says
        // `&src[sp.start..sp.end]` (an LSP hover / diagnostic underline / span-based edit) can be taken
        // WITHOUT panicking. The reader synthesizes spans for structural nodes (tables/arrays), so an
        // off-by-one or a span past a truncated source is a real risk on the error path.
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
            if let cadenza_syntax_core::ast::Struct::List(kids) = a.get(id) {
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
}
