//! The flat instruction rung (Lir) — this backend's representation of the core.
//!
//! Lir is NOT a shared pipeline rung: it is what the wasm backend produces because its target is a
//! linear instruction stream (`backends-and-targets.md` §The Flat Instruction Rung Is A Property Of A
//! Linearizing Backend). A backend for a structured target would consume the core directly and never
//! build this. The compiler works in `ValType`/`BlockType` here; the single raw-byte encoding lives
//! in the serializer (`ValType::byte`/`BlockType::byte`), so no other pass hard-codes a wasm byte.
//!
//! This is also where the TARGET valtype for a solved [`crate::ty::Ty`] is decided: mapping a
//! language type to a wasm value type is a wasm concern, so it lives here (via [`valtype_of`]), not on
//! `Ty` (`backends-and-targets.md` §a target-specific concern lives in the target that has it).

use crate::backend::wasm::wasm_abi;
use crate::ty::{IntTy, Ty};

/// A core-wasm value type — the machine representation a scalar takes inside a function body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValType {
    I64,
    I32,
    /// A 32-bit IEEE float — the machine slot a `(Float 32)` (Float32) value occupies.
    F32,
    /// A 64-bit IEEE float — the machine slot a `Ty::Float` (Float64) value occupies. Added for float
    /// literals crossing the boundary; float arithmetic (f64.add/…) builds on it.
    F64,
}

impl ValType {
    /// The core-wasm valtype byte (`0x7E` i64, `0x7F` i32, `0x7D` f32, `0x7C` f64) — from the generated
    /// `wasm_abi` table (extracted from `wasm-encoder`'s `ValType`), not a hand-typed byte. The raw
    /// encoding lives here (the serializer's concern), but its VALUE is the spec's.
    pub fn byte(self) -> u8 {
        match self {
            ValType::I64 => wasm_abi::CORE_I64,
            ValType::I32 => wasm_abi::CORE_I32,
            ValType::F32 => wasm_abi::CORE_F32,
            ValType::F64 => wasm_abi::CORE_F64,
        }
    }
}

/// The type of a wasm structured block (`if`/`block`/`loop`): it leaves no value (`Empty`) or leaves
/// one value of a given type (`Val`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockType {
    Empty,
    Val(ValType),
}

impl BlockType {
    /// The block-type byte: `0x40` for empty (the generated `wasm_abi::BLOCK_EMPTY`), else the value
    /// type's byte.
    pub fn byte(self) -> u8 {
        match self {
            BlockType::Empty => wasm_abi::BLOCK_EMPTY,
            BlockType::Val(vt) => vt.byte(),
        }
    }
}

