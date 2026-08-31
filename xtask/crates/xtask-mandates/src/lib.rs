//! `cargo xtask lint-mandates` — gate-level enforcement of the operator's mechanizable STANDING
//! MANDATES over the Rust compiler codebase, so a violating MR fails the gate rather than being caught
//! post-hoc by the operator reading diffs. Mirrors `emoji_free_lint` (a cheap source-scan, ONE native
//! `check()` step, no per-crate rebuild); v-fleet-tooling wires it into `cargo xtask check` + the nix
//! `localGate`, exactly as the NO-emoji mandate is enforced today.
//!
//! This is a RUST-SOURCE lint (scans `implementation/**/*.rs`) — distinct from `cadenza-syntax`'s
//! `cdz lint`, which matches CADENZA programs. The mandates it enforces are about the Rust codebase, so
//! Rust source is the right surface.
//!
//! DENY rules (a hit fails the gate), each with a scoped `mandate:allow` escape hatch for a justified
//! exception (a hex-parse at a wire boundary, a component E2E that cannot be an in-crate test):
//!  - **no-integration-tests** — ANY `tests/*.rs` cargo integration test (operator prefer-unit-tests:
//!    a `tests/*.rs` is a separate binary + linkage + slow compile + separate nix derivation). A
//!    ZERO-TOLERANCE full-tree deny (operator ruling: no exceptions — even a component/binary E2E
//!    converts to an env-gated in-crate `#[cfg(test)]` test). The transitional grandfather allowlist is
//!    GONE (the fleet reached zero non-fixture `tests/*.rs`); no per-file escape. The only excluded
//!    trees are vendored `reference/` and `tests/fixtures/**` guest wasm-component crates (not test
//!    binaries).
//!  - **no-hard-coded-runtime-hash** — a bare content-address string literal in NON-test source is a
//!    hard-coded content address (operator no-hard-coded-runtime-names): a 45-char base62 (the platform
//!    `Hash` text form, §8) or a legacy 64-hex string. Excludes the codegen'd `runtime_abi.rs`
//!    hash-constant home, `#[cfg(test)]` golden-hash assertions, and a `mandate:allow` line. syn-based:
//!    parses only files whose lines contain such a literal (a tiny population), skips test-gated items.
//!
//! no-hex-except-tracing and no-thin-wrapper-fns are DELIBERATELY NOT gate-lint rules (concierge ruling
//! 2026-08-09): both need DATA-FLOW / judgment a source-scan cannot do — no-hex would flood on all ~127
//! legit `Hash::to_hex` (the intent, no hex to STORE/TRANSMIT, is semantic not syntactic) and
//! no-thin-wrapper (fn that only delegates) is inherently fuzzy. They stay enforced by the comprehensive
//! audits + reviewer judgment, not an automated flood. Only CLEANLY-mechanizable mandates belong here.

use std::path::{Path, PathBuf};

/// A mandate violation: the file it is in and a human-readable reason. Rendered `path: reason`.
pub struct Violation {
    pub file: PathBuf,
    pub reason: String,
}

/// Run every mechanizable mandate check over `implementation/**/*.rs` under `repo`. Returns the
/// violations (empty = clean). The caller (the `lint-mandates` subcommand / the `check()` step) fails
/// the gate when any are returned.
pub fn lint_mandates(repo: &Path) -> Result<Vec<Violation>, String> {
    let mut out = Vec::new();
    out.extend(no_new_integration_tests(repo)?);
    out.extend(no_hard_coded_runtime_hash(repo)?);
    // Follow-on: no_hex_except_tracing, no_thin_wrapper_fns (syn-based).
    Ok(out)
}

