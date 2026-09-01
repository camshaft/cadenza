//! AST-stripping load passes carved out of db.rs: leading-`(doc …)`, `(const …)`/`(quote …)`
//! param modifiers, comment nodes, and `(@ …)` annotations — see each fn's citations.
use super::*;

/// Strip a LEADING `(doc …)` form from every `(def …)` in the arena, IN PLACE, and RETURN the doc text
/// captured off each def — keyed by the def's SIGNATURE occurrence (`children[1]`), the stable node id
/// both name lookup (`def_by_name` → `Def::sig_occ`) and body lookup (`def_index_by_body` → `sig_occ`)
/// converge on, so a doc query can reach the string from either the def NAME or a cursor OFFSET.
///
/// A definition — value, function, or nullary — may carry a documentation form immediately after its
/// name/signature that documents it and is NOT part of the value (`glossary`: a definition is "a value,
/// function, type", so the doc affordance cannot depend on which — `(def (f) (doc "…") body)` AND `(def
/// answer (doc "…") 42)` both bind ignoring the doc). Every consumer reads a def's body as `tail.get(1)`
/// (the element right after the signature), so an un-stripped `(doc …)` would be mis-read AS the body
/// ("unbound name doc"). This normalizes every `(def sig (doc …)… body)` to `(def sig body)` at load —
/// one place, so every def KIND (top-level, do-local, module member) is fixed uniformly. The def keeps
/// its `StructId` (only its child list shrinks); the orphaned doc node stays in the arena, unreferenced.
///
/// Conservative: only a `(doc …)`-HEADED form BETWEEN the signature and the LAST tail element is dropped
/// (the body is always last). A def with ≤1 tail element, or whose only post-sig element is the body, is
/// untouched — byte-identical to before for every doc-less program.
///
/// The captured doc is carried in the canonical representation (this column, keyed off the AST node),
/// NOT discarded as lexical trivia — and because it is stripped from the body it never lowers to Core, so
/// it cannot change the program's runtime meaning. Every def KIND can carry a `(doc …)` (the affordance
/// does not depend on whether the def is a value, function, or type), so every part of a program is
/// documentable.
//= spec/capabilities/agent-authoring.md#documentation-is-part-of-the-representation
//# Documentation attached to a definition MUST be carried in the canonical representation rather than discarded as lexical trivia.
//= spec/capabilities/agent-authoring.md#documentation-is-part-of-the-representation
//# Any definition MUST be able to carry documentation, so that every part of a program can be documented.
//= spec/capabilities/agent-authoring.md#documentation-is-machine-readable
//# Documentation MUST NOT change the runtime meaning of a program.
// The `(doc "…")` rides as an ordinary node of the binary AST attached to its def — the codec's generic
// encode/decode round-trips it like any list node, and this load-time pass captures it into the doc
// column (post-decode). So the AST carries documentation attached to a definition, per ast-encoding.
// (The COMMENT half of that section — a comment as a tree node — is ALSO realized now: a `//` comment
// reifies to a `(comment "text" <form>)` node that `strip_comments` peels; see there for its citations.)
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# The abstract syntax tree MUST be able to carry documentation attached to a definition, as required by the agent-authoring capability.
pub(crate) fn strip_def_docs(ast: &mut Arenas) -> crate::fxhash::FxHashMap<StructId, String> {
    let mut docs: crate::fxhash::FxHashMap<StructId, String> = crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if ast.as_form(id, "def").is_none() {
            continue;
        }
        // The full child list `[def-head, sig, middle…, body]` — need the head to rebuild it. Nothing to
        // strip unless there is a middle region (a def of `[head, sig, body]` = 3 children is minimal).
        let Struct::List(children) = ast.get(id) else {
            continue;
        };
        if children.len() <= 3 {
            continue;
        }
        // Keep the head + signature and the body (last); drop any `(doc …)`-headed form between them. A
        // non-doc middle element is left in place (defensive — a malformed multi-body def is diagnosed
        // elsewhere, not silently reshaped here).
        let n = children.len();
        let head = children[0];
        let sig = children[1];
        let body = children[n - 1];
        let middle = children[2..n - 1].to_vec();
        let mut middle_kept: Vec<StructId> = Vec::with_capacity(middle.len());
        let mut doc_text: Option<String> = None;
        for m in middle {
            // A `(doc "text")` form — capture its FIRST string operand (keyed by the def's signature), then
            // drop it from the def body. A malformed doc (no string operand) captures nothing but is still
            // stripped, so the body reader is unaffected. First doc wins if a def somehow carries several.
            if let Some(tail) = ast.as_form(m, "doc") {
                if doc_text.is_none() {
                    doc_text = tail
                        .first()
                        .and_then(|&t| ast.as_str(t))
                        .map(str::to_string);
                }
                continue;
            }
            middle_kept.push(m);
        }
        if let Some(text) = doc_text {
            docs.insert(sig, text);
        }
        if middle_kept.len() + 3 == n {
            continue; // no doc form found — leave the def untouched (the body stayed last)
        }
        let mut kept = vec![head, sig];
        kept.extend(middle_kept);
        kept.push(body);
        ast.structure[i] = Struct::List(kept);
    }
    docs
}

