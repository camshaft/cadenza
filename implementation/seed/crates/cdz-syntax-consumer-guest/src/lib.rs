//! The part-2 CONSUMER guest — a wit-bindgen reducer guest that IMPORTS cadenza:syntax (a
//! content-addressed +hash dep) and, in its fold, CALLS the composed `parse` (P2 flavor-2, design note
//! p2-reducer-invoke-cadenza-syntax-as-composable-wit-dep). The MIRROR of the producer #2673: the
//! producer EXPORTS the cadenza:syntax world; this consumer IMPORTS it and drives a call.
//!
//! ## The composed-dep call is a DIRECT linked import call
//! `parse::read-sexpr` is an ordinary cross-component WIT-import call (v-metaprog: flavor-2 needs NO
//! invoke-wire — the composed dep is statically bound at compose time, not the generic tagged-AST
//! invoke path). The host's `compose_dep_into_linker` resolves the +hash dep from CAS + links it
//! leaves-first, so from the guest it's just a normal imported func.
//!
//! ## Behavior (tiny + observable, like reducer-guest)
//! On an inbound `message` event whose payload is Cadenza s-expr source, the fold calls the composed
//! `cadenza:syntax parse::read-sexpr(source)` and records — in its own KV, via the kernel `kv` import —
//! whether the parse produced a clean AST (diagnostics empty) and the AST byte length. This is the
//! observable proof that the composed cadenza:syntax dep was resolved, linked, and CALLED end-to-end.
//! On a result/timer event (`resumes` set) it stops. Total: never traps (the parse is error-recovering;
//! a non-UTF8 or unreadable payload just records a zero-length result).

wit_bindgen::generate!({
    world: "consumer",
    path: "wit",
    generate_all,
});

use cadenza::agent_kernel::kv;
use cadenza::agent_kernel::types::{ContentType, EffectRequest};
use exports::cadenza::agent_kernel::fold::Guest;
// The composed cadenza:syntax dep — imported by content-addressed +hash (templated at build).
use cadenza::syntax::parse;

struct Consumer;

impl Guest for Consumer {
    fn apply(
        content_type: ContentType,
        payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Vec<EffectRequest> {
        // A result/timer event — nothing to do (this tiny consumer doesn't cascade).
        if resumes.is_some() {
            return Vec::new();
        }
        // Inbound message carrying Cadenza s-expr source → drive the composed cadenza:syntax parse.
        if content_type.family == "message" {
            // The payload is s-expr source text; a non-UTF8/absent payload degrades to empty source
            // (the parse is total — never traps).
            let source = payload
                .as_deref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("");
            // THE COMPOSED-DEP CALL: a direct linked import call into cadenza:syntax (resolved by +hash,
            // linked by compose_dep_into_linker). Returns the error-recovering `parsed { ast, diagnostics }`.
            let parsed = parse::read_sexpr(source);
            // Record the observable proof in KV: byte 0 = parse-clean flag (1 = no diagnostics), then the
            // AST byte length as 4 LE bytes. The host E2E reads `parse-result` to assert the composed dep
            // was actually called (not a stub) — a clean parse of valid source yields [1, len_le…].
            let clean: u8 = if parsed.diagnostics.is_empty() { 1 } else { 0 };
            let mut record = vec![clean];
            record.extend_from_slice(&(parsed.ast.len() as u32).to_le_bytes());
            kv::put(b"parse-result", &record);
        }
        // A pure-compute consumer: it requests NO effects (the parse is a direct linked call, not an
        // effect). The observable output is the KV record above.
        Vec::new()
    }
}

export!(Consumer);
