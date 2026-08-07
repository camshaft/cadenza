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
///   * runs of separators are collapsed and leading/trailing separators trimmed (no `--`, no edge `-`).
///
/// This is a PURE TEXT MAPPING that does NOT guarantee a valid kebab word for every input (see the
/// precondition below). In particular a DIGIT IMMEDIATELY AFTER A WORD SEPARATOR (`a_0`, `step-2`) yields
/// a `-`-led segment (`a-0`, `step-2`) that is NOT a valid kebab word — a kebab word cannot START with a
/// digit (`[a-z][a-z0-9]*`). Such a name is NOT silently collapsed to a valid one (`a_0` → `a0`): per the
/// operator ruling (2026-07-16), a separator-before-digit boundary name is DECLINED at the compile
/// boundary with a CDZ0201 + an actionable rename, rather than silently rewritten to a DIFFERENT name
/// (which would mean two different things across the component / path-deps boundary). The invalid result
/// is caught upstream by `invalid_kebab_export_name` (rcdzc backend/wasm/mod.rs), the same guard that
/// catches a non-ASCII char — see the precondition.
///
/// WARNING: PRECONDITION — the "always a valid kebab word" guarantee holds ONLY for names over ASCII letters,
/// digits, `_` and `-`. A Cadenza identifier may contain NON-ASCII letters (the ML lexer's
/// `is_ident_start` admits `c.is_alphabetic()` and any `!c.is_ascii() && !c.is_whitespace()` char — so
/// `π`, `café`, `ναμε` are valid identifiers). Such a char is NOT `[a-z0-9-]`, so this function passes it
/// through VERBATIM and the result FAILS [`is_kebab_word`]. This function is UNCHANGED by that fact — it
/// is a pure text mapping — and the invalid extern name it would produce for a non-ASCII name is caught
/// UPSTREAM, at the compile boundary: `rcdzc`'s `invalid_kebab_export_name` (backend/wasm/mod.rs) runs
/// this function + `is_kebab_word` over every export and REJECTS a non-kebab result BEFORE emit with a
/// cause-specific diagnostic (naming the offending non-ASCII char + an ASCII-kebab-rename hint, e.g.
/// `π` → `pi`). So a non-ASCII export name is a clean compile-time error, NOT a silent unloadable
/// component. (This resolved a bug I mis-filed as a "silent miscompile" — the consumer-side guard already
/// existed; the real fix was diagnostic wording. See the `non_ascii_names_pass_through_verbatim` test.)
/// Callers that pass ASCII-letter-led identifiers are unaffected; callers with a possibly-non-ASCII name
/// MUST validate via that boundary guard (or `is_kebab_word` on the result) rather than assume validity.
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
            // A lowercase letter or a digit — kept verbatim. A digit that lands right after a word
            // separator (`a_0`, `step-2`) thus produces a `-`-led segment (`a-0`, `step-2`), which is NOT
            // a valid kebab word (a word is `[a-z][a-z0-9]*` — a segment cannot START with a digit). We do
            // NOT silently collapse the separator to make it valid (the earlier behavior: `a_0` → `a0`):
            // per the operator ruling (2026-07-16), a separator-before-digit name is DECLINED at the
            // compile boundary with a CDZ0201 + an actionable rename, NOT silently mangled to a different
            // name — silent-collapse would make the author's chosen name mean something else across the
            // component/path-deps boundary. The invalid result is caught UPSTREAM by
            // `invalid_kebab_export_name` (rcdzc backend/wasm/mod.rs), exactly like a non-ASCII char.
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
///
/// NOTE for the emit-side export-name check (a backend calls this on a NORMALIZED name to decide the
/// component-validity reject): this is NOT interchangeable with [`is_kebab_extern_name`]. The latter is
/// `kebab_extern_name(x) == x` — a NORMALIZER FIXPOINT — which is true for a non-ASCII name that
/// `kebab_extern_name` keeps verbatim, whereas `is_kebab_word` correctly REJECTS non-ASCII (it is not a
/// valid component word). So an emit guard must check `is_kebab_word(&kebab_extern_name(n))`, NOT the
/// fixpoint, or it silently admits an invalid non-ASCII export name (see the reject-family test
/// `a_normalizer_fixpoint_is_not_the_same_as_a_valid_kebab_word`).
pub fn is_kebab_word(word: &str) -> bool {
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
    fn kebab_extern_name_yields_a_valid_kebab_word_unless_a_separator_precedes_a_digit() {
        // The invariant, as narrowed by the operator ruling (2026-07-16): for ANY letter-led source
        // identifier, `kebab_extern_name` produces a valid kebab extern-name word (`is_kebab_word`) —
        // EXCEPT the one class where an explicit word separator (`_`/`-`) is immediately followed by a
        // digit (`a_0`, `x-0y`): a kebab word cannot START with a digit, and we deliberately do NOT
        // silently collapse the separator (that would rename the author's identifier). That class yields
        // a `-`-led segment that is NOT a valid kebab word and is DECLINED at the compile boundary
        // (`invalid_kebab_export_name`), not emitted. So the sweep asserts: either the result is a valid
        // kebab word, OR the source name contains a separator-immediately-before-a-digit (the declined
        // class). A regression that lets some OTHER identifier normalize to a non-kebab word (a `--`, an
        // edge `-`, a stray char) is still caught. Names are letter-led (a digit-led token is a numeric
        // literal, rejected earlier). Idempotence holds for the valid results.
        let alphabet = ['a', 'B', 'z', 'Q', '0', '9', '_', '-'];
        let first = ['a', 'Z', 'm', 'A']; // a source identifier is letter-led
        // Does `name` have an explicit separator (`_`/`-`) immediately followed by an ASCII digit? (That
        // is the exact class the mapping keeps verbatim as an invalid `-`-led segment, declined upstream.)
        let has_separator_before_digit = |name: &str| {
            let b = name.as_bytes();
            (1..b.len()).any(|i| (b[i - 1] == b'_' || b[i - 1] == b'-') && b[i].is_ascii_digit())
        };
        let mut count = 0usize;
        let mut declined = 0usize;
        for &f in &first {
            // lengths 1..=4: the leading letter plus 0..=3 alphabet chars, enumerated exhaustively.
            for len in 0..=3usize {
                let combos = alphabet.len().pow(len as u32);
                for mut n in 0..combos {
                    let mut name = String::new();
                    name.push(f);
                    for _ in 0..len {
                        name.push(alphabet[n % alphabet.len()]);
                        n /= alphabet.len();
                    }
                    let k = kebab_extern_name(&name);
                    if has_separator_before_digit(&name) {
                        // The declined class: the result is (correctly) NOT a valid kebab word — the
                        // boundary guard rejects it rather than the mapping silently fixing it.
                        assert!(
                            !k.is_empty(),
                            "{name:?} normalized to empty (a letter-led name cannot)"
                        );
                        declined += 1;
                    } else {
                        // Every OTHER letter-led identifier still normalizes to a valid kebab word, and
                        // the normalization is idempotent (a re-normalization downstream cannot drift it).
                        assert!(
                            !k.is_empty() && is_kebab_word(&k),
                            "{name:?} normalized to {k:?}, which is NOT a valid kebab extern-name word"
                        );
                        assert_eq!(
                            kebab_extern_name(&k),
                            k,
                            "normalization not idempotent for {name:?}"
                        );
                    }
                    count += 1;
                }
            }
        }
        assert!(count > 2_000, "swept a meaningful space, got {count}");
        assert!(
            declined > 0,
            "the sweep must exercise the separator-before-digit declined class at least once"
        );
    }

    #[test]
    fn non_ascii_names_pass_through_verbatim_and_are_caught_at_the_compile_boundary() {
        // `kebab_extern_name` is a PURE TEXT MAPPING: a non-ASCII letter (a valid Cadenza identifier char —
        // the ML lexer's `is_ident_start` admits `c.is_alphabetic()` / any `!c.is_ascii() &&
        // !c.is_whitespace()` char) is NOT in the kebab alphabet `[a-z0-9-]`, so it passes through
        // VERBATIM and the result is NOT a valid kebab word. This is BY DESIGN, not a gap: the invalid
        // name is caught UPSTREAM at the compile boundary — `rcdzc`'s `invalid_kebab_export_name`
        // (backend/wasm/mod.rs) runs this fn + `is_kebab_word` over every export and REJECTS a non-kebab
        // result pre-emit with a cause-specific diagnostic (names the offending char + an ASCII-rename
        // hint). So this asserts the STABLE pure-mapping contract this fn owns; the reject behavior is
        // tested on rcdzc's side. (Earlier framing here called it a "known gap that will flip when fixed"
        // — WRONG: the fix was a consumer-side reject, so this fn is unchanged and this test does not flip.)
        for name in ["café", "π", "ναμε", "myFünc"] {
            let k = kebab_extern_name(name);
            assert!(
                !is_kebab_word(&k),
                "`{name}` should pass through verbatim to the non-kebab `{k}` (the compile-boundary guard \
                 `invalid_kebab_export_name` is what rejects it, not this pure mapping). If this fn were \
                 changed to transliterate, update this test AND the precondition doc + the boundary guard."
            );
        }
        // The ASCII precondition still holds unconditionally: an ASCII-letter-led name is always valid.
        let name = "café".chars().filter(|c| c.is_ascii()).collect::<String>();
        assert!(
            is_kebab_word(&kebab_extern_name(&name)),
            "ASCII residue stays valid"
        );
    }

    #[test]
    fn separators_are_collapsed_and_trimmed() {
        assert_eq!(kebab_extern_name("a__b"), "a-b");
        assert_eq!(kebab_extern_name("a-_b"), "a-b");
        assert_eq!(kebab_extern_name("a_"), "a");
    }

    #[test]
    fn a_digit_after_an_explicit_separator_is_kept_verbatim_and_declined_not_silently_collapsed() {
        // OPERATOR RULING (2026-07-16): a digit right after an EXPLICIT word separator (`_`/`-`) cannot
        // START a kebab word (a word is `[a-z][a-z0-9]*`), so the mapping keeps it VERBATIM as a `-`-led
        // segment (`a_0` → `a-0`, `step-2` → `step-2`) that is NOT a valid kebab word. It is deliberately
        // NOT silently collapsed to a valid different name (`a_0` → `a0`) — the compile boundary DECLINES
        // it with a CDZ0201 + an actionable rename instead (aligning to the backend copy + v-iterators'
        // shipped policy `emit_tests_declines_a_digit_led_kebab_segment_name`), because a silent rewrite
        // would make the author's chosen name mean a DIFFERENT name across the component / path-deps
        // boundary. Here we pin the PURE MAPPING (the reject is tested on rcdzc's side).
        assert_eq!(kebab_extern_name("a_0"), "a-0");
        assert_eq!(kebab_extern_name("my_2nd"), "my-2nd");
        assert_eq!(kebab_extern_name("x_1y"), "x-1y");
        assert_eq!(kebab_extern_name("a-0"), "a-0");
        // Each such result is NOT a valid kebab word — the boundary guard `invalid_kebab_export_name`
        // rejects it, exactly like a non-ASCII char (see `non_ascii_names_pass_through_verbatim_*`).
        for n in ["a_0", "my_2nd", "x_1y", "a-0", "step-2"] {
            assert!(
                !is_kebab_word(&kebab_extern_name(n)),
                "{n} → must NOT be a valid kebab word (it is declined at the boundary, not collapsed)"
            );
        }
        // NO separator between the two: an uppercase-then-digit boundary is NOT a separator-before-digit
        // (the uppercase starts the word, the digit extends it) — `A0` → `a0`, still valid.
        assert_eq!(kebab_extern_name("A0"), "a0");
        // A digit after a LETTER is unaffected — it extends the current word normally.
        assert_eq!(kebab_extern_name("foo2"), "foo2");
        assert_eq!(kebab_extern_name("fooBar2"), "foo-bar2");
        for n in ["A0", "foo2", "fooBar2"] {
            assert!(
                is_kebab_word(&kebab_extern_name(n)),
                "{n} → must stay a kebab word"
            );
        }
    }

    #[test]
    fn a_normalizer_fixpoint_is_not_the_same_as_a_valid_kebab_word() {
        // A LOAD-BEARING distinction the emit path depends on: `is_kebab_extern_name(name)` asks "is
        // `name` a FIXPOINT of `kebab_extern_name`?" (does normalization leave it unchanged), which is NOT
        // the same as "is `name` a VALID component extern word?" (`is_kebab_word`). The two DISAGREE on the
        // declined separator-before-digit class: `kebab_extern_name("a-0") == "a-0"` (a fixpoint, so
        // `is_kebab_extern_name` is TRUE) yet `is_kebab_word("a-0")` is FALSE (a kebab word cannot start a
        // segment with a digit). So the compile-boundary guard MUST validate via `is_kebab_word(&
        // kebab_extern_name(name))`, NOT via `is_kebab_extern_name(name)` — using the fixpoint check as the
        // emit guard would silently re-admit `a-0`/`step-2`-style names and reintroduce the invalid-
        // component miscompile (`invalid_kebab_export_name` in rcdzc is the real guard). This pins that the
        // two predicates are genuinely different, so a refactor can't conflate them without failing here.
        for n in ["a-0", "step-2", "my-2nd", "x-1y"] {
            assert!(
                is_kebab_extern_name(n),
                "{n} IS a normalizer fixpoint (kebab_extern_name leaves it unchanged)"
            );
            assert!(
                !is_kebab_word(n),
                "{n} is NOT a valid kebab word — the fixpoint check must not be trusted as the emit guard"
            );
        }
        // Where they AGREE (the common, well-behaved case): a genuinely-valid already-kebab name is both a
        // fixpoint and a valid word; a non-kebab source name is neither a fixpoint nor (as written) valid.
        for n in ["inc", "my-func", "foo2", "sum-to"] {
            assert!(is_kebab_extern_name(n) && is_kebab_word(n), "{n} both");
        }
        for n in ["fA", "myFunc", "Foo"] {
            assert!(
                !is_kebab_extern_name(n),
                "{n} is not a fixpoint (it normalizes)"
            );
        }
        // The GUARANTEE the guard actually relies on: normalize THEN validate. For every letter-led name
        // NOT in the declined class, `is_kebab_word(kebab_extern_name(n))` holds — that composition, not the
        // fixpoint predicate, is the correct emit check.
        for n in [
            "fA",
            "myFunc",
            "Foo",
            "my_func",
            "parseHTTPResponse",
            "fooBar2",
        ] {
            assert!(
                is_kebab_word(&kebab_extern_name(n)),
                "normalize-then-validate must accept the well-formed name {n}"
            );
        }
        // The NON-ASCII face of the same fixpoint-vs-valid gap (the exact case a shared backend export
        // guard depends on, per v-wasm-opt's hoist): `kebab_extern_name` keeps a non-ASCII name VERBATIM,
        // so `is_kebab_extern_name` reports it a fixpoint (TRUE) — but `is_kebab_word` correctly REJECTS it
        // (a component word is ASCII kebab only). An emit guard using the fixpoint check would silently
        // admit an invalid non-ASCII export; `is_kebab_word(&kebab_extern_name(n))` rejects it.
        for n in ["café", "naïve", "π", "日本語", "a-é"] {
            assert!(
                is_kebab_extern_name(n),
                "{n} is a normalizer fixpoint (kebab_extern_name keeps non-ASCII verbatim)"
            );
            assert!(
                !is_kebab_word(n) && !is_kebab_word(&kebab_extern_name(n)),
                "{n} is NOT a valid component kebab word — the normalize-then-validate guard rejects it"
            );
        }
    }

    #[test]
    fn is_kebab_word_pins_the_wasmparser_kebabstr_grammar_directly() {
        // `is_kebab_word` documents that it mirrors `wasmparser`'s `KebabStr` state machine EXACTLY — a
        // name it accepts loads under wasmtime, one it rejects does not. Yet every other test exercises it
        // only INDIRECTLY (composed inside `kebab_extern_name(…)` / `is_valid_interface_name`), so its own
        // boundary grammar has no direct pin. Pin it here so a state-machine drift (a reset bug, an
        // edge-`-` slip, a case-mixing regression) is caught at the function itself, independent of any
        // caller. The grammar: a `-`-separated run of alphanumeric words, each word SAME-CASE-LED
        // (`[a-z][a-z0-9]*` OR `[A-Z][A-Z0-9]*`), non-empty, no leading/trailing/doubled `-`.

        // ACCEPTED — the well-formed shapes.
        for w in [
            "a",
            "abc",
            "a-b",
            "a-b-c", // lowercase words, single and hyphenated
            "foo2",
            "a1",
            "call0",
            "leb128", // a digit EXTENDS a word (after a letter)
            "HTTP",
            "A",
            "A1",
            "HTTP-V2", // an uppercase word is a valid segment (wasmparser allows it)
            "parse-json",
            "sum-to", // realistic multi-word names
        ] {
            assert!(is_kebab_word(w), "`{w}` is a valid kebab word");
        }

        // REJECTED — every boundary the state machine must refuse.
        for w in [
            "",       // empty is not a word
            "-",      // a lone separator
            "-a",     // leading separator
            "a-",     // trailing separator
            "a--b",   // doubled separator (empty inner segment)
            "1",      // digit-led (a word can't START with a digit)
            "1a",     // digit-led with a trailing letter
            "0-a",    // a digit-led first segment
            "a-0",    // a digit-led LATER segment (the declined separator-before-digit class)
            "HTTP-2", // a digit-led segment is rejected even after an UPPERCASE word (case doesn't help)
            "aB",     // case MIX within a word (lower then upper)
            "Ab",     // case mix within a word (upper then lower)
            "a_b",    // underscore is not in the kebab alphabet
            "a b",    // space
            "a.b",    // dot
            "café",   // non-ASCII letter
            "π",      // non-ASCII
        ] {
            assert!(!is_kebab_word(w), "`{w}` is NOT a valid kebab word");
        }
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

    #[test]
    fn is_valid_interface_name_is_total_and_accepts_only_wellformed_names() {
        // `is_valid_interface_name` is the guard that turns a silent invalid-component miscompile into a
        // CDZ diagnostic — it runs on UNTRUSTED author strings (`(bind E "ns:pkg/iface")`), so it must be
        // TOTAL: never panic, whatever garbage (multibyte, control chars, pathological runs of the three
        // delimiters `:`/`/`/`@`). The hand tests pin specific accept/reject cases; nothing swept that it
        // can't PANIC, nor that ACCEPTANCE actually implies the structural contract the emit path trusts.
        // Sweep a delimiter-rich alphabet and assert both: (a) no panic, and (b) whenever it ACCEPTS a
        // name, that name really decomposes as `ns(:seg)+ / proj(/proj)* [@ver]` with every package
        // segment a LOWERCASE-kebab word and every projection segment a kebab word — i.e. the predicate
        // and the grammar it documents agree, so a name it blesses is one wasmtime's extern-name grammar
        // also accepts.
        let alphabet: Vec<char> = "abcABC01-:/@. λ中".chars().collect();
        // SplitMix64 inline (crate house style — no dep; matches the other surfaces' test PRNGs).
        let mut state: u64 = 0x1f7e_a5e0_c0de_face;
        let mut next = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut accepted = 0usize;
        for len in 0..=8usize {
            for _ in 0..400 {
                let s: String = (0..len)
                    .map(|_| alphabet[(next() as usize) % alphabet.len()])
                    .collect();
                // (a) TOTALITY — must not panic on arbitrary input.
                if is_valid_interface_name(&s) {
                    accepted += 1;
                    // (b) ACCEPTANCE CONTRACT — re-derive the decomposition independently and check the
                    // structural grammar holds, so a bless from the predicate is a real interface name.
                    let path = match s.split_once('@') {
                        Some((p, ver)) => {
                            assert!(!ver.is_empty(), "accepted `{s}` with an empty @version");
                            p
                        }
                        None => &s,
                    };
                    let (pkg, proj) = path
                        .split_once('/')
                        .unwrap_or_else(|| panic!("accepted `{s}` with no `/` projection"));
                    let pkg_segs: Vec<&str> = pkg.split(':').collect();
                    assert!(
                        pkg_segs.len() >= 2,
                        "accepted `{s}` with a single-segment package `{pkg}`"
                    );
                    assert!(
                        pkg_segs.iter().all(|seg| is_lowercase_kebab_word(seg)),
                        "accepted `{s}` but a package segment is not lowercase-kebab"
                    );
                    assert!(
                        proj.split('/').all(is_kebab_word),
                        "accepted `{s}` but a projection segment is not a kebab word"
                    );
                }
            }
        }
        // A few deliberately pathological delimiter runs — the shapes likeliest to trip an index/split
        // assumption — must also just return a bool, never panic.
        for s in [
            "@",
            "/",
            ":",
            "::",
            "//",
            "@@",
            ":/@",
            "a:b/c@",
            "@1",
            "///",
            "a::b//c@@v",
            ":::///@@@",
            "λ:中/x",
            "a:b/λ",
            "\u{0}:\u{0}/\u{0}",
        ] {
            let _ = is_valid_interface_name(s); // must not panic
        }
        // The `accepted` count is only a coverage HINT (couples to the alphabet/iteration count), so it
        // is NOT asserted — instead exercise the acceptance path DETERMINISTICALLY on constructed valid
        // names, so the accept-branch contract is checked every run regardless of the sweep's luck.
        let _ = accepted;
        for s in [
            "ns:pkg/iface",
            "a:b/c/d",
            "cadenza:math/api@1.0.0",
            "wasi:cli/run",
        ] {
            assert!(
                is_valid_interface_name(s),
                "constructed valid name `{s}` must be accepted"
            );
        }
    }
}
