//! `layout` — the query that computes the program's boundary surface, target-neutrally.
//!
//! Above the backend seam sits the computation of *what the program presents at its boundary*: which
//! definitions are exported, under what (verbatim) names, with what solved parameter and result
//! types, and which definitions are reachable — computed ONCE and consumed by whichever backend runs
//! (`backends-and-targets.md` §The Boundary Layout Is Computed Once, Target-Neutrally, And Reused).
//!
//! The interface is read from each export's DECLARED signature, never inferred from a body
//! (`reference-compiler.md` §The Exported Interface Is The Declared Signature). An export's signature
//! is its solved parameter types (each `(name-occurrence, type)`) and result type, obtained by
//! demanding `type_of`/`def_scheme` (a lazy read of the type column). The export NAME crosses
//! verbatim; no export is recognized by name or body shape. So the boundary calling convention is
//! fixed by the declared signature alone, not by any compiler internal:
//!
//= spec/contracts/component-abi.md#lowering-and-lifting-are-fixed-inverses
//# The calling convention across the boundary MUST be a function of the declared signature alone, independent of compiler internals.
//!
//! The exported entries come from the program's `(export …)` clauses — the compiler reads which
//! definitions are exported from the source, so the entry the runtime invokes is determined by the
//! program rather than left implicit:
//!
//= spec/contracts/component-abi.md#a-component-exports-a-defined-entry
//# A derived component MUST export an entry through which the runtime invokes it.
//!
//= spec/contracts/component-abi.md#a-component-exports-a-defined-entry
//# The compiler MUST determine a program's entry from the program rather than leave it implicit.
//!
//! Reachability lives here: an export drives which definitions are reached, and only reachable
//! definitions are emitted (dead code dropped once, target-neutrally). A runtime `Core::Call` reaches
//! its callee, so the reachable set is grown by a worklist over each reachable body's calls (not just
//! the exports — a recursive or helper def a call names is emitted too). This is the place that set,
//! the emission order, and each function's absolute index are fixed for every backend.

use crate::ast::StructId;
use crate::db::Db;
use crate::diag::Reject;
use crate::infer::type_of;
use crate::ty::Ty;
use tracing::trace;

/// One exported entry, resolved to a target-neutral boundary plan.
#[derive(Clone, PartialEq, Debug)]
pub struct ExportPlan {
    /// The name the entry crosses the boundary under — verbatim from the source.
    pub name: String,
    /// The definition (index into `db.defs`) this export names.
    pub def: usize,
    /// The AST occurrence of the definition body — the root the backend walks the core from.
    pub body: StructId,
    /// The parameters, in signature order — each the `(name-occurrence, solved-type)` the backend
    /// needs to assign a local slot and a boundary valtype (`select_function`). Empty for a nullary
    /// export. The name occurrence is what a body reference to the parameter binds to (seen through a
    /// `(: a T)` annotated binder), so it is the slot-map key.
    pub params: Vec<(StructId, Ty)>,
    /// The solved result type the entry returns.
    pub result: Ty,
}

/// The whole boundary layout: the exported entries (declaration order), the definitions reachable
/// from them in emission order, and each reachable definition's absolute wasm-function index.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    pub exports: Vec<ExportPlan>,
    /// Reachable definition indices in emission order (exported first, declaration order; then the
    /// rest). A body emitted at position `k` is DEFINED wasm func `import_base + k` — runtime imports
    /// occupy the function index space `0..import_base` ahead of every defined function.
    pub order: Vec<usize>,
    /// The number of runtime-op imports the program declares — the offset added to a defined
    /// function's emission position to get its absolute wasm function index. `0` for a program that
    /// imports nothing (a scalar program), which is then byte-identical to a runtime-free build.
    pub import_base: u32,
    /// `def → its position in `order``, the O(1) inverse of the `order` sequence. `abs` (called once
    /// per `Core::Call` during selection AND per export during serialization) needs a def's emission
    /// position; without this map it did an O(len) `order.position()` scan, making a call-heavy or
    /// export-heavy program O(N²) in the backend. Built once alongside `order` by [`Layout::new`], so
    /// it cannot drift; the backend's `import_base` reshuffle preserves it via [`Layout::with_import_base`].
    order_pos: std::collections::HashMap<usize, usize>,
    /// `def → its index in `exports``, for the def→ExportPlan lookup the emit loop does once per
    /// emitted function (an export's params come from its plan). Without it that was an O(exports)
    /// `exports.iter().find()` per func — O(N²) on a many-export program. `None`-absent means the def
    /// is not an export (an internal reachable callee), which reads its params via `def_params`.
    export_of_def: std::collections::HashMap<usize, usize>,
    /// The LAMBDA-LIFTED closures, in funcref-table-slot order (a copy of `db.lifted` at layout time).
    /// Each lifted lambda is emitted as a standalone wasm function AFTER the `order` def functions, so
    /// its wasm function index is `import_base + order.len() + its slot`. The funcref table's element `k`
    /// points at lifted lambda `k`'s function, so a `Core::Closure { code: k }` stored slot selects it
    /// through `call_indirect`. Empty for a program with no runtime closure (byte-identical to before).
    pub lifted: Vec<crate::lower::LiftedLambda>,
    /// Parallel to `lifted`: whether slot `k` is REACHED by a `Core::Closure` in an emitted body. An
    /// UNREACHED slot (a lambda demanded during type-checking / a fold that erased it) is emitted as an
    /// inert STUB and gets NO funcref-table element (never called), so a dead lift is neither unsound nor
    /// referenced. `true` for every slot of a program whose closures are all live.
    pub lifted_reached: Vec<bool>,
    /// The HOST-import order (E2h-2): `(effect, op)` name pairs, one per host-delegated operation the
    /// program performs, in the order the backend lays them in the core module's import section. A
    /// `Core::HostCall` resolves its `(effect, op)` to a core-func index by its position here. Empty for a
    /// program that delegates no effect (byte-identical to before). Target-agnostic (plain names), so it
    /// lives on the layout the backend fills — set by the backend's `with_host_order` once the set is
    /// collected, mirroring how `import_base` is fixed once the runtime-op set is known.
    pub host_order: Vec<(String, String)>,
    /// The STRING constants a host call passes as a `string` argument (E2h-string), each with its BYTE
    /// OFFSET in the program core module's data segment. A `Core::HostCall` with a string arg emits
    /// `(ptr=offset, len)` (the `(ptr,len)` the canonical ABI reads the string from). Assigned by the
    /// backend before selection (each distinct string laid once, at a running offset); empty when no host
    /// call passes a string (then the core module needs no memory/data — byte-identical to the scalar
    /// host shape). Target-agnostic (plain strings + offsets), so it lives on the layout.
    pub host_strings: Vec<(String, u32)>,
    /// Whether the program core module must import the shared `mem` because a host op takes/needs linear
    /// memory EVEN WITH NO const string args — a host op with a `string` PARAMETER passed a RUNTIME string
    /// (the `_mem` runtime-arg path): the guest marshals the runtime rope's bytes into `mem` via a copy
    /// loop, so it needs `mem` imported even though `host_strings` (the CONST-string data segment) is empty.
    /// Set by the backend from `host::set_needs_memory(host_imports)`. OR'd with `!host_strings.is_empty()`
    /// to gate the core module's `mem` import; false for a program with no String-param host op (byte-
    /// identical to before). Target-agnostic (a plain flag), so it lives on the layout.
    pub host_needs_memory: bool,
    /// The CROSS-COMPONENT extern-import order (X4b): `(interface, op)` name pairs, one per peer operation
    /// a `Core::ExternCall` names, in the order the backend lays them in the core module's import section
    /// (from module `"peer"`). A `Core::ExternCall` resolves its `(interface, op)` to a core-func index by
    /// its position here — the exact `host_order` analogue. Laid AFTER the host + runtime imports so an
    /// extern call's core-func index is `host + runtime + its position`. Empty for a program binding no
    /// peer (byte-identical to before). Target-agnostic, filled by the backend's `with_extern_order`.
    pub extern_order: Vec<(String, String)>,
    /// EXTRA closure-application functypes a `call_indirect` needs but NO lifted lambda supplies — each a
    /// `(env-prefixed param valtypes, result valtype-or-unit)` shape. A `Core::CallClosure` reaches a
    /// closure through a sum/param whose lifted body is NOT built in this program: the applied variant is
    /// statically reachable (the `match` arm is emitted) but dynamically dead (no `Core::Closure` of that
    /// type is ever constructed, so no lifted lambda of that shape exists). The `call_indirect` still needs
    /// a TYPE-SECTION functype of the right structural shape to reference. These are collected in `compute`
    /// from the reachable `Core::CallClosure` signatures that no reached lifted lambda covers, laid in the
    /// type section AFTER the lifted functypes (so no existing index shifts), and resolved by
    /// `closure_call_type_index`. Empty for a program whose every applied closure has a built lifted body
    /// (byte-identical to before). Each entry is `(param valtypes INCLUDING the leading i32 env, ret)`.
    pub closure_call_types: Vec<(Vec<crate::backend::wasm::lir::ValType>, Ty)>,
    /// OPTION C (consumer emit): `def index → its position in `extern_order`` for each CROSS-EDGE the
    /// consumer imports from the shared-closure provider component. A `Core::Call` whose callee is in this
    /// map is NOT a local emitted func (it was excluded from `order` by [`compute_tests_consumer`]) — select
    /// emits a `Lir::CallExternImport(pos)` to the imported interface func instead of a `Lir::Call`. EMPTY
    /// for every non-consumer layout (`compute`/`compute_tests`/`compute_shared_closure_provider`), so the
    /// select branch never fires there — byte-identical to before. The `extern_order` entries these positions
    /// index into are `(closure-interface, source_boundary_name(def))`, in CANONICAL `order`-derived
    /// cross-edge order so the consumer's import index MATCHES the provider's export index (the
    /// index-agreement invariant that keeps the composed module valid).
    pub cross_edge_import: std::collections::HashMap<usize, usize>,
    /// CONTENT-ADDRESSED SPEC DEDUP: `merged_spec_def → representative_def` for every recursive-effectful
    /// spec collapsed into a structurally-identical representative by [`effect_spec_merge_map`]. The wasm
    /// backend redirects a merged spec's func-index via `order_pos` (see `add_merged_spec_redirects`), but
    /// the RUST backend resolves a `Core::Call` callee BY NAME (`fn_ident`), so a merged spec — dropped from
    /// `order`, never emitted — would be a dangling by-name call (rustc E0425) unless `fn_ident` also
    /// canonicalizes the callee to its representative. This map is that canonicalization source, consulted by
    /// `fn_ident`. Empty for a program with no redundant specializations (byte-identical to before) — the
    /// common case. This is the fix for the revert of the layout-congruence dedup: the redirect must cover
    /// BOTH the wasm func-index path (order_pos) AND the rust by-name path (fn_ident).
    pub spec_merge: std::collections::HashMap<usize, usize>,
    /// STATIC BYTES (`DESIGN-static-data.md` §2d): the DISTINCT fully-constant `Bytes` payloads the
    /// program builds ONCE — each materialized into a module GLOBAL by a `start` init function and read
    /// with `global.get` (+ a dup) at every use, instead of re-`bytes-alloc`+`bytes-set`-ing per
    /// evaluation. The index of a payload here IS its module global index (globals `0..static_bytes.len()`;
    /// a defined `cabi_realloc` cursor, when present, follows at `static_bytes.len()`). Filled by the
    /// backend before selection (`collect_static_bytes`), so selection's `Core::BytesOf` arm can route a
    /// constant literal to its global and `core_module_impl` can emit the GLOBAL + START sections. Empty
    /// for a program with no constant bytes literal → no GLOBAL/START additions, byte-identical to before.
    pub static_bytes: Vec<Vec<u8>>,
    /// STATIC COMPOUNDS (`DESIGN-static-data.md` §2d, increment 6): the markable constant `Tuple`/`Record`
    /// ROOT node ids the program builds ONCE (`collect_static_compounds`). Each occupies a module GLOBAL laid
    /// AFTER the `static_bytes` globals — so compound `k`'s global index is `static_bytes.len() + k`. The
    /// `Core::Tuple`/`Core::Record` emit arm routes a node whose id is here to that global; the `start` init
    /// builds each once (immortal). Keyed by node id: two uses of the same node share one global. Empty →
    /// no compound globals, byte-identical.
    pub static_compounds: Vec<StructId>,
    /// The PRECOMPUTED `start`-init body for the static compounds — the flat `Lir` that, for each entry in
    /// `static_compounds`, builds its immortal tree (`select::emit_immortal_static`) and `global.set`s it to
    /// `static_bytes.len() + k`. Built with `Db` access in the backend (the tree walk needs `core_of`/
    /// `type_of`/box selection), then handed to `core_module_impl` (which has no `Db`) to APPEND to the
    /// static-bytes init in the START function. Parallel to `static_compounds`; empty when there are none.
    pub static_compound_init: Vec<crate::backend::wasm::lir::Lir>,
    /// The number of i32 SCRATCH LOCALS the `static_compound_init` body uses — the START function that runs
    /// it must DECLARE this many locals (else a `local.get`/`local.set` in the init is out-of-bounds =
    /// invalid wasm). Zero unless a hoisted Map/Set has a LIST key/element: `emit_key_canonicalize` stashes
    /// the raw key + descriptor in two i32 scratch locals (the only immortal-build op that uses locals — the
    /// tuple/record/list/sum/scalar builds are all stack-threaded). Derived in `with_static_compounds` by
    /// scanning the init for the max `Local{Get,Set,Tee}` index (+1); all such scratch is i32 (handles).
    pub static_compound_init_locals: u32,
}

impl Layout {
    /// Assemble a `Layout` from its emission plan, deriving the two O(1) inverse indices (`order_pos`,
    /// `export_of_def`) so they can never drift from `order`/`exports`. The one way to build a `Layout`
    /// — `compute` and the backend's `import_base` reshuffle both go through it — so the indices are a
    /// maintained invariant, not a field a caller could forget or set inconsistently.
    pub fn new(exports: Vec<ExportPlan>, order: Vec<usize>, import_base: u32) -> Layout {
        Layout::with_lifted(exports, order, import_base, Vec::new(), Vec::new())
    }

