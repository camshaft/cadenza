//! The import-CLOSURE loader — shared by `cdz check`/`type`/`def`/… (whole-project analysis) and
//! `cdz lsp` (cross-file diagnostics/hover/def). One implementation so the editor follows a document's
//! `(import …)` closure with the EXACT logic the one-shot commands use (no ~50-line duplicate in
//! `lsp.rs`, the divergence hazard we avoided by extracting `crate::fix`).
//!
//! [`load`] walks the entry's transitive imports (resolved as siblings in the entry's directory), reading
//! each file's source through a caller-supplied `open` resolver: a caller can OVERRIDE a file's text
//! (`cdz lsp` returns an open buffer's unsaved edits) or fall back to disk (`cdz check` passes `|_| None`,
//! byte-identical to a pure-disk load). The closure module holds ZERO editor knowledge — `open` is just a
//! `&dyn Fn(&Path) -> Option<String>`.

use std::path::Path;

/// One file of an import closure: its on-disk path, package name (= file stem, the `(import "stem" …)`
/// key), source, parsed arenas + span table, and recovered-parse-error count.
pub(crate) struct LoadedFile {
    /// On-disk path (what a diagnostic's `path:line:col` prints, and what the reporter's fixes edit).
    pub(crate) path: String,
    /// Package name = file stem — the identifier an `(import "stem" …)` names it by, and the `ast`
    /// artifact name `link()` indexes it under.
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) arenas: cadenza_syntax::Arenas,
    pub(crate) spans: cadenza_syntax::spans::SpanTable,
    /// Count of RECOVERED parse errors (the ML reader never aborts — it prints each syntax error, then
    /// returns a truncated arena of `<error>` placeholders). Nonzero means this file did not fully parse,
    /// so `cdz check` must report FAILURE even when the recovered arena carries no semantic fault, and
    /// suppress the `<error>`-placeholder cascade. Always `0` for an s-expr file (its reader hard-errors).
    pub(crate) parse_errors: usize,
}

/// The IMPORT PATHS a top-level program declares — the `"path"` string of each `(import "path" …)`
/// clause at the program's root. Used to walk a check's import closure (only the files the entry
/// TRANSITIVELY imports are pulled in, not every sibling in the directory). Reads the arenas directly
/// (the same shape `link::resolve_import_clause` parses): a root that is a `(do …)` has one item per
/// child; a bare single top-level form is its own root. A malformed/aliased import (no string path)
/// contributes nothing here — `link()` reports it as a diagnostic once the file is pulled in.
pub(crate) fn declared_import_paths(arenas: &cadenza_syntax::Arenas) -> Vec<String> {
    // Peel a leading `(comment/doc …)` off the root before matching `(do …)` — a doc'd program root is
    // wrapped, and we must see the `(do …)` inside it to find the imports.
    let root = crate::unwrap_comment(arenas, arenas.root);
    // The items to scan: a `(do …)` root's children, else the single root form itself.
    let items: Vec<cadenza_syntax::StructId> = match arenas.as_form(root, "do") {
        Some(tail) => tail.to_vec(),
        None => vec![root],
    };
    let mut paths = Vec::new();
    for item in items {
        // A `//` line comment / `///` doc on an import wraps it as `(comment "…" (import …))`; peel the
        // wrapper so the import is detected (else the closure walk misses it and `import` looks unmodeled).
        let item = crate::unwrap_comment(arenas, item);
        if let Some(tail) = arenas.as_form(item, "import")
            && let Some(&path_id) = tail.first()
            && let Some(path) = arenas.as_str(path_id)
        {
            paths.push(path.to_string());
        }
    }
    paths
}

