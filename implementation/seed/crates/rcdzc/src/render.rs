//! The type-directed renderer — emits the wasm that walks a value-heap result and writes its
//! canonical text (`(tuple 7 8)`, `42`, `true`) into linear memory, so a compound result crosses the
//! run boundary as a string. Driven by the SOLVED `Ty` (not the old `Shape` re-derivation): a
//! `Ty::Tuple([Int, Int])` renders `(tuple ` + each element + `)`, reading the value through the
//! runtime's tag-free accessors (`arr-get`, `get-int`, …).
//!
//! Produces the render-fn code bodies (one per distinct compound type reached) + the `run : () -> i32`
//! body. `run` calls the entry (`main` = user func 0 = `RT_FUNC_BASE`), renders its result into a
//! cursor starting at `STR_BASE`, and writes the `(ptr, len)` return pair at offset 0.

// Drafted with the heap path; wired into the pipeline in the tuple slice. Staged, so silence the
// interim dead-code noise wholesale (see `heap.rs`).

use crate::heap::{RT_FUNC_BASE, RT_ITOA};
use crate::heap_envelope::himport;
use crate::op;
use crate::ty::Ty;
use crate::wasm::{sleb128, uleb128, uleb_bytes};

/// The linear-memory offset where the output string is assembled (above the ret-pair scratch).
const STR_BASE: i64 = 16;

/// Build the renderer for an entry returning `ret`, given the module's user-function count `n_user`
/// (render fns are indexed AFTER the user funcs). Returns `(render_bodies, run_body)` as
/// code-section entries. `n_user` fixes the first render fn's index: `RT_FUNC_BASE + n_user + pos`.
pub fn build(ret: &Ty, n_user: usize) -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
    let render_base = RT_FUNC_BASE + n_user as u32;
    let mut r = Renderer {
        types: Vec::new(),
        render_base,
    };

    // run: local 0 = the entry result (a handle for a compound, or the raw scalar), local 1 = cursor
    // (i32). The entry result local's TYPE is the entry return type's core valtype (i32 handle for a
    // compound/Bool, i64 for an Int; a Unit entry produces no value, so local 0 is unused).
    let mut run = Vec::new();
    // result = call the entry (user func 0). A Unit entry pushes nothing — skip storing it.
    let entry_produces_value = !matches!(ret, Ty::Unit);
    if entry_produces_value {
        run.push(op::CALL);
        uleb128(RT_FUNC_BASE as u64, &mut run);
        run.extend_from_slice(&[op::LOCAL_SET, 0]);
    }
    // cursor = STR_BASE
    run.push(op::I32_CONST);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::LOCAL_SET, 1]);
    // Render the entry result into the cursor. A compound entry's local 0 is a heap handle → the
    // handle path; a scalar entry's local 0 is the RAW value → render it directly (no unbox).
    let result_expr = vec![op::LOCAL_GET, 0];
    r.render_entry(ret, &result_expr, 1, &mut run)?;
    // write (ptr,len) at offset 0: ptr = STR_BASE, len = cursor - STR_BASE
    run.push(op::I32_CONST);
    sleb128(0, &mut run);
    run.push(op::I32_CONST);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::I32_STORE, 2, 0]);
    run.push(op::I32_CONST);
    sleb128(4, &mut run);
    run.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST]);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::I32_SUB, op::I32_STORE, 2, 0]);
    // return retptr 0
    run.extend_from_slice(&[op::I32_CONST, 0]);
    // Local decls: local 0 = entry result (its core valtype), local 1 = cursor (i32). Grouped: if
    // local 0 is i32 (handle/Bool) both are one i32 group of 2; else local 0 is i64 then local 1 i32.
    let run_body = match ret.core_valtype() {
        Some(crate::ir::ValType::I32) => code_body(&[0x01, 0x02, 0x7F], &run), // 2×i32
        Some(crate::ir::ValType::I64) => code_body(&[0x02, 0x01, 0x7E, 0x01, 0x7F], &run), // i64, i32
        Some(crate::ir::ValType::F64) => code_body(&[0x02, 0x01, 0x7C, 0x01, 0x7F], &run), // f64, i32
        // Unit entry: local 0 unused; just the cursor (one i32). Two i32s is harmless and uniform.
        None => code_body(&[0x01, 0x02, 0x7F], &run),
    };

    // Drain the worklist: a body per distinct compound type reached (may intern more as it goes).
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    let mut i = 0;
    while i < r.types.len() {
        let ty = r.types[i].clone();
        bodies.push(r.render_fn_body(&ty)?);
        i += 1;
    }
    Ok((bodies, run_body))
}

