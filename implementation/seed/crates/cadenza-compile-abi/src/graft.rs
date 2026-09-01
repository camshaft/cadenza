//! Shared subtree-graft helper for the `*_wire` codec modules. Every wire module that carries a full
//! structured `Ty` payload needs to copy a type sub-AST from a source [`Arenas`] into the wire builder;
//! that copy was byte-identical across all of them, so it lives here once.

use cadenza_ast::ast::{Arenas, Builder, Struct, StructId};

/// Copy the subtree rooted at `id` of `src` into builder `b`, returning the new root id. Iterative
/// post-order so a deep type payload can't overflow the native stack (mirrors `cdz::doc_module::copy_from`).
pub(crate) fn copy_from(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    enum Job {
        Visit(StructId),
        EmitList(usize),
    }
    let mut jobs = vec![Job::Visit(id)];
    let mut results: Vec<StructId> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match src.get(sid) {
                Struct::Atom(lid) => {
                    let leaf = src.leaf(*lid).clone();
                    let n = b.atom_leaf(leaf);
                    results.push(n);
                }
                Struct::List(kids) => {
                    jobs.push(Job::EmitList(kids.len()));
                    for &k in kids.iter().rev() {
                        jobs.push(Job::Visit(k));
                    }
                }
            },
            Job::EmitList(n) => {
                let kids = results.split_off(results.len() - n);
                let node = b.list(kids);
                results.push(node);
            }
        }
    }
    results.pop().expect("copy_from leaves a root")
}
