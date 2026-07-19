;; BUG (2026-07-19, v-effects, root-caused from v-cad's "no local slot" issue MR a718fb474):
;; a lambda that PERFORMS, passed as a fn-typed PARAM to a function that APPLIES it under a handler,
;; is wrongly rejected CDZ0401 "no home" — the perform is flagged at the lambda's DEFINITION site
;; (in `main`, no enclosing handler) even though the lambda is APPLIED inside `with-seed` UNDER the
;; `Rand` handler, so the perform IS homed at runtime. The no-home analysis (check_no_home_walk,
;; effects.rs ~1140) checks a lambda's body where it is DEFINED, not where it is APPLIED.
;;
;; ISOLATION: an INLINE lambda applied under the handler `((fn (u) (Rand.roll)) unit)` COMPILES
;; (it β-reduces at the handle site, exposing the perform to the handler). A fn-PARAM `(body unit)`
;; where body = a performing lambda passed in does NOT (the param application is opaque to the walk).
;; The THREADED equivalent (no effect) compiles. This is the `handler runs a passed-in closure that
;; performs` idiom — legitimate + common (v-cad's `with-seed(body)` snowflake).
;;
;; RELATED to v-cad's fuller "parameter reference has no local slot" (a later codegen symptom on a
;; recursive Solid-returning builder) — same family (effect under a passed closure); this is the
;; minimal earlier face. FIX (deferred, multi-tick): the no-home walk must home a perform in a lambda
;; that flows into a handler-enclosed application (track apply-site, not def-site), OR the fold must
;; β-reduce/inline the (body unit) application at the handle site like an inline lambda.
(do
 (effect Rand (op roll (-> Unit Int64)))
 (def (with-seed (: body (-> Unit Int64))) (handle Rand 5 ((roll (u) s (resume s s))) (body unit)))
 (def (main) (with-seed (fn (u) (Rand.roll))))
 (export main))
