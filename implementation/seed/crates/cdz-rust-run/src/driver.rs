//! Generate the RUST driver source spliced around an emitted `--target rust[-async]` module — the
//! crate-root host-call shim fns the emitted `mod prog` references (`crate::__cdz_host_<id>()`). Pure
//! string generation from a case's recorded host tape; no process/filesystem. Ported from `xtask`'s
//! `build_rust_host_shims` family. Later increments add the export-call assembly + the `rustc`/run.

use std::collections::{BTreeMap, BTreeSet};

use crate::sig::{is_env_param, parse_emitted_sig};

/// Marshal a call's canonical-sexp arg VALUES to the Rust expressions the emitted export expects. A scalar
/// passes through, a compound (`(tuple …)`/`(record …)`) is rebuilt by `rust_call_arg`. TYPE-AWARE for a
/// BIGINT param: a corpus arg is a BARE decimal (`5`) — its `(: 5 BigInt)` annotation is stripped by the
/// corpus parser, so unlike a self-identifying `"…"` String there is nothing telling `rust_call_arg` it
/// must cross as `cdz_num::Big` (emitted verbatim `5` is an i64 literal → rustc E0308 against `fn(a:
/// cdz_num::Big)`). So read the emitted fn's param TYPES off its signature and marshal a decimal arg to a
/// `cdz_num::Big` param via `big_arg_expr` (the owned-BigInt construction). Non-BigInt params, or a
/// non-decimal arg, keep the ordinary `rust_call_arg`.
pub fn marshal_call_args(
    module: &str,
    export: &str,
    args: &[String],
    async_mode: bool,
) -> Vec<String> {
    let name = cdz_rust_render::rust_ident(export);
    let arg_param_tys: Vec<Option<String>> = parse_emitted_sig(module, &name, async_mode)
        .map(|sig| {
            sig.params
                .iter()
                .filter(|p| !is_env_param(p))
                .map(|p| p.split_once(':').map(|(_, ty)| ty.trim().to_string()))
                .collect()
        })
        .unwrap_or_default();
    args.iter()
        .enumerate()
        .map(|(i, a)| {
            let is_bigint_param = arg_param_tys
                .get(i)
                .and_then(|o| o.as_deref())
                .is_some_and(|ty| ty == "cdz_num::Big");
            if is_bigint_param && let Ok(n) = a.trim().parse::<i128>() {
                cdz_rust_render::big_arg_expr(n)
            } else {
                cdz_rust_render::rust_call_arg(a)
            }
        })
        .collect()
}

/// The ORDINARY (non-factory, non-closure) call expression for an export — `<ident>(<marshaled args>)`,
/// the common corpus case (the factory/closure-consumer builders handle the host-closure subset). `args`
/// are canonical-sexp value texts; the emitted export's parameter types drive the marshal.
pub fn ordinary_call_expr(module: &str, export: &str, args: &[String], async_mode: bool) -> String {
    let name = cdz_rust_render::rust_ident(export);
    let marshaled = marshal_call_args(module, export, args, async_mode);
    format!("{name}({})", marshaled.join(", "))
}

