/// A standalone core module that exports a closure resource's `make` + `call` primitives, plus the
/// lifted closure body, wired through a funcref table. No heap-runtime import — the "closure cell" IS
/// the table slot (an i32), so `make` registers that slot as the resource rep and `call` dispatches
/// `call_indirect` on it directly. Imports only `resource-new`/`resource-rep` (threaded by the
/// envelope). Exports: `make : () -> i32` (rep = the table slot), `call : (i32 rep, i64 x) -> i64`.
///
///  * lifted closure `lifted(x: i64) -> i64` = `x + 1` (the body of `(fn (x) (+ x 1))`). It takes NO
///    env param here (no captures in C-HOST-1); the real compiler prepends an env cell, added in
///    C-HOST-2.
///  * `make()` → `resource.new(0)`: table slot 0 is the closure's code, so the rep IS 0. (A capturing
///    closure's rep will be a heap cell handle instead.)
///  * `call(self_handle, x)` → `resource.rep(self_handle)` recovers the rep (the table slot); push
///    `x`, then `call_indirect` the recovered slot against the lifted functype.
fn closure_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/resource-rep (i32)->i32; 1 = lifted (i64)->i64 (the call_indirect
    // functype); 2 = make ()->i32; 3 = call (i32,i64)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1 (lifted / indirect)
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 3 call
    m.section(&types);

    // Imports: resource-new (func 0), resource-rep (func 1) from "heap".
    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    // Defined funcs: lifted = 2 (type 1), make = 3 (type 2), call = 4 (type 3).
    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;

    // One funcref table of size 1, slot 0 = the lifted closure.
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    // Active element segment: table 0, offset 0, [lifted].
    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(x) = x + 1
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::I64Const(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)  — rep is the table slot (0) of the closure's code
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, x) = call_indirect[type 1](x, table_slot = resource.rep(self))
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // x (the closure arg)
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → the rep (table slot)
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component that publishes the closure resource WITH its `call` method — the
/// closure analog of `inner_reexport_component` (which re-exports `make`/`encode`). It imports the
/// resource abstractly (`SubResource`) plus `make : () -> own<t>` and `call : (self: own<t>, i64) ->
/// i64`, then re-exports the resource type DIRECTLY (publishing its identity — no `SubResource`
/// ascription, which would mint a distinct resource) and the two funcs ASCRIBED against the exported
/// identity. The outer component instantiates this with the real rep-carrying resource + lifted funcs.
fn inner_reexport_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    ); // type 0
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty)); // func 0
    // call : (self: own<0>, x: s64) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty)); // func 1
    // RE-EXPORT the resource type directly (publish `imp_t`'s identity under `t`).
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    // make ascribed against the exported identity.
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    // call ascribed against the exported identity.
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component: wraps `closure_call_core` in a resource with `make` + `call`, published
/// as `cadenza:closure/exports`. Standalone (no heap runtime) — a `heap` core-instance exports only
/// `resource-new`/`resource-rep`, which the core imports. `call` is lifted against `own<t>` for now
/// (own/no-drop; the `borrow<t>` migration is C-HOST-5, shared with the value-escape's `encode`).
fn oracle_closure_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // dtor module (imports nothing) → instantiate first → the resource type has a real dtor core-func.
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    // heap core-instance exporting the two resource intrinsics; instantiate the program core.
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, x: s64) -> s64
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    // inner re-export → cadenza:closure/exports.
    let inner_idx = c.component(inner_reexport_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// A minimal dtor core module: `t-dtor : (i32 rep) -> ()`, empty body (own/no-drop C-HOST-1 — the rep
/// is a table slot, nothing to release). Imports nothing → instantiates first. The `borrow<t>` +
/// real-drop dtor is C-HOST-5.
fn dtor_stub_module() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![]);
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("t-dtor", ExportKind::Func, 0);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

/// C-HOST-1 END-TO-END: the closure-resource oracle RUNS under wasmtime. Build the component, call
/// `make()` to get the closure resource handle, then `call(handle, 5)` — which dispatches
/// `(fn (x) (+ x 1))` through the guest's funcref table via `call_indirect` — and expect 6. This is
/// the proof that a Cadenza closure can cross to the host as a resource the host invokes.
#[test]
fn a_closure_crosses_as_a_resource_the_host_calls() {
    let comp = oracle_closure_component(&closure_call_core());
    // Validate structurally first (localize any byte/index error before wasmtime).
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("closure-resource component validates");
    // The compiled-closure RUN behavior (make → a resource handle; call(handle, 5) = 6; a fresh handle
    // called with 41 = 42) is now covered by spec/semantics/21-host-closures.sexp. Per the wasmtime
    // dev-dep drop (v-wasmtime-migration), this test keeps only the STRUCTURAL oracle-validity check
    // (wasmparser, no wasmtime); the behavioral run moved to the corpus.
}