    /// [`Layout::new`] plus the lambda-lifted closures (in table-slot order) + a parallel `reached` flag
    /// per slot — the emission plan when a program has runtime closures. The lifted functions emit after
    /// the `order` defs, so their wasm indices are `import_base + order.len() + slot`.
    pub fn with_lifted(
        exports: Vec<ExportPlan>,
        order: Vec<usize>,
        import_base: u32,
        lifted: Vec<crate::lower::LiftedLambda>,
        lifted_reached: Vec<bool>,
    ) -> Layout {
        let order_pos = order.iter().enumerate().map(|(k, &d)| (d, k)).collect();
        let export_of_def = exports
            .iter()
            .enumerate()
            .map(|(i, e)| (e.def, i))
            .collect();
        Layout {
            exports,
            order,
            import_base,
            order_pos,
            export_of_def,
            lifted,
            lifted_reached,
            host_order: Vec::new(),
            host_strings: Vec::new(),
            host_needs_memory: false,
            extern_order: Vec::new(),
            closure_call_types: Vec::new(),
            cross_edge_import: std::collections::HashMap::new(),
            spec_merge: std::collections::HashMap::new(),
            static_bytes: Vec::new(),
            static_compounds: Vec::new(),
            static_compound_init: Vec::new(),
            static_compound_init_locals: 0,
        }
    }

    /// A copy of this layout with the Option-C cross-edge import map + the extern_order entries it indexes
    /// set — the consumer layout ([`compute_tests_consumer`]) records each cross-edge def's `(closure-iface,
    /// source_boundary_name(def))` extern-import (in canonical `order` cross-edge order) + the `def → its
    /// extern_order position` map select reads. The `interface` is the shared-closure provider's published
    /// interface name (the provider↔consumer contract). Extends (does not replace) any existing extern_order.
    pub fn with_cross_edge_imports(
        &self,
        extern_order_additions: Vec<(String, String)>,
        cross_edge_import: std::collections::HashMap<usize, usize>,
    ) -> Layout {
        let mut extern_order = self.extern_order.clone();
        extern_order.extend(extern_order_additions);
        Layout {
            extern_order,
            cross_edge_import,
            ..self.clone()
        }
    }

    /// A copy of this layout with every `cross_edge_import` position SHIFTED UP by `delta` — the count of
    /// extern imports laid down AHEAD of the cross-edge block. `compute_tests_consumer` computes the cross-edge
    /// positions 0-based (the consumer layout carries no other extern imports at layout time). But the backend
    /// emit may prepend OTHER extern imports before the cross-edge block — specifically a PEER-BOUND escaping
    /// EFFECT (`db.effect_bindings`) is moved into `extern_imports` FIRST, so the cross-edges land at
    /// `delta..delta+M` in the final `extern_order`, not `0..M`. A `Lir::CallExternImport(pos)` resolves against
    /// that final order, so `cross_edge_import` must name the FINAL position. The backend applies this shift by
    /// the peer-extern count once `extern_imports` is assembled (`delta = 0` → no-op, byte-identical to a
    /// consumer with no coexisting peer-bound effect). Without it, a consumer that BOTH imports the shared
    /// closure AND binds a peer effect emits every cross-edge call off by `delta` → wrong import / invalid
    /// module (the index-agreement invariant, extended to a mixed extern-import set).
    pub fn with_cross_edge_import_shift(&self, delta: usize) -> Layout {
        if delta == 0 {
            return self.clone();
        }
        let cross_edge_import = self
            .cross_edge_import
            .iter()
            .map(|(&d, &p)| (d, p + delta))
            .collect();
        Layout {
            cross_edge_import,
            ..self.clone()
        }
    }

    /// A copy of this layout with `host_needs_memory` set — the backend sets it from
    /// `host::set_needs_memory` so the core module imports `mem` for a runtime String host-arg even with no
    /// const-string data segment (see the field doc).
    pub fn with_host_needs_memory(&self, host_needs_memory: bool) -> Layout {
        Layout {
            host_needs_memory,
            ..self.clone()
        }
    }

    /// A copy of this layout with the EXTRA closure-application functypes set — the `call_indirect`
    /// signatures no lifted lambda supplies (see the field doc). Set in `compute` after the lifted set +
    /// the reachable `Core::CallClosure` signatures are both known.
    pub fn with_closure_call_types(
        &self,
        closure_call_types: Vec<(Vec<crate::backend::wasm::lir::ValType>, Ty)>,
    ) -> Layout {
        Layout {
            closure_call_types,
            ..self.clone()
        }
    }

    /// The TYPE-section index of the EXTRA closure-application functype at position `i` in
    /// `closure_call_types` — laid AFTER the imports, the `order` defs, AND the lifted lambdas, so its
    /// index is `import_count + order.len() + lifted.len() + i`. A `call_indirect` applying a closure whose
    /// shape no lifted lambda supplies references this. (Structural functypes: the index only needs the
    /// matching `(env, param…)->result` shape.)
    pub fn closure_call_type_index(&self, i: usize, import_count: u32) -> u32 {
        import_count + (self.order.len() + self.lifted.len() + i) as u32
    }

    /// A copy of this layout with a different `import_base` (the backend shifts the base once the
    /// runtime-import count is known). The inverse indices are unchanged by the shift, so they carry
    /// over without a rebuild.
    pub fn with_import_base(&self, import_base: u32) -> Layout {
        Layout {
            import_base,
            ..self.clone()
        }
    }

    /// A copy of this layout with the HOST-import order set (E2h-2) — the `(effect, op)` name pairs a
    /// `Core::HostCall` resolves its call index against. Set by the backend once it has collected the
    /// program's host-import set (like `with_import_base` for the runtime-op count).
    pub fn with_host_order(&self, host_order: Vec<(String, String)>) -> Layout {
        Layout {
            host_order,
            ..self.clone()
        }
    }

    /// A copy of this layout with the host-call STRING constants (+ their data-segment offsets) set
    /// (E2h-string). Set by the backend after it lays out the host-arg strings.
    pub fn with_host_strings(&self, host_strings: Vec<(String, u32)>) -> Layout {
        Layout {
            host_strings,
            ..self.clone()
        }
    }

    /// A copy of this layout with the STATIC BYTES table set (`DESIGN-static-data.md` §2d) — the distinct
    /// fully-constant `Bytes` payloads the program builds once (`collect_static_bytes`). Set by the backend
    /// before selection, so the `Core::BytesOf` arm can route a constant literal to its global.
    pub fn with_static_bytes(&self, static_bytes: Vec<Vec<u8>>) -> Layout {
        Layout {
            static_bytes,
            ..self.clone()
        }
    }

    /// A copy of this layout with the STATIC COMPOUNDS table + its precomputed `start`-init body set
    /// (`DESIGN-static-data.md` §2d, increment 6). `compounds` are the markable constant Tuple/Record root
    /// node ids (`collect_static_compounds`); `init` is the flat `Lir` (`select::build_static_compound_init`)
    /// that builds each immortal + `global.set`s it. Set by the backend before selection so the
    /// `Core::Tuple`/`Core::Record` arm can route a constant literal to its global.
    pub fn with_static_compounds(
        &self,
        static_compounds: Vec<StructId>,
        static_compound_init: Vec<crate::backend::wasm::lir::Lir>,
    ) -> Layout {
        use crate::backend::wasm::lir::Lir;
        // The START function that runs this init must declare enough scratch locals to cover every
        // `local.get`/`local.set`/`local.tee` the init references — 1 + the max index used (0 if none). A
        // hoisted Map/Set with a LIST key is the only shape that emits any (two i32 canonicalize slots).
        let static_compound_init_locals = static_compound_init
            .iter()
            .filter_map(|op| match op {
                Lir::LocalGet(i) | Lir::LocalSet(i) | Lir::LocalTee(i) => Some(*i),
                _ => None,
            })
            .max()
            .map_or(0, |m| m + 1);
        Layout {
            static_compounds,
            static_compound_init,
            static_compound_init_locals,
            ..self.clone()
        }
    }

    /// A copy of this layout with the CROSS-COMPONENT extern-import order set (X4b) — the `(interface,
    /// op)` name pairs a `Core::ExternCall` resolves its call index against. Set by the backend once it
    /// has collected the program's extern-import set (the `with_host_order` analogue).
    pub fn with_extern_order(&self, extern_order: Vec<(String, String)>) -> Layout {
        Layout {
            extern_order,
            ..self.clone()
        }
    }

    /// The extern-import index of the peer op `(interface, op)` — its position in `extern_order`. `None`
    /// if the program does not bind it (a compiler bug — the order is collected from the same
    /// `Core::ExternCall` nodes selection emits).
    pub fn extern_index(&self, interface: &str, op: &str) -> Option<usize> {
        self.extern_order
            .iter()
            .position(|(i, o)| i == interface && o == op)
    }

    /// The core-func index of the host-delegated op `(effect, op)` — its position in `host_order`. `None`
    /// if the program does not delegate it (a compiler bug — the order is collected from the same
    /// `Core::HostCall` nodes selection emits).
    pub fn host_index(&self, effect: &str, op: &str) -> Option<usize> {
        self.host_order
            .iter()
            .position(|(e, o)| e == effect && o == op)
    }

    /// The data-segment byte OFFSET of the host-arg string constant `s` — where its UTF-8 bytes lie in the
    /// program core module's memory, so a `Core::HostCall` string arg emits `(ptr=offset, len)`. `None` if
    /// the string was not laid out (a compiler bug — the same walk that collects them drives emission).
    pub fn host_string_offset(&self, s: &str) -> Option<u32> {
        self.host_strings
            .iter()
            .find(|(v, _)| v == s)
            .map(|(_, off)| *off)
    }

    /// The absolute wasm-function index of definition `def`, or `None` if it is not emitted. Imports
    /// occupy `0..import_base`, so a defined function's index is `import_base + its position in order`.
    /// O(1) via the `order_pos` index (the emission-position map built in `compute`).
    pub fn abs(&self, def: usize) -> Option<u32> {
        self.order_pos
            .get(&def)
            .map(|&k| self.import_base + k as u32)
    }

    /// CONTENT-ADDRESSED SPEC DEDUP redirect: point each merged spec at its representative's emission slot,
    /// so `abs(merged)` returns the representative's func index (the merged body is not in `order` and never
    /// emitted, but a `Core::Call` still names it — this makes that call resolve to the identical rep). The
    /// map is `merged_def → representative_def`; the rep is in `order`, so its `order_pos` slot is known.
    pub fn add_merged_spec_redirects(
        &mut self,
        spec_merge: &std::collections::HashMap<usize, usize>,
    ) {
        for (&merged, &rep) in spec_merge {
            if let Some(&k) = self.order_pos.get(&rep) {
                self.order_pos.insert(merged, k);
            }
        }
        // Store the map so the RUST backend's `fn_ident` can canonicalize a merged spec's callee to its
        // representative's name (the wasm `order_pos` redirect above only fixes the wasm func-index path).
        self.spec_merge = spec_merge.clone();
    }

    /// The representative a merged spec collapsed into, or `def` itself if it was not merged — the
    /// canonicalization the RUST backend applies to a `Core::Call` callee so a call to a merged-away spec
    /// (dropped from `order`, never emitted) names the structurally-identical representative that IS emitted,
    /// instead of a dangling by-name reference (rustc E0425). Identity for every def in a program with no
    /// redundant specializations (`spec_merge` empty). The map is flat (a representative is never itself
    /// merged — it is the lowest-index member of its class), so a single lookup suffices, no chase.
    pub fn spec_representative(&self, def: usize) -> usize {
        self.spec_merge.get(&def).copied().unwrap_or(def)
    }

    /// The [`ExportPlan`] for definition `def`, or `None` if `def` is not an export — an O(1) lookup
    /// (via `export_of_def`) replacing an `exports.iter().find(|e| e.def == def)` scan.
    pub fn export_plan(&self, def: usize) -> Option<&ExportPlan> {
        self.export_of_def.get(&def).map(|&i| &self.exports[i])
    }

    /// The absolute wasm-function index of lambda-lifted closure `slot` — the lifted functions emit
    /// AFTER the `order` defs, so lifted `slot` is wasm func `import_base + order.len() + slot`. This is
    /// what the funcref-table element section points at for table slot `slot`.
    pub fn lifted_abs(&self, slot: usize) -> u32 {
        self.import_base + (self.order.len() + slot) as u32
    }

    /// The TYPE-section index of lambda-lifted closure `slot`'s functype — the functypes are laid
    /// imports first, then `order` defs, then lifted lambdas, so lifted `slot`'s type index is
    /// `import_count + order.len() + slot`. A `call_indirect` applying a closure of that signature
    /// references this type. (Structural functypes: any type index with the matching `(param)->result`
    /// signature validates; using the lifted function's own type keeps it exact.)
    pub fn lifted_type_index(&self, slot: usize, import_count: u32) -> u32 {
        import_count + (self.order.len() + slot) as u32
    }
}

/// Compute the boundary layout for the program in `db` — a query the "compile" request drives. Demands
/// each export's result type (a lazy `type_of`); touches only the exported/reachable definitions.
/// A program with no export is rejected: nothing is public, so there is nothing to emit.
pub fn compute(db: &mut Db) -> Result<Layout, Reject> {
    if db.exports.is_empty() {
        return Err(Reject::decline("no `(export …)`: nothing is public"));
    }

    // Resolve each export to a plan by its DECLARED signature. An export naming no definition declines.
    let mut exports: Vec<ExportPlan> = Vec::new();
    for i in 0..db.exports.len() {
        let name = db.exports[i].name.clone();
        let def = match db.exports[i].def {
            Some(d) => d,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}` names no definition"
                )));
            }
        };
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}`: definition has no body"
                )));
            }
        };
        // The parameters — each `(name-occurrence, solved-type)`. An exported parameter needs a
        // DEFINITE type: its type is solved by demanding `type_of` on its binder, which is the
        // annotation type for an annotated param and `Any` for an unannotated one. An unannotated
        // (ambiguous) parameter has no machine width, so it DECLINES asking for an annotation — the
        // no-implicit-width rule (the backend can't pick a width the program didn't ask for).
        let params = export_params(db, def, &name)?;
        // The result type is the entry body's solved type — a lazy read of the type column.
        let result = type_of(db, body);
        trace!(target: "rcdzc::layout", %name, def, params = params.len(), result = %result.render_name(&db.name_ctx()), "export plan");
        exports.push(ExportPlan {
            name,
            def,
            body,
            params,
            result,
        });
    }

    finish_layout(db, exports)
}