struct Renderer {
    /// The distinct compound types reached, in intern order; each gets one render fn at
    /// `render_base + pos`.
    types: Vec<Ty>,
    render_base: u32,
}

impl Renderer {
    /// Intern a compound type, returning its render-fn position (dedup by structural equality so one
    /// fn serves every occurrence of the same type).
    fn intern(&mut self, ty: &Ty) -> usize {
        if let Some(pos) = self.types.iter().position(|t| t == ty) {
            return pos;
        }
        self.types.push(ty.clone());
        self.types.len() - 1
    }

    fn fn_index(&self, pos: usize) -> u32 {
        self.render_base + pos as u32
    }

    /// Render the ENTRY result of type `ty` into the cursor. Unlike `render_into` (which renders a
    /// heap-RESIDENT value read through the runtime's box accessors), the entry result is the raw
    /// value the entry returned: a compound is a heap handle (same as `render_into`), but a SCALAR is
    /// the raw i64/i32/f64 (NOT boxed) — so it renders directly, without a `get-int`/`get-bool` unbox.
    fn render_entry(
        &mut self,
        ty: &Ty,
        val_expr: &[u8],
        cur: u32,
        c: &mut Vec<u8>,
    ) -> Result<(), String> {
        match ty {
            Ty::Int => {
                // Raw i64 → itoa(value, cursor) directly.
                c.extend_from_slice(val_expr);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(RT_ITOA as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
            Ty::Bool => {
                // Raw i32 → branch on it directly (no get-bool unbox).
                c.extend_from_slice(val_expr);
                c.push(op::IF);
                c.push(0x40);
                write_lit(b"true", cur, c);
                c.push(op::ELSE);
                write_lit(b"false", cur, c);
                c.push(op::END);
                Ok(())
            }
            // A compound entry result is a heap handle — identical to a heap-resident value.
            Ty::Tuple(_)
            | Ty::Record(_)
            | Ty::List(_)
            | Ty::Map(..)
            | Ty::Set(_)
            | Ty::Sum { .. }
            | Ty::Bytes
            | Ty::String
            | Ty::Unit => self.render_into(ty, val_expr, cur, c),
            // A Type-TYPED value (whose type is `Ty::Type`) renders `(: <the-type-name> Type)`. But this
            // arm is for when the VALUE's type is checked (at the entry level); we don't have the actual
            // TypeVal node here. For Layer 1, a type-value should never reach render (the fence catches
            // it). If it does, emit a placeholder.
            Ty::Type => {
                write_lit(b"(: <type> Type)", cur, c);
                Ok(())
            }
            Ty::Fn(..) => Err("cannot render a function value".to_string()),
            Ty::Param(_) | Ty::Var(_) => Err("cannot render an unsolved type".to_string()),
        }
    }

    /// Emit code that renders the value denoted by `h_expr` (bytes that push its i32 handle/scalar)
    /// of type `ty` into the cursor local `cur`, updating `cur`. A scalar renders inline; a compound
    /// calls its render fn (interning it).
    fn render_into(
        &mut self,
        ty: &Ty,
        h_expr: &[u8],
        cur: u32,
        c: &mut Vec<u8>,
    ) -> Result<(), String> {
        match ty {
            Ty::Int => {
                // For an Int LEAF that is boxed on the heap, `h_expr` pushes the boxed handle and we
                // `get-int` it; but at the TOP level `main` returns an i64 directly (not boxed). The
                // caller distinguishes: `render_into` is only called on a heap-resident value here, so
                // an Int reached inside a tuple is a boxed element (`get-int` the handle). The top Int
                // case never reaches here (a scalar entry takes the scalar frame, not this renderer).
                c.extend_from_slice(h_expr);
                c.push(op::CALL);
                uleb128(himport::GET_INT as u64, c);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(RT_ITOA as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
            Ty::Bool => {
                c.extend_from_slice(h_expr);
                c.push(op::CALL);
                uleb128(himport::GET_BOOL as u64, c);
                c.push(op::IF);
                c.push(0x40);
                write_lit(b"true", cur, c);
                c.push(op::ELSE);
                write_lit(b"false", cur, c);
                c.push(op::END);
                Ok(())
            }
            Ty::Unit => {
                write_lit(b"unit", cur, c);
                Ok(())
            }
            Ty::Tuple(_)
            | Ty::Record(_)
            | Ty::List(_)
            | Ty::Map(..)
            | Ty::Set(_)
            | Ty::Sum { .. }
            | Ty::Bytes
            | Ty::String => {
                // Compound: call its render fn with (handle, cur), take the returned cursor.
                let pos = self.intern(ty);
                c.extend_from_slice(h_expr);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(self.fn_index(pos) as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
            // A type-value should never reach render_into (the fence catches it at fold time).
            Ty::Type => Err(
                "a type-value reached render (erasure fence should have rejected it)".to_string(),
            ),
            Ty::Fn(..) => Err("cannot render a function value".to_string()),
            Ty::Param(_) | Ty::Var(_) => Err("cannot render an unsolved type".to_string()),
        }
    }

    /// The body of the render fn for a COMPOUND `ty`. Params: local 0 = handle, local 1 = cursor;
    /// returns the updated cursor. Writes the canonical text for the type, reading each element
    /// through `arr-get` + the element's leaf accessor / a recursive render call.
    fn render_fn_body(&mut self, ty: &Ty) -> Result<Vec<u8>, String> {
        match ty {
            Ty::Tuple(elems) => {
                let mut c = Vec::new();
                // write "(tuple"
                write_lit(b"(tuple", 1, &mut c);
                for (i, elem) in elems.iter().enumerate() {
                    // write a space separator
                    write_lit(b" ", 1, &mut c);
                    // element handle = arr-get(handle, i)
                    let elem_h = arr_get_expr(0, i);
                    self.render_into(elem, &elem_h, 1, &mut c)?;
                    let _ = i;
                }
                // write ")"
                write_lit(b")", 1, &mut c);
                // return cursor
                c.extend_from_slice(&[op::LOCAL_GET, 1]);
                Ok(code_body(&[0x00], &c)) // no extra locals (params 0,1 are handle,cursor)
            }
            Ty::Record(fields) => {
                // `(record (name0 v0) (name1 v1) …)` — fields are already SORTED by name (the `Ty`
                // canonical form), and the value-heap slots were sorted by the same key at
                // construction, so slot `i` holds the value of `fields[i].name`. The field NAME is
                // baked from the static type (the runtime is tag-free).
                let mut c = Vec::new();
                write_lit(b"(record", 1, &mut c);
                for (i, (name, elem)) in fields.iter().enumerate() {
                    // write " (name " then the element value then ")"
                    let mut open = b" (".to_vec();
                    open.extend_from_slice(name.as_bytes());
                    open.push(b' ');
                    write_lit(&open, 1, &mut c);
                    let elem_h = arr_get_expr(0, i);
                    self.render_into(elem, &elem_h, 1, &mut c)?;
                    write_lit(b")", 1, &mut c);
                }
                write_lit(b")", 1, &mut c);
                c.extend_from_slice(&[op::LOCAL_GET, 1]);
                Ok(code_body(&[0x00], &c))
            }
            Ty::List(elem) => {
                // `(list e0 e1 …)` — a list is the runtime's persistent sequence (`vec-*`), so its
                // LENGTH is runtime data: walk it with a counter loop over `vec-len`/`vec-get` (NOT the
                // fixed `arr-*` a tuple/record uses). Its representation is unobservable
                // (collections-and-text.md §A List's Representation Is Unspecified And Unobservable), so
                // it still renders `(list …)`. Local 0 = handle, local 1 = cursor, local 2 = the i32
                // loop counter (the one declared local this render fn needs).
                const I32_GE_S: u8 = 0x4E; // not in `op` (only the unsigned 0x4F is); the signed compare.
                let ctr: u8 = 2;
                let mut c = Vec::new();
                write_lit(b"(list", 1, &mut c);
                // i = 0
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, ctr]);
                // block { loop { if i >= vec-len(handle) break; ' ' ; render elem ; i += 1 ; continue } }
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                // i >= vec-len(handle) → br 1 (out of the block)
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::VEC_LEN as u64, &mut c);
                c.extend_from_slice(&[I32_GE_S, op::BR_IF, 1]);
                // separator, then render the element at vec-get(handle, i)
                write_lit(b" ", 1, &mut c);
                let elem_h = vec_get_expr(0, ctr);
                self.render_into(elem, &elem_h, 1, &mut c)?;
                // i += 1 ; continue
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, ctr, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]); // end loop, end block
                write_lit(b")", 1, &mut c);
                c.extend_from_slice(&[op::LOCAL_GET, 1]);
                // One declared local: the i32 counter (local 2).
                Ok(code_body(&[0x01, 0x01, 0x7F], &c))
            }
            Ty::Sum { def, args } => {
                // A sum renders `(Variant payload)` — the runtime is tag-free, so the compiler bakes the
                // variant NAME keyed by the runtime discriminant. Read `sum-disc(handle)` into a local,
                // then a cascade `if disc == i { write "(Name "; render payload; write ")" }`. The name
                // is BARE for an unqualified prelude sum (`Some`) or QUALIFIED `Type.Variant` for a
                // user/Sign sum. The payload type is the variant's template instantiated with `args`.
                let d: u8 = 2; // local 2 = the discriminant (the one declared local)
                let mut c = Vec::new();
                // d = sum-disc(handle)
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::SUM_DISC as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, d]);
                for (i, variant) in def.variants().iter().enumerate() {
                    // if d == i { … }  (a value-producing i32 block: each arm leaves the cursor)
                    c.extend_from_slice(&[op::LOCAL_GET, d, op::I32_CONST]);
                    sleb128(i as i64, &mut c);
                    c.push(0x46); // i32.eq
                    c.extend_from_slice(&[op::IF, 0x7F]); // → i32 (the updated cursor)
                    // write "(Name " (qualified `Type.Variant` for a qualified sum, else bare).
                    let mut open = Vec::from(&b"("[..]);
                    if def.qualified {
                        open.extend_from_slice(def.name.as_bytes());
                        open.push(b'.');
                    }
                    open.extend_from_slice(variant.name.as_bytes());
                    open.push(b' ');
                    write_lit(&open, 1, &mut c);
                    // render the payload: sum-payload(handle) at the variant's instantiated payload type
                    // (a nullary variant's payload is unit → renders `unit`).
                    let payload_ty = crate::ty::instantiate(&variant.payload, args);
                    let payload_h = vec![op::LOCAL_GET, 0, op::CALL, himport::SUM_PAYLOAD as u8];
                    self.render_into(&payload_ty, &payload_h, 1, &mut c)?;
                    write_lit(b")", 1, &mut c);
                    c.extend_from_slice(&[op::LOCAL_GET, 1]); // leave the cursor as this block's value
                    c.push(op::ELSE);
                }
                // Innermost else: unreachable (a well-typed sum's disc always matched a variant). Each
                // `if` block is i32-typed and leaves the updated cursor, so the whole nested-if
                // EXPRESSION yields the cursor — that IS the render fn's return value (no trailing get).
                c.push(op::UNREACHABLE);
                for _ in def.variants() {
                    c.push(op::END);
                }
                Ok(code_body(&[0x01, 0x01, 0x7F], &c)) // one declared local: the i32 discriminant
            }
            Ty::Bytes => {
                // `b"…"` — the byte-string display (10-bytes.sexp; the exact inverse of the `b"…"`
                // reader). Loop i in 0..bytes-len(h), read each byte via `bytes-get`, escape it: named
                // `\n \r \t \\ \"`; printable ASCII 0x20..=0x7e (except `"` and `\`) raw; else `\xNN`
                // (two lowercase hex). Locals: 0=handle, 1=cursor, 2=counter, 3=byte value, 4=nibble.
                const I32_GE_S: u8 = 0x4E;
                let (ctr, bv, nib): (u8, u8, u8) = (2, 3, 4);
                let mut c = Vec::new();
                write_lit(b"b\"", 1, &mut c);
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, ctr]);
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                // i >= bytes-len(h) → break
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::BYTES_LEN as u64, &mut c);
                c.extend_from_slice(&[I32_GE_S, op::BR_IF, 1]);
                // bv = bytes-get(h, i)
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, ctr, op::CALL]);
                uleb128(himport::BYTES_GET as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, bv]);
                emit_byte_escape(bv, nib, 1, &mut c);
                // i += 1 ; continue
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, ctr, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]);
                write_lit(b"\"", 1, &mut c);
                c.extend_from_slice(&[op::LOCAL_GET, 1]);
                // Three declared i32 locals: counter, byte value, nibble scratch.
                Ok(code_body(&[0x01, 0x03, 0x7F], &c))
            }
            Ty::String => {
                // `"…"` — the String display (13-strings.sexp: the escape set is CLOSED — `\n \r \t \\
                // \"` ONLY; EVERY other byte is written RAW, including multi-byte UTF-8 (≥0x80,
                // reproducing `café`/`😀` verbatim) and non-printable control bytes (no `\xNN`/`\u` — a
                // closed set with no numeric escape, so a rendered string reads back to the same value).
                // A String is a Bytes-backed leaf → loop `bytes-len`/`bytes-get`, escaping each byte.
                const I32_GE_S: u8 = 0x4E;
                let (ctr, bv): (u8, u8) = (2, 3);
                let mut c = Vec::new();
                write_lit(b"\"", 1, &mut c);
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, ctr]);
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::BYTES_LEN as u64, &mut c);
                c.extend_from_slice(&[I32_GE_S, op::BR_IF, 1]);
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, ctr, op::CALL]);
                uleb128(himport::BYTES_GET as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, bv]);
                emit_string_byte_escape(bv, 1, &mut c);
                c.extend_from_slice(&[op::LOCAL_GET, ctr, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, ctr, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]);
                write_lit(b"\"", 1, &mut c);
                c.extend_from_slice(&[op::LOCAL_GET, 1]);
                // Two declared i32 locals: counter, byte value (no nibble — no `\xNN`).
                Ok(code_body(&[0x01, 0x02, 0x7F], &c))
            }
            // A Map/Set VALUE crossing the run boundary must render `(map (k v) …)` / `(set …)` in
            // CANONICAL KEY ORDER (collections-and-text.md §A Map Renders As Its Entries In Canonical Key
            // Order) — but the runtime's `map-iter`/`set-iter` walk the CHAMP trie in HASH order, not
            // canonical text order. A faithful render needs to collect the entries and sort by canonical
            // text (the full canonicalization machine) — DECLINE (a later phase) rather than emit
            // hash-ordered output, which would be a nondeterministic miscompile. (Map/Set OPERATIONS work
            // — only RETURNING one at the boundary declines; a self-hosted compiler returns Bytes/Ast.)
            Ty::Map(..) | Ty::Set(_) => Err(
                "rendering a Map/Set value at the run boundary (canonical key order) is a later phase"
                    .to_string(),
            ),
            _ => Err("render fn requested for a non-compound type".to_string()),
        }
    }
}

