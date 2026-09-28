; General WIT-ABI boundary — typed component exports crossing the canonical-ABI boundary.
;
; This file is the corpus home for the "general WIT-ABI" (shape-2) coverage that used to live as
; in-crate `wasmtime`-running #[test]s in rcdzc (`Component::from_binary` + `run_returns`). Per the
; operator (2026-08-25): migrate every such behavioral test OUT of the compiler and test it once,
; fully end-to-end, in the corpus — reusing this established `.sexp` format rather than a new one.
; The mission retires rcdzc's `wasmtime` dev-dependency once all its behavioral coverage lands here.
;
; A shape-2 case declares a guest that EXPORTS a typed function; `(call <export> <arg>...)` invokes it
; across the component boundary and `(output ...)` asserts the returned value. The interesting content
; is the ABI marshalling of the param/result TYPES (records, options, lists, variants/enums) as they
; cross the canonical component boundary — a broken lift (wrong discriminant, payload offset, or element
; shape) yields a different observable value. The WIT world is USUALLY SYNTHESIZED from the guest's own
; type annotations (no external artifact). A case may instead IMPOSE an explicit world with a
; `(wit-world <world-sexpr>)` + `(component-name "iface")` clause — the export then crosses under that
; named interface (`(call <export>)` is invoked as `<iface>#<export>`); an imposed-world case runs only
; on the wasm backend (Rust/ML decline → Todo, no external-world ingest there). World type-tag heads MUST
; be STRING LITERALS: `("record" …)`, `("option" …)`, `("list" …)`; field names / scalars stay bare.
;
; This file grows one shape at a time as v-rust-backend feeds each anchored shape; each corpus case
; that goes green retires its in-crate `run_returns` equivalent (no coverage gap).
;   SHAPE 1 — option<s64> RESULT (both arms).             SHAPE 5 — bare list<s64> RESULT.
;   SHAPE 2 — named VARIANT-with-payload RESULT.          SHAPE 7 — list-of-records RESULT.
;   SHAPE 3 — scalar identity export.                     SHAPE 8 — none-only option<s64> via an IMPOSED
;   SHAPE 4 — multi-field record RESULT.                    WIT world (element type from the world decl).
; Later shapes (all via an imposed WIT world): SHAPE 9 — list-of-records with a Bytes leaf + a Bytes export
; param. SHAPE 10 — a scalar host-import RESULT (clock.now) threaded into the step. SHAPE 11 — a RECORD
; host-import RESULT (probe.info) with a field-order-follows-WIT reorder. SHAPE 12 — a list<s64> host-op ARG
; (sink.push) lowered + invoked e2e (the align-8 scalar-element list-arg marshal). SHAPE 13 — an all-scalar
; record{a,b} host-op ARG (deliver.push) flattened to two i64 core slots + invoked e2e. SHAPE 14 — a Bytes
; host-op ARG with a scalar u64 RESULT (hasher.hash) threaded into the step's deadline-nanos. SHAPE 15 — a
; COMPOUND result<Bytes, enum> host-import RESULT (run.run) matched Ok(v)->payload / Err->fallback. SHAPE 16 —
; the option<s64> PARAM read (the read side: a record{d: option<s64>} param matched Some(x)->x / None->-1).
; SHAPE 17 — the result<Bytes, enum> PARAM read (Ok(bs)->Bytes.len / Err(e)->10+disc; both arms, enum-err arg).
; SHAPE 18 — a named-VARIANT RESULT writer NAME-matches (not positionally): a reversed guest decl still maps
; each case to the WIT case by name (Continue->continue disc 0, Close->close disc 1). SHAPE 19 — a record PARAM
; whose WIT field order is NOT name-lex is read by NAME (guest reads .contract across a payload,contract WIT order).
; SHAPE 20 — a record RESULT whose WIT field order is NOT name-lex is written by NAME (guest builds {first,second}
; against a second,first WIT result order). SHAPE 21 — a NESTED record param with a bytes leaf (record{a, sub:{b}})
; reads both the outer and the nested list<u8> leaf.
; SHAPE 22 — a record param carrying a list<u8> LEAF beside a scalar (record{data,tag}) reads the bytes leaf
; lifted through guest memory (Bytes.len).
; SHAPE 23 — a record PARAM interface export built via the boundary wrapper (f(record{a})=m.a, f({a:7})=7).
; SHAPE 24 — a nested list<list<s64>> host-op ARG (sink.push) recursively marshalled + invoked e2e. SHAPE 25 — a MULTI-EXPORT
; record interface (two members f,g), one boundary wrapper per member, both run under the interface.
; SHAPE 26 — a no-effects reducer step (empty requests list; dead element-writer derived from the WIT type).
; SHAPE 27 — the CAPSTONE full reducer-step: list<record> requests + 3 byte leaves + option + named variant, every field asserted.
; SHAPE 28 — a list<u8> LEAF param AND a spilled record result in ONE member (both memory paths + all scratch locals).
; SHAPE 29 — the flagship reducer-echo: the real message{contract,sender:{reducer,host},payload,token} (nested
; sender record) round-trips into the full step (param permute + nested-record + whole step writer at once).
; SHAPE 30 — a list<record{contract,n}> host-op ARG (sink.push): each record element written in place at
; canonical layout (s64 inline + Bytes rope spilled with (ptr,len) inline) + invoked e2e.
; SHAPE 31 — a record host-op ARG whose FIELD is a list<s64> (sink.push(record{ids, n})): the record flattens
; to core slots, the list field marshalled into mem + pushed as (ptr,count) + invoked e2e.
; SHAPE 32 — a BOOL host-import RESULT (kv.delete : (Bytes) -> bool) branched on (true -> one request), the
; runtime bool-branch coverage the platform state (delete returns unit) has no home for.
; SHAPE 33 — a list<tuple<s64,Bytes>> host-op ARG (sink.push): each tuple element written in place at its
; positional layout (s64 inline + Bytes rope spilled with (ptr,len)) + invoked e2e.
; SHAPE 34 — a list<tuple<Bytes,Bytes>> host-import RESULT (kv.prefix-scan) branched on List.len>0; needed a
; cdz-run coerce_one fix (record-erased sorting was misfiring on positional tuples of lists).
; SHAPE 35 — a record host-op ARG with an option<s64> FIELD (sink.push(record{d, n})): the record flattens,
; the option field to (disc, payload) — Some(42) -> (1,42) — + invoked e2e.
; SHAPE 36 — a record host-op ARG with an option<Bytes> FIELD (sink.push(record{d, n})): the option flattens
; to (disc, ptr, len), Some copies the payload rope + invoked e2e.
; SHAPE 37 — a record host-op ARG with a DIRECT Bytes FIELD beside a scalar (sink.push(record{b, n})): the
; bytes field rope marshalled into mem with (ptr,len) inline + invoked e2e.
; SHAPE 38 — a list<option<s64>> host-op ARG (sink.push): each option element written in place (disc byte +
; payload), both Some(5)->(1,5) and None->(0,0) arms + invoked e2e.
; SHAPE 39 — a list<record{option<s64>, n}> host-op ARG (sink.push): each record element written in place, its
; option field via emit_option_to_mem (Some + None across two elements) + invoked e2e.
; SHAPE 40 — a TOP-LEVEL bare list<u8>/Bytes PARAM member of a typed export interface (decode-check(list<u8>)
; -> bool): the wrapper copies the incoming (ptr,len) out of memory into a value-heap Bytes (mem_leaf_params
; lift) and reclaims it after the call — the decode-check half of the operator §2 two-export shape.
; SHAPE 41 — the CAPSTONE operator §2 shape: ONE component with BOTH exports — encode-quoted() -> list<u8>
; (bytes RESULT, CopyBytes) AND decode-check(list<u8>) -> bool (bytes PARAM, mem_leaf lift) — in one
; interface, proving the per-member wrappers (SHAPE 34/40) compose: each member emits its own wrapper.
; SHAPE 42 — a bare string/String PARAM member (MemLeafKind::Str), the byte-leaf-copy sibling of SHAPE 40.
; SHAPE 43 — a bare list<scalar>/List PARAM member (MemLeafKind::List): a value-heap VEC built element-by-
; element from the (ptr,count) layout (distinct rep from Bytes), reading count+element to prove stride+box.
; SHAPE 44 — a bare option<scalar>/Option PARAM member (sum_params): the (disc,payload) flattening is rebuilt
; into the guest sum cell via sum-new (SumArgRebuild), both Some and None arms exercised, shell dropped after.
; SHAPE 45/46/47 — MULTI/MIXED top-level param composition in one member: two mem-leaf params (Bytes+list),
; a mem-leaf param interleaved with a scalar, and a sum (option) param beside a mem-leaf — each pins that the
; wrapper's flattened-leaf CURSOR advances correctly across differently-sized top-level params (2 for a
; (ptr,len) mem-leaf, 1 for a scalar, disc+payload for a sum). A broken cursor would misread a later param.
; SHAPE 48 — a bare tuple<…>/Tuple PARAM member (a POSITIONAL record): the wrapper builds the value-heap cell
; with the same arr-alloc/arr-set shape as a record param, identity slots (no name-permute). Position-sensitive
; witness catches a swapped element. Before this a tuple param mis-emitted a handle-erased scalar param.
; SHAPE 49/50 — a tuple PARAM whose element is COMPOUND: tuple<list<u8>, s64> (a Bytes-leaf element, exercising
; the tuple arm's param_field_rebuild BytesLeaf recursion + the copy-in scratch) and tuple<record{a}, s64> (a
; nested-record element, the Nested rebuild) — each read + combined with the scalar element to prove recursion.
; SHAPE 51 — a bare result<ok,err>/Result PARAM member (sum_params, Result shape): the (disc,payload-JOIN)
; flattening with Ok=disc 0 is rebuilt via the Result-shaped SumArgRebuild; both Ok and Err arms exercised.
(diagnostic-quality)

(case
  "an option<s64> field in a record result VALUE round-trips via the run/encode envelope both arms (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc
    "The general option<T> RESULT lift across the WIT export boundary. The guest takes a record
           `{ x: Int64 }` and returns a record `{ d: Option Int64 }`, mapping x=0 to None and any
           other x to Some(x). Asserting BOTH arms exercises the Option discriminant (Some vs None)
           and, on the Some arm, the s64 payload — a broken lift (wrong disc, wrong payload offset,
           or a dropped payload) produces a different d. Migrated from the in-crate wasmtime test
           `an_option_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 1/18).")
  (input
    (do
      (def
        (f (: m (Record (: x Int64))))
        #record((= d (if (= m.x 0) Option.None (Option.Some m.x)))))
      (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (output (: #record((= d (Some 42))) (record (d (Option Int64)))))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= d (None unit))) (record (d (Option Int64)))))
  (live-objects 0))

(case
  "a bare Result<s64,String> RESULT round-trips DIRECTLY (unwrapped) via the run/encode envelope both arms"
  (doc
    "The bare-DIRECT sum RESULT: the guest returns a `Result Int64 String` UNWRAPPED (not the
           record-wrapped twin above, not a param — a top-level sum RESULT crossing via the run/encode
           envelope). Ok(x) carries an s64 payload, Err(msg) a String payload, exercising BOTH the
           discriminant and BOTH payload lanes (scalar + heap-String). A broken result lift (wrong disc,
           dropped/misplaced payload) yields a different value. This is the UNTYPED-envelope route and is
           DISTINCT from the still-open typed-WIT-export gap noted on the record-wrapped case above (those
           cases carry no wit-world clause either, but a DECLARED typed record/sum EXPORT interface remains
           a separate gap). Motivated by #8268: the differential wasm oracle (oracle-lean) now DECODES
           built-in Option/Result heap `.sum` results at the boundary — this fences the corpus-gate
           value-render side (which, unlike the oracle, already handles built-in AND user sums) so the two
           observation paths cannot silently drift on the bare-sum-result shape. Census pin PER-CALL
           `(live-objects 0 0)` under drop-before-census (operator 2026-09-19): the two arms rc-traced
           (v-memory-safety, post-#8394): call 0 `(mk 5)` -> `(Ok 5)` produces a Result Sum node that the host
           OWNS at census (a benign escaping-value retain, node count 1) — the harness now models the HOST
           resource-dropping its transferred value before census, reclaiming that node → 0; call 1 `(mk 0)` ->
           `(Err \"z\")` retains 0 already (the Err payload is a CONST-IMMORTAL String literal → no fresh heap
           node → the returned Sum reclaims to 0). Under host-drop both arms census 0 (the per-call vector is
           retained to document that the arms still differ pre-drop). NOT a leak; a guest-side reclaim of the
           escaping return value would UAF the host's transferred value."
    "tri-target: wasm + rust + cadenza-hop all PASS.")
  (input
    (do
      (def (mk (: x Int64)) (if (= x 0) (Err "z") (Ok x)))
      (export mk)))
  (call mk (: 5 Int64))
  (output (: (Ok 5) (Result Int64 String)))
  (call mk (: 0 Int64))
  (output (: (Err "z") (Result Int64 String)))
  (live-objects 0 0))

(case
  "a bare LIST of String RESULT round-trips via the run/encode envelope (recursive String decode in a list container)"
  (doc
    "The recursive-String-in-a-CONTAINER heap result: the guest returns a `List String` of three
           varying-length string literals crossing via the run/encode envelope. The decode must walk the
           list and, per element, decode a heap String — the recursive type-directed String fixup. A broken
           fixup (a dropped element, a wrong length, a swapped ptr) yields a different list. Motivated by
           #8256 (oracle-wasm recursive type-directed String fixup — decode nested String, replacing the
           #8246 decline): this fences the corpus-gate value-render side of the same nested-String heap
           result so the oracle-decode and gate-render paths cannot drift. No prior case pins a
           `#list(\"…\")` VALUE output. No census pin (in-process count non-discriminating for the escaping
           list-of-heap-Strings result)."
    "tri-target: wasm + rust + cadenza-hop all PASS.")
  (input (do (def (mk) #list("a" "bb" "ccc")) (export mk)))
  (call mk)
  (output (: #list("a" "bb" "ccc") (List String))))

(case
  "a named variant-with-payload field in a record result VALUE round-trips via the run/encode envelope both arms (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc
    "SHAPE 2 — the general named-VARIANT RESULT lift (a discriminated union, distinct from a bare
           enum: one case carries a payload). The guest returns a record `{ o: Outcome }` where
           `Outcome = Continue | Close(Int64)`, mapping x=0 to Continue (nullary) and any other x to
           Close(x) (s64 payload). Asserting BOTH arms exercises the variant discriminant AND the
           payload case's s64 lane — a broken lift (wrong disc, missing/misplaced payload) yields a
           different o. Migrated from the in-crate wasmtime test
           `a_named_variant_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 2/18).")
  (input
    (do
      (type Outcome (Continue) (Close Int64))
      (def
        (f (: m (Record (: x Int64))))
        #record((= o (if (= m.x 0) Outcome.Continue (Outcome.Close m.x)))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= o (Continue unit))) (record (o Outcome))))
  (call f (: #record((= x 7)) (Record (: x Int64))))
  (output (: #record((= o (Close 7))) (record (o Outcome))))
  (live-objects 0))

(case
  "a scalar export crosses the WIT boundary and returns its argument"
  (doc
    "SHAPE 3 — the simplest WIT export boundary: s64 in, s64 out (identity). Exercises the plain
           scalar param/result canonical-ABI lowering with no compound structure. Migrated from the
           in-crate wasmtime test `a_scalar_interface_export_guest_compiles_and_runs` (v-rb feed 4).")
  (input (do (def (f (: x Int64)) x) (export f)))
  (call f (: 7 Int64))
  (output (: 7 Int64)))

(case
  "a multi-field record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc
    "SHAPE 4 — the multi-field RECORD RESULT spill: the guest returns { a, b } from a record
           input, exercising the record result lift (two s64 fields, in order). Migrated from the
           in-crate wasmtime test `a_record_result_guest_compiles_and_runs_via_result_spill` (v-rb feed 5).")
  (input (do (def (f (: m (Record (: x Int64)))) #record((= a m.x) (= b (+ m.x m.x)))) (export f)))
  (call f (: #record((= x 21)) (Record (: x Int64))))
  (output (: #record((= a 21) (= b 42)) (Record (: a Int64) (: b Int64))))
  (live-objects 0))

(case
  "a bare list of scalars in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc
    "SHAPE 5 — the bare-LIST RESULT: a record field that is a list<s64>, exercising the list
           result lift (element count + s64 element stride). Migrated from the in-crate wasmtime test
           `a_list_result_guest_compiles_and_runs` (v-rb feed 6).")
  (input
    (do (def (f (: m (Record (: x Int64)))) #record((= xs #list(m.x (+ m.x m.x))))) (export f)))
  (call f (: #record((= x 5)) (Record (: x Int64))))
  (output (: #record((= xs #list(5 10))) (record (xs (List Int64)))))
  (live-objects 0))

(case
  "a list of records in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc
    "SHAPE 7 — the list-of-records RESULT (the structural workhorse of every reducer Step's
           `requests: list<request>`): a record field that is a LIST of RECORDS. The guest returns
           `{ items: [ { a, b } ] }` with one element built from the input (a=x, b=x+x). Exercises the
           LIST result lift (count + stride) AND the element RECORD lift (two s64 fields in order).
           Migrated from the in-crate wasmtime test `a_list_of_records_result_guest_compiles_and_runs`
           (v-rust-backend shape-2 feed 3/18).")
  (input
    (do
      (def
        (f (: m (Record (: x Int64))))
        #record((= items #list(#record((= a m.x) (= b (+ m.x m.x)))))))
      (export f)))
  (call f (: #record((= x 7)) (Record (: x Int64))))
  (output
    (:
      #record((= items #list(#record((= a 7) (= b 14)))))
      (record (items (List (record (a Int64) (b Int64)))))))
  (live-objects 0))

(case
  "a FLOAT-KEYED map result round-trips via the run/encode envelope, rendering its float keys (float map-key render, #6211 key-adopt/use + #6274 render)"
  (doc
    "The float-KEYED map RESULT: a `(Map Float32 Int64)` crosses via the run/encode value-form escape
           (crosses_as_resource_escape) and renders `#map((= <key> <value>) …)` in canonical key order. The
           keys are Float32 — a bare `2.0` ADOPTS the annotated `(: 1.0 Float32)` sibling's width (seq-40),
           and each float key is stored in the total-order `__CdzF32` Ord wrapper on the rust backend. Pins
           that BOTH backends render the float keys correctly: the wasm value-form render_val (v-rb) and the
           rust boundary value-form render (which must UNWRAP the `__CdzF{N}` shell before the float render —
           the E0605/E0624/private-interface chain #6274 closed). Both keys 1.0/2.0 are f32-exact, so the
           render shows `1.0`/`2.0`. Guards the float-keyed-collection RENDER path against regression.")
  (input
    (do (def (main) (Map.insert (Map.insert Map.empty (: 1.0 Float32) 5) 2.0 6)) (export main)))
  (call main)
  (output (: #map((= 1.0 5) (= 2.0 6)) (Map Float32 Int64)))
  ; tighten (v-memory-safety): known-leak->0 pin hygiene — float-keyed map RESULT round-trip (escv value-render, #6211 key-adopt/use + #6274 render). Determinism-CLEARED by v-corpus-harness (opt-sweep x3, O0-O3, 142/0-divergence 12 execs — map result host-dropped-before-census so keys reclaim WITH the map; float-key canonicalization affects only the rendered VALUE, not the live count). Value-render round-trip, NOT key-reclaim code — key-ownership CODE hold untouched. Pin-flip only.
  (live-objects 0))

(case
  "a none-only option<s64> record field resolves its element type from an imposed WIT world"
  (doc
    "SHAPE 8 — the general WIT-ABI shape where the export boundary is DECLARED by an explicit WIT
           world, not synthesized from the guest. The guest's field d is STATICALLY Option.None (no Some
           arm), so its option element type cannot be inferred from a payload — it is fixed by the world's
           `d: option<s64>` declaration (the `(wit-world …)` clause). The guest exports f UNDER the
           interface `cadenza:demo/iface` (`(component-name …)`), so the run invokes it through that
           interface instance. A broken WIT-element-type resolution fails to emit or mis-types d. Migrated
           from the in-crate wasmtime test `a_none_only_option_result_resolves_via_wit`.")
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param m (record (= x (s64)))) (result (record (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) #record((= d Option.None))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output #record((= d (None unit))))
  (live-objects 0))

(case
  "a list of records with a bytes leaf crosses the export boundary via an imposed WIT world"
  (doc
    "SHAPE 9 — the reducer-echo Step shape (list of records each carrying a Bytes/list<u8> leaf,
           AND a Bytes/list<u8> export PARAM), via an imposed world. Guest f: {tok: Bytes} ->
           {items: [{echo: tok}]}; exercises the list result lift, the element record lift, a bytes leaf
           that echoes the input, AND the list<u8> export-param decode. A guest Bytes field crosses as the
           world's list<u8>. Migrated from the in-crate wasmtime test
           `a_list_of_records_with_bytes_leaf_result_compiles_and_runs` (v-rb shape-2 feed 7).")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= tok (list (u8)))))
            (result (record (= items (list (record (= echo (list (u8)))))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: m (Record (: tok Bytes)))) #record((= items #list(#record((= echo m.tok))))))
      (export f)))
  (call f (: #record((= tok #list(10 20 30))) (Record (: tok Bytes))))
  (output #record((= items #list(#record((= echo b"\n\x14\x1e"))))))
  (live-objects 0))

(case
  "a bare list<u8>/Bytes RESULT member of a typed export interface crosses (multi-export list<u8> result — operator §2 encode-quoted half)"
  (doc
    "SHAPE 34 — a TOP-LEVEL bare `list<u8>`/Bytes RESULT member of a DECLARED export interface (not a
           nested record leaf like SHAPE 9). The def returns a value-heap Bytes handle; the typed-interface
           wrapper copies the runtime bytes into a `cabi_realloc`'d buffer and writes the canonical
           `(ptr,len)` `list<u8>` return (`ResultLower::CopyBytes` — the multi-member-interface twin of the
           single-export bytes provider `emit_bytes_roundtrip_apply_body`). This is the `encode-quoted`
           half of the operator-mandated single-component TWO-export shape (§2, seq-107/108). The bytes
           cross type-blind as `list<u8>` so the boundary render is `#list` (the consumer's `decode-check`
           takes the `list<u8>` wire back). Guards the bytes-RESULT-member emit against regression.")
  (wit-world (world w (export iface (member encode-quoted (func (result (list (u8))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (encodeQuoted) (Bytes.of #list(104 105))) (export encodeQuoted)))
  (call encode-quoted)
  (output #list(104 105))
  (live-objects 0))

(case
  "a bare list<u8>/Bytes PARAM member of a typed export interface crosses (multi-export list<u8> param — operator §2 decode-check half)"
  (doc
    "SHAPE 40 — a TOP-LEVEL bare `list<u8>`/Bytes PARAM member of a DECLARED export interface (not a
           record leaf like SHAPE 22, nor a single bare export like the plain-export entry-param route). The
           typed-interface wrapper copies the incoming `(ptr,len)` `list<u8>` out of linear memory into a
           value-heap Bytes handle (the `mem_leaf_params` lift — `bytes-alloc`/`bytes-set` copy-in), passes it
           to the def, and reclaims the borrowed handle after the call (`drop`). This is the `decode-check`
           half of the operator-mandated single-component TWO-export shape (§2, seq-107/108) — the inverse of
           the `encode-quoted` bytes-RESULT member (`ResultLower::CopyBytes`). Guest decodeCheck(x: Bytes) =
           Bytes.len(x) > 0; calling with `#list(104 105)` returns true, proving the byte-leaf param copy-in +
           the borrow reclaim end to end. Guards the bytes-PARAM-member lift against regression.")
  (wit-world
    (world w (export iface (member decode-check (func (param x (list (u8))) (result (bool)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (decodeCheck (: x Bytes)) (> (Bytes.len x) 0)) (export decodeCheck)))
  (call decode-check (: #list(104 105) Bytes))
  (output (: true Bool))
  (live-objects 0))

(case
  "a bare string/String PARAM member of a typed export interface crosses (mem_leaf Str-arm coverage)"
  (doc
    "SHAPE 42 — a TOP-LEVEL bare `string`/String PARAM member of a typed export interface. Same
           `mem_leaf_params` copy-in as SHAPE 40's `list<u8>`/Bytes, but `MemLeafKind::Str`: a Cadenza String
           IS a flat UTF-8 byte-leaf, so a WIT `string` param (guaranteed valid UTF-8) copies straight into a
           value-heap String handle with NO `str-from-bytes` decode — only the boundary TYPE differs from the
           Bytes case. Pins the Str arm of the increment-2 param lift (SHAPE 40 witnessed only the Bytes arm).
           Guest checkStr(x: String) = String.byte-len(x) > 0; \"hi\" -> true. The wrapper reclaims the borrowed
           String handle after the call, same borrow-only 0-leak lift as the Bytes param.")
  (wit-world (world w (export iface (member check-str (func (param x (string)) (result (bool)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (checkStr (: x String)) (> (String.byte-len x) 0)) (export checkStr)))
  (call check-str (: "hi" String))
  (output (: true Bool))
  (live-objects 0))

(case
  "a single component exports BOTH a list<u8>-result member and a list<u8>-param member of one interface (operator §2 two-export capstone)"
  (doc
    "SHAPE 41 — the CAPSTONE of the operator-mandated single-component TWO-export shape (§2, seq-107/108):
           ONE interface with BOTH members — encode-quoted() -> list<u8> (the bytes-RESULT member, emitted via
           `ResultLower::CopyBytes`, SHAPE 34) AND decode-check(list<u8>) -> bool (the bytes-PARAM member,
           lifted via the `mem_leaf_params` copy-in, SHAPE 40). `record_interface_export` emits one boundary
           wrapper PER member, so this proves the two independently-landed member emitters COMPOSE in a single
           component: the result-copy-out wrapper and the param-copy-in wrapper coexist, share the one memory +
           `cabi_realloc` + the two `list<u8>` scratch locals, and each crosses its own bytes independently.
           Guest defines encodeQuoted() = Bytes.of([104,105]) and decodeCheck(x) = Bytes.len(x) > 0; running
           BOTH (encode-quoted -> #list(104 105); decode-check([104,105]) -> true) exercises the full two-export
           boundary end to end — the shape the operator's real encode/decode contract needs.")
  (wit-world
    (world
      w
      (export
        iface
        (member encode-quoted (func (result (list (u8)))))
        (member decode-check (func (param x (list (u8))) (result (bool)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (encodeQuoted) (Bytes.of #list(104 105)))
      (def (decodeCheck (: x Bytes)) (> (Bytes.len x) 0))
      (export encodeQuoted)
      (export decodeCheck)))
  (call encode-quoted)
  (output #list(104 105))
  (call decode-check (: #list(104 105) Bytes))
  (output (: true Bool))
  (live-objects 0))

(case
  "a bare list<scalar>/List PARAM member of a typed export interface crosses (mem_leaf List-arm coverage)"
  (doc
    "SHAPE 43 — a TOP-LEVEL `list<s64>`/List Int64 PARAM member of a typed export interface. Unlike the
           Bytes byte-leaf copy (SHAPE 40), `MemLeafKind::List` builds a value-heap VEC element-by-element
           (`vec-empty` + per-element load-at-stride / `box-int` / `vec-push`) from the canonical `(ptr, count)`
           layout — a DISTINCT value rep from Bytes (boxed elements, not a packed byte-leaf). The guest reads
           BOTH the count AND an element value to prove the stride + box are right: readElem(xs) =
           100*List.len(xs) + xs[1]; [7,42,9] -> 342 (a broken stride/box would mis-read element 1). Borrow-only
           0-leak lift (the wrapper drops the vec after the call). Mirrors the bare-entry route's list<scalar>
           param (el4/el5, ch09) onto the typed-interface-MEMBER route.")
  (wit-world
    (world w (export iface (member read-elem (func (param xs (list (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (readElem (: xs (List Int64)))
        (+ (* 100 (List.len xs)) (match (List.at xs 1) ((Option.Some v) v) ((Option.None) -1))))
      (export readElem)))
  (call read-elem (: #list(7 42 9) (List Int64)))
  (output (: 342 Int64))
  (live-objects 0))

(case
  "a bare option<scalar>/Option PARAM member of a typed export interface crosses (sum_params option arm, both variants)"
  (doc
    "SHAPE 44 — a TOP-LEVEL `option<s64>`/Option Int64 PARAM member of a typed export interface. The
           member crosses as a native component `option<s64>` flattened to `(disc, payload)`; the wrapper
           branches on the boundary disc and builds the guest sum cell via `sum-new` (`SumArgRebuild`), passes
           it to the def, and drops the borrowed shell after the call (the extracted payload escapes by its own
           copy, independent of the shell). Mirrors the bare-entry route's Option param (eo1/eo2, ch09) onto
           the typed-interface-MEMBER route — the `sum_params` sibling of the mem_leaf param arms. Guest
           checkOpt(x) = match x (Some v)->v (None)->-1; BOTH variants exercised: Some(42)->42, None->-1 (a
           broken disc read or payload-cursor would take the wrong arm).")
  (wit-world
    (world w (export iface (member check-opt (func (param x (option (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (checkOpt (: x (Option Int64))) (match x ((Option.Some v) v) ((Option.None) -1)))
      (export checkOpt)))
  (call check-opt (: (Some 42) (Option Int64)))
  (output (: 42 Int64))
  (call check-opt (: (None unit) (Option Int64)))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a member with TWO top-level mem-leaf params (list<u8> + list<s64>) threads the flattened cursor across both"
  (doc
    "SHAPE 45 — a typed-interface member with TWO top-level memory-bearing params: a `list<u8>`/Bytes AND
           a `list<s64>`/List, each flattening to `(ptr, len)`. The wrapper copies BOTH out of memory (bytes-leaf
           + list-vec) in sequence, advancing the flattened-leaf cursor by 2 per param, then reclaims both. A
           cursor that failed to advance past the first `(ptr,len)` would read the second param at the wrong
           offset. Guest combine(b, xs) = Bytes.len(b) + List.len(xs); ([1,2,3], [10,20]) -> 5. Pins the
           multi-mem-leaf-param composition established by SHAPE 40/43.")
  (wit-world
    (world
      w
      (export
        iface
        (member combine (func (param b (list (u8))) (param xs (list (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (combine (: b Bytes) (: xs (List Int64))) (+ (Bytes.len b) (List.len xs)))
      (export combine)))
  (call combine (: #list(1 2 3) Bytes) (: #list(10 20) (List Int64)))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a member with a mem-leaf param interleaved with a scalar param threads the cursor across mixed widths"
  (doc
    "SHAPE 46 — a typed-interface member mixing a top-level `list<u8>`/Bytes param (flattens to `(ptr,len)`
           = 2 leaves) with a bare `s64` SCALAR param (1 leaf). The wrapper must advance the flattened-leaf
           cursor by 2 for the mem-leaf and by 1 for the scalar; a miscount would swap them. Guest tag(x, n) =
           Bytes.len(x) + n; ([7,8,9], 100) -> 103. Pins the mem-leaf + scalar mixed-width param threading.")
  (wit-world
    (world
      w
      (export iface (member tag (func (param x (list (u8))) (param n (s64)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (tag (: x Bytes) (: n Int64)) (+ (Bytes.len x) n)) (export tag)))
  (call tag (: #list(7 8 9) Bytes) (: 100 Int64))
  (output (: 103 Int64))
  (live-objects 0))

(case
  "a member with an option<s64> param beside a mem-leaf param composes the sum rebuild with the byte copy-in"
  (doc
    "SHAPE 47 — a typed-interface member with a top-level `option<s64>` param (sum rebuild: disc + payload)
           beside a `list<u8>`/Bytes param (mem-leaf: ptr + len). The wrapper builds the sum cell (branching on
           the disc, advancing the cursor past disc+payload) THEN copies the bytes out (advancing past ptr+len),
           reclaiming both borrowed cells. Pins the sum-param + mem-leaf-param cursor composition (a broken sum
           payload-cursor would offset the bytes param). Guest both(o, b) = (match o Some->v None->0) +
           Bytes.len(b); (Some 40, [1,2]) -> 42.")
  (wit-world
    (world
      w
      (export
        iface
        (member both (func (param o (option (s64))) (param b (list (u8))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (both (: o (Option Int64)) (: b Bytes))
        (+ (match o ((Option.Some v) v) ((Option.None) 0)) (Bytes.len b)))
      (export both)))
  (call both (: (Some 40) (Option Int64)) (: #list(1 2) Bytes))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a bare tuple<scalar,scalar>/Tuple PARAM member of a typed export interface crosses (positional cell rebuild)"
  (doc
    "SHAPE 48 — a TOP-LEVEL `tuple<s64,s64>`/Tuple Int64 Int64 PARAM member of a typed export interface.
           A tuple is a POSITIONAL record: the canon lift flattens it depth-first, and the wrapper rebuilds the
           value-heap cell with the SAME `arr-alloc`/`arr-set` shape as a record param but with IDENTITY slots
           (no WIT-vs-name-lex permute — a tuple has no field names). Reuses `param_field_rebuild` per element
           (recursing on a compound element). Guest sumPair(p) = 100*p.0 + p.1 — POSITION-SENSITIVE so a
           swapped or mis-slotted element is caught; (5,10) -> 510. Before this a top-level tuple param declined
           in record_interface_export and fell through to a handle-erased scalar (`u32`) param the boundary
           driver could not marshal a tuple arg against. The arg is written in the native `#tuple(5 10)` form
           (the M3 native-#ctor form; `coerce_one`'s tuple-arg parser accepts it for a `tuple<…>` param and it
           exercises the same positional cell rebuild — verified 510).")
  (wit-world
    (world w (export iface (member sum-pair (func (param p (tuple (s64) (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do (def (sumPair (: p (Tuple Int64 Int64))) (+ (* 100 (. p 0)) (. p 1))) (export sumPair)))
  (call sum-pair (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 510 Int64))
  (live-objects 0))

(case
  "a tuple PARAM member whose element is a list<u8>/Bytes leaf rebuilds the compound element in-cell"
  (doc
    "SHAPE 49 — a TOP-LEVEL `tuple<list<u8>, s64>`/Tuple(Bytes, Int64) PARAM member. The tuple arm's
           per-element `param_field_rebuild` recurses into the Bytes element as a `FieldRebuild::BytesLeaf`
           (the same byte copy-in + scratch locals a record's bytes leaf uses), beside the scalar element — so
           a compound tuple element crosses correctly, not just scalars. Guest f(p) = Bytes.len(p.0) + p.1;
           ([1,2,3], 100) -> 103. Pins the tuple arm composing with a mem-bearing element.")
  (wit-world
    (world w (export iface (member f (func (param p (tuple (list (u8)) (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: p (Tuple Bytes Int64))) (+ (Bytes.len (. p 0)) (. p 1))) (export f)))
  (call f (: #tuple(#list(1 2 3) 100) (Tuple Bytes Int64)))
  (output (: 103 Int64))
  (live-objects 0))

(case
  "a tuple PARAM member whose element is a nested record rebuilds the compound element in-cell"
  (doc
    "SHAPE 50 — a TOP-LEVEL `tuple<record{a: s64}, s64>`/Tuple(Record, Int64) PARAM member. The tuple
           arm's per-element `param_field_rebuild` recurses into the record element as a `FieldRebuild::Nested`
           (building the inner record cell), beside the scalar element. Guest f(p) = p.0.a + p.1;
           ({a:7}, 100) -> 107. Pins the tuple arm composing with a nested-record element (the recursive
           cell-build the record-param route uses, now reached positionally through a tuple).")
  (wit-world
    (world
      w
      (export iface (member f (func (param p (tuple (record (= a (s64))) (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do (def (f (: p (Tuple (Record (: a Int64)) Int64))) (+ (. (. p 0) a) (. p 1))) (export f)))
  (call f (: #tuple(#record((= a 7)) 100) (Tuple (Record (: a Int64)) Int64)))
  (output (: 107 Int64))
  (live-objects 0))

(case
  "a bare result<ok,err>/Result PARAM member of a typed export interface crosses (sum_params Result shape, both arms)"
  (doc
    "SHAPE 51 — a TOP-LEVEL `result<s64,s64>`/Result Int64 Int64 PARAM member of a typed export interface.
           A `result<ok,err>` crosses as `(disc: i32, payload-JOIN)` with Ok=disc 0 (Err=1) and the payload the
           JOIN of the ok/err leaves (`wrap_join` recovers a narrower side). The wrapper branches on the disc
           and rebuilds the guest sum cell via the Result-shaped `SumArgRebuild` (the SAME lift used for a
           `result<…>` record FIELD, SHAPE 17 — now reached as a TOP-LEVEL param). The bare-entry route DECLINES
           a Result param (it synthesizes the WIT as a `variant`, a disagreeing type); here the WIT is DECLARED
           `result<ok,err>`, matching the rebuild — so the typed-interface member route supports it. Guest
           chk(r) = match r (Ok v)->v (Err e)->-e; BOTH arms: Ok(7)->7, Err(5)->-5 (a wrong disc — Ok/Err
           swapped, or the option Some=1 convention — would take the wrong arm). Borrow-only (wrapper drops the
           shell).")
  (wit-world
    (world w (export iface (member chk (func (param r (result (s64) (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (chk (: r (Result Int64 Int64))) (match r ((Result.Ok v) v) ((Result.Err e) (- 0 e))))
      (export chk)))
  (call chk (: (Ok 7) (Result Int64 Int64)))
  (output (: 7 Int64))
  (call chk (: (Err 5) (Result Int64 Int64)))
  (output (: -5 Int64))
  (live-objects 0))

(case
  "a bare enum RESULT member of a typed export interface crosses (payloadless-enum disc passthrough)"
  (doc
    "SHAPE 52 — a TOP-LEVEL bare `enum{…}` RESULT member of a typed export interface. An all-nullary
           Cadenza sum (`db.is_enum_disc`) is represented as its raw i32 DISCRIMINANT (no heap handle), which
           IS the canonical-ABI core rep of a WIT `enum` (`flatten(enum) = [i32]`), so `record_result_lower`
           passes it straight through (`ResultLower::Passthrough`) — the def returns the disc directly and the
           declared WIT `enum` becomes the member's result type. GUARD: the guest case order (kebab) must equal
           the WIT case order (else the disc would index the wrong case — a reorder needs a runtime remap, a
           later increment). Guest returns Color.Green over `enum{red,green,blue}`; it crosses + renders the
           canonical nullary-variant value `(green unit)`. Pins the enum-result-member passthrough (distinct
           from an enum as a record FIELD or a host-result — a top-level enum export member).")
  (wit-world (world w (export iface (member choose (func (result (enum red green blue)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Color (Red) (Green) (Blue)) (def (choose) Color.Green) (export choose)))
  (call choose)
  (output (green unit))
  (live-objects 0))

(case
  "a bare enum PARAM member of a typed export interface crosses (payloadless-enum disc passthrough)"
  (doc
    "SHAPE 77 — a TOP-LEVEL bare `enum{…}` PARAM member of a typed export interface: the PARAM twin of
           SHAPE 52's enum RESULT. An all-nullary Cadenza sum (`db.is_enum_disc`) is represented as its raw i32
           DISCRIMINANT (no heap handle), which IS the canonical-ABI core rep of a WIT `enum`
           (`flatten(enum) = [i32]`), so the enum arg crosses as a bare i32 disc the wrapper hands the def
           directly (`record_interface_export`'s enum-param arm; `enum_disc_params` = identity passthrough when
           the guest case order == the WIT case order). Guest `f` matches its `Color` param → 1/2/3; `red`→1,
           `green`→2, `blue`→3, proving all three discs decode. GUARD: guest case order (kebab) must equal the
           WIT case order (a reorder needs a runtime disc remap — a later increment). The param analogue of the
           enum-result-member passthrough SHAPE 52 (distinct from an enum as a record FIELD or a host arg).")
  (wit-world (world w (export iface (member f (func (param c (enum red green blue)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export f)))
  (call f (: (red unit) Color))
  (output (: 1 Int64))
  (call f (: (blue unit) Color))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a reducer performing a scalar host import threads the u64 result into the step (via an imposed WIT world)"
  (doc
    "SHAPE 10 — a scalar host-import RESULT (clock.now : () -> u64) driven through an imposed WIT world.
           The reducer on-message performs clock.now (nullary scalar host op) and threads the u64 into the
           step's request deadline-nanos = Some(now). Stubbing clock.now -> 42 + asserting deadline-nanos ==
           Some(42) makes the scalar host result LOAD-BEARING. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_scalar_host_import_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import cadenza:platform/clock (member now (func (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect clock (op now (-> Unit UInt64)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (clock)
          #record((=
              requests
              #list(#record((= contract m.contract)
                  (= payload m.payload)
                  (= token m.token)
                  (= deadline-nanos (Option.Some (clock.now unit))))))
            (= outcome Outcome.Continue))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond clock.now (: 42 UInt64)))
  (host-calls (call cadenza:platform/clock.now))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(2))
            (= token #list(3))
            (= deadline-nanos (Some 42)))))
      (= outcome (continue unit))))
  ; tighten (v-memory-safety): known-leak->0 pin hygiene — reducer-escape via imposed WIT world (result threaded/written out the envelope), post-#9310 escv class. Pin-flip only (no reclaim code). Census-validated per-chapter GREEN. (float-keyed map-result sibling HELD pending v-corpus-harness float-key determinism clearance.)
  (live-objects 0))

(case
  "a reducer performing a RECORD host import reads a field of the result (via an imposed WIT world)"
  (doc
    "SHAPE 11 — a RECORD host-import RESULT (probe.info : (Bytes) -> record{zebra, alpha}) driven through
           an imposed WIT world; the host RECORD's declared order (zebra, alpha) differs from the guest name-lex
           order, exercising the field-order-follows-WIT reorder on the lift. The reducer reads the result's
           `alpha` field into the request payload. Stubbing probe.info -> {zebra:(9), alpha:(7)} and asserting
           payload == (7) makes the record host result + its field-reorder load-bearing. Migrated from the
           in-crate wasmtime test `a_reducer_performing_a_record_result_host_op_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/probe
        (member
          info
          (func
            (param key (list (u8)))
            (result (record (= zebra (list (u8))) (= alpha (list (u8))))))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect probe (op info (-> Bytes (Record (: zebra Bytes) (: alpha Bytes)))))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (probe)
          #record((=
              requests
              #list(#record((= contract m.contract)
                  (= payload (. (probe.info m.token) alpha))
                  (= token m.token)
                  (= deadline-nanos Option.None))))
            (= outcome Outcome.Continue))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses
    (respond
      probe.info
      (: #record((= zebra #list(9)) (= alpha #list(7))) (Record (: zebra Bytes) (: alpha Bytes)))))
  (host-calls (call cadenza:platform/probe.info))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(7))
            (= token #list(3))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list-of-scalars host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 12 — a `list<s64>` host-op ARG (sink.push : (list<s64>) -> unit) driven through an imposed
           WIT world. The reducer on-message performs sink.push (list 1 2 3) (a unit-result, observe-only host
           op) then returns a Continue step with an empty requests list. Running the guest LOADS the emitted
           component and INVOKES sink.push, exercising the list<s64> arg lower (value-heap List<Int64> -> the
           (ptr,count) the component list<s64> param lowers to, elements unboxed at i64 stride) e2e — a strictly
           stronger check than the anchor's emit+validate+load, since a broken flatten-arity or element stride
           fails to instantiate or run. The observed host-call sequence pins that sink.push actually fired.
           Migrated from the in-crate wasmtime test `a_reducer_performing_a_list_scalar_arg_emits_and_loads`
           (v-rust-backend shape-2 synthetic arg feed, align-8 stress).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import cadenza:platform/sink (member push (func (param vals (list (s64))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List Int64) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do (sink.push #list(1 2 3)) #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing an all-scalar record host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 13 — an all-scalar `record { a: s64, b: s64 }` host-op ARG (deliver.push : (record{a,b}) -> unit)
           driven through an imposed WIT world. The reducer on-message performs deliver.push (record a=1 b=2)
           (a unit-result, observe-only host op) then returns a Continue step with an empty requests list.
           Running the guest LOADS the emitted component and INVOKES deliver.push, exercising the record arg
           lower (value-heap record -> the two i64 core slots the component record param flattens to, each field
           unboxed in the WIT record's declared order) e2e — a strictly stronger check than the anchor's
           emit+validate+load, since a broken field flatten (wrong arity/order/offset) fails to instantiate or
           run. The observed host-call sequence pins that deliver.push actually fired. Migrated from the in-crate
           wasmtime test `a_typed_reducer_with_a_record_arg_host_import_emits_and_loads` (v-rb shape-2 arg feed).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/deliver
        (member push (func (param r (record (= a (s64)) (= b (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect deliver (op push (-> (Record (: a Int64) (: b Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (deliver)
          (do
            (deliver.push #record((= a 1) (= b 2)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/deliver.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a bytes host arg with a scalar result threads the u64 into the step (via an imposed WIT world)"
  (doc
    "SHAPE 14 — a Bytes host-op ARG with a scalar RESULT (hasher.hash : (Bytes) -> u64) driven through an
           imposed WIT world. The reducer on-message performs hasher.hash(m.payload) (a list<u8> ARG, u64 result)
           and threads the u64 into the step's request deadline-nanos = Some(hash). Stubbing hasher.hash -> 42
           and asserting deadline-nanos == Some(42) makes the call load-bearing: the u64 result only reaches the
           output if the Bytes arg lowered and the call succeeded. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_bytes_param_host_import_emits_and_loads` (v-rb shape-2 arg feed 2/6).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import cadenza:platform/hasher (member hash (func (param bytes (list (u8))) (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect hasher (op hash (-> Bytes UInt64)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (hasher)
          #record((=
              requests
              #list(#record((= contract m.contract)
                  (= payload m.payload)
                  (= token m.token)
                  (= deadline-nanos (Option.Some (hasher.hash m.payload))))))
            (= outcome Outcome.Continue))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond hasher.hash (: 42 UInt64)))
  (host-calls (call cadenza:platform/hasher.hash))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(2))
            (= token #list(3))
            (= deadline-nanos (Some 42)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a run.run result host op threads the Ok bytes into the step (via an imposed WIT world)"
  (doc
    "SHAPE 15 — a COMPOUND result<Bytes, enum> host-import RESULT (run.run : (Bytes,Bytes,Bytes) -> result<list<u8>, variant{timeout,faulted}>)
           driven through an imposed WIT world. The reducer on-message performs run.run(contract,contract,payload) and
           matches the result: Ok(v) -> the request payload is v; Err(_) -> the payload falls back to m.payload. Stubbing
           run.run -> Ok(b\"RAN\") and asserting payload == (82 65 78) makes the compound result<T,E> lift load-bearing (the
           spilled result disc + Ok list<u8> payload). Migrated from the in-crate wasmtime test
           `a_reducer_performing_run_with_a_result_host_result_emits_and_loads` (v-rb shape-2 run.run feed; #3301 landed the
           RunSink host half so this needs NO Entry::Run — host-responses stubs the result + output-assertion suffices).
           The sole-use world builder is RETAINED (a separate WIT-type unit test still reads it), so only the anchor retires.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/run
        (member
          run
          (func
            (param program (list (u8)))
            (param contract (list (u8)))
            (param input (list (u8)))
            (result (result (list (u8)) (variant (timeout) (faulted)))))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type Error (Timeout) (Faulted))
      (effect run (op run (-> Bytes Bytes Bytes (Result Bytes Error))))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (run)
          #record((=
              requests
              #list(#record((= contract m.contract)
                  (=
                    payload
                    (match
                      (run.run m.contract m.contract m.payload)
                      ((Ok v) v)
                      ((Err _e) m.payload)))
                  (= token m.token)
                  (= deadline-nanos Option.None))))
            (= outcome Outcome.Continue))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond run.run (: (Ok #list(82 65 78)) (Result Bytes Error))))
  (host-calls (call cadenza:platform/run.run))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(82 65 78))
            (= token #list(3))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "an option<s64> param field is read and rebuilt by the wrapper on both arms (via an imposed WIT world)"
  (doc
    "SHAPE 16 — the option<T> PARAM lift (the read side, complement to SHAPE 1's option RESULT): the guest
           f takes a record { d: option<s64> } and matches it (Some(x) -> x, None -> -1). Feeding d=Some(42) -> 42
           and d=None -> -1 exercises BOTH arms of the boundary option read (disc None=0/Some=1 + the payload at
           the variant payload offset), which the wrapper rebuilds into the guest option cell. The export crosses
           under the interface cadenza:demo/iface. Migrated from the in-crate wasmtime test
           `an_option_param_field_is_read_by_the_wrapper` (the param-side variant reader, deadline-nanos read shape).")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= d (option (s64))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: m (Record (: d (Option Int64)))))
        (match m.d ((Option.Some x) x) (Option.None -1)))
      (export f)))
  (call f (: #record((= d (Some 42))) (Record (: d (Option Int64)))))
  (output (: 42 Int64))
  (call f (: #record((= d None)) (Record (: d (Option Int64)))))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a result<Bytes, enum> param field is read and rebuilt by the wrapper on Ok and Err arms (via an imposed WIT world)"
  (doc
    "SHAPE 17 — the result<Ok, Err> PARAM lift (the read side, complement to SHAPE 15's result RESULT): the
           guest f takes a record { a: result<Bytes, Error> } where Error is a 4-case enum, and matches m.a:
           Ok(bs) -> Bytes.len(bs); Err(e) -> 10 + e's decl disc. Feeding Ok([1,2,3]) -> 3 exercises the Bytes Ok
           arm (ptr/len copy-in inside the sum); Err(timeout) -> 10 and Err(faulted) -> 13 exercise the Enum Err
           arm (the flattened disc leaf rebuilt into the guest error cell, disc-preserving) - a misread of the
           Bytes arm's 2 leaves would misalign the enum disc and flip the Err results. The enum-err ARG is passed
           as the render form `(<case> unit)` (cdz-run's coerce_one gained a Type::Enum arm for this). Migrated
           from the in-crate wasmtime test `a_result_bytes_enum_param_field_is_read_by_the_wrapper`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= a (result (list (u8)) (enum timeout missing schema faulted)))))
            (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Error (Timeout) (Missing) (Schema) (Faulted))
      (def
        (f (: m (Record (: a (Result Bytes Error)))))
        (match
          m.a
          ((Result.Ok bs) (Bytes.len bs))
          ((Result.Err e)
            (match e (Error.Timeout 10) (Error.Missing 11) (Error.Schema 12) (Error.Faulted 13)))))
      (export f)))
  (call f (: #record((= a (Ok #list(1 2 3)))) (Record (: a (Result Bytes Error)))))
  (output (: 3 Int64))
  (call f (: #record((= a (Err (timeout unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 10 Int64))
  (call f (: #record((= a (Err (faulted unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 13 Int64))
  (live-objects 0))

(case
  "a named-variant result writer name-matches (not positionally) with a reversed guest decl (via an imposed WIT world)"
  (doc
    "SHAPE 18 — the named-VARIANT writer keys on the case NAME, not decl position. The guest declares its sum
           in the OPPOSITE order to the WIT variant cases: (type Rev (Close Int64) (Continue)) (Close is guest
           decl-disc 0) against the world's variant { continue, close(s64) } (continue is boundary case 0). A
           name-match maps Close->close (boundary disc 1) and Continue->continue (boundary disc 0); a POSITIONAL
           match would put Close(payload) onto the nullary continue case and the payload-shape guard would reject
           at compile. A green run of BOTH arms (x=0 -> continue, x=5 -> close 5) proves the writer is keyed on the
           case NAME, immune to guest decl reordering. Migrated from the in-crate wasmtime test
           `the_named_variant_writer_name_matches_not_positionally`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= x (s64))))
            (result (record (= o (variant (continue) (close (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Rev (Close Int64) (Continue))
      (def
        (f (: m (Record (: x Int64))))
        #record((= o (if (= m.x 0) Rev.Continue (Rev.Close m.x)))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output #record((= o (continue unit))))
  (call f (: #record((= x 5)) (Record (: x Int64))))
  (output #record((= o (close 5))))
  (live-objects 0))

(case
  "a record param field is read by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc
    "SHAPE 19 — a record PARAM whose WIT field order differs from the guest name-lex order is read by NAME, not
           position. The world declares f's param as record { payload: list<u8>, contract: list<u8> } (WIT order
           payload,contract), while the guest reads (. m contract) and returns Bytes.len(m.contract). Calling with
           payload=[9,9] (len 2) and contract=[1,2,3] (len 3) must return 3 (the contract length): a positional
           misroute would read payload and return 2. Proves the param permute keys on the field NAME across the
           WIT/guest order mismatch. Migrated from the in-crate wasmtime test
           `a_non_name_lex_record_param_permutes_by_name`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func (param m (record (= payload (list (u8))) (= contract (list (u8))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) (Bytes.len m.contract))
      (export f)))
  (call
    f
    (:
      #record((= payload #list(9 9)) (= contract #list(1 2 3)))
      (Record (: contract Bytes) (: payload Bytes))))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a record result field is written by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc
    "SHAPE 20 — the record RESULT writer places fields by NAME, not by the guest name-lex slot order. The
           world declares f's result as record { second: s64, first: s64 } (WIT order second,first; name-lex is
           first < second), the shape of a real step/request (declaration-ordered, not alphabetical). The guest
           builds { first: m.x, second: 2*m.x }; the writer must place first at WIT-position 1 and second at
           WIT-position 0, reading each from its guest name-lex slot. f({x:10}) renders (in WIT order) as
           { second: 20, first: 10 } - a positional write would swap them. Migrated from the in-crate wasmtime
           test `a_non_name_lex_record_result_permutes_by_name`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func (param m (record (= x (s64)))) (result (record (= second (s64)) (= first (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: m (Record (: x Int64)))) #record((= first m.x) (= second (+ m.x m.x))))
      (export f)))
  (call f (: #record((= x 10)) (Record (: x Int64))))
  (output #record((= second 20) (= first 10)))
  (live-objects 0))

(case
  "a nested record param with a bytes leaf compiles and runs (via an imposed WIT world)"
  (doc
    "SHAPE 21 — a record PARAM with a NESTED record field carrying a list<u8> leaf, the shape of a reducer
           message's sender (a record-within-record with byte leaves). The guest reads both the outer bytes leaf
           and the nested one: f(m: record{a: Bytes, sub: record{b: Bytes}}) = Bytes.len(m.a) + Bytes.len(m.sub.b).
           The wrapper builds the outer value-heap cell with a nested sub-cell for sub, copying each list<u8> leaf
           out of shared memory. f({a:[1,2], sub:{b:[1,2,3]}}) == 5 (len(a)=2 + len(sub.b)=3). Migrated from the
           in-crate wasmtime test `a_nested_record_bytes_param_guest_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= a (list (u8))) (= sub (record (= b (list (u8)))))))
            (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: m (Record (: a Bytes) (: sub (Record (: b Bytes))))))
        (+ (Bytes.len m.a) (Bytes.len m.sub.b)))
      (export f)))
  (call
    f
    (:
      #record((= a #list(1 2)) (= sub #record((= b #list(1 2 3)))))
      (Record (: a Bytes) (: sub (Record (: b Bytes))))))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a record param carrying a bytes leaf beside a scalar compiles and runs (via an imposed WIT world)"
  (doc
    "SHAPE 22 — a record PARAM carrying a list<u8> LEAF beside a scalar (the memory boundary every real
           reducer needs: Message/Step carry list<u8>). The canon lift lowers the incoming `data` list into the
           guest's linear memory, the wrapper copies those bytes into a value-heap Bytes, builds the {data, tag}
           record, and the def returns Bytes.len(data). f({data:[1,2,3,4,5], tag:99}) == 5 proves the copied bytes
           have the right length (bytes-alloc + the copy loop + the memory lift all agree end to end). Migrated
           from the in-crate wasmtime test `a_record_with_a_bytes_leaf_guest_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param m (record (= data (list (u8))) (= tag (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: data Bytes) (: tag Int64)))) (Bytes.len m.data)) (export f)))
  (call f (: #record((= data #list(1 2 3 4 5)) (= tag 99)) (Record (: data Bytes) (: tag Int64))))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a record param interface export builds the record via the boundary wrapper and runs (via an imposed WIT world)"
  (doc
    "SHAPE 23 — a RECORD-param interface export handled by the boundary WRAPPER (the on-message(message)->step
           shape, record in). The canon lift hands the def the flattened field; the wrapper builds the value-heap
           record handle then calls the def. Guest f(m: record{a: s64}) = m.a; f({a:7}) == 7 proves the wrapper's
           record build (arr-alloc/box-int) + the field read agree end to end. Migrated from the in-crate wasmtime
           test `a_record_param_guest_compiles_and_runs_via_a_wrapper`.")
  (wit-world
    (world w (export iface (member f (func (param m (record (= a (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Int64)))) m.a) (export f)))
  (call f (: #record((= a 7)) (Record (: a Int64))))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a multi-export record interface guest emits a wrapper per member and runs both (via an imposed WIT world)"
  (doc
    "SHAPE 25 — a MULTI-EXPORT record-interface guest: the world's interface iface has TWO record-param
           members f(record{a: s64})->s64 and g(record{b: s64})->s64 (the shape a real reducer needs:
           on-message/on-response/on-notification are separate members). The compiler emits one boundary wrapper
           per member appended to the core module. Guest defines both f(m)=m.a and g(m)=m.b; running BOTH under
           the interface (f({a:7})==7, g({b:9})==9) proves each member's wrapper builds its own record + reads its
           own field independently. Migrated from the in-crate wasmtime test
           `a_multi_export_record_interface_guest_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param m (record (= a (s64)))) (result (s64))))
        (member g (func (param m (record (= b (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: m (Record (: a Int64)))) m.a)
      (def (g (: m (Record (: b Int64)))) m.b)
      (export f)
      (export g)))
  (call f (: #record((= a 7)) (Record (: a Int64))))
  (output (: 7 Int64))
  (call g (: #record((= b 9)) (Record (: b Int64))))
  (output (: 9 Int64))
  (live-objects 0))

(case
  "a reducer emitting no effects builds an empty-requests step and runs (via an imposed WIT world)"
  (doc
    "SHAPE 26 - a reducer that emits NO effects: an empty requests list with a Continue outcome, a very common
           output (a fold that only reads or updates state). The empty list has an unresolved element type, so the
           result writer must derive the dead element writer of the request list from the WIT type alone
           (canon_write_from_wit - the same principle as the None-only option), or emit falls through to a
           wrong-signature component. Migrated from `a_reducer_emitting_no_effects_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= contract (list (u8))) (= payload (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (s64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (def
        (f (: m (Record (: contract Bytes) (: payload Bytes))))
        #record((= requests #list()) (= outcome Outcome.Continue)))
      (export f)))
  (call
    f
    (:
      #record((= contract #list(1)) (= payload #list(2)))
      (Record (: contract Bytes) (: payload Bytes))))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a full reducer-step-shaped guest writes every field of the step and runs (via an imposed WIT world)"
  (doc
    "SHAPE 27 - the CAPSTONE full reducer-step-shaped guest, the whole result writer end to end. The guest
           returns one request whose contract and token both copy m.contract, payload copies m.payload, and
           deadline-nanos is Some(5), with a Continue outcome. Exercises record permute (step and request are
           declaration-ordered) plus list-of-records plus three byte leaves plus option plus a named variant - the
           reducer-echo step shape. Asserting every field pins the whole step lift. Migrated from
           `a_full_step_shaped_guest_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= contract (list (u8))) (= payload (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (s64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (def
        (f (: m (Record (: contract Bytes) (: payload Bytes))))
        #record((=
            requests
            #list(#record((= contract m.contract)
                (= payload m.payload)
                (= token m.contract)
                (= deadline-nanos (Option.Some 5)))))
          (= outcome Outcome.Continue)))
      (export f)))
  (call
    f
    (:
      #record((= contract #list(170 187)) (= payload #list(1 2 3)))
      (Record (: contract Bytes) (: payload Bytes))))
  (output
    #record((=
        requests
        #list(#record((= contract b"\xaa\xbb")
            (= payload b"\x01\x02\x03")
            (= token b"\xaa\xbb")
            (= deadline-nanos (Some 5)))))
      (= outcome (continue unit))))
  ; tighten (v-memory-safety): known-leak->0 pin hygiene — reducer-escape via imposed WIT world (result threaded/written out the envelope), post-#9310 escv class. Pin-flip only (no reclaim code). Census-validated per-chapter GREEN. (float-keyed map-result sibling HELD pending v-corpus-harness float-key determinism clearance.)
  (live-objects 0))

(case
  "a typed reducer performing a nested list<list<s64>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 24 — a NESTED list<list<s64>> host-op ARG (sink.push : (list<list<s64>>) -> unit) driven through an
           imposed WIT world. The reducer on-message performs sink.push (list (list 1 2) (list 3)) (a unit-result,
           observe-only host op) then returns a Continue step with an empty requests list. Running the guest LOADS
           the emitted component and INVOKES sink.push, exercising the RECURSIVE nested-list arg marshal (the outer
           list of (ptr,count) inner lists, each an i64-strided array) e2e - runtime byte-movement coverage a
           validate-only check cannot provide: a broken nested marshal (wrong inner stride, dropped inner list,
           or list<s64>-instead-of-list<list<s64>>) fails to instantiate or run. The observed host-call sequence
           pins that sink.push actually fired. Runtime coverage for v-rust-backend INCREMENT 1 (#3321, recursive
           nested-list host-arg lowering); complements the in-crate validate_all test which cannot check byte-movement.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param vals (list (list (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List (List Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list(#list(1 2) #list(3)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a bytes-param leaf and a spilled record result in one member run through both memory paths (via an imposed WIT world)"
  (doc
    "SHAPE 28 — BOTH memory boundaries in ONE member: a list<u8> LEAF param AND a spilled record result,
           the exact combined shape of a reducer's on-message(message) -> step. The wrapper uses both memory
           paths (bytes copy-in for the param leaf + result spill) and all four scratch locals without collision.
           f(m: record{data: Bytes}) = let k = Bytes.len(m.data) in { n: k, twice: 2*k }; f({data:[1..7]}) ==
           { n: 7, twice: 14 } proves the copied-in bytes length + the spilled two-field record result agree.
           Migrated from the in-crate wasmtime test `a_bytes_param_and_record_result_guest_compiles_and_runs`.")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= data (list (u8)))))
            (result (record (= n (s64)) (= twice (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: m (Record (: data Bytes))))
        (let ((k (Bytes.len m.data))) #record((= n k) (= twice (+ k k)))))
      (export f)))
  (call f (: #record((= data #list(1 2 3 4 5 6 7))) (Record (: data Bytes))))
  (output #record((= n 7) (= twice 14)))
  (live-objects 0))

(case
  "the identity-less reducer-echo round-trips the real message shape into a step (via an imposed WIT world)"
  (doc
    "SHAPE 29 — the flagship reducer-echo (on-message(message) -> step) round-tripping the REAL
           declaration-ordered message{contract, sender:{reducer, host}, payload, token} (a NESTED sender record
           + four list<u8> leaves) into the full step. Exercises the message param permute + the nested-record
           param read + the whole step result writer at once: f echoes contract/payload/token into one request
           with deadline-nanos None and outcome Continue. Migrated from the in-crate wasmtime test
           `the_identity_less_reducer_echo_round_trips` (the SUNSET-CORE echo relation minus identity).")
  (wit-world
    (world
      w
      (export
        iface
        (member
          on-message
          (func
            (param
              m
              (record
                (= contract (list (u8)))
                (= sender (record (= reducer (list (u8))) (= host (list (u8)))))
                (= payload (list (u8)))
                (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (def
        (onMessage
          (:
            m
            (Record
              (: contract Bytes)
              (: sender (Record (: reducer Bytes) (: host Bytes)))
              (: payload Bytes)
              (: token Bytes))))
        #record((=
            requests
            #list(#record((= contract m.contract)
                (= payload m.payload)
                (= token m.token)
                (= deadline-nanos Option.None))))
          (= outcome Outcome.Continue)))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(170 187))
        (= sender #record((= reducer #list(1)) (= host #list(2))))
        (= payload #list(3 4 5))
        (= token #list(9 9)))
      (Record
        (: contract Bytes)
        (: sender (Record (: reducer Bytes) (: host Bytes)))
        (: payload Bytes)
        (: token Bytes))))
  (output
    #record((=
        requests
        #list(#record((= contract #list(170 187))
            (= payload #list(3 4 5))
            (= token #list(9 9))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  ; tighten (v-memory-safety): known-leak->0 pin hygiene — reducer-escape via imposed WIT world (result threaded/written out the envelope), post-#9310 escv class. Pin-flip only (no reclaim code). Census-validated per-chapter GREEN. (float-keyed map-result sibling HELD pending v-corpus-harness float-key determinism clearance.)
  (live-objects 0))

(case
  "a typed reducer performing a list<record> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 30 — a list<record{contract: bytes, n: s64}> host-op ARG (sink.push): each record element written in place into the outer array at its canonical layout - the s64 field inline + the Bytes field's rope spilled after the array with its (ptr,len) inline, WIT-declaration-ordered. Exercises emit_record_to_mem e2e. Runtime coverage for v-rust-backend increment 2 (#3334, the list<record> host-arg marshal).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param items (list (record (= contract (list (u8))) (= n (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List (Record (: contract Bytes) (: n Int64))) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list(#record((= contract m.contract) (= n 5))))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-a-list-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 31 — a record host-op ARG whose field is a list<s64> (sink.push(record{ids: list<s64>, n: s64})): the record flattens to core slots, the list field marshalled into mem (backing array) + pushed as (ptr,count). Exercises emit_record_arg_marshal's list-field arm e2e. Runtime coverage for v-rust-backend increment 3 (#3338).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param r (record (= ids (list (s64))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (Record (: ids (List Int64)) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= ids #list(1 2 3)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer branching on a bool host-op result emits a request when true (via an imposed WIT world)"
  (doc
    "SHAPE 32 - a BOOL host-import RESULT (kv.delete : (Bytes) -> bool) driven through an imposed WIT world. The reducer on-message performs kv.delete(m.token) and branches on the bool: true -> one echo request, false -> no requests. Stubbing kv.delete -> true and asserting the non-empty branch fires (one request) makes the bool result lift load-bearing (a flat scalar disc read). The platform state.delete returns unit (no bool), so this bool host-result lift has no conformance home - it belongs in the typed host-result corpus. Complements the emit+load-only a_host_fused_kv_delete_bool_reducer with the RUNTIME bool-branch coverage.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import cadenza:platform/kv (member delete (func (param key (list (u8))) (result (bool)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect kv (op delete (-> Bytes Bool)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (kv)
          (if
            (kv.delete m.token)
            #record((=
                requests
                #list(#record((= contract m.contract)
                    (= payload m.payload)
                    (= token m.token)
                    (= deadline-nanos Option.None))))
              (= outcome Outcome.Continue))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond kv.delete (: true Bool)))
  (host-calls (call cadenza:platform/kv.delete))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(2))
            (= token #list(3))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<tuple> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 33 — a list<tuple<s64, bytes>> host-op ARG (sink.push): each tuple element written in place into the outer array at its canonical positional layout - the s64 element inline + the Bytes element's rope spilled after the array with (ptr,len) inline. Exercises emit_tuple_to_mem e2e. Runtime coverage for v-rust-backend increment 4 (#3343).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param items (list (tuple (s64) (list (u8))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List (Tuple Int64 Bytes)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list(#tuple(5 m.contract)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer threading a list<tuple<bytes,bytes>> host-op result branches on its length (via an imposed WIT world)"
  (doc
    "SHAPE 34 - a list<tuple<Bytes,Bytes>> host-import RESULT (kv.prefix-scan : (Bytes) -> list<tuple<Bytes,Bytes>>) driven through an imposed WIT world. The reducer on-message performs kv.prefix-scan(m.token) and branches on List.len(result) > 0: non-empty -> one echo request, empty -> no requests. Stubbing prefix-scan -> two pairs and asserting the non-empty branch fires (one request) makes the list<tuple> RESULT lift load-bearing: the retptr count read + 16-byte element stride + nested byte-list copy. A broken lift reading the retptr'd list as empty would take the empty branch. Covers the general list<tuple<Bytes,Bytes>> host-result lift v-platform-itest's state iface has no scan-returning-pairs op to exercise. Runtime coverage for the kv.prefix-scan result shape (retires the in-crate run_reducer_bytes_with_scan test).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/kv
        (member
          prefix-scan
          (func (param key (list (u8))) (result (list (tuple (list (u8)) (list (u8))))))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect kv (op prefix-scan (-> Bytes (List (Tuple Bytes Bytes)))))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (kv)
          (if
            (> (List.len (kv.prefix-scan m.token)) 0)
            #record((=
                requests
                #list(#record((= contract m.contract)
                    (= payload m.payload)
                    (= token m.token)
                    (= deadline-nanos Option.None))))
              (= outcome Outcome.Continue))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses
    (respond
      kv.prefix-scan
      (: #list((#list(107) #list(49)) (#list(107) #list(50))) (List (Tuple Bytes Bytes)))))
  (host-calls (call cadenza:platform/kv.prefix-scan))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(2))
            (= token #list(3))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-an-option-scalar-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 35 — a record host-op ARG with an option<s64> field (sink.push(record{d: option<s64>, n: s64})): the record flattens, the option field to (disc, payload) - Some(42)->(1,42). Exercises emit_record_arg_marshals option-field arm e2e. Runtime coverage for v-rust-backend increment 5 (#3349).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param r (record (= d (option (s64))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (Record (: d (Option Int64)) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= d (Option.Some 42)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-an-option-bytes-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 36 — record{d: option<bytes>, n: s64} host-op ARG; the option field flattens to (disc, ptr, len), Some copies the payload rope. Runtime coverage for v-rust-backend increment 6 (#3354).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param r (record (= d (option (list (u8)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (Record (: d (Option Bytes)) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= d (Option.Some m.contract)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

And

a

direct

record-with-bytes-field:

same

shape

but

the

sink

push

param

is

#record((= c #list((u8))) (= n (s64)))

(unquote guest)

op

(-> (Record (: c Bytes) (: n Int64)) Unit)

(unquote body)

(sink.push #record((= c m.contract) (= n 7)))

.

Ping

if

you

want

me

to

write

the

second

one

out

fully.

Both

are

new

NON-record-result

cases

(no sibling bytes param)

(case
  "a typed reducer performing a record-with-a-direct-bytes-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 37 - a record host-op ARG with a DIRECT Bytes (list<u8>) FIELD beside a scalar (sink.push(record{b: Bytes, n: s64})): the record flattens to core slots, the bytes field's rope marshalled into shared mem with its (ptr,len) inline. Exercises emit_record_arg_marshal's direct-list<u8>-field arm e2e (the sibling-list<u8>-param restriction lifted in #3354). Second half of v-rust-backend INCREMENT 6 (#3354); complements SHAPE 36's option<Bytes> field.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param r (record (= b (list (u8))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (Record (: b Bytes) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= b m.contract) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<option<s64>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 38 — a list<option<s64>> host-op ARG (sink.push): each option element written in place at its canonical layout (disc byte + payload) - Some(5)->(1,5), None->(0,0). Exercises emit_option_to_mem e2e (both arms). Runtime coverage for v-rust-backend increment 7 (#3358).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param items (list (option (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List (Option Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list((Option.Some 5) Option.None))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<record-with-option-field> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 39 — a list<record{d: option<s64>, n: s64}> host-op ARG (sink.push): each record element written in place, its option field via emit_option_to_mem - Some(5) and None across two elements. Runtime coverage for v-rust-backend increment 8 (#3360).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param items (list (record (= d (option (s64))) (= n (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (List (Record (: d (Option Int64)) (: n Int64))) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push
              #list(#record((= d (Option.Some 5)) (= n 7)) #record((= d Option.None) (= n 8))))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-a-tuple-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 40 — a record host-op ARG with a tuple<s64, bytes> field (sink.push(record{t: tuple<s64, bytes>, n: s64})): the tuple field flattens inline (s64 slot + bytes element rope-copied as (ptr,len)). Runtime coverage for v-rust-backend increment 9 (#3362).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param r (record (= t (tuple (s64) (list (u8)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect sink (op push (-> (Record (: t (Tuple Int64 Bytes)) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= t #tuple(5 m.contract)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a nested-record host arg with an option + bytes leaf emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 41 — composition: record{a: s64, sub: record{d: option<s64>, b: bytes}} host arg — the nested-record arm recurses into the option + bytes field arms. Verifies deep composition of the arg-side marshal (all constituent arms already on main: #3349 option-scalar-field, #3354 bytes/option-bytes; nested-record pre-existing). No new v-rust-backend emit; compositional coverage.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param r (record (= a (s64)) (= sub (record (= d (option (s64))) (= b (list (u8)))))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect
        sink
        (op push (-> (Record (: a Int64) (: sub (Record (: d (Option Int64)) (: b Bytes)))) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= a 9) (= sub #record((= d (Option.Some 5)) (= b m.contract)))))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-a-variant-scalar-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 42 — a record host-op ARG with a variant<scalar> field (sink.push(record{v: variant{a, b(s64), c(s64)}, n: s64})): the variant field flattens (canonical variant flatten) to (disc:i32, payload) — the guest sum-disc IS the component discriminant (decl order, like an enum); a payload case unboxes sum-payload, a nullary case emits the payload-width zero. Pushing V.b(5) -> (disc 1, payload 5). Runtime coverage for v-rust-backend increment 10 (#3368).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param r (record (= v (variant (a) (b (s64)) (c (s64)))) (= n (s64))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Int64) (C Int64))
      (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= v (V.B 5)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<variant<scalar>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 43 — a list<variant{a, b(s64), c(s64)}> host-op ARG (sink.push): each variant element written in place at its canonical variant layout (disc + uniform scalar payload) via emit_variant_to_mem; the guest sum-disc IS the component discriminant (decl order). Pushing (V.B 5), V.A, (V.C 9) exercises a payload case, a nullary case, and a second payload case. Runtime coverage for v-rust-backend increment 11 (PR #3379).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param items (list (variant (a) (b (s64)) (c (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Int64) (C Int64))
      (effect sink (op push (-> (List V) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list((V.B 5) V.A (V.C 9)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<record-with-a-variant-field> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 44 — variant<scalar> as a record field inside a list element (emit_product_to_mem variant arm). Runtime coverage for v-rust-backend increment 12 (PR #3394).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param items (list (record (= v (variant (a) (b (s64)) (c (s64)))) (= n (s64)))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Int64) (C Int64))
      (effect sink (op push (-> (List (Record (: v V) (: n Int64))) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list(#record((= v (V.B 5)) (= n 7)) #record((= v V.A) (= n 8))))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a list<tuple-with-a-variant-element> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 45 — variant<scalar> as a tuple element inside a list element (emit_product_to_mem variant arm, positional). Runtime coverage for v-rust-backend increment 12 (PR #3394).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param items (list (tuple (variant (a) (b (s64)) (c (s64))) (s64))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Int64) (C Int64))
      (effect sink (op push (-> (List (Tuple V Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list(#tuple((V.C 9) 1) #tuple(V.A 2)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a nested-record host arg with a variant<scalar> leaf emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 46 — composition: record{a: s64, sub: record{v: variant{x, y(s64), z(s64)}, n: s64}} host arg; the nested-record arm recurses into the variant field arm. No new v-rust-backend emit (variant field #3368 + nested-record pre-existing); compositional coverage.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param
              r
              (record
                (= a (s64))
                (= sub (record (= v (variant (x) (y (s64)) (z (s64)))) (= n (s64))))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (X) (Y Int64) (Z Int64))
      (effect sink (op push (-> (Record (: a Int64) (: sub (Record (: v V) (: n Int64)))) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= a 9) (= sub #record((= v (V.Y 5)) (= n 7)))))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-a-variant-u32-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 47 — variant<u32> record-field host arg: pins the i32-width payload store branch of the variant marshal (prior variant SHAPEs use s64/i64). Runtime coverage for v-rust-backend uniform-scalar variant, non-i64 width.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param r (record (= v (variant (a) (b (u32)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B UInt32))
      (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= v (V.B 4000000000)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a typed reducer performing a record-with-a-variant-f64-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 48 — variant<f64> record-field host arg: pins the f64-width payload store branch of the variant marshal.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func (param r (record (= v (variant (a) (b (f64)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Float64))
      (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= v (V.B 3.5)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "Int64.of over a u64 host-op RESULT evaluates the host call ONCE (the range-check names the operand)"
  (doc
    "SHAPE 49 - the runtime checked conversion `Int64.of` over a HOST-LIFTED u64 result. The emit composes `if operand > i64::MAX then trap else wrap(operand)`, which NAMES the operand in the compare AND the else; a host-call operand must be materialized ONCE (a self-keyed let) or its effect FIRES PER USE - breaker adv-tof-host-u64 saw an in-range 1000 spuriously TRAP because the second host invocation drained its lone queued response. The `(host-calls ...)` clause asserts EXACTLY ONE call to hosti.base, so a regression to per-reference re-invocation fails here (host-response exhaustion), not just a wrong value. Runtime coverage for the operand-materialize fix over the merged #3537 .of compose.")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op base (-> Unit UInt64)))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (Int64.of (hosti.base unit))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "Int64.checked-add over a u64 host-op RESULT evaluates the host call ONCE (the overflow formula names the operand)"
  (doc
    "SHAPE 50 - the runtime checked arithmetic `Int64.checked-add` over a HOST-LIFTED u64 result (narrowed by Int64.of). The overflow-check compose names the operand in the wrapping result AND the two's-complement formula, so a host-call operand is materialized ONCE (else the effect fires per reference). `(host-calls ...)` asserts EXACTLY ONE call to hosti.base. main = match (checked-add (Int64.of (hosti.base)) 1) Some v -> v, None -> -1; with the host stubbed 1000 the sum 1001 fits -> Some 1001. Runtime coverage for the operand-materialize fix over the merged #3569 checked-arith compose.")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op base (-> Unit UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (match (Int64.checked-add (Int64.of (hosti.base unit)) 1) ((Some v) v) ((None _) -1))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1001 Int64))
  (live-objects 0))

; -- host-u64 checked-conversion end-to-end: intact-compare control, T.of over a host response, handler x host x conversion, record-arg x value-result x conversion (breaker batch 382; the #3537->#3572 wrong-trap arc witnesses) --
(case
  "u64h1 the u64 host response compared WITHOUT T.of arrives intact"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op base (-> Unit UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (if (= (hosti.base unit) (UInt64.wrap 1000)) 7 8)))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "u64h2 T.of over the u64 host response (isolated, in-range 1000)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op base (-> Unit UInt64)))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (Int64.of (hosti.base unit))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "cr03 an export combining an IN-GUEST handler AND a host import"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect Cnt (op tick (-> Int64)))
      (effect hosti (op base (-> Unit UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (+
            (Int64.of (hosti.base unit))
            (handle Cnt m.x ((tick () s (resume (* s 10) (+ s 1)))) (+ (Cnt.tick) (Cnt.tick))))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects 0))

(case
  "cq04 host IMPORT: RECORD param + scalar result (reverse direction)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (Int64.of (hosti.put #record((= a m.x) (= b 9))))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.put (: 42 UInt64)))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a record-with-a-MIXED-WIDTH-variant-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 51 - a record host-op ARG whose variant field has MIXED-WIDTH scalar payloads (sink.push(record{v: variant{a, b(s64), c(u8)}, n: s64})). The canonical flatten JOIN of a b(s64)/c(u8) variant is the widest register type (i64) - a payload case stores its own value, narrower than the join, at the field's flattened slot; the guest sum-disc IS the component discriminant. Pushing V.b(-5000000000) drives a NEGATIVE s64 payload through the mixed join (a naive i32 join would truncate it). Runtime coverage for v-rust-backend's mixed-width variant marshal (the register-flatten face v-platform-itest's arg-probe value-gate verified byte-correct); this case pins that it EMITS + INSTANTIATES + RUNS (a wrong join type fails wasm-tools validate / instantiation).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member
          push
          (func
            (param r (record (= v (variant (a) (b (s64)) (c (u8)))) (= n (s64))))
            (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A) (B Int64) (C UInt8))
      (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #record((= v (V.B -5000000000)) (= n 7)))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a list<MIXED-WIDTH-variant<scalar>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 52 - a list<variant{a(u8), b(u16), c}> host-op ARG (sink.push): each element written in place at the canonical variant layout, whose payload area is the MAX-NATURAL width of the mixed u8/u16 cases (the memory face of the flatten join, distinct from SHAPE 51's register face). Pushing (V.A 200), (V.B 60000), V.C exercises a u8 payload, a u16 payload, and a nullary case - the u8 reads from the right bits and the u16 stays intact per element. Runtime coverage for the mixed-width variant marshal's memory path (v-platform-itest's arg-probe value-gate verified the list items byte-correct); pins EMIT + INSTANTIATE + RUN.")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import
        cadenza:platform/sink
        (member push (func (param items (list (variant (a (u8)) (b (u16)) (c)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (type V (A UInt8) (B UInt16) (C))
      (effect sink (op push (-> (List V) Unit)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (sink)
          (do
            (sink.push #list((V.A 200) (V.B 60000) V.C))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects 0))

(case
  "a BARE mixed-width variant as the direct host-op arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 53 - a scalar-payload variant{tiny(u8), big(s64), mark} passed BARE as the TOP-LEVEL host-op param (hosti.put(v: variant), NOT nested in a record/list). The param crosses as a component `variant` DEFINED type; the guest decomposes the value-heap variant handle into the canonical `(disc, payload)` register-flatten (join = i64 for the mixed u8/s64 cases) via emit_variant_reg_flatten - the SAME helper a record-field/list-element variant uses, now at the param position (HostParam::Variant). Three calls exercise a u8 payload case, an s64 payload case, and the nullary `mark` (payload-width zero). Runtime coverage for v-rust-backend's bare-variant host-arg param (breaker mwv1); a wrong join/flatten fails wasm-tools validate / instantiation.")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member put (func (param v (variant (tiny (u8)) (big (s64)) (mark))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Tiny UInt8) (Big Int64) (Mark))
      (effect hosti (op put (-> V Unit)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (do
            (hosti.put (V.Tiny (UInt8.wrap 7)))
            (hosti.put (V.Big 900000000000))
            (hosti.put V.Mark)
            m.x)))
      (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls
    (call cadenza:demo/hosti.put)
    (call cadenza:demo/hosti.put)
    (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects 0))

; -- bare + record-wrapped MIXED-WIDTH variant host args: u8/s64/nullary arms each dispatched (breaker batch 385; the #3579->#3588 HostParam::Variant arc) --
(case
  "mwv1 a MIXED-WIDTH scalar-payload variant host ARG delivers each arm's payload"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member put (func (param v (variant (tiny (u8)) (big (s64)) (mark))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Tiny UInt8) (Big Int64) (Mark))
      (effect hosti (op put (-> V Unit)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (do
            (hosti.put (V.Tiny (UInt8.wrap 7)))
            (hosti.put (V.Big 900000000000))
            (hosti.put V.Mark)
            m.x)))
      (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls
    (call cadenza:demo/hosti.put)
    (call cadenza:demo/hosti.put)
    (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "mwv2 the SAME mixed-width variant wrapped in a RECORD host arg"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member
          put
          (func
            (param r (record (= v (variant (tiny (u8)) (big (s64)) (mark))) (= n (s64))))
            (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Tiny UInt8) (Big Int64) (Mark))
      (effect hosti (op put (-> (Record (: v V) (: n Int64)) Unit)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (do (hosti.put #record((= v (V.Big 900000000000)) (= n 1))) m.x)))
      (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a variant-WITH-PAYLOAD host RESULT is lifted into a guest Sum and matched (via an imposed WIT world)"
  (doc
    "SHAPE 54 - a host op returning a scalar-payload variant{a(u8), b(s64), mark} (hosti.get). The result is SPILLED (flattens to disc+payload > 1 core value → retptr); the guest LIFTS the (disc, payload) from the retptr'd region into a value-heap Sum via emit_variant_sum_lift - the N-case generalization of the option-result lift, the RESULT-side twin of the bare-variant ARG marshal. Stubbing get -> (b 900000000000) and matching selects the B arm (k). Runtime coverage for v-rust-backend's variant-payload host-result lift (breaker w10c); a wrong disc/payload-offset read would mis-select the arm. (The cdz-run driver's coerce_one gained a Type::Variant arm to encode the variant response.)")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member get (func (result (variant (a (u8)) (b (s64)) (mark))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (A UInt8) (B Int64) (Mark))
      (effect hosti (op get (-> Unit V)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.get unit) ((A n) (Int64.of n)) ((B k) k) ((Mark) -1))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (b 900000000000) V)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 900000000000 Int64))
  (live-objects 0))

; -- variant-with-payload host RESULTS: payload arm, mixed-width per-arm across three dispatches, negative-s64 join (breaker batch 387; the pre-delivered #3592 acceptance ladder) --
(case
  "vres1 a variant-with-payload host RESULT delivers the payload arm (w10c shape)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member pick (func (result (variant (small (s64)) (big))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Pick (Small Int64) (Big))
      (effect hosti (op pick (-> Unit Pick)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.pick unit) ((Pick.Small k) k) ((Pick.Big) 999))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.pick (: (small 5) pick)))
  (host-calls (call cadenza:demo/hosti.pick))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "vres2 a MIXED-WIDTH variant host RESULT delivers each arm across three dispatches"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member next (func (result (variant (tiny (u8)) (big (s64)) (mark))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Tiny UInt8) (Big Int64) (Mark))
      (effect hosti (op next (-> Unit V)))
      (def (rd (: v V)) (match v ((V.Tiny t) (Int64.of t)) ((V.Big b) b) ((V.Mark) -1)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (+ (rd (hosti.next unit)) (+ (rd (hosti.next unit)) (rd (hosti.next unit))))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses
    (respond hosti.next (: (tiny 7) v))
    (respond hosti.next (: (big 900000000000) v))
    (respond hosti.next (: (mark unit) v)))
  (host-calls
    (call cadenza:demo/hosti.next)
    (call cadenza:demo/hosti.next)
    (call cadenza:demo/hosti.next))
  (output (: 900000000006 Int64))
  (live-objects 0))

(case
  "vres3 a NEGATIVE s64 payload through the variant host-result join"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member get (func (result (variant (val (s64)) (none))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type R (Val Int64) (NoneArm))
      (effect hosti (op get (-> Unit R)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.get unit) ((R.Val v) v) ((R.NoneArm) 0))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (val -5000000000) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: -5000000000 Int64))
  (live-objects 0))

(case
  "a variant-with-a-COMPOUND-PAYLOAD host RESULT is lifted (bytes payload case) (via an imposed WIT world)"
  (doc
    "SHAPE 55 - a host op returning a variant one of whose cases carries a NON-scalar (compound) payload: variant{raw(list<u8>), empty}. The result spills; the guest lifts the disc + the selected case's payload from the retptr'd region via emit_variant_sum_lift, which RECURSES emit_result_lift for a compound payload (a list<u8> = copy-out of the bytes) rather than the scalar leaf-box. Stubbing get -> (raw (list 1 2 3 4 5)) selects the Raw arm and reads Bytes.len = 5; the (empty) arm returns -1. Generalizes the scalar variant-result lift (SHAPE 54) to a liftable-compound payload (list/bytes/tuple/record via the shared recursion) - the variant_liftable_payload_cases admission, RESULT-side only (the ARG marshal stays scalar-only). Consumer-relevant: the deliver-response dispatch returns variant-payload results.")
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member get (func (result (variant (raw (list (u8))) (empty))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Raw Bytes) (VEmpty))
      (effect hosti (op get (-> Unit V)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.get unit) ((Raw b) (Int64.of (Bytes.len b))) ((VEmpty) -1))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (raw #list(1 2 3 4 5)) V)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 5 Int64))
  (live-objects 0))

; -- variant host RESULTS with COMPOUND payloads: list payload measured, record payload projected (breaker batch 397a; the #3655 flip) --
(case
  "cvp1 a variant host RESULT with a LIST payload lifts and is measured"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member get (func (result (variant (items (list (s64))) (none))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type R (Items (List Int64)) (NoneArm))
      (effect hosti (op get (-> Unit R)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.get unit) ((R.Items xs) (List.len xs)) ((R.NoneArm) -1))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (items #list(5 6 7)) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "cvp2 a variant host RESULT with a RECORD payload lifts and projects"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member get (func (result (variant (tag (record (= a (s64)) (= b (s64)))) (none))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type R (Tag (Record (: a Int64) (: b Int64))) (NoneArm))
      (effect hosti (op get (-> Unit R)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.get unit) ((R.Tag t) (+ t.a t.b)) ((R.NoneArm) -1))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (tag #record((= a 40) (= b 2))) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 42 Int64))
  (live-objects 0))

; -- breaker batch 404 (2026-08-26): in-guest-handler x host-import COMBINATION faces (cr01-cr03d:
; two exports in one interface, exported body running a handled effect, handler+host-call in
; let-sequenced / nested / body-inside shapes) and the nullary-import + record-host-arg s64-result
; faces (cq04c). Imposed-world: wasm pass, rust todo (import-side emit pending).
(case
  "cr01 TWO exported members in one interface — both callable"
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param m (record (= x (s64)))) (result (s64))))
        (member g (func (param m (record (= x (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: m (Record (: x Int64)))) (* m.x 2))
      (def (g (: m (Record (: x Int64)))) (+ m.x 100))
      (export f)
      (export g)))
  (call g (: #record((= x 5)) (Record (: x Int64))))
  (output (: 105 Int64))
  (live-objects 0))

(case
  "cr02 an export whose body runs an IN-GUEST handled effect"
  (wit-world
    (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect Cnt (op tick (-> Int64)))
      (def
        (f (: m (Record (: x Int64))))
        (handle Cnt m.x ((tick () s (resume (* s 10) (+ s 1)))) (+ (Cnt.tick) (Cnt.tick))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (output (: 70 Int64))
  (live-objects 0))

(case
  "cr03b let-sequenced: handle FIRST, then host call (same combination, flat nesting)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect Cnt (op tick (-> Int64)))
      (effect hosti (op base (-> Unit UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (let
            ((k (handle Cnt m.x ((tick () s (resume (* s 10) (+ s 1)))) (+ (Cnt.tick) (Cnt.tick)))))
            (+ (Int64.of (hosti.base unit)) k))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects 0))

(case
  "cr03c host call INSIDE the handled body"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect Cnt (op tick (-> Int64)))
      (effect hosti (op base (-> Unit UInt64)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (handle
            Cnt
            m.x
            ((tick () s (resume (* s 10) (+ s 1))))
            (+ (Cnt.tick) (Int64.of (hosti.base unit))))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1030 Int64))
  (live-objects 0))

(case
  "cr03d combination with s64 host result (no Int64.of) — nested"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member base (func (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect Cnt (op tick (-> Int64)))
      (effect hosti (op base (-> Unit Int64)))
      (def
        (f (: m (Record (: x Int64))))
        (host
          (hosti)
          (+
            (hosti.base unit)
            (handle Cnt m.x ((tick () s (resume (* s 10) (+ s 1)))) (+ (Cnt.tick) (Cnt.tick))))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 Int64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects 0))

(case
  "cq04c record host-ARG with s64 result (no Int64.of)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) Int64)))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (hosti.put #record((= a m.x) (= b 9)))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.put (: 42 Int64)))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects 0))

; -- breaker batch 405 (2026-08-26): host-IMPORT result-shape coverage + record-param export
; controls. cq01/cq03 scalar-param imports with record/list results; cq02b/c/d NULLARY imports with
; record/list results (once, once-list, twice-with-two-responds); cq04b record arg + unit result;
; wen1 the FIRST enum host-import RESULT pin. cord4/cord5 pin the RECORD-param export twins (2- and
; 20-field record results) that pass at every size — the isolating controls for the one remaining
; export gap: a bare SCALAR-param export with a compound result renders a raw pointer (cor02..co02,
; cord2/cord3 all fail identically at 2..20 fields; routed to v-rust-backend with this ladder).
(case
  "cq01 host IMPORT: scalar param + RECORD result — guest reads a field"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member info (func (param k (s64)) (result (record (= alpha (s64)) (= beta (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op info (-> Int64 (Record (: alpha Int64) (: beta Int64)))))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (. (hosti.info m.x) beta)))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses
    (respond
      hosti.info
      (: #record((= alpha 7) (= beta 42)) (Record (: alpha Int64) (: beta Int64)))))
  (host-calls (call cadenza:demo/hosti.info))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "cq03 host IMPORT: scalar param + LIST result — guest measures it"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member fetch (func (param k (s64)) (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op fetch (-> Int64 (List Int64))))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (List.len (hosti.fetch m.x))))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.fetch (: #list(5 6 7) (List Int64))))
  (host-calls (call cadenza:demo/hosti.fetch))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "cq02b host IMPORT: NULLARY + RECORD result, called ONCE"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member peek (func (result (record (= a (s64)) (= b (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op peek (-> Unit (Record (: a Int64) (: b Int64)))))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (. (hosti.peek unit) b)))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses
    (respond hosti.peek (: #record((= a 10) (= b 32)) (Record (: a Int64) (: b Int64)))))
  (host-calls (call cadenza:demo/hosti.peek))
  (output (: 32 Int64))
  (live-objects 0))

(case
  "cq02c host IMPORT: NULLARY + LIST result, called once"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member all (func (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op all (-> Unit (List Int64))))
      (def (f (: m (Record (: x Int64)))) (host (hosti) (List.len (hosti.all unit))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.all (: #list(4 5) (List Int64))))
  (host-calls (call cadenza:demo/hosti.all))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "cq04b host IMPORT: RECORD arg + UNIT result (SHAPE-13 control in my namespace)"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import
        cadenza:demo/hosti
        (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) Unit)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (do (hosti.put #record((= a m.x) (= b 9))) 7)))
      (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "cq02d NULLARY + RECORD result called TWICE with TWO respond clauses"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member peek (func (result (record (= a (s64)) (= b (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (effect hosti (op peek (-> Unit (Record (: a Int64) (: b Int64)))))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (+ (. (hosti.peek unit) a) (. (hosti.peek unit) b))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses
    (respond hosti.peek (: #record((= a 10) (= b 0)) (Record (: a Int64) (: b Int64))))
    (respond hosti.peek (: #record((= a 0) (= b 32)) (Record (: a Int64) (: b Int64)))))
  (host-calls (call cadenza:demo/hosti.peek) (call cadenza:demo/hosti.peek))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "cord4 IMPOSED world: RECORD param + 2-field scalar record result"
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param m (record (= x (s64)))) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) #record((= b1 m.x) (= b2 2))) (export f)))
  (call f (: #record((= x 1)) (Record (: x Int64))))
  (output #record((= b1 1) (= b2 2)))
  (live-objects 0))

(case
  "cord5 IMPOSED world: RECORD param + 20-field record result (the co02 shape, record param)"
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= x (s64))))
            (result
              (record
                (= b1 (s64))
                (= b2 (s64))
                (= b3 (s64))
                (= b4 (s64))
                (= b5 (s64))
                (= b6 (s64))
                (= b7 (s64))
                (= b8 (s64))
                (= b9 (s64))
                (= b10 (s64))
                (= b11 (s64))
                (= b12 (s64))
                (= b13 (s64))
                (= b14 (s64))
                (= b15 (s64))
                (= b16 (s64))
                (= b17 (s64))
                (= b18 (s64))
                (= b19 (s64))
                (= b20 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: m (Record (: x Int64))))
        #record((= b1 m.x)
          (= b2 2)
          (= b3 3)
          (= b4 4)
          (= b5 5)
          (= b6 6)
          (= b7 7)
          (= b8 8)
          (= b9 9)
          (= b10 10)
          (= b11 11)
          (= b12 12)
          (= b13 13)
          (= b14 14)
          (= b15 15)
          (= b16 16)
          (= b17 17)
          (= b18 18)
          (= b19 19)
          (= b20 20)))
      (export f)))
  (call f (: #record((= x 9)) (Record (: x Int64))))
  (output
    #record((= b1 9)
      (= b2 2)
      (= b3 3)
      (= b4 4)
      (= b5 5)
      (= b6 6)
      (= b7 7)
      (= b8 8)
      (= b9 9)
      (= b10 10)
      (= b11 11)
      (= b12 12)
      (= b13 13)
      (= b14 14)
      (= b15 15)
      (= b16 16)
      (= b17 17)
      (= b18 18)
      (= b19 19)
      (= b20 20)))
  (live-objects 0))

(case
  "wen1 an enum host-import RESULT lifts and selects the guest arm"
  (wit-world
    (world
      w
      (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
      (import cadenza:demo/hosti (member mode (func (result (enum fast slow)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Mode (Fast) (Slow))
      (effect hosti (op mode (-> Unit Mode)))
      (def
        (f (: m (Record (: x Int64))))
        (host (hosti) (match (hosti.mode unit) ((Mode.Fast) 1) ((Mode.Slow) 2))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.mode (: (fast unit) mode)))
  (host-calls (call cadenza:demo/hosti.mode))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a SCALAR-param export returning a RECORD lifts the result (not a raw handle) (via an imposed WIT world)"
  (doc
    "SHAPE 56 - a scalar-param export with a COMPOUND (record) RESULT: f(x: s64) -> record{b1,b2,b3: s64}. The result SPILLS (3 flat > the 1-result cap) → the canonical ABI returns it via a caller-provided retptr, which the guest must WRITE. The record-PARAM route already did this (record_interface_export's SpillRecord result-lower); the SCALAR-param route was gated out by `any_record` and fell through to the provider path, which handed back the value-heap u32 HANDLE (a leaked pointer, not the value — breaker's cor02/co02 rendered ~1114400). Fix: admit the typed-interface wrapper when a member has a spilled compound result too (needs_result_wrapper), not only a record param. Pins that a scalar-param compound result LIFTS to the record value on all backends' wasm path.")
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64)) (= b3 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #record((= b1 x) (= b2 (* x 2)) (= b3 (+ x 100)))) (export f)))
  (call f (: 7 Int64))
  (output #record((= b1 7) (= b2 14) (= b3 107)))
  (live-objects 0))

(case
  "a typed reducer threading a string host-op result branches on its byte-len (via an imposed WIT world)"
  (doc
    "SHAPE 57 - a STRING host-import RESULT (kv.lookup : (Bytes) -> string) driven through an imposed WIT world - the result-side twin of the string ARG (which already crosses on every path). A `string` result crosses on the WORLD-DRIVEN boundary as the SAME (ptr,len) spill a `list<u8>` (Bytes) result rides: the guest lift (emit_result_lift's `Ty::Bytes | Ty::String` arm) copies the host's bytes into a value-heap byte-rope handle, and the WIT type is `string` (ty_natural_wit). Before #(this) the result gate (result_is_liftable) admitted `list<u8>` but NOT `string`, so a bare string host-result DECLINED at compile despite the lift + WIT machinery being shape-identical - a one-shape hole in the otherwise-general world-import result surface. The reducer on-message performs kv.lookup(m.token) and branches on String.byte-len(result) > 0: non-empty -> one echo request, empty -> no requests. Stubbing lookup -> \"hi\" (byte-len 2) and asserting the non-empty branch fires makes the string result lift load-bearing (a broken lift reading len 0 would take the empty branch). Closes the host-string-RESULT wasm-emit gap (operator-blocking for run_agent + io.fetch).")
  (wit-world
    (world
      w
      (export
        guest
        (member
          on-message
          (func
            (param
              m
              (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8)))))
            (result
              (record
                (=
                  requests
                  (list
                    (record
                      (= contract (list (u8)))
                      (= payload (list (u8)))
                      (= token (list (u8)))
                      (= deadline-nanos (option (u64))))))
                (=
                  outcome
                  (variant
                    (continue)
                    (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))
      (import cadenza:platform/kv (member lookup (func (param key (list (u8))) (result (string)))))))
  (component-name "cadenza:platform/guest")
  (input
    (do
      (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes))))
      (effect kv (op lookup (-> Bytes String)))
      (def
        (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
        (host
          (kv)
          (if
            (> (String.byte-len (kv.lookup m.token)) 0)
            #record((=
                requests
                #list(#record((= contract m.contract)
                    (= payload m.payload)
                    (= token m.token)
                    (= deadline-nanos Option.None))))
              (= outcome Outcome.Continue))
            #record((= requests #list()) (= outcome Outcome.Continue)))))
      (export onMessage)))
  (call
    on-message
    (:
      #record((= contract #list(1)) (= payload #list(2)) (= token #list(3)))
      (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond kv.lookup (: "hi" String)))
  (host-calls (call cadenza:platform/kv.lookup))
  (output
    #record((=
        requests
        #list(#record((= contract #list(1))
            (= payload #list(2))
            (= token #list(3))
            (= deadline-nanos (None unit)))))
      (= outcome (continue unit))))
  (live-objects 0))

(case
  "a payloadless enum result VALUE round-trips via the run/encode envelope (no wit-world clause; typed enum export is a separate gap)"
  (doc
    "SHAPE 58 - a payloadless enum (Color = Red|Green|Blue) returned from a scalar-param export, no wit-world clause. CORRECTION (verified by WIT-dump, not just gate PASS): this does NOT emit a typed WIT `enum` export - the compiler CANNOT emit a typed enum export today (ty_natural_wit(Ty::Sum)->None in the export lift), so it FALLS BACK to the generic cadenza:run/run resource envelope (make/run/encode) and the enum value crosses as ENCODED BYTES. So this SHAPE pins only the VALUE ROUND-TRIP of an enum through the guest + encode envelope - a broken enum lower/encode renders a wrong case. It does NOT verify typed enum self-declaration; that is a DECLINED emit gap (a typed enum EXPORT), tracked in WIT-BOUNDARY-SHAPE-COVERAGE.md. Promoted from a v-rust-backend probe (kept as an honest round-trip pin).")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green))
      (export f)))
  (call f (: 0 Int64))
  (output (: (Red unit) Color)))

(case
  "a payloadless enum in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; typed export is a separate gap)"
  (doc
    "SHAPE 59 - the record-wrapped twin of SHAPE 58: a payloadless enum as a record-result FIELD (record{c: Color}), no wit-world clause. Like SHAPE 58 this does NOT emit a typed WIT record/enum export - it falls back to the generic run/encode envelope (verified by WIT-dump), so it pins the enum-in-record VALUE ROUND-TRIP, not typed self-declaration. Complements SHAPE 2 (variant-WITH-payload) with the NULLARY-enum face. The typed enum EXPORT (and typed record-with-enum-field export) is a DECLINED emit gap tracked in WIT-BOUNDARY-SHAPE-COVERAGE.md. Promoted from a v-rust-backend probe (honest round-trip pin).")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: m (Record (: x Int64)))) #record((= c (if (= m.x 0) Color.Red Color.Green))))
      (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= c (Red unit))) (record (c Color))))
  (live-objects 0))

(case
  "a payloadless enum EXPORT result crosses as a TYPED WIT enum (imposed world) — Direction A"
  (doc
    "SHAPE 60 - a payloadless enum (Color = Red|Green|Blue) as a typed EXPORT result under an imposed world declaring `(result (\"enum\" red green blue))`. Before this, emit crossed the enum as a bare `u32` handle via the provider path (the declared WitType::Enum bypassed) - verified by WIT-dump. Fix (record_result_lower payloadless-enum arm → Passthrough i32 + needs_result_wrapper): the def already returns the raw i32 disc (= flatten(Enum)), so it passes straight through as the declared enum; the enum DEFINED type is emitted + re-exported by the typed-interface `note` pass. WIT-dump now shows `enum t0 { red, green, blue }` + `f: func(x: s64) -> t0` (NOT u32). Guard: guest decl-order case names must equal the WIT case order (else a runtime disc remap - declines). This closes the typed enum EXPORT (Direction A) in-algebra gap per the operator full-WIT-algebra ruling.")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (enum red green blue)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green))
      (export f)))
  (call f (: 0 Int64))
  (output (: (red unit) Color))
  (call f (: 5 Int64))
  (output (: (green unit) Color)))

(case
  "a variant-with-payload EXPORT result crosses as a typed WIT variant (declared world)"
  (doc
    "SHAPE 61 - the payloaded-VARIANT twin of SHAPE 60: a bare `variant { continue, close(s64) }` EXPORT result under a declared world. Already WIRED (no emit change) via record_result_lower's SpillRecord path + canon_write_of's variant arm - this SHAPE VERIFIES the previously-untested cell (WIT-dump confirms `variant t0 { continue, close(s64) }` + `f: func(x: s64) -> t0`, NOT a bare u32/run-encode). Both arms exercised: x=0 -> Continue (nullary, disc 0), x!=0 -> Close(x) (s64 payload, disc 1). A broken variant lower (wrong disc, missing payload) renders a different arm. Complements SHAPE 2 (variant in a RECORD result) with the BARE (top-level) variant result.")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (variant (continue) (close (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Outcome (Continue) (Close Int64))
      (def (f (: x Int64)) (if (= x 0) Outcome.Continue (Outcome.Close x)))
      (export f)))
  ; PAYLOAD arm FIRST: the harness balance-checks the FIRST call only, and the payload (Close) arm is the
  ; one that leaks the SpillRecord result cell (known-leak 1, same class as SHAPE 60/62/63); ordering it
  ; first makes that leak the CHECKED one (a nullary-first order hid it — breaker WIT-dump audit).
  (call f (: 7 Int64))
  (output (: (close 7) Outcome))
  (call f (: 0 Int64))
  (output (: (continue unit) Outcome))
  (live-objects 0))

(case
  "a bare TUPLE export result crosses as a typed WIT tuple (declared world)"
  (doc
    "SHAPE 62 - a bare `tuple<s64, s64>` EXPORT result under a declared world. Before this, canon_write_of had NO Ty::Tuple arm, so a tuple result declined the typed path and degraded to a bare u32 via the provider path (verified by WIT-dump). Fix: canon_write_of gained a Ty::Tuple arm (the POSITIONAL twin of the Record arm - element i at cell slot i, written at the WIT tuple's canonical offset; reuses CanonWrite::Record, no new writer). WIT-dump now shows `f: func(x: s64) -> tuple<s64, s64>`. Element writes recurse, so a nested tuple/record/bytes element composes.")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (tuple (s64) (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #tuple(x (* x 2))) (export f)))
  (call f (: 5 Int64))
  (output #tuple(5 10))
  (live-objects 0))

(case
  "a variant with a TUPLE payload crosses as a typed WIT variant (declared world)"
  (doc
    "SHAPE 63 - a variant whose payloaded case carries a TUPLE (`two(tuple<s64,s64>)`), under a declared world. Exercises canon_write_of's variant arm recursing into the new Ty::Tuple arm for the payload. Before the Tuple arm this degraded to a bare u32. WIT-dump now shows `variant t0 { one(s64), two(tuple<s64, s64>) }` + `f: func(x: s64) -> t0`. Both arms: x=0 -> One(x) (scalar payload), x!=0 -> Two(tuple x x) (tuple payload). The compound-payload twin of SHAPE 61 (scalar payload).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (variant (one (s64)) (two (tuple (s64) (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Pair (One Int64) (Two (Tuple Int64 Int64)))
      (def (f (: x Int64)) (if (= x 0) (Pair.One x) (Pair.Two #tuple(x x))))
      (export f)))
  (call f (: 0 Int64))
  (output (: (one 0) Pair))
  (call f (: 4 Int64))
  (output (two #tuple(4 4)))
  (live-objects 0))

(case
  "a declared-world enum EXPORT whose guest case order MISMATCHES the WIT remaps by case NAME to the declared order"
  (doc
    "SHAPE 64 - breaker FINDING 1 regression pin. A payloadless enum EXPORT RESULT whose GUEST case-declaration order MISMATCHES the imposed world's declared order. Guest `(type Color (Red)(Green)(Blue))` under a world declaring `(result (\"enum\" green red blue))` [red/green REVERSED]. The value crosses BY CASE NAME: `record_result_lower`'s enum arm builds a guest-disc->WIT-disc permutation (by kebab-normalized name) and, for a genuine reorder, lowers via `ResultLower::EnumRemap` which remaps the disc in the typed-interface wrapper (`emit_enum_disc_remap`) — so guest Red (guest disc 0) crosses as WIT `red` (WIT disc 1) and guest Green (guest disc 1) as WIT `green` (WIT disc 0), IDENTICAL semantics to the order-MATCHING sibling SHAPE 60. This mirrors the record boundary that places fields BY NAME not guest slot order (SHAPE 20), an enum being the degenerate variant (v-rust-backend WIT-semantics ruling). f(0)->Red->`red`, f(5)->Green->`green`. The PARAM twin is SHAPE 68 (`inv_perm` remap). (History: this originally DECLINED — an order-match guard returned None; earlier still it silently fell through to the PROVIDER path exporting `f: func(s64) -> u32`, a wrong type the round-trip masked, hence the loud imposed-world contract guard. The disc-remap increment closed the reorder, so it now WORKS by name.) A component-name-ONLY peer provider (no imposed wit_world) is unaffected — it still crosses compounds as handles (29-* peer cases).")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (enum green red blue)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green))
      (export f)))
  (call f (: 0 Int64))
  (output (: (red unit) Color))
  (call f (: 5 Int64))
  (output (: (green unit) Color)))

(case
  "a typed RECORD result with a VARIANT field crosses the export boundary (declared world)"
  (doc
    "SHAPE 65 - a typed export result `record { o: variant{continue, close(s64)}, n: s64 }` under a declared world. Verifies canon_write_of's Record arm recursing into its Variant arm for a compound field (the record + variant defined types both emitted + re-exported). WIT-dump: `variant t0 {continue, close(s64)}` + `record t1 {o: t0, n: s64}` + `f: func(x: s64) -> t1`. Both variant arms x=0->Continue / x!=0->Close(x). Previously WIRED-but-untested (the doc's record-result-with-variant-field cell); now pinned.")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          f
          (func
            (param x (s64))
            (result (record (= o (variant (continue) (close (s64)))) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Outcome (Continue) (Close Int64))
      (def (f (: x Int64)) #record((= o (if (= x 0) Outcome.Continue (Outcome.Close x))) (= n x)))
      (export f)))
  (call f (: 0 Int64))
  (output #record((= o (continue unit)) (= n 0)))
  (call f (: 7 Int64))
  (output #record((= o (close 7)) (= n 7)))
  (live-objects 0))

(case
  "a typed record result with an option<COMPOUND> field crosses the export boundary (declared world)"
  (doc
    "SHAPE 66 - a typed export result `record { d: option<record{a}>, n: s64 }` under a declared world - the option<COMPOUND> RESULT face (the doc's untested option<compound-leaf> result cell; only option<scalar>/option<bytes> had SHAPEs). canon_write_of's option arm recurses its payload into the Record arm; both defined types emit + re-export. WIT-dump: `record t0 {a}` + `record t1 {d: option<t0>, n}` + `f: func(s64)->t1`. Both arms x=0->None / x!=0->Some(record{a=x}). NOTE: this is the RESULT side; an option<compound> host-op ARG field / list element is a separate marshal-side gap.")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          f
          (func (param x (s64)) (result (record (= d (option (record (= a (s64))))) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: x Int64))
        #record((= d (if (= x 0) Option.None (Option.Some #record((= a x))))) (= n x)))
      (export f)))
  (call f (: 0 Int64))
  (output #record((= d (None unit)) (= n 0)))
  (call f (: 5 Int64))
  (output #record((= d (Some #record((= a 5)))) (= n 5)))
  (live-objects 0))

; -- breaker batch 408 (2026-08-26): the scalar-param + compound-result acceptance ladder, promoted
; on the #3721 fix (gate admission: a scalar-param member with a SpillRecord compound result now takes
; the typed-interface wrapper instead of leaking the raw handle). All 8 faces flipped on the fix:
; record 2-field (minimal, no spill) / 20-field (spill-sized), option-in-record Some+None, bare
; option, list, TWO scalar params, variant-with-payload. cord1 pins the SYNTHESIZED-world (no
; wit-world clause) 2-field record result twin, which passes BOTH targets.
(case
  "sp1 SCALAR param + 2-field record result lifts (minimal face, no spill)"
  (wit-world
    (world
      w
      (export iface (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #record((= b1 x) (= b2 2))) (export f)))
  (call f (: 1 Int64))
  (output #record((= b1 1) (= b2 2)))
  (live-objects 0))

(case
  "sp2 SCALAR param + 20-field record result lifts (spill-sized, same fix)"
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param x (s64))
            (result
              (record
                (= b1 (s64))
                (= b2 (s64))
                (= b3 (s64))
                (= b4 (s64))
                (= b5 (s64))
                (= b6 (s64))
                (= b7 (s64))
                (= b8 (s64))
                (= b9 (s64))
                (= b10 (s64))
                (= b11 (s64))
                (= b12 (s64))
                (= b13 (s64))
                (= b14 (s64))
                (= b15 (s64))
                (= b16 (s64))
                (= b17 (s64))
                (= b18 (s64))
                (= b19 (s64))
                (= b20 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: x Int64))
        #record((= b1 x)
          (= b2 2)
          (= b3 3)
          (= b4 4)
          (= b5 5)
          (= b6 6)
          (= b7 7)
          (= b8 8)
          (= b9 9)
          (= b10 10)
          (= b11 11)
          (= b12 12)
          (= b13 13)
          (= b14 14)
          (= b15 15)
          (= b16 16)
          (= b17 17)
          (= b18 18)
          (= b19 19)
          (= b20 (* x 2))))
      (export f)))
  (call f (: 9 Int64))
  (output
    #record((= b1 9)
      (= b2 2)
      (= b3 3)
      (= b4 4)
      (= b5 5)
      (= b6 6)
      (= b7 7)
      (= b8 8)
      (= b9 9)
      (= b10 10)
      (= b11 11)
      (= b12 12)
      (= b13 13)
      (= b14 14)
      (= b15 15)
      (= b16 16)
      (= b17 17)
      (= b18 18)
      (= b19 19)
      (= b20 18)))
  (live-objects 0))

(case
  "sp3 SCALAR param + record result with an Option field (Some side)"
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #record((= a 9) (= d (Option.Some x)))) (export f)))
  (call f (: 5 Int64))
  (output #record((= a 9) (= d (Some 5))))
  (live-objects 0))

(case
  "sp3n SCALAR param + record result with an Option field (None side, branch-selected)"
  (wit-world
    (world
      w
      (export
        iface
        (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) #record((= a x) (= d (if (> x 0) (Option.Some x) Option.None))))
      (export f)))
  (call f (: 0 Int64))
  (output #record((= a 0) (= d (None unit))))
  (live-objects 0))

(case
  "sp4 SCALAR param + bare option result"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (option (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) (Option.Some (* x 3))) (export f)))
  (call f (: 4 Int64))
  (output (: (Some 12) (Option Int64)))
  (live-objects 0))

(case
  "sp5 SCALAR param + list result"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #list(x (* x 2) (* x 3))) (export f)))
  (call f (: 2 Int64))
  (output #list(2 4 6))
  (live-objects 0))

(case
  "sp6 TWO scalar params + 2-field record result (multi-scalar face)"
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func (param x (s64)) (param y (s64)) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64) (: y Int64)) #record((= b1 (+ x y)) (= b2 (* x y)))) (export f)))
  (call f (: 3 Int64) (: 4 Int64))
  (output #record((= b1 7) (= b2 12)))
  (live-objects 0))

(case
  "sp7 SCALAR param + variant-with-payload result (sum face of the same gate)"
  (wit-world
    (world
      w
      (export iface (member f (func (param x (s64)) (result (variant (small (s64)) (big))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Pick (Small Int64) (Big))
      (def (f (: x Int64)) (if (< x 100) (Pick.Small x) Pick.Big))
      (export f)))
  (call f (: 5 Int64))
  (output (: (small 5) pick))
  (live-objects 0))

(case
  "cord1 SYNTHESIZED world: 2-field s64 record result (no wit-world clause) — the fully-constant record now hoists build-once (WIT static encoding), so it is a census-excluded immortal, NOT a per-call mortal leak"
  (input (do (def (f (: x Int64)) #record((= b1 1) (= b2 2))) (export f)))
  (call f (: 1 Int64))
  (output (: (record (= b1 1) (= b2 2)) (Record (: b1 Int64) (: b2 Int64))))
  (live-objects 0))

; ── Single-variant newtype escape (adv-63b/adv-64, migrated from rcdzc): a scalar-erased newtype returned
; from a PARAM'D export must emit a VALID module (not the recursive-sum-resource path) and render its NOMINAL
; name; a compound-inner newtype stays on the heap escape; matched/wrapped-then-matched controls round-trip.
(case
  "a scalar-erased single-variant newtype escaping a param'd export emits a valid module and renders the nominal name"
  (doc
    "adv-63b/adv-64. A single-variant newtype over a SCALAR, `(type W (Mk Int64))`, returned from a
           PARAMETERIZED export erases to a bare core int; the boundary escape router must NOT take the
           nominal recursive-sum-resource path (which expects a heap handle) for a raw i64 — that emitted an
           INVALID module ('expected i32, found i64'). The scalar-erased newtype falls through to the scalar
           value-form branch (scalar_box boxes a bare Int result), so it escapes+renders as the NOMINAL
           `(: 5 W)` — a running case here implicitly proves the module is VALID (an invalid module could not
           run), and the output pins the nominal rendering (adv-64: NOT the erased `(: 5 Int64)`). The
           nullary path always rendered the nominal; this pins the param'd path agrees.")
  (input (do (type W (Mk Int64)) (def (main (: k Int64)) (Mk k)) (export main)))
  (call main (: 5 Int64))
  (output (: 5 W)))

(case
  "a GENERIC scalar newtype instantiated at Int64 escapes a param'd export as a valid module"
  (doc
    "The generic face of the scalar-newtype escape: `(type Box (Mk a))` instantiated at Int64 takes the
           same erased-scalar escape path and emits a valid module, rendering the nominal `(: 5 Box)`. Pins
           that the escape router's scalar fall-through is not confused by the type parameter.")
  (input (do (type Box (Mk a)) (def (main (: k Int64)) (Mk k)) (export main)))
  (call main (: 5 Int64))
  (output (: 5 Box)))

(case
  "a NARROW-inner scalar newtype escaping a param'd export emits a valid module (i32-slot box)"
  (doc
    "The width-edge face: `(type U8 (Mk UInt8))` has a sub-i32 (i32-slot) inner, so the scalar box's
           i32->i64 extend must fire for the <=32 slot (and must NOT for a mid/full width) — a wrong extend
           was an invalid-module risk. Escapes+renders the nominal `(: 5 U8)`, and running proves the module
           is valid. Pairs with the full-width W case (Int64, no extend).")
  (input (do (type U8 (Mk UInt8)) (def (main (: k UInt8)) (Mk k)) (export main)))
  (call main (: 5 UInt8))
  (output (: 5 U8)))

(case
  "a COMPOUND-inner single-variant newtype takes the heap escape and its inner is read back"
  (doc
    "The compound-inner counterpart: `(type LW (Mk (List Int64)))` erases to a list HANDLE, so it stays
           on the heap/resource escape branch (the recursive-sum-branch guard keeps it there) — the fix only
           diverts SCALAR-erased newtypes to the value-form branch. Wrapping a runtime-built [1,2] and matching
           it back reads the inner list length 2, confirming the compound path is unaffected.")
  (input
    (do
      (type LW (Mk (List Int64)))
      (def (wrap (: xs (List Int64))) (Mk xs))
      (def (main) (match (wrap (List.push (List.push #list() 1) 2)) ((Mk ys) (List.len ys))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a scalar newtype matched back in place round-trips the erased inner"
  (doc
    "Control (the matched face always worked — the match re-erases the Payload step): `(match (Mk k)
           ((Mk v) v))` deconstructs the newtype in place and returns the erased inner k. k=5 -> 5. Pins the
           scalar-newtype value round-trip is unbroken by the escape-branch fix.")
  (input (do (type W (Mk Int64)) (def (main (: k Int64)) (match (Mk k) ((Mk v) v))) (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a scalar newtype wrapped by a param'd helper then matched back crosses the internal call boundary"
  (doc
    "Control: a param'd helper `wrap` returns the newtype (the escaping-def value), and main matches it
           back — the erased scalar crosses the internal call boundary and is deconstructed. A NEGATIVE value
           (wrap(-9) then (Mk v)->v = -9) exercises sign preservation of the erased scalar across the internal
           call. Pins the wrapped-then-matched round-trip alongside the direct escape.")
  (input
    (do
      (type W (Mk Int64))
      (def (wrap (: k Int64)) (Mk k))
      (def (main (: k Int64)) (match (wrap k) ((Mk v) v)))
      (export main)))
  (call main (: -9 Int64))
  (output (: -9 Int64)))

(case
  "the NULLARY scalar-newtype escape renders the nominal (: 5 W), agreeing with the param'd path"
  (doc
    "The nullary counterpart of the param'd escape: `(def (main) (Mk 5))` bakes the constant and returns
           the newtype. It renders the SAME nominal `(: 5 W)` the param'd export does — pinning that the two
           escape paths AGREE (the adv-64 regression was the param'd path DIVERGING from the always-nominal
           nullary path). With the param'd case above, this closes the adv-64 agreement pin.")
  (input (do (type W (Mk Int64)) (def (main) (Mk 5)) (export main)))
  (output (: 5 W)))

(case
  "a NARROW-inner scalar newtype escape at a NEGATIVE value renders the nominal (: -300 I16)"
  (doc
    "The second width-edge face (paired with the U8 case): `(type I16 (Mk Int16))` at a NEGATIVE value
           exercises the i32-slot box's SIGN handling across the escape — a wrong (zero- vs sign-) extend would
           corrupt a negative narrow inner. Escapes+renders the nominal `(: -300 I16)`; a running case proves
           the module valid, covering the I16 validity face the migrated Rust test checked, in the corpus.")
  (input (do (type I16 (Mk Int16)) (def (main (: k Int16)) (Mk k)) (export main)))
  (call main (: -300 Int16))
  (output (: -300 I16)))

(case
  "a multi-variant sum box is REAL not erased: (Some k) wrapped then matched round-trips"
  (doc
    "Control: a MULTI-variant sum `(Some k)` is a genuinely boxed value (unlike the erased single-variant
           newtype), so its box is not erased away. A param'd helper wraps it and main matches it back —
           wrap(42) then (Some v)->v / (None)->0 = 42. Pins that the single-variant erase-and-escape fix leaves
           a real multi-variant sum box untouched.")
  (input
    (do
      (def (wrap (: k Int64)) (Some k))
      (def (main (: k Int64)) (match (wrap k) ((Some v) v) ((None) 0)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64)))

(case
  "a payloadless enum PARAM member (order matches) crosses as a TYPED WIT enum — Direction B (the param twin of SHAPE 60)"
  (doc
    "SHAPE 67 - a payloadless enum (Color = Red|Green|Blue) as a typed EXPORT-interface PARAM member under an imposed world declaring `(param c (\"enum\" red green blue))`. The PARAM twin of SHAPE 60 (enum RESULT): the guest def receives the raw i32 disc (select's enum-disc rep — no heap handle), so the typed-interface wrapper passes the boundary disc STRAIGHT THROUGH as the guest disc when the WIT/guest case orders MATCH (a `params` `None` passthrough), and the enum DEFINED type is emitted + re-exported by the `note` pass. Fix (record_interface_export Sum-param arm → is_enum_disc branch → i32 disc + any_enum_disc_param forces the typed wrapper). The harness marshals the arg BY WIT CASE NAME (`(red unit)` -> WIT disc 0), proving the typed enum boundary is emitted (a u32-handle fallback would reject a case-name arg). setColor(Red)->10, setColor(Green)->20, setColor(Blue)->30. Closes the typed enum PARAM (Direction B, order-matching) in the WIT boundary coverage matrix.")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member set-color (func (param c (enum red green blue)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (setColor (: c Color)) (match c ((Color.Red _) 10) ((Color.Green _) 20) ((Color.Blue _) 30)))
      (export setColor)))
  (call set-color (: (red unit) Color))
  (output (: 10 Int64))
  (call set-color (: (green unit) Color))
  (output (: 20 Int64))
  (call set-color (: (blue unit) Color))
  (output (: 30 Int64)))

(case
  "an enum PARAM member whose guest case order MISMATCHES the WIT remaps the disc by case NAME — the param twin of SHAPE 64"
  (doc
    "SHAPE 68 - the PARAM twin of SHAPE 64 (enum RESULT reorder). Guest `(type Color (Red)(Green)(Blue))` under an imposed world declaring `(param c (\"enum\" green red blue))` [red/green REVERSED]. The boundary supplies the WIT disc (green=0, red=1, blue=2); the typed-interface wrapper REMAPS it to the guest disc BY NAME (`inv_perm[wit_disc] = guest_disc`, so inv_perm=[1,0,2]: WIT `green`(0)->guest Green(1), WIT `red`(1)->guest Red(0), WIT `blue`(2)->guest Blue(2)) via a nested-if disc chain BEFORE the def call — the PARAM twin of ResultLower::EnumRemap (the emit is the shared emit_enum_disc_remap helper). SHOULD-WORK, name-keyed: supplying `(green unit)` yields the Green result (20) regardless of wire order, supplying `(red unit)` yields Red (10), `(blue unit)` yields Blue (30) — identical semantics to the order-MATCHING SHAPE 67, the enum being the degenerate name-keyed variant (same ruling as the record boundary SHAPE 20 / the enum RESULT SHAPE 64). Pure i32 compare/select, no runtime op, no memory, no reclaim (a raw i32 disc).")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member set-color (func (param c (enum green red blue)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (setColor (: c Color)) (match c ((Color.Red _) 10) ((Color.Green _) 20) ((Color.Blue _) 30)))
      (export setColor)))
  (call set-color (: (green unit) Color))
  (output (: 20 Int64))
  (call set-color (: (red unit) Color))
  (output (: 10 Int64))
  (call set-color (: (blue unit) Color))
  (output (: 30 Int64)))

(case
  "a typed list<tuple<s64,s64>> EXPORT result crosses as a WIT list of tuples (declared world)"
  (doc
    "SHAPE 69 - a TYPED `list<tuple<s64,s64>>` EXPORT result under an imposed world. The result-lower's SpillRecord path (`canon_write_of` Ty::List → CanonWrite::List whose element is the Tuple arm's 2-field Record write) composes with NO new emit: `vec-len`/`vec-get` over the def's list, each element written at the canonical `tuple<s64,s64>` offsets. The TYPED-EXPORT twin of SHAPE 7 (which round-trips a list-of-records only via the untyped run/encode envelope, NOT a self-declared WIT type). getPairs(x) = [(x, x+1), (x+10, x+11)]; x=5 -> [(5,6),(15,16)]. KNOWN-LEAK: the spilled list result + its boxed tuple elements are not reclaimed after the copy-out (the SpillRecord-result reclaim class, same as SHAPE 60/62/63; value-correct, routed to v-memory-safety) -> pinned `(live-objects 0)`.")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member get-pairs (func (param x (s64)) (result (list (tuple (s64) (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (getPairs (: x Int64)) #list(#tuple(x (+ x 1)) #tuple((+ x 10) (+ x 11))))
      (export getPairs)))
  (call get-pairs (: 5 Int64))
  (output (: #list(#tuple(5 6) #tuple(15 16)) (List (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "a typed list<record> EXPORT result crosses as a WIT list of records (declared world)"
  (doc
    "SHAPE 70 - the RECORD-element twin of SHAPE 69: a TYPED `list<record{lo,hi}>` EXPORT result. The list element is a `record` written by `canon_write_of`'s Record arm (fields placed BY NAME at their WIT canonical offsets — the same name-permute as a bare record result, exercised per element). getRecs(x) = [{lo:x, hi:x+1}]; x=5 -> [{lo:5,hi:6}]. Pins the list<record> typed export result (SHAPE 7 was untyped run/encode). KNOWN-LEAK (SpillRecord-result reclaim class, as SHAPE 69).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          get-recs
          (func (param x (s64)) (result (list (record (= lo (s64)) (= hi (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (getRecs (: x Int64)) #list(#record((= lo x) (= hi (+ x 1)))))
      (export getRecs)))
  (call get-recs (: 5 Int64))
  (output
    (:
      #list(#record((= lo 5) (= hi 6)))
      (List (Record (: lo Int64) (: hi Int64)))))
  (live-objects 0))

(case
  "a typed list<tuple<s64, list<s64>>> EXPORT result — a NESTED-list element field crosses (declared world)"
  (doc
    "SHAPE 71 - the NESTED-compound-element case the coverage doc flagged as an open `list<record|tuple> element with a nested list field` gap: it actually COMPOSES with no new emit. The recursive `canon_write_of` builds CanonWrite::List{ elem = Record[ s64 @0, List @8 ] }, and the element write recurses into the inner `CanonWrite::List` (a `(ptr,len)` write at the tuple's second-field offset, its own `cabi_realloc`'d element buffer). getNested(x) = [(x, [x, x+1])]; x=5 -> [(5, [5,6])]. Disproves the doc gap — a nested list/record/tuple element already crosses on the RESULT side via recursive composition. KNOWN-LEAK (SpillRecord-result reclaim class, as SHAPE 69).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          get-nested
          (func (param x (s64)) (result (list (tuple (s64) (list (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (getNested (: x Int64)) #list(#tuple(x #list(x (+ x 1)))))
      (export getNested)))
  (call get-nested (: 5 Int64))
  (output
    (:
      #list(#tuple(5 #list(5 6)))
      (List (Tuple Int64 (List Int64)))))
  (live-objects 0))

(case
  "a typed list<variant> EXPORT result crosses as a WIT list of variants (declared world)"
  (doc
    "SHAPE 72 - a TYPED `list<variant{lo, hi(s64)}>` EXPORT result: the list element is a VARIANT written by `canon_write_of`'s Variant arm (per-arm disc + payload at the canonical variant offsets), exercised PER element. Composes with no new emit (CanonWrite::List{ elem = Variant }). getVs(x) = [Lo, Hi(x)]; x=9 -> [lo, hi(9)]. Closes the variant-element half of the list<compound>-element coverage (SHAPE 69/70/71 covered tuple/record/nested-list elements). KNOWN-LEAK (SpillRecord-result reclaim class, SHAPE 60/62/63).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member get-vs (func (param x (s64)) (result (list (variant (lo) (hi (s64)))))))))
  )
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Lo) (Hi Int64))
      (def (getVs (: x Int64)) #list((V.Lo unit) (V.Hi x)))
      (export getVs)))
  (call get-vs (: 9 Int64))
  (output (: #list((lo unit) (hi 9)) (List V)))
  (live-objects 0))

(case
  "a typed list<tuple<s64, variant>> EXPORT result — a VARIANT field inside a tuple element (declared world)"
  (doc
    "SHAPE 73 - the coverage-doc's flagged `tuple element whose field is a VARIANT` case: it COMPOSES with no new emit. `canon_write_of` builds CanonWrite::List{ elem = Record[ s64 @0, Variant @8 ] }, the element write recursing into the Variant arm at the tuple's second-field offset (per-arm disc + payload). getTv(x) = [(x, Hi(x+1)), (x+5, Lo)]; x=3 -> [(3, hi(4)), (8, lo)]. Together with SHAPE 71 (nested list field) this disproves the whole `list<...> element with a nested record/list/tuple/variant field` doc gap on the RESULT side — recursive composition already crosses every compound element. KNOWN-LEAK (SpillRecord-result reclaim class).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          get-tv
          (func (param x (s64)) (result (list (tuple (s64) (variant (lo) (hi (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Lo) (Hi Int64))
      (def (getTv (: x Int64)) #list(#tuple(x (V.Hi (+ x 1))) #tuple((+ x 5) (V.Lo unit))))
      (export getTv)))
  (call get-tv (: 3 Int64))
  (output
    (:
      #list(#tuple(3 (hi 4)) #tuple(8 (lo unit)))
      (List (Tuple Int64 V))))
  (live-objects 0))

(case
  "a result<s64,s64> EXPORT result crosses as a typed WIT result (declared world)"
  (doc
    "SHAPE 74 - a typed `result<s64,s64>` EXPORT result. Was a DECLINE: `canon_write_of`'s Sum arm handled only `option<T>` (nullary+payload) and payloadless enums; a `result<ok,err>` (a 2-variant sum whose BOTH arms carry a payload) fell through to `return None`. Now a `canon_write_of` Result arm maps each guest variant BY NAME (`Ok`->boundary disc 0, `Err`->disc 1) to a `CanonWrite::Variant` arm writing the payload at the canonical result layout (1-byte disc + payload at `align_up(1, max(align(ok), align(err)))`). Reuses the existing `CanonWrite::Variant` emit (SHAPE 61) - no new writer. classify(x) = x>0 ? Ok(x) : Err(-x); x=5 -> Ok(5), x=-3 -> Err(3). KNOWN-LEAK (SpillRecord-result reclaim class, SHAPE 60/62/63).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member classify (func (param x (s64)) (result (result (s64) (s64))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (classify (: x Int64)) (if (> x 0) (Result.Ok x) (Result.Err (- 0 x))))
      (export classify)))
  (call classify (: 5 Int64))
  (output (: (Ok 5) (Result Int64 Int64)))
  (call classify (: -3 Int64))
  (output (: (Err 3) (Result Int64 Int64)))
  (live-objects 0))

(case
  "a result<record,s64> EXPORT result crosses — a COMPOUND ok payload (declared world)"
  (doc
    "SHAPE 75 - the compound-payload twin of SHAPE 74: a typed `result<record{lo,hi}, s64>` EXPORT result. The Ok arm's payload is a RECORD, written by the `canon_write_of` Result arm recursing into the Record arm at the payload offset (the same recursion the variant/list arms use). Proves the result<> writer composes for a compound payload, not just scalars. cl(x) = x>0 ? Ok({lo:x, hi:x+1}) : Err(-x); x=5 -> Ok({lo:5,hi:6}), x=-3 -> Err(3). KNOWN-LEAK (SpillRecord-result reclaim class).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          cl
          (func (param x (s64)) (result (result (record (= lo (s64)) (= hi (s64))) (s64))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (cl (: x Int64))
        (if (> x 0) (Result.Ok #record((= lo x) (= hi (+ x 1)))) (Result.Err (- 0 x))))
      (export cl)))
  (call cl (: 5 Int64))
  (output (: (Ok #record((= lo 5) (= hi 6))) (Result (Record (: lo Int64) (: hi Int64)) Int64)))
  (call cl (: -3 Int64))
  (output (: (Err 3) (Result (Record (: lo Int64) (: hi Int64)) Int64)))
  (live-objects 0))

(case
  "a flat single-scalar-field record EXPORT result crosses (returned directly, not by pointer)"
  (doc
    "SHAPE 76 - a `record{v: s64}` EXPORT result. It flattens to ONE core value (MAX_FLAT_RESULTS=1), so the canonical ABI returns it DIRECTLY (in a register), NOT via a retptr - so `record_result_lower`'s SpillRecord path (which returns a pointer) declined it (`!sig_needs_memory -> return None`, the flat-1-value-record gap). Now a `ResultLower::FlatScalarField` lower: the def returns the record HANDLE, the wrapper reads its one field (`arr-get(handle, 0)` -> unbox, narrowing a <=32-bit value) and returns that scalar as the flattened result. No memory (returned in a register). f(x) = {v: x+1}; x=5 -> {v: 6}. KNOWN-LEAK (the def's record handle is not reclaimed after the field read - the SpillRecord-result reclaim class, SHAPE 60/62/63).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (record (= v (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) #record((= v (+ x 1))))
      (export f)))
  (call f (: 5 Int64))
  (output (: #record((= v 6)) (Record (: v Int64))))
  (live-objects 0))

(case
  "a spilled record EXPORT result RECLAIMS the def's result handle — ZERO live objects"
  (doc
    "SHAPE 77 - the SpillRecord-result reclaim regression-guard (NO `known-leak`, asserts 0 live objects). A spilled compound EXPORT result (`record{a,b}`) is written to the retptr'd return area by the canonical writer (`emit_result_spill`), which BORROWS the def's result handle (arr-get/etc.); the wrapper then `drop`s that handle (the def returns an OWNED result, callee-owns-args, so the caller-wrapper reclaims it — the borrowing writer retained nothing, so `drop` deep-reclaims the whole value tree). Before the reclaim the spilled result cell + its boxed children LEAKED one per call (the SpillRecord-result known-leak class the other 28-wit compound-result SHAPEs still pin as `known-leak`); this case pins the FIX (`(live-objects)` default = expect 0). A regression re-introducing the leak reds here.")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (record (= a (s64)) (= b (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) #record((= a x) (= b (+ x 1))))
      (export f)))
  (call f (: 5 Int64))
  (output (: #record((= a 5) (= b 6)) (Record (: a Int64) (: b Int64)))))

(case
  "a bare option<record> PARAM member crosses (compound option payload — sum_params + payload rebuild)"
  (doc
    "SHAPE 78 - a TOP-LEVEL `option<record{lo,hi}>` PARAM member. The prior option-param coverage was option<scalar> (SHAPE 44) + an option<s64> record FIELD; this is a COMPOUND option payload at the top-level param position, rebuilt via the sum-param path (branch on the boundary disc → build the guest Some(record)/None cell). f(Some{lo,hi}) = lo+hi, f(None) = -1; Some{3,4} -> 7. KNOWN-LEAK (the wrapper-built sum cell reclaim class).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          f
          (func (param o (option (record (= lo (s64)) (= hi (s64))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: o (Option (Record (: lo Int64) (: hi Int64)))))
        (match o ((Some r) (+ (. r lo) (. r hi))) ((None) -1)))
      (export f)))
  (call f (: (Some #record((= lo 3) (= hi 4))) (Option (Record (: lo Int64) (: hi Int64)))))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a typed list<option<s64>> EXPORT result crosses as a WIT list of options"
  (doc
    "SHAPE 79 - a typed `list<option<s64>>` EXPORT result (the RESULT twin of SHAPE 38's list<option> host-op ARG). Composes via `canon_write_of`'s List arm recursing into the option arm per element (disc byte + payload at the canonical option layout). f(x) = [Some(x), None]; x=5 -> [Some(5), (None unit)]. KNOWN-LEAK (SpillRecord-result reclaim class).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (list (option (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) #list((Some x) None))
      (export f)))
  (call f (: 5 Int64))
  (output (: #list((Some 5) (None unit)) (List (Option Int64))))
  (live-objects 0))

(case
  "a typed option<Bytes> EXPORT result crosses (top-level option<list<u8>>)"
  (doc
    "SHAPE 80 - a typed top-level `option<list<u8>>`/`option<Bytes>` EXPORT result (the bytes-payload twin of SHAPE 66's option<record> result). `canon_write_of`'s option arm recurses its payload into the Bytes arm (Some copies the rope + writes (ptr,len); None writes the nullary disc). f(x>0) -> Some(b\"hi\"), else None; x=5 -> Some(b\"hi\"). KNOWN-LEAK (SpillRecord-result reclaim class).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (option (list (u8)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) (if (> x 0) (Some (Bytes.of #list(104 105))) None))
      (export f)))
  (call f (: 5 Int64))
  (output (: (Some b"hi") (Option Bytes)))
  (live-objects 0))

(case
  "a result<record,record> EXPORT result crosses — BOTH arms carry a compound payload"
  (doc
    "SHAPE 81 - a typed `result<record{a}, record{b}>` EXPORT result: BOTH the Ok and Err arms carry a COMPOUND (record) payload. `canon_write_of`'s Result arm (#7217) recurses into the Record arm for EACH arm's payload — the both-compound case (SHAPE 75 had a scalar err arm). Ok{a=x} for x>0, Err{b=-x} otherwise; x=5 -> Ok{a=5}. Value-correct + 0-leak reclaim rides #7226 (the def result handle is dropped after the write). KNOWN-LEAK pin kept for parity with the SpillRecord-result family (now over-specified since #7226; a follow-up tightens the family).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member
          f
          (func (param x (s64)) (result (result (record (= a (s64))) (record (= b (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: x Int64))
        (if (> x 0) (Result.Ok #record((= a x))) (Result.Err #record((= b (- 0 x))))))
      (export f)))
  (call f (: 5 Int64))
  (output (: (Ok #record((= a 5))) (Result (Record (: a Int64)) (Record (: b Int64)))))
  (live-objects 0))

(case
  "a nested option<option<s64>> EXPORT result crosses — a SUM inside an option payload"
  (doc
    "SHAPE 82 - a typed `option<option<s64>>` EXPORT result: the outer option's payload is ITSELF an option (a nested SUM), distinct from option<scalar/record/list/bytes> (SHAPE 8/66/79/80). `canon_write_of`'s option arm recurses its payload into the option arm again (disc byte + a nested {disc, payload} at the canonical layout). f(x>0) -> Some(Some(x)), else Some(None); x=5 -> Some(Some(5)). Value-verified; 0-leak reclaim rides #7226. KNOWN-LEAK pin kept for SpillRecord-result-family parity (now over-specified since #7226).")
  (wit-world
    (world
      w
      (export
        cadenza:demo/iface
        (member f (func (param x (s64)) (result (option (option (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: x Int64)) (if (> x 0) (Some (Some x)) (Some None)))
      (export f)))
  (call f (: 5 Int64))
  (output (: (Some (Some 5)) (Option (Option Int64))))
  (live-objects 0))

(case
  "a plain (non-reducer) guest reads a field of a RECORD host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 83 — a RECORD host-import RESULT (probe.info : () -> record{sec,nsec}) on a PURE-IMPORT custom
           wit-world with a PLAIN top-level export (no typed record-interface EXPORT, no component-name) — the
           v-wit-boundary B1 shape. The world declares ONLY an import interface, so the world-driven compound
           boundary path did NOT engage (the old `allow_option_bytes` gate required a qualifying EXPORT), and
           the compound RECORD host result declined CDZ0903 on the bare/plain host-delegating envelope. B0
           broadened the decline gate to any imposed IMPORT interface; B1 wired the plain host-delegating
           envelope (`assemble_host_runtime_mem`) to DECLARE the result's WIT `record` defined-type in the host
           import instance-type (via `build_host_result_types`) + a shared-memory `cabi_realloc` so the spilled
           result the host writes is lifted into a value-heap Record. run() performs probe.info and returns its
           `.sec` field. Stubbing probe.info -> {sec:42, nsec:7} and asserting 42 makes the compound host result
           + its field projection load-bearing. WIT-dump verified: `import probe: interface { record host-result-t0
           {sec: s64, nsec: s64}; info: func() -> host-result-t0 }`. This is the IMPORT-side twin of the reducer
           SHAPE 11 (which crossed the same shape only because a typed record EXPORT set component-name).")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member info (func (result (record (= sec (s64)) (= nsec (s64)))))))))
  (input
    (do
      (effect probe (op info (-> Unit (Record (: sec Int64) (: nsec Int64)))))
      (def (run) (host (probe) (. (probe.info unit) sec)))
      (export run)))
  (call run)
  (host-responses
    (respond
      probe.info
      (: #record((= sec 42) (= nsec 7)) (Record (: sec Int64) (: nsec Int64)))))
  (host-calls (call cadenza:platform/probe.info))
  (output 42)
  (live-objects 0))

(case
  "a plain guest reads the LENGTH of a Bytes host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 84 — a Bytes (`list<u8>`) host-import RESULT (probe.blob : () -> list<u8>) on a PURE-IMPORT
           custom wit-world, plain export, result CONSUMED IN-GUEST (Bytes.len). Pins the Bytes leaf arm of
           the compound host-RESULT lift on the plain host-delegating envelope (v-wit-boundary B1). This is a
           conformance-vocabulary shape (v-hivemind's host results are always read in-guest). Stub blob ->
           3 bytes, assert len 3.")
  (wit-world
    (world
      w
      (import cadenza:platform/probe (member blob (func (result (list (u8))))))))
  (input
    (do
      (effect probe (op blob (-> Unit Bytes)))
      (def (run) (host (probe) (Bytes.len (probe.blob unit))))
      (export run)))
  (call run)
  (host-responses (respond probe.blob (: #list(1 2 3) Bytes)))
  (host-calls (call cadenza:platform/probe.blob))
  (output 3)
  (live-objects 0))

(case
  "a plain guest matches an option<s64> host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 85 — an option<s64> host-import RESULT (probe.maybe : () -> option<s64>) on a PURE-IMPORT custom
           wit-world, plain export, result CONSUMED IN-GUEST (match Some(x)->x / None->-1). Pins the
           option-shaped-sum arm of the compound host-RESULT lift on the plain host-delegating envelope
           (v-wit-boundary B1). Stub maybe -> Some(7), assert 7.")
  (wit-world
    (world
      w
      (import cadenza:platform/probe (member maybe (func (result (option (s64))))))))
  (input
    (do
      (effect probe (op maybe (-> Unit (Option Int64))))
      (def (run) (host (probe) (match (probe.maybe unit) ((Option.Some x) x) (Option.None -1))))
      (export run)))
  (call run)
  (host-responses (respond probe.maybe (: (Some 7) (Option Int64))))
  (host-calls (call cadenza:platform/probe.maybe))
  (output 7)
  (live-objects 0))

(case
  "a plain guest reads the LENGTH of a list<s64> host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 86 — a list<s64> host-import RESULT (probe.nums : () -> list<s64>) on a PURE-IMPORT custom
           wit-world, plain export, result CONSUMED IN-GUEST (List.len). Pins the List arm of the compound
           host-RESULT lift on the plain host-delegating envelope (v-wit-boundary B1). Stub nums ->
           [10,20,30], assert len 3.")
  (wit-world
    (world
      w
      (import cadenza:platform/probe (member nums (func (result (list (s64))))))))
  (input
    (do
      (effect probe (op nums (-> Unit (List Int64))))
      (def (run) (host (probe) (List.len (probe.nums unit))))
      (export run)))
  (call run)
  (host-responses (respond probe.nums (: #list(10 20 30) (List Int64))))
  (host-calls (call cadenza:platform/probe.nums))
  (output 3)
  (live-objects 0))

(case
  "a plain guest reads the scalar-length of a String host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 87 — a String host-import RESULT (probe.greet : () -> string) on a PURE-IMPORT custom wit-world,
           plain export, result CONSUMED IN-GUEST (String.scalar-len). Pins the String leaf arm of the compound
           host-RESULT lift on the plain host-delegating envelope (v-wit-boundary B1). A conformance-vocabulary
           candidate (v-hivemind uses String results). Stub greet -> \"hello\", assert scalar-len 5.")
  (wit-world
    (world w (import cadenza:platform/probe (member greet (func (result (string)))))))
  (input
    (do
      (effect probe (op greet (-> Unit String)))
      (def (run) (host (probe) (String.scalar-len (probe.greet unit))))
      (export run)))
  (call run)
  (host-responses (respond probe.greet (: "hello" String)))
  (host-calls (call cadenza:platform/probe.greet))
  (output 5)
  (live-objects 0))

(case
  "a plain guest matches an option<record> host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 88 — an option<record{lo,hi}> host-import RESULT (probe.maybe-pt) on a PURE-IMPORT custom
           wit-world, plain export, result CONSUMED IN-GUEST (match Some(r)->r.lo / None->-1). Pins the
           option-of-COMPOUND arm of the compound host-RESULT lift on the plain host-delegating envelope
           (v-wit-boundary B1). A conformance-vocabulary candidate (Option<record>). Stub -> Some({lo:7,hi:9}),
           assert 7.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member maybe-pt (func (result (option (record (= lo (s64)) (= hi (s64))))))))))
  (input
    (do
      (effect probe (op maybe-pt (-> Unit (Option (Record (: lo Int64) (: hi Int64))))))
      (def
        (run)
        (host (probe) (match (probe.maybe-pt unit) ((Option.Some r) (. r lo)) (Option.None -1))))
      (export run)))
  (call run)
  (host-responses
    (respond
      probe.maybe-pt
      (:
        (Some #record((= lo 7) (= hi 9)))
        (Option (Record (: lo Int64) (: hi Int64))))))
  (host-calls (call cadenza:platform/probe.maybe-pt))
  (output 7)
  (live-objects 0))

(case
  "a plain guest reads the length of a list<record> host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 89 — a list<record{lo,hi}> host-import RESULT (probe.events) on a PURE-IMPORT custom wit-world,
           plain export, result CONSUMED IN-GUEST (List.len). Pins the list-of-COMPOUND arm of the compound
           host-RESULT lift on the plain host-delegating envelope (v-wit-boundary B1). A conformance-vocabulary
           candidate (v-hivemind's event stream is a list<record>). Stub -> [{1,2},{3,4}], assert len 2.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member events (func (result (list (record (= lo (s64)) (= hi (s64))))))))))
  (input
    (do
      (effect probe (op events (-> Unit (List (Record (: lo Int64) (: hi Int64))))))
      (def (run) (host (probe) (List.len (probe.events unit))))
      (export run)))
  (call run)
  (host-responses
    (respond
      probe.events
      (:
        #list(#record((= lo 1) (= hi 2)) #record((= lo 3) (= hi 4)))
        (List (Record (: lo Int64) (: hi Int64))))))
  (host-calls (call cadenza:platform/probe.events))
  (output 2)
  (live-objects 0))

(case
  "a plain guest matches an ENUM host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 90 — a payloadless ENUM host-import RESULT (probe.color : () -> enum{red,green,blue}) crossing
           BY VALUE (one i32 discriminant) on a PURE-IMPORT custom wit-world, plain export, matched IN-GUEST.
           The plain-path twin of `wen1` (which crossed the same shape only via a typed record-interface EXPORT
           + component-name). Closes the v-wit-boundary enum-by-value gap: an enum RESULT rides the same
           `result_crefs[i]` path as a spilled compound (`build_host_result_types` maps `enum_result` to the
           enum's nominal `enum` DEFINED+EXPORTED type; the core result stays a bare i32), so removing the
           plain-path enum-result decline guard suffices — no extra emit. WIT-dump verified `enum host-result-t0
           {red,green,blue}` + `color: func() -> that`. Stub color -> green, assert 1.")
  (wit-world
    (world w (import cadenza:platform/probe (member color (func (result (enum red green blue)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op color (-> Unit Col)))
      (def
        (run)
        (host (probe) (match (probe.color unit) ((Col.Red) 0) ((Col.Green) 1) ((Col.Blue) 2))))
      (export run)))
  (call run)
  (host-responses (respond probe.color (: (green unit) color)))
  (host-calls (call cadenza:platform/probe.color))
  (output 1)
  (live-objects 0))

(case
  "a plain guest matches a scalar-payload VARIANT host-import result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 91 — a scalar-payload VARIANT host-import RESULT (probe.stat : () -> variant{ok, err(s64)}) on a
           PURE-IMPORT custom wit-world, plain export, matched IN-GUEST (Ok->0 / Err(e)->e). Pins the general
           scalar-payload variant arm of the compound host-RESULT lift on the plain host-delegating envelope
           (v-wit-boundary B1) — the last shape of the consumed-in-guest result vocabulary. Stub stat ->
           err(42), assert 42.")
  (wit-world
    (world
      w
      (import cadenza:platform/probe (member stat (func (result (variant (ok) (err (s64)))))))))
  (input
    (do
      (type St (Ok) (Err Int64))
      (effect probe (op stat (-> Unit St)))
      (def (run) (host (probe) (match (probe.stat unit) ((St.Ok) 0) ((St.Err e) e))))
      (export run)))
  (call run)
  (host-responses (respond probe.stat (: (err 42) stat)))
  (host-calls (call cadenza:platform/probe.stat))
  (output 42)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a RECORD host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 92 — an all-scalar RECORD host-op ARGUMENT (probe.push : func(record{a,b}) -> s64) on a
           PURE-IMPORT custom wit-world with a PLAIN top-level export (no reducer/typed interface, no
           component-name) — the v-wit-boundary B3 shape: compound host ARGUMENTS on the plain host-delegating
           envelope. B1/B1b crossed compound host RESULTS on this path; B3 routes the world-imposed plain path
           through `build_host_group`, which declares the record's WIT type in the host import instance-type,
           lays it as a nominal DEFINED type (`record_defs`), and bakes the nominal-arg type index into the
           op's component functype — so the guest FLATTENS the value-heap record into the op's core slots (two
           s64), exactly as the reducer path's record-arg marshal (`emit_record_arg_marshal`) does. Before B3
           the plain path declined a record ARG (it crossed only scalar/string/`list<u8>` args). run() builds
           {a:3,b:4}, performs probe.push, returns the stubbed result. A VALID component that runs is the pin:
           a mis-declared record-arg type (missing the nominal DEFINED type or a wrong nominal index) fails
           component validation (CDZ0910). This is the IMPORT-side plain-path twin of the reducer SHAPE 13
           (record{a,b} arg via deliver.push under a typed reducer export).")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member push (func (param m (record (= a (s64)) (= b (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: a Int64) (: b Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= a 3) (= b 4)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 99 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 99)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a scalar-payload VARIANT host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 93 — a scalar-payload bare-VARIANT host-op ARGUMENT (probe.emit : func(variant{go, stop(s64)})
           -> s64) on a PURE-IMPORT custom wit-world with a PLAIN top-level export — the bare-variant arm of
           the v-wit-boundary B3 compound host-ARGUMENT support. `build_host_group` lays a single shared
           `variant` DEFINED+EXPORTED type and bakes its index into the op's component functype; the guest
           flattens the variant to `(disc:i32, join(payloads))` core slots via `emit_variant_reg_flatten`
           (the same marshal a record-field variant uses, now at the top-level param position). Before B3 the
           plain path declined a variant ARG. run() emits Stop(7) and performs probe.emit; the host stub
           returns 88. A VALID component that runs is the pin (a mis-declared/mis-flattened variant-arg type
           fails validation, CDZ0910). Plain-path twin of the reducer named-variant-arg SHAPE 18. Verified the
           emitted component IMPORTS `cadenza:platform/probe` + a bare `run: func()` export (the plain host-
           delegating shape).")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member emit (func (param v (variant (go) (stop (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op emit (-> Sig Int64)))
      (def (run) (host (probe) (probe.emit (Sig.Stop 7))))
      (export run)))
  (call run)
  (host-responses (respond probe.emit (: 88 Int64)))
  (host-calls (call cadenza:platform/probe.emit))
  (output 88)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a list<s64> host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 94 — a `list<s64>` host-op ARGUMENT (probe.sum : func(list<s64>) -> s64) on a PURE-IMPORT custom
           wit-world with a PLAIN top-level export — the list arm of the v-wit-boundary B3 compound host-
           ARGUMENT support. `build_host_group`'s `arg_list_crefs` prepends the shared `(list u8)` + the
           element defined type and the op's functype references the list arg; the guest marshals the
           value-heap list into `(ptr, count)` core slots + the element array via `emit_list_arg_marshal`.
           Before B3 the plain path crossed only a `list<u8>` (Bytes) arg. run() builds [3,4,5] and performs
           probe.sum; the host stub returns 12. A VALID component that runs is the pin. Plain-path twin of the
           reducer list<s64>-arg SHAPE 12.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member sum (func (param xs (list (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op sum (-> (List Int64) Int64)))
      (def (run) (host (probe) (probe.sum #list(3 4 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.sum (: 12 Int64)))
  (host-calls (call cadenza:platform/probe.sum))
  (output 12)
  (live-objects 0))

(case
  "a String host-import result escapes DIRECTLY as the entrypoint result via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 95 (v-wit-boundary B2, corpus TODO) — a STRING host-import RESULT (probe.spell : func(s64) -> string)
           that ESCAPES DIRECTLY as run()'s result, NOT consumed in-guest (contrast SHAPE 87, which reads its
           scalar-len). A directly-escaping compound host result routes to the resource-escape entrypoint emit
           (assemble_host_runtime_resource*), which does not yet declare the B1 result-lift machinery the plain
           host-delegating envelope has; without it the lift op resolved to an out-of-range func index and the
           component failed validation (invalid wasm). The idealistic behavior is that the host's string crosses
           out unchanged (assert \"ok\"). Until B2 threads the result-lift through the resource-escape sites the
           compiler DECLINES CLEANLY (CDZ0900, decline-don't-miscompile) rather than emitting invalid wasm — so
           this case grades Todo now and auto-locks to Pass when B2 lands. Stub spell -> \"ok\".")
  (wit-world
    (world w (import cadenza:platform/probe (member spell (func (param n (s64)) (result (string)))))))
  (input
    (do
      (effect probe (op spell (-> Int64 String)))
      (def (run) (host (probe) (probe.spell 5)))
      (export run)))
  (call run)
  (host-responses (respond probe.spell (: "ok" String)))
  (host-calls (call cadenza:platform/probe.spell))
  (output (: "ok" String))
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes an all-nullary ENUM host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 96 — an all-nullary ENUM host-op ARGUMENT (probe.tag : func(enum{red,green,blue}) -> s64) on a
           PURE-IMPORT custom wit-world with a PLAIN top-level export — the enum arm of the v-wit-boundary B3
           compound host-ARGUMENT support, and the un-park of the tick-18 enum-arg gap. On HEAD the enum arg
           REACHES the plain path: the perform lowers to `Core::HostCall` (is_world_import_op TRUE, NO reify),
           the component imports `cadenza:platform/probe`, and the enum arg crosses as a properly-cased WIT
           `enum{red,green,blue}` (verified via wit-dump); the tick-18 resource-escape reification is no longer
           reproducible. The guest flattens the value-heap nullary enum to the op's single core i32 disc slot.
           run() builds Col.Green, performs probe.tag, returns the stubbed result. A VALID component that runs is
           the pin (the enum-arg twin of the RECORD arg SHAPE 92 / VARIANT arg SHAPE 93 / list<s64> arg SHAPE
           94). KNOWN RESIDUE (cosmetic, does not affect run/validation): the reflected enum arg type is named by
           the generic `host-record-p<n>` scheme, so it emits as `enum host-record-p0 {red,green,blue}` — a real
           WIT enum with the right cases (structural WIT match at link is by case-set), only the type NAME is a
           misnomer. Stub tag -> 55.")
  (wit-world
    (world w (import cadenza:platform/probe (member tag (func (param c (enum red green blue)) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op tag (-> Col Int64)))
      (def (run) (host (probe) (probe.tag (Col.Green))))
      (export run)))
  (call run)
  (host-responses (respond probe.tag (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.tag))
  (output 55)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes an OPTION host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 97 — an `option<s64>` host-op ARGUMENT (probe.f : func(option<s64>) -> s64) on a PURE-IMPORT custom
           wit-world with a PLAIN top-level export. The OPTION arm of the compound-ARG vocabulary (after record
           SHAPE 92 / variant SHAPE 93 / list SHAPE 94 / enum SHAPE 96). The value-heap `Option` flattens to the
           canonical `(disc:i32, payload)` core form (the register twin of the option<scalar> record-FIELD
           flatten, `select::emit_option_reg_flatten`, mapping the guest some-disc to WIT option some=1/none=0)
           and crosses as the BUILT-IN WIT `option<s64>` (verified via wit-dump: `f: func(p0: option<s64>)` — a
           per-param structural CRef, NOT a nominal variant substitute), referenced by the op's component
           functype. run() performs probe.f(Some 7), the host stub returns 20. A VALID component that runs is the
           pin. SCOPED to a SCALAR payload; an option<compound> (bytes/record) top-level arg is a later increment
           (declined). Twin of the option-in-record SHAPE 99.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (option (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Option Int64) Int64)))
      (def (run) (host (probe) (probe.f (Some 7))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 20 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 20)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a TUPLE host-op ARGUMENT via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 98 — a `tuple<s64, s64>` host-op ARGUMENT (probe.g : func(tuple<s64, s64>) -> s64) on a PURE-IMPORT
           custom wit-world with a PLAIN top-level export. The TUPLE arm of the compound-ARG vocabulary
           (positional sibling of the RECORD arg SHAPE 92). The value-heap tuple flattens POSITIONALLY to one
           scalar core slot per element (no discriminant, via `select::emit_tuple_reg_flatten` — arr-get each
           element + unbox) and crosses as the BUILT-IN WIT `tuple<s64, s64>` (verified via wit-dump: `g: func(p0:
           tuple<s64, s64>)` — a per-param structural CRef), referenced by the op's component functype. run()
           performs probe.g(#tuple(3 4)), the host stub returns 30. A VALID component that runs is the pin. SCOPED
           to ALL-SCALAR elements; a tuple with a compound element is a later increment (declined). Twin of the
           tuple-in-record SHAPE 100.")
  (wit-world
    (world w (import cadenza:platform/probe (member g (func (param p (tuple (s64) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op g (-> (Tuple Int64 Int64) Int64)))
      (def (run) (host (probe) (probe.g #tuple(3 4))))
      (export run)))
  (call run)
  (host-responses (respond probe.g (: 30 Int64)))
  (host-calls (call cadenza:platform/probe.g))
  (output 30)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a RECORD host-op ARGUMENT with an OPTION field via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 99 — a RECORD host-op ARGUMENT one of whose fields is an `option<s64>` (probe.p : func(record{a:s64,
           m:option<s64>}) -> s64) on a PURE-IMPORT custom wit-world with a PLAIN top-level export. While a
           TOP-LEVEL option arg is not yet supported (SHAPE 97, TODO), an option-typed record FIELD already
           crosses via the existing `RecordFieldAbi::Option` arm: the field is laid as an `(option <payload>)`
           DEFINED type (wit-dump: `type host-record-p0 = option<s64>` referenced by `record host-record-p1`),
           and the guest flattens the field to `(disc, payload)` in the record's core run. Pins the option-in-
           record composite-arg support (the arg-side analogue of the option RESULT vocab). run() builds
           {a:3, m:Some 9}, performs probe.p, the host stub returns 42. A VALID component that runs is the pin.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member p (func (param r (record (= a (s64)) (= m (option (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op p (-> (Record (: a Int64) (: m (Option Int64))) Int64)))
      (def (run) (host (probe) (probe.p #record((= a 3) (= m (Some 9))))))
      (export run)))
  (call run)
  (host-responses (respond probe.p (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.p))
  (output 42)
  (live-objects 0))

(case
  "a plain (non-reducer) guest passes a RECORD host-op ARGUMENT with a TUPLE field via a pure-IMPORT custom wit-world"
  (doc
    "SHAPE 100 — a RECORD host-op ARGUMENT one of whose fields is a `tuple<s64, s64>` (probe.q : func(record{
           a:s64, t:tuple<s64,s64>}) -> s64) on a PURE-IMPORT custom wit-world with a PLAIN top-level export.
           While a TOP-LEVEL tuple arg is not yet supported (SHAPE 98, TODO), a tuple-typed record FIELD already
           crosses via the existing `RecordFieldAbi::Tuple` arm: the field is laid as a `(tuple <elem>…)` DEFINED
           type (wit-dump: `type host-record-p0 = tuple<s64, s64>` referenced by the record) and the guest
           flattens the tuple's elements positionally into the record's core run. Pins the tuple-in-record
           composite-arg support (positional sibling of the option-in-record SHAPE 99). run() builds
           {a:1, t:(2,3)}, performs probe.q, the host stub returns 24. A VALID component that runs is the pin.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/probe
        (member q (func (param r (record (= a (s64)) (= t (tuple (s64) (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op q (-> (Record (: a Int64) (: t (Tuple Int64 Int64))) Int64)))
      (def (run) (host (probe) (probe.q #record((= a 1) (= t #tuple(2 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.q (: 24 Int64)))
  (host-calls (call cadenza:platform/probe.q))
  (output 24)
  (live-objects 0))

(case
  "a run() body performing SEVEN host ops ending in a non-empty STRING-arg query does not corrupt the string arg"
  (doc
    "SHAPE 101 (v-wit-boundary regression guard, requested by v-hivemind + concierge) — a PURE-IMPORT custom
           wit-world with SEVEN host ops delegated in ONE run() body (mixed scalar / list<u8> / string ARGS and
           scalar / list<record> RESULTS): store, compute, node, put, spawn, send, then a LAST materialize op
           whose FIRST arg is a NON-EMPTY string. v-hivemind reported (on OLD pins 4ada599 / ab7db95) a
           context-sensitive miscompile where a non-empty string host-arg mislowered to a bad (ptr,len) once
           ENOUGH host ops shared a frame — wasmtime's canonical-ABI lift of that string then trapped invalid
           utf-8; the empty-string variant passed. On CURRENT main this PLAIN-HOST shape PASSES (verified in real
           wasmtime via cdz-run): every arg — including materialize's `session-spawned` — lifts correctly, so this
           pins that the many-host-op inline arg-marshal + slot-allocation keeps each string arg's (ptr,len)
           intact across a 7-op frame. NOTE: this guards the PLAIN-HOST path; v-hivemind's real flow additionally
           uses RESOURCE-typed ops (spawn/cluster return handle resources) — if a residual repro survives it is on
           the resource path, tracked separately. materialize returns 2 records → List.len 2.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/sys
        (member store (func (param k (list (u8))) (param v (list (u8))) (result (u64))))
        (member compute (func (param n (u64)) (param name (string)) (result (u64))))
        (member node (func (param label (string)) (result (u64))))
        (member put (func (param prog (list (u8))) (result (u64))))
        (member spawn (func (param nid (u64)) (param prog (u64)) (result (u64))))
        (member send (func (param to (u64)) (param msg (list (u8))) (result (u64))))
        (member
          materialize
          (func
            (param kind (string))
            (param session (list (u8)))
            (param source (string))
            (result (list (record (= id (s64))))))))))
  (input
    (do
      (effect sys
        (op store (-> Bytes (-> Bytes UInt64)))
        (op compute (-> UInt64 (-> String UInt64)))
        (op node (-> String UInt64))
        (op put (-> Bytes UInt64))
        (op spawn (-> UInt64 (-> UInt64 UInt64)))
        (op send (-> UInt64 (-> Bytes UInt64)))
        (op materialize (-> String (-> Bytes (-> String (List (Record (: id Int64))))))))
      (def (run)
        (host (sys)
          (let ((_s1 (sys.store b"key" b"val")))
            (let ((_c1 (sys.compute 3 "adder")))
              (let ((n1 (sys.node "c1")))
                (let ((p1 (sys.put b"program-bytes")))
                  (let ((a1 (sys.spawn n1 p1)))
                    (let ((_r1 (sys.send a1 b"m")))
                      (List.len (sys.materialize "session-spawned" b"" ""))))))))))
      (export run)))
  (call run)
  (host-responses
    (respond sys.store (: 0 UInt64))
    (respond sys.compute (: 0 UInt64))
    (respond sys.node (: 1 UInt64))
    (respond sys.put (: 2 UInt64))
    (respond sys.spawn (: 3 UInt64))
    (respond sys.send (: 4 UInt64))
    (respond sys.materialize (: #list(#record((= id 1)) #record((= id 2))) (List (Record (: id Int64))))))
  (host-calls
    (call cadenza:platform/sys.store)
    (call cadenza:platform/sys.compute)
    (call cadenza:platform/sys.node)
    (call cadenza:platform/sys.put)
    (call cadenza:platform/sys.spawn)
    (call cadenza:platform/sys.send)
    (call cadenza:platform/sys.materialize))
  (output 2)
  (live-objects 0))

(case
  "a single host op with a NON-EMPTY string arg AND a spilled list<record> result crosses both correctly"
  (doc
    "SHAPE 102 (v-wit-boundary regression guard) — a SINGLE host op materialize(kind:string, session:list<u8>,
           source:string) -> list<record{id}> on a PURE-IMPORT custom wit-world: a NON-EMPTY string ARG
           ALONGSIDE a spilled compound (list<record>) RESULT on the SAME op. This is the minimal single-op form
           of the interaction v-hivemind originally suspected (a compound-list result mislowering a non-empty
           string arg). It was UNPINNED: existing string-ARG cases (log.emit) pair a string arg with a scalar/
           unit result, and existing spilled-RESULT cases pair a compound result with scalar/unit args — the
           COMBINATION (non-empty string arg + spilled list<record> result, one op) had no guard. On current main
           it PASSES (verified in real wasmtime): the `session-spawned` arg lifts correctly host-side AND the
           list<record> result lifts into the value-heap, List.len 2. Pins that the string-arg (ptr,len) marshal
           and the spilled-result retptr/lift do not corrupt each other on one op. Complements the 7-op SHAPE 101.")
  (wit-world
    (world
      w
      (import
        cadenza:platform/sys
        (member
          materialize
          (func
            (param kind (string))
            (param session (list (u8)))
            (param source (string))
            (result (list (record (= id (s64))))))))))
  (input
    (do
      (effect sys (op materialize (-> String (-> Bytes (-> String (List (Record (: id Int64))))))))
      (def (run) (host (sys) (List.len (sys.materialize "session-spawned" b"" ""))))
      (export run)))
  (call run)
  (host-responses
    (respond sys.materialize (: #list(#record((= id 7)) #record((= id 8))) (List (Record (: id Int64))))))
  (host-calls (call cadenza:platform/sys.materialize))
  (output 2)
  (live-objects 0))

(case
  "a compile-time-constant None passed as a top-level option host-op ARGUMENT (corpus TODO — pre-existing bug)"
  (doc
    "SHAPE 103 (v-wit-boundary corpus TODO) — a COMPILE-TIME-CONSTANT `(None)` passed as a top-level
           `option<s64>` host-op ARGUMENT. This exposes a PRE-EXISTING defect in the option<scalar>-arg emit
           (landed SHAPE 97 / #9601, which only exercised `Some`): a literal `(None)` option arg makes the guest
           emit an INVALID module (CDZ0910 'values remaining on stack at end of block') — the const-None value
           lowering leaves an extra operand that the option arg-marshal path does not balance. NARROW: a RUNTIME
           option (Some OR a None from a conditional) marshals correctly, and a const `(None)` in a NON-arg
           context (e.g. matched) is fine — ONLY a compile-time-constant None in the top-level option host-arg
           position trips it. The idealistic behavior is that the const None crosses as WIT `option none` and the
           host returns its scalar (assert 5). Grades Todo now (CDZ0910 is a coded compile error) and auto-locks
           to Pass when the const-None option-arg emit is fixed. Runtime-option control is SHAPE 97 (Some) + the
           conditional-option path. ROOT CAUSE (verified via a WAT dump of the invalid module): a bare `(None)`
           arg's inferred type is `Option(Var)` — an UNGROUNDED payload var, because a perform/host-call argument
           is NOT checked against the operation's DECLARED parameter type (capabilities-and-effects.md #Performing
           An Operation Is Typed), so nothing grounds the None payload to `s64`; `(Some 5)` only works because the
           literal `5` self-provides an `Int(Deferred)` payload. Both the host-import FUNCTYPE builder
           (`collect_host_imports_at`) and the arg MARSHAL (`select/emit.rs`) guard the option arm on
           `abi_val_type(payload).is_some()`, which FAILS for the unground var: the functype builder's fallback
           drops the param (0 core slots) while the marshal's scalar fallback still pushes the folded None handle
           (1 slot) — the 1-vs-0 mismatch IS the 'values remaining on stack' imbalance. Confirmed: annotating the
           arg `(: (None) (Option Int64))` grounds the payload and compiles clean. Same defect class as the
           handler-state func-12 fix (`infer::ground_handler_state_ty`): an ungrounded `Option(_)` read at a
           width-dependent site. Correct fix is at the perform-argument check — ground the arg against the op's
           declared param type; routed to the inference owner. Blast-radius-scoped, so not landed with this pin.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (option (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Option Int64) Int64)))
      (def (run) (host (probe) (probe.f (None))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 5 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 5)
  (live-objects 0))

(case
  "a bare empty-list literal passed as a top-level list<s64> host-op ARGUMENT (corpus TODO — same ungrounded-arg root as the const-None case)"
  (doc
    "SHAPE 104 (v-wit-boundary corpus TODO) — a bare EMPTY-list literal `(list)` passed as a top-level
           `list<s64>` host-op ARGUMENT. SAME ungrounded-perform-arg root as SHAPE 103 (the const-None option arg):
           a bare `(list)` infers as `(List Any)` — an ungrounded ELEMENT type, because a perform/host-call
           argument is NOT checked against the operation's DECLARED parameter type (capabilities-and-effects.md
           #Performing An Operation Is Typed) — so nothing grounds the element to `s64`, the boundary guard sees
           `List Any` (which has no element boundary ABI), and the op DECLINES with CDZ0903. NARROW: a NON-empty
           list literal `(list 1 2)` grounds the element from its elements and crosses fine, and annotating
           `(: (list) (List Int64))` also crosses — ONLY a bare empty list literal in a top-level list host-arg
           position (where the element is otherwise unconstrained) trips it. The idealistic behavior is that the
           empty list crosses as WIT `list<s64>` with count 0 and the host returns its scalar (assert 7). Grades
           Todo now (CDZ0903 is a coded decline) and auto-locks to Pass when the perform-argument grounding fix
           lands — the SAME infer:: fix as SHAPE 103 (ground each perform arg against the op's declared param
           type). Companion regression gate to SHAPE 103: proving the fix generalizes from the option family to
           the list family, and that the empty-element case DECLINES cleanly (CDZ0903) rather than emitting an
           invalid module.")
  (wit-world
    (world w (import cadenza:platform/probe (member g (func (param xs (list (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op g (-> (List Int64) Int64)))
      (def (run) (host (probe) (probe.g #list())))
      (export run)))
  (call run)
  (host-responses (respond probe.g (: 7 Int64)))
  (host-calls (call cadenza:platform/probe.g))
  (output 7)
  (live-objects 0))

(case
  "a top-level tuple<list<u8>, s64> host-op ARGUMENT (bytes-carrying tuple) crosses as built-in tuple, Bytes element rope copied to mem"
  (doc
    "SHAPE 105 (v-wit-boundary) — a top-level `tuple<list<u8>, s64>` host-op ARGUMENT: a tuple one of whose
           elements is a `list<u8>` (Bytes). The all-SCALAR tuple arg crosses as the built-in WIT `tuple<T…>`
           (SHAPE 98/100, flattened positionally inline); this widens it to a tuple carrying a Bytes element.
           The tuple-arg classifier (`collect_host_imports_at`) admits a tuple whose every element is a scalar
           (`abi_val_type`) OR `Bytes`, mapping a Bytes element to `RecordFieldAbi::Bytes` (which
           `flatten_record_field_abi` lowers to 2 core slots); `emit_tuple_reg_flatten` flattens each element
           positionally — a SCALAR inline, a `Bytes` element copied into the shared `mem` at the running scratch
           cursor and pushed as `(ptr,len)`, exactly as a Bytes RECORD FIELD does (`emit_record_arg_marshal`'s
           Bytes branch). The tuple crosses as WIT `tuple<list<u8>, s64>` (the Bytes element as `(ptr,len)`, the
           s64 inline) and the host returns its scalar (assert 2 = len of b\"hi\"). Distinct from the
           ungrounded-arg TODOs (SHAPE 103/104): this was a marshal-widening gap, not a perform-arg typing gap.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (tuple (list (u8)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Tuple Bytes Int64) Int64)))
      (def (run) (host (probe) (probe.f #tuple(b"hi" 7))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 2 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 2)
  (live-objects 0))

(case
  "a top-level option<list<u8>> host-op ARGUMENT with a Some payload crosses as built-in option, rope copied to mem"
  (doc
    "SHAPE 106 (v-wit-boundary) — a top-level `option<list<u8>>` host-op ARGUMENT with a `Some b\"…\"`
           payload. Widens the built-in `option<T>` arg (SHAPE 97, option<scalar>) to a Bytes payload: the
           guest flattens the value-heap Option to `(disc, ptr, len)` core slots — on Some it copies the payload
           rope into the shared `mem` at the running scratch cursor and pushes `(ptr,len)`
           (`emit_option_reg_flatten`'s bytes branch, the register twin of a Bytes record FIELD). The component
           boundary type is the built-in `option<list<u8>>`. `live-objects 0` proves the marshaled-arg reclaim
           balances (the Option shell is deep-dropped after its rope is copied out — no leak, no UAF).")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (option (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Option Bytes) Int64)))
      (def (run) (host (probe) (probe.f (Some b"hi"))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 2 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 2)
  (live-objects 0))

(case
  "a top-level option<list<u8>> host-op ARGUMENT with a None value crosses as built-in option none"
  (doc
    "SHAPE 107 (v-wit-boundary) — the None arm of the `option<list<u8>>` host-op ARGUMENT (SHAPE 106's
           twin): a `(None)` value flattens to `(disc=0, ptr=0, len=0)` — a WIT `option` none carries no
           payload, so the guest writes nothing to `mem` and the host receives `none`. Exercises the branch the
           SHAPE-97 option<scalar> path only covered for scalars; the const-`(None)` payload now grounds to
           `list<u8>` via the perform-arg-vs-declared-param grounding (the SHAPE-103 fix), so the option<bytes>
           functype + marshal agree. Host returns its scalar (assert 5). `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (option (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Option Bytes) Int64)))
      (def (run) (host (probe) (probe.f (None))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 5 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 5)
  (live-objects 0))

(case
  "a top-level tuple<list<u8>, list<u8>> host-op ARGUMENT (TWO Bytes elements) crosses as built-in tuple, both ropes copied to disjoint mem"
  (doc
    "SHAPE 108 (v-wit-boundary) — a top-level `tuple<list<u8>, list<u8>>` host-op ARGUMENT: a tuple with TWO
           `list<u8>` (Bytes) elements. Pins the invariant that `emit_tuple_reg_flatten`'s single shared scratch
           CURSOR advances across MULTIPLE Bytes elements so the second rope is copied to a DISJOINT `mem` region
           past the first (each element captures `ptr = cursor` before its copy, then `cursor += len`), and the
           two `(ptr,len)` pairs flatten positionally in element order — the multi-Bytes-element twin of the
           single-Bytes SHAPE 105. Without a shared advancing cursor the second copy would clobber the first.
           The tuple crosses as WIT `tuple<list<u8>, list<u8>>` and the host returns its scalar
           (assert 5 = len b\"hi\" (2) + len b\"xyz\" (3)). `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (tuple (list (u8)) (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Tuple Bytes Bytes) Int64)))
      (def (run) (host (probe) (probe.f #tuple(b"hi" b"xyz"))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 5 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 5)
  (live-objects 0))

(case
  "a top-level tuple<list<u8>, s64, list<u8>> host-op ARGUMENT (Bytes-SCALAR-Bytes interleave) crosses as built-in tuple"
  (doc
    "SHAPE 109 (v-wit-boundary) — a top-level `tuple<list<u8>, s64, list<u8>>` host-op ARGUMENT: two Bytes
           elements with a SCALAR BETWEEN them. Pins the INTERLEAVE invariant of `emit_tuple_reg_flatten` —
           element 0 (Bytes) copies its rope to `mem` and advances the shared cursor, element 1 (scalar) pushes
           its i64 INLINE without touching the cursor/scratch, and element 2 (Bytes) advances the cursor again to
           a region disjoint from element 0's — so the flattened operand stack is `(ptr0,len0, s, ptr2,len2)`
           matching the positional core-slot layout (a Bytes element = 2 slots, a scalar = 1). A distinct path
           from the Bytes-only SHAPE 105 / Bytes-Bytes SHAPE 108: it exercises a scalar push threaded between two
           cursor-advancing Bytes copies. The tuple crosses as WIT `tuple<list<u8>, s64, list<u8>>`; host returns
           its scalar (assert 12 = len b\"hi\" (2) + 7 + len b\"abc\" (3)). `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (tuple (list (u8)) (s64) (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Tuple Bytes Int64 Bytes) Int64)))
      (def (run) (host (probe) (probe.f #tuple(b"hi" 7 b"abc"))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 12 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 12)
  (live-objects 0))

(case
  "a top-level tuple<list<u8>, list<u8>> host-op ARGUMENT with an EMPTY first Bytes element (len-0 rope, cursor no-advance)"
  (doc
    "SHAPE 110 (v-wit-boundary) — a top-level `tuple<list<u8>, list<u8>>` host-op ARGUMENT whose FIRST Bytes
           element is EMPTY (`b\"\"`). Pins the len-0 rope edge of `emit_tuple_reg_flatten`'s Bytes branch: the
           copy loop runs ZERO iterations (its `pos >= len` guard is true at entry), `cursor += 0` does NOT
           advance, and the pushed `(ptr, len)` is `(cursor, 0)` — so the SECOND (non-empty) element's rope is
           still copied to `cursor` (which the empty element left unmoved) and the two `(ptr,len)` pairs remain
           well-formed. Guards against an off-by-one in the copy-loop bound / a spurious cursor bump on a
           zero-length element (which would misplace the following element). The tuple crosses as WIT
           `tuple<list<u8>, list<u8>>`; host returns its scalar (assert 3 = len b\"\" (0) + len b\"xyz\" (3)).
           `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param x (tuple (list (u8)) (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Tuple Bytes Bytes) Int64)))
      (def (run) (host (probe) (probe.f #tuple(b"" b"xyz"))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 3 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 3)
  (live-objects 0))

(case
  "a host op with TWO params (list<u8> and s64) crosses as a two-param WIT import — cross-arg scratch cursor"
  (doc
    "SHAPE 111 (v-wit-boundary) — a host op with TWO params, `f(list<u8>, s64) -> s64`, on the plain
           host-delegating envelope. Every prior host-arg SHAPE was SINGLE-param (a multi-field record/tuple
           bundled the params); this pins that a host WIT func with MULTIPLE top-level params crosses as a genuine
           two-param import and that the per-call scratch cursor is threaded across DISTINCT args — arg 0's
           `list<u8>` (guest `Bytes`) backing is copied to `mem` at the cursor (which advances) while arg 1's
           `s64` passes INLINE without touching it, so the core slots interleave `(ptr,len, s)`. Host returns its
           scalar (assert 5 = len b\"abc\" (3) + 2). `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param b (list (u8))) (param n (s64)) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> Bytes Int64 Int64)))
      (def (run) (host (probe) (probe.f b"abc" 2)))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 5 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 5)
  (live-objects 0))

(case
  "a host op with TWO COMPOUND params (tuple<list<u8>,s64> and option<list<u8>>) — cross-arg scratch cursor, two cursor-consumers"
  (doc
    "SHAPE 112 (v-wit-boundary) — a host op `f(tuple<list<u8>, s64>, option<list<u8>>) -> s64` with TWO
           COMPOUND params BOTH consuming the per-call scratch cursor. Newly reachable now that multi-param host
           ops cross (SHAPE 111): every earlier multi-region-cursor case put the Bytes copies WITHIN a single
           compound arg (tuple<bytes,bytes> 108, bytes-scalar-bytes 109); this is the first with two SEPARATE
           compound args each copying a rope to `mem`. Pins that the cursor advances ACROSS args — arg 0's tuple
           Bytes element copies at the cursor (which advances), then arg 1's `option` Some payload copies to the
           ALREADY-ADVANCED cursor — so the two args' mem regions are DISJOINT and the flattened core slots are
           `(ptr0,len0, s, disc1,ptr1,len1)`. Host returns its scalar (assert 5 = len b\"hi\" (2) + len b\"xyz\"
           (3)). `live-objects 0`.")
  (wit-world
    (world w (import cadenza:platform/probe (member f (func (param p (tuple (list (u8)) (s64))) (param q (option (list (u8)))) (result (s64)))))))
  (input
    (do
      (effect probe (op f (-> (Tuple Bytes Int64) (Option Bytes) Int64)))
      (def (run) (host (probe) (probe.f #tuple(b"hi" 7) (Some b"xyz"))))
      (export run)))
  (call run)
  (host-responses (respond probe.f (: 5 Int64)))
  (host-calls (call cadenza:platform/probe.f))
  (output 5)
  (live-objects 0))

(case
  "a WIT flags PARAM member crosses as a bitset unpacked into a record-of-bools"
  (doc
    "SHAPE 113 — a TOP-LEVEL WIT `flags{read,write,execute}` PARAM of a typed export interface. WIT `flags`
           is a PRODUCT (each label an independent on/off), so the guest models it as a `record{read: bool,
           write: bool, execute: bool}` (operator ruling). It crosses as a SINGLE packed i32 bitset (bit i = the
           i-th DECLARED label), which `record_interface_export`'s flags-param arm unpacks into the guest record
           cell (`box-bool((bits>>bit)&1)` per field, matched to its label bit BY NAME). Guest `f` sums
           1*read + 2*write + 4*execute; the guest record BTreeMap-sorts to execute,read,write so its cell SLOTS
           differ from the WIT bit order — proving the by-name slot<->bit mapping. `(flags read execute)` -> 5,
           all three -> 7. The flags twin of the enum-param SHAPE 77 (one-of-N choice -> a subset).")
  (wit-world
    (world w (export iface (member f (func (param c (flags read write execute)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: c (Record (: read Bool) (: write Bool) (: execute Bool))))
        (+ (if (. c read) 1 0) (+ (if (. c write) 2 0) (if (. c execute) 4 0))))
      (export f)))
  (call f (: (flags read execute) (Record (: read Bool) (: write Bool) (: execute Bool))))
  (output (: 5 Int64))
  (call f (: (flags read write execute) (Record (: read Bool) (: write Bool) (: execute Bool))))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a WIT flags RESULT member packs a record-of-bools into the bitset"
  (doc
    "SHAPE 114 — a TOP-LEVEL WIT `flags{read,write,execute}` RESULT of a typed export interface: the RESULT
           twin of SHAPE 113. The guest returns a `record{read,write,execute}` of bools; `record_result_lower`'s
           flags arm (ResultLower::FlagsPack) packs it into the single i32 bitset — per field
           `get-bool(arr-get(handle, slot))` shifted into its WIT-decl bit, OR-ed together — then deep-drops the
           owned record result (`live-objects 0`). The guest returns read=true, write=false, execute=true, so the
           bitset renders `(flags read execute)`. The flags twin of the enum-result SHAPE 52/60.")
  (wit-world
    (world w (export iface (member g (func (result (flags read write execute)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (g) #record((= read true) (= write false) (= execute true)))
      (export g)))
  (call g)
  (output (flags read execute))
  (live-objects 0))

(case
  "a WIT flags field of a record PARAM unpacks into a nested record-of-bools"
  (doc
    "SHAPE 115 — a WIT `flags{read,write,execute}` FIELD of a record PARAM (`record{p: flags, n: u64}`).
           The nested position twin of the top-level flags param (SHAPE 113): `param_field_rebuild`'s flags arm
           (FieldRebuild::Flags) unpacks the field's single i32 bitset leaf into a nested record-of-bools cell,
           matched to the field's label bit BY NAME. Guest `f` sums 1*p.read + 2*p.write + 4*p.execute + n;
           f({p:(read,execute), n:10}) -> 1+4+10 = 15. The record cell + its nested flags cell reclaim
           (`live-objects 0`).")
  (wit-world
    (world
      w
      (export
        iface
        (member
          f
          (func
            (param m (record (= p (flags read write execute)) (= n (u64))))
            (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def
        (f (: m (Record (: p (Record (: read Bool) (: write Bool) (: execute Bool))) (: n (Int 64)))))
        (+ (if (. (. m p) read) 1 0) (+ (if (. (. m p) write) 2 0) (+ (if (. (. m p) execute) 4 0) (. m n)))))
      (export f)))
  (call f (: #record((= p (flags read execute)) (= n 10)) (Record (: p (Record (: read Bool) (: write Bool) (: execute Bool))) (: n (Int 64)))))
  (output (: 15 Int64))
  (live-objects 0))

(case
  "a WIT flags field of a record RESULT packs a nested record-of-bools into the bitset"
  (doc
    "SHAPE 116 — a WIT `flags{read,write,execute}` FIELD of a record RESULT (`record{p: flags, n: u64}`): the
           RESULT twin of SHAPE 115. The def returns a record whose `p` field is a nested record-of-bools;
           `canon_write_of`'s flags arm (CanonWrite::Flags) packs that nested cell into the field's canonical
           bitset (stored at the flags canonical width) as the parent record spills to memory. Guest returns
           p=(read=T,write=F,execute=T), n=9, so the result renders `#record((= p (flags read execute)) (= n
           9))`. The record result + nested flags cell reclaim (`live-objects 0`).")
  (wit-world
    (world
      w
      (export
        iface
        (member
          g
          (func (result (record (= p (flags read write execute)) (= n (u64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (g) #record((= p #record((= read true) (= write false) (= execute true))) (= n 9)))
      (export g)))
  (call g)
  (output #record((= p (flags read execute)) (= n 9)))
  (live-objects 0))

(case
  "a WIT option<flags> PARAM crosses as (disc, bitset) unpacked into an Option of record-of-bools"
  (doc
    "SHAPE 117 — a WIT `option<flags{read,write,execute}>` entry param. The Some payload is a flags, which
           flattens to a SINGLE i32 bitset, so the option crosses as `(disc: i32, bitset: i32)`. The guest-only
           option classifier would misread the record-of-bools Some payload as an N-leaf compound (CDZ0910
           signature mismatch — [I32,I32] boundary vs [I32,I32,I32,I32] rebuild); the WIT-aware `option_flags_arg`
           builds the correct Some arm (SumArmPayload::Flags, one i32 leaf -> record cell). Guest sums the set
           bits of the Some payload, or -1 for None: Some((read,execute)) -> 5, Some(()) -> 0, None -> -1. The
           flags twin of the option<list<scalar>> param (eop2).")
  (wit-world
    (world w (export iface (member f (func (param o (option (flags read write execute))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (f (: o (Option (Record (: read Bool) (: write Bool) (: execute Bool)))))
        (match o
          ((Option.Some p) (+ (if (. p read) 1 0) (+ (if (. p write) 2 0) (if (. p execute) 4 0))))
          ((Option.None) -1)))
      (export f)))
  (call f (: (Some (flags read execute)) (Option (Record (: read Bool) (: write Bool) (: execute Bool)))))
  (output (: 5 Int64))
  (call f (: None (Option (Record (: read Bool) (: write Bool) (: execute Bool)))))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a WIT list<flags> RESULT writes each element as a packed bitset"
  (doc
    "SHAPE 118 — a WIT `list<flags{read,write,execute}>` RESULT. The guest returns a list of record-of-bools;
           `canon_write_of` recurses WIT-aware into each element (CanonWrite::Flags), packing it into the
           element's canonical flags bitset at the list element stride. Guest returns
           [(read=T,write=F,execute=T), (read=F,write=T,execute=F)], rendering `#list((flags read execute)
           (flags write))`. The result direction works because the canonical WRITER threads the WIT element type;
           the list<flags> PARAM direction (whose element READER must unpack the packed bitset WIT-awarely) is
           SHAPE 119. `live-objects 0`.")
  (wit-world
    (world w (export iface (member g (func (result (list (flags read write execute))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (g)
        #list(#record((= read true) (= write false) (= execute true))
              #record((= read false) (= write true) (= execute false))))
      (export g)))
  (call g)
  (output #list((flags read execute) (flags write)))
  (live-objects 0))

(case
  "a WIT list<flags> PARAM unpacks each element's packed bitset into an Option of record-of-bools"
  (doc
    "SHAPE 119 — a WIT `list<flags{read,write,execute}>` entry PARAM: the READER twin of SHAPE 118. Each
           element is a PACKED bitset at the list-element stride (canonical flags width), while the guest models
           `flags` as `record{read,write,execute}` of bools. The guest-only list-element reader
           (`list_scalar_elem`) would misread the packed byte as a 3-field record — a wrong-layout miscompile —
           so the WIT-aware `list_flags_elem` builds each element's record-of-bools cell (`arr-alloc`/`box-bool`,
           bit i -> the i-th declared label, matched by name), the list twin of the top-level flags param
           (SHAPE 113). Borrow-only 0-leak lift (the wrapper drops the vec after the call). Guest perms(xs) =
           100*List.len(xs) + bitsum(xs[1]) — reading ELEMENT 1 (not 0) proves the per-element stride + bitset
           unpack: [(read,write),(execute)] -> 100*2 + execute(4) = 204; a broken stride or a bitset misread as a
           record would return a different value. `live-objects 0`.")
  (wit-world
    (world w (export iface (member perms (func (param xs (list (flags read write execute))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (def (perms (: xs (List (Record (: read Bool) (: write Bool) (: execute Bool)))))
        (+ (* 100 (List.len xs))
           (match (List.at xs 1)
             ((Option.Some p) (+ (if (. p read) 1 0) (+ (if (. p write) 2 0) (if (. p execute) 4 0))))
             ((Option.None) -1))))
      (export perms)))
  (call perms (: #list((flags read write) (flags execute)) (List (Record (: read Bool) (: write Bool) (: execute Bool)))))
  (output (: 204 Int64))
  (live-objects 0))

(case
  "a typed result<ok,err> EXPORT result over a CUSTOM (non-prelude) sum with CONCRETE payloads crosses"
  (doc
    "SHAPE 120 — a typed `result<s64,s64>` EXPORT result whose guest is a CUSTOM monomorphic sum
           `(type Res (Ok Int64) (Err Int64))`, NOT the generic prelude `Result a b`. Was a DECLINE:
           `canon_write_of`'s Result arm resolved each arm's payload type via `dr.params.position(payload-type-
           name)` — which only works for a GENERIC payload (a type PARAM instantiated via `args`); a concrete
           `Int64` payload has no matching param, so the arm returned None → the result reached the provider path
           → CDZ0900. Now the Result arm resolves each payload the SAME way the Variant arm does — via the
           variant's ctor occ + `payload_ty_at_instantiation` — so a concrete custom-sum payload crosses too.
           cl(x) = x>0 ? Res.Ok(x) : Res.Err(-x); x=5 -> Ok(5), x=-3 -> Err(3).")
  (wit-world
    (world w (export cadenza:demo/iface
      (member cl (func (param x (s64)) (result (result (s64) (s64))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Res (Ok Int64) (Err Int64))
      (def (cl (: x Int64)) (if (> x 0) (Res.Ok x) (Res.Err (- 0 x))))
      (export cl)))
  (call cl (: 5 Int64))
  (output (: (Ok 5) (Result Int64 Int64)))
  (call cl (: -3 Int64))
  (output (: (Err 3) (Result Int64 Int64)))
  (live-objects 0))

(case
  "a typed result<T> EXPORT result with a NULLARY err arm crosses"
  (doc
    "SHAPE 121 — a typed `result<s64>` EXPORT result: the ok arm carries an `s64`, the err arm is ABSENT
           (WIT `result<T>` = err unit), over a custom sum `(type Res (Ok Int64) (Err))` with a NULLARY Err
           ctor. Was a DECLINE: `canon_write_of`'s Result arm required BOTH arms to carry a payload. Now it maps
           each guest variant to its WIT arm BY NAME (`ok`->boundary disc 0 / `err`->1) with payload-presence
           agreement — a guest payload arm iff the WIT arm carries a payload — and writes a nullary arm as the
           disc ALONE (`VariantArm { payload: None }`), exactly like the general Variant arm; disc size + payload
           offset from `variant_disc_layout` over the two (possibly-absent) arm WITs. cl(x) = x>0 ? Res.Ok(x) :
           Res.Err; x=5 -> Ok(5), x=-3 -> Err (rendered `(Err unit)`, the boundary decodes the absent err arm as
           unit). The nullary-arm sibling of SHAPE 120.")
  (wit-world
    (world w (export cadenza:demo/iface
      (member cl (func (param x (s64)) (result (result (s64) (none))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Res (Ok Int64) (Err))
      (def (cl (: x Int64)) (if (> x 0) (Res.Ok x) (Res.Err)))
      (export cl)))
  (call cl (: 5 Int64))
  (output (: (Ok 5) (Result Int64 Unit)))
  (call cl (: -3 Int64))
  (output (: (Err unit) (Result Int64 Unit)))
  (live-objects 0))

(case
  "a variant case with ≥2 PAYLOADS (a multi-arg ctor) crosses as a WIT variant case with a tuple payload"
  (doc
    "SHAPE 122 — a guest sum ctor with TWO payloads (`(type V (Pair Int64 Int64) (One Int64))`), distinct from
           SHAPE 63's single-TUPLE-payload ctor (`(Two (Tuple Int64 Int64))`). A multi-payload ctor's payloads
           pack into the WIT variant case's single `tuple<…>` payload: `canon_write_of`'s Variant arm resolves
           the case payload via the ctor + `payload_ty_at_instantiation` (which yields the tuple of the ctor's
           payload types) and writes it through the Tuple arm at the canonical variant layout. cl(x) = x>0 ?
           V.Pair(x,x) : V.One(-x); x=5 -> pair(tuple 5 5), x=-3 -> one(3). Closes the gap-map
           `multi-payload variant case (≥2 payloads)` on the RESULT side.")
  (wit-world
    (world w (export cadenza:demo/iface
      (member cl (func (param x (s64)) (result (variant (pair (tuple (s64) (s64))) (one (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type V (Pair Int64 Int64) (One Int64))
      (def (cl (: x Int64)) (if (> x 0) (V.Pair x x) (V.One (- 0 x))))
      (export cl)))
  (call cl (: 5 Int64))
  (output (: (pair #tuple(5 5)) V))
  (call cl (: -3 Int64))
  (output (: (one 3) V))
  (live-objects 0))

(case
  "a host-op record ARGUMENT with an option<tuple-of-scalars> FIELD crosses the plain host-delegating envelope"
  (doc
    "SHAPE 123 — an `option<tuple<s64,s64>>` FIELD of a RECORD host-op argument (probe.push : func(record{opt:
           option<tuple<s64,s64>>, n: s64}) -> s64) on a pure-IMPORT custom wit-world with a plain top-level
           export. Extends the v-wit-boundary compound host-ARG support: before this an option<compound> field
           declined (field_boundary_abi admitted only option<scalar>/option<bytes>). Now field_boundary_abi
           recurses an option<tuple-of-scalars> payload and emit_record_arg_marshal SCRATCH-FLATTENS it — the
           canonical variant flatten `(disc:i32, flatten(tuple))` = disc + one core slot per element, marshalled
           into N element scratch slots (Some → per-element arr-get+unbox; None → the element's width zero) and
           pushed after the single-value `if` (LIR blocks are single-value, so the variable payload-slot count
           can't be pushed from the branch). A TUPLE payload is POSITIONAL, so no name-lex/WIT field-order
           ambiguity (an option<record> payload is a later slice). run() builds {opt: Some((10,20)), n:5},
           performs probe.push, returns the stub. A VALID component that runs is the pin: a mis-flattened
           option<tuple> arg (wrong core arity vs the declared `option<tuple<s64,s64>>` import type) fails
           component validation (CDZ0910). The option<compound>-field twin of SHAPE 99 (option<scalar> field).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= opt (option (tuple (s64) (s64)))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: opt (Option (Tuple Int64 Int64))) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= opt (Some #tuple(10 20))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 99 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 99)
  (live-objects 0))

(case
  "a host-op record ARGUMENT with an option<record-of-scalars> FIELD (WIT field order ≠ name-lex) crosses"
  (doc
    "SHAPE 124 — an `option<record{lo,hi}>` FIELD of a RECORD host-op argument (probe.push : func(record{opt:
           option<record{lo: s64, hi: s64}>, n: s64}) -> s64), the record twin of SHAPE 123's option<tuple>.
           A record payload is name-lex in the value-heap cell but DECLARATION-ordered in the host WIT, so
           `reorder_record_fields_to_wit` now recurses into the `Option(Record)` payload (reordering the inner
           record's abi to the option payload WIT record order), and `emit_record_arg_marshal`'s
           option<record-of-scalars> arm reads each payload WIT field FROM ITS NAME-LEX cell index, scratch-
           flattening `(disc, flatten(record))` = disc + one core slot per field in WIT order (Some →
           arr-get+unbox per field; None → the field's width zero; push after the single-value `if`). The WIT
           declares `lo, hi` but name-lex is `hi, lo` (h < l), so a marshal that read/declared in the wrong
           order would mis-flatten. run() builds {opt: Some({lo:10, hi:20}), n:5}, performs probe.push, returns
           the stub. A VALID component that runs is the pin (a mis-declared/mis-ordered option<record> arg fails
           component validation, CDZ0910). The option<compound>-field family sibling of SHAPE 123.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= opt (option (record (= lo (s64)) (= hi (s64))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: opt (Option (Record (: lo Int64) (: hi Int64)))) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= opt (Some #record((= lo 10) (= hi 20)))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 77 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 77)
  (live-objects 0))

(case
  "a host-op record ARG with an option<tuple> FIELD = None exercises the zero-fill marshal branch"
  (doc
    "SHAPE 125 — the NONE arm of the option<compound> host-arg-field marshal (SHAPE 123/124 exercised only
           Some). An `option<tuple<s64,s64>>` field that is None flattens to `(disc=0, 0, 0)` — the marshal's
           else branch zero-fills every payload element scratch slot (the element's width zero) and pushes
           disc=0. run() builds {opt: None, n:5}, performs probe.push, returns the stub. A VALID component that
           runs is the pin: a broken zero-fill (wrong slot count/width, or reading an absent payload) would
           trap or mis-flatten. Complements SHAPE 123 (option<tuple> Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= opt (option (tuple (s64) (s64)))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: opt (Option (Tuple Int64 Int64))) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= opt None) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a host-op record ARG with an option<record-with-a-Bytes-field> FIELD marshals the byte-leaf payload"
  (doc
    "SHAPE 126 — an `option<record{data: list<u8>, n: s64}>` FIELD of a RECORD host-op argument (probe.push :
           func(record{opt: option<record{data: Bytes, n: s64}>, k: s64}) -> s64), extending SHAPE 124's
           option<record-of-scalars> to a payload record whose leaf is a BYTES field. The option<compound>
           marshal recurses into the payload record via the shared record-field marshal, which flattens the
           Bytes field to (ptr, len) — TWO core slots — after copying its rope into the shared linear memory at
           the reserved scratch cursor; the scalar `n` field flattens to one slot. So the whole field flattens
           to (disc, ptr, len, n) = FOUR core slots on Some. The pin this case guards is byte-leaf slot COUNT:
           the option marshal must reserve/capture exactly as many scratch slots as the payload marshal pushes
           (Bytes=2, not 1 — `valtype_of(Bytes)` is a handle, so a scalar-first slot count would leave a value
           on the stack and fail component validation, CDZ0910). run() builds {opt: Some({data: b\"hi\", n: 9}),
           k: 5}, performs probe.push, returns the stub. Complements SHAPE 124 (option<record-of-scalars>) and
           SHAPE 125 (None).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= opt (option (record (= data (list (u8))) (= n (s64))))) (= k (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: opt (Option (Record (: data Bytes) (: n Int64)))) (: k Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= opt (Some #record((= data b"hi") (= n 9)))) (= k 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<tuple-of-scalars> host-op arg (not nested in a record) marshals its Some payload"
  (doc
    "SHAPE 127 — an `option<tuple<s64,s64>>` as the BARE top-level param of a host op (probe.push :
           func(option<tuple<s64,s64>>) -> s64), the register-twin entry point of the option<tuple> record-FIELD
           flatten (SHAPE 123). The component type + serialize flatten are already general over the payload abi
           (built from the declared WIT `(option (tuple s64 s64))` / flatten_record_field_abi); the guest marshal
           is `emit_option_reg_flatten`'s tuple branch: on Some it flattens the payload tuple POSITIONALLY via
           `emit_tuple_reg_flatten` (one core slot per element), captured into N scratch slots and pushed as
           `(disc=1, elem0, elem1)` after the single-value `if`. run() builds Some((7,8)), performs probe.push,
           returns the stub 55. A VALID component that runs is the pin: a broken flatten (wrong slot count/order,
           or reading a None payload) traps or mis-marshals at component validation.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (tuple (s64) (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Tuple Int64 Int64)) Int64)))
      (def (run) (host (probe) (probe.push (Some #tuple(7 8)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<tuple-of-scalars> host-op arg = None exercises the zero-fill flatten branch"
  (doc
    "SHAPE 128 — the NONE arm of the top-level option<tuple> arg marshal (SHAPE 127 exercised only Some). An
           `option<tuple<s64,s64>>` bare arg that is None flattens to `(disc=0, 0, 0)` — `emit_option_reg_flatten`'s
           tuple branch else-arm zero-fills every payload element scratch slot at its element width and pushes
           disc=0. run() builds None, performs probe.push, returns the stub 42. A broken zero-fill (wrong slot
           count/width, or reading an absent payload) would trap or mis-flatten. Complements SHAPE 127 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (tuple (s64) (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Tuple Int64 Int64)) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL option<tuple<Bytes,s64>> host-op arg marshals its Some payload (byte-leaf rope copy)"
  (doc
    "SHAPE 129 — a top-level `option<tuple<list<u8>, s64>>` bare host-op arg whose payload tuple carries a
           BYTES element (probe.push : func(option<tuple<list<u8>,s64>>) -> s64), extending SHAPE 127's
           option<tuple-of-scalars> to a byte-leaf tuple element. `emit_option_reg_flatten`'s tuple branch
           recurses `emit_tuple_reg_flatten`, which copies the Bytes element's rope into shared linear memory at
           the reserved scratch cursor and pushes it as `(ptr, len)` — TWO core slots — while the `s64` element
           pushes one. So the payload flattens POSITIONALLY to `(disc, ptr, len, s64)` = FOUR slots on Some. The
           pin is the byte-leaf slot COUNT: the option branch must expand a Bytes element to 2 scratch slots
           (`valtype_of(Bytes)` is a handle = `Some(I32)`, so a scalar-first count leaves a value on the stack
           and fails component validation, CDZ0910) AND the emit.rs cursor pre-scan must reserve the cursor for
           an `option<tuple-with-bytes>` arg. run() builds Some((b\"hi\", 9)), performs probe.push, returns the
           stub 55. Complements SHAPE 127/128 (tuple-of-scalars Some/None).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (tuple (list (u8)) (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Tuple Bytes Int64)) Int64)))
      (def (run) (host (probe) (probe.push (Some #tuple(b"hi" 9)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record-of-scalars> host-op arg marshals its Some payload in WIT field order"
  (doc
    "SHAPE 130 — a top-level `option<record{lo: s64, hi: bool}>` bare host-op arg (probe.push :
           func(option<record{lo,hi}>) -> s64), the register-twin entry point of the option<record> record-FIELD
           flatten (SHAPE 124). Unlike a tuple (positional), a record's value-heap cells are NAME-LEX ordered
           ({hi, lo}) but the host WIT declares {lo, hi}, so `emit_option_reg_flatten`'s record branch recurses
           `emit_record_arg_marshal` which reads each WIT field from its name-lex cell and PUSHES in WIT order,
           and the `option`'s payload record abi is REORDERED to WIT order (`reorder_record_fields_to_wit`) so
           the emitted `(option (record …))` component type + core flatten agree with the marshal. The DISTINCT
           widths (lo: s64 = i64 slot, hi: bool = i32 slot) make the reorder LOAD-BEARING: a name-lex-order
           flatten would emit core `(disc, hi:i32, lo:i64)` against the WIT-order component param `(disc, lo:i64,
           hi:i32)` — a signature mismatch the runtime rejects at instantiation. run() builds Some({lo:3,
           hi:true}), performs probe.push, returns the stub 55. Complements SHAPE 124 (option<record> field).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= lo (s64)) (= hi (bool))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: lo Int64) (: hi Bool))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= lo 3) (= hi true))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record-of-scalars> host-op arg = None exercises the zero-fill flatten branch"
  (doc
    "SHAPE 131 — the NONE arm of the top-level option<record> arg marshal (SHAPE 130 exercised only Some). An
           `option<record{lo: s64, hi: bool}>` bare arg that is None flattens to `(disc=0, 0, 0)` —
           `emit_option_reg_flatten`'s record branch else-arm zero-fills every payload field scratch slot at its
           field width (i64 lo = 0i64, i32 hi = 0) and pushes disc=0. run() builds None, performs probe.push,
           returns the stub 42. Complements SHAPE 130 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= lo (s64)) (= hi (bool))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: lo Int64) (: hi Bool))) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record-with-a-Bytes-field> host-op arg marshals its Some payload (byte-leaf + reorder)"
  (doc
    "SHAPE 132 — a top-level `option<record{n: s64, data: list<u8>}>` bare host-op arg, extending SHAPE 130's
           option<record-of-scalars> to a record whose leaf is a BYTES field. `emit_option_reg_flatten`'s record
           branch recurses `emit_record_arg_marshal`, which reads each WIT field from its name-lex cell, copies
           the Bytes field's rope into shared mem at the reserved cursor, and pushes it as `(ptr, len)` — TWO
           slots — while `n` pushes one. The record's value-heap cells are NAME-LEX ordered ({data, n}) but the
           host WIT declares {n, data}, so BOTH the reorder AND the byte-leaf slot count are load-bearing: the
           WIT-order flatten is `(disc, n:i64, ptr:i32, len:i32)`, whereas a name-lex order OR a scalar-count of
           the Bytes field would emit a different core signature that the runtime rejects at instantiation. Pins
           the two SHAPE-129/130 lessons combined: the payload record abi is reordered to WIT order
           (`reorder_record_fields_to_wit`) AND a Bytes field expands to 2 scratch slots (`valtype_of(Bytes)` is
           `Some(I32)`) AND the emit.rs cursor pre-scan reserves for an `option<record-with-bytes>` arg. run()
           builds Some({n:9, data:b\"hi\"}), performs probe.push, returns the stub 55.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= data (list (u8)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: data Bytes))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= n 9) (= data b"hi"))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record-with-a-Bytes-field> host-op arg = None zero-fills every payload slot"
  (doc
    "SHAPE 133 — the NONE arm of the top-level option<record-with-a-Bytes-field> arg marshal (SHAPE 132
           exercised only Some). An `option<record{n: s64, data: list<u8>}>` bare arg that is None flattens to
           `(disc=0, 0, 0, 0)` — the record branch else-arm zero-fills the `n` slot (i64) AND both `(ptr,len)`
           slots of the absent Bytes field (a None option never reads its payload rope, so no cursor copy) and
           pushes disc=0. run() builds None, performs probe.push, returns the stub 42. Complements SHAPE 132.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= data (list (u8)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: data Bytes))) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a NESTED tuple element flattens the inner tuple positionally inline"
  (doc
    "SHAPE 134 — a top-level `tuple<tuple<s64, bool>, s64>` bare host-op arg, extending the tuple-arg marshal
           (scalar/Bytes elements) to a NESTED tuple element. `emit_tuple_reg_flatten` reads the inner tuple's
           handle (`arr-get`, borrows the outer tuple) and RECURSES — the inner elements flatten POSITIONALLY
           inline onto the operand stack, matching serialize's `RecordFieldAbi::Tuple` recursion + the component
           `tuple<tuple<…>, …>` type. So the whole arg flattens to `(inner0: i64, inner1: i32, outer1: i64)` = 3
           core slots, no discriminant (a tuple is not a variant). The MIXED inner widths (s64 = i64 slot, bool =
           i32 slot) make the flatten load-bearing: a non-recursed inner (treating the nested tuple as one slot)
           or a wrong element order would emit a core signature the runtime rejects at instantiation. run() builds
           ((7, true), 9), performs probe.push, returns the stub 55. The tuple analogue of the nested-record
           record FIELD (`emit_record_arg_marshal`'s nested-record recursion).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (tuple (s64) (bool)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Tuple Int64 Bool) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#tuple(7 true) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a RECORD-of-scalars element flattens the record inline in WIT field order"
  (doc
    "SHAPE 135 — a top-level `tuple<record{lo: s64, hi: bool}, s64>` bare host-op arg, extending the tuple-arg
           marshal (scalar/Bytes/nested-tuple elements, SHAPE 134) to a RECORD element. `emit_tuple_reg_flatten`
           reads the record element's handle (`arr-get`, borrows the outer tuple) and recurses
           `emit_record_arg_marshal`, which reads each WIT field from its name-lex cell and pushes in the host
           WIT DECLARATION order — so `tuple_wit` (the tuple's declared WIT type, threaded from the caller) gives
           element `i`'s WIT record type. The record's value-heap cells are NAME-LEX ordered ({hi, lo}) but the
           WIT declares {lo, hi}, so the classifier REORDERS the element's record abi to WIT order
           (`reorder_record_fields_to_wit`) — matching the marshal + the component `tuple<record<…>, …>` type.
           The DISTINCT widths (lo: s64 = i64, hi: bool = i32) make the reorder LOAD-BEARING: a name-lex-order
           flatten would emit `(hi:i32, lo:i64, outer1:i64)` against the WIT-order component param `(lo:i64,
           hi:i32, outer1:i64)`, rejected at instantiation. run() builds ({lo:7, hi:true}, 9), performs
           probe.push, returns the stub 55. The tuple analogue of the nested-record record FIELD.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (record (= lo (s64)) (= hi (bool))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Record (: lo Int64) (: hi Bool)) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#record((= lo 7) (= hi true)) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a NESTED tuple element that carries a Bytes LEAF flattens the inner rope-copy inline"
  (doc
    "SHAPE 139 (v-wit-boundary) — a top-level `tuple<tuple<list<u8>, s64>, s64>` bare host-op arg, extending
           the nested-tuple element marshal (SHAPE 134, all-scalar inner) to an inner tuple that carries a
           BYTES LEAF. `emit_tuple_reg_flatten` reads the inner tuple's handle (`arr-get`, borrows the outer
           tuple) and RECURSES — the recursion routes the inner element 0 through its own Bytes branch, copying
           the rope into `mem` at the SHARED scratch `cursor` and pushing `(ptr, len)`, then pushes inner
           element 1 (s64) inline; the outer element 1 (s64) pushes last. So the whole arg flattens to
           `(ptr0:i32, len0:i32, inner1:i64, outer1:i64)` = 4 core slots, no discriminant. The Bytes LEAF makes
           the recursion load-bearing: the inner tuple is not a flat scalar run, so the shared cursor must be
           reserved by the pre-scan (`tuple_has_bytes_element` recurses into nested tuple elements) and threaded
           through the recursion — a non-recursed inner (treating the nested tuple as one slot) or a missing
           cursor reservation would emit a wrong core signature / panic. run() builds ((b\"hi\", 7), 9), performs
           probe.push, returns the stub 55. Matches serialize's `RecordFieldAbi::Tuple` recursion + the component
           `tuple<tuple<list<u8>, s64>, s64>` type. Completes the tuple-arg family (scalar/Bytes/nested-tuple/
           record element, and now a Bytes leaf INSIDE a nested tuple element).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (tuple (list (u8)) (s64)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Tuple Bytes Int64) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#tuple(b"hi" 7) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a RECORD element that carries a Bytes FIELD copies the field rope inline"
  (doc
    "SHAPE 140 (v-wit-boundary) — a top-level `tuple<record{n: s64, data: list<u8>}, s64>` bare host-op arg,
           extending the tuple record-element marshal (SHAPE 135, all-scalar fields) to a record element that
           carries a BYTES FIELD. `emit_tuple_reg_flatten` reads the record element's handle (`arr-get`, borrows
           the outer tuple) and recurses `emit_record_arg_marshal`, which pushes each field in the host WIT
           DECLARATION order — a scalar field inline, and the `data` Bytes field by copying its rope into `mem`
           at the SHARED scratch `cursor` and pushing `(ptr, len)` (2 slots, the same rope→mem copy a Bytes ARG /
           a Bytes record FIELD / a Some option<bytes> does). So the whole arg flattens to `(n: i64, ptr: i32,
           len: i32, outer1: i64)` = 4 core slots. The classifier maps the record's `data` field to
           `RecordFieldAbi::Bytes` (was scalar-only, which would have declined the whole arg), and the cursor
           pre-scan (`tuple_has_bytes_element`) recurses into the record element's fields to reserve the cursor.
           run() builds ({n: 7, data: b\"hi\"}, 9), performs probe.push, returns the stub 55. Matches serialize's
           `RecordFieldAbi::Record` recursion + the component `tuple<record<…, list<u8>>, s64>` type. Together
           with SHAPE 139 (Bytes leaf in a nested tuple element) this closes the Bytes-leaf-inside-a-compound-
           element gap of the tuple-arg family.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (record (= n (s64)) (= data (list (u8)))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Record (: n Int64) (: data Bytes)) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#record((= n 7) (= data b"hi")) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a RECORD element that has a NESTED TUPLE field flattens the field inline"
  (doc
    "SHAPE 141 (v-wit-boundary) — a top-level `tuple<record{n: s64, pt: tuple<s64, bool>}, s64>` bare host-op
           arg, extending the tuple record-element marshal (SHAPE 135 all-scalar fields, SHAPE 140 a Bytes
           field) to a record element that carries a NESTED TUPLE field. `emit_tuple_reg_flatten` reads the
           record element's handle (`arr-get`) and recurses `emit_record_arg_marshal`, whose tuple-field arm
           reads the `pt` tuple handle and flattens its elements POSITIONALLY inline — a scalar element pushes
           one slot. So the whole arg flattens to `(n: i64, pt0: i64, pt1: i32, outer1: i64)` = 4 core slots.
           This is the tuple-element analogue of the DIRECT record-arg nested-tuple field (which already crosses
           via `field_boundary_abi`): the classifier now maps a record element's tuple-of-scalars field to
           `RecordFieldAbi::Tuple` (was scalar/`Bytes`-only, which would have declined the whole arg), matching
           the marshal + the component `tuple<record<…, tuple<s64, bool>>, s64>` type. The MIXED nested widths
           (pt0: s64 = i64, pt1: bool = i32) make the inline flatten load-bearing. run() builds ({n: 7, pt: (5,
           true)}, 9), performs probe.push, returns the stub 55. Extends the tuple record-element family to a
           nested-compound field.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (record (= n (s64)) (= pt (tuple (s64) (bool)))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Record (: n Int64) (: pt (Tuple Int64 Bool))) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#record((= n 7) (= pt #tuple(5 true))) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a RECORD element that has a NESTED RECORD field flattens the sub-record inline"
  (doc
    "SHAPE 142 (v-wit-boundary) — a top-level `tuple<record{n: s64, inner: record{a: s64, b: bool}}, s64>`
           bare host-op arg: a record element of a tuple arg carrying a NESTED RECORD field. This tick the
           tuple-arg classifier's record-element field build was GENERALIZED to the shared recursive
           `field_boundary_abi` (the SAME builder the DIRECT record arg + `emit_record_arg_marshal` use), so a
           record element now crosses with ANY field `field_boundary_abi` accepts — here a nested record field,
           whose sub-fields flatten inline in the sub-record's WIT declaration order (`emit_record_arg_marshal`
           recurses on a nested-record field; `reorder_record_fields_to_wit` recurses into the nested record's
           abi). The whole arg flattens to `(n: i64, a: i64, b: i32, outer1: i64)` = 4 core slots. run() builds
           ({n: 7, inner: {a: 5, b: true}}, 9), performs probe.push, returns the stub 55. The tuple-element
           analogue of the direct record-arg nested-record field.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (record (= n (s64)) (= inner (record (= a (s64)) (= b (bool))))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Record (: n Int64) (: inner (Record (: a Int64) (: b Bool)))) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#record((= n 7) (= inner #record((= a 5) (= b true)))) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a RECORD element that has a list<s64> field marshals the list into shared mem"
  (doc
    "SHAPE 143 (v-wit-boundary) — a top-level `tuple<record{n: s64, xs: list<s64>}, s64>` bare host-op arg: a
           record element of a tuple arg carrying a `list<s64>` FIELD. Same classifier generalization as SHAPE
           142 (`field_boundary_abi` admits a `list<T>` field), plus the tuple-arg cursor pre-scan was widened
           (`tuple_arg_needs_cursor`) to reserve the running scratch cursor for a record element whose field
           needs `mem` (a list marshals its backing array + elements into shared memory at the cursor) — a
           `Bytes`-only detection (`tuple_has_bytes_element`) would have MISSED the list field and panicked the
           marshal's `cursor.expect(...)`. `emit_record_arg_marshal`'s list-field arm runs `emit_list_arg_marshal`
           and pushes `(ptr, count)`; the whole arg flattens to `(n: i64, xs_ptr: i32, xs_count: i32, outer1:
           i64)` = 4 core slots. run() builds ({n: 7, xs: [5, 6]}, 9), performs probe.push, returns the stub 55.
           Pins the cursor-reservation widening the record-element generalization required.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (record (= n (s64)) (= xs (list (s64)))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Record (: n Int64) (: xs (List Int64))) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#record((= n 7) (= xs #list(5 6))) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a list<s64> ELEMENT marshals the list into shared mem inline"
  (doc
    "SHAPE 144 (v-wit-boundary) — a top-level `tuple<list<s64>, s64>` bare host-op arg: a `list<T>` as a tuple
           ELEMENT (not a record field). `emit_tuple_reg_flatten` gained a list-element arm symmetric to the
           record marshal's list-FIELD arm: it `arr-get`s the element's List handle and runs `emit_list_arg_
           marshal`, which writes the list's backing array + elements into shared `mem` at the running cursor and
           leaves `(ptr, count)` — the SAME 2 core slots a `list<T>` ARG / a record list FIELD lowers to. So the
           whole arg flattens to `(xs_ptr: i32, xs_count: i32, outer1: i64)` = 3 core slots. `tuple_arg_crosses`
           now admits a list element whose element crosses (`field_boundary_abi`), and `tuple_arg_needs_cursor`
           already reserves the scratch cursor for a list leaf. run() builds ([5, 6], 9), performs probe.push,
           returns the stub 55. Completes the tuple ELEMENT set to scalar / Bytes / nested-tuple / record / list
           (an option/variant element remains a later increment — `emit_tuple_reg_flatten` has no such arm).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (list (s64)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (List Int64) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(#list(5 6) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with an option<s64> ELEMENT flattens (disc, payload) inline — Some"
  (doc
    "SHAPE 145 (v-wit-boundary) — a top-level `tuple<option<s64>, s64>` bare host-op arg: an `option<T>` as a
           tuple ELEMENT. `emit_tuple_reg_flatten` gained an option-element arm that `arr-get`s the element's
           Option handle and runs `emit_option_reg_flatten` (the SAME register twin a top-level option ARG /
           an option record FIELD uses), pushing `(disc:i32, payload…)` INLINE into the tuple's positional
           flatten — so the whole arg flattens to `(opt_disc: i32, opt_payload: i64, outer1: i64)` = 3 core
           slots. `tuple_arg_crosses` now admits an option element whose payload crosses (`option_arg_crosses`,
           shared with the top-level option-arg gate). This is the Some arm: guest disc → WIT some=1, payload
           unboxed. run() builds ((Some 5), 9), performs probe.push, returns the stub 55. The None twin is SHAPE
           146.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (option (s64)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Option Int64) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Some 5) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with an option<s64> ELEMENT flattens (disc, payload) inline — None"
  (doc
    "SHAPE 146 (v-wit-boundary) — the NONE arm of SHAPE 145 (`tuple<option<s64>, s64>` tuple ELEMENT). The
           option-element flatten in `emit_tuple_reg_flatten` pushes `(disc=0, payload-width zero)` for None (a
           none `option` never reads its payload), so the arg flattens to `(0:i32, 0:i64, outer1:i64)`. run()
           builds (None, 9), performs probe.push, returns the stub 42. Complements SHAPE 145 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (option (s64)) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Option Int64) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple(None 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple with a scalar-payload VARIANT element flattens (disc, payload-join) inline"
  (doc
    "SHAPE 147 (v-wit-boundary) — a top-level `tuple<variant{go, stop(s64)}, s64>` bare host-op arg: a
           scalar-payload `variant` as a tuple ELEMENT. `emit_tuple_reg_flatten` gained a variant-element arm
           that `arr-get`s the element's variant handle and runs `emit_variant_reg_flatten` (the SAME register
           twin a bare-variant ARG (SHAPE 93) / a variant record FIELD / a list-element variant uses), pushing
           `(disc:i32, payload-join)` INLINE into the tuple's positional flatten. The guest `sum-disc` IS the
           component discriminant (decl order); the payload slot is the canonical JOIN valtype so the mixed
           nullary/`s64` cases read back correctly. So the whole arg flattens to `(disc: i32, payload: i64,
           outer1: i64)` = 3 core slots (no cursor — scalar payloads only). `tuple_arg_crosses` now admits a
           variant element via `variant_scalar_payload_cases` (checked after the option branch, which
           `variant_scalar_payload_cases` excludes). run() builds ((Stop 7), 9), performs probe.push, returns
           the stub 55. Completes the tuple ELEMENT set to scalar / Bytes / nested-tuple / record / list /
           option / variant — every WIT `tuple` element shape the marshals handle now crosses at the tuple
           ARG position.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (variant (go) (stop (s64))) (s64))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Tuple Sig Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Sig.Stop 7) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a payloadless enum as a FIELD of a typed record EXPORT result crosses BY NAME"
  (doc
    "SHAPE 136 — a payloadless `enum` as a FIELD of a typed `record` EXPORT result under a declared world:
           f(x:s64) -> record{c: enum{red,green,blue}, n: s64}, guest returns {c: Red|Green, n: x}. The enum
           field crosses BY CASE NAME (order matches here: guest Red -> `red`, Green -> `green`), so f(0) ->
           {c: red, n: 0} and f(5) -> {c: green, n: 5} — the nested/spilled twin of the top-level enum result
           (SHAPE 60/64), placing the case by name like a record field (SHAPE 20). `canon_write_of`'s
           payloadless-enum arm lowers via `CanonWrite::EnumDisc`, which UNBOXES the boxed enum value in the
           value-heap cell (`get-int` + wrap — a spilled enum field is BOXED like any scalar field, unlike a
           top-level enum result whose def returns the raw i32 disc) and stores the WIT disc. (History: this
           originally MISCOMPILED — the arm stored the box HANDLE raw with no unbox → runtime `discriminant N out
           of range`, N = the box handle — then briefly DECLINED CDZ0900 decline-don't-miscompile; the unbox fix
           closed it.) The REORDER twin is SHAPE 137, the `list<enum>` twin SHAPE 138. The TOP-LEVEL enum
           result/param are a different lowering path (SHAPE 60/64/67/68).")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (record (= c (enum red green blue)) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: x Int64)) #record((= c (if (= x 0) Color.Red Color.Green)) (= n x)))
      (export f)))
  (call f (: 0 Int64))
  (output #record((= c (red unit)) (= n 0)))
  (call f (: 5 Int64))
  (output #record((= c (green unit)) (= n 5))))

(case
  "an enum FIELD of a typed record result whose GUEST case order MISMATCHES the WIT remaps by NAME"
  (doc
    "SHAPE 137 — the REORDER twin of SHAPE 136 (the canon-write nested analogue of the top-level enum-result
           reorder SHAPE 64). Guest `(type Color (Red)(Green)(Blue))` in a record result field `c` under a world
           declaring the field `(enum green red blue)` [red/green REVERSED]. The enum field crosses BY CASE NAME:
           `canon_write_of`'s enum arm builds `guest_to_wit[guest_disc] = wit_disc` (the WIT index of the guest
           case's kebab name — here guest Red(0)->WIT `red`(1), Green(1)->WIT `green`(0), Blue(2)->`blue`(2)),
           and `CanonWrite::EnumDisc` UNBOXES the boxed disc then remaps it via a `select`-fold before the store.
           So f(0)->Red crosses as WIT `red` and f(5)->Green as WIT `green` — IDENTICAL semantics to the
           order-MATCHING SHAPE 136, the enum being the degenerate name-keyed variant (§1 nominal identity). A
           broken remap (raw guest disc, or a mis-permuted fold) would render the wrong case. The `list<enum>`
           twin is SHAPE 138.")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (record (= c (enum green red blue)) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: x Int64)) #record((= c (if (= x 0) Color.Red Color.Green)) (= n x)))
      (export f)))
  (call f (: 0 Int64))
  (output #record((= c (red unit)) (= n 0)))
  (call f (: 5 Int64))
  (output #record((= c (green unit)) (= n 5))))

(case
  "a typed list<enum> EXPORT result writes each element's disc BY NAME (canon_write EnumDisc as a list element)"
  (doc
    "SHAPE 138 — the LIST-ELEMENT twin of SHAPE 136/137: a typed `list<enum{red,green,blue}>` EXPORT result.
           `canon_write_of`'s List arm composes with its payloadless-enum arm (`CanonWrite::List{ elem =
           EnumDisc }`), writing each element's disc at the canonical `enum` element stride — UNBOXED (`get-int`
           + wrap, each element is a boxed value-heap cell) and, here, order-matching so the guest disc IS the
           WIT index. getColors(x) = [Red, Green]; x=0 -> [red, green]. A broken element write (box handle raw,
           or missing per-element unbox) traps `discriminant out of range` — the exact miscompile this closes.")
  (wit-world
    (world
      w
      (export cadenza:demo/iface (member f (func (param x (s64)) (result (list (enum red green blue))))))))
  (component-name "cadenza:demo/iface")
  (input
    (do
      (type Color (Red) (Green) (Blue))
      (def (f (: _x Int64)) #list(Color.Red Color.Green))
      (export f)))
  (call f (: 0 Int64))
  (output #list((red unit) (green unit))))

(case
  "a TOP-LEVEL option<record{n: s64, xs: list<s64>}> host-op arg marshals the payload record (list field into mem) — Some"
  (doc
    "SHAPE 148 (v-wit-boundary) — a top-level `option<record{n: s64, xs: list<s64>}>` bare host-op arg: the
           option<record> arm now accepts a record payload whose field is itself compound (a `list<s64>`), the
           record twin of SHAPE 143's tuple-record-list-field at the OPTION-arg position. Three fixes compose:
           (1) `option_arg_crosses` admits a record payload iff every field crosses (`is_boundary_record` /
           `field_boundary_abi`) — the SAME admit set the direct record ARG uses — so the classifier, the
           representability gate, the emit dispatch, and used-ops all widen in lockstep; (2) `emit_option_reg_
           flatten`'s record branch derives its capture `slot_vts` from each field's flattened boundary ABI
           (`field_boundary_abi` -> `flatten_record_field_abi`), so a `list<s64>` field is counted as its 2
           `(ptr, count)` core slots — a `valtype_of`-based count treated the list handle as one i32 slot and
           left a value on the operand stack (CDZ0910); (3) the option-arg cursor pre-scan reserves the running
           scratch cursor for an `option<record-with-a-list-field>` (the list marshals its backing array into
           shared `mem`), and the cursor-slot reservation now bumps the declared-locals top past the cursor slot
           (a cursor-only reservation formerly excluded it, panicking `coalesce_func`'s remap). On Some the
           marshal flattens `(disc=1, n: i64, xs_ptr: i32, xs_count: i32)` = 4 core slots (WIT declaration order),
           `n` riding the operand stack beneath the list field's Block/Loop (an empty-type block is net-neutral).
           run() builds Some({n: 7, xs: [5, 6]}), performs probe.push, returns the stub 55. A VALID component
           that runs is the pin (a mis-counted/mis-ordered option<record> arg fails component validation).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= xs (list (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: xs (List Int64)))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= n 7) (= xs #list(5 6)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record{n: s64, xs: list<s64>}> host-op arg = None zero-fills the payload slots"
  (doc
    "SHAPE 149 (v-wit-boundary) — the NONE arm of SHAPE 148. An `option<record-with-a-list-field>` that is None
           flattens to `(disc=0, n=0: i64, xs_ptr=0: i32, xs_count=0: i32)` — the marshal's else branch zero-fills
           every payload field scratch slot at its OWN width (the `n` field's i64, the list field's two i32s), a
           none `option` never reading its payload. The per-slot zero width is the pin: a broken zero-fill (wrong
           slot count/width, or reading an absent payload) traps or mis-flattens. run() builds None, performs
           probe.push, returns the stub 42. Complements SHAPE 148 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= xs (list (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: xs (List Int64)))) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record{n: s64, inner: record{a, b}}> host-op arg marshals a NESTED record field — Some"
  (doc
    "SHAPE 150 (v-wit-boundary) — a top-level `option<record>` arg whose payload record has a NESTED RECORD field.
           The option<record> arm admits ANY payload record `is_boundary_record` accepts (SHAPE 148 generalized it
           to `field_boundary_abi`), so a nested-record field crosses at the option position exactly where it
           crosses at the bare-record position (`emit_record_arg_marshal` recurses the nested record inline). The
           capture `slot_vts` come from each field's flattened boundary ABI, so the nested record's fields flatten
           INLINE into the parent run: the arg flattens to `(disc=1, n: i64, a: i64, b: i64)` = 4 core slots (a
           nested record does NOT spill — its fields join the parent's flattened run). run() builds Some({n: 7,
           inner: {a: 1, b: 2}}), performs probe.push, returns the stub 55. Pins the `field_boundary_abi` recursion
           at the option-arg position for a nested-record field.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= inner (record (= a (s64)) (= b (s64))))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: inner (Record (: a Int64) (: b Int64))))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= n 7) (= inner #record((= a 1) (= b 2))))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record{n: s64, o: option<s64>}> host-op arg marshals an OPTION field — Some"
  (doc
    "SHAPE 151 (v-wit-boundary) — a top-level `option<record>` arg whose payload record has an `option<s64>` FIELD.
           The `field_boundary_abi` recursion admits an `option<scalar>` field (its `RecordFieldAbi::Option`), so an
           option field crosses inside the option payload record: `emit_record_arg_marshal`'s option-field arm reads
           the field's Option cell, flattens `(field_disc: i32, payload: i64)` inline. The whole arg flattens to
           `(disc=1, n: i64, o_disc: i32, o_payload: i64)` = 4 core slots — the OUTER option's disc, the scalar n,
           then the INNER option field's own `(disc, payload)`. run() builds Some({n: 7, o: Some(5)}), performs
           probe.push, returns the stub 55. Pins the nested-option field recursion at the option-arg position.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= o (option (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: o (Option Int64)))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= n 7) (= o (Some 5)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL list<option<record{a, b}>> host-op arg writes each option element in place (compound payload) — Some + None"
  (doc
    "SHAPE 152 (v-wit-boundary) — a top-level `list<option<record{a: s64, b: s64}>>` bare host-op arg: an option
           element whose payload is a COMPOUND (record). `emit_option_to_mem` gained a compound-payload branch —
           on Some it writes the payload record IN PLACE at the option's payload offset via `emit_record_to_mem`
           (the same product writer `list<record>` uses), threading the running spill cursor + the payload record's
           WIT (from the element's `option<…>` WIT) for field order; on None the payload area is left unwritten (a
           none option's payload is never read at the canonical lift). `list_elem_marshalable`'s option arm was
           widened from scalar-only to also admit a record/tuple payload whose fields are `product_field_marshalable`
           (in lockstep with the marshal + `collect_list_elem_ops`, which recurses the payload's field ops). Each
           element occupies the canonical `option<record{a,b}>` stride (disc byte + 8-byte-aligned record payload).
           run() builds [Some({a:1, b:2}), None, Some({a:3, b:4})], performs probe.push, returns the stub 55. A
           VALID component that runs is the pin (a wrong option/record layout traps at the host's list.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (record (= a (s64)) (= b (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (Record (: a Int64) (: b Int64)))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some #record((= a 1) (= b 2))) None (Some #record((= a 3) (= b 4)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL list<option<tuple<s64, s64>>> host-op arg writes each option element in place (tuple payload) — Some + None"
  (doc
    "SHAPE 153 (v-wit-boundary) — a top-level `list<option<tuple<s64, s64>>>` bare host-op arg: the tuple-payload
           twin of SHAPE 152. On Some `emit_option_to_mem` writes the payload tuple IN PLACE at the payload offset
           via `emit_tuple_to_mem` (positional — a tuple's WIT order IS its element order, no WIT threading needed);
           None leaves the payload area unwritten. Each element occupies the canonical `option<tuple<s64,s64>>`
           stride (disc byte + 8-byte-aligned 16-byte tuple payload). run() builds [Some((1, 2)), None], performs
           probe.push, returns the stub 55. Complements SHAPE 152 (record payload).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (tuple (s64) (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (Tuple Int64 Int64))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some #tuple(1 2)) None))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL list<option<bytes>> host-op arg writes each option element in place (byte-leaf payload) — Some + None"
  (doc
    "SHAPE 154 (v-wit-boundary) — a top-level `list<option<list<u8>>>` (`list<option<bytes>>`) bare host-op arg:
           an option element whose payload is a `Bytes` byte-leaf. `emit_option_to_mem` gained a Bytes-payload
           branch — on Some it copies the payload rope into shared `mem` at the running spill cursor and writes the
           canonical `option<list<u8>>` payload `(ptr, len)` at the payload offset (`align_up(1, align(list)=4) = 4`),
           advancing the cursor; on None the payload area is left unwritten (a none option's `(ptr,len)` is never
           read at the canonical lift). `list_elem_marshalable`'s option arm was widened to admit a `Bytes` payload
           (in lockstep with `emit_list_arg_marshal`'s `option_elem` detector + `collect_list_elem_ops`, which
           declares the payload's `bytes-len`/`bytes-get` via the shared recursion). Each element occupies the
           canonical `option<list<u8>>` stride (disc byte + 4-byte-aligned `(ptr,len)`); the ropes spill after the
           element array. run() builds [Some(b\"\\x01\\x02\\x03\"), None, Some(b\"\\x04\\x05\")], performs probe.push,
           returns the stub 55. A VALID component that runs is the pin (a wrong option/bytes layout traps at the
           host's list.lift). The byte-leaf twin of SHAPE 152/153 (record/tuple payload).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (list (u8))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option Bytes)) Int64)))
      (def (run) (host (probe) (probe.push #list((Some (Bytes.of #list(1 2 3))) None (Some (Bytes.of #list(4 5)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a record host-op ARG whose FIELD is a list<record{a}> marshals the nested list of records into shared mem"
  (doc
    "SHAPE 155 (v-wit-boundary) — a record host-op arg (`probe.push : func(record{n: s64, xs: list<record{a:
           s64}>}) -> s64)`) whose FIELD is a `list<record>` — the record-element twin of SHAPE 31 (a list<s64>
           field). `emit_record_arg_marshal`'s list-field arm runs `emit_list_arg_marshal` on the field's List
           handle, and its element writer (`emit_record_to_mem`) writes each record element IN PLACE into the
           backing array at the running cursor; the record flattens to `(n: i64, xs_ptr: i32, xs_count: i32)` = 3
           core slots. The `field_boundary_abi` element recursion (list → record) already builds the component
           `(list (record …))` field type, and the marshal + `collect_record_field_ops` recurse it in lockstep.
           run() builds {n: 7, xs: [{a: 1}, {a: 2}]}, performs probe.push, returns the stub 55. Pins the
           record-field list-of-records path (SHAPE 31 covered only a list<scalar> field).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= n (s64)) (= xs (list (record (= a (s64))))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: n Int64) (: xs (List (Record (: a Int64))))) Int64)))
      (def (run) (host (probe) (probe.push #record((= n 7) (= xs #list(#record((= a 1)) #record((= a 2))))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<list<s64>> host-op arg flattens (disc, ptr, count) — Some marshals the payload list into mem"
  (doc
    "SHAPE 156 (v-wit-boundary) — a top-level `option<list<s64>>` bare host-op arg: an option whose payload is a
           `list`. `emit_option_reg_flatten` gained a list-payload branch — the register analogue of the
           option<bytes> `(disc, ptr, len)` branch: on Some it marshals the payload list into shared `mem` at the
           running scratch cursor via `emit_list_arg_marshal` (which leaves `(outer-ptr, count)`), captures them,
           and pushes `(disc=1, ptr, count)`; on None all three slots are 0. `option_arg_crosses` was widened to
           admit a `list<T>` payload whose element crosses (`field_boundary_abi`), in lockstep with the classifier
           option-arg build (`RecordFieldAbi::Option(List(<elem>))`), the emit dispatch, the cursor pre-scan, and
           `used_ops` (which declares `vec-len`/`vec-get` + the element ops). `field_boundary_abi` itself is NOT
           widened — an `option<list>` RECORD FIELD (whose inline marshal has no list arm) still declines, so no
           miscompile. run() builds Some([1, 2, 3]), performs probe.push, returns the stub 55. A VALID component
           that runs is the pin (a wrong option/list flatten traps at the host's option/list lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (list (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (List Int64)) Int64)))
      (def (run) (host (probe) (probe.push (Some #list(1 2 3)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL option<list<s64>> host-op arg = None flattens to (0, 0, 0)"
  (doc
    "SHAPE 157 (v-wit-boundary) — the NONE arm of SHAPE 156. An `option<list<s64>>` that is None flattens to
           `(disc=0, ptr=0, count=0)` — `emit_option_reg_flatten`'s list branch else-fills the three slots (a none
           option never reads its payload, so the list is not marshalled). run() builds None, performs probe.push,
           returns the stub 42. Complements SHAPE 156 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (list (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (List Int64)) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL tuple<option<list<s64>>, s64> host-op arg flattens the option<list> element inline — Some"
  (doc
    "SHAPE 158 (v-wit-boundary) — a top-level `tuple<option<list<s64>>, s64>` bare host-op arg: an `option<list>`
           as a tuple ELEMENT. `emit_tuple_reg_flatten`'s option-element arm routes through the SAME
           `emit_option_reg_flatten` list branch as the top-level arg, so `tuple_arg_crosses` (which uses
           `option_arg_crosses` for an option element) admits it in lockstep. The tuple-element abi build
           constructs the `Option(List(<elem>))` abi INLINE (mirroring the top-level arg; `field_boundary_abi`
           stays unwidened). The arg flattens to `(opt_disc, opt_ptr, opt_count, outer1: i64)`. run() builds
           (Some([5, 6]), 9), performs probe.push, returns the stub 55. Pins the option<list> tuple-element path.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (option (list (s64))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Option (List Int64)) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Some #list(5 6)) 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a record host-op ARG with an option<list<s64>> FIELD flattens (disc, ptr, count) — Some marshals the payload list"
  (doc
    "SHAPE 159 (v-wit-boundary) — a record host-op arg (`probe.push : func(record{n: s64, o: option<list<s64>>})
           -> s64)`) whose FIELD is an `option<list>`. `emit_record_arg_marshal` gained an option<list> field arm
           (the list analogue of its option<bytes> field arm): on Some it marshals the payload list into `mem` at
           the running cursor via `emit_list_arg_marshal` (which leaves `(outer-ptr, count)`), captures them, and
           pushes `(disc=1, ptr, count)`; on None `(0,0,0)`. `field_boundary_abi`'s option arm was widened to admit
           `option<list>` (an `Option(List(<elem>))` abi), so `is_boundary_record` now admits a record carrying an
           option<list> field — in lockstep with the new marshal arm (decline-don't-miscompile). The whole record
           flattens to `(n: i64, o_disc: i32, o_ptr: i32, o_count: i32)`. The cursor pre-scan
           (`record_has_option_field_needing_mem`) reserves the scratch cursor for an option<list>-field record.
           run() builds {n: 7, o: Some([1, 2])}, performs probe.push, returns the stub 55. A VALID component that
           runs is the pin. Completes the option<list> family (arg + tuple element = SHAPE 156-158; this is the
           record-field position).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= n (s64)) (= o (option (list (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: n Int64) (: o (Option (List Int64)))) Int64)))
      (def (run) (host (probe) (probe.push #record((= n 7) (= o (Some #list(1 2)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a record host-op ARG with an option<list<s64>> FIELD = None flattens to (0, 0, 0)"
  (doc
    "SHAPE 160 (v-wit-boundary) — the NONE arm of SHAPE 159. An `option<list>` record field that is None flattens
           to `(disc=0, ptr=0, count=0)` — the marshal's else branch zero-fills the three slots (a none option
           never reads its payload, so the list is not marshalled). run() builds {n: 7, o: None}, performs
           probe.push, returns the stub 42. Complements SHAPE 159 (Some).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= n (s64)) (= o (option (list (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: n Int64) (: o (Option (List Int64)))) Int64)))
      (def (run) (host (probe) (probe.push #record((= n 7) (= o None)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a TOP-LEVEL option<record{n, o: option<list<s64>>}> host-op arg composes the option<record> + option<list> field marshals"
  (doc
    "SHAPE 161 (v-wit-boundary) — a top-level `option<record{n: s64, o: option<list<s64>>}>` bare host-op arg: the
           COMPOSED shape proving the option<list> field marshal (SHAPE 159) rides inside an option<record>
           payload. `option_arg_crosses` → `is_boundary_record` admits the payload record (its option<list> field
           now crosses via `field_boundary_abi`), and `emit_option_reg_flatten`'s record branch recurses
           `emit_record_arg_marshal`, whose option<list> field arm marshals the inner list. run() builds Some({n:
           7, o: Some([1, 2])}), performs probe.push, returns the stub 55. Pins the two-level compound composition.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= n (s64)) (= o (option (list (s64))))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Record (: n Int64) (: o (Option (List Int64))))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= n 7) (= o (Some #list(1 2))))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL list<option<list<s64>>> host-op arg writes each option element in place (nested-list payload) — Some + None"
  (doc
    "SHAPE 162 (v-wit-boundary) — a top-level `list<option<list<s64>>>` bare host-op arg: an option list element
           whose payload is itself a `list`. `emit_option_to_mem` gained a list-payload branch (the list analogue
           of its Bytes branch) — on Some it marshals the payload list into shared `mem` at the running cursor via
           `emit_list_arg_marshal` (which leaves `(outer-ptr, count)`) and writes the canonical `option<list>`
           payload `(ptr, count)` header at the payload offset; on None the payload area is left unwritten (a none
           option's `(ptr,count)` is never read on lift). `list_elem_marshalable`'s option arm was widened to admit
           a `list` payload whose element crosses, in lockstep with `emit_list_arg_marshal`'s `option_elem`
           detector + `collect_list_elem_ops` (declares the payload's `vec-len`/`vec-get` + element ops). Each
           element occupies the canonical `option<list<s64>>` stride (disc byte + 4-byte-aligned `(ptr,count)`);
           the inner-list backings spill after the outer element array. run() builds [Some([1, 2]), None,
           Some([3, 4, 5])], performs probe.push, returns the stub 55. Completes the option<list> family at the
           list-ELEMENT position (arg/tuple/record-field were SHAPE 156-161).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (list (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (List Int64))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some #list(1 2)) None (Some #list(3 4 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL list<result<list<u8>, enum>> host-op arg writes each result element in place (Ok bytes / Err enum)"
  (doc
    "SHAPE 163 (v-wit-boundary) — a top-level `list<result<list<u8>, enum>>` bare host-op arg: a `result` as a list
           ELEMENT. A new in-place writer `emit_result_to_mem` writes each element per its canonical result layout
           (disc byte at 0, payload at `payload_off = align_up(1, 4) = 4`): the guest sum-disc IS the component
           result disc (Ok=0 declared first, matching the result-FIELD flatten arm); Ok copies the Bytes payload
           rope into shared `mem` at the running cursor and writes `(ptr@off, len@off+4)`, advancing the cursor;
           Err writes the err enum's discriminant at `off` (at its canonical width) with `off+4` zero-padded.
           `list_elem_marshalable` gained a result arm (`result_bytes_enum`), in lockstep with the element dispatch
           in `emit_list_arg_marshal` + `collect_list_elem_ops` (declares `sum-disc`/`sum-payload` + the Ok
           `bytes-len`/`bytes-get`). `field_boundary_abi` already builds the `(list (result …))` component type.
           run() builds [Ok(b\"\\x01\\x02\\x03\"), Err(bad), Ok(b\"\\x09\")], performs probe.push, returns the stub 55.
           A VALID component that runs is the pin (a wrong result/enum layout traps at the host's list.lift). The
           result-ELEMENT sibling of the `result<list<u8>, enum>` record FIELD (already covered).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (result (list (u8)) (enum bad worse)))) (result (s64)))))))
  (input
    (do
      (type E (Bad) (Worse))
      (effect probe (op push (-> (List (Result Bytes E)) Int64)))
      (def (run) (host (probe) (probe.push #list((Ok (Bytes.of #list(1 2 3))) (Err E.Bad) (Ok (Bytes.of #list(9)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL result<list<u8>, enum> host-op arg crosses on the Ok arm (payload bytes copied into mem)"
  (doc
    "SHAPE 164 (v-wit-boundary) — a top-level `result<list<u8>, enum>` BARE host-op arg, the Ok arm. A `result`
           at the param position (the register twin of the `result` record FIELD, SHAPE 17, and the list ELEMENT,
           SHAPE 163). The classifier pushes `HostParam::Result(err-cases)`; the guest flattens the value-heap
           Result to `(disc:i32, i32, i32)` core slots via `emit_result_arg_reg_flatten`: the guest sum-disc IS
           the component result disc (Ok=0 declared first). On Ok it copies the `list<u8>` payload rope into shared
           `mem` at the running cursor and passes `(0, ptr, len)`. The component `(result (list u8) (enum …))`
           param type builds from the world's declared WIT via `add_wit_type_deduped`; the core functype adds the
           3 i32 slots (`host_import_functype`). `first_unrepresentable_host_op` + `option_arg_crosses`/`variant`/
           `enum` decline a result shape, so this is admitted by its own `result_bytes_enum` gate, in lockstep with
           the emit dispatch + `collect_used_ops` (declares `sum-disc`/`sum-payload` + `bytes-len`/`bytes-get`).
           run() builds Ok(b\"\\x01\\x02\\x03\") and performs probe.push; a VALID component that runs is the pin (a
           wrong result/bytes layout traps at the host's result.lift). Err arm = SHAPE 165.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (u8)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type E (Bad) (Worse))
      (effect probe (op push (-> (Result Bytes E) Int64)))
      (def (run) (host (probe) (probe.push (Ok (Bytes.of #list(1 2 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a TOP-LEVEL result<list<u8>, enum> host-op arg crosses on the Err arm (err enum disc + zero-pad)"
  (doc
    "SHAPE 165 (v-wit-boundary) — the Err arm of SHAPE 164. A top-level `result<list<u8>, enum>` bare host-op arg
           that is Err(E.Worse): `emit_result_arg_reg_flatten` reads the guest sum-disc (≠0 → the component result
           Err disc), reads the err enum payload's `sum-disc` as the component enum discriminant, and passes
           `(disc, err-enum-disc, 0)` with no `mem` write. Completes the `result<list<u8>, enum>` bare-arg family
           (Ok = SHAPE 164). run() builds Err(E.Worse) and performs probe.push; a VALID component that runs is the
           pin (a wrong err-enum disc traps at the host's result.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (u8)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type E (Bad) (Worse))
      (effect probe (op push (-> (Result Bytes E) Int64)))
      (def (run) (host (probe) (probe.push (Err E.Worse))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with a scalar-payload VARIANT field crosses (variant flattened in place)"
  (doc
    "SHAPE 166 (v-wit-boundary) — a RECORD host-op ARGUMENT with a scalar-payload `variant` FIELD
           (probe.push : func(record{ v: variant{go, stop(s64)}, n: s64 }) -> s64). The variant FIELD rides
           the SAME `field_boundary_abi` Variant arm the bare-variant ARG (SHAPE 93) / a list-element variant
           uses; `emit_record_arg_marshal`'s variant-field arm flattens it to `(disc:i32, payload-join)` via the
           shared `emit_variant_reg_flatten`, joining the record's core run alongside the `n: s64` scalar field;
           `collect_record_field_ops` declares the variant arm's `sum-disc`/`sum-payload` + payload unbox in
           lockstep. This was already reachable (the three sites were widened for the variant algebra) but
           UNTESTED — locking in the value round-trip. run() builds { v: Stop(7), n: 5 } and performs probe.push;
           a VALID component that runs is the pin (a wrong variant flatten traps at the host's record.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (variant (go) (stop (s64)))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Record (: v Sig) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v (Sig.Stop 7)) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<variant> host-op arg crosses on the Some arm (variant flattened in the payload)"
  (doc
    "SHAPE 167 (v-wit-boundary) — a top-level `option<variant>` bare host-op ARGUMENT (probe.push :
           func(option<variant{go, stop(s64)}>) -> s64), the Some arm. The option payload is a scalar-payload
           `variant` — `option_arg_crosses` now admits it, and `emit_option_reg_flatten`'s variant branch
           flattens the value-heap option to `(opt-disc:i32, var-disc:i32, payload-join)` = the option disc +
           the payload variant's own `(disc, join)` flatten (via the shared `emit_variant_reg_flatten`, the SAME
           helper the bare-variant ARG (SHAPE 93) / a record variant FIELD (SHAPE 166) uses). The classifier
           builds `RecordFieldAbi::Option(Variant(cases))` via the shared `field_boundary_abi` Variant arm;
           `flatten_record_field_abi` already flattens `Option(Variant)` to the 3 core slots, and the
           `(option (variant …))` component type builds from the world's WIT. Widened in lockstep:
           `option_arg_crosses`, the classifier option arm, `emit_option_reg_flatten`, and `collect_used_ops`'s
           option-payload dispatch (declares the variant's `sum-disc`/`sum-payload` + payload unbox). run() emits
           Some(Stop(7)) and performs probe.push; a VALID component that runs is the pin (a wrong variant flatten
           traps at the host's option.lift). None arm = SHAPE 168.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (variant (go) (stop (s64))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Option Sig) Int64)))
      (def (run) (host (probe) (probe.push (Some (Sig.Stop 7)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<variant> host-op arg crosses on the None arm (all slots zero)"
  (doc
    "SHAPE 168 (v-wit-boundary) — the None arm of SHAPE 167. A top-level `option<variant>` bare host-op arg that
           is None flattens to `(0, 0, 0)` — the option disc 0 (WIT none) with both the variant-disc and the
           payload-join slots zero-filled (a none option never reads its payload). Completes the `option<variant>`
           bare-arg family (Some = SHAPE 167). run() performs probe.push None; the host stub returns 42.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (variant (go) (stop (s64))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Option Sig) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a RECORD host-op arg with an option<variant> FIELD crosses on the Some arm (variant flattened in the payload)"
  (doc
    "SHAPE 169 (v-wit-boundary) — a RECORD host-op ARGUMENT with an `option<variant>` FIELD (probe.push :
           func(record{ v: option<variant{go, stop(s64)}>, n: s64 }) -> s64), the Some arm. The record-FIELD
           composition of the top-level `option<variant>` arg (SHAPE 167): `field_boundary_abi`'s option arm now
           admits a scalar-payload variant payload (→ `Option(Variant)`), so `is_boundary_record` accepts the
           record; `emit_record_arg_marshal`'s new option<variant> field arm flattens the field to
           `(opt-disc:i32, var-disc:i32, payload-join)` via the shared `emit_variant_reg_flatten`, joining the
           record's core run alongside `n: s64`; None zero-fills. `flatten_record_field_abi` already flattens
           `Option(Variant)` to the 3 slots, and `collect_record_field_ops`'s option arm recurses into the
           variant payload (declaring its `sum-disc`/`sum-payload` + unbox). run() builds { v: Some(Stop(7)),
           n: 5 } and performs probe.push; a VALID component that runs is the pin. None arm = SHAPE 170.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (option (variant (go) (stop (s64))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Record (: v (Option Sig)) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v (Some (Sig.Stop 7))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with an option<variant> FIELD crosses on the None arm (payload slots zero)"
  (doc
    "SHAPE 170 (v-wit-boundary) — the None arm of SHAPE 169. A record host-op arg whose `option<variant>` field
           is None flattens that field to `(0, 0, 0)` — the option disc 0 (WIT none) with both the variant-disc
           and the payload-join slots zero-filled — joining `n: s64` in the record's core run. Completes the
           record option<variant>-field family (Some = SHAPE 169).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (option (variant (go) (stop (s64))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Record (: v (Option Sig)) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v None) (= n 9)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a top-level list<variant> host-op arg crosses (each scalar-payload variant element written in place)"
  (doc
    "SHAPE 171 (v-wit-boundary) — a top-level `list<variant>` bare host-op ARGUMENT (probe.push :
           func(list<variant{go, stop(s64)}>) -> s64). A scalar-payload `variant` as a list ELEMENT: the list
           marshal (`emit_list_arg_marshal`) writes each element in place at the canonical variant layout (disc
           + uniform scalar payload) via `emit_variant_to_mem`, the SAME writer a list-of-variant / record-field
           variant uses; `list_elem_marshalable` admits it and `collect_list_elem_ops` declares its ops. Already
           reachable (the variant algebra was widened across the element sites); SHAPE 171 locks in the value
           round-trip that was previously untested. run() builds [Stop(7), Go] and performs probe.push; a VALID
           component that runs is the pin (a wrong variant element layout traps at the host's list.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (go) (stop (s64))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (List Sig) Int64)))
      (def (run) (host (probe) (probe.push #list((Sig.Stop 7) (Sig.Go)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<variant, s64> host-op arg crosses (variant element flattened positionally)"
  (doc
    "SHAPE 172 (v-wit-boundary) — a top-level `tuple<variant, s64>` bare host-op ARGUMENT (probe.push :
           func(tuple<variant{go, stop(s64)}, s64>) -> s64). A scalar-payload `variant` as a tuple ELEMENT:
           `emit_tuple_reg_flatten` flattens the variant element positionally to `(var-disc, payload-join)` via
           the shared `emit_variant_reg_flatten` (the SAME helper the bare-variant ARG / a record variant FIELD
           uses), joined with the `s64` element's one slot. `tuple_arg_crosses` admits a variant element and
           `field_boundary_abi` builds its `Variant` abi. Already reachable (the variant algebra was widened
           across the tuple-element sites); SHAPE 172 locks in the value round-trip. run() builds (Stop(7), 5)
           and performs probe.push; a VALID component that runs is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (variant (go) (stop (s64))) (s64))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Tuple Sig Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Sig.Stop 7) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<record-with-variant-field> host-op arg crosses on the Some arm"
  (doc
    "SHAPE 173 (v-wit-boundary) — a top-level `option<record{ v: variant{go, stop(s64)}, n: s64 }>` bare host-op
           ARGUMENT (probe.push), the Some arm. Composes the option<record> arg (the payload record has a
           scalar-payload `variant` FIELD, SHAPE 166's field shape): `is_boundary_record` admits the payload
           record (its variant field crosses via `field_boundary_abi`'s Variant arm), and
           `emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`, whose variant-field arm
           flattens the variant to `(disc, join)` — so the option flattens to `(opt-disc, var-disc, join, n)`.
           Already reachable (option<record> + the record variant-field arm compose); SHAPE 173 locks in the
           value round-trip. run() builds Some({ v: Stop(7), n: 5 }) and performs probe.push; a VALID component
           that runs is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= v (variant (go) (stop (s64)))) (= n (s64))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop Int64))
      (effect probe (op push (-> (Option (Record (: v Sig) (: n Int64))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= v (Sig.Stop 7)) (= n 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with a payloadless ENUM field crosses (disc flattened as one i32)"
  (doc
    "SHAPE 174 (v-wit-boundary) — a RECORD host-op ARGUMENT with a payloadless `enum` FIELD (probe.push :
           func(record{ e: enum{red,green,blue}, n: s64 }) -> s64). A payload-less enum was crossable only at
           the TOP-LEVEL arg position (`HostParam::Enum`); nested in a record it declined because
           `field_boundary_abi` had no enum arm. Now `field_boundary_abi` maps a payload-less `Sum` (via
           `enum_cases`) to a new `RecordFieldAbi::Enum(cases)`, so `is_boundary_record` accepts the record;
           `flatten_record_field_abi` flattens it to ONE i32 disc slot, and `record_field_cref` lays a nominal
           `enum` DEFINED+EXPORTED type in the record's instance-type (the nested analogue of the top-level enum
           arg's enum type). The guest reads the value-heap sum's disc (a payloadless enum's in-guest rep is a
           bare disc) and writes it inline, joining the `n: s64` slot. run() builds { e: Green, n: 5 } and
           performs probe.push; a VALID component that runs is the pin (a wrong enum type/disc flatten fails
           component validation, CDZ0910, or traps at the host's record.lift). The record-FIELD analogue of the
           top-level enum arg (SHAPE 96).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= e (enum red green blue)) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (Record (: e Col) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= e (Col.Green)) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<enum, s64> host-op arg crosses (enum element flattened as one i32 disc)"
  (doc
    "SHAPE 175 (v-wit-boundary) — a top-level `tuple<enum, s64>` bare host-op ARGUMENT (probe.push :
           func(tuple<enum{red,green,blue}, s64>) -> s64). A payload-less `enum` as a tuple ELEMENT: extends the
           record-FIELD enum support (SHAPE 174) to the tuple-element position. `tuple_arg_crosses` now admits an
           enum element and the tuple-element classifier builds its `RecordFieldAbi::Enum` via the shared
           `field_boundary_abi`; `emit_tuple_reg_flatten` flattens it positionally as one i32 disc via the
           scalar-unbox path (a payloadless enum's in-guest rep is a bare disc), joined with the `s64` element's
           slot. The `(tuple (enum …) s64)` component type carries the enum from the world WIT. run() builds
           (Green, 5) and performs probe.push; a VALID component that runs is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (enum red green blue) (s64))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (Tuple Col Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Col.Green) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<enum> host-op arg crosses on the Some arm (enum disc in the payload slot)"
  (doc
    "SHAPE 176 (v-wit-boundary) — a top-level `option<enum>` bare host-op ARGUMENT (probe.push :
           func(option<enum{red,green,blue}>) -> s64), the Some arm. Extends the nested payload-less `enum`
           support (record FIELD SHAPE 174, tuple ELEMENT SHAPE 175) to the option-PAYLOAD position. An enum's
           disc reads inline as one i32 (the scalar-unbox path), so `option<enum>` flattens to `(opt-disc,
           enum-disc)` EXACTLY like `option<scalar>` — `option_arg_crosses` now admits an enum payload, the
           classifier builds `RecordFieldAbi::Option(Enum)` (so the component type is `(option (enum …))`,
           matching the world), and `emit_option_reg_flatten`'s scalar branch marshals it with NO dedicated arm.
           run() emits Some(Green) and performs probe.push; a VALID component that runs is the pin. None arm =
           SHAPE 177.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (enum red green blue))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (Option Col) Int64)))
      (def (run) (host (probe) (probe.push (Some (Col.Green)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<enum> host-op arg crosses on the None arm (both slots zero)"
  (doc
    "SHAPE 177 (v-wit-boundary) — the None arm of SHAPE 176. A top-level `option<enum>` bare host-op arg that is
           None flattens to `(0, 0)` — the option disc 0 (WIT none) with the enum-disc payload slot zero-filled
           (a none option never reads its payload). Completes the `option<enum>` bare-arg family (Some = SHAPE
           176). run() performs probe.push None; the host stub returns 42.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (enum red green blue))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (Option Col) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a top-level list<enum> host-op arg crosses (each enum element written as its disc at the canonical width)"
  (doc
    "SHAPE 178 (v-wit-boundary) — a top-level `list<enum>` bare host-op ARGUMENT (probe.push :
           func(list<enum{red,green,blue}>) -> s64). The last enum-in-compound position: extends the nested
           payload-less `enum` support (record FIELD 174, tuple ELEMENT 175, option PAYLOAD 176/177) to the
           list-ELEMENT position. `list_elem_marshalable` now admits an enum element (via `enum_cases`); the
           element rides `emit_list_arg_marshal`'s scalar-store path — each element's disc is written in place at
           the enum's canonical width (`disc_size(n_cases)`, one byte for a 3-case enum), read via the guest
           sum's disc-unbox. The `(list (enum …))` component type builds from the world WIT (its element type is
           the enum, via `field_boundary_abi`'s enum arm). run() builds [Green, Red] and performs probe.push; a
           VALID component that runs is the pin (a wrong element stride/disc traps at the host's list.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (enum red green blue))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (List Col) Int64)))
      (def (run) (host (probe) (probe.push #list((Col.Green) (Col.Red)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a list<record> host-op arg with an ENUM field in the record element crosses (enum disc written in place)"
  (doc
    "SHAPE 179 (v-wit-boundary) — a `list<record{ e: enum, n: s64 }>` bare host-op ARGUMENT (probe.push). A
           payload-less `enum` FIELD of a record ELEMENT — the in-mem-writer analogue of the top-level record
           enum FIELD (SHAPE 174). `product_field_marshalable` now admits an enum field (via `enum_cases`), so
           `list_elem_marshalable` accepts the record element; the record element is written in place at its
           canonical layout by `emit_record_to_mem`, whose scalar-field path writes the enum field's disc at the
           field's canonical offset+width (`disc_size(n_cases)`) — no dedicated writer arm, the enum rides the
           scalar store (its disc reads via the guest sum's disc-unbox). The `(list (record … (enum …) …))`
           component type builds from the world WIT. run() builds [{ e: Green, n: 5 }] and performs probe.push;
           a VALID component that runs is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (record (= e (enum red green blue)) (= n (s64))))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (List (Record (: e Col) (: n Int64))) Int64)))
      (def (run) (host (probe) (probe.push #list(#record((= e (Col.Green)) (= n 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with a NESTED record that has an ENUM field crosses (enum disc flattened inline at depth)"
  (doc
    "SHAPE 180 (v-wit-boundary) — a record host-op ARGUMENT `record{ inner: record{ e: enum, n: s64 }, k: s64 }`
           — a payload-less `enum` FIELD one level DEEP (inside a nested record). Composes the nested-record arg
           support (a record field recurses `emit_record_arg_marshal`) with the record enum FIELD (SHAPE 174):
           `field_boundary_abi` builds the inner record's `Enum` field abi, `is_boundary_record` accepts the
           outer record (every field, recursively, crosses), and the inner record flattens inline into the
           parent's core run — the enum field's disc as one i32 via the scalar-unbox path. The `(record (inner
           (record … (enum …) …)) …)` component type builds from the world WIT. run() builds { inner: { e:
           Green, n: 5 }, k: 9 } and performs probe.push; a VALID component that runs is the pin. Locks in the
           value round-trip for an enum at record depth (previously untested).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= inner (record (= e (enum red green blue)) (= n (s64)))) (= k (s64)))) (result (s64)))))))
  (input
    (do
      (type Col (Red) (Green) (Blue))
      (effect probe (op push (-> (Record (: inner (Record (: e Col) (: n Int64))) (: k Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= inner #record((= e (Col.Green)) (= n 5))) (= k 9)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a nested option<option<s64>> host-op arg crosses on the Some(Some) arm (recursive option flatten)"
  (doc
    "SHAPE 181 (v-wit-boundary) — a top-level `option<option<s64>>` bare host-op ARGUMENT (probe.push), the
           Some(Some) arm. A NESTED option: the payload is itself an `option<scalar>`. Flattens to `(outer-disc,
           inner-disc, scalar)` via `emit_option_reg_flatten`'s new nested-option branch, which on outer Some
           reads the inner option handle (sum-payload) and RECURSES `emit_option_reg_flatten` on it (pushing the
           inner `(disc, scalar)`), capturing in reverse. `field_boundary_abi` + `option_arg_crosses` gained a
           nested-option arm (scalar inner), the classifier builds `RecordFieldAbi::Option(Option(Scalar))`, and
           `flatten_record_field_abi` already flattens it to the 3 slots; the `(option (option s64))` component
           type builds from the world WIT. Widened in LOCKSTEP so the shared tuple-element path stays consistent.
           run() emits Some(Some(7)) and performs probe.push; a VALID component that runs is the pin. Other arms =
           SHAPE 182 (Some(None)) / 183 (None).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option Int64)) Int64)))
      (def (run) (host (probe) (probe.push (Some (Some 7)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a nested option<option<s64>> host-op arg crosses on the Some(None) arm (inner None zero-fills the payload)"
  (doc
    "SHAPE 182 (v-wit-boundary) — the Some(None) arm of SHAPE 181. Outer Some, inner None: flattens to
           `(1, 0, 0)` — the outer disc 1 (WIT some), then the inner option's `(inner-disc=0, scalar=0)` from the
           recursive `emit_option_reg_flatten` on the inner None. run() emits Some(None) and performs probe.push;
           a VALID component that runs is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option Int64)) Int64)))
      (def (run) (host (probe) (probe.push (Some None))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a nested option<option<s64>> host-op arg crosses on the outer None arm (all slots zero)"
  (doc
    "SHAPE 183 (v-wit-boundary) — the outer-None arm of SHAPE 181. Outer None flattens to `(0, 0, 0)` — the
           outer disc 0 (WIT none) with both inner slots (inner-disc, scalar) zero-filled (a none option never
           reads its payload). Completes the `option<option<s64>>` bare-arg family (Some(Some) = 181, Some(None)
           = 182). run() performs probe.push None; the host stub returns 42.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option Int64)) Int64)))
      (def (run) (host (probe) (probe.push None)))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare named-variant host-op arg (3 cases, one scalar payload) crosses on the payload arm"
  (doc
    "SHAPE 184 (v-wit-boundary) — the top-level BARE-VARIANT ARG marshal, which `emit_variant_reg_flatten`
           has always handled but the corpus never pinned directly (every prior `variant` case sat inside a
           record field / element / result). A 3-case `variant{a, b, c(s64)}` arg flattens to `(disc:i32, join)`:
           on the `c` arm (decl-disc 2) the payload s64 rides the join slot, so run() emits `(C 9)` -> `(2, 9)`.
           A 3-case variant is NOT reducible to an option (unlike the 2-case some/none), so this genuinely
           exercises the N-case bare-variant register flatten. The host stub returns a fixed 55; a VALID
           component that runs and crosses the boundary is the pin (the marshal shape is pinned by the module
           validating with the right core signature). The nullary arm = SHAPE 185.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b) (c (s64)))) (result (s64)))))))
  (input
    (do
      (type V (A) (B) (C Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (probe.push (C 9))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare named-variant host-op arg crosses on a nullary arm (payload join slot zero-filled)"
  (doc
    "SHAPE 185 (v-wit-boundary) — the nullary-arm counterpart of SHAPE 184. The same 3-case
           `variant{a, b, c(s64)}` arg on its `b` arm (decl-disc 1, no payload) flattens to `(1, 0)`: the disc
           1 with the join slot zero-filled (a nullary variant case never reads the payload), via the same
           `emit_variant_reg_flatten`. run() emits `(B)`; the host stub returns 42. Completes the bare-variant
           ARG pin (payload arm = SHAPE 184).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b) (c (s64)))) (result (s64)))))))
  (input
    (do
      (type V (A) (B) (C Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (probe.push (B))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<s64, enum> host-op arg crosses on the Ok arm (2-slot (disc, i64-join) flatten)"
  (doc
    "SHAPE 186 (v-wit-boundary) — a top-level `result<scalar, enum>` ARG, the scalar-Ok sibling of the
           `result<list<u8>, enum>` arg (SHAPE 164/165). Where the Bytes result flattens to 3 slots
           `(disc, ptr, len)` and copies a rope into `mem`, a scalar-Ok result flattens to just 2 slots
           `(disc:i32, join)` with NO memory: `emit_result_scalar_arg_reg_flatten` reads the result disc, and on
           Ok unboxes the scalar payload into the join slot, on Err reads the err enum's disc into it. The join
           is `i64` here because the Ok scalar is `s64` (the `i32` err disc widens to fit) — the component
           boundary is `result<s64, enum{bad,worse}>`, the core import sig `(param i32 i64)`. run() emits
           `(Ok 7)`; a VALID component that runs and crosses the boundary is the pin (the 2-slot flatten + join
           width are pinned by the module validating with that core signature). The Err arm = SHAPE 187; a
           narrower `i32` join = SHAPE 188.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (s64) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result Int64 Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok 7))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<s64, enum> host-op arg crosses on the Err arm (err enum disc into the join slot)"
  (doc
    "SHAPE 187 (v-wit-boundary) — the Err-arm counterpart of SHAPE 186. On Err the result disc is non-zero
           and `emit_result_scalar_arg_reg_flatten` reads the err ENUM's discriminant (here `worse` = decl-disc 1)
           into the join slot (widened to `i64` to match the s64-Ok join width). run() emits `(Err (Worse))`; the
           host stub returns 42. Same `result<s64, enum>` boundary as SHAPE 186, exercising the other arm.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (s64) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result Int64 Er) Int64)))
      (def (run) (host (probe) (probe.push (Err (Worse)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<bool, enum> host-op arg crosses on the Ok arm (narrower i32 join width)"
  (doc
    "SHAPE 188 (v-wit-boundary) — the narrow-join counterpart of SHAPE 186. A `result<bool, enum>` Ok scalar
           is `i32`-width, so the join stays `i32` (both the Ok bool and the `i32` err disc fit) — the core import
           sig is `(param i32 i32)`, distinct from SHAPE 186's `(param i32 i64)`. This pins that
           `emit_result_scalar_arg_reg_flatten` derives the join width from the Ok scalar (no spurious widening).
           run() emits `(Ok true)`; a VALID component that runs and crosses is the pin. Completes the
           `result<scalar, enum>` bare-arg family (s64 Ok = 186/187, bool Ok = 188).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (bool) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result Bool Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok true))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record-of-scalars, enum> host-op arg crosses on the Ok arm (multi-slot join)"
  (doc
    "SHAPE 189 (v-wit-boundary) — a top-level `result<record, enum>` ARG, the record-Ok sibling of the
           scalar-Ok result (SHAPE 186). Where a scalar Ok flattens to 2 slots `(disc, join)`, a record-of-scalars
           Ok flattens to `(disc, field0, field1, …)` — the discriminant then the record's fields POSITIONALLY in
           WIT declaration order, with the `i32` err disc riding the FIRST field's slot on the Err arm.
           `emit_result_record_arg_reg_flatten` recurses `emit_record_arg_marshal` on Ok (the payload record's N
           pushes captured into the join slots) and puts the err enum's disc in slot 0 on Err. Every Ok field is a
           SCALAR, so each is one register slot and NO memory is needed (a compound field is a later increment).
           Here `record{x:s64, y:s64}` gives the core import sig `(param i32 i64 i64)` — the disc + two s64 fields.
           run() emits `(Ok #record((= x 3) (= y 4)))`; a VALID component that runs and crosses is the pin (the
           multi-slot flatten is pinned by the module validating with that core signature). The Err arm = SHAPE
           190; a mixed-width record (a leading i32 field) = SHAPE 191.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= x (s64)) (= y (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: x Int64) (: y Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= x 3) (= y 4))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record-of-scalars, enum> host-op arg crosses on the Err arm (err disc in slot 0)"
  (doc
    "SHAPE 190 (v-wit-boundary) — the Err-arm counterpart of SHAPE 189. On Err the result disc is non-zero
           and `emit_result_record_arg_reg_flatten` writes the err ENUM's discriminant (here `worse` = decl-disc 1)
           into slot 0 (widened to that slot's `i64` width to match the s64 first field) and zero-fills the
           remaining field slots (a record's Ok fields are never read on the Err arm). run() emits `(Err (Worse))`;
           the host stub returns 42. Same `result<record{x,y}, enum>` boundary as SHAPE 189, other arm.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= x (s64)) (= y (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: x Int64) (: y Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Err (Worse)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<record{bool,s64}, enum> host-op arg crosses on the Ok arm (mixed field widths)"
  (doc
    "SHAPE 191 (v-wit-boundary) — a mixed-width-record counterpart of SHAPE 189. A `record{a:bool, b:s64}`
           Ok flattens to `(disc:i32, a:i32, b:i64)` — the core sig `(param i32 i32 i64)`. The leading `bool`
           field is `i32`-width, so slot 0 (which also carries the `i32` err disc on Err) stays `i32`; the trailing
           `s64` field is `i64`. This pins that `emit_result_record_arg_reg_flatten` derives each slot width from
           its own field (no spurious widening, and the err disc fits the first slot) across differing widths.
           run() emits `(Ok #record((= a true) (= b 9)))`; a VALID component that runs and crosses is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= a (bool)) (= b (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: a Bool) (: b Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= a true) (= b 9))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<tuple-of-scalars, enum> host-op arg crosses on the Ok arm (positional multi-slot join)"
  (doc
    "SHAPE 192 (v-wit-boundary) — a top-level `result<tuple, enum>` ARG, the tuple-Ok sibling of the
           record-Ok result (SHAPE 189). Like the record case it flattens to `(disc, elem0, elem1, …)` — the
           discriminant then the Ok tuple's elements, but POSITIONALLY (element order, NO name-lex/WIT reorder — a
           tuple is positional), the `i32` err disc riding the FIRST element's slot on Err.
           `emit_result_tuple_arg_reg_flatten` recurses `emit_tuple_reg_flatten` on Ok (the N pushes captured into
           the join slots) and puts the err enum's disc in slot 0 on Err. Every Ok element is a non-float SCALAR,
           so each is one register slot and NO memory is needed. Here `tuple<s64, s64>` gives the core import sig
           `(param i32 i64 i64)`. run() emits `(Ok #tuple(3 4))`; a VALID component that runs and crosses is the
           pin. The Err arm = SHAPE 193; a mixed-width tuple = SHAPE 194.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (s64) (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Int64 Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(3 4)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<tuple-of-scalars, enum> host-op arg crosses on the Err arm (err disc in slot 0)"
  (doc
    "SHAPE 193 (v-wit-boundary) — the Err-arm counterpart of SHAPE 192. On Err the result disc is non-zero
           and `emit_result_tuple_arg_reg_flatten` writes the err ENUM's discriminant (here `worse` = decl-disc 1)
           into slot 0 (widened to that slot's `i64` width to match the first s64 element) and zero-fills the
           remaining element slots (a tuple's Ok elements are never read on the Err arm). run() emits
           `(Err (Worse))`; the host stub returns 42. Same `result<tuple<s64,s64>, enum>` boundary as SHAPE 192.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (s64) (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Int64 Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Err (Worse)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<tuple{bool,s64}, enum> host-op arg crosses on the Ok arm (mixed element widths)"
  (doc
    "SHAPE 194 (v-wit-boundary) — a mixed-width-tuple counterpart of SHAPE 192. A `tuple<bool, s64>` Ok
           flattens to `(disc:i32, e0:i32, e1:i64)` — the core sig `(param i32 i32 i64)`. The leading `bool`
           element is `i32`-width, so slot 0 (which also carries the `i32` err disc on Err) stays `i32`; the
           trailing `s64` element is `i64`. Pins that `emit_result_tuple_arg_reg_flatten` derives each slot width
           from its own element across differing widths. run() emits `(Ok #tuple(true 9))`; a VALID component that
           runs and crosses is the pin. Completes the `result<tuple-of-scalars, enum>` bare-arg family.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (bool) (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Bool Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(true 9)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<scalar>, enum> host-op arg crosses on the Ok arm (list marshalled into mem)"
  (doc
    "SHAPE 195 (v-wit-boundary) — a top-level `result<list<T>, enum>` ARG, the list-Ok sibling of the
           Bytes-Ok result (SHAPE 164). It flattens to the SAME 3 slots `(disc:i32, ptr:i32, count:i32)`, but where
           the Bytes result copies a rope into `mem`, a `result<list<scalar>, enum>` MARSHALS the value-heap list
           into `mem` (an outer `count`-slot array at the running cursor, each element inline) via
           `emit_result_list_arg_reg_flatten` → `emit_list_arg_marshal`, passing `(outer-ptr, count)` on Ok. So
           unlike the register-only scalar/record/tuple results, this needs `mem` + the scratch cursor
           (`set_needs_memory` + the emit.rs cursor pre-scan admit it). The core import sig is `(param i32 i32 i32)`.
           run() emits `(Ok #list(3 4 5))`; a VALID component that runs and crosses the boundary is the pin. The
           Err arm = SHAPE 196.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list(3 4 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<scalar>, enum> host-op arg crosses on the Err arm (err disc, count 0)"
  (doc
    "SHAPE 196 (v-wit-boundary) — the Err-arm counterpart of SHAPE 195. On Err the result disc is non-zero
           and `emit_result_list_arg_reg_flatten` passes `(err-enum-disc, 0)` for the `(ptr/errdisc, count)` slots
           (the list is never marshalled on the Err arm — no `mem` write, so the cursor is untouched), here
           `worse` = decl-disc 1. run() emits `(Err (Worse))`; the host stub returns 42. Same
           `result<list<s64>, enum>` boundary as SHAPE 195, other arm. live-objects=0 confirms the Err arm leaks
           no list.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Err (Worse)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<list<record-of-scalars>, enum> host-op arg crosses on the Ok arm (compound list element)"
  (doc
    "SHAPE 197 (v-wit-boundary) — widens the `result<list, enum>` arg (SHAPE 195) from a scalar element to an
           all-scalar-PRODUCT element: a `list<record{x:s64, y:s64}>` Ok. `result_list_enum` now admits a record
           (or tuple) every field/element of which is a scalar — such an element marshals inline via
           `emit_list_arg_marshal` → `emit_record_to_mem` (each field at its offset in the outer array slot) and
           never reaches `list<u8>` (so `has_list_param` stays false). The 3-slot `(disc, ptr, count)` flatten is
           unchanged; only the per-element in-`mem` layout differs. The component boundary is `result<list<record{
           x,y}>, enum>`. run() emits `(Ok #list(#record((= x 1) (= y 2)) #record((= x 3) (= y 4))))`; a VALID
           component that runs and crosses is the pin. A `list<tuple>` element = SHAPE 198.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (record (= x (s64)) (= y (s64)))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List (Record (: x Int64) (: y Int64))) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list(#record((= x 1) (= y 2)) #record((= x 3) (= y 4)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<tuple-of-scalars>, enum> host-op arg crosses on the Ok arm (compound list element)"
  (doc
    "SHAPE 198 (v-wit-boundary) — the tuple-element counterpart of SHAPE 197. A `list<tuple<s64, s64>>` Ok:
           each element is a positional all-scalar tuple, marshalled inline via `emit_list_arg_marshal` →
           `emit_tuple_to_mem` (element i at cell i, no name reorder). Same 3-slot `(disc, ptr, count)` flatten;
           component boundary `result<list<tuple<s64,s64>>, enum>`. run() emits `(Ok #list(#tuple(1 2)
           #tuple(3 4)))`; a VALID component that runs and crosses is the pin. Completes the compound-element
           `result<list<all-scalar-product>, enum>` increment (a Bytes/nested-list element — which reaches
           `list<u8>` — is a further increment).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (tuple (s64) (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List (Tuple Int64 Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list(#tuple(1 2) #tuple(3 4))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<bytes>, enum> host-op arg crosses on the Ok arm (element reaches list<u8>)"
  (doc
    "SHAPE 199 (v-wit-boundary) — widens `result<list, enum>` (SHAPE 197/198) to a list element that REACHES
           `list<u8>`: a `list<list<u8>>` (list of `Bytes`) Ok. `result_list_enum` now admits any element
           `list_elem_marshalable` accepts (the SAME element capability a bare `list<T>` arg uses); the element
           marshals identically via `emit_list_arg_marshal` (a Bytes element writes a `(ptr,len)` header in the
           outer array slot + spills its rope at the running cursor). The `result<list<list<u8>>, enum>` component
           type is built STRUCTURALLY from the declared WIT — `ResultList` rides the structural-CRef path, so a
           `list<u8>`-reaching element needs NO `has_list_param` shared-`(list u8)`-type change (the emitted module
           validates + runs). The 3-slot `(disc, ptr, count)` flatten is unchanged. run() emits
           `(Ok #list((Bytes.of #list(1 2)) (Bytes.of #list(3))))`; a VALID running component is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (list (u8))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List Bytes) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list((Bytes.of #list(1 2)) (Bytes.of #list(3)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<list<scalar>>, enum> host-op arg crosses on the Ok arm (nested-list element)"
  (doc
    "SHAPE 200 (v-wit-boundary) — a nested-list element counterpart of SHAPE 199. A `list<list<s64>>` Ok:
           each element is itself a list, marshalled by the recursive `emit_list_arg_marshal` (an 8-byte
           `(ptr, count)` header in the outer array slot, the inner backing array + element data laid after it).
           Component boundary `result<list<list<s64>>, enum>`, built structurally from WIT. run() emits
           `(Ok #list(#list(1 2) #list(3)))`; a VALID running component is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (list (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List (List Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list(#list(1 2) #list(3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<list<option<scalar>>, enum> host-op arg crosses on the Ok arm (option element)"
  (doc
    "SHAPE 201 (v-wit-boundary) — an option-element counterpart of SHAPE 199. A `list<option<s64>>` Ok: each
           element is written at its canonical option layout (disc byte + payload) by `emit_option_to_mem` within
           the outer array. Component boundary `result<list<option<s64>>, enum>`. run() emits
           `(Ok #list((Some 1) None (Some 3)))`; a VALID running component is the pin. Completes the
           list-element widening of the `result<list, enum>` arg to the full `list_elem_marshalable` set.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (list (option (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (List (Option Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #list((Some 1) None (Some 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record-with-a-Bytes-field, enum> host-op arg crosses on the Ok arm (mem-writing field)"
  (doc
    "SHAPE 202 (v-wit-boundary) — widens the `result<record, enum>` arg (SHAPE 189) from all-scalar fields to
           any boundary record (`result_record_enum` now admits `is_boundary_record` — the SAME field set the
           direct record arg uses). A `record{n:s64, b:list<u8>}` Ok: `emit_result_record_arg_reg_flatten` now
           threads a `cursor` to `emit_record_arg_marshal`, whose Bytes-field arm copies the rope into `mem` and
           writes `(ptr,len)` into the field's two slots. This is the first `result<record>` that needs `mem` —
           `set_needs_memory` (grouped per-field like the direct record arg) + the emit.rs cursor pre-scan admit
           it, and `collect_used_ops` declares the field ops via `collect_record_field_ops` (the scalar-only
           `get_op_ty` would miss `bytes-len`/`bytes-get` → CDZ0910 u32::MAX). The component boundary is
           `result<record{n, b:list<u8>}, enum>`; the `(list u8)` type is built structurally from the declared WIT
           (no `has_list_param` shared-type change). run() emits `(Ok #record((= n 5) (= b (Bytes.of #list(1 2)))))`;
           a VALID running component (live-objects=0) is the pin. A `list` field = SHAPE 203.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= n (s64)) (= b (list (u8)))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: n Int64) (: b Bytes)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= n 5) (= b (Bytes.of #list(1 2))))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record-with-a-list-field, enum> host-op arg crosses on the Ok arm (mem-writing field)"
  (doc
    "SHAPE 203 (v-wit-boundary) — the list-field counterpart of SHAPE 202. A `record{n:s64, xs:list<s64>}` Ok:
           the `list<s64>` field is marshalled into `mem` (an outer `(ptr,count)` header in the record layout, the
           backing array spilled at the cursor) by `emit_record_arg_marshal`'s list-field arm. Same cursor +
           `set_needs_memory` + `collect_record_field_ops` machinery as SHAPE 202. Component boundary
           `result<record{n, xs:list<s64>}, enum>`. run() emits `(Ok #record((= n 5) (= xs #list(1 2 3))))`; a
           VALID running component (live-objects=0) is the pin. Completes the compound-record-FIELD widening of the
           `result<record, enum>` arg.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= n (s64)) (= xs (list (s64)))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: n Int64) (: xs (List Int64))) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= n 5) (= xs #list(1 2 3)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<tuple-with-a-Bytes-element, enum> host-op arg crosses on the Ok arm (mem-writing element)"
  (doc
    "SHAPE 204 (v-wit-boundary) — widens the `result<tuple, enum>` arg (SHAPE 192) from all-scalar elements to
           any boundary element (`result_tuple_enum` now admits every shape `field_boundary_abi` does — the SAME
           element set the direct tuple arg uses, symmetric with the record-FIELD widening of SHAPE 202). A
           `tuple<s64, bytes>` Ok: `emit_result_tuple_arg_reg_flatten` now carries a `Vec<RecordFieldAbi>` and
           threads a `cursor` to `emit_tuple_reg_flatten`, whose Bytes-element arm copies the rope into `mem` and
           writes `(ptr,len)` into the element's two slots. This is the first `result<tuple>` that needs `mem` —
           `set_needs_memory` (per-element like the direct tuple arg) + the emit.rs cursor pre-scan admit it, and
           `collect_used_ops` declares the element ops via `collect_record_field_ops` (the scalar-only `get_op_ty`
           would miss `bytes-len`/`bytes-get` → CDZ0910 u32::MAX). The component boundary is
           `result<tuple<s64, list<u8>>, enum>`; the `(list u8)` type is built structurally from the declared WIT.
           run() emits `(Ok #tuple(5 (Bytes.of #list(1 2))))`; a VALID running component (live-objects=0) is the
           pin. A `list` element = SHAPE 205.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (s64) (list (u8))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Int64 Bytes) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(5 (Bytes.of #list(1 2)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<tuple-with-a-list-element, enum> host-op arg crosses on the Ok arm (mem-writing element)"
  (doc
    "SHAPE 205 (v-wit-boundary) — the list-element counterpart of SHAPE 204. A `tuple<s64, list<s64>>` Ok:
           the `list<s64>` element is marshalled into `mem` (an outer `(ptr,count)` header in the tuple layout, the
           backing array spilled at the cursor) by `emit_tuple_reg_flatten`'s list-element arm. Same cursor +
           `set_needs_memory` + `collect_record_field_ops` machinery as SHAPE 204. Component boundary
           `result<tuple<s64, list<s64>>, enum>`. run() emits `(Ok #tuple(5 #list(1 2 3)))`; a VALID running
           component (live-objects=0) is the pin. Completes the compound-tuple-ELEMENT widening of the
           `result<tuple, enum>` arg.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (s64) (list (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Int64 (List Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(5 #list(1 2 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<tuple<s64,f64>, enum> host-op arg crosses on the Ok arm (float in a non-slot-0 element)"
  (doc
    "SHAPE 206 (v-wit-boundary) — a FLOAT tuple element in a NON-slot-0 position. The payloadless-enum Err arm
           flattens to a single `i32` (its disc), so the result flatten joins that `i32` with ONLY the Ok payload's
           FIRST slot; slots 1+ have no Err counterpart and keep their own core type. Here slot 0 is the `s64`
           (an integer that absorbs the `i32` disc by widening to `i64`) and slot 1 is the `f64`, which rides its
           own `f64` slot on Ok and is zero-filled (`F64ConstBits(0)`) on Err — no reinterpret join needed. Core
           `(param i32 i64 f64)`, exactly the canonical `result<tuple<f64…>, enum>` flatten. `result_tuple_enum`
           admits a float element in any position EXCEPT slot 0 (a float FIRST slot would need the canonical
           reinterpret join, which is DECLINED for now). run() emits `(Ok #tuple(7 1.5))`; a VALID running
           component (live-objects=0) is the pin. The record twin = SHAPE 207.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (s64) (f64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Int64 Float64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(7 1.5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record{n:s64,x:f64}, enum> host-op arg crosses on the Ok arm (float in a non-slot-0 field)"
  (doc
    "SHAPE 207 (v-wit-boundary) — the record twin of SHAPE 206: a FLOAT field in a NON-slot-0 (WIT-order) position.
           The fields are marshalled in WIT declaration order, so slot 0 is `n:s64` (an integer that absorbs the
           `i32` err disc by widening to `i64`) and slot 1 is `x:f64` (its own `f64` slot on Ok, `F64ConstBits(0)`
           on Err). Core `(param i32 i64 f64)`. `result_record_enum` admits any boundary field; a float field only
           needs care when it lands in slot 0 (the err-disc join slot) — here it does not. run() emits
           `(Ok #record((= n 7) (= x 1.5)))`; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= n (s64)) (= x (f64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: n Int64) (: x Float64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= n 7) (= x 1.5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a list<u8>-RESULT host op immediately before a string-ARG host op lowers the string arg cleanly (adjacency regression)"
  (doc
    "SHAPE 208 (v-wit-boundary) — REGRESSION PIN for a suspected guest-side string-ARG mislowering
           (v-hivemind issue, re-verified on current main via the real-wasmtime nix value gate). TRIGGER: a host op
           with a `list<u8>` RESULT (`put-blob`) called IMMEDIATELY before a host op with a `string` ARG and a
           `list<record>` RESULT (`materialize`), the record being the full 4-field `event{kind:string,
           source:string, session:list<u8>, at:u64}` — the exact shape that once drove the wasmtime canonical-ABI
           lift to trap `invalid utf-8` when lowering the `kind` string arg with a bad (ptr,len). The concern was
           that the preceding list-RESULT lift's scratch/retptr allocation collided with the following string-arg
           lowering. On current main the string arg lowers with a correct (ptr,len): the guest calls put-blob
           (dropping its list<u8> result), then materialize(\"session-spawned\", empty, empty) and returns
           `List.len` of its list<event> result. A real component-model host LIFTS the `kind` string arg (a
           mislowering would trap `invalid utf-8` at the lift, BEFORE the host fn runs); it lifts cleanly and the
           list<event> response (2 elements) is returned → output 2, live-objects=0. This case is the permanent
           guard the adjacency defect never returns.")
  (wit-world
    (world w (import cadenza:platform/sys
      (member put-blob (func (param bytes (list (u8))) (result (list (u8)))))
      (member materialize
        (func
          (param kind (string))
          (param session (list (u8)))
          (param source (string))
          (result (list (record (= kind (string)) (= source (string)) (= session (list (u8))) (= at (u64))))))))))
  (input
    (do
      (effect sys
        (op put-blob (-> Bytes Bytes))
        (op materialize (-> String (-> Bytes (-> String (List (Record (: kind String) (: source String) (: session Bytes) (: at UInt64))))))))
      (def (run)
        (host (sys)
          (let ((_b (sys.put-blob b"data")))
            (List.len (sys.materialize "session-spawned" b"" "")))))
      (export run)))
  (call run)
  (host-responses
    (respond sys.put-blob (: #list(1 2 3) Bytes))
    (respond sys.materialize
      (:
        #list(#record((= kind "a") (= source "b") (= session #list()) (= at 1))
              #record((= kind "c") (= source "d") (= session #list()) (= at 2)))
        (List (Record (: kind String) (: source String) (: session Bytes) (: at UInt64))))))
  (host-calls
    (call cadenza:platform/sys.put-blob)
    (call cadenza:platform/sys.materialize))
  (output 2)
  (live-objects 0))

(case
  "a bare result<f64, enum> host-op arg crosses on the Ok arm (float reinterpret join to the i64 slot)"
  (doc
    "SHAPE 209 (v-wit-boundary) — a FLOAT Ok in a `result<scalar, enum>` arg, the float counterpart of SHAPE 186
           (s64 Ok). The result flatten joins the Ok's single slot with the `i32` err disc; for an `f64` the
           canonical join is the reinterpret lattice `join(f64,i32)=i64`, so the core boundary is `(param i32 i64)`
           — SAME core sig as the s64 case, but the Ok arm now bit-REINTERPRETS the `f64` payload into the `i64`
           join slot (`I64ReinterpretF64`) rather than storing an integer, and the host lift reads the `i64` back
           as the `f64`. `result_scalar_enum` now admits a float Ok (was excluded); `emit_result_scalar_arg_reg_
           flatten` emits the reinterpret; `serialize` keys the join width off the Ok's core width (i64 for f64,
           not just `== i64`). run() emits `(Ok 1.5)`; a VALID running component (live-objects=0) is the pin (the
           reinterpret + join width are pinned by the module validating with `(param i32 i64)` — a wrong join slot
           would fail validation). The f32 twin = SHAPE 210. Only the SLOT-0 float needs the reinterpret; a float
           in a later record/tuple slot rides its own f-slot (SHAPE 206/207).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (f64) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result Float64 Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok 1.5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<f32, enum> host-op arg crosses on the Ok arm (float reinterpret join to the i32 slot)"
  (doc
    "SHAPE 210 (v-wit-boundary) — the f32 twin of SHAPE 209. For an `f32` Ok the canonical join is
           `join(f32,i32)=i32` (both 4-byte), so the core boundary is `(param i32 i32)` and the Ok arm
           bit-reinterprets the `f32` payload into the `i32` join slot (`I32ReinterpretF32`); the Err arm stores
           the err disc directly (no widen, the slot is already `i32`). run() emits `(Ok (: 1.5 Float32))` (the
           `Float32` annotation pins the literal width — a bare `1.5` is `Float64`); a VALID running component
           (live-objects=0) is the pin. Completes the `result<float-scalar, enum>` reinterpret-join arg.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (f32) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result Float32 Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok (: 1.5 Float32)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<tuple<f64,s64>, enum> host-op arg crosses on the Ok arm (float FIRST element, reinterpret join)"
  (doc
    "SHAPE 211 (v-wit-boundary) — a FLOAT in SLOT 0 of a `result<tuple, enum>` arg, the tuple counterpart of the
           scalar float reinterpret join (SHAPE 209). Only slot 0 joins the `i32` err disc (the payloadless-enum
           Err flattens to a single `i32`), so a float FIRST element bit-reinterprets into the integer slot-0 join
           — `join(f64,i32)=i64` — while slot 1 (the `s64`) keeps its own `i64`. Core `(param i32 i64 i64)`:
           `emit_result_tuple_arg_reg_flatten` overrides `slot_vts[0]` to the join int and emits `I64ReinterpretF64`
           at the k==0 reverse-capture (the Ok arm), the Err arm stores the err disc into slot 0 (widened to i64);
           `serialize` emits the join int for slot 0; `result_tuple_enum` now admits a float first element (the
           SHAPE 206 slot-0 decline is lifted). The host lift reads slot 0 back as the `f64`. run() emits
           `(Ok #tuple(1.5 7))`; a VALID running component (live-objects=0) is the pin. The f32 twin = SHAPE 212;
           a float in a LATER slot rides its own f-slot (SHAPE 206).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (f64) (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Float64 Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple(1.5 7)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<tuple<f32,s64>, enum> host-op arg crosses on the Ok arm (f32 FIRST element, reinterpret join)"
  (doc
    "SHAPE 212 (v-wit-boundary) — the f32 twin of SHAPE 211. For an `f32` in slot 0 the join is
           `join(f32,i32)=i32`, so the slot-0 join stays `i32`: the Ok arm emits `I32ReinterpretF32`, the Err arm
           stores the err disc directly (no widen). Slot 1 (`s64`) is `i64`. Core `(param i32 i32 i64)`. run()
           emits `(Ok #tuple((: 1.5 Float32) 7))` (the `Float32` annotation pins the literal width). A VALID
           running component (live-objects=0) is the pin. Completes the `result<tuple>` float-slot-0 reinterpret join.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (tuple (f32) (s64)) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Tuple Float32 Int64) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #tuple((: 1.5 Float32) 7)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<record{a:f64,b:s64}, enum> host-op arg crosses on the Ok arm (float slot-0 field, reinterpret join)"
  (doc
    "SHAPE 213 (v-wit-boundary) — a FLOAT in the WIT-first (slot-0) field of a `result<record, enum>` arg, the
           record counterpart of the tuple float-slot-0 (SHAPE 211) and the scalar (SHAPE 209). Only slot 0 joins
           the `i32` err disc, so the WIT-first field, when a float, bit-reinterprets into the integer slot-0 join
           — `join(f64,i32)=i64` — while the `s64` field keeps its own `i64`. Here the WIT order `(a, b)` equals
           name-lex `(a, b)` so no reorder; slot 0 = `a:f64`. Core `(param i32 i64 i64)`:
           `emit_result_record_arg_reg_flatten` overrides `slot_vts[0]` to the join int and emits
           `I64ReinterpretF64` at the k==0 reverse-capture (Ok arm), `serialize` emits the join int for slot 0.
           run() emits `(Ok #record((= a 1.5) (= b 7)))`; a VALID running component (live-objects=0) is the pin.
           The reorder case (float WIT-first but name-lex-second) = SHAPE 214.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= a (f64)) (= b (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: a Float64) (: b Int64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= a 1.5) (= b 7))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a bare result<record, enum> host-op arg with a float WIT-first field reordered from name-lex-second crosses (reinterpret at the reordered slot 0)"
  (doc
    "SHAPE 214 (v-wit-boundary) — the RECORD-distinguishing case: the float lands in slot 0 by WIT REORDER, not
           by declaration position. The guest record is `{a:s64, b:f64}` (name-lex order a, b) but the host WIT
           declares `(record (= b (f64)) (= a (s64)))`, so `emit_result_record_arg_reg_flatten` reorders the fields
           to WIT order — slot 0 = `b:f64` (the name-lex-SECOND field), slot 1 = `a:s64`. The reinterpret join
           targets the REORDERED slot 0: `slot_vts[0]` (WIT order) is the `f64`, overridden to the `i64` join, and
           the k==0 capture (which pops the first WIT field `emit_record_arg_marshal` pushed) reinterprets it. Core
           `(param i32 i64 i64)`. run() emits `(Ok #record((= a 7) (= b 1.5)))`; a VALID running component
           (live-objects=0) is the pin — proving the float reinterpret follows WIT slot-0, not name-lex position.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= b (f64)) (= a (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Er (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: a Int64) (: b Float64)) Er) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= a 7) (= b 1.5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 42 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 42)
  (live-objects 0))

(case
  "a RECORD host-op arg with a result<bytes,enum> FIELD marshals the Ok arm (guest -> host ARG direction)"
  (doc
    "SHAPE 215 (v-wit-boundary) — a RECORD host-op ARGUMENT (probe.push : func(record{a: result<list<u8>,
           enum{timeout, missing}>, k: s64}) -> s64) whose FIELD is a `result<bytes, enum>`. Exercises
           `emit_record_arg_marshal`'s `RecordFieldAbi::Result` field arm in the GUEST -> HOST arg direction — the
           arm is fully implemented but was previously reached only in the HOST -> GUEST export/lift direction (the
           `result<bytes,enum>` record-PARAM lift). The field flattens to `(disc, p0, p1)`: on Ok the Bytes payload
           is rope->mem-copied at the reserved scratch cursor giving `(0, ptr, len)`; on Err the payloadless enum's
           disc + a 0-pad give `(1, enum-disc, 0)`. So the whole record arg flattens to `(a-disc, p0, p1, k)` = 4
           core slots. run() builds {a: Ok(b\"hi\"), k: 5} and performs probe.push; a VALID component that runs
           (live-objects=0) is the pin — a wrong slot count/order (e.g. treating the Bytes handle as one slot) fails
           component validation. Complements the export-side result-field lift (SHAPE at the guest-param direction)
           with the guest->host ARG marshal. The cursor RESERVATION is load-bearing: the emit.rs `has_runtime_compound`
           pre-scan must reserve the scratch cursor for a record ARG with a `result` field (`record_has_result_field`)
           — the Ok-arm rope->mem copy consumes it, and a missing reservation PANICS the marshal's `cursor.expect(...)`
           (the bug this case first exposed).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= a (result (list (u8)) (enum timeout missing))) (= k (s64)))) (result (s64)))))))
  (input
    (do
      (type Er2 (Timeout) (Missing))
      (effect probe (op push (-> (Record (: a (Result Bytes Er2)) (: k Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= a (Ok b"hi")) (= k 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<record-with-a-result<bytes,enum>-field> host-op arg crosses on the Some arm"
  (doc
    "SHAPE 216 (v-wit-boundary) — a top-level `option<record{a: result<list<u8>, enum>, k: s64}>` bare host-op
           ARGUMENT (probe.push), the Some arm. Composes the option<record> arg (SHAPE 130 family) with the
           record `result<bytes,enum>` FIELD marshal (SHAPE 215): `option_arg_crosses` admits the payload record
           via `is_boundary_record` (its result field crosses through `field_boundary_abi`'s Result arm), and
           `emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`, whose Result-field arm
           rope->mem-copies the Ok payload at the reserved scratch cursor. The cursor RESERVATION is load-bearing
           and rides the SAME `record_has_result_field` pre-scan predicate the direct-record arg uses — the emit.rs
           `has_runtime_compound` gate checks it on the option payload record too (a missing reservation panics the
           marshal's `cursor.expect(...)`). On Some the arg flattens to `(opt-disc, a-disc, p0, p1, k)`. run()
           builds Some({a: Ok(b\"hi\"), k: 5}) and performs probe.push; a VALID running component (live-objects=0)
           is the pin. Complements SHAPE 215 (the direct-record twin).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= a (result (list (u8)) (enum timeout missing))) (= k (s64))))) (result (s64)))))))
  (input
    (do
      (type Er2 (Timeout) (Missing))
      (effect probe (op push (-> (Option (Record (: a (Result Bytes Er2)) (: k Int64))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= a (Ok b"hi")) (= k 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a bare result<record-with-a-result<bytes,enum>-field, enum> host-op arg crosses on the Ok arm (doubly-nested result)"
  (doc
    "SHAPE 217 (v-wit-boundary) — a top-level `result<record{a: result<list<u8>, enum>, k: s64}, enum>` bare
           host-op ARGUMENT (probe.push), the Ok arm. Composes the `result<record, enum>` arg (SHAPE 189/202) with
           the record `result<bytes,enum>` FIELD marshal (SHAPE 215): the OUTER result's Ok arm carries a record
           whose `a` field is ITSELF a `result<bytes,enum>` (doubly-nested). `result_record_enum` admits the Ok
           record via `is_boundary_record` (its result field crosses through `field_boundary_abi`'s Result arm), and
           `emit_result_record_arg_reg_flatten` recurses `emit_record_arg_marshal`, whose Result-field arm
           rope->mem-copies the INNER Ok payload at the reserved scratch cursor. The cursor RESERVATION rides
           `record_has_result_field` on the `result<record>` Ok payload (added in #9923 — a missing reservation
           panics the marshal's `cursor.expect(...)`). run() builds `(Ok #record((= a (Ok b\"hi\")) (= k 5)))` and
           performs probe.push; a VALID running component (live-objects=0) is the pin. Completes the trio of
           `record_has_result_field` cursor sites: direct record (SHAPE 215), option<record> (SHAPE 216), and this
           result<record> Ok.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (result (record (= a (result (list (u8)) (enum timeout missing))) (= k (s64))) (enum bad worse))) (result (s64)))))))
  (input
    (do
      (type Inner (Timeout) (Missing))
      (type Outer (Bad) (Worse))
      (effect probe (op push (-> (Result (Record (: a (Result Bytes Inner)) (: k Int64)) Outer) Int64)))
      (def (run) (host (probe) (probe.push (Ok #record((= a (Ok b"hi")) (= k 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<option<bytes>> host-op arg crosses on the outer+inner Some arm (nested-option mem payload)"
  (doc
    "SHAPE 218 (v-wit-boundary) — a top-level `option<option<list<u8>>>` bare host-op ARGUMENT (probe.push),
           outer+inner Some. Widens the nested-option arg (SHAPE at option<option<scalar>>) from a SCALAR inner
           payload to a `Bytes` inner: `option_arg_crosses` admits it (the nested-option arm now accepts a Bytes
           inner), and `emit_option_reg_flatten`'s nested-option branch computes the inner option's flatten width
           DYNAMICALLY — `(inner-disc:i32, ptr:i32, len:i32)` for a Bytes inner (vs `(inner-disc, scalar)` for a
           scalar), recursing into the inner option<bytes> branch which copies the rope into `mem` at the threaded
           cursor and advances it. So the arg flattens to `(outer-disc, inner-disc, ptr, len)` = 4 core slots
           `(param i32 i32 i32 i32)`. The cursor RESERVATION rides the emit.rs pre-scan's new option<option<bytes>>
           check (a missing reservation panics the inner branch's `cursor.expect(...)`). run() builds
           Some(Some(b\"hi\")) and performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (list (u8))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option Bytes)) Int64)))
      (def (run) (host (probe) (probe.push (Some (Some b"hi")))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<option<list<s64>>> host-op arg crosses on the outer+inner Some arm (nested-option list-mem payload)"
  (doc
    "SHAPE 219 (v-wit-boundary) — a top-level `option<option<list<s64>>>` bare host-op ARGUMENT (probe.push),
           outer+inner Some. Widens the nested-option arg (SHAPE 218 did the Bytes inner) to a `list<T>` inner:
           `option_arg_crosses` admits it (the nested-option arm now accepts a `list` inner whose ELEMENT crosses
           via `field_boundary_abi`), the classifier builds `Option(Option(List(Scalar)))`, and
           `emit_option_reg_flatten`'s nested-option branch computes the inner option's flatten width DYNAMICALLY —
           `(inner-disc:i32, ptr:i32, count:i32)` for a list inner (the SAME 3-slot shape as a Bytes inner),
           recursing into the inner option<list> branch which marshals the payload list into `mem` at the threaded
           cursor via `emit_list_arg_marshal`. So the arg flattens to `(outer-disc, inner-disc, ptr, count)` = 4
           core slots. `collect_used_ops` declares the inner list's `vec-len`/`vec-get` + element ops (a list inner
           has no single get-op, so it is handled explicitly, else CDZ0910). run() builds Some(Some([1,2,3])) and
           performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (list (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option (List Int64))) Int64)))
      (def (run) (host (probe) (probe.push (Some (Some #list(1 2 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<option<tuple<s64,s64>>> host-op arg crosses on the outer+inner Some arm (nested-option product payload)"
  (doc
    "SHAPE 220 (v-wit-boundary) — a top-level `option<option<tuple<s64,s64>>>` bare host-op ARGUMENT (probe.push),
           outer+inner Some. Generalizes the nested-option arg (SHAPE 218/219 did Bytes/list inners) to a PRODUCT
           inner via a variable-width flatten: `emit_option_reg_flatten`'s nested-option branch now derives the inner
           option's flatten GENERICALLY from `field_boundary_abi(option<tuple<s64,s64>>)` = `Option(Tuple([s64,s64]))`
           → `flatten_record_field_abi` → `(inner-disc:i32, s64, s64)` = 3 slots (i32, i64, i64). So the arg flattens
           to `(outer-disc, inner-disc, e0, e1)` = 4 core slots `(param i32 i32 i64 i64)`. An all-scalar tuple inner
           writes NOTHING to `mem` (no cursor). `option_arg_crosses` + the classifier + `collect_used_ops` all admit
           it via the single `field_boundary_abi(inner-option).is_some()` gate. run() builds Some(Some((2,3))) and
           performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (tuple (s64) (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option (Tuple Int64 Int64))) Int64)))
      (def (run) (host (probe) (probe.push (Some (Some #tuple(2 3))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<option<record{a,b}>> host-op arg crosses on the outer+inner Some arm (nested-option record payload)"
  (doc
    "SHAPE 221 (v-wit-boundary) — a top-level `option<option<record{a: s64, b: s64}>>` bare host-op ARGUMENT
           (probe.push), outer+inner Some. The record twin of SHAPE 220: the nested-option branch derives the inner
           option's flatten from `field_boundary_abi(option<record{a,b}>)` = `Option(Record([a,b]))` →
           `(inner-disc:i32, a:s64, b:s64)` = 3 slots. The inner recursion (`emit_option_reg_flatten`'s record
           branch) reads each field WIT-ordered; an all-scalar record writes nothing to `mem` (no cursor). Arg
           flattens to `(outer-disc, inner-disc, a, b)`. run() builds Some(Some({a: 1, b: 2})) and performs
           probe.push; a VALID running component (live-objects=0) is the pin. Completes the top-level nested-option
           family (scalar/bytes/list/tuple/record inner) via the single generic `field_boundary_abi` flatten.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (option (record (= a (s64)) (= b (s64)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Option (Option (Record (: a Int64) (: b Int64)))) Int64)))
      (def (run) (host (probe) (probe.push (Some (Some #record((= a 1) (= b 2)))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with an option<option<bytes>> FIELD crosses on the outer+inner Some arm (nested-option record FIELD)"
  (doc
    "SHAPE 222 (v-wit-boundary) — a RECORD host-op ARGUMENT (probe.push : func(record{a: option<option<list<u8>>>,
           k: s64}) -> s64) whose field `a` is a nested `option<option<bytes>>`. Extends the TOP-LEVEL nested-option
           family (SHAPE 218–221) to the record-FIELD position: `field_boundary_abi`'s nested-option arm now admits
           ANY inner that crosses (building `Option(Option(Bytes))`), so `is_boundary_record` admits the record, and
           `emit_record_arg_marshal`'s nested-option field arm DELEGATES to the shared `emit_option_reg_flatten`
           (which derives the inner flatten from the abi + copies the inner Bytes rope into `mem` at the reserved
           cursor). The `a` field flattens to `(a-outer-disc, a-inner-disc, ptr, len)`; the whole arg to that + `k`.
           The cursor RESERVATION rides `record_has_option_field_needing_mem`'s new nested-option clause (reserve iff
           the inner option's abi `record_field_abi_needs_memory`). run() builds {a: Some(Some(b\"hi\")), k: 5} and
           performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= a (option (option (list (u8))))) (= k (s64)))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Record (: a (Option (Option Bytes))) (: k Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= a (Some (Some b"hi"))) (= k 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<option<option<bytes>>, s64> host-op arg crosses (nested-option TUPLE element)"
  (doc
    "SHAPE 223 (v-wit-boundary) — a top-level `tuple<option<option<list<u8>>>, s64>` bare host-op ARGUMENT
           (probe.push). Extends the nested-option family (SHAPE 218–222) to the TUPLE-ELEMENT position — which
           needs NO new code: `tuple_arg_crosses` admits the element via `option_arg_crosses` (generalized to any
           inner in #9933), `emit_tuple_reg_flatten`'s option-element arm DELEGATES to the shared
           `emit_option_reg_flatten` (which derives the inner flatten + copies the inner Bytes rope into `mem` at the
           cursor), `tuple_arg_needs_cursor` recurses nested options to reserve the cursor for the Bytes leaf, and
           `collect_used_ops`' tuple-element option arm recurses `collect_record_field_ops` (declaring the inner
           `bytes-len`/`bytes-get`). This case PINS that the composition works. The element flattens to
           `(outer-disc, inner-disc, ptr, len)` positionally, then `s64`. run() builds (Some(Some(b\"hi\")), 5) and
           performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (option (option (list (u8)))) (s64))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (Tuple (Option (Option Bytes)) Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Some (Some b"hi")) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a typed reducer performing a list<option<option<s64>>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 224 (v-wit-boundary) — a top-level `list<option<option<s64>>>` bare host-op ARGUMENT (probe.push).
           Extends the nested-option family (SHAPE 218–223) to the LIST-ELEMENT position: each element is an
           `option<option<s64>>` written IN PLACE into the backing array by `emit_option_to_mem`, whose new
           nested-option arm RECURSES itself on the inner option — on the outer Some it fetches the inner option
           (`sum-payload`) and writes it at `dest + payload_off`, the inner recursion writing the inner disc byte +
           the inner scalar payload inline (or its zero on inner None); on the outer None the payload area is left
           unwritten. The element detector (`option_elem`), the representability gate (`list_elem_marshalable`'s
           option arm), and `collect_list_elem_ops` all gained the same nested-option recursion, so a
           `list<option<option<scalar>>>` arg is admitted and every op it calls is declared (no CDZ0910). run()
           builds [Some(Some(5)), Some(None), None] — exercising all three states (outer+inner Some, outer Some /
           inner None, outer None) — and performs probe.push; a VALID running component (live-objects=0) is the pin.
           Closes the last nested-option-family shape (the list-element position).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (option (s64))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (Option Int64))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some (Some 5)) (Some None) None))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a typed reducer performing a list<option<option<bytes>>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 225 (v-wit-boundary) — a top-level `list<option<option<list<u8>>>>` bare host-op ARGUMENT (probe.push).
           Widens the nested-option list element (SHAPE 224 did the SCALAR inner) to a BYTES inner. It needs NO new
           code: `emit_option_to_mem`'s nested-option arm (SHAPE 224) recurses on the inner option, whose Bytes arm
           copies the payload rope into `mem` at the running spill cursor and writes `(ptr, len)` at the inner
           payload offset; the list-arg pre-scan reserves that cursor UNCONDITIONALLY for any `Ty::List` arg
           (emit.rs `has_runtime_compound`), so the Bytes spill has a cursor; `list_elem_marshalable`'s option arm +
           the `option_elem` detector admit the nested-option-bytes element (the inner option is itself a
           marshalable element); and `collect_list_elem_ops`' nested-option recursion reaches the inner Bytes arm,
           declaring `bytes-len`/`bytes-get` (else CDZ0910). This case PINS that the Bytes-inner composition works.
           run() builds [Some(Some(b\"hi\")), Some(None), None] — all three states — and performs probe.push; a VALID
           running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (option (list (u8)))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (Option Bytes))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some (Some b"hi")) (Some None) None))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a typed reducer performing a list<option<option<record{a,b}>>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 226 (v-wit-boundary) — a top-level `list<option<option<record{a: s64, b: s64}>>>` bare host-op ARGUMENT
           (probe.push). Widens the nested-option list element (SHAPE 224 scalar / SHAPE 225 bytes inner) to a
           PRODUCT (record) inner, closing the nested-option-in-list family. It needs NO new code: the SHAPE 224
           nested-option arm in `emit_option_to_mem` recurses on the inner `option<record>`, which falls through to
           the existing RECORD arm (`emit_record_to_mem`) writing the product IN PLACE at the inner payload offset
           (each field WIT-ordered from the element's `option<option<record>>` WIT); an all-scalar record writes
           nothing extra to `mem` (the list-arg pre-scan still reserves the cursor). `list_elem_marshalable`'s option
           arm + the `option_elem` detector admit the nested-option-record element (the inner option is itself a
           marshalable element whose record fields are `product_field_marshalable`), and `collect_list_elem_ops`'
           nested-option recursion reaches the inner Record arm (`arr-get` per field + each field's ops). This case
           PINS the compound-inner composition. run() builds [Some(Some({a:1,b:2})), Some(None), None] — all three
           states — and performs probe.push; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (option (option (record (= a (s64)) (= b (s64))))))) (result (s64)))))))
  (input
    (do
      (effect probe (op push (-> (List (Option (Option (Record (: a Int64) (: b Int64))))) Int64)))
      (def (run) (host (probe) (probe.push #list((Some (Some #record((= a 1) (= b 2)))) (Some None) None))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(bytes)} as the direct host-op arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 227 (v-wit-boundary) — a `variant{a, b(list<u8>)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the FIRST compound-payload variant ARG. Unlike the scalar-payload bare variant (SHAPE 53,
           `HostParam::Variant` → the nominal-`AbiValType` builder), a Bytes payload cannot be expressed by
           `AbiValType`, so this crosses via the NEW additive `HostParam::VariantBytes`: the component `variant`
           DEFINED type is laid STRUCTURALLY from the declared WIT (`add_wit_type_deduped` → `CDef::Variant` with a
           `(list u8)` payload case, export-remapped like a record), and the guest flattens to `(disc:i32, ptr:i32,
           len:i32)` — the SAME 3-slot shape as `result<list<u8>, enum>` (a result IS the 2-case Ok(bytes)/Err(enum)
           special case) but at arbitrary case discs — via `emit_variant_bytes_arg_reg_flatten`: on the Bytes case
           (disc ∈ bytes-discs) it copies the payload rope into `mem` at the reserved cursor and pushes
           `(disc, ptr, len)`; on a nullary case pushes `(disc, 0, 0)`. Mirrors the result-family additively across
           ~11 sites (detector `variant_bytes_payload_cases`, HostParam, classifier, first_unrepresentable, marshal,
           emit dispatch + reclaim, serialize, host_imports structural CRef, used_ops, set_needs_memory, cursor
           reservation). run() performs TWO pushes — `(B b\"hi\")` the Bytes case then `A` the nullary case —
           exercising BOTH marshal arms; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (u8))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Bytes))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B b"hi")) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)) (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push) (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(bytes), c(bytes)} with TWO bytes cases as the direct host-op arg emits, loads, and runs"
  (doc
    "SHAPE 228 (v-wit-boundary) — a `variant{a, b(list<u8>), c(list<u8>)}` bare TOP-LEVEL host-op ARGUMENT
           (probe.push) with TWO Bytes-payload cases. Hardens the `HostParam::VariantBytes` path SHAPE 227 landed:
           SHAPE 227 had a single Bytes case (`bytes_discs = [1]`), so the marshal's multi-disc OR (`is_bytes =
           OR over bytes_discs of disc == bd`, the `k > 0 → i32.or` fold in `emit_variant_bytes_arg_reg_flatten`)
           was UNEXERCISED. Here `bytes_discs = [1, 2]`, so the guest must recognize BOTH case discs as Bytes
           cases and copy the rope for each (a wrong OR — e.g. matching only the first disc — would push `(2, 0, 0)`
           for the C case and drop its payload). run() performs THREE pushes — `(B b\"hi\")` (disc 1),
           `(C b\"yo\")` (disc 2), and `A` (nullary, disc 0) — exercising both Bytes discs and the nullary arm; a
           VALID running component (live-objects=0) is the pin. The declared `variant` DEFINED type carries two
           `(list u8)` payload cases, laid structurally via `add_wit_type_deduped`.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (u8))) (c (list (u8))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Bytes) (C Bytes))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B b"hi")) (probe.push (V.C b"yo")) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(list<s64>)} as the direct host-op arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 229 (v-wit-boundary) — a `variant{a, b(list<s64>)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the FIRST list-payload variant ARG (the `list` sibling of the Bytes-payload SHAPE 227).
           Crosses via the NEW additive `HostParam::VariantList`: the component `variant` DEFINED type is laid
           STRUCTURALLY from the declared WIT (`add_wit_type_deduped` → `CDef::Variant` with a `(list s64)` payload
           case, export-remapped), and the guest flattens to `(disc:i32, ptr:i32, count:i32)` — the SAME 3-slot
           shape as `HostParam::VariantBytes`, but on a list case `emit_variant_list_arg_reg_flatten` MARSHALS the
           payload list into `mem` at the reserved cursor via `emit_list_arg_marshal` (`vec-len`/`vec-get` + the
           scalar element) and pushes `(disc, ptr, count)`; on a nullary case pushes `(disc, 0, 0)`. Mirrors the
           VariantBytes family additively (detector `variant_list_payload_cases` requiring a shared scalar element,
           HostParam, classifier, first_unrepresentable, marshal, emit dispatch + reclaim, serialize, host_imports
           structural CRef, used_ops, set_needs_memory, cursor reservation). run() performs TWO pushes —
           `(B [1,2,3])` the list case then `A` the nullary case — exercising BOTH arms; a VALID running component
           (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B (List Int64)))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B #list(1 2 3))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)) (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push) (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(tuple<s64,s64>)} as the direct host-op arg emits, loads, and runs (via an imposed WIT world)"
  (doc
    "SHAPE 230 (v-wit-boundary) — a `variant{a, b(tuple<s64, s64>)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the FIRST PRODUCT-payload variant ARG. Crosses via the NEW additive `HostParam::VariantTuple`:
           the component `variant` DEFINED type is laid STRUCTURALLY from the declared WIT (`add_wit_type_deduped` →
           `CDef::Variant` with a `(tuple s64 s64)` payload case, export-remapped), and the guest flattens
           POSITIONALLY to `(disc:i32, e0:i64, e1:i64)` — the discriminant then the tuple's elements INLINE — via
           `emit_variant_tuple_arg_reg_flatten` (the register twin of the `result<tuple,enum>` Ok flatten MINUS the
           err-disc/float-join): on the tuple case it recurses `emit_tuple_reg_flatten` to fill the element slots; on
           the nullary case it zero-fills ALL payload slots. All-scalar tuple → NO `mem`/cursor (register-only).
           Mirrors the VariantBytes/List family additively but with a VARIABLE slot count derived from the element
           ABIs (detector `variant_tuple_payload_case`, HostParam carrying the tuple disc + element ABIs, classifier,
           first_unrepresentable, marshal, emit dispatch + reclaim, serialize positional flatten, host_imports
           structural CRef, used_ops). run() performs TWO pushes — `(B (2,3))` the tuple case then `A` the nullary
           case — exercising BOTH arms; a VALID running component (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (tuple (s64) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B (Tuple Int64 Int64)))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B #tuple(2 3))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)) (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push) (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(record{x: s64, y: bool})} as the direct host-op arg emits, loads, and runs (imposed WIT)"
  (doc
    "SHAPE 231 (v-wit-boundary) — a `variant{a, b(record{x: s64, y: bool})}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push), the RECORD-payload variant ARG (the near-twin of the tuple-payload SHAPE 230).
           Crosses via the NEW additive `HostParam::VariantRecord`: the component `variant` DEFINED type is laid
           STRUCTURALLY from the declared WIT (`add_wit_type_deduped` → `CDef::Variant` with a `(record …)` payload
           case, export-remapped), and the guest flattens POSITIONALLY to `(disc:i32, x:i64, y:i32)` — the
           discriminant then the record's fields in WIT declaration order — via `emit_variant_record_arg_reg_flatten`
           (recurses `emit_record_arg_marshal`, which reads each field WIT-ordered; a nullary case zero-fills ALL
           payload slots). DISTINCT field widths (s64 then bool) pin the positional slot widths — a wrong flatten
           (mis-ordered / mis-widthed slots) fails component instantiation. All-scalar record → NO `mem`/cursor.
           Mirrors the VariantTuple family additively but with the record field-abi build + WIT reorder
           (`reorder_record_fields_to_wit`, the SAME helper the `result<record,enum>` arg uses). run() performs TWO
           pushes — `(B {x:7, y:#true})` the record case then `A` the nullary case — exercising BOTH arms; a VALID
           running component (live-objects=0) is the pin. Completes the single-compound-payload variant ARG family
           (scalar/bytes/list/tuple/record).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (record (= x (s64)) (= y (bool)))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B (Record (: x Int64) (: y Bool))))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B #record((= x 7) (= y true)))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)) (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push) (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE MIXED variant{a, b(s64), c(bytes)} as the direct host-op arg emits, loads, and runs (imposed WIT)"
  (doc
    "SHAPE 232 (v-wit-boundary) — a `variant{a, b(s64), c(list<u8>)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the FIRST MIXED (heterogeneous) tagged-union: it MIXES a scalar payload case (b: s64) with
           a Bytes payload case (c: list<u8>), the canonical variant use. Crosses via the NEW additive
           `HostParam::VariantMixed`: the component `variant` DEFINED type is laid STRUCTURALLY from the declared
           WIT (`add_wit_type_deduped`), and the guest flattens to the canonical variant JOIN `[disc:i32] ++
           position-wise-join(payload flattens)` = `(i32, i64, i32)` — b's `[i64]` joined with c's `[i32 ptr, i32
           len]` gives slot0=`join(i64,i32)=i64`, slot1=`i32` (`host::variant_mixed_join_slots`, matching
           `wit_ctype::flatten_variant`). `emit_variant_mixed_arg_reg_flatten` DISPATCHES per case (a nested
           `if disc==d … else …` chain): b → unbox s64 into slot0 (i64), slot1=0; c → rope-copy the Bytes into
           `mem` at the cursor, slot0 = ptr EXTENDED to i64, slot1 = len; a (nullary) → zero both slots. run()
           performs THREE pushes — `(B 42)` the scalar case, `(C b\"hi\")` the bytes case, `A` the nullary case —
           exercising ALL THREE arms of the join; a VALID running component (live-objects=0) is the pin. This is the
           hardest variant flatten (mixed widths + per-case dispatch + slot-0 width coercion).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (u8))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Bytes))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C b"hi")) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(s64), c(f64)} mixing an int and a float scalar case crosses as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 233 (v-wit-boundary) — a `variant{a, b(s64), c(f64)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the FIRST int↔float REINTERPRET-JOIN scalar variant. The uniform scalar-variant path
           (`HostParam::Variant`) DECLINES a payload set mixing an integer with a float (its join has no clean
           slot); the NEW additive `HostParam::VariantScalarsMixed` handles it via the canonical reinterpret
           join. b's payload flattens to `[i64]`, c's to `[f64]`; the canonical `wit_ctype::flatten_variant` join
           of the ONE payload slot is `join(i64, f64) = i64` (`reinterpret_join_vt`), so the core flatten is
           `(disc:i32, i64)`. `emit_variant_mixed_scalar_arg_reg_flatten` DISPATCHES per case (a nested
           `if disc==d … else …` chain), each unboxing with ITS OWN read op and coercing into the shared i64
           slot: b → `get-int` (i64, no coercion); c → `get-float` (f64) then `i64.reinterpret_f64` (the float
           bit-reinterprets into the integer slot); a (nullary) → the i64 zero. run() performs THREE pushes —
           `(B 42)`, `(C 3.5)`, `A` — exercising all three arms; the component TYPE-CHECKS the flatten against the
           declared `variant` (a missing reinterpret would leave an `f64` where the `i64` slot is required → a
           validation error), so a VALID running component (live-objects=0) pins the reinterpret coercion.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (f64)))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Float64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C 3.5)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(s32), c(f32)} int↔float mix joining to an i32 slot crosses as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 234 (v-wit-boundary) — a `variant{a, b(s32), c(f32)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the SAME-WIDTH-32 twin of SHAPE 233 exercising the i32 join slot. b's payload flattens to
           `[i32]` (a narrow int), c's to `[f32]`; the canonical join is `join(i32, f32) = i32`, so the core
           flatten is `(disc:i32, i32)`. Per-case coercion into the i32 slot: b → `get-int` (i64, NORMALIZED) then
           `i32.wrap_i64` (narrow int → i32 slot); c → `get-float32` (f32) then `i32.reinterpret_f32` (the float
           bit-reinterprets into the i32 slot); a (nullary) → the i32 zero. Pins the i32-slot reinterpret path +
           the narrow-int wrap (distinct from SHAPE 233's i64 slot). VALID running component = the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s32)) (c (f32)))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int32) (C Float32))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B (: 42 Int32))) (probe.push (V.C (: 3.5 Float32))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(s64), c(f32)} cross-width int↔float mix widening to an i64 slot crosses (imposed WIT)"
  (doc
    "SHAPE 235 (v-wit-boundary) — a `variant{a, b(s64), c(f32)}` passed BARE as the TOP-LEVEL host-op ARGUMENT
           (probe.push), the CROSS-WIDTH int↔float mix pinning the trickiest coercion. b's payload flattens to
           `[i64]`, c's to `[f32]`; the canonical join is `join(i64, f32) = i64` (the `else` arm — an `f32` mixed
           with an `i64` widens to `i64`, NOT the f32/i32 same-width pairing), so the core flatten is
           `(disc:i32, i64)`. Per-case coercion into the i64 slot: b → `get-int` (i64, no coercion); c →
           `get-float32` (f32) then `i32.reinterpret_f32` then `i64.extend_i32_u` (the f32 bits reinterpret to i32,
           then zero-extend into the i64 slot — the two-step canonical coercion); a (nullary) → the i64 zero.
           Pins the f32→i64 two-step coercion arm, the last distinct reinterpret path. VALID running component = the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (f32)))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Float32))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C (: 3.5 Float32))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE variant{a, b(s64, s64)} with a MULTI-PAYLOAD case crosses as a host-op arg (tuple payload, imposed WIT)"
  (doc
    "SHAPE 236 (v-wit-boundary) — a `variant{a, b(s64, s64)}` whose payload-bearing case `b` carries TWO payloads
           (a MULTI-payload ctor `(B x y)`, NOT a single tuple-typed payload `(B (Tuple x y))`), passed BARE as the
           TOP-LEVEL host-op ARGUMENT. At the value-heap level a multi-payload variant case stores its payloads as a
           TUPLE handle (core.rs: `sum-payload` yields the payload array, `arr-get i` indexes it) and
           `variant_payload_ty_at` SYNTHESIZES the tuple of the ctor's payload types — so it is REPRESENTATIONALLY
           IDENTICAL to the single-tuple-payload variant (SHAPE 230) and crosses via the SAME `HostParam::VariantTuple`
           machinery: the WIT declares the case with a `(tuple (s64) (s64))` payload, and the guest flattens
           POSITIONALLY to `(disc:i32, e0:i64, e1:i64)` via `emit_variant_tuple_arg_reg_flatten`. The ONLY code change
           is relaxing `variant_tuple_payload_case` to admit a multi-payload case (n>=2) — the marshal / serialize /
           used_ops / host_imports all derive from the synthesized tuple type unchanged. run() pushes `(B 2 3)` (the
           multi-payload case) then `A` (nullary) — both arms; a VALID running component (live-objects=0) is the pin.
           A multi-payload case with a compound (bytes/list/nested) element still declines (all-scalar this increment).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (tuple (s64) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 2 3)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)) (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push) (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a Qty-over-Int64 host-op ARG crosses as its ERASED inner scalar (imposed WIT)"
  (doc
    "SHAPE 237 (v-wit-boundary) — a `(Qty Int64 (Unit.base \"meter\"))` unit-of-measure quantity passed as a
           host-op ARGUMENT. A `Qty` type ERASES to its inner numeric at codegen (`(Qty.of 5 meter)` is
           byte-identical to bare `5`), so `abi_val_type` peels `Qty{inner}` to the inner scalar and the arg
           crosses as `HostParam::Scalar(S64)` — the WIT declares the erased `s64`. This pins the WIRED-but-
           UNTESTED `Qty`-over-scalar boundary behavior (no code — the peel was already wired); a future change
           mishandling `Qty` at the boundary now reds. run() passes `(Qty.of 5 meter)`; the host returns 55.")
  (wit-world (world w (import cadenza:platform/probe
    (member push (func (param m (s64)) (result (s64)))))))
  (input (do
    (effect probe (op push (-> (Qty Int64 (Unit.base #"meter")) Int64)))
    (def (run) (host (probe) (probe.push (Qty.of 5 (Unit.base #"meter")))))
    (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a Qty-over-Int64 host-op RESULT crosses as its erased inner scalar and lifts back to a Qty (imposed WIT)"
  (doc
    "SHAPE 238 (v-wit-boundary) — a host op whose RESULT is `(Qty Int64 (Unit.base \"meter\"))`. The result
           crosses as the erased inner `s64` (the host returns a plain scalar) and lifts back into a `Qty`-typed
           value on the guest (erased = the i64, no wrapper). `Qty.value` reads the magnitude. Pins the flagged
           WIRED-but-UNTESTED `Qty`-over-scalar RESULT path (the result-side twin of SHAPE 237, no code).")
  (wit-world (world w (import cadenza:platform/probe
    (member measure (func (param m (s64)) (result (s64)))))))
  (input (do
    (effect probe (op measure (-> Int64 (Qty Int64 (Unit.base #"meter")))))
    (def (run) (host (probe) (Qty.value (probe.measure 0))))
    (export run)))
  (call run)
  (host-responses (respond probe.measure (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.measure))
  (output 55)
  (live-objects 0))

(case
  "a Qty-over-Float64 host-op ARG crosses as its erased inner f64 scalar (imposed WIT)"
  (doc
    "SHAPE 239 (v-wit-boundary) — the FLOAT-inner twin of SHAPE 237: a `(Qty Float64 (Unit.base \"meter\"))`
           quantity host-op ARG erases to its inner `Float64` and crosses as `HostParam::Scalar(F64)` (WIT `f64`).
           Confirms the `Qty` peel is width-faithful across the numeric tower (int + float inner). run() passes
           `(Qty.of 2.5 meter)`; the host returns 55.")
  (wit-world (world w (import cadenza:platform/probe
    (member push (func (param m (f64)) (result (s64)))))))
  (input (do
    (effect probe (op push (-> (Qty Float64 (Unit.base #"meter")) Int64)))
    (def (run) (host (probe) (probe.push (Qty.of (: 2.5 Float64) (Unit.base #"meter")))))
    (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a WIT flags host-op ARG crosses as a packed bitset from a guest record-of-bools (imposed WIT)"
  (doc
    "SHAPE 240 (v-wit-boundary) — a WIT `flags{read,write,execute}` HOST-OP ARGUMENT (import side, imposed
           wit-world), the IMPORT twin of the export-side flags PARAM (SHAPE 113). The guest models flags as a
           PRODUCT record-of-bools (operator ruling), so the arg is a `Record{read:Bool, write:Bool,
           execute:Bool}` whose imposed WIT param is `flags`. Before this it DECLINED (`a record host-arg has no
           matching WIT record type` — the classifier made HostParam::Record, found no WIT record because the
           world says flags). Now the NEW additive `HostParam::Flags` PACKS the bools into a `ceil(n/32)`-word
           bitset: the classifier consults `wit_params[arg_i]` (`flags`) + `host::flags_field_bits` (each bool
           field's kebab name → its WIT-label bit index, the PACK inverse of `param_field`'s flags-UNPACK), the
           guest marshals via `select::emit_flags_arg_pack` (per field `arr-get`+`get-bool`, shift into its bit,
           OR into the word), the component declares a nominal `flags` DEFINED type (build_host_group's
           flags_params, the enum-like single-leaf path), and serialize flattens to ONE i32 (≤32 labels). run()
           passes `#record((= read true) (= write false) (= execute true))`; a VALID running component that links
           against the imposed `flags` world (live-objects=0) is the pin. >32 labels / a non-bool field declines.")
  (wit-world (world w (import cadenza:platform/probe
    (member push (func (param m (flags read write execute)) (result (s64)))))))
  (input (do
    (effect probe (op push (-> (Record (: read Bool) (: write Bool) (: execute Bool)) Int64)))
    (def (run) (host (probe) (probe.push #record((= read true) (= write false) (= execute true)))))
    (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a BARE MIXED variant{a, b(s64), c(list<s64>)} mixing a scalar and a LIST case as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 241 (v-wit-boundary) — a `variant{a, b(s64), c(list<s64>)}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push), the MIXED (heterogeneous) tagged-union that mixes a SCALAR payload case (b: s64)
           with a LIST payload case (c: list<s64>) — the list-of-scalar twin of SHAPE 232's bytes case. Both a
           Bytes case and a list case marshal into `mem` and flatten to the SAME two i32 slots `(ptr, len|count)`,
           so this rides the SAME `HostParam::VariantMixed` join as SHAPE 232: b's `[i64]` joined with c's `[i32
           ptr, i32 count]` gives slot0=`join(i64,i32)=i64`, slot1=`i32` (`host::variant_mixed_join_slots`,
           matching `wit_ctype::flatten_variant`), so the core flatten is `(disc:i32, i64, i32)`.
           `emit_variant_mixed_arg_reg_flatten` DISPATCHES per case (a nested `if disc==d … else …` chain): b →
           unbox s64 into slot0 (i64), slot1=0; c → marshal the `list<s64>` into `mem` as an inline element array
           via `emit_list_arg_marshal` (which advances the cursor), slot0 = outer-ptr EXTENDED to i64, slot1 =
           count; a (nullary) → zero both slots. run() performs THREE pushes — `(B 42)` the scalar case,
           `(C #list(1 2 3))` the list case, `A` the nullary case — exercising ALL THREE arms; a VALID running
           component that links against the imposed WIT world (live-objects=0) is the pin. This closes the mixed
           variant's list-payload gap (previously the mixed detector required a Bytes case; a list case declined).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (List Int64)))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C #list(1 2 3))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(bytes), d(list<s64>)} with THREE payload-bearing cases as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 242 (v-wit-boundary) — a `variant{a, b(s64), c(list<u8>), d(list<s64>)}` passed BARE as the TOP-LEVEL
           host-op ARGUMENT (probe.push), pinning the MULTI-mem-case position-wise JOIN: unlike SHAPE 232
           (scalar+Bytes) and SHAPE 241 (scalar+List), this mixes a SCALAR case (b: s64) with TWO distinct mem
           cases — a Bytes case (c: list<u8>) AND a List case (d: list<s64>) — so slot 1 is contributed by BOTH
           mem cases. `HostParam::VariantMixed` joins position-wise over ALL payload cases
           (`host::variant_mixed_join_slots`, matching `wit_ctype::flatten_variant`): b's `[i64]`, c's `[i32 ptr,
           i32 len]`, d's `[i32 ptr, i32 count]` join to slot0=`join(i64,i32,i32)=i64`, slot1=`join(i32,i32)=i32`,
           so the core flatten is `(disc:i32, i64, i32)`. `emit_variant_mixed_arg_reg_flatten` DISPATCHES per case
           (a nested `if disc==d … else …` chain): b → unbox s64 into slot0 (i64), slot1=0; c → rope-copy the
           Bytes into `mem` at the cursor, slot0 = ptr EXTENDED to i64, slot1 = len; d → marshal the `list<s64>`
           into `mem` via `emit_list_arg_marshal`, slot0 = outer-ptr EXTENDED to i64, slot1 = count; a (nullary) →
           zero both slots. run() performs FOUR pushes — `(B 42)`, `(C b\"hi\")`, `(D #list(1 2 3))`, `A` —
           exercising ALL FOUR arms of the join; a VALID running component (live-objects=0) is the pin. This
           witnesses the multi-payload-CASE join (≥3 payload-bearing cases where a Bytes and a List case share
           slot 1), previously exercised only across two separate 2-case shapes.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (u8))) (d (list (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Bytes) (D (List Int64)))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C b"hi")) (probe.push (V.D #list(1 2 3))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(s32,s64)} with a scalar case + a TUPLE(multi-payload) case as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 243 (v-wit-boundary) — a `variant{a, b(s64), c(tuple<s32,s64>)}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push), the FIRST MIXED variant whose payload cases mix a SCALAR case (b: s64) with a
           TUPLE case (c, a MULTI-payload ctor `(C Int32 Int64)` whose payloads `variant_payload_ty_at` synthesizes
           into a `tuple<s32,s64>`; its runtime rep is a tuple handle). A single tuple case is `HostParam::
           VariantTuple` (SHAPE 236) and an all-scalar mix is `HostParam::Variant`/`VariantScalarsMixed`; the
           heterogeneous scalar+tuple mix routes to `HostParam::VariantMixed` via the new `VariantPayloadKind::
           Tuple(elem-abis)`. The tuple case flattens POSITIONALLY inline (one core slot per element), joined
           slot-wise with the scalar case: b's `[i64]`, c's `[i32, i64]` join to slot0=`join(i64,i32)=i64`,
           slot1=`join(-, i64)=i64` (`host::variant_mixed_join_slots`), so the core flatten is `(disc:i32, i64,
           i64)`. `emit_variant_mixed_arg_reg_flatten`'s Tuple arm marshals the payload via the shared
           `emit_tuple_reg_flatten` and COERCES each element into its joined slot width — element 0 is a NATURAL
           i32 joined to i64, so it is widened `i64.extend_i32_u` (the coercion this shape exercises); element 1
           (i64) stays. The scalar case b unboxes into slot0 (i64), zeroing slot1; a (nullary) zeroes both. run()
           performs THREE pushes — `(B 42)`, `(C (: 1 Int32) 2)`, `A` — exercising all three arms; a VALID running
           component that TYPE-CHECKS the flatten against the declared `variant` (a missed widen would leave an
           i32 where the i64 slot is required → validation error) with live-objects=0 pins the tuple-payload mixed
           variant + the per-element join coercion. This closes the compound (tuple)-variant-payload-at-ARG gap in
           the MIXED position.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (tuple (s32) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Int32 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C (: 1 Int32) 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(f64), c(bytes)} with a FLOAT scalar case + a Bytes case as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 244 (v-wit-boundary) — a `variant{a, b(f64), c(list<u8>)}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push), the FIRST MIXED variant with a FLOAT scalar payload case. Earlier mixed variants
           were INT-only (a float scalar declined); this exercises the canonical variant reinterpret JOIN across a
           float case and a mem case. b's payload flattens to `[f64]`, c's (Bytes) to `[i32 ptr, i32 len]`; the
           position-wise join (`host::variant_mixed_join_slots`, matching `wit_ctype::flatten_variant`) gives
           slot0=`join(f64, i32)=i64` (a float mixed with an integer slot → the integer width), slot1=`i32`, so the
           core flatten is `(disc:i32, i64, i32)`. `emit_variant_mixed_arg_reg_flatten` DISPATCHES per case: b →
           `get-float` (f64) then `emit_scalar_coerce_into_slot` REINTERPRETS f64→i64 (`i64.reinterpret_f64`) into
           slot0, slot1=0; c → rope-copy the Bytes into `mem`, slot0 = ptr EXTENDED to i64, slot1 = len; a
           (nullary) → zero both slots. run() performs THREE pushes — `(B 3.5)`, `(C b\"hi\")`, `A` — exercising
           all three arms; the component TYPE-CHECKS the flatten against the declared `variant` (a missing
           reinterpret would leave an f64 where the i64 slot is required → a validation error), so a VALID running
           component (live-objects=0) pins the float-scalar-in-mixed-variant reinterpret join. Closes the int-only
           restriction on a mixed variant's scalar case.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (f64)) (c (list (u8))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Float64) (C Bytes))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 3.5)) (probe.push (V.C b"hi")) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(f64,s64)} with a TUPLE case carrying a FLOAT element as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 245 (v-wit-boundary) — a `variant{a, b(s64), c(tuple<f64,s64>)}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push), extending SHAPE 243's tuple-payload mixed variant to a FLOAT tuple ELEMENT. The
           tuple case c flattens POSITIONALLY inline to `[f64, i64]`; joined with the scalar case b's `[i64]`,
           slot0=`join(i64, f64)=i64`, slot1=`i64` (`host::variant_mixed_join_slots`, matching
           `wit_ctype::flatten_variant`), so the core flatten is `(disc:i32, i64, i64)`.
           `emit_variant_mixed_arg_reg_flatten`'s Tuple arm marshals the payload via the shared
           `emit_tuple_reg_flatten` and coerces EACH element into its joined slot via `emit_scalar_coerce_into_slot`
           — element 0 is an f64 folded against b's i64 at slot0, so it REINTERPRETS `i64.reinterpret_f64`; element
           1 (i64) stays. The scalar case b unboxes into slot0 (i64), zeroing slot1; a (nullary) zeroes both. run()
           performs THREE pushes — `(B 42)`, `(C 3.5 2)`, `A` — exercising all three arms; the component
           TYPE-CHECKS the flatten (a missing reinterpret would leave an f64 where the i64 slot is required → a
           validation error), so a VALID running component (live-objects=0) pins the float-tuple-element mixed
           variant. With SHAPE 244 (float scalar case) this completes float support in a mixed variant's inline
           (scalar/tuple) cases.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (tuple (f64) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Float64 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C 3.5 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a record host-op ARG with a FLAGS field (nested record-of-bools) crosses via a pure-IMPORT wit-world (imposed)"
  (doc
    "SHAPE 246 (v-wit-boundary) — a record host-op ARGUMENT one of whose FIELDS is a WIT `flags{read,write,
           execute}` (import side, imposed wit-world), the record-FIELD twin of the top-level flags ARG (SHAPE
           240). The guest models flags as a nested record-of-bools (operator ruling: flags is a PRODUCT), so the
           arg is `record{ p: record{read,write,execute: Bool}, n: s64 }` whose WIT declares field `p` as `flags`.
           Before this it DECLINED (`a record host-arg's declared WIT type is not a record` — the field marshal saw
           field `p`'s guest type is a record but its WIT is flags). Now the NEW `RecordFieldAbi::Flags` +
           `reorder_record_fields_to_wit`'s bool-record→flags conversion + `emit_record_arg_marshal`'s flags-field
           arm PACK the nested bool-record's cells into a bitset word: field `p` flattens to ONE i32
           (`flatten_record_field_abi` → `ceil(labels/32)`), `emit_flags_arg_pack` reads each bool cell
           (`arr-get`+`get-bool`) and shifts it into its WIT-label bit, and the component record's `p` field is a
           nominal `flags` DEFINED type (`record_field_cref`'s Flags arm / the structural WIT path). The core
           flatten is `(p:i32-bitset, n:i64)` (WIT field order). run() passes `#record((= p #record((= read true)
           (= write false) (= execute true))) (= n 5))`; a VALID running component that links against the imposed
           world (live-objects=0) is the pin. Closes the flags-at-record-FIELD ARG gap. (>32 labels stays a
           Component Model spec limit — declined.)")
  (wit-world (world w (import cadenza:platform/probe
    (member push (func (param m (record (p (flags read write execute)) (n (s64)))) (result (s64)))))))
  (input (do
    (effect probe (op push (-> (Record (: p (Record (: read Bool) (: write Bool) (: execute Bool))) (: n Int64)) Int64)))
    (def (run) (host (probe) (probe.push #record((= p #record((= read true) (= write false) (= execute true))) (= n 5)))))
    (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(bytes), d(s32,s64)} with scalar + bytes + tuple cases as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 247 (v-wit-boundary) — a `variant{a, b(s64), c(list<u8>), d(tuple<s32,s64>)}` passed BARE as the
           TOP-LEVEL host-op ARGUMENT (probe.push), mixing ALL THREE inline/mem case kinds at once: a SCALAR case
           (b: s64), a mem (Bytes) case (c: list<u8>), and a TUPLE case (d: tuple<s32,s64>). This exposed + now
           pins a slot-WIDTH-JOIN bug: the tuple case's element 1 (s64) is an i64 at slot 1, so the position-wise
           join widens slot 1 to i64 (`join(len:i32, s64:i64)=i64`); the core flatten is `(disc:i32, i64, i64)`
           (slot0=join(i64, ptr:i32, s32:i32)=i64). Before the fix, `emit_variant_mixed_arg_reg_flatten`'s Bytes
           (and List) arm wrote the LEN/COUNT (an i32) into slot 1 WITHOUT coercing — so when a tuple case widened
           slot 1 to i64, the i32 len was stored into an i64 local → the component failed validation (`expected
           i64, found i32`, CDZ0910). The fix extends the len/count `i64.extend_i32_u` when slot 1 joined wide
           (mirroring the slot-0 ptr extend). run() performs FOUR pushes — `(B 42)`, `(C b\"hi\")`, `(D (: 1
           Int32) 2)`, `A` — exercising all four arms; a VALID running component (live-objects=0) pins the
           multi-kind join. Earlier mixed variants (SHAPE 241/242 scalar+bytes/list) never hit this because slot 1
           stayed i32 with no tuple case to widen it.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (u8))) (d (tuple (s32) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Bytes) (D Int32 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C b"hi")) (probe.push (V.D (: 1 Int32) 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(list<s64>), d(s32,s64)} with scalar + LIST + tuple cases as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 248 (v-wit-boundary) — a `variant{a, b(s64), c(list<s64>), d(tuple<s32,s64>)}` passed BARE as the
           TOP-LEVEL host-op ARGUMENT (probe.push), the genuine-LIST sibling of SHAPE 247: a SCALAR case (b: s64),
           a LIST case (c: list<s64>, NOT list<u8>/Bytes — the value-heap list-marshal path), and a TUPLE case
           (d: tuple<s32,s64>). This exposed + now pins a LOCAL-SLOT TYPE-CONFLICT bug distinct from SHAPE 247's
           slot-width join: the List arm and the Tuple arm of `emit_variant_mixed_arg_reg_flatten` BOTH allocated
           their sub-marshal scratch from a FIXED base (`pay + 4`), so the List arm's i32 loop counter and a tuple
           case's i64 s64-element temp landed on the SAME emit-local INDEX. `scratch_ty` is ONE type map for the
           whole function, so that index got a SINGLE declared type (i64) and the List arm's i32 loop-counter store
           became an i32-into-i64 write → an invalid component (`expected i64, found i32`, CDZ0910). SHAPE 247
           (scalar+BYTES+tuple) never hit it because the Bytes arm uses fixed LOW scratch locals and never allocates
           in the overlapping range. The fix bumps each dynamic arm's scratch off the RUNNING high-water (`*high`)
           so the arms' locals are DISJOINT and no index carries two ValTypes (the coalesce pass compacts them
           afterward). run() performs FOUR pushes — `(B 42)`, `(C #list(1 2 3))`, `(D (: 1 Int32) 2)`, `A` —
           exercising all four arms; a VALID running component (live-objects=0) pins the scalar+list+tuple mix.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (s64))) (d (tuple (s32) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (List Int64)) (D Int32 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C #list(1 2 3))) (probe.push (V.D (: 1 Int32) 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(list<u8>), c(list<s64>), d(s32,s64)} with NO scalar case — Bytes + List + tuple as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 249 (v-wit-boundary) — a `variant{a, b(list<u8>), c(list<s64>), d(tuple<s32,s64>)}` passed BARE as the
           TOP-LEVEL host-op ARGUMENT (probe.push), with NO scalar case: a Bytes case (b: list<u8>), a List case
           (c: list<s64>), and a Tuple case (d: tuple<s32,s64>). This closes a gap: `variant_mixed_payload_cases`
           gated on `any_scalar && (any_mem || any_tuple)`, so a variant whose multi-slot cases were NOT accompanied
           by a scalar case had NO home — `variant_bytes_payload_cases` requires ALL cases Bytes, `variant_list_
           payload_cases` requires ALL cases `list<scalar>` of one element type, `variant_tuple_payload_case`
           requires exactly ONE tuple case + rest nullary — so a Bytes+List+tuple mix (or two tuples, or a
           tuple beside a list) fell through every detector to CDZ0903 (a legitimate WIT type, wrongly declined).
           `VariantMixed` is dispatched LAST (after those narrower detectors), so relaxing its gate to just
           `(any_mem || any_tuple)` claims only the RESIDUE they decline — no case is stolen. A scalar-less set
           flattens the same way: the join is computed over whatever cases exist and the per-case emit simply never
           takes a Scalar arm. run() performs FOUR pushes — `(B b\"hi\")`, `(C #list(1 2 3))`, `(D (: 1 Int32) 2)`,
           `A` — exercising the Bytes, List, Tuple, and nullary arms with NO scalar arm; a VALID running component
           (live-objects=0) pins the scalar-less mem+tuple mix.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (u8))) (c (list (s64))) (d (tuple (s32) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Bytes) (C (List Int64)) (D Int32 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B b"hi")) (probe.push (V.C #list(1 2 3))) (probe.push (V.D (: 1 Int32) 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(list<s64>), c(list<s32>)} — two list cases of DIFFERENT element types, no scalar (imposed WIT)"
  (doc
    "SHAPE 250 (v-wit-boundary) — a `variant{a, b(list<s64>), c(list<s32>)}` passed BARE as the TOP-LEVEL host-op
           ARGUMENT (probe.push): TWO list cases whose element types DIFFER (list<s64> vs list<s32>), no scalar.
           `variant_list_payload_cases` (the all-`list<scalar>` detector) explicitly requires a SHARED element
           type across every list case (a single `emit_list_arg_marshal(elem)` covers whichever case fired), so a
           MIXED-element-type list variant declined there — and before SHAPE 249 the `VariantMixed` gate needed a
           scalar case, so this shape had no home. Now `VariantMixed` fires (its gate is `(any_mem || any_tuple)`),
           and each case carries its OWN element type via `VariantPayloadKind::List(elem)`, so the per-case emit
           marshals case b as list<s64> and case c as list<s32> independently — the mixed classifier subsumes the
           shared-element restriction. Both cases flatten to the two-i32-slot `(ptr, count)` mem form, joined
           position-wise (`(disc:i32, i32, i32)`). run() pushes `(B #list(1 2))`, `(C #list(3 4))`, `A` —
           exercising both List arms (distinct element types) and the nullary arm; a VALID running component
           (live-objects=0) pins per-case list element types in a mixed variant.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (s64))) (c (list (s32))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B (List Int64)) (C (List Int32)))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B #list(1 2))) (probe.push (V.C #list((: 3 Int32) (: 4 Int32)))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(list<f64>), c(s32,s64)} — a FLOAT-element list case beside a tuple, no scalar (imposed WIT)"
  (doc
    "SHAPE 251 (v-wit-boundary) — a `variant{a, b(list<f64>), c(tuple<s32,s64>)}` passed BARE as the TOP-LEVEL
           host-op ARGUMENT (probe.push): a List case whose element is a FLOAT (`list<f64>`) beside a Tuple case,
           no scalar. Pins the FLOAT list-element marshal path in a mixed variant — `emit_list_arg_marshal` stores
           each element with the element's canonical width store (an `f64.store` here, vs the `i64.store`/`i32.store`
           of the earlier int-element list cases SHAPE 241/247/250). The List case flattens to the two-i32-slot
           `(ptr, count)` mem form (the element WIDTH does not change the outer header), joined position-wise with
           the tuple case to `(disc:i32, i32, i64)`: slot0 = join(list-ptr:i32, s32:i32) = i32, slot1 =
           join(list-count:i32, s64:i64) = i64 (the tuple's s64 element widens slot 1, so the count extends).
           This is reachable via the SHAPE 249 no-scalar relaxation; SHAPE 245 pinned a float TUPLE element but no
           float LIST element existed. run() pushes `(B #list(1.5 2.5))`, `(C (: 1 Int32) 2)`, `A` — exercising the
           float-element List arm, the Tuple arm, and the nullary arm; a VALID running component (live-objects=0)
           pins the float list element in a mixed variant.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (list (f64))) (c (tuple (s32) (s64))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B (List Float64)) (C Int32 Int64))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B #list(1.5 2.5))) (probe.push (V.C (: 1 Int32) 2)) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(record{x,y})} with a RECORD payload case whose WIT field order differs from guest name-lex (imposed WIT)"
  (doc
    "SHAPE 252 (v-wit-boundary) — a `variant{a, b(s64), c(record{y:s32, x:s64})}` passed BARE as the TOP-LEVEL
           host-op ARGUMENT (probe.push): a scalar case (b: s64) mixed with a RECORD payload case (c). Closes the
           record-payload-in-a-MIXED-variant gap: `variant_record_payload_case` handles a SINGLE record case + rest
           nullary, but a record case MIXED with another case fell to `VariantMixed`, whose detector returned None
           at the record branch. Now `variant_mixed_payload_cases` recognizes a scalar-field record case
           (`VariantPayloadKind::Record`), flattening it POSITIONALLY inline like a tuple — one slot per field. The
           slot ORDER follows the WIT record's field DECLARATION order: here the WIT declares `(y:s32, x:s64)` but
           the guest record is name-lex `(x, y)`, so the field ABIs must be REORDERED to WIT before the join, and
           the emit marshals field VALUES in WIT order (via `emit_record_arg_marshal`). `variant_mixed_payload_cases_wit`
           applies this reorder at the two sites that consume the slot order (the classifier -> serialize, and the
           emit), keeping serialize (the param core type), the emit (pushed values), and host_imports (the WIT
           `variant` type) all in the SAME order — a wrong reorder would fail canonical-ABI validation (CDZ0910).
           The join is `(disc:i32, i64, i64)`: slot0 = join(b:s64=i64, record y:s32=i32) = i64, slot1 =
           join(record x:s64=i64) = i64. run() pushes `(B 42)`, `(C {x:1, y:2})`, `A` — exercising the Scalar arm,
           the Record arm (with a permuted WIT field order), and the nullary arm; a VALID running component
           (live-objects=0) pins the WIT-ordered record payload case in a mixed variant.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (record (= y (s32)) (= x (s64)))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (Record (: x Int64) (: y Int32))))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C #record((= x 1) (= y (: 2 Int32))))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a MIXED variant{a, b(s64), c(list<record{x,y}>)} with a LIST-of-RECORD element case as a host-op arg (imposed WIT)"
  (doc
    "SHAPE 253 (v-wit-boundary) — a `variant{a, b(s64), c(list<record{x:s64, y:s64}>)}` passed BARE as the
           TOP-LEVEL host-op ARGUMENT (probe.push): a scalar case (b) mixed with a LIST case whose ELEMENT is a
           COMPOUND record (c). Closes a gap: `variant_mixed_payload_cases`'s List arm previously admitted only a
           SCALAR element (`abi_val_type(inner)`), so a `list<record>`/`list<tuple>`/`list<list>` element case in a
           mixed variant declined. Now the List arm admits any element `emit_list_arg_marshal` handles
           (`list_elem_marshalable`: a record/tuple product, a nested list, an option, a `result<list<u8>,enum>`, a
           scalar variant), and the emit's List arm threads the ELEMENT WIT (from the variant's WIT at this case's
           `WitType::List`) so a record/nested element orders/offsets correctly. The outer flatten is UNCHANGED — a
           list case is always `(ptr, count)` two i32 slots regardless of element — so the join / serialize /
           host_imports are untouched; only the in-`mem` element array layout differs (each record element written
           in place at its canonical layout by `emit_list_arg_marshal`'s record arm). run() pushes `(B 42)`,
           `(C [#record{x:1,y:2}, #record{x:3,y:4}])`, `A` — exercising the Scalar arm, the List arm with a RECORD
           element, and the nullary arm; a VALID running component (live-objects=0) pins the list-of-record element
           case in a mixed variant.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (variant (a) (b (s64)) (c (list (record (= x (s64)) (= y (s64))))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (List (Record (: x Int64) (: y Int64)))))
      (effect probe (op push (-> V Int64)))
      (def (run) (host (probe) (do (probe.push (V.B 42)) (probe.push (V.C #list(#record((= x 1) (= y 2)) #record((= x 3) (= y 4))))) (probe.push V.A))))
      (export run)))
  (call run)
  (host-responses
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64))
    (respond probe.push (: 55 Int64)))
  (host-calls
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push)
    (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{go, stop(tuple<s32,s64>)}> host-op arg crosses (each tuple-payload variant element written in place)"
  (doc
    "SHAPE 254 (v-wit-boundary) — a top-level `list<variant{go, stop(tuple<s32,s64>)}>` bare host-op ARGUMENT
           (probe.push : func(list<variant{go, stop(tuple<s32,s64>)}>) -> s64). A variant with a nullary case
           (go) and a SINGLE TUPLE-payload case (stop) as a list ELEMENT. Closes a gap: `emit_variant_to_mem`
           wrote only a UNIFORM SCALAR payload (a single width store), so a tuple-payload variant element
           declined. Now `emit_variant_to_mem` dispatches a `variant_tuple_payload_case` shape to a compound
           writer (`emit_variant_tuple_to_mem`) that stores the disc then lays the payload TUPLE at the canonical
           payload offset via `emit_product_to_mem` (a nullary case zero-fills the payload region); the payload
           offset and region size come from `canonical_layout`, so they agree with the per-element stride the
           list marshal reserves. `list_elem_marshalable` now admits it and `collect_list_elem_ops` declares its
           ops (`sum-disc`/`sum-payload` + the tuple's `arr-get` + element unboxes). run() builds
           [Stop((3,7)), Go] and performs probe.push; a VALID component that runs is the pin (a wrong tuple-payload
           element layout traps at the host's list.lift).")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (go) (stop (tuple (s32) (s64)))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop (Tuple Int32 Int64)))
      (effect probe (op push (-> (List Sig) Int64)))
      (def (run) (host (probe) (probe.push #list((Sig.Stop #tuple(3 7)) (Sig.Go)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<tuple<variant{go, stop(tuple<s32,s64>)}, s64>> host-op arg crosses (tuple-payload variant as a product FIELD in mem)"
  (doc
    "SHAPE 255 (v-wit-boundary) — a top-level `list<tuple<variant{go, stop(tuple<s32,s64>)}, s64>>` bare host-op
           ARGUMENT (probe.push): each list element is a TUPLE whose first FIELD is a tuple-payload variant. Extends
           SHAPE 254 (the tuple-payload variant as a bare list ELEMENT) to the PRODUCT-FIELD position, reusing the
           SAME `emit_variant_to_mem` tuple writer with NO new emit: `emit_product_to_mem`'s variant-field arm now
           admits a `variant_tuple_payload_case` (its gate previously scalar-only), and `product_field_marshalable`
           / `collect_record_field_ops` gained the tuple-variant admission + ops in lockstep. A tuple element is
           purely POSITIONAL so no WIT field reorder is involved. run() builds [((Stop((3,7)), 5), (Go, 9))] and
           performs probe.push, exercising the tuple case (payload) and the nullary case (Go) at a product field
           beside a scalar field; a VALID running component (live-objects=0) pins the field-position round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (tuple (variant (go) (stop (tuple (s32) (s64)))) (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop (Tuple Int32 Int64)))
      (effect probe (op push (-> (List (Tuple Sig Int64)) Int64)))
      (def (run) (host (probe) (probe.push #list(#tuple((Sig.Stop #tuple(3 7)) 5) #tuple(Sig.Go 9)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level record{v: variant{go, stop(tuple<s32,s64>)}, n: s64} host-op arg crosses (register-flattened variant-tuple FIELD)"
  (doc
    "SHAPE 256 (v-wit-boundary) — a top-level `record{v: variant{go, stop(tuple<s32,s64>)}, n: s64}` bare host-op
           ARGUMENT (probe.push), register-flattened. A tuple-payload `variant` as a RECORD FIELD at a REGISTER
           position (not in `mem`). Closes the gap the SHAPE 254/255 correction left open: `field_boundary_abi`
           returns `RecordFieldAbi::VariantTuple` (carrying the case names + tuple disc + element ABIs), and
           `emit_record_arg_marshal` now flattens the field POSITIONALLY to `(disc:i32, e0, e1)` via
           `emit_variant_tuple_arg_reg_flatten` (the SAME helper the top-level bare variant-tuple ARG uses, SHAPE
           236) — where before this FIELD position DECLINED. `record_field_cref` builds the field's component
           `variant` type (all cases in declaration order, the tuple case carrying `(tuple <elem>…)`); serialize
           flattens `(disc, e0, e1)` — the two agree. The record's fields cross in WIT DECLARATION order (v, n),
           so the guest name-lex order (n, v) is REORDERED (`reorder_record_fields_to_wit`) — exercised here. The
           record flattens overall to `(v-disc:i32, e0:i32, e1:i64, n:i64)`. run() builds { v: Stop((3,7)), n: 5 }
           and performs probe.push; a VALID running component (live-objects=0) pins the register field round-trip.
           (The bare `tuple<variant-tuple, …>` ARG element position remains a clean decline — a later slice.)")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (variant (go) (stop (tuple (s32) (s64))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop (Tuple Int32 Int64)))
      (effect probe (op push (-> (Record (: v Sig) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v (Sig.Stop #tuple(3 7))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<variant{go, stop(tuple<s32,s64>)}, s64> host-op arg crosses (register-flattened variant-tuple ELEMENT)"
  (doc
    "SHAPE 257 (v-wit-boundary) — a top-level `tuple<variant{go, stop(tuple<s32,s64>)}, s64>` bare host-op
           ARGUMENT (probe.push), register-flattened. A tuple-payload `variant` as a TUPLE ELEMENT at a REGISTER
           position — the last register-position variant-tuple gap (SHAPE 256 closed the RECORD-field position).
           `emit_tuple_reg_flatten` now flattens the element POSITIONALLY to `(disc:i32, e0, e1)` via
           `emit_variant_tuple_arg_reg_flatten` (the SAME helper the bare variant-tuple ARG / a variant-tuple
           record FIELD use) — where before this ELEMENT position DECLINED (CDZ0903). `tuple_arg_crosses` +
           the tuple-arg abi-builder + the used_ops tuple-element collector gained the `variant_tuple_payload_case`
           admission in lockstep. A tuple element is purely POSITIONAL → no reorder. The outer tuple flattens to
           `(v-disc:i32, e0:i32, e1:i64, n:i64)`. run() builds ((Stop((3,7))), 5) and performs probe.push; a VALID
           running component (live-objects=0) pins the register element round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (variant (go) (stop (tuple (s32) (s64)))) (s64))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop (Tuple Int32 Int64)))
      (effect probe (op push (-> (Tuple Sig Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Sig.Stop #tuple(3 7)) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{a, b(s64), c(tuple<s32,s64>)}> host-op arg crosses (HETEROGENEOUS scalar+tuple variant element at mem)"
  (doc
    "SHAPE 258 (v-wit-boundary) — a top-level `list<variant{a, b(s64), c(tuple<s32,s64>)}>` bare host-op ARGUMENT
           (probe.push): each list element is a HETEROGENEOUS variant MIXING a scalar payload case (b) and a
           tuple-of-scalars payload case (c), plus a nullary case (a). Closes the last arg-side mem gap: the mem
           writer `emit_variant_to_mem` handled only a UNIFORM scalar payload (SHAPE 171) or a SINGLE tuple case
           (SHAPE 254); a scalar+tuple MIX declined. Now it dispatches a `variant_mixed_payload_cases` shape (all
           Scalar/Tuple kinds) to `emit_variant_mixed_to_mem`, a general PER-CASE dispatcher: it stores the disc,
           zero-fills the payload region, then on the SELECTED case writes a scalar at its width OR the tuple
           product (via `emit_product_to_mem`) at the canonical payload offset. `field_boundary_abi` returns the
           new `RecordFieldAbi::VariantMemMixed` (so the classifier builds `HostParam::List` → memory declared),
           `list_elem_marshalable` + the list marshal's `is_variant` gate + `collect_list_elem_ops` gained the
           scalar+tuple-mixed admission in lockstep. The bare-ARG mix already rode `HostParam::VariantMixed` (SHAPE
           243); this is its mem twin. run() pushes `(C (3,7))`, `(B 42)`, `A` — exercising the tuple case, the
           scalar case, and the nullary case; a VALID running component (live-objects=0) pins the round-trip.
           (A Bytes/List/record payload case in such a mixed variant at mem remains a clean decline — a later slice.)")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (a) (b (s64)) (c (tuple (s32) (s64)))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (Tuple Int32 Int64)))
      (effect probe (op push (-> (List V) Int64)))
      (def (run) (host (probe) (probe.push #list((V.C #tuple(3 7)) (V.B 42) V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{a, b(s64), c(bytes)}> host-op arg crosses (HETEROGENEOUS scalar+bytes variant element at mem)"
  (doc
    "SHAPE 259 (v-wit-boundary) — a top-level `list<variant{a, b(s64), c(list<u8>)}>` bare host-op ARGUMENT
           (probe.push): each list element is a HETEROGENEOUS variant MIXING a scalar payload case (b) and a
           BYTES payload case (c), plus a nullary case (a). Extends SHAPE 258 (scalar+tuple mem mix) with a Bytes
           payload case: `emit_variant_mixed_to_mem` gained a Bytes arm that writes a `(ptr, len)` header at the
           payload offset and copies the rope into `mem` at the running cursor (advancing it) — the canonical
           `list<u8>` case layout. A real cursor is now threaded into `emit_variant_to_mem` (from the list-element
           / product-field callers) for the spill; the scalar/tuple paths ignore it. `field_boundary_abi`'s
           `VariantMemMixed` scope, `list_elem_marshalable`, the list marshal's `is_variant` gate, and
           `collect_list_elem_ops` gained the Bytes-case admission in lockstep (a Bytes case → the shared
           `(list u8)` type + `bytes-len`/`bytes-get` ops). The bare-ARG scalar+bytes mix already rode
           `HostParam::VariantMixed` (SHAPE 244); this is its mem twin. run() pushes `[C(\"hi\"), B(42), A]` in one
           list — exercising the bytes case, the scalar case, and the nullary case; a VALID running component
           (live-objects=0) pins the round-trip. (A List/record payload case in such a mixed variant at mem
           remains a clean decline — a later slice.)")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (a) (b (s64)) (c (list (u8)))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C Bytes))
      (effect probe (op push (-> (List V) Int64)))
      (def (run) (host (probe) (probe.push #list((V.C b"hi") (V.B 42) V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{a, b(s64), c(list<s64>)}> host-op arg crosses (HETEROGENEOUS scalar+list variant element at mem)"
  (doc
    "SHAPE 260 (v-wit-boundary) — a top-level `list<variant{a, b(s64), c(list<s64>)}>` bare host-op ARGUMENT
           (probe.push): each list element is a HETEROGENEOUS variant MIXING a scalar payload case (b) and a
           LIST-of-scalar payload case (c), plus a nullary case (a). Extends SHAPE 258/259 (scalar+tuple, +bytes)
           with a List payload case: `emit_variant_mixed_to_mem` gained a List arm that marshals the payload
           `list<scalar>` into `mem` via the shared `emit_list_arg_marshal` (backing array at the running cursor)
           and writes a `(ptr, count)` header at the payload offset — the canonical `list` case layout.
           `variant_mem_mixed_kind_supported` (the single gate helper shared by `field_boundary_abi`,
           `list_elem_marshalable`, and the list marshal's `is_variant`) now admits a List-of-SCALAR case;
           `collect_list_elem_ops` (→ `vec-len`/`vec-get` + element unbox), `record_field_cref` (→ a `(list <elem>)`
           type), and serialize gained it. Scoped to a SCALAR list element — a `list<compound>` case (needing the
           element WIT) or a record payload case remains a clean decline (later slices). The bare-ARG scalar+list
           mix already rode `HostParam::VariantMixed` (SHAPE 241); this is its mem twin. run() pushes
           `[C([1,2,3]), B(42), A]` in one list — exercising the list case, the scalar case, and the nullary case;
           a VALID running component (live-objects=0) pins the round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (a) (b (s64)) (c (list (s64)))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (List Int64)))
      (effect probe (op push (-> (List V) Int64)))
      (def (run) (host (probe) (probe.push #list((V.C #list(1 2 3)) (V.B 42) V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{a, b(s64), c(record{p: s32, q: s64})}> host-op arg crosses (HETEROGENEOUS scalar+record variant element at mem)"
  (doc
    "SHAPE 261 (v-wit-boundary) — a top-level `list<variant{a, b(s64), c(record{p: s32, q: s64})}>` bare host-op
           ARGUMENT (probe.push): each list element is a HETEROGENEOUS variant MIXING a scalar payload case (b)
           and a RECORD-of-scalars payload case (c), plus a nullary case (a). Extends SHAPE 258/259/260
           (scalar+tuple, +bytes, +list) with a RECORD payload case — the LAST payload kind for the mem mixed
           variant: `emit_variant_mixed_to_mem` gained a Record arm that writes the record PRODUCT at the payload
           offset via `emit_record_to_mem` (each field at its canonical offset, WIT-ordered) — the mem twin of the
           register mixed Record arm. `variant_mem_mixed_kind_supported` (the single gate helper shared by
           `field_boundary_abi`, `list_elem_marshalable`, and the list marshal's `is_variant`) now admits a
           Record-of-SCALAR case; `emit_variant_to_mem`/`emit_variant_mixed_to_mem` thread the element's declared
           WIT variant so the record case's fields order to WIT declaration order; `collect_list_elem_ops` (the
           used_ops element collector) gained a Record arm (`arr-get` per field + each field's unbox). GUARD
           (correct-or-declines): the per-element stride the list marshal reserves comes from
           `canonical_layout(record)` in GUEST name-lex order, so the emit declines cleanly when the WIT field
           order diverges from the guest order (record padding is field-order-dependent) — here p<q matches the
           WIT `(p s32)(q s64)` declaration order. run() pushes `[C({p:3, q:7}), B(42), A]` in one list —
           exercising the record case, the scalar case, and the nullary case; a VALID running component
           (live-objects=0) pins the round-trip. (This closes the mem mixed-variant payload-kind algebra:
           scalar/tuple/bytes/list/record are all expressible.)")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (a) (b (s64)) (c (record (= p (s32)) (= q (s64))))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (Record (: p Int32) (: q Int64))))
      (effect probe (op push (-> (List V) Int64)))
      (def (run) (host (probe) (probe.push #list((V.C #record((= p 3) (= q 7))) (V.B 42) V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level list<variant{a, b(s64), c(list<record{x: s32, y: s64}>)}> host-op arg crosses (HETEROGENEOUS scalar + list-of-COMPOUND variant element at mem)"
  (doc
    "SHAPE 262 (v-wit-boundary) — a top-level `list<variant{a, b(s64), c(list<record{x: s32, y: s64}>)}>` bare
           host-op ARGUMENT (probe.push): each list element is a HETEROGENEOUS variant MIXING a scalar payload
           case (b) and a LIST-of-COMPOUND (list<record>) payload case (c), plus a nullary case (a). Extends
           SHAPE 260 (list-of-SCALAR payload case) to a COMPOUND list element: `emit_variant_mixed_to_mem`'s List
           arm now EXTRACTS this case's element WIT from the variant WIT (`list<elem>` → elem) and THREADS it into
           the shared `emit_list_arg_marshal`, so a record element's fields order to WIT declaration order (the
           scalar element still passes `None`, offset-agnostic). `variant_mem_mixed_kind_supported`'s List arm is
           now unconditional `true` — the detector `variant_mixed_payload_cases` ALREADY validates the element is
           marshalable (`abi_val_type OR list_elem_marshalable`), so a List case reaching the gate is
           known-marshalable and the emit handles the full marshalable element set (record/tuple/nested list/
           option). `collect_list_elem_ops` (the used_ops element collector) already recurses the payload list's
           element ops. The record element's WIT field order matches the guest name-lex order (x<y ↔ WIT
           `(x s32)(y s64)`), so `emit_record_to_mem`'s in-place write agrees with the reserved element stride.
           run() pushes `[C([{x:1,y:2}, {x:3,y:4}]), B(42), A]` in one list — exercising the list-of-record case,
           the scalar case, and the nullary case; a VALID running component (live-objects=0) pins the round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (list (variant (a) (b (s64)) (c (list (record (= x (s32)) (= y (s64)))))))) (result (s64)))))))
  (input
    (do
      (type V (A) (B Int64) (C (List (Record (: x Int32) (: y Int64)))))
      (effect probe (op push (-> (List V) Int64)))
      (def (run) (host (probe) (probe.push #list((V.C #list(#record((= x 1) (= y 2)) #record((= x 3) (= y 4)))) (V.B 42) V.A))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level option<record{v: variant{go, stop(tuple<s32,s64>)}, n: s64}> host-op arg crosses on the Some arm"
  (doc
    "SHAPE 263 (v-wit-boundary) — a top-level `option<record{ v: variant{go, stop(tuple<s32,s64>)}, n: s64 }>`
           bare host-op ARGUMENT (probe.push), the Some arm. COMPOSES the option<record> arg (SHAPE 173's shape)
           with a TUPLE-payload variant FIELD (SHAPE 256's register variant-tuple record-field arm) — a deeper
           nesting than SHAPE 173's scalar-payload variant field. `is_boundary_record` admits the payload record
           (its variant-tuple field crosses via `field_boundary_abi`'s VariantTuple arm); `emit_option_reg_flatten`'s
           record branch recurses `emit_record_arg_marshal`, whose VariantTuple field arm flattens the variant to
           `(v-disc, e0, e1)` — so the option flattens POSITIONALLY to `(opt-disc, v-disc, e0:i32, e1:i64, n:i64)`.
           Reachable by composition (no new code — pins the deeper-nesting round-trip per the operator add-a-case-
           even-if-it-passes directive). run() builds Some({ v: Stop((3,7)), n: 5 }) and performs probe.push; a
           VALID component that runs (live-objects=0) is the pin.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (option (record (= v (variant (go) (stop (tuple (s32) (s64))))) (= n (s64))))) (result (s64)))))))
  (input
    (do
      (type Sig (Go) (Stop (Tuple Int32 Int64)))
      (effect probe (op push (-> (Option (Record (: v Sig) (: n Int64))) Int64)))
      (def (run) (host (probe) (probe.push (Some #record((= v (Sig.Stop #tuple(3 7))) (= n 5))))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with a HETEROGENEOUS mixed variant{a, b(s64), c(tuple<s32,s64>)} field crosses (register flatten)"
  (doc
    "SHAPE 264 (v-wit-boundary) — a RECORD host-op ARGUMENT `record{ v: variant{a, b(s64), c(tuple<s32,s64>)},
           n: s64 }` (probe.push) whose `v` FIELD is a HETEROGENEOUS MIXED variant (a scalar payload case b + a
           tuple payload case c + a nullary case a) at the REGISTER record-FIELD position. Previously DECLINED
           (`emit_record_arg_marshal`'s field dispatch had no mixed-variant arm — the scalar-payload
           (`variant_scalar_payload_cases`) and single-tuple (`variant_tuple_payload_case`) arms do not claim a
           scalar+tuple MIX, so it fell to the None catch-all). Now `emit_record_arg_marshal` gains a
           VariantMemMixed field arm that reads the field's variant handle (`arr-get`) and flattens it via
           `emit_variant_mixed_arg_reg_flatten` — the SAME helper the bare-ARG mixed variant uses (SHAPE 241/243) —
           to `(v-disc:i32, joined-slots…)`. The abi (`field_boundary_abi`'s VariantMemMixed) + `host_imports`'s
           declared `variant` DEFINED type were already produced; serialize's VariantMemMixed flatten is now the
           canonical position-wise join (`variant_mixed_join_slots`, matching the bare-ARG `HostParam::VariantMixed`
           + the guest push + `wit_ctype::flatten_variant`) rather than the former discarded widest-case placeholder,
           so the record's core param flatten agrees with the guest marshal. `collect_record_field_ops` gained the
           matching arm. SCOPED to NO-mem payload cases (scalar/tuple/record) this increment — a Bytes/List case
           (needing a `mem` spill + a reserved cursor) still declines cleanly. The whole record flattens to
           `(v-disc:i32, e0:i32, e1:i64, n:i64)`. run() builds { v: C((3,7)), n: 5 } and performs probe.push; a
           VALID running component (live-objects=0) pins the register field round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (variant (a) (b (s64)) (c (tuple (s32) (s64))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (A) (B Int64) (C (Tuple Int32 Int64)))
      (effect probe (op push (-> (Record (: v Sig) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v (Sig.C #tuple(3 7))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<variant{a, b(s64), c(tuple<s32,s64>)}, s64> host-op arg crosses (HETEROGENEOUS mixed variant ELEMENT, register flatten)"
  (doc
    "SHAPE 265 (v-wit-boundary) — a top-level `tuple<variant{a, b(s64), c(tuple<s32,s64>)}, s64>` bare host-op
           ARGUMENT (probe.push) whose ELEMENT 0 is a HETEROGENEOUS MIXED variant (scalar case b + tuple case c +
           nullary a) at the REGISTER tuple-ELEMENT position — the tuple-element twin of SHAPE 264's record-FIELD
           mixed variant (as SHAPE 257 was the tuple-element twin of the SHAPE 256 record-field variant-tuple).
           Previously DECLINED (`emit_tuple_reg_flatten`'s element dispatch had scalar-/single-tuple variant arms
           but no MIX arm — a scalar+tuple mix fell to the final `get_op_ty` decline). Now `emit_tuple_reg_flatten`
           gains a mixed-variant element arm reading the element's variant handle (`arr-get`) and flattening via
           `emit_variant_mixed_arg_reg_flatten` — the SAME helper the bare-ARG mixed variant / a mixed-variant
           record FIELD use — to `(v-disc, joined-slots…)`. `tuple_arg_crosses` + the tuple-arg abi-builder (a
           VariantMemMixed element branch before the record else, which would else panic on a Sum) + the used_ops
           tuple-element collector gained the mixed-variant admission in lockstep; serialize's VariantMemMixed
           flatten is the canonical position-wise join (aligned in SHAPE 264). SCOPED to NO-mem payload cases
           (scalar/tuple/record); a Bytes/List case still declines cleanly. The outer tuple flattens to
           `(v-disc:i32, e0:i32, e1:i64, n:i64)`. run() builds (C((3,7)), 5) and performs probe.push; a VALID
           running component (live-objects=0) pins the register element round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (variant (a) (b (s64)) (c (tuple (s32) (s64)))) (s64))) (result (s64)))))))
  (input
    (do
      (type Sig (A) (B Int64) (C (Tuple Int32 Int64)))
      (effect probe (op push (-> (Tuple Sig Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Sig.C #tuple(3 7)) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a RECORD host-op arg with a mixed variant{a, b(s64), c(record{p: s32, q: s64})} field crosses (WIT-ordered record payload case, register flatten)"
  (doc
    "SHAPE 266 (v-wit-boundary) — a RECORD host-op ARGUMENT `record{ v: variant{a, b(s64), c(record{p: s32,
           q: s64})}, n: s64 }` (probe.push) whose `v` FIELD is a HETEROGENEOUS MIXED variant with a RECORD
           payload case (c) beside a scalar case (b) + a nullary case (a), at the REGISTER record-FIELD position.
           SHAPE 264 landed the scalar+TUPLE mix at this position but a RECORD payload case CDZ0910'd — the
           component variant type built by `host_imports::record_field_cref` declared the record case as NULLARY
           (payload `None`), so its canonical flatten under-counted by the record's slots and disagreed with
           serialize's `VariantMemMixed` join (`expected (i32 i64 i64 i64)` vs `found (i32 i64 i64)`). Fixed:
           `record_field_cref`'s VariantMemMixed arm now lays a real `(record (p s32) (q s64))` DEFINED type for a
           Record case (kebab field names in guest NAME-LEX order), so the component type flattens to the same
           slots serialize + the guest push (`emit_variant_mixed_arg_reg_flatten`'s Record arm) produce. The
           register arm re-admits a Record case guarded by `mixed_variant_record_cases_wit_ordered` (guest name-lex
           order == WIT declaration order — here p<q ↔ WIT `(p s32)(q s64)` — since the component `(record …)` is
           built name-lex; a DIVERGENT order still declines cleanly, decline-don't-miscompile). The bare-ARG +
           mem `list`-element (SHAPE 261) Record cases were always fine. The whole record flattens to
           `(v-disc:i32, p:i64, q:i64, n:i64)` (p's i32 joins into the i64 slot 0). run() builds
           { v: C({p:3, q:7}), n: 5 } and performs probe.push; a VALID running component (live-objects=0) pins the
           register record-payload-case round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (record (= v (variant (a) (b (s64)) (c (record (= p (s32)) (= q (s64)))))) (= n (s64)))) (result (s64)))))))
  (input
    (do
      (type Sig (A) (B Int64) (C (Record (: p Int32) (: q Int64))))
      (effect probe (op push (-> (Record (: v Sig) (: n Int64)) Int64)))
      (def (run) (host (probe) (probe.push #record((= v (Sig.C #record((= p 3) (= q 7)))) (= n 5)))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))

(case
  "a top-level tuple<variant{a, b(s64), c(record{p: s32, q: s64})}, s64> host-op arg crosses (WIT-ordered record payload case at a register tuple ELEMENT)"
  (doc
    "SHAPE 267 (v-wit-boundary) — a top-level `tuple<variant{a, b(s64), c(record{p: s32, q: s64})}, s64>` bare
           host-op ARGUMENT (probe.push) whose ELEMENT 0 is a HETEROGENEOUS MIXED variant with a RECORD payload
           case (c) at the REGISTER tuple-ELEMENT position — the tuple-element twin of SHAPE 266 (record-FIELD).
           SHAPE 265 landed the scalar+tuple mix at this position but excluded a Record case (the register-Record
           defect, since fixed in SHAPE 266). Now that `record_field_cref` lays a proper `(record …)` DEFINED type
           for a Record case (SHAPE 266), the tuple-element CRef path (build_host_result_types → record_field_cref)
           produces the correct component type, so `emit_tuple_reg_flatten`'s mixed-variant element arm re-admits a
           Record case — guarded by `mixed_variant_record_cases_wit_ordered` on the ELEMENT's WIT
           (`elem_wits[i]`), so a divergent guest-name-lex-vs-WIT order still declines cleanly
           (decline-don't-miscompile). `tuple_arg_crosses` admits the element (no per-element WIT there; the emit
           runs the order guard). The outer tuple flattens to `(v-disc:i32, p:i64, q:i64, n:i64)`. run() builds
           (C({p:3, q:7}), 5) and performs probe.push; a VALID running component (live-objects=0) pins the register
           tuple-element record-payload-case round-trip.")
  (wit-world
    (world w (import cadenza:platform/probe
      (member push (func (param m (tuple (variant (a) (b (s64)) (c (record (= p (s32)) (= q (s64))))) (s64))) (result (s64)))))))
  (input
    (do
      (type Sig (A) (B Int64) (C (Record (: p Int32) (: q Int64))))
      (effect probe (op push (-> (Tuple Sig Int64) Int64)))
      (def (run) (host (probe) (probe.push #tuple((Sig.C #record((= p 3) (= q 7))) 5))))
      (export run)))
  (call run)
  (host-responses (respond probe.push (: 55 Int64)))
  (host-calls (call cadenza:platform/probe.push))
  (output 55)
  (live-objects 0))
