; Rows and open sums — witnesses type-system.md #Records Are Rows, Open By Default Under Inference,
; #A Sum Type May Be Open, With A Mandatory Open-Tail Arm, and #An Open Sum's Payload May Be
; Schema-Typed. These are (needs rows) / (needs open-sums) cases a later generation realizes; the seed
; realizes closed records and closed sums (05-compound-types) but not row polymorphism or open sums.
; The primary clause is the recorded oracle: a well-typed program's value, or — for an ill-typed one —
; its (error <CODE>) rejection (a rule a generation does not yet cover is declined, not run).

(case "a function open over a record's extra fields accepts any record with the used field"
  (doc    "Witnesses type-system.md #Records Are Rows, Open By Default Under Inference: `get-x` uses only
           field `x`, so it is typed open over the other fields and accepts a record that also has `y`.
           Row polymorphism, not a fixed shape, is what inference assigns.")
  (needs  rows)
  (input  (module m
            (def (get-x r) (. r x))
            (def (main) (get-x (record (x 1) (y 2))))))
  (output (: 1 Int64)))

(case "subset record comparison is explicit projection, not an overloaded equality"
  (doc    "Witnesses type-system.md #Records Are Rows (subset comparison is explicit projection-then-=):
           comparing a two-field record against a one-field record by first projecting the shared field
           yields true; `=` is never silently widened to ignore the extra field.")
  (needs  rows)
  (input  (module m
            (def (main)
              (= (. (record (x 1) (y 2)) x)
                 (. (record (x 1)) x)))))
  (output (: true Bool)))

(case "a match on an open sum with an open-tail arm is exhaustive"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: an open sum
           carries variants the module does not close; a match covering the known variant plus an
           open-tail arm is exhaustive and handles an unknown variant as data.")
  (needs  open-sums)
  (input  (module m
            (def (name-of e)
              (match e
                ((Known _) "known")
                (_         "other")))
            (def (main) (name-of (Known unit)))))
  (output (: "known" String)))

(case "a match on an open sum omitting the open-tail arm is rejected"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open (a match that omits the open-tail arm is a
           compile-time rejection): because an open sum's variant set is not closed, a match without an
           open-tail arm cannot be exhaustive and is rejected (CDZ0210) rather than run.")
  (needs  open-sums)
  (input  (module m
            (def (name-of e)
              (match e
                ((Known _) "known")))
            (def (main) (name-of (Unknown unit)))))
  (error  CDZ0210))

(case "an open sum's payload decodes against a schema to a typed result"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed: a variant's payload is
           decoded against a schema resolved at run time, yielding a typed Ok result on a match. A
           successful decode of an Int64 payload yields (Ok 7).")
  (needs  open-sums)
  (input  (module m
            (def (main)
              (decode Int64-schema (payload-of (Measured 7))))))
  (output (: (Ok 7) (Result Int64 DecodeError))))

(case "an open sum payload that does not match its schema yields a typed failure, not a trap"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed (a mismatch yields a typed
           failure result rather than a trap): decoding a String payload against an Int64 schema yields
           an Err, so a fold over an open vocabulary handles a malformed payload as data rather than
           halting.")
  (needs  open-sums)
  (input  (module m
            (def (main)
              (decode Int64-schema (payload-of (Labeled "x"))))))
  (output (: (Err (DecodeError unit)) (Result Int64 DecodeError))))