/// Resolve an imported package `name` to a sibling file in `dir` — the first of `<name>.{cdz,ml,sexp,
/// sexpr}` that exists. `None` if no sibling matches (an unresolved import — the compiler reports it).
pub(crate) fn resolve_import_file(dir: &Path, name: &str) -> Option<String> {
    for ext in [".cdz", ".ml", ".sexp", ".sexpr"] {
        let candidate = dir.join(format!("{name}{ext}"));
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Load `entry` and the transitive closure of the files it `(import …)`s (resolved as siblings in the
/// entry's directory). The entry is element 0; the rest are its imported libraries in breadth-first
/// discovery order (deterministic). A file that fails to load, or an import naming no sibling file, is
/// SKIPPED here (not fatal) — the compiler then reports the unresolved import as a normal diagnostic, so
/// `cdz check` still surfaces a helpful error rather than aborting. Dedups by package name (the import
/// target key), so a diamond or a cycle terminates.
///
/// Each file's source is read through `open`: `open(path)` returns `Some(text)` to OVERRIDE that file's
/// text (an editor's live buffer) or `None` to read from disk. `cdz check` passes `|_| None` (pure disk,
/// byte-identical to before); `cdz lsp` passes a resolver over its open documents so an unsaved edit to an
/// imported sibling is analyzed, not its stale on-disk version.
pub(crate) fn load(
    entry: &str,
    open: &dyn Fn(&Path) -> Option<String>,
) -> Result<Vec<LoadedFile>, String> {
    let (source, arenas, spans, parse_errors) = read_or_open(entry, open)?;
    let dir = Path::new(entry)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let mut files = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // A work queue of import paths still to resolve; seed it from the entry's own imports.
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let entry_name = crate::program_name(entry);
    seen.insert(entry_name.clone());
    for p in declared_import_paths(&arenas) {
        queue.push_back(p);
    }
    files.push(LoadedFile {
        path: entry.to_string(),
        name: entry_name,
        source,
        arenas,
        spans,
        parse_errors,
    });

    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue; // already loaded (dedup diamonds / break cycles)
        }
        let Some(path) = resolve_import_file(&dir, &name) else {
            continue; // unresolved import — the compiler reports it as a diagnostic
        };
        let (source, arenas, spans, parse_errors) = match read_or_open(&path, open) {
            Ok(t) => t,
            // An imported file that itself fails to parse: skip it (its importer will fault on the
            // missing name). Don't abort the whole check on a library's parse error.
            Err(_) => continue,
        };
        for p in declared_import_paths(&arenas) {
            queue.push_back(p);
        }
        files.push(LoadedFile {
            path,
            name,
            source,
            arenas,
            spans,
            parse_errors,
        });
    }
    Ok(files)
}

/// Read `file`'s source via the `open` overlay (an editor buffer) if it returns `Some`, else from disk,
/// then parse it (surface from the extension) — the per-file load step of [`load`].
fn read_or_open(
    file: &str,
    open: &dyn Fn(&Path) -> Option<String>,
) -> Result<
    (
        String,
        cadenza_syntax::Arenas,
        cadenza_syntax::spans::SpanTable,
        usize,
    ),
    String,
> {
    match open(Path::new(file)) {
        // The editor has this file open — parse its live buffer text (no disk read).
        Some(text) => crate::parse_program_spanned_counted(file, text),
        // Not open — the ordinary disk load.
        None => crate::load_program_spanned_counted(file),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `source` as an s-expr program (via the canonical loader, keyed off a `.sexp` name) and return
    /// the declared import paths — exercising `declared_import_paths` over the real arena shape a closure
    /// walk reads, not a hand-built arena.
    fn imports_of(source: &str) -> Vec<String> {
        let (_src, arenas, _spans, _errs) =
            crate::parse_program_spanned_counted("t.sexp", source.to_string())
                .expect("the fixture parses");
        declared_import_paths(&arenas)
    }

    #[test]
    fn declared_import_paths_reads_a_multi_form_do_root() {
        // A multi-form program is a `(do …)` root; each `(import "path" (name…))` clause contributes its
        // PATH string, in order. Two imports + a def → both paths, def ignored.
        assert_eq!(
            imports_of("(do (import \"lib-a\" (f)) (import \"lib-b\" (g)) (def main 1))"),
            vec!["lib-a".to_string(), "lib-b".to_string()]
        );
    }

    #[test]
    fn declared_import_paths_reads_a_bare_single_form_root() {
        // A single top-level form is its OWN root (no `(do …)` wrapper). A lone import is still detected —
        // the `None => vec![root]` arm.
        assert_eq!(
            imports_of("(import \"solo\" (h))"),
            vec!["solo".to_string()]
        );
    }

    #[test]
    fn declared_import_paths_is_empty_when_there_are_no_imports() {
        // A program with no import clause declares nothing — the closure walk pulls in no siblings.
        assert!(imports_of("(do (def main 1) (def helper 2))").is_empty());
        assert!(imports_of("(def main 1)").is_empty());
    }

    #[test]
    fn declared_import_paths_ignores_a_malformed_import_with_no_string_path() {
        // A malformed import whose first tail element is NOT a string (an aliased/qualified form the parser
        // preserves, or a bare `(import)`) contributes NOTHING here — `link()` reports it as a diagnostic
        // once the file is pulled in; the closure walk simply doesn't chase a non-path. A well-formed
        // import alongside it is still collected (the malformed one doesn't poison the scan).
        assert_eq!(
            imports_of("(do (import \"good\" (f)) (import bad (g)) (def main 1))"),
            vec!["good".to_string()],
            "a string-path import is kept; a non-string-path import is skipped, not fatal"
        );
    }

    /// A throwaway directory unique to `tag`, created empty. The caller populates + removes it.
    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cdz-closure-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn resolve_import_file_finds_a_sibling_by_each_supported_extension() {
        // An import names a bare stem; the resolver appends each supported extension and returns the first
        // existing sibling file. One file per extension, one at a time, so each extension is exercised.
        for ext in [".cdz", ".ml", ".sexp", ".sexpr"] {
            let dir = tmp_dir(&format!("ext{ext}"));
            let path = dir.join(format!("lib{ext}"));
            std::fs::write(&path, "(def x 1)").unwrap();
            let got = resolve_import_file(&dir, "lib");
            assert_eq!(
                got.as_deref(),
                path.to_str(),
                "an import `lib` resolves to the sibling lib{ext}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn resolve_import_file_prefers_earlier_extensions_in_the_precedence_order() {
        // When multiple sibling files share a stem, the resolver picks by the FIXED precedence
        // `.cdz` > `.ml` > `.sexp` > `.sexpr` — a determinism guarantee the package/LSP path relies on
        // (so which surface an ambiguous import resolves to never depends on filesystem enumeration order).
        let dir = tmp_dir("precedence");
        // Write ALL four; `.cdz` must win.
        for ext in [".cdz", ".ml", ".sexp", ".sexpr"] {
            std::fs::write(dir.join(format!("lib{ext}")), "(def x 1)").unwrap();
        }
        assert_eq!(
            resolve_import_file(&dir, "lib").as_deref(),
            dir.join("lib.cdz").to_str(),
            ".cdz wins over .ml/.sexp/.sexpr"
        );
        // Remove `.cdz`; now `.ml` wins.
        std::fs::remove_file(dir.join("lib.cdz")).unwrap();
        assert_eq!(
            resolve_import_file(&dir, "lib").as_deref(),
            dir.join("lib.ml").to_str(),
            ".ml wins once .cdz is gone"
        );
        // Remove `.ml`; now `.sexp` wins over `.sexpr`.
        std::fs::remove_file(dir.join("lib.ml")).unwrap();
        assert_eq!(
            resolve_import_file(&dir, "lib").as_deref(),
            dir.join("lib.sexp").to_str(),
            ".sexp wins over .sexpr"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_import_file_is_none_for_a_missing_sibling_or_a_directory() {
        let dir = tmp_dir("missing");
        // No sibling with any supported extension → None.
        assert!(
            resolve_import_file(&dir, "nope").is_none(),
            "an import naming no sibling file resolves to None"
        );
        // A DIRECTORY named `libdir.cdz` is not a file → still None (the `is_file` guard, not `exists`).
        std::fs::create_dir_all(dir.join("libdir.cdz")).unwrap();
        assert!(
            resolve_import_file(&dir, "libdir").is_none(),
            "a directory matching the name+ext is not a loadable file"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
