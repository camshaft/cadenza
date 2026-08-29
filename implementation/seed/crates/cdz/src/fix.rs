//! The structural fix-APPLICATION engine — shared by `cdz fix` (whole-file edited text), `cdz check
//! --json` (primitive `[{start,end,text}]` edits), and `cdz lsp` codeAction (the editor quick-fix). One
//! implementation so all three apply a diagnostic's fix IDENTICALLY: build the new tree (the target node
//! transformed per the fix `kind`), then hand old+new to `cadenza_syntax`'s formatting-preserving
//! structural rewriter. Factored out of `main.rs` so `lsp.rs` need not duplicate ~200 lines of tree
//! surgery (wrap/insert quick-fixes need the same builder `cdz fix` uses).

/// Apply a structural fix to `source`, returning the edited text — the STRUCTURAL realization of a
/// diagnostic's suggested fix (`rcdzc`'s `DiagnosticFix`, delivered in-process or over the sidecar wire).
/// Rather than splice bytes by hand (finding a list's closing paren for an
/// insert, trimming a separator for a delete, substituting a `…` sentinel for a wrap, reshaping the wrap
/// for the surface), it builds the NEW TREE — the parsed program with the target node transformed per the
/// fix — and hands old+new to `cadenza_syntax`'s formatting-preserving structural rewriter
/// (`textedit::rewrite_preserving`), the SAME engine `cdz rewrite` uses. That engine edits only the
/// changed subtree at its span, reprints it in the file's SURFACE (ML pretty-print vs s-expr), and leaves
/// all other bytes — layout, comments — verbatim. So surface-correctness, insert placement, and
/// separator hygiene all come from one shared, tested mechanism instead of four hand-rolled text cases.
///
/// `arenas`/`spans` are the parsed program + its node→span table (from `load_program_spanned`); `target`
/// is the fix's node id; `kind`/`repl` its operation + payload. `surface` is the file's format. `None`
/// when the fix cannot be built structurally (an unparseable payload, a node not found) — the caller then
/// declines the fix rather than corrupting.
pub(crate) fn apply_fix_to_source(
    source: &str,
    old: &cadenza_syntax::query::Tree,
    spans: &cadenza_syntax::spans::SpanTable,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<String> {
    let new = fix_new(old, kind, target, repl)?;
    let span_of = |t: &cadenza_syntax::query::Tree| -> Option<(usize, usize)> {
        t.origin()
            .and_then(|id| spans.get(id))
            .map(|s| (s.start, s.end))
    };
    let edited =
        cadenza_syntax::query::textedit::rewrite_preserving(source, old, &new, &span_of, surface);
    Some(edited.output)
}

/// The STRUCTURAL PATCH a fix realizes — the minimal, surface-correct, span-anchored byte edits
/// (`[{start, end, text}]`) turning `source` into the fixed program. The machine-channel (`cdz check
/// --json`) counterpart of [`apply_fix_to_source`]: same new-tree build + same `cadenza_syntax` engine,
/// but returns the primitive edits (via `textedit::edits_preserving`) so an agent applies them directly
/// (`source[start..end] := text`) instead of re-deriving positions from a kind/prefix/suffix. `None` when
/// the fix cannot be built (unparseable payload, node not found).
#[allow(clippy::too_many_arguments)] // source + tree + origin-index + spans + kind/target/repl/surface
pub(crate) fn fix_edits(
    source: &str,
    old: &cadenza_syntax::query::Tree,
    origins: &OriginPaths,
    spans: &cadenza_syntax::spans::SpanTable,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
    surface: cadenza_syntax::convert::Format,
) -> Option<Vec<cadenza_syntax::query::textedit::Edit>> {
    let span_of = |t: &cadenza_syntax::query::Tree| -> Option<(usize, usize)> {
        t.origin()
            .and_then(|id| spans.get(id))
            .map(|s| (s.start, s.end))
    };
    // DELETE fast path: the edit is exactly the target child's (widened) span removed — we already know
    // WHICH child vanishes, so we skip both the parent-subtree diff AND its LCS alignment. Diffing the
    // parent (via `localized_change` → `edits_preserving` → `align`) re-runs an O(children²) alignment DP
    // to rediscover the one deleted child; for a WIDE parent (a `do` block / match with N children) that
    // is O(N²) per fix, so N delete fixes on one file were O(N³) (a `do` of N discarded statements:
    // N=100/200/400 = 33/207/1639ms). `delete_edit` emits the identical edit in O(1) from the known span.
    if kind == "delete" {
        let span = spans.get(target)?;
        return cadenza_syntax::query::textedit::delete_edit(source, span.start, span.end, surface)
            .map(|ed| vec![ed]);
    }
    // Diff only the CHANGED SUBTREE, not the whole program. A fix touches one node (`replace`/`wrap`/
    // `insert` at the target), and `edits_preserving` reports edits only within the changed span — so
    // diffing `(old_subtree, new_subtree)` yields the SAME edits as diffing the whole `(old, new)` tree,
    // but walks O(subtree) instead of O(program). Computing a fix PER diagnostic over the whole tree was
    // O(fixes × program) = O(N²) on a file with many fixable warnings (a wide match with N unused-binder
    // arms: N `transform_target` whole-tree rebuilds, each deep-cloning the other N−1 arms).
    // `localized_change` finds the target's subtree (O(depth) via `origins`) + builds only ITS replacement.
    let (old_sub, new_sub) = localized_change(old, origins, kind, target, repl)?;
    Some(cadenza_syntax::query::textedit::edits_preserving(
        source, old_sub, &new_sub, &span_of, surface,
    ))
}

/// The smallest `(old_subtree, new_subtree)` pair whose structural diff yields a fix's byte edits — the
/// CHANGED subtree, not the whole program. `replace`/`wrap`/`insert` change the TARGET node itself, so its
/// subtree is the diff root; `delete` removes the target from its PARENT list, so the parent is the root.
/// Because `edits_preserving` emits edits only within the changed span, diffing this local pair is
/// byte-identical to diffing the whole tree — at O(subtree) not O(program). The target (or, for delete, its
/// parent) is located in O(depth) via the precomputed `origins` index — NOT an O(program) scan (which,
/// per fix over N fixes, was O(N²)). Returns a BORROW of the old subtree (into `old`) + the freshly built
/// new subtree. `None` if the target is not found or the payload does not parse.
pub(crate) fn localized_change<'t>(
    old: &'t cadenza_syntax::query::Tree,
    origins: &OriginPaths,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
) -> Option<(&'t cadenza_syntax::query::Tree, cadenza_syntax::query::Tree)> {
    use cadenza_syntax::query::Tree;
    if kind == "delete" {
        // Delete edits the PARENT list (the target vanishes from its children). Diff the parent subtree.
        let parent = origins.parent(old, target)?;
        let new_parent = delete_target(parent, target)?;
        return Some((parent, new_parent));
    }
    // `replace`/`wrap`/`insert` change the target node in place — its subtree is the diff root.
    let node = origins.node(old, target)?;
    let new_node = match kind {
        "replace" => parse_fragment(repl)?,
        "wrap" => {
            let ctor = parse_fragment(repl)?;
            substitute_hole(&ctor, node)
        }
        "insert" => {
            let Tree::List(children, origin) = node else {
                return None;
            };
            let mut children = children.clone();
            for arm in split_top_forms(repl) {
                children.push(parse_fragment(&arm)?);
            }
            Tree::List(children, *origin)
        }
        _ => return None,
    };
    Some((node, new_node))
}