/// Compute the boundary layout for a `cdz test` build: the exported entries are every `@test`
/// definition (`db.test_defs`), IN PLACE OF the program's `(export …)` clauses. Each becomes an
/// `ExportPlan` with its solved result type (a test's body typically diverges — it traps on failure —
/// so `result` is a `Never`/`Unit`, which the backend's diverging-export path crosses as a no-result
/// entry). The reachable set + lifted closures are closed exactly as [`compute`] does (shared
/// `finish_layout`). A build with NO `@test` def declines ("no `@test`: nothing to run") — the
/// test-artifact analogue of `compute`'s "no export" decline.
///
/// A `@test` with PARAMETERS is a PROPERTY test: its parameters cross the boundary as ordinary export
/// parameters (`export_params`, the boundary-representable variant), and `cdz test` runs it over many
/// trials with generated inputs (`cdz-run --call NAME --arg …`). A NULLARY `@test` is the plain
/// (single-run) case — `export_params` returns an empty list for it, so both go through one path. A
/// parameter whose type has no boundary valtype still declines with the "annotate it" message
/// `export_params` gives (a property input must be a concrete scalar the runner can generate + pass).
///
/// This is the ONE place the export SOURCE differs from `compute`; everything downstream (reachability,
/// selection, serialization, the diverging-export→unit-entry crossing) is source-agnostic, so a test
/// export rides the identical machinery. The normal `(export …)` path is untouched — a test build is a
/// distinct sidecar request (`Request::EmitTests`), never the default.
pub fn compute_tests(db: &mut Db) -> Result<Layout, Reject> {
    compute_tests_for(db, &db.test_defs())
}

/// The subset variant of [`compute_tests`]: lay out the boundary from a GIVEN list of `@test` def
/// indices (in place of ALL `db.test_defs()`), so one linked closure can be lowered ONCE and emitted as N
/// per-file test components — each a layout-view rooted at that file's `@test` bucket, sharing the arena's
/// Core (no re-lower, no relocation). The `EmitTestsPerFile` request buckets `db.test_defs()` by
/// `linkage.file_of(sig_occ)` and calls this once per file. `defs` MUST be a subset of `db.test_defs()`
/// (each a nullary/property `@test`); an empty list declines like the all-tests case. The reachable set +
/// lifted closures close over just this bucket's bodies (`finish_layout`), so each view emits only the
/// functions its own tests reach — exactly what a per-file `cdz test <file>` compile lays out today.
pub fn compute_tests_for(db: &mut Db, defs: &[usize]) -> Result<Layout, Reject> {
    if defs.is_empty() {
        return Err(Reject::decline(
            "no `@test` definition: nothing to run (mark a nullary def with `@test`)",
        ));
    }
    // Iterate the caller's slice BY REF — the def indices are `Copy`, and `defs` is a caller-owned slice
    // (never aliasing `db`), so it coexists with the `&mut db` reborrows in the loop body; no clone needed.
    // (`compute_tests` passes `&db.test_defs()`, a temporary that lives for this call.)
    let mut exports: Vec<ExportPlan> = Vec::new();
    for &def in defs {
        let name = db.defs[def].name.clone();
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!(
                    "`@test` definition `{name}` has no body"
                )));
            }
        };
        // A test's PARAMETERS (empty for a plain test) cross as boundary-representable export params — the
        // property-test inputs `cdz test` generates + passes. `export_params` solves each param's type and
        // declines a non-representable one (asking for an annotation), exactly as a normal export does; a
        // nullary test yields an empty list, so the plain and property cases share this one path.
        let params = export_params(db, def, &name)?;
        let result = type_of(db, body);
        trace!(target: "rcdzc::layout", %name, def, params = params.len(), result = %result.render_name(&db.name_ctx()), "test export plan");
        exports.push(ExportPlan {
            name,
            def,
            body,
            params,
            result,
        });
    }
    finish_layout(db, exports)
}

/// OPTION C (shared-closure component) — partition a test build's reachable emitted defs into the two sets
/// the component split needs: `own` (defs belonging to the `own_file` — a file's own `@test` bodies + its
/// file-local helpers) and `shared` (defs belonging to ANOTHER file — the imported closure a `@test` reaches
/// through `run-src`/etc., which Option C emits ONCE as its own component rather than re-embedding per file).
/// A def's file is `db.file_of(def.sig_occ)`; a def in NO file (`None` — prelude / synthesized / the linked
/// `(do …)` root) is treated as SHARED (it is not the test file's own source, and belongs in the one shared
/// component every test view imports). Returns `(own, shared)` as `defs` indices, preserving `layout.order`
/// (emission order) within each. Pure over the reachable set — the analysis increment (a); the provider/
/// consumer EMIT increments (b)/(c) consume this. `own_file` is the file index a test bucket belongs to
/// (`db.file_of` of any of its `@test` sig-occurrences), the same file identity `EmitTestsPerFile` buckets by.
pub fn partition_reachable_for_file(
    db: &Db,
    layout: &Layout,
    own_file: usize,
) -> (Vec<usize>, Vec<usize>) {
    let mut own = Vec::new();
    let mut shared = Vec::new();
    for &def in &layout.order {
        let sig = db.defs[def].sig_occ;
        if db.file_of(sig) == Some(own_file) {
            own.push(def);
        } else {
            shared.push(def);
        }
    }
    (own, shared)
}

/// OPTION C increment (b)(i) — the CROSS-COMPONENT INTERFACE export set: the `shared` defs that an `own`
/// def CALLS (a `Core::Call` edge from own → shared). This is what the shared-closure component must EXPORT
/// as interface funcs (and each per-file @test component IMPORTS) — NOT the whole `shared` set: a shared def
/// called only WITHIN the closure (shared→shared) stays emitted inside the provider, never crossing the peer
/// boundary; only a shared def on a cross-file call edge needs an interface func. Computed off the partition
/// ([`partition_reachable_for_file`]): walk each `own` def's body for its `Core::Call` callees
/// (`collect_call_callees`), keep those in `shared`. Returns the cross-edge shared-def indices in
/// `layout.order` (emission) order, deduplicated.
///
/// A cross-edge whose callee's signature has NO cross-component boundary rep (a higher-order def — a bare
/// function-typed param/result, which `host::extern_abi_val_type` returns `None` for) is the KNOWN CONSTRAINT
/// the provider emit (b) must DECLINE cleanly (a `todo`, not a miscompile) — reported there, not here; this
/// analysis just names the edge set.
pub fn cross_component_edges(db: &mut Db, layout: &Layout, own_file: usize) -> Vec<usize> {
    let (own, shared) = partition_reachable_for_file(db, layout, own_file);
    let shared_set: crate::fxhash::FxHashSet<usize> = shared.iter().copied().collect();
    // Membership SET (not a Vec with `.contains` — O(1) dedup + O(1) final filter, vs the O(N²) the
    // nested own×callees scan + the layout.order re-scan would be; PR#877). Determinism is preserved
    // because the RETURNED order iterates `layout.order` (source-fixed) filtered by set membership — the
    // set only decides WHICH edges, never their ORDER (the reproducible-derivation contract below).
    let mut edges: crate::fxhash::FxHashSet<usize> = crate::fxhash::FxHashSet::default();
    for &owner in &own {
        if let Some(body) = db.defs[owner].body {
            let mut callees = Vec::new();
            collect_call_callees(db, body, &mut callees);
            for c in callees {
                if shared_set.contains(&c) {
                    edges.insert(c);
                }
            }
        }
    }
    // Return in emission (`layout.order`) order for determinism, not discovery order.
    layout
        .order
        .iter()
        .copied()
        .filter(|d| edges.contains(d))
        .collect()
}

/// OPTION C increment (c)(iii) — the UNION cross-component edge set across MANY files, for a COMPOSED
/// `cdz test <dir>` build (the `EmitTestsComposed` driver). A single [`cross_component_edges`] is per-file
/// (`own_file`); a composed build has ONE shared-closure provider that must export the UNION of every file's
/// cross-edges (a def is a provider export if ANY target file calls it across the file boundary), and EACH
/// per-file consumer imports the WHOLE provider interface in this canonical order (a file hitting only a subset
/// still indexes at the right provider-export position — the index-agreement invariant [`compute_tests_consumer`]
/// honors). Folds each file's [`cross_component_edges`] into one set, returned in `layout.order` (emission)
/// order for determinism (same reproducible-derivation contract: the set decides WHICH edges, `layout.order`
/// decides their ORDER). A test-LOCAL def can never enter the union: it lives in its own file's partition
/// (`own`), never in another file's `shared`, so it is never a cross-edge in ANY file (verified against the
/// self-host `sread-eval-*` files, which all `import { run-src } from "sread-eval"` — one shared lib, no
/// file imports another test file). Empty `files` → empty (no shared closure to hoist).
pub fn cross_component_edges_union(db: &mut Db, layout: &Layout, files: &[usize]) -> Vec<usize> {
    let mut edges: crate::fxhash::FxHashSet<usize> = crate::fxhash::FxHashSet::default();
    for &own_file in files {
        for e in cross_component_edges(db, layout, own_file) {
            edges.insert(e);
        }
    }
    // Return in emission (`layout.order`) order — the SAME canonical order each per-file
    // `cross_component_edges` uses, so the provider export order and every consumer's import order agree.
    layout
        .order
        .iter()
        .copied()
        .filter(|d| edges.contains(d))
        .collect()
}

/// OPTION C increment (b)(ii) — build the SHARED-CLOSURE PROVIDER layout: a `Layout` whose EXPORTS are the
/// cross-component edge defs ([`cross_component_edges`] — the `shared` defs each per-file `@test` component
/// calls). Each edge def becomes an exported interface func (an [`ExportPlan`], its params/result solved the
/// same way [`compute`]/[`compute_tests_for`] do); [`finish_layout`] then closes reachability so the edges'
/// OWN intra-closure callees are emitted INSIDE the provider (not re-exported). This is the layout the
/// provider emit (`db.component_name` set) consumes to emit the shared closure ONCE as its own component.
/// A build with no cross-edge (a single-file program, or @tests that call nothing shared) declines "no
/// shared closure" — there is nothing to hoist. The per-edge ABI-representability check (a higher-order edge
/// has no cross-component boundary rep) is the EMIT layer's (target-neutral layout does not know the wasm
/// extern ABI); this just names the export set + closes its reachable body.
/// The SOURCE name a (possibly transformed) def crosses a component boundary under — the base before any
/// internal transform suffix. The compiler renames a transformed def by APPENDING a `$`/`#`-marked suffix
/// to its source base (`{base}$acc` for the linear-non-tail-recursion accumulator rewrite, accum.rs; `{base}
/// #monoN` for a monomorphized specialization, lower.rs); a def may carry both (`f$acc#mono3`). Those markers
/// are invalid in a component extern name (ASCII kebab only), and the boundary contract wants the STABLE
/// source name anyway, so the boundary name is everything before the FIRST `$` or `#`. Shared by the Option-C
/// provider EXPORT (b) and consumer IMPORT (c) so both name the same interface func. A def with no transform
/// suffix returns its whole name unchanged.
pub fn source_boundary_name(emitted_name: &str) -> &str {
    match emitted_name.find(['$', '#']) {
        Some(i) => &emitted_name[..i],
        None => emitted_name,
    }
}

/// The boundary export names for an ORDERED cross-edge slice, guaranteed UNIQUE. Each edge's name is
/// [`source_boundary_name`] of its def name; when that is unique across the slice (the common case — one
/// shared library function per base name) the result is byte-identical to mapping `source_boundary_name`
/// directly. But two DISTINCT specializations of one base — effect specs `f#eff529`/`f#eff531`, or two
/// monomorphizations `f#mono3`/`f#mono7` — both strip to the same base `f`, so if both cross the provider
/// boundary they would export TWICE under `f`: a DUPLICATE component export name, which is invalid wasm
/// (`failed to parse WebAssembly module`, caught only at load). This disambiguates the 2nd+ occurrence of a
/// colliding base with a deterministic letter-led kebab suffix (`f-dup2`, `f-dup3`, …) so every export is
/// unique and a valid extern name, re-bumping if a disambiguated name would itself collide with another
/// edge's base. The derivation is a pure function of the (order-fixed) edge slice, so the PROVIDER export
/// order and every CONSUMER import order name the same interface func at the same position — the shared
/// provider↔consumer boundary-name convention both [`compute_provider_for_edges`] and
/// [`compute_tests_consumer`] MUST derive identically (they both call THIS).
pub fn boundary_export_names(db: &Db, edges: &[usize]) -> Vec<String> {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(edges.len());
    for &def in edges {
        let base = source_boundary_name(&db.defs[def].name).to_string();
        let mut name = base.clone();
        let mut k = 2usize;
        while used.contains(&name) {
            name = format!("{base}-dup{k}");
            k += 1;
        }
        used.insert(name.clone());
        out.push(name);
    }
    out
}

pub fn compute_shared_closure_provider(
    db: &mut Db,
    test_layout: &Layout,
    own_file: usize,
) -> Result<Layout, Reject> {
    let edges = cross_component_edges(db, test_layout, own_file);
    compute_provider_for_edges(db, &edges)
}

