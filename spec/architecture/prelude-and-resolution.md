# Prelude-And-Resolution Architecture

> **NORMATIVE — REFERENCE RESOLUTION ARCHITECTURE.** This document prescribes the mechanism by which a
> name acquires meaning in the reference compiler: that resolution is two generic operations over a single
> map of values plus a fixed grammar, that a name is resolved under an explicit evaluation mode, and that a
> built-in type is a record whose meaning as a type is read lazily where it is used. Its RFC-2119
> requirements bind a compiler built to the Cadenza *reference architecture* and are citable by the
> requirement gate for such a compiler.
>
> **This document is the mechanism companion to [reference-compiler.md §Nothing Is Privileged By Name](./reference-compiler.md).**
> That section fixes the *principle* — the resolver recognizes values, not names; every built-in is an
> ordinary value; a type, a constructor, and a pattern are ordinary values and expressions. This document
> fixes the *mechanism* that makes the principle enforceable: the two generic operations, the evaluation
> mode, and the meta channel by which a type record carries its meaning. It is **the foundation a
> from-scratch compiler establishes first**, because it fixes how every name — an operator, a constructor, a
> type, a module, an effect — acquires meaning, and every later pass reads meanings this layer assigns.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly
> one obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the requirements
> below name no concrete engine, prior prototype, or source path; the descriptive lead-ins and the learning
> they cite carry the concrete grounding.

## Purpose And Scope

[reference-compiler.md §Nothing Is Privileged By Name](./reference-compiler.md) establishes that every
construct a naive compiler recognizes by spelling — a built-in module, operation, type, constructor, or
pattern — is instead an ordinary value reached by the ordinary lookup-and-project mechanism. That principle
is repeatedly violated in practice the same way: an implementer, needing a built-in name to mean something,
adds a case that tests the name's spelling in the resolver rather than a value to the map — and each such
case is a second code path that must agree with a first, the disagree-and-miscompile class the whole
architecture exists to remove. This document fixes the mechanism precisely enough that adding a built-in is
*only ever* a map entry, so the violation has no place to live. It is the resolution-and-meaning foundation
the rest of the pipeline rests on: [reference-compiler.md](./reference-compiler.md) fixes the ladder that
carries meanings forward, [intermediate-representations.md](./intermediate-representations.md) fixes the
shape that holds them, and this document fixes how a name acquires a meaning in the first place.

The grounding is recorded in the learning
[the implementation design directions fold into one architecture — records everywhere is the foundation built first](../learnings/2026-07-10-the-implementation-design-directions-fold-into-the-architecture-records-everywhere-first.md).
It does not restate the language's name-resolution semantics
([core-semantics.md](../capabilities/core-semantics.md),
[modules-and-namespaces.md](../capabilities/modules-and-namespaces.md)) or the type system
([type-system.md](../capabilities/type-system.md)); it fixes the compiler-internal mechanism that realizes
them without a per-name special case.

## Resolution Is Two Generic Operations Over One Map, Plus A Fixed Grammar

The resolver performs exactly two operations that can produce a meaning from a name, and recognizes a fixed,
closed set of grammar forms. Every built-in — an operator, a collection constructor, a scalar or compound
type, a sum, a module, an effect — is an entry in one map of values that these two operations consult
generically. The resolver never branches on a source name's spelling, nor on a member key's spelling, to
decide what a name means.

### Name Resolution Is One Ordered Lookup Returning The Bound Value Verbatim

Resolving a name MUST be a single ordered lookup — the lexical scope, then the current module's own
definitions, then the prelude of built-in bindings — that returns the bound value unchanged, so that no step
rewrites a particular built-in name to a particular meaning and a program binding shadows a built-in of the
same name by the ordinary precedence of the lookup
([reference-compiler.md §The Prelude Is A Single Map The Resolver Consults By Name Alone](./reference-compiler.md)).

The lookup MUST return a built-in type name as the record bound to it rather than as the type that name
denotes, so that the meaning of the name as a type is read later, at the site it is used as one
(§A Built-In Type Is A Record Carrying A Meta Channel), rather than assigned by the resolver.

### The Names The Resolver Treats As Grammar Are A Fixed, Closed Set

The resolver MUST recognize a fixed, closed set of forms by name — the binding, control, and declaration
forms that bind names or control evaluation, and the member-access form — and MUST resolve every other name
through the one ordered lookup, so that adding a built-in value never adds a name the resolver recognizes as
grammar and the set of spellings the resolver matches does not grow when the language gains an operator, a
type, a width, an effect, or a macro.

