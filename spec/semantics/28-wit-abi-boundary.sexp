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

(case "an option<s64> field in a record result VALUE round-trips via the run/encode envelope both arms (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc    "The general option<T> RESULT lift across the WIT export boundary. The guest takes a record
           `{ x: Int64 }` and returns a record `{ d: Option Int64 }`, mapping x=0 to None and any
           other x to Some(x). Asserting BOTH arms exercises the Option discriminant (Some vs None)
           and, on the Some arm, the s64 payload — a broken lift (wrong disc, wrong payload offset,
           or a dropped payload) produces a different d. Migrated from the in-crate wasmtime test
           `an_option_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 1/18).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             #record((= d (if (= (. m x) 0) Option.None (Option.Some (. m x))))))
           (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (output (: #record((= d (Some 42))) (record (d (Option Int64)))))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= d (None unit))) (record (d (Option Int64)))))
  (live-objects known-leak))

(case "a named variant-with-payload field in a record result VALUE round-trips via the run/encode envelope both arms (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc    "SHAPE 2 — the general named-VARIANT RESULT lift (a discriminated union, distinct from a bare
           enum: one case carries a payload). The guest returns a record `{ o: Outcome }` where
           `Outcome = Continue | Close(Int64)`, mapping x=0 to Continue (nullary) and any other x to
           Close(x) (s64 payload). Asserting BOTH arms exercises the variant discriminant AND the
           payload case's s64 lane — a broken lift (wrong disc, missing/misplaced payload) yields a
           different o. Migrated from the in-crate wasmtime test
           `a_named_variant_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 2/18).")
  (input (do
           (type Outcome (Continue) (Close Int64))
           (def (f (: m (Record (: x Int64))))
             #record((= o (if (= (. m x) 0) Outcome.Continue (Outcome.Close (. m x))))))
           (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= o (Continue unit))) (record (o Outcome))))
  (call f (: #record((= x 7)) (Record (: x Int64))))
  (output (: #record((= o (Close 7))) (record (o Outcome))))
  (live-objects known-leak))

(case "a scalar export crosses the WIT boundary and returns its argument"
  (doc    "SHAPE 3 — the simplest WIT export boundary: s64 in, s64 out (identity). Exercises the plain
           scalar param/result canonical-ABI lowering with no compound structure. Migrated from the
           in-crate wasmtime test `a_scalar_interface_export_guest_compiles_and_runs` (v-rb feed 4).")
  (input (do (def (f (: x Int64)) x) (export f)))
  (call f (: 7 Int64))
  (output (: 7 Int64)))

(case "a multi-field record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc    "SHAPE 4 — the multi-field RECORD RESULT spill: the guest returns { a, b } from a record
           input, exercising the record result lift (two s64 fields, in order). Migrated from the
           in-crate wasmtime test `a_record_result_guest_compiles_and_runs_via_result_spill` (v-rb feed 5).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             #record((= a (. m x)) (= b (+ (. m x) (. m x)))))
           (export f)))
  (call f (: #record((= x 21)) (Record (: x Int64))))
  (output (: (record (= a 21) (= b 42)) (Record (: a Int64) (: b Int64))))
  (live-objects known-leak))

(case "a bare list of scalars in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc    "SHAPE 5 — the bare-LIST RESULT: a record field that is a list<s64>, exercising the list
           result lift (element count + s64 element stride). Migrated from the in-crate wasmtime test
           `a_list_result_guest_compiles_and_runs` (v-rb feed 6).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             #record((= xs #list((. m x) (+ (. m x) (. m x))))))
           (export f)))
  (call f (: #record((= x 5)) (Record (: x Int64))))
  (output (: #record((= xs #list(5 10))) (record (xs (List Int64)))))
  (live-objects known-leak))

(case "a list of records in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; a typed record/sum EXPORT is a separate gap)"
  (doc    "SHAPE 7 — the list-of-records RESULT (the structural workhorse of every reducer Step's
           `requests: list<request>`): a record field that is a LIST of RECORDS. The guest returns
           `{ items: [ { a, b } ] }` with one element built from the input (a=x, b=x+x). Exercises the
           LIST result lift (count + stride) AND the element RECORD lift (two s64 fields in order).
           Migrated from the in-crate wasmtime test `a_list_of_records_result_guest_compiles_and_runs`
           (v-rust-backend shape-2 feed 3/18).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             #record((= items #list(#record((= a (. m x)) (= b (+ (. m x) (. m x))))))))
           (export f)))
  (call f (: #record((= x 7)) (Record (: x Int64))))
  (output (: #record((= items #list(#record((= a 7) (= b 14))))) (record (items (List (record (a Int64) (b Int64)))))))
  (live-objects known-leak))

(case "a FLOAT-KEYED map result round-trips via the run/encode envelope, rendering its float keys (float map-key render, #6211 key-adopt/use + #6274 render)"
  (doc    "The float-KEYED map RESULT: a `(Map Float32 Int64)` crosses via the run/encode value-form escape
           (crosses_as_resource_escape) and renders `#map((= <key> <value>) …)` in canonical key order. The
           keys are Float32 — a bare `2.0` ADOPTS the annotated `(: 1.0 Float32)` sibling's width (seq-40),
           and each float key is stored in the total-order `__CdzF32` Ord wrapper on the rust backend. Pins
           that BOTH backends render the float keys correctly: the wasm value-form render_val (v-rb) and the
           rust boundary value-form render (which must UNWRAP the `__CdzF{N}` shell before the float render —
           the E0605/E0624/private-interface chain #6274 closed). Both keys 1.0/2.0 are f32-exact, so the
           render shows `1.0`/`2.0`. Guards the float-keyed-collection RENDER path against regression.")
  (input (do (def (main) (Map.insert (Map.insert Map.empty (: 1.0 Float32) 5) 2.0 6)) (export main)))
  (call main)
  (output (: #map((= 1.0 5) (= 2.0 6)) (Map Float32 Int64)))
  (live-objects known-leak))

(case "a none-only option<s64> record field resolves its element type from an imposed WIT world"
  (doc    "SHAPE 8 — the general WIT-ABI shape where the export boundary is DECLARED by an explicit WIT
           world, not synthesized from the guest. The guest's field d is STATICALLY Option.None (no Some
           arm), so its option element type cannot be inferred from a payload — it is fixed by the world's
           `d: option<s64>` declaration (the `(wit-world …)` clause). The guest exports f UNDER the
           interface `cadenza:demo/iface` (`(component-name …)`), so the run invokes it through that
           interface instance. A broken WIT-element-type resolution fails to emit or mis-types d. Migrated
           from the in-crate wasmtime test `a_none_only_option_result_resolves_via_wit`.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (record (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) #record((= d Option.None))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output #record((= d (None unit))))
  (live-objects known-leak))

(case "a list of records with a bytes leaf crosses the export boundary via an imposed WIT world"
  (doc    "SHAPE 9 — the reducer-echo Step shape (list of records each carrying a Bytes/list<u8> leaf,
           AND a Bytes/list<u8> export PARAM), via an imposed world. Guest f: {tok: Bytes} ->
           {items: [{echo: tok}]}; exercises the list result lift, the element record lift, a bytes leaf
           that echoes the input, AND the list<u8> export-param decode. A guest Bytes field crosses as the
           world's list<u8>. Migrated from the in-crate wasmtime test
           `a_list_of_records_with_bytes_leaf_result_compiles_and_runs` (v-rb shape-2 feed 7).")
  (wit-world (world w (export iface (member f (func (param m (record (= tok (list (u8))))) (result (record (= items (list (record (= echo (list (u8)))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: tok Bytes)))) #record((= items #list(#record((= echo (. m tok))))))) (export f)))
  (call f (: #record((= tok #list(10 20 30))) (Record (: tok Bytes))))
  (output #record((= items #list(#record((= echo b"\n\x14\x1e"))))))
  (live-objects known-leak))

(case "a bare list<u8>/Bytes RESULT member of a typed export interface crosses (multi-export list<u8> result — operator §2 encode-quoted half)"
  (doc    "SHAPE 34 — a TOP-LEVEL bare `list<u8>`/Bytes RESULT member of a DECLARED export interface (not a
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
  (live-objects known-leak))

(case "a bare list<u8>/Bytes PARAM member of a typed export interface crosses (multi-export list<u8> param — operator §2 decode-check half)"
  (doc    "SHAPE 40 — a TOP-LEVEL bare `list<u8>`/Bytes PARAM member of a DECLARED export interface (not a
           record leaf like SHAPE 22, nor a single bare export like the plain-export entry-param route). The
           typed-interface wrapper copies the incoming `(ptr,len)` `list<u8>` out of linear memory into a
           value-heap Bytes handle (the `mem_leaf_params` lift — `bytes-alloc`/`bytes-set` copy-in), passes it
           to the def, and reclaims the borrowed handle after the call (`drop`). This is the `decode-check`
           half of the operator-mandated single-component TWO-export shape (§2, seq-107/108) — the inverse of
           the `encode-quoted` bytes-RESULT member (`ResultLower::CopyBytes`). Guest decodeCheck(x: Bytes) =
           Bytes.len(x) > 0; calling with `#list(104 105)` returns true, proving the byte-leaf param copy-in +
           the borrow reclaim end to end. Guards the bytes-PARAM-member lift against regression.")
  (wit-world (world w (export iface (member decode-check (func (param x (list (u8))) (result (bool)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (decodeCheck (: x Bytes)) (> (Bytes.len x) 0)) (export decodeCheck)))
  (call decode-check (: #list(104 105) Bytes))
  (output (: true Bool))
  (live-objects known-leak))

(case "a bare string/String PARAM member of a typed export interface crosses (mem_leaf Str-arm coverage)"
  (doc    "SHAPE 42 — a TOP-LEVEL bare `string`/String PARAM member of a typed export interface. Same
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
  (live-objects known-leak))

(case "a single component exports BOTH a list<u8>-result member and a list<u8>-param member of one interface (operator §2 two-export capstone)"
  (doc    "SHAPE 41 — the CAPSTONE of the operator-mandated single-component TWO-export shape (§2, seq-107/108):
           ONE interface with BOTH members — encode-quoted() -> list<u8> (the bytes-RESULT member, emitted via
           `ResultLower::CopyBytes`, SHAPE 34) AND decode-check(list<u8>) -> bool (the bytes-PARAM member,
           lifted via the `mem_leaf_params` copy-in, SHAPE 40). `record_interface_export` emits one boundary
           wrapper PER member, so this proves the two independently-landed member emitters COMPOSE in a single
           component: the result-copy-out wrapper and the param-copy-in wrapper coexist, share the one memory +
           `cabi_realloc` + the two `list<u8>` scratch locals, and each crosses its own bytes independently.
           Guest defines encodeQuoted() = Bytes.of([104,105]) and decodeCheck(x) = Bytes.len(x) > 0; running
           BOTH (encode-quoted -> #list(104 105); decode-check([104,105]) -> true) exercises the full two-export
           boundary end to end — the shape the operator's real encode/decode contract needs.")
  (wit-world (world w (export iface (member encode-quoted (func (result (list (u8))))) (member decode-check (func (param x (list (u8))) (result (bool)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (encodeQuoted) (Bytes.of #list(104 105))) (def (decodeCheck (: x Bytes)) (> (Bytes.len x) 0)) (export encodeQuoted) (export decodeCheck)))
  (call encode-quoted)
  (output #list(104 105))
  (call decode-check (: #list(104 105) Bytes))
  (output (: true Bool))
  (live-objects known-leak))

(case "a bare list<scalar>/List PARAM member of a typed export interface crosses (mem_leaf List-arm coverage)"
  (doc    "SHAPE 43 — a TOP-LEVEL `list<s64>`/List Int64 PARAM member of a typed export interface. Unlike the
           Bytes byte-leaf copy (SHAPE 40), `MemLeafKind::List` builds a value-heap VEC element-by-element
           (`vec-empty` + per-element load-at-stride / `box-int` / `vec-push`) from the canonical `(ptr, count)`
           layout — a DISTINCT value rep from Bytes (boxed elements, not a packed byte-leaf). The guest reads
           BOTH the count AND an element value to prove the stride + box are right: readElem(xs) =
           100*List.len(xs) + xs[1]; [7,42,9] -> 342 (a broken stride/box would mis-read element 1). Borrow-only
           0-leak lift (the wrapper drops the vec after the call). Mirrors the bare-entry route's list<scalar>
           param (el4/el5, ch09) onto the typed-interface-MEMBER route.")
  (wit-world (world w (export iface (member read-elem (func (param xs (list (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (readElem (: xs (List Int64))) (+ (* 100 (List.len xs)) (match (List.at xs 1) ((Option.Some v) v) ((Option.None) -1)))) (export readElem)))
  (call read-elem (: #list(7 42 9) (List Int64)))
  (output (: 342 Int64))
  (live-objects known-leak))

(case "a bare option<scalar>/Option PARAM member of a typed export interface crosses (sum_params option arm, both variants)"
  (doc    "SHAPE 44 — a TOP-LEVEL `option<s64>`/Option Int64 PARAM member of a typed export interface. The
           member crosses as a native component `option<s64>` flattened to `(disc, payload)`; the wrapper
           branches on the boundary disc and builds the guest sum cell via `sum-new` (`SumArgRebuild`), passes
           it to the def, and drops the borrowed shell after the call (the extracted payload escapes by its own
           copy, independent of the shell). Mirrors the bare-entry route's Option param (eo1/eo2, ch09) onto
           the typed-interface-MEMBER route — the `sum_params` sibling of the mem_leaf param arms. Guest
           checkOpt(x) = match x (Some v)->v (None)->-1; BOTH variants exercised: Some(42)->42, None->-1 (a
           broken disc read or payload-cursor would take the wrong arm).")
  (wit-world (world w (export iface (member check-opt (func (param x (option (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (checkOpt (: x (Option Int64))) (match x ((Option.Some v) v) ((Option.None) -1))) (export checkOpt)))
  (call check-opt (: (Some 42) (Option Int64)))
  (output (: 42 Int64))
  (call check-opt (: (None unit) (Option Int64)))
  (output (: -1 Int64))
  (live-objects known-leak))

(case "a member with TWO top-level mem-leaf params (list<u8> + list<s64>) threads the flattened cursor across both"
  (doc    "SHAPE 45 — a typed-interface member with TWO top-level memory-bearing params: a `list<u8>`/Bytes AND
           a `list<s64>`/List, each flattening to `(ptr, len)`. The wrapper copies BOTH out of memory (bytes-leaf
           + list-vec) in sequence, advancing the flattened-leaf cursor by 2 per param, then reclaims both. A
           cursor that failed to advance past the first `(ptr,len)` would read the second param at the wrong
           offset. Guest combine(b, xs) = Bytes.len(b) + List.len(xs); ([1,2,3], [10,20]) -> 5. Pins the
           multi-mem-leaf-param composition established by SHAPE 40/43.")
  (wit-world (world w (export iface (member combine (func (param b (list (u8))) (param xs (list (s64))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (combine (: b Bytes) (: xs (List Int64))) (+ (Bytes.len b) (List.len xs))) (export combine)))
  (call combine (: #list(1 2 3) Bytes) (: #list(10 20) (List Int64)))
  (output (: 5 Int64))
  (live-objects known-leak))

(case "a member with a mem-leaf param interleaved with a scalar param threads the cursor across mixed widths"
  (doc    "SHAPE 46 — a typed-interface member mixing a top-level `list<u8>`/Bytes param (flattens to `(ptr,len)`
           = 2 leaves) with a bare `s64` SCALAR param (1 leaf). The wrapper must advance the flattened-leaf
           cursor by 2 for the mem-leaf and by 1 for the scalar; a miscount would swap them. Guest tag(x, n) =
           Bytes.len(x) + n; ([7,8,9], 100) -> 103. Pins the mem-leaf + scalar mixed-width param threading.")
  (wit-world (world w (export iface (member tag (func (param x (list (u8))) (param n (s64)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (tag (: x Bytes) (: n Int64)) (+ (Bytes.len x) n)) (export tag)))
  (call tag (: #list(7 8 9) Bytes) (: 100 Int64))
  (output (: 103 Int64))
  (live-objects known-leak))

(case "a member with an option<s64> param beside a mem-leaf param composes the sum rebuild with the byte copy-in"
  (doc    "SHAPE 47 — a typed-interface member with a top-level `option<s64>` param (sum rebuild: disc + payload)
           beside a `list<u8>`/Bytes param (mem-leaf: ptr + len). The wrapper builds the sum cell (branching on
           the disc, advancing the cursor past disc+payload) THEN copies the bytes out (advancing past ptr+len),
           reclaiming both borrowed cells. Pins the sum-param + mem-leaf-param cursor composition (a broken sum
           payload-cursor would offset the bytes param). Guest both(o, b) = (match o Some->v None->0) +
           Bytes.len(b); (Some 40, [1,2]) -> 42.")
  (wit-world (world w (export iface (member both (func (param o (option (s64))) (param b (list (u8))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (both (: o (Option Int64)) (: b Bytes)) (+ (match o ((Option.Some v) v) ((Option.None) 0)) (Bytes.len b))) (export both)))
  (call both (: (Some 40) (Option Int64)) (: #list(1 2) Bytes))
  (output (: 42 Int64))
  (live-objects known-leak))

(case "a reducer performing a scalar host import threads the u64 result into the step (via an imposed WIT world)"
  (doc    "SHAPE 10 — a scalar host-import RESULT (clock.now : () -> u64) driven through an imposed WIT world.
           The reducer on-message performs clock.now (nullary scalar host op) and threads the u64 into the
           step's request deadline-nanos = Some(now). Stubbing clock.now -> 42 + asserting deadline-nanos ==
           Some(42) makes the scalar host result LOAD-BEARING. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_scalar_host_import_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/clock (member now (func (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect clock (op now (-> Unit UInt64))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (clock) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos (Option.Some (clock.now unit)))))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond clock.now (: 42 UInt64)))
  (host-calls (call cadenza:platform/clock.now))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(2)) (= token #list(3)) (= deadline-nanos (Some 42))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a reducer performing a RECORD host import reads a field of the result (via an imposed WIT world)"
  (doc    "SHAPE 11 — a RECORD host-import RESULT (probe.info : (Bytes) -> record{zebra, alpha}) driven through
           an imposed WIT world; the host RECORD's declared order (zebra, alpha) differs from the guest name-lex
           order, exercising the field-order-follows-WIT reorder on the lift. The reducer reads the result's
           `alpha` field into the request payload. Stubbing probe.info -> {zebra:(9), alpha:(7)} and asserting
           payload == (7) makes the record host result + its field-reorder load-bearing. Migrated from the
           in-crate wasmtime test `a_reducer_performing_a_record_result_host_op_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/probe (member info (func (param key (list (u8))) (result (record (= zebra (list (u8))) (= alpha (list (u8))))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect probe (op info (-> Bytes (Record (: zebra Bytes) (: alpha Bytes))))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (probe) #record((= requests #list(#record((= contract (. m contract)) (= payload (. (probe.info (. m token)) alpha)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond probe.info (: #record((= zebra #list(9)) (= alpha #list(7))) (Record (: zebra Bytes) (: alpha Bytes)))))
  (host-calls (call cadenza:platform/probe.info))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(7)) (= token #list(3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a typed reducer performing a list-of-scalars host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 12 — a `list<s64>` host-op ARG (sink.push : (list<s64>) -> unit) driven through an imposed
           WIT world. The reducer on-message performs sink.push (list 1 2 3) (a unit-result, observe-only host
           op) then returns a Continue step with an empty requests list. Running the guest LOADS the emitted
           component and INVOKES sink.push, exercising the list<s64> arg lower (value-heap List<Int64> -> the
           (ptr,count) the component list<s64> param lowers to, elements unboxed at i64 stride) e2e — a strictly
           stronger check than the anchor's emit+validate+load, since a broken flatten-arity or element stride
           fails to instantiate or run. The observed host-call sequence pins that sink.push actually fired.
           Migrated from the in-crate wasmtime test `a_reducer_performing_a_list_scalar_arg_emits_and_loads`
           (v-rust-backend shape-2 synthetic arg feed, align-8 stress).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param vals (list (s64))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List Int64) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(1 2 3)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a typed reducer performing an all-scalar record host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 13 — an all-scalar `record { a: s64, b: s64 }` host-op ARG (deliver.push : (record{a,b}) -> unit)
           driven through an imposed WIT world. The reducer on-message performs deliver.push (record a=1 b=2)
           (a unit-result, observe-only host op) then returns a Continue step with an empty requests list.
           Running the guest LOADS the emitted component and INVOKES deliver.push, exercising the record arg
           lower (value-heap record -> the two i64 core slots the component record param flattens to, each field
           unboxed in the WIT record's declared order) e2e — a strictly stronger check than the anchor's
           emit+validate+load, since a broken field flatten (wrong arity/order/offset) fails to instantiate or
           run. The observed host-call sequence pins that deliver.push actually fired. Migrated from the in-crate
           wasmtime test `a_typed_reducer_with_a_record_arg_host_import_emits_and_loads` (v-rb shape-2 arg feed).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/deliver (member push (func (param r (record (= a (s64)) (= b (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect deliver (op push (-> (Record (: a Int64) (: b Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (deliver) (do (deliver.push #record((= a 1) (= b 2))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/deliver.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a typed reducer performing a bytes host arg with a scalar result threads the u64 into the step (via an imposed WIT world)"
  (doc    "SHAPE 14 — a Bytes host-op ARG with a scalar RESULT (hasher.hash : (Bytes) -> u64) driven through an
           imposed WIT world. The reducer on-message performs hasher.hash(m.payload) (a list<u8> ARG, u64 result)
           and threads the u64 into the step's request deadline-nanos = Some(hash). Stubbing hasher.hash -> 42
           and asserting deadline-nanos == Some(42) makes the call load-bearing: the u64 result only reaches the
           output if the Bytes arg lowered and the call succeeded. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_bytes_param_host_import_emits_and_loads` (v-rb shape-2 arg feed 2/6).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/hasher (member hash (func (param bytes (list (u8))) (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect hasher (op hash (-> Bytes UInt64))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (hasher) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos (Option.Some (hasher.hash (. m payload))))))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond hasher.hash (: 42 UInt64)))
  (host-calls (call cadenza:platform/hasher.hash))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(2)) (= token #list(3)) (= deadline-nanos (Some 42))))) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a typed reducer performing a run.run result host op threads the Ok bytes into the step (via an imposed WIT world)"
  (doc    "SHAPE 15 — a COMPOUND result<Bytes, enum> host-import RESULT (run.run : (Bytes,Bytes,Bytes) -> result<list<u8>, variant{timeout,faulted}>)
           driven through an imposed WIT world. The reducer on-message performs run.run(contract,contract,payload) and
           matches the result: Ok(v) -> the request payload is v; Err(_) -> the payload falls back to m.payload. Stubbing
           run.run -> Ok(b\"RAN\") and asserting payload == (82 65 78) makes the compound result<T,E> lift load-bearing (the
           spilled result disc + Ok list<u8> payload). Migrated from the in-crate wasmtime test
           `a_reducer_performing_run_with_a_result_host_result_emits_and_loads` (v-rb shape-2 run.run feed; #3301 landed the
           RunSink host half so this needs NO Entry::Run — host-responses stubs the result + output-assertion suffices).
           The sole-use world builder is RETAINED (a separate WIT-type unit test still reads it), so only the anchor retires.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/run (member run (func (param program (list (u8))) (param contract (list (u8))) (param input (list (u8))) (result (result (list (u8)) (variant (timeout) (faulted)))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type Error (Timeout) (Faulted)) (effect run (op run (-> Bytes Bytes Bytes (Result Bytes Error)))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (run) #record((= requests #list(#record((= contract (. m contract)) (= payload (match (run.run (. m contract) (. m contract) (. m payload)) ((Ok v) v) ((Err _e) (. m payload)))) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond run.run (: (Ok #list(82 65 78)) (Result Bytes Error))))
  (host-calls (call cadenza:platform/run.run))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(82 65 78)) (= token #list(3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))
(case "an option<s64> param field is read and rebuilt by the wrapper on both arms (via an imposed WIT world)"
  (doc    "SHAPE 16 — the option<T> PARAM lift (the read side, complement to SHAPE 1's option RESULT): the guest
           f takes a record { d: option<s64> } and matches it (Some(x) -> x, None -> -1). Feeding d=Some(42) -> 42
           and d=None -> -1 exercises BOTH arms of the boundary option read (disc None=0/Some=1 + the payload at
           the variant payload offset), which the wrapper rebuilds into the guest option cell. The export crosses
           under the interface cadenza:demo/iface. Migrated from the in-crate wasmtime test
           `an_option_param_field_is_read_by_the_wrapper` (the param-side variant reader, deadline-nanos read shape).")
  (wit-world (world w (export iface (member f (func (param m (record (= d (option (s64))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: d (Option Int64))))) (match (. m d) ((Option.Some x) x) (Option.None (- 0 1)))) (export f)))
  (call f (: #record((= d (Some 42))) (Record (: d (Option Int64)))))
  (output (: 42 Int64))
  (call f (: #record((= d None)) (Record (: d (Option Int64)))))
  (output (: -1 Int64))
  (live-objects known-leak))
(case "a result<Bytes, enum> param field is read and rebuilt by the wrapper on Ok and Err arms (via an imposed WIT world)"
  (doc    "SHAPE 17 — the result<Ok, Err> PARAM lift (the read side, complement to SHAPE 15's result RESULT): the
           guest f takes a record { a: result<Bytes, Error> } where Error is a 4-case enum, and matches m.a:
           Ok(bs) -> Bytes.len(bs); Err(e) -> 10 + e's decl disc. Feeding Ok([1,2,3]) -> 3 exercises the Bytes Ok
           arm (ptr/len copy-in inside the sum); Err(timeout) -> 10 and Err(faulted) -> 13 exercise the Enum Err
           arm (the flattened disc leaf rebuilt into the guest error cell, disc-preserving) - a misread of the
           Bytes arm's 2 leaves would misalign the enum disc and flip the Err results. The enum-err ARG is passed
           as the render form `(<case> unit)` (cdz-run's coerce_one gained a Type::Enum arm for this). Migrated
           from the in-crate wasmtime test `a_result_bytes_enum_param_field_is_read_by_the_wrapper`.")
  (wit-world (world w (export iface (member f (func (param m (record (= a (result (list (u8)) (enum timeout missing schema faulted))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Error (Timeout) (Missing) (Schema) (Faulted)) (def (f (: m (Record (: a (Result Bytes Error))))) (match (. m a) ((Result.Ok bs) (Bytes.len bs)) ((Result.Err e) (match e (Error.Timeout 10) (Error.Missing 11) (Error.Schema 12) (Error.Faulted 13))))) (export f)))
  (call f (: #record((= a (Ok #list(1 2 3)))) (Record (: a (Result Bytes Error)))))
  (output (: 3 Int64))
  (call f (: #record((= a (Err (timeout unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 10 Int64))
  (call f (: #record((= a (Err (faulted unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 13 Int64))
  (live-objects known-leak))
(case "a named-variant result writer name-matches (not positionally) with a reversed guest decl (via an imposed WIT world)"
  (doc    "SHAPE 18 — the named-VARIANT writer keys on the case NAME, not decl position. The guest declares its sum
           in the OPPOSITE order to the WIT variant cases: (type Rev (Close Int64) (Continue)) (Close is guest
           decl-disc 0) against the world's variant { continue, close(s64) } (continue is boundary case 0). A
           name-match maps Close->close (boundary disc 1) and Continue->continue (boundary disc 0); a POSITIONAL
           match would put Close(payload) onto the nullary continue case and the payload-shape guard would reject
           at compile. A green run of BOTH arms (x=0 -> continue, x=5 -> close 5) proves the writer is keyed on the
           case NAME, immune to guest decl reordering. Migrated from the in-crate wasmtime test
           `the_named_variant_writer_name_matches_not_positionally`.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (record (= o (variant (continue) (close (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Rev (Close Int64) (Continue)) (def (f (: m (Record (: x Int64)))) #record((= o (if (= (. m x) 0) Rev.Continue (Rev.Close (. m x)))))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output #record((= o (continue unit))))
  (call f (: #record((= x 5)) (Record (: x Int64))))
  (output #record((= o (close 5))))
  (live-objects known-leak))
(case "a record param field is read by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc    "SHAPE 19 — a record PARAM whose WIT field order differs from the guest name-lex order is read by NAME, not
           position. The world declares f's param as record { payload: list<u8>, contract: list<u8> } (WIT order
           payload,contract), while the guest reads (. m contract) and returns Bytes.len(m.contract). Calling with
           payload=[9,9] (len 2) and contract=[1,2,3] (len 3) must return 3 (the contract length): a positional
           misroute would read payload and return 2. Proves the param permute keys on the field NAME across the
           WIT/guest order mismatch. Migrated from the in-crate wasmtime test
           `a_non_name_lex_record_param_permutes_by_name`.")
  (wit-world (world w (export iface (member f (func (param m (record (= payload (list (u8))) (= contract (list (u8))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) (Bytes.len (. m contract))) (export f)))
  (call f (: #record((= payload #list(9 9)) (= contract #list(1 2 3))) (Record (: contract Bytes) (: payload Bytes))))
  (output (: 3 Int64))
  (live-objects known-leak))
(case "a record result field is written by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc    "SHAPE 20 — the record RESULT writer places fields by NAME, not by the guest name-lex slot order. The
           world declares f's result as record { second: s64, first: s64 } (WIT order second,first; name-lex is
           first < second), the shape of a real step/request (declaration-ordered, not alphabetical). The guest
           builds { first: m.x, second: 2*m.x }; the writer must place first at WIT-position 1 and second at
           WIT-position 0, reading each from its guest name-lex slot. f({x:10}) renders (in WIT order) as
           { second: 20, first: 10 } - a positional write would swap them. Migrated from the in-crate wasmtime
           test `a_non_name_lex_record_result_permutes_by_name`.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (record (= second (s64)) (= first (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) #record((= first (. m x)) (= second (+ (. m x) (. m x))))) (export f)))
  (call f (: #record((= x 10)) (Record (: x Int64))))
  (output #record((= second 20) (= first 10)))
  (live-objects known-leak))
(case "a nested record param with a bytes leaf compiles and runs (via an imposed WIT world)"
  (doc    "SHAPE 21 — a record PARAM with a NESTED record field carrying a list<u8> leaf, the shape of a reducer
           message's sender (a record-within-record with byte leaves). The guest reads both the outer bytes leaf
           and the nested one: f(m: record{a: Bytes, sub: record{b: Bytes}}) = Bytes.len(m.a) + Bytes.len(m.sub.b).
           The wrapper builds the outer value-heap cell with a nested sub-cell for sub, copying each list<u8> leaf
           out of shared memory. f({a:[1,2], sub:{b:[1,2,3]}}) == 5 (len(a)=2 + len(sub.b)=3). Migrated from the
           in-crate wasmtime test `a_nested_record_bytes_param_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= a (list (u8))) (= sub (record (= b (list (u8))))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Bytes) (: sub (Record (: b Bytes)))))) (+ (Bytes.len (. m a)) (Bytes.len (. (. m sub) b)))) (export f)))
  (call f (: #record((= a #list(1 2)) (= sub #record((= b #list(1 2 3))))) (Record (: a Bytes) (: sub (Record (: b Bytes))))))
  (output (: 5 Int64))
  (live-objects known-leak))
(case "a record param carrying a bytes leaf beside a scalar compiles and runs (via an imposed WIT world)"
  (doc    "SHAPE 22 — a record PARAM carrying a list<u8> LEAF beside a scalar (the memory boundary every real
           reducer needs: Message/Step carry list<u8>). The canon lift lowers the incoming `data` list into the
           guest's linear memory, the wrapper copies those bytes into a value-heap Bytes, builds the {data, tag}
           record, and the def returns Bytes.len(data). f({data:[1,2,3,4,5], tag:99}) == 5 proves the copied bytes
           have the right length (bytes-alloc + the copy loop + the memory lift all agree end to end). Migrated
           from the in-crate wasmtime test `a_record_with_a_bytes_leaf_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= data (list (u8))) (= tag (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: data Bytes) (: tag Int64)))) (Bytes.len (. m data))) (export f)))
  (call f (: #record((= data #list(1 2 3 4 5)) (= tag 99)) (Record (: data Bytes) (: tag Int64))))
  (output (: 5 Int64))
  (live-objects known-leak))
(case "a record param interface export builds the record via the boundary wrapper and runs (via an imposed WIT world)"
  (doc    "SHAPE 23 — a RECORD-param interface export handled by the boundary WRAPPER (the on-message(message)->step
           shape, record in). The canon lift hands the def the flattened field; the wrapper builds the value-heap
           record handle then calls the def. Guest f(m: record{a: s64}) = m.a; f({a:7}) == 7 proves the wrapper's
           record build (arr-alloc/box-int) + the field read agree end to end. Migrated from the in-crate wasmtime
           test `a_record_param_guest_compiles_and_runs_via_a_wrapper`.")
  (wit-world (world w (export iface (member f (func (param m (record (= a (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Int64)))) (. m a)) (export f)))
  (call f (: #record((= a 7)) (Record (: a Int64))))
  (output (: 7 Int64))
  (live-objects known-leak))
(case "a multi-export record interface guest emits a wrapper per member and runs both (via an imposed WIT world)"
  (doc    "SHAPE 25 — a MULTI-EXPORT record-interface guest: the world's interface iface has TWO record-param
           members f(record{a: s64})->s64 and g(record{b: s64})->s64 (the shape a real reducer needs:
           on-message/on-response/on-notification are separate members). The compiler emits one boundary wrapper
           per member appended to the core module. Guest defines both f(m)=m.a and g(m)=m.b; running BOTH under
           the interface (f({a:7})==7, g({b:9})==9) proves each member's wrapper builds its own record + reads its
           own field independently. Migrated from the in-crate wasmtime test
           `a_multi_export_record_interface_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= a (s64)))) (result (s64)))) (member g (func (param m (record (= b (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Int64)))) (. m a)) (def (g (: m (Record (: b Int64)))) (. m b)) (export f) (export g)))
  (call f (: #record((= a 7)) (Record (: a Int64))))
  (output (: 7 Int64))
  (call g (: #record((= b 9)) (Record (: b Int64))))
  (output (: 9 Int64))
  (live-objects known-leak))

(case "a reducer emitting no effects builds an empty-requests step and runs (via an imposed WIT world)"
  (doc    "SHAPE 26 - a reducer that emits NO effects: an empty requests list with a Continue outcome, a very common
           output (a fold that only reads or updates state). The empty list has an unresolved element type, so the
           result writer must derive the dead element writer of the request list from the WIT type alone
           (canon_write_from_wit - the same principle as the None-only option), or emit falls through to a
           wrong-signature component. Migrated from `a_reducer_emitting_no_effects_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= contract (list (u8))) (= payload (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (s64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) #record((= requests #list()) (= outcome Outcome.Continue))) (export f)))
  (call f (: #record((= contract #list(1)) (= payload #list(2))) (Record (: contract Bytes) (: payload Bytes))))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a full reducer-step-shaped guest writes every field of the step and runs (via an imposed WIT world)"
  (doc    "SHAPE 27 - the CAPSTONE full reducer-step-shaped guest, the whole result writer end to end. The guest
           returns one request whose contract and token both copy m.contract, payload copies m.payload, and
           deadline-nanos is Some(5), with a Continue outcome. Exercises record permute (step and request are
           declaration-ordered) plus list-of-records plus three byte leaves plus option plus a named variant - the
           reducer-echo step shape. Asserting every field pins the whole step lift. Migrated from
           `a_full_step_shaped_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= contract (list (u8))) (= payload (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (s64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m contract)) (= deadline-nanos (Option.Some 5))))) (= outcome Outcome.Continue))) (export f)))
  (call f (: #record((= contract #list(170 187)) (= payload #list(1 2 3))) (Record (: contract Bytes) (: payload Bytes))))
  (output #record((= requests #list(#record((= contract b"\xaa\xbb") (= payload b"\x01\x02\x03") (= token b"\xaa\xbb") (= deadline-nanos (Some 5))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a nested list<list<s64>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 24 — a NESTED list<list<s64>> host-op ARG (sink.push : (list<list<s64>>) -> unit) driven through an
           imposed WIT world. The reducer on-message performs sink.push (list (list 1 2) (list 3)) (a unit-result,
           observe-only host op) then returns a Continue step with an empty requests list. Running the guest LOADS
           the emitted component and INVOKES sink.push, exercising the RECURSIVE nested-list arg marshal (the outer
           list of (ptr,count) inner lists, each an i64-strided array) e2e - runtime byte-movement coverage a
           validate-only check cannot provide: a broken nested marshal (wrong inner stride, dropped inner list,
           or list<s64>-instead-of-list<list<s64>>) fails to instantiate or run. The observed host-call sequence
           pins that sink.push actually fired. Runtime coverage for v-rust-backend INCREMENT 1 (#3321, recursive
           nested-list host-arg lowering); complements the in-crate validate_all test which cannot check byte-movement.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param vals (list (list (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (List Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#list(1 2) #list(3))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a bytes-param leaf and a spilled record result in one member run through both memory paths (via an imposed WIT world)"
  (doc    "SHAPE 28 — BOTH memory boundaries in ONE member: a list<u8> LEAF param AND a spilled record result,
           the exact combined shape of a reducer's on-message(message) -> step. The wrapper uses both memory
           paths (bytes copy-in for the param leaf + result spill) and all four scratch locals without collision.
           f(m: record{data: Bytes}) = let k = Bytes.len(m.data) in { n: k, twice: 2*k }; f({data:[1..7]}) ==
           { n: 7, twice: 14 } proves the copied-in bytes length + the spilled two-field record result agree.
           Migrated from the in-crate wasmtime test `a_bytes_param_and_record_result_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m (record (= data (list (u8))))) (result (record (= n (s64)) (= twice (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: data Bytes)))) (let ((k (Bytes.len (. m data)))) #record((= n k) (= twice (+ k k))))) (export f)))
  (call f (: #record((= data #list(1 2 3 4 5 6 7))) (Record (: data Bytes))))
  (output #record((= n 7) (= twice 14)))
  (live-objects known-leak))
(case "the identity-less reducer-echo round-trips the real message shape into a step (via an imposed WIT world)"
  (doc    "SHAPE 29 — the flagship reducer-echo (on-message(message) -> step) round-tripping the REAL
           declaration-ordered message{contract, sender:{reducer, host}, payload, token} (a NESTED sender record
           + four list<u8> leaves) into the full step. Exercises the message param permute + the nested-record
           param read + the whole step result writer at once: f echoes contract/payload/token into one request
           with deadline-nanos None and outcome Continue. Migrated from the in-crate wasmtime test
           `the_identity_less_reducer_echo_round_trips` (the SUNSET-CORE echo relation minus identity).")
  (wit-world (world w (export iface (member on-message (func (param m (record (= contract (list (u8))) (= sender (record (= reducer (list (u8))) (= host (list (u8))))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8))))))))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (def (onMessage (: m (Record (: contract Bytes) (: sender (Record (: reducer Bytes) (: host Bytes))) (: payload Bytes) (: token Bytes)))) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue))) (export onMessage)))
  (call on-message (: #record((= contract #list(170 187)) (= sender #record((= reducer #list(1)) (= host #list(2)))) (= payload #list(3 4 5)) (= token #list(9 9))) (Record (: contract Bytes) (: sender (Record (: reducer Bytes) (: host Bytes))) (: payload Bytes) (: token Bytes))))
  (output #record((= requests #list(#record((= contract #list(170 187)) (= payload #list(3 4 5)) (= token #list(9 9)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<record> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 30 — a list<record{contract: bytes, n: s64}> host-op ARG (sink.push): each record element written in place into the outer array at its canonical layout - the s64 field inline + the Bytes field's rope spilled after the array with its (ptr,len) inline, WIT-declaration-ordered. Exercises emit_record_to_mem e2e. Runtime coverage for v-rust-backend increment 2 (#3334, the list<record> host-arg marshal).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (record (= contract (list (u8))) (= n (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (Record (: contract Bytes) (: n Int64))) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#record((= contract (. m contract)) (= n 5)))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-a-list-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 31 — a record host-op ARG whose field is a list<s64> (sink.push(record{ids: list<s64>, n: s64})): the record flattens to core slots, the list field marshalled into mem (backing array) + pushed as (ptr,count). Exercises emit_record_arg_marshal's list-field arm e2e. Runtime coverage for v-rust-backend increment 3 (#3338).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= ids (list (s64))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: ids (List Int64)) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= ids #list(1 2 3)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer branching on a bool host-op result emits a request when true (via an imposed WIT world)"
  (doc "SHAPE 32 - a BOOL host-import RESULT (kv.delete : (Bytes) -> bool) driven through an imposed WIT world. The reducer on-message performs kv.delete(m.token) and branches on the bool: true -> one echo request, false -> no requests. Stubbing kv.delete -> true and asserting the non-empty branch fires (one request) makes the bool result lift load-bearing (a flat scalar disc read). The platform state.delete returns unit (no bool), so this bool host-result lift has no conformance home - it belongs in the typed host-result corpus. Complements the emit+load-only a_host_fused_kv_delete_bool_reducer with the RUNTIME bool-branch coverage.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/kv (member delete (func (param key (list (u8))) (result (bool)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect kv (op delete (-> Bytes Bool))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (kv) (if (kv.delete (. m token)) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond kv.delete (: true Bool)))
  (host-calls (call cadenza:platform/kv.delete))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(2)) (= token #list(3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<tuple> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 33 — a list<tuple<s64, bytes>> host-op ARG (sink.push): each tuple element written in place into the outer array at its canonical positional layout - the s64 element inline + the Bytes element's rope spilled after the array with (ptr,len) inline. Exercises emit_tuple_to_mem e2e. Runtime coverage for v-rust-backend increment 4 (#3343).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (tuple (s64) (list (u8))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (Tuple Int64 Bytes)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#tuple(5 (. m contract)))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))
(case "a typed reducer threading a list<tuple<bytes,bytes>> host-op result branches on its length (via an imposed WIT world)"
  (doc "SHAPE 34 - a list<tuple<Bytes,Bytes>> host-import RESULT (kv.prefix-scan : (Bytes) -> list<tuple<Bytes,Bytes>>) driven through an imposed WIT world. The reducer on-message performs kv.prefix-scan(m.token) and branches on List.len(result) > 0: non-empty -> one echo request, empty -> no requests. Stubbing prefix-scan -> two pairs and asserting the non-empty branch fires (one request) makes the list<tuple> RESULT lift load-bearing: the retptr count read + 16-byte element stride + nested byte-list copy. A broken lift reading the retptr'd list as empty would take the empty branch. Covers the general list<tuple<Bytes,Bytes>> host-result lift v-platform-itest's state iface has no scan-returning-pairs op to exercise. Runtime coverage for the kv.prefix-scan result shape (retires the in-crate run_reducer_bytes_with_scan test).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/kv (member prefix-scan (func (param key (list (u8))) (result (list (tuple (list (u8)) (list (u8))))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect kv (op prefix-scan (-> Bytes (List (Tuple Bytes Bytes))))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (kv) (if (> (List.len (kv.prefix-scan (. m token))) 0) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond kv.prefix-scan (: #list((#list(107) #list(49)) (#list(107) #list(50))) (List (Tuple Bytes Bytes)))))
  (host-calls (call cadenza:platform/kv.prefix-scan))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(2)) (= token #list(3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-an-option-scalar-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 35 — a record host-op ARG with an option<s64> field (sink.push(record{d: option<s64>, n: s64})): the record flattens, the option field to (disc, payload) - Some(42)->(1,42). Exercises emit_record_arg_marshals option-field arm e2e. Runtime coverage for v-rust-backend increment 5 (#3349).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= d (option (s64))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: d (Option Int64)) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= d (Option.Some 42)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-an-option-bytes-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 36 — record{d: option<bytes>, n: s64} host-op ARG; the option field flattens to (disc, ptr, len), Some copies the payload rope. Runtime coverage for v-rust-backend increment 6 (#3354).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= d (option (list (u8)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: d (Option Bytes)) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= d (Option.Some (. m contract))) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

And a direct record-with-bytes-field: same shape but the sink push param is #record((= c #list((u8))) (= n (s64))), guest op (-> (Record (: c Bytes) (: n Int64)) Unit), body (sink.push #record((= c (. m contract)) (= n 7))). Ping if you want me to write the second one out fully. Both are new NON-record-result cases (no sibling bytes param)

(case "a typed reducer performing a record-with-a-direct-bytes-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 37 - a record host-op ARG with a DIRECT Bytes (list<u8>) FIELD beside a scalar (sink.push(record{b: Bytes, n: s64})): the record flattens to core slots, the bytes field's rope marshalled into shared mem with its (ptr,len) inline. Exercises emit_record_arg_marshal's direct-list<u8>-field arm e2e (the sibling-list<u8>-param restriction lifted in #3354). Second half of v-rust-backend INCREMENT 6 (#3354); complements SHAPE 36's option<Bytes> field.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= b (list (u8))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: b Bytes) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= b (. m contract)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<option<s64>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 38 — a list<option<s64>> host-op ARG (sink.push): each option element written in place at its canonical layout (disc byte + payload) - Some(5)->(1,5), None->(0,0). Exercises emit_option_to_mem e2e (both arms). Runtime coverage for v-rust-backend increment 7 (#3358).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (option (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (Option Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list((Option.Some 5) Option.None)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<record-with-option-field> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 39 — a list<record{d: option<s64>, n: s64}> host-op ARG (sink.push): each record element written in place, its option field via emit_option_to_mem - Some(5) and None across two elements. Runtime coverage for v-rust-backend increment 8 (#3360).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (record (= d (option (s64))) (= n (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (Record (: d (Option Int64)) (: n Int64))) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#record((= d (Option.Some 5)) (= n 7)) #record((= d Option.None) (= n 8)))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-a-tuple-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 40 — a record host-op ARG with a tuple<s64, bytes> field (sink.push(record{t: tuple<s64, bytes>, n: s64})): the tuple field flattens inline (s64 slot + bytes element rope-copied as (ptr,len)). Runtime coverage for v-rust-backend increment 9 (#3362).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= t (tuple (s64) (list (u8)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: t (Tuple Int64 Bytes)) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= t #tuple(5 (. m contract))) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a nested-record host arg with an option + bytes leaf emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 41 — composition: record{a: s64, sub: record{d: option<s64>, b: bytes}} host arg — the nested-record arm recurses into the option + bytes field arms. Verifies deep composition of the arg-side marshal (all constituent arms already on main: #3349 option-scalar-field, #3354 bytes/option-bytes; nested-record pre-existing). No new v-rust-backend emit; compositional coverage.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= a (s64)) (= sub (record (= d (option (s64))) (= b (list (u8))))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (Record (: a Int64) (: sub (Record (: d (Option Int64)) (: b Bytes)))) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= a 9) (= sub #record((= d (Option.Some 5)) (= b (. m contract)))))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-a-variant-scalar-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 42 — a record host-op ARG with a variant<scalar> field (sink.push(record{v: variant{a, b(s64), c(s64)}, n: s64})): the variant field flattens (canonical variant flatten) to (disc:i32, payload) — the guest sum-disc IS the component discriminant (decl order, like an enum); a payload case unboxes sum-payload, a nullary case emits the payload-width zero. Pushing V.b(5) -> (disc 1, payload 5). Runtime coverage for v-rust-backend increment 10 (#3368).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= v (variant (a) (b (s64)) (c (s64)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Int64) (C Int64)) (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= v (V.B 5)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<variant<scalar>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 43 — a list<variant{a, b(s64), c(s64)}> host-op ARG (sink.push): each variant element written in place at its canonical variant layout (disc + uniform scalar payload) via emit_variant_to_mem; the guest sum-disc IS the component discriminant (decl order). Pushing (V.B 5), V.A, (V.C 9) exercises a payload case, a nullary case, and a second payload case. Runtime coverage for v-rust-backend increment 11 (PR #3379).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (variant (a) (b (s64)) (c (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Int64) (C Int64)) (effect sink (op push (-> (List V) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list((V.B 5) V.A (V.C 9))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<record-with-a-variant-field> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 44 — variant<scalar> as a record field inside a list element (emit_product_to_mem variant arm). Runtime coverage for v-rust-backend increment 12 (PR #3394).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (record (= v (variant (a) (b (s64)) (c (s64)))) (= n (s64))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Int64) (C Int64)) (effect sink (op push (-> (List (Record (: v V) (: n Int64))) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#record((= v (V.B 5)) (= n 7)) #record((= v V.A) (= n 8)))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a list<tuple-with-a-variant-element> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 45 — variant<scalar> as a tuple element inside a list element (emit_product_to_mem variant arm, positional). Runtime coverage for v-rust-backend increment 12 (PR #3394).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (tuple (variant (a) (b (s64)) (c (s64))) (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Int64) (C Int64)) (effect sink (op push (-> (List (Tuple V Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list(#tuple((V.C 9) 1) #tuple(V.A 2))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a nested-record host arg with a variant<scalar> leaf emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 46 — composition: record{a: s64, sub: record{v: variant{x, y(s64), z(s64)}, n: s64}} host arg; the nested-record arm recurses into the variant field arm. No new v-rust-backend emit (variant field #3368 + nested-record pre-existing); compositional coverage.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= a (s64)) (= sub (record (= v (variant (x) (y (s64)) (z (s64)))) (= n (s64)))))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (X) (Y Int64) (Z Int64)) (effect sink (op push (-> (Record (: a Int64) (: sub (Record (: v V) (: n Int64)))) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= a 9) (= sub #record((= v (V.Y 5)) (= n 7))))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-a-variant-u32-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 47 — variant<u32> record-field host arg: pins the i32-width payload store branch of the variant marshal (prior variant SHAPEs use s64/i64). Runtime coverage for v-rust-backend uniform-scalar variant, non-i64 width.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= v (variant (a) (b (u32)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B UInt32)) (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= v (V.B 4000000000)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a typed reducer performing a record-with-a-variant-f64-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 48 — variant<f64> record-field host arg: pins the f64-width payload store branch of the variant marshal.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= v (variant (a) (b (f64)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Float64)) (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= v (V.B 3.5)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "Int64.of over a u64 host-op RESULT evaluates the host call ONCE (the range-check names the operand)"
  (doc "SHAPE 49 - the runtime checked conversion `Int64.of` over a HOST-LIFTED u64 result. The emit composes `if operand > i64::MAX then trap else wrap(operand)`, which NAMES the operand in the compare AND the else; a host-call operand must be materialized ONCE (a self-keyed let) or its effect FIRES PER USE - breaker adv-tof-host-u64 saw an in-range 1000 spuriously TRAP because the second host invocation drained its lone queued response. The `(host-calls ...)` clause asserts EXACTLY ONE call to hosti.base, so a regression to per-reference re-invocation fails here (host-response exhaustion), not just a wrong value. Runtime coverage for the operand-materialize fix over the merged #3537 .of compose.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64))))) (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (effect hosti (op base (-> Unit UInt64))) (def (f (: m (Record (: x Int64)))) (host (hosti) (Int64.of (hosti.base unit)))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1000 Int64))
  (live-objects known-leak))

(case "Int64.checked-add over a u64 host-op RESULT evaluates the host call ONCE (the overflow formula names the operand)"
  (doc "SHAPE 50 - the runtime checked arithmetic `Int64.checked-add` over a HOST-LIFTED u64 result (narrowed by Int64.of). The overflow-check compose names the operand in the wrapping result AND the two's-complement formula, so a host-call operand is materialized ONCE (else the effect fires per reference). `(host-calls ...)` asserts EXACTLY ONE call to hosti.base. main = match (checked-add (Int64.of (hosti.base)) 1) Some v -> v, None -> -1; with the host stubbed 1000 the sum 1001 fits -> Some 1001. Runtime coverage for the operand-materialize fix over the merged #3569 checked-arith compose.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64))))) (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (effect hosti (op base (-> Unit UInt64))) (def (f (: m (Record (: x Int64)))) (host (hosti) (match (Int64.checked-add (Int64.of (hosti.base unit)) 1) ((Some v) v) ((None _) -1)))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1001 Int64))
  (live-objects known-leak))

;; -- host-u64 checked-conversion end-to-end: intact-compare control, T.of over a host response, handler x host x conversion, record-arg x value-result x conversion (breaker batch 382; the #3537->#3572 wrong-trap arc witnesses) --
(case "u64h1 the u64 host response compared WITHOUT T.of arrives intact"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op base (-> Unit UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (if (= (hosti.base unit) (UInt64.wrap 1000)) 7 8)))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 7 Int64))
  (live-objects known-leak))

(case "u64h2 T.of over the u64 host response (isolated, in-range 1000)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op base (-> Unit UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (Int64.of (hosti.base unit))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1000 Int64))
  (live-objects known-leak))

(case "cr03 an export combining an IN-GUEST handler AND a host import"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect Cnt (op tick (-> Int64)))
    (effect hosti (op base (-> Unit UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (+ (Int64.of (hosti.base unit))
           (handle Cnt (. m x)
             ((tick () s (resume (* s 10) (+ s 1))))
             (+ (Cnt.tick) (Cnt.tick))))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects known-leak))

(case "cq04 host IMPORT: RECORD param + scalar result (reverse direction)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (Int64.of (hosti.put #record((= a (. m x)) (= b 9))))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.put (: 42 UInt64)))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects known-leak))

(case "a record-with-a-MIXED-WIDTH-variant-field host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 51 - a record host-op ARG whose variant field has MIXED-WIDTH scalar payloads (sink.push(record{v: variant{a, b(s64), c(u8)}, n: s64})). The canonical flatten JOIN of a b(s64)/c(u8) variant is the widest register type (i64) - a payload case stores its own value, narrower than the join, at the field's flattened slot; the guest sum-disc IS the component discriminant. Pushing V.b(-5000000000) drives a NEGATIVE s64 payload through the mixed join (a naive i32 join would truncate it). Runtime coverage for v-rust-backend's mixed-width variant marshal (the register-flatten face v-platform-itest's arg-probe value-gate verified byte-correct); this case pins that it EMITS + INSTANTIATES + RUNS (a wrong join type fails wasm-tools validate / instantiation).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param r (record (= v (variant (a) (b (s64)) (c (u8)))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A) (B Int64) (C UInt8)) (effect sink (op push (-> (Record (: v V) (: n Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #record((= v (V.B -5000000000)) (= n 7))) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a list<MIXED-WIDTH-variant<scalar>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 52 - a list<variant{a(u8), b(u16), c}> host-op ARG (sink.push): each element written in place at the canonical variant layout, whose payload area is the MAX-NATURAL width of the mixed u8/u16 cases (the memory face of the flatten join, distinct from SHAPE 51's register face). Pushing (V.A 200), (V.B 60000), V.C exercises a u8 payload, a u16 payload, and a nullary case - the u8 reads from the right bits and the u16 stays intact per element. Runtime coverage for the mixed-width variant marshal's memory path (v-platform-itest's arg-probe value-gate verified the list items byte-correct); pins EMIT + INSTANTIATE + RUN.")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/sink (member push (func (param items (list (variant (a (u8)) (b (u16)) (c)))) (result (unit)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type V (A UInt8) (B UInt16) (C)) (effect sink (op push (-> (List V) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push #list((V.A 200) (V.B 60000) V.C)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output #record((= requests #list()) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a BARE mixed-width variant as the direct host-op arg emits, loads, and runs (via an imposed WIT world)"
  (doc "SHAPE 53 - a scalar-payload variant{tiny(u8), big(s64), mark} passed BARE as the TOP-LEVEL host-op param (hosti.put(v: variant), NOT nested in a record/list). The param crosses as a component `variant` DEFINED type; the guest decomposes the value-heap variant handle into the canonical `(disc, payload)` register-flatten (join = i64 for the mixed u8/s64 cases) via emit_variant_reg_flatten - the SAME helper a record-field/list-element variant uses, now at the param position (HostParam::Variant). Three calls exercise a u8 payload case, an s64 payload case, and the nullary `mark` (payload-width zero). Runtime coverage for v-rust-backend's bare-variant host-arg param (breaker mwv1); a wrong join/flatten fails wasm-tools validate / instantiation.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64))))) (import cadenza:demo/hosti (member put (func (param v (variant (tiny (u8)) (big (s64)) (mark))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type V (Tiny UInt8) (Big Int64) (Mark)) (effect hosti (op put (-> V Unit))) (def (f (: m (Record (: x Int64)))) (host (hosti) (do (hosti.put (V.Tiny (UInt8.wrap 7))) (hosti.put (V.Big 900000000000)) (hosti.put V.Mark) (. m x)))) (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put) (call cadenza:demo/hosti.put) (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects known-leak))

;; -- bare + record-wrapped MIXED-WIDTH variant host args: u8/s64/nullary arms each dispatched (breaker batch 385; the #3579->#3588 HostParam::Variant arc) --
(case "mwv1 a MIXED-WIDTH scalar-payload variant host ARG delivers each arm's payload"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member put (func (param v (variant (tiny (u8)) (big (s64)) (mark))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type V (Tiny UInt8) (Big Int64) (Mark))
    (effect hosti (op put (-> V Unit)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (do (hosti.put (V.Tiny (UInt8.wrap 7)))
            (hosti.put (V.Big 900000000000))
            (hosti.put V.Mark)
            (. m x))))
    (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put) (call cadenza:demo/hosti.put) (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects known-leak))
(case "mwv2 the SAME mixed-width variant wrapped in a RECORD host arg"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member put (func (param r (record (= v (variant (tiny (u8)) (big (s64)) (mark))) (= n (s64)))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type V (Tiny UInt8) (Big Int64) (Mark))
    (effect hosti (op put (-> (Record (: v V) (: n Int64)) Unit)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (do (hosti.put #record((= v (V.Big 900000000000)) (= n 1)))
            (. m x))))
    (export f)))
  (call f (: #record((= x 42)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects known-leak))

(case "a variant-WITH-PAYLOAD host RESULT is lifted into a guest Sum and matched (via an imposed WIT world)"
  (doc "SHAPE 54 - a host op returning a scalar-payload variant{a(u8), b(s64), mark} (hosti.get). The result is SPILLED (flattens to disc+payload > 1 core value → retptr); the guest LIFTS the (disc, payload) from the retptr'd region into a value-heap Sum via emit_variant_sum_lift - the N-case generalization of the option-result lift, the RESULT-side twin of the bare-variant ARG marshal. Stubbing get -> (b 900000000000) and matching selects the B arm (k). Runtime coverage for v-rust-backend's variant-payload host-result lift (breaker w10c); a wrong disc/payload-offset read would mis-select the arm. (The cdz-run driver's coerce_one gained a Type::Variant arm to encode the variant response.)")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64))))) (import cadenza:demo/hosti (member get (func (result (variant (a (u8)) (b (s64)) (mark))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type V (A UInt8) (B Int64) (Mark)) (effect hosti (op get (-> Unit V))) (def (f (: m (Record (: x Int64)))) (host (hosti) (match (hosti.get unit) ((A n) (Int64.of n)) ((B k) k) ((Mark) -1)))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (b 900000000000) V)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 900000000000 Int64))
  (live-objects known-leak))

;; -- variant-with-payload host RESULTS: payload arm, mixed-width per-arm across three dispatches, negative-s64 join (breaker batch 387; the pre-delivered #3592 acceptance ladder) --
(case "vres1 a variant-with-payload host RESULT delivers the payload arm (w10c shape)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member pick (func (result (variant (small (s64)) (big))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type Pick (Small Int64) (Big))
    (effect hosti (op pick (-> Unit Pick)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (match (hosti.pick unit) ((Pick.Small k) k) ((Pick.Big) 999))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.pick (: (small 5) pick)))
  (host-calls (call cadenza:demo/hosti.pick))
  (output (: 5 Int64))
  (live-objects known-leak))

(case "vres2 a MIXED-WIDTH variant host RESULT delivers each arm across three dispatches"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member next (func (result (variant (tiny (u8)) (big (s64)) (mark))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type V (Tiny UInt8) (Big Int64) (Mark))
    (effect hosti (op next (-> Unit V)))
    (def (rd (: v V))
      (match v ((V.Tiny t) (Int64.of t)) ((V.Big b) b) ((V.Mark) -1)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (+ (rd (hosti.next unit)) (+ (rd (hosti.next unit)) (rd (hosti.next unit))))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.next (: (tiny 7) v)) (respond hosti.next (: (big 900000000000) v)) (respond hosti.next (: (mark unit) v)))
  (host-calls (call cadenza:demo/hosti.next) (call cadenza:demo/hosti.next) (call cadenza:demo/hosti.next))
  (output (: 900000000006 Int64))
  (live-objects known-leak))

(case "vres3 a NEGATIVE s64 payload through the variant host-result join"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member get (func (result (variant (val (s64)) (none))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type R (Val Int64) (NoneArm))
    (effect hosti (op get (-> Unit R)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (match (hosti.get unit) ((R.Val v) v) ((R.NoneArm) 0))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (val -5000000000) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: -5000000000 Int64))
  (live-objects known-leak))

(case "a variant-with-a-COMPOUND-PAYLOAD host RESULT is lifted (bytes payload case) (via an imposed WIT world)"
  (doc "SHAPE 55 - a host op returning a variant one of whose cases carries a NON-scalar (compound) payload: variant{raw(list<u8>), empty}. The result spills; the guest lifts the disc + the selected case's payload from the retptr'd region via emit_variant_sum_lift, which RECURSES emit_result_lift for a compound payload (a list<u8> = copy-out of the bytes) rather than the scalar leaf-box. Stubbing get -> (raw (list 1 2 3 4 5)) selects the Raw arm and reads Bytes.len = 5; the (empty) arm returns -1. Generalizes the scalar variant-result lift (SHAPE 54) to a liftable-compound payload (list/bytes/tuple/record via the shared recursion) - the variant_liftable_payload_cases admission, RESULT-side only (the ARG marshal stays scalar-only). Consumer-relevant: the deliver-response dispatch returns variant-payload results.")
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64))))) (import cadenza:demo/hosti (member get (func (result (variant (raw (list (u8))) (empty))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type V (Raw Bytes) (VEmpty)) (effect hosti (op get (-> Unit V))) (def (f (: m (Record (: x Int64)))) (host (hosti) (match (hosti.get unit) ((Raw b) (Int64.of (Bytes.len b))) ((VEmpty) -1)))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (raw #list(1 2 3 4 5)) V)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 5 Int64))
  (live-objects known-leak))

;; -- variant host RESULTS with COMPOUND payloads: list payload measured, record payload projected (breaker batch 397a; the #3655 flip) --
(case "cvp1 a variant host RESULT with a LIST payload lifts and is measured"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member get (func (result (variant (items (list (s64))) (none))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type R (Items (List Int64)) (NoneArm))
    (effect hosti (op get (-> Unit R)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (match (hosti.get unit) ((R.Items xs) (List.len xs)) ((R.NoneArm) -1))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (items #list(5 6 7)) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 3 Int64))
  (live-objects known-leak))

(case "cvp2 a variant host RESULT with a RECORD payload lifts and projects"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member get (func (result (variant (tag (record (= a (s64)) (= b (s64)))) (none))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type R (Tag (Record (: a Int64) (: b Int64))) (NoneArm))
    (effect hosti (op get (-> Unit R)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (match (hosti.get unit) ((R.Tag t) (+ (. t a) (. t b))) ((R.NoneArm) -1))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.get (: (tag #record((= a 40) (= b 2))) r)))
  (host-calls (call cadenza:demo/hosti.get))
  (output (: 42 Int64))
  (live-objects known-leak))

; -- breaker batch 404 (2026-08-26): in-guest-handler x host-import COMBINATION faces (cr01-cr03d:
; two exports in one interface, exported body running a handled effect, handler+host-call in
; let-sequenced / nested / body-inside shapes) and the nullary-import + record-host-arg s64-result
; faces (cq04c). Imposed-world: wasm pass, rust todo (import-side emit pending).

(case "cr01 TWO exported members in one interface — both callable"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))) (member g (func (param m (record (= x (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: x Int64)))) (* (. m x) 2))
    (def (g (: m (Record (: x Int64)))) (+ (. m x) 100))
    (export f)
    (export g)))
  (call g (: #record((= x 5)) (Record (: x Int64))))
  (output (: 105 Int64))
  (live-objects known-leak))

(case "cr02 an export whose body runs an IN-GUEST handled effect"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect Cnt (op tick (-> Int64)))
    (def (f (: m (Record (: x Int64))))
      (handle Cnt (. m x)
        ((tick () s (resume (* s 10) (+ s 1))))
        (+ (Cnt.tick) (Cnt.tick))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (output (: 70 Int64))
  (live-objects known-leak))

(case "cr03b let-sequenced: handle FIRST, then host call (same combination, flat nesting)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect Cnt (op tick (-> Int64)))
    (effect hosti (op base (-> Unit UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (let ((k (handle Cnt (. m x)
                   ((tick () s (resume (* s 10) (+ s 1))))
                   (+ (Cnt.tick) (Cnt.tick)))))
          (+ (Int64.of (hosti.base unit)) k))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects known-leak))

(case "cr03c host call INSIDE the handled body"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (u64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect Cnt (op tick (-> Int64)))
    (effect hosti (op base (-> Unit UInt64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (handle Cnt (. m x)
          ((tick () s (resume (* s 10) (+ s 1))))
          (+ (Cnt.tick) (Int64.of (hosti.base unit))))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 UInt64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1030 Int64))
  (live-objects known-leak))

(case "cr03d combination with s64 host result (no Int64.of) — nested"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member base (func (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect Cnt (op tick (-> Int64)))
    (effect hosti (op base (-> Unit Int64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti)
        (+ (hosti.base unit)
           (handle Cnt (. m x)
             ((tick () s (resume (* s 10) (+ s 1))))
             (+ (Cnt.tick) (Cnt.tick))))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.base (: 1000 Int64)))
  (host-calls (call cadenza:demo/hosti.base))
  (output (: 1070 Int64))
  (live-objects known-leak))

(case "cq04c record host-ARG with s64 result (no Int64.of)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) Int64)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (hosti.put #record((= a (. m x)) (= b 9)))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.put (: 42 Int64)))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 42 Int64))
  (live-objects known-leak))

; -- breaker batch 405 (2026-08-26): host-IMPORT result-shape coverage + record-param export
; controls. cq01/cq03 scalar-param imports with record/list results; cq02b/c/d NULLARY imports with
; record/list results (once, once-list, twice-with-two-responds); cq04b record arg + unit result;
; wen1 the FIRST enum host-import RESULT pin. cord4/cord5 pin the RECORD-param export twins (2- and
; 20-field record results) that pass at every size — the isolating controls for the one remaining
; export gap: a bare SCALAR-param export with a compound result renders a raw pointer (cor02..co02,
; cord2/cord3 all fail identically at 2..20 fields; routed to v-rust-backend with this ladder).

(case "cq01 host IMPORT: scalar param + RECORD result — guest reads a field"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member info (func (param k (s64)) (result (record (= alpha (s64)) (= beta (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op info (-> Int64 (Record (: alpha Int64) (: beta Int64)))))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (. (hosti.info (. m x)) beta)))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.info (: #record((= alpha 7) (= beta 42)) (Record (: alpha Int64) (: beta Int64)))))
  (host-calls (call cadenza:demo/hosti.info))
  (output (: 42 Int64))
  (live-objects known-leak))

(case "cq03 host IMPORT: scalar param + LIST result — guest measures it"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member fetch (func (param k (s64)) (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op fetch (-> Int64 (List Int64))))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (List.len (hosti.fetch (. m x)))))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-responses (respond hosti.fetch (: #list(5 6 7) (List Int64))))
  (host-calls (call cadenza:demo/hosti.fetch))
  (output (: 3 Int64))
  (live-objects known-leak))

(case "cq02b host IMPORT: NULLARY + RECORD result, called ONCE"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member peek (func (result (record (= a (s64)) (= b (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op peek (-> Unit (Record (: a Int64) (: b Int64)))))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (. (hosti.peek unit) b)))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.peek (: #record((= a 10) (= b 32)) (Record (: a Int64) (: b Int64)))))
  (host-calls (call cadenza:demo/hosti.peek))
  (output (: 32 Int64))
  (live-objects known-leak))

(case "cq02c host IMPORT: NULLARY + LIST result, called once"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member all (func (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op all (-> Unit (List Int64))))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (List.len (hosti.all unit))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.all (: #list(4 5) (List Int64))))
  (host-calls (call cadenza:demo/hosti.all))
  (output (: 2 Int64))
  (live-objects known-leak))

(case "cq04b host IMPORT: RECORD arg + UNIT result (SHAPE-13 control in my namespace)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member put (func (param v (record (= a (s64)) (= b (s64)))) (result (unit)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op put (-> (Record (: a Int64) (: b Int64)) Unit)))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (do (hosti.put #record((= a (. m x)) (= b 9))) 7)))
    (export f)))
  (call f (: #record((= x 3)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.put))
  (output (: 7 Int64))
  (live-objects known-leak))

(case "cq02d NULLARY + RECORD result called TWICE with TWO respond clauses"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member peek (func (result (record (= a (s64)) (= b (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (effect hosti (op peek (-> Unit (Record (: a Int64) (: b Int64)))))
    (def (f (: m (Record (: x Int64))))
      (host (hosti) (+ (. (hosti.peek unit) a) (. (hosti.peek unit) b))))
    (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.peek (: #record((= a 10) (= b 0)) (Record (: a Int64) (: b Int64)))) (respond hosti.peek (: #record((= a 0) (= b 32)) (Record (: a Int64) (: b Int64)))))
  (host-calls (call cadenza:demo/hosti.peek) (call cadenza:demo/hosti.peek))
  (output (: 42 Int64))
  (live-objects known-leak))

(case "cord4 IMPOSED world: RECORD param + 2-field scalar record result"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: x Int64)))) #record((= b1 (. m x)) (= b2 2)))
    (export f)))
  (call f (: #record((= x 1)) (Record (: x Int64))))
  (output #record((= b1 1) (= b2 2)))
  (live-objects known-leak))

(case "cord5 IMPOSED world: RECORD param + 20-field record result (the co02 shape, record param)"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (record (= b1 (s64)) (= b2 (s64)) (= b3 (s64)) (= b4 (s64)) (= b5 (s64)) (= b6 (s64)) (= b7 (s64)) (= b8 (s64)) (= b9 (s64)) (= b10 (s64)) (= b11 (s64)) (= b12 (s64)) (= b13 (s64)) (= b14 (s64)) (= b15 (s64)) (= b16 (s64)) (= b17 (s64)) (= b18 (s64)) (= b19 (s64)) (= b20 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: x Int64))))
      #record((= b1 (. m x)) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 20)))
    (export f)))
  (call f (: #record((= x 9)) (Record (: x Int64))))
  (output #record((= b1 9) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 20)))
  (live-objects known-leak))

(case "wen1 an enum host-import RESULT lifts and selects the guest arm"
  (wit-world (world w (export iface (member f (func (param m (record (= x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member mode (func (result (enum fast slow)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Mode (Fast) (Slow))
             (effect hosti (op mode (-> Unit Mode)))
             (def (f (: m (Record (: x Int64))))
               (host (hosti) (match (hosti.mode unit) ((Mode.Fast) 1) ((Mode.Slow) 2))))
             (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.mode (: (fast unit) mode)))
  (host-calls (call cadenza:demo/hosti.mode))
  (output (: 1 Int64))
  (live-objects known-leak))

(case "a SCALAR-param export returning a RECORD lifts the result (not a raw handle) (via an imposed WIT world)"
  (doc "SHAPE 56 - a scalar-param export with a COMPOUND (record) RESULT: f(x: s64) -> record{b1,b2,b3: s64}. The result SPILLS (3 flat > the 1-result cap) → the canonical ABI returns it via a caller-provided retptr, which the guest must WRITE. The record-PARAM route already did this (record_interface_export's SpillRecord result-lower); the SCALAR-param route was gated out by `any_record` and fell through to the provider path, which handed back the value-heap u32 HANDLE (a leaked pointer, not the value — breaker's cor02/co02 rendered ~1114400). Fix: admit the typed-interface wrapper when a member has a spilled compound result too (needs_result_wrapper), not only a record param. Pins that a scalar-param compound result LIFTS to the record value on all backends' wasm path.")
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64)) (= b3 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #record((= b1 x) (= b2 (* x 2)) (= b3 (+ x 100)))) (export f)))
  (call f (: 7 Int64))
  (output #record((= b1 7) (= b2 14) (= b3 107)))
  (live-objects known-leak))

(case "a typed reducer threading a string host-op result branches on its byte-len (via an imposed WIT world)"
  (doc "SHAPE 57 - a STRING host-import RESULT (kv.lookup : (Bytes) -> string) driven through an imposed WIT world - the result-side twin of the string ARG (which already crosses on every path). A `string` result crosses on the WORLD-DRIVEN boundary as the SAME (ptr,len) spill a `list<u8>` (Bytes) result rides: the guest lift (emit_result_lift's `Ty::Bytes | Ty::String` arm) copies the host's bytes into a value-heap byte-rope handle, and the WIT type is `string` (ty_natural_wit). Before #(this) the result gate (result_is_liftable) admitted `list<u8>` but NOT `string`, so a bare string host-result DECLINED at compile despite the lift + WIT machinery being shape-identical - a one-shape hole in the otherwise-general world-import result surface. The reducer on-message performs kv.lookup(m.token) and branches on String.byte-len(result) > 0: non-empty -> one echo request, empty -> no requests. Stubbing lookup -> \"hi\" (byte-len 2) and asserting the non-empty branch fires makes the string result lift load-bearing (a broken lift reading len 0 would take the empty branch). Closes the host-string-RESULT wasm-emit gap (operator-blocking for run_agent + io.fetch).")
  (wit-world (world w (export guest (member on-message (func (param m (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))))) (result (record (= requests (list (record (= contract (list (u8))) (= payload (list (u8))) (= token (list (u8))) (= deadline-nanos (option (u64)))))) (= outcome (variant (continue) (close (record (= schema (list (u8))) (= reason (list (u8)))))))))))) (import cadenza:platform/kv (member lookup (func (param key (list (u8))) (result (string)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect kv (op lookup (-> Bytes String))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (kv) (if (> (String.byte-len (kv.lookup (. m token))) 0) #record((= requests #list(#record((= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)) #record((= requests #list()) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: #record((= contract #list(1)) (= payload #list(2)) (= token #list(3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond kv.lookup (: "hi" String)))
  (host-calls (call cadenza:platform/kv.lookup))
  (output #record((= requests #list(#record((= contract #list(1)) (= payload #list(2)) (= token #list(3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))))
  (live-objects known-leak))

(case "a payloadless enum result VALUE round-trips via the run/encode envelope (no wit-world clause; typed enum export is a separate gap)"
  (doc "SHAPE 58 - a payloadless enum (Color = Red|Green|Blue) returned from a scalar-param export, no wit-world clause. CORRECTION (verified by WIT-dump, not just gate PASS): this does NOT emit a typed WIT `enum` export - the compiler CANNOT emit a typed enum export today (ty_natural_wit(Ty::Sum)->None in the export lift), so it FALLS BACK to the generic cadenza:run/run resource envelope (make/run/encode) and the enum value crosses as ENCODED BYTES. So this SHAPE pins only the VALUE ROUND-TRIP of an enum through the guest + encode envelope - a broken enum lower/encode renders a wrong case. It does NOT verify typed enum self-declaration; that is a DECLINED emit gap (a typed enum EXPORT), tracked in WIT-BOUNDARY-SHAPE-COVERAGE.md. Promoted from a v-rust-backend probe (kept as an honest round-trip pin).")
  (input (do (type Color (Red) (Green) (Blue)) (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green)) (export f)))
  (call f (: 0 Int64))
  (output (: (Red unit) Color)))

(case "a payloadless enum in a record result VALUE round-trips via the run/encode envelope (no wit-world clause; typed export is a separate gap)"
  (doc "SHAPE 59 - the record-wrapped twin of SHAPE 58: a payloadless enum as a record-result FIELD (record{c: Color}), no wit-world clause. Like SHAPE 58 this does NOT emit a typed WIT record/enum export - it falls back to the generic run/encode envelope (verified by WIT-dump), so it pins the enum-in-record VALUE ROUND-TRIP, not typed self-declaration. Complements SHAPE 2 (variant-WITH-payload) with the NULLARY-enum face. The typed enum EXPORT (and typed record-with-enum-field export) is a DECLINED emit gap tracked in WIT-BOUNDARY-SHAPE-COVERAGE.md. Promoted from a v-rust-backend probe (honest round-trip pin).")
  (input (do (type Color (Red) (Green) (Blue)) (def (f (: m (Record (: x Int64)))) #record((= c (if (= (. m x) 0) Color.Red Color.Green)))) (export f)))
  (call f (: #record((= x 0)) (Record (: x Int64))))
  (output (: #record((= c (Red unit))) (record (c Color))))
  (live-objects known-leak))

(case "a payloadless enum EXPORT result crosses as a TYPED WIT enum (imposed world) — Direction A"
  (doc "SHAPE 60 - a payloadless enum (Color = Red|Green|Blue) as a typed EXPORT result under an imposed world declaring `(result (\"enum\" red green blue))`. Before this, emit crossed the enum as a bare `u32` handle via the provider path (the declared WitType::Enum bypassed) - verified by WIT-dump. Fix (record_result_lower payloadless-enum arm → Passthrough i32 + needs_result_wrapper): the def already returns the raw i32 disc (= flatten(Enum)), so it passes straight through as the declared enum; the enum DEFINED type is emitted + re-exported by the typed-interface `note` pass. WIT-dump now shows `enum t0 { red, green, blue }` + `f: func(x: s64) -> t0` (NOT u32). Guard: guest decl-order case names must equal the WIT case order (else a runtime disc remap - declines). This closes the typed enum EXPORT (Direction A) in-algebra gap per the operator full-WIT-algebra ruling.")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (enum red green blue)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Color (Red) (Green) (Blue)) (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green)) (export f)))
  (call f (: 0 Int64))
  (output (: (red unit) Color))
  (call f (: 5 Int64))
  (output (: (green unit) Color)))

(case "a variant-with-payload EXPORT result crosses as a typed WIT variant (declared world)"
  (doc "SHAPE 61 - the payloaded-VARIANT twin of SHAPE 60: a bare `variant { continue, close(s64) }` EXPORT result under a declared world. Already WIRED (no emit change) via record_result_lower's SpillRecord path + canon_write_of's variant arm - this SHAPE VERIFIES the previously-untested cell (WIT-dump confirms `variant t0 { continue, close(s64) }` + `f: func(x: s64) -> t0`, NOT a bare u32/run-encode). Both arms exercised: x=0 -> Continue (nullary, disc 0), x!=0 -> Close(x) (s64 payload, disc 1). A broken variant lower (wrong disc, missing payload) renders a different arm. Complements SHAPE 2 (variant in a RECORD result) with the BARE (top-level) variant result.")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (variant (continue) (close (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close Int64)) (def (f (: x Int64)) (if (= x 0) Outcome.Continue (Outcome.Close x))) (export f)))
  ; PAYLOAD arm FIRST: the harness balance-checks the FIRST call only, and the payload (Close) arm is the
  ; one that leaks the SpillRecord result cell (known-leak 1, same class as SHAPE 60/62/63); ordering it
  ; first makes that leak the CHECKED one (a nullary-first order hid it — breaker WIT-dump audit).
  (call f (: 7 Int64))
  (output (: (close 7) Outcome))
  (call f (: 0 Int64))
  (output (: (continue unit) Outcome))
  (live-objects known-leak))

(case "a bare TUPLE export result crosses as a typed WIT tuple (declared world)"
  (doc "SHAPE 62 - a bare `tuple<s64, s64>` EXPORT result under a declared world. Before this, canon_write_of had NO Ty::Tuple arm, so a tuple result declined the typed path and degraded to a bare u32 via the provider path (verified by WIT-dump). Fix: canon_write_of gained a Ty::Tuple arm (the POSITIONAL twin of the Record arm - element i at cell slot i, written at the WIT tuple's canonical offset; reuses CanonWrite::Record, no new writer). WIT-dump now shows `f: func(x: s64) -> tuple<s64, s64>`. Element writes recurse, so a nested tuple/record/bytes element composes.")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (tuple (s64) (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #tuple(x (* x 2))) (export f)))
  (call f (: 5 Int64))
  (output #tuple(5 10))
  (live-objects known-leak))

(case "a variant with a TUPLE payload crosses as a typed WIT variant (declared world)"
  (doc "SHAPE 63 - a variant whose payloaded case carries a TUPLE (`two(tuple<s64,s64>)`), under a declared world. Exercises canon_write_of's variant arm recursing into the new Ty::Tuple arm for the payload. Before the Tuple arm this degraded to a bare u32. WIT-dump now shows `variant t0 { one(s64), two(tuple<s64, s64>) }` + `f: func(x: s64) -> t0`. Both arms: x=0 -> One(x) (scalar payload), x!=0 -> Two(tuple x x) (tuple payload). The compound-payload twin of SHAPE 61 (scalar payload).")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (variant (one (s64)) (two (tuple (s64) (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Pair (One Int64) (Two (Tuple Int64 Int64))) (def (f (: x Int64)) (if (= x 0) (Pair.One x) (Pair.Two #tuple(x x)))) (export f)))
  (call f (: 0 Int64))
  (output (: (one 0) Pair))
  (call f (: 4 Int64))
  (output (two #tuple(4 4)))
  (live-objects known-leak))

(case "a declared-world enum EXPORT whose guest case order MISMATCHES the WIT declines (not a silent u32 degrade)"
  (doc "SHAPE 64 - breaker FINDING 1 regression pin. Guest `(type Color (Red)(Green)(Blue))` under an imposed world declaring `(result (\"enum\" green red blue))` [case order REVERSED]. Before, the SHAPE-60 order-mismatch guard returned None → silently fell through to the PROVIDER path and exported `f: func(s64) -> u32` — a DIFFERENT type than the world declares (the value even round-tripped, masking it). Now the imposed-world contract guard in the export dispatch DECLINES loudly: an explicit wit_world's typed member that can't be emitted must NOT silently cross as a u32 handle. A reorder needs a runtime disc-remap (a later increment), so it declines rather than mis-emitting. A component-name-ONLY peer provider (no imposed wit_world) is unaffected — it still crosses compounds as handles (29-* peer cases).")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (enum green red blue)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Color (Red) (Green) (Blue)) (def (f (: x Int64)) (if (= x 0) Color.Red Color.Green)) (export f)))
  (declines (message "a different type than the world declares")))

(case "a typed RECORD result with a VARIANT field crosses the export boundary (declared world)"
  (doc "SHAPE 65 - a typed export result `record { o: variant{continue, close(s64)}, n: s64 }` under a declared world. Verifies canon_write_of's Record arm recursing into its Variant arm for a compound field (the record + variant defined types both emitted + re-exported). WIT-dump: `variant t0 {continue, close(s64)}` + `record t1 {o: t0, n: s64}` + `f: func(x: s64) -> t1`. Both variant arms x=0->Continue / x!=0->Close(x). Previously WIRED-but-untested (the doc's record-result-with-variant-field cell); now pinned.")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (record (= o (variant (continue) (close (s64)))) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close Int64)) (def (f (: x Int64)) #record((= o (if (= x 0) Outcome.Continue (Outcome.Close x))) (= n x))) (export f)))
  (call f (: 0 Int64)) (output #record((= o (continue unit)) (= n 0)))
  (call f (: 7 Int64)) (output #record((= o (close 7)) (= n 7)))
  (live-objects known-leak))

(case "a typed record result with an option<COMPOUND> field crosses the export boundary (declared world)"
  (doc "SHAPE 66 - a typed export result `record { d: option<record{a}>, n: s64 }` under a declared world - the option<COMPOUND> RESULT face (the doc's untested option<compound-leaf> result cell; only option<scalar>/option<bytes> had SHAPEs). canon_write_of's option arm recurses its payload into the Record arm; both defined types emit + re-export. WIT-dump: `record t0 {a}` + `record t1 {d: option<t0>, n}` + `f: func(s64)->t1`. Both arms x=0->None / x!=0->Some(record{a=x}). NOTE: this is the RESULT side; an option<compound> host-op ARG field / list element is a separate marshal-side gap.")
  (wit-world (world w (export cadenza:demo/iface (member f (func (param x (s64)) (result (record (= d (option (record (= a (s64))))) (= n (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Int64)) #record((= d (if (= x 0) Option.None (Option.Some #record((= a x))))) (= n x))) (export f)))
  (call f (: 0 Int64)) (output #record((= d (None unit)) (= n 0)))
  (call f (: 5 Int64)) (output #record((= d (Some #record((= a 5)))) (= n 5)))
  (live-objects known-leak))

; -- breaker batch 408 (2026-08-26): the scalar-param + compound-result acceptance ladder, promoted
; on the #3721 fix (gate admission: a scalar-param member with a SpillRecord compound result now takes
; the typed-interface wrapper instead of leaking the raw handle). All 8 faces flipped on the fix:
; record 2-field (minimal, no spill) / 20-field (spill-sized), option-in-record Some+None, bare
; option, list, TWO scalar params, variant-with-payload. cord1 pins the SYNTHESIZED-world (no
; wit-world clause) 2-field record result twin, which passes BOTH targets.

(case "sp1 SCALAR param + 2-field record result lifts (minimal face, no spill)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64)) #record((= b1 x) (= b2 2)))
    (export f)))
  (call f (: 1 Int64))
  (output #record((= b1 1) (= b2 2)))
  (live-objects known-leak))

(case "sp2 SCALAR param + 20-field record result lifts (spill-sized, same fix)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (record (= b1 (s64)) (= b2 (s64)) (= b3 (s64)) (= b4 (s64)) (= b5 (s64)) (= b6 (s64)) (= b7 (s64)) (= b8 (s64)) (= b9 (s64)) (= b10 (s64)) (= b11 (s64)) (= b12 (s64)) (= b13 (s64)) (= b14 (s64)) (= b15 (s64)) (= b16 (s64)) (= b17 (s64)) (= b18 (s64)) (= b19 (s64)) (= b20 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64))
      #record((= b1 x) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 (* x 2))))
    (export f)))
  (call f (: 9 Int64))
  (output #record((= b1 9) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 18)))
  (live-objects known-leak))

(case "sp3 SCALAR param + record result with an Option field (Some side)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64)) #record((= a 9) (= d (Option.Some x))))
    (export f)))
  (call f (: 5 Int64))
  (output #record((= a 9) (= d (Some 5))))
  (live-objects known-leak))

(case "sp3n SCALAR param + record result with an Option field (None side, branch-selected)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (record (= a (s64)) (= d (option (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64)) #record((= a x) (= d (if (> x 0) (Option.Some x) Option.None))))
    (export f)))
  (call f (: 0 Int64))
  (output #record((= a 0) (= d (None unit))))
  (live-objects known-leak))

(case "sp4 SCALAR param + bare option result"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (option (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64)) (Option.Some (* x 3)))
    (export f)))
  (call f (: 4 Int64))
  (output (: (Some 12) (Option Int64)))
  (live-objects known-leak))

(case "sp5 SCALAR param + list result"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (list (s64))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64)) #list(x (* x 2) (* x 3)))
    (export f)))
  (call f (: 2 Int64))
  (output #list(2 4 6))
  (live-objects known-leak))

(case "sp6 TWO scalar params + 2-field record result (multi-scalar face)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (param y (s64)) (result (record (= b1 (s64)) (= b2 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64) (: y Int64)) #record((= b1 (+ x y)) (= b2 (* x y))))
    (export f)))
  (call f (: 3 Int64) (: 4 Int64))
  (output #record((= b1 7) (= b2 12)))
  (live-objects known-leak))

(case "sp7 SCALAR param + variant-with-payload result (sum face of the same gate)"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result (variant (small (s64)) (big))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type Pick (Small Int64) (Big))
    (def (f (: x Int64)) (if (< x 100) (Pick.Small x) Pick.Big))
    (export f)))
  (call f (: 5 Int64))
  (output (: (small 5) pick))
  (live-objects known-leak))

(case "cord1 SYNTHESIZED world: 2-field s64 record result (no wit-world clause) — the fully-constant record now hoists build-once (WIT static encoding), so it is a census-excluded immortal, NOT a per-call mortal leak"
  (input (do
    (def (f (: x Int64)) #record((= b1 1) (= b2 2)))
    (export f)))
  (call f (: 1 Int64))
  (output (: (record (= b1 1) (= b2 2)) (Record (: b1 Int64) (: b2 Int64))))
  (live-objects 0))

; ── Single-variant newtype escape (adv-63b/adv-64, migrated from rcdzc): a scalar-erased newtype returned
; from a PARAM'D export must emit a VALID module (not the recursive-sum-resource path) and render its NOMINAL
; name; a compound-inner newtype stays on the heap escape; matched/wrapped-then-matched controls round-trip.

(case "a scalar-erased single-variant newtype escaping a param'd export emits a valid module and renders the nominal name"
  (doc    "adv-63b/adv-64. A single-variant newtype over a SCALAR, `(type W (Mk Int64))`, returned from a
           PARAMETERIZED export erases to a bare core int; the boundary escape router must NOT take the
           nominal recursive-sum-resource path (which expects a heap handle) for a raw i64 — that emitted an
           INVALID module ('expected i32, found i64'). The scalar-erased newtype falls through to the scalar
           value-form branch (scalar_box boxes a bare Int result), so it escapes+renders as the NOMINAL
           `(: 5 W)` — a running case here implicitly proves the module is VALID (an invalid module could not
           run), and the output pins the nominal rendering (adv-64: NOT the erased `(: 5 Int64)`). The
           nullary path always rendered the nominal; this pins the param'd path agrees.")
  (input  (do (type W (Mk Int64)) (def (main (: k Int64)) (Mk k)) (export main)))
  (call   main (: 5 Int64)) (output (: 5 W)))

(case "a GENERIC scalar newtype instantiated at Int64 escapes a param'd export as a valid module"
  (doc    "The generic face of the scalar-newtype escape: `(type Box (Mk a))` instantiated at Int64 takes the
           same erased-scalar escape path and emits a valid module, rendering the nominal `(: 5 Box)`. Pins
           that the escape router's scalar fall-through is not confused by the type parameter.")
  (input  (do (type Box (Mk a)) (def (main (: k Int64)) (Mk k)) (export main)))
  (call   main (: 5 Int64)) (output (: 5 Box)))

(case "a NARROW-inner scalar newtype escaping a param'd export emits a valid module (i32-slot box)"
  (doc    "The width-edge face: `(type U8 (Mk UInt8))` has a sub-i32 (i32-slot) inner, so the scalar box's
           i32->i64 extend must fire for the <=32 slot (and must NOT for a mid/full width) — a wrong extend
           was an invalid-module risk. Escapes+renders the nominal `(: 5 U8)`, and running proves the module
           is valid. Pairs with the full-width W case (Int64, no extend).")
  (input  (do (type U8 (Mk UInt8)) (def (main (: k UInt8)) (Mk k)) (export main)))
  (call   main (: 5 UInt8)) (output (: 5 U8)))

(case "a COMPOUND-inner single-variant newtype takes the heap escape and its inner is read back"
  (doc    "The compound-inner counterpart: `(type LW (Mk (List Int64)))` erases to a list HANDLE, so it stays
           on the heap/resource escape branch (the recursive-sum-branch guard keeps it there) — the fix only
           diverts SCALAR-erased newtypes to the value-form branch. Wrapping a runtime-built [1,2] and matching
           it back reads the inner list length 2, confirming the compound path is unaffected.")
  (input  (do (type LW (Mk (List Int64)))
              (def (wrap (: xs (List Int64))) (Mk xs))
              (def (main) (match (wrap (List.push (List.push #list() 1) 2)) ((Mk ys) (List.len ys))))
              (export main)))
  (output (: 2 Int64)))

(case "a scalar newtype matched back in place round-trips the erased inner"
  (doc    "Control (the matched face always worked — the match re-erases the Payload step): `(match (Mk k)
           ((Mk v) v))` deconstructs the newtype in place and returns the erased inner k. k=5 -> 5. Pins the
           scalar-newtype value round-trip is unbroken by the escape-branch fix.")
  (input  (do (type W (Mk Int64)) (def (main (: k Int64)) (match (Mk k) ((Mk v) v))) (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

(case "a scalar newtype wrapped by a param'd helper then matched back crosses the internal call boundary"
  (doc    "Control: a param'd helper `wrap` returns the newtype (the escaping-def value), and main matches it
           back — the erased scalar crosses the internal call boundary and is deconstructed. A NEGATIVE value
           (wrap(-9) then (Mk v)->v = -9) exercises sign preservation of the erased scalar across the internal
           call. Pins the wrapped-then-matched round-trip alongside the direct escape.")
  (input  (do (type W (Mk Int64))
              (def (wrap (: k Int64)) (Mk k))
              (def (main (: k Int64)) (match (wrap k) ((Mk v) v)))
              (export main)))
  (call   main (: -9 Int64)) (output (: -9 Int64)))

(case "the NULLARY scalar-newtype escape renders the nominal (: 5 W), agreeing with the param'd path"
  (doc    "The nullary counterpart of the param'd escape: `(def (main) (Mk 5))` bakes the constant and returns
           the newtype. It renders the SAME nominal `(: 5 W)` the param'd export does — pinning that the two
           escape paths AGREE (the adv-64 regression was the param'd path DIVERGING from the always-nominal
           nullary path). With the param'd case above, this closes the adv-64 agreement pin.")
  (input  (do (type W (Mk Int64)) (def (main) (Mk 5)) (export main)))
  (output (: 5 W)))

(case "a NARROW-inner scalar newtype escape at a NEGATIVE value renders the nominal (: -300 I16)"
  (doc    "The second width-edge face (paired with the U8 case): `(type I16 (Mk Int16))` at a NEGATIVE value
           exercises the i32-slot box's SIGN handling across the escape — a wrong (zero- vs sign-) extend would
           corrupt a negative narrow inner. Escapes+renders the nominal `(: -300 I16)`; a running case proves
           the module valid, covering the I16 validity face the migrated Rust test checked, in the corpus.")
  (input  (do (type I16 (Mk Int16)) (def (main (: k Int16)) (Mk k)) (export main)))
  (call   main (: -300 Int16)) (output (: -300 I16)))

(case "a multi-variant sum box is REAL not erased: (Some k) wrapped then matched round-trips"
  (doc    "Control: a MULTI-variant sum `(Some k)` is a genuinely boxed value (unlike the erased single-variant
           newtype), so its box is not erased away. A param'd helper wraps it and main matches it back —
           wrap(42) then (Some v)->v / (None)->0 = 42. Pins that the single-variant erase-and-escape fix leaves
           a real multi-variant sum box untouched.")
  (input  (do (def (wrap (: k Int64)) (Some k))
              (def (main (: k Int64)) (match (wrap k) ((Some v) v) ((None) 0)))
              (export main)))
  (call   main (: 42 Int64)) (output (: 42 Int64)))
