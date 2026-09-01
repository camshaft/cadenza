(chapter
  (slug "symbols")
  (title "Symbols")
  (pillar "language")
  (section "Fundamentals")
  (blurb "Interned names, compared by identity.")
  (lede "Sometimes a value is just a " (em "name") ", like a status, a mode, or one choice from a fixed set. A symbol is exactly that: an interned name you compare by identity.")
  (p "A symbol is written " (c "#\"…\"") ", where the " (c "#") " tells it apart from a text string. Where a string is " (em "content") " you might slice, join, or measure, a symbol is a bare label whose only question is \"is it this one?\". So the operation on symbols is equality:")
  (runnable
    (source (= #"red" #"red")))
  (p "Two symbols are equal exactly when they're spelled the same, and no matter how long the name, the comparison is a single identity check, not a character-by-character scan.")
  (note "In the conventional surface the quotes are just noise when the name is a plain identifier, so " (c "#\"red\"") " may be written " (c "#red") ", and the two are the same symbol. The quotes are only needed when the content isn't an identifier: a name with a space, a leading digit, or a dot ( " (c "#\"List.at\"") ") keeps them. Toggle the syntax and a snippet's " (c "#red") " reappears as " (c "#\"red\"") " in the s-expression surface, where the quoted form is canonical.")
  (h2 "One choice from a fixed set")
  (p "That's what symbols are for: a value drawn from a small, known set of names. A traffic light is " (cdz #"red") ", " (cdz #"yellow") ", or " (cdz #"green") ", and a function can decide on it, here choosing how many seconds to wait:")
  (runnable
    (source (def (wait light)
  (if (= light #"red") 30
    (if (= light #"yellow") 5
      0)))
(def (main) (wait #"red"))))
  (p (cdz #"red") " waits 30, " (cdz #"yellow") " 5, and anything else (green) 0. The light is passed around as a plain value and matched by name where the decision is made, with no numbers to remember and no strings to keep in sync.")
  (why (tenet "A name compared by identity, not by its text") "Why a distinct type, rather than just a string like " (c "\"red\"") "? Because the intent differs. A string is text you might transform; a symbol is a fixed label whose only meaning is " (em "which label it is") ". Making it its own type says so, so you can't accidentally slice a status tag or take its length, and lets the compiler treat it as an interned identity (one cheap comparison, whatever the name's length) instead of a sequence to scan. Same instinct as keeping " (c "Bytes") " apart from " (c "String") ": one type per kind of thing, so the compiler catches a category mistake.")
  (h2 "From a string, explicitly")
  (p "When a name arrives as text, whether parsed from input or assembled at run time, " (c "Symbol.of") " interns it into a symbol. The result is the very same value as writing the literal: a symbol built from the pieces " (c "\"ye\"") " and " (c "\"s\"") " equals " (cdz #"yes") ":")
  (runnable
    (source (= (Symbol.of (String.concat "ye" "s")) #"yes")))
  (note "Text-to-symbol is an explicit step (" (c "Symbol.of") "), just like " (c "String.to-bytes") ", so the one place you cross between the two types is spelled out, and a symbol and a string never silently stand in for each other.")
  (p "That's the last of the everyday value shapes: numbers, text, bytes, symbols, and the collections that hold them. Now we look harder at " (em "one") " of them: how Cadenza models numbers, and why it refuses to convert them behind your back. " (em "The numeric model") ", next.")
  (h2 "Your turn")
  (exercise
    (id "symbols:1")
    (prompt (c "score") " dispatches on a medal from the fixed set " (cdz #"gold") " / " (cdz #"silver") " / " (cdz #"bronze") ": gold scores " (c "3") ", silver " (c "2") ", anything else " (c "1") ". The gold and fallback arms are done, so fill the middle comparison to make " (cdz (score #"silver")) " give " (c "2") ".")
    (starter (def (score m)
  (if (= m #"gold") 3
    (if (= m ?) 2 1)))
(def (main) (score #"silver")))
    (solution (def (score m)
  (if (= m #"gold") 3
    (if (= m #"silver") 2 1)))
(def (main) (score #"silver")))
    (expected "2")
    (hint "The middle arm handles silver, so compare " (c "m") " against " (cdz #"silver") ". Each symbol is checked by equality; " (cdz #"bronze") " matches neither and falls through to " (c "1") "."))
  (exercise
    (id "symbols:2")
    (prompt "A symbol is an ordinary value, so a function can " (em "return") " one, not just test it. " (c "next") " advances a traffic light around its cycle: red turns to green, green to yellow, yellow back to red. The green and yellow cases are written, so fill the hole with the symbol red becomes, making " (cdz (next #"red")) " return " (cdz #"green") " and the check give " (c "true") ".")
    (starter (def (next light)
  (if (= light #"red") ?
    (if (= light #"green") #"yellow"
      #"red")))
(def (main) (= (next #"red") #"green")))
    (solution (def (next light)
  (if (= light #"red") #"green"
    (if (= light #"green") #"yellow"
      #"red")))
(def (main) (= (next #"red") #"green")))
    (expected "true")
    (hint "The hole is the value the function " (em "hands back") " for red, a symbol literal, " (cdz #"green") ". The result is a symbol like any other, which the check then compares against " (cdz #"green") ".")))