/// The `no-integration-tests` mandate: ANY `tests/*.rs` cargo integration test is a violation — a
/// ZERO-TOLERANCE full-tree deny (operator ruling 2026-08-09: no integration-test exceptions AT ALL,
/// even a component/binary E2E converts to an env-gated in-crate `#[cfg(test)]` test — a `cfg(test)` mod
/// reads the same `CDZ_LIVE_*` artifact-path env and skips identically). A `tests/*.rs` is a separate
/// test binary (extra linkage, slow compile, separate nix derivation); the operator prefers in-crate
/// `#[cfg(test)]` unit tests. The transitional grandfather allowlist is GONE (the fleet reached zero
/// non-fixture `tests/*.rs`); there is no per-file escape for this mandate. The ONLY excluded trees are
/// (a) vendored `reference/` (not ours to police) and (b) `tests/fixtures/**` guest wasm-component
/// CRATES (own `Cargo.toml`/`src` — NOT cargo test binaries), both handled by [`is_integration_test_path`]
/// / [`is_vendored_path`].
fn no_new_integration_tests(repo: &Path) -> Result<Vec<Violation>, String> {
    let impl_root = repo.join("implementation");
    let mut rs = Vec::new();
    collect_rs_files(&impl_root, &mut rs).map_err(|e| {
        format!(
            "cannot enumerate {} for the mandate lint: {e}",
            impl_root.display()
        )
    })?;
    let mut out = Vec::new();
    for f in rs {
        if !is_integration_test_path(&f) {
            continue;
        }
        let rel = f.strip_prefix(repo).unwrap_or(&f);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // EXCLUDE VENDORED / third-party trees (`reference/`) — not ours to police. (The fixtures-tree
        // guest crates are excluded inside `is_integration_test_path` — they aren't test binaries.)
        if is_vendored_path(&rel_str) {
            continue;
        }
        out.push(Violation {
            file: f.clone(),
            reason:
                "cargo integration test (tests/*.rs) — the operator mandate is ZERO-TOLERANCE: NO \
                 integration tests, no exceptions. Move the coverage into a #[cfg(test)] mod in the \
                 crate (an env-gated component/binary E2E converts too — a cfg(test) mod reads the same \
                 CDZ_LIVE_* artifact-path env and skips identically). There is no allowlist"
                    .to_string(),
        });
    }
    Ok(out)
}

/// The `no-hard-coded-runtime-hash` mandate: a bare content-address string literal in NON-TEST Rust
/// source is a hard-coded content-address — the operator's no-hard-coded-runtime-names directive (a hash
/// pinned in source drifts silently on a `REQUIRED_RUNTIME_HASH` bump — exactly the class of the guide-FOD
/// / genesis-reducer-compose breakage a runtime-hash bump caused, and the pinned
/// `cadenza:runtime/heap@0.0.0+<hash>` the operator flagged). A content address must be DERIVED (read
/// the live hash / an input-addressed artifact), never a literal.
///
/// Two shapes are a content-address literal: the platform text form — a `"<45 base62 chars>"`
/// (`0-9A-Za-z`, the `cdz_contract::Hash` `Display`, §8) — and the legacy `"<64 lowercase hex>"` a raw
/// blake3 digest rendered before the base62 flip. Both are flagged so a stale hex pin AND a new base62 pin
/// are caught. base62 is exactly `Hash::TEXT_LEN` (45) chars, so a whole-literal length match keeps this
/// specific (an ordinary alphanumeric string of some other length does not trip it).
///
/// EXCLUSIONS (why the current tree is clean, so this is a going-forward guard, not a fleet-redder):
///  - `rcdzc/src/backend/wasm/runtime_abi.rs` — the CODEGEN'd home of `REQUIRED_RUNTIME_HASH` /
///    `DEBUG_RUNTIME_HASH` / `REQUIRED_NFC_HASH`. These ARE the canonical derived-then-recorded hashes
///    (`xtask codegen` regenerates them from the built bytes); the parity gate proves them against the
///    built artifact. This one file is the single sanctioned literal-hash site.
///  - anything under a `#[cfg(test)]` item — a test golden-hash assertion (e.g. `event.rs`'s frozen
///    on-disk-format hash pin) is a legitimate regression witness, not a runtime dependency.
///  - a `mandate:allow` line comment on (or just above) the literal — the escape hatch for a justified
///    non-derivable hash, same convention as the other mandates.
///  - VENDORED (`reference/`) trees — not ours to police.
fn no_hard_coded_runtime_hash(repo: &Path) -> Result<Vec<Violation>, String> {
    let impl_root = repo.join("implementation");
    let mut rs = Vec::new();
    collect_rs_files(&impl_root, &mut rs).map_err(|e| {
        format!(
            "cannot enumerate {} for the mandate lint: {e}",
            impl_root.display()
        )
    })?;
    let mut out = Vec::new();
    for f in rs {
        let rel = f.strip_prefix(repo).unwrap_or(&f);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if is_vendored_path(&rel_str) || is_sanctioned_hash_home(&rel_str) {
            continue;
        }
        let src = std::fs::read_to_string(&f)
            .map_err(|e| format!("cannot read {} for the hash mandate: {e}", f.display()))?;
        // Cheap prefilter: only parse files that actually contain a content-address-looking literal
        // (parsing every .rs with syn would be needless work — the population is tiny).
        if !line_has_content_address_literal(&src) {
            continue;
        }
        let file = syn::parse_file(&src)
            .map_err(|e| format!("cannot parse {} for the hash mandate: {e}", f.display()))?;
        let mut finder = HashLiteralFinder::default();
        syn::visit::visit_file(&mut finder, &file);
        for lit in finder.hits {
            out.push(Violation {
                file: f.clone(),
                reason: format!(
                    "hard-coded content address \"{}…\" in non-test source — a runtime/content \
                     hash must be DERIVED (read the live `REQUIRED_RUNTIME_HASH` / an input-addressed \
                     artifact), never pinned as a literal (it drifts silently on a hash bump — the \
                     operator no-hard-coded-runtime-names directive). If this is a genuinely-fixed \
                     external hash, add a `// mandate:allow no-hard-coded-runtime-hash: <reason>` comment",
                    &lit[..12]
                ),
            });
        }
    }
    Ok(out)
}