/// COMPOUND-**ARGUMENT** oracle core (the byte anchor for a closure whose closure-argument is a
/// FIXED-SHAPE SCALAR tuple `(Tuple Int64 Int64)`, supplied by the host over the DIRECT-CALL boundary).
/// THE HYPOTHESIS UNDER TEST (attacking "needs a nonexistent value-decode runtime op"): a fixed-shape
/// scalar tuple can cross as a NATIVE component `tuple<s64,s64>` type. The canonical ABI FLATTENS a small
/// tuple (≤16 scalar fields) into its scalar core params — so the guest `call` receives the fields as
/// plain core `i64`s (NO memory, NO realloc, NO runtime decode), rebuilds the tuple cell in-guest with
/// the ORDINARY tuple-build ops (here modelled directly: the closure body sums the two fields), and
/// dispatches `call_indirect`. If wasmtime lifts `tuple<s64,s64>` → two core i64 params, this validates
/// AND runs, proving the direct-call compound-ARG decline is an implementation gap, not an ABI wall.
///
/// Standalone (no heap runtime): the closure "cell" is the funcref table slot; the lifted closure is
/// `lifted(e0: i64, e1: i64) -> i64 = e0 + e1` (the body of `(fn (p) (+ (. p 0) (. p 1)))` once `p` is
/// flattened to its two fields). `call(self, e0, e1)` = recover the slot from the rep, push `e0`,`e1`,
/// `call_indirect` the `(i64,i64)->i64` lifted functype.
fn closure_tuple_arg_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/resource-rep (i32)->i32; 1 = lifted (i64,i64)->i64 (call_indirect);
    // 2 = make ()->i32; 3 = call (i32 self, i64 e0, i64 e1)->i64 (self + the two FLATTENED tuple fields).
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types
        .ty()
        .function(vec![ValType::I64, ValType::I64], vec![ValType::I64]); // 1 lifted / indirect
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types.ty().function(
        vec![ValType::I32, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 3 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(e0, e1) = e0 + e1  (the closure `(fn (p) (+ (. p 0) (. p 1)))` over the flattened fields)
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::LocalGet(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, e0, e1) = call_indirect[type 1](e0, e1, slot = resource.rep(self))
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // e0
    call.instruction(&Instruction::LocalGet(2)); // e1
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → rep (table slot)
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the tuple-ARG closure: like `inner_reexport_component` but `call`'s
/// argument is a `tuple<s64,s64>` DEFINED TYPE (not a bare scalar). Proves the component-level type of a
/// fixed-shape-scalar compound argument is expressible and re-exportable.
fn inner_reexport_component_tuple_arg() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // the tuple<s64,s64> argument defined type (import side).
    let (tup_imp, td) = c.type_defined();
    td.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    // call : (self: own<0>, p: tuple<s64,s64>) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("p", ComponentValType::Type(tup_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    // RE-EXPORT the resource type + funcs.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (tup_exp, td2) = c.type_defined();
    td2.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("p", ComponentValType::Type(tup_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the tuple-ARG closure: like `oracle_closure_component` but `call` is
/// lifted against `(self: own<t>, p: tuple<s64,s64>) -> s64`. NO Memory/Realloc canon options — the
/// HYPOTHESIS is that wasmtime FLATTENS the small tuple into scalar core params on lift, so the core
/// `call` receives `(i32 self, i64 e0, i64 e1)` directly. If the lift required indirect (memory) passing,
/// this would fail to instantiate — the test IS the refutation attempt.
fn oracle_closure_tuple_arg_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, p: tuple<s64,s64>) -> s64  — NO canon options (flatten hypothesis).
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (tup_t, tdef) = c.type_defined();
    tdef.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("p", ComponentValType::Type(tup_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_tuple_arg());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT: does a fixed-shape scalar `tuple<s64,s64>` closure ARGUMENT cross the
/// direct-call boundary by NATIVE tuple flattening (no runtime decode)? Build the oracle, `make()` the
/// closure handle, then `call(handle, (3, 4))` supplying the tuple as a `Val::Tuple` — expect 7. If this
/// validates + runs, the direct-call compound-ARG decline is an implementation gap (the compiler can
/// hand-emit this shape), NOT an ABI wall requiring a `value-decode` op.
#[test]
fn a_fixed_shape_tuple_closure_arg_crosses_by_native_flattening() {
    // Interim (a) (operator bulk wasmtime-out push): the VALIDATE face stays as a lean Rust #[test] via
    // wasmparser (no wasmtime), proving the emitted tuple-arg closure component is a valid module. The
    // make+call(arg) BEHAVIORAL drive is the RESIDUAL, pending the corpus closure make+call host harness
    // (the resource-CALL / WASI IO-DAG scope decision); it returns to the corpus once that lands.
    let comp = oracle_closure_tuple_arg_component(&closure_tuple_arg_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("tuple-arg closure component validates");
}

/// SUM-ARG oracle core: a closure `(fn (o) (match o ((Some x) x) (None 0)))` whose ONE argument is an
/// `(Option Int64)`. The HYPOTHESIS: the canonical ABI flattens a component `option<s64>` param into TWO
/// scalar core params — `(disc: i32, payload: i64)` — with NO memory/realloc/runtime decode (a fixed-shape
/// scalar payload). So the guest `call` receives `(i32 self, i32 disc, i64 payload)` and branches on `disc`
/// (`disc == 1` → Some → return the payload; else None → return 0), matching how a guest would rebuild the
/// sum cell + dispatch a real match. If this validates + runs, an `(Option scalar)` direct-call ARG is an
/// implementation gap (decode the flattened disc/payload + `sum-new`), NOT an ABI wall needing `value-decode`.
/// Standalone (no heap): the lifted body is inlined into `call` since the match is a bare i32 branch on disc.
fn closure_option_arg_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = make ()->i32;
    // 2 = call (i32 self, i32 disc, i64 payload)->i64 (self + the FLATTENED option disc + payload).
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![], vec![ValType::I32]); // 1 make
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I64],
        vec![ValType::I64],
    ); // 2 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // make
    funcs.function(2); // call
    m.section(&funcs);
    let f_make = 2u32;
    let f_call = 3u32;

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut code = CodeSection::new();
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, disc, payload) = if disc==1 { payload } else { 0 }  — the `(match o ((Some x) x) (None 0))`
    // over the flattened option. (Some = disc 1, None = disc 0 in the built-in Option decl order.)
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // disc
    call.instruction(&Instruction::I32Const(1)); // Some's discriminant
    call.instruction(&Instruction::I32Eq);
    call.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    call.instruction(&Instruction::LocalGet(2)); // payload
    call.instruction(&Instruction::Else);
    call.instruction(&Instruction::I64Const(0));
    call.instruction(&Instruction::End);
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the OPTION-ARG closure: like `inner_reexport_component_tuple_arg` but
/// `call`'s argument is an `option<s64>` DEFINED TYPE. Proves the component-level type of a fixed-payload
/// `(Option scalar)` argument is expressible and re-exportable across the resource boundary.
fn inner_reexport_component_option_arg() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // the option<s64> argument defined type (import side).
    let (opt_imp, od_o) = c.type_defined();
    od_o.option(ComponentValType::Primitive(PrimitiveValType::S64));
    // call : (self: own<0>, o: option<s64>) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("o", ComponentValType::Type(opt_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    // RE-EXPORT the resource type + funcs.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (opt_exp, od_o2) = c.type_defined();
    od_o2.option(ComponentValType::Primitive(PrimitiveValType::S64));
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("o", ComponentValType::Type(opt_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the OPTION-ARG closure: `call` is lifted against `(self: own<t>, o:
/// option<s64>) -> s64`, NO Memory/Realloc canon options — the HYPOTHESIS is that wasmtime FLATTENS the
/// fixed-payload option into `(i32 disc, i64 payload)` core params on lift, so the guest `call` receives
/// `(i32 self, i32 disc, i64 payload)`. If the lift required indirect passing, this would fail to
/// instantiate — the test IS the refutation attempt.
fn oracle_closure_option_arg_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, o: option<s64>) -> s64  — NO canon options (flatten hypothesis).
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (opt_t, odef_o) = c.type_defined();
    odef_o.option(ComponentValType::Primitive(PrimitiveValType::S64));
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("o", ComponentValType::Type(opt_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_option_arg());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT: does an `(Option Int64)` closure ARGUMENT cross the direct-call boundary by
/// NATIVE option flattening (`option<s64>` → `(disc: i32, payload: i64)`), with no runtime decode? Build
/// the oracle, `make()` the handle, then `call(handle, Some(42))` → 42 and `call(handle, None)` → 0. If
/// this validates + runs, the `(Option scalar)` direct-call ARG decline is an implementation gap (decode
/// the flattened disc/payload + rebuild the sum via `sum-new`), NOT an ABI wall requiring `value-decode`.
#[test]
fn an_option_scalar_closure_arg_crosses_by_native_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); the make+call(arg) behavioral drive
    // is the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp = oracle_closure_option_arg_component(&closure_option_arg_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("option-arg closure component validates");
}

/// RESULT-ARG oracle core: a closure `(fn (r) (match r ((Ok x) x) ((Err e) (- 0 e))))` whose ONE argument
/// is a `(Result Int64 Int64)` — a TWO-PAYLOAD-variant sum (unlike Option's one-payload+nullary). The
/// HYPOTHESIS: it crosses as a component `result<s64, s64>` (the `0x6a` former — a general `variant` must
/// be NAMED, but `result`/`option` are anonymous-allowed), which the canonical ABI FLATTENS into `(disc:
/// i32, payload: i64)` — the payload slot the JOIN of ok/err scalars (both s64). So the guest `call`
/// receives `(i32 self, i32 disc, i64 payload)` and branches on disc (0=ok→x, 1=err→-e — the canonical
/// `result` disc order, INDEPENDENT of Cadenza's decl). If this validates + runs, a `(Result scalar
/// scalar)` arg is an implementation gap (a `result<…>` former + per-case rebuild), NOT an ABI wall.
/// Standalone (no heap): the match is a bare i32 branch on the flattened disc.
fn closure_variant_arg_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // 0 = resource-new/rep (i32)->i32; 1 = make ()->i32; 2 = call (i32 self, i32 disc, i64 payload)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![], vec![ValType::I32]); // 1 make
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I64],
        vec![ValType::I64],
    ); // 2 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // make
    funcs.function(2); // call
    m.section(&funcs);
    let f_make = 2u32;
    let f_call = 3u32;

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut code = CodeSection::new();
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, disc, payload) = if disc==0 { payload } else { 0 - payload }  — `(match r ((Ok x) x)
    // ((Err e) (- 0 e)))` over the flattened variant (Ok=disc 0, Err=disc 1).
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // disc
    call.instruction(&Instruction::I32Const(0)); // Ok's discriminant
    call.instruction(&Instruction::I32Eq);
    call.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    call.instruction(&Instruction::LocalGet(2)); // payload (x)
    call.instruction(&Instruction::Else);
    call.instruction(&Instruction::I64Const(0));
    call.instruction(&Instruction::LocalGet(2)); // payload (e)
    call.instruction(&Instruction::I64Sub); // 0 - e
    call.instruction(&Instruction::End);
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the VARIANT-ARG (Result) closure: like the option one but `call`'s
/// argument is a `variant { ok(s64), err(s64) }` DEFINED TYPE.
fn inner_reexport_component_variant_arg() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    let (var_imp, od_v) = c.type_defined();
    od_v.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("r", ComponentValType::Type(var_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (var_exp, od_v2) = c.type_defined();
    od_v2.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("r", ComponentValType::Type(var_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the VARIANT-ARG (Result) closure: `call` lifted against `(self: own<t>,
/// r: variant{ok(s64),err(s64)}) -> s64`, NO canon options — the flatten hypothesis is `(i32 disc, i64
/// payload)`.
fn oracle_closure_variant_arg_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (var_t, odef_v) = c.type_defined();
    odef_v.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("r", ComponentValType::Type(var_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_variant_arg());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT: does a `(Result Int64 Int64)` closure ARGUMENT cross by NATIVE variant
/// flattening (`variant{ok(s64),err(s64)}` → `(disc: i32, payload: i64)`)? `make()`, then
/// `call(handle, Ok(7))` → 7 and a fresh `call(h2, Err(3))` → -3. If it validates + runs, a two-payload
/// `(Result scalar scalar)` arg is an implementation gap (a `variant<…>` former + per-case rebuild), NOT
/// an ABI wall.
#[test]
fn a_result_scalar_closure_arg_crosses_by_native_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp = oracle_closure_variant_arg_component(&closure_variant_arg_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("variant-arg closure component validates");
}

/// DIFFERENT-WIDTH RESULT-ARG oracle core: a closure `(fn (r) (match r ((Ok x) x) ((Err e) (Int64.of e))))`
/// whose ONE argument is a `(Result Int64 Int32)` — the ok payload s64 (an i64 core leaf), the err payload
/// s32 (an i32 core leaf). The HYPOTHESIS about the canonical ABI's payload JOIN: `result<s64, s32>`
/// flattens to `(disc: i32, payload: i64)` — the join takes the WIDER core valtype (i64), and a narrow s32
/// err value arrives SIGN-EXTENDED into that i64 slot (wasmtime lowers the s32 into the joined i64). So the
/// guest `call` receives `(i32 self, i32 disc, i64 payload)`; the Ok arm reads the i64 directly, the Err arm
/// recovers the s32 by `i32.wrap_i64` (the low 32 bits — the value already correct because it arrived
/// sign-extended). If this validates + runs (incl. a NEGATIVE narrow err), a different-core-width `(Result
/// scalar scalar)` arg is an implementation gap (the join-width + narrow-recover), NOT an ABI wall. Test
/// closure: Ok(x:s64)→x; Err(e:s32)→(Int64.of e) = sign-extend e to i64. Standalone (no heap): a bare branch.
fn closure_diff_width_result_arg_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // 0 = resource-new/rep (i32)->i32; 1 = make ()->i32; 2 = call (i32 self, i32 disc, i64 payload)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![], vec![ValType::I32]); // 1 make
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I64],
        vec![ValType::I64],
    ); // 2 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // make
    funcs.function(2); // call
    m.section(&funcs);
    let f_make = 2u32;
    let f_call = 3u32;

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut code = CodeSection::new();
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, disc, payload) = if disc==0 { payload (Ok x:s64) }
    //                             else { (Int64.of (i32.wrap payload)) — recover s32 err then re-extend }.
    // The Err path wraps the joined i64 to its low 32 bits (the s32 e), then sign-extends back to i64 — the
    // (Int64.of e) the closure body computes. Because the value arrived sign-extended, wrap+extend is an
    // identity on the value; the point is to prove the guest can RECOVER the narrow arm from the joined slot.
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // disc
    call.instruction(&Instruction::I32Const(0)); // Ok's discriminant
    call.instruction(&Instruction::I32Eq);
    call.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    call.instruction(&Instruction::LocalGet(2)); // payload (x:s64) directly
    call.instruction(&Instruction::Else);
    call.instruction(&Instruction::LocalGet(2)); // joined payload (i64)
    call.instruction(&Instruction::I32WrapI64); // → low 32 bits = the s32 err value
    call.instruction(&Instruction::I64ExtendI32S); // (Int64.of e) — sign-extend back to i64
    call.instruction(&Instruction::End);
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The outer oracle component for the DIFFERENT-WIDTH RESULT-ARG closure: `call` lifted against `(self:
/// own<t>, r: result<s64, s32>) -> s64`, NO canon options — the flatten hypothesis is `(i32 disc, i64
/// payload)` where the payload join is the WIDER core (i64). Reuses the same inner re-export SHAPE as the
/// same-width oracle but with an s32 err side.
fn oracle_closure_diff_width_result_arg_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (var_t, odef_v) = c.type_defined();
    odef_v.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S32)),
    );
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("r", ComponentValType::Type(var_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_diff_width_result_arg());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// The inner re-export component for the DIFFERENT-WIDTH RESULT-ARG closure: like the same-width one but
/// `call`'s argument is a `result<s64, s32>` DEFINED TYPE (asymmetric payload widths).
fn inner_reexport_component_diff_width_result_arg() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    let (var_imp, od_v) = c.type_defined();
    od_v.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S32)),
    );
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("r", ComponentValType::Type(var_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (var_exp, od_v2) = c.type_defined();
    od_v2.result(
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
        Some(ComponentValType::Primitive(PrimitiveValType::S32)),
    );
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("r", ComponentValType::Type(var_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// THE REFUTATION ATTEMPT: does a `(Result Int64 Int32)` closure ARGUMENT (DIFFERENT-width payloads) cross
/// by native flattening where the payload JOIN is the WIDER core (i64)? `make()`, then `call(handle, Ok(7))`
/// → 7, a fresh `call(h2, Err(3))` → 3, and a fresh `call(h3, Err(-5))` → -5 (the NEGATIVE narrow case pins
/// the sign-extend into the joined i64). If it validates + runs, a different-core-width `(Result scalar
/// scalar)` arg is an implementation gap (join-width + narrow-recover), NOT an ABI wall.
#[test]
fn a_diff_width_result_scalar_closure_arg_crosses_by_native_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp =
        oracle_closure_diff_width_result_arg_component(&closure_diff_width_result_arg_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("diff-width result-arg closure component validates");
}

/// COMPOUND-SUM-PAYLOAD oracle core: a closure `(fn (o) (match o ((Some p) (+ (. p 0) (. p 1))) (None 0)))`
/// whose ONE argument is `(Option (Tuple Int64 Int64))` — a sum whose payload is itself a fixed-shape TUPLE
/// (not a bare scalar). The HYPOTHESIS: `option<tuple<s64,s64>>` (both formers anonymous-allowed — no variant
/// wall) flattens by the canonical ABI to `(disc: i32, f0: i64, f1: i64)` — the disc then the payload
/// tuple's OWN recursively-flattened leaves (depth-first), exactly as a bare `tuple<s64,s64>` arg flattens to
/// its 2 leaves. So the guest `call` receives `(i32 self, i32 disc, i64 f0, i64 f1)`; the Some arm rebuilds
/// the payload tuple CELL from `(f0, f1)` (arr-alloc + box + arr-set, like `emit_tuple_rebuild`) then
/// `sum-new`s the Some over that handle; the None arm builds `sum-new(None, unit)`. If it validates + runs, a
/// compound (tuple) sum payload is an implementation gap (recurse the tuple rebuild inside the sum arm), NOT
/// an ABI wall. Standalone (no heap): the lifted body sums the two flattened leaves directly (Some), 0 (None).
fn closure_option_tuple_payload_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // 0 = resource-new/rep (i32)->i32; 1 = make ()->i32;
    // 2 = call (i32 self, i32 disc, i64 f0, i64 f1)->i64 (self + flattened disc + the payload tuple's 2 leaves).
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![], vec![ValType::I32]); // 1 make
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 2 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // make
    funcs.function(2); // call
    m.section(&funcs);
    let f_make = 2u32;
    let f_call = 3u32;

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut code = CodeSection::new();
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, disc, f0, f1) = if disc==1 { f0 + f1 (Some p → p.0 + p.1) } else { 0 (None) }.
    // (component `option` sends Some=1; the flattened payload leaves f0/f1 are the Some tuple's fields.)
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // disc
    call.instruction(&Instruction::I32Const(1)); // Some's boundary disc
    call.instruction(&Instruction::I32Eq);
    call.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    call.instruction(&Instruction::LocalGet(2)); // f0
    call.instruction(&Instruction::LocalGet(3)); // f1
    call.instruction(&Instruction::I64Add); // f0 + f1
    call.instruction(&Instruction::Else);
    call.instruction(&Instruction::I64Const(0));
    call.instruction(&Instruction::End);
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the COMPOUND-SUM-PAYLOAD closure: `call`'s argument is an
/// `option<tuple<s64,s64>>` DEFINED TYPE — the inner `tuple<s64,s64>` minted first, then `option`
/// referencing it by index. Both formers are anonymous-allowed (unlike `variant`), so this validates.
fn inner_reexport_component_option_tuple_payload() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // option<tuple<s64,s64>>: mint the inner tuple first, then the option referencing it.
    let (tup_imp, od_t) = c.type_defined();
    od_t.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (opt_imp, od_o) = c.type_defined();
    od_o.option(ComponentValType::Type(tup_imp));
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("o", ComponentValType::Type(opt_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (tup_exp, od_t2) = c.type_defined();
    od_t2.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (opt_exp, od_o2) = c.type_defined();
    od_o2.option(ComponentValType::Type(tup_exp));
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("o", ComponentValType::Type(opt_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the COMPOUND-SUM-PAYLOAD closure: `call` lifted against `(self: own<t>, o:
/// option<tuple<s64,s64>>) -> s64`, NO canon options — the flatten hypothesis is `(i32 disc, i64 f0, i64
/// f1)`.
fn oracle_closure_option_tuple_payload_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (tup_t, od_t) = c.type_defined();
    od_t.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (opt_t, od_o) = c.type_defined();
    od_o.option(ComponentValType::Type(tup_t));
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("o", ComponentValType::Type(opt_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_option_tuple_payload());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT: does an `(Option (Tuple Int64 Int64))` closure ARGUMENT (a COMPOUND sum payload)
/// cross by native flattening (`option<tuple<s64,s64>>` → `(disc: i32, f0: i64, f1: i64)`)? `make()`, then
/// `call(handle, Some((3,4)))` → 7 and a fresh `call(h2, None)` → 0. If it validates + runs, a compound
/// (tuple) sum payload is an implementation gap (recurse the tuple rebuild inside the sum arm), NOT an ABI
/// wall — the natural next widening after scalar sum payloads.
#[test]
fn an_option_tuple_payload_closure_arg_crosses_by_native_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp =
        oracle_closure_option_tuple_payload_component(&closure_option_tuple_payload_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("option-tuple-payload closure component validates");
}

/// COMPOUND-RESULT-PAYLOAD oracle core: a closure `(fn (r) (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err e) (-
/// 0 e))))` whose ONE argument is `(Result (Tuple Int64 Int64) Int64)` — the OK payload a fixed-shape TUPLE,
/// the ERR payload a bare scalar. The HYPOTHESIS about the canonical ABI's variant JOIN: `result<tuple<s64,
/// s64>, s64>` flattens EACH arm's payload then JOINS position-by-position — ok flattens to `[i64, i64]`, err
/// to `[i64]`; the join is `(disc: i32, j0: i64, j1: i64)` where j0 = join(ok.0, err) and j1 = join(ok.1,
/// <none>) = ok.1. So the OK arm reads BOTH joined slots as its tuple fields; the ERR arm reads ONLY j0 (j1
/// unused). If it validates + runs, a compound Result payload is an implementation gap (per-arm rebuild over
/// the joined slots, each arm consuming a PREFIX of the join), NOT an ABI wall. Standalone (no heap): a bare
/// branch — Ok→j0+j1, Err→0-j0.
fn closure_result_tuple_payload_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // 0 = resource-new/rep (i32)->i32; 1 = make ()->i32; 2 = call (i32 self, i32 disc, i64 j0, i64 j1)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![], vec![ValType::I32]); // 1 make
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 2 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // make
    funcs.function(2); // call
    m.section(&funcs);
    let f_make = 2u32;
    let f_call = 3u32;

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut code = CodeSection::new();
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, disc, j0, j1) = if disc==0 { j0 + j1 (Ok (p.0,p.1)) } else { 0 - j0 (Err e, j0=e) }.
    // (component `result` sends Ok=0; the ok tuple's 2 fields are the joined slots j0,j1; the err scalar is
    // joined into j0.)
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // disc
    call.instruction(&Instruction::I32Const(0)); // Ok's discriminant
    call.instruction(&Instruction::I32Eq);
    call.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
    call.instruction(&Instruction::LocalGet(2)); // j0 (p.0)
    call.instruction(&Instruction::LocalGet(3)); // j1 (p.1)
    call.instruction(&Instruction::I64Add);
    call.instruction(&Instruction::Else);
    call.instruction(&Instruction::I64Const(0));
    call.instruction(&Instruction::LocalGet(2)); // j0 (e)
    call.instruction(&Instruction::I64Sub); // 0 - e
    call.instruction(&Instruction::End);
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the COMPOUND-RESULT-PAYLOAD closure: `call`'s argument is a
/// `result<tuple<s64,s64>, s64>` DEFINED TYPE (the ok side a tuple minted first, then the result referencing
/// it + a scalar err). Both `result` and `tuple` are anonymous-allowed, so this validates (unlike a variant).
fn inner_reexport_component_result_tuple_payload() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // result<tuple<s64,s64>, s64>: mint the ok tuple first, then the result referencing it + a scalar err.
    let (tup_imp, od_t) = c.type_defined();
    od_t.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (res_imp, od_r) = c.type_defined();
    od_r.result(
        Some(ComponentValType::Type(tup_imp)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("r", ComponentValType::Type(res_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (tup_exp, od_t2) = c.type_defined();
    od_t2.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (res_exp, od_r2) = c.type_defined();
    od_r2.result(
        Some(ComponentValType::Type(tup_exp)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("r", ComponentValType::Type(res_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the COMPOUND-RESULT-PAYLOAD closure: `call` lifted against `(self: own<t>,
/// r: result<tuple<s64,s64>, s64>) -> s64`, NO canon options — the flatten hypothesis is `(i32 disc, i64 j0,
/// i64 j1)` (the ok tuple's 2 leaves joined with the err scalar at position 0).
fn oracle_closure_result_tuple_payload_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (tup_t, od_t) = c.type_defined();
    od_t.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (res_t, od_r) = c.type_defined();
    od_r.result(
        Some(ComponentValType::Type(tup_t)),
        Some(ComponentValType::Primitive(PrimitiveValType::S64)),
    );
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("r", ComponentValType::Type(res_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_result_tuple_payload());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT: does a `(Result (Tuple Int64 Int64) Int64)` closure ARGUMENT (a COMPOUND ok
/// payload joined with a scalar err) cross by native flattening (`result<tuple<s64,s64>, s64>` → `(disc: i32,
/// j0: i64, j1: i64)`, ok reading both joined slots, err reading j0)? `make()`, then `call(handle, Ok((3,4)))`
/// → 7 and a fresh `call(h2, Err(5))` → -5. If it validates + runs, a compound Result payload is an
/// implementation gap (per-arm rebuild over the joined slots), NOT an ABI wall.
#[test]
fn a_result_tuple_payload_closure_arg_crosses_by_native_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp =
        oracle_closure_result_tuple_payload_component(&closure_result_tuple_payload_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("result-tuple-payload closure component validates");
}

/// NESTED-COMPOUND oracle core: a closure `(fn (p) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))` whose ONE
/// argument is a NESTED fixed-shape tuple `(Tuple Int64 (Tuple Int64 Int64))`. The HYPOTHESIS: the canonical
/// ABI flattens a nested `tuple<s64, tuple<s64,s64>>` RECURSIVELY into THREE leaf core params `(i64, i64,
/// i64)` (depth-first), so the guest `call` receives `(i32 self, i64 a, i64 b, i64 c)` — NO memory, NO
/// realloc, NO runtime decode. If this validates + runs, a nested fixed-shape compound arg is an
/// implementation gap (recursive rebuild), NOT an ABI wall. Standalone (no heap): the lifted closure is
/// `lifted(a,b,c) = a + b + c` (the body once the nested tuple is flattened to its 3 leaves).
fn closure_nested_tuple_arg_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64,i64,i64)->i64 (call_indirect);
    // 2 = make ()->i32; 3 = call (i32 self, i64 a, i64 b, i64 c)->i64 (self + the 3 FLATTENED leaf fields).
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(
        vec![ValType::I64, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 1 lifted / indirect
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types.ty().function(
        vec![ValType::I32, ValType::I64, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 3 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(a, b, c) = a + b + c
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::LocalGet(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::LocalGet(2));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, a, b, c) = call_indirect[type 1](a, b, c, slot = resource.rep(self))
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // a
    call.instruction(&Instruction::LocalGet(2)); // b
    call.instruction(&Instruction::LocalGet(3)); // c
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → rep (table slot)
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the NESTED-tuple-ARG closure: `call`'s argument is a
/// `tuple<s64, tuple<s64,s64>>` DEFINED TYPE (a nested tuple). Proves a nested fixed-shape-scalar compound
/// argument's component type is expressible + re-exportable (the inner tuple is its own defined type,
/// referenced by index from the outer tuple).
fn inner_reexport_component_nested_tuple_arg() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // the nested tuple<s64, tuple<s64,s64>> argument (import side): mint the INNER tuple first, then the
    // outer tuple referencing it by index.
    let (inner_imp, itd) = c.type_defined();
    itd.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_imp, td) = c.type_defined();
    td.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Type(inner_imp),
    ]);
    // call : (self: own<0>, p: tuple<s64, tuple<s64,s64>>) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("p", ComponentValType::Type(tup_imp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    // RE-EXPORT the resource type + funcs.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (inner_exp, itd2) = c.type_defined();
    itd2.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_exp, td2) = c.type_defined();
    td2.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Type(inner_exp),
    ]);
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("p", ComponentValType::Type(tup_exp)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the NESTED-tuple-ARG closure: `call` is lifted against `(self: own<t>,
/// p: tuple<s64, tuple<s64,s64>>) -> s64` with NO Memory/Realloc canon options — the HYPOTHESIS is that
/// wasmtime RECURSIVELY flattens the nested tuple into THREE scalar core params, so the core `call`
/// receives `(i32 self, i64 a, i64 b, i64 c)`. If the lift required indirect (memory) passing, this fails
/// to instantiate — the test IS the refutation attempt.
fn oracle_closure_nested_tuple_arg_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, p: tuple<s64, tuple<s64,s64>>) -> s64 — NO canon options (flatten hyp).
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (inner_t, itdef) = c.type_defined();
    itdef.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_t, tdef) = c.type_defined();
    tdef.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Type(inner_t),
    ]);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("p", ComponentValType::Type(tup_t)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_nested_tuple_arg());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT for a NESTED fixed-shape compound arg: does `tuple<s64, tuple<s64,s64>>` cross
/// the direct-call boundary by RECURSIVE native flattening (no runtime decode)? `make()` the handle, then
/// `call(handle, (100, (10, 3)))` supplying the nested tuple as a `Val::Tuple` containing a `Val::Tuple` —
/// expect 113. If this validates + runs, the nested-compound-ARG decline is an implementation gap (a
/// recursive rebuild), NOT an ABI wall requiring `value-decode`.
#[test]
fn a_nested_fixed_shape_tuple_closure_arg_crosses_by_recursive_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp = oracle_closure_nested_tuple_arg_component(&closure_nested_tuple_arg_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("nested-tuple-arg closure component validates");
}

/// TWO-COMPOUND-ARGS oracle core: a closure `(fn (p q) (+ (. p 0) (. p 1) (. q 0) (. q 1)))` taking TWO
/// fixed-shape `tuple<s64,s64>` args. The HYPOTHESIS: the canonical ABI flattens EACH tuple independently,
/// so the guest `call` receives `(i32 self, i64 a, i64 b, i64 c, i64 d)` — the first tuple's fields then
/// the second's, NO memory / realloc / runtime decode. If this validates + runs, N compound args is an
/// implementation gap (a `Vec` of rebuilds, one per tuple), NOT an ABI wall. Standalone (no heap): the
/// lifted closure is `lifted(a,b,c,d) = a+b+c+d` (the body once both tuples are flattened to their fields).
fn closure_two_tuple_args_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64×4)->i64 (call_indirect); 2 = make ()->i32;
    // 3 = call (i32 self, i64 a, i64 b, i64 c, i64 d)->i64 (self + the two tuples' FLATTENED fields).
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(
        vec![ValType::I64, ValType::I64, ValType::I64, ValType::I64],
        vec![ValType::I64],
    ); // 1 lifted / indirect
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types.ty().function(
        vec![
            ValType::I32,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I64,
        ],
        vec![ValType::I64],
    ); // 3 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(a, b, c, d) = a + b + c + d
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::LocalGet(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::LocalGet(2));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::LocalGet(3));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, a, b, c, d) = call_indirect[type 1](a, b, c, d, slot = resource.rep(self))
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // a
    call.instruction(&Instruction::LocalGet(2)); // b
    call.instruction(&Instruction::LocalGet(3)); // c
    call.instruction(&Instruction::LocalGet(4)); // d
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → rep (table slot)
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the TWO-tuple-ARG closure: `call`'s two arguments are each a
/// `tuple<s64,s64>` DEFINED TYPE. Proves two independent `tuple<…>` args are expressible + re-exportable.
fn inner_reexport_component_two_tuple_args() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // two tuple<s64,s64> argument types (import side).
    let (tup_p, tdp) = c.type_defined();
    tdp.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_q, tdq) = c.type_defined();
    tdq.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    // call : (self: own<0>, p: tuple<s64,s64>, q: tuple<s64,s64>) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("p", ComponentValType::Type(tup_p)),
        ("q", ComponentValType::Type(tup_q)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty));
    // RE-EXPORT the resource type + funcs.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (tup_pe, tdpe) = c.type_defined();
    tdpe.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_qe, tdqe) = c.type_defined();
    tdqe.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("p", ComponentValType::Type(tup_pe)),
        ("q", ComponentValType::Type(tup_qe)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer oracle component for the TWO-tuple-ARG closure: `call` lifted against `(self: own<t>, p:
/// tuple<s64,s64>, q: tuple<s64,s64>) -> s64` with NO Memory/Realloc — the HYPOTHESIS is that wasmtime
/// flattens EACH tuple into two scalar core params, so the core `call` receives `(i32 self, i64,i64,i64,i64)`.
fn oracle_closure_two_tuple_args_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (tup_p, tdp) = c.type_defined();
    tdp.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (tup_q, tdq) = c.type_defined();
    tdq.tuple([
        ComponentValType::Primitive(PrimitiveValType::S64),
        ComponentValType::Primitive(PrimitiveValType::S64),
    ]);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("p", ComponentValType::Type(tup_p)),
        ("q", ComponentValType::Type(tup_q)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(inner_reexport_component_two_tuple_args());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// THE REFUTATION ATTEMPT for N compound args: do TWO fixed-shape `tuple<s64,s64>` closure ARGUMENTS cross
/// by INDEPENDENT native flattening (no runtime decode)? `make()` the handle, then `call(handle, (3,4),
/// (10,20))` → 3+4+10+20 = 37. If this validates + runs, N compound args is an implementation gap (a `Vec`
/// of `TupleArgRebuild`, one per tuple, each at its own `base_param`), NOT an ABI wall.
#[test]
fn two_fixed_shape_tuple_closure_args_cross_by_independent_flattening() {
    // Interim (a): VALIDATE-only Rust face (wasmparser, no wasmtime); make+call(arg) drive is
    // the RESIDUAL pending the corpus closure make+call host harness. (See a_fixed_shape_tuple.)
    let comp = oracle_closure_two_tuple_args_component(&closure_two_tuple_args_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("two-tuple-arg closure component validates");
}

/// COMPOUND-RESULT oracle core (the byte anchor for a closure whose result is a `list<u8>` / a compound
/// rendered as the canonical value form): a closure `call : (self: own<t>, x: s64) -> list<u8>`. The
/// core carries a MEMORY + `cabi_realloc` (which a scalar `call` does not need) so the canonical ABI can
/// return the `(ptr, len)` area. `call` dispatches the lifted closure via `call_indirect` (recovering the
/// code slot from the resource rep, exactly as the scalar `call` does) to compute a byte `n`, then writes
/// a 2-byte payload `[n, n+1]` + the return area and returns the retptr — the same `(ptr, len)` shape
/// `working_list_core` proves for a nullary `-> list<u8>`, now behind the closure resource's `call`.
/// This isolates the ONE new thing over the scalar closure oracle: the `call` lift carries Memory/Realloc
/// canon options and a `list<u8>` result, so the compiler's real compound-result `call` can hand-emit it.
fn closure_list_call_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64)->i64 (the call_indirect functype);
    // 2 = make ()->i32; 3 = call (i32 self, i64 x)->i32 retptr; 4 = cabi_realloc (i32×4)->i32.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1 lifted / indirect
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I32]); // 3 call → retptr
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    ); // 4 cabi_realloc
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    // Defined funcs: lifted = 2, make = 3, call = 4, cabi_realloc = 5.
    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    funcs.function(4); // cabi_realloc
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;
    let f_realloc = 5u32;

    // Core section ORDER: table (sec 4) precedes memory (sec 5).
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    exports.export("cabi_realloc", ExportKind::Func, f_realloc);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(x) = x + 1 — the closure body (the same one the scalar oracle dispatches).
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::I64Const(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0).
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, x): n = call_indirect(lifted)(x) via the recovered slot; write [n, n+1] at 0, the
    // return area [ptr=0, len=2] at 8, return retptr=8. One i32 scratch local for the low byte of n.
    let mut call = Function::new(vec![(1, ValType::I32)]); // local 2 = i32 scratch
    call.instruction(&Instruction::LocalGet(1)); // x
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → slot
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::I32WrapI64); // n (i32)
    call.instruction(&Instruction::LocalSet(2));
    // mem[0] = n (low byte)
    call.instruction(&Instruction::I32Const(0));
    call.instruction(&Instruction::LocalGet(2));
    call.instruction(&Instruction::I32Store8(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
    // mem[1] = n+1 (low byte)
    call.instruction(&Instruction::I32Const(1));
    call.instruction(&Instruction::LocalGet(2));
    call.instruction(&Instruction::I32Const(1));
    call.instruction(&Instruction::I32Add);
    call.instruction(&Instruction::I32Store8(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
    // return area at 8: [ptr=0, len=2]
    call.instruction(&Instruction::I32Const(8));
    call.instruction(&Instruction::I32Const(0));
    call.instruction(&Instruction::I32Store(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }));
    call.instruction(&Instruction::I32Const(12));
    call.instruction(&Instruction::I32Const(2));
    call.instruction(&Instruction::I32Store(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }));
    call.instruction(&Instruction::I32Const(8)); // return the retarea pointer
    call.instruction(&Instruction::End);
    code.function(&call);
    // cabi_realloc stub (not called for this fixed-size return area).
    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);
    code.function(&realloc);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the COMPOUND-RESULT closure: `make : () -> own<t>` and
/// `call : (self: own<t>, x: s64) -> list<u8>`. Mirrors `inner_reexport_component` but the `call` result
/// is a `list<u8>` defined type instead of `s64`. The list type is minted on both the import and export
/// sides (each component-type space is independent). The outer component lifts `call` with Memory/Realloc
/// canon options; here the import/export functypes just declare the `list<u8>` result shape.
fn inner_reexport_component_list() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    ); // type 0
    // make : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty)); // func 0
    // call : (self: own<0>, x: s64) -> list<u8>
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (list_imp, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Type(list_imp)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty)); // func 1
    // RE-EXPORT the resource type directly.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    // make ascribed.
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    // call ascribed (list<u8> result).
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (list_exp, ld2) = c.type_defined();
    ld2.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Type(list_exp)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer COMPOUND-RESULT oracle: wraps `closure_list_call_core` in a resource with `make` + a