/// Assemble the full RUST driver source for a SYNC, ORDINARY-call corpus case — the common path (the
/// host-closure factory/consumer application + async `block_on` harness are later increments). It wraps the
/// emitted `module` in `mod prog { … }` (so its `pub fn main` becomes `prog::main`, not a duplicate crate
/// `main`), splices the host-response shim fns (`build_rust_host_shims`), and emits a `fn main` that calls
/// `prog::<call>`, renders the result to cdz-run's canonical text via `cdz_render_expr` (driven by the
/// backend's `// cdz-*` render notes the module carries), and prints it. A DIVERGING export (`-> !` / `Any`
/// / `?N`) is just CALLED (it traps) — binding + printing a `!` is a build error, and the panic IS the
/// recorded `(trap …)` outcome; an export with no parsed `cdz-return` note falls back to `Display` (`{}`),
/// the scalar shape. Pure string generation; the result rendering + note parsing live in `cdz-rust-render`.
pub fn build_driver_source(
    module: &str,
    export: &str,
    args: &[String],
    host_responses: &[(String, String)],
    host_calls: &[String],
) -> String {
    use cdz_rust_render::{
        cdz_newtype_descriptors, cdz_qty_at, cdz_render_expr, cdz_return_type, cdz_scale,
        cdz_sum_descriptors, cdz_sum_params, cdz_sum_qualified_heads, cdz_unit_form,
    };

    let call_or_await = format!("prog::{}", ordinary_call_expr(module, export, args, false));
    let ret_ty = cdz_return_type(module, export);

    // A diverging result type (`-> !`) — its `cdz-return` note is `Any` / `!` / a bare `?N` result var.
    let diverging = ret_ty.as_deref().is_some_and(|t| {
        t == "Any" || t == "!" || (t.starts_with('?') && t[1..].chars().all(|c| c.is_ascii_digit()))
    });

    let body = if diverging {
        format!("fn main() {{ {call_or_await}; }}\n")
    } else {
        match ret_ty.as_deref() {
            Some(ty) => {
                let sums = cdz_sum_descriptors(module);
                let newtypes = cdz_newtype_descriptors(module);
                let sum_params = cdz_sum_params(module);
                let qualified_heads = cdz_sum_qualified_heads(module);
                let unit_form = cdz_unit_form(module, export);
                let unit_scale = cdz_scale(module, export);
                let qty_at = cdz_qty_at(module, export);
                let render = cdz_render_expr(
                    ty,
                    &sums,
                    &newtypes,
                    &sum_params,
                    unit_form.as_deref(),
                    unit_scale,
                    &qty_at,
                    &qualified_heads,
                );
                format!(
                    "fn main() {{ let __r = {call_or_await}; println!(\"{{}}\", {render}); }}\n"
                )
            }
            // Unknown return type (no emitted note) — fall back to `{}` (a scalar via Display).
            None => format!("fn main() {{ println!(\"{{}}\", {call_or_await}); }}\n"),
        }
    };

    let host_shims = build_rust_host_shims(module, host_responses, host_calls);
    format!("mod prog {{\n{module}\n}}\n{host_shims}{body}")
}

