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

// The `entry`/`component-name` INPUT-artifact kinds are compile-BOUNDARY vocabulary — moved to
// the shared `cadenza-compile-abi` crate (the front-end builds them, the compiler reads them).
// Re-exported so `crate::link::{KIND_ENTRY, KIND_COMPONENT_NAME}` + `compile`'s `find(kind == …)`
// stay byte-stable.
pub use cadenza_compile_abi::abi::{KIND_COMPONENT_NAME, KIND_ENTRY};

/// The input-artifact kind naming which PROVIDER export members cross the boundary as CANONICAL BYTES
/// (`list<u8>` via `value-encode`/`value-decode`) rather than as opaque runtime `u32` HANDLES (X5).
/// Its bytes are newline-separated export member names (e.g. `apply`). The TOOLCHAIN derives this from
/// the target WIT world the program declares it exports (the driver parses the WIT — the compiler needs
/// The input-artifact kind carrying the PREPARSED TARGET WIT WORLD in binary-AST form (§3b full-A
/// end-state, operator 2026-08-11). Its bytes are a `cadenza-ast` binary document — the world tree
/// (world → import/export interfaces → members → func signatures with `build_type` type descriptors)
/// produced by an external WIT→binary-AST step OR by v-syntax's inline-declaration lowering (both emit the
/// SAME locked world node). rcdzc NEVER parses WIT text; it consumes this artifact and reads each member's
/// declared canonical-ABI type to drive emit-to-match (`value-encode`/`value-decode`-bridging wherever the
/// guest value-model type differs from the declared type — `DESIGN-compiler-platform-separation.md` §3b).
/// Absent → no world-targeted emit; a provider export crosses as a shared-runtime handle (byte-identical to
/// before). This is the SOLE bytes-boundary signal — the compiler decides which members cross as `list<u8>`
/// purely from this declared world (no separate member-name list), so the fold's `apply` is just the first
/// member the world declares that way, not a hard-coded contract.
pub const KIND_WIT_WORLD: &str = "wit-world";

/// The input-artifact kind that OVERRIDES effect→peer bindings at COMPILE time (U3, the effects-unification
/// of cross-component interop). Its bytes are newline-separated `Effect=cadenza:pkg/iface` lines that WIN
/// over a program's in-source `(bind …)` defaults: `Effect=<iface>` rebinds an effect to a different peer;
/// `Effect=` (empty value) UNBINDS it (so the effect escapes to the host, or a test's in-program `(handle
/// Effect …)` handles it locally). The precedence — in-source default < compile-request override <
/// in-program handler — lets the same source be a real build (source/request binding) or a unit test (drop
/// the binding + handle it). Absent → the in-source bindings stand.
pub const KIND_EFFECT_BIND: &str = "effect-bind";

// The link-map RESULT wire (the diagnostics-demux `KIND_LINK_MAP` artifact + `FileSpan` table +
// its codec) now lives in the shared `cadenza-compile-abi` crate — a compile-boundary concern a
// consumer (`cdz check`) reads WITHOUT linking `rcdzc`. Re-exported so `crate::link::{…}` and the
// linker internals (which build the `FileSpan` table + call `encode_link_map`) stay byte-stable.
pub use cadenza_compile_abi::link_map::{
    FileSpan, KIND_LINK_MAP, decode_link_map, encode_link_map,
};

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
    /// WHOLE-MODULE ALIAS import (`(import "path" alias)`): `local` binds the module `from_file` (its
    /// exports) as a handle reached by qualified projection `(. local member)`, NOT a single flat name.
    /// `false` for the ordinary named-list / `__ast__` forms (which bind `local == exported` to one def).
    pub module_alias: bool,
    /// The `(import …)` clause's GLOBAL occurrence, for a diagnostic to anchor to.
    pub occ: StructId,
}

/// How much of a sum TYPE's constructor surface a file makes public. A type's HANDLE and its
/// CONSTRUCTORS are independently exportable (opaque/abstract types — `modules-and-namespaces.md`
/// §Visibility Is Explicit): exporting the handle alone yields an ABSTRACT type (its constructors,
/// match capability, strip, and structural `=` are not importable); a program builds and takes apart an
/// abstract type only through the module's exported functions ("smart constructors"). This records, for
/// a type whose handle is exported, WHICH of its constructors are also exported:
///  - `All` — the wildcard `(export (. T *))` / `T.*`: the handle + every constructor (CONCRETE).
///  - `Named(set)` — one or more `(export (. T A))`: the handle + exactly the named constructors.
///
/// A type present in `exports` but ABSENT from `type_ctor_exports` is ABSTRACT (handle only).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CtorVis {
    /// The wildcard `(. T *)` — every constructor of the type is exported (concrete).
    All,
    /// The specific constructor names exported via `(. T A)` clauses.
    Named(Vec<String>),
}

/// One file's link-time surface: the names it makes public (`(export …)`), the constructor visibility of
/// its exported types, and the names it pulls in (`(import …)`). Parallel to `LinkedProgram.files` (same
/// spliced index). A file's importable surface IS its export list (`modules-and-namespaces.md`
/// §Visibility Is Explicit — one mechanism, reused): a definition's cross-file visibility is the explicit
/// `(export …)` rule (not its source position), and a name a file does not export is not importable by
/// another (the sibling-import path rejects it).
//= spec/capabilities/modules-and-namespaces.md#visibility-is-explicit
//# Whether a definition is visible outside its module MUST be determined by an explicit rule fixed by this specification, not by its position in the source.
//= spec/capabilities/modules-and-namespaces.md#visibility-is-explicit
//# A definition that is not made visible MUST NOT be importable by another module.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FileScope {
    /// The public names this file exports (its `(export …)` clause names, including a type HANDLE named
    /// bare `T` or reached by a `(. T *)` / `(. T A)` constructor-export clause).
    pub exports: Vec<String>,
    /// Per exported sum TYPE, which of its constructors are also public. A type in `exports` but absent
    /// here exports ONLY its handle → it is ABSTRACT to importers. See [`CtorVis`].
    pub type_ctor_exports: std::collections::BTreeMap<String, CtorVis>,
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

