//! Parse the EMITTED Rust function signatures out of a `--target rust[-async]` module — the arrow-aware
//! param-list walk the driver generation + call marshalling build on (a closure-typed param
//! `g: Rc<dyn Fn(i64) -> i64>` is ONE param, not split at its inner `->`/`,`). Pure string analysis over
//! the emitted source; no process or filesystem. Ported from `xtask`'s `parse_emitted_sig` family.

/// The parsed shape of an emitted `pub fn <name>(…) -> <ret>` signature.
pub struct EmittedSig<'a> {
    /// Each top-level parameter's verbatim `<name>: <type>` text, in source order. Empty for a nullary fn.
    /// The env param (`__cdz_env: &mut __CdzE`) is INCLUDED here (callers that care filter it).
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
/// a source param. Its emitted names mirror the rcdzc rust backend's `ENV_PARAM`/`ENV_TYPE_PARAM`
/// (`backend/rust/mod.rs`): value `__cdz_env`, type `__CdzE`.
pub fn is_env_param(param: &str) -> bool {
    param.trim_start().starts_with("__cdz_env") || param.contains("&mut __CdzE")
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
    fn async_header_skips_the_generic_list_to_the_params() {
        let m = "pub async fn f<E: CdzEnv>(__cdz_env: &mut E, n: i64) -> i64 { n }";
        let s = parse_emitted_sig(m, "f", true).expect("found");
        assert_eq!(s.params.len(), 2);
        assert!(is_env_param(s.params[0]), "first param is the env plumbing");
        assert!(!is_env_param(s.params[1]));
    }

    #[test]
    fn a_missing_export_is_none() {
        assert!(parse_emitted_sig("pub fn a() {}", "nope", false).is_none());
    }

    #[test]
    fn env_param_recognized_by_name_or_type() {
        assert!(is_env_param("__cdz_env: &mut E"));
        assert!(is_env_param("e: &mut __CdzE"));
        assert!(!is_env_param("x: i64"));
    }
}
