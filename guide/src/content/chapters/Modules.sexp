(chapter
  (slug "modules")
  (title "Modules")
  (pillar "language")
  (section "What makes Cadenza different")
  (blurb "Grouping definitions under a name; a module is a record of its exports.")
  (lede "As a program grows, related definitions want a home. A module groups them under a name, and because a module is just a record of what it defines, you reach inside it the way you reach into any record.")
  (p "A " (c "module") " gathers definitions and binds a name for them. It stands at the top level as a sibling of your other definitions, not something tucked inside a function, and it " (c "export") "s the pieces callers may use. Say we have a temperature conversion; it belongs with other temperature code, under a " (c "Temp") " name, reached by qualifying it: " (c "Temp.c-to-f") ".")
  (runnable
    (source (do
  (module Temp
    (def (c-to-f c) (+ (/ (* c 9) 5) 32))
    (export c-to-f))
  (def (main) (Temp.c-to-f 100))
  (export main))))
  (p "100°C is 212°F. " (c "Temp") " names the group and exports " (c "c-to-f") "; " (c "main") " reaches the conversion with the qualified name " (c "Temp.c-to-f") ". The definition lives in the module's namespace rather than loose in the surrounding scope, the same dotted access you already use for a record's field.")
  (h2 "A module keeps its own pieces together")
  (p "The real value shows once a module has more than one piece. Here " (c "Circle") " holds a constant " (c "pi") " and an " (c "area") " that uses it. The caller only deals with " (c "Circle.area") ", since the " (c "pi") " is an internal detail the module manages for itself:")
  (runnable
    (id "circle-area")
    (source (do
  (module Circle
    (def pi 3)
    (def (area r) (* pi (* r r)))
    (export area))
  (def (main) (Circle.area 10))
  (export main))))
  (p (c "area 10") " is " (c "3 × 10 × 10") " = " (result (of "circle-area") 300) ". The function reads " (c "pi") " directly, because inside the module they're siblings; from outside you just call " (c "area") " and don't think about how it's computed.")
  (h2 "Composing across modules")
  (p "Two modules, each with its own job, combine cleanly, since a qualified name says exactly which piece you mean, so there's never a question of whose " (c "f") " is whose:")
  (runnable
    (id "compose")
    (source (do
  (module Inc (def (f x) (+ x 1)) (export f))
  (module Scale (def (g x) (* x 10)) (export g))
  (def (main) (Scale.g (Inc.f 4)))
  (export main))))
  (p (c "Inc.f 4") " is 5, then " (c "Scale.g 5") " is " (result (of "compose") 50) ". Swap the order to " (cdz (Inc.f (Scale.g 4))) " and you'd get 41 instead, so the qualified names make the pipeline unambiguous either way.")
  (h2 "Modules nest")
  (p "A module can hold another module, so one file can carry a whole tree of scopes, much like a module tree in Rust. You reach through the layers with the same dotted access, one name per level. Here a " (c "Geometry") " module contains a " (c "Square") " module with an " (c "area") ":")
  (runnable
    (id "nested-module")
    (source (do
  (module Geometry
    (module Square
      (def (area s) (* s s))
      (export area))
    (export Square))
  (def (main) (Geometry.Square.area 5))
  (export main))))
  (p (c "Geometry.Square.area 5") " reads left to right, into " (c "Geometry") ", then " (c "Square") ", then " (c "area") ", and gives " (result (of "nested-module") 25) ". It's the same field access as a record inside a record; nesting modules is nothing new, because a module was a record all along.")
  (h2 "Declaring the world a module targets")
  (p "A module that compiles to a WebAssembly component targets a " (em "WIT world") ": the set of interfaces it " (c "import") "s from its host and " (c "export") "s back, each with typed members. You can name that world inline, right in the source, with a " (c "world") " declaration, so the compile target is self-contained and reads the same as the WIT world it corresponds to. Here a " (c "Reducer") " world exports a " (c "fold") " interface (the guest provides " (c "apply") ") and imports a " (c "kv") " interface (the host provides " (c "get") " and " (c "put") "):")
  (note "world Reducer = " (br) " &nbsp;&nbsp;| export fold = " (br) " &nbsp;&nbsp;&nbsp;&nbsp;| apply : (event : Bytes) -&gt; Bytes " (br) " &nbsp;&nbsp;| import kv = " (br) " &nbsp;&nbsp;&nbsp;&nbsp;| get : (key : String) -&gt; Bytes " (br) " &nbsp;&nbsp;&nbsp;&nbsp;| put : (key : String, value : Bytes) -&gt; Unit")
  (p "Read it top-down: " (c "world Name =") " heads the declaration, then one or more bar-led interfaces; each is " (c "import") " or " (c "export") " followed by the interface name and one or more bar-led members; each member is " (c "name : (param : Type, ...) -&gt; ResultType") ". An " (c "export") " is what the guest provides (here the reducer's " (c "apply") "); an " (c "import") " is what the host provides and the world therefore depends on (the key-value store). The vocabulary is kept deliberately close to WIT, so the correspondence to the world the component targets is meant to be obvious on sight.")
  (p "Two small rules are worth stating. A member's result is " (em "always") " present, so an operation that returns nothing writes " (c "-&gt; Unit") " rather than omitting the arrow; and a member with no parameters elides the parameter list, as in " (c "now : () -&gt; Timestamp") ". The " (c "world") " keyword is also " (em "contextual") ": it only heads a declaration in the " (c "world Name =") " position, so " (c "world") " stays an ordinary name everywhere else, and " (c "(let ((world 5)) (+ world 1))") " still reads " (c "world") " as a plain variable.")
  (why (tenet "One world, however you spell it") "The inline " (c "world") " declaration isn't a second, parallel notion of a compile target. It names the " (em "same") " WIT world a component targets, and it lowers to the very same internal world description the compiler would build from an external world artifact, so an inline declaration and a separate artifact are interchangeable inputs to the same compile. One concept, two spellings: you reach for the inline form when you want the target to be self-evident in the source, and neither spelling teaches the compiler anything the other couldn't.")
  (p "Two rules keep the declaration unambiguous as a compile target. First, an " (em "external") " world artifact wins: if a component is compiled with a separate WIT world " (em "and") " the source also carries an inline " (c "world") " declaration, the external artifact overrides the in-source one, the same way a bound effect request overrides the source's own declaration. Second, a module may name " (em "at most one") " world, because a component targets exactly one: two top-level " (c "world") " declarations are rejected outright.")
  (runnable
    (source (world Reducer (export fold (member apply (func (param event Bytes) (result Bytes)))))
(world Other (export fold (member apply (func (param event Bytes) (result Bytes))))))
    (expect "error")
    (wrap "false"))
  (note "The compiler parses a " (c "world") " declaration, prints it back identically, and lowers it to the same canonical world description it would build from an external artifact, so a top-level " (c "world") " declaration now drives a component's emit directly. The " (link (slug "writing-a-reducer") " reducer ") " chapter puts a " (c "world Reducer") " to work as the target of a real fold.")
  (why (tenet "A module is a record of its exports") "Cadenza doesn't bolt on a separate \"module system\" with its own rules, since a module is just a " (em "value") ", a record whose fields are its definitions, bound to a name. That's why you reach into it with the same " (c ".") " you use for any record: there's one idea of \"a named thing with fields\", not two. Grouping and namespacing fall out of a feature the language already has, so everything you know about records, how they're built, accessed, and passed around, already tells you how modules behave.")
  (note "This is scoping " (em "within") " one program. A larger project also splits across files, where a module " (c "import") "s the names another " (c "export") "s, the same grouping idea, scaled up to a whole package.")
  (h2 "Your turn")
  (exercise
    (id "modules:1")
    (prompt "Two modules, each with one job: " (c "Money.cents") " turns dollars into cents (×100), and " (c "Tax.add") " adds " (c "5") ". Compose them by feeding " (c "2") " dollars through both, qualifying the inner call with the right module name, so the answer is " (c "205") ".")
    (starter (do
  (module Money (def (cents d) (* d 100)) (export cents))
  (module Tax   (def (add c) (+ c 5)) (export add))
  (def (main) (Tax.add (?.cents 2)))
  (export main)))
    (solution (do
  (module Money (def (cents d) (* d 100)) (export cents))
  (module Tax   (def (add c) (+ c 5)) (export add))
  (def (main) (Tax.add (Money.cents 2)))
  (export main)))
    (expected "205")
    (hint (c "cents") " lives in " (c "Money") ", so the qualified name is " (c "Money.cents") ". Then " (c "2 × 100 = 200") ", and " (c "Tax.add") " makes it " (c "205") ". (Qualify it with " (c "Tax") " and the compiler declines, since " (c "Tax") " has no " (c "cents") ".)"))
  (exercise
    (id "modules:2")
    (prompt (c "f") " lives inside " (c "Double") ", which lives inside " (c "Mathy") ". Write the qualified path to call it on " (c "8") ", so doubling gives " (c "16") ".")
    (starter (do
  (module Mathy
    (module Double
      (def (f x) (* x 2))
      (export f))
    (export Double))
  (def (main) (?.f 8))
  (export main)))
    (solution (do
  (module Mathy
    (module Double
      (def (f x) (* x 2))
      (export f))
    (export Double))
  (def (main) (Mathy.Double.f 8))
  (export main)))
    (expected "16")
    (hint "Name each level from the outside in, separated by dots: " (c "Mathy.Double.f") ". Then " (c "8 × 2 = 16") ".")))
