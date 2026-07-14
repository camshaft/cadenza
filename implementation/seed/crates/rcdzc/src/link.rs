//! `link` — package linking: merge N named `ast` artifacts into ONE compilation unit before the
//! pure pipeline runs (`DESIGN-package-linking.md`). Each file is a module; all files are spliced
//! into one [`Arenas`] under a synthesized `(do …)` root, so the existing `Db::load` sees exactly
//! one program in one arena — the same thing it sees for a single file, just assembled from many.
//!
//! Every module in the tree is supplied as its CANONICAL BINARY AST (an `ast`-kinded artifact — the
//! codec's bytes, `ast-encoding.md`), so linking reads each module by its binary form and its identity
//! is that AST, never a textual rendering:
//!
//= spec/contracts/source-tree-encoding.md#a-module-is-stored-as-its-canonical-binary-ast
//# Each module in a source tree MUST be stored as the canonical binary AST fixed by the ast-encoding contract.
//!
//= spec/contracts/source-tree-encoding.md#a-module-is-stored-as-its-canonical-binary-ast
//# The canonical encoding of the tree MUST NOT depend on any textual rendering of a module, because a module's identity is its binary AST rather than a rendering of it.
//!
//! This is INTRA-PACKAGE linking only: nothing crosses a component boundary, so there is zero
//! component-ABI / envelope work. Monomorphization is the existing β-reduction; one component is the
//! existing backend. The link step is the structured analogue of the bootstrap Makefile's `cat`, but
//! at the arena level with real ids: it appends each file's `leaves`/`structure` with a per-file id
//! offset and re-parents every file's top-level items under one `(do …)`.
//!
//! Increment status (`DESIGN-package-linking.md` §8): this module realizes steps 2, 3, and 4 — the
//! arena splice + `FileSpan` demux table (step 2), explicit `(import …)` + per-file visibility (step
//! 3), and cyclic-import + colliding-import rejection (step 4). `link()` reads each file's `(import
//! "path" (name…))` clauses (compile-time LINK DIRECTIVES, NOT spliced as runtime items) and each
//! file's `(export …)` public surface into a per-file [`FileScope`]; only the ENTRY file's `(export
//! …)` survives into the merged `(do …)`, so `db.exports` IS the component boundary. Name resolution
//! is then FILE-SCOPED (`resolve.rs`): a bare name in file `f` resolves against `f`'s own defs and
//! `f`'s imports, never a sibling's defs. A colliding imported name → CDZ0201; an import cycle
//! (`find_import_cycle`, a back-edge DFS over the import graph) → CDZ0201. A linked package also emits
//! the `link-map` OUTPUT artifact (step 5, `encode_link_map`) so a consumer demuxes a cross-file
//! diagnostic's global node id → `(file, local id)`. The bootstrap re-author (step 6) is what remains.
//!
//! A sibling file's definition is reachable ONLY through an explicit `(import …)` naming it: file-scoped
//! resolution never sees another file's defs, and an import binds exactly the names it lists:
//!
//= spec/capabilities/modules-and-namespaces.md#imports-are-explicit
//# A name defined in another module MUST be brought into scope only by an explicit import.
//!
//= spec/capabilities/modules-and-namespaces.md#imports-are-explicit
//# An import MUST NOT introduce names into scope beyond those it explicitly names or the module it explicitly binds.

use crate::ast::{Arenas, Leaf, LeafId, Struct, StructId};
use crate::diag::Reject;

/// The input-artifact kind that names the package ENTRY file. Its bytes are the entry's artifact name
/// (a UTF-8 string) — the file whose `(export …)` forms the component boundary. It rides the artifact
/// stream exactly like the `sidecar`/`spans` inputs (`DESIGN-package-linking.md` §3c): a new kind, a
/// `.find(kind == KIND_ENTRY)`, no change to `compile`'s signature. Absent + a single `ast` = today's
/// single-file compile; absent + multiple `ast` = a package with no named entry, which declines.
pub const KIND_ENTRY: &str = "entry";

/// The input-artifact kind that names the INTERFACE a PROVIDER component publishes its exports under
/// (X4b, `DESIGN-cross-component-interop-rcdzc.md`). Its bytes are the interface name (`cadenza:pkg/iface`)
/// a peer consumer's `(extern "cadenza:pkg/iface" …)` binds to. Rides the artifact stream like
/// `KIND_ENTRY`. Absent (the common case) → the component exports its boundary funcs at top level
/// (byte-identical to before); present → `emit` wraps them as that named interface instance so a peer can
/// import them. The compile REQUEST specifies it (operator: peers must agree on the published name).
pub const KIND_COMPONENT_NAME: &str = "component-name";

/// The OUTPUT-artifact kind carrying the diagnostics DEMUX table for a linked package
/// (`DESIGN-package-linking.md` §6). A cross-file diagnostic's `node` is a GLOBAL merged `StructId`;
/// with several files spliced into one arena, that global id no longer maps to a single file's span
/// table. This artifact lets a consumer demux: a global node `n` falls in exactly one file's
/// `[struct_base, struct_base+struct_count)` range → `(path, n - struct_base)` = the per-file LOCAL id
/// that file's own span table (or its `spans` input) is keyed by. One line per file:
/// `<path>\t<struct_base>\t<struct_count>`, mirroring the `uses` artifact's node-id-per-line style —
/// a consumer-side two-level back-reference (global id → (file, local id) → span), so the compiler
/// stays span-free. Present only for a multi-file package.
pub const KIND_LINK_MAP: &str = "link-map";

/// Encode a package's `FileSpan` table as the `link-map` artifact bytes — one line per file,
/// `<path>\t<struct_base>\t<struct_count>`, in splice order. The inverse a consumer applies:
/// find the file whose `[base, base+count)` contains a diagnostic's global node id, then subtract
/// `base` for the per-file local id.
pub fn encode_link_map(files: &[FileSpan]) -> Vec<u8> {
    let mut out = String::new();
    for f in files {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            f.path, f.struct_base, f.struct_count
        ));
    }
    out.into_bytes()
}

/// Decode the `link-map` artifact bytes back into its `FileSpan` list — the inverse of
/// [`encode_link_map`], so a CLI reporter can demux a linked-package diagnostic's GLOBAL node id to the
/// `(file, local id)` its per-file span table is keyed by. A malformed line (wrong column count, a
/// non-numeric base/count) is skipped rather than failing the whole decode — the reporter degrades to a
/// location-less diagnostic, never a crash. `path` may itself contain no tab (an artifact name), so the
/// split takes exactly the first two tabs.
pub fn decode_link_map(bytes: &[u8]) -> Vec<FileSpan> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| {
            let mut cols = line.rsplitn(3, '\t');
            // Right-split: count, base, then the remaining prefix is the path (which never contains a
            // tab in practice, but rsplitn keeps a tabbed path intact as the final piece).
            let count = cols.next()?.parse::<u32>().ok()?;
            let base = cols.next()?.parse::<u32>().ok()?;
            let path = cols.next()?.to_string();
            Some(FileSpan {
                path,
                struct_base: base,
                struct_count: count,
            })
        })
        .collect()
}

