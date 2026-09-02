//! Parse the EMITTED Rust function signatures out of a `--target rust[-async]` module — the arrow-aware
//! param-list walk the driver generation + call marshalling build on (a closure-typed param
//! `g: Rc<dyn Fn(i64) -> i64>` is ONE param, not split at its inner `->`/`,`). Pure string analysis over
//! the emitted source; no process or filesystem. Ported from `xtask`'s `parse_emitted_sig` family.

/// The parsed shape of an emitted `pub fn <name>(…) -> <ret>` signature.
pub struct EmittedSig<'a> {
    /// Each top-level parameter's verbatim `<name>: <type>` text, in source order. Empty for a nullary fn.
    /// The env param (`__cdz_env: &mut dyn DynCdzEnv`) is INCLUDED here (callers that care filter it).
    pub params: Vec<&'a str>,
    /// The return-type text (up to the fn body `{`).
    pub ret_head: String,
}

/// Parse the emitted signature of `name` (the SOURCE-level export ident) out of `module`. Returns the
/// param-list (arrow-aware split, so a closure-typed param `g: Rc<dyn Fn(i64) -> i64>` is ONE param, not
/// split at its inner `->`/`,`) and the return-type head. `None` if no such exported fn header is found or
/// its param list is malformed.
pub fn parse_emitted_sig<'a>(
    module: &'a str,
    name: &str,
    async_mode: bool,
) -> Option<EmittedSig<'a>> {
    let marker = if async_mode {
        "pub async fn "
    } else {
        "pub fn "
    };
    // Find the exact `<marker><name>` header. The name boundary matters: a bare `split` on `pub fn both`
    // also matches `pub fn both2(` (prefix), so a MULTI-export module grabs the wrong occurrence. Only an
    // occurrence whose next char starts the param list `(` (sync) or the generic list `<` (async) — never
    // an identifier-continuation char — is the real header.
    let needle = format!("{marker}{name}");
    let after = module
        .match_indices(&needle)
        .map(|(i, _)| &module[i + needle.len()..])
        .find(|rest| matches!(rest.chars().next(), Some('(') | Some('<')))?;
    // Skip an async generic-parameter list `<…>` if present, to reach the param-list `(`.
    let after = after.trim_start();
    let after = if after.starts_with('<') {
        &after[after.find('>').map(|i| i + 1)?..]
    } else {
        after
    };
    let after = after.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    // Walk the param list, tracking nesting depth so a `(…)`/`<…>` inside a param TYPE isn't miscounted,
    // and recording each top-level comma so params can be split. A `>` closes an angle group EXCEPT the
    // `>` of a `->` return arrow (which appears INSIDE the list when a param type is itself a closure,
    // `g: Rc<dyn Fn(i64) -> i64>`); counting it as a close underflows depth so the list's own `)` never
    // returns to depth 0 → the slice below would panic `begin > end`. Guard: a `>` immediately preceded by
    // `-` is an arrow, not a bracket close.
    let bytes = after.as_bytes();
    let mut depth = 0usize;
    let mut end = 0usize;
    let mut comma_positions = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            b'>' if i == 0 || bytes[i - 1] != b'-' => depth = depth.saturating_sub(1),
            b',' if depth == 1 => comma_positions.push(i),
            _ => {}
        }
    }
    // If the walk never found the param-list close (`end` still 0 — a malformed/unexpected shape), this is
    // not a signature we can analyze: return None rather than slicing `&after[1..0]` (panics `begin > end`).
    if end == 0 {
        return None;
    }
    // Split the param list into top-level params at the recorded commas (indices into `after`; the list
    // runs `1..end` past the leading `(`). An empty list (nullary fn) → no params.
    let params: Vec<&str> = if after[1..end].trim().is_empty() {
        Vec::new()
    } else {
        let mut parts = Vec::new();
        let mut start = 1usize; // just past the `(`
        for &c in &comma_positions {
            parts.push(after[start..c].trim());
            start = c + 1;
        }
        parts.push(after[start..end].trim());
        parts
    };
    let ret_head: String = after[end + 1..].chars().take_while(|&c| c != '{').collect();
    Some(EmittedSig { params, ret_head })
}