/// A `provenance id → path-from-root` index over a parsed `Tree`, so locating a fix's target node is
/// O(depth) (follow the path) instead of O(program) (scan every node comparing origins). Built ONCE per
/// file in a single walk (`OriginPaths::of`) and shared across all its fixes: a file with N fixable
/// diagnostics located each target by a fresh whole-tree scan (`find_by_origin`) → O(N × program) = O(N²)
/// (`find_by_origin` + `Tree::origin` were ~82% of a wide-fixable-warnings check). A `path` is the child
/// indices from the root down to the node (empty = the root itself).
pub(crate) struct OriginPaths {
    path: std::collections::HashMap<cadenza_syntax::StructId, Vec<usize>>,
}

/// A file's parsed `Tree` + its `origin → path` index, both built once and shared (`Rc`) across every fix
/// that targets the file (the `check`-loop cache value).
pub(crate) type FileTree = (
    std::rc::Rc<cadenza_syntax::query::Tree>,
    std::rc::Rc<OriginPaths>,
);

impl OriginPaths {
    /// One walk of `tree`, recording each origin-bearing node's path from the root.
    pub(crate) fn of(tree: &cadenza_syntax::query::Tree) -> OriginPaths {
        use cadenza_syntax::query::Tree;
        fn walk(
            t: &Tree,
            path: &mut Vec<usize>,
            out: &mut std::collections::HashMap<cadenza_syntax::StructId, Vec<usize>>,
        ) {
            if let Some(id) = t.origin() {
                out.insert(id, path.clone());
            }
            if let Tree::List(items, _) = t {
                for (i, c) in items.iter().enumerate() {
                    path.push(i);
                    walk(c, path, out);
                    path.pop();
                }
            }
        }
        let mut out = std::collections::HashMap::default();
        walk(tree, &mut Vec::new(), &mut out);
        OriginPaths { path: out }
    }

