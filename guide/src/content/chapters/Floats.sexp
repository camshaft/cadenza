(chapter
  (slug "floats")
  (title "Floating-point numbers")
  (pillar "language")
  (section "Fundamentals")
  (blurb "Real-valued arithmetic with its own operators.")
  (lede "Real-valued arithmetic with one set of operators dispatched on the operand type, so a mix with integers can never slip by unnoticed.")
  (p "Alongside the checked integers from " (c "The numeric model") ", Cadenza has floating-point numbers. A number written with a decimal point is a " (c "Float64") ", a 64-bit IEEE-754 value that most languages call a \"double\".")
  (runnable
    (source 3.14))
  (h2 "The same operators, dispatched on the type")
  (p "You add, subtract, multiply, and divide floats with the " (em "same") " " (c "+") ", " (c "-") ", " (c "*") ", " (c "/") " operators as integers. There's no separate float operator to remember, because when both operands are floats " (c "+") " " (em "is") " floating-point addition, and when both are integers it's integer addition. The operand type decides which arithmetic you get.")
  (runnable
    (source (- 5.0 1.5)))
  (runnable
    (id "float-div")
    (source (/ 7.0 2.0)))
  (p "Notice " (cdz (/ 7.0 2.0)) " is " (result (of "float-div") 3.5) ", which is real division because the operands are floats. Give " (c "/") " two whole numbers and the very same operator does the integer division you saw earlier. Same symbol; the values in your hands choose the meaning.")
  (h2 "Floating-point is approximate, and honest about it")
  (p "IEEE-754 floats can't represent every decimal exactly, and Cadenza doesn't pretend otherwise. The classic example: add a tenth and two tenths, and the result isn't quite three tenths.")
  (runnable
    (id "float-imprecision")
    (source (+ 0.1 0.2)))
  (p "That " (result (of "float-imprecision") 0.30000000000000004) " isn't a bug, since it's the true value of the nearest float to the sum, the same answer you'd get in any IEEE-754 language. Cadenza shows you the real number rather than rounding it away, so what you read is what your program actually computed.")
  (h2 "Ints and floats never mix silently")
  (p "Try to add an integer to a float and the compiler refuses, the same way it refuses to add a number and a boolean. There is no automatic widening from " (c "Int64") " to " (c "Float64") ". The rejection doesn't come from the operator naming a type; it comes from the two operands disagreeing.")
  (note "This example is " (strong "meant to be refused") ". Run it and read the diagnostic: the compiler declines with " (c "CDZ0301") " rather than inventing a conversion, and it suggests a one-token fix to make the two operands agree (here, dropping the " (c ".0") " so both are integers).")
  (runnable
    (source (+ 2 2.0))
    (expect "error"))
  (why (tenet "No silent promotion; refuse the ambiguity") "Most languages quietly promote the " (c "2") " to " (c "2.0") " here. That convenience hides a real decision: converting an exact integer to an approximate float can lose information, and doing it automatically means the loss happens somewhere you never wrote. Cadenza makes you say it. There's just one " (c "+") ", and it requires both operands to be the " (em "same") " numeric type, so an integer and a float can't blur together by accident. A mismatch is a compile-time error pointing right at the spot, not a rounding surprise discovered in production.")
  (h2 "Converting on purpose")
  (p "When you " (em "do") " want to turn an integer into a float, you ask for it by name with " (c "Float64.of-int") ". It's an ordinary function, visible and deliberate and exactly where you meant the conversion to happen.")
  (runnable
    (source (Float64.of-int 7)))
  (p "Now the number is a float, so it composes with float arithmetic using the same " (c "*") ", now multiplying two floats:")
  (runnable
    (source (* (Float64.of-int 3) 1.5)))
  (h2 "A worked example")
  (p "Putting it together, here's the area of a circle, all in floating-point. Edit the radius and Run.")
  (runnable
    (source (def (area (: r Float64))
  (* 3.14159 (* r r)))
(def (main) (area 2.0))))
  (h2 "Two widths, never mixed silently")
  (p "There's a 32-bit float too, " (c "Float32") ", with its own " (c "Float32.of-int") ". It's a real, runnable value, and dividing two of them gives a " (c "Float32") " back, " (c "7 ÷ 2 = 3.5") ":")
  (runnable
    (source (/ (Float32.of-int 7) (Float32.of-int 2))))
  (p "And the two widths follow the same rule as everything else: they don't blend on their own. Add a " (c "Float32") " to a " (c "Float64") " and the compiler stops you, because a " (c "Float32") " and a " (c "Float64") " have different precision, so combining them is a conversion you must write, not one the language guesses:")
  (note "This one is " (strong "meant to be refused") ". Run it and the diagnostic is " (c "CDZ0301") ", \"floating-point precisions differ\", the same no-silent-widening rule that keeps " (c "Int64") " and " (c "Float64") " apart, now between the two float sizes.")
  (runnable
    (source (+ (Float32.of-int 1) (Float64.of-int 2)))
    (expect "error"))
  (p "A float trades exactness for speed, and is honest about it. But when a rounding error would be a " (em "bug") ", as with money or exact ratios, you want the opposite trade. That's " (em "exact fractions") ", next.")
  (h2 "Your turn")
  (exercise
    (id "floats:1")
    (prompt "Finish the expression so it halves " (c "9.0") " to give " (c "4.5") ".")
    (starter (/ 9.0 ?))
    (solution (/ 9.0 2.0))
    (expected "4.5")
    (hint "Both operands must be floats for " (c "/") " to do real division, so the divisor is " (c "2.0") ", not " (c "2") " (a whole " (c "2") " would make it an int/float mix, which is refused)."))
  (exercise
    (id "floats:2")
    (prompt "The two floats here must be the " (em "same width") " to divide. One operand is already a " (c "Float32") ", so convert the " (c "5") " at the matching width for " (c "/") " to work and " (c "5 ÷ 2") " to give " (c "2.5") ". Which of " (c "Float32") " / " (c "Float64") " goes in the blank?")
    (starter (/ (Float?.of-int 5) (Float32.of-int 2)))
    (solution (/ (Float32.of-int 5) (Float32.of-int 2)))
    (expected "2.5")
    (hint "The other operand is a " (c "Float32") ", and widths don't mix, so convert at " (c "Float32") " too. Pick " (c "Float64") " instead and the compiler declines (" (c "CDZ0301") ", precisions differ).")))
