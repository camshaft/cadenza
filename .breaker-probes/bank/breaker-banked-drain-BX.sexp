(case "eval of a quoted record-construction form folds to the runtime record"
  (doc    "`(eval (quote (record (x 7) (y 5))))` reconstructs the `record` constructor form and folds
           it to the runtime record, so `(. r x)` projects 7. The RECORD companion of the pinned
           list/tuple construction cases: the reconstructed form must build the canonical
           (sorted-field) record a literal builds, observable by member access — pins that eval's
           data-construction vocabulary includes the labeled-product constructor, not only the
           positional ones.")
  (input  (. (eval (quote (record (x 7) (y 5)))) x))
  (output (: 7 Int64)))

(case "eval of a quoted Map-construction form folds to the runtime map"
  (doc    "`(eval (quote (Map.insert Map.empty 1 42)))` reconstructs a member-access op head
           (`Map.insert`, the desugared `(. Map insert)` — not a bare prelude name like `list`) applied
           to the empty-map constant, folds it to the runtime CHAMP map, and `Map.lookup` reads the
           entry back (42). Extends the eval data-construction family to a COLLECTION built through a
           module-op chain — both the op-head reconstruction and the CHAMP value surviving the eval
           boundary.")
  (input  (match (Map.lookup (eval (quote (Map.insert Map.empty 1 42))) 1)
            ((Some v) v) ((None u) -1)))
  (output (: 42 Int64)))