    /// The node at `origin` (O(depth) — follow the cached path), a borrow into `tree`. `None` if absent
    /// or the path does not resolve (a stale index against a different tree).
    pub(crate) fn node<'t>(
        &self,
        tree: &'t cadenza_syntax::query::Tree,
        origin: cadenza_syntax::StructId,
    ) -> Option<&'t cadenza_syntax::query::Tree> {
        use cadenza_syntax::query::Tree;
        let mut cur = tree;
        for &i in self.path.get(&origin)? {
            let Tree::List(items, _) = cur else {
                return None;
            };
            cur = items.get(i)?;
        }
        Some(cur)
    }

    /// The PARENT list node of `origin` (the node a `delete` edits) — the node at the path with its last
    /// step dropped. `None` if `origin` is the root (no parent) or absent.
    pub(crate) fn parent<'t>(
        &self,
        tree: &'t cadenza_syntax::query::Tree,
        origin: cadenza_syntax::StructId,
    ) -> Option<&'t cadenza_syntax::query::Tree> {
        use cadenza_syntax::query::Tree;
        let full = self.path.get(&origin)?;
        let (_, parent_path) = full.split_last()?;
        let mut cur = tree;
        for &i in parent_path {
            let Tree::List(items, _) = cur else {
                return None;
            };
            cur = items.get(i)?;
        }
        Some(cur)
    }
}

/// Build the `(old, new)` tree pair a fix transforms — the shared core of [`apply_fix_to_source`] and
/// [`fix_edits`]. `old` is the parsed program; `new` is it with the target node transformed per kind (a
/// PURE tree op — no text, no `…` sentinel, no paren-finding). `None` if the payload doesn't parse or the
/// target isn't found.
pub(crate) fn fix_new(
    old: &cadenza_syntax::query::Tree,
    kind: &str,
    target: cadenza_syntax::StructId,
    repl: &str,
) -> Option<cadenza_syntax::query::Tree> {
    use cadenza_syntax::query::Tree;
    let new = if kind == "delete" {
        // Delete removes the node from its parent's child list — a structural op the node-transform
        // closure can't express (it returns a replacement node, not "no node"), so its own builder.
        delete_target(old, target)?
    } else {
        transform_target(old, target, &mut |node: &Tree| -> Option<Tree> {
            match kind {
                // Replace the node with the parsed payload subtree.
                "replace" => parse_fragment(repl),
                // Wrap: build the ctor form from `repl`'s parse and substitute its `…` hole atom with the
                // ORIGINAL node subtree (spans intact) — `(Some …)` + node → `(Some <node>)`.
                "wrap" => {
                    let ctor = parse_fragment(repl)?;
                    Some(substitute_hole(&ctor, node))
                }
                // Insert: append the parsed arm form(s) as new children at the end of the target LIST.
                "insert" => {
                    let Tree::List(children, origin) = node else {
                        return None; // an insert targets a list (the `(match …)` form)
                    };
                    let mut children = children.clone();
                    for arm in split_top_forms(repl) {
                        children.push(parse_fragment(&arm)?);
                    }
                    Some(Tree::List(children, *origin))
                }
                _ => None,
            }
        })?
    };
    Some(new)
}

