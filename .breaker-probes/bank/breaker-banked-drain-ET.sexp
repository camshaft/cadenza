(case "a CONST checked-arithmetic chain folds through Option match arms exactly"
  (doc    "The FOLD-path composition of checked arithmetic at the range boundary: `checked-mul 2^62 2`
           overflows by EXACTLY ONE past Int64.max (the tightest miss — a fold using unsigned or
           128-bit intermediate without the signed-range check says Some 2^63), taking the None arm;
           the fallback `checked-add (max-1) 1` lands EXACTLY ON max (the tightest hit — an
           off-by-one range check says None), folding to Some max → 9223372036854775807. Both
           checked ops and BOTH match dispatches fold at compile time; a fold-path range check that
           diverged from the runtime semantics by one ULP at either boundary flips an arm. The const
           companion of the :4891 family (whose operands are far from the boundary).")
  (input  (do
            (def (main)
              (match (Int64.checked-mul 4611686018427387904 2)
                ((Some v) v)
                ((None u) (match (Int64.checked-add 9223372036854775806 1)
                            ((Some w) w)
                            ((None u2) -1)))))
            (export main)))
  (call   main) (output (: 9223372036854775807 Int64)))
