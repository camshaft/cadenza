//! Module-wrapping for the guide shred (a faithful Rust port of guide/src/components/wrapModule.ts).
//!
//! Guide example snippets are authored WITHOUT the `export`/`main` ceremony a compilable program needs
//! (a bare expression, or a `def`/`type`/`effect` block). `wrap_module` supplies only what's missing — at
//! top level, no `module { … }` shell — so the snippet compiles, exactly as the live app + the node shred
//! did. Operator direction (binary AST = universal exchange): the shred takes binary AST, but the wrapped
//! `program.sexpr`/`program.ml` it emits are TEXT the compiler ingests, so this operates on the printed
//! snippet text (string ops, no parser). Kept byte-for-byte equivalent to wrapModule.ts — the shredded
//! program must compile identically to what ships.
//!
//! NOTE: this is a lightweight top-level-form scan, NOT a parser (mirrors the .ts, which is also textual).

/// Surface of a snippet / wrapped program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    Sexpr,
    // Part of the faithful wrapModule.ts port (ml wrapping) + exercised by the unit tests. The shred wraps in
    // sexpr and renders ml via `cdz convert`, so the binary never constructs this — kept for port fidelity.
    #[allow(dead_code)]
    Ml,
}

/// A regex `\b`-style word char: `[A-Za-z0-9_]` (matches JS `\w`; `-` is NOT a word char, so `(def-x` has a
/// boundary after `def`, exactly as the .ts regexes treat it).
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `s` begins with `head` followed by a word boundary (next char is non-word or end) — the `^head\b` check.
fn head_word(s: &str, head: &str) -> bool {
    s.strip_prefix(head)
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word(c)))
}

/// Top-level `def` NAMES in source order (deduped). sexpr: `(def NAME …` / `(def (NAME …`; ml: an
/// UNINDENTED `def NAME`. A Cadenza name is `[A-Za-z_][\w-]*`. Lightweight scan (not a parser).
pub fn top_level_def_names(trimmed: &str, surface: Surface) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |n: &str| {
        if !n.is_empty() && !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    };
    match surface {
        // sexpr: `\(def\s+\(?\s*([A-Za-z_][\w-]*)` anywhere.
        Surface::Sexpr => {
            let bytes = trimmed.as_bytes();
            let mut i = 0;
            while let Some(p) = trimmed[i..].find("(def") {
                let start = i + p;
                let mut j = start + 4; // after "(def"
                // require whitespace after `(def`
                if bytes.get(j).is_some_and(|b| b.is_ascii_whitespace()) {
                    while bytes.get(j).is_some_and(|b| b.is_ascii_whitespace()) {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'(') {
                        j += 1;
                        while bytes.get(j).is_some_and(|b| b.is_ascii_whitespace()) {
                            j += 1;
                        }
                    }
                    push(&scan_name(trimmed, j));
                }
                i = start + 4;
            }
        }
        // ml: `^def[ \t]+([A-Za-z_][\w-]*)` per line.
        Surface::Ml => {
            for line in trimmed.lines() {
                if let Some(rest) = line.strip_prefix("def")
                    && rest.starts_with([' ', '\t'])
                {
                    let name_start = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                    push(&scan_name(rest, name_start));
                }
            }
        }
    }
    names
}

/// Read a Cadenza identifier `[A-Za-z_][\w-]*` starting at byte `start` of `s` (empty if none).
fn scan_name(s: &str, start: usize) -> String {
    let rest = &s[start.min(s.len())..];
    let mut end = 0;
    for (k, c) in rest.char_indices() {
        let ok = if k == 0 {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            is_word(c) || c == '-'
        };
        if ok {
            end = k + c.len_utf8();
        } else {
            break;
        }
    }
    rest[..end].to_string()
}

/// The names a DEFINITIONS-block snippet exports: `main` if present, else the top-level `def` names, else
/// `main` (matches wrapModule.ts `exportNames`).
pub fn export_names(trimmed: &str, surface: Surface) -> Vec<String> {
    let names = top_level_def_names(trimmed, surface);
    if names.iter().any(|n| n == "main") {
        return vec!["main".to_string()];
    }
    if names.is_empty() {
        vec!["main".to_string()]
    } else {
        names
    }
}