/// One file's contribution to the merged arena — the span of structure ids it owns, so a global
/// `StructId` demuxes back to `(path, local id)` for source mapping (`DESIGN-package-linking.md` §6).
/// A diagnostic's global node id `n` falls in exactly one file's `[struct_base, struct_base +
/// struct_count)` range; `n - struct_base` is the per-file local id that file's own span table is
/// keyed by. The link-synthesized `(do …)` root sits OUTSIDE every file's range (it belongs to no
/// source file), so it never mis-demuxes to a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileSpan {
    /// The file's artifact name (the `<path>` an `(import …)` names it by).
    pub path: String,
    /// The first merged `StructId` this file's structure entries occupy.
    pub struct_base: u32,
    /// How many structure entries this file contributed (its range length).
    pub struct_count: u32,
}

impl FileSpan {
    /// Whether the global structure id `id` falls in this file's range.
    pub fn contains(&self, id: StructId) -> bool {
        let n = id.0;
        n >= self.struct_base && n < self.struct_base + self.struct_count
    }
}

/// One resolved `(import "path" (name…))` binding: a public name of another file brought into the
/// importing file's scope under `local` (the named-list form binds `local == exported`). Resolved at
/// link time: `from_file` is the SPLICED index of the module named by `"path"`, and `exported` has
/// already been checked to be in that module's `(export …)` list (`DESIGN-package-linking.md` §4).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Import {
    /// The name introduced into the importing file's scope.
    pub local: String,
    /// The SPLICED index (into `LinkedProgram.files`/`scopes`) of the module this name comes from.
    pub from_file: usize,
    /// The name as the source module exports it (== `local` for the named-list form).
    pub exported: String,
    /// The `(import …)` clause's GLOBAL occurrence, for a diagnostic to anchor to.
    pub occ: StructId,
}

/// One file's link-time surface: the names it makes public (`(export …)`) and the names it pulls in
/// (`(import …)`). Parallel to `LinkedProgram.files` (same spliced index). A file's importable surface
/// IS its export list (`modules-and-namespaces.md` §Visibility Is Explicit — one mechanism, reused).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FileScope {
    /// The public names this file exports (its `(export …)` clause names).
    pub exports: Vec<String>,
    /// The names this file imports from sibling modules.
    pub imports: Vec<Import>,
}

/// The per-file linkage a merged multi-file arena carries into `Db::load_linked` — the demux table +
/// import/export scopes that make name resolution FILE-SCOPED. A SINGLE-file compile carries NO
/// linkage (`None`): its namespace is flat, byte-identical to the pre-linking compiler. This is
/// `Some` only for a genuine package (>1 file), so the file-scoping logic never touches the
/// overwhelmingly-common single-file path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Linkage {
    /// The `StructId → file` demux table (splice order), keyed by structure-id range.
    pub files: Vec<FileSpan>,
    /// Per-file import/export surface, parallel to `files`.
    pub scopes: Vec<FileScope>,
}

impl Linkage {
    /// The index of the file whose `FileSpan` contains `id`, or `None` for a node in no file — a
    /// prelude/evaluator-synthesized node (a β-reduced body copy, a built `(Int W)` module) or the
    /// synthesized `(do …)` root. A `None` result is the signal for resolution to fall back safely
    /// (`DESIGN-package-linking.md` §4, the β-copy-hygiene note).
    pub fn file_of(&self, id: StructId) -> Option<usize> {
        self.files.iter().position(|f| f.contains(id))
    }
}

/// The result of linking a package: the merged arena (ready for `Db::load`), the per-file demux table,
/// the per-file import/export surface, and which file is the entry. Files are in the CALLER's order
/// (request order — deterministic; a topological order is a later refinement, §3a/§10). Correctness
/// does not depend on the order: a reference resolves against ITS OWN file's scope (own defs +
/// imports), never a global first-wins scan, so a same-named sibling def is invisible unless imported.
#[derive(Clone, PartialEq, Debug)]
pub struct LinkedProgram {
    /// All files' structure/leaves appended (ids offset per file), re-rooted under a `(do …)`.
    pub arenas: Arenas,
    /// One entry per input file, in splice order — the `StructId → file` demux table.
    pub files: Vec<FileSpan>,
    /// Per-file import/export surface, parallel to `files`.
    pub scopes: Vec<FileScope>,
    /// Index into `files` of the entry file (whose `(export …)` forms the component boundary).
    pub entry: usize,
}

impl LinkedProgram {
    /// The linkage (demux + scopes) this program carries into `Db::load_linked`.
    pub fn linkage(&self) -> Linkage {
        Linkage {
            files: self.files.clone(),
            scopes: self.scopes.clone(),
        }
    }
}

