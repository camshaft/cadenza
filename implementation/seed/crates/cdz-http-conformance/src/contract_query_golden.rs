//! Conformance-layer golden pin of the QUERY contract-id WIRE format (654(a) slice 3).
//!
//! A query contract-id is a routed, decode-confirmed wire value: its leading tag byte is
//! `HashTag::ContractQuery` (7, vs `Contract`'s 1) and its canonical declaration carries a trailing
//! `(kind query)` marker so the DIGEST — not just the tag — commits to the class. That trailing-marker byte
//! shape was self-grounded and confirmed by THIS vertical (v-gateway-conformance), so this pins it at the
//! conformance layer: given the canonical declaration BYTES alone, the marker must decode to `Query` and the
//! id must be `0x07 ++ blake3(declaration)`.
//!
//! This is a deliberate CROSS-CRATE cross-check, complementary to cdz-contract's own byte-stability golden
//! (#8913): that test pins the *construction* path (`name/types/in/out → id`), while this pins the *wire
//! rule* from the declaration bytes — `kind_from_declaration` decode + `Hash::of(ContractQuery, …)`
//! derivation — so a drift in either the marker decode or the tag/digest rule fails loudly here too. The
//! golden triple is supplied by v-mutate-query-tag (`contract_declaration_with_kind` /
//! `contract_id_with_kind(_, Query)`) and matches the id pinned in #8913.

#[cfg(test)]
mod tests {
    use cdz_contract::{ContractKind, Hash, HashTag, kind_from_declaration};

    // Golden query contract: name="temp.celsius", types=(type Temp (Mk f64)), input=output="Temp",
    // kind=Query — the same shape as the cdz-contract golden, as a query. The declaration is the canonical
    // `(contract "temp.celsius" (types (type Temp (Mk f64))) Temp Temp (kind query))` encoding (123 bytes).
    // Whitespace in the literal is line-wrap only (stripped before decoding); the true bytes are contiguous.
    const DECL_HEX: &str = "63647a6173740001090a08636f6e7472616374070c74656d702e63656c736975730a05747970\
65730a04747970650a0454656d700a024d6b0a036636340a046b696e640a057175657279100000000100020003000400050006010205\
0601030304070102020800040004000700080102 0c0d01060001090a0b0e0f";

    // 0x07 (ContractQuery) ++ blake3(declaration), rendered base62 — matches cdz-contract #8913.
    const GOLDEN_ID: &str = "06vdvYIj4KNUp2NXrH17VfKVqXNaI5K5CmsLCpFyexjg0";

    /// Decode the (whitespace-tolerant) hex literal to the raw declaration bytes.
    fn declaration_bytes() -> Vec<u8> {
        let hex: String = DECL_HEX.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(hex.len() % 2, 0, "declaration hex must be whole bytes");
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex digit pair"))
            .collect()
    }

    #[test]
    fn query_declaration_decodes_to_query_and_derives_the_golden_id() {
        let decl = declaration_bytes();
        assert_eq!(decl.len(), 123, "golden query declaration is 123 bytes");

        // (a) The trailing `(kind query)` marker decodes to Query — from the declaration bytes alone.
        assert_eq!(
            kind_from_declaration(&decl),
            Some(ContractKind::Query),
            "trailing (kind query) marker must decode to Query"
        );

        // (b) The wire rule: id = HashTag::ContractQuery (0x07) ++ blake3(declaration). Rendered base62,
        // it must equal the host golden id (independent of the name→declaration construction path).
        let id = Hash::of(HashTag::ContractQuery, &decl);
        assert_eq!(
            id.to_string(),
            GOLDEN_ID,
            "0x07 ++ blake3(declaration) must render to the golden query contract-id"
        );
    }

    #[test]
    fn a_query_id_differs_from_its_mutation_twin_in_tag_and_digest() {
        let decl = declaration_bytes();
        let query = Hash::of(HashTag::ContractQuery, &decl);
        // The mutation twin shares NEITHER: a mutation declaration omits the trailing marker (distinct
        // digest) AND carries the Contract tag (1, not 7). Hashing the SAME query-declaration bytes under
        // the mutation tag proves the tag alone already distinguishes them, and the id is not the golden.
        let same_bytes_mutation_tag = Hash::of(HashTag::Contract, &decl);
        assert_ne!(
            query.to_string(),
            same_bytes_mutation_tag.to_string(),
            "the leading tag byte alone must distinguish a query id from a mutation id"
        );
    }
}
