//! The `KIND_DOC` wire — the documentation answer for a name (`DocOf`) or a node (`DocAt`). Canonical
//! BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator P0
//! seq-284/307-308: "Binary AST everywhere" — no bespoke text format, and CRUCIALLY no discriminator
//! carried in prose the consumer has to string-match). The producer (`rcdzc`'s `run_query`
//! `DocOf`/`DocAt` arms) calls [`encode_doc`]; the consumers (`cdz`'s `doc`/`doc-at` CLI + the LSP hover
//! handlers in `main.rs`/`lsp.rs`) call [`decode_doc`]. ONE shared codec, so neither side hand-rolls a
//! parser.
//!
//! **Why a STRUCTURED variant, not a string.** The old wire packed THREE distinct outcomes into one text
//! blob — the doc prose, a `no documentation for `X`` sentinel, and a `no such definition `X`` sentinel
//! (± a `did you mean `Y`?` suggestion) — which forced the consumer to STRING-MATCH the sentinel to tell
//! a typo (non-zero exit) from an undocumented-but-real name (exit 0). That post-decode parsing is
//! exactly what the operator ruling forbids. So the discriminator is now a proper tagged value
//! ([`DocAnswer`]) and the USER-FACING message wording lives on the CONSUMER (which has the queried
//! name); the wire carries only the outcome + the optional suggestion.
//!
//! Shape: a root `Ast.List` headed by a `Str` tag. `("doc" <Str text>)` = documentation found;
//! `("undocumented")` = the name/node refers to something real but carries no doc (DocOf's "no
//! documentation"; DocAt's empty answer both map here); `("no-such-def")` or `("no-such-def" <Str near>)`
//! = the name resolves to NOTHING (a typo, DocOf only), the second form carrying a "did you mean `near`?"
//! suggestion. TOTAL on decode: a malformed / wrong-shape / unknown-tag tree yields
//! [`DocAnswer::Undocumented`] (the safe "no answer" — never a crash, never a false "found doc").

use cadenza_ast::ast::{Builder, Leaf, Struct};

/// The documentation-query outcome — the structured `KIND_DOC` answer (see module docs). The consumer
/// formats any user-facing message from this (it holds the queried name); the wire carries no prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocAnswer {
    /// Documentation text was found.
    Doc(String),
    /// The name/node refers to something real, but it carries no documentation.
    Undocumented,
    /// The name resolves to nothing — a typo — with an optional nearest-name suggestion.
    NoSuchDef { suggestion: Option<String> },
}

/// Encode a [`DocAnswer`] as the `KIND_DOC` artifact bytes — canonical binary AST (see module docs).
/// Round-trips with [`decode_doc`].
pub fn encode_doc(answer: &DocAnswer) -> Vec<u8> {
    let mut b = Builder::new();
    let children = match answer {
        DocAnswer::Doc(text) => {
            let tag = b.atom_leaf(Leaf::Str("doc".into()));
            let t = b.atom_leaf(Leaf::Str(text.as_str().into()));
            vec![tag, t]
        }
        DocAnswer::Undocumented => vec![b.atom_leaf(Leaf::Str("undocumented".into()))],
        DocAnswer::NoSuchDef { suggestion } => {
            let tag = b.atom_leaf(Leaf::Str("no-such-def".into()));
            match suggestion {
                Some(near) => {
                    let s = b.atom_leaf(Leaf::Str(near.as_str().into()));
                    vec![tag, s]
                }
                None => vec![tag],
            }
        }
    };
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_DOC` bytes back into a [`DocAnswer`] — the inverse of [`encode_doc`], read via the
/// shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape / unknown-tag tree yields
/// [`DocAnswer::Undocumented`].
pub fn decode_doc(bytes: &[u8]) -> DocAnswer {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return DocAnswer::Undocumented;
    };
    let Struct::List(children) = a.get(a.root).clone() else {
        return DocAnswer::Undocumented;
    };
    let tag = children.first().and_then(|&c| a.as_str(c));
    match tag {
        Some("doc") => match children.get(1).and_then(|&c| a.as_str(c)) {
            Some(text) => DocAnswer::Doc(text.to_string()),
            None => DocAnswer::Undocumented,
        },
        Some("no-such-def") => DocAnswer::NoSuchDef {
            suggestion: children
                .get(1)
                .and_then(|&c| a.as_str(c))
                .map(str::to_string),
        },
        // "undocumented", an unknown tag, or an empty list all read as the safe no-answer.
        _ => DocAnswer::Undocumented,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST doc answer round-trips exactly for every variant (operator P0 seq-284/307-308: no
    // bespoke text, no prose discriminator): found doc, undocumented, no-such-def with and without a
    // suggestion. A malformed tree degrades to Undocumented (never a false Doc). This is the drift guard
    // the rcdzc producer + cdz consumers rely on.
    #[test]
    fn doc_answer_binary_ast_round_trips() {
        for a in [
            DocAnswer::Doc("Adds two numbers.".to_string()),
            DocAnswer::Doc(String::new()),
            DocAnswer::Undocumented,
            DocAnswer::NoSuchDef { suggestion: None },
            DocAnswer::NoSuchDef {
                suggestion: Some("compute".to_string()),
            },
        ] {
            assert_eq!(decode_doc(&encode_doc(&a)), a);
        }
        assert_eq!(
            decode_doc(b"not a binary-ast tree"),
            DocAnswer::Undocumented
        );
    }
}