/// Is this a sanctioned literal-hash file — a codegen'd home of `REQUIRED_RUNTIME_HASH` etc.? Two files
/// qualify: `cadenza-compile-abi/src/runtime_hash.rs` (the relocated declaration home, read by the thin
/// `!standalone` `cdz` without linking `rcdzc`) and `rcdzc/src/backend/wasm/runtime_abi.rs` (the ABI table,
/// which now RE-EXPORTS the hashes — no literal there today, kept sanctioned for robustness). Matched on
/// the path tail so it holds regardless of the crate root prefix.
fn is_sanctioned_hash_home(rel_slash: &str) -> bool {
    rel_slash.ends_with("cadenza-compile-abi/src/runtime_hash.rs")
        || rel_slash.ends_with("rcdzc/src/backend/wasm/runtime_abi.rs")
}

/// A cheap line-level prefilter: does the source contain a content-address-shaped literal (a 64-lowercase-
/// hex or a 45-char base62 quoted run) AND lack a `mandate:allow no-hard-coded-runtime-hash` escape on that
/// line? Used to skip syn-parsing files that can't possibly hit. (The authoritative check is the syn
/// visitor; this only avoids needless parses.)
fn line_has_content_address_literal(src: &str) -> bool {
    src.lines().any(|l| {
        !l.contains("mandate:allow no-hard-coded-runtime-hash") && line_contains_hash_string(l)
    })
}

/// Does a single line contain a double-quoted run that is a content-address literal — exactly 64
/// lowercase-hex characters (legacy raw digest) or exactly 45 base62 characters (`0-9A-Za-z`, the platform
/// `Hash` text form, §8)? The run must fill the whole literal (quote to quote), so a longer/shorter
/// alphanumeric string does not match.
fn line_contains_hash_string(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            // The maximal alphanumeric run right after the quote; a hash literal is that whole run.
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let run = &line[start..j];
                if is_hash_literal(run) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Is `s` a content-address literal — a 64-char all-lowercase-hex string (legacy raw blake3 digest) or a
/// 45-char base62 string (`0-9A-Za-z`, the platform `Hash` text form, §8)? Both are exact-width, so an
/// alphanumeric string of any other length is not a hash.
fn is_hash_literal(s: &str) -> bool {
    let is_hex64 = s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    let is_base62_45 = s.len() == 45 && s.bytes().all(|b| b.is_ascii_alphanumeric());
    is_hex64 || is_base62_45
}

/// A `syn` visitor that collects content-address string literals (64-lowercase-hex or 45-char base62),
/// SKIPPING any item gated by `#[cfg(test)]` (a test golden-hash assertion is a legitimate witness, not a
/// runtime dependency). The `mandate:allow` escape is handled by the line prefilter before parsing, so a
/// file whose only such literal carries the escape never reaches here.
#[derive(Default)]
struct HashLiteralFinder {
    hits: Vec<String>,
    in_test: usize,
}

impl<'ast> syn::visit::Visit<'ast> for HashLiteralFinder {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let gated = item_attrs(item).is_some_and(has_cfg_test);
        if gated {
            self.in_test += 1;
        }
        syn::visit::visit_item(self, item);
        if gated {
            self.in_test -= 1;
        }
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if self.in_test == 0 {
            let v = lit.value();
            if is_hash_literal(&v) {
                self.hits.push(v);
            }
        }
    }
}

/// The attributes on any `syn::Item` variant that can carry them (enough variants to cover a
/// `#[cfg(test)] mod tests` / gated fn/impl; a variant without attrs yields `None`).
fn item_attrs(item: &syn::Item) -> Option<&[syn::Attribute]> {
    Some(match item {
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        _ => return None,
    })
}

/// Does an attribute list contain `#[cfg(test)]`?
fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let mut is_test = false;
        // `cfg(test)` — the meta is a single `test` path token.
        let _ = a.parse_nested_meta(|m| {
            if m.path.is_ident("test") {
                is_test = true;
            }
            Ok(())
        });
        is_test
    })
}

