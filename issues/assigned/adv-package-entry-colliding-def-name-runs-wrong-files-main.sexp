; BREAKER FINDING 2026-07-17 (trunk e47142e5d) — PACKAGE-LINKING silent WRONG-PROGRAM: when two files
; of a multi-file package both define a def with the SAME NAME (e.g. both `main`), the `--entry` file's
; export boundary silently binds the OTHER file's def — the artifact runs the wrong file's code and
; returns its value, no diagnostic.
;
; MINIMAL (two files, no imports):
;     aaa.sexp: (do (def (main (: n Int64)) (* n 100)) (export main))
;     zzz.sexp: (do (def (main (: n Int64)) (* n 7))   (export main))
;     cdz compile <dir> --entry zzz -o pm1.wasm ; cdz run pm1.wasm --arg 3   -> 300   (aaa's main!)
;     cdz compile <dir> --entry aaa                                          -> 300   (same artifact)
;     explicit file list + --entry zzz                                       -> 300   (same)
; Expected: --entry zzz -> 21 (the entry file's OWN main), or at minimum a loud duplicate-name reject.
;
; CONTROL (disjoint names, same layout): aaa exports `amain`, zzz exports `zmain` — --entry zzz
; correctly exports only `zmain` and runs 21; --entry aaa exports only `amain`. So the ENTRY-selection
; and boundary-scan are fine; the failure is NAME RESOLUTION: resolve's flat `db.def_by_name` (all
; files spliced flat) resolves the entry's exported name `main` to the FIRST same-named def in file
; order, not the entry file's own. DESIGN-package-linking.md §4 calls this exact spot out — "the ONE
; new resolver rule" (per-file namespaces / import-scoped resolution) — the linker currently splices
; without it.
;
; SEVERITY: silent wrong-program (a library that happens to define a same-named private helper hijacks
; the entry's def — no error, plausible output). Discovered via the effect-twin: an eapp/elib pair
; sharing a directory with an unrelated app/lib pair ran the WRONG main (15 instead of 41) as soon as
; the colliding-name files were present.
;
; Also worth pinning alongside the fix: a same-file duplicate def name is rejected (11-modules
; duplicate-definition family) — the CROSS-FILE collision should either resolve per-file (design §4)
; or reject as loudly.
;
; SHARPER: the corpus ALREADY pins per-(file,name) private helpers not colliding (11-modules:305 —
; entry's `foo` vs lib's private `foo`, both CALLED internally, resolve per-file, 16). What is broken
; is the ENTRY'S EXPORTED name specifically: the export-boundary binding (scan_top_level -> db.exports
; -> layout) goes through the flat def_by_name, not the per-file resolution the call path uses. Even a
; PRIVATE (unexported) library `main` hijacks the entry's exported `main` (control: lib defines main
; privately + exports `other`; --entry zzz STILL runs lib's main -> 300). So internal CALLS resolve
; per-file (the pinned case), but the EXPORT binding resolves flat — one missed resolution site.
(case "a package entry's exported def wins over a same-named def in a library file"
  (doc    "Two package files each define `main`; the compile names `--entry` the file whose `main`
           multiplies by 7. The component's exported `main` must be the ENTRY file's own def — n=3 ->
           21 — not the other file's (n*100 = 300). Currently the flat cross-file `def_by_name` binds
           the export to the alphabetically-first file's `main`, so BOTH `--entry` choices produce the
           n*100 artifact: a silent wrong program. (Disjoint-name control works — the bug is only the
           cross-file name collision; DESIGN-package-linking.md §4's per-file resolver rule is the
           missing piece.)")
  (input  (package
            (file "aaa" (do (def (main (: n Int64)) (* n 100)) (export main)))
            (file "zzz" (do (def (main (: n Int64)) (* n 7)) (export main)))
            (entry "zzz")))
  (call   main (: 3 Int64))
  (output (: 21 Int64)))
