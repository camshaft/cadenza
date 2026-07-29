;; HELD-FOR-RULING (corpus-bugfix, 2026-07-29): breaker #41-family CONSISTENCY GAP (issue 17964).
;; Abstract-typed values as CHAMP MAP KEYS bypass the CDZ0202 opacity ruling that direct comparison honors.
;; CONFIRMED trunk 2cb5af98f both backends:
;;   • DIRECT (= (mk k) (mk k)) on abstract Temp -> CORRECTLY rejects CDZ0202 ("abstract type ... cannot be
;;     observed through a built-in comparison") — breaker pinned this today.
;;   • ABSTRACT value as a CHAMP KEY: (Map.lookup (Map.insert Map.empty (mk k) 42) (mk k)) -> COMPILES,
;;     returns 42 (wasm+rust). champ_eq reads the same content the direct (=) reject forbids. An eq-oracle
;;     built from it ((match (Map.lookup (Map.insert Map.empty a 1) b) (Some _ -> equal) (None -> unequal)))
;;     distinguishes equal/unequal abstract values — the exact observation CDZ0202 denies.
;; INCONSISTENCY: opacity blocks DIRECT (=) but the KEY path (internal content-eq) doesn't. Smaller leak
;;   than the bare-match (no payload READ, only equality probes) but the same discipline.
;; RULING NEEDED (two options flip DIFFERENT pins — not a unilateral pin):
;;   (a) REJECT abstract-typed CHAMP KEYS with CDZ0202 (VALUES stay legal — breaker's held-as-values pin is
;;       the boundary). Consistent with the direct-eq reject + bare-pattern opacity discipline.
;;   (b) BLESS content-eq for abstract types + DROP CDZ0202 (then breaker's fresh eq-reject pin flips).
;; LEAN (corpus-bugfix) = (a) REJECT abstract keys: consistent with the just-pinned direct-eq CDZ0202 + the
;;   #41 bare-pattern opacity fixes; blessing content-eq weakens the ADT/smart-ctor discipline the
;;   verification kernel relies on (same trust story as the bare-pattern soundness hole). ROUTED concierge.
;; ON RULING: (a) -> pin an (error CDZ0202) abstract-key case + route the key-path reject to v-inference/
;;   v-runtime (champ key visibility gate); (b) -> pin a value case + flip breaker's eq-reject pin, doc it.
;; Same visibility sweep as #41 bare-pattern (one gate could cover both). No unilateral pin.

;; ============================================================================
;; RULED = (a) REJECT (concierge, 2026-07-29, answer 17967). Matches my lean; type-system.md:180 makes
;; it a MUST: "A built-in structural comparison whose operand is a value of an abstract type ... MUST be
;; rejected outside the declaring module." A CHAMP map keyed by an abstract value invokes a built-in
;; structural comparison at insert/lookup (champ_eq/value-eq over the key spine — 03-equality:839/879
;; value-eq IS structural, :297 CHAMP/Set keys hash+match by it). The rule is about the OBSERVATION, not
;; the surface `(= a b)`, so the key path is the SAME violation (indirect route, same leak).
;; SEMANTIC OUTCOME: an abstract-typed value used as a CHAMP/collection KEY (Map or Set) outside its
;; declaring module MUST reject CDZ0202, same as direct (=). VALUES stay legal to HOLD (as payloads etc.);
;; only the key-EQUALITY-OBSERVATION is rejected. (b) rejected (violates :180 MUST + flips breaker's pin).
;; CURRENT trunk 2cb5af98f: still COMPILES (returns 42) -> PIN-ON-FIX, not landable now.
;; FLIP-SET: concierge's scan finds NO pin blessing abstract Map/Set keys (README:213 opacity: holding a
;;   value is fine, comparing its structure is not), but — per the last over-claim lesson — v-inference
;;   produces the AUTHORITATIVE flip-set across ALL spec/semantics/*.sexp (embedded call-sites too) before
;;   landing; migration-first if any surface. ROUTED to v-inference (key insert/lookup type-check path);
;;   can SHARE the #41 visibility sweep (one gate covers direct-eq + key-path). ON FIX: gate x3 ->
;;   (error CDZ0202); pin into 19-sets or 03-equality beside the direct-eq CDZ0202 pin; baseline x3.

(case "an abstract-typed value used as a CHAMP map key is rejected (opacity — key path invokes the forbidden structural comparison)"
  (input  (do
        (import "temp" (Temp mk))
        (def (main (: k Int64)) (match (Map.lookup (Map.insert Map.empty (mk k) 42) (mk k)) ((Some v) v) ((None _u) -1)))
        (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (export Temp)
      (export mk)))
  (call   main (: 5 Int64)) (error CDZ0202))

;; ACCEPTED + SEQUENCED (v-inference, 2026-07-29, reply 17971): reject is a sibling of the direct-eq
;; abstract check (infer.rs:8512 abstract_operand = nominal_or_sum_decl(ty) && is_abstract_type_at) applied
;; to the KEY type at the champ-key prims (MapInsert 2658/9650, SetOf 9721/13090, MapGet, Set membership).
;; Shared "abstract KEY type" gate; diag "abstract type used as a map/set key — representation can't be
;; observed through key equality; compare via a function the module exports". FLIP-SET METHOD = fix-then-
;; gate --check (the pass->CDZ0202 cases ARE the authoritative migration list; grep can't do it — needs the
;; compiler's visibility+type analysis; v-inference's crude scan matched 10 files by feature-cooccurrence,
;; none confirmed). Same discovery as the CDZ0215 lockstep. SEQUENCING: v-inference builds this AFTER the
;; CDZ0215/CDZ0214 lockstep lands (avoid a 4th held reject on an un-gate-able store); NOT forgotten, queued.
;; ON LAND: v-inference hands me the gate-derived migration list (migration-first if any surface) + routes
;; the reject; then gate my graded (error CDZ0202) key case x3 + pin into 19-sets/03-equality.

;; BUILT + MR'd (v-inference, 2026-07-29): 68e580932 (queued). FLIP-SET is EMPTY — gate 5164/0/0 clean, NO
;; corpus case keys a Map/Set by an abstract value (my grep-scan was right; the compiler's visibility+type
;; analysis confirms). So it lands STANDALONE — no migration, no lockstep. Abstract-type soundness arc now
;; COMPLETE across all 3 routes: CDZ0214 (ctor pattern) + CDZ0215 (field-label) + CDZ0202 (collection key).
;; ON LAND (68e580932): gate my graded (error CDZ0202) key case x3; pin into 19-sets or 03-equality beside
;; the direct-eq CDZ0202 pin; baseline x3.