/// Whether a repo-relative path is under a VENDORED / third-party tree the mandate must NOT police.
/// A `reference/` path segment marks vendored code we don't author (e.g.
/// `implementation/music/reference/euphony-rs/…`). Our prefer-unit-tests (and other) mandates are our
/// own discipline; a third-party crate's test layout is not ours to change, and allowlisting each
/// vendored file is churn (new vendored files would re-red). Pure over the slash-normalized rel path so
/// the exclusion is unit-tested.
fn is_vendored_path(rel_slash: &str) -> bool {
    rel_slash.split('/').any(|seg| seg == "reference")
}

/// Is `path` a cargo INTEGRATION-test binary source — a `.rs` under a crate's top-level `tests/` dir,
/// EXCLUDING the two things a `tests/` subtree can hold that are NOT test binaries:
///  - `tests/fixtures/**` — a guest wasm-component CRATE (its own `Cargo.toml`/`src/lib.rs`), a build
///    fixture, not a cargo test target. Its `src/lib.rs` must stay.
///  - a nested crate anywhere under `tests/` (a `src/` segment AFTER `tests/`) — likewise a sub-crate's
///    own source, not the enclosing crate's integration surface.
///
/// A `tests` MODULE inside `src/` (`src/…/tests.rs`, `src/…/tests/mod.rs`) is an in-crate unit test we
/// WANT, so a `src/` segment BEFORE `tests` disqualifies too. Everything else under a crate-root `tests/`
/// (`<crate>/tests/foo.rs`, `<crate>/tests/common/mod.rs`) IS a cargo integration-test source.
fn is_integration_test_path(path: &Path) -> bool {
    if path.extension().is_none_or(|x| x != "rs") {
        return false;
    }
    let comps: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let Some(tests_ix) = comps.iter().position(|&c| c == "tests") else {
        return false;
    };
    // A `src/` BEFORE `tests` = an in-crate module named tests, not the cargo integration surface.
    if comps[..tests_ix].contains(&"src") {
        return false;
    }
    let after = &comps[tests_ix + 1..];
    // A guest wasm-component crate under `tests/fixtures/**` is a build fixture, not a test binary.
    if after.first() == Some(&"fixtures") {
        return false;
    }
    // A nested crate under `tests/` (a `src/` after `tests/`) is a sub-crate's own source, not the
    // enclosing crate's integration test.
    if after.contains(&"src") {
        return false;
    }
    true
}

/// The DECLINE-PROFESSIONALISM hard-fail lexicon (operator seq-280 write-literally: a user-facing decline
/// must be professional — the "we'll build it later" intent lives in a comment/tracker, never in the text a
/// USER sees). OWNED by v-deferral-declines (their spec v1, 2026-08-31) — kept as a DATA const so they can
/// tune it without touching the scan logic. Case-insensitive, word-boundary matched. Deliberately EXCLUDES
/// "currently"/"presently" (usually FACTUAL — "the cores currently thread tuples" — an advisory tier at
/// most, not a hard fail) and "not implemented" (a borderline terminal statement). "yet" is highest-signal
/// (subsumes not-yet / yet-supported / yet-emitted / boundary-yet).
pub const DEFERRAL_LEXICON: &[&str] = &[
    "yet",
    "not yet",
    "for now",
    "for the moment",
    "at this time",
    "later slice",
    "later increment",
    "later widening",
    "next brick",
    "someday",
    "eventually",
    "to be implemented",
    "planned",
    "will support",
    "coming soon",
];

