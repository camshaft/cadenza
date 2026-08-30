//! The `name` INPUT-artifact wire — the bare NAME string carried by the `KIND_ENTRY` (a package's entry
//! file name) and `KIND_COMPONENT_NAME` (a provider's published interface name) artifacts. Carried as
//! canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks
//! (operator P0, seq-284: "Binary AST everywhere. No exceptions." — no raw-UTF-8-bytes-as-name form). A
//! driver builds the bytes with [`encode_name`]; the compiler (`rcdzc::compile`/`link`) reads them with
//! [`decode_name`].
//!
//! Shape: the root is a single `Ast.Str` LEAF holding the name — the structured form of an atomic string
//! (there is nothing to "parse" beyond the codec decode). TOTAL on decode: a non-AST / non-`Str` payload
//! yields `None` (the caller treats it as an absent name), never a crash.

use cadenza_ast::ast::{Builder, Leaf, Struct};

/// Encode a bare name (entry file / component interface) as the `KIND_ENTRY` / `KIND_COMPONENT_NAME`
/// artifact bytes — a root `Ast.Str` leaf (canonical binary AST). Round-trips with [`decode_name`].
pub fn encode_name(name: &str) -> Vec<u8> {
    let mut b = Builder::new();
    let root = b.atom_leaf(Leaf::Str(name.into()));
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode a `KIND_ENTRY` / `KIND_COMPONENT_NAME` artifact back into its name string — the inverse of
/// [`encode_name`], read via the shared `cadenza_ast::codec`. TOTAL: a non-AST payload or a non-`Str` root
/// yields `None`.
pub fn decode_name(bytes: &[u8]) -> Option<String> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::Atom(_) = a.get(a.root) else {
        return None;
    };
    a.as_str(a.root).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST name wire round-trips exactly (operator P0 seq-284: no raw-bytes name form): an ordinary
    // name, an interface name with punctuation (`cadenza:pkg/iface`), a path-shaped entry (`src/lib/util`),
    // and the empty name all survive encode→decode; garbage decodes to None.
    #[test]
    fn name_binary_ast_round_trips() {
        for n in ["app", "cadenza:pkg/iface", "src/lib/util", ""] {
            assert_eq!(decode_name(&encode_name(n)).as_deref(), Some(n));
        }
        assert_eq!(decode_name(b"not a binary-ast tree"), None);
    }
}