/// `call` that returns `list<u8>`. Like `oracle_closure_component` but the `call` lift carries
/// Memory/Realloc canon options (the compound result crosses through linear memory by the canonical
/// ABI), and the inner re-export component types `call`'s result as `list<u8>`.
fn oracle_closure_list_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, x: s64) -> list<u8>  WITH Memory/Realloc options.
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (list_u8, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Type(list_u8)));
    let call_comp = c.lift_func(
        call_core,
        call_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    let inner_idx = c.component(inner_reexport_component_list());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// COMPOUND-RESULT END-TO-END ORACLE: a closure whose `call` returns `list<u8>` runs under wasmtime.
/// `make()` → a handle; `call(handle, 5)` dispatches the lifted `(x) -> x+1` (n = 6) and returns the
/// bytes `[6, 7]` through the canonical `(ptr, len)` ABI (Memory/Realloc lift). Proves the ONE new
/// piece over the scalar closure resource — a `call` returning a compound via linear memory — before the
/// compiler hand-emits a real compound-result `call`. Byte-shape validated + behavior observed.
#[test]
fn a_closure_returning_a_list_crosses_and_the_host_reads_the_bytes() {
    let comp = oracle_closure_list_component(&closure_list_call_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("compound-result closure component validates");
    // The compiled-closure list-result RUN behavior (make → handle; call(handle, 5) returns the byte
    // payload [6, 7]) is now covered by spec/semantics/21-host-closures.sexp. Per the wasmtime dev-dep
    // drop (v-wasmtime-migration), this keeps only the STRUCTURAL oracle-validity check (wasmparser).
}

/// MULTI-EXPORT oracle core (the byte anchor for the next real increment): TWO closures of the SAME
/// signature `(-> Int64 Int64)` in one funcref table (slot 0 = `(fn (x) (+ x 1))`, slot 1 = `(fn (x)
/// (* x 3))`), TWO `make` functions (`make-inc` → `resource.new(0)`, `make-triple` → `resource.new(1)`),
/// and ONE SHARED `call`. The shared `call` is the load-bearing realization: because the closure's code
/// slot is recovered from the resource rep at call time (`resource.rep` → `call_indirect`), a single
/// `call` dispatches ANY closure of the signature regardless of which `make` built it. So N same-
/// signature exports need N `make`s + 1 `call` + 1 resource type — the compiler-side envelope this
/// oracle licenses. Exports: `make-inc : () -> i32`, `make-triple : () -> i32`, `call : (i32, i64) -> i64`.
fn multi_closure_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64)->i64 (the shared call_indirect functype);
    // 2 = make ()->i32; 3 = call (i32,i64)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 3 call
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    // Defined funcs: lifted-inc = 2, lifted-triple = 3, make-inc = 4, make-triple = 5, call = 6.
    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted-inc
    funcs.function(1); // lifted-triple
    funcs.function(2); // make-inc
    funcs.function(2); // make-triple
    funcs.function(3); // call
    m.section(&funcs);
    let f_lifted_inc = 2u32;
    let f_lifted_triple = 3u32;
    let f_make_inc = 4u32;
    let f_make_triple = 5u32;
    let f_call = 6u32;

    // A funcref table of size 2: slot 0 = inc, slot 1 = triple.
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 2,
        maximum: Some(2),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make-inc", ExportKind::Func, f_make_inc);
    exports.export("make-triple", ExportKind::Func, f_make_triple);
    exports.export("call", ExportKind::Func, f_call);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted_inc, f_lifted_triple])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted-inc(x) = x + 1
    let mut li = Function::new(vec![]);
    li.instruction(&Instruction::LocalGet(0));
    li.instruction(&Instruction::I64Const(1));
    li.instruction(&Instruction::I64Add);
    li.instruction(&Instruction::End);
    code.function(&li);
    // lifted-triple(x) = x * 3
    let mut lt = Function::new(vec![]);
    lt.instruction(&Instruction::LocalGet(0));
    lt.instruction(&Instruction::I64Const(3));
    lt.instruction(&Instruction::I64Mul);
    lt.instruction(&Instruction::End);
    code.function(&lt);
    // make-inc() = resource.new(0)  — slot 0's code is `inc`
    let mut mi = Function::new(vec![]);
    mi.instruction(&Instruction::I32Const(0));
    mi.instruction(&Instruction::Call(f_rnew));
    mi.instruction(&Instruction::End);
    code.function(&mi);
    // make-triple() = resource.new(1)  — slot 1's code is `triple`
    let mut mt = Function::new(vec![]);
    mt.instruction(&Instruction::I32Const(1));
    mt.instruction(&Instruction::Call(f_rnew));
    mt.instruction(&Instruction::End);
    code.function(&mt);
    // call(self, x) = call_indirect[type 1](x, slot = resource.rep(self)) — SHARED across both makes.
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1)); // x
    call.instruction(&Instruction::LocalGet(0)); // self handle
    call.instruction(&Instruction::Call(f_rrep)); // → the slot (0 or 1)
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the MULTI-EXPORT oracle: imports the abstract resource + `make-inc`,
/// `make-triple` (both `() -> own<t>`), and the shared `call : (self: own<t>, s64) -> s64`, then
/// re-exports the resource type DIRECTLY + all three funcs ascribed against the exported identity. The
/// closure analog of `inner_reexport_component` but with two `make`s sharing one `call` — the multi-
/// export shape a compiler envelope emits for two same-signature closure exports.
fn multi_inner_reexport_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    ); // type 0
    // make-inc / make-triple : () -> own<0>
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_inc_fn = c.import("import-func-make-inc", ComponentTypeRef::Func(make_ty)); // func 0
    let (own_imp_b, odb) = c.type_defined();
    odb.own(imp_t);
    let (make_ty_b, mut mfb) = c.type_function();
    mfb.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp_b)));
    let make_triple_fn = c.import("import-func-make-triple", ComponentTypeRef::Func(make_ty_b)); // func 1
    // call : (self: own<0>, x: s64) -> s64
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_imp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty)); // func 2
    // RE-EXPORT the resource type directly.
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    // make-inc ascribed.
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_inc_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make-inc",
        ComponentExportKind::Func,
        make_inc_fn,
        Some(ComponentTypeRef::Func(make_inc_exp_ty)),
    );
    // make-triple ascribed.
    let (own_exp_b, od3b) = c.type_defined();
    od3b.own(exp_t);
    let (make_triple_exp_ty, mut mf2b) = c.type_function();
    mf2b.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp_b)));
    c.export(
        "make-triple",
        ComponentExportKind::Func,
        make_triple_fn,
        Some(ComponentTypeRef::Func(make_triple_exp_ty)),
    );
    // call ascribed (shared).
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (call_exp_ty, mut cf2) = c.type_function();
    cf2.params([
        ("self", ComponentValType::Type(own_exp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call",
        ComponentExportKind::Func,
        call_fn,
        Some(ComponentTypeRef::Func(call_exp_ty)),
    );
    c
}

/// The outer MULTI-EXPORT oracle: wraps `multi_closure_core` in a resource with `make-inc`,
/// `make-triple`, and a shared `call`, published as `cadenza:closure/exports`. Standalone (no heap
/// runtime — the cell IS the table slot). Proves the multi-export shape composes and runs.
fn oracle_multi_closure_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_inc_core = c.core_alias_export(prog_inst, "make-inc", ExportKind::Func);
    let make_triple_core = c.core_alias_export(prog_inst, "make-triple", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    // lift make-inc : () -> own<t>
    let (own_a, oda) = c.type_defined();
    oda.own(res_ty);
    let (make_inc_ty, mut mfa) = c.type_function();
    mfa.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_a)));
    let make_inc_comp = c.lift_func(make_inc_core, make_inc_ty, []);
    // lift make-triple : () -> own<t>
    let (own_b, odb) = c.type_defined();
    odb.own(res_ty);
    let (make_triple_ty, mut mfb) = c.type_function();
    mfb.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_b)));
    let make_triple_comp = c.lift_func(make_triple_core, make_triple_ty, []);
    // lift call : (self: own<t>, x: s64) -> s64
    let (own_c, odc) = c.type_defined();
    odc.own(res_ty);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_c)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    let inner_idx = c.component(multi_inner_reexport_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            (
                "import-func-make-inc",
                ComponentExportKind::Func,
                make_inc_comp,
            ),
            (
                "import-func-make-triple",
                ComponentExportKind::Func,
                make_triple_comp,
            ),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// MULTI-EXPORT end-to-end ORACLE: two closure exports (`make-inc`, `make-triple`) of the same
/// signature share ONE `call`, and the host drives each (`make-inc()` then `call(_, 5)` yields 6;
/// `make-triple()` then `call(_, 5)` yields 15). Proves that a single shared `call` correctly
/// dispatches whichever closure a given handle names (the code slot travels in the resource rep).
/// This is the byte anchor licensing the compiler's multi-export envelope (the hand-emitted
/// production path is the next increment).
#[test]
fn multi_export_closures_share_one_call_and_the_host_drives_each() {
    let comp = oracle_multi_closure_component(&multi_closure_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("multi-export closure component validates");
    // The multi-export shared-call RUN behavior (make-inc → call(_,5)=6; make-triple → the SAME shared
    // call dispatches (* x 3), call(_,5)=15) is now covered by spec/semantics/21-host-closures.sexp.
    // Per the wasmtime dev-dep drop, this keeps only the STRUCTURAL oracle-validity check (wasmparser).
}

/// MIXED-EXPORT oracle core (the byte anchor for "a closure ALONGSIDE a non-closure export"): ONE
/// closure export `adder : () -> (-> Int64 Int64)` (slot 0 = `(fn (x) (+ x 1))`, `make`/`call`) PLUS a
/// PLAIN scalar export `two : () -> i64` (returns 2). Both live in the SAME core module (one program
/// instance): the module exports `make`, `call`, AND `two`. The outer component publishes the closure
/// `make`/`call` under the `cadenza:closure/exports` instance AND `two` as an ORDINARY top-level
/// component func — proving the resource envelope and the plain boundary compose in one component.
fn mixed_closure_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64)->i64; 2 = make ()->i32;
    // 3 = call (i32,i64)->i64; 4 = two ()->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1 (lifted / indirect)
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 3 call
    types.ty().function(vec![], vec![ValType::I64]); // 4 two
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    // Defined funcs: lifted = 2, make = 3, call = 4, two = 5.
    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // call
    funcs.function(4); // two
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_call = 4u32;
    let f_two = 5u32;

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("call", ExportKind::Func, f_call);
    exports.export("two", ExportKind::Func, f_two);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(x) = x + 1
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::I64Const(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // call(self, x) = call_indirect[type 1](x, resource.rep(self))
    let mut call = Function::new(vec![]);
    call.instruction(&Instruction::LocalGet(1));
    call.instruction(&Instruction::LocalGet(0));
    call.instruction(&Instruction::Call(f_rrep));
    call.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    call.instruction(&Instruction::End);
    code.function(&call);
    // two() = 2
    let mut two = Function::new(vec![]);
    two.instruction(&Instruction::I64Const(2));
    two.instruction(&Instruction::End);
    code.function(&two);
    m.section(&code);
    m.finish()
}

/// The outer MIXED-EXPORT oracle: wraps `mixed_closure_core` so the closure `make`/`call` publish under
/// `cadenza:closure/exports` while the plain `two` publishes as an ORDINARY top-level component func
/// `two : () -> s64`. Standalone (no heap runtime — the cell IS the table slot). The byte anchor for the
/// compiler emitting a closure export alongside a non-closure export.
fn oracle_mixed_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
    let two_core = c.core_alias_export(prog_inst, "two", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift call : (self: own<t>, x: s64) -> s64
    let (own_t2, odef2) = c.type_defined();
    odef2.own(res_ty);
    let (call_ty, mut cf) = c.type_function();
    cf.params([
        ("self", ComponentValType::Type(own_t2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let call_comp = c.lift_func(call_core, call_ty, []);
    // lift two : () -> s64  (a PLAIN top-level export, no resource envelope).
    let (two_ty, mut tf) = c.type_function();
    tf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let two_comp = c.lift_func(two_core, two_ty, []);
    // inner re-export → cadenza:closure/exports.
    let inner_idx = c.component(inner_reexport_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-call", ComponentExportKind::Func, call_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    // The plain scalar export, published DIRECTLY at the top level.
    c.export("two", ComponentExportKind::Func, two_comp, None);
    c.finish()
}

/// MIXED-EXPORT END-TO-END ORACLE: a program that exports a closure resource (`make`/`call`) ALONGSIDE
/// a plain scalar export (`two : () -> 2`) validates + runs. Proves the closure interface instance and
/// an ordinary top-level func coexist in one component — the byte anchor licensing the compiler's
/// mixed-export envelope (the hand-emitted production path is the next increment).
#[test]
fn a_closure_export_and_a_plain_export_coexist_and_the_host_drives_both() {
    let comp = oracle_mixed_component(&mixed_closure_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("mixed-export component validates");
    // The RUN behavior (the plain top-level export two() = 2 AND the closure interface make/call(5) = 6
    // coexisting + both driven in ONE component) is now covered by spec/semantics/21-host-closures.sexp
    // ("a closure export alongside a plain scalar export -- the plain export runs" + "-- the closure
    // runs"). Per the wasmtime dev-dep drop (v-wasmtime-migration), this keeps only the STRUCTURAL
    // oracle-validity check (wasmparser, no wasmtime) — proving the closure envelope + the plain boundary
    // compose in one valid component.
}

/// DISTINCT-SIGNATURE oracle core (the byte anchor for the N-resource-type multi-export): TWO closures
/// of DIFFERENT signatures — `inc : (-> Int64 Int64)` (slot 0) and `isz : (-> Int64 Bool)` (slot 1) —
/// each with its OWN `make` + `call` (distinct call functypes: `(i32,i64)->i64` vs `(i32,i64)->i32`).
/// Two funcref-table slots, ONE guest table (both lifteds live in it), but the boundary needs TWO
/// resource types (one per signature) since `own<t0>` and `own<t1>` are distinct. Exports: `make-inc`,
/// `call-inc`, `make-isz`, `call-isz`.
fn distinct_sig_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted-inc (i64)->i64; 2 = lifted-isz (i64)->i32;
    // 3 = make ()->i32; 4 = call-inc (i32,i64)->i64; 5 = call-isz (i32,i64)->i32.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1 inc
    types.ty().function(vec![ValType::I64], vec![ValType::I32]); // 2 isz (bool→i32)
    types.ty().function(vec![], vec![ValType::I32]); // 3 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 4 call-inc
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I32]); // 5 call-isz
    m.section(&types);

    // Import resource-new/rep for BOTH resource types: t0's (new=0, rep=1) and t1's (new=2, rep=3).
    // A core `resource.new`/`resource.rep` is typed to ONE resource type, so `make-isz` must new a t1
    // handle through t1's intrinsic (the rep is a plain table slot, but the resource-TYPE distinction
    // is real at the canon boundary).
    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    imports.import("heap", "resource-new-t1", EntityType::Function(0));
    imports.import("heap", "resource-rep-t1", EntityType::Function(0));
    m.section(&imports);
    let f_rnew0 = 0u32;
    let f_rrep0 = 1u32;
    let f_rnew1 = 2u32;
    let f_rrep1 = 3u32;

    // Defined funcs: lifted-inc=4(ty1), lifted-isz=5(ty2), make-inc=6(ty3), make-isz=7(ty3),
    // call-inc=8(ty4), call-isz=9(ty5).
    let mut funcs = FunctionSection::new();
    funcs.function(1);
    funcs.function(2);
    funcs.function(3);
    funcs.function(3);
    funcs.function(4);
    funcs.function(5);
    m.section(&funcs);
    let (f_lifted_inc, f_lifted_isz) = (4u32, 5u32);
    let (f_make_inc, f_make_isz, f_call_inc, f_call_isz) = (6u32, 7u32, 8u32, 9u32);

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 2,
        maximum: Some(2),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make-inc", ExportKind::Func, f_make_inc);
    exports.export("call-inc", ExportKind::Func, f_call_inc);
    exports.export("make-isz", ExportKind::Func, f_make_isz);
    exports.export("call-isz", ExportKind::Func, f_call_isz);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted_inc, f_lifted_isz])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted-inc(x) = x + 1
    let mut li = Function::new(vec![]);
    li.instruction(&Instruction::LocalGet(0));
    li.instruction(&Instruction::I64Const(1));
    li.instruction(&Instruction::I64Add);
    li.instruction(&Instruction::End);
    code.function(&li);
    // lifted-isz(x) = (x == 0) as i32
    let mut lz = Function::new(vec![]);
    lz.instruction(&Instruction::LocalGet(0));
    lz.instruction(&Instruction::I64Eqz);
    lz.instruction(&Instruction::End);
    code.function(&lz);
    // make-inc() = resource.new-t0(slot 0)
    let mut mi = Function::new(vec![]);
    mi.instruction(&Instruction::I32Const(0));
    mi.instruction(&Instruction::Call(f_rnew0));
    mi.instruction(&Instruction::End);
    code.function(&mi);
    // make-isz() = resource.new-t1(slot 1)
    let mut mz = Function::new(vec![]);
    mz.instruction(&Instruction::I32Const(1));
    mz.instruction(&Instruction::Call(f_rnew1));
    mz.instruction(&Instruction::End);
    code.function(&mz);
    // call-inc(self, x) = call_indirect[ty1](x, resource.rep-t0(self))
    let mut ci = Function::new(vec![]);
    ci.instruction(&Instruction::LocalGet(1));
    ci.instruction(&Instruction::LocalGet(0));
    ci.instruction(&Instruction::Call(f_rrep0));
    ci.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    ci.instruction(&Instruction::End);
    code.function(&ci);
    // call-isz(self, x) = call_indirect[ty2](x, resource.rep-t1(self))
    let mut cz = Function::new(vec![]);
    cz.instruction(&Instruction::LocalGet(1));
    cz.instruction(&Instruction::LocalGet(0));
    cz.instruction(&Instruction::Call(f_rrep1));
    cz.instruction(&Instruction::CallIndirect {
        type_index: 2,
        table_index: 0,
    });
    cz.instruction(&Instruction::End);
    code.function(&cz);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the DISTINCT-SIGNATURE oracle: imports TWO abstract resources
/// (`import-type-t0`, `import-type-t1`) + each signature's `make`/`call` typed against its own
/// resource, then re-exports BOTH resources (`t0`, `t1`) + all four funcs ascribed. Proves a single
/// interface can publish two resource-with-methods of distinct signatures. `t0` = `(-> Int64 Int64)`,
/// `t1` = `(-> Int64 Bool)`.
fn distinct_sig_inner_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // Import the two abstract resources → types 0, 1.
    let imp_t0 = c.import(
        "import-type-t0",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let imp_t1 = c.import(
        "import-type-t1",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    // make-inc : () -> own<t0>
    let (own_mi, o) = c.type_defined();
    o.own(imp_t0);
    let (mi_ty, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_mi)));
    let mi_fn = c.import("import-func-make-inc", ComponentTypeRef::Func(mi_ty));
    // call-inc : (self: own<t0>, s64) -> s64
    let (own_ci, o) = c.type_defined();
    o.own(imp_t0);
    let (ci_ty, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own_ci)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let ci_fn = c.import("import-func-call-inc", ComponentTypeRef::Func(ci_ty));
    // make-isz : () -> own<t1>
    let (own_mz, o) = c.type_defined();
    o.own(imp_t1);
    let (mz_ty, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_mz)));
    let mz_fn = c.import("import-func-make-isz", ComponentTypeRef::Func(mz_ty));
    // call-isz : (self: own<t1>, s64) -> bool
    let (own_cz, o) = c.type_defined();
    o.own(imp_t1);
    let (cz_ty, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own_cz)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::Bool)));
    let cz_fn = c.import("import-func-call-isz", ComponentTypeRef::Func(cz_ty));
    // RE-EXPORT both resources directly.
    let exp_t0 = c.export("t0", ComponentExportKind::Type, imp_t0, None);
    let exp_t1 = c.export("t1", ComponentExportKind::Type, imp_t1, None);
    // Ascribe + export each func against the exported resource identities.
    let (own, o) = c.type_defined();
    o.own(exp_t0);
    let (t, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own)));
    c.export(
        "make-inc",
        ComponentExportKind::Func,
        mi_fn,
        Some(ComponentTypeRef::Func(t)),
    );
    let (own, o) = c.type_defined();
    o.own(exp_t0);
    let (t, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "call-inc",
        ComponentExportKind::Func,
        ci_fn,
        Some(ComponentTypeRef::Func(t)),
    );
    let (own, o) = c.type_defined();
    o.own(exp_t1);
    let (t, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own)));
    c.export(
        "make-isz",
        ComponentExportKind::Func,
        mz_fn,
        Some(ComponentTypeRef::Func(t)),
    );
    let (own, o) = c.type_defined();
    o.own(exp_t1);
    let (t, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::Bool)));
    c.export(
        "call-isz",
        ComponentExportKind::Func,
        cz_fn,
        Some(ComponentTypeRef::Func(t)),
    );
    c
}