/// Link a package of named `ast` artifacts into one compilation unit. `files` is `(artifact name,
/// decoded arena)` in the order the caller supplied them; `entry` names the entry file (its
/// `(export …)` forms the component boundary). Splice order is the given order (a topological order
/// over the import graph is a later refinement, `DESIGN-package-linking.md` §3a/§10 — with no imports
/// yet, request order is already deterministic and dependency-respecting).
///
/// Returns a [`Reject`] if `files` is empty (nothing to compile) or `entry` names no supplied file
/// (the caller asked to build a package whose entry is absent — decline, don't guess).
pub fn link(files: &[(String, Arenas)], entry: &str) -> Result<LinkedProgram, Reject> {
    if files.is_empty() {
        return Err(Reject::decline("package has no `ast` input files to link"));
    }
    let entry_ix = match files.iter().position(|(name, _)| name == entry) {
        Some(i) => i,
        None => {
            // A did-you-mean over the SUPPLIED file names — a mistyped `--entry app`→`apps` is the
            // closed-set-suggestion case (like a typoed import path, M27, or an unbound name): the
            // candidate pool IS the package's files, so the suggestion always names a real one. Uses the
            // shared `suggest::nearest` (its 1-char/empty guards apply).
            let names = files.iter().map(|(n, _)| n.as_str());
            let msg = match crate::diag::suggest::nearest(entry, names) {
                Some(near) => format!(
                    "package entry `{entry}` names no supplied `ast` file — did you mean `{near}`?"
                ),
                None => format!("package entry `{entry}` names no supplied `ast` file"),
            };
            return Err(Reject::decline(msg));
        }
    };

    // Index every file by name, so an `(import "path" …)` resolves its target module to a spliced
    // index. A duplicate file name is ambiguous — decline (which module does `"path"` mean?).
    let mut name_to_ix: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, (name, _)) in files.iter().enumerate() {
        if name_to_ix.insert(name.as_str(), i).is_some() {
            return Err(Reject::decline(format!(
                "package has two `ast` files named `{name}` (ambiguous import target)"
            )));
        }
    }

    // Combined arenas, built by append-with-offset. Leaves are copied verbatim (they are values —
    // dedup across files is unnecessary for correctness); structure entries are remapped by this
    // file's leaf/struct base. We gather each file's remapped TOP-LEVEL items to re-parent under one
    // synthesized `(do …)` root at the end.
    let mut leaves: Vec<Leaf> = Vec::new();
    let mut structure: Vec<Struct> = Vec::new();
    let mut file_spans: Vec<FileSpan> = Vec::with_capacity(files.len());
    let mut do_children: Vec<StructId> = Vec::new();
    // Per-file export NAME sets, gathered in the first pass so an import can be validated against the
    // target module's public surface in the second (a file may import a name from a file spliced later).
    let mut exports_of: Vec<Vec<String>> = Vec::with_capacity(files.len());
    // Per-file DEFINED names (top-level def/type/effect) — parallel to `exports_of`; used only to make an
    // "imported name is defined but not exported" diagnostic actionable.
    let mut defined_of: Vec<Vec<String>> = Vec::with_capacity(files.len());
    // Per-file base offsets, kept so the second pass can map an import clause's local id → global.
    let mut struct_bases: Vec<u32> = Vec::with_capacity(files.len());

    for (path, ast) in files {
        let leaf_base = leaves.len() as u32;
        let struct_base = structure.len() as u32;
        struct_bases.push(struct_base);

        // Copy this file's leaves verbatim.
        leaves.extend(ast.leaves.iter().cloned());

        // Copy this file's structure, shifting every embedded id by this file's base.
        for entry in &ast.structure {
            structure.push(match entry {
                Struct::Atom(LeafId(l)) => Struct::Atom(LeafId(l + leaf_base)),
                Struct::List(children) => Struct::List(
                    children
                        .iter()
                        .map(|c| StructId(c.0 + struct_base))
                        .collect(),
                ),
            });
        }

        // This file's top-level items, remapped, become children of the combined `(do …)` — EXCEPT:
        //  - `(import …)` clauses are compile-time LINK DIRECTIVES, not runtime items: they are read
        //    into `FileScope.imports` (below) and NOT spliced (a spliced `(import …)` would be an
        //    unmodeled top-level form and decline the whole program).
        //  - a NON-ENTRY file's `(export …)` clauses do not form the component boundary, so they are
        //    dropped from the merged `(do …)` (their names still populate `exports_of` for import
        //    visibility). Only the ENTRY file's `(export …)` survives → `db.exports` IS the boundary.
        let is_entry = path == entry;
        for item in top_items(ast) {
            if ast.as_form(item, "import").is_some() {
                continue;
            }
            if ast.as_form(item, "export").is_some() && !is_entry {
                continue;
            }
            do_children.push(StructId(item.0 + struct_base));
        }

        // Gather this file's public surface (its `(export …)` names) for import validation, AND its
        // DEFINED names (top-level `def`/`type`/`effect`) — the latter lets an import of a name the file
        // DEFINES but does not EXPORT give an actionable "add `export`" message instead of a bare "does
        // not export".
        let mut exports = Vec::new();
        let mut defined = Vec::new();
        for item in top_items(ast) {
            if let Some(tail) = ast.as_form(item, "export") {
                // Gather EVERY name in the clause — `(export a b)` publishes both, matching the main
                // scan (`scan_top_level`). Reading only `tail.first()` here silently dropped every name
                // past the first, so an importer of a valid `(export Color mk)` library saw only `Color`
                // and an `(import "lib" (mk))` was falsely rejected as "does not export mk". A
                // member-access element (`(. T A)`, the concrete-ctor export form) contributes no bare
                // name here; its constructor visibility is handled by the type-export path, not this
                // value-import surface.
                for &s in tail.iter() {
                    if let Some(name) = ast.as_name(s) {
                        exports.push(name.to_string());
                    }
                }
            } else if let Some(name) = top_item_defined_name(ast, item) {
                defined.push(name);
            }
        }
        exports_of.push(exports);
        defined_of.push(defined);

        file_spans.push(FileSpan {
            path: path.clone(),
            struct_base,
            struct_count: ast.structure.len() as u32,
        });
    }

    // Second pass: resolve each file's `(import "path" (name…))` clauses against the (now fully
    // gathered) per-file export sets. An import of an unknown module, or of a name that module does
    // not export, is a coded reject (`DESIGN-package-linking.md` §7) — never a silent bind-to-nothing.
    let mut scopes: Vec<FileScope> = Vec::with_capacity(files.len());
    for (fi, (_, ast)) in files.iter().enumerate() {
        let mut imports = Vec::new();
        for item in top_items(ast) {
            if let Some(tail) = ast.as_form(item, "import") {
                resolve_import_clause(
                    ast,
                    item,
                    tail,
                    struct_bases[fi],
                    &name_to_ix,
                    &exports_of,
                    &defined_of,
                    &mut imports,
                )?;
            }
        }
        scopes.push(FileScope {
            exports: exports_of[fi].clone(),
            imports,
        });
    }

    // CYCLIC IMPORTS: the import graph (file → each file it imports FROM) must be acyclic. A back-edge
    // in a DFS is a cycle → a coded reject (CDZ0201). The same shape as the static-recursion call-graph
    // DFS (`eval::is_recursive`), here over the fixed link-time import edges. (Value-level mutual
    // recursion across files is fine — that is a runtime call, not an import edge; only a compile-time
    // dependency LOOP is forbidden.)
    //= spec/capabilities/modules-and-namespaces.md#cyclic-module-dependencies-are-rejected
    //# A set of modules whose import relationships form a cycle MUST be rejected at compile time.
    if let Some(cycle) = find_import_cycle(&scopes) {
        let names: Vec<&str> = cycle.iter().map(|&i| files[i].0.as_str()).collect();
        // Anchor at the `(import …)` clause forming the cycle's FIRST edge (`cycle[0] → cycle[1]`) — the
        // import whose `from_file` is the next file in the cycle. Its `occ` (a global node id) gives the
        // error a `file:line:col` at a real import in the loop, instead of an unanchored `cdz:` prefix. A
        // link is total (runs outside the Db node-walk that auto-stamps origins), so this must anchor
        // explicitly. Fall back to unanchored only if the edge's import can't be located (defensive; the
        // cycle came from these very import edges, so it is normally present).
        let reject = Reject::coded(
            crate::diag::Code::Malformed,
            format!("cyclic module imports: {}", names.join(" → ")),
        );
        let edge_occ = cycle.first().zip(cycle.get(1)).and_then(|(&from, &to)| {
            scopes[from]
                .imports
                .iter()
                .find(|imp| imp.from_file == to)
                .map(|imp| imp.occ)
        });
        return Err(match edge_occ {
            Some(occ) => reject.at(occ),
            None => reject,
        });
    }

    // Synthesize the `(do …)` root: a fresh `do` name leaf + its atom, then the list whose head is
    // that atom and whose tail is every file's top-level items. These nodes sit AFTER all files, so
    // they are outside every `FileSpan` (they belong to no source file).
    let do_leaf = LeafId(leaves.len() as u32);
    leaves.push(Leaf::Name("do".to_string()));
    let do_atom = StructId(structure.len() as u32);
    structure.push(Struct::Atom(do_leaf));
    let mut root_children = Vec::with_capacity(do_children.len() + 1);
    root_children.push(do_atom);
    root_children.extend(do_children);
    let root = StructId(structure.len() as u32);
    structure.push(Struct::List(root_children));

    Ok(LinkedProgram {
        arenas: Arenas {
            leaves,
            structure,
            root,
        },
        files: file_spans,
        scopes,
        entry: entry_ix,
    })
}