/// Whether a parameter slice (`<name>: <type>`) is the async gas/yield env param — backend plumbing, not
/// a source param. The uniform-env change retired the generic env TYPE-PARAM (`__CdzE`); the current
/// rcdzc rust backend (`backend/rust/mod.rs` `ENV_PARAM`) emits `__cdz_env: &mut dyn DynCdzEnv` — an
/// object-safe env (its RPITIT `consume` reached via the `DynCdzEnv::consume_boxed` facet). Matched by
/// the `__cdz_env` NAME, which is stable across the type change.
pub fn is_env_param(param: &str) -> bool {
    param.trim_start().starts_with("__cdz_env")
}

/// Whether a type string names a runtime closure VALUE — either the SYNC `Rc<dyn Fn(…)>` or the ASYNC
/// (Option A) `Rc<dyn cdz_rt::EnvClosure<A, R>>` (a lifted async closure crosses as an `EnvClosure` trait
/// object, not a `dyn Fn`, since its `call` future borrows the `&mut env`). Both closure-detection sites
/// (a PARAM type, a factory RESULT type) key off this so the async host-closure cases are recognized as
/// factories/consumers/producers exactly like the sync ones.
pub fn names_closure_value(ty: &str) -> bool {
    ty.contains("Rc<dyn Fn(")
        || ty.contains("Rc<dyn cdz_rt::EnvClosure<")
        || ty.contains("Rc<dyn EnvClosure<")
}

/// Whether a parameter slice (`<name>: <type>`) is a closure — sync `Rc<dyn Fn(…)>` or async `Rc<dyn
/// EnvClosure<…>>`.
pub fn is_closure_param(param: &str) -> bool {
    names_closure_value(param)
}

/// The closure TYPE of a parameter slice (`g: std::rc::Rc<dyn Fn(i64) -> i64>`) → the `Rc<dyn Fn…>` text,
/// or `None` if the param is not a closure. Extracts the BALANCED `Rc<…>` (stops at the angle bracket that
/// matches the opening `<` of `Rc<`), so a param that is NOT last in the list (`g: Rc<dyn Fn(i64)->i64>,
/// x: i64`) yields ONLY the closure type. This matters for a HIGHER-ORDER producer: a substring-tolerant
/// match would false-pair a first-order consumer param `Rc<dyn Fn(i64)->i64>` to a higher-order producer
/// `Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->i64>` (the former is a substring of the latter), so the pairing must
/// compare EXACT balanced closure types.
pub fn closure_param_type(param: &str) -> Option<&str> {
    let start = closure_spelling_pos(param)?;
    let rest = &param[start..];
    // Find the `<` that opens `Rc<` and walk to its MATCHING `>` (depth-balanced over `<`/`>`), so a nested
    // `Rc<dyn Fn(Rc<…>)…>` returns its whole self and a trailing `, x: i64` is excluded. CRITICAL: the
    // return arrow `->` contains a `>` that must NOT be counted as a closing angle bracket — skip a `>`
    // immediately preceded by `-` (matching Rust's `->` in the emitted `Rc<dyn Fn(A) -> R>`).
    let open = rest.find('<')?;
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    for (i, c) in rest.char_indices().skip(open) {
        match c {
            '<' => depth += 1,
            '>' if i > 0 && bytes[i - 1] == b'-' => {} // the `>` of a `->` return arrow — not a bracket
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=i].trim());
                }
            }
            _ => {}
        }
    }
    Some(rest.trim())
}

/// The TYPE of a parameter slice `<name>: <type>` → the `<type>` text (everything after the first `:`).
pub fn param_type_of(param: &str) -> String {
    match param.split_once(':') {
        Some((_, ty)) => ty.trim().to_string(),
        None => param.trim().to_string(),
    }
}

/// The closure type out of a return-head — sync `-> …Rc<dyn Fn(i64) -> i64>` or async `-> …Rc<dyn
/// cdz_rt::EnvClosure<i64, i64>>`. `None` if the return is not a closure.
pub fn closure_ret_type(ret_head: &str) -> Option<String> {
    let start = closure_spelling_pos(ret_head)?;
    Some(ret_head[start..].trim().to_string())
}