/// The outer DISTINCT-SIGNATURE oracle: wraps `distinct_sig_core` in TWO resource types (one per
/// signature) with their make/call pairs, published together under `cadenza:closure/exports`.
fn oracle_distinct_sig_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // Two dtor stubs → two resource types (each rep i32).
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst0 = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor0 = c.core_alias_export(dtor_inst0, "t-dtor", ExportKind::Func);
    let res_t0 = c.type_resource(ValType::I32, Some(dtor0));
    let dtor_inst1 = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor1 = c.core_alias_export(dtor_inst1, "t-dtor", ExportKind::Func);
    let res_t1 = c.type_resource(ValType::I32, Some(dtor1));
    // Both resources' new/rep share the guest heap instance (the reps are into the one funcref table).
    let rnew0 = c.resource_new(res_t0);
    let rrep0 = c.resource_rep(res_t0);
    let rnew1 = c.resource_new(res_t1);
    let rrep1 = c.resource_rep(res_t1);
    // The core imports ONE resource-new + ONE resource-rep (it makes both t0 and t1 handles through the
    // same canon intrinsics — the rep is just a table slot; the resource TYPE distinction is a
    // boundary/type-level concern, not a core one). Bind them to t0's intrinsics (t0 and t1 share the
    // core rep space — a slot is a slot). Wait: a core resource.new is typed to ONE resource; make-isz
    // must new a t1. So the core needs resource-new-t0/resource-rep-t0 AND -t1. Provide all four.
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew0),
        ("resource-rep", ExportKind::Func, rrep0),
        ("resource-new-t1", ExportKind::Func, rnew1),
        ("resource-rep-t1", ExportKind::Func, rrep1),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let get =
        |c: &mut ComponentBuilder, n: &str| c.core_alias_export(prog_inst, n, ExportKind::Func);
    let mi_core = get(&mut c, "make-inc");
    let ci_core = get(&mut c, "call-inc");
    let mz_core = get(&mut c, "make-isz");
    let cz_core = get(&mut c, "call-isz");
    // lift make-inc : () -> own<t0>
    let (own, o) = c.type_defined();
    o.own(res_t0);
    let (t, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own)));
    let mi_comp = c.lift_func(mi_core, t, []);
    // lift call-inc : (own<t0>, s64) -> s64
    let (own, o) = c.type_defined();
    o.own(res_t0);
    let (t, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let ci_comp = c.lift_func(ci_core, t, []);
    // lift make-isz : () -> own<t1>
    let (own, o) = c.type_defined();
    o.own(res_t1);
    let (t, mut f) = c.type_function();
    f.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own)));
    let mz_comp = c.lift_func(mz_core, t, []);
    // lift call-isz : (own<t1>, s64) -> bool
    let (own, o) = c.type_defined();
    o.own(res_t1);
    let (t, mut f) = c.type_function();
    f.params([
        ("self", ComponentValType::Type(own)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::Bool)));
    let cz_comp = c.lift_func(cz_core, t, []);
    let inner_idx = c.component(distinct_sig_inner_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t0", ComponentExportKind::Type, res_t0),
            ("import-type-t1", ComponentExportKind::Type, res_t1),
            ("import-func-make-inc", ComponentExportKind::Func, mi_comp),
            ("import-func-call-inc", ComponentExportKind::Func, ci_comp),
            ("import-func-make-isz", ComponentExportKind::Func, mz_comp),
            ("import-func-call-isz", ComponentExportKind::Func, cz_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// DISTINCT-SIGNATURE end-to-end ORACLE (the byte anchor for N-resource-type multi-export): two
/// closures of DIFFERENT signatures cross as TWO resource types in one interface. `make-inc()` +
/// `call-inc(_, 5)` = 6 (an `(-> Int64 Int64)`); `make-isz()` + `call-isz(_, 0)` = true (an `(-> Int64
/// Bool)`). Proves a single `cadenza:closure/exports` can publish resources of distinct signatures,
/// each with its own `make`/`call` typed against its own resource. Licenses the compiler's N-resource
/// multi-export envelope (the hand-emitted path is a later increment).
#[test]
fn distinct_signature_closures_cross_as_distinct_resource_types() {
    let comp = oracle_distinct_sig_component(&distinct_sig_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("distinct-signature closure component validates");
    // The distinct-signature RUN behavior (make-inc → call-inc(_,5)=6 on resource t0; make-isz →
    // call-isz(_,0)=true on a DISTINCT resource t1 with a Bool result) is now covered by
    // spec/semantics/21-host-closures.sexp. Per the wasmtime dev-dep drop, this keeps only the
    // STRUCTURAL oracle-validity check (wasmparser, no wasmtime).
}

/// ROUND-TRIP oracle core (the byte anchor for C-HOST-4, Direction 2): a `make` that produces a
/// closure resource + a SEPARATE `apply` CONSUMER that takes the closure resource as `own<t>` plus an
/// arg, recovers the cell (`resource.rep`), and dispatches it (`call_indirect`). Unlike the single/
/// multi-export `call` (which is a resource METHOD on the same resource `make` produced), `apply`
/// models a DISTINCT export whose PARAMETER is a closure — the host threads a handle from `make` back
/// into `apply`. The dispatch is identical (`resource.rep` → slot → `call_indirect`); what is new is
/// that the handle ORIGINATES in one export call and is CONSUMED by another. Exports: `make : () -> i32`
/// (rep = table slot 0), `apply : (i32 self, i64 x) -> i64`.
fn roundtrip_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // Types: 0 = resource-new/rep (i32)->i32; 1 = lifted (i64)->i64; 2 = make ()->i32; 3 = apply
    // (i32,i64)->i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
    types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1
    types.ty().function(vec![], vec![ValType::I32]); // 2 make
    types
        .ty()
        .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 3 apply
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    imports.import("heap", "resource-rep", EntityType::Function(0));
    m.section(&imports);
    let f_rnew = 0u32;
    let f_rrep = 1u32;

    // Defined funcs: lifted = 2 (type 1), make = 3 (type 2), apply = 4 (type 3).
    let mut funcs = FunctionSection::new();
    funcs.function(1); // lifted
    funcs.function(2); // make
    funcs.function(3); // apply
    m.section(&funcs);
    let f_lifted = 2u32;
    let f_make = 3u32;
    let f_apply = 4u32;

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        minimum: 1,
        maximum: Some(1),
        table64: false,
        shared: false,
    });
    m.section(&tables);

    let mut exports = ExportSection::new();
    exports.export("make", ExportKind::Func, f_make);
    exports.export("apply", ExportKind::Func, f_apply);
    m.section(&exports);

    let mut elems = ElementSection::new();
    elems.active(
        Some(0),
        &ConstExpr::i32_const(0),
        Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
    );
    m.section(&elems);

    let mut code = CodeSection::new();
    // lifted(x) = x + 1
    let mut lifted = Function::new(vec![]);
    lifted.instruction(&Instruction::LocalGet(0));
    lifted.instruction(&Instruction::I64Const(1));
    lifted.instruction(&Instruction::I64Add);
    lifted.instruction(&Instruction::End);
    code.function(&lifted);
    // make() = resource.new(0)
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(0));
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);
    code.function(&make);
    // apply(self, x) = call_indirect[type 1](x, slot = resource.rep(self)) — the CONSUMER: it takes a
    // closure resource as a parameter and dispatches it. This is what a Cadenza `(def (apply (: g (->
    // Int64 Int64)) (: x Int64)) (g x))` lowers to at the boundary.
    let mut apply = Function::new(vec![]);
    apply.instruction(&Instruction::LocalGet(1)); // x
    apply.instruction(&Instruction::LocalGet(0)); // self handle
    apply.instruction(&Instruction::Call(f_rrep)); // → the slot
    apply.instruction(&Instruction::CallIndirect {
        type_index: 1,
        table_index: 0,
    });
    apply.instruction(&Instruction::End);
    code.function(&apply);
    m.section(&code);
    m.finish()
}

