//! `cdz-nativize` — M3 guide-source nativization aid (THROWAWAY; deleted at M3 Phase-2 completion).
//!
//! Reads ONE s-expr program source on stdin and writes it back on stdout with every name-head compound
//! LITERAL/PATTERN — `(list …)`/`(tuple …)`/`(record …)`/`(set …)`/`(map …)` — rewritten to the native
//! `#word(…)` surface (see `cadenza_syntax_sexpr::nativize_compound_source`). HOF/shadow-guarded (a
//! `(map f xs)` HOF call or a `let`/`fn`/`def`-bound ctor name is left name-head), surface-preserving, and
//! it handles a BARE multi-form snippet (not only a full `(do … (export …))` module).
//!
//! For v-guide-infra's guide-source run: pipe each extracted example source through it, e.g.
//!   nix develop -c cargo run -q -p cadenza-syntax-sexpr --bin cdz-nativize < in.sexp > out.sexp
//! On a parse error it writes a message to stderr and exits 1 (leaving stdout empty), so a driver can
//! detect + skip a source that does not parse (rather than emit a truncated result).
//!
//! `--skip-outputs`: the CORPUS inputs-only mode — nativize `(input …)` programs + `(call …)` argument
//! values but leave every `(output …)` expected value untouched (v-corpus-harness owns the render re-pin
//! of expected outputs; a text-nativize would not match the gate's normalizing render). Use this to
//! nativize a whole corpus `.sexp` file's INPUT side without clobbering the output re-pin (Phase-2 seq A).

use std::io::{Read, Write};

fn main() {
    let skip_outputs = std::env::args().any(|a| a == "--skip-outputs");
    let mut src = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("cdz-nativize: failed to read stdin: {e}");
        std::process::exit(2);
    }
    let result = if skip_outputs {
        cadenza_syntax_sexpr::nativize_compound_source_skip_outputs(&src)
    } else {
        cadenza_syntax_sexpr::nativize_compound_source(&src)
    };
    match result {
        Ok(out) => {
            if let Err(e) = std::io::stdout().write_all(out.as_bytes()) {
                eprintln!("cdz-nativize: failed to write stdout: {e}");
                std::process::exit(2);
            }
        }
        Err(e) => {
            eprintln!(
                "cdz-nativize: parse error (source left unmodified): {}",
                e.0
            );
            std::process::exit(1);
        }
    }
}