/// Parse a fix-payload s-expression fragment (`(Some …)`, `(B unit)`, `compute`) into an owned [`Tree`],
/// or `None` if it does not parse. New nodes carry NO provenance (they are synthesized), which the
/// structural rewriter handles — only ORIGINAL nodes need spans.
pub(crate) fn parse_fragment(text: &str) -> Option<cadenza_syntax::query::Tree> {
    cadenza_syntax::sexpr::read(text)
        .ok()
        .map(|a| cadenza_syntax::query::Tree::of(&a))
}

/// Split a space-joined run of top-level s-expression forms (the `insert` payload, e.g. `(Green unit)
/// (Blue unit)`) into its individual forms. Uses the reader's multi-form parse, then renders each back —
/// so each element is a complete, independently-parseable form.
pub(crate) fn split_top_forms(text: &str) -> Vec<String> {
    match cadenza_syntax::sexpr::read_all(text) {
        Ok(a) => {
            let tree = cadenza_syntax::query::Tree::of(&a);
            match &tree {
                // `read_all` wraps multiple forms in a synthetic `(do …)`; unwrap to the forms.
                cadenza_syntax::query::Tree::List(items, _)
                    if matches!(
                        items.first(),
                        Some(cadenza_syntax::query::Tree::Atom(
                            cadenza_syntax::ast::Leaf::Name(n),
                            _,
                        )) if &**n == "do"
                    ) =>
                {
                    items.iter().skip(1).map(|t| t.to_sexpr()).collect()
                }
                other => vec![other.to_sexpr()],
            }
        }
        Err(_) => vec![text.to_string()],
    }
}

/// The wrap-fix HOLE sentinel — the placeholder atom a wrap fix-template carries (`(Some …)`) for the
/// wrapped subtree to replace. A cdz-LOCAL copy of the `…` char rcdzc's fix templates emit, so a
/// `!standalone` `cdz` need not link `rcdzc` just for this const (the fix templates cross the delegated
/// sidecar boundary as cadenza-ast, carrying this same `…` atom either way — the dep-flip's crate-split).
/// A `#[cfg(feature = "standalone")]` drift-guard test pins it EQUAL to `rcdzc::WRAP_HOLE` so the copy can
/// never silently diverge from the compiler's sentinel.
pub(crate) const WRAP_HOLE: char = '…';