/// Resolve one `(import "path" (name…))` clause of the file at spliced index whose base is `base`,
/// appending an [`Import`] per imported name. `tail` is the clause's arguments in the file's LOCAL
/// arena `ast`; `item` is the clause's LOCAL occurrence (its global id is `item + base`, used to
/// anchor a diagnostic). Outcomes:
///  - a malformed clause (not `("path" (name…))`) → CDZ0201 (ill-formed);
///  - the ALIAS form `(import "path" alias)` → a DECLINE (a later phase, §2/§7) — not ill-formed;
///  - an unknown module `"path"`, or a name the target does not `(export …)` (visibility, §4) →
///    CDZ0201 (a positively-proven ill-formed program: it names a file / a private name that the
///    package does not provide, exactly as an unbound name is ill-formed).
// Each parameter is a distinct per-file link input the clause reads (the arena, the clause occurrence,
// the name→file map, the export + defined tables); bundling them into a struct would only obscure the
// one call site. One over the lint's soft threshold.
#[allow(clippy::too_many_arguments)]
fn resolve_import_clause(
    ast: &Arenas,
    item: StructId,
    tail: &[StructId],
    base: u32,
    name_to_ix: &std::collections::HashMap<&str, usize>,
    exports_of: &[Vec<String>],
    defined_of: &[Vec<String>],
    out: &mut Vec<Import>,
) -> Result<(), Reject> {
    use crate::diag::Code;
    let occ = StructId(item.0 + base);
    // `(import "path" <spec>)` — exactly two arguments: the module path string and the name spec.
    let (Some(&path_id), Some(&spec_id)) = (tail.first(), tail.get(1)) else {
        return Err(Reject::coded(
            Code::Malformed,
            "malformed `(import …)`: expected `(import \"path\" (name…))`",
        )
        .at(occ));
    };
    let Some(path) = ast.as_str(path_id) else {
        return Err(Reject::coded(
            Code::Malformed,
            "`(import …)` path must be a string literal naming a package file",
        )
        .at(occ));
    };
    // The name spec must be a `(name…)` LIST (the named-list form). A bare NAME spec is the ALIAS
    // form `(import "path" alias)`, which needs module-as-record projection — deferred (§2/§7). This
    // is a DECLINE (unrealized), not an ill-formed program.
    let names: &[StructId] = match ast.get(spec_id) {
        Struct::List(items) => items,
        Struct::Atom(_) => {
            return Err(Reject::decline(
                "qualified import `(import \"path\" alias)` is a later phase; \
                 use the named-list form `(import \"path\" (name…))`",
            )
            .at(occ));
        }
    };
    let Some(&from_file) = name_to_ix.get(path) else {
        // A did-you-mean over the OTHER package files — a mistyped path (`"libb"` for `lib`) is the
        // file-name analogue of the typoed import NAME handled below, so it gets the same treatment
        // (the shared `nearest` guards the 1-char/empty cases). `name_to_ix`'s keys ARE the package's
        // file names, so no extra plumbing is needed.
        let msg = match crate::diag::suggest::nearest(path, name_to_ix.keys().copied()) {
            Some(near) => {
                format!("`(import …)` names unknown package file `{path}` — did you mean `{near}`?")
            }
            None => format!("`(import …)` names unknown package file `{path}`"),
        };
        return Err(Reject::coded(Code::Malformed, msg).at(occ));
    };

    for &name_id in names {
        let Some(name) = ast.as_name(name_id) else {
            return Err(Reject::coded(
                Code::Malformed,
                "`(import …)` name list may contain only bare names",
            )
            .at(occ));
        };
        if !exports_of[from_file].iter().any(|e| e == name) {
            // Distinguish the two reasons the name is not importable, so the message is ACTIONABLE:
            //  - `{path}` DEFINES `{name}` but does not `(export …)` it → say so + name the fix (add an
            //    export to that file), the "private item" case (rustc's "consider making it public").
            //  - `{path}` does not define it at all → the plain "does not export", enriched with a "did
            //    you mean?" over what the file DOES export (a typoed import name).
            let msg = if defined_of[from_file].iter().any(|d| d == name) {
                format!(
                    "`(import …)`: `{path}` defines `{name}` but does not export it — add `export \
                     {{ {name} }}` to `{path}`"
                )
            } else {
                match crate::diag::suggest::nearest(name, &exports_of[from_file]) {
                    Some(near) => format!(
                        "`(import …)`: `{path}` does not export `{name}` — did you mean `{near}`?"
                    ),
                    None => format!("`(import …)`: `{path}` does not export `{name}`"),
                }
            };
            return Err(Reject::coded(Code::Malformed, msg).at(occ));
        }
        // COLLIDING IMPORTED NAMES: two imports binding the SAME local name into one file's scope is a
        // compile-time error (CDZ0201), never resolved by an implicit precedence. This is a
        // positively-proven ill-formed program — a CODED reject, not a decline.
        //= spec/capabilities/modules-and-namespaces.md#colliding-imported-names-are-rejected
        //# Importing two definitions under the same name into one scope MUST be a compile-time error rather than resolved by an implicit precedence.
        if out.iter().any(|i| i.local == name) {
            return Err(Reject::coded(
                crate::diag::Code::Malformed,
                format!("`(import …)`: `{name}` is imported more than once into this file"),
            )
            .at(occ));
        }
        out.push(Import {
            local: name.to_string(),
            from_file,
            exported: name.to_string(),
            occ,
        });
    }
    Ok(())
}