/// The byte position of the first runtime-closure type spelling in `s` — sync `Rc<dyn Fn(…)>` or async
/// `Rc<dyn …EnvClosure<…>>`, qualified (`std::rc::Rc<…`) or bare — or `None`. Both closure-type extractors
/// ([`closure_param_type`], [`closure_ret_type`]) scan for this same spelling set.
fn closure_spelling_pos(s: &str) -> Option<usize> {
    s.find("std::rc::Rc<dyn Fn(")
        .or_else(|| s.find("std::rc::Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| s.find("Rc<dyn Fn("))
        .or_else(|| s.find("Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| s.find("Rc<dyn EnvClosure<"))
}

/// A FACTORY (producer) export's CAPTURE-param count — its source params minus the async env param — or
/// `None` if `name` is not a factory (its return type is not a closure). Detecting the env param by NAME
/// (not a blind `-1`) keeps a hand-authored env-less async fixture correct.
pub fn rust_factory_param_count(module: &str, name: &str, async_mode: bool) -> Option<usize> {
    let sig = parse_emitted_sig(module, name, async_mode)?;
    if !names_closure_value(&sig.ret_head) {
        return None;
    }
    Some(sig.params.iter().filter(|p| !is_env_param(p)).count())
}

/// Peel a CURRIED arrow type down to its final (non-arrow) RESULT — `(-> Int64 (-> Int64 (Tuple Int64
/// Int64)))` → `(Tuple Int64 Int64)`. A host-closure factory's `cdz-return` note is the returned closure's
/// arrow; the driver applies the factory to full arity, so the rendered value is this final result, not the
/// arrow. A non-arrow type is returned unchanged. Balanced-paren aware: the arrow is `(-> <arg> <rest>)`
/// where `<arg>` may itself be a parenthesized compound, so skip the first top-level sub-term and take
/// `<rest>`, recursing while `<rest>` is itself a `(-> …)`. Ported from xtask's `peel_arrow_result`.
pub fn peel_arrow_result(ty: &str) -> String {
    let mut cur = ty.trim();
    loop {
        let inner = match cur.strip_prefix("(-> ") {
            Some(i) => i.trim_end().strip_suffix(')').map(str::trim),
            None => None,
        };
        let Some(inner) = inner else {
            return cur.to_string();
        };
        // `inner` = `<arg> <rest>`. Skip the first top-level term (`<arg>`), balancing parens.
        let bytes = inner.as_bytes();
        let mut depth = 0usize;
        let mut split = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b' ' if depth == 0 => {
                    split = Some(i);
                    break;
                }
                _ => {}
            }
        }
        match split {
            Some(i) => cur = inner[i + 1..].trim(),
            None => return cur.to_string(),
        }
    }
}

/// Marshal an async FACTORY's flat APPLIED args into the SINGLE argument an `EnvClosure::call(&mut env, a)`
/// takes. The lifted async closure convention (matching the emit's `CallClosure`) collapses a multi-arg
/// closure application into one tuple `A`: 0 applied args → the unit `()`, exactly 1 → the bare arg, ≥2 →
/// a tuple `(a, b, …)`. The comma count is TOP-LEVEL only — a compound argument (`(3, 4)`, `Foo { … }`, an
/// `x as Rc<dyn Fn(A) -> R>` coercion) is one arg, so its inner commas at depth > 0 don't split it. A `>`
/// preceded by `-` is the `->` arrow of a closure-typed arg, NOT a `<` group close, so it is not depth-
/// decremented (mirroring the emitted `Rc<dyn Fn(A) -> R>` spelling). Ported from xtask `env_closure_call_arg`.
pub(crate) fn env_closure_call_arg(applied: &str) -> String {
    let applied = applied.trim();
    if applied.is_empty() {
        return "()".to_string();
    }
    let bytes = applied.as_bytes();
    let mut depth = 0usize;
    let mut n_top_commas = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            b'>' if i > 0 && bytes[i - 1] == b'-' => {} // the `>` of a `->` arrow, not a group close
            b')' | b'>' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => n_top_commas += 1,
            _ => {}
        }
    }
    if n_top_commas == 0 {
        applied.to_string() // 1 arg → the bare arg
    } else {
        format!("({applied})") // ≥2 args → a tuple of them
    }
}

/// Split a FACTORY call expression `export(caps…)(applied…)` into `("export(caps…)", "(applied…)")` at the
/// boundary between the factory's own arg group and the returned-closure application. `None` when there is
/// no top-level application group (a non-factory `export(args…)`, or a factory whose closure is not
/// applied). The split is the FIRST `)` at paren-depth 0 immediately followed by `(` — a nested `)(` inside
/// a compound argument sits at depth > 0 and is skipped, so only the real factory/application seam matches.
pub fn split_factory_application(call_expr: &str) -> Option<(String, String)> {
    let bytes = call_expr.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 && call_expr[i + 1..].starts_with('(') {
                    return Some((call_expr[..=i].to_string(), call_expr[i + 1..].to_string()));
                }
            }
            _ => {}
        }
    }
    None
}