/// The inner re-export component for the ROUND-TRIP oracle: imports the abstract resource + `make : ()
/// -> own<t>` + `apply : (self: own<t>, s64) -> s64`, then re-exports the resource + both funcs
/// ascribed. Structurally like the single-export inner component but the second func is `apply` (a
/// consumer taking the resource as a parameter) rather than `call` (a method) — the byte shapes are the
/// same (both take `own<t>` first); the semantic difference is what the outer host does with it.
fn roundtrip_inner_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    ); // type 0
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty)); // func 0
    let (own_imp2, od2) = c.type_defined();
    od2.own(imp_t);
    let (apply_ty, mut af) = c.type_function();
    af.params([
        ("g", ComponentValType::Type(own_imp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let apply_fn = c.import("import-func-apply", ComponentTypeRef::Func(apply_ty)); // func 1
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od3) = c.type_defined();
    od3.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (own_exp2, od4) = c.type_defined();
    od4.own(exp_t);
    let (apply_exp_ty, mut af2) = c.type_function();
    af2.params([
        ("g", ComponentValType::Type(own_exp2)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    c.export(
        "apply",
        ComponentExportKind::Func,
        apply_fn,
        Some(ComponentTypeRef::Func(apply_exp_ty)),
    );
    c
}

/// The outer ROUND-TRIP oracle: wraps `roundtrip_core` in a resource with `make` + `apply`, published
/// as `cadenza:closure/exports`. Standalone (no heap runtime). Proves the host can produce a closure
/// handle from one export and thread it into another.
fn oracle_roundtrip_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let dtor_idx = c.core_module_raw(&dtor_stub_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let heap_inst = c.core_instantiate_exports([
        ("resource-new", ExportKind::Func, rnew_core),
        ("resource-rep", ExportKind::Func, rrep_core),
    ]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let apply_core = c.core_alias_export(prog_inst, "apply", ExportKind::Func);
    // lift make : () -> own<t>
    let (own_a, oda) = c.type_defined();
    oda.own(res_ty);
    let (make_ty, mut mfa) = c.type_function();
    mfa.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_a)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    // lift apply : (g: own<t>, x: s64) -> s64
    let (own_b, odb) = c.type_defined();
    odb.own(res_ty);
    let (apply_ty, mut afb) = c.type_function();
    afb.params([
        ("g", ComponentValType::Type(own_b)),
        ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
    ])
    .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let apply_comp = c.lift_func(apply_core, apply_ty, []);
    let inner_idx = c.component(roundtrip_inner_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-apply", ComponentExportKind::Func, apply_comp),
        ],
    );
    c.export(
        "cadenza:closure/exports",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c.finish()
}

/// ROUND-TRIP end-to-end ORACLE (C-HOST-4): the host produces a closure handle from `make()` and
/// threads it BACK into a separate consumer `apply(handle, 5)` = 6. Proves host-as-custodian: a closure
/// crosses to the host as a resource and is handed back into another Cadenza export, which recovers the
/// cell and dispatches it via the guest's own `call_indirect`. The byte anchor licensing the compiler's
/// closure-parameter path (the hand-emitted `own<closure>` param ABI is the next increment).
#[test]
fn a_closure_handle_round_trips_through_a_consumer_export() {
    let comp = oracle_roundtrip_component(&roundtrip_core());
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("round-trip closure component validates");
    // The RUN behavior (make → a closure resource handle the host holds; apply(handle, 5) threads it BACK
    // into the consumer export and dispatches it = (+ x 1)(5) = 6 — the round trip) is now covered by
    // spec/semantics/21-host-closures.sexp ("a produced closure is handed back into a consumer export (the
    // round trip)" + non-leading/multi variants). Per the wasmtime dev-dep drop (v-wasmtime-migration),
    // this keeps only the STRUCTURAL oracle-validity check (wasmparser, no wasmtime).
}

/// C-HOST-1 (compiler serializer): `serialize::closure_resource_core_module` — the PRODUCTION core
/// module a closure-resource export emits — produces a structurally VALID core module. Unlike the
/// standalone oracle above (whose "cell" is the bare table slot), this is the real shape: the export
/// body builds a value-heap CELL (`arr-alloc`/`box-int`/`arr-set`), and `call` recovers it
/// (`resource.rep`) + reads the code slot (`arr-get`/`get-int`) + `call_indirect`s the lifted body. It
/// imports the heap ops + `resource-new`/`resource-rep`, and carries the funcref table from
/// `layout.lifted`. This pins the serializer's byte shape (validated by `wasmparser`) with real,
/// minimally-constructed `SelectedFunc`s — the runnable end-to-end path (wiring this core into the
/// production envelope + composing the runtime) is the next increment.
#[test]
fn closure_resource_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    // The heap ops the core imports, in the SORTED order `collect_used_ops` (a BTreeSet) produces:
    // arr-alloc, arr-get, arr-set, box-int, get-int. `call` uses arr-get/get-int; the export body's
    // cell build uses arr-alloc/arr-set/box-int.
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    // Defined func 0 = the export body `main : () -> own<closure>` — its RESULT is the closure type,
    // whose machine valtype is an i32 cell handle. Build a 1-slot cell holding box-int(0) (the
    // closure's table slot 0, no captures), returning the cell handle. (Mirrors `Core::Closure`
    // selection.) Uses arr-alloc/box-int/arr-set.
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    let export_body = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(), // a closure result → an i32 cell handle in the type section
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Defined func 1 = the lifted closure `(env: i32, x: i64) -> i64` = x + 1.
    let lifted_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![export_body, lifted_body];

    // Layout: one export (def 0), order [0, 1] (export then lifted-as-def), one lifted lambda in the
    // table. The lifted lambda's `body`/`params` here are placeholder (StructId 0 / the arg types) —
    // only its presence in `layout.lifted` matters for the table + `lifted_type_index`.
    let export = ExportPlan {
        name: "main".to_string(),
        def: 0,
        body: crate::ast::StructId(0),
        params: vec![],
        result: Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone())),
    };
    let lifted = LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: s64.clone(),
        captures: vec![],
    };
    let import_base = imports.len() as u32 + 2; // k ops + resource-new + resource-rep
    // `order` is just the ONE export def; the lifted closure is appended to `funcs` AFTER the order
    // defs (as the production path does), so its abs index + type index are `import_base +
    // order.len() + slot`. `funcs = [export_body, lifted_body]` matches this layout.
    let layout = Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);

    // The export body is at emission position 0 → absolute core-func index `import_base + 0`.
    let export_abs = import_base;
    // The lifted functype index (env i32, x i64) -> i64 in the core type section.
    let lifted_type_idx = layout.lifted_type_index(0, import_base);

    let core = crate::backend::wasm::serialize::closure_resource_core_module(
        &funcs,
        &imports,
        export_abs,
        &[ValType::I64],
        ValType::I64,
        &[], // nullary export → make() has no params
        lifted_type_idx,
        &layout,
    )
    .expect("closure-resource core serializes");

    // The emitted CORE MODULE must be structurally valid wasm (funcref table + call_indirect + the
    // resource-intrinsic imports all well-formed). This is the compiler-side byte proof; the runnable
    // end-to-end path (below) wires the WHOLE pipeline through the composed runtime.
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("closure-resource core module validates");
}

/// BRICK B (compound closure ARG, DIRECT-CALL path): the production serializer's `call` rebuilds a
/// FLATTENED fixed-shape scalar tuple argument into the single value-heap CELL its lifted closure body
/// expects — `arr-alloc N` + per field (index, the flattened core param, box, `arr-set`) — then dispatches
/// `call_indirect`, and drops the rebuilt cell afterward (an owned per-call temporary). Models
/// `(def (mk) (fn (p) (+ (. p 0) (. p 1))))` whose closure arg `(Tuple Int64 Int64)` crosses the boundary
/// FLATTENED into two `s64` core params (`arg_vts = [I64, I64]`) — the `a_fixed_shape_tuple_closure_arg_
/// crosses_by_native_flattening` oracle proved that ABI runs. This pins the serializer's byte shape
/// (validated by `wasmparser`); the runnable end-to-end path wires this core into the tuple-typed
/// envelope (Brick C) + `emit_closure_resource` routing (Brick D). `TupleArgRebuild = None` is byte-
/// identical to the scalar path (the 53 closure tests above prove that).
#[test]
fn closure_tuple_arg_resource_core_rebuilds_the_cell_and_validates() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::{
        ClosureMake, FieldRebuild, TupleArgRebuild,
        multi_closure_resource_core_module_with_host_borrow,
    };
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let tuple_ty = Ty::Tuple(vec![s64.clone(), s64.clone()].into());
    // The closure is `(-> (Tuple Int64 Int64) Int64)`; the lifted body reads its arg (a tuple cell) via
    // two projections. Imports (SORTED, as `collect_used_ops` produces): arr-alloc, arr-get, arr-set,
    // box-int, drop, get-int. The `call` rebuild uses arr-alloc/box-int/arr-set/drop; the lifted body's
    // projections use arr-get/get-int; the make cell build arr-alloc/box-int/arr-set.
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    // Defined func 0 = the export body `mk : () -> own<closure>` — build the 1-slot code cell (slot 0),
    // returning the cell handle (no captures). Uses arr-alloc/box-int/arr-set.
    let fn_ty = Ty::Fn(Box::new(tuple_ty.clone()), Box::new(s64.clone()));
    let export_body = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Defined func 1 = the lifted closure `(env: i32, p: i32) -> i64` = `(. p 0) + (. p 1)` — two tuple
    // projections over the REBUILT cell `call` hands it: arr-get(p,i) → get-int, added.
    let lifted_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I32], // env cell, the tuple arg cell
        ret: s64.clone(),
        code: vec![
            Lir::LocalGet(1),
            Lir::ConstI32(0),
            Lir::CallImport("arr-get"),
            Lir::CallImport("get-int"),
            Lir::LocalGet(1),
            Lir::ConstI32(1),
            Lir::CallImport("arr-get"),
            Lir::CallImport("get-int"),
            Lir::I64Add,
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![export_body, lifted_body];

    let export = ExportPlan {
        name: "mk".to_string(),
        def: 0,
        body: crate::ast::StructId(0),
        params: vec![],
        result: fn_ty.clone(),
    };
    // The lifted lambda takes ONE param — the tuple cell (an i32 handle) — NOT the two flattened fields;
    // the `call` wrapper does the reassembly, so the `call_indirect` functype is `(env i32, p i32) -> i64`.
    let lifted = LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), tuple_ty.clone())],
        ret_ty: s64.clone(),
        captures: vec![],
    };
    let import_base = imports.len() as u32 + 2;
    let layout = Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);
    let export_abs = import_base;
    let lifted_type_idx = layout.lifted_type_index(0, import_base);

    // The tuple `(Tuple Int64 Int64)` crosses FLATTENED as two s64 core params; each field boxes with
    // `box-int` (which takes an i64). A 64-bit field's core param is ALREADY i64, so NO i32→i64 extend
    // (`extend = None`); the extend is only for a NARROW int field (an i32-width core param).
    // The `call`'s boundary/core arg list is thus `[I64, I64]` (the flattened fields), NOT one i32 handle.
    let rebuild = TupleArgRebuild {
        fields: vec![
            FieldRebuild::Scalar {
                box_op: "box-int",
                extend: None,
            },
            FieldRebuild::Scalar {
                box_op: "box-int",
                extend: None,
            },
        ],
        base_param: 1, // the tuple is the sole closure arg → leaves at core params 1..1+N
    };
    let core = multi_closure_resource_core_module_with_host_borrow(
        &funcs,
        &imports,
        &[],
        &[ClosureMake {
            export_name: "make".to_string(),
            export_abs,
            param_vts: vec![],
        }],
        &[],
        &[ValType::I64, ValType::I64], // the FLATTENED tuple fields
        ValType::I64,
        lifted_type_idx,
        &layout,
        false,
        std::slice::from_ref(&rebuild),
        &[],
    )
    .expect("tuple-arg closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("tuple-arg closure-resource core module validates");
}

/// HOST-COMPOSED closure-resource core (`multi_closure_resource_core_module_with_host`): the build-time-
/// delegated closure-capture shape — a closure export whose `make` body performs a host call
/// (`(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))`). Host ops are laid FIRST (core funcs `0..h`),
/// so a `Lir::CallHostImport(0)` in the export body resolves to core func 0 verbatim (the same invariant
/// the plain `emit` path relies on) with NO index recomputation; the runtime cell ops shift to `h..h+k`
/// and the resource intrinsics to `h+k`,`h+k+1`. Pins that the host-threaded layout emits a
/// STRUCTURALLY VALID core (the `CallHostImport` index, the shifted runtime-op `CallImport` indices, the
/// resource intrinsics, and the `call_indirect` functype are all consistent). This is brick 1 of the
/// closure-capture feature — the envelope (importing the `host` interface) + `emit_closure_resource`
/// wiring are the following bricks.
#[test]
fn closure_resource_core_with_host_is_structurally_valid() {
    use crate::backend::wasm::host::{HostImport, HostParam};
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    // ONE host op `log.emit : () -> ()` (no params, UNIT result — leaves nothing on the stack, so the
    // minimal make body needs no drop) laid at core func 0. The point of this test is the LAYOUT/index
    // consistency of a `CallHostImport` in a closure-make body, not a specific op signature.
    let host_fns = vec![HostImport {
        effect: "log".to_string(),
        op: "emit".to_string(),
        params: Vec::<HostParam>::new(),
        result: None,
        spilled_result: None,
        enum_result: None,
    }];
    // The runtime cell ops (make builds the capturing cell; call recovers it). Shift to h..h+k.
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let h = host_fns.len();
    // Export body `main : () -> own<closure>`: call the host op (`CallHostImport(0)` → the captured v),
    // then build a 1-slot cell holding box-int(0) — a minimal make that both performs a host call AND
    // builds the closure cell. (The host result is dropped here; the real body captures it — this test
    // only pins the LAYOUT/index consistency, not the capture dataflow.)
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    let export_body = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::CallHostImport(0), // log.emit() → () (host func 0; leaves nothing on the stack)
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let lifted_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![export_body, lifted_body];
    let export = ExportPlan {
        name: "main".to_string(),
        def: 0,
        body: crate::ast::StructId(0),
        params: vec![],
        result: fn_ty.clone(),
    };
    let lifted = LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: s64.clone(),
        captures: vec![],
    };
    // import_base = h host + k runtime + 2 resource intrinsics.
    let import_base = (h + imports.len() + 2) as u32;
    let layout = Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);
    let export_abs = import_base;
    let lifted_type_idx = layout.lifted_type_index(0, import_base);

    let core = crate::backend::wasm::serialize::multi_closure_resource_core_module_with_host(
        &funcs,
        &imports,
        &host_fns,
        &[crate::backend::wasm::serialize::ClosureMake {
            export_name: "make".to_string(),
            export_abs,
            param_vts: vec![],
        }],
        &[],
        &[ValType::I64],
        ValType::I64,
        lifted_type_idx,
        &layout,
    )
    .expect("host-composed closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("host-composed closure-resource core module validates");
}

/// BRICK (c): the HOST+runtime closure-resource ENVELOPE (`envelope::assemble_closure_host_runtime_resource`)
/// wraps the brick-(b) core into a VALID component that wasmtime parses. The component imports BOTH the
/// host effect interface (as `host`) AND the value-heap runtime (as `heap`), aliases+lowers both op
/// sets, threads them into the program instance, and re-exports the `make`/`call` closure interface —
/// the fusion of `assemble_closure_resource` (closure machinery) + `assemble_host_runtime` (dual import).
/// This pins the component-index arithmetic (host instance-type 0, runtime 1, resource type 2, own/make/
/// call types 3..6, make/call comp funcs h+k/h+k+1, core instances host 0 / heap 3 / program 4). The
/// `emit_closure_resource` wiring that drives real programs through it is the next brick.
#[test]
fn closure_host_runtime_resource_envelope_is_a_valid_component() {
    use crate::backend::wasm::host::{HostImport, HostParam};
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let host_fns_imp = vec![HostImport {
        effect: "log".to_string(),
        op: "emit".to_string(),
        params: Vec::<HostParam>::new(),
        result: None, // () -> () — leaves nothing on the stack
        spilled_result: None,
        enum_result: None,
    }];
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let h = host_fns_imp.len();
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    let export_body = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::CallHostImport(0), // log.emit() (host func 0)
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let lifted_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![export_body, lifted_body];
    let export = ExportPlan {
        name: "main".to_string(),
        def: 0,
        body: crate::ast::StructId(0),
        params: vec![],
        result: fn_ty.clone(),
    };
    let lifted = LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: s64.clone(),
        captures: vec![],
    };
    let import_base = (h + imports.len() + 2) as u32;
    let layout = Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);
    let export_abs = import_base;
    let lifted_type_idx = layout.lifted_type_index(0, import_base);

    let core = crate::backend::wasm::serialize::multi_closure_resource_core_module_with_host(
        &funcs,
        &imports,
        &host_fns_imp,
        &[crate::backend::wasm::serialize::ClosureMake {
            export_name: "make".to_string(),
            export_abs,
            param_vts: vec![],
        }],
        &[],
        &[ValType::I64],
        ValType::I64,
        lifted_type_idx,
        &layout,
    )
    .expect("host-composed closure core serializes");

    // The envelope needs `HostFn` (with the op's COMPONENT functype). A nullary Unit-result op's
    // comp functype item is `COMP_FUNCTYPE_FORM, 0 params, 0x01 0x00 (no result)`.
    let comp_ft = {
        use crate::backend::wasm::wasm_abi;
        let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
        item.extend_from_slice(&[0x00]); // 0 params
        item.extend_from_slice(&[0x01, 0x00]); // no result
        item
    };
    let host_fns = vec![crate::backend::wasm::envelope::HostFn {
        op: "emit".to_string(),
        comp_functype: comp_ft,
        core_functype: Vec::new(),
        has_list_param: false,
    }];
    let dtor = crate::backend::wasm::serialize::resource_dtor_module_with_drop();
    let s64_comp = crate::backend::wasm::runtime_abi::AbiValType::S64.comp_byte();
    let component = crate::backend::wasm::envelope::assemble_closure_host_runtime_resource(
        &core,
        &dtor,
        &imports,
        "cadenza:runtime/heap@0.0.0",
        "log",
        &host_fns,
        &[],         // nullary make (no export params)
        &[s64_comp], // one s64 closure arg (component primitive byte)
        s64_comp,    // s64 result (component primitive byte)
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&component)
        .expect("the host+runtime closure-resource component must be valid");
}