/// Unwrap every `(const BINDER)` PARAMETER wrapper in a `def` signature or `fn` parameter list, in place,
/// and return the set of the stripped params' NAME occurrences — the EXPLICIT compile-time parameters
/// (`DESIGN-recursive-generic-monomorphization-rcdzc.md` Addendum 3). A `const` param is inlined + erased
/// at instantiation; `const` is a DECLARATION consumed by the specializer, not part of the binder shape
/// the resolver/typer walk — so it is removed here (BEFORE `scan_top_level` / resolution), exactly as
/// `strip_def_docs` removes a doc form, leaving every downstream reader a plain binder.
///
/// A `const` param may wrap a bare name `(const d)` or an annotated binder `(const (: d T))`; the inner
/// node replaces the wrapper in the parameter list, and its NAME occurrence (the bare name, or the `(: name
/// T)`'s first child) is recorded. Runs over ALL forms (a def OR an fn), so a lambda `(fn ((const d) …)
/// …)` marks its const params too. Non-`const` params are untouched. A `(const …)` in a NON-parameter
/// position (a value expression) is NOT a parameter and is left alone (only a direct child of a def
/// signature / fn param list is unwrapped).
pub(crate) fn strip_const_params(ast: &mut Arenas) -> crate::fxhash::FxHashSet<StructId> {
    let mut const_params: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    // The NAME occurrence of a (already-unwrapped) binder: a bare name is itself; a `(: name T)` binder is
    // its first child. Mirrors `param_name_occ` but over `&Arenas` (no `Db` yet at load).
    fn name_occ_of(ast: &Arenas, binder: StructId) -> StructId {
        if ast.as_name(binder).is_some() {
            return binder;
        }
        if let Some(tail) = ast.as_form(binder, ":")
            && let Some(&n) = tail.first()
        {
            return n;
        }
        binder
    }
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        // The parameter list to scan: a `def`'s SIGNATURE (its first tail element, `[NAME, p…]` — params
        // are indices 1..) or an `fn`'s PARAMETER LIST (its first tail element, `(p…)` — params are all).
        let (list_occ, first_param_ix) = if let Some(tail) = ast.as_form(id, "def") {
            match tail.first() {
                Some(&sig) if matches!(ast.get(sig), Struct::List(_)) => (sig, 1usize),
                _ => continue,
            }
        } else if let Some(tail) = ast.as_form(id, "fn") {
            match tail.first() {
                Some(&params) if matches!(ast.get(params), Struct::List(_)) => (params, 0usize),
                _ => continue,
            }
        } else {
            continue;
        };
        let Struct::List(children) = ast.get(list_occ) else {
            continue;
        };
        let children = children.clone();
        let mut rewritten = children.clone();
        let mut changed = false;
        for (ix, &child) in children.iter().enumerate() {
            if ix < first_param_ix {
                continue; // the def NAME at index 0 is not a parameter
            }
            // A `(const BINDER)` wrapper — exactly one operand. Unwrap to BINDER, record its name occ.
            if let Some(tail) = ast.as_form(child, "const")
                && tail.len() == 1
            {
                let binder = tail[0];
                const_params.insert(name_occ_of(ast, binder));
                rewritten[ix] = binder;
                changed = true;
            }
        }
        if changed {
            ast.structure[list_occ.0 as usize] = Struct::List(rewritten);
        }
    }
    const_params
}

