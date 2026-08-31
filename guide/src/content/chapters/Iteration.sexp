(chapter
  (slug "iteration")
  (title "Iteration without loops")
  (pillar "language")
  (section "Fundamentals")
  (blurb "Cadenza has no for or while; you repeat work with recursion and the fold family. Here's how, and why.")
  (lede "Coming from most languages, you would reach for a " (c "for") " or a " (c "while") " here. Cadenza has neither. This chapter is how you repeat work instead, and why the language leaves loops out.")
  (p "There is no loop keyword in Cadenza. The full set of keywords is " (c "def") ", " (c "do") ", " (c "effect") ", " (c "else") ", " (c "export") ", " (c "handle") ", " (c "if") ", " (c "import") ", " (c "in") ", " (c "let") ", " (c "match") ", " (c "module") ", " (c "return") ", " (c "then") ", and " (c "type") ", with no " (c "for") ", no " (c "while") ", and no " (c "loop") ". Repetition is done with " (em "functions that call themselves") ": recursion. That sounds like a limitation. It removes a whole class of bugs.")
  (h2 "Why no loops")
  (p "A loop is a " (em "statement") ": it runs for its side effects, mutating a counter and an accumulator until a condition flips. That mutable state is where off-by-one errors, uninitialized accumulators, and forgotten updates live. Cadenza is expression-oriented (everything computes a " (em "value") "), so repetition computes a value too. There is no loop variable to misinitialize and no " (c "i++") " to forget, because there is no mutable loop state at all: each step is a fresh function call with its arguments spelled out.")
  (h2 "The mechanism: a recursive accumulator")
  (p "The workhorse pattern is a function that carries the answer-so-far in an argument, the " (em "accumulator") ", and calls itself with an updated one. It needs two things: a " (strong "base case") " that stops the recursion and returns the accumulator, and a " (strong "recursive case") " that does one step and recurses on the rest. Here is a sum from " (c "n") " down to " (c "1") ":")
  (runnable
    (source (def (main) (sum-to 5 0))
(def (sum-to n acc)
  (if (= n 0)
    acc
    (sum-to (- n 1) (+ acc n))))))
  (p "Read it as a loop turned inside out: " (c "acc") " is the running total, " (c "n") " counts down, the " (cdz "(= n 0)") " check is the exit condition, and each call adds " (c "n") " to " (c "acc") " and continues. When " (c "n") " reaches " (c "0") " the base case hands back the total, " (c "15") ". Nothing mutates; each call just receives the next pair of values.")
  (p "The same shape works over a list. Match the list by its structure, either the empty list " (cdz "#list()") " or a non-empty " (cdz "#list(x .. rest)") " that binds the first element to " (c "x") " and the remainder to " (c "rest") ", and thread the accumulator through:")
  (runnable
    (source (def (main) (sum-list #list(10 20 30) 0))
(def (sum-list xs acc)
  (match xs
    (#list() acc)
    (#list(x .. rest) (sum-list rest (+ acc x)))))))
  (p "The empty list is the base case (return the accumulator); the non-empty case adds the head to the accumulator and recurses on the tail. Building a value instead of a number is the identical move. Here it reverses a list by taking each element off the front and putting it on the " (em "front") " of the accumulator, so the first element read ends up deepest and the last read ends up first:")
  (runnable
    (source (def (main) (rev #list(1 2 3) #list()))
(def (rev xs acc)
  (match xs
    (#list() acc)
    (#list(x .. rest) (rev rest (List.prepend acc x)))))))
  (p "Prepending is what does the reversing: element " (c "1") " is placed first, then " (c "2") " goes in front of it, then " (c "3") " in front of that, so " (cdz "#list(1 2 3)") " comes back as " (cdz "#list(3 2 1)") ". " (link (slug "lists") " " (c "List.prepend")) " adds an element to the front, which is what flips the order; appending each element to the end with " (c "List.push") " would instead copy the list unchanged. A quick " (c "@test") " pins it, reading the three positions of the result back and checking they spell " (c "3") ", " (c "2") ", " (c "1") " (as the single number " (c "321") "):")
  (runnable
    (source (def (rev xs acc)
  (match xs
    (#list() acc)
    (#list(x .. rest) (rev rest (List.prepend acc x)))))
(def (nth xs i) (match (List.at xs i) ((Some v) v) ((None _) 0)))
(@ test (def (rev-reverses)
  (let ((r (rev #list(1 2 3) #list())))
    (assert-eq (+ (* 100 (nth r 0)) (+ (* 10 (nth r 1)) (nth r 2))) 321
      "rev of (1 2 3) should read back as 3,2,1")))))
    (mode "test"))
  (note "Notice the recursive call is the " (em "last") " thing each step does: it sits in " (em "tail position") ". A recursion in tail position compiles to a loop. It reuses one stack frame rather than stacking a new one per element, so an accumulator over a long list runs in constant stack space. Threading the accumulator is what puts the call in tail position; a version that adds " (em "after") " the recursive call (" (cdz "(+ x (sum rest))") ") does not, and you meet exactly that shape in the next chapter.")
  (why (tenet "Repetition is a value, not a statement") "A loop mutates state for effect; a recursive function " (em "returns") " the result of repeating. Making iteration an expression means every repetition has a value and a type, there is no mutable loop counter to get wrong, and the same tool, a function, does the job with no special loop syntax to learn. Uniformity over special cases, applied to the oldest control structure there is.")
  (p "You will rarely write the accumulator by hand for long. The " (em "fold") " family packages exactly this pattern (a base value and a step that combines the running result with each element), so you state the step and let the traversal disappear. The next chapter, " (link (slug "lists") "Lists") ", puts recursion to work over sequences, and " (link (slug "iterators") "Iterators") " adds a lazy, on-demand layer on top: the fold vocabulary (" (c "map") ", " (c "filter") ", " (c "fold") ") built from the very mechanism you just saw."))