A form whose head is not a grammar name MUST be dispatched by the *kind* of the value its head resolves to —
a constructor, an intrinsic, a collection-building marker, a module function, a bound local — never by the
head's spelling, so that application is generic over what the head *is* rather than what it is *called*.

### Member Access Is One Generic Projection That Does Not Inspect Its Key

Member access MUST be one generic projection of an operand by a key, and the projection MUST NOT branch on
the key's spelling or on the syntactic shape of the key node, so that projecting any field — an operation of
a module, a variant of a sum, a meta field of a type — is one operation rather than a case per member
([reference-compiler.md §Member Access Projects A Record Field](../capabilities/core-semantics.md)).

The resolver MUST recognize a member-access node in the stored form by its reserved member leaf kind — the
head-leaf KIND IDENTITY, not the head text `.` — consistent with recognizing a form by its kind rather than
its spelling ([core-semantics.md §Member Access Projects A Record Field](../capabilities/core-semantics.md));
the member leaf kind is one of the reserved structural leaf kinds that the constructor leaf kinds also belong
to (core-semantics.md §"A Compound Value Has A Symbol Constructor And A Shadowable Alias").

A projection of a compile-time-known aggregate — a literal record or tuple, which every prelude type,
module, sum, and meta record is — MUST reduce to the projected field during the construction of the resolved
representation, and a projection of a value not known until run time MUST survive as the one projection node
to the later passes, so that compile-time member access and run-time member access are the same operation
resolved at different times rather than two mechanisms.

## A Name Is Resolved Under An Explicit Evaluation Mode

Whether a bare name is looked up as a value, taken as a label, or bound as a pattern variable is a property
of the *position* the name appears in, not of the name. The resolver carries that position as an explicit
mode, so that the interpretation of a name is a stated rule rather than a shape inspection of the name's
surroundings — replacing the ad-hoc peeking (at a key node's shape, at whether a name is in the prelude
inside a binder collector) that a mode makes unnecessary.

### Resolution Carries An Explicit Mode

Resolution MUST carry an explicit mode that selects how a name is interpreted — as a value to be looked up,
as a member key, or as a pattern — and each form MUST choose the mode of each of its children, so that a
name's interpretation is determined by the mode its parent assigns rather than inferred from the name's
spelling or the shapes around it.

### A Member Key Is A Label, Not A Value

In key mode a bare name MUST become a symbol without any scope, module, or prelude lookup, so that a member
key names a field rather than resolving to whatever value that name would otherwise denote, and a member
access whose key is itself an expression MUST resolve that expression as a value, so that a computed key and
a literal label are distinguished by the key's structure under one mode rule rather than by a name test.

### A Pattern Name Binds Unless It Names A Constructor

In pattern mode a bare name MUST bind a fresh variable unless it resolves, through the one ordered lookup, to
a constructor — in which case it is that constructor's nullary pattern — so that the binder-versus-constructor
rule is stated once and read from the same lookup that resolves a value, rather than re-tested against the
prelude at each place a pattern is walked
([reference-compiler.md §A Type, A Constructor, And A Pattern Are Ordinary Values And Expressions](./reference-compiler.md)).

## A Built-In Type Is A Record Carrying A Meta Channel

A built-in type name resolves to an ordinary record — its operations are ordinary fields — that additionally
carries a **meta channel**: a reserved set of fields, distinct from its operation fields, that carries the
name's meaning as a type. Because the meaning is data in the record rather than a rewrite the resolver
performs, the record flows through resolution untouched, and "what type does this name denote" is answered by
projecting the meta channel at the site the name is used as a type. This is what lets one record serve both
roles — a value whose fields are projected, and a name that denotes a type — with no grammar for the meta
channel and no name test anywhere.

### A Type Name Resolves To A Record, Not Directly To A Type

A built-in type name MUST resolve to a record carrying its operations as ordinary fields plus a meta channel,
rather than to a type-value, so that a type name is an ordinary record in the resolved representation and its
distinction from a data record is carried in data — the presence of the meta channel — not in a name the
resolver recognizes.

### The Meaning Of A Type Record Is Read At Its Use Site