/// Emit the `"…"` (String) escape of the byte in local `bv` into cursor `cur`: the CLOSED escape set
/// `\n \r \t \\ \"` (named), and EVERY other byte written RAW (13-strings.sexp — no `\xNN`/`\u`, so a
/// rendered string reads back identically; multi-byte UTF-8 and control bytes pass through verbatim).
fn emit_string_byte_escape(bv: u8, cur: u32, c: &mut Vec<u8>) {
    let named: &[(i64, &[u8])] = &[
        (10, b"\\n"),
        (13, b"\\r"),
        (9, b"\\t"),
        (92, b"\\\\"),
        (34, b"\\\""),
    ];
    let mut depth = 0u32;
    for (k, lit) in named {
        c.extend_from_slice(&[op::LOCAL_GET, bv, op::I32_CONST]);
        sleb128(*k, c);
        c.push(0x46); // i32.eq
        c.extend_from_slice(&[op::IF, 0x40]);
        write_lit(lit, cur, c);
        c.push(op::ELSE);
        depth += 1;
    }
    // else: write the byte RAW — `cur[0] = bv ; cur += 1`.
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::LOCAL_GET,
        bv,
        op::I32_STORE8,
        0,
        0,
    ]);
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::I32_CONST,
        1,
        op::I32_ADD,
        op::LOCAL_SET,
        cur as u8,
    ]);
    for _ in 0..depth {
        c.push(op::END);
    }
}