/// Kebab-normalize an EFFECT name (matching the backend's `canonical_host_op_key`): CamelCase / `_` / `-`
/// runs collapse to single `-`, lowercased, no leading/trailing `-`.
pub fn kebab_effect(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Derive the crate-root host-call shim fn ident from a recorded response key (`effect.op`) — kebab-normalize
/// the EFFECT part (matching the backend's `canonical_host_op_key`), keep the op verbatim, then map the
/// dotted key's non-ident chars → `_`. MUST equal the backend's emitted `host_shim_ident` for the same op.
pub fn host_shim_ident_from_key(op_key: &str) -> String {
    let (eff, op) = op_key.split_once('.').unwrap_or(("", op_key));
    let canonical = format!("{}.{}", kebab_effect(eff), op);
    let mut s = String::with_capacity(canonical.len() + 11);
    s.push_str("__cdz_host_");
    for c in canonical.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

/// Generate the crate-root host-call shim fns the emitted `mod prog` references (`crate::__cdz_host_<id>()`).
/// A shim is generated for EVERY distinct `__cdz_host_*` symbol the module names — including UNEXERCISED
/// defs — since every referenced symbol must be DEFINED or rustc E0425s at link. A symbol matched to
/// recorded responses (by the driver-derived ident, which kebab-normalizes the response-key effect to agree
/// with the backend) returns them in order + prints `host-call\t<recorded-op>`; a unit-result op prints its
/// op and returns `()`; an unmatched symbol gets a `panic!` stub (never reached on a passing trial).
pub fn build_rust_host_shims(
    module: &str,
    host_responses: &[(String, String)],
    host_calls: &[String],
) -> String {
    // Map recorded op key → (its CANONICAL dotted key for the host-call print, values in order), by shim
    // ident. The printed `host-call\t<op>` is the CANONICAL key (kebab-normalized effect + verbatim op), NOT
    // the raw recorded key — the grader compares observed vs expected by exact string, so a source-cased
    // response key (`Param.width`) must be normalized (`param.width`) before printing.
    let mut by_ident: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (op, value) in host_responses {
        let ident = host_shim_ident_from_key(op);
        by_ident
            .entry(ident)
            .or_insert_with(|| {
                let (eff, opname) = op.split_once('.').unwrap_or(("", op.as_str()));
                (format!("{}.{}", kebab_effect(eff), opname), Vec::new())
            })
            .1
            .push(value.clone());
    }
    // UNIT-RESULT ops (H8): a `(host-calls …)` entry whose op has NO `(host-response …)` is a pure effect op
    // that returns the unit value (crosses the boundary only to be OBSERVED — e.g. `log.emit`). It records
    // its call NAME but no response VALUE. Keyed by shim IDENT so an op that IS in host_responses under a
    // source-cased key still matches and is NOT mis-treated as unit-result.
    let response_idents: BTreeSet<String> = host_responses
        .iter()
        .map(|(op, _)| host_shim_ident_from_key(op))
        .collect();
    let mut unit_ops: BTreeMap<String, String> = BTreeMap::new();
    for op in host_calls {
        let ident = host_shim_ident_from_key(op);
        if response_idents.contains(&ident) {
            continue; // a VALUE-result op: handled via by_ident above.
        }
        unit_ops.insert(ident, op.clone());
    }
    // Every `crate::__cdz_host_<ident>(<args>)` the module references, with its ARG COUNT (the shim's fn
    // arity must match every call site or rustc E0061s). The backend emits args as simple `__ha0, __ha1, …`
    // idents (H3), so counting the `__ha` tokens in the call's paren group gives the arity reliably.
    let mut referenced: BTreeMap<String, usize> = BTreeMap::new();
    let mut rest = module;
    while let Some(pos) = rest.find("crate::__cdz_host_") {
        let after = &rest[pos + "crate::".len()..];
        let end = after
            .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
            .unwrap_or(after.len());
        let ident = after[..end].to_string();
        let arity = after[end..]
            .strip_prefix('(')
            .and_then(|s| s.find(')').map(|c| &s[..c]))
            .map(|argstr| argstr.matches("__ha").count())
            .unwrap_or(0);
        referenced.entry(ident).or_insert(arity);
        rest = &after[end..];
    }
    if referenced.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (fn_name, &arity) in &referenced {
        // The shim's params are GENERIC + ignored — the arg VALUES crossed the boundary but do not select
        // the response (host_responses is keyed per-op, arg-independent) and the corpus host-call sequence
        // compares the op NAME only. `<A0: …>(_a0: A0)` accepts ANY arg type so a String/Bytes arg (H7)
        // type-checks without the driver knowing arg types.
        let generics = (0..arity)
            .map(|i| format!("A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let generics = if generics.is_empty() {
            String::new()
        } else {
            format!("<{generics}>")
        };
        let params = (0..arity)
            .map(|i| format!("_a{i}: A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        match by_ident.get(fn_name) {
            Some((op, values)) => {
                // RETURN TYPE keyed on the recorded response value text (matches the backend's per-result-
                // kind read): a quoted "…" → `String`; a `.`-bearing non-bool → `f64`; else `i64` (bool
                // true/false → 1/0). The `__V` response table is that type; the shim hands out one per call.
                let all_quoted = values.iter().all(|v| {
                    let t = v.trim();
                    t.starts_with('"') && t.ends_with('"') && t.len() >= 2
                });
                let is_float = !all_quoted
                    && values
                        .iter()
                        .any(|v| v.trim().contains('.') && v.trim() != "true" && v.trim() != "false");
                let (ret_ty, arr, is_owned) = if all_quoted {
                    (
                        "String".to_string(),
                        values
                            .iter()
                            .map(|v| format!("{}.to_string()", v.trim()))
                            .collect::<Vec<_>>()
                            .join(", "),
                        true,
                    )
                } else if is_float {
                    (
                        "f64".to_string(),
                        values.iter().map(|v| v.trim().to_string()).collect::<Vec<_>>().join(", "),
                        false,
                    )
                } else {
                    (
                        "i64".to_string(),
                        values
                            .iter()
                            .map(|v| match v.trim() {
                                "true" => "1".to_string(),
                                "false" => "0".to_string(),
                                other => other.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        false,
                    )
                };
                let n = values.len();
                if is_owned {
                    // An owned (String/Vec) response can't live in a `static` array (non-const); build a
                    // fresh owned value per call, indexed by the call counter via a match.
                    let arms = values
                        .iter()
                        .enumerate()
                        .map(|(k, v)| format!("{k} => {}.to_string(),", v.trim()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         match __k {{ {arms} _ => unreachable!() }} }}\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         static __V: [{ret_ty}; {n}] = [{arr}]; \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         __V[__k] }}\n"
                    ));
                }
            }
            // A referenced shim with NO recorded response: (a) a UNIT-RESULT op (H8) — a `()`-returning shim
            // that prints its canonical op; or (b) an UNEXERCISED def — a panic stub (never reached on a
            // passing trial) so the artifact links.
            None => match unit_ops.get(fn_name) {
                Some(op) => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) {{ \
                     eprintln!(\"host-call\\t{op}\"); }}\n"
                )),
                None => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> i64 {{ panic!(\"unexercised host op {fn_name}\") }}\n"
                )),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_effect_normalizes() {
        assert_eq!(kebab_effect("Param"), "param");
        assert_eq!(kebab_effect("my_effect"), "my-effect");
        assert_eq!(kebab_effect("HttpClient"), "http-client");
        assert_eq!(kebab_effect(""), "");
    }

    #[test]
    fn shim_ident_matches_the_backend_mangling() {
        assert_eq!(
            host_shim_ident_from_key("Param.width"),
            "__cdz_host_param_width"
        );
        assert_eq!(host_shim_ident_from_key("io.log"), "__cdz_host_io_log");
    }

    #[test]
    fn value_response_shim_prints_canonical_op_and_returns_the_recorded_values() {
        let m = "let x = crate::__cdz_host_ask_ask();";
        let shims = build_rust_host_shims(m, &[("ask.ask".into(), "10".into())], &[]);
        assert!(
            shims.contains("fn __cdz_host_ask_ask"),
            "shim defined: {shims}"
        );
        assert!(
            shims.contains("host-call\\task.ask"),
            "prints the canonical op"
        );
        assert!(
            shims.contains("[10]") || shims.contains("[10 ]"),
            "returns the value: {shims}"
        );
        assert!(shims.contains("-> i64"), "int response → i64");
    }

    #[test]
    fn a_source_cased_response_key_prints_the_kebab_canonical_op() {
        let m = "crate::__cdz_host_param_width();";
        let shims = build_rust_host_shims(m, &[("Param.width".into(), "8".into())], &[]);
        // The IDENT is derived from the canonical form, so the source-cased key still matches the call site.
        assert!(shims.contains("fn __cdz_host_param_width"));
        assert!(
            shims.contains("host-call\\tparam.width"),
            "printed op is kebab-canonical: {shims}"
        );
    }

    #[test]
    fn a_unit_result_op_gets_a_unit_shim_that_prints_its_op() {
        let m = "crate::__cdz_host_log_emit(__ha0);";
        let shims = build_rust_host_shims(m, &[], &["log.emit".into()]);
        assert!(shims.contains("fn __cdz_host_log_emit"));
        assert!(shims.contains("host-call\\tlog.emit"));
        assert!(
            !shims.contains("-> i64"),
            "unit shim has no return type: {shims}"
        );
        assert!(
            shims.contains("<A0>"),
            "arity-1 shim is generic over its arg"
        );
    }

    #[test]
    fn an_unexercised_referenced_shim_is_a_panic_stub() {
        let m = "if false { crate::__cdz_host_dead_op(); }";
        let shims = build_rust_host_shims(m, &[], &[]);
        assert!(
            shims.contains("panic!(\"unexercised host op __cdz_host_dead_op\")"),
            "{shims}"
        );
    }

    #[test]
    fn no_referenced_shims_is_empty() {
        assert_eq!(build_rust_host_shims("fn main() {}", &[], &[]), "");
    }

    #[test]
    fn a_quoted_response_returns_owned_string_via_match() {
        let m = "crate::__cdz_host_ask_name();";
        let shims = build_rust_host_shims(m, &[("ask.name".into(), "\"hi\"".into())], &[]);
        assert!(shims.contains("-> String"), "quoted → String: {shims}");
        assert!(shims.contains(".to_string()"));
    }

    #[test]
    fn ordinary_call_marshals_scalar_and_compound_args() {
        let m = "pub fn f(a: i64, b: (i64, i64)) -> i64 { a }";
        assert_eq!(
            ordinary_call_expr(m, "f", &["20".into(), "(tuple 7 9)".into()], false),
            "f(20, (7, 9))"
        );
    }

    #[test]
    fn a_bigint_param_marshals_a_bare_decimal_via_big_arg_expr() {
        // A `cdz_num::Big` param must NOT get the bare `5` (i64 literal → E0308) — it goes through
        // big_arg_expr. A non-BigInt param keeps the verbatim scalar.
        let m = "pub fn f(a: cdz_num::Big, b: i64) -> i64 { b }";
        let got = marshal_call_args(m, "f", &["5".into(), "7".into()], false);
        assert_ne!(
            got[0], "5",
            "the BigInt arg is not a bare i64 literal: {got:?}"
        );
        assert!(
            got[0].contains("Big") || got[0].contains("big"),
            "BigInt marshal: {got:?}"
        );
        assert_eq!(got[1], "7", "the i64 arg passes through verbatim");
    }

    #[test]
    fn a_nullary_export_call_has_empty_args() {
        assert_eq!(
            ordinary_call_expr("pub fn main() -> i64 { 42 }", "main", &[], false),
            "main()"
        );
    }

    #[test]
    fn driver_wraps_the_module_and_calls_prog_main() {
        let m = "pub fn main() -> i64 { 42 }";
        let d = build_driver_source(m, "main", &[], &[], &[]);
        assert!(d.contains("mod prog {"), "wraps in mod prog: {d}");
        assert!(d.contains(m), "embeds the module");
        assert!(
            d.contains("fn main()") && d.contains("prog::main()"),
            "calls prog::main: {d}"
        );
        assert!(d.contains("println!"), "prints the result");
    }

    #[test]
    fn driver_renders_via_cdz_return_note_when_present() {
        let m = "// cdz-return[f]: Int64\npub fn f() -> i64 { 7 }";
        let d = build_driver_source(m, "f", &[], &[], &[]);
        assert!(d.contains("let __r = prog::f()"), "binds the result: {d}");
        assert!(d.contains("println!"), "prints it");
    }

    #[test]
    fn a_diverging_export_is_just_called_no_println() {
        let m = "// cdz-return[boom]: !\npub fn boom() -> ! { panic!() }";
        let d = build_driver_source(m, "boom", &[], &[], &[]);
        assert!(d.contains("prog::boom()"), "calls it: {d}");
        assert!(!d.contains("println!"), "diverging → no print: {d}");
        assert!(!d.contains("let __r"), "diverging → no result binding: {d}");
    }

    #[test]
    fn driver_splices_host_shims() {
        let m = "// cdz-return[g]: Int64\npub fn g() -> i64 { crate::__cdz_host_ask_ask() }";
        let d = build_driver_source(m, "g", &[], &[("ask.ask".into(), "10".into())], &[]);
        assert!(d.contains("fn __cdz_host_ask_ask"), "shim spliced: {d}");
    }
}