/// OPTION C increment (c)(iii)b — build the shared-closure PROVIDER layout from an EXPLICIT cross-edge set,
/// the generalization [`compute_shared_closure_provider`] (single-file) delegates to. A COMPOSED `cdz test
/// <dir>` build ([`EmitTestsComposed`]) passes the UNION cross-edge set ([`cross_component_edges_union`] over
/// all files) so the ONE provider exports every file's cross-edges; the single-file path passes one file's
/// [`cross_component_edges`]. Each edge def becomes an exported interface func (an [`ExportPlan`], boundary
/// name = [`source_boundary_name`] so a transformed `f$acc`/`f#monoN` exports as the kebab source name);
/// [`finish_layout`] closes reachability so the edges' own intra-closure callees emit INSIDE the provider.
/// `edges` MUST be in the canonical `layout.order`-derived order both `cross_component_edges`(_union) return,
/// so the provider's export order matches every consumer's import order (the index-agreement invariant). An
/// EMPTY edge set declines "no shared closure" — there is nothing to hoist.
pub fn compute_provider_for_edges(db: &mut Db, edges: &[usize]) -> Result<Layout, Reject> {
    if edges.is_empty() {
        return Err(Reject::decline(
            "no shared closure: the @tests call no imported (cross-file) definition",
        ));
    }
    // The boundary export name for each edge — its SOURCE name (NOT its emitted name, which for a TRANSFORMED
    // def carries an internal `$`/`#`-marked suffix — `f$acc`, `f#monoN`, `f#effN` — invalid in a component
    // extern name), DISAMBIGUATED so two distinct specializations of one base (`f#eff529`/`f#eff531`) do not
    // both export as bare `f` (a duplicate export name → invalid wasm). `boundary_export_names` derives this
    // as a pure function of the order-fixed edge slice, so the CONSUMER side (increment c) derives the SAME
    // names at the SAME positions — the shared provider↔consumer boundary-name convention.
    let names = boundary_export_names(db, edges);
    let mut exports: Vec<ExportPlan> = Vec::new();
    for (ei, &def) in edges.iter().enumerate() {
        let name = names[ei].clone();
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!(
                    "shared-closure export `{name}` has no body"
                )));
            }
        };
        let params = export_params(db, def, &name)?;
        let result = type_of(db, body);
        exports.push(ExportPlan {
            name,
            def,
            body,
            params,
            result,
        });
    }
    finish_layout(db, exports)
}

/// The EMPTY-library provider — a valid component with ZERO exports (no boundary funcs). The
/// whole-library-provider analogue of [`compute_provider_for_edges`] for a FULLY-INLINED suite: when every
/// reachable non-`@test` def inlines away, the library edge set is empty and `compute_provider_for_edges`
/// DECLINES ("no shared closure"). But the per-test-shred exec model is UNIFORM — every `@test` component
/// `--peer`s a "main" — so a fully-inlined suite still wants a (trivial) main to link, and its per-test
/// consumers simply import NOTHING from it (a harmless no-op peer). This emits exactly that: `finish_layout`
/// over an empty export set → an empty-but-valid component. (`finish_layout` is private to layout; this is
/// the pub seam the shred emit calls for the empty-`library_edges` case — emitting each test STANDALONE
/// instead would re-introduce the has-main bifurcation the whole-library model removes. v-cdz-crate-split.)
pub fn compute_empty_provider(db: &mut Db) -> Result<Layout, Reject> {
    finish_layout(db, Vec::new())
}

/// OPTION C increment (c) — the CONSUMER (per-file `@test`) layout: like [`compute_tests_for`] but with the
/// cross-component edges as a BOUNDARY (excluded from `order`, recorded as extern imports). Each per-file
/// `@test` component EXCLUDES the shared cross-edge defs (they live in the shared-closure PROVIDER component,
/// [`compute_shared_closure_provider`]) and routes its `Core::Call`s into them as extern imports. Returns a
/// [`Layout`] whose `extern_order` lists EVERY provider edge (as `(closure_iface, source_boundary_name(def))`,
/// in the PROVIDER's export order = `provider_edges` order) and whose `cross_edge_import` maps each cross-edge
/// `def → its position in that order` — so a `Core::Call` to a cross-edge callee emits a
/// `Lir::CallExternImport` of the matching provider-export index (the v-wasm-opt index-agreement invariant:
/// consumer import index == provider export index). It builds that mapping ITSELF (via
/// [`Layout::with_cross_edge_imports`]); the positions are 0-based here (this layout carries no other extern
/// imports), and the backend shifts them by any peer-extern count it prepends
/// ([`Layout::with_cross_edge_import_shift`]). `test_defs` is this file's `@test` bucket; `provider_edges` is
/// the whole closure's cross-edge set ([`cross_component_edges`] over ALL `@tests`, in provider-export order)
/// — the consumer imports the WHOLE provider interface (a file hitting only a subset still indexes at the
/// right provider-export position; unused imports are harmless).
pub fn compute_tests_consumer(
    db: &mut Db,
    test_defs: &[usize],
    provider_edges: &[usize],
    closure_iface: &str,
) -> Result<Layout, Reject> {
    if test_defs.is_empty() {
        return Err(Reject::decline(
            "no `@test` definition: nothing to run (mark a nullary def with `@test`)",
        ));
    }
    let mut exports: Vec<ExportPlan> = Vec::new();
    for &def in test_defs {
        let name = db.defs[def].name.clone();
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!("`@test` `{name}` has no body")));
            }
        };
        let params = export_params(db, def, &name)?;
        let result = type_of(db, body);
        exports.push(ExportPlan {
            name,
            def,
            body,
            params,
            result,
        });
    }
    // The boundary = the WHOLE provider edge set (this file may HIT only some, but the boundary must exclude
    // ALL of them so nothing from the shared closure is re-emitted here).
    let boundary: std::collections::HashSet<usize> = provider_edges.iter().copied().collect();
    let (layout, boundary_hits) = finish_layout_bounded(db, exports, &boundary)?;
    // KEYSTONE: INDEX-AGREEMENT: the consumer's import position for a cross-edge is its position in the PROVIDER's
    // EXPORT order (`provider_edges`, the SAME sequence `compute_shared_closure_provider` exports — both from
    // `cross_component_edges`'s `layout.order` order). So `extern_order` lists ALL provider edges (not just
    // this file's hits) in provider order, and each cross-edge def maps to its provider export index. A file
    // that hits only a subset still indexes into the full provider interface at the RIGHT position — the
    // consumer import index == the provider export index, keeping the composed module valid (v-wasm-opt's
    // index-agreement invariant). `boundary_hits` is a subset of `provider_edges`; we emit imports for the
    // WHOLE provider interface (a component imports the full interface; unused funcs are harmless).
    let _ = boundary_hits; // hits are a subset of provider_edges; we map the full provider interface.
    let mut additions: Vec<(String, String)> = Vec::new();
    let mut import_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let base = layout.extern_order.len();
    // Derive the import op names the SAME way the provider derives its export names (`boundary_export_names`
    // over the SAME order-fixed `provider_edges` slice), so the consumer imports match the provider exports
    // by name AND position — including the disambiguation of two specializations of one base (`f`/`f-dup2`).
    let op_names = boundary_export_names(db, provider_edges);
    for (i, &def) in provider_edges.iter().enumerate() {
        additions.push((closure_iface.to_string(), op_names[i].clone()));
        import_map.insert(def, base + i);
    }
    Ok(layout.with_cross_edge_imports(additions, import_map))
}

/// Close the reachable set + lifted closures over a resolved list of `exports`, and build the [`Layout`]
/// — the shared tail of [`compute`] (the program's `(export …)` clauses) and [`compute_tests`] (the
/// `@test` defs). Everything here is agnostic to WHERE the exports came from: reachability follows
/// `Core::Call` edges, lifted closures follow `Core::Closure` edges, both seeded from the export bodies.
fn finish_layout(db: &mut Db, exports: Vec<ExportPlan>) -> Result<Layout, Reject> {
    finish_layout_bounded(db, exports, &std::collections::HashSet::new()).map(|(l, _)| l)
}

/// [`finish_layout`] with a `boundary` set of def indices that the `Core::Call`-reachability worklist treats
/// as LEAVES: a callee in `boundary` is NOT added to `order` (so its body is never walked → its exclusive
/// callees + closures never enter the emission set) and is instead recorded into the returned `boundary_hits`
/// set. This is the OPTION-C CONSUMER primitive (increment c): the cross-edge shared defs are the boundary —
/// each per-file `@test` component EXCLUDES them (they live in the shared-closure provider component) and
/// routes its `Core::Call`s into them as extern imports. The lifted-closure + functype passes then operate on
/// the already-pruned `order`, so no closure/functype of a boundary-only def enters either. `finish_layout`
/// passes an EMPTY boundary → byte-identical to before (no hit, nothing pruned). Returns `(layout,
/// boundary_hits)` — the caller turns `boundary_hits` into the consumer's `extern_order`.
fn finish_layout_bounded(
    db: &mut Db,
    exports: Vec<ExportPlan>,
    boundary: &std::collections::HashSet<usize>,
) -> Result<(Layout, std::collections::BTreeSet<usize>), Reject> {
    // `boundary_hits` is a BTreeSet (not a HashSet) so its iteration order is DETERMINISTIC (ascending def
    // index) — a caller that turns hits into the consumer's extern-import order must not derive it from a
    // nondeterministic hash order, or two derivations of the same source could assign different import
    // indices (an index-mismatch invalid-module risk; the v-wasm-opt index-agreement invariant). Today's
    // caller (`compute_tests_consumer`) derives import order from the ordered `provider_edges` slice and
    // discards `boundary_hits`, so this is a latent-footgun fix, not a live bug — but the ordered type keeps
    // any future hits-driven caller reproducible by construction (PR#880 Copilot nit).
    let mut boundary_hits: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    // Emission order: exported definitions first (declaration order, deduplicated), then every
    // definition REACHABLE from them through a runtime `Core::Call` — a recursive callee, or a callee
    // that a recursive function reaches. A worklist closes the reachable set: for each def in `order`,
    // lower its body and append any `Core::Call` callee not already present. (Non-recursive calls
    // inline, so they add nothing here — only a `Core::Call` grows the set.)
    //
    // This SEQUENCE is a deterministic function of the source (export declaration order, then a
    // source-structure worklist) — no filesystem enumeration order or nondeterministic collection
    // iteration reaches it, so the backend emits definitions/data/interface entries in a source-fixed
    // order and two derivations of the same source byte-match.
    //= spec/contracts/reproducible-derivation.md#codegen-order-is-source-determined
    //# The order in which the compiler emits definitions, data, and interface entries MUST be a deterministic function of the source.
    //= spec/contracts/reproducible-derivation.md#codegen-order-is-source-determined
    //# The compiler MUST NOT let filesystem enumeration order or nondeterministic collection iteration affect the order of its output.
    //= constitution.md#ii-compilation-is-reproducible
    //# The compiler MUST emit its output in an order that is a function of the source alone, independent of filesystem enumeration order or nondeterministic collection iteration.
    //= spec/contracts/reproducible-derivation.md#derivation-is-a-function-of-source-and-toolchain
    //# Deriving the same canonical source with the same pinned toolchain MUST produce byte-identical component output.
    // `order` keeps the emission SEQUENCE (exports first, then reachable callees); `in_order` is the
    // O(1) membership check that goes with it. A plain `order.contains(&x)` here is an O(len) scan, and
    // it runs once per export AND once per discovered callee — O(N²) on a program with many exports or
    // a wide call fan-out (a 3200-export program spent ~all its layout time in these scans + the Vec
    // regrowth they drove). The set keeps each "already queued?" test O(1).
    let mut order: Vec<usize> = Vec::new();
    let mut in_order: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for e in &exports {
        if in_order.insert(e.def) {
            order.push(e.def);
        }
    }
    // The `Core::Call`-reachability worklist, resumable from `call_i` — closes `order` over every runtime
    // callee of the def bodies currently in `order`. Factored out because the LIFTED-closure worklist
    // below can ADD new defs to `order` (a lifted body specializes + calls a fresh spec — the nested
    // recursive const-closure-driver case), and those newly-added defs' OWN callees must then be walked
    // too. Without re-closing, a def reachable only through a lifted body's call chain (e.g. a nested
    // `filter-step`→`drive` specialization) is appended to `order` but its callee is never discovered →
    // a `Core::Call` to an un-laid-out function index → INVALID WASM ("function index out of bounds").
    let mut call_i = 0;
    let close_call_worklist =
        |db: &mut Db,
         order: &mut Vec<usize>,
         in_order: &mut std::collections::HashSet<usize>,
         call_i: &mut usize,
         boundary_hits: &mut std::collections::BTreeSet<usize>| {
            while *call_i < order.len() {
                let def = order[*call_i];
                if let Some(body) = db.defs[def].body {
                    let mut callees = Vec::new();
                    collect_call_callees(db, body, &mut callees);
                    for c in callees {
                        // A BOUNDARY callee (an Option-C cross-edge) is an EXTERN IMPORT, not an emitted def:
                        // record it + do NOT add it to `order` (so its body is never walked → its exclusive
                        // callees/closures never enter this consumer's emission set). The `is_empty` guard
                        // short-circuits the per-callee hash lookup on the ORDINARY (`finish_layout`,
                        // empty-boundary) path → byte-identical AND no added work there (PR#880 Copilot nit).
                        if !boundary.is_empty() && boundary.contains(&c) {
                            boundary_hits.insert(c);
                            continue;
                        }
                        if in_order.insert(c) {
                            trace!(target: "rcdzc::layout", def = c, "reachable via a runtime call — added to emission order");
                            order.push(c);
                        }
                    }
                }
                *call_i += 1;
            }
        };
    close_call_worklist(
        db,
        &mut order,
        &mut in_order,
        &mut call_i,
        &mut boundary_hits,
    );

    // LAMBDA-LIFTED closures: lowering the def bodies above (via `collect_call_callees` → `core_of`)
    // registers each surviving `(fn …)` into `db.lifted` (a `Core::Closure` naming its table slot). But
    // `db.lifted` accumulates EVERY lambda `lower_lambda_value` touched — including one demanded during
    // type-checking / fold exploration that the final emitted code FOLDS AWAY (a constant closure applied
    // immediately). Emitting such a DEAD lift is both wasteful and unsound (its body may read captures
    // from an env no reachable `Core::Closure` ever builds). So collect only the lifted lambdas REACHED
    // by a `Core::Closure { code }` in an EMITTED body (the reachable defs' bodies), transitively (a
    // reached lambda's body may itself build a closure). This is the closure analogue of the
    // `Core::Call` reachability above.
    let mut reached_codes: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Seed from every reachable def body.
    for &def in &order {
        if let Some(body) = db.defs[def].body {
            collect_closure_codes(db, body, &mut reached_codes);
        }
    }
    // Transitively close: a reached lambda's body may build further closures AND call further defs.
    let mut work: Vec<usize> = reached_codes.iter().copied().collect();
    while let Some(code) = work.pop() {
        let body = db.lifted[code].body;
        let mut more = std::collections::HashSet::new();
        collect_closure_codes(db, body, &mut more);
        for c in more {
            if reached_codes.insert(c) {
                work.push(c);
            }
        }
        // A reached lifted body's own `Core::Call` callees must be emitted too.
        let mut callees = Vec::new();
        collect_call_callees(db, body, &mut callees);
        for c in callees {
            // A boundary (cross-edge) callee reached via a lifted body is likewise an extern import, not
            // an emitted def — record + skip (see the `close_call_worklist` note; `is_empty` guards the
            // per-callee lookup on the empty-boundary path).
            if !boundary.is_empty() && boundary.contains(&c) {
                boundary_hits.insert(c);
                continue;
            }
            if in_order.insert(c) {
                trace!(target: "rcdzc::layout", def = c, "reachable via a lifted closure body — added to emission order");
                order.push(c);
            }
        }
        // Re-close the `Core::Call` worklist over any defs this lifted body just added to `order`: such a
        // def (a spec a lifted body specialized + called) has its OWN callees, and its body may build
        // further closures — both must be reached. `close_call_worklist` resumes from `call_i`, and any
        // new closure codes it surfaces are picked up by the seed-and-transitively-close pass below (this
        // `while work` loop) since `collect_closure_codes` runs on the growing `order` too. This is the
        // joint fixpoint of the call-reachability and lifted-closure worklists — a nested recursive
        // const-closure driver (filter-step under drive) needs both to converge or a spec dangles.
        close_call_worklist(
            db,
            &mut order,
            &mut in_order,
            &mut call_i,
            &mut boundary_hits,
        );
        // Seed closure codes from any defs just added to `order` (a spec's body may build closures whose
        // lifted bodies must be reached); push newly-seen codes onto the lifted worklist so this loop
        // converges to the joint fixpoint.
        let order_snapshot: Vec<usize> = order.clone();
        for def in order_snapshot {
            if let Some(dbody) = db.defs[def].body {
                let mut more = std::collections::HashSet::new();
                collect_closure_codes(db, dbody, &mut more);
                for c in more {
                    if reached_codes.insert(c) {
                        work.push(c);
                    }
                }
            }
        }
    }
    // The lifted set snapshotted in table-slot order. `reached` marks which slots a reachable
    // `Core::Closure` actually builds — an UNREACHED slot (a lambda demanded during type-checking / fold
    // exploration that the emitted code folds away) is emitted as an inert STUB and its table entry left
    // out (never called), so a dead lift is neither unsound nor referenced.
    let lifted = db.lifted.clone();
    let lifted_reached: Vec<bool> = (0..lifted.len())
        .map(|code| reached_codes.contains(&code))
        .collect();

    // CONTENT-ADDRESSED SPEC DEDUP (transient-spec cost-cliff): now that `order` holds the full reachable
    // set (both worklists closed) and every reachable body is lowered, collapse the recursive-effectful
    // specializations (`f#eff{n}`) that are congruent (structurally identical up to occurrence id + the
    // classes of the specs they call — a partition-refinement over the spec call graph). A merged spec is
    // DROPPED from `order` (its body never emitted) and its func-index resolution redirected to its
    // representative (structurally identical, so it serves the merged spec's callers). The redirect is
    // applied to `order_pos` after layout construction. Empty map (the common case: no congruent specs) →
    // byte-identical to before, so a program without redundant effect specializations is unaffected.
    let spec_merge = effect_spec_merge_map(db, &order);
    if !spec_merge.is_empty() {
        order.retain(|d| !spec_merge.contains_key(d));
    }

    // `import_base` is 0 until a program uses a runtime op: the per-program runtime-import set is
    // computed by the backend when a `Core` compound op lowers to a heap call (value-heap H2). A
    // program that imports nothing keeps base 0 and is byte-identical to a runtime-free build.
    let mut base_layout = Layout::with_lifted(exports, order, 0, lifted, lifted_reached);
    // Redirect each merged spec's func-index resolution to its representative's slot, so a `Core::Call`
    // whose callee is a merged spec resolves to the (identical) representative's emitted function.
    if !spec_merge.is_empty() {
        base_layout.add_merged_spec_redirects(&spec_merge);
    }

    // EXTRA closure-application functypes: a `Core::CallClosure` can reach a closure of a type NO lifted
    // lambda in this program builds — the applied variant's `match` arm is statically emitted but never
    // dynamically constructed (e.g. an `Iter` sum with a `ScanI(Int64->Int64->Int64)` AND a
    // `FlatMapI(Int64->Iter)` variant where only `ScanI` is ever built: the `FlatMapI` arm still emits a
    // `call_indirect f(x)` needing an `(env:i32, i64)->i32` functype, but no such lifted body exists). The
    // `call_indirect` needs a type-section functype of the matching structural shape regardless. Collect
    // the reachable `Core::CallClosure` signatures (over the emitted defs + reached lifted bodies) whose
    // `(env, arg valtypes)->ret` shape no reached lifted lambda supplies, and register one functype each —
    // laid after the lifted functypes so no existing index shifts. Without this the application declines
    // ("a runtime closure application has no matching function type"), rejecting a valid program.
    let mut sigs: Vec<(Vec<crate::backend::wasm::lir::ValType>, Ty)> = Vec::new();
    let mut seen_sig: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The shapes a reached lifted lambda already covers (its functype is `(env:i32, params…)->ret`).
    let mut lifted_shapes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (code, l) in base_layout.lifted.iter().enumerate() {
        if !base_layout
            .lifted_reached
            .get(code)
            .copied()
            .unwrap_or(true)
        {
            continue; // an unreached lifted body is a stub, but its functype IS in the type section
        }
        if let Some(key) = closure_shape_key(&closure_env_param_vts(l), &l.ret_ty) {
            lifted_shapes.insert(key);
        }
    }
    // Every reached lifted lambda's functype exists whether reached or not (both emit a function), so an
    // UNREACHED lifted lambda's shape is covered too — include those shapes so we don't double-register.
    for (code, l) in base_layout.lifted.iter().enumerate() {
        let _ = code;
        if let Some(key) = closure_shape_key(&closure_env_param_vts(l), &l.ret_ty) {
            lifted_shapes.insert(key);
        }
    }
    // Walk every emitted body (order defs + reached lifted bodies) for `Core::CallClosure` signatures.
    let mut walk_roots: Vec<StructId> = Vec::new();
    for &def in &base_layout.order {
        if let Some(body) = db.defs[def].body {
            walk_roots.push(body);
        }
    }
    for (code, l) in base_layout.lifted.iter().enumerate() {
        if base_layout
            .lifted_reached
            .get(code)
            .copied()
            .unwrap_or(true)
        {
            walk_roots.push(l.body);
        }
    }
    for root in walk_roots {
        let mut found: Vec<(Vec<crate::backend::wasm::lir::ValType>, Ty)> = Vec::new();
        collect_closure_call_sigs(db, root, &mut found);
        for (param_vts, ret) in found {
            let Some(key) = closure_shape_key(&param_vts, &ret) else {
                continue;
            };
            if lifted_shapes.contains(&key) || !seen_sig.insert(key) {
                continue; // covered by a lifted lambda, or an already-collected extra shape
            }
            sigs.push((param_vts, ret));
        }
    }

    Ok((base_layout.with_closure_call_types(sigs), boundary_hits))
}