/// COMPOUND-RESULT compiler serializer: `serialize::closure_bytes_resource_core_module` — the production
/// core a closure whose result is a runtime `Bytes` emits — is structurally valid. The closure body
/// `(env, x) -> Bytes` builds a 2-byte `[x, x+1]` (`bytes-alloc`/`bytes-set`); `call` dispatches it via
/// `call_indirect`, copies the returned Bytes handle to the `(ptr, len)` return area (`bytes-len`/
/// `bytes-get` loop over the exported memory), and drops both the cell + the Bytes handle. The core
/// carries a MEMORY + `cabi_realloc` (a scalar `call` needs neither) so the canonical `list<u8>` ABI can
/// read the return area — the shape `oracle_closure_list_component` proved runs under wasmtime.
#[test]
fn closure_bytes_resource_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    // Imports (any order — the serializer maps name→index by position): the cell ops (make + call), the
    // Bytes ops (the closure body builds a Bytes; `call` copies it), drop (own<t> + Bytes release).
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.bytes_alloc,
        OPS.bytes_get,
        OPS.bytes_len,
        OPS.bytes_set,
        OPS.drop,
        OPS.get_int,
    ];
    // Defined func 0 = the export body `main : () -> own<closure>` — build a 1-slot cell (slot 0, no
    // captures), returning the cell handle.
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(Ty::Bytes));
    let export_body = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Defined func 1 = the lifted closure `(env: i32, x: i64) -> Bytes(i32)` = the 2-byte `[x, x+1]`.
    // bytes-alloc(2); bytes-set(_, 0, wrap x); bytes-set(_, 1, wrap x + 1).
    let lifted_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: Ty::Bytes,
        code: vec![
            Lir::ConstI32(2),
            Lir::CallImport("bytes-alloc"),
            Lir::ConstI32(0),
            Lir::LocalGet(1),
            Lir::I32WrapI64,
            Lir::CallImport("bytes-set"),
            Lir::ConstI32(1),
            Lir::LocalGet(1),
            Lir::I32WrapI64,
            Lir::ConstI32(1),
            Lir::I32Add,
            Lir::CallImport("bytes-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![export_body, lifted_body];

    let export = ExportPlan {
        name: "main".to_string(),
        def: 0,
        body: crate::ast::StructId(0),
        params: vec![],
        result: fn_ty.clone(),
    };
    let lifted = LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: Ty::Bytes,
        captures: vec![],
    };
    let import_base = imports.len() as u32 + 2; // k ops + resource-new + resource-rep
    let layout = Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);
    let export_abs = import_base;
    let lifted_type_idx = layout.lifted_type_index(0, import_base);

    let core = crate::backend::wasm::serialize::closure_bytes_resource_core_module(
        &funcs,
        &imports,
        export_abs,
        &[ValType::I64], // the closure's one arg
        &[],             // nullary export → make() has no params
        lifted_type_idx,
        &layout,
    )
    .expect("compound-result closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("compound-result closure-resource core module validates");
}

/// MULTI-EXPORT compiler serializer: `serialize::multi_closure_resource_core_module` — the production
/// core a program with TWO same-signature closure exports emits — is structurally valid. Two export
/// bodies each build a 1-slot cell (its own funcref slot), two lifted bodies, and the serializer emits
/// TWO `make-<name>` functions (each calling its export body + `resource.new`) sharing ONE `call`.
/// Mirrors the single-export serializer test but with N=2 makes, pinning the index layout the
/// multi-export oracle proved runnable.
#[test]
fn multi_closure_resource_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::ClosureMake;
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    // Two export bodies, each `() -> own<closure>`: build a 1-slot cell holding box-int(slot). Def 0's
    // cell points at table slot 0, def 1's at slot 1 (a distinct lifted body).
    let export_body = |slot: i64| SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(slot),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Two lifted bodies `(env: i32, x: i64) -> i64`: inc = x+1, triple = x*3.
    let lifted_inc = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let lifted_triple = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(3), Lir::I64Mul],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // funcs = [export0, export1, lifted_inc, lifted_triple] — the two export bodies at emission
    // positions 0,1 (in `order`), the two lifted bodies appended after.
    let funcs = vec![export_body(0), export_body(1), lifted_inc, lifted_triple];

    let mk_export = |name: &str, def: usize| ExportPlan {
        name: name.to_string(),
        def,
        body: crate::ast::StructId(0),
        params: vec![],
        result: fn_ty.clone(),
    };
    let mk_lifted = || LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: s64.clone(),
        captures: vec![],
    };
    let import_base = imports.len() as u32 + 2;
    // order = [0, 1] (two export defs); two lifted appended after → their abs/type idx = import_base +
    // order.len() + slot.
    let layout = Layout::with_lifted(
        vec![mk_export("inc", 0), mk_export("triple", 1)],
        vec![0, 1],
        import_base,
        vec![mk_lifted(), mk_lifted()],
        vec![true, true],
    );
    // Both closures share the signature → one shared call functype = slot 0's lifted type index.
    let lifted_type_idx = layout.lifted_type_index(0, import_base);
    let makes = vec![
        ClosureMake {
            export_name: "make-inc".to_string(),
            export_abs: import_base, // def 0 at emission position 0
            param_vts: vec![],
        },
        ClosureMake {
            export_name: "make-triple".to_string(),
            export_abs: import_base + 1, // def 1 at emission position 1
            param_vts: vec![],
        },
    ];

    let core = crate::backend::wasm::serialize::multi_closure_resource_core_module(
        &funcs,
        &imports,
        &makes,
        &[],
        &[ValType::I64],
        ValType::I64,
        lifted_type_idx,
        &layout,
    )
    .expect("multi-export closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("multi-export closure-resource core module validates");
}

/// ROUND-TRIP compiler serializer (C-HOST-4): `serialize::roundtrip_resource_core_module` — the core a
/// producer+consumer program emits — is structurally valid. A `make` produces a closure resource; a
/// separate `apply` CONSUMER takes the closure resource back (as an i32 handle) + a scalar, `resource
/// .rep`s the handle to the guest cell, and calls the consumer body which applies the closure
/// (`Core::CallClosure`). Pins the consumer-wrapper index layout the round-trip oracle proved runnable.
#[test]
fn roundtrip_resource_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::{ClosureConsume, ClosureMake, ConsumeParam};
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    // Producer body `() -> own<closure>`: build a 1-slot cell holding box-int(0) (table slot 0).
    let producer = SelectedFunc {
        params: vec![],
        ret: fn_ty.clone(),
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(0),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Consumer body `(g_cell: i32, x: i64) -> i64` = `(g x)` — the CallClosure sequence over a CELL
    // param (slot 0) and scalar arg (slot 1): push env(cell) + x, arr-get(cell,0)→get-int→wrap,
    // call_indirect the lifted functype. (The serializer's wrapper `resource.rep`s the boundary handle
    // to this cell before calling this body.)
    // import_base (k+2) + order.len() (2) + slot 0 = the lifted functype's type index in the core.
    let lifted_type_idx_placeholder = imports.len() as u32 + 2 + 2; // = layout.lifted_type_index(0)
    let consumer_body = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![
            Lir::LocalGet(0), // env (the cell)
            Lir::LocalGet(1), // x
            Lir::LocalGet(0),
            Lir::ConstI32(0),
            Lir::CallImport("arr-get"),
            Lir::CallImport("get-int"),
            Lir::I32WrapI64,
            Lir::CallIndirect(lifted_type_idx_placeholder),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Lifted `(env: i32, x: i64) -> i64` = x + 1.
    let lifted = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // funcs = [producer(def0), consumer_body(def1), lifted] — two order defs, one lifted appended.
    let funcs = vec![producer, consumer_body, lifted];

    let import_base = imports.len() as u32 + 2;
    let layout = Layout::with_lifted(
        vec![
            ExportPlan {
                name: "make-it".to_string(),
                def: 0,
                body: crate::ast::StructId(0),
                params: vec![],
                result: fn_ty.clone(),
            },
            ExportPlan {
                name: "apply-it".to_string(),
                def: 1,
                body: crate::ast::StructId(0),
                params: vec![],
                result: s64.clone(),
            },
        ],
        vec![0, 1],
        import_base,
        vec![LiftedLambda {
            body: crate::ast::StructId(0),
            params: vec![(crate::ast::StructId(0), s64.clone())],
            ret_ty: s64.clone(),
            captures: vec![],
        }],
        vec![true],
    );
    let lifted_type_idx = layout.lifted_type_index(0, import_base);
    assert_eq!(
        lifted_type_idx, lifted_type_idx_placeholder,
        "the consumer body's call_indirect type index must match the lifted lambda's"
    );
    let makes = vec![ClosureMake {
        export_name: "make-it".to_string(),
        export_abs: import_base, // producer at emission position 0
        param_vts: vec![],
    }];
    let consumers = vec![ClosureConsume {
        export_name: "apply-it".to_string(),
        consume_abs: import_base + 1, // consumer body at emission position 1
        params: vec![ConsumeParam::Closure, ConsumeParam::Scalar(ValType::I64)],
        ret_vt: ValType::I64,
        ret_is_bytes: false,
        ret_template: None,
        ret_descriptor: None,
    }];

    let core = crate::backend::wasm::serialize::roundtrip_resource_core_module(
        &funcs,
        &imports,
        &makes,
        &consumers,
        &[],
        lifted_type_idx,
        &layout,
    )
    .expect("round-trip closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("round-trip closure-resource core module validates");
}

/// DISTINCT-SIGNATURE compiler serializer: `serialize::distinct_sig_resource_core_module` — the core a
/// program with TWO signature groups emits — is structurally valid. Group 0 = `inc : (-> Int64 Int64)`
/// (lifted `(i64)->i64`), group 1 = `isz : (-> Int64 Bool)` (lifted `(i64)->i32`). Each group gets its
/// own `resource-new-<g>`/`resource-rep-<g>` intrinsics + its own make + call. Pins the N-resource-type
/// core index layout the distinct-signature oracle proved runnable.
#[test]
fn distinct_sig_resource_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::{ClosureMake, SigGroup};
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let boolt = Ty::Bool;
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let fn_ii = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone())); // (-> Int64 Int64)
    let fn_ib = Ty::Fn(Box::new(s64.clone()), Box::new(boolt.clone())); // (-> Int64 Bool)
    // Two export bodies, each `() -> own<closure>`: build a 1-slot cell holding box-int(slot).
    let export_body = |slot: i64, ret: Ty| SelectedFunc {
        params: vec![],
        ret,
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(slot),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // lifted-inc `(env, x) -> i64` = x + 1; lifted-isz `(env, x) -> i32` = (x == 0).
    let lifted_inc = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let lifted_isz = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: boolt.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(0), Lir::I64Eq],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let funcs = vec![
        export_body(0, fn_ii.clone()),
        export_body(1, fn_ib.clone()),
        lifted_inc,
        lifted_isz,
    ];

    let import_base = imports.len() as u32 + 2 * 2; // k + 2 intrinsics per group × 2 groups
    let layout = Layout::with_lifted(
        vec![
            ExportPlan {
                name: "inc".into(),
                def: 0,
                body: crate::ast::StructId(0),
                params: vec![],
                result: fn_ii.clone(),
            },
            ExportPlan {
                name: "isz".into(),
                def: 1,
                body: crate::ast::StructId(0),
                params: vec![],
                result: fn_ib.clone(),
            },
        ],
        vec![0, 1],
        import_base,
        vec![
            LiftedLambda {
                body: crate::ast::StructId(0),
                params: vec![(crate::ast::StructId(0), s64.clone())],
                ret_ty: s64.clone(),
                captures: vec![],
            },
            LiftedLambda {
                body: crate::ast::StructId(0),
                params: vec![(crate::ast::StructId(0), s64.clone())],
                ret_ty: boolt.clone(),
                captures: vec![],
            },
        ],
        vec![true, true],
    );
    let groups = vec![
        SigGroup {
            makes: vec![ClosureMake {
                export_name: "make-inc".into(),
                export_abs: import_base,
                param_vts: vec![],
            }],
            arg_vts: vec![ValType::I64],
            ret_vt: ValType::I64,
            lifted_slot: 0, // lifted-inc is table slot 0
            ret_is_bytes: false,
            ret_template: None,
            ret_descriptor: None,
            tuples: vec![],
            sums: vec![],
        },
        SigGroup {
            makes: vec![ClosureMake {
                export_name: "make-isz".into(),
                export_abs: import_base + 1,
                param_vts: vec![],
            }],
            arg_vts: vec![ValType::I64],
            ret_vt: ValType::I32, // Bool → i32
            lifted_slot: 1,       // lifted-isz is table slot 1
            ret_is_bytes: false,
            ret_template: None,
            ret_descriptor: None,
            tuples: vec![],
            sums: vec![],
        },
    ];

    let core = crate::backend::wasm::serialize::distinct_sig_resource_core_module(
        &funcs,
        &imports,
        &groups,
        &[],
        &layout,
        false,
    )
    .expect("distinct-signature closure-resource core serializes");

    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("distinct-signature closure-resource core module validates");
}

