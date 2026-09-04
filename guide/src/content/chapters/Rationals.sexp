(chapter
  (slug "rationals")
  (title "Exact fractions")
  (pillar "language")
  (section "Fundamentals")
  (blurb "Rationals: exact ratios, always in lowest terms, so where floats round, these don't.")
  (lede "A " (c "Float64") " is fast but approximate, so when you need an exact fraction like a third that really is a third, reach for a " (c "Rational") ".")
  (p "You saw in " (strong "Floating-point numbers") " that " (cdz (+ 0.1 0.2)) " isn't quite " (c "0.3") ", because floats trade exactness for speed. A " (c "Rational") " makes the other trade by holding a number as an exact ratio of two integers, so arithmetic never rounds. Build one with " (c "Rational.of") ", giving a numerator and a denominator:")
  (runnable
    (id "rational-half")
    (source (Rational.of 1 2)))
  (p "The value comes back as " (result (of "rational-half") (: 1/2 Rational)) " tagged with its type, which reads " (c "1/2 : Rational") " in the conventional surface and " (c "(: 1/2 Rational)") " in s-expressions. Since a whole number is just a denominator of one, " (c "Rational.of-int") " makes that explicit:")
  (runnable
    (source (Rational.of-int 5)))
  (h2 "Writing one directly: the " (c "R") " suffix")
  (p "Spelling out " (c "Rational.of") " every time is wordy when you already know the number. A decimal with an " (c "R") " suffix is a rational " (em "literal") " that the compiler reads exactly and converts to a fraction, so " (c "0.5R") " is " (c "1/2") " and " (c "1.25R") " is " (c "5/4") ":")
  (runnable
    (source 0.5R))
  (p "It's the very same value as the constructor, since " (c "0.5R") " equals " (cdz (Rational.of 1 2)) ", just terser to write. And it's where the contrast with " (strong "Floating-point numbers") " stops being a claim and becomes something you can watch. Add a tenth and two tenths as " (em "floats") " and the answer isn't " (c "0.3") " but the nearest float to it, which isn't quite " (c "0.3") ":")
  (runnable
    (source (+ 0.1 0.2)))
  (p (c "0.30000000000000004") ", so the drift is real rather than hypothetical, and the natural test " (em "fails") " because the float sum is not equal to " (c "0.3") ".")
  (runnable
    (source (= (+ 0.1 0.2) 0.3)))
  (p (c "false") ". Now write the very same digits as rational literals. The sum is " (em "exactly") " " (c "3/10") ", so the same equality that failed for floats holds for rationals:")
  (runnable
    (source (= (+ 0.1R 0.2R) 0.3R)))
  (p (c "true") ", the same digits you'd type for a float with one letter's difference, and now " (c "0.1 + 0.2") " is the number you " (em "meant") ". The float wasn't buggy; it was doing exactly what binary floating-point must. The " (c "Rational") " simply makes the other trade of exactness over speed, so the arithmetic never rounds in the first place.")
  (h2 "Whole numbers that outgrow Int64: " (c "BigInt"))
  (p "The same instinct of trading speed for exactness when it matters has a whole-number counterpart. An " (c "Int64") " refuses to hold a value past its range, so " (c "9223372036854775807 × 1000") " overflows and the compiler declines rather than wrap.")
  (runnable
    (source (* 9223372036854775807 1000))
    (expect "error"))
  (p "When you genuinely need bigger, " (c "BigInt") " is the arbitrary-precision integer that grows to fit any whole number. Build one with " (c "BigInt.of") " (or write the " (c "N") " literal suffix), and the product that overflowed an " (c "Int64") " is exact:")
  (runnable
    (source (* (BigInt.of 9223372036854775807) (BigInt.of 1000))))
  (p "The result comes back as a " (c "BigInt") " far beyond the 64-bit range with no overflow and no wrap. It's the same trade as " (c "Rational") ": reach for it when a value must be exact whatever its size, and pay for the arbitrary precision only where you asked for it.")
  (h2 "Always in lowest terms")
  (p "A rational normalizes itself on construction, stored in lowest terms with the sign on the numerator. Ask for " (c "2/4") " and you get back " (c "1/2") ", the same number canonically written:")
  (runnable
    (source (Rational.of 2 4)))
  (p "Because two rationals that denote the same number normalize identically, " (c "=") " compares them by " (em "value") ": " (c "2/4") " and " (c "1/2") " are equal, however you wrote them.")
  (h2 "Taking a rational apart")
  (p "Sometimes you want the two integers back out, to display a fraction or to feed its parts on somewhere. " (c "Rational.numerator") " and " (c "Rational.denominator") " hand them over, and because a rational is always stored in lowest terms, they give you the " (em "reduced") " pair, not whatever you happened to type. Ask " (c "2/4") " for its numerator and it's " (result (of "rat-num") (: 1 BigInt)) ", since the value is really " (c "1/2") ":")
  (runnable
    (id "rat-num")
    (source (Rational.numerator (Rational.of 2 4))))
  (p "The denominator of that same " (c "2/4") " is " (result (of "rat-den") (: 2 BigInt)) ", completing the reduced " (c "1/2") ". Both come back as a " (c "BigInt") ", so a numerator or denominator that outgrows 64 bits is carried exactly like any other exact integer:")
  (runnable
    (id "rat-den")
    (source (Rational.denominator (Rational.of 2 4))))
  (p "This is a clean way to " (em "see") " that arithmetic really did stay exact. Add a third three times and ask the result for its denominator: it's " (result (of "rat-den-sum") (: 1 BigInt)) ", because the sum is exactly " (c "1/1") ", not a fraction a hair away from one.")
  (runnable
    (id "rat-den-sum")
    (source (Rational.denominator (+ (+ (Rational.of 1 3) (Rational.of 1 3)) (Rational.of 1 3)))))
  (h2 "Rational to a whole number")
  (p "The numerator and denominator hand back the exact integer " (em "parts") ", each an unbounded " (c "BigInt") ". Sometimes you instead want the whole value " (em "as") " one integer at a boundary, a MIDI tick, an array index, a pixel, and that's a projection to a fixed " (c "Int64") ". There are four, differing only in how they handle a fraction: " (c "truncate") " drops toward zero, " (c "floor") " rounds toward negative infinity, " (c "ceil") " toward positive infinity, and " (c "round") " to the nearest (ties going away from zero). They agree on positive whole-ish values and diverge on negatives:")
  (note (c "value    truncate  floor  ceil  round") " " (br) " " (c "  7/2        3        3     4     4") " " (br) " " (c " -7/2       -3       -4    -3    -4") " " (br) " " (c "  7/3        2        2     3     2"))
  (p "The split to watch is on negatives: " (c "truncate") " of " (c "-7/2") " is " (result (of "rational-trunc") -3) " (toward zero) while " (c "floor") " is " (c "-4") " (toward negative infinity). They only look the same on positives, so a sign change is where a wrong choice bites:")
  (runnable
    (id "rational-trunc")
    (source (Rational.truncate (Rational.of -7 2))))
  (p "And " (c "round") " breaks a tie by going " (em "away") " from zero, so " (c "5/2") " rounds to " (result (of "rational-round") 3) ", not the " (c "2") " that banker's (nearest-even) rounding would give. Cadenza names the rule rather than letting you assume it, the same refusal to guess that runs through the numeric model. All four narrow to " (c "Int64") " and trap on overflow, never silently wrapping:")
  (runnable
    (id "rational-round")
    (source (Rational.round (Rational.of 5 2))))
  (h2 "Arithmetic stays exact")
  (p (c "+") ", " (c "-") ", " (c "*") ", and " (c "/") " over rationals compute the exact result and renormalize. Here's the sum floats can't get right, a third plus a third plus a third, and with rationals it is " (em "exactly") " one:")
  (runnable
    (id "rat-sum")
    (source (+ (+ (Rational.of 1 3) (Rational.of 1 3)) (Rational.of 1 3))))
  (p (result (of "rat-sum") (: 1/1 Rational)) ", not " (c "0.9999999999999999") ". Division is exact too, and unlike integer division it stays total for any nonzero divisor, so " (c "(3/4) / (2/1)") " is " (result (of "rat-div") (: 3/8 Rational)) " with no remainder and no rounding. You can try exact fractions yourself in the " (app-link (route "/calculator") " calculator ") " by typing " (c "1 / 3 + 1 / 3 + 1 / 3") " and watching it come back " (c "1") ".")
  (runnable
    (id "rat-div")
    (source (/ (Rational.of 3 4) (Rational.of 2 1))))
  (why (tenet "Exactness is a choice you can make") "Cadenza doesn't pick one number type and make its weaknesses your problem. A " (c "Float64") " is the right tool when you want speed and can tolerate rounding, as in measurements, graphics, and physics. A " (c "Rational") " is the right tool when a rounding error would be a " (em "bug") ", as in money, exact ratios, and anything that must add up. They're different types with different operators, so you say which guarantee you want, and the compiler never silently swaps one for the other. Same instinct as keeping " (c "Int64") " and " (c "Float64") " apart: one type per kind of number, no surprises.")
  (note "A zero denominator has no value to denote, so " (cdz (Rational.of 1 0)) " is a compile-time error (" (c "CDZ0304") "), the same \"no correct answer, so refuse\" rule as dividing an integer by zero.")
  (p "That refuse-when-there's-no-answer instinct runs through every number type you've now met. The next chapter, " (em "Errors &amp; absence") ", makes it a tool you hold: " (c "Option") " and " (c "Result") " turn a might-not-have-an-answer into an ordinary value you handle.")
  (h2 "Your turn")
  (exercise
    (id "rationals:1")
    (prompt "A rational is compared by value, so equal fractions are " (c "=") " however they're written. Write the fraction " (c "3/6") " in lowest terms so it equals " (cdz (Rational.of 1 2)) " and the comparison gives " (c "true") ".")
    (starter (= (Rational.of 3 6) (Rational.of 1 ?)))
    (solution (= (Rational.of 3 6) (Rational.of 1 2)))
    (expected "true")
    (hint (c "3/6") " reduces to " (c "1/2") ", so the denominator is " (c "2") ". Equal rationals compare " (c "=") ", giving " (c "true") "."))
  (exercise
    (id "rationals:2")
    (prompt "Division is exact, so ask how many quarters are in a half. Divide " (c "1/2") " by a quarter so the result is " (c "2/1") ", filling in the divisor's denominator, and when it's right the check gives " (c "true") ".")
    (starter (= (/ (Rational.of 1 2) (Rational.of 1 ?)) (Rational.of 2 1)))
    (solution (= (/ (Rational.of 1 2) (Rational.of 1 4)) (Rational.of 2 1)))
    (expected "true")
    (hint "A quarter is " (cdz (Rational.of 1 4)) ". Dividing by it multiplies by its reciprocal " (c "4/1") ", so " (c "1/2") " becomes " (c "4/2 = 2/1") ", exactly two with no rounding."))
  (exercise
    (id "rationals:3")
    (prompt "A rational is stored in lowest terms, so its parts come back " (em "reduced") ": " (c "6/8") " is really " (c "3/4") ", so its numerator is " (c "3") ". Which accessor reads the top of the fraction, " (c "numerator") " or " (c "denominator") "? Fill in the blank so the check confirms the numerator is " (c "3") " and gives " (c "true") ".")
    (starter (= (Rational.? (Rational.of 6 8)) (BigInt.of 3)))
    (solution (= (Rational.numerator (Rational.of 6 8)) (BigInt.of 3)))
    (expected "true")
    (hint (c "6/8") " reduces to " (c "3/4") ", whose numerator is " (c "3") ". " (c "Rational.numerator") " reads the top; " (c "Rational.denominator") " would give " (c "4") ". The accessor returns a " (c "BigInt") ", so it's compared against " (cdz (BigInt.of 3)) ".")))