/// Substitute the [`WRAP_HOLE`] atom inside `template` with `fill` — the structural realization of
/// a wrap: `(Some …)` with `…` replaced by the wrapped subtree becomes `(Some <subtree>)`. Recurses; a
/// non-hole node is copied structurally (preserving its provenance so an unchanged child keeps its span).
pub(crate) fn substitute_hole(
    template: &cadenza_syntax::query::Tree,
    fill: &cadenza_syntax::query::Tree,
) -> cadenza_syntax::query::Tree {
    use cadenza_syntax::query::Tree;
    match template {
        Tree::Atom(cadenza_syntax::ast::Leaf::Name(n), _)
            if &**n == WRAP_HOLE.to_string().as_str() =>
        {
            fill.clone()
        }
        Tree::Atom(..) => template.clone(),
        Tree::List(items, origin) => Tree::List(
            items.iter().map(|t| substitute_hole(t, fill)).collect(),
            *origin,
        ),
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: total sibling `Tree` clones `transform_target` performs since the last reset. On a HIT it
    /// clones the target's siblings (the other children of each rebuilt-spine node) ONCE; a MISS clones
    /// NOTHING. The old code deep-cloned every child of every visited node into an `out` it then discarded
    /// on a miss → O(subtree) wasted clones per level → O(depth²) per fix. This counter locks the clone
    /// work to O(siblings-along-the-spine), independent of the untouched subtrees' size. See
    /// `transform_target_does_not_clone_untouched_subtrees`.
    pub(crate) static TRANSFORM_SIBLING_CLONES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Rebuild `tree`, applying `f` to the node whose origin is `target` (replacing it with `f`'s result).
/// `None` if the target is not found or `f` declines. Recurses structurally, preserving provenance on the
/// untouched nodes so the rewriter edits only the one changed subtree.
pub(crate) fn transform_target(
    tree: &cadenza_syntax::query::Tree,
    target: cadenza_syntax::StructId,
    f: &mut dyn FnMut(&cadenza_syntax::query::Tree) -> Option<cadenza_syntax::query::Tree>,
) -> Option<cadenza_syntax::query::Tree> {
    use cadenza_syntax::query::Tree;
    if tree.origin() == Some(target) {
        return f(tree);
    }
    match tree {
        Tree::Atom(..) => None,
        Tree::List(items, origin) => {
            // Find the ONE child that yields a transform, then rebuild the list with the OTHER children
            // cloned. Critically, do NOT build the output (cloning every child) until a hit is confirmed:
            // the old code `out.push(child.clone())`-ed every child as it went and only THEN checked `hit`,
            // so a subtree NOT containing `target` still deep-cloned all its children into an `out` that was
            // then discarded (returned `None`) — and the recursion did that at every level, so a fix whose
            // target sits beside a deep sibling cloned that sibling's whole subtree per level → O(depth²)
            // per fix, O(N·depth²)=O(N³) over N per-diagnostic fixes (a 200-deep-tuple match with 200 unused
            // binders: 897ms; 400: 7.3s). Now a miss returns `None` with ZERO cloning, and a hit clones only
            // the siblings ONCE.
            let hit = items
                .iter()
                .enumerate()
                .find_map(|(i, child)| transform_target(child, target, f).map(|nc| (i, nc)));
            hit.map(|(i, new_child)| {
                #[cfg(test)]
                TRANSFORM_SIBLING_CLONES.with(|c| c.set(c.get() + (items.len() - 1) as u64));
                let mut out = Vec::with_capacity(items.len());
                out.extend(items[..i].iter().cloned());
                out.push(new_child);
                out.extend(items[i + 1..].iter().cloned());
                Tree::List(out, *origin)
            })
        }
    }
}

/// Rebuild `tree` with the node whose origin is `target` REMOVED from its parent's child list — the
/// structural realization of a delete (`(host (log) 42)` → `(host () 42)`; the separator hygiene the old
/// text path hand-trimmed is handled by the rewriter's child-alignment). `None` if not found.
pub(crate) fn delete_target(
    tree: &cadenza_syntax::query::Tree,
    target: cadenza_syntax::StructId,
) -> Option<cadenza_syntax::query::Tree> {
    use cadenza_syntax::query::Tree;
    match tree {
        Tree::Atom(..) => None,
        Tree::List(items, origin) => {
            if items.iter().any(|c| c.origin() == Some(target)) {
                let kept: Vec<Tree> = items
                    .iter()
                    .filter(|c| c.origin() != Some(target))
                    .cloned()
                    .collect();
                return Some(Tree::List(kept, *origin));
            }
            let mut hit = false;
            let mut out = Vec::with_capacity(items.len());
            for child in items {
                if !hit && let Some(nc) = delete_target(child, target) {
                    out.push(nc);
                    hit = true;
                } else {
                    out.push(child.clone());
                }
            }
            hit.then_some(Tree::List(out, *origin))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DRIFT-GUARD: the cdz-local [`WRAP_HOLE`] must equal the `…` char rcdzc's fix templates emit
    /// (`rcdzc::WRAP_HOLE`) — pinned here so the local copy (which lets a `!standalone` `cdz` avoid linking
    /// `rcdzc` for this const) can never silently diverge from the compiler's sentinel. Runs only in a
    /// `standalone` build (the only config where `rcdzc` is linked to compare against).
    #[cfg(feature = "standalone")]
    #[test]
    fn wrap_hole_matches_rcdzc_sentinel() {
        assert_eq!(WRAP_HOLE, rcdzc::WRAP_HOLE);
    }

    #[test]
    fn split_top_forms_keeps_a_single_form_whole() {
        // One top-level form (`read_all` returns it directly, not wrapped in a synthetic `(do …)`) → a
        // single-element vec holding that form, re-rendered. This is the common `insert` payload of one arm.
        let forms = split_top_forms("(Green unit)");
        assert_eq!(forms.len(), 1, "one form → one element: {forms:?}");
        assert_eq!(forms[0], "(Green unit)");
    }

    #[test]
    fn split_top_forms_unwraps_multiple_forms_from_the_synthetic_do() {
        // A space-joined RUN of forms (`read_all` wraps them in a synthetic `(do a b …)`); the split must
        // UNWRAP the `do` and return each form independently — the case a naive "one payload = one node"
        // would get wrong (it would insert the whole `(do …)` as a single bogus arm).
        let forms = split_top_forms("(Green unit) (Blue unit)");
        assert_eq!(
            forms,
            vec!["(Green unit)".to_string(), "(Blue unit)".to_string()]
        );
        // Three forms unwrap the same way.
        let three = split_top_forms("(A) (B) (C)");
        assert_eq!(
            three,
            vec!["(A)".to_string(), "(B)".to_string(), "(C)".to_string()]
        );
    }

    #[test]
    fn split_top_forms_returns_the_text_verbatim_when_it_does_not_parse() {
        // An unparseable payload (a dangling paren) can't be split — the `Err(_)` fallback returns the raw
        // text as the single element, so the caller inserts it verbatim rather than dropping the fix. Total,
        // never a panic.
        let forms = split_top_forms("(unbalanced");
        assert_eq!(forms, vec!["(unbalanced".to_string()]);
    }

    #[test]
    fn split_top_forms_does_not_mistake_a_user_do_for_the_synthetic_wrapper() {
        // A payload that IS a single `(do …)` form (a user's sequencing block, one top-level form) must be
        // returned WHOLE, not unwrapped into its members — `read_all` returns a lone form as itself (no
        // synthetic wrapper is added for a single form), so the `do`-unwrap arm (which only fires on the
        // MULTI-form synthetic wrap) does not strip a genuine single `(do …)`.
        let forms = split_top_forms("(do (a) (b))");
        assert_eq!(
            forms.len(),
            1,
            "a lone `(do …)` form is one payload, not unwrapped: {forms:?}"
        );
        assert_eq!(forms[0], "(do (a) (b))");
    }

    #[test]
    fn parse_fragment_parses_a_form_and_declines_an_unparseable_one() {
        // A well-formed fix payload parses into an owned Tree; re-rendering it round-trips the text.
        let tree = parse_fragment("(Some x)").expect("a parseable fragment");
        assert_eq!(tree.to_sexpr(), "(Some x)");
        // A bare atom is also a valid fragment (a `replace` payload can be a single name).
        assert_eq!(
            parse_fragment("compute").map(|t| t.to_sexpr()),
            Some("compute".to_string())
        );
        // An unparseable payload (dangling paren) declines with None — the caller drops that fix rather
        // than panicking. Total.
        assert!(
            parse_fragment("(unbalanced").is_none(),
            "an unparseable fragment is None, not a panic"
        );
    }

    #[test]
    fn substitute_hole_fills_the_wrap_hole_and_copies_everything_else() {
        use cadenza_syntax::query::Tree;
        // A wrap realizes `(Some <HOLE>)` with the wrapped subtree in place of the hole atom. Build the
        // template from the real WRAP_HOLE spelling so the atom matches the production code's sentinel.
        let template = parse_fragment(&format!("(Some {})", WRAP_HOLE)).expect("template parses");
        let fill = parse_fragment("(compute 1)").expect("fill parses");
        let filled = substitute_hole(&template, &fill);
        assert_eq!(
            filled.to_sexpr(),
            "(Some (compute 1))",
            "the hole is replaced by the fill; the `Some` head is copied verbatim"
        );
        // A template with NO hole is copied structurally unchanged (the fill is unused).
        let no_hole = parse_fragment("(A b c)").expect("parses");
        assert_eq!(
            substitute_hole(&no_hole, &fill).to_sexpr(),
            "(A b c)",
            "a hole-free template is unchanged"
        );
        // A bare hole atom at the root becomes exactly the fill.
        let bare_hole = Tree::Atom(
            cadenza_syntax::ast::Leaf::Name(WRAP_HOLE.to_string().into()),
            None,
        );
        assert_eq!(
            substitute_hole(&bare_hole, &fill).to_sexpr(),
            "(compute 1)",
            "a root-level hole is wholly replaced by the fill"
        );
    }
}