/// DISTINCT-SIGNATURE ROUND-TRIP serializer: `serialize::distinct_sig_roundtrip_core_module` — a core
/// with TWO signature groups, EACH a producer (`make-<name>`) AND a consumer (a named export taking a
/// closure of that sig back). Group 0 = `(-> Int64 Int64)` (mk0 + app0), group 1 = `(-> Int64 Bool)`
/// (mk1 + app1). Each group gets its own `resource-new-<g>`/`resource-rep-<g>`; the consumer wrapper
/// reps its closure param via THAT group's rrep. Pins the index layout the distinct-sig-round-trip needs.
#[test]
fn distinct_sig_roundtrip_core_module_is_structurally_valid() {
    use crate::backend::wasm::lir::{Lir, ValType};
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::{ClosureConsume, ClosureMake, ConsumeParam, RtSigGroup};
    use crate::layout::{ExportPlan, Layout};
    use crate::lower::LiftedLambda;
    use crate::ty::{IntTy, Ty};

    let s64 = Ty::Int(IntTy::fixed(true, 64));
    let boolt = Ty::Bool;
    let imports = vec![
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ];
    let fn_ii = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
    let fn_ib = Ty::Fn(Box::new(s64.clone()), Box::new(boolt.clone()));
    // Producer bodies (build a 1-slot cell → its lifted slot) for each group.
    let producer = |slot: i64, ret: Ty| SelectedFunc {
        params: vec![],
        ret,
        code: vec![
            Lir::ConstI32(1),
            Lir::CallImport("arr-alloc"),
            Lir::ConstI32(0),
            Lir::ConstI64(slot),
            Lir::CallImport("box-int"),
            Lir::CallImport("arr-set"),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Consumer bodies `(g_cell: i32, x: <arg>) -> <ret>` = `(g x)` (CallClosure over the group's lifted).
    let consumer = |lifted_ty: u32, ret: Ty| SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret,
        code: vec![
            Lir::LocalGet(0),
            Lir::LocalGet(1),
            Lir::LocalGet(0),
            Lir::ConstI32(0),
            Lir::CallImport("arr-get"),
            Lir::CallImport("get-int"),
            Lir::I32WrapI64,
            Lir::CallIndirect(lifted_ty),
        ],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // Lifted bodies: inc (i64->i64), isz (i64->i32/Bool).
    let lifted_inc = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: s64.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let lifted_isz = SelectedFunc {
        params: vec![ValType::I32, ValType::I64],
        ret: boolt.clone(),
        code: vec![Lir::LocalGet(1), Lir::ConstI64(0), Lir::I64Eq],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    // order = [mk0, app0, mk1, app1] (4 defs); lifteds appended after.
    let import_base = imports.len() as u32 + 2 * 2; // k + 2 intrinsics × 2 groups
    // Lifted functype indices: defined_type_base + order.len() + slot. The distinct-sig-rt core emits
    // 2*G rintr functypes, so defined_type_base = k + 2*G (G=2 here); order.len()=4.
    let lty = |slot: usize| (imports.len() + 2 * 2 + 4 + slot) as u32;
    let funcs = vec![
        producer(0, fn_ii.clone()),      // mk0 → def 0
        consumer(lty(0), s64.clone()),   // app0 → def 1
        producer(1, fn_ib.clone()),      // mk1 → def 2
        consumer(lty(1), boolt.clone()), // app1 → def 3
        lifted_inc,                      // slot 0
        lifted_isz,                      // slot 1
    ];
    let ep = |name: &str, def: usize, result: Ty| ExportPlan {
        name: name.into(),
        def,
        body: crate::ast::StructId(0),
        params: vec![],
        result,
    };
    let mk_lifted = |ret: Ty| LiftedLambda {
        body: crate::ast::StructId(0),
        params: vec![(crate::ast::StructId(0), s64.clone())],
        ret_ty: ret,
        captures: vec![],
    };
    let layout = Layout::with_lifted(
        vec![
            ep("mk0", 0, fn_ii.clone()),
            ep("app0", 1, s64.clone()),
            ep("mk1", 2, fn_ib.clone()),
            ep("app1", 3, boolt.clone()),
        ],
        vec![0, 1, 2, 3],
        import_base,
        vec![mk_lifted(s64.clone()), mk_lifted(boolt.clone())],
        vec![true, true],
    );
    let groups = vec![
        RtSigGroup {
            makes: vec![ClosureMake {
                export_name: "mk0".into(),
                export_abs: import_base,
                param_vts: vec![],
            }],
            consumers: vec![ClosureConsume {
                export_name: "app0".into(),
                consume_abs: import_base + 1,
                params: vec![ConsumeParam::Closure, ConsumeParam::Scalar(ValType::I64)],
                ret_vt: ValType::I64,
                ret_is_bytes: false,
                ret_template: None,
                ret_descriptor: None,
            }],
        },
        RtSigGroup {
            makes: vec![ClosureMake {
                export_name: "mk1".into(),
                export_abs: import_base + 2,
                param_vts: vec![],
            }],
            consumers: vec![ClosureConsume {
                export_name: "app1".into(),
                consume_abs: import_base + 3,
                params: vec![ConsumeParam::Closure, ConsumeParam::Scalar(ValType::I64)],
                ret_vt: ValType::I32,
                ret_is_bytes: false,
                ret_template: None,
                ret_descriptor: None,
            }],
        },
    ];
    let core = crate::backend::wasm::serialize::distinct_sig_roundtrip_core_module(
        &funcs,
        &imports,
        &groups,
        &[],
        &layout,
    )
    .expect("distinct-sig round-trip core serializes");
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&core)
        .expect("distinct-sig round-trip core module validates");
}

/// A CONSUMER-ONLY program (a closure export PARAMETER with no producer that mints one) stays out of
/// scope — the host would have to FABRICATE a Cadenza closure (a host-implemented function of a
/// Cadenza signature), which needs an import-side resource + a second dispatch path. Declines cleanly
/// naming the missing producer, rather than emitting a component whose consumer has no closure to
/// receive. (A round-trip WITH a producer compiles — see `a_produced_closure_round_trips_…`.)
#[test]
fn a_consumer_only_closure_program_declines() {
    use crate::testkit::parse;
    let src = "(module m (def (invoke (: g (-> Int64 Int64)) (: x Int64)) (g x)) (export invoke))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a consumer-only closure program must DECLINE (no producer mints the closure)");
    assert!(
        err.message.contains("PRODUCER") && err.code.is_none(),
        "expected the no-producer decline, got: {:?} / {}",
        err.code,
        err.message
    );
}

/// A closure TRANSFORMER export — one that both RECEIVES a closure (a param) and RETURNS one (its
/// result), e.g. `(def (twice (: g (-> Int64 Int64))) (fn (x) (g (g x))))` — is out of scope: the host
/// would hand a closure in and get one out of the same call. It declines CLEANLY, naming the export and
/// the shape, rather than the confusing internal "a producer parameter has no scalar representation"
/// (the round-trip `make`-forwarding site would otherwise choke on the closure param). Pins the honest
/// feature-decline (a companion producer keeps the program on the round-trip path so this is the arm hit).
#[test]
fn a_closure_transformer_export_declines_naming_the_shape() {
    use crate::testkit::parse;
    let src = "(do (def (mk) (fn ((: x Int64)) (+ x 1))) \
                   (def (twice (: g (-> Int64 Int64))) (fn ((: x Int64)) (g (g x)))) \
                   (export mk) (export twice))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a closure transformer (closure param AND closure result) must DECLINE");
    assert!(
        err.message.contains("closure transformer")
            && err.message.contains("twice")
            && err.code.as_deref() == Some("CDZ0900"),
        "expected the closure-transformer CDZ0900 decline naming `twice`, got: {:?} / {}",
        err.code,
        err.message
    );
}

/// Regression (v-cdz-smith fuzzer Bucket 2): a closure param used at TWO incompatible types — as Int64
/// (`(+ v0 …)`) AND as a tuple (`(. v0 0)`) — inside an UNCALLED inline closure `(list (fn (v0) …))`
/// escaped the checker: `collect_node` does NOT fault-check an uncalled inline closure body (it relies on
/// the β-reduction call site, which never happens for a closure merely STORED). The body-solved SCALAR
/// param then lowered a runtime tuple-projection (an `arr-get`) on a scalar slot → InvalidWasm. The Proj
/// lowering now DECLINES CDZ0201 "tuple projection requires a tuple, found Int64" — the SAME fault the
/// top-level twin `(def (v0) (+ v0 (. v0 0)))` gets — a clean decline, never a miscompile.
#[test]
fn an_uncalled_inline_closure_with_a_conflicting_param_declines_not_miscompiles() {
    use crate::testkit::parse;
    let src = "(module m (def (main) (List.len (list (fn (v0) (+ v0 (. v0 0)))))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect_err(
            "an uncalled inline closure with a param used as both Int64 and a tuple must DECLINE, not miscompile",
        );
    assert!(
        err.message.contains("tuple projection requires a tuple"),
        "expected the tuple-projection CDZ0201 decline, got: {:?} / {}",
        err.code,
        err.message
    );
}

/// The CLASS fix (v-cdz-smith + breaker + v-rb): `collect_node` now fault-checks an UNCALLED INLINE
/// closure body (with its params body-SOLVED), so a param used at TWO incompatible CONCRETE types faults
/// CDZ0201 at the CHECK — for EVERY conflict-kind, not one runtime lowering at a time. Previously each
/// kind (if-join, int-vs-list, …) escaped inference and emitted INVALID WASM. Here: an if-join over
/// Bool-vs-Float (`(if v0 v0 174.81)`) and an int-vs-List (`(+ v0 (List.len v0))`) — both DECLINE. The
/// SINGLE-use controls (`(if v0 1 2)` uses v0 only as a Bool cond; `(+ v0 1)` only as Int64) must still
/// COMPILE — an unpinned/consistently-used param is never over-rejected.
#[test]
fn the_uncalled_inline_closure_conflict_check_covers_every_conflict_kind_at_check() {
    use crate::testkit::parse;
    let prog = |body: &str| {
        format!("(module m (def (main) (List.len (list (fn (v0) {body})))) (export main))")
    };
    // Each conflict-kind DECLINES (a coded reject) at the check — not a miscompile.
    for body in [
        "(if v0 v0 174.81)",    // Bool (cond+then) vs Float (else) — if-join
        "(+ v0 (List.len v0))", // Int64 (+) vs List (List.len) — int-vs-collection
    ] {
        let err =
            crate::compile::compile_component(&crate::codec::encode(&parse(&prog(body)))).err();
        assert!(
            err.as_ref().and_then(|e| e.code.as_deref()).is_some(),
            "an uncalled inline closure with the conflict `{body}` must DECLINE (coded) at check, not \
                 miscompile; got {err:?}"
        );
    }
    // Single-use controls still COMPILE (no over-rejection of a consistently-used param).
    for body in ["(if v0 1 2)", "(+ v0 1)"] {
        assert!(
            crate::compile::compile_component(&crate::codec::encode(&parse(&prog(body)))).is_ok(),
            "a single-use closure param `{body}` must still COMPILE (not over-rejected)"
        );
    }
}

/// A FIXED-SHAPE SCALAR tuple closure ARG on the direct-call path now COMPILES (the tuple crosses as a
/// native component `tuple<s64,s64>` the canonical ABI flattens; the core `call` rebuilds the cell). But
/// a compound arg with a VARIABLE-LENGTH element (a tuple/record CONTAINING a List/Map/Set) must still
/// DECLINE — such a field has no fixed flattened form and would need host→guest runtime decode (a
/// nonexistent `value-decode` op). Pins both sides of the boundary: the fixed-shape scalar case emits, the
/// collection-bearing case declines cleanly.
#[test]
fn a_fixed_shape_scalar_tuple_arg_emits_but_a_collection_bearing_one_declines() {
    use crate::testkit::parse;
    // (a) a fixed-shape SCALAR tuple arg → emits a valid component (was a decline before the emit vertical).
    let ok_src = "(module m (def (main) (fn ((: p (Tuple Int64 Int64))) (. p 0))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(ok_src)))
        .expect("a fixed-shape scalar tuple closure arg now emits (native tuple flattening)");
    // (b) a tuple whose field is a variable-length LIST → still declines (no fixed flattened form).
    let bad_src =
        "(module m (def (main) (fn ((: p (Tuple Int64 (List Int64))) ) (. p 0))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(bad_src))).expect_err(
        "a tuple arg with a variable-length List field must DECLINE (needs runtime decode)",
    );
    assert!(
        err.message
            .contains("no scalar host-boundary representation")
            && err.code.is_none(),
        "expected the collection-bearing-compound-arg decline, got: {:?} / {}",
        err.code,
        err.message
    );
}

/// A fixed-shape scalar tuple closure ARG whose fields are NOT all `Int64` — a `Bool` field, a `Float`
/// field, a narrow int, or a mix — compiles cleanly. Each field's cell rebuild box op (`box-bool` /
/// `box-float` / `box-float32` / `box-int`) is named by `TupleArgRebuild::field_box_ops`, and the closure
/// `call`'s import-collection pass must register EXACTLY those ops. Before the fix it registered only
/// `box-int` (all-int tuples worked because that op was pulled in elsewhere), so a Bool/Float field's box
/// op was absent from the import index and `emit_tuple_rebuild`'s `imp(bop)` PANICKED the compiler
/// ("rebuild op imported"). Keying the imports off `field_box_ops` makes every field-type mix emit — this
/// covers the single-export, mixed (closure + plain export), and among-scalars shapes (the three funcs the
/// fix touched; the distinct-sig path already collected its `tuple_box_ops`).
#[test]
fn a_bool_or_float_field_tuple_closure_arg_compiles() {
    use crate::testkit::parse;
    let ok = |src: &str| {
        crate::compile::compile_component(&crate::codec::encode(&parse(src)))
            .unwrap_or_else(|e| panic!("must compile (no rebuild-op panic): {src}\n  got: {e:?}"));
    };
    // A Bool field (single-export direct-call).
    ok(
        "(module m (def (main) (fn ((: p (Tuple Int32 Bool))) (if (. p 1) (. p 0) 0))) (export main))",
    );
    // A Bool field in the OTHER position.
    ok("(module m (def (main) (fn ((: p (Tuple Bool Int64))) (. p 1))) (export main))");
    // Two Float fields.
    ok("(module m (def (main) (fn ((: p (Tuple Float64 Float64))) (. p 0))) (export main))");
    // A narrow Float field (box-float32).
    ok("(module m (def (main) (fn ((: p (Tuple Int32 Float32))) (. p 0))) (export main))");
    // A mixed int + float tuple (box-int AND box-float in one rebuild).
    ok("(module m (def (main) (fn ((: p (Tuple Int64 Float64))) (. p 0))) (export main))");
    // A RECORD arg with a Bool field.
    ok(
        "(module m (def (main) (fn ((: r (Record (a Int64) (f Bool)))) (if (. r f) (. r a) 0))) (export main))",
    );
    // Among scalar args (prefix + suffix), Bool field — the `single_compound_among_scalars` path.
    ok(
        "(module m (def (main) (fn ((: x Int64) (: p (Tuple Int64 Bool)) (: y Int64)) (if (. p 1) (+ x y) (. p 0)))) (export main))",
    );
    // MIXED shape: a Bool-field tuple closure export alongside a plain (non-closure) export.
    ok(
        "(module m (def (main) (fn ((: p (Tuple Int64 Bool))) (if (. p 1) (. p 0) 0))) (def (plain (: n Int64)) n) (export main) (export plain))",
    );
    // All-int must STILL compile (no regression from the added registration).
    ok(
        "(module m (def (main) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1)))) (export main))",
    );
}

/// A fixed-shape scalar tuple closure ARG now composes with a BYTE-ROPE result: the bytes-result core +
/// envelope thread the `TupleArgRebuild` (the `call` rebuilds the flattened tuple cell, then copies the
/// closure's `Bytes` result out as `list<u8>`). Emits a valid component. A COMPOUND/COLLECTION result
/// combined with a tuple arg still declines (its cores don't yet thread the rebuild — the companion test).
#[test]
fn a_tuple_arg_with_a_bytes_result_emits() {
    use crate::testkit::parse;
    let src = "(module m (def (main) (fn ((: p (Tuple Int64 Int64))) \
                   (bin (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. p 1)))))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect(
        "a tuple arg with a Bytes result now emits (rebuild threaded through the bytes core)",
    );
}

/// A fixed-shape scalar tuple ARGUMENT now composes with EVERY single-export result shape — a fixed-shape
/// COMPOUND (value-form) result AND a VARIABLE-LENGTH COLLECTION (value-encode) result both emit (all four
/// result cores — scalar, byte-rope, value-form, value-encode — + the shared list<u8> envelope thread the
/// `TupleArgRebuild`). The genuinely-unsupported case is a compound arg with a VARIABLE-LENGTH FIELD (a
/// tuple CONTAINING a List): that has no fixed flattened form and declines at ARG detection.
#[test]
fn a_tuple_arg_composes_with_compound_and_collection_results() {
    use crate::testkit::parse;
    // (a) a `(Tuple Int64 Int64)` argument AND a `(Tuple Int64 Int64)` result → EMITS (value-form).
    let compound_res = "(module m (def (main) (fn ((: p (Tuple Int64 Int64))) \
                   (tuple (. p 0) (. p 1)))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(compound_res)))
        .expect("a tuple arg + a fixed-shape compound result emits (value-form + rebuild)");
    // (b) a `(Tuple Int64 Int64)` argument AND a `(List Int64)` result → EMITS (value-encode).
    let coll_res = "(module m (def (main) (fn ((: p (Tuple Int64 Int64))) \
                   (list (. p 0) (. p 1)))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(coll_res)))
        .expect("a tuple arg + a variable-length collection result emits (value-encode + rebuild)");
    // (c) a compound arg with a VARIABLE-LENGTH FIELD → still declines at arg detection (no fixed form).
    let bad_arg =
        "(module m (def (main) (fn ((: p (Tuple Int64 (List Int64))) ) (. p 0))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(bad_arg))).expect_err(
        "a tuple arg with a variable-length List field must DECLINE (needs runtime decode)",
    );
    assert!(
        err.message
            .contains("no scalar host-boundary representation")
            && err.code.is_none(),
        "expected the collection-bearing-compound-arg decline, got: {:?} / {}",
        err.code,
        err.message
    );
}

#[test]
fn a_partial_application_escaping_as_a_result_declines_with_an_arity_message() {
    use crate::testkit::parse;
    // An entrypoint whose result is a PARTIAL APPLICATION — `(f 1)` for a two-parameter `f` — is a
    // closure whose remaining parameter has an UNCONSTRAINED (`Any`) type, so it cannot cross the
    // host boundary. The message must explain THAT (a partial application escaping as the result),
    // not the misleading "a closure argument of type Any has no scalar representation" (which reads
    // as if a real type is unsupported). Internal partial application still WORKS (a separate test);
    // only escaping one as the export result declines.
    // M15 moved the detection into `collect_faults` (so `cdz check` reports it too, not only
    // `compile`), coded CDZ0201, explaining the partial-application cause.
    let src = "(do (def (f x y) (+ x y)) (def (main) (f 1)) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a partial application escaping as the export result must decline");
    assert_eq!(err.code.as_deref(), Some("CDZ0201"), "got: {}", err.message);
    assert!(
        err.message.contains("partial application")
            && err.message.contains("cannot cross the component boundary"),
        "expected the partial-application explanation, got: {}",
        err.message
    );
}

/// A PARTIAL APPLICATION whose residual parameter type is an unresolved unification variable — NOT
/// `Ty::Any` — escaping as an exported FUNCTION's result. `(def (g (: n Int64)) (Map.insert (Map.empty)
/// n))` returns `(-> ?7 (Map Int64 ?7))`: the residual first-parameter type inference never grounded
/// surfaces as a `Ty::Var(_)`, not `Any` (the `Any` case is the nullary-export sibling above). Before
/// the fix, `arrow_has_unconstrained` matched only `Any`, so `cdz check` accepted this while the backend
/// declined it deep in closure-resource emit — and the backend's message LEAKED the internal `?7`
/// verbatim ("a closure argument of type ?7 has no scalar host-boundary representation"). Matching
/// `Any | Var(_)` in BOTH the `collect_faults` detector and the backend's `closure_boundary_reject`
/// closes the check-vs-emit gap and replaces the leaky `?7` with the actionable partial-application
/// explanation. Pins: CDZ0201 on the compile surface, the partial-application wording, and that the raw
/// `?7` is NOT the whole message (it may appear once inside the backticked type context, but the
/// explanation must carry the cause).
#[test]
fn a_partial_application_with_a_var_residual_reports_at_both_surfaces_without_leaking_a_type_var() {
    use crate::testkit::parse;
    let src = "(module m (def (g (: n Int64)) (Map.insert (Map.empty) n)) (export g))";
    let encoded = crate::codec::encode(&parse(src));
    // The COMPILE surface: a coded CDZ0201, not the leaky bare-`?7` backend decline.
    let err = crate::compile::compile_component(&encoded)
        .expect_err("a partial application escaping as a function result must decline");
    assert_eq!(err.code.as_deref(), Some("CDZ0201"), "got: {}", err.message);
    assert!(
        err.message.contains("cannot cross the component boundary")
            && err.message.contains("partial application"),
        "expected the partial-application explanation, got: {}",
        err.message
    );
    // The CHECK surface must report the SAME fault (the check-vs-emit gap this increment closed): the
    // `collect_faults` detector's `arrow_has_unconstrained` now matches a `Var(_)` residual too.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("cannot cross the component boundary")),
        "cdz check must also report the closure-boundary fault, got: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// A closure that PERFORMS AN EFFECT cannot escape to the host (operator decision 2026-07-13:
/// "closures escaping effects — that's going to be super weird and I don't really want to support
/// it"). The closure's handler context is the `(host …)`/`(handle …)` frame open when the closure
/// was BUILT; that frame is gone when the host later invokes `call()`. Here the effect is DELEGATED
/// with `(host (ask) …)`, so absent this check the program would decline with an incidental internal
/// error ("not in the host-import set") — we reject it INTENTIONALLY with a message that names the
/// unsupported feature. (A fully intra-program-HANDLED effect leaves no `Core::HostCall` and is NOT
/// caught here — only an effect that would escape the boundary is.)
#[test]
fn a_closure_escaping_an_effect_declines_intentionally() {
    use crate::testkit::parse;
    let src = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (fn ((: x Int64)) (+ x (ask.ask))))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect_err(
        "a closure whose body performs an effect must REJECT (closures can't escape effects)",
    );
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0406"),
        "expected the CDZ0406 closures-escaping-effects code, got: {:?} / {}",
        err.code,
        err.message
    );
    assert!(
        err.message.contains("performs an effect")
            && err.message.contains("cannot cross the host boundary"),
        "expected the closures-escaping-effects rejection naming the op, got: {}",
        err.message
    );
    assert!(
        err.message.contains("ask.ask"),
        "the rejection should name the escaping effect operation, got: {}",
        err.message
    );
}