/// The full param valtypes of a lifted lambda's functype — the leading i32 ENV cell, then each source
/// param's valtype. Mirrors the backend's `(env, param…) -> result` lowering (a param with no machine rep
/// is dropped, matching `stub_function`; such a lambda would already have declined). Used to key a lifted
/// lambda's structural shape against a `Core::CallClosure`'s.
fn closure_env_param_vts(
    l: &crate::lower::LiftedLambda,
) -> Vec<crate::backend::wasm::lir::ValType> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    let mut vts = vec![ValType::I32]; // slot 0: the env cell (an i32 handle)
    for (_, pt) in &l.params {
        if let Some(v) = valtype_of(pt) {
            vts.push(v);
        }
    }
    vts
}

/// A stable string key for a closure functype's structural shape (`(param valtypes) -> ret valtype-or-unit`)
/// — used to dedup extra closure-application functypes and to test lifted-lambda coverage. `None` if the
/// result type has no machine representation (the application would decline elsewhere).
fn closure_shape_key(param_vts: &[crate::backend::wasm::lir::ValType], ret: &Ty) -> Option<String> {
    use crate::backend::wasm::lir::valtype_of;
    let params: Vec<String> = param_vts.iter().map(|v| format!("{v:?}")).collect();
    let ret_key = match valtype_of(ret) {
        Some(v) => format!("{v:?}"),
        None if matches!(ret, Ty::Unit) => "unit".to_string(),
        None => return None,
    };
    Some(format!("{}->{ret_key}", params.join(",")))
}