/// Emit the `b"…"` escape of the byte value in local `bv` (an i32 0..=255), writing into cursor `cur`
/// (using `nib` as a hex-nibble scratch local). A cascade of `if bv == k` for the named escapes and the
/// printable range, else `\xNN` with two lowercase hex digits. Matches the old compiler's
/// `emit_byte_escape` and the `b"…"` reader (10-bytes.sexp) so a rendered byte reads back identically.
fn emit_byte_escape(bv: u8, nib: u8, cur: u32, c: &mut Vec<u8>) {
    // Helper: `if bv == k { write lit; } else { … }` — opens an i32-neutral (empty) block per branch.
    // Named escapes: \n=10 \r=13 \t=9 \\=92 \"=34.
    let named: &[(i64, &[u8])] = &[
        (10, b"\\n"),
        (13, b"\\r"),
        (9, b"\\t"),
        (92, b"\\\\"),
        (34, b"\\\""),
    ];
    let mut depth = 0u32;
    for (k, lit) in named {
        c.extend_from_slice(&[op::LOCAL_GET, bv, op::I32_CONST]);
        sleb128(*k, c);
        c.push(0x46); // i32.eq
        c.extend_from_slice(&[op::IF, 0x40]);
        write_lit(lit, cur, c);
        c.push(op::ELSE);
        depth += 1;
    }
    // Printable ASCII 0x20..=0x7e (except `"`=34 and `\`=92, already handled above): write the byte raw.
    //   printable = (bv >= 32) & (bv <= 126)
    c.extend_from_slice(&[op::LOCAL_GET, bv, op::I32_CONST, 32, 0x4E /*i32.ge_s*/]);
    // `i32.const` takes a SIGNED LEB — 126 has bit 6 set, so its single byte 0x7E would decode as -2.
    // Use `sleb128` for any value ≥ 64.
    c.extend_from_slice(&[op::LOCAL_GET, bv, op::I32_CONST]);
    sleb128(126, c);
    c.push(0x4C); // i32.le_s
    c.push(0x71); // i32.and
    c.extend_from_slice(&[op::IF, 0x40]);
    // raw: cur[0] = bv ; cur += 1
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::LOCAL_GET,
        bv,
        op::I32_STORE8,
        0,
        0,
    ]);
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::I32_CONST,
        1,
        op::I32_ADD,
        op::LOCAL_SET,
        cur as u8,
    ]);
    c.push(op::ELSE);
    // else: `\xNN` — write '\','x', then the high and low hex nibbles.
    write_lit(b"\\x", cur, c);
    emit_hex_nibble(bv, nib, 4, cur, c); // high nibble = bv >> 4
    emit_hex_nibble(bv, nib, 0, cur, c); // low nibble  = bv & 15
    c.push(op::END); // close the printable-if
                     // Close all the named-escape else-blocks.
    for _ in 0..depth {
        c.push(op::END);
    }
}

