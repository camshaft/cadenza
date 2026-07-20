;; LEAK (not a miscompile — value is CORRECT): `Option.expect` (Core::SumExpect) over an OWNED-TEMPORARY
;; `Some` sum-shell does NOT drop the shell — it leaks ONE heap cell PER CALL. Surfaced by the direct
;; live-objects gate (owned_temporary_list_producers_leave_no_live_objects's read-op companion), NOT by
;; value-equality or drop-import-presence.
;;
;; ROOT (candidate): the `Core::SumExpect` emit (backend/wasm/select.rs ~7937) reads its scrutinee handle
;; TWICE (sum-disc probe + sum-payload read), BOTH BORROWING, and NEVER drops the scrutinee. When the
;; scrutinee is an OWNED-TEMPORARY `Some` (e.g. the result of `List.at`/`Map.lookup`, which build a fresh
;; `sum-new` shell around the extracted+dup'd payload), that shell is never reclaimed. `heap_operand_ownership`
;; would classify a `SumNew`/`Call` scrutinee as Owned — so `SumExpect` should stash the scrutinee and, after
;; the payload is extracted (and the compound payload dup'd for the borrow), DROP the owned shell.
;; ⚠ DELICATE: the drop must free ONLY the Some SHELL, not cascade into the extracted payload the caller now
;; owns — the payload is already dup'd in the present arm (~select.rs 8018), so dropping the shell after that
;; dup is refcount-correct. A borrowed scrutinee (a param/kept-local `Some`) must NOT be dropped. Mirror the
;; List.at/Map.lookup owned-collection reclaim (heap_operand_ownership==Owned gate).
;;
;; WITNESS (needs the debug-counters runtime + live-objects; a loop so it SCALES past the benign
;; entrypoint-return temp): List.at over a runtime list, Option.expect'd, in a 500-iter loop → live-objects
;; 500 (leaks 1/call). Map.lookup identical. Set.contains (bool result, no Some shell) → 0 (clean).
;; Value is CORRECT throughout (List.at→1, Map.lookup→10). SHARED SEAM: the SumExpect/MatchSum emit is
;; v-patterns territory — coordinate before fixing.
(module m
  (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
    (if (< i n) (build (+ i 1) n ((. List push) acc i)) acc))
  (def (loop (: j Int64) (: n Int64) (: tot Int64))
    (if (< j n)
        (loop (+ j 1) n (+ tot ((. Option expect) ((. List at) (build 0 3 (list)) 1) "v")))
        tot))
  (def (f (: n Int64)) (loop 0 n 0))
  (export f))

;; ===== PM triage (corpus-bugfix, 2026-07-20, trunk 7a065bbf7) =====
;; Value-correctness VERIFIED (f(5)=5); the LEAK itself needs the debug-counters/live-objects harness (not
;; the standard wasm run), so not cheaply witnessed here. ROUTED to v-memory-safety (primary: reclaim/
;; ownership classification) with v-patterns cc'd (SumExpect/MatchSum emit shared seam) — repro asks them to
;; coordinate before fixing. NOT a fix agent (deep Perceus+emit shared-seam work in the owners' lane).
;; corpus-bugfix to add a live-objects pin once fixed. Copy already in both owners' issues/.

;; ===== RESOLVED (corpus-bugfix, 2026-07-20, trunk 995fa4134) =====
;; ALREADY FIXED + MERGED — my route last tick was STALE (fix landed after the 2026-07-18 filing).
;; SumExpect fix: d4b77be35 'reclaim the owned Some-shell in Option.expect (SumExpect)' — stashes an Owned
;; scrutinee + drops the Some shell after payload extract+dup, gated to owned-temporaries (borrowed left alone).
;; Verified: d4b77be35 is-ancestor-of-trunk = true; select.rs carries the shell-reclaim. MatchSum twin
;; cbd1b35ab also landed (same all-scalar-payload shell drop; compound-payload shells left un-dropped =
;; residual leak, never UAF). Regression-guarded by the direct-live-objects rc-leak gate (core-4 + nightly).
;; Retired from queue; the v-memory-safety/v-patterns route I sent is moot.