/// Find an import CYCLE in the package's import graph (file `i` → each `imp.from_file` it imports),
/// returning the files on the cycle (in dependency order, closing back to the first) if one exists.
/// An iterative DFS with a three-colour marking (unvisited / on-stack / done): a back-edge to an
/// on-stack node is a cycle. Deterministic (files + edges are fixed), the link-time twin of
/// `eval::is_recursive`'s call-graph DFS. `None` = the import graph is acyclic.
fn find_import_cycle(scopes: &[FileScope]) -> Option<Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unvisited,
        OnStack,
        Done,
    }
    let n = scopes.len();
    let mut mark = vec![Mark::Unvisited; n];
    // Explicit DFS stack of (node, path-so-far) so the cycle can be reconstructed; no native recursion
    // over a potentially-large graph.
    for start in 0..n {
        if mark[start] != Mark::Unvisited {
            continue;
        }
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(start, vec![start])];
        mark[start] = Mark::OnStack;
        while let Some((node, path)) = stack.pop() {
            let mut advanced = false;
            for imp in &scopes[node].imports {
                let to = imp.from_file;
                match mark[to] {
                    Mark::OnStack => {
                        // Back-edge → cycle. Reconstruct from where `to` first appears on the path.
                        let mut cyc: Vec<usize> =
                            if let Some(pos) = path.iter().position(|&x| x == to) {
                                path[pos..].to_vec()
                            } else {
                                vec![to]
                            };
                        cyc.push(to); // close the loop back to the re-entered node
                        return Some(cyc);
                    }
                    Mark::Unvisited => {
                        mark[to] = Mark::OnStack;
                        let mut child_path = path.clone();
                        child_path.push(to);
                        // Re-push the current node so its remaining edges are explored after `to`'s
                        // subtree completes (its on-stack mark is cleared when it is fully done, below).
                        stack.push((node, path.clone()));
                        stack.push((to, child_path));
                        advanced = true;
                        break;
                    }
                    Mark::Done => {}
                }
            }
            if !advanced {
                // No unvisited edge left from `node`: it and its subtree are fully explored.
                mark[node] = Mark::Done;
            }
        }
    }
    None
}

/// The top-level items of one file's arena — the SAME rule `db::top_items` applies (a `(module …)`
/// or `(do …)` root contributes its children; any other root contributes itself). Duplicated here
/// (rather than depending on `db`) because linking runs BEFORE `Db::load` and operates on a bare
/// `Arenas`; the two must stay in lock-step (a change to how a root is unwrapped belongs in both).
fn top_items(ast: &Arenas) -> Vec<StructId> {
    let root = ast.root;
    if let Some(tail) = ast.as_form(root, "module") {
        return tail.get(1..).unwrap_or(&[]).to_vec();
    }
    if let Some(tail) = ast.as_form(root, "do") {
        return tail.to_vec();
    }
    vec![root]
}