/// Normalize away a `(quote BINDER)` PARAMETER wrapper on every `def`/`fn` signature, recording the
/// stripped param's NAME occurrence in the returned set (populates `Db::quote_params`). A `quote` param is
/// an UNEVALUATED (call-by-AST) macro parameter (`DESIGN-macro-system.md` §2): the caller's argument is
/// passed as its reified `Ast` instead of an eager value. The twin of [`strip_const_params`] — same
/// signature-scan (a `def`'s params are its signature tail `[NAME, p…]`; an `fn`'s are its whole param
/// list), same in-place unwrap `(quote (: x T))` → `(: x T)` (inner binder ids unchanged), same
/// name-occ keying. Runs BEFORE `reify_quotes`, so a binder-position `(quote x)` is unwrapped here and
/// never reified as a quote EXPRESSION. No-op (empty set, byte-identical) for a program with no `quote`
/// param.
pub(crate) fn strip_quote_params(ast: &mut Arenas) -> crate::fxhash::FxHashSet<StructId> {
    let mut quote_params: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    // The NAME occurrence of a (already-unwrapped) binder: a bare name is itself; a `(: name T)` binder is
    // its first child. Mirrors `strip_const_params`'s `name_occ_of` (over `&Arenas`, no `Db` yet at load).
    fn name_occ_of(ast: &Arenas, binder: StructId) -> StructId {
        if ast.as_name(binder).is_some() {
            return binder;
        }
        if let Some(tail) = ast.as_form(binder, ":")
            && let Some(&n) = tail.first()
        {
            return n;
        }
        binder
    }
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        // The parameter list to scan: a `def`'s SIGNATURE (params are indices 1..) or an `fn`'s PARAMETER
        // LIST (params are all) — identical to `strip_const_params`.
        let (list_occ, first_param_ix) = if let Some(tail) = ast.as_form(id, "def") {
            match tail.first() {
                Some(&sig) if matches!(ast.get(sig), Struct::List(_)) => (sig, 1usize),
                _ => continue,
            }
        } else if let Some(tail) = ast.as_form(id, "fn") {
            match tail.first() {
                Some(&params) if matches!(ast.get(params), Struct::List(_)) => (params, 0usize),
                _ => continue,
            }
        } else {
            continue;
        };
        let Struct::List(children) = ast.get(list_occ) else {
            continue;
        };
        let children = children.clone();
        let mut rewritten = children.clone();
        let mut changed = false;
        for (ix, &child) in children.iter().enumerate() {
            if ix < first_param_ix {
                continue; // the def NAME at index 0 is not a parameter
            }
            // A `(quote BINDER)` wrapper — exactly one operand. Unwrap to BINDER, record its name occ.
            if let Some(tail) = ast.as_form(child, "quote")
                && tail.len() == 1
            {
                let binder = tail[0];
                quote_params.insert(name_occ_of(ast, binder));
                rewritten[ix] = binder;
                changed = true;
            }
        }
        if changed {
            ast.structure[list_occ.0 as usize] = Struct::List(rewritten);
        }
    }
    quote_params
}