/// Whether an ML snippet already declares a top-level `export` (`^`- or newline-anchored, optional leading
/// whitespace) — the `(^|\n)\s*export\b` check.
fn ml_has_export(trimmed: &str) -> bool {
    trimmed.lines().any(|l| head_word(l.trim_start(), "export"))
}

/// Supply the missing `export` (and, for a bare expression, a `main`) so a snippet compiles — a faithful
/// port of wrapModule.ts `wrapModule`. sexpr gathers top-level forms under one `(do …)` (s-expr has no bare
/// multi-form top level); ml uses its native newline-separated top level.
pub fn wrap_module(src: &str, surface: Surface) -> String {
    let trimmed = src.trim();
    match surface {
        Surface::Sexpr => {
            if head_word(trimmed, "(module") || head_word(trimmed, "(do") {
                return trimmed.to_string();
            }
            if head_word(trimmed, "(pragma")
                || head_word(trimmed, "(def")
                || head_word(trimmed, "(type")
                || head_word(trimmed, "(effect")
                || head_word(trimmed, "(Unit.define")
            {
                return format!(
                    "(do {trimmed} (export {}))",
                    export_names(trimmed, surface).join(" ")
                );
            }
            format!("(do (def (main) {trimmed}) (export main))")
        }
        Surface::Ml => {
            if head_word(trimmed, "module") || ml_has_export(trimmed) {
                return trimmed.to_string();
            }
            if trimmed.starts_with("@!")
                || head_word(trimmed, "def")
                || head_word(trimmed, "type")
                || head_word(trimmed, "effect")
                || head_word(trimmed, "Unit.define")
            {
                return format!(
                    "{trimmed}\nexport {{ {} }}",
                    export_names(trimmed, surface).join(", ")
                );
            }
            format!("def main() = {trimmed}\nexport {{ main }}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Surface::{Ml, Sexpr};
    use super::*;

    #[test]
    fn sexpr_shapes() {
        // bare expression → def main + export
        assert_eq!(
            wrap_module("(f 5)", Sexpr),
            "(do (def (main) (f 5)) (export main))"
        );
        // defs block leading with (def, main present → export main
        assert_eq!(
            wrap_module("(def (main) (f 5))\n(def (f x) x)", Sexpr),
            "(do (def (main) (f 5))\n(def (f x) x) (export main))"
        );
        // defs block, no main → export the def names
        assert_eq!(
            wrap_module("(def (c-to-f c) c)", Sexpr),
            "(do (def (c-to-f c) c) (export c-to-f))"
        );
        // already a (do …) / (module …) → untouched
        assert_eq!(
            wrap_module("(do (def (main) 1) (export main))", Sexpr),
            "(do (def (main) 1) (export main))"
        );
        assert_eq!(wrap_module("(module m)", Sexpr), "(module m)");
        // (type …) block, no def → export main fallback
        assert_eq!(
            wrap_module("(type T (A) (B))", Sexpr),
            "(do (type T (A) (B)) (export main))"
        );
    }

    #[test]
    fn ml_shapes() {
        assert_eq!(
            wrap_module("f(5)", Ml),
            "def main() = f(5)\nexport { main }"
        );
        assert_eq!(
            wrap_module("def c-to-f(c) = c", Ml),
            "def c-to-f(c) = c\nexport { c-to-f }"
        );
        // already has export → untouched
        assert_eq!(
            wrap_module("def f() = 1\nexport { f }", Ml),
            "def f() = 1\nexport { f }"
        );
        assert_eq!(wrap_module("module M {}", Ml), "module M {}");
    }

    #[test]
    fn def_names_and_boundary() {
        assert_eq!(
            top_level_def_names("(def (main) 1) (def (f x) x)", Sexpr),
            vec!["main", "f"]
        );
        assert_eq!(
            top_level_def_names("def main() = 1\ndef f(x) = x", Ml),
            vec!["main", "f"]
        );
        // `(default …)` must NOT be read as a `(def …)` — boundary after `def`.
        assert!(!head_word("(default 3)", "(def"));
        assert!(head_word("(def (f) 1)", "(def"));
        // dedup
        assert_eq!(
            top_level_def_names("(def (f) 1) (def (f) 2)", Sexpr),
            vec!["f"]
        );
    }
}