/// One flat wasm instruction. Frozen for the Stage-0 slice.
///
/// This is the dedicated instruction sum: emitted instructions are values of a typed Rust `enum`
/// deconstructed by pattern matching (see `serialize::instr`), never string-tagged pseudo-structures —
/// so an instruction is inspected variant-by-variant like any other value, not by matching a string tag.
//= spec/capabilities/compiler-pipeline.md#the-compiler-operates-on-ast-values
//# The compiler MUST represent the instructions it emits as values of a typed sum type — the AST sum or a dedicated instruction sum — deconstructible by pattern matching, not as string-tagged pseudo-structures, so that an instruction is inspected like any other Cadenza value rather than by matching on a string tag.
#[derive(Clone, PartialEq, Debug)]
pub enum Lir {
    /// `i64.const N` — a signed 64-bit constant (emitted via SLEB128).
    ConstI64(i64),
    /// `i32.const N` — a signed 32-bit constant (emitted via SLEB128).
    ConstI32(i32),
    /// `f64.const` — a 64-bit float constant, carried as its raw IEEE-754 BIT PATTERN (the exact bits,
    /// so `-0.0` and a NaN round-trip verbatim; `f64.const` is emitted as 8 little-endian bytes of the
    /// bit pattern, NOT LEB128). A float literal crossing the boundary emits this.
    F64ConstBits(u64),
    /// `f32.const` — a 32-bit float constant, carried as its raw IEEE-754 bit pattern (4 little-endian
    /// bytes, NOT LEB128). A `Float32` constant emits this.
    F32ConstBits(u32),
    /// Float ARITHMETIC — the machine op a runtime `+.`/`-.`/`*.`/`/.` selects, at the operand width
    /// (f64/f32). IEEE, never trapping (no overflow guard, unlike the integer arith). Each is a single
    /// primitive wasm op (`f64.add`, …) under wasm's ONE fixed round-to-nearest-even mode — never a fused
    /// multiply-add or a contracted op, so the result is the separately-rounded value every conforming
    /// runtime computes.
    //= spec/contracts/determinism-and-fuel.md#floating-point-emission-is-determinism-constrained
    //# The compiler MUST emit floating-point operations under a single fixed rounding mode.
    //= spec/contracts/determinism-and-fuel.md#floating-point-emission-is-determinism-constrained
    //# The compiler MUST NOT emit a fused or contracted floating-point operation whose result varies from the separately-rounded operations across conforming runtimes.
    //= spec/capabilities/numeric-model.md#floating-point-follows-the-determinism-contract
    //# A floating-point operation MUST produce a result consistent with the emission constraints in the determinism-and-fuel contract.
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    /// Float EQUALITY / inequality — a runtime float `=` (IEEE: `-0.0 == 0.0`, `NaN != NaN`).
    F64Eq,
    F64Ne,
    F32Eq,
    F32Ne,
    /// Float ORDERING — runtime `< <= > >=` under IEEE PARTIAL order (operator ruling): a NaN operand
    /// yields false (unordered), and `-0.0` compares equal-under-ordering to `+0.0`. These are the RAW
    /// IEEE compares, emitted directly for `Core::FloatCompare` with an ordering prim — DISTINCT from the
    /// canonical-byte EQUALITY path above (the two relations disagree on NaN + signed zero, by design).
    F64Lt,
    F64Le,
    F64Gt,
    F64Ge,
    F32Lt,
    F32Le,
    F32Gt,
    F32Ge,
    /// REINTERPRET a float's bits as the same-width integer (`i64.reinterpret_f64` 0xBD /
    /// `i32.reinterpret_f32` 0xBC) — no numeric conversion, just the bit pattern. Used by the
    /// canonical-byte float EQUALITY (`Core::FloatCompare`): after canonicalizing a NaN, the two floats
    /// compare as integers (`i64.eq`/`i32.eq`), giving `nan == nan` true and `-0.0 != +0.0` — NOT IEEE
    /// `f64.eq`.
    I64ReinterpretF64,
    I32ReinterpretF32,
    /// Float width conversion — `f32.demote_f64` (Float64→Float32, rounds) / `f64.promote_f32`
    /// (Float32→Float64, exact). Emitted by the width-conversion `Float32.of`/`Float64.of` (F5).
    F32DemoteF64,
    F64PromoteF32,
    /// INT→FLOAT conversion — `f64.convert_i64_s` / `f32.convert_i64_s` (signed i64 → f64/f32,
    /// round-to-nearest-even). Emitted by a runtime `Float N.of-int`.
    F64ConvertI64S,
    F32ConvertI64S,
    /// `local.get I` — read local `I`.
    LocalGet(u32),
    /// `local.set I` — pop the stack top into local `I`. Used by the checked-arithmetic guard to stash
    /// operands and the result in scratch locals.
    LocalSet(u32),
    /// `local.tee I` — pop the stack top into local `I` AND leave a copy on the stack (`local.set`
    /// followed by an implicit `local.get`, in one opcode). Not emitted directly by selection; the
    /// [`peephole`] pass folds an adjacent `local.set I ; local.get I` into it, saving one instruction
    /// wherever a value is stashed and immediately re-read (a nested op's result feeding an outer
    /// operand slot, a checked op's result feeding its guard).
    LocalTee(u32),
    /// `global.get G` — read module global `G` (an i32 handle). A build-once static compound (§2d) is
    /// stored in a mutable global by the init/start function; every use reads it here (then `dup`s to
    /// retain — the global is a persistent root, so the consumer's `drop` balances the retained `dup`).
    GlobalGet(u32),
    /// `global.set G` — store the stack top into module global `G`. Emitted only by the init function,
    /// which builds each static compound once and stows its handle in its global.
    GlobalSet(u32),
    /// `i32.store8 (align=0, offset=O)` — pop `[addr:i32, val:i32]` and write the low BYTE of `val` to
    /// linear memory 0 at `addr + O`. The byte-granular store used to marshal a runtime String/Bytes value
    /// into the SHARED host memory at a host-call site (the `_mem` runtime-arg path): the guest reads the
    /// value-heap rope byte-by-byte and writes each into `mem` for the host to read as `(ptr, len)`. The
    /// stack order is address-then-value (wasm's store convention: push addr, push val, `store`). `offset`
    /// is the static memarg displacement (usually 0 — a computed base address carries the real offset);
    /// alignment is 0 (byte store, no alignment requirement). Memory index is implicitly 0 (the sole memory).
    I32Store8 {
        offset: u32,
    },
    /// `i32.store (align=2, offset=O)` — pop `[addr:i32, val:i32]` and write the full i32 `val` to linear
    /// memory 0 at `addr + O`. Writes the `(ptr, len)` slots of a `list<T>` host-ARG's outer array marshaled
    /// into the shared host memory (the arg-side analogue of the result lift's `I32Load` reads). Stack order
    /// is address-then-value; natural i32 alignment (2); memory index 0.
    I32Store {
        offset: u32,
    },
    /// `i64.store` / `f32.store` / `f64.store` / `i32.store16` (align natural / low bits) — the wider + narrow
    /// stores that write a SCALAR `list<T>` host-ARG element (or a scalar record leaf into `mem`) at its
    /// canonical stride offset: an `i64` (8), an `f32` (4) / `f64` (8), a 2-byte `i16`/`u16`. Stack order is
    /// address-then-value; memory index 0. (The 1-byte `i32.store8` + 4-byte `i32.store` already exist.)
    I64Store {
        offset: u32,
    },
    F32Store {
        offset: u32,
    },
    F64Store {
        offset: u32,
    },
    I32Store16 {
        offset: u32,
    },
    /// `i32.load (align=2, offset=O)` — pop `[addr:i32]`, push the i32 read from linear memory 0 at `addr+O`.
    /// The kv.get lift reads the spilled `option<list<u8>>` result struct a host import wrote at the caller-
    /// provided retptr (disc@0, list-ptr@4, list-len@8). Memory index 0; natural i32 alignment (2).
    I32Load {
        offset: u32,
    },
    /// `i32.load8_u (align=0, offset=O)` — pop `[addr:i32]`, push the zero-extended BYTE at `addr+O`. The
    /// copy-loop read that lifts a host result's `list<u8>` bytes out of shared memory into a value-heap Bytes.
    I32Load8U {
        offset: u32,
    },
    // Width-specific loads reading a SCALAR LEAF a spilled compound result stored at its natural width (the
    // general result-lift's scalar-leaf boxing): a 64-bit int (`i64.load`), a float (`f64`/`f32.load`), and
    // the sign/zero-extending narrow int loads (`i32.load8_s`, `i32.load16_s`, `i32.load16_u`). A 32-bit int /
    // char reuses `I32Load`; an unsigned byte reuses `I32Load8U`. Each pops `[addr:i32]`, pushes the value at
    // `addr+offset` on memory 0.
    I64Load {
        offset: u32,
    },
    F32Load {
        offset: u32,
    },
    F64Load {
        offset: u32,
    },
    I32Load8S {
        offset: u32,
    },
    I32Load16S {
        offset: u32,
    },
    I32Load16U {
        offset: u32,
    },
    /// `call F` — call wasm function index `F` (its arguments already pushed in order). The index is a
    /// definition's ABSOLUTE emission position (`layout.abs`), resolved at selection.
    Call(u32),
    /// `return_call F` — a TAIL call to wasm function index `F` (arguments already pushed). Replaces the
    /// caller's frame instead of pushing a new one, so a tail-recursive loop runs in O(1) stack (a
    /// self-call in tail position no longer traps by stack exhaustion). Emitted for a `Core::Call` in
    /// TAIL position; the callee's result type equals the caller's, so it is the caller's return value.
    ReturnCall(u32),
    /// `call_indirect (type T) (table 0)` — an INDIRECT call through the module's funcref table: the
    /// arguments then the TABLE INDEX (an i32) are already on the stack; pops the index, reads the funcref
    /// at that table slot, and calls it against functype `T`. This is how a runtime CLOSURE VALUE is
    /// applied (`Core::CallClosure`): the closure's stored table slot selects the lifted function's code.
    /// `T` is a TYPE-section index for the closure's signature `(arg) -> result`; the table is always 0
    /// (the one funcref table). Distinct from `Call` (a static function index) — the callee is dynamic.
    CallIndirect(u32),
    /// `call <import-index>` — call a value-heap runtime op by NAME (`arr-alloc`, `box-int`, …). The op
    /// name is carried symbolically because its concrete core function index (its position `0..k` in the
    /// program's sorted used-set) is only fixed once the whole used-set is known; `serialize` resolves
    /// the name to that index against the import order it lays. Distinct from `Call` (a defined-function
    /// index) so the two index spaces do not collide.
    CallImport(&'static str),
    /// `call <host-import-index>` — call a HOST-DELEGATED effect operation by its position in the
    /// program's host-import set (`0..h`). A host import is a component-level WIT function (the effect is
    /// an interface, the op a func) the boundary resolves; its core-func index is its position in the
    /// host-import set, which `emit` fixes BEFORE selection (like the runtime-op set). Distinct from
    /// `CallImport` (a value-heap runtime op) and `Call` (a defined func) so the three index spaces stay
    /// separate — the serializer lays host imports first, then runtime ops, then defined funcs.
    CallHostImport(usize),
    /// `call <extern-import-index>` — call a CROSS-COMPONENT peer operation by its position in the
    /// program's extern-import set (X4b). A peer op is a component-level WIT function (the peer interface
    /// bound under module `"peer"`) the composition resolves; the serializer lays the peer imports FIRST
    /// (`0..e`), then runtime ops, so the core-func index is the raw extern position. Since U4 a peer op is
    /// an escaping `Core::HostCall` whose effect is peer-bound (`db.effect_bindings`); `select` maps it to
    /// this node when the effect is bound (else `CallHostImport` for a host effect) — the index spaces stay
    /// separate. A peer op may coexist with the value-heap runtime (X5, `assemble_extern_runtime`).
    CallExternImport(usize),
    /// `if <blocktype>` — a two-way branch leaving a value of the block type.
    If(BlockType),
    /// `block <blocktype>` — open a forward block; a `Br` targeting it jumps FORWARD to its `end` (the
    /// block label is at the `end`, unlike a `loop` whose label is at the top). Used to build the
    /// br_table decision tree: nested blocks whose exits land right before each match arm's body.
    /// Closed by an `End` like `if`/`loop`.
    Block(BlockType),
    /// `loop <blocktype>` — open a loop block; a `Br` targeting it jumps BACK to this point (the loop
    /// label is at the top, unlike a `block` whose label is at the `end`). Used to compile a
    /// self-tail-recursive function as an in-place iteration: the body is wrapped in a loop and each
    /// self-tail-call becomes "update the parameter locals, then `Br` back to the loop top" — no wasm
    /// call frame per iteration. Closed by an `End` like `if`/`block`.
    Loop(BlockType),
    /// `br <depth>` — an unconditional branch to the enclosing control construct `depth` levels out
    /// (0 = the innermost). To a `loop` it jumps to the loop's top (iterate); to a `block`/`if` it
    /// jumps to the `end` (exit). Used by the self-tail-call → loop transform (`br 0` back to the loop).
    Br(u32),
    /// `br_if <depth>` — pop an i32 condition; branch to `depth` (like `br`) iff it is nonzero, else
    /// fall through. Used by the scalar br_table's out-of-range bounds guard (an i64 index that could
    /// wrap-alias into the table range branches to the default before the table).
    BrIf(u32),
    /// `br_table <targets> <default>` — an indexed multi-way branch: pop an i32 index off the stack and
    /// branch to `targets[index]` levels out, or `default` if the index is out of range. One O(1) jump
    /// table replacing a linear `if (== k)` probe cascade. Used by the decision-tree lowering of a dense
    /// scalar `match` and the sum-discriminant switch (discriminants are contiguous 0..k).
    BrTable(Vec<u32>, u32),
    /// `else`.
    Else,
    /// `end`.
    End,
    /// `select` — branchless conditional. Pops `[a, b, cond]` (cond on top) and pushes `a` if `cond` is
    /// nonzero, else `b`. BOTH operands are evaluated unconditionally, so it replaces a two-branch `if`
    /// ONLY when both branches are cheap, trap-free, and effect-free (a value the condition picks
    /// between, not control flow) — `(if c a b)` with leaf branches. No block, no jump.
    Select,
    /// `unreachable` — an unconditional trap.
    Unreachable,
    /// `return` (wasm `0x0f`) — a NON-LOCAL exit from the enclosing function, popping the function's
    /// result values off the stack. A terminator: like `unreachable` it leaves the stack POLYMORPHIC, so
    /// it validates in ANY result position. Used by the recursive effect-abort (`Core::HandleAbort`): the
    /// fold produces `HandleAbort` ONLY when the reduced abortive handle is the WHOLE function body (so the
    /// handle result IS the function result), and the abort emits `emit(value); Return` — abandoning the
    /// pending continuation + the self-loop and yielding `value` as the handle/function result. (Distinct
    /// from `ReturnCall`, the tail-call frame replacement.)
    Return,
    /// `drop` — pop and discard the top stack value. Used where a value was produced for its side of the
    /// stack but is not consumed: a UNIT heap slot stores the inline-unit sentinel `IMM_UNIT`, so reading
    /// such a slot (`arr-get`/`sum-payload`) yields that handle, which must be dropped because a `Unit`
    /// projection leaves NO machine value (`valtype_of(Unit) = None`).
    Drop,
    /// `if (empty) unreachable end` — trap when the i32 condition on the stack is nonzero, leaving
    /// nothing. The overflow guard's trip: it pushes a boolean "did overflow", then this traps if set.
    /// (One fused instruction so the guard is a flat run with no dangling block.)
    IfUnreachableEnd,
    /// `if (empty) <INT_MIN> <-1> i32.div_s drop end` — trap with wasm's native INTEGER-OVERFLOW reason
    /// when the i32 condition on the stack is nonzero, leaving nothing. Same trip shape as
    /// [`Lir::IfUnreachableEnd`], but the trap carries a DISTINGUISHABLE reason: `i32.div_s` of
    /// `i32::MIN / -1` is the one wasm arithmetic op that traps as "integer overflow" (a bare
    /// `unreachable` reports only "unreachable"). Used by the arithmetic overflow guards + narrow
    /// range-checks so a runtime integer overflow surfaces its kind to a debugger / the trap-reason
    /// gate, per `numeric-model.md #Overflow Is Defined`. The divide result is dropped — it never runs
    /// (the guard traps first); it exists only to name the trap.
    //= spec/capabilities/core-semantics.md#a-trap-halts-execution-at-a-defined-point
    //# The kind of trap a given operation raises MUST be a deterministic function of the operation and its inputs.
    IfIntegerOverflowEnd,
    /// `i64.add` / `i64.sub` / `i64.mul` — 64-bit integer arithmetic (operands already on the stack).
    I64Add,
    I64Sub,
    I64Mul,
    /// `i32.add` / `i32.sub` / `i32.mul` — 32-bit integer arithmetic. The machine op for a ≤32-bit
    /// width (`Int8`/`Int16`/`Int32`, and their unsigned peers), whose value sits sign-/zero-extended in
    /// an i32 slot.
    I32Add,
    I32Sub,
    I32Mul,
    /// `i32.div_s` / `i32.div_u` / `i32.rem_s` / `i32.rem_u` — 32-bit division / remainder. Trap on ÷0
    /// (and `div_s` on `i32::MIN/-1`, which IS the overflow at width 32). For a NARROW signed width
    /// (8/16) the machine op does not overflow, so the `MIN/-1` case is guarded explicitly.
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    /// `i32.and` / `i32.or` / `i32.xor` — 32-bit bitwise (total, never trap); `and`/`xor` also serve the
    /// signed-overflow guard at width 32.
    I32And,
    I32Or,
    I32Xor,
    /// `i32.ne` — 32-bit inequality leaving an i32 boolean (the width-32 mul guard `r/a != b`).
    I32Ne,
    /// `i32.shl` / `i32.shr_s` / `i32.shr_u` — 32-bit shifts (count masked mod 32). The runtime shift
    /// path guards the count and (for `<<`) overflow, so the mask/wrap is never observed with a bad count.
    I32Shl,
    I32ShrS,
    I32ShrU,
    /// 64-bit integer comparisons leaving an i32 boolean. Signed (`_s`) and unsigned (`_u`) variants —
    /// the operands' SIGNEDNESS selects which (an unsigned type orders by magnitude, so `_u`, where a
    /// signed type orders by two's-complement value, so `_s`); equality (`eq`) is sign-agnostic.
    I64Eq,
    I64LtS,
    I64GtS,
    I64LeS,
    I64GeS,
    I64LtU,
    I64GtU,
    I64LeU,
    I64GeU,
    /// `i64.eqz` — 1 (i32) if the i64 on the stack is 0, else 0. Emits `x == 0` on a 64-bit operand as
    /// ONE instruction instead of `i64.const 0 ; i64.eq` — the shape of every recursion base case
    /// `(if (= n 0) …)`. (The result is an i32 boolean, like every comparison.)
    I64Eqz,
    /// `i32.eqz` — 1 if the i32 on the stack is 0, else 0. On a boolean (an i32 0/1) it is logical NOT;
    /// used to emit a boolean negation `!c` (the `(if c false true)` fold) AND `x == 0` on a ≤32-bit
    /// operand (one instruction vs `i32.const 0 ; i32.eq`).
    I32Eqz,
    /// 32-bit integer comparisons leaving an i32 boolean — for a ≤32-bit integer OR a boolean operand
    /// (a bool is an i32). Signed and unsigned variants (a ≤32-bit value is properly sign-/zero-extended
    /// in its slot, so `_s`/`_u` compare it correctly); `eq` is sign-agnostic.
    I32Eq,
    I32LtS,
    I32GtS,
    I32LeS,
    I32GeS,
    I32LtU,
    I32GtU,
    I32LeU,
    I32GeU,
    /// `i64.and` / `i64.or` / `i64.xor` — bitwise ops. Total on the two's-complement value (never trap),
    /// so a runtime `&`/`|`/`^` is just the operands then the op. `and`/`xor` also serve the signed-
    /// overflow guard, which tests the sign bits of the operands and result (`((r^a)&(r^b))<0` for add).
    I64And,
    I64Or,
    I64Xor,
    /// `i64.ne` — inequality leaving an i32 boolean, used by the mul guard (`r/a != b`).
    I64Ne,
    /// `i64.shl` / `i64.shr_s` / `i64.shr_u` — the machine shifts (count masked mod 64, never trapping).
    /// The runtime shift path GUARDS these: an out-of-range count and (for `<<`) an overflow are tested
    /// explicitly and trap, so the masking/wrapping wasm shift is never observed with a bad count. `shr_s`
    /// is the arithmetic (sign-extending) right shift for a signed type; `shr_u` the logical one for an
    /// unsigned type.
    I64Shl,
    I64ShrS,
    I64ShrU,
    /// `i64.div_s` / `i64.div_u` — signed / unsigned division. Trap natively on division by zero, and
    /// `div_s` also on `MIN/-1` (the two defined division traps, and the sole mul-overflow case the
    /// `r/a≠b` guard can't catch). `div_s` is reused by the mul overflow guard (`r/a`). Each native trap
    /// carries wasm's defined kind naming why it halted — "integer divide by zero" vs "integer overflow" —
    /// not a bare `unreachable`.
    //= spec/capabilities/core-semantics.md#a-trap-halts-execution-at-a-defined-point
    //# A trap MUST carry a defined kind that identifies why the program halted.
    I64DivS,
    I64DivU,
    /// `i64.rem_s` / `i64.rem_u` — signed / unsigned remainder. Trap on division by zero; `rem_s` yields
    /// 0 for `MIN % -1` (it forms no quotient, so it does not overflow) — exactly the numeric model.
    I64RemS,
    I64RemU,
    /// Slot-crossing conversions used by a runtime `wrap` (and any op moving a value between machine
    /// slots): `i32.wrap_i64` drops the high 32 bits of an i64 into an i32; `i64.extend_i32_s`/`_u`
    /// widen an i32 into an i64, sign- or zero-extending by the SOURCE signedness (to preserve the
    /// source's value before the target mask). Never trap. (A runtime `wrap` also reuses the i32/i64
    /// `shl`+`shr_{s,u}` above for the width-generic narrow extend `(x << (M-N)) >> (M-N)`.)
    I32WrapI64,
    I64ExtendI32S,
    I64ExtendI32U,
}

/// The wasm value type a value of solved type `ty` occupies inside a function body, or `None` for a
/// type that occupies no runtime slot (unit). An integer's width chooses i32 vs i64 (a width ≤ 32 uses
/// i32, wider uses i64); a boolean is an i32. This is the wasm backend's read-off of the solved type
/// (`reference-compiler.md` §A Value's Machine Representation Follows Its Solved Type At Selection).
///
/// This read-off is where TYPES ARE ERASED: only a value's machine shape survives — a nominal tag, a
/// record's field NAMES, a quantity's unit, and a type-value itself all map THROUGH to (or to no) slot,
/// so the emitted component carries no runtime type reflection. A type that is not itself a runtime
/// value (`Ty::Type`) has no representation at all, and the runnable behavior reads only these erased
/// machine slots — never any type information, so nothing the compiler could not erase can affect it.
//= spec/capabilities/type-system.md#types-are-erased-from-the-component
//# The compiler MUST erase types from the emitted component so that the runnable form carries no runtime type reflection.
//= constitution.md#vii-strong-static-typing-is-mandatory
//# The compiler MUST erase types from the emitted component so that the runnable form carries no runtime type reflection.
//= spec/capabilities/type-system.md#types-are-erased-from-the-component
//# The behavior of an emitted component MUST NOT depend on any type information the compiler could not erase.
pub fn valtype_of(ty: &Ty) -> Option<ValType> {
    match ty {
        Ty::Int(it) => Some(int_valtype(*it)),
        Ty::Bool => Some(ValType::I32),
        Ty::Unit => None,
        // A record and a tuple are BOTH value-heap compounds — at run time each is an OPAQUE u32 HANDLE
        // into the runtime's store (a record IS a positional array; field names are compile-time-only),
        // occupying an i32 machine slot. So a runtime record/tuple value (a `let`-bound one, one threaded
        // between construct and use, one flowing through an `if` branch, or one escaping to the host)
        // lives in an i32 local. (A compound consumed only to read a field/element folds at compile time
        // and keeps no runtime slot; one that survives to selection genuinely crosses through a handle.)
        Ty::Record(_) | Ty::Tuple(_) => Some(ValType::I32),
        // A sum is a value-heap compound too — at run time an OPAQUE u32 HANDLE (the `sum-new`/
        // `sum-disc`/`sum-payload` handle), the nominal tag adding nothing to the representation
        // (`type-system.md §156`). So a runtime sum value lives in an i32 local, like a tuple/record.
        Ty::Sum { .. } => Some(ValType::I32),
        // A nominal's machine representation IS its underlying type's — the tag is erased at run time
        // (`§156` — it "adds nothing to the value's runtime representation"), so a `(type UserId (Mk
        // Int64))` value occupies an i64 slot exactly as a bare `Int64` does, a `(type Point (Mk Int64
        // Int64))` an i32 handle like the tuple it wraps. This read-THROUGH is what keeps the erased
        // construction/binding's slot matching the value's declared type.
        Ty::Nominal { inner, .. } => valtype_of(inner),
        // A list is a value-heap compound too — at run time an OPAQUE u32 HANDLE into the persistent
        // `vec-*` store, so it lives in an i32 local like a tuple/record/sum.
        Ty::List(_) => Some(ValType::I32),
        // A map is a value-heap compound too — at run time an OPAQUE u32 HANDLE into the persistent
        // CHAMP `map-*` store, so it lives in an i32 local like a list/tuple/record/sum.
        Ty::Map(_, _) => Some(ValType::I32),
        // A set is a value-heap compound too — an OPAQUE u32 HANDLE into the persistent CHAMP `set-*`
        // store, so it lives in an i32 local like a map/list/tuple/record/sum.
        Ty::Set(_) => Some(ValType::I32),
        // A bytes sequence is a value-heap leaf — at run time an OPAQUE u32 HANDLE into the persistent
        // rope `bytes-*` store, so it lives in an i32 local like a list/tuple/record/sum.
        Ty::Bytes => Some(ValType::I32),
        // A string is a value-heap value (a UTF-8 byte-rope handle), an i32 local like the compounds. A
        // CONSTANT string folds (no runtime slot); a runtime string handle lives here (its runtime ops
        // arrive with the byte-rope heap ops).
        Ty::String => Some(ValType::I32),
        // A char is a Unicode SCALAR VALUE (`0..=0x10FFFF`, surrogates excluded) — machine-identical to a
        // 32-bit integer, so a runtime `Char` value occupies an i32 slot exactly as a `UInt32`/`Int32`
        // does (its code point in the low 21 bits). This is the runtime Char rep (Char-rep 1/N): a char
        // threaded through a param/local/`if`-branch-join (e.g. `(Char.to-int (if b #\a #\z))`) now has a
        // real slot instead of declining. A CONSTANT char still folds where it can (equality/ordering at
        // compile time); the slot is for the genuinely-runtime char. Boxing into a compound / CHAMP key is
        // a later slice (the `box-int`/`get-int` scalar-cell path over the same i32 rep).
        Ty::Char => Some(ValType::I32),
        // A symbol is a nominal over a String — at run time an i32 HANDLE to its interned byte leaf, the
        // same slot a `String` uses. A CONSTANT symbol folds (no runtime slot this increment); this is
        // the slot a runtime symbol handle would occupy.
        Ty::Symbol => Some(ValType::I32),
        // A `BigInt` is a value-heap leaf — an i32 HANDLE to its sign-magnitude limb array (the
        // `Bytes`-leaf slot), so a runtime `BigInt` value (a param/local, an arithmetic result) lives in
        // an i32 local like a bytes/list handle. (B3b wired the runtime ops; a CONSTANT `BigInt` still
        // folds and keeps no slot.)
        Ty::BigInt => Some(ValType::I32),
        // A `Rational` is a value-heap compound (a normalized 2-BigInt-handle node — `rational-of`/the
        // runtime arithmetic return handles), so a runtime `Rational` value (a param, an arithmetic
        // result) lives in an i32 handle slot, like a BigInt/tuple/list handle (R3b wired the runtime ops).
        // A CONSTANT `Rational` still folds in `lower` and keeps no slot.
        Ty::Rational => Some(ValType::I32),
        // A float occupies its width's machine slot: `Float32` → f32, `Float64` → f64. A float literal
        // that crosses the boundary (or a float arithmetic result) lives here.
        Ty::Float(ft) => Some(if ft.ground_width() == 32 {
            ValType::F32
        } else {
            ValType::F64
        }),
        // A runtime FUNCTION VALUE (a closure) is a funcref-TABLE SLOT — an i32, like a heap handle. A
        // no-capture closure IS that slot directly; a capturing closure (later) is an i32 handle to a
        // heap cell holding the slot + captures. Either way the value occupies an i32 machine slot, so a
        // `Ty::Fn`-typed parameter / local / argument lives here. (A lambda that FOLDS away at compile
        // time never reaches a slot; only one that survives as a runtime value does — via `call_indirect`.)
        Ty::Fn(_, _) => Some(ValType::I32),
        // A reified continuation is a value-heap frame-chain HANDLE — an i32 machine slot, like a closure
        // handle. (Reserved: no program constructs a `Ty::Cont` until the step-3 heap rep lands; the slot
        // width is fixed here so the later reify/apply emit has it.)
        Ty::Cont { .. } => Some(ValType::I32),
        // A quantity ERASES to its inner numeric type before emission (`lower` strips the `Qty`), so its
        // machine slot IS the inner type's — a `(Qty Float64 meter)` occupies an f64 slot, byte-identical
        // to a bare `Float64`. A `Ty::Qty` should not survive to selection, but classify it by its inner
        // type defensively (never inventing a slot the erased value would not have).
        Ty::Qty { inner, .. } => valtype_of(inner),
        // A type value is compile-time-only (erased before runtime) — no machine representation. So a
        // type parameter is resolvable to a concrete type at compile time and a type-value never flows
        // from RUNTIME data into a type-determining position (a runtime `(UInt m)` width faults CDZ0302).
        //= spec/capabilities/type-system.md#generics-are-type-valued-parameters-not-a-separate-polymorphism-mechanism
        //# A type parameter MUST be resolvable to a concrete type at compile time, so that a type-value never flows from runtime data into a position that determines a type.
        Ty::Type => None,
        // An unresolved variable has no machine representation — an undetermined type never reaches a
        // real slot (it is a rejection at the boundary, not a defaulted representation).
        Ty::Var(_) => None,
        // A poison's `Any` type never reaches a real machine slot (a poison fails the build before
        // emission); treat it as no representation rather than guess one.
        Ty::Any => None,
    }
}

/// The wasm value type for an integer of a given (possibly-deferred) width — its GROUND width chooses
/// the slot: ≤32 bits is an i32, otherwise an i64. A deferred width grounds to the default (64 → i64).
fn int_valtype(it: IntTy) -> ValType {
    if it.ground_width() <= 32 {
        ValType::I32
    } else {
        ValType::I64
    }
}

/// The component-model boundary valtype byte a value of solved type `ty` lowers to when it crosses the
/// component edge, or `None` for a type with NO boundary representation (unit, a non-aliased integer
/// width, a compound). Fixed by the component ABI (`contracts/component-abi.md`): each integer crosses
/// as its FAITHFUL component primitive — `s8`/`u8`/`s16`/`u16`/`s32`/`u32`/`s64`/`u64` — and a bool as
/// `bool`. This is the wasm/component backend's boundary mapping (a target concern, kept here); the
/// CORE function signature is separate (`valtype_of` — a ≤32-bit value occupies an i32 slot regardless),
/// and the canonical ABI lifts/lowers the primitive↔core representation.
///
/// The component-model boundary valtype of a type, or `None` for a type with no boundary form. The
/// scalars that cross are: `Bool`; a `Float` (f32/f64); and — of the integers — ONLY THE EIGHT ALIASED
/// WIDTHS (8/16/32/64, each signedness) (`numeric-model.md` §The named widths are aliases; only they have
/// a boundary form). A tuple threaded internally maps to a `u32` handle (see below). A NON-ALIASED
/// integer width — `(UInt 7)`, `(UInt 48)`, … — is internal-only: it returns `None` here, so exporting or
/// taking it as a boundary parameter DECLINES rather than inventing a wider primitive that would misreport
/// the value's type at the edge. (Its constant bounds still fold and its arithmetic is correct; it simply
/// cannot cross the component boundary.) A record/sum/list/map/set/string/bytes value returns `None` too:
/// it escapes as the canonical binary value form via the resource `encode()` path, not a primitive valtype.
///
/// The mapping is a pure function of the type — each representable type has exactly ONE component-model
/// valtype, read from the generated `wasm_abi` table, so it does not vary with the compiler generation
/// that emits it:
///
//= spec/contracts/component-abi.md#every-exported-type-has-a-stable-boundary-representation
//# Each Cadenza type that may appear in an exported or imported signature MUST have a single stable representation in the host interface's type system.
///
//= spec/contracts/component-abi.md#every-exported-type-has-a-stable-boundary-representation
//# The boundary representation of a type MUST be a function of the type alone, independent of the compiler generation that emits it.
///
/// This mapping IS the pinned lowering: total over every type (a representable type yields its one fixed
/// component valtype, a no-boundary-form type yields `None`, so the function is defined everywhere), and
/// fixed by the ABI table rather than the emitting generation. The canonical ABI's lift of the same
/// primitive↔core representation is its inverse over the lowering's range.
//= spec/contracts/component-abi.md#lowering-and-lifting-are-fixed-inverses
//# The lowering of a Cadenza value to its boundary representation MUST be a total function fixed by this contract.
//= spec/contracts/component-abi.md#lowering-and-lifting-are-fixed-inverses
//# The lifting of a boundary representation to a Cadenza value MUST be the inverse of the pinned lowering for every value in the lowering's range.
pub fn comp_valtype_of(ty: &Ty) -> Option<u8> {
    match ty {
        Ty::Int(it) => {
            // Component-model primitive valtype bytes, from the generated `wasm_abi` table (extracted
            // from `wasm-encoder`'s `PrimitiveValType`): s8/u8/s16/u16/s32/u32/s64/u64. Only an aliased
            // width maps; any other width has no boundary form (`None`).
            match (it.ground_signed(), it.ground_width()) {
                (true, 8) => Some(wasm_abi::COMP_S8),
                (false, 8) => Some(wasm_abi::COMP_U8),
                (true, 16) => Some(wasm_abi::COMP_S16),
                (false, 16) => Some(wasm_abi::COMP_U16),
                (true, 32) => Some(wasm_abi::COMP_S32),
                (false, 32) => Some(wasm_abi::COMP_U32),
                (true, 64) => Some(wasm_abi::COMP_S64),
                (false, 64) => Some(wasm_abi::COMP_U64),
                // A non-aliased width (7, 24, 48, …) is internal-only — no boundary representation.
                _ => None,
            }
        }
        Ty::Bool => Some(wasm_abi::COMP_BOOL), // bool
        Ty::Unit => None,
        // A tuple threaded BETWEEN functions is a `u32` HANDLE — a ref into the composed runtime's
        // value store (the same opaque handle the heap ops thread). This is the INTERNAL representation;
        // it is NOT how a compound leaves the program to the host. A compound host-escape crosses as the
        // canonical binary value form through the resource `encode()` path (`wasm::emit`), not this
        // primitive-valtype mapping. Records and sums escape the same way, but they have no primitive
        // handle valtype defined here (they are not threaded as bare u32 params), so `None`. The handle is
        // OPAQUE — its byte meaning belongs to the runtime; the program threads it without dereferencing.
        //= spec/contracts/component-abi.md#a-runtime-value-crosses-as-an-opaque-handle
        //# A runtime value that crosses between a program and the value-heap runtime MUST cross as an opaque handle whose interpretation belongs solely to the runtime, so that the value's byte representation is the runtime's internal concern and never a layout the program or the host depends on.
        //= spec/contracts/component-abi.md#a-runtime-value-crosses-as-an-opaque-handle
        //# The program MUST NOT dereference or interpret a runtime handle, so that the acyclic reference-counted heap the runtime owns is not aliased by another linear memory and the handle is a capability-free token rather than a pointer into shared state.
        Ty::Tuple(_) => Some(0x79), // u32
        Ty::Record(_) => None,
        Ty::Sum { .. } => None,
        // A nominal crosses the boundary as its UNDERLYING type does — the tag is erased, so a `(type
        // UserId (Mk Int64))` crosses as its `Int64` primitive (`s64`), a nominal-over-tuple as the
        // tuple's `u32` handle. The host reads the value as its underlying representation; the nominal
        // NAME is carried in the type surface (`type_ast`), not the wire valtype.
        Ty::Nominal { inner, .. } => comp_valtype_of(inner),
        // A list escapes as the canonical binary value form via the resource `encode()` path, not a
        // primitive handle valtype (like a record/sum).
        Ty::List(_) => None,
        // A map escapes as the canonical binary value form via the resource `encode()` path (rendering
        // `(map (k v) …)` in sorted key order), not a primitive handle valtype (like a list/record/sum).
        Ty::Map(_, _) => None,
        // A set escapes as the canonical binary value form via the resource `encode()` path (rendering
        // `(Set.of (list …))` in sorted element order), not a primitive handle valtype.
        Ty::Set(_) => None,
        // Bytes escapes as the canonical binary value form via the resource `encode()` path (rendering
        // `b"…"`), not a primitive handle valtype (like a list/record/sum).
        Ty::Bytes => None,
        // A string escapes as the canonical binary value form via the resource `encode()` path, like a
        // list/record/sum — no primitive boundary valtype. (Constant string escape is a later increment.)
        Ty::String => None,
        // A char has no boundary valtype this increment — a constant char folds and never crosses; a char
        // value crossing the boundary is a later increment (declines cleanly until then).
        Ty::Char => None,
        // A symbol has no primitive boundary valtype — it would cross as the canonical value form via the
        // resource `encode()` path, like a String. (Constant symbol escape is a later increment; for now
        // a Symbol crossing the boundary declines cleanly.)
        Ty::Symbol => None,
        // A `BigInt` is NOT a primitive valtype — it has no primitive boundary form. It crosses the
        // boundary via the value-form escape (the value-encode walker renders it as a `KIND_INT` leaf),
        // like the compound types that escape via `encode()`, NOT through this primitive-valtype map — so
        // this returns `None` (the escape path in `mod.rs` handles the actual crossing).
        Ty::BigInt => None,
        // A `Rational` is likewise NOT a primitive valtype — it crosses via the value-form escape as a
        // `{numerator, denominator}` record (`ShapeNode::Rational`, R3c), handled by the escape path in
        // `mod.rs`, not this primitive-valtype map. So this returns `None`.
        Ty::Rational => None,
        // A float crosses the component boundary as its width's primitive: `Float32` → `f32`, `Float64`
        // → `f64`. Both admitted widths ({32,64}) have a faithful component-model float primitive.
        Ty::Float(ft) => Some(if ft.ground_width() == 32 {
            wasm_abi::COMP_F32
        } else {
            wasm_abi::COMP_F64
        }),
        // A function value does not cross the boundary (generics/functions monomorphize away or
        // decline); no boundary valtype.
        Ty::Fn(_, _) => None,
        // A reified continuation NEVER crosses the host boundary (the host-composition invariant: a `k` is
        // a chain of run-local heap handles a re-deriving host cannot reconstruct). So no boundary form.
        Ty::Cont { .. } => None,
        // A quantity ERASES to its inner numeric type — it crosses the boundary as that inner type
        // (`units-of-measure.md` §A quantity crossing a signature crosses as its underlying `T`). A
        // `(Qty Float64 meter)` export crosses as `f64`, byte-identical to a bare `Float64`.
        Ty::Qty { inner, .. } => comp_valtype_of(inner),
        // A type value is compile-time-only — it never crosses the boundary.
        Ty::Type => None,
        // An unresolved variable has no boundary representation (an undetermined type is rejected).
        Ty::Var(_) => None,
        Ty::Any => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::IntTy;

    fn comp(signed: bool, width: u32) -> Option<u8> {
        comp_valtype_of(&Ty::Int(IntTy::fixed(signed, width)))
    }

    #[test]
    fn aliased_widths_map_to_their_faithful_primitive() {
        // The eight aliased widths cross as their exact component primitive: s8/u8/s16/u16/s32/u32/
        // s64/u64. These are the LITERAL spec bytes, pinned independently of the generated `wasm_abi`
        // constants `comp_valtype_of` now reads — a wrong byte from codegen would fail here.
        assert_eq!(comp(true, 8), Some(0x7E)); // s8
        assert_eq!(comp(false, 8), Some(0x7D)); // u8
        assert_eq!(comp(true, 16), Some(0x7C)); // s16
        assert_eq!(comp(false, 16), Some(0x7B)); // u16
        assert_eq!(comp(true, 32), Some(0x7A)); // s32
        assert_eq!(comp(false, 32), Some(0x79)); // u32
        assert_eq!(comp(true, 64), Some(0x78)); // s64
        assert_eq!(comp(false, 64), Some(0x77)); // u64
    }

    #[test]
    fn non_aliased_widths_have_no_boundary_representation() {
        // A non-aliased width is internal-only — no component primitive, so it declines at the boundary
        // (7, 24, 48, 62, and even 1 — only 8/16/32/64 are aliased).
        for w in [1u32, 7, 24, 48, 62, 63] {
            assert_eq!(comp(true, w), None, "signed width {w} must not cross");
            assert_eq!(comp(false, w), None, "unsigned width {w} must not cross");
        }
    }
}
