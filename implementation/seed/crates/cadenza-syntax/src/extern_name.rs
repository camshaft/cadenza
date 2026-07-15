//! The component-model extern name a source export/import name crosses under.
//!
//! A Cadenza identifier is broader than a component-model KEBAB-CASE extern name: it may contain
//! uppercase letters (`fA`, `Foo`), underscores (`my_func`), and camelCase runs (`myFunc`) — all valid
//! source names. The component model, however, requires an export's extern name to be kebab-case
//! (lowercase words, hyphen-separated: `[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*`). Emitting a non-kebab name
//! verbatim produces a component that fails to validate ("export name `fA` is not a valid extern name")
//! — an unloadable artifact. So a non-kebab source name is NORMALIZED to a valid kebab extern name at
//! every component-name site, and the runner maps a requested call name through the same rule.
//!
//! The mapping is DETERMINISTIC (a pure function of the name), so the compiler and the runner agree
//! without threading a table across the boundary. Two DISTINCT source names that normalize to the SAME
//! extern name is a collision the compiler must reject (`kebab_extern_name` is the shared rule; the
//! collision check lives at the export-planning site that sees the whole export set).

/// The kebab-case component extern name for a source identifier `name`. Already-kebab names (lowercase,
/// hyphen-separated, with trailing digits) are returned UNCHANGED, so the common case is identity and
/// byte-for-byte stable. The normalization:
///   * an UPPERCASE letter begins a new word — a `-` separator is inserted before it (unless the output
///     is empty or already ends in `-`), then it is lowercased (`fA` → `f-a`, `myFunc` → `my-func`,
///     `Foo` → `foo`);
///   * an UNDERSCORE `_` becomes a `-` word separator (`my_func` → `my-func`);
///   * a `-`, a lowercase letter, or a digit is kept as-is;
///   * runs of separators are collapsed and leading/trailing separators trimmed, so the result is a
///     well-formed kebab name (no `--`, no edge `-`).
///
/// A source identifier always starts with a letter (a digit-led token is a numeric literal, rejected
/// earlier), so the result always starts with a letter — a valid kebab word.
pub fn kebab_extern_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            // A new word: separate from a preceding word, then lowercase.
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == '-' {
            // A word separator — collapse a run to a single `-`.
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            // A lowercase letter or a digit — kept verbatim.
            out.push(c);
        }
    }
    // Trim a trailing separator (a name ending in `_`/`-`/an uppercase-then-nothing can't, but be safe).
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Whether `name` is ALREADY a valid kebab-case extern name — i.e. `kebab_extern_name` is the identity
/// on it. Used where the check reads more clearly than comparing the normalized form.
pub fn is_kebab_extern_name(name: &str) -> bool {
    kebab_extern_name(name) == name
}

/// Whether `word` is a single valid component-model KEBAB word — a `-`-separated run of
/// same-case-led alphanumeric words (`a`, `a-b`, `foo2`, `HTTP`, `parse-json`), non-empty and not
/// ending in `-`. This mirrors `wasmparser`'s `KebabStr` grammar EXACTLY (the state machine that
/// decides whether the component validator accepts a segment), so a name this function accepts loads
/// under wasmtime and one it rejects does not — the whole point of validating BEFORE emit.
fn is_kebab_word(word: &str) -> bool {
    let mut lower = false;
    let mut upper = false;
    for c in word.chars() {
        match c {
            'a'..='z' if !lower && !upper => lower = true,
            'A'..='Z' if !lower && !upper => upper = true,
            'a'..='z' if lower => {}
            'A'..='Z' if upper => {}
            '0'..='9' if lower || upper => {}
            '-' if lower || upper => {
                lower = false;
                upper = false;
            }
            _ => return false,
        }
    }
    !word.is_empty() && !word.ends_with('-')
}

/// Whether `word` is a kebab word with NO uppercase letter — the stricter form a package NAMESPACE or
/// PACKAGE label must take (`cadenza`, `wasi-cli`), where the component model additionally forbids
/// uppercase. An interface PROJECTION (the part after `/`) uses the looser [`is_kebab_word`].
fn is_lowercase_kebab_word(word: &str) -> bool {
    is_kebab_word(word) && !word.chars().any(|c| c.is_ascii_uppercase())
}