/// The known annotation NAMES `strip_annotations` records into a policy set off a `(@ NAME (def …))`
/// wrapper — the single source of truth for "which annotations carry compiler SEMANTICS today". The set
/// is the inline-policy pair (`inline-never`/`inline-always`), the test markers (`test`/`exhaustive`), and
/// the verification-contract predicates (`requires`/`ensures`). The contract pair is CALL-STYLE — the
/// annotation head is an APPLICATION `@requires(pred)`/`@ensures(pred)` carrying a predicate argument, not
/// a bare `NAME` like the others (`strip_annotations` matches the application head against this list). An
/// annotation whose name is NOT one of these is still UNWRAPPED (the def takes effect) but recorded
/// nowhere: a transparent, inert marker. So a future `@deprecated`/`@lint`/… works as a no-op the day it is
/// written and gains meaning by joining this list.
pub(crate) const KNOWN_ANNOTATIONS: &[&str] = &[
    "inline-never",
    "inline-always",
    "test",
    "exhaustive",
    "requires",
    "ensures",
    "invariant",
];
/// The strippable annotations a definition carries, each a set of the annotated defs' BODY occurrences.
pub(crate) struct StrippedAnnotations {
    pub(crate) inline_never: crate::fxhash::FxHashSet<StructId>,
    pub(crate) inline_always: crate::fxhash::FxHashSet<StructId>,
    /// Definitions marked `@test` — a UNIT TEST the `cdz test` build hoists into the test artifact as a
    /// nullary export (`property-based-testing.md` sibling: a plain assertion test). Recorded by BODY occ
    /// like the inline sets; `Db::is_test`/`test_defs` read it. Empty for an ordinary (non-test) build.
    pub(crate) tests: crate::fxhash::FxHashSet<StructId>,
    /// The `@tag("…")` string tags each annotated def carries, keyed by BODY occ (like `tests`). A def
    /// may carry several tags; they accumulate in annotation order. Read by `Db::tags_of` to back
    /// `cdz test --tag`. See the `Db::tags` field for the surface + semantics.
    pub(crate) tags: crate::fxhash::FxHashMap<StructId, Vec<String>>,
    /// Definitions marked `@exhaustive` — a property test the `cdz test` runner drives over its ENTIRE
    /// finite input domain (every combination of its scalar parameters) rather than by random sampling, so
    /// a pass is a proof over the domain (`property-based-testing.md` §Exhaustive). Recorded by BODY occ
    /// like `tests`; `Db::is_exhaustive` reads it. `@exhaustive` implies the def is a property test but is
    /// recorded independently of `@test` (the runner treats an `@exhaustive` def as a test too).
    pub(crate) exhaustive: crate::fxhash::FxHashSet<StructId>,
    /// `(@ (tag …) …)` annotation heads whose argument is not exactly one STRING — a malformed `@tag`
    /// (`@tag(5)`, `@tag(foo)`, `@tag()`, `@tag("a" "b")`). Recorded by the offending `(tag …)` occurrence
    /// so `collect_faults` rejects it (rather than silently dropping the tag and masking the author error).
    pub(crate) malformed_tags: Vec<StructId>,
    /// The `@requires(pred)` PRECONDITION predicates each annotated def carries, keyed by BODY occ (like
    /// `tags`). A def may stack several (`@requires(> x 0) @requires(< x 100)`); they accumulate in
    /// annotation order (a conjunction). The stored `StructId` is the PREDICATE-form occurrence — a
    /// Cadenza expression over the def's params — which the verification layer (Inc-b b4) DENOTES into a
    /// HOL obligation `Term`. This is the `@requires`/`@ensures`→node channel the proof-guided-elision
    /// oracle (`discharged_no_overflow`) needs: a checked-arith node's precondition is looked up here.
    /// (This b4a slice RECORDS the predicates; the denotation + discharge are later b4 slices. Recording
    /// them is behavior-neutral — nothing yet reads these sets, so an annotated def compiles exactly as
    /// today, just no longer as an inert unknown marker.)
    pub(crate) requires: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,
    /// The `@ensures(pred)` POSTCONDITION predicates each annotated def carries, keyed by BODY occ. The
    /// stored `StructId` is the predicate-form occurrence, a Cadenza expression over the def's params +
    /// the implicit result binder `it` (v-syntax's result-binding convention). Denoted + discharged in a
    /// later b4 slice; recorded here (behavior-neutral) as the surface half of the channel.
    pub(crate) ensures: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,
    /// The `@invariant(pred)` DATA-TYPE-INVARIANT predicate each annotated `(type …)` declaration carries,
    /// keyed by the TYPE decl occ (not a def BODY occ — `@invariant` annotates a type, not a def). The stored
    /// `StructId` is the predicate-form occurrence, a Cadenza expression over the value binder `it`. Recorded
    /// here (behavior-neutral) as the surface half of the data-level channel (design §10); consumed by
    /// `Db::invariant_of`.
    pub(crate) invariants: crate::fxhash::FxHashMap<StructId, StructId>,
    /// `(@ (requires …) …)` / `(@ (ensures …) …)` / `(@ (invariant …) …)` heads whose argument is not exactly
    /// one predicate form — a malformed `@requires`/`@ensures`/`@invariant` (`@requires()`, `@invariant(a b)`).
    /// Recorded by the offending occurrence so `collect_faults` REJECTS it (CDZ0201), the same arity discipline
    /// `malformed_tags` applies to `@tag` — a silently-unrecorded predicate would mask the author error (b4a2).
    pub(crate) malformed_verify: Vec<StructId>,
}

