//! The `rcdzc.compile` reducer target — the `run`-callable compile entry that makes "anything on the
//! platform can compile a Cadenza program" concrete (brief §Targets 1). Pure `bytes -> bytes`: decode the
//! kinded-input bundle from the request payload, compile it to a wasm component, and encode the whole
//! `{artifacts, diagnostics}` result as the response envelope. This is exactly the body a `pure-reducer-world`
//! guest's `on-message` wraps — the returned bytes become the `close(closed{schema, reason})` reason (B3
//! wires the wit-bindgen export + componentization around this).

use crate::request_wire::decode_compile_request;
use cadenza_compile_abi::encode_compile_output;
use rcdzc::Target;

/// Handle a compile request. Decodes the kinded inputs (`message.payload`), compiles to a wasm component
/// (`Target::Wasm`), and returns the `{artifacts, diagnostics}` envelope bytes (the guest's close reason).
/// A failing program is not an error here — it rides in the envelope's diagnostics with no component
/// artifact (decline-don't-miscompile), so a caller reads success/failure from the decoded output, never a
/// trap. Targets default to `[Wasm]`: a platform reducer compiling for the platform wants a component.
pub fn handle(request_payload: &[u8]) -> Vec<u8> {
    let inputs = decode_compile_request(request_payload);
    let out = rcdzc::compile(&inputs, &[Target::Wasm]);
    encode_compile_output(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_wire::encode_compile_request;
    use cadenza_compile_abi::{decode_compile_output, Artifact};

    // Bridge source -> AST bytes the way the platform would feed a compile request (via cadenza-syntax's
    // codec, a dev-dep — exactly as rcdzc-wasm's tests do).
    fn ast_of(src: &str) -> Vec<u8> {
        let arenas = cadenza_syntax::sexpr::read(src).expect("test source parses");
        cadenza_syntax::codec::encode(&arenas)
    }

    #[test]
    fn compiles_a_valid_program_through_the_request_response_envelopes() {
        // The full guest body: a request bundle carrying one AST input -> a response envelope carrying a real
        // wasm component. This is the e2e-in-native form of the run round-trip (the wasm round-trip is B3).
        let ast = ast_of("(do (def (main) 42) (export main))");
        let request = encode_compile_request(&[Artifact::new(Artifact::KIND_AST, "main", ast)]);
        let out = decode_compile_output(&handle(&request));
        let component = out.artifact("component").unwrap_or_else(|| {
            panic!(
                "expected a component artifact; diags: {:?}",
                out.diagnostics
            )
        });
        assert_eq!(
            &component[..4],
            b"\0asm",
            "the response envelope carries a real wasm component"
        );
    }

    #[test]
    fn surfaces_diagnostics_for_a_bad_program_with_no_component() {
        // Decline-don't-miscompile: an unbound name yields NO component + an error diagnostic in the envelope,
        // not a trap — the caller reads the failure from the decoded output.
        let ast = ast_of("(do (def (main) undefined-name) (export main))");
        let request = encode_compile_request(&[Artifact::new(Artifact::KIND_AST, "main", ast)]);
        let out = decode_compile_output(&handle(&request));
        assert!(
            out.artifact("component").is_none(),
            "a bad program yields no component"
        );
        assert!(
            out.has_error(),
            "a bad program yields an error-severity diagnostic"
        );
    }
}