/// Write one lowercase-hex digit of `bv` (the nibble at bit-shift `shift`: 4 = high, 0 = low) into
/// `cur`, via `nib`: `nib = (bv >> shift) & 15 ; nib += (nib < 10 ? '0' : 'a'-10) ; cur[0]=nib ; cur++`.
fn emit_hex_nibble(bv: u8, nib: u8, shift: i64, cur: u32, c: &mut Vec<u8>) {
    // nib = (bv >> shift) & 15
    c.extend_from_slice(&[op::LOCAL_GET, bv]);
    if shift != 0 {
        c.push(op::I32_CONST);
        sleb128(shift, c);
        c.push(0x76); // i32.shr_u
    }
    c.extend_from_slice(&[op::I32_CONST, 15, 0x71 /*i32.and*/, op::LOCAL_SET, nib]);
    // digit = nib + (nib < 10 ? 48 : 87)   (87 = 'a' - 10)
    c.extend_from_slice(&[
        op::LOCAL_GET,
        nib,
        op::I32_CONST,
        10,
        0x48, /*i32.lt_s*/
    ]);
    c.extend_from_slice(&[op::IF, 0x7F]); // → i32 (the ASCII digit)
    c.extend_from_slice(&[op::LOCAL_GET, nib, op::I32_CONST, 48, op::I32_ADD]);
    c.push(op::ELSE);
    // 87 = 'a'-10; bit 6 set, so single-byte would sign-extend — use sleb128.
    c.extend_from_slice(&[op::LOCAL_GET, nib, op::I32_CONST]);
    sleb128(87, c);
    c.push(op::I32_ADD);
    c.push(op::END);
    c.extend_from_slice(&[op::LOCAL_SET, nib]);
    // cur[0] = digit ; cur += 1
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::LOCAL_GET,
        nib,
        op::I32_STORE8,
        0,
        0,
    ]);
    c.extend_from_slice(&[
        op::LOCAL_GET,
        cur as u8,
        op::I32_CONST,
        1,
        op::I32_ADD,
        op::LOCAL_SET,
        cur as u8,
    ]);
}