/// Unwrap EVERY `@`-ANNOTATION on a definition — `(@ inline-never (def …))`, `(@ inline-always (def …))`,
/// `(@ test (def …))`, and any `(@ <other-name> (def …))` — in place, returning the sets of the
/// annotated defs' BODY occurrences for the names in [`KNOWN_ANNOTATIONS`]
/// (`DESIGN-recursive-generic-monomorphization-rcdzc.md` Addendum 4). `@` is the GENERAL-PURPOSE
/// annotation head (`(@ NAME FORM)`, from the ML `@name form` sigil); a KNOWN name records its def into a
/// policy set, an UNKNOWN name is unwrapped just the same but recorded nowhere (a transparent no-op marker,
/// so a future `@deprecated`/… works the day it is written). An annotation is a declaration a build phase
/// consumes (the emitter's inline policy, the `cdz test` hoist) — not part of the def shape every reader
/// walks — so it is removed here BEFORE `scan_top_level`, exactly as `strip_def_docs`/`strip_const_params`
/// remove their wrappers, leaving every downstream reader a plain `(def …)`.
///
/// The annotation NODE is rewritten IN PLACE to BE the inner `(def SIG BODY)` (it adopts the def's
/// children), so its `StructId` now identifies the def and every parent that already pointed at the
/// annotation needs no update — the inner def node is left orphaned (harmless; unreferenced). The
/// recorded key is the def's BODY occurrence (`def` tail index 1 = child index 2), the identity
/// `lower`/`layout` key on (`def_index_by_body`). An UNKNOWN annotation name (or a non-name head) whose
/// inner IS a def is STILL unwrapped — recorded in no policy set, a transparent no-op — so the def takes
/// effect; only its (unmodeled) name is ignored. An annotation around a NON-def (or a malformed def) is
/// left untouched (a well-formedness concern elsewhere). A SINGLE annotation per def is the modeled case
/// (as with the original inline-policy pass); stacking two known annotations on one def is not relied on.
/// Peel every `(comment "…" <form>)` wrapper to its inner `<form>`, IN PLACE (the wrapper node adopts the
/// inner form's children, ids unchanged), so the compiler sees THROUGH a comment exactly as it sees
/// through a `(doc …)`. A comment is semantically inert (self-hosting-surface.md §the tree carries
/// comments and documentation — "each a node the compiler sees through"); a leading `//` on a form reifies
/// (by the reader) to `(comment "text" <form>)`, and without this pass the top-level scan reads `comment`
/// as an unknown declaration head → the wrapped `def`/`type`/`export` is invisible ("unbound name
/// `comment`" + the def's name unbound). Stacked comments NEST — `// a` then `// b` on one form is
/// `(comment "a" (comment "b" <form>))` — so each wrapper is peeled to the INNERMOST non-comment form (the
/// intervening comment nodes are then orphaned, harmless like `strip_annotations`'s unwrapped inner). A
/// `(comment …)` of the wrong arity (not exactly `<string> <form>`) is left untouched (a malformed node a
/// later pass handles). Runs FIRST in `load_linked`, before the doc/const/annotation strips + the scan.
///
/// The `(comment "text" <form>)` this pass peels IS a comment carried as a NODE of the AST — the reader
/// reifies a `//` comment into it, wrapping (attached to) the form it annotates, and it is an ordinary
/// `Struct::List` so it lives in the stored BINARY form (the codec's generic encode/decode round-trips it
/// unchanged, exactly as it does the `(doc …)` node), not only in a textual rendering. That this pass can
/// find and peel a `(comment …)` at load is the proof the tree carries it.
///
/// `pub(crate)` so `compile.rs::link_inputs` can peel each package file's arena BEFORE `link` scans its
/// imports/exports (the link scan runs before `Db::load`'s own call to this, so a comment on an
/// `(import …)` would otherwise leave it wrapped + unrecognized).
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# The abstract syntax tree MUST be able to carry a comment as a node of the tree, attached to the node it annotates, so that a comment is preserved in the stored binary form rather than only in a textual rendering.
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# A comment or documentation carried by the tree MUST survive encoding and decoding unchanged.
// Peeling the wrapper at LOAD, before resolve/type/lower ever see the form, is what makes a comment
// SEMANTICALLY INERT: the compiler sees through it to the inner form, so a `(comment …)` changes neither
// the runtime meaning of a program nor the type its expressions are assigned — it is pure annotation.
//= spec/capabilities/agent-authoring.md#comments-are-semantically-inert
//# A comment MUST NOT change the runtime meaning of a program.
//= spec/capabilities/agent-authoring.md#comments-are-semantically-inert
//# A comment MUST NOT change the type a program's expressions are assigned.
pub(crate) fn strip_comments(ast: &mut Arenas) {
    // A reader-produced comment node the compiler peels through: a LEADING `(comment "text" form)` (a
    // `//`/`///` on its own line above `form`) OR a TRAILING `(comment-after "text" form)` (a `//` that
    // followed `form` on the same source line, e.g. `| Ctor(T) // note`). Both have the identical
    // `[<string>, <form>]` tail, so both peel to `form` by the same rule — the compiler never sees either.
    let comment_form = |ast: &Arenas, id: StructId| -> Option<StructId> {
        let tail = ast
            .as_form(id, "comment")
            .or_else(|| ast.as_form(id, "comment-after"))?;
        let (&text, &form) = (tail.first()?, tail.get(1)?);
        // The first tail element must be a STRING (the comment text); else it's not a reader comment node.
        matches!(ast.get(text), Struct::Atom(l) if matches!(ast.leaf(*l), Leaf::Str(_)))
            .then_some(form)
    };
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if comment_form(ast, id).is_none() {
            continue;
        }
        // Follow the chain of nested comment nodes (leading and/or trailing, in any mix) down to the
        // first NON-comment form. A malformed comment stops the descent (left for a well-formedness pass).
        let mut inner = id;
        while let Some(form) = comment_form(ast, inner) {
            inner = form;
        }
        // `inner` is the innermost non-comment form (or `id` itself if the chain was malformed at the top —
        // then this is a no-op copy). Rewrite the OUTER comment node to BE that form (adopt its children),
        // so its `StructId` now identifies the form and every parent pointing at the comment needs no update.
        if inner != id {
            let entry = ast.get(inner).clone();
            ast.structure[i] = entry;
        }
    }
}

