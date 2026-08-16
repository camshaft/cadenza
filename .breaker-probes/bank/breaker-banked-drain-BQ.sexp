; breaker probe V2 — the COMPUTED-expression face of the static-label rule: the :783 pin rejects a
; BARE-IDENTIFIER field-name operand (the pun footgun); this pins that a genuinely COMPUTED label —
; an if branching between two REAL #symbol literals — also rejects CDZ0215 (uniform ×3 targets).

(case "a computed (runtime-branched) label operand to Record.with is rejected — labels are static"
  (doc    "The computed-expression companion of the bare-identifier CDZ0215 pin above: `(Record.with r
           (if b #\"x\" #\"y\") 9)` supplies the name-introduction operand as an `if` over two genuine
           `#label` literals — a well-typed Symbol expression, but not a STATIC label. The static-label
           rule (a field name is part of the record's TYPE, so it cannot be runtime data) rejects it
           CDZ0215 exactly as it rejects the bare-identifier pun, on all targets uniformly. Guards the
           other half of the footgun: a user computing a Symbol and expecting dynamic field naming gets
           the coded static-label diagnostic, not a silent pun or a backend-dependent behavior. (Dynamic
           key→value association is what Map is for.)")
  (input  (do
            (def (main (: b Bool))
              (let ((r2 (Record.with (record (x 1) (y 2)) (if b #"x" #"y") 9)))
                (+ (* 10 (. r2 x)) (. r2 y))))
            (export main)))
  (call   main (: true Bool)) (error CDZ0215))