/// Collect each reachable `Core::CallClosure`'s functype shape under `id` — `(env-prefixed param valtypes,
/// result type)`, matching the backend's `(env:i32, args…)->result` `call_indirect` signature. The env i32
/// is prepended; each arg's valtype comes from its solved type; the result type is the closure type peeled
/// by the arg count. A signature whose args/result have no machine rep is skipped (the application declines
/// at select). Walks the whole core tree via `core_child_ids` (the generic child walker), depth-guarded like
/// the sibling closure/callee walks so a non-normalizing deep core chain cannot overflow the compiler stack.
fn collect_closure_call_sigs(
    db: &mut Db,
    id: StructId,
    out: &mut Vec<(Vec<crate::backend::wasm::lir::ValType>, Ty)>,
) {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    use crate::core::Core;
    if db.walk_depth >= crate::db::WALK_DEPTH_LIMIT {
        return;
    }
    // VISITED-SET (see [`Db::closure_sig_visited`] and the twin in `collect_call_callees`): the collected
    // call-sig shapes are dedup'd by the consumer (`closure_shape_key` + `seen_sig`), so a node visited once
    // vs. many yields identical `sigs` — skipping a re-visited shared-DAG node changes no output while
    // avoiding the `O(K^depth)` re-descent. Cleared at the top-level entry (`walk_depth == 0`).
    if db.walk_depth == 0 {
        db.closure_sig_visited.clear();
    }
    if !db.closure_sig_visited.insert(id) {
        return;
    }
    db.walk_depth += 1;
    if let Core::CallClosure { closure, args } = crate::lower::core_of(db, id) {
        // The application's env-prefixed param valtypes + result type — the shape the `call_indirect` needs.
        // A `Unit` argument is ELIDED (it occupies no wasm slot — the same elision `select::closure_type_index`
        // and the lifted lambda's own functype apply), so it is DROPPED here rather than making the whole
        // collection `None`. A non-Unit arg with no machine rep makes the shape unrepresentable → skip it
        // (the application declines at select). Keeping this in lockstep with `closure_type_index`'s arg
        // handling is load-bearing: a mismatch registers a functype of the wrong shape (or none), so the
        // `call_indirect` type index disagrees with the emitted body — an "indirect call type mismatch" trap.
        let arg_vts: Option<Vec<ValType>> = {
            let mut vts = Vec::new();
            let mut ok = true;
            for &a in args.iter() {
                let ty = type_of(db, a);
                if matches!(ty.strip_nominal(), Ty::Unit) {
                    continue; // Unit arg → no slot, elided
                }
                match valtype_of(&ty) {
                    Some(v) => vts.push(v),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok { Some(vts) } else { None }
        };
        if let Some(arg_vts) = arg_vts {
            let mut result_ty = type_of(db, closure);
            let mut ok = true;
            for _ in 0..args.len() {
                result_ty = match result_ty {
                    Ty::Fn(_, r) => *r,
                    _ => {
                        ok = false;
                        break;
                    }
                };
            }
            if ok {
                let mut param_vts = vec![ValType::I32]; // the env cell
                param_vts.extend(arg_vts);
                out.push((param_vts, result_ty));
            }
        }
    }
    for child in crate::backend::wasm::select::core_child_ids(db, id) {
        collect_closure_call_sigs(db, child, out);
    }
    db.walk_depth -= 1;
}

/// Force MONOMORPHIZATION to run over the whole program — lower EVERY definition body, so
/// `type_specialize` fires at every recursive-generic / type-valued / `const` call and
/// `db.instantiations` holds the complete instantiation set. This is the driver behind the
/// `Instantiations` query: unlike [`compute`], it does not gate on `(export …)` (a query is TOTAL — it
/// answers even for a module with no export) and it does not stop at the reachable set (an instantiation
/// under an unreachable-but-well-formed def is still a real instantiation the query should report — the
/// same "every def body, reachable or not" totality the diagnostics pass has). Lowering a body is
/// idempotent and memoized (the `core` column caches each node), so this is safe to call after `compute`
/// or on its own, and a second call adds nothing. Deterministic: walks `db.defs` in index order.
///
/// The walk reuses [`collect_call_callees`] purely for its side effect — it drives `core_of` over every
/// sub-position of a body, which is what triggers `type_specialize`; the collected callee vec is
/// discarded. As `type_specialize` appends specialized defs to `db.defs`, the range grows; a plain index
/// loop over the growing `len()` also lowers each freshly-synthesized specialization's body (closing over
/// transitive instantiations a specialization itself introduces).
pub fn force_monomorphize(db: &mut Db) {
    let mut i = 0;
    while i < db.defs.len() {
        if let Some(body) = db.defs[i].body {
            let mut sink = Vec::new();
            collect_call_callees(db, body, &mut sink);
            // `db.called` (the "emitted as a real function" disposition) is populated by
            // `emit_call_or_specialize` at the `Core::Call` construction site — keyed by the SOURCE callee
            // (before an accumulator/specialization transform renames it), which the reachability walk's
            // synthesized-copy index cannot recover. Here we only need the walk's SIDE EFFECT (it drives
            // `core_of`, firing `type_specialize` + `emit_call_or_specialize`); the collected `sink` is
            // discarded, exactly as before the disposition extension.
            let _ = sink;
        }
        i += 1;
    }
}

/// Collect the funcref-table slots (`Core::Closure { code }`) a body BUILDS, into `out` — the closure
/// analogue of [`collect_call_callees`]. A closure value reaching a reachable body means its lifted
/// function is genuinely used (so it must be emitted with a real body + its table entry); a lambda in
/// `db.lifted` NOT reached this way was demanded only during type-checking / a fold that erased it, so it
/// is a dead lift. Descends every sub-position (both `if` branches, arm bodies, operands) like the call
/// walk. A `Core::CallClosure` dispatches dynamically (no static code), so it adds no slot itself.
fn collect_closure_codes(db: &mut Db, id: StructId, out: &mut std::collections::HashSet<usize>) {
    // WALK-DEPTH GUARD (see [`crate::db::WALK_DEPTH_LIMIT`] and `collect_call_callees`): a non-normalizing
    // self-application in a sum-constructor payload materializes a deep `Core::SumNew` chain this walk would
    // descend until the native stack overflows. Bound the walk's OWN recursion with the dedicated
    // `walk_depth` counter (NOT `core_of`'s `descent_depth`, which this walk also drives — sharing would
    // spuriously decline a valid moderately-deep program). Past the limit stop descending; the program is
    // rejected by `collect_faults` anyway, so a clipped closure set changes no accepted program.
    if db.walk_depth >= crate::db::WALK_DEPTH_LIMIT {
        return;
    }
    // VISITED-SET (see [`Db::closure_code_visited`] and the twin in `collect_call_callees`): the closure-code
    // set is multiplicity-independent, so skipping an already-walked shared-DAG node changes no output while
    // avoiding the `O(K^depth)` re-descent. Cleared at the top-level entry (`walk_depth == 0`).
    if db.walk_depth == 0 {
        db.closure_code_visited.clear();
    }
    if !db.closure_code_visited.insert(id) {
        return;
    }
    db.walk_depth += 1;
    collect_closure_codes_at(db, id, out);
    db.walk_depth -= 1;
}

fn collect_closure_codes_at(db: &mut Db, id: StructId, out: &mut std::collections::HashSet<usize>) {
    use crate::core::Core;
    match crate::lower::core_of(db, id) {
        Core::Closure { code, captures } => {
            out.insert(code);
            for &c in captures.iter() {
                collect_closure_codes(db, c, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_closure_codes(db, closure, out);
            for &arg in args.iter() {
                collect_closure_codes(db, arg, out);
            }
        }
        Core::If { cond, then_, else_ } => {
            collect_closure_codes(db, cond, out);
            collect_closure_codes(db, then_, out);
            collect_closure_codes(db, else_, out);
        }
        Core::Let { bindings, body } => {
            for (_, value) in bindings.iter().copied() {
                collect_closure_codes(db, value, out);
            }
            collect_closure_codes(db, body, out);
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::BytesConcat { lhs, rhs }
        | Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalOfInts { num: lhs, den: rhs }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            collect_closure_codes(db, lhs, out);
            collect_closure_codes(db, rhs, out);
        }
        Core::BigIntOfI64 { value } => collect_closure_codes(db, value, out),
        Core::BigIntToI64 { operand } => collect_closure_codes(db, operand, out),
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            collect_closure_codes(db, operand, out)
        }
        Core::RationalOfIntWiden { value } => collect_closure_codes(db, value, out),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            collect_closure_codes(db, operand, out)
        }
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, elem, out);
        }
        Core::ListUpdate { list, index, elem } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, index, out);
            collect_closure_codes(db, elem, out);
        }
        Core::ListAt { list, index, .. } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, index, out);
        }
        Core::MapNew { entries, .. } => {
            for (k, v) in entries.iter().copied() {
                collect_closure_codes(db, k, out);
                collect_closure_codes(db, v, out);
            }
        }
        Core::MapInsert { map, key, val, .. } => {
            collect_closure_codes(db, map, out);
            collect_closure_codes(db, key, out);
            collect_closure_codes(db, val, out);
        }
        Core::MapLookup { map, key, .. } | Core::MapRemove { map, key, .. } => {
            collect_closure_codes(db, map, out);
            collect_closure_codes(db, key, out);
        }
        Core::MapSize { map } => collect_closure_codes(db, map, out),
        Core::SetOf { elems, .. } => {
            for &e in elems.iter() {
                collect_closure_codes(db, e, out);
            }
        }
        Core::SetContains { set, elem, .. }
        | Core::SetInsert { set, elem, .. }
        | Core::SetRemove { set, elem, .. } => {
            collect_closure_codes(db, set, out);
            collect_closure_codes(db, elem, out);
        }
        Core::SetLen { set } => collect_closure_codes(db, set, out),
        Core::SetToList { set, .. } => collect_closure_codes(db, set, out),
        Core::MapToList { map, .. } => collect_closure_codes(db, map, out),
        Core::SetAlgebra { lhs, rhs, .. } => {
            collect_closure_codes(db, lhs, out);
            collect_closure_codes(db, rhs, out);
        }
        Core::BytesAt { bytes, index, .. } => {
            collect_closure_codes(db, bytes, out);
            collect_closure_codes(db, index, out);
        }
        Core::StrAt { string, index, .. } => {
            collect_closure_codes(db, string, out);
            collect_closure_codes(db, index, out);
        }
        Core::StrScalarAt { operand, index, .. } => {
            collect_closure_codes(db, operand, out);
            collect_closure_codes(db, index, out);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            collect_closure_codes(db, string, out);
            collect_closure_codes(db, start, out);
            collect_closure_codes(db, end, out);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_closure_codes(db, bytes, out);
            collect_closure_codes(db, start, out);
            collect_closure_codes(db, len, out);
        }
        Core::BytesCompact { operand }
        | Core::Blake3Of { operand }
        | Core::AstPrint { operand, .. }
        | Core::AstEncode { operand, .. }
        | Core::AstDecode { operand, .. }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::ValueEncode { value: operand, .. }
        | Core::ValueDecode { bytes: operand, .. }
        | Core::NfcNormalize { string: operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => collect_closure_codes(db, operand, out),
        Core::Call { args, .. } | Core::HostCall { args, .. } => {
            for &a in args.iter() {
                collect_closure_codes(db, a, out);
            }
        }
        Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                collect_closure_codes(db, s, out);
            }
            collect_closure_codes(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value (a `?` operand may capture a
        // closure). BRICK 1: the tree-walk arm; the desugar + emit follow.
        Core::Block { body, .. } => collect_closure_codes(db, body, out),
        Core::Break { value } => collect_closure_codes(db, value, out),
        Core::Match { scrutinee, arms } => {
            collect_closure_codes(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_closure_codes(db, g, out);
                }
                collect_closure_codes(db, arm.body, out);
            }
        }
        Core::Record { fields } => {
            for value in fields.values() {
                collect_closure_codes(db, *value, out);
            }
        }
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for &e in elems.iter() {
                collect_closure_codes(db, e, out);
            }
        }
        Core::BinBuild { segs } => {
            for s in segs {
                collect_closure_codes(db, s.value, out);
            }
        }
        Core::BinBitsBuild { fields } => {
            for f in fields {
                collect_closure_codes(db, f.value, out);
            }
        }
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            collect_closure_codes(db, bytes, out);
            if let Some(op) = off_plus {
                collect_closure_codes(db, op, out);
            }
        }
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            collect_closure_codes(db, bytes, out);
            if let Some(op) = off_plus {
                collect_closure_codes(db, op, out);
            }
            collect_closure_codes(db, len, out);
        }
        Core::Proj { operand, .. } => collect_closure_codes(db, operand, out),
        Core::SumNew { payloads, .. } => {
            for &p in payloads.iter() {
                collect_closure_codes(db, p, out);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            collect_closure_codes(db, scrutinee, out);
            collect_cont_closure_codes(db, &root, out);
        }
        Core::MatchList { scrutinee, arms } => {
            collect_closure_codes(db, scrutinee, out);
            for arm in &arms {
                collect_closure_codes(db, arm.body, out);
            }
        }
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            collect_closure_codes(db, scrutinee, out)
        }
        // Leaves / references build no closure.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Param { .. }
        | Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// The closure-slot analogue of `collect_cont_callees` — walk a sum-match continuation for the closures
/// its arm bodies build.
fn collect_cont_closure_codes(
    db: &mut Db,
    cont: &crate::core::SumCont,
    out: &mut std::collections::HashSet<usize>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_closure_codes(db, *body, out),
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_closure_codes(db, *cond, out);
            collect_closure_codes(db, *body, out);
            collect_cont_closure_codes(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_closure_codes(db, then_, out);
            collect_cont_closure_codes(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_closure_codes(db, &arm.cont, out);
            }
        }
    }
}

/// Whether a body reaches ANY runtime `Core::Call` — a `db.defs` function call — at any sub-position.
/// The Rust `--target rust-async` backend uses this to decide whether a lambda-lifted closure body can be
/// emitted as a plain SYNC `fn`: in async mode the ONLY body-emit site that threads the gas/yield `env`
/// (and produces an `.await`) is the `Core::Call` arm — every runtime collection/heap op and a
/// `Core::CallClosure` emit identically in sync and async — so a call-free lifted body compiles verbatim as
/// sync, while a body WITH a call would name an async callee (needs `env`) and stays a clean decline (the
/// deferred boxed-future closure ABI). Reuses the exhaustive [`collect_call_callees`] walk so it can never
/// drift from the set of nodes that actually emit a call.
pub(crate) fn body_has_call(db: &mut Db, id: StructId) -> bool {
    let mut callees = Vec::new();
    collect_call_callees(db, id, &mut callees);
    !callees.is_empty()
}

/// Collect the `db.defs` indices a body CALLS at runtime — the `Core::Call` callees reached from the
/// core form at `id`, descending through every sub-position (both `if` branches are reachable code, so
/// a callee in either counts). Reads the core column on demand. A callee's OWN calls are found when it
/// is itself expanded from the worklist, so this walk does not recurse into a callee's body.
///
/// WALK-DEPTH GUARD (see [`crate::db::WALK_DEPTH_LIMIT`]): a non-normalizing self-application in a
/// SUM-CONSTRUCTOR payload — `((fn v (Some (v v))) (fn v (Some (v v))))` — β-reduces (bounded, so inference
/// declines CDZ0999) into a `Core::SumNew` chain this walk materializes+descends without bound (each
/// `core_of` on a payload β-reduces one more level, unbounded by the reduction-DEPTH guard which sum
/// construction does not hold across its payload), OVERFLOWING THE COMPILER'S STACK on `cdz compile` — the
/// shape `82410c6d` fixed for the tuple/record/list walks, now reached via the sum node. Bound the walk's
/// OWN recursion with the dedicated `walk_depth` counter — NOT `core_of`'s `descent_depth`, which this walk
/// also drives at each node (sharing would inflate `core_of`'s view and spuriously decline a valid
/// moderately-deep program). Past the limit STOP descending: this walk only gathers the reachable-callee
/// set for layout, and a callee buried past the limit belongs to a program `collect_faults` REJECTS anyway
/// (the fault walk's own descent bound clips it to a coded decline), so omitting it changes no ACCEPTED
/// program. A compiler must never crash on well-formed input, only decline or complete.
fn collect_call_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    if db.walk_depth >= crate::db::WALK_DEPTH_LIMIT {
        return;
    }
    // VISITED-SET (see [`Db::callee_visited`]): a shared core DAG (a compound reached via several
    // sub-positions, since `core_of` resolves a `Ref` to its target) would otherwise be re-walked as a tree
    // — `O(K^depth)` on a wide fan-out (the CMB1 hang). The callee set is multiplicity-independent, so
    // skipping an already-walked node changes no output. Cleared at the top-level entry (`walk_depth == 0`),
    // which is a fresh per-entry set — required because this walk runs PER-DEF and a stale set would drop a
    // later def's callees. Placed after the depth guard: a depth-clipped node is still recorded, which is
    // sound here because the clip is accepted-program-neutral (the program is rejected by `collect_faults`).
    if db.walk_depth == 0 {
        db.callee_visited.clear();
    }
    if !db.callee_visited.insert(id) {
        return;
    }
    db.walk_depth += 1;
    collect_call_callees_at(db, id, out);
    db.walk_depth -= 1;
}