pub(crate) fn strip_annotations(ast: &mut Arenas) -> StrippedAnnotations {
    let mut inline_never: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut inline_always: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut tests: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut tags: crate::fxhash::FxHashMap<StructId, Vec<String>> =
        crate::fxhash::FxHashMap::default();
    let mut exhaustive: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    // `(@ (tag …) …)` heads whose argument is not exactly one STRING — malformed tag annotations to reject.
    let mut malformed_tags: Vec<StructId> = Vec::new();
    let mut requires: crate::fxhash::FxHashMap<StructId, Vec<StructId>> =
        crate::fxhash::FxHashMap::default();
    let mut ensures: crate::fxhash::FxHashMap<StructId, Vec<StructId>> =
        crate::fxhash::FxHashMap::default();
    let mut invariants: crate::fxhash::FxHashMap<StructId, StructId> =
        crate::fxhash::FxHashMap::default();
    let mut malformed_verify: Vec<StructId> = Vec::new();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        // `(@ NAME INNER)` — the annotation head `@`, its name, and the annotated form. Only a known
        // annotation name is consumed here; every other `@` annotation is left in place.
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        // The annotation NAME, if it is a bare name at all. A KNOWN name (in `KNOWN_ANNOTATIONS`) records
        // its def into a policy set below; an UNKNOWN name is TRANSPARENT — the wrapper is still unwrapped
        // to the inner def (so the def registers and resolves), the name simply ignored. This is the
        // `@`-sigil's advertised extensibility (`7221f7bc`: "future annotations `@deprecated`, `@test` layer
        // in with no new lexer/parser/resolver rules" / "leaves other annotations in place") — "in place"
        // means the annotated FORM still takes effect, NOT that the `(@ …)` node survives to resolve (where
        // `@` has no declaration arm → the whole def would be DROPPED with a misleading "unbound name `@`"
        // plus a phantom unbound-name for the def it wrapped). A future annotation that needs real semantics
        // adds its name to `KNOWN_ANNOTATIONS` + a set here; until then it is an inert, unwrapped marker.
        let name = ast.as_name(name_occ).map(str::to_string);
        // A CALL-STYLE annotation carries an ARGUMENT: `@tag("slow")` reifies to `(@ (tag "slow") def)`,
        // so the NAME position is the APPLICATION `(tag "slow")` — a form, not a bare name (`as_name`
        // above is `None`). Read it as `(HEAD ARG…)`: `@tag("…")` is the modeled call-style annotation,
        // its head `tag` + a single STRING argument the tag text. A def may stack several
        // (`@tag("slow") @tag("net")`), each recorded below. Any other application-name annotation is an
        // inert unknown marker (unwrapped, recorded nowhere), exactly like an unknown bare name.
        let tag_app = ast.as_form(name_occ, "tag");
        let tag_arg: Option<String> = tag_app.and_then(|app_tail| {
            match app_tail {
                [only] => ast.as_str(*only).map(str::to_string),
                _ => None, // `@tag` with not-exactly-one-arg (or a non-string arg) — not a modeled tag
            }
        });
        // `@requires(pred)` / `@ensures(pred)` reify like `@tag`, but their ARGUMENT is a PREDICATE FORM
        // (a Cadenza expression over the def's params, and — for `@ensures` — the result binder `it`), not
        // a string. `@requires(> x 0)` → `(@ (requires (> x 0)) def)`, so the name position is the
        // application `(requires (> x 0))` and its single tail element is the predicate occurrence. Record
        // that `StructId`; the verification layer denotes it into a HOL obligation (a later b4 slice).
        //
        // ARITY validation (b4a2): a `@requires`/`@ensures` with NOT-exactly-one argument (`@requires()`,
        // `@requires(a b)`) is a pure SHAPE error `strip_annotations` can see — exactly like `@tag`
        // validates one-string. Record the offending occurrence in `malformed_verify` so `collect_faults`
        // REJECTS it (CDZ0201), rather than silently recording no predicate (which would mask the author
        // error, exactly the `@tag` masking bug). NAME-resolution / boolean-typedness of the predicate is
        // NOT checked here — that needs the def's param scope + the `it` binder, so it is deferred to the
        // denotation pass (b4c), which reports an unbound name at the annotation span.
        let requires_app = ast.as_form(name_occ, "requires");
        let ensures_app = ast.as_form(name_occ, "ensures");
        let requires_pred: Option<StructId> = requires_app.and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        let ensures_pred: Option<StructId> = ensures_app.and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        // A `(requires …)` / `(ensures …)` head with not-exactly-one arg → malformed (recorded below,
        // once we know the inner is a def, mirroring the `@tag` malformed discipline).
        let malformed_verify_here = (requires_app.is_some() && requires_pred.is_none())
            || (ensures_app.is_some() && ensures_pred.is_none());
        // `@invariant(pred)` — the DATA-level family member (design §10). UNLIKE `@requires`/`@ensures`, it
        // annotates a `(type …)` DECLARATION, not a `(def …)`. So it must be handled HERE, before the
        // def-only path below `continue`s on a non-def inner (which would otherwise route a `@invariant (type
        // …)` to the "annotation wraps no definition" CDZ0201). `@invariant(> (len it) 0)` reifies to
        // `(@ (invariant (> (len it) 0)) (type …))`, so the name position is the application `(invariant …)`
        // with one predicate tail element (over the value binder `it`). Record it keyed by the TYPE decl occ,
        // then UNWRAP the wrapper to the type decl (adopt its children) so the type still takes effect. Arity
        // is validated like `@requires`/`@ensures` (b4a2): not-exactly-one arg → `malformed_verify` (CDZ0201).
        let invariant_app = ast.as_form(name_occ, "invariant");
        if let Some(app_tail) = invariant_app {
            // Only meaningful when the inner is a `(type …)` declaration (the `@invariant` annotand). A
            // `@invariant` on a NON-type is left to the "wraps no definition" path below (not a modeled shape).
            if ast.as_form(inner, "type").is_some() {
                match app_tail {
                    [only] => {
                        // Key by `id` (the WRAPPER slot) — NOT `inner`. The unwrap below overwrites slot `id`
                        // with the type's children, so post-strip the `(type …)` DECLARATION lives at `id`,
                        // and `scan_top_level` computes `TypeDecl::occ = id`. Keying by `inner` would miss
                        // (`invariant_of(TypeDecl::occ)` looks up `id`).
                        invariants.insert(id, *only);
                    }
                    // not-exactly-one predicate arg → malformed (arity), rejected in `collect_faults`.
                    _ => malformed_verify.push(name_occ),
                }
                // Unwrap the wrapper to BE the inner `(type …)` decl (adopt its full child list), so the type
                // declaration takes effect exactly as an un-annotated `(type …)` would — the `@invariant`
                // wrapper is consumed here (recorded above), never surfacing as a top-level annotation form.
                if let Struct::List(inner_children) = ast.get(inner).clone() {
                    ast.structure[i] = Struct::List(inner_children);
                }
                continue;
            }
        }
        // The inner must be a `(def SIG BODY …)` — read its children to adopt them + find the BODY occ.
        // NOTE: this def check is BEFORE the malformed-`@tag` recording below on purpose — the `@tag`
        // contract only applies when the annotation wraps a DEFINITION, so a `@tag` around a NON-def is
        // handled solely by the existing "annotation wraps no definition" rejection; recording a
        // malformed-tag fault here too would double-diagnose one mistake (Copilot PR#484).
        let Some(def_tail) = ast.as_form(inner, "def") else {
            continue; // an annotation around a non-def — leave untouched (a well-formedness concern elsewhere)
        };
        // A `(tag …)` HEAD that is NOT a valid `@tag("string")` — the arg is not exactly one STRING (a
        // number `@tag(5)`, a bare name `@tag(foo)`, zero args `@tag()`, or two `@tag("a" "b")`). This is
        // always an author mistake: silently ignoring it (recording no tag) masks the error, so record the
        // offending `(tag …)` occurrence for `collect_faults` to REJECT (a malformed tag annotation). Only
        // meaningful once the inner IS a def (checked above) — a `@tag` on a non-def is already the
        // "wraps no definition" mistake, not additionally a malformed-tag one.
        if tag_app.is_some() && tag_arg.is_none() {
            malformed_tags.push(name_occ);
        }
        // A `@requires`/`@ensures` with not-exactly-one arg — record for rejection (b4a2), same discipline
        // as the malformed `@tag` above: only once the inner IS a def (a `@requires` on a non-def is the
        // "wraps no definition" mistake). Silently recording no predicate would mask the author error.
        if malformed_verify_here {
            malformed_verify.push(name_occ);
        }
        // The def's BODY occurrence: `def_tail = [SIG, BODY, …]`, so index 1 (a well-formed def has ≥2).
        let Some(&body) = def_tail.get(1) else {
            continue;
        };
        // Rewrite the WRAPPER node to BE the inner def (adopt its full child list `[def-head, SIG, BODY…]`).
        let Struct::List(inner_children) = ast.get(inner).clone() else {
            continue;
        };
        ast.structure[i] = Struct::List(inner_children);
        // The match arms below record exactly the SEMANTIC annotations; `KNOWN_ANNOTATIONS` is the
        // published catalog of those names. Assert the two agree, so adding a name to one without the other
        // trips in debug (a known name that falls to the transparent `_ => {}` would silently lose its
        // policy). An unknown name is intentionally not in the catalog → the `_` arm's inert unwrap.
        debug_assert!(
            name.as_deref()
                .is_none_or(|n| KNOWN_ANNOTATIONS.contains(&n)
                    == matches!(
                        n,
                        "inline-never"
                            | "inline-always"
                            | "test"
                            | "exhaustive"
                            | "requires"
                            | "ensures"
                            // `invariant` is call-style (`(invariant pred)`) so its `name` read is `None` and
                            // it is handled + `continue`d above, never reaching here as a bare name; listed
                            // for catalog parity with `KNOWN_ANNOTATIONS` (like `requires`/`ensures`).
                            | "invariant"
                    )),
            "KNOWN_ANNOTATIONS and the strip_annotations match arms disagree on `{name:?}`"
        );
        match name.as_deref() {
            Some("inline-never") => {
                inline_never.insert(body);
            }
            Some("inline-always") => {
                inline_always.insert(body);
            }
            Some("test") => {
                tests.insert(body);
            }
            // `@exhaustive` marks a property test to drive over its ENTIRE finite domain rather than by
            // random sampling. It also makes the def a test (an `@exhaustive` def need not ALSO be
            // `@test`-marked — recording it in `tests` too means `test_defs` hoists it), so record BOTH.
            Some("exhaustive") => {
                exhaustive.insert(body);
                tests.insert(body);
            }
            // An unknown annotation name (or a non-name annotation head): unwrapped above, recorded in no
            // policy set — a transparent no-op marker so the wrapped def still takes effect. A call-style
            // `@tag("…")` lands here (its name is an application, not a bare name) and is recorded below.
            _ => {}
        }
        // A `@tag("…")` records its string against the def's BODY occ, accumulating across stacked tags
        // (`@tag("slow") @tag("net")` → both). Independent of `@test` — a tag is metadata; the `cdz test
        // --tag` filter reads it via `Db::tags_of`. Recorded AFTER the unwrap so `body` is the def's occ.
        if let Some(tag) = tag_arg {
            tags.entry(body).or_default().push(tag);
        }
        // `@requires(pred)`/`@ensures(pred)` record their predicate occ against the def's BODY occ,
        // accumulating across stacked annotations (a conjunction). The name position is the `(requires …)`
        // / `(ensures …)` application, so `name` (the bare-name read) is `None` and these fall to the `_`
        // arm above — recorded here, like `@tag`, after the unwrap so `body` is the def's occ.
        if let Some(pred) = requires_pred {
            requires.entry(body).or_default().push(pred);
        }
        if let Some(pred) = ensures_pred {
            ensures.entry(body).or_default().push(pred);
        }
    }
    StrippedAnnotations {
        inline_never,
        inline_always,
        tests,
        tags,
        exhaustive,
        malformed_tags,
        requires,
        ensures,
        invariants,
        malformed_verify,
    }
}