/// Bytes that push the i32 handle of tuple element `index`: `arr-get(local[handle_local], index)`.
fn arr_get_expr(handle_local: u32, index: usize) -> Vec<u8> {
    let mut e = Vec::new();
    e.push(op::LOCAL_GET);
    uleb128(handle_local as u64, &mut e);
    e.push(op::I32_CONST);
    sleb128(index as i64, &mut e);
    e.push(op::CALL);
    uleb128(himport::ARR_GET as u64, &mut e);
    e
}

/// Bytes that push the i32 handle of list element `index`: `vec-get(local[handle_local], local[i_local])`
/// — the persistent-sequence read (himport 26), the list counterpart of `arr_get_expr`. `index` is a
/// LOCAL (the loop counter), not a compile-time constant, since a list's length is runtime data.
fn vec_get_expr(handle_local: u32, i_local: u8) -> Vec<u8> {
    let mut e = Vec::new();
    e.push(op::LOCAL_GET);
    uleb128(handle_local as u64, &mut e);
    e.extend_from_slice(&[op::LOCAL_GET, i_local]);
    e.push(op::CALL);
    uleb128(himport::VEC_GET as u64, &mut e);
    e
}

/// Emit `cur[k] = byte` for each byte of `lit`, advancing `cur` by `lit.len()`. A fixed-string write
/// into the output buffer.
fn write_lit(lit: &[u8], cur: u32, c: &mut Vec<u8>) {
    for &b in lit {
        // cur (address) ; byte ; i32.store8 ; then cur += 1
        c.push(op::LOCAL_GET);
        uleb128(cur as u64, c);
        c.push(op::I32_CONST);
        sleb128(b as i64, c);
        c.extend_from_slice(&[op::I32_STORE8, 0, 0]);
        c.push(op::LOCAL_GET);
        uleb128(cur as u64, c);
        c.extend_from_slice(&[op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET]);
        uleb128(cur as u64, c);
    }
}

/// Wrap a code body (`local-decls … end`) with its byte length — a code-section entry. `decls` is
/// the already-encoded local-declaration prefix (count + groups).
fn code_body(decls: &[u8], code: &[u8]) -> Vec<u8> {
    let mut inner = decls.to_vec();
    inner.extend_from_slice(code);
    inner.push(op::END);
    let mut out = uleb_bytes(inner.len() as u64);
    out.extend_from_slice(&inner);
    out
}