/// The `decline-professionalism` lint (operator seq-280): scan `rcdzc/src/**/*.rs` for a `Reject::decline`
/// / `unsupported` / `declined` / `coded` call whose MESSAGE string carries a [`DEFERRAL_LEXICON`] phrase —
/// deferral-trash wording ("… yet", "for now") that leaked into user-facing text. Returns the violations
/// (empty = clean). A COMPLETE static source scan (every decline site), the durable complement to a manual
/// sweep — which had a 3-tick false-clean (a `\`-continued "not\nyet" the line-based regex could not see).
///
/// This is DELIBERATELY NOT part of [`lint_mandates`] (which is already folded into the live `mandateLint`
/// gate): v-deferral's ratchet is turn-on-at-ZERO with NO grandfather, and 3 residue sites are still pending
/// their owner's land — so v-fleet-tooling wires this as its OWN check (a `decline-professionalism`
/// runCommand + a `.#lint-declines` app over the `xtask-mandates declines` subcommand) and folds it into
/// localGate only AFTER the residue clears, so the gate starts green. Until then this fn is inert.
///
/// syn resolves the `\`-continuation JOIN + escapes for free ([`syn::LitStr::value`] gives the actual
/// string content), which is exactly the miss a naive line scan makes — the whole reason for an AST lint.
pub fn lint_decline_professionalism(repo: &Path) -> Result<Vec<Violation>, String> {
    let src_root = repo.join("implementation/seed/crates/rcdzc/src");
    let mut rs = Vec::new();
    collect_rs_files(&src_root, &mut rs).map_err(|e| {
        format!(
            "cannot enumerate {} for the decline lint: {e}",
            src_root.display()
        )
    })?;
    rs.sort(); // deterministic report order
    let mut out = Vec::new();
    for f in rs {
        let rel = f.strip_prefix(repo).unwrap_or(&f);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // Generated decline tables are machine-emitted (not hand-authored user text) — excluded.
        if is_generated_declines_file(&rel_str) {
            continue;
        }
        let src = std::fs::read_to_string(&f)
            .map_err(|e| format!("cannot read {} for the decline lint: {e}", f.display()))?;
        // Cheap prefilter: only parse a file that actually calls a `Reject::` constructor (most don't).
        if !src.contains("Reject::") {
            continue;
        }
        let file = syn::parse_file(&src)
            .map_err(|e| format!("cannot parse {} for the decline lint: {e}", f.display()))?;
        let mut finder = DeclineFinder::default();
        syn::visit::visit_file(&mut finder, &file);
        for (line, phrase, excerpt) in finder.hits {
            out.push(Violation {
                file: f.clone(),
                reason: format!(
                    "line {line}: deferral wording '{phrase}' in a user-facing decline message: {excerpt:?} \
                     — operator seq-280 requires professional decline text; move the \"build it later\" \
                     intent to a code comment or tracker, not the message a USER sees"
                ),
            });
        }
    }
    Ok(out)
}

/// A generated decline table (machine-emitted, not hand-authored user text) — excluded from the scan.
fn is_generated_declines_file(rel_slash: &str) -> bool {
    rel_slash.ends_with("declines_generated.rs") || rel_slash.ends_with("proptest_gen.rs")
}

/// The first [`DEFERRAL_LEXICON`] phrase present in `msg` as a WHOLE WORD (case-insensitive), or `None`.
/// Word-boundary anchored so "yet" hits "not yet supported" / "yet-emitted" but NOT "yeti" / "keytype".
fn deferral_hit(msg: &str) -> Option<&'static str> {
    let lower = msg.to_lowercase();
    DEFERRAL_LEXICON
        .iter()
        .copied()
        .find(|&p| word_bounded_contains(&lower, p))
}

/// Does `hay` (already lowercased) contain `needle` (ASCII lowercase) as a whole word — the char just
/// before and just after are non-alphanumeric (or a string edge)? Internal spaces in a multi-word phrase
/// ("not yet") are fine; only the phrase ENDS are boundary-checked.
fn word_bounded_contains(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let s = from + rel;
        let e = s + needle.len();
        let before_ok = s == 0 || !hay[..s].chars().next_back().unwrap().is_alphanumeric();
        let after_ok = e == hay.len() || !hay[e..].chars().next().unwrap().is_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = s + 1;
    }
    false
}

