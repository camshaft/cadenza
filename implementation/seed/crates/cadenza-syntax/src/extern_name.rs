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
}