The pass that determines types MUST read a type record's meaning from its meta channel at the site the record
is used as a type — a ground type from the meta field that carries the type it denotes, a type constructor
from the meta field that carries the constructor to apply — rather than the resolver assigning that meaning,
so that the meaning of a name as a type is solved once, lazily, by the pass that already reasons about types
([reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md)).

A type record reduced to the type it denotes MUST be subject to the erasure fence that forbids a
compile-time-only value at the runtime boundary, so that a type used as a value is a compile-time structure
that leaves no runtime trace, exactly as any other compile-time reduction
([reference-compiler.md §A Construct Whose Value Is Fully Determined At Compile Time](./reference-compiler.md);
[metaprogramming.md §Compile-Time Evaluation Is One Tier](../capabilities/metaprogramming.md)).

### A Parametric Type Is Applied Through Its Meta Channel

Applying a parametric type name to arguments MUST proceed by reading the meta field that carries its type
constructor and applying that, so that a type application is an ordinary application of a value the one
lookup already returned rather than a construct the resolver special-cases
([type-system.md §Generics Are Type-Valued Parameters](../capabilities/type-system.md)).

### The Meta Channel Is Extended By Adding A Field, Never A Grammar Rule

A new kind of compile-time knowledge a name carries — the type it denotes, the constructor it applies, the
capabilities it delegates, the entrypoint it marks, the evaluation discipline a macro requests — MUST be
added as a field of the meta channel read where that knowledge is needed, never as a new grammar form or a
new name the resolver recognizes, so that the meta channel is an open, data-extended record rather than a
growing set of keywords.

## The Consequence — One Model For Types, Widths, Effects, And Modules

Because a type is a record and its meaning is a meta field read at the use site, the constructs a naive
compiler would each special-case collapse to one model. This section records that the model is uniform; the
concrete semantics live in the capability specifications it cites.

### A Numeric Width Is A Type Record, Its Machine Operation Read From Its Meta

An integer type of a given width and signedness MUST be a type record whose meta channel carries that width
and signedness, so that a width is one prelude entry, an unusual width is a type the compiler computes rather
than a hand-written case, and the machine operation a use selects — a signed versus an unsigned shift or
comparison, the range an overflow check tests — is read from the signedness-and-width meta rather than from
the type name ([numeric-model.md](../capabilities/numeric-model.md), which fixes the width semantics — the
overflow-traps rule per width, the checked-versus-wrapping conversions, the no-implicit-promotion rule that
two integer types unify only at equal width and signedness).

An integer type's signedness, like its width, MUST be a value the type solver determines under unification —
each an axis that a literal leaves open and that a use or an annotation grounds — rather than a fixed property
a literal is assigned before it is used, so that a literal is polymorphic in both signedness and width until a
context fixes them, an operation that serves both signednesses is one prelude entry generic over a signedness
variable rather than one entry per signedness, and the rule that an annotation constrains a value's type
without contradicting it is a consequence of unifying the annotation's signedness and width into the value
rather than a special case for a literal. Two integer types whose signedness or width are each fixed to
different values MUST fail to unify (the no-implicit-promotion rule above), so that grounding an axis by
unification never silently promotes across a genuine mismatch.

### An Effect Is A Record Of Its Operations, Reached By The One Projection

A declared effect MUST resolve to a record of its operations, and a performed operation MUST be reached by
the one generic member projection, so that an effect and its operations are ordinary values the pipeline
carries and resolves by the same lookup and projection as any other record, rather than a construct named in
the resolver ([reference-compiler.md §Effects Are Classified First And Resolved By Monomorphization](./reference-compiler.md);
[capabilities-and-effects.md](../capabilities/capabilities-and-effects.md)).

## The Discipline

This layer exists because the recurring failure is hard-coding a built-in's meaning into the resolver rather
than adding a map entry. The enforceable invariant is therefore stated as a property of what the resolver
matches:

### The Set Of Spellings The Resolver Matches Is The Fixed Grammar Set And Does Not Grow

The set of source-name spellings the resolver matches to produce a meaning MUST be exactly the fixed grammar
set, and that set MUST NOT grow when a built-in value — an operator, a collection constructor, a type, a sum,
a width, an effect, a meta key, or a macro — is added, so that a change that requires a new arm in a resolver
match on a name or a member-key spelling is a violation of this architecture rather than a way to add a
feature. Every added built-in is a map entry, and every added kind of compile-time knowledge is a meta field.