/// A `syn` visitor collecting deferral-worded strings that appear INSIDE a `Reject::{decline,unsupported,
/// declined,coded}` call — the only string literals in such a call are the user-facing message (`declined`'s
/// id / `coded`'s Code arg are not string literals), so scanning every string literal within the call
/// (direct, `.to_string()` receiver, or inside a `format!`/`concat!` macro) captures the message in all its
/// shapes. SKIPS anything under a `#[cfg(test)]` / `#[test]` item (a test author's assertion message is not
/// user-facing — the false-hit class v-deferral's manual sweep tripped on).
#[derive(Default)]
struct DeclineFinder {
    /// `(line, matched-phrase, excerpt)` per hit.
    hits: Vec<(usize, &'static str, String)>,
    in_reject: usize,
    in_test: usize,
}

impl DeclineFinder {
    /// Check one resolved string value (a message literal seen inside a Reject call) and record a hit.
    fn scan_value(&mut self, value: &str, line: usize) {
        if self.in_reject == 0 || self.in_test > 0 {
            return;
        }
        if let Some(phrase) = deferral_hit(value) {
            // Excerpt: the message, single-lined + capped so a gate failure is readable.
            let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
            let excerpt: String = if flat.chars().count() > 90 {
                format!("{}…", flat.chars().take(90).collect::<String>())
            } else {
                flat
            };
            self.hits.push((line, phrase, excerpt));
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for DeclineFinder {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let gated = item_attrs(item).is_some_and(is_test_gated);
        if gated {
            self.in_test += 1;
        }
        syn::visit::visit_item(self, item);
        if gated {
            self.in_test -= 1;
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let is_reject = matches!(&*call.func, syn::Expr::Path(p) if is_reject_ctor_path(&p.path));
        if is_reject {
            self.in_reject += 1;
        }
        syn::visit::visit_expr_call(self, call);
        if is_reject {
            self.in_reject -= 1;
        }
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        // syn resolves `\`-continuation + escapes → the actual message content.
        let line = lit.span().start().line;
        self.scan_value(&lit.value(), line);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // A message built by `format!(…)` / `concat!(…)` inside the Reject call: the macro body is an opaque
        // token stream syn does not descend into for `visit_lit_str`, so scan its string-literal tokens here.
        if self.in_reject > 0 && self.in_test == 0 {
            for tok in mac.tokens.clone() {
                if let proc_macro2::TokenTree::Literal(l) = tok
                    && let syn::Lit::Str(s) = syn::Lit::new(l.clone())
                {
                    self.scan_value(&s.value(), l.span().start().line);
                }
            }
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Is `path` a `Reject::{decline,unsupported,declined,coded}` constructor path (the seq-280 user-facing
/// rejection surface)? Matched on the last two segments so a `crate::…::Reject::decline` qualification holds.
fn is_reject_ctor_path(path: &syn::Path) -> bool {
    let segs: Vec<&syn::Ident> = path.segments.iter().map(|s| &s.ident).collect();
    let n = segs.len();
    n >= 2
        && segs[n - 2] == "Reject"
        && matches!(
            segs[n - 1].to_string().as_str(),
            "decline" | "unsupported" | "declined" | "coded"
        )
}

/// Does an attribute list gate the item to TEST builds — `#[cfg(test)]` or a `#[test]`/`#[…::test]` attr?
/// (A Reject call inside a test is an author assertion, not user-facing text.)
fn is_test_gated(attrs: &[syn::Attribute]) -> bool {
    has_cfg_test(attrs)
        || attrs
            .iter()
            .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
}

/// Recursively collect `.rs` files under `dir` (skipping `target/`). Mirrors the emoji lint's walker.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_test_path_detection() {
        // A crate-root `tests/` .rs is an integration test — INCLUDING a shared helper (under the
        // zero-tolerance flip, `tests/common/mod.rs` must move in-crate too).
        assert!(is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz/tests/lint_cli.rs"
        )));
        assert!(is_integration_test_path(Path::new(
            "implementation/seed/crates/cadenza-syntax/tests/suite/corpus_roundtrip.rs"
        )));
        assert!(is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz/tests/common/mod.rs"
        )));
        // A `src/**` file — even one named tests.rs or under a `tests` MODULE dir under src — is NOT a
        // cargo integration test (it's an in-crate unit test, which the mandate WANTS).
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cadenza-syntax/src/query.rs"
        )));
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/foo/src/tests.rs"
        )));
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/foo/src/parser/tests/helpers.rs"
        )));
        // A non-.rs file under tests/ (a fixture) is not flagged.
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz/tests/fixtures/example.cdz"
        )));
        // A guest wasm-component CRATE under `tests/fixtures/**/src` is a build fixture, NOT a test
        // binary — it must survive the flip (its own Cargo.toml/src).
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz/tests/fixtures/some-guest/src/lib.rs"
        )));
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cadenza-syntax/tests/fixtures/other-guest/src/lib.rs"
        )));
        // A nested crate anywhere under `tests/` (a `src/` after `tests/`) is a sub-crate's own source.
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/foo/tests/helper-crate/src/lib.rs"
        )));
    }

    #[test]
    fn vendored_path_excludes_reference_trees_only() {
        // A `reference/` segment = vendored third-party (not ours to mandate) → excluded.
        assert!(is_vendored_path(
            "implementation/music/reference/euphony-rs/euphony/tests/api.rs"
        ));
        assert!(is_vendored_path(
            "implementation/foo/reference/bar/tests/x.rs"
        ));
        // Our own crates (no `reference/` segment) are NOT excluded — the mandate applies.
        assert!(!is_vendored_path(
            "implementation/seed/crates/cdz/tests/lint_cli.rs"
        ));
        // A substring-but-not-a-segment (e.g. a crate literally named with 'reference' inside) must not
        // match on substring — only a whole path SEGMENT `reference` counts.
        assert!(!is_vendored_path(
            "implementation/seed/crates/reference-impl/tests/x.rs"
        ));
    }

    fn hash_hits(src: &str) -> Vec<String> {
        let file = syn::parse_file(src).expect("parse");
        let mut f = HashLiteralFinder::default();
        syn::visit::visit_file(&mut f, &file);
        f.hits
    }

    #[test]
    fn hard_coded_hash_finder_flags_non_test_literals_only() {
        let hex = "0652838621bb88fcdc0a348bd81a5c8cc84eefa960af78c5cf7885b2811b2614";
        // A bare 64-hex literal (legacy raw digest) in ordinary (non-test) source is flagged.
        let flagged = format!("const H: &str = \"{hex}\";");
        assert_eq!(hash_hits(&flagged), vec![hex.to_string()]);
        // A 45-char base62 literal (the platform Hash text form, §8) is likewise flagged.
        let b62 = "05jmLkKthwJA6JwfqGJrnJmYouvD6erfbnP1jMgVdHsrV";
        assert_eq!(b62.len(), 45);
        assert_eq!(
            hash_hits(&format!("const H: &str = \"{b62}\";")),
            vec![b62.to_string()]
        );
        // The SAME literal inside a `#[cfg(test)]` module is NOT flagged (a golden-hash assertion).
        let in_test = format!("#[cfg(test)]\nmod tests {{\n  const H: &str = \"{hex}\";\n}}");
        assert!(hash_hits(&in_test).is_empty(), "test-gated hash is exempt");
        // A #[cfg(test)] fn body is likewise exempt.
        let in_test_fn = format!("#[cfg(test)]\nfn t() {{ let _ = \"{b62}\"; }}");
        assert!(hash_hits(&in_test_fn).is_empty());
    }

    #[test]
    fn hard_coded_hash_finder_ignores_non_hash_strings() {
        // Not 64 chars, has uppercase, or non-hex → not a (legacy hex) content-address literal.
        assert!(
            hash_hits("const A: &str = \"deadbeef\";").is_empty(),
            "short"
        );
        // A 64-char string with a non-hex char is not a hex hash (uppercase would BE base62 but is 64,
        // not 45, chars — so neither shape matches).
        let almost = "z652838621bb88fcdc0a348bd81a5c8cc84eefa960af78c5cf7885b2811b2614";
        assert!(hash_hits(&format!("const A: &str = \"{almost}\";")).is_empty());
        // An alphanumeric literal of a length that is neither 64 nor 45 is not a hash (width is exact).
        let wrong_len = "05jmLkKthwJA6JwfqGJrnJmYouvD6erfbnP1jMgVdHsr"; // 44 chars, one short
        assert_eq!(wrong_len.len(), 44);
        assert!(hash_hits(&format!("const A: &str = \"{wrong_len}\";")).is_empty());
    }

    #[test]
    fn hash_line_prefilter_matches_the_syn_finder() {
        let hex = "b2a4957895809e29d3e5d15adbca4408a952c8de6c47eadc80e26fe38427d7ed";
        let b62 = "05jmLkKthwJA6JwfqGJrnJmYouvD6erfbnP1jMgVdHsrV";
        assert!(line_contains_hash_string(&format!("  x = \"{hex}\";")));
        assert!(line_contains_hash_string(&format!("  x = \"{b62}\";")));
        // The mandate:allow escape suppresses the line-level prefilter.
        assert!(!line_has_content_address_literal(&format!(
            "  x = \"{b62}\"; // mandate:allow no-hard-coded-runtime-hash: external fixed id"
        )));
        // A bare hash with no escape passes the prefilter.
        assert!(line_has_content_address_literal(&format!(
            "  x = \"{b62}\";"
        )));
        // Interface-name embedding (heap@0.0.0+<hash>) is NOT a bare content-address string (the run after
        // the opening quote is `cadenza`, stopped by `:`), so not matched by this rule — that pattern is a
        // separate concern; this rule is exactly bare content-address literals, the tightest form. Holds
        // for both the legacy hex and the base62 suffix.
        assert!(!line_contains_hash_string(&format!(
            "  x = \"cadenza:runtime/heap@0.0.0+{hex}\";"
        )));
        assert!(!line_contains_hash_string(&format!(
            "  x = \"cadenza:runtime/heap@0.0.0+{b62}\";"
        )));
    }

    fn decline_hits(src: &str) -> Vec<(usize, &'static str, String)> {
        let file = syn::parse_file(src).expect("parse");
        let mut f = DeclineFinder::default();
        syn::visit::visit_file(&mut f, &file);
        f.hits
    }

    #[test]
    fn word_bounded_matches_whole_words_only() {
        assert!(word_bounded_contains("not yet supported", "yet"));
        assert!(word_bounded_contains("emitted yet-again", "yet")); // "yet" then '-' = boundary
        assert!(word_bounded_contains("yet", "yet")); // whole string
        assert!(!word_bounded_contains("a yeti walks", "yet")); // "yet" inside "yeti" → no
        assert!(!word_bounded_contains("the keytype", "yet")); // substring mid-word → no
        assert!(word_bounded_contains("done for now, ok", "for now")); // multi-word phrase
    }

    #[test]
    fn deferral_lexicon_excludes_currently_and_not_implemented() {
        assert_eq!(deferral_hit("this is not supported yet"), Some("yet"));
        assert_eq!(deferral_hit("this is planned"), Some("planned"));
        // "currently"/"presently" are factual (advisory tier), NOT hard-fail; "not implemented" is terminal.
        assert_eq!(deferral_hit("the cores currently thread tuples"), None);
        assert_eq!(deferral_hit("this is not implemented"), None);
        assert_eq!(deferral_hit("a clean professional decline message"), None);
    }

    #[test]
    fn decline_flags_deferral_wording_only_in_reject_calls() {
        // A direct string-literal decline with "yet" → hit.
        let src = r#"fn f() { Reject::decline("this shape is not supported yet"); }"#;
        let hits = decline_hits(src);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "yet");
        // The SAME wording in a plain (non-Reject) call is NOT flagged (scope = Reject ctors only).
        assert!(decline_hits(r#"fn f() { log("not done yet"); }"#).is_empty());
        // coded / unsupported / declined surfaces are all in scope.
        assert_eq!(
            decline_hits(r#"fn f() { Reject::coded(Code::X, "will support this soon"); }"#).len(),
            1
        );
        assert_eq!(
            decline_hits(r#"fn f() { Reject::unsupported("for now, no"); }"#).len(),
            1
        );
    }

    #[test]
    fn decline_joins_backslash_continued_literal_before_matching() {
        // THE case a line-based regex misses: a `\`-continued literal splits "not\nyet" across lines. syn's
        // LitStr::value() collapses the continuation → "…not yet supported", so "yet" IS seen. (v-deferral's
        // 3-tick false-clean class.)
        let src = "fn f() { Reject::decline(\"this is not \\\n        yet supported\"); }";
        let hits = decline_hits(src);
        assert_eq!(
            hits.len(),
            1,
            "continuation-joined literal must be scanned: {hits:?}"
        );
        assert_eq!(hits[0].1, "yet");
    }

    #[test]
    fn decline_scans_format_and_concat_macro_messages() {
        // `format!(…)` message inside the Reject call — the literal is in an opaque macro token stream, so
        // visit_lit_str never fires for it; visit_macro handles it.
        let src =
            r#"fn f(x: u32) { Reject::declined(Id::A, format!("case {x} is not done yet")); }"#;
        let hits = decline_hits(src);
        assert_eq!(hits.len(), 1, "format! message must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "yet");
        // A clean format! message → no hit.
        assert!(
            decline_hits(r#"fn f() { Reject::decline(concat!("a clean ", "message")); }"#)
                .is_empty()
        );
    }

    #[test]
    fn decline_ignores_comments_and_test_gated_items() {
        // Deferral wording in a COMMENT is ALLOWED (that's where the "later" intent belongs) — the scan is
        // string-literal-bounded, so a comment is never seen.
        let src = r#"fn f() {
            // we will support this later, someday, eventually
            Reject::decline("a clean professional message");
        }"#;
        assert!(
            decline_hits(src).is_empty(),
            "comment wording must not be flagged"
        );
        // A Reject call inside a #[cfg(test)] item is a test author's message, not user-facing → skipped.
        let gated = r#"#[cfg(test)]
        mod t { fn g() { Reject::decline("not built yet"); } }"#;
        assert!(
            decline_hits(gated).is_empty(),
            "cfg(test) decline must be exempt"
        );
        // A #[test] fn likewise.
        let test_fn = r#"#[test]
        fn g() { Reject::decline("not built yet"); }"#;
        assert!(
            decline_hits(test_fn).is_empty(),
            "#[test] decline must be exempt"
        );
    }

    #[test]
    fn decline_reports_the_line_of_the_hit() {
        let src = "fn f() {\n    let x = 1;\n    Reject::decline(\"not done yet\");\n}";
        let hits = decline_hits(src);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 3, "the Reject::decline is on line 3 (1-based)");
    }

    #[test]
    fn generated_declines_file_excluded() {
        assert!(is_generated_declines_file(
            "implementation/seed/crates/rcdzc/src/declines_generated.rs"
        ));
        assert!(is_generated_declines_file(
            "implementation/seed/crates/rcdzc/src/foo/proptest_gen.rs"
        ));
        assert!(!is_generated_declines_file(
            "implementation/seed/crates/rcdzc/src/backend/rust/mod.rs"
        ));
    }

    #[test]
    fn sanctioned_hash_home_is_exempt() {
        assert!(is_sanctioned_hash_home(
            "implementation/seed/crates/rcdzc/src/backend/wasm/runtime_abi.rs"
        ));
        assert!(is_sanctioned_hash_home(
            "implementation/seed/crates/cadenza-compile-abi/src/runtime_hash.rs"
        ));
        assert!(!is_sanctioned_hash_home(
            "implementation/seed/crates/cdz/src/main.rs"
        ));
    }
}
