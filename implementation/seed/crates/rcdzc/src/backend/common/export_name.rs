//! Component-boundary NAME policy — backend-AGNOSTIC, dependency-free. The kebab normalization + the
//! export/interface-name validators every consumer (both backends AND the compile driver) needs to
//! decide whether a boundary name is well-formed. Hoisted here (operator shared-backend-code directive)
//! out of `backend/wasm/mod.rs`, where the rust backend AND `compile.rs` were cross-calling it — so the
//! shared policy lives in one backend-agnostic module, not `pub` on wasm and reached into.
//!
//! These are an IN-CRATE COPY of `cadenza_syntax::extern_name` (the rcdzc lib is deliberately
//! dependency-free — `cadenza-syntax` is a DEV-dependency, tests-only — so the compile path cannot call
//! it; the copy is kept byte-identical, an invariant a dev-dep round-trip test guards). Component-model
//! kebab grammar mirrors `wasmparser`'s `KebabStr`: a name this accepts loads under wasmtime, one it
//! rejects does not.

use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// The kebab-case component EXTERN name for a source export identifier. A Cadenza identifier is broader
/// than a component-model extern name: it may contain uppercase letters (`fA`, `Foo`) or underscores
/// (`my_func`) — all valid source names — but the component model requires an export's extern name to be
/// kebab-case (`[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*`). Emitting a non-kebab name verbatim yields a component
/// that fails to validate ("export name `fA` is not a valid extern name") — an unloadable artifact. So a
/// non-kebab source name is NORMALIZED here at the component boundary. The rule (matches
/// `cadenza-syntax`'s `extern_name::kebab_extern_name`, which `cdz-run` uses to resolve a `--call` name):
///   * an UPPERCASE letter begins a word — insert a `-` before it (unless the output is empty / ends in
///     `-`), then lowercase it (`fA`→`f-a`, `myFunc`→`my-func`, `Foo`→`foo`);
///   * `_` becomes a `-` separator (`my_func`→`my-func`); runs of separators collapse; a trailing
///     separator is trimmed;
///   * a lowercase letter, a digit, or a `-` is kept — so an ALREADY-kebab name is the IDENTITY (every
///     corpus export is unchanged, byte-for-byte).
///
/// A source identifier always starts with a letter (a digit-led token is a numeric literal, rejected in
/// the reader), so the result always starts with a valid kebab word. Deterministic — the compiler and
/// the runner agree without threading a mapping across the boundary; a COLLISION (two source names → one
/// extern name) is rejected at export planning (`kebab_export_collision`) before emit.
pub(crate) fn kebab_extern_name(name: &str) -> String {
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

/// Whether `word` is a single valid component-model KEBAB word — a `-`-separated run of
/// same-case-led alphanumeric words (`a`, `a-b`, `foo2`, `HTTP`), non-empty and not ending in `-`.
/// Mirrors `cadenza_syntax::extern_name::is_kebab_word` (which itself mirrors `wasmparser`'s `KebabStr`
/// state machine) — kept as an in-crate COPY because the pure lib core takes `cadenza-syntax` only as a
/// DEV dependency (the "copy, don't depend" discipline `kebab_extern_name` above follows). A word this
/// accepts loads under wasmtime; one it rejects does not.
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

/// Whether `name` is a valid component-model INTERFACE name — the string a peer binding
/// (`(bind E "ns:pkg/iface")`) imports a peer instance under, or a provider's `--component-name`
/// publishes its interface instance under. Grammar (component-model `pkgpath` with a required
/// projection, matching what `wasmtime` accepts at load): ≥2 `:`-separated LOWERCASE-kebab package
/// segments (`ns:pkg`), then a required `/`-separated projection of ≥1 kebab segments (`iface`),
/// then an OPTIONAL non-empty `@<version>` suffix.
///
/// This is the guard that turns a silent INVALID-COMPONENT miscompile into a compile error: an
/// author's `"Math/API"` (or any non-conforming string) would otherwise `kebab_extern_name`-mangle to
/// `math/-a-p-i` (not a valid extern name) and emit a component `wasmtime` rejects at LOAD with no
/// compiler diagnostic. Mirrors `cadenza_syntax::extern_name::is_valid_interface_name`.
pub(crate) fn is_valid_interface_name(name: &str) -> bool {
    let path = match name.split_once('@') {
        Some((p, version)) => {
            if version.is_empty() {
                return false;
            }
            p
        }
        None => name,
    };
    let Some((pkg, projection)) = path.split_once('/') else {
        return false;
    };
    let pkg_segments: Vec<&str> = pkg.split(':').collect();
    if pkg_segments.len() < 2
        || !pkg_segments
            .iter()
            .all(|s| is_kebab_word(s) && !s.chars().any(|c| c.is_ascii_uppercase()))
    {
        return false;
    }
    projection.split('/').all(is_kebab_word)
}

/// If two DISTINCT source export names normalize to the SAME kebab extern name, return a reject naming
/// the collision (else `None`). Two exports that share a normalized extern name cannot both cross the
/// component boundary — the component would carry a duplicate export name (invalid) or silently drop one
/// — so the compiler declines rather than miscompile, exactly as the duplicate-export check does for
/// identical names. (Exports with the SAME source name are the duplicate-export case, caught earlier; a
/// name colliding with ITSELF under normalization is not a collision.) The FIRST such pair is reported.
///
/// `pub(crate)` so the RUST backend applies the SAME export-name reject: an export-boundary name colliding
/// under kebab normalization is a language-level ill-formedness (CDZ0201), not a wasm-only concern — both
/// backends must agree (the corpus grades these `(error CDZ0201)`; the rust backend emits no component, so
/// without this call it silently emitted a value where wasm rejected).
pub(crate) fn kebab_export_collision(layout: &Layout) -> Option<Reject> {
    let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for e in &layout.exports {
        let extern_name = kebab_extern_name(&e.name);
        if let Some(&prior) = seen.get(&extern_name)
            && prior != e.name
        {
            return Some(Reject::coded(
                crate::diag::Code::Malformed,
                format!(
                    "exports `{prior}` and `{}` both normalize to the component extern name `{extern_name}` \
                     — rename one so each export has a distinct kebab-case boundary name",
                    e.name
                ),
            ));
        }
        seen.insert(extern_name, &e.name);
    }
    None
}

/// If an export's NORMALIZED extern name is not a valid component-model kebab word, return a reject
/// naming it (else `None`). `kebab_extern_name` maps uppercase/underscore runs to well-formed kebab
/// words, but it keeps `-`, digits, and — critically — NON-ASCII characters VERBATIM. Two distinct source
/// shapes therefore normalize to a non-kebab extern name that `wasmtime` rejects WHOLESALE at load (an
/// unloadable-component miscompile with no runtime diagnostic; the [[rcdzc-kebab-extern-name-gotcha]]
/// family), so both are rejected here before emit with a CAUSE-SPECIFIC message:
///   (1) a NON-ASCII identifier — Cadenza's ML lexer admits Unicode idents (`def π`, `def café`, `a·b`),
///       but a component extern name is ASCII kebab only (`[a-z0-9-]`, `wasmparser`'s `KebabStr`). A
///       non-ASCII char is kept verbatim and fails to load. Name the offending character in the fix.
///   (2) a DIGIT- or HYPHEN-LED segment — a valid Cadenza identifier (`step-by-2`, `a-2-b`) whose
///       `-`-delimited label starts with a non-letter, which `KebabStr` forbids (each label must start
///       with an ASCII letter). Point at the letter-led rewrite.
/// The FIRST offending export (layout order) is reported. This is the export-NAME analogue of the
/// interface-NAME guard (`is_valid_interface_name`): a boundary name that isn't valid kebab is a
/// compile-time reject, not a silent load failure. (Mirrors `cadenza_syntax::extern_name`'s ASCII
/// precondition — kept a CONSUMER-side reject since the pure lib core takes `cadenza-syntax` DEV-only.)
// `pub(crate)` so the RUST backend applies the SAME reject (an export name that is not valid kebab is a
// language-level CDZ0201, not a wasm-only load failure — both backends must agree; see `kebab_export_collision`).
pub(crate) fn invalid_kebab_export_name(db: &Db, layout: &Layout) -> Option<Reject> {
    for e in &layout.exports {
        let extern_name = kebab_extern_name(&e.name);
        if is_kebab_word(&extern_name) {
            continue;
        }
        // Anchor the reject at the offending definition's SIGNATURE occurrence (where its name is written),
        // so the diagnostic POINTS at the `@test`/export name the author must rename, not just describes it.
        // For a `@test` build the export is a `@test` def; for a normal build it is an `(export …)`'d def —
        // `sig_occ` is the name-bearing occurrence for both.
        let name_span = db.defs.get(e.def).map(|d| d.sig_occ);
        // Pinpoint WHY it fails, so the fix is actionable rather than a generic "not valid kebab".
        // A char outside the kebab alphabet `[A-Za-z0-9-]` (a non-ASCII letter like `π`/`é`, or a symbol
        // like `·`) is the (1) non-ASCII cause; otherwise the alphabet is fine but a segment is digit-/
        // hyphen-led (2).
        let bad_char = extern_name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-'));
        let msg = if let Some(bad) = bad_char {
            format!(
                "the export/`@test` name `{}` is not a valid component boundary name: it contains `{bad}`, \
                 but a component extern name is ASCII kebab-case only (`[a-z0-9-]`) — Cadenza accepts a \
                 Unicode identifier in source, but the emitted component would fail to load (`wasmtime` \
                 rejects a non-kebab extern name wholesale). Rename this export to an ASCII kebab name \
                 (e.g. `π` → `pi`, `café` → `cafe`)",
                e.name
            )
        } else {
            format!(
                "the export/`@test` name `{}` is not a valid component boundary name: it normalizes to \
                 `{extern_name}`, whose `-`-separated segments must each START WITH A LETTER (a digit-led \
                 segment like `-2` is not a valid component extern name, so the emitted component would \
                 fail to load) — rename it so every hyphen-delimited segment begins with a letter (e.g. \
                 `step-by-2` → `step-by-two` or `step-by2`)",
                e.name
            )
        };
        let reject = Reject::coded(crate::diag::Code::Malformed, msg);
        return Some(match name_span {
            Some(occ) => reject.at(occ),
            None => reject,
        });
    }
    None
}