/// The SOLE export's source name — the fn to call for a corpus case with no `(call …)` (the common
/// nullary-entry shape). Recovered from the emitted signature: sync mode emits `pub fn <name>(`, async
/// mode `pub async fn <name>(__cdz_env: &mut dyn DynCdzEnv, …)` (uniform-env: no generic type-param), so
/// split on whichever marker is present and stop the name at `(` — OR `<`, retained defensively for a
/// would-be generic list. `None` if no exported fn header is present. MUST match `parse_emitted_sig`'s
/// marker so the recovered name parses back.
pub fn sole_export_name(module: &str, async_mode: bool) -> Option<String> {
    let marker = if async_mode {
        "pub async fn "
    } else {
        "pub fn "
    };
    // Match a MODULE-LEVEL `pub fn` only — anchor the marker at the START of a line. A plain `.split(marker)`
    // also matches an INDENTED impl METHOD (`    pub fn get(self) -> f64` on the `__CdzF{N}` ord-key wrapper,
    // emitted for a float-keyed Set/Map), which precedes `pub fn main` in the module — so `.nth(1)` grabbed
    // `get` and the driver called `prog::get(..)` → E0425 (a method is not a free fn). The exported entry
    // points are always top-level (col 0); impl methods are indented, so a per-line `strip_prefix` skips them.
    // (Mirrors `cdz`'s own `emitted_pub_fn_names`, which anchors the same way.)
    module
        .lines()
        .find_map(|line| line.strip_prefix(marker))
        .map(|s| s.split(['(', '<']).next().unwrap_or("").trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_nullary_sync_export() {
        let m = "pub fn main() -> i64 { 42 }";
        let s = parse_emitted_sig(m, "main", false).expect("found");
        assert!(s.params.is_empty());
        assert_eq!(s.ret_head.trim(), "-> i64");
    }

    #[test]
    fn parses_params_and_return() {
        let m = "pub fn add(a: i64, b: i64) -> i64 { a + b }";
        let s = parse_emitted_sig(m, "add", false).expect("found");
        assert_eq!(s.params, vec!["a: i64", "b: i64"]);
        assert_eq!(s.ret_head.trim(), "-> i64");
    }

    #[test]
    fn a_closure_typed_param_is_one_param_not_split_at_its_inner_arrow_or_comma() {
        // The `->` inside `Fn(i64) -> i64` must NOT close the param list, and there is no top-level comma.
        let m = "pub fn apply_it(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { g(x) }";
        let s = parse_emitted_sig(m, "apply_it", false).expect("found");
        assert_eq!(
            s.params,
            vec!["g: std::rc::Rc<dyn Fn(i64) -> i64>", "x: i64"]
        );
    }

    #[test]
    fn a_name_that_is_a_prefix_of_another_export_is_not_mismatched() {
        // `both` must not match the `both2` header first.
        let m = "pub fn both2(a: i64) -> i64 { a }\npub fn both(x: i64) -> i64 { x }";
        let s = parse_emitted_sig(m, "both", false).expect("found the exact header");
        assert_eq!(s.params, vec!["x: i64"]);
    }

    #[test]
    fn sole_export_skips_an_indented_impl_method() {
        // A float-keyed Set/Map emits the `__CdzF{N}` ord-wrapper whose `pub fn get` PRECEDES `pub fn main`.
        // `sole_export_name` must return the MODULE-LEVEL export `main`, NOT the indented impl method `get`
        // (a `.split("pub fn ")` grabbed `get` → the driver called `prog::get(..)` → E0425). The anchor is
        // that a module-level export is at col 0 while an impl method is indented.
        let m = "#[derive(Clone, Copy)]\npub struct __CdzF64(u64);\nimpl __CdzF64 {\n    fn new(v: f64) -> Self { __CdzF64(v.to_bits()) }\n    pub fn get(self) -> f64 { f64::from_bits(self.0) }\n}\npub fn main() -> i64 { 2 }";
        assert_eq!(sole_export_name(m, false).as_deref(), Some("main"));
    }

    #[test]
    fn async_header_parses_the_dyn_env_and_source_params() {
        // CURRENT shape (uniform-env: no generic env type-param, env is `&mut dyn DynCdzEnv`).
        let m = "pub async fn f(__cdz_env: &mut dyn DynCdzEnv, n: i64) -> i64 { n }";
        let s = parse_emitted_sig(m, "f", true).expect("found");
        assert_eq!(s.params.len(), 2);
        assert!(is_env_param(s.params[0]), "first param is the env plumbing");
        assert!(!is_env_param(s.params[1]));
        // DEFENSIVE: the retired generic-env header (`<E: CdzEnv>(__cdz_env: &mut E, …)`) is still
        // tolerated — the `<…>`-skip is a harmless no-op if the emit ever re-introduced a generic list.
        let legacy = "pub async fn f<E: CdzEnv>(__cdz_env: &mut E, n: i64) -> i64 { n }";
        assert_eq!(
            parse_emitted_sig(legacy, "f", true)
                .expect("found")
                .params
                .len(),
            2
        );
    }

    #[test]
    fn a_missing_export_is_none() {
        assert!(parse_emitted_sig("pub fn a() {}", "nope", false).is_none());
    }

    #[test]
    fn env_param_recognized_by_name() {
        // Matched by the `__cdz_env` NAME (stable across the env-type change to `&mut dyn DynCdzEnv`).
        assert!(is_env_param("__cdz_env: &mut dyn DynCdzEnv"));
        assert!(!is_env_param("x: i64"));
    }

    #[test]
    fn closure_value_detection_sync_and_async() {
        assert!(names_closure_value("std::rc::Rc<dyn Fn(i64) -> i64>"));
        assert!(names_closure_value("Rc<dyn cdz_rt::EnvClosure<i64, i64>>"));
        assert!(!names_closure_value("i64"));
        assert!(is_closure_param("g: Rc<dyn Fn(i64) -> i64>"));
        assert!(!is_closure_param("x: i64"));
    }

    #[test]
    fn closure_param_type_is_balanced_not_substring() {
        // A trailing `, x: i64` is excluded; the `->` inside is not miscounted as a close.
        let p = "g: std::rc::Rc<dyn Fn(i64) -> i64>";
        assert_eq!(
            closure_param_type(p),
            Some("std::rc::Rc<dyn Fn(i64) -> i64>")
        );
        // Higher-order: the whole nested Rc<…> is returned, not the inner one.
        let ho = "g: Rc<dyn Fn(Rc<dyn Fn(i64) -> i64>) -> i64>";
        assert_eq!(
            closure_param_type(ho),
            Some("Rc<dyn Fn(Rc<dyn Fn(i64) -> i64>) -> i64>")
        );
        assert_eq!(closure_param_type("x: i64"), None);
    }

    #[test]
    fn param_type_and_closure_ret() {
        assert_eq!(
            param_type_of("g: Rc<dyn Fn(i64)->i64>"),
            "Rc<dyn Fn(i64)->i64>"
        );
        assert_eq!(
            closure_ret_type("-> std::rc::Rc<dyn Fn(i64) -> i64>").as_deref(),
            Some("std::rc::Rc<dyn Fn(i64) -> i64>")
        );
        assert_eq!(closure_ret_type("-> i64"), None);
    }

    #[test]
    fn factory_param_count_counts_captures_minus_env() {
        // A factory: return type is a closure; two capture params (a, b), env excluded.
        let m = "pub fn both(a: i64, b: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { todo!() }";
        assert_eq!(rust_factory_param_count(m, "both", false), Some(2));
        // Async: the env param is not a capture.
        let am = "pub async fn f(__cdz_env: &mut dyn DynCdzEnv, a: i64) -> Rc<dyn cdz_rt::EnvClosure<i64,i64>> { todo!() }";
        assert_eq!(rust_factory_param_count(am, "f", true), Some(1));
        // A non-factory (scalar return) is None.
        assert_eq!(
            rust_factory_param_count("pub fn g(a: i64) -> i64 { a }", "g", false),
            None
        );
    }

    #[test]
    fn env_closure_call_arg_tuples_top_level_args_only() {
        // 0 args → unit; 1 → bare; ≥2 → a tuple (matching the lifted EnvClosure::call `A`).
        assert_eq!(env_closure_call_arg(""), "()");
        assert_eq!(env_closure_call_arg("5"), "5");
        assert_eq!(env_closure_call_arg("3, 4"), "(3, 4)");
        // A compound arg's inner commas are at depth > 0 → not a top-level split.
        assert_eq!(env_closure_call_arg("a, (x, y)"), "(a, (x, y))");
        assert_eq!(env_closure_call_arg("(1, 2)"), "(1, 2)");
        assert_eq!(
            env_closure_call_arg("Foo { a: 1, b: 2 }"),
            "Foo { a: 1, b: 2 }"
        );
        // A `->` arrow inside a closure-typed coercion is not a `<` group close.
        assert_eq!(
            env_closure_call_arg("x as Rc<dyn Fn(i64) -> (i64, i64)>"),
            "x as Rc<dyn Fn(i64) -> (i64, i64)>"
        );
    }

    #[test]
    fn factory_application_seam_split() {
        assert_eq!(
            split_factory_application("both(10, 20)(5)"),
            Some(("both(10, 20)".to_string(), "(5)".to_string()))
        );
        // A nested `)(` inside a compound arg is at depth > 0 → not the seam.
        assert_eq!(
            split_factory_application("f((tuple 1 2), (record x))"),
            None
        );
        // A plain non-factory call has no application group.
        assert_eq!(split_factory_application("f(1, 2)"), None);
    }

    #[test]
    fn peel_arrow_result_takes_the_final_type() {
        assert_eq!(peel_arrow_result("(-> Int64 Int64)"), "Int64");
        assert_eq!(
            peel_arrow_result("(-> Int64 (-> Int64 (Tuple Int64 Int64)))"),
            "(Tuple Int64 Int64)"
        );
        // A compound ARG is skipped as one term.
        assert_eq!(peel_arrow_result("(-> (Tuple Int64 Int64) Int64)"), "Int64");
        // A non-arrow type is unchanged.
        assert_eq!(peel_arrow_result("Int64"), "Int64");
    }

    #[test]
    fn sole_export_name_recovers_the_entry() {
        assert_eq!(
            sole_export_name("// note\npub fn main() -> i64 { 42 }", false).as_deref(),
            Some("main")
        );
        // Async: the current header is `pub async fn run(__cdz_env: &mut dyn DynCdzEnv)` — the name
        // stops at `(`. (The `<`-stop is retained defensively for a would-be generic list.)
        assert_eq!(
            sole_export_name("pub async fn run(__cdz_env: &mut dyn DynCdzEnv) {}", true).as_deref(),
            Some("run")
        );
        // No exported fn → None.
        assert_eq!(sole_export_name("fn helper() {}", false), None);
    }
}