fn collect_call_callees_at(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match crate::lower::core_of(db, id) {
        crate::core::Core::Call { callee, args } => {
            if !out.contains(&callee) {
                out.push(callee);
            }
            for &a in args.iter() {
                collect_call_callees(db, a, out);
            }
        }
        crate::core::Core::If { cond, then_, else_ } => {
            collect_call_callees(db, cond, out);
            collect_call_callees(db, then_, out);
            collect_call_callees(db, else_, out);
        }
        crate::core::Core::Let { bindings, body } => {
            for (_, value) in bindings.iter().copied() {
                collect_call_callees(db, value, out);
            }
            collect_call_callees(db, body, out);
        }
        crate::core::Core::Arith { lhs, rhs, .. }
        | crate::core::Core::Compare { lhs, rhs, .. }
        | crate::core::Core::StrCmp { lhs, rhs, .. }
        | crate::core::Core::FloatCompare { lhs, rhs, .. }
        | crate::core::Core::ValueEq { lhs, rhs }
        | crate::core::Core::ValueCmp { lhs, rhs, .. }
        | crate::core::Core::ValueEqShaped { lhs, rhs, .. }
        | crate::core::Core::And { lhs, rhs, .. }
        | crate::core::Core::ListConcat { lhs, rhs } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::ListPush { list, elem }
        | crate::core::Core::ListPrepend { list, elem } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, elem, out);
        }
        crate::core::Core::ListUpdate { list, index, elem } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, index, out);
            collect_call_callees(db, elem, out);
        }
        crate::core::Core::ListAt { list, index, .. } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::MapNew { entries, .. } => {
            for (k, v) in entries.iter().copied() {
                collect_call_callees(db, k, out);
                collect_call_callees(db, v, out);
            }
        }
        crate::core::Core::MapInsert { map, key, val, .. } => {
            collect_call_callees(db, map, out);
            collect_call_callees(db, key, out);
            collect_call_callees(db, val, out);
        }
        crate::core::Core::MapLookup { map, key, .. }
        | crate::core::Core::MapRemove { map, key, .. } => {
            collect_call_callees(db, map, out);
            collect_call_callees(db, key, out);
        }
        crate::core::Core::MapSize { map } => collect_call_callees(db, map, out),
        crate::core::Core::SetOf { elems, .. } => {
            for &e in elems.iter() {
                collect_call_callees(db, e, out);
            }
        }
        crate::core::Core::SetContains { set, elem, .. }
        | crate::core::Core::SetInsert { set, elem, .. }
        | crate::core::Core::SetRemove { set, elem, .. } => {
            collect_call_callees(db, set, out);
            collect_call_callees(db, elem, out);
        }
        crate::core::Core::SetLen { set } => collect_call_callees(db, set, out),
        crate::core::Core::SetToList { set, .. } => collect_call_callees(db, set, out),
        crate::core::Core::MapToList { map, .. } => collect_call_callees(db, map, out),
        crate::core::Core::SetAlgebra { lhs, rhs, .. } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::BytesAt { bytes, index, .. } => {
            collect_call_callees(db, bytes, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::StrAt { string, index, .. } => {
            collect_call_callees(db, string, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::StrScalarAt { operand, index, .. } => {
            collect_call_callees(db, operand, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::StrSlice {
            string, start, end, ..
        } => {
            collect_call_callees(db, string, out);
            collect_call_callees(db, start, out);
            collect_call_callees(db, end, out);
        }
        crate::core::Core::BytesConcat { lhs, rhs } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::BigIntBinOp { lhs, rhs, .. }
        | crate::core::Core::BigIntCmp { lhs, rhs, .. }
        | crate::core::Core::RationalOfInts { num: lhs, den: rhs }
        | crate::core::Core::RationalBinOp { lhs, rhs, .. }
        | crate::core::Core::RationalCmp { lhs, rhs, .. } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::BigIntOfI64 { value } => collect_call_callees(db, value, out),
        crate::core::Core::BigIntToI64 { operand } => collect_call_callees(db, operand, out),
        crate::core::Core::CharToInt { operand }
        | crate::core::Core::IntToCharChecked { operand, .. } => {
            collect_call_callees(db, operand, out)
        }
        crate::core::Core::RationalOfIntWiden { value } => collect_call_callees(db, value, out),
        crate::core::Core::RationalNum { operand } | crate::core::Core::RationalDen { operand } => {
            collect_call_callees(db, operand, out)
        }
        crate::core::Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_call_callees(db, bytes, out);
            collect_call_callees(db, start, out);
            collect_call_callees(db, len, out);
        }
        crate::core::Core::BytesCompact { operand }
        | crate::core::Core::Blake3Of { operand }
        | crate::core::Core::AstPrint { operand, .. }
        | crate::core::Core::AstEncode { operand, .. }
        | crate::core::Core::AstDecode { operand, .. }
        | crate::core::Core::StrFromBytes { bytes: operand, .. }
        | crate::core::Core::StrToBytes { string: operand }
        | crate::core::Core::ValueEncode { value: operand, .. }
        | crate::core::Core::ValueDecode { bytes: operand, .. }
        | crate::core::Core::NfcNormalize { string: operand } => {
            collect_call_callees(db, operand, out)
        }
        crate::core::Core::Convert { operand, .. } | crate::core::Core::Not { operand } => {
            collect_call_callees(db, operand, out)
        }
        crate::core::Core::Match { scrutinee, arms } => {
            collect_call_callees(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_call_callees(db, g, out);
                }
                collect_call_callees(db, arm.body, out);
            }
        }
        crate::core::Core::Record { fields } => {
            for value in fields.values() {
                collect_call_callees(db, *value, out);
            }
        }
        crate::core::Core::Tuple { elems }
        | crate::core::Core::ListNew { elems }
        | crate::core::Core::BytesOf { elems } => {
            for &e in elems.iter() {
                collect_call_callees(db, e, out);
            }
        }
        crate::core::Core::BinBuild { segs } => {
            for s in segs {
                collect_call_callees(db, s.value, out);
            }
        }
        crate::core::Core::BinBitsBuild { fields } => {
            for f in fields {
                collect_call_callees(db, f.value, out);
            }
        }
        crate::core::Core::BinIntRead {
            bytes, off_plus, ..
        }
        | crate::core::Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            collect_call_callees(db, bytes, out);
            if let Some(op) = off_plus {
                collect_call_callees(db, op, out);
            }
        }
        crate::core::Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            collect_call_callees(db, bytes, out);
            if let Some(op) = off_plus {
                collect_call_callees(db, op, out);
            }
            collect_call_callees(db, len, out);
        }
        crate::core::Core::Proj { operand, .. }
        | crate::core::Core::ListLen { operand }
        | crate::core::Core::BytesLen { operand }
        | crate::core::Core::StrScalarLen { operand } => collect_call_callees(db, operand, out),
        // A sum construction's payloads are unconditionally evaluated — descend for their calls.
        crate::core::Core::SumNew { payloads, .. } => {
            for &p in payloads.iter() {
                collect_call_callees(db, p, out);
            }
        }
        // A sum match: the scrutinee + every arm's continuation are reachable code (a self-call in an arm
        // is a recursion edge, like an `if` branch). A nested switch's arms recurse. A sum-payload read
        // evaluates the scrutinee.
        crate::core::Core::MatchSum { scrutinee, root } => {
            collect_call_callees(db, scrutinee, out);
            collect_cont_callees(db, &root, out);
        }
        crate::core::Core::MatchList { scrutinee, arms } => {
            collect_call_callees(db, scrutinee, out);
            for arm in &arms {
                collect_call_callees(db, arm.body, out);
            }
        }
        crate::core::Core::SumPayload { scrutinee, .. } => collect_call_callees(db, scrutinee, out),
        // `expect` evaluates its scrutinee (which may CALL — a `checked-add` composes here); the trap path
        // calls nothing.
        crate::core::Core::SumExpect { scrutinee, .. } => collect_call_callees(db, scrutinee, out),
        // A closure's captured values are unconditionally evaluated at construction — descend for their
        // calls. The lifted function's OWN body is reached via the lifted-def worklist (a lifted lambda is
        // a synthetic def added to the emission set separately), not here.
        crate::core::Core::Closure { captures, .. } => {
            for &c in captures.iter() {
                collect_call_callees(db, c, out);
            }
        }
        // A closure application evaluates the closure value and its arguments; the callee is dynamic
        // (`call_indirect`), so no static callee to add — the lifted functions are already in the set.
        crate::core::Core::CallClosure { closure, args } => {
            collect_call_callees(db, closure, out);
            for &arg in args.iter() {
                collect_call_callees(db, arg, out);
            }
        }
        // A host call OR a cross-component call dispatches to a component IMPORT (not a `db.defs`
        // function), so no static callee to add; its arguments may still reach callees.
        crate::core::Core::HostCall { args, .. } => {
            for &arg in args.iter() {
                collect_call_callees(db, arg, out);
            }
        }
        crate::core::Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                collect_call_callees(db, s, out);
            }
            collect_call_callees(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value to reach any call inside.
        crate::core::Core::Block { body, .. } => collect_call_callees(db, body, out),
        crate::core::Core::Break { value } => collect_call_callees(db, value, out),
        // Leaves and references have no sub-calls (a `Captured` read is a heap read of the env cell).
        crate::core::Core::ConstInt(_)
        | crate::core::Core::ConstRational(_, _)
        | crate::core::Core::ConstBool(_)
        | crate::core::Core::ConstStr(_)
        | crate::core::Core::ConstBytes(_)
        | crate::core::Core::ConstChar(_)
        | crate::core::Core::ConstFloat(_)
        | crate::core::Core::ConstFloatNan
        | crate::core::Core::ConstFloatInf
        | crate::core::Core::Unit
        | crate::core::Core::Trap
        | crate::core::Core::TrapDivZero
        | crate::core::Core::TrapOverflow
        | crate::core::Core::Param { .. }
        | crate::core::Core::Captured { .. }
        | crate::core::Core::LocalRef { .. }
        | crate::core::Core::Poison(_) => {}
    }
}

/// CONTENT-ADDRESSED SPEC DEDUP (transient-spec cost-cliff, op ruling B) — a CONGRUENCE / partition-
/// refinement over the reachable recursive-effectful specializations (`f#eff{n}` defs). The group fold mints
/// one spec per `(def-body-occ, handler-context-key)`, but many occurrence-distinct contexts thread to
/// STRUCTURALLY IDENTICAL specs that differ only by the `#eff{n}` occurrence id embedded in their names +
/// the names of the specs they call. Lowering every copy via `core_of` is the dominant compile cost of the
/// compiler-ml self-compile (measured 264s→39s, 6.7x, when 16 reachable specs collapse to 2). Two specs are
/// EQUIVALENT iff their bodies+sigs are structurally equal treating every `#eff` name reference UP TO
/// equivalence — a reference to X in A matches a reference to Y in B iff X and Y are themselves equivalent
/// (a congruence, computed by partition-refinement to a fixpoint, à la DFA minimization). A blanket
/// occurrence-id strip is UNSOUND (it merges two specs that call genuinely-different partners → the caller
/// recurses into the wrong partner → non-terminating; caught by db-query-diff.cdz::diff-let-agrees-int).
///
/// Returns a MERGE MAP `merged_def → representative_def` for every non-representative spec in a class of
/// size > 1. The caller drops the merged defs from `order` and points their func-index resolution at the
/// representative (so a `Core::Call{callee: merged}` resolves to the representative's emitted function — the
/// representative is structurally identical, incl. sig arity, so it serves the merged spec's callers). No
/// Core surgery: the redirect lives in the layout's `order_pos`.
fn effect_spec_merge_map(db: &mut Db, order: &[usize]) -> std::collections::HashMap<usize, usize> {
    // The reachable #eff specs, by def index, with their AST body present. Only these are candidates.
    let specs: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&d| db.defs[d].name.contains("#eff") && db.defs[d].body.is_some())
        .collect();
    if specs.len() < 2 {
        return std::collections::HashMap::new();
    }
    // name → def index, for resolving a #eff reference (self or partner) to its spec. A reference leaf may
    // carry a `$s{k}`/`$t{k}` suffix (a state/temp param name of the referenced spec's sig); strip it to the
    // bare spec name before lookup. Not every #eff name is a spec in `specs` (an unreachable partner) — such
    // a ref resolves to None and is compared by its (suffix-canonicalized) text, keeping distinct.
    let mut name_to_def: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for &d in &specs {
        name_to_def.insert(db.defs[d].name.clone(), d);
    }

    // The current class of each spec (index into a class-id space). Initialized by a coarse structural hash
    // that HOLES every #eff reference (so occurrence-only-different specs start together), then refined.
    let mut class: std::collections::HashMap<usize, u64> = std::collections::HashMap::new();
    for &d in &specs {
        let sig = db.defs[d].sig_occ;
        let body = db.defs[d].body.expect("filtered to body.is_some()");
        let mut h = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        0x5347u16.hash(&mut h); // sig tag
        spec_shape_hash(db, sig, &mut h);
        0x424fu16.hash(&mut h); // body tag
        spec_shape_hash(db, body, &mut h);
        class.insert(d, h.finish());
    }

    // Refine to a fixpoint: recompute each spec's signature from (its shape + the CURRENT class of every
    // spec it references at each #eff-ref position); two specs stay together only if these agree. A spec's
    // own self-reference resolves to its OWN current class, so recursion is handled uniformly.
    loop {
        let mut next: std::collections::HashMap<usize, u64> = std::collections::HashMap::new();
        for &d in &specs {
            let sig = db.defs[d].sig_occ;
            let body = db.defs[d].body.expect("body");
            let mut h = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            // Seed with the current class so two specs that are already distinguished stay distinct.
            class[&d].hash(&mut h);
            0x5347u16.hash(&mut h);
            spec_ref_class_hash(db, sig, &class, &name_to_def, &mut h);
            0x424fu16.hash(&mut h);
            spec_ref_class_hash(db, body, &class, &name_to_def, &mut h);
            next.insert(d, h.finish());
        }
        // Re-canonicalize the raw hashes into a stable partition: group specs by their `next` hash, and only
        // KEEP a distinction if a class actually split. We compare the induced PARTITION, not the raw hashes.
        if same_partition(&specs, &class, &next) {
            break;
        }
        class = next;
    }

    // Build the merge map: within each final class, the lowest def index is the representative; every other
    // member maps to it. Verify structural equality (occurrence-canonicalized) as a belt — a hash collision
    // then degrades to a skipped merge, never a wrong alias.
    let mut by_class: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
    for &d in &specs {
        by_class.entry(class[&d]).or_default().push(d);
    }
    let mut merged: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (_c, mut members) in by_class {
        if members.len() < 2 {
            continue;
        }
        members.sort_unstable();
        let rep = members[0];
        for &m in &members[1..] {
            if spec_congruent(db, rep, m, &class, &name_to_def) {
                merged.insert(m, rep);
            }
        }
    }
    merged
}

/// A coarse structural hash of an AST subtree that HOLES every `#eff` reference leaf (so occurrence-only-
/// different specs hash equal at the start of refinement). Non-`#eff` leaves hash by content; lists by shape.
fn spec_shape_hash(db: &Db, node: StructId, h: &mut std::collections::hash_map::DefaultHasher) {
    use crate::ast::{Leaf, Struct};
    use std::hash::Hash;
    match db.ast.get(node) {
        Struct::Atom(lid) => {
            0u8.hash(h);
            match db.ast.leaf(*lid) {
                Leaf::Name(n) if n.contains("#eff") => 9u8.hash(h), // HOLE — any #eff ref
                other => {
                    1u8.hash(h);
                    other.hash(h);
                }
            }
        }
        Struct::List(children) => {
            2u8.hash(h);
            (children.len() as u64).hash(h);
            let kids: Vec<StructId> = children.clone();
            for c in kids {
                spec_shape_hash(db, c, h);
            }
        }
    }
}

