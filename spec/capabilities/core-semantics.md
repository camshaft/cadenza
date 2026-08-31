# Capability — Core Semantics

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines evaluation, binding, scope, control flow, pattern matching, failure and
> termination, equality and ordering, and the observable-behavior projection, and binds their
> behavior to the single executable-semantics corpus. Requirements realize
> [Core Principle III](../../constitution.md), [Core Principle IX](../../constitution.md), and
> [Core Principle XIV](../../constitution.md) and trace to [overview §3](../overview.md),
> [overview §10](../overview.md), and [overview §11](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes the invariants of evaluating a Cadenza program: how expressions reduce to
values, how names bind, how scope works, how control flow behaves, and what pattern matching
guarantees. It states the invariants; the concrete, case-by-case behavior of every construct is the
executable-semantics corpus under [`spec/semantics/`](../semantics/), which is this capability's
single source of truth. Where an invariant here and a corpus case appear to disagree, the corpus is
authoritative and the invariant is corrected to match.

## Evaluation

### Evaluation Is Deferred To The Corpus

The observable behavior of every language construct MUST match the construct's case in the executable-semantics corpus.

The compiler MUST NOT implement a construct's behavior in a way that disagrees with the corpus.

### Evaluation Is Deterministic Given Its Inputs And Capabilities

Evaluation of an expression MUST depend only on the expression, the bindings in scope, and the responses to the capabilities the expression invokes.

Evaluation MUST NOT depend on any outside influence the expression did not obtain through a binding in scope or a declared capability.

Evaluation of an expression MUST NOT observe an order among independent subexpressions beyond the order their data dependencies impose.

## Binding And Scope

### Binding Is Lexical

A name MUST resolve to the nearest enclosing binding of that name.

A reference to a name with no enclosing binding MUST be a compile-time error.

### Shadowing Is Well-Defined

A binding that shadows an outer binding of the same name MUST take effect for references in its scope as defined by the corpus.

### The Bindings Of One `let` Take Effect In Order

The bindings of a single `let` MUST take effect in the order they are written: each binding's initializer MUST observe the bindings written before it in the same `let`, and MUST NOT observe the bindings written after it.

A binding whose name repeats an earlier binding in the same `let` MUST shadow the earlier one for the initializers and body that follow it, in accordance with §"Shadowing Is Well-Defined".

## Functions

### A Function Is A First-Class Value

A function MUST be a value that can be bound to a name, passed as an argument, returned as a result, and stored in a data structure, like any other value.

A function value MUST capture the bindings in scope at the point it is created, so that applying it later observes those captured bindings rather than the bindings in scope at the point of application.

### Functions Are Single-Arity

A function MUST take exactly one argument and return exactly one value.

Multi-parameter syntax `(fn (x y) body)` MUST desugar to curried form `(fn x (fn y body))`.

Multi-argument application `(f a b)` MUST desugar to curried application `((f a) b)`.

Partial application MUST be natural: applying a curried function to fewer arguments than its full chain returns a closure awaiting the remaining arguments.

### Applying A Function Binds Its Parameter To Its Argument

Applying a function to its argument MUST evaluate the function body in an environment that extends the function's captured environment with its parameter bound to the argument.

## Control Flow

### Conditionals Evaluate One Branch

A conditional MUST evaluate only the branch its condition selects.

Every branch of a conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated branch cannot carry a deferred error.

### Boolean Connectives Short-Circuit

The language MUST offer a logical conjunction, a logical disjunction, and a logical negation over boolean values, so that a program composes conditions without nesting a conditional per condition.

A logical conjunction MUST evaluate its right operand only when its left operand is true, and a logical disjunction MUST evaluate its right operand only when its left operand is false, so that a connective shields a trapping or effectful right operand exactly as the unselected branch of a conditional does.

Each operand of a boolean connective MUST be type-checked as a boolean whether or not it is evaluated, so that an unevaluated operand cannot carry a deferred error, exactly as every branch of a conditional is type-checked.

## Sequencing

### A Sequencing Block Evaluates Its Forms In Order

A sequencing block MUST evaluate each of its forms in the order they are written.

A sequencing block MUST evaluate to the value of its last form.

A host call a form in a sequencing block makes MUST be observed before a host call made by a later form in the same block.

### A Discarded Pure Non-Final Value Is Diagnosed

Because a non-final form of a sequencing block is evaluated only for its effect — its value is discarded — a non-final form that is pure (it reaches no host call) and yields a value of a type other than unit computes a value that can never affect the program's observable behavior, which is far more likely a program defect than an intent. An implementation SHOULD emit a diagnostic of non-error severity — one that leaves the build successful — for such a form, so that a program does not silently discard the value of a pure computation whose result it never observes.

### A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It

A declaration form in a sequencing block MUST bind its name for the forms that follow it in that block, so that a name a declaration introduces is in scope without a separate binding form.

## Pattern Matching

### Matching Is Exhaustive Or Rejected

A match whose patterns do not cover every value of the scrutinee's type MUST be a compile-time error.

A match MUST evaluate the branch of the first pattern that matches the scrutinee, as defined by the corpus.

### Bindings Introduced By A Pattern Are Scoped To Its Branch

A name a pattern binds MUST be in scope only in the branch guarded by that pattern.

A pattern MUST bind each name at most once; a pattern that binds the same name more than once MUST be a compile-time error (`CDZ0102`), so that a pattern is linear rather than silently shadowing an earlier binder or imposing a hidden equality constraint.

### Patterns Compose

A pattern MUST admit any pattern in each of its binder positions, so that a constructor pattern's binder and a tuple pattern's element MAY themselves be a wildcard, a name, a tuple pattern, or a constructor pattern, matched recursively to any depth.

A composed pattern MUST bind the union of its sub-patterns' bindings, matched recursively, and MUST remain linear across the whole pattern, so that a name appearing in more than one sub-pattern is the same `CDZ0102` error as one appearing twice in a flat pattern.

A destructuring of a tagged value carrying a tuple of sub-values in a single arm — the shape every tree-walking pass over a recursive sum takes — MUST therefore be expressible directly as one nested pattern rather than requiring a bind-then-rematch.

### A Binding Position Accepts An Irrefutable Pattern

A binding position — a `let` binder, a function or `fn` parameter — MUST accept an irrefutable pattern in place of a bare name, binding the names the pattern introduces to the corresponding sub-values of the bound value, exactly as the same pattern would in a single match arm over that value. A bare name and a wildcard are the trivial irrefutable patterns; a tuple pattern whose every element is itself irrefutable is irrefutable, matched recursively to any depth in the sense of *Patterns Compose*. A destructuring parameter MUST NOT change the function's arity — the parameter occupies one argument position and names its parts, so `(def (f (tuple a b)) …)` remains a single-argument function.

A binding position has no alternative arm, so its pattern MUST be irrefutable — it MUST match every value of the bound value's type. A refutable pattern in a binding position — a constructor pattern of a multi-variant sum, a literal, or a length-constrained list pattern, none of which matches every value of its type — MUST be a compile-time error (`CDZ0210`), the same non-exhaustiveness the equivalent single-arm match would raise under *Matching Is Exhaustive Or Rejected*. A pattern whose shape cannot match the bound value's type at all — a tuple pattern of the wrong arity, or a tuple pattern against a non-tuple value — MUST be a compile-time error (`CDZ0201`), and a non-linear binding pattern MUST be the same `CDZ0102` error as in any other pattern position.

A rest binder — the `..`-prefixed tail that binds the residual of a tuple, record, list, or set pattern — is itself irrefutable, because it binds whatever remains and so matches unconditionally; whether a pattern ending in a rest binder is irrefutable as a whole MUST be decided by the same rule as any other binding pattern, namely whether it matches every value of the bound value's type, not by the presence of the rest binder. A tuple or record binding pattern whose named sub-patterns are each irrefutable and which ends in a rest binder MUST be irrefutable and MUST be accepted in a binding position — binding its named parts and binding the rest binder to the residual tuple or record — because a tuple's arity and a record's field set are fixed by the bound value's type, so such a pattern matches every value of that type, exactly as the same pattern binds in a single match arm; it MUST NOT be rejected as an unrecognized or ill-shaped pattern (`CDZ0201`) nor as refutable (`CDZ0210`), the rest binder absorbing the unnamed positions so that no arity or field mismatch arises. A pattern that additionally tests presence remains refutable and MUST be the `CDZ0210` error above regardless of its rest binder — a map binding pattern or a set binding pattern that names any key or element tests runtime presence, and a list pattern with one or more leading element patterns before the rest does not match the empty list — because none of these matches every value of its type.

A binding pattern MAY carry a type annotation `(: <pattern> <Type>)`, which constrains the bound value's type while the inner pattern binds its names, in accordance with *Annotations Constrain, Never Contradict* (`type-system.md`): the annotation participates in inference as an added constraint, and a value whose type cannot satisfy it MUST be a compile-time error (`CDZ0203`), exactly as a value annotation `(: <expression> <Type>)` is.

### A List Is Deconstructed By Element Patterns With An Optional Rest

A list MUST be matchable by an element pattern that names some number of leading elements positionally and MAY end in a rest binder for the remaining elements.

An element pattern naming exactly `n` elements with no rest binder MUST match a list of length exactly `n`, binding each named element position to the corresponding element; a list of any other length MUST NOT match that pattern. An element pattern naming `n` leading elements followed by a rest binder MUST match any list of length at least `n`, binding the leading positions to the first `n` elements and the rest binder to a list of the remaining elements in order. In particular, the empty element pattern MUST match exactly the empty list, and a single-leading-element-plus-rest pattern MUST match any non-empty list, binding its first element and the rest of the list.

Each element position and the rest binder MUST be a binder position in the sense of *Patterns Compose*, so an element MAY itself be any pattern (a wildcard, a name, a tuple pattern, a constructor pattern, or a nested element pattern) matched recursively, and the whole pattern MUST remain linear (`CDZ0102`). The rest binder MUST bind a value of the same list type as the scrutinee, so a recursive function MAY match it again.

A set of list-element arms MUST be treated as exhaustive when it covers both the empty list and every non-empty list — for example an empty-list arm together with a leading-element-plus-rest arm, or an arm ending in a rest binder that names no leading elements — and a set of arms that leaves some length uncovered MUST be a compile-time error under *Matching Is Exhaustive Or Rejected* unless a later arm (a name or wildcard pattern) covers the remainder.

An element pattern MUST observe a list only through its length and its elements in order; it MUST NOT expose or depend on any internal cell or node structure of the list's representation, so that the same pattern matches a list regardless of how the list is represented.

### A Map Is Matched By Key-Directed Patterns

A map MUST be matchable by a key-directed pattern that names some number of keys, each with a value binder position, and MAY end in a rest binder for the remaining entries. A map's key set is runtime data, not a static shape, so a map pattern is a QUERY on the presence of specific keys rather than a structural decomposition of a fixed layout.

A key-directed pattern naming keys `k₁ … kₙ` MUST match a map that CONTAINS every named key, binding each key's value binder to the value the map associates with that key; a map lacking any named key MUST NOT match that pattern, so that matching falls through to a later arm. Each named key MUST be an ordinary value expression, compared to the map's keys by the same value equality the map itself uses, so that a key computed at run time selects an entry exactly as a constant key does.

Each value binder position MUST be a binder position in the sense of *Patterns Compose*, so a value MAY be bound by any pattern (a wildcard, a name, a tuple pattern, a constructor pattern) matched recursively against the value at that key, and the whole pattern MUST remain linear (`CDZ0102`). A pattern MAY end in a rest binder that binds a map of the same type containing every entry of the matched map EXCEPT the named keys, so that the named entries are consumed and the remainder is available for further matching.

Because a map's key set is unbounded, no finite set of key-directed patterns can cover every map, so a match on a map MUST end in a name or wildcard pattern that binds the whole map; a set of key-directed arms with no such catch-all MUST be a compile-time error under *Matching Is Exhaustive Or Rejected*.

A key-directed pattern MUST observe a map only through the presence of keys and the values it associates with them; it MUST NOT expose or depend on any internal ordering or node structure of the map's representation, so that the same pattern matches a map regardless of how the map is represented.

### A Set Is Matched By Element-Membership Patterns

A set MUST be matchable by a membership pattern that names some number of elements and MAY end in a rest binder for the remaining elements. A set's element set is runtime data, not a static shape, so a set pattern is a QUERY on the presence of specific elements rather than a structural decomposition of a fixed layout.

A membership pattern naming elements `e₁ … eₙ` MUST match a set that CONTAINS every named element; a set lacking any named element MUST NOT match that pattern, so that matching falls through to a later arm. Each named element MUST be an ordinary value expression, compared to the set's elements by the same value equality the set itself uses, so that an element computed at run time selects a member exactly as a constant element does. A named element position is NOT a binder position — a set element is the value itself, so a bare name at an element position is an ordinary value expression (an in-scope name), not a new binding, exactly as a named key in a map pattern is a value expression rather than a binder.

A pattern MAY end in a rest binder that binds a set of the same type containing every element of the matched set EXCEPT the named elements, so that the named elements are consumed and the remainder is available for further matching. The rest binder is the only binder position in a set pattern; the whole pattern MUST remain linear (`CDZ0102`).

Because a set's element set is unbounded, no finite set of membership patterns can cover every set, so a match on a set MUST end in a name or wildcard pattern that binds the whole set; a set of membership arms with no such catch-all MUST be a compile-time error under *Matching Is Exhaustive Or Rejected*.

A membership pattern MUST observe a set only through the presence of elements; it MUST NOT expose or depend on any internal ordering or node structure of the set's representation, so that the same pattern matches a set regardless of how the set is represented.

## Types As First-Class Values

### Types Are First-Class Values

A Type MUST be a first-class value that can be bound to a name, passed as an argument, returned from a function, and inspected at runtime.

A type annotation `(: <expr> <Type>)` MUST carry its type as a value, not as a syntactic marker erased before evaluation.

The compiler MUST validate a type annotation against the annotated expression's static type at compile time.

The compiler MUST reject a program in which a type annotation's declared type does not match the annotated expression's static type before that program runs.

## Tuples

### A Tuple Is A Fixed-Size Positional Product

A tuple MUST be a fixed-size value whose elements are accessed positionally.

A tuple MAY hold elements of distinct types.

The empty tuple MUST be the unit value, so that unit and `()` are the same value.

A tuple MUST be deconstructible by pattern matching, so that `(tuple a b)` in pattern position binds the elements.

A tuple pattern MAY name some number of leading positions and end in a rest binder — `(tuple a .. rest)` — binding the leading positions to the corresponding elements and the rest binder to a tuple of the remaining elements in order, so that a fixed-size positional product is deconstructed positionally without naming every element. Because a tuple's arity is fixed by its type, a tuple pattern naming no more leading positions than the tuple's arity matches every tuple of that type and is therefore irrefutable in the sense of *A Binding Position Accepts An Irrefutable Pattern*, while a pattern naming more leading positions than the tuple's arity MUST be a compile-time arity error (`CDZ0201`) rather than a runtime non-match.

## Sum Types

### A Sum Type Constructor Is A Single-Arity Function Producing The Tagged Variant

A sum type constructor MUST be represented as a single-arity function that, when applied to exactly one argument, produces a Sum value tagged with the constructor's variant name.

A "nullary" variant MUST be a constructor whose argument type is Unit, not a pre-constructed Sum value.

Construction MUST be via application in all cases: `(Some 5)`, `(None unit)`, `(Sign.Zero unit)`.

A nullary variant MAY also be constructed by the bare parenthesized form `(Ctor)` — the constructor's name applied to no explicit argument, as the corpus commonly writes `(None)`, `(Idle)`, `(Nil)` — which MUST denote the same value as the explicit `(Ctor unit)`, so that `(None)` and `(None unit)` are the same value; the bare form is surface sugar the reader completes with the unit argument, so construction remains an application in the sense of the preceding sentence. The canonical value form of a nullary variant is `(Ctor unit)` regardless of which surface form constructed it. The bare parenthesized form is distinct from the bare name `Ctor` used in value position, which denotes the constructor value itself (a single-arity function awaiting its argument), not the constructed variant.

A pattern matching a sum type constructor MUST have the form `(Ctor binder)` in all cases: `(Some x)`, `(None _)`, `(Sign.Zero _)`.

A nullary variant's pattern MAY be written bare — the parenthesized `(Ctor)` or the bare name `Ctor` — as the binderless companion of `(Ctor _)`, matching the variant and binding nothing, as the corpus writes `((None) 2)`; per [prelude-and-resolution.md §A Pattern Name Binds Unless It Names A Constructor](../architecture/prelude-and-resolution.md), a bare name in pattern position that resolves to a constructor is that constructor's nullary pattern rather than a fresh binder. The bare pattern is surface sugar for `(Ctor _)`, so a match still handles it as the single-arity application form of the following sentences.

The prelude MUST bind Constructor values only for sum type variants, not pre-applied Sum values.

The pattern matcher MUST NOT special-case "nullary" vs "unary+" constructors by arity.

The pattern matcher MUST handle all constructor patterns uniformly as single-arity applications.

## Records, Maps, And Member Access

### A Record Has A Fixed Set Of Named Fields

A record MUST associate a fixed set of statically-known field names each with a value, where distinct fields may hold values of distinct types.

A record MUST be deconstructible by pattern matching on its field names, binding each named field's sub-value.

A record pattern MAY name a subset of the fields, ignoring the rest.

A record pattern that names a subset of the fields MAY end in a rest binder — `(record (= x a) .. rest)` — binding the rest binder to a record of exactly the fields the pattern does not name, so the unnamed fields are available as a record value rather than only ignored. Because a record's field set is fixed by its type, a record pattern that names only fields the type has and binds the remainder matches every record of that type and is therefore irrefutable in the sense of *A Binding Position Accepts An Irrefutable Pattern*.

A map MUST associate keys with values as a dynamic homogeneous collection whose set of keys is not fixed by the value's form, distinct from a record's fixed field set.

### A Record Field And A Map Entry Are Written As A Key-Value Pair

A record field and a map entry MUST both be written as a key-value pair headed by the field-pair marker `=` — `(= <key> <value>)` — so that a record construction `(record (= x 1) (= y 2))` and a map construction `(map (= k1 v1) (= k2 v2))` share one uniform key-value entry form, the field-pair form of *A Compound Value Has A Symbol Constructor And A Shadowable Alias*, rather than two. In the canonical stored form the field-pair head MUST be a reserved **field-pair leaf kind**, a single payloadless leaf kind recognized by its KIND IDENTITY, structurally distinct from the equality operator (which remains an ordinary name `=`), so that a `(= key value)` entry is never confused with an `(= a b)` equality by position or spelling; its two children are the entry's key and value.

An implementation MUST accept a map entry written as a bare positional pair `(<key> <value>)` without the `=` marker and MUST treat it as building the same map as the `(= <key> <value>)` form, so that the two spellings of a map entry are equivalent while programs are migrated to the uniform `=` form; this transitional acceptance applies to a map entry only, not to a record field, which is always the `(= <key> <value>)` form.

The bare positional map-entry form is deprecated in favor of the `(= <key> <value>)` form, and an implementation MAY, once programs no longer rely on the bare form, reject a map entry that omits the `=` marker rather than accept it, so that the key-value entry form becomes a single uniform form across records and maps.

### Member Access Projects A Record Field

Member access MUST project the field named by its key from the record it is applied to, evaluating to the value that field holds.

In the canonical stored form a member-access node `(. <obj> <key>)` MUST carry a reserved **member leaf kind** in head position, a single payloadless leaf kind recognized by its KIND IDENTITY rather than by the head text `.`, with the operand and the key as its two children; like the constructor leaf kinds (§"A Compound Value Has A Symbol Constructor And A Shadowable Alias") it is a declared-default concrete-encoding detail added additively and is its own canonical byte identity.

Member access applied to a value that is not a record MUST be rejected at compile time with the machine-readable code for a type error rather than produce an unspecified value or a runtime trap. A record's field names with their types are part of its type (*type-system.md §The Structural Types Are Record, Tuple, And Sum*), so whether the operand is a record and which fields it has are statically known.

Member access naming a field the record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent rather than produce an unspecified value or a runtime trap, so that a projection cannot name a field the operand's type never held. This is the bare-access companion of the row-projection rule *type-system.md §A Record Is Restricted To A Named Set Of Its Fields*, under which naming an absent field is likewise a compile-time rejection: `(. r f)` and a projection of `r` onto `{f}` reject an absent `f` identically.

## Modules

### A Module Binds Its Name In Its Enclosing Scope

Evaluating a module MUST bind the module's declared name in the enclosing scope to the record of the module's exports, so that a module is named by its declaration without a separate binding form.

A reference to a module's name in its enclosing scope MUST resolve to that export record under the same lexical scope and shadowing rules as any other binding.

### A Module Evaluates To A Record Of Its Exports

Evaluating a module MUST produce a record whose fields are the names its definitions export bound to their values.

Each definition a module exports MUST register its name and value as a field of the module's record.

A definition a module does not export MUST NOT register a field of the module's record, so that the record's fields are exactly the module's visible surface (modules-and-namespaces.md §Visibility Is Explicit).

A module's exported definition MUST be reachable by member access on the module's record.

### A Module Carries Its Manifest And Entry As Metadata

A module MUST carry the capabilities it declares as metadata separate from its exported fields, so that a declared capability is not itself an export.

A module's metadata MUST be reachable by a metadata key distinct from every export name, so that metadata access cannot collide with an export.

### A Built-In Module Is A Record Of Its Operations

A built-in module — a collection of operations the language provides rather than a program defines — MUST be a record whose fields name those operations, indistinguishable in form from a module a program defines. A reference to a built-in module's name MUST resolve to that record under the same scope and shadowing rules as any other binding, and an operation MUST be reached by member access on that record, so that projecting a built-in operation (`Mod.op` denoting `(. Mod op)`) is the ordinary record projection of *Member Access Projects A Record Field*, not a distinct construct. A built-in module and a program-defined module MUST be accessed by the identical mechanism; the language MUST NOT recognize a built-in module's name in any position a program-defined module's name would not be recognized.

A field of a built-in module whose implementation the language provides MUST hold a **built-in operation value** — a first-class value denoting that operation. A built-in operation value MUST be a value in the sense of *A Function Is A First-Class Value*: projecting it MUST yield the value itself, so that member access on a built-in module evaluates to the operation it names without applying it.

Applying a built-in operation value to arguments MUST produce the same result the operation defines for those arguments, so that `(Mod.op args)` — the application of the projected field — is equivalent to invoking the built-in operation directly. Applying it at an argument count the operation does not accept MUST be a compile-time error under *Applying A Function Binds Its Parameter To Its Argument* and the arity rules, exactly as for any other function value, rather than an unspecified result.

A built-in operation value used other than by application — bound to a name, stored in a data structure, compared, or partially applied — has no outcome fixed by this document beyond that it MUST NOT produce a wrong result: a compiler that does not realize such a use MUST decline to compile the program rather than emit code that computes an incorrect value. This preserves *reject-don't-miscompile* while leaving the first-class treatment of a built-in operation value (storage, partial application) to be specified as it is realized.

### A Compound Value Has A Symbol Constructor And A Shadowable Alias

A compound value — a tuple, a record — MUST have a **primitive constructor named by a string literal** in head position: a tuple is constructed by `("tuple" …)` and a record by `("record" …)`. A string literal is not something a name binding can introduce (a binding introduces an identifier, never a string), so the primitive constructor MUST NOT be shadowable, and the language recognizes the string-headed form structurally. The string spelling IS the reserved symbol — no distinct sigil is introduced, and the surface reader needs no dedicated literal syntax to write one.

Each such primitive MUST ALSO be reachable through an ordinary **alias name** — `tuple` for `("tuple" …)`, `record` for `("record" …)` — bound in the prelude exactly as any other built-in name, and therefore subject to *Binding Is Lexical* and *A Built-In Module Is A Record Of Its Operations*: a reference to the alias MUST resolve to the nearest enclosing binding of that name. Consequently a program binding named `tuple` or `record` (by `let`, a definition, or a parameter) MUST shadow the built-in alias for the extent of its scope — an application `(tuple a b)` in that scope MUST apply the bound value, not construct a tuple — precisely as a binding named `list` shadows the list constructor. The alias name MUST resolve identically in application-head position and in value position: the language MUST NOT recognize the alias name as the built-in constructor in a position a program-defined name would not be, so that one name never resolves two ways by syntactic position (the resolution split *Binding Is Lexical* forbids). Only the string-named primitive is beyond shadowing (a name binding cannot spell it); the alias is an ordinary name.

The canonical stored form of a compound-construction node is fixed by the AST-encoding contract (`../contracts/ast-encoding.md` §"The Encoding Is General And Stable"), which represents every node as a head applied to an ordered sequence of children. In that stored form a compound-construction node's head MUST be a reserved per-collection **constructor leaf kind** — one distinct payloadless leaf kind for each of the tuple, record, list, map, and set constructors — and a reader MUST recognize which compound a node constructs by the head leaf's KIND IDENTITY, never by comparing the head against the text `tuple` / `record` / `list` / `map` / `set`. Each such constructor leaf kind is a single reserved kind carrying no body, so it has exactly one canonical byte form and is byte-identical by construction, and it is its own identity — it MUST NOT collapse to or from a name leaf or a string-literal leaf under structural equality or canonicalization. A surface program MAY still write a compound with a string-headed spelling `("tuple" …)` or an alias-name spelling `(tuple …)`, and a reader MUST resolve such a spelling that denotes construction to the corresponding constructor leaf kind; a program that shadows an alias name instead resolves that occurrence to the shadowing binding under *Binding Is Lexical*, an ordinary reference node and not a constructor leaf. In a compound-construction node so encoded, a tuple's, list's, and set's children MUST be its element expressions in order and a record's and a map's children MUST be its `(= <key> <value>)` field-pair entries (§"A Record Field And A Map Entry Are Written As A Key-Value Pair"), so that a reader recovers the compound's shape from the constructor leaf kind and its children rather than from any surface spelling. The reserved constructor leaf kinds are a declared-default concrete-encoding detail added additively — a new leaf kind adds no container-encoding version (`../contracts/ast-encoding.md` §"New Constructs Do Not Bump The Encoding Version") — not a change to the frozen container form.

## Failure And Termination

### A Program That Terminates Ends In One Of Two Terminal Conditions

A program run that terminates MUST end in exactly one terminal condition: a normal result or a trap of a defined kind.

The terminal condition of a program run that terminates MUST be a deterministic function of its input and its declared capabilities' responses, so that whether a run terminates is a property of the environment that hosts it while the terminal condition of one that does is fixed by the program.

### A Trap Halts Execution At A Defined Point

A trap MUST halt the program at a defined point rather than continue with an unspecified value.

A trap MUST carry a defined kind that identifies why the program halted.

The kind of trap a given operation raises MUST be a deterministic function of the operation and its inputs.

### A Trap Occurs Only Where Its Computation Is Observed

A trap MUST occur when the computation that would raise it is observed — when its value flows to the program's result, to a host call, or to an operation that inspects it (an arithmetic or comparison operand, an `if` condition, a match scrutinee, a projected tuple element or record field, a referenced binding, or an argument bound to a parameter the function body uses). A computation whose value is observed in this sense MUST be evaluated, so its trap MUST occur.

An implementation MAY decline to evaluate a computation whose value cannot affect the program's observable behavior — one whose result reaches neither the program's terminal value nor any host call — and so MAY elide a trap that computation would raise. A tuple or record element that is never projected, a `let` binding that is never referenced, and an argument bound to a parameter the function body never uses are unobserved in this sense: constructing the surrounding value does not require evaluating them, so an implementation that omits them, and the traps they would have raised, is conformant. A heap-materialized collection constructor — a `(list …)`, a set constructor, or a map constructor — is NOT deferrable in this sense: whenever the constructor expression is evaluated it MUST evaluate every one of its element or entry ARGUMENT expressions, so the traps and effects those arguments would raise occur regardless of which consumer receives the collection — whether its length or size is taken, its membership is queried, it is compared for equality, it is bound and then discarded, or it flows to the program's result or a host call. An implementation MAY elide the collection's heap allocation when the object itself is unobserved, but ONLY while preserving that argument evaluation, so a heap-collection constructor is strict in its element arguments even when the object it would build is optimized away; a list, a set, and a map are thus strict in their element arguments at every point the constructor is evaluated. The deferral above is therefore specific to an unprojected tuple element or record field, whose surrounding value a projection can construct without it, and to a constructor expression that is itself unreached — in a conditional's unselected branch, a boolean connective's shielded operand, or a match arm that does not match — which is not evaluated at all. A structural equality over collections is still decided element-by-element and MAY short-circuit at the first differing element, but that short-circuit governs only which already-evaluated element values the comparison inspects, not whether a list operand's construction evaluates its arguments: evaluating `(= (list 1 (/ 5 d)) (list 9 9))` constructs the left operand — evaluating `(/ 5 d)` — before the comparison runs, so a trapping element in a constructed list operand traps regardless of the comparison's outcome. This is the same laziness the language already grants a conditional's unselected branch, a boolean connective's shielded operand, and a match arm that does not match — generalized to any subexpression whose value no observation depends on, save a heap-collection constructor's element or entry arguments, which a reached construction evaluates for their traps and effects even when the collection's value is unobserved. Whether a trap occurs is therefore a property of whether its computation's value is observed — or, for a heap-collection constructor's arguments, of whether the constructor is reached — not of the syntactic form the computation appears in.

Because eliding a computation the implementation can PROVE would trap is far more likely a program defect than an intent, an implementation SHOULD emit a diagnostic of non-error severity — one that leaves the build successful — when it drops a provably-trapping computation whose value is unobserved, so that a program does not silently discard a computation that could never have produced a value.

### Partial Operations Have A Defined Outcome

An operation that has no result for some inputs MUST, on those inputs, either evaluate to a value the executable semantics defines or raise a trap of a defined kind.

An operation that has no result for some inputs MUST NOT produce an unspecified value.

### Requiring The Value Of An Optional Traps On Absence

An optional MUST offer an operation that returns its contained value when one is present and raises a trap when it is absent, so that turning absence into a halt is one explicit operation rather than a behavior wired into each operation that produces an optional.

This operation MUST require a message argument, so that requiring a present value states, at the point it does so, why the value is expected to be present.

The trap this operation raises on an absent optional MUST carry that message as its reason, so that the halt names the expectation the program stated rather than the upstream operation that produced the absence.

## Equality And Ordering

### Equality Is Structural

Two values MUST be equal when they have the same type and their contents are equal component-wise.

Value equality MUST agree with the canonical byte form, so that two values are equal exactly when their canonical byte forms are identical.

### Floating-Point Equality Follows The Canonical Byte Form

A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.

### Ordering Where Offered Is Total

A type that offers an ordering MUST offer a total order over its values.

A floating-point type MUST NOT be treated as offering an ordering in the sense of this section, because its relational operators are the IEEE partial order defined for the floating-point type rather than a total order — so the requirement that an offered ordering be total does not apply to the floating-point relational operators.

The ordering a type offers MUST be a deterministic function of the values compared.

The Bool type MUST offer a total order in which false is less than true.

A `Bytes` value MUST offer a total order that is lexicographic over its unsigned byte values: comparing bytes from the first, the first differing byte is decisive under the numeric order of the two bytes taken as unsigned, and a byte sequence that is a proper prefix of another MUST compare less than that longer sequence. This order agrees with the byte-sequence equality already required, and is the same content-lexicographic ordering a string offers over its UTF-8 bytes; because it is a total order, a `Bytes` value may be used where an ordering is required — as a set element, a map key, or a component of an ordered compound.

### A Total Order Is Observed Through A Three-Way Comparison

A type that offers a total order MUST offer a three-way comparison that yields an ordering value with exactly three variants — less, equal, and greater — so that a single comparison reports the full relation between two values rather than a single boolean bit of it.

The ordering value's type MUST be an ordinary closed sum type of the language, so that a comparison result is deconstructed by the same exhaustive match as any other sum and every consumer handles all three cases.

The boolean ordering operators MUST agree with the three-way comparison, so that a type has one total order surfaced two ways that cannot disagree.

### Compound Ordering Is Lexicographic

A compound value — a tuple, a record, a list, or a sum — MUST offer a total order exactly when every one of its component types offers a total order.

A tuple or record MUST be ordered by comparing its components in the same canonical order its equality and canonical byte form use, taking the first component that differs as decisive and comparing equal only when every component compares equal.

A list MUST be ordered lexicographically: comparing elements from the first, the first differing element is decisive, and a list that is a proper prefix of another MUST compare less than that longer list.

A sum MUST be ordered first by the discriminant as encoded in its canonical byte form, and then, for two values of the same variant, lexicographically by payload, so that the order agrees with the canonical byte form equality already requires.

A compound any of whose component types does not, transitively, offer a total order MUST NOT be treated as offering one; in particular a floating-point component makes the compound unordered, because a floating-point type offers only the IEEE partial order. Such a compound remains equality-comparable by its canonical byte form, but is not ordered.

The total order a compound offers MUST agree with its structural equality — two values compare equal under the three-way comparison exactly when they are equal — and MUST be surfaced through the same three-way comparison and boolean ordering operators as any other total order.

This section defines ordering for tuples, records, lists, and sums; a map or a set is a compound value whose ordering is not yet offered and is a later addition.

## Observable Behavior

### Observable Behavior Is A Defined Projection Of A Run

The observable behavior of a program run MUST comprise its terminal condition, the value it produces on normal termination in canonical value form, and the ordered sequence of host calls it made with the arguments it passed.

The observable behavior of a program run MUST NOT include its internal representation, its timing, or its diagnostics.

### Host Calls Are Ordered And Part Of Observable Behavior

The sequence of host calls a program makes MUST be observed in the order the program made them.

Two runs whose observable behaviors differ in any host call, in host-call order, or in terminal condition MUST be treated as behaving differently.

## The Unit Value

### An Expression Evaluated Only For Its Effect Yields The Unit Value

An expression evaluated only for the host call it makes MUST yield the value that host call returns, which is the unit value when the call's WIT signature returns unit.

A program that terminates normally without producing a value other than through the host calls it makes MUST produce the unit value as its normal-termination value.
