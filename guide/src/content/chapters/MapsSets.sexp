(chapter
  (slug "maps-sets")
  (title "Maps & sets")
  (pillar "language")
  (section "Fundamentals")
  (blurb "Membership and key→value association, without duplicates.")
  (lede "A list keeps things in order. When you care about " (em "membership") " or a " (em "key→value") " association instead, reach for a set or a map.")
  (p "Like lists, both are immutable, persistent values: every \"insert\" or \"remove\" returns a new collection and leaves the original alone. They just answer a different question: a list is about position, a set is about \"is this in here?\", a map is about \"what's stored under this key?\".")
  (h2 "Sets: membership, without duplicates")
  (p "Build one from a list with " (c "Set.of") ". A set collapses duplicates and forgets order; return one and you'll see the collapse directly: the two " (c "2") "s in the input become a single " (c "2") ", so the set holds " (c "1 2 3") ":")
  (runnable
    (source (Set.of #list(1 2 2 3))))
  (p (c "Set.contains") " answers membership directly:")
  (runnable
    (source (Set.contains (Set.of #list(1 2 3)) 2)))
  (h2 "Set algebra")
  (p "Sets combine the way they do in maths: " (c "Set.union") " (in either), " (c "Set.intersection") " (in both), " (c "Set.difference") " (in the first but not the second). Each returns a new set; Run it and you see the set itself:")
  (runnable
    (source (Set.union (Set.of #list(1 2)) (Set.of #list(2 3 4)))))
  (p (c "{1,2}") " ∪ " (c "{2,3,4}") " = " (c "{1,2,3,4}") ": the duplicate " (c "2") " collapses, so four distinct elements. Put " (c "Set.intersection") " in its place and Run again: only the shared " (c "2") " survives, so you get " (c "{2}") ".")
  (p (c "Set.difference") " is the one where " (em "order matters") ": it keeps what's in the first set and not the second. Return the set itself and you can see it: " (c "{1,2,3}") " minus " (c "{2,3,4}") " leaves just " (c "{1}") ":")
  (runnable
    (source (Set.difference (Set.of #list(1 2 3)) (Set.of #list(2 3 4)))))
  (p "Swap the two sets and Run again: " (c "{2,3,4}") " minus " (c "{1,2,3}") " is " (c "{4}") " instead, a different answer, because \"in the first but not the second\" isn't symmetric. Union and intersection don't care which side is which; difference does.")
  (h2 "Maps: values under keys")
  (p "A map starts empty with " (c "Map.empty") " and grows with " (c "Map.insert") ". " (c "Map.len") " reports how many keys it holds:")
  (runnable
    (source (def (main)
  (Map.len (Map.insert (Map.insert (Map.empty) 1 10) 2 20)))))
  (p (c "Map.lookup") " is what you reach for a map to do, and like reaching into a list, it can miss. So it returns an " (c "Option") ": " (cdz (Some v)) " when the key is present, " (cdz (None unit)) " when it isn't. You take it apart with " (c "match") ":")
  (runnable
    (source (def (main)
  (match (Map.lookup (Map.insert (Map.empty) 7 99) 7)
    ((Some v) v)
    ((None _) 0)))))
  (p "Look up a key that isn't there (change the second " (c "7") " to " (c "8") ") and the " (c "None") " arm gives " (c "0") ": no crash, just \"nothing under that key\".")
  (h2 "Literal shorthand")
  (p "Writing " (c "Set.of") " over a list, or a chain of " (c "Map.insert") "s, gets wordy. The conventional surface has a shorthand for each, rounding out the " (c "#") "-prefixed literal family you've been seeing: " (c "[…]") " is a list, " (c "#(…)") " is a " (em "set") ", and " (c "#{…}") " is a " (em "map") ". They're pure sugar (the same programs underneath), so this set literal is exactly the " (c "Set.of") " call from the top of the chapter, and it still collapses the duplicate to the set " (c "{1, 2, 3}") ":")
  (runnable
    (source (Set.of #list(1 2 2 3))))
  (p "Toggle to the conventional surface and that reads " (c "#(1, 2, 2, 3)") ", and in fact every " (c "Set.of #list(…)") " earlier in this chapter has been showing as " (c "#(…)") " whenever the toggle was on. A map literal spells each entry " (c "key = value") " inside " (c "#{…}") "; here two entries, and returning it shows both:")
  (runnable
    (source #map((= 1 10) (= 2 20))))
  (p "And it's an ordinary map, so " (c "Map.lookup") " works on it just the same: the value under key " (c "2") " is " (c "20") ":")
  (runnable
    (source (def (main)
  (Option.expect (Map.lookup #map((= 1 10) (= 2 20)) 2) "missing"))))
  (note "These are the same three collections, not new ones: " (c "#(…)") " desugars to " (c "Set.of") " and " (c "#{…}") " to a map, exactly the forms above. Reach for the literal when you're writing a collection out by hand; reach for " (c "Set.of") " / " (c "Map.insert") " when you're building one from values you already have.")
  (why (tenet "One question per collection") "List, set, map: three shapes for three questions: order, membership, association. Picking the right one puts your intent in the type, and lets the compiler pick an efficient representation (a hash trie for a map or set; an array or tree for a list) without you managing it. And all three share the same discipline as the rest of the language: they're immutable, so an \"update\" is a new value; a lookup that can miss returns an " (c "Option") " rather than a crash or a bogus default.")
  (h2 "Inserting over a key replaces it")
  (p "A map holds one value per key, so inserting the same key again replaces the old value, and the size doesn't grow:")
  (runnable
    (source (def (main)
  (Option.expect
    (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 1 99) 1)
    "missing"))))
  (p "Two inserts at key " (c "1") ", and the second one wins: " (c "99") ".")
  (note "Keys and elements are compared by value, using the same structural equality as everywhere else: two equal keys " (em "are") " the same key, whatever built them. That's why " (c "Set.of") " can collapse duplicates and a re-insert can replace: equality is a property of values, not of identity.")
  (h2 "Removing and reporting in one step: " (c "Map.take"))
  (p (c "Map.remove") " discards whatever was under the key. When you want to " (em "see") " it on the way out, " (c "Map.take") " does both at once: it returns a tuple of the value that was there (as an " (c "Option") ", since the key might be absent) and the new map with the key gone. Reach the dropped value with " (c ".0") " and " (c "match") " it; here taking key " (c "1") " from a two-entry map reports the " (c "10") " it held:")
  (runnable
    (source (def (main)
  (match (. (Map.take #map((= 1 10) (= 2 20)) 1) 0)
    ((Some v) v)
    ((None _) -1)))))
  (p "The other half of the tuple, " (c ".1") ", is the smaller map with one entry left. Return it and you can see the removal: " (c "{2 = 20}") ", with key " (c "1") " gone:")
  (runnable
    (source (def (main)
  (. (Map.take #map((= 1 10) (= 2 20)) 1) 1))))
  (p "Take a key that isn't there and " (c ".0") " is " (cdz (None unit)) " while " (c ".1") " equals the original: removal stays total, and you learn it held nothing in the same step.")
  (p (c "Map.insert") " has the same value-yielding twin, " (c "Map.swap") ": it inserts (or replaces) and reports what the key held " (em "before") ", again as a " (c "(prior-value . new-map)") " tuple. So swapping key " (c "1") " (already " (c "10") ") for " (c "99") " hands back the old " (c "10") " in " (c ".0") ", no separate lookup needed:")
  (runnable
    (source (def (main)
  (match (. (Map.swap #map((= 1 10)) 1 99) 0)
    ((Some old) old)
    ((None _) -1)))))
  (p "Swap a key that's new and " (c ".0") " is " (cdz (None unit)) ": nothing was replaced. Between them, " (c "take") " reports what a remove " (em "dropped") " and " (c "swap") " what an insert " (em "overwrote") ", each in a single step.")
  (p "Numbers, symbols, lists, maps: all collections of values. Text is its own thing, with its own honest questions (how long " (em "is") " a string?). " (em "Strings &amp; text") ", next.")
  (h2 "Your turn")
  (exercise
    (id "maps-sets:1")
    (prompt "How many elements are in " (em "both") " " (c "{1,2,3}") " and " (c "{2,3,4}") "? Use " (c "Set.intersection") " and count the result: the shared elements are " (c "2") " and " (c "3") ", so the answer is " (c "2") ".")
    (starter (Set.len
  (Set.? (Set.of #list(1 2 3)) (Set.of #list(2 3 4)))))
    (solution (Set.len
  (Set.intersection (Set.of #list(1 2 3)) (Set.of #list(2 3 4)))))
    (expected "2")
    (hint "\"In both\" is the intersection, " (c "Set.intersection") ". Only " (c "2") " and " (c "3") " appear in each set."))
  (exercise
    (id "maps-sets:2")
    (prompt "This map literal holds two keys, " (c "1") " and " (c "2") ". Every \"update\" is a new map, so removing a key builds one without it, which is what " (c "Map.remove") " does. Take one key away, then ask " (c "Map.len") " how many remain: the answer should be " (c "1") ". Fill in the operation.")
    (starter (def (main)
  (Map.len
    (Map.? #map((= 1 10) (= 2 20)) 1))))
    (solution (def (main)
  (Map.len
    (Map.remove #map((= 1 10) (= 2 20)) 1))))
    (expected "1")
    (hint "The op that deletes a key is " (c "Map.remove") "; it takes the map and the key. Two keys minus one leaves " (c "1") ", and the original map, as ever, is untouched.")))
