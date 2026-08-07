//! The `cadenza:syntax` reducer-facing wasm-COMPONENT guest — exports the front-end (parse), the
//! structural query engine (query), and syntax-only doc extraction (doc) by delegating to the
//! `cadenza-syntax` pure library. P2 (design note `p2-reducer-invoke-cadenza-syntax-as-composable-wit-dep`;
//! operator Option-A): a content-addressed, in-wasm linked pure-library dep a reducer imports directly —
//! NOT a host effect (parse/query/doc are pure functions of their inputs).
//!
//! ## The boundary carries cdzast bytes
//! Every AST crossing the WIT boundary is a canonical `cdzast\x00\x01` artifact (`list<u8>`), the SAME
//! wire the shared `cadenza-syntax::codec` bottom crate uses (design: keep ASTs as opaque versioned
//! bytes so the boundary is codec-versioned + arena-rep-independent). So each shim is thin:
//! bytes-in → `codec::decode` → run the pure library fn → `codec::encode` → bytes-out. Source text
//! crosses as `string`. There is NO structured `Struct`/`Leaf` mirroring in the ABI (that would freeze
//! the arena rep into the wire).
//!
//! ## Totality
//! No shim traps. Parsing is error-recovering (`Parsed` always yields an arena + diagnostics); an
//! undecodable input AST (a malformed/foreign `list<u8>`) degrades to an empty result (empty match list
//! / empty doc-module bytes), never a panic — the host treats a trap as a hard failure, so a doc/query
//! over a bad blob returns "nothing", not a crash.

wit_bindgen::generate!({
    world: "syntax",
    path: "wit/syntax.wit",
});

use cadenza_syntax::ast::Arenas;
use cadenza_syntax::query::{Pattern, Tree};
use cadenza_syntax::{codec, doc_item, parser, sexpr};

use exports::cadenza::syntax::doc::Guest as DocGuest;
use exports::cadenza::syntax::parse::Guest as ParseGuest;
use exports::cadenza::syntax::query::Guest as QueryGuest;
use exports::cadenza::syntax::types::{Diagnostic, Parsed};

struct Component;

/// Encode a `Parsed`-style result (arena + parse errors) into the WIT `parsed` record: the AST bytes
/// plus a diagnostic per recovered error. The arena is always present (error recovery yields a repaired
/// tree), so `ast` is never empty for a real parse.
fn parsed_of(arenas: &Arenas, errors: &[parser::ParseError]) -> Parsed {
    Parsed {
        ast: codec::encode(arenas),
        diagnostics: errors
            .iter()
            .map(|e| Diagnostic {
                message: e.message.clone(),
                byte_offset: e.span.start as u32,
                len: e.span.len() as u32,
            })
            .collect(),
    }
}

impl ParseGuest for Component {
    fn read_ml(source: String) -> Parsed {
        let p = parser::read_ml(&source);
        parsed_of(&p.arenas, &p.errors)
    }

    fn read_sexpr(source: String) -> Parsed {
        // The s-expr reader is stricter than the ML path (a hard `ReadError`, not error-recovery). Map a
        // read error to a single diagnostic with empty `ast` (offset 0 — the reader doesn't carry a byte
        // span), so the caller checks `diagnostics` before using `ast`. A clean read → the arena bytes,
        // no diagnostics.
        match sexpr::read(&source) {
            Ok(arenas) => Parsed {
                ast: codec::encode(&arenas),
                diagnostics: Vec::new(),
            },
            Err(e) => Parsed {
                ast: Vec::new(),
                diagnostics: vec![Diagnostic {
                    message: e.0,
                    byte_offset: 0,
                    len: 0,
                }],
            },
        }
    }
}

impl QueryGuest for Component {
    fn search(program: Vec<u8>, pattern: String) -> Vec<Vec<u8>> {
        // Decode the program; an undecodable blob or an invalid pattern → empty (never a trap).
        let Some(arenas) = codec::decode(&program) else {
            return Vec::new();
        };
        let Ok(pat) = Pattern::compile(&pattern) else {
            return Vec::new();
        };
        let tree = Tree::of(&arenas);
        cadenza_syntax::query::search(&pat, &tree, None)
            .into_iter()
            // Each match's node is re-encoded as a standalone AST-bytes subtree (its own arena), in
            // source order — the caller decodes each to read bound metavars / the matched form.
            .map(|m| codec::encode(&m.node.to_arena()))
            .collect()
    }
}

impl DocGuest for Component {
    fn project(program: Vec<u8>, module_name: String) -> Vec<u8> {
        // Decode the program and project its public surface into a structural doc-module (I1). An
        // undecodable blob → empty bytes (graceful-degrade; the host sees "no doc-module", not a crash).
        match codec::decode(&program) {
            Some(arenas) => codec::encode(&doc_item::project(&arenas, &module_name)),
            None => Vec::new(),
        }
    }
}

export!(Component);
