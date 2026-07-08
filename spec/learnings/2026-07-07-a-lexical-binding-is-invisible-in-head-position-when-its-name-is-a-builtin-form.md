# A lexical binding is invisible in head position when its name is a built-in form

*2026-07-07*

**What happened.** Adversarial probing of name resolution found a wrong-value miscompile:
`(let ((list (fn (a b) (+ a b)))) (list 3 4))` evaluates to `(list 3 4)` — a two-element
built-in list — instead of `7`. The `let` binds `list` to a function, but applying it in head
position ignores the binding entirely and builds the built-in list. The sharpest witness:
`(let ((list 42)) (list 1 2))` also yields `(list 1 2)` — binding `list` to an *integer* does
not even make `(list 1 2)` a type error (applying a non-function); the binding is simply not
consulted. The bug hits `list`, `tuple`, `record`, `map` (the built-in constructor forms). A
non-reserved name is fine: `(let ((mylist (fn (a b) (+ a b)))) (mylist 3 4))` = 7.

**Why it is a break.** core-semantics.md #Binding Is Lexical: "A name MUST resolve to the
nearest enclosing binding of that name." A `let`-bound `list` shadows the built-in constructor
for its scope, so `(list 3 4)` in the body MUST apply the bound function. The compiler resolves
`list` two different ways by syntactic position: a bare reference `(let ((list 99)) list)`
correctly yields `99` (value position consults the environment), but `list` in application-head
position silently prefers the built-in. Resolving one name two ways by position is exactly what
#Binding Is Lexical forbids, and the head-position answer is a wrong *value*, not a decline.

**Root cause — the head dispatch matches the built-in string before the environment.** In the
seed (`codegen.rs::emit`), an application `(head args…)` is dispatched by `match head { … }`
over the head *string*: `"tuple" | "record" | "list" => self.gen_runtime_ctor(…)` fires whenever
the head is literally `"list"`, before the fallthrough `gen_call`/`gen_apply` that would consult
the lexical environment. So a local binding named `list` is unreachable in head position — the
match on the built-in name intercepts it. The const-fold dispatch (`eval_const`) has the same
head-string match. Value position is unaffected because a bare `Node::Name("list")` resolves
through the environment lookup, not the application dispatch.

**The lesson.** Dispatching an application by matching the head *name* against a fixed set of
built-in forms bakes in a resolution order that puts built-ins ahead of lexical bindings —
silently, and only in head position. Lexical scoping requires the environment to be consulted
*first* at every occurrence of a name, head or not; a built-in form is the fallback for a name
with no binding, not an override of one that has a binding. The give-away is a name that
resolves one way as a value and another way as an operator: that asymmetry is the signature of a
syntactic-position dispatch shadowing a lexical one. The fix is to look up the head in the
environment before the built-in match, so a shadowing binding wins (or, for a name a generation
does not realize shadowing, to decline rather than choose the built-in).

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a let binding shadows a
built-in constructor name in application-head position" — `(let ((list (fn (a b) (+ a b)))) (list
3 4))` MUST be `7`. Native seed; the behavior gate catches it (expected output 7, observed
`(list 3 4)`).