/// adv es1 (v-rust-backend/breaker, LOW diagnostic-parity): an escaping closure NESTED IN A TUPLE —
/// `(host (ask) (tuple 1 (fn (x) (+ x (ask.ask)))))` — must reject the SAME CDZ0406 as a bare escaping
/// closure, matching the rust backend. Before: the wasm CDZ0406 escaping-closure scan lived only inside
/// the individual `emit_*_resource` emitters, and a closure nested in a COMPOUND result routed to a
/// compound-escape path that lacked the scan, so it fell through to `select`'s generic "not in the
/// host-import set" decline (code None) — a parity gap (rust rejected CDZ0406, wasm declined generically;
/// both refuse, so it was diagnostic-only, no wrong value). Fix: hoist the escaping-closure scan to the
/// emit dispatch head (BEFORE routing), so every escape face — bare or compound-nested — declines CDZ0406
/// uniformly. Pins the tuple-nested face; the build-time-delegated companion below stays COMPILING (the
/// scan targets lifted closure BODIES, not a make-time host call in the export body).
#[test]
fn an_escaping_closure_nested_in_a_tuple_rejects_cdz0406_like_a_bare_one() {
    use crate::testkit::parse;
    let src = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (tuple 1 (fn ((: x Int64)) (+ x (ask.ask)))))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a closure nested in a tuple that performs an effect must REJECT CDZ0406");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0406"),
        "a tuple-nested escaping closure must reject CDZ0406 (parity with a bare one + the rust \
             backend), NOT fall through to a generic 'not in the host-import set' decline; got: {:?} / {}",
        err.code,
        err.message
    );
    assert!(
        err.message.contains("ask.ask") && err.message.contains("cannot cross the host boundary"),
        "the rejection should name the escaping op, got: {}",
        err.message
    );
}

/// PR #1792 review (MED false-reject): the hoisted CDZ0406 escaping-closure scan must skip UNREACHED
/// lifted lambdas. `layout.lifted` holds lambdas DEMANDED during type-checking but built by no reachable
/// `Core::Closure` — `append_lifted_bodies` emits those as inert never-called STUBS (gated on
/// `layout.lifted_reached`). A HostCall in such a dead stub is PROVABLY unreachable, so flagging it is a
/// spurious CDZ0406. Here a `Box` sum boxes two distinct-sig closures; `main` builds ONLY the plain `Un`,
/// so the `Bin` arm dispatches over a closure the program never constructs = a dead/unreached lift — and
/// that unbuilt `Bin` closure is EFFECTFUL (`ask.ask`). It can never run, so the program MUST COMPILE;
/// before the reached-gate fix the scan collected the unreached stub's HostCall and wrongly rejected
/// CDZ0406. Pins that a reachable escaping closure still rejects (the sibling test above) while an
/// unreachable one does not. `run (Un (fn (x) (+ x 1)))` → `g 9` = 10.
#[test]
fn an_unreached_effectful_lifted_closure_does_not_spuriously_reject_cdz0406() {
    use crate::testkit::parse;
    let src = "(do (effect ask (op ask (-> Unit Int64))) \
            (type Box (Bin (-> Int64 Int64 Int64)) (Un (-> Int64 Int64))) \
            (def (run (: b Box)) (match b ((Box.Bin f) (f 2 3)) ((Box.Un g) (g 9)))) \
            (def (pick (: which Int64)) \
              (if (> which 0) \
                (Box.Bin (fn ((: a Int64) (: x Int64)) (+ a (ask.ask)))) \
                (Box.Un (fn ((: x Int64)) (+ x 1))))) \
            (def (main) (host (ask) (run (pick 0)))) \
            (export main))";
    // Must COMPILE — the effectful Bin closure is an unreached lift, never invoked by the host.
    crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect(
            "an UNREACHED effectful lifted closure must not spuriously reject CDZ0406 (it can never run)",
        );
}

/// A closure export whose BUILD-TIME code delegates a host effect — `(host (ask) (let ((v (ask.ask)))
/// (fn (x) (+ x v))))` — now COMPILES to a VALID component (brick d: the closure-resource emit composes
/// the host interface via `multi_closure_resource_core_module_with_host` +
/// `assemble_closure_host_runtime_resource`). The `ask.ask` is discharged make-time while the delegation
/// is in scope; the returned closure captures only the plain result. Verified across capture positions
/// (let-init, operand-feeding-a-capture, one-of-several-captures) — all emit valid components. The
/// CDZ0406 ESCAPE (perform IN the lifted body) still REJECTS — the closure-capture emit only composes a
/// make-time host call, never a call-time one. (The end-to-end value — `call(3)` = 13 with `ask.ask`→10
/// — is the corpus case "a build-time delegated effect whose result a returned closure captures does not
/// escape".)
#[test]
fn a_closure_export_delegating_a_build_time_effect_emits_a_valid_component() {
    use crate::testkit::parse;
    let emits_valid = |src: &str, what: &str| {
        let bytes = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
            .unwrap_or_else(|e| panic!("{what} must emit, got decline: {}", e.message));
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(&bytes)
            .unwrap_or_else(|e| panic!("{what} must be a VALID component: {e}"));
    };
    // The canonical case + two other capture positions — all now emit valid components.
    emits_valid(
        "(do (effect ask (op ask (-> Unit Int64))) \
             (def (main) (host (ask) (let ((v (ask.ask))) (fn ((: x Int64)) (+ x v))))) (export main))",
        "a let-init build-time host capture",
    );
    emits_valid(
        "(do (effect ask (op ask (-> Unit Int64))) \
             (def (main) (host (ask) (let ((v (+ (ask.ask) 1))) (fn ((: x Int64)) (+ x v))))) (export main))",
        "a host call in an operand feeding a capture",
    );
    emits_valid(
        "(do (effect ask (op ask (-> Unit Int64))) \
             (def (main) (host (ask) (let ((a (ask.ask)) (b 5)) (fn ((: x Int64)) (+ (+ x a) b))))) (export main))",
        "one of several captures performing",
    );
    // A PLAIN capturing closure (no host) still emits (unaffected by the host route).
    emits_valid(
        "(do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder))",
        "a plain capturing closure",
    );
    // The CDZ0406 ESCAPE (perform INSIDE the closure BODY) still REJECTS — the host-composition route
    // only fires for a make-time (export-body) host call, never a call-time (lifted-body) one.
    let escape = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (fn ((: x Int64)) (+ x (ask.ask))))) (export main))";
    let esc_err = crate::compile::compile_component(&crate::codec::encode(&parse(escape)))
        .expect_err("a closure whose body performs must still reject CDZ0406");
    assert_eq!(
        esc_err.code.as_deref(),
        Some("CDZ0406"),
        "the escape case must still be CDZ0406, got: {:?} / {}",
        esc_err.code,
        esc_err.message
    );
}

/// A host operation with a STRING (or compound) RESULT has no component boundary form this compiler
/// emits yet — its result was collected as `result: None` (indistinguishable from a Unit result), then
/// selection hit the INTERNAL "not in the host-import set" path, a message documented as "a compiler
/// bug" surfacing for a VALID-but-unsupported program. It now declines HONESTLY at emit, naming the
/// operation, the offending position (`result`), the type, and the feature limitation — never the
/// internal-invariant message. (A scalar/unit result stays on the emit path — verified separately.)
#[test]
fn a_host_op_with_a_string_result_declines_with_an_honest_message() {
    use crate::testkit::parse;
    // A host op with a STRING RESULT whose value is CONSUMED to a scalar (so the EXPORT is a scalar, and
    // the program flows through the main `emit` path, not the value-escape resource path): `String.len`
    // of the host's String result. The op still has no scalar boundary form for its String result.
    let src = "(do (effect ask (op greet (-> Unit String))) \
                   (def (main) (host (ask) (String.byte-len (ask.greet)))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a host op returning a String has no boundary form yet — must decline");
    assert!(
        err.message.contains("greet")
            && err.message.contains("result")
            && err.message.contains("String")
            && err.message.contains("no component"),
        "expected an honest feature-limitation decline naming the op/position/type, got: {}",
        err.message
    );
    assert!(
        !err.message.contains("not in the host-import set"),
        "must NOT surface the internal-invariant \"compiler bug\" message, got: {}",
        err.message
    );
    // The SAME honest decline covers a String-result op used AS THE EXPORT (routes to the value-escape
    // path, `emit_runtime_resource`) — the representability guard is hoisted to the TOP of `emit`, before
    // any escape/closure/main dispatch, so BOTH routings decline honestly (not the internal message).
    let as_export = "(do (effect ask (op greet (-> Unit String))) \
                   (def (main) (host (ask) (ask.greet))) (export main))";
    let err2 = crate::compile::compile_component(&crate::codec::encode(&parse(as_export)))
        .expect_err("a host op returning a String, used as the export, must also decline");
    assert!(
        err2.message.contains("greet") && err2.message.contains("no component"),
        "the used-as-export routing must also decline honestly, got: {}",
        err2.message
    );
    assert!(
        !err2.message.contains("not in the host-import set"),
        "the used-as-export routing must NOT surface the internal message, got: {}",
        err2.message
    );
    // A host op with a DETERMINED COMPOUND ARGUMENT declines honestly too (the guard checks args as
    // well as the result), with the grammatical article "an argument" — never silently dropping the arg.
    let compound_arg = "(do (effect ask (op send (-> (Tuple Int64 Int64) Int64))) \
                   (def (main) (host (ask) (ask.send (tuple 1 2)))) (export main))";
    let err3 = crate::compile::compile_component(&crate::codec::encode(&parse(compound_arg)))
        .expect_err("a host op taking a compound argument has no boundary form yet — must decline");
    assert!(
        err3.message.contains("send")
            && err3.message.contains("an argument")
            && err3.message.contains("no component"),
        "a compound-argument host op must decline honestly with the correct article, got: {}",
        err3.message
    );
    // A SCALAR-result host op still compiles (the honest decline must not over-reject).
    let ok = "(do (effect ask (op ask (-> Unit Int64))) \
                  (def (main) (host (ask) (+ (ask.ask) 1))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(ok)))
        .expect("a scalar-result host op still compiles (the honest decline is result-type-gated)");
}

/// PATH-PARITY diagnostic honesty (breaker tick 380): a bare-effect `Bytes` RESULT has no bare-path
/// wasm boundary emit yet — the `list<u8>` lift (`select::emit_result_lift`) is wired only on the
/// WORLD-DRIVEN / bytes-provider path (gated by `allow_option_bytes`), so the bare guard correctly
/// declines rather than emit invalid wasm. The PRIOR message self-contradicted: it listed `list<u8>`
/// (Bytes) as a supported RESULT form in the same breath it rejected a Bytes result. It now declines
/// HONESTLY — names the op/result/type, keeps `no component`, and points to the world-driven
/// `(wit-world …)` path instead of claiming a bare `list<u8>` result is supported. (The RUST backend
/// emits this natively as a `Vec<u8>`; this guard is the WASM boundary path only.)
#[test]
fn a_bare_effect_bytes_result_declines_pointing_to_the_world_driven_path() {
    use crate::testkit::parse;
    let src = "(do (effect H (op seed (-> Unit Bytes))) \
                   (def (main) (host (H) (Bytes.len (H.seed)))) (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a bare-effect Bytes result has no bare-path boundary emit — must decline");
    assert!(
        err.message.contains("seed")
            && err.message.contains("result")
            && err.message.contains("Bytes")
            && err.message.contains("no component")
            && err.message.contains("WORLD-DRIVEN"),
        "expected an honest decline that points to the world-driven path, got: {}",
        err.message
    );
    // Must NOT self-contradict by listing a bare `list<u8>` RESULT as supported (the old wording did).
    assert!(
        !err.message
            .contains("RESULTS cross as: a scalar/unit, a `list<u8>`"),
        "must not list a bare `list<u8>` result as supported (the self-contradicting wording), got: {}",
        err.message
    );
}

/// MULTI-EXPORT closures COMPILE for the same-signature (one resource type, shared `call`), the
/// DISTINCT-signature (N resource types, per-group `call-g<n>`), AND the MIXED shape (closures ALONGSIDE
/// a plain non-closure export — the closures via the resource envelope, the plain export as an ordinary
/// top-level func) for BOTH same-signature and DISTINCT-signature closure sets. Pins the four working
/// cases (same-sig, distinct-sig, same-sig+plain, distinct-sig+plain).
#[test]
fn multi_export_closures_compile_same_or_distinct_signature() {
    use crate::testkit::parse;
    // Same signature → COMPILES (shared call).
    let same = "(do (def (inc) (fn ((: x Int64)) (+ x 1))) \
                    (def (triple) (fn ((: x Int64)) (* x 3))) (export inc) (export triple))";
    crate::compile::compile_component(&crate::codec::encode(&parse(same)))
        .expect("two same-signature closure exports compile (multi-export path)");

    // DIFFERENT signatures → now COMPILES (N resource types, per-group call).
    let diff = "(do (def (inc) (fn ((: x Int64)) (+ x 1))) \
                    (def (isz) (fn ((: x Int64)) (= x 0))) (export inc) (export isz))";
    crate::compile::compile_component(&crate::codec::encode(&parse(diff)))
        .expect("distinct-signature closure exports compile (distinct-sig path)");

    // A same-signature closure ALONGSIDE a non-closure export → now COMPILES (the mixed envelope: the
    // closure crosses via make/call, the plain export as an ordinary top-level func).
    let mixed = "(do (def (inc) (fn ((: x Int64)) (+ x 1))) \
                     (def (two) 2) (export inc) (export two))";
    crate::compile::compile_component(&crate::codec::encode(&parse(mixed)))
        .expect("a same-signature closure alongside a plain export compiles (mixed-export path)");

    // DISTINCT closure signatures ALONGSIDE a non-closure export → now COMPILES too (the distinct-sig
    // envelope carries plain exports: N resource types + the plain export as a top-level func).
    let mixed_distinct = "(do (def (inc) (fn ((: x Int64)) (+ x 1))) \
                              (def (isz) (fn ((: x Int64)) (= x 0))) \
                              (def (two) 2) (export inc) (export isz) (export two))";
    crate::compile::compile_component(&crate::codec::encode(&parse(mixed_distinct))).expect(
        "distinct-signature closures alongside a plain export compile (distinct-sig mixed path)",
    );

    // A SINGLE closure returning a byte-rope (Bytes/String) COMPILES (the compound-result `call` path),
    // and now TWO such closures sharing one `call` COMPILE too (the multi-export list-returning `call`:
    // N makes + one shared bytes-`call`).
    let one_bytes = "(do (def (main) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n))))) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(one_bytes)))
        .expect("a single Bytes-returning closure compiles (compound-result path)");
    let two_bytes = "(do (def (a) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n))))) \
                         (def (b) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap n))))) \
                         (export a) (export b))";
    crate::compile::compile_component(&crate::codec::encode(&parse(two_bytes)))
        .expect("two Bytes-returning closures sharing one call compile (multi-export bytes path)");
}

/// The escape check is SCOPED to the returned closure's body — a BUILD-TIME delegated effect whose
/// result the closure merely CAPTURES does NOT escape and must not be rejected CDZ0406. Here `ask.ask`
/// runs in the `let` initializer, at export-execution time INSIDE the `(host (ask) …)` delegation's
/// dynamic extent (where it has a home); the returned `(fn (x) (+ x v))` captures the plain result `v`
/// and is effect-free. The over-rejection was scanning the WHOLE export body for a `Core::HostCall`
/// (catching the `make`-time one); the fix scans only the LIFTED closure bodies (the code that crosses
/// the boundary and runs later). This program does not yet RUN (the export-time host-call boundary is
/// E2-host WIP, so it declines codeless "not in the host-import set" — grades todo), but the point is
/// the COMPILE-TIME outcome must NOT be the CDZ0406 over-rejection. Mirrors the intra-program
/// `(handle … (let ((v (E.get))) (fn (x) (+ x v))))`, which compiles.
#[test]
fn a_build_time_delegated_effect_captured_by_a_closure_is_not_a_cdz0406_escape() {
    use crate::testkit::parse;
    let src = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (let ((v (ask.ask))) (fn ((: x Int64)) (+ x v))))) \
                   (export main))";
    let r = crate::compile::compile_component(&crate::codec::encode(&parse(src)));
    // Whether it compiles or declines on the E2-host boundary, it must NOT be the CDZ0406 escape
    // over-rejection — the build-time effect is discharged in scope, not escaped.
    if let Err(d) = &r {
        assert_ne!(
            d.code.as_deref(),
            Some("CDZ0406"),
            "a build-time delegated effect the closure only captures must NOT be a CDZ0406 escape; \
                 got: {:?} / {}",
            d.code,
            d.message
        );
    }
    // The genuine escape — the effect performed INSIDE a NESTED (curried) returned closure — is still
    // caught, so the fix did not blind the check to a real escape reachable only through nesting.
    let nested = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (fn ((: x Int64)) (fn ((: y Int64)) (+ (+ x y) (ask.ask)))))) \
                   (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(nested)))
        .expect_err("an effect inside a nested returned closure still escapes");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0406"),
        "a nested escaping closure must still reject CDZ0406, got: {:?} / {}",
        err.code,
        err.message
    );
}
