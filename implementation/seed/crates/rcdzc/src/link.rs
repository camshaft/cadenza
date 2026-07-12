//! `link` — package linking: merge N named `ast` artifacts into ONE compilation unit before the
//! pure pipeline runs (`DESIGN-package-linking.md`). Each file is a module; all files are spliced
//! into one [`Arenas`] under a synthesized `(do …)` root, so the existing `Db::load` sees exactly
//! one program in one arena — the same thing it sees for a single file, just assembled from many.
//!
//! This is INTRA-PACKAGE linking only: nothing crosses a component boundary, so there is zero
//! component-ABI / envelope work. Monomorphization is the existing β-reduction; one component is the
//! existing backend. The link step is the structured analogue of the bootstrap Makefile's `cat`, but
//! at the arena level with real ids: it appends each file's `leaves`/`structure` with a per-file id
//! offset and re-parents every file's top-level items under one `(do …)`.
//!
//! Increment status (`DESIGN-package-linking.md` §8): this module realizes step 2 — the arena splice,
//! the `link()` skeleton, and the `FileSpan` demux table. It does NOT yet file-scope name resolution
//! (step 3): after the splice every file's defs share one flat namespace, so a package whose files do
//! not name-collide compiles exactly like the concatenation. Explicit `(import …)`, per-file
//! visibility, cyclic-import rejection, and the diagnostics link-map artifact are later steps that
//! build on this one.

use crate::ast::{Arenas, Leaf, LeafId, Struct, StructId};
use crate::diag::Reject;

/// The input-artifact kind that names the package ENTRY file. Its bytes are the entry's artifact name
/// (a UTF-8 string) — the file whose `(export …)` forms the component boundary. It rides the artifact
/// stream exactly like the `sidecar`/`spans` inputs (`DESIGN-package-linking.md` §3c): a new kind, a
/// `.find(kind == KIND_ENTRY)`, no change to `compile`'s signature. Absent + a single `ast` = today's
/// single-file compile; absent + multiple `ast` = a package with no named entry, which declines.
pub const KIND_ENTRY: &str = "entry";

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

/// The result of linking a package: the merged arena (ready for `Db::load`), the per-file demux table,
/// and which file is the entry (whose `(export …)` becomes the component boundary — used from step 3
/// on; step 2 records it without yet gating on it).
#[derive(Clone, PartialEq, Debug)]
pub struct LinkedProgram {
    /// All files' structure/leaves appended (ids offset per file), re-rooted under a `(do …)`.
    pub arenas: Arenas,
    /// One entry per input file, in splice order — the `StructId → file` demux table.
    pub files: Vec<FileSpan>,
    /// Index into `files` of the entry file (whose exports form the component boundary).
    pub entry: usize,
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
            return Err(Reject::decline(format!(
                "package entry `{entry}` names no supplied `ast` file"
            )));
        }
    };

    // Combined arenas, built by append-with-offset. Leaves are copied verbatim (they are values —
    // dedup across files is unnecessary for correctness); structure entries are remapped by this
    // file's leaf/struct base. We gather each file's remapped TOP-LEVEL items to re-parent under one
    // synthesized `(do …)` root at the end.
    let mut leaves: Vec<Leaf> = Vec::new();
    let mut structure: Vec<Struct> = Vec::new();
    let mut file_spans: Vec<FileSpan> = Vec::with_capacity(files.len());
    let mut do_children: Vec<StructId> = Vec::new();

    for (path, ast) in files {
        let leaf_base = leaves.len() as u32;
        let struct_base = structure.len() as u32;

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

        // This file's top-level items (its `(module …)` / `(do …)` children, or the bare root),
        // remapped, become children of the combined `(do …)`. Reusing the SAME item-extraction rule
        // `db::top_items` applies to a single file keeps a one-file package identical to today.
        for item in top_items(ast) {
            do_children.push(StructId(item.0 + struct_base));
        }

        file_spans.push(FileSpan {
            path: path.clone(),
            struct_base,
            struct_count: ast.structure.len() as u32,
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
        entry: entry_ix,
    })
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
        // The merged `(do …)` gathers BOTH files' top-level items (2 defs + 2 exports + head = 5).
        let items = match linked.arenas.get(linked.arenas.root) {
            Struct::List(items) => items.len(),
            _ => panic!("root is not a list"),
        };
        assert_eq!(items, 5);
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

    /// The Inc-2 GATE (`DESIGN-package-linking.md` §8.2): a package of files spliced together compiles
    /// and runs as ONE program. With flat (not-yet-file-scoped) resolution, a `main` in one file can
    /// already reach a `helper` in another — which is exactly what proves the splice produced a single
    /// coherent arena (same defs, same lookup) equivalent to concatenating the sources. Drives the full
    /// `compile()` path (decode → link → Db::load → layout → emit) and asserts a real component comes
    /// out. (Cross-file resolution becoming EXPLICIT via `(import …)` is Inc 3; here it is the flat
    /// default, used only to witness that the merged arena is well-formed and emit-able.)
    #[test]
    fn a_two_file_package_splices_into_one_emittable_component() {
        use crate::abi::Artifact;
        use crate::backend::Target;

        // File `lib` defines a helper; file `app` (the entry) calls it and exports `main`.
        let lib = crate::codec::encode(&arena_of("(do (def (helper) 40))"));
        let app = crate::codec::encode(&arena_of("(do (def (main) (+ (helper) 2)) (export main))"));
        let inputs = vec![
            Artifact::new(Artifact::KIND_AST, "lib", lib),
            Artifact::new(Artifact::KIND_AST, "app", app),
            Artifact::new(KIND_ENTRY, "entry", b"app".to_vec()),
        ];
        let out = crate::compile(&inputs, &[Target::Wasm]);
        assert!(
            !out.has_error(),
            "two-file package should compile clean; diagnostics: {:?}",
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
}