/// The name a top-level item DEFINES, if it is a `def`/`type`/`effect` — used only to tell an import of
/// a defined-but-unexported name from an import of an absent one. A `def` is `(def (NAME param…) BODY)`
/// (list signature → its head) or `(def NAME VALUE)` (bare-name value def), mirroring `db::scan_top_level`;
/// a `type`/`effect` is `(type NAME …)`/`(effect NAME …)` (the name is the first tail element). `None`
/// for anything else (an import/export clause, a bare expression).
fn top_item_defined_name(ast: &Arenas, item: StructId) -> Option<String> {
    if let Some(tail) = ast.as_form(item, "def") {
        return match tail.first().map(|&s| (s, ast.get(s))) {
            // `(def (NAME param…) …)` — the signature is a non-empty list; the name is its head.
            Some((_, Struct::List(children))) if !children.is_empty() => {
                ast.as_name(children[0]).map(str::to_string)
            }
            // `(def NAME VALUE)` — a bare-name value def.
            Some((sig, Struct::Atom(_))) => ast.as_name(sig).map(str::to_string),
            _ => None,
        };
    }
    if let Some(tail) = ast
        .as_form(item, "type")
        .or_else(|| ast.as_form(item, "effect"))
    {
        return tail
            .first()
            .and_then(|&s| ast.as_name(s))
            .map(str::to_string);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a one-file arena from a small program via the real s-expr reader (the path the corpus
    /// uses), so the tests exercise real decoded arenas rather than hand-built ones.
    fn arena_of(src: &str) -> Arenas {
        crate::testkit::parse(src)
    }

    #[test]
    fn link_of_one_file_reroots_under_a_do() {
        let a = arena_of("(do (def (main) 42) (export main))");
        let linked = link(&[("only".to_string(), a)], "only").expect("link");
        // The merged root is a `(do …)` whose children are the file's two top-level items.
        assert_eq!(linked.entry, 0);
        assert_eq!(linked.files.len(), 1);
        assert_eq!(linked.arenas.head_name(linked.arenas.root), Some("do"));
        let items = match linked.arenas.get(linked.arenas.root) {
            Struct::List(items) => items.len(),
            _ => panic!("root is not a list"),
        };
        assert_eq!(items, 3); // `do` head + def + export
    }

    #[test]
    fn link_merges_two_files_and_offsets_ids() {
        let a = arena_of("(do (def (a) 1) (export a))");
        let b = arena_of("(do (def (b) 2) (export b))");
        let a_structs = a.structure.len() as u32;
        let linked = link(&[("a".to_string(), a), ("b".to_string(), b)], "b").expect("link");
        assert_eq!(linked.entry, 1); // `b` is the named entry
        assert_eq!(linked.files.len(), 2);
        // File A starts at 0; file B is offset by A's structure length.
        assert_eq!(linked.files[0].struct_base, 0);
        assert_eq!(linked.files[1].struct_base, a_structs);
        // The two file ranges are disjoint and adjacent.
        assert_eq!(
            linked.files[0].struct_base + linked.files[0].struct_count,
            linked.files[1].struct_base
        );
        // The merged `(do …)` gathers both files' DEFS but only the ENTRY (`b`) file's `(export …)`:
        // head + def a + def b + export b = 4 (non-entry `a`'s `(export a)` is dropped — only the
        // entry's exports form the component boundary).
        let items = match linked.arenas.get(linked.arenas.root) {
            Struct::List(items) => items.len(),
            _ => panic!("root is not a list"),
        };
        assert_eq!(items, 4);
        // Each file still records its own public surface for import visibility.
        assert_eq!(linked.scopes[0].exports, vec!["a".to_string()]);
        assert_eq!(linked.scopes[1].exports, vec!["b".to_string()]);
    }

    #[test]
    fn link_rejects_a_missing_entry() {
        let a = arena_of("(do (def (main) 1) (export main))");
        let r = link(&[("only".to_string(), a)], "absent");
        assert!(r.is_err(), "an entry that names no file must decline");
    }

    #[test]
    fn link_rejects_an_empty_package() {
        let files: Vec<(String, Arenas)> = Vec::new();
        assert!(link(&files, "any").is_err());
    }

    /// The Inc-3 GATE (`DESIGN-package-linking.md` §8.3): a cross-file call resolves through an EXPLICIT
    /// `(import …)`, monomorphizes, and emits one component. File `app` imports `helper` from `lib`,
    /// which `lib` exports; `main` calls it. Drives the full `compile()` path (decode → link → Db::load
    /// → resolve → layout → emit) and asserts a real component comes out. The companion test
    /// `an_unimported_sibling_def_is_not_visible` witnesses the other half: WITHOUT the import the same
    /// call is unbound (file-scoping isolates the sibling).
    #[test]
    fn a_cross_file_import_resolves_and_emits_a_component() {
        use crate::abi::Artifact;
        use crate::backend::Target;

        // File `lib` defines + exports a helper; file `app` (the entry) imports and calls it.
        let lib = crate::codec::encode(&arena_of("(do (def (helper) 40) (export helper))"));
        let app = crate::codec::encode(&arena_of(
            "(do (import \"lib\" (helper)) (def (main) (+ (helper) 2)) (export main))",
        ));
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "lib", lib),
            Artifact::new(Artifact::KIND_AST, "app", app),
            Artifact::new(KIND_ENTRY, "entry", b"app".to_vec()),
        ];
        let out = crate::compile(&inputs, &[Target::Wasm]);
        assert!(
            !out.has_error(),
            "cross-file import should compile clean; diagnostics: {:?}",
            out.diagnostics
        );
        assert!(
            out.artifact(Target::Wasm.artifact_kind()).is_some(),
            "a wasm component should be produced from the spliced package"
        );
    }

    /// A multi-file package with NO `entry` marker declines — there is no rule to pick the entry, so
    /// the compiler rejects rather than guessing (`DESIGN-package-linking.md` §3c).
    #[test]
    fn a_multi_file_package_without_an_entry_declines() {
        use crate::abi::Artifact;
        use crate::backend::Target;

        let a = crate::codec::encode(&arena_of("(do (def (a) 1) (export a))"));
        let b = crate::codec::encode(&arena_of("(do (def (b) 2) (export b))"));
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "a", a),
            Artifact::new(Artifact::KIND_AST, "b", b),
        ];
        let out = crate::compile(&inputs, &[Target::Wasm]);
        assert!(
            out.has_error(),
            "a multi-file package with no entry must decline"
        );
    }

    #[test]
    fn a_file_span_demuxes_a_global_id_to_its_file() {
        let a = arena_of("(do (def (a) 1) (export a))");
        let b = arena_of("(do (def (b) 2) (export b))");
        let linked = link(&[("a".to_string(), a), ("b".to_string(), b)], "a").expect("link");
        // A node in B's range is claimed by B, not A.
        let in_b = StructId(linked.files[1].struct_base);
        assert!(!linked.files[0].contains(in_b));
        assert!(linked.files[1].contains(in_b));
        // The synthesized `(do …)` root sits outside every file's range (it belongs to no file).
        assert!(!linked.files[0].contains(linked.arenas.root));
        assert!(!linked.files[1].contains(linked.arenas.root));
    }

    use crate::abi::Artifact;
    use crate::backend::Target;

    /// Compile a two-file package and return its `CompileOutput` — a helper for the visibility tests.
    fn compile_package(lib_src: &str, app_src: &str) -> crate::abi::CompileOutput {
        let lib = crate::codec::encode(&arena_of(lib_src));
        let app = crate::codec::encode(&arena_of(app_src));
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "lib", lib),
            Artifact::new(Artifact::KIND_AST, "app", app),
            Artifact::new(KIND_ENTRY, "entry", b"app".to_vec()),
        ];
        crate::compile(&inputs, &[Target::Wasm])
    }

    /// The other half of file-scoping: WITHOUT an `(import …)`, a sibling file's def is INVISIBLE.
    /// `app`'s `main` calls `helper` (defined + exported by `lib`) but does not import it → unbound.
    #[test]
    fn an_unimported_sibling_def_is_not_visible() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (def (main) (+ (helper) 2)) (export main))",
        );
        assert!(out.has_error(), "an unimported sibling must be unbound");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("helper")),
            "expected an unbound-name error for `helper`; got {:?}",
            out.diagnostics
        );
    }

    /// Visibility is the export list: importing a name a module does NOT export is a reject, even
    /// though the def exists in that file (`DESIGN-package-linking.md` §4).
    #[test]
    fn importing_a_non_exported_name_declines() {
        // `lib` defines `helper` but does NOT export it.
        let out = compile_package(
            "(do (def (helper) 40))",
            "(do (import \"lib\" (helper)) (def (main) (helper)) (export main))",
        );
        assert!(
            out.has_error(),
            "importing an unexported name must be rejected"
        );
        // The message is ACTIONABLE: `lib` DEFINES `helper` but does not export it, so name the fix (add
        // an export) rather than the bare "does not export".
        assert!(
            out.diagnostics.iter().any(|d| d
                .message
                .contains("defines `helper` but does not export it")
                && d.message.contains("add `export { helper }`")),
            "expected the actionable add-export message; got {:?}",
            out.diagnostics
        );
    }

    /// A TYPO of an exported name — the file does not DEFINE it either — gets a "did you mean?" over the
    /// module's actual exports, the import analogue of the unbound-name suggestion.
    #[test]
    fn importing_a_typoed_exported_name_suggests_the_nearest() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lib\" (helpr)) (def (main) (helpr)) (export main))",
        );
        assert!(out.has_error(), "a typoed import name must be rejected");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("does not export `helpr`")
                    && d.message.contains("did you mean `helper`?")),
            "expected a did-you-mean suggestion; got {:?}",
            out.diagnostics
        );
    }

    /// An import naming an unknown package file declines.
    #[test]
    fn importing_from_an_unknown_module_declines() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"nope\" (helper)) (def (main) (helper)) (export main))",
        );
        assert!(
            out.has_error(),
            "an import of an unknown module must decline"
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("unknown package file")),
            "expected an 'unknown package file' diagnostic; got {:?}",
            out.diagnostics
        );
    }

    /// A mistyped import PATH that is a near-miss for a real package file carries a did-you-mean — the
    /// file-name analogue of a typoed import NAME's suggestion. `lib` is the sibling file; `"lipb"`
    /// (a transposition) must suggest it.
    #[test]
    fn importing_from_a_typoed_module_path_suggests_the_nearest_file() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lipb\" (helper)) (def (main) (helper)) (export main))",
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("unknown package file `lipb`")
                    && d.message.contains("did you mean `lib`?")),
            "expected a did-you-mean suggestion for the module path; got {:?}",
            out.diagnostics
        );
    }

    /// A mistyped `--entry NAME` that near-misses a supplied file carries a did-you-mean — the same
    /// closed-set suggestion a typoed import path gets, over the package's own file names.
    #[test]
    fn a_typoed_package_entry_suggests_the_nearest_file() {
        let out = compile_files(
            &[
                ("lib", "(do (def (helper) 1) (export helper))"),
                ("app", "(do (def (main) 2) (export main))"),
            ],
            "apps", // a typo of `app`
        );
        assert!(
            out.diagnostics.iter().any(|d| d
                .message
                .contains("package entry `apps` names no supplied `ast` file")
                && d.message.contains("did you mean `app`?")),
            "expected a did-you-mean for the mistyped entry; got {:?}",
            out.diagnostics
        );
    }

    /// The ALIAS form `(import "path" alias)` is a later phase — it declines for now (§2/§7).
    #[test]
    fn the_alias_import_form_declines() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lib\" lib) (def (main) 1) (export main))",
        );
        assert!(
            out.has_error(),
            "the alias import form must decline for now"
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("qualified import")),
            "expected a 'qualified import' decline; got {:?}",
            out.diagnostics
        );
    }

    /// β-COPY HYGIENE (`DESIGN-package-linking.md` §4 note): `app` imports `pub-helper` from `lib`;
    /// `pub-helper`'s body calls a PRIVATE sibling `priv-helper` (also in `lib`, NOT imported by
    /// `app`). When `pub-helper` inlines into `app`'s `main`, its copied body's reference to
    /// `priv-helper` must still resolve in `lib`'s scope — not become unbound, and not bind to any
    /// same-named def in `app`. Here `priv-helper` is defined only in `lib`, so it is unambiguous and
    /// resolves; the package compiles clean.
    #[test]
    fn an_inlined_import_reaches_its_own_files_private_sibling() {
        let out = compile_package(
            "(do (def (priv-helper) 40) \
                 (def (pub-helper) (+ (priv-helper) 1)) \
                 (export pub-helper))",
            "(do (import \"lib\" (pub-helper)) (def (main) (+ (pub-helper) 1)) (export main))",
        );
        assert!(
            !out.has_error(),
            "an inlined import reaching its own file's private sibling should compile clean; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// TYPE NAMES ARE FILE-SCOPED: the entry and an imported lib may EACH declare a same-named `type C`
    /// without a duplicate error — a package splices its files into one merged `(do …)`, but a sibling's
    /// `type` is file-local (`DESIGN-package-linking.md` §Imports Are Explicit), so the two declarations
    /// are NOT a duplicate. They are DISTINCT NOMINAL TYPES (type-system.md §Nominal — identity is the
    /// fully-qualified name); this guards that the two same-named decls COEXIST (no duplicate reject) with
    /// the entry using its OWN `C` locally. Uses a NON-recursive sum: a recursive same-named sum's
    /// self-referential payload type is still resolved through the flat type index at synthesis time
    /// (recursive-sum synthesis is not yet file-scoped — a separate follow-up), so a recursive re-declare
    /// splits its own spine's type. The composing path is to IMPORT the type (next test), which needs no
    /// re-declaration at all.
    #[test]
    fn a_same_type_name_in_a_lib_and_the_entry_coexist_but_are_distinct() {
        // Both files declare `C`; the entry does NOT import the lib's `C` and uses only its OWN `C`
        // locally (constructs + matches). No duplicate error, and the entry's `C` is its own.
        let out = compile_package(
            "(do (type C (A) (B)) (def (mk) C.A) (export mk))",
            "(do (import \"lib\" (mk)) (type C (A) (B)) \
                 (def (main) (match C.B ((C.A) 1) ((C.B) 2))) (export main))",
        );
        assert!(
            !out.has_error(),
            "two same-named `type C` decls (lib + entry) coexist without a duplicate error when each is used within its own file; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// The composing form for a recursive user sum across a module boundary: IMPORT the type both files
    /// share. `lib` exports its `L` + `mk`; the entry imports `(L mk)` and folds the imported value with
    /// a `sm` typed over the imported `L`. Because both refer to the SAME nominal declaration (the lib's),
    /// the value satisfies `sm`'s parameter and the fold composes → 11. This is the file-scoped-types
    /// replacement for the old "structural copy" form (which relied on the flat type index collapsing two
    /// same-named decls — a forging path §Nominal forbids). Mirrors the 11-modules corpus case.
    #[test]
    fn an_imported_recursive_sum_is_folded_over_the_imported_type() {
        let out = compile_package(
            "(do (type L (Nil) (Cons Int64 L)) \
                 (def (mk) (L.Cons 5 (L.Cons 6 (L.Nil)))) (export L mk))",
            "(do (import \"lib\" (L mk)) \
                 (def (sm (: l L)) (match l ((L.Nil) 0) ((L.Cons h t) (+ h (sm t))))) \
                 (def (main) (sm (mk))) (export main))",
        );
        assert!(
            !out.has_error(),
            "an imported recursive sum should fold over the imported type; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// Compile an N-file package: `files` is `(name, source)` pairs, `entry` names the entry file.
    fn compile_files(files: &[(&str, &str)], entry: &str) -> crate::abi::CompileOutput {
        let mut inputs: Vec<Artifact> = files
            .iter()
            .map(|(name, src)| {
                Artifact::new(
                    Artifact::KIND_AST,
                    *name,
                    crate::codec::encode(&arena_of(src)),
                )
            })
            .collect();
        inputs.push(Artifact::new(
            KIND_ENTRY,
            "entry",
            entry.as_bytes().to_vec(),
        ));
        crate::compile(&inputs, &[Target::Wasm])
    }

    /// Importing the same local name twice into one file → CDZ0201 (`modules-and-namespaces.md`
    /// §Colliding Imported Names Are Rejected), never resolved by an implicit precedence.
    #[test]
    fn a_colliding_import_is_rejected() {
        let out = compile_files(
            &[
                ("a", "(do (def (x) 1) (export x))"),
                ("b", "(do (def (x) 2) (export x))"),
                (
                    "app",
                    "(do (import \"a\" (x)) (import \"b\" (x)) (def (main) (x)) (export main))",
                ),
            ],
            "app",
        );
        assert!(out.has_error(), "a colliding import must be rejected");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0201")
                    && d.message.contains("more than once")),
            "expected a CDZ0201 colliding-import reject; got {:?}",
            out.diagnostics
        );
    }

    /// A two-file import CYCLE (`a` imports from `b`, `b` imports from `a`) is rejected
    /// (`modules-and-namespaces.md` §Cyclic Module Dependencies Are Rejected).
    #[test]
    fn a_two_file_import_cycle_is_rejected() {
        let out = compile_files(
            &[
                ("a", "(do (import \"b\" (g)) (def (f) (g)) (export f))"),
                ("b", "(do (import \"a\" (f)) (def (g) (f)) (export g))"),
            ],
            "a",
        );
        assert!(out.has_error(), "a two-file import cycle must be rejected");
        let cyc = out
            .diagnostics
            .iter()
            .find(|d| d.message.contains("cyclic module imports"))
            .unwrap_or_else(|| {
                panic!(
                    "expected a cyclic-import diagnostic; got {:?}",
                    out.diagnostics
                )
            });
        // It carries a NODE (the cycle's first import clause), so the error maps to `file:line:col`
        // rather than printing an unanchored `cdz:` prefix.
        assert!(
            cyc.node.is_some(),
            "the cyclic-import diagnostic must anchor to a node, not be unanchored"
        );
    }

    /// A three-file cycle (a→b→c→a) is rejected too — the DFS finds the back-edge regardless of length.
    #[test]
    fn a_three_file_import_cycle_is_rejected() {
        let out = compile_files(
            &[
                ("a", "(do (import \"b\" (gb)) (def (ga) (gb)) (export ga))"),
                ("b", "(do (import \"c\" (gc)) (def (gb) (gc)) (export gb))"),
                ("c", "(do (import \"a\" (ga)) (def (gc) (ga)) (export gc))"),
            ],
            "a",
        );
        assert!(
            out.has_error(),
            "a three-file import cycle must be rejected"
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("cyclic module imports")),
            "expected a cyclic-import diagnostic; got {:?}",
            out.diagnostics
        );
    }

    /// The `link-map` artifact (`DESIGN-package-linking.md` §6) rides a linked package's output and
    /// demuxes a cross-file diagnostic's GLOBAL node id → the right file + local id. Here `app` uses an
    /// unbound name; the emitted `link-map` lets the consumer attribute the error node to `app`.
    #[test]
    fn a_package_carries_a_link_map_that_demuxes_a_diagnostic() {
        let out = compile_files(
            &[
                ("lib", "(do (def (helper) 1) (export helper))"),
                // `app` references `nope` — unbound (not defined, not imported).
                ("app", "(do (def (main) (nope)) (export main))"),
            ],
            "app",
        );
        assert!(out.has_error(), "the package has an unbound name");
        // The `link-map` artifact is present even though compilation failed (it rides the fault path).
        let map = out
            .artifacts
            .iter()
            .find(|a| a.kind == KIND_LINK_MAP)
            .expect("a linked package must carry a link-map artifact");
        let text = String::from_utf8(map.bytes.clone()).unwrap();
        // One line per file: `<path>\t<base>\t<count>`. Parse into (path, base, count).
        let rows: Vec<(&str, u32, u32)> = text
            .lines()
            .map(|l| {
                let mut it = l.split('\t');
                let path = it.next().unwrap();
                let base: u32 = it.next().unwrap().parse().unwrap();
                let count: u32 = it.next().unwrap().parse().unwrap();
                (path, base, count)
            })
            .collect();
        assert_eq!(rows.len(), 2, "one link-map row per file");
        assert!(rows.iter().any(|(p, ..)| *p == "lib"));
        assert!(rows.iter().any(|(p, ..)| *p == "app"));
        // The unbound-name diagnostic's node demuxes into exactly ONE file's range.
        let node = out
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .and_then(|d| d.node)
            .expect("an unbound-name diagnostic anchored to a node");
        let owning: Vec<&str> = rows
            .iter()
            .filter(|(_, base, count)| node >= *base && node < base + count)
            .map(|(p, ..)| *p)
            .collect();
        assert_eq!(
            owning,
            vec!["app"],
            "the `nope` reference must demux to `app` (node {node})"
        );
    }

    /// `decode_link_map` is the inverse of `encode_link_map` — the CLI reporter uses it to demux a
    /// linked diagnostic's GLOBAL node id to `(file, local id)` so the error carries `file:line:col`
    /// instead of a bare `cdz:` prefix. Round-trips the emitted artifact and confirms the decoded spans
    /// attribute a real diagnostic node to exactly one file (the same `app` the manual parse above found).
    #[test]
    fn decode_link_map_round_trips_and_demuxes() {
        let out = compile_files(
            &[
                ("lib", "(do (def (helper) 1) (export helper))"),
                ("app", "(do (def (main) (nope)) (export main))"),
            ],
            "app",
        );
        let map = out
            .artifacts
            .iter()
            .find(|a| a.kind == KIND_LINK_MAP)
            .expect("a linked package carries a link-map");
        let files = decode_link_map(&map.bytes);
        assert_eq!(files.len(), 2, "decoded one FileSpan per file");
        // Byte-identical re-encode — the decode is a true inverse.
        assert_eq!(
            encode_link_map(&files),
            map.bytes,
            "encode∘decode is identity"
        );
        let node = out
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .and_then(|d| d.node)
            .expect("an unbound-name node");
        let owner: Vec<&str> = files
            .iter()
            .filter(|f| f.contains(StructId(node)))
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(
            owner,
            vec!["app"],
            "the decoded map demuxes node {node} to `app`"
        );
    }

    /// A single-file compile carries NO `link-map` (it is not a package — the demux is unneeded).
    #[test]
    fn a_single_file_compile_has_no_link_map() {
        let ast = crate::codec::encode(&arena_of("(do (def (main) (nope)) (export main))"));
        let out = crate::compile(
            &[Artifact::new(Artifact::KIND_AST, "main", ast)],
            &[Target::Wasm],
        );
        assert!(
            out.artifacts.iter().all(|a| a.kind != KIND_LINK_MAP),
            "a single-file compile must not emit a link-map"
        );
    }

    /// A DIAMOND (app→util, app→helper, helper→util) is ACYCLIC — it must link cleanly. `util` is
    /// imported by two files but there is no back-edge, so the cycle check must not false-positive.
    #[test]
    fn an_acyclic_diamond_links_cleanly() {
        let out = compile_files(
            &[
                ("util", "(do (def (base) 10) (export base))"),
                (
                    "helper",
                    "(do (import \"util\" (base)) (def (mid) (+ (base) 1)) (export mid))",
                ),
                (
                    "app",
                    "(do (import \"util\" (base)) (import \"helper\" (mid)) \
                         (def (main) (+ (base) (mid))) (export main))",
                ),
            ],
            "app",
        );
        assert!(
            !out.has_error(),
            "an acyclic diamond must link cleanly; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }
}