/// The reserved import name that reflects the target module's canonical AST as a compile-time `Ast`
/// value (import reflection, DESIGN-compiler-primitives.md §3a/D1). Double-underscore namespaces it out
/// of user names (the `__`-prefixed synthesized-name convention: `__invariant_check_*`, `__bytes_of_rt$`).
pub(crate) const AST_REFLECT_NAME: &str = "__ast__";

/// The synthesized nullary value-def name that carries a reflected module's AST, one per reflected file.
/// A bare reference to a nullary def denotes its body (resolve.rs), so the local `__ast__` an import binds
/// to this def resolves to the reflected `Ast` value. The `$` keeps it out of the source name space.
fn ast_reflect_def_name(from_file: usize) -> String {
    format!("{AST_REFLECT_NAME}${from_file}")
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
    // Per-file constructor-visibility of each exported TYPE — parallel to `exports_of`. A type in
    // `exports_of` but absent here exports only its handle (ABSTRACT). Drives whether importing the type
    // brings its constructors (`build_file_scope`).
    let mut type_ctor_exports_of: Vec<std::collections::BTreeMap<String, CtorVis>> =
        Vec::with_capacity(files.len());
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
            // A `(module-doc …)` is a file/module-level doc-comment (a `///` header) — inert, declares
            // nothing. Skip it so it never lands in the merged `(do …)` as an expression to compile
            // (it would fault as an unbound `module-doc` call). Mirrors the `import` skip above.
            if ast.as_form(item, "module-doc").is_some() {
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
        let mut type_ctor_exports: std::collections::BTreeMap<String, CtorVis> =
            std::collections::BTreeMap::new();
        let mut defined = Vec::new();
        for item in top_items(ast) {
            if let Some(tail) = ast.as_form(item, "export") {
                // Gather EVERY element in the clause — `(export a b)` publishes both, matching the main
                // scan (`scan_top_level`). Reading only `tail.first()` once silently dropped every name
                // past the first. Three element FORMS (opaque/abstract types — the handle and the
                // constructors are independently exportable):
                //  - a BARE NAME `T` / `f` → publish the name (a value def OR a type HANDLE; a type
                //    handle exported bare is ABSTRACT unless a ctor-export clause below also names it).
                //  - `(. T *)` → the WILDCARD: publish the handle `T` + mark its ctors `All` (concrete).
                //  - `(. T A)` → publish the handle `T` + mark ctor `A` exported (`Named`).
                // A malformed element is left for the well-formedness pass (`malformed_exports`).
                //= spec/capabilities/modules-and-namespaces.md#a-type-s-handle-and-its-constructors-are-independently-visible
                //# A sum type's handle — the name that denotes the type itself — and its constructors MUST be independently exportable, so that a module can publish a type for other modules to name and hold values of without publishing the way to construct or take those values apart.
                //= spec/capabilities/modules-and-namespaces.md#a-type-s-handle-and-its-constructors-are-independently-visible
                //# A module MUST be able to make every constructor of a type visible in one act that also makes the type's handle visible, so that publishing a type together with its whole constructor set does not require enumerating the constructors one by one and does not drift as the constructor set changes.
                for &s in tail.iter() {
                    if let Some(name) = ast.as_name(s) {
                        exports.push(name.to_string());
                    } else if let Some((ty, ctor)) = as_ctor_export(ast, s) {
                        // The handle is public (an importer must be able to NAME the type it can
                        // construct). Idempotent — a repeated `(. T A)` re-adds the same handle name.
                        exports.push(ty.to_string());
                        match ctor {
                            // `(. T *)` — every constructor. `All` subsumes any `Named`.
                            None => {
                                type_ctor_exports.insert(ty.to_string(), CtorVis::All);
                            }
                            // `(. T A)` — accumulate the named ctor unless the type is already `All`.
                            Some(c) => match type_ctor_exports
                                .entry(ty.to_string())
                                .or_insert_with(|| CtorVis::Named(Vec::new()))
                            {
                                CtorVis::All => {}
                                CtorVis::Named(names) => {
                                    if !names.iter().any(|n| n == c) {
                                        names.push(c.to_string());
                                    }
                                }
                            },
                        }
                    }
                }
            } else if let Some(name) = top_item_defined_name(ast, item) {
                defined.push(name);
            }
        }
        exports_of.push(exports);
        type_ctor_exports_of.push(type_ctor_exports);
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
            type_ctor_exports: type_ctor_exports_of[fi].clone(),
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

    // The merged arena so far (root filled in below). Assemble it now so import reflection can reify
    // sibling module ASTs directly into it — the merge above copied every file's FULL structure, so each
    // reflected module's own `(do …)` root already lives here at its `FileSpan.struct_base` offset.
    let mut merged = Arenas {
        leaves,
        structure,
        root: StructId(0),
    };

    // IMPORT REFLECTION (DESIGN-compiler-primitives.md §3a): for each sibling module that some file
    // reflects via `import { __ast__ }`, splice a synthesized nullary value-def `(def (__ast__$<from_file>)
    // <reflected module AST>)` into the merged program. The def's body is the module's `(do …)` root
    // reflected structurally as an `Ast` value; the local `__ast__` an import bound (above) resolves to it
    // through the ordinary import→def binding. Dedup by `from_file` so two importers share one synth def.
    let mut reflect_files: Vec<usize> = scopes
        .iter()
        .flat_map(|s| s.imports.iter())
        .filter(|i| i.local == AST_REFLECT_NAME)
        .map(|i| i.from_file)
        .collect();
    reflect_files.sort_unstable();
    reflect_files.dedup();
    for from_file in reflect_files {
        // The reflected module's own `(do …)` root, at its position in the merged arena.
        let sib_root = StructId(file_spans[from_file].struct_base + files[from_file].1.root.0);
        let Some(reified) = crate::quote::reflect_document(&mut merged, sib_root) else {
            // `reflect_document` bails ONLY on a leaf with no `Ast` variant — a reader error-recovery
            // marker (`BadChar`/`BadEscape`, produced only from MALFORMED source) or a stray unquote
            // escape (`,x` / `,@x` outside a quasiquote). Every ordinary syntax leaf — including `Char`
            // (`#\a`) and `Symbol` (`#"x"`) — DOES reflect, so reflection is TOTAL over a well-formed
            // module (operator directive; see the "reflection is total" corpus case). Hence when this
            // fires the reflected module is NOT well-formed: it is a genuine REJECTION (CDZ0201
            // malformed, seq-32 reclassify), not a "cannot-yet" capability decline (there is nothing
            // left to build). Bails, never miscompiles.
            return Err(Reject::coded(
                crate::diag::Code::Malformed,
                format!(
                    "`import {{ __ast__ }}` from `{}`: the module contains a syntax node with no \
                     `Ast` representation — a reader error-recovery marker or a stray unquote escape \
                     (`,x`) outside a quasiquote; only a well-formed module reflects",
                    file_spans[from_file].path
                ),
            ));
        };
        // Splice `(def (__ast__$N) <reified>)` — a nullary value-def; a bare reference denotes its body.
        let name_leaf = LeafId(merged.leaves.len() as u32);
        merged
            .leaves
            .push(Leaf::Name(ast_reflect_def_name(from_file).into()));
        let name_atom = StructId(merged.structure.len() as u32);
        merged.structure.push(Struct::Atom(name_leaf));
        let sig = StructId(merged.structure.len() as u32);
        merged.structure.push(Struct::List(vec![name_atom]));
        let def_leaf = LeafId(merged.leaves.len() as u32);
        merged.leaves.push(Leaf::Name("def".into()));
        let def_atom = StructId(merged.structure.len() as u32);
        merged.structure.push(Struct::Atom(def_leaf));
        let def_form = StructId(merged.structure.len() as u32);
        merged
            .structure
            .push(Struct::List(vec![def_atom, sig, reified]));
        do_children.push(def_form);
    }

    // Synthesize the `(do …)` root: a fresh `do` name leaf + its atom, then the list whose head is
    // that atom and whose tail is every file's top-level items (plus any reflection synth-defs). These
    // nodes sit AFTER all files, so they are outside every `FileSpan` (they belong to no source file).
    let do_leaf = LeafId(merged.leaves.len() as u32);
    merged.leaves.push(Leaf::Name("do".into()));
    let do_atom = StructId(merged.structure.len() as u32);
    merged.structure.push(Struct::Atom(do_leaf));
    let mut root_children = Vec::with_capacity(do_children.len() + 1);
    root_children.push(do_atom);
    root_children.extend(do_children);
    let root = StructId(merged.structure.len() as u32);
    merged.structure.push(Struct::List(root_children));
    merged.root = root;

    Ok(LinkedProgram {
        arenas: merged,
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
    // A BARE-NAME spec is the WHOLE-MODULE ALIAS form `(import "path" alias)`: bind the local `alias` to
    // the module `path` (its exports), reached by qualified projection `(. alias member)` — the resolution
    // of a uniformly-named export (`descriptor`) imported from 2+ modules that would COLLIDE under the
    // flat named-list form (v-platform-itest's multi-contract dispatch). Distinguished POSITIONALLY from
    // the named-list `(import "path" (name…))` (a LIST spec) — the two never collide. Record ONE
    // module-alias `Import` (local = the alias, `module_alias` true); `build_file_scope` registers it as a
    // module handle and `resolve_member` projects members against `from_file`'s exports (defs).
    let names: &[StructId] = match ast.get(spec_id) {
        Struct::List(items) => items,
        Struct::Atom(_) => {
            let Some(alias) = ast.as_name(spec_id) else {
                return Err(Reject::coded(
                    Code::Malformed,
                    "`(import \"path\" alias)`: the alias must be a bare name",
                )
                .at(occ));
            };
            let Some(&from_file) = name_to_ix.get(path) else {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!("`(import …)` names unknown package file `{path}`"),
                )
                .at(occ));
            };
            if out.iter().any(|i| i.local == alias) {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!("`(import …)`: `{alias}` is imported more than once into this file"),
                )
                .at(occ));
            }
            out.push(Import {
                local: alias.to_string(),
                from_file,
                exported: alias.to_string(),
                module_alias: true,
                occ,
            });
            return Ok(());
        }
    };
    let Some(&from_file) = name_to_ix.get(path) else {
        // A did-you-mean over the OTHER package files — a mistyped path (`"libb"` for `lib`) is the
        // file-name analogue of the typoed import NAME handled below, so it gets the same treatment
        // (the shared `nearest` guards the 1-char/empty cases). `name_to_ix`'s keys ARE the package's
        // file names, so no extra plumbing is needed.
        // A near-miss also carries the STRUCTURAL fix (rewrite the mistyped path to the near file), so
        // the "did you mean?" is APPLYABLE, not just prose — the import-NAME-typo treatment extended to
        // the path. Anchored at the PATH node `path_id`. WARNING: `path_id` is a STRING LITERAL, so the
        // replacement must be QUOTED (`"lib"`): the consumer re-parses it as a sub-form and a bare `lib`
        // would read as a Name → the malformed `(import lib …)`. Heuristic: the near name is a guess.
        let mut fix = None;
        let msg = match crate::diag::suggest::nearest(path, name_to_ix.keys().copied()) {
            Some(near) => {
                let m = format!(
                    "`(import …)` names unknown package file `{path}` — did you mean `{near}`?"
                );
                fix = Some(crate::diag::Fix::replace_heuristic(
                    path_id,
                    format!("\"{near}\""),
                ));
                m
            }
            None => format!("`(import …)` names unknown package file `{path}`"),
        };
        let mut reject = Reject::coded(Code::Malformed, msg).at(occ);
        if let Some(fix) = fix {
            reject = reject.with_fix(fix);
        }
        return Err(reject);
    };

    for &name_id in names {
        // An element is either a BARE Name (plain import: local == exported) or a per-name RENAME
        // `(as orig alias)` — an `as`-headed list binding the module export `orig` under the local name
        // `alias` (local = alias, exported = orig). The rename lets one file import two modules'
        // uniformly-named exports (`descriptor`) under distinct local names WITHOUT the whole-module
        // alias. Export-visibility + the did-you-mean fix key off `exported` (anchored at `exported_id`);
        // the collision check + the introduced binding key off `local`.
        let (exported, local, exported_id): (&str, &str, StructId) = if let Some(args) =
            ast.as_form(name_id, "as")
        {
            let [orig_id, alias_id] = args else {
                return Err(Reject::coded(
                        Code::Malformed,
                        "`(import …)` rename must be `(as orig alias)`: an exported name and a local alias",
                    )
                    .at(occ));
            };
            let (Some(orig), Some(alias)) = (ast.as_name(*orig_id), ast.as_name(*alias_id)) else {
                return Err(Reject::coded(
                    Code::Malformed,
                    "`(import …)` rename `(as orig alias)`: `orig` and `alias` must be bare names",
                )
                .at(occ));
            };
            (orig, alias, *orig_id)
        } else if let Some(n) = ast.as_name(name_id) {
            (n, n, name_id)
        } else {
            return Err(Reject::coded(
                Code::Malformed,
                "`(import …)` name list may contain only bare names or `(as orig alias)` renames",
            )
            .at(occ));
        };
        // `__ast__` — the reserved IMPORT-REFLECTION name every module implicitly exports (DESIGN-compiler-
        // primitives.md §3a/D1). It is NOT a real export, so bypass the export-visibility check. It binds to
        // the target module's canonical AST reified as an `Ast` value — a synthesized nullary value-def
        // (`__ast__$<from_file>`, spliced into the merged program by `link()` below) whose body is that
        // reflected AST. Point this import's `exported` at that synth def so the ordinary import→def binding
        // (`db::build_file_scope`) wires the local `__ast__` to it, with no db-side special-casing.
        if exported == AST_REFLECT_NAME {
            if out.iter().any(|i| i.local == local) {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!("`(import …)`: `{local}` is imported more than once into this file"),
                )
                .at(occ));
            }
            out.push(Import {
                local: local.to_string(),
                from_file,
                exported: ast_reflect_def_name(from_file),
                module_alias: false,
                occ,
            });
            continue;
        }
        if !exports_of[from_file].iter().any(|e| e == exported) {
            // Distinguish the two reasons the name is not importable, so the message is ACTIONABLE:
            //  - `{path}` DEFINES `{name}` but does not `(export …)` it → say so + name the fix (add an
            //    export to that file), the "private item" case (rustc's "consider making it public").
            //  - `{path}` does not define it at all → the plain "does not export", enriched with a "did
            //    you mean?" over what the file DOES export (a typoed import name).
            // A near-miss export name also carries the STRUCTURAL fix (rewrite the mistyped import name
            // to the near export), so the "did you mean?" is applyable, not just prose — anchored at the
            // NAME token `name_id`, not the enclosing clause `occ`. Heuristic: the near name is a guess at
            // intent. The private-item case has no single-node rewrite (the fix is to edit the OTHER file's
            // export list), so it stays message-only.
            let mut fix = None;
            let msg = if defined_of[from_file].iter().any(|d| d == exported) {
                format!(
                    "`(import …)`: `{path}` defines `{exported}` but does not export it — add `export \
                     {{ {exported} }}` to `{path}`"
                )
            } else {
                match crate::diag::suggest::nearest(exported, &exports_of[from_file]) {
                    Some(near) => {
                        let m = format!(
                            "`(import …)`: `{path}` does not export `{exported}` — did you mean `{near}`?"
                        );
                        fix = Some(crate::diag::Fix::replace_heuristic(exported_id, near));
                        m
                    }
                    None => format!("`(import …)`: `{path}` does not export `{exported}`"),
                }
            };
            let mut reject = Reject::coded(Code::Malformed, msg).at(occ);
            if let Some(fix) = fix {
                reject = reject.with_fix(fix);
            }
            return Err(reject);
        }
        // COLLIDING IMPORTED NAMES: two imports binding the SAME local name into one file's scope is a
        // compile-time error (CDZ0201), never resolved by an implicit precedence. This is a
        // positively-proven ill-formed program — a CODED reject, not a decline.
        //= spec/capabilities/modules-and-namespaces.md#colliding-imported-names-are-rejected
        //# Importing two definitions under the same name into one scope MUST be a compile-time error rather than resolved by an implicit precedence.
        if out.iter().any(|i| i.local == local) {
            return Err(Reject::coded(
                crate::diag::Code::Malformed,
                format!("`(import …)`: `{local}` is imported more than once into this file"),
            )
            .at(occ));
        }
        out.push(Import {
            local: local.to_string(),
            from_file,
            exported: exported.to_string(),
            module_alias: false,
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

/// Parse an `(export …)` element that is a member-access ctor-export form `(. T A)` or the wildcard
/// `(. T *)`. Returns `Some((type_name, Some(ctor)))` for a specific constructor, `Some((type_name,
/// None))` for the wildcard `*`, or `None` if `s` is not a `(. name name)` member access (a bare name,
/// an integer projection `(. t 0)`, or anything else — handled elsewhere). This is the surface for
/// exporting a type's constructors alongside (or instead of implying) its handle: `*` is a RESERVED
/// final member segment meaning "every constructor", not a name glob — it is recognized only in this
/// export position, so it never collides with the multiply operator.
fn as_ctor_export(ast: &Arenas, s: StructId) -> Option<(&str, Option<&str>)> {
    let tail = ast.as_form(s, ".")?;
    if tail.len() != 2 {
        return None;
    }
    let ty = ast.as_name(tail[0])?;
    let key = ast.as_name(tail[1])?;
    if key == "*" {
        Some((ty, None))
    } else {
        Some((ty, Some(key)))
    }
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
    // A `(type …)` decl's name is decoded via the shared helper: a bare atom `(type Box …)` OR a
    // parenthesized generic head `(type (Box a) …)` (the head atom is the name). Without the helper, reading
    // a bare name off the first tail element missed the parenthesized case — a generic head `(type (Name a)
    // …)` has a LIST as its first tail element, not an atom, so a name-only read yields nothing. That gave a
    // parenthesized-head generic type no defined-name here, making it invisible to the export/import name
    // resolution that reads this fn — treated un-exported/absent.
    if let Some(tail) = ast.as_form(item, "type") {
        return tail
            .first()
            .and_then(|&s| ast.type_decl_head_name(s))
            .map(str::to_string);
    }
    // An `(effect E …)` name is always a bare atom (no parenthesized generic-effect form), so read it plain.
    if let Some(tail) = ast.as_form(item, "effect") {
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
            cadenza_compile_abi::abi::entry_artifact("app"),
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

    /// Import reflection (DESIGN-compiler-primitives.md §3a): `import { __ast__ }` binds the reserved
    /// name to the target module's canonical AST reified as an `Ast` value — byte-identical to a `quote`
    /// of the module body. Emits a component (the reflected value is a compile-time constant). The
    /// corpus (`11-modules.sexp`) drives the value assertion end-to-end.
    #[test]
    fn import_ast_reflection_binds_the_module_ast() {
        use crate::abi::Artifact;
        use crate::backend::Target;
        let lib = crate::codec::encode(&arena_of("(do (def (answer) 42) (export answer))"));
        let app = crate::codec::encode(&arena_of(
            "(do (import \"lib\" (__ast__)) \
             (def (main) (= (Ast.encode __ast__) \
                            (Ast.encode (quote (do (def (answer) 42) (export answer)))))) \
             (export main))",
        ));
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "lib", lib),
            Artifact::new(Artifact::KIND_AST, "app", app),
            cadenza_compile_abi::abi::entry_artifact("app"),
        ];
        let out = crate::compile(&inputs, &[Target::Wasm]);
        assert!(
            !out.has_error(),
            "import reflection should compile clean; diagnostics: {:?}",
            out.diagnostics
        );
        assert!(
            out.artifact(Target::Wasm.artifact_kind()).is_some(),
            "a component should be produced for the reflecting package"
        );
    }

    /// `Ast.module` (the self-reflection intrinsic) reflects the ENCLOSING module's AST — a module can
    /// hash its own canonical AST and export the digest as a compile-time constant with no self-import.
    /// Filled at lowering from the per-file source snapshot (`Prim::ReflectModule`).
    #[test]
    fn ast_module_reflects_the_enclosing_module() {
        use crate::abi::Artifact;
        use crate::backend::Target;
        let m = crate::codec::encode(&arena_of(
            "(do (def (cid) (Bytes.len (Blake3.of (Ast.encode Ast.module)))) (export cid))",
        ));
        let out = crate::compile(
            &[Artifact::new(Artifact::KIND_AST, "m", m)],
            &[Target::Wasm],
        );
        assert!(
            !out.has_error(),
            "Ast.module should reflect the enclosing module and compile; diagnostics: {:?}",
            out.diagnostics
        );
        assert!(
            out.artifact(Target::Wasm.artifact_kind()).is_some(),
            "an Ast.module-using module should emit a component"
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
            cadenza_compile_abi::abi::entry_artifact("app"),
        ];
        crate::compile(&inputs, &[Target::Wasm])
    }

    /// Compile a package whose ENTRY (`app`) file is authored in the ML SURFACE (`.cdz`), driven through
    /// the real front-end `cadenza_syntax::parser::read_ml` → cadenza-syntax codec → rcdzc decode — the
    /// same seam the CLI uses. Each `libs` entry is `(artifact-name, s-expr source)` (a lib's surface is
    /// not under test, so it stays s-expr); the entry imports them by name. This exercises an ML-surface
    /// linking feature (like the alias import) END-TO-END rather than hand-feeding the arena.
    fn compile_package_ml_app(libs: &[(&str, &str)], app_ml: &str) -> crate::abi::CompileOutput {
        let parsed = cadenza_syntax::parser::read_ml(app_ml);
        assert!(
            parsed.ok(),
            "ML app failed to parse: {:?}\n  src: {app_ml}",
            parsed.errors
        );
        let app_bytes = cadenza_syntax::codec::encode(&parsed.arenas);
        let app_arena = crate::codec::decode(&app_bytes)
            .unwrap_or_else(|| panic!("cadenza-syntax bytes failed rcdzc decode: {app_ml}"));
        let mut inputs: Vec<Artifact> = libs
            .iter()
            .map(|&(name, src)| {
                Artifact::new(
                    Artifact::KIND_AST,
                    name,
                    crate::codec::encode(&arena_of(src)),
                )
            })
            .collect();
        inputs.push(Artifact::new(
            Artifact::KIND_AST,
            "app",
            crate::codec::encode(&app_arena),
        ));
        inputs.push(cadenza_compile_abi::abi::entry_artifact("app"));
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
        let d = out
            .diagnostics
            .iter()
            .find(|d| {
                d.message.contains("does not export `helpr`")
                    && d.message.contains("did you mean `helper`?")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected a did-you-mean suggestion; got {:?}",
                    out.diagnostics
                )
            });
        // The "did you mean?" is now APPLYABLE: it carries a heuristic Replace fix rewriting the mistyped
        // import name `helpr` to the near export `helper` — not just prose (the import analogue of the
        // unbound-name / export-typo replace fixes).
        let fix = d
            .fix
            .as_ref()
            .expect("the typoed-import did-you-mean carries a replace fix");
        assert_eq!(fix.kind, crate::abi::FixKind::Replace);
        assert_eq!(
            fix.replacement, "helper",
            "rewrites the typo to the near export"
        );
        assert!(!fix.verified, "a nearest-name guess is heuristic");
        // ROUND-TRIP: applying the fix (`helpr` → `helper` at both the import clause AND the use site)
        // yields a package that links clean — the suggestion is a real repair, not just a plausible name.
        // Witnessed by compiling the corrected source (mirrors the applied edit; the import-path and
        // unknown-unit did-you-means carry the same round-trip pin).
        let repaired = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lib\" (helper)) (def (main) (helper)) (export main))",
        );
        assert!(
            !repaired.has_error(),
            "applying the import-name fix (`helpr` → `helper`) links clean; got {:?}",
            repaired.diagnostics
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
        let d = out
            .diagnostics
            .iter()
            .find(|d| {
                d.message.contains("unknown package file `lipb`")
                    && d.message.contains("did you mean `lib`?")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected a did-you-mean suggestion for the module path; got {:?}",
                    out.diagnostics
                )
            });
        // The path did-you-mean is now APPLYABLE: it carries a heuristic Replace fix rewriting the
        // mistyped path to the near file — the import-NAME-typo treatment extended to the path. Because
        // the path is a STRING LITERAL, the replacement is QUOTED (`"lib"`), so applying it re-renders a
        // well-formed `(import "lib" …)`, not the malformed `(import lib …)` a bare name would give.
        let fix = d
            .fix
            .as_ref()
            .expect("the typoed-path did-you-mean carries a replace fix");
        assert_eq!(fix.kind, crate::abi::FixKind::Replace);
        assert_eq!(
            fix.replacement, "\"lib\"",
            "rewrites the typo to the near file, QUOTED so it stays a string literal"
        );
        assert!(!fix.verified, "a nearest-name guess is heuristic");
        // ROUND-TRIP: applying the fix (`"lipb"` → the quoted near file `"lib"`) yields a package that
        // links clean — the suggestion is a real repair, not just a plausible-looking string. Witnessed
        // by compiling the corrected source (mirrors the applied edit: the string literal now names `lib`).
        let repaired = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lib\" (helper)) (def (main) (helper)) (export main))",
        );
        assert!(
            !repaired.has_error(),
            "applying the path fix (\"lipb\" → \"lib\") links clean; got {:?}",
            repaired.diagnostics
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

    /// The WHOLE-MODULE ALIAS form `(import "path" alias)` binds the module under `alias`, reached by
    /// qualified projection `(. alias member)` — the collision-free path for a uniformly-named export
    /// imported from 2+ modules (the descriptor-collision realization, #3600). `lib` exports `helper`
    /// (→ 40); the entry aliases `lib` and projects `(. lib helper)` = 40, compiling clean. (Formerly the
    /// negative `the_alias_import_form_declines` — the "realize a feature, update its negative test" flip.)
    #[test]
    fn the_alias_import_form_resolves_and_projects() {
        let out = compile_package(
            "(do (def (helper) 40) (export helper))",
            "(do (import \"lib\" lib) (def (main) (. lib helper)) (export main))",
        );
        assert!(
            !out.has_error(),
            "the alias import form must resolve + project a member; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// PER-NAME import RENAME `(as orig alias)` (v-syntax's surface `import { descriptor as foo } from
    /// "path"`): bind the module's export `orig` under the LOCAL name `alias`. The linker discriminant is
    /// per-element — a bare Name is a plain import (local == exported); an `as`-headed 3-list is a rename
    /// (local = alias, exported = orig). Export-visibility keys off `orig`, the introduced binding + the
    /// collision check key off `alias`. Here `lib` exports `descriptor` (→ 30); the entry imports it as
    /// `foo` and calls `foo` → 30, resolving via the aliased local with no whole-module handle. Dormant-
    /// safe ahead of the ML surface (tested at the arena level the parser will emit).
    #[test]
    fn a_per_name_import_rename_binds_the_export_under_the_local_alias() {
        let out = compile_package(
            "(do (def (descriptor) 30) (export descriptor))",
            "(do (import \"lib\" ((as descriptor foo))) (def (main) (foo)) (export main))",
        );
        assert!(
            !out.has_error(),
            "a per-name rename must bind the export under the local alias; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// A per-name rename's COLLISION is checked on the LOCAL alias, not the source export: importing two
    /// different exports under the SAME local name is a CDZ0201 (colliding imported names), exactly as two
    /// bare imports of the same name would be. `lib` exports `descriptor` + `other`; the entry renames
    /// BOTH to `foo` → the second binding of `foo` collides.
    #[test]
    fn a_per_name_rename_colliding_on_the_local_alias_is_rejected() {
        let out = compile_package(
            "(do (def (descriptor) 30) (def (other) 12) (export descriptor) (export other))",
            "(do (import \"lib\" ((as descriptor foo) (as other foo))) (def (main) (foo)) (export main))",
        );
        assert!(
            out.has_error(),
            "two renames to the same local alias must collide (CDZ0201)"
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("`foo` is imported more than once")),
            "expected a colliding-local-alias error for `foo`; got {:?}",
            out.diagnostics
        );
    }

    /// END-TO-END through the ML SURFACE: the alias import `import alias from "path"` (v-syntax #3686,
    /// respelled from `as` to `from` in #3692) links + projects through the #3656 module-alias machinery
    /// when authored in a `.cdz`. `read_ml` parses `import lib from "lib"` to the `(import "lib" lib)`
    /// arena (bare-NAME third element = the
    /// linker's alias discriminant), so `lib.helper` → `(. lib helper)` resolves to the aliased module's
    /// export (→ 40) and the package compiles clean. This is the seam neither v-syntax's parser tests
    /// (parse SHAPE only) nor `the_alias_import_form_resolves_and_projects` (arena-direct s-expr) cover —
    /// the full ML front-end → codec → rcdzc decode → link path for the alias surface.
    #[test]
    fn the_ml_surface_alias_import_resolves_and_projects() {
        let out = compile_package_ml_app(
            &[("lib", "(do (def (helper) 40) (export helper))")],
            "import lib from \"lib\"\ndef main() = lib.helper\nexport { main }",
        );
        assert!(
            !out.has_error(),
            "the ML-surface alias import must resolve + project a member; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// The COLLISION-AVOIDANCE core of the alias import, through the ML SURFACE — the exact §8 shape the
    /// conformance dispatcher authors: TWO modules that each export the SAME name (`descriptor`), imported
    /// under distinct aliases (`a`/`b`), each projected to ITS OWN module's export. A plain named-list
    /// `import { descriptor } from …` twice would COLLIDE (CDZ0201); the alias is the collision-free path
    /// (#3656). This pins that the two same-named exports RESOLVE DISTINCTLY (no collision error) when
    /// authored in a `.cdz` — the composition (`the_ml_surface_alias_import_resolves_and_projects` covers a
    /// single alias; `11-modules.sexp` covers collision in s-expr; neither covers ML-surface collision).
    #[test]
    fn ml_surface_aliases_disambiguate_a_uniformly_named_export_from_two_modules() {
        let out = compile_package_ml_app(
            &[
                ("liba", "(do (def (descriptor) 30) (export descriptor))"),
                ("libb", "(do (def (descriptor) 12) (export descriptor))"),
            ],
            "import a from \"liba\"\n\
             import b from \"libb\"\n\
             def from-a() = a.descriptor\n\
             def from-b() = b.descriptor\n\
             export { from-a, from-b }",
        );
        assert!(
            !out.has_error(),
            "two aliased modules exporting the same name must resolve distinctly (no CDZ0201); got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// END-TO-END through the ML SURFACE: the PER-NAME rename `import { orig as alias } from "path"`
    /// (v-syntax #3716) binds the module export `orig` under the local `alias` through the #3719
    /// resolve/link rebind. `read_ml` parses it to `(import "lib" ((as descriptor foo)))`; the entry uses
    /// the bare local `foo` (a flat import, no module handle) and it resolves to `lib`'s `descriptor` (→
    /// 30). Complements the arena-level pins (`a_per_name_import_rename_*`) with the full ML front-end →
    /// codec → rcdzc decode → link path the parser landed.
    #[test]
    fn the_ml_surface_per_name_rename_binds_the_export_under_the_alias() {
        let out = compile_package_ml_app(
            &[("lib", "(do (def (descriptor) 30) (export descriptor))")],
            "import { descriptor as foo } from \"lib\"\ndef main() = foo\nexport { main }",
        );
        assert!(
            !out.has_error(),
            "the ML-surface per-name rename must bind the export under the alias; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// The §8 dispatcher's ACTUAL shape through the ML SURFACE: two modules each export `descriptor`, and
    /// the entry imports each under a DISTINCT local name via per-name rename — the flat-import alternative
    /// to the whole-module alias for the descriptor-collision case. `import { descriptor as a-desc } from
    /// "liba"` + `import { descriptor as b-desc } from "libb"` bind two distinct locals, both resolving to
    /// their own module's `descriptor` (no CDZ0201). This is the form v-platform-itest will switch its
    /// §8/§7 descriptor imports to.
    #[test]
    fn ml_surface_per_name_renames_disambiguate_same_name_from_two_modules() {
        let out = compile_package_ml_app(
            &[
                ("liba", "(do (def (descriptor) 30) (export descriptor))"),
                ("libb", "(do (def (descriptor) 12) (export descriptor))"),
            ],
            "import { descriptor as a-desc } from \"liba\"\n\
             import { descriptor as b-desc } from \"libb\"\n\
             def from-a() = a-desc\n\
             def from-b() = b-desc\n\
             export { from-a, from-b }",
        );
        assert!(
            !out.has_error(),
            "per-name renames of the same export name from two modules must resolve distinctly (no CDZ0201); got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// A MIXED name-list — v-syntax's canonical example `import { descriptor as foo, other } from "path"`
    /// — where ONE import binds a RENAMED element (`descriptor as foo`) AND a PLAIN element (`helper`) in
    /// the same list. The per-element loop must bind each by its own kind (rename → local=foo/exported=
    /// descriptor; plain → local==exported==helper) without one element's handling disturbing the next.
    /// This is the realistic authoring form (rename the colliding name, import the rest plainly); the
    /// earlier pins used all-rename or all-plain lists, so the heterogeneous case was uncovered.
    #[test]
    fn ml_surface_a_mixed_rename_and_plain_import_list_binds_both() {
        let out = compile_package_ml_app(
            &[(
                "lib",
                "(do (def (descriptor) 30) (def (helper) 12) (export descriptor) (export helper))",
            )],
            "import { descriptor as foo, helper } from \"lib\"\n\
             def from-renamed() = foo\n\
             def from-plain() = helper\n\
             export { from-renamed, from-plain }",
        );
        assert!(
            !out.has_error(),
            "a mixed rename+plain import list must bind both the renamed and the plain name; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
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
    /// share, exported CONCRETELY (`L.*` — the handle + all constructors). `lib` exports `(. L *)` + `mk`;
    /// the entry imports `(L mk)` and folds the imported value with a `sm` typed over the imported `L`,
    /// MATCHING on `L.Nil`/`L.Cons` (which the wildcard export makes visible). Because both refer to the
    /// SAME nominal declaration (the lib's), the value satisfies `sm`'s parameter and the fold composes →
    /// 11. This is the file-scoped-types replacement for the old "structural copy" form (which relied on
    /// the flat type index collapsing two same-named decls — a forging path §Nominal forbids). A bare
    /// `(export L)` would export the HANDLE ONLY (abstract), and the entry's `L.Nil`/`L.Cons` match would
    /// be CDZ0214 — so a type whose constructors an importer must use is exported `L.*`.
    #[test]
    fn an_imported_recursive_sum_is_folded_over_the_imported_type() {
        let out = compile_package(
            "(do (type L (Nil) (Cons Int64 L)) \
                 (def (mk) (L.Cons 5 (L.Cons 6 (L.Nil)))) (export (. L *) mk))",
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

    /// OPAQUE TYPE: `lib` exports the HANDLE `Color` bare (abstract) + a smart constructor `mk`, NOT its
    /// variant constructors. The entry may name `Color` and call `mk`, but CONSTRUCTING `(Color.Green)`
    /// reaches a withheld constructor → CDZ0214 (the constructor is hidden on purpose, distinct from an
    /// unbound name — the type IS in scope). The abstract-data-type / smart-constructor guarantee.
    #[test]
    fn an_abstract_types_constructor_is_not_reachable_outside_its_module() {
        let out = compile_package(
            "(do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color mk))",
            "(do (import \"lib\" (Color mk)) (def (main) (Color.Green)) (export main))",
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0214")),
            "constructing an abstract type's withheld variant should be CDZ0214; got {:?}",
            out.diagnostics
        );
    }

    /// The concrete companion: the SAME type exported with the wildcard `(. Color *)` makes every
    /// constructor public, so the entry CAN construct `(Color.Green)` and it compiles clean. Pins that
    /// `T.*` is the opt-in that turns an otherwise-abstract handle export concrete.
    #[test]
    fn a_wildcard_export_makes_every_constructor_reachable() {
        let out = compile_package(
            "(do (type Color (Red) (Green) (Blue)) \
                 (def (rank (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3))) \
                 (export (. Color *) rank))",
            "(do (import \"lib\" (Color rank)) (def (main) (rank (Color.Green))) (export main))",
        );
        assert!(
            !out.has_error(),
            "a wildcard-exported type's constructor should be reachable; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// A built-in `=` on a value of an ABSTRACT type (imported handle-only) observes the module's
    /// private representation → CDZ0202 (nominal boundary). The module exports a comparison FUNCTION if
    /// it wants its abstract type compared; the built-in structural `=` is not published by the handle.
    #[test]
    fn a_builtin_comparison_on_an_abstract_type_is_rejected() {
        let out = compile_package(
            "(do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color mk))",
            "(do (import \"lib\" (Color mk)) (def (main) (= (mk) (mk))) (export main))",
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0202")),
            "a built-in comparison on an abstract type's value should be CDZ0202; got {:?}",
            out.diagnostics
        );
    }

    /// The concrete companion: with the type exported `Color.*`, its representation is public in the
    /// importing file, so a built-in `=` on its values is allowed (compiles clean).
    #[test]
    fn a_builtin_comparison_on_a_concrete_imported_type_is_allowed() {
        let out = compile_package(
            "(do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export (. Color *) mk))",
            "(do (import \"lib\" (Color mk)) (def (main) (= (mk) (mk))) (export main))",
        );
        assert!(
            !out.has_error(),
            "a built-in comparison on a concretely-imported type should be allowed; got {:?}",
            out.diagnostics
        );
        assert!(out.artifact(Target::Wasm.artifact_kind()).is_some());
    }

    /// PRIVACY — the prelude-collision construct-shadow path (`resolve_name` step 3d) does NOT leak a
    /// sibling file's private variant constructor. This pins amazon-q PR #392's "sibling variant leak"
    /// (the `scoped.is_none()` guard on the `prelude_colliding_variant_ctor` fall-through) as a FALSE
    /// POSITIVE — `Some(Err(()))` already does not leak — AND that the guard is LOAD-BEARING: rewriting
    /// it to also fire on `Some(Err(()))` (file known, variant not visible) re-opens a real leak.
    ///
    /// The subtlety: `prelude_colliding_variant_ctor` is a PACKAGE-WIDE flat index (every file's
    /// prelude-colliding variants), and step 3d precedes the prelude map. So if `Some(Err(()))` — the
    /// entry's file knows the collision surface but `Int` is not a visible ctor there — fell through to
    /// that index, the entry's bare `(Int …)` would bind `lib`'s PRIVATE `Foo.Int`. The `is_none()`
    /// guard confines the fall-through to the indeterminate/single-file case, so a linked file's hidden
    /// collision instead falls to the prelude WIDTH constructor (`Int` = the width type, no runtime form).
    #[test]
    fn a_sibling_files_prelude_colliding_variant_ctor_does_not_leak_in_construct_position() {
        // `lib` declares a prelude-colliding variant `Int` (lands in the package-wide flat index) but
        // exports only `mk` — nothing of `Foo`. The entry constructs bare `(Int 5)` in HEAD position
        // WITHOUT importing `Foo`. It must NOT bind `lib`'s private `Foo.Int`; bare `Int` here can only
        // mean the prelude WIDTH TYPE constructor, which is a type value with no runtime form → rejected.
        let out = compile_package(
            "(do (type Foo (Int Int64)) (def (mk) (Int 7)) (export mk))",
            "(do (import \"lib\" (mk)) (def (main) (Int 5)) (export main))",
        );
        assert!(
            out.has_error(),
            "bare `(Int 5)` in a sibling file that neither declares nor imports `Foo` must NOT leak \
             `lib`'s private `Foo.Int` — it falls to the prelude width `Int` (a type value, no runtime \
             form); got a clean compile {:?}",
            out.diagnostics
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.message.contains("type value has no runtime form")),
            "the reject must be the prelude WIDTH-type reading of `Int`, not a resolution to `Foo.Int`; \
             got {:?}",
            out.diagnostics
        );
        // Contrast — the LEGITIMATE case: when the entry declares its OWN `Foo` with the colliding
        // variant, bare `(Int 5)` DOES shadow to that local variant (`Some(Ok(_))`) and compiles. This
        // is the `file_scoped_variant_ctor_qualified == Some(Ok(_))` first arm, unaffected by the guard.
        let local = compile_package(
            "(do (def (helper) 1) (export helper))",
            "(do (import \"lib\" (helper)) (type Foo (Int Int64)) (def (main) (Int 5)) (export main))",
        );
        assert!(
            !local.has_error(),
            "a LOCALLY-declared colliding variant `Int` must still construct-shadow in its own file; \
             got {:?}",
            local.diagnostics
        );
        assert!(local.artifact(Target::Wasm.artifact_kind()).is_some());
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
        inputs.push(cadenza_compile_abi::abi::entry_artifact(entry));
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
        // The link-map payload is canonical BINARY AST (seq-254); decode via the shared codec into its
        // `FileSpan` table rather than parsing bytes by hand.
        let rows = decode_link_map(&map.bytes);
        assert_eq!(rows.len(), 2, "one link-map row per file");
        assert!(rows.iter().any(|f| f.path == "lib"));
        assert!(rows.iter().any(|f| f.path == "app"));
        // The unbound-name diagnostic's node demuxes into exactly ONE file's range.
        let node = out
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .and_then(|d| d.node)
            .expect("an unbound-name diagnostic anchored to a node");
        let owning: Vec<&str> = rows
            .iter()
            .filter(|f| f.contains(StructId(node)))
            .map(|f| f.path.as_str())
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