/// The refinement-step hash: like [`spec_shape_hash`] but each `#eff` reference contributes the CURRENT
/// CLASS of the spec it names (self or partner), so two specs split when a reference position points to
/// specs in different classes. A `#eff` name with a `$s`/`$t` suffix is stripped to the bare spec name for
/// the class lookup; a name not resolving to a tracked spec contributes its suffix-canonicalized text.
fn spec_ref_class_hash(
    db: &Db,
    node: StructId,
    class: &std::collections::HashMap<usize, u64>,
    name_to_def: &std::collections::HashMap<String, usize>,
    h: &mut std::collections::hash_map::DefaultHasher,
) {
    use crate::ast::{Leaf, Struct};
    use std::hash::Hash;
    match db.ast.get(node) {
        Struct::Atom(lid) => {
            0u8.hash(h);
            match db.ast.leaf(*lid) {
                Leaf::Name(n) if n.contains("#eff") => {
                    let (base, suffix) = split_eff_suffix(n);
                    match name_to_def.get(base).and_then(|d| class.get(d)) {
                        Some(&cls) => {
                            8u8.hash(h);
                            cls.hash(h);
                            suffix.hash(h); // keep $s0 vs $s1 distinct
                        }
                        None => {
                            // Not a tracked spec — compare by canonicalized text (base + suffix).
                            7u8.hash(h);
                            base.hash(h);
                            suffix.hash(h);
                        }
                    }
                }
                other => {
                    1u8.hash(h);
                    other.hash(h);
                }
            }
        }
        Struct::List(children) => {
            2u8.hash(h);
            (children.len() as u64).hash(h);
            let kids: Vec<StructId> = children.clone();
            for c in kids {
                spec_ref_class_hash(db, c, class, name_to_def, h);
            }
        }
    }
}

/// Split a `#eff` name into its bare-spec part and the `$s{k}`/`$t{k}` param suffix (if any). The suffix
/// begins at the first `$` AFTER the `#eff{digits}`; a name with no `$` has an empty suffix. E.g.
/// `type-of#eff540$s0` → (`type-of#eff540`, `$s0`); `type-of#eff540` → (`type-of#eff540`, ``).
fn split_eff_suffix(name: &str) -> (&str, &str) {
    match name.find('$') {
        Some(p) => (&name[..p], &name[p..]),
        None => (name, ""),
    }
}

/// Whether two `#eff` names refer to the same spec UP TO the current partition — same bare-spec class (or
/// same base text if untracked) AND identical `$s`/`$t` suffix.
fn eff_ref_equiv(
    a: &str,
    b: &str,
    class: &std::collections::HashMap<usize, u64>,
    name_to_def: &std::collections::HashMap<String, usize>,
) -> bool {
    let (ba, sa) = split_eff_suffix(a);
    let (bb, sb) = split_eff_suffix(b);
    if sa != sb {
        return false;
    }
    match (
        name_to_def.get(ba).and_then(|d| class.get(d)),
        name_to_def.get(bb).and_then(|d| class.get(d)),
    ) {
        (Some(ca), Some(cb)) => ca == cb,
        (None, None) => ba == bb,
        _ => false,
    }
}

/// Structural equality of two `#eff` specs UP TO the current partition — the verification belt confirming a
/// same-class pair really is congruent before merging (so a hash collision degrades to a skipped merge, not
/// a wrong alias). Compares sig then body in lockstep; `#eff` reference leaves compare via [`eff_ref_equiv`].
fn spec_congruent(
    db: &Db,
    a: usize,
    b: usize,
    class: &std::collections::HashMap<usize, u64>,
    name_to_def: &std::collections::HashMap<String, usize>,
) -> bool {
    let (sa, sb) = (db.defs[a].sig_occ, db.defs[b].sig_occ);
    let (ba, bb) = match (db.defs[a].body, db.defs[b].body) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    ast_congruent(db, sa, sb, class, name_to_def) && ast_congruent(db, ba, bb, class, name_to_def)
}

/// Lockstep structural equality of two AST subtrees, `#eff` refs compared up to the current partition.
fn ast_congruent(
    db: &Db,
    a: StructId,
    b: StructId,
    class: &std::collections::HashMap<usize, u64>,
    name_to_def: &std::collections::HashMap<String, usize>,
) -> bool {
    use crate::ast::{Leaf, Struct};
    match (db.ast.get(a), db.ast.get(b)) {
        (Struct::Atom(la), Struct::Atom(lb)) => match (db.ast.leaf(*la), db.ast.leaf(*lb)) {
            (Leaf::Name(na), Leaf::Name(nb)) => {
                let (ea, eb) = (na.contains("#eff"), nb.contains("#eff"));
                if ea && eb {
                    eff_ref_equiv(na, nb, class, name_to_def)
                } else if ea || eb {
                    false
                } else {
                    na == nb
                }
            }
            (Leaf::Name(_), _) | (_, Leaf::Name(_)) => false,
            (oa, ob) => oa == ob,
        },
        (Struct::List(ca), Struct::List(cb)) => {
            ca.len() == cb.len() && {
                let (ca, cb): (Vec<StructId>, Vec<StructId>) = (ca.clone(), cb.clone());
                ca.iter()
                    .zip(cb.iter())
                    .all(|(&x, &y)| ast_congruent(db, x, y, class, name_to_def))
            }
        }
        _ => false,
    }
}

/// Whether two class assignments induce the SAME partition over `specs` (refinement fixpoint test): the
/// equivalence "same class" is identical under `a` and `b`. Compared by mapping each spec to the set of
/// specs sharing its class and checking those groupings match.
fn same_partition(
    specs: &[usize],
    a: &std::collections::HashMap<usize, u64>,
    b: &std::collections::HashMap<usize, u64>,
) -> bool {
    // Canonicalize each partition to a map: representative(min index in class) → sorted members. Equal iff
    // the two canonical groupings are identical.
    let group = |m: &std::collections::HashMap<usize, u64>| {
        let mut by: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        for &s in specs {
            by.entry(m[&s]).or_default().push(s);
        }
        let mut canon: Vec<Vec<usize>> = by
            .into_values()
            .map(|mut v| {
                v.sort_unstable();
                v
            })
            .collect();
        canon.sort_unstable();
        canon
    };
    group(a) == group(b)
}

/// Collect the callees reachable through a sum-match CONTINUATION — a leaf's body, or a nested switch's
/// arms (each recursing). Mirrors the `MatchSum` arm walk so a self-call at any tree depth is a recursion
/// edge (the `Payload`/`Elem` steps are heap reads, no calls).
fn collect_cont_callees(db: &mut Db, cont: &crate::core::SumCont, out: &mut Vec<usize>) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_call_callees(db, *body, out),
        // A guarded arm reaches callees through its guard cond, its body, AND the fall-through.
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_call_callees(db, *cond, out);
            collect_call_callees(db, *body, out);
            collect_cont_callees(db, els, out);
        }
        // A literal test reaches callees through both continuations (the `path` walk has no calls).
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_callees(db, then_, out);
            collect_cont_callees(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_callees(db, &arm.cont, out);
            }
        }
    }
}

/// The definitions called from the body at `id` — the emitted call edges (the SAME relation the backend
/// walks). `pub(crate)` so the Rust async backend can compute its own await-call reachability (a call
/// needs `Box::pin` only if the callee's async future is self-referential). A `Core::Call` in ANY
/// position is an edge here; the caller decides which edges are awaited (a loop-group tail edge is a
/// `continue`, not an await, so the async backend prunes those before its cycle check).
pub fn callees_of(db: &mut Db, id: StructId) -> Vec<usize> {
    let mut out = Vec::new();
    collect_call_callees(db, id, &mut out);
    out
}

/// The parameters of definition `def` for INTERNAL emission — each `(name-occurrence, solved-type)`,
/// in signature order. Same as [`export_params`] but WITHOUT the boundary-representability decline: an
/// internal (non-exported) callee's parameters need only a CORE machine valtype (i32/i64), not a
/// component-boundary primitive, so a width that could not cross the boundary is still fine for a local
/// call. The name occurrence is the slot-map key (seen through a `(: a T)` annotated binder). Used by
/// the backend to select a reachable non-export function (a recursive callee) with its own local slots.
pub fn def_params(db: &mut Db, def: usize) -> Vec<(StructId, Ty)> {
    let sig_params = db.defs[def].params.clone();
    let mut out = Vec::new();
    for p in sig_params {
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(db, binder);
        out.push((binder, ty));
    }
    out
}

/// The exported parameters of definition `def` — each `(name-occurrence, solved-type)`, in signature
/// order. The name occurrence is what a body reference binds to (through a `(: a T)` binder); its type
/// is solved by `type_of` on that occurrence (the annotation type, or `Any` if unannotated). An
/// exported parameter with NO definite scalar type (an unannotated/ambiguous one, whose type has no
/// machine representation) DECLINES asking for an annotation — the backend must not invent a width the
/// program did not write (`numeric-model.md` no implicit width; the operator's "ambiguous params
/// require annotations").
pub fn export_params(db: &mut Db, def: usize, name: &str) -> Result<Vec<(StructId, Ty)>, Reject> {
    let sig_params = db.defs[def].params.clone();
    let mut out = Vec::new();
    for p in sig_params {
        // The name occurrence — bare `a`, or the inner name of an annotated binder `(: a T)`.
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(db, binder);
        // A parameter must have a machine representation to cross the boundary. Two DISTINCT causes need
        // DIFFERENT advice (the message must be actionable — `diagnostics.md` rustc-gold):
        //   - AMBIGUOUS: the type is `Any` (or still carries a free var) because nothing fixed it — an
        //     UNANNOTATED param. The fix is to ANNOTATE it (the backend must not invent a width the program
        //     did not write; `numeric-model.md` no implicit width).
        //   - NO BOUNDARY REPRESENTATION: the type is DETERMINED (annotated, ground) but is a type that
        //     simply cannot cross the component boundary — a `Char`, a bare arrow, an internal-only width.
        //     Annotating does NOT help (it is already annotated); the message must NAME the type and say it
        //     has no boundary representation, not "ambiguous — annotate it" (which sends the author to add
        //     an annotation that is already present). The scalar-`Char` export gap (v-property-testing).
        // A `Char` now has a CORE machine slot (`valtype_of(Ty::Char) == I32`, the runtime Char rep) but
        // still has NO component-BOUNDARY representation (`comp_valtype_of(Ty::Char) == None`, and unlike a
        // Record/List it does not escape via the resource `encode()` path) — so a Char EXPORT PARAM must
        // still decline with the NO-BOUNDARY-REP diagnostic (naming the type), NOT slip through the core-slot
        // gate to a later, worse decline. The runtime Char rep is for IN-BODY chars (an `if`-join, a local);
        // crossing a Char at the component boundary is a separate later increment.
        let no_boundary_rep = crate::backend::wasm::lir::valtype_of(&ty).is_none()
            || matches!(ty.strip_nominal(), Ty::Char);
        if no_boundary_rep {
            let ambiguous = matches!(ty, Ty::Any) || crate::infer::ty_has_free_var(db, &ty);
            trace!(target: "rcdzc::layout", %name, binder = binder.0, ty = %ty.render_name(&db.name_ctx()), ambiguous, "decline: exported parameter has no boundary machine type");
            let msg = if ambiguous {
                format!(
                    "export `{name}`: parameter type is ambiguous — annotate it, e.g. `(: p Int64)`"
                )
            } else {
                format!(
                    "export `{name}`: parameter type `{}` has no component-boundary representation — \
                     only the aliased integer widths, `Bool`, and `Float` cross the boundary; it is \
                     already annotated, so an annotation cannot fix this (use a boundary-representable \
                     parameter type)",
                    ty.render_name(&db.name_ctx())
                )
            };
            return Err(Reject::decline(msg).at(binder));
        }
        out.push((binder, ty));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::scalar_program;

    #[test]
    fn empty_provider_is_a_valid_zero_export_component() {
        // `compute_empty_provider` yields a valid component with ZERO exports — the empty-library provider
        // for a FULLY-INLINED shred suite (its per-test consumers import nothing from it, a no-op peer, so
        // the exec model stays uniform). Where `compute_provider_for_edges(&[])` DECLINES ("no shared
        // closure"), this succeeds with an empty layout (no exports, no reached defs).
        let (ast, _) = scalar_program();
        let mut db = Db::load(ast);
        let layout =
            compute_empty_provider(&mut db).expect("empty provider is a valid empty component");
        assert!(layout.exports.is_empty(), "empty provider has zero exports");
        assert!(
            layout.order.is_empty(),
            "empty provider reaches no defs (empty order)"
        );
        // Contrast: the edge-list provider DECLINES on an empty edge set — the case this fills.
        assert!(
            compute_provider_for_edges(&mut db, &[]).is_err(),
            "compute_provider_for_edges declines an empty edge set"
        );
    }

    #[test]
    fn one_export_by_signature() {
        let (ast, _) = scalar_program();
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        assert_eq!(layout.exports.len(), 1);
        assert_eq!(layout.exports[0].name, "main");
        assert!(layout.exports[0].params.is_empty());
        assert!(layout.exports[0].result.agrees_with(&Ty::int64()));
        // The single exported definition is wasm func 0.
        assert_eq!(layout.order, vec![0]);
        assert_eq!(layout.abs(0), Some(0));
    }

    #[test]
    fn a_recursive_callee_is_reachable_past_the_exports() {
        // `main` (def 0, the export) calls `sum-to` (def 1) — a recursive callee reached by a runtime
        // `Core::Call`. Reachability must ADD `sum-to` to the emission order after the export, and its
        // absolute index (1) is what a `call` from `main` targets.
        let ast = crate::testkit::parse(
            "(module m (def (main) (sum-to 3)) (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        let main = db.def_by_name("main").expect("main");
        let sum_to = db.def_by_name("sum-to").expect("sum-to");
        // Both emitted; the export is first (index 0), the reachable callee second.
        assert_eq!(layout.order, vec![main, sum_to]);
        assert_eq!(layout.abs(main), Some(0));
        assert_eq!(layout.abs(sum_to), Some(1));
    }

    #[test]
    fn an_uncalled_def_is_not_reachable() {
        // A def neither exported nor called is dead — it does NOT enter the emission order.
        let ast = crate::testkit::parse(
            "(module m (def (main) 42) (def (unused (: n Int64)) (+ n 1)) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        let main = db.def_by_name("main").expect("main");
        assert_eq!(layout.order, vec![main]);
        assert_eq!(layout.abs(db.def_by_name("unused").unwrap()), None);
    }

    #[test]
    fn no_export_declines() {
        // A program with a def but no export presents nothing — layout declines.
        use crate::ast::{Builder, IntValue, Leaf, Radix};
        let mut b = Builder::new();
        let module = b.name("module");
        let m = b.name("m");
        let def = b.name("def");
        let sig = {
            let main = b.name("main");
            b.list(vec![main])
        };
        let body = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let def_form = b.list(vec![def, sig, body]);
        let root = b.list(vec![module, m, def_form]);
        let mut db = Db::load(b.finish(root));
        assert!(compute(&mut db).is_err());
    }
}