/// Whether `name` is a valid component-model INTERFACE name — the string a peer binding
/// (`(bind E "ns:pkg/iface")`) or a provider's `--component-name` publishes/imports an interface
/// instance under. The grammar (component-model `pkgpath` with a required projection, matching what
/// `wasmtime` accepts): one or more `:`-separated LOWERCASE-kebab package segments (`ns:pkg`, ≥2 —
/// at least a namespace and a package label), then a required `/`-separated projection of ≥1 kebab
/// segments (`iface`, `iface/nested`), then an OPTIONAL `@<version>` suffix (any non-empty tail).
///
/// This is the guard that turns a silent INVALID-COMPONENT miscompile into a compile error: without
/// it, an author's `"Math/API"` (or any non-conforming string) compiles to a component whose import/
/// export extern name is not a valid interface name, which `wasmtime` rejects at LOAD with no
/// compiler diagnostic. The compile-time reject naming the offending string is the actionable fix.
pub fn is_valid_interface_name(name: &str) -> bool {
    // An optional trailing `@<version>` — the component model validates it as semver, but we only
    // require it be non-empty here (a bare `@` is malformed); the structural path is what matters for
    // the extern-name validity that bites at load.
    let path = match name.split_once('@') {
        Some((p, version)) => {
            if version.is_empty() {
                return false;
            }
            p
        }
        None => name,
    };
    // The path splits at the FIRST `/` into the package part (namespace(s) + label) and the projection.
    let Some((pkg, projection)) = path.split_once('/') else {
        // No projection → a bare package name / label, not an interface name (an instance import/export
        // needs the `/iface` projection). Reject.
        return false;
    };
    // Package: `ns(:ns)*:label` — ≥2 `:`-separated LOWERCASE-kebab segments.
    let pkg_segments: Vec<&str> = pkg.split(':').collect();
    if pkg_segments.len() < 2 || !pkg_segments.iter().all(|s| is_lowercase_kebab_word(s)) {
        return false;
    }
    // Projection: `iface(/iface)*` — ≥1 `/`-separated kebab segments (uppercase-kebab permitted).
    projection.split('/').all(is_kebab_word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_kebab_names_are_unchanged() {
        for n in ["inc", "my-func", "f2", "call0", "sum-to", "a", "leb128"] {
            assert_eq!(kebab_extern_name(n), n, "kebab name {n} must be identity");
            assert!(is_kebab_extern_name(n));
        }
    }

    #[test]
    fn non_kebab_names_normalize_to_the_documented_forms() {
        assert_eq!(kebab_extern_name("fA"), "f-a");
        assert_eq!(kebab_extern_name("myFunc"), "my-func");
        assert_eq!(kebab_extern_name("Foo"), "foo");
        assert_eq!(kebab_extern_name("my_func"), "my-func");
        // A camelCase run of several words.
        assert_eq!(
            kebab_extern_name("parseHTTPResponse"),
            "parse-h-t-t-p-response"
        );
        // Trailing digits stay attached to their word.
        assert_eq!(kebab_extern_name("fooBar2"), "foo-bar2");
        for n in ["fA", "myFunc", "Foo", "my_func"] {
            assert!(!is_kebab_extern_name(n), "{n} is not already kebab");
            // The normalized form is itself kebab (idempotent).
            let k = kebab_extern_name(n);
            assert_eq!(
                kebab_extern_name(&k),
                k,
                "normalization is idempotent for {n}"
            );
        }
    }

    #[test]
    fn separators_are_collapsed_and_trimmed() {
        assert_eq!(kebab_extern_name("a__b"), "a-b");
        assert_eq!(kebab_extern_name("a-_b"), "a-b");
        assert_eq!(kebab_extern_name("a_"), "a");
    }

    #[test]
    fn well_formed_interface_names_are_accepted() {
        for n in [
            "cadenza:math/api",
            "cadenza:pairs/api",
            "wasi:cli/run",
            "wasi:filesystem/types",
            // Multiple namespace segments and a nested projection.
            "a:b:c/d/e",
            // A version suffix is tolerated.
            "cadenza:math/api@0.0.0",
            "cadenza:math/api@1.2.3-alpha",
        ] {
            assert!(is_valid_interface_name(n), "should accept `{n}`");
        }
    }

    #[test]
    fn malformed_interface_names_are_rejected() {
        for n in [
            // The load-bearing case: uppercase in the package part — `kebab_extern_name` would MANGLE
            // this to `math/-a-p-i` (invalid), so the emitted component fails to load.
            "Math/API",
            // Uppercase namespace/package label (forbidden in a package name).
            "Cadenza:math/api",
            "cadenza:Math/api",
            // No projection — a bare package name is not an interface name.
            "cadenza:math",
            // Missing the package label (only one `:`-segment).
            "cadenza/api",
            // Empty segments.
            "cadenza:/api",
            ":math/api",
            "cadenza:math/",
            // A leading/trailing `-` in a segment (not a valid kebab word).
            "cadenza:math/-api",
            "cadenza:math/api-",
            // A bare `@` with no version.
            "cadenza:math/api@",
            // Whitespace / punctuation.
            "cadenza math/api",
            "cadenza:math.api/x",
            "",
        ] {
            assert!(!is_valid_interface_name(n), "should reject `{n}`");
        }
    }
}
