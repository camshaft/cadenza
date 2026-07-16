import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function AdHocPolymorphism() {
  return (
    <article>
      <H1>Ad-hoc polymorphism</H1>
      <Lede>
        One name, more than one meaning — chosen by the <em>type</em> of what you give it, and settled at
        compile time. Cadenza has two forms of this: the built-in operators that dispatch on their operand
        type, and generic functions that the compiler specializes once per type they're used at.
      </Lede>

      <P>
        "Polymorphism" just means one piece of code working over many types. There are two flavours.{" "}
        <em>Parametric</em> polymorphism is one implementation, uniform over any type — the code can't look
        at the type, so it does the same thing regardless. <em>Ad-hoc</em> polymorphism is the other kind:
        potentially <em>different</em> behaviour per type. This chapter is about how Cadenza does the ad-hoc
        kind — and, importantly, how it does <em>not</em>.
      </P>

      <H2>Operators dispatch on their operand type</H2>
      <P>
        The clearest ad-hoc polymorphism in Cadenza is hiding in plain sight: the arithmetic and comparison
        operators. <C>+</C> is one surface name, but it means <em>different machine operations</em>{" "}
        depending on what you add — integer addition for integers, floating-point addition for floats (and
        exact addition for rationals, big-integer addition for bignums). The compiler picks which by the
        operand type. Here the very same <C>+</C> and <C>&lt;</C> appear over floats and over integers in
        one program:
      </P>
      <Runnable
        source={`(def (main)
  (if (< (+ 1.5 1.5) 4.0)
    (+ 1 2)
    99))`}
      />
      <P>
        The <C>(+ 1.5 1.5)</C> and <C>(&lt; … 4.0)</C> are float operations; the <C>(+ 1 2)</C> is an
        integer operation — same symbols, chosen by type, folding to <C>3</C>. Equality is the same story:{" "}
        <C>=</C> compares integers by value and strings by contents, dispatching to the right one:
      </P>
      <Runnable
        source={`(def (main)
  (if (if (= 3 3) (= "a" "a") false) 1 0))`}
      />
      <P>
        You never chose an "int-equals" or "string-equals" function; <C>=</C> is one name that resolves,
        by type, to distinct implementations. That's ad-hoc polymorphism — and it's built into the prelude
        operators, not something you assemble.
      </P>

      <H2>Generic functions specialize per type</H2>
      <P>
        The other form is your own generic functions. Write a function with no type annotations and
        inference gives it a type <em>variable</em> — it works for any type that fits. A recursive{" "}
        <C>len</C> over a list doesn't care what the elements are:
      </P>
      <Runnable
        source={`(def (len xs)
  (match xs
    ((list) 0)
    ((list h .. t) (+ 1 (len t)))))
(def (main)
  (+ (len (list 1 2 3)) (len (list "a" "b"))))`}
      />
      <P>
        That program calls <C>len</C> at two different types — a <C>List Int64</C> and a{" "}
        <C>List String</C> — and gets <C>3 + 2 = 5</C>. Under the hood the compiler doesn't keep one
        type-erased <C>len</C> that inspects its argument at run time. It <em>monomorphizes</em>: it emits a
        distinct, specialized copy of <C>len</C> for each concrete type it's actually called at — one for
        lists of integers, one for lists of strings. Dispatch is entirely static, resolved by the inferred
        type at each call site, with no runtime type tag.
      </P>
      <Note>
        The specialization is normally invisible, but the toolchain can show it:{" "}
        <C>cdz instantiations len some-file.cdz</C> lists the concrete types <C>len</C> was compiled at,
        each with its source location. It's the one place you can <em>see</em> that a single generic{" "}
        <C>len</C> became several machine functions.
      </Note>

      <H2>What Cadenza deliberately does not have</H2>
      <P>
        It's worth being precise, because many languages spell ad-hoc polymorphism differently. Cadenza has
        no typeclasses, traits, or overloading you can <em>declare</em>. You cannot write two functions with
        the same name and have the compiler pick between them by argument type — that isn't a feature, and
        the compiler will reject the redefinition rather than guess. Ad-hoc polymorphism here is exactly two
        things: the built-in typed operators, and the automatic specialization of generic functions. No
        more, no less.
      </P>

      <Note>
        There's a third, explicit door onto the same idea, covered in{" "}
        <em>Types as values</em>: a function can take a <C>const</C> parameter that's a <em>type</em> — a
        type passed as a compile-time argument — and branch on it. Where the two mechanisms here dispatch{" "}
        <em>implicitly</em> (the operator reads its operand type; the generic is specialized at its call
        site), a const type-parameter lets the caller name the type explicitly. Same compile-time,
        zero-runtime-cost machinery, reached a different way.
      </Note>

      <Why tenet="One name per type, resolved and erased at compile time">
        Ad-hoc polymorphism is a convenience — <C>+</C> reading naturally over ints and floats, one{" "}
        <C>len</C> serving every list — but Cadenza buys it without a runtime price. The operator's type is
        known when the program is checked, so the right machine op is chosen then; a generic is specialized
        into concrete copies, so each call is a direct call to type-specific code. Nothing carries a type
        tag into the running program, and nothing is dispatched dynamically. And by refusing user-declared
        overloading, the language keeps a name's meaning predictable: it comes from the prelude or from one
        definition, never from an invisible resolution you have to reason about.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="ad-hoc-polymorphism:1"
        prompt={
          <>
            A generic function works at any type. <C>id</C> returns its argument unchanged; here it's used
            once at <C>Bool</C> (the condition) and once at <C>Int64</C>. Fill the integer so the{" "}
            <C>Int64</C> use gives <C>7</C>.
          </>
        }
        starter={`(def (id x) x)
(def (main) (if (id true) (id ?) 0))`}
        solution={`(def (id x) x)
(def (main) (if (id true) (id 7) 0))`}
        expected="7"
        hint={
          <>
            <C>(id true)</C> is the <C>Bool</C> use (it picks the true branch); <C>(id 7)</C> is the{" "}
            <C>Int64</C> use and is what's returned. Put <C>7</C> in the blank. The one <C>id</C> is
            specialized at both types.
          </>
        }
      />

      <Exercise
        id="ad-hoc-polymorphism:2"
        prompt={
          <>
            The same <C>+</C> means integer or float addition depending on its operands. Fill the float so
            the comparison <C>(&lt; (+ 2.0 ?) 5.0)</C> is <em>true</em> — selecting the{" "}
            <C>(+ 10 20)</C> integer branch, giving <C>30</C>.
          </>
        }
        starter={`(def (main)
  (if (< (+ 2.0 ?) 5.0) (+ 10 20) 0))`}
        solution={`(def (main)
  (if (< (+ 2.0 1.0) 5.0) (+ 10 20) 0))`}
        expected="30"
        hint={
          <>
            You need <C>(+ 2.0 ?)</C> to stay under <C>5.0</C>, so any float below <C>3.0</C> works —{" "}
            <C>1.0</C> gives <C>3.0 &lt; 5.0</C>, true. Note the addition on <C>2.0</C> is float addition,
            while <C>(+ 10 20)</C> is integer addition: one operator, two types.
          </>
        }
      />
    </article>
  );
}
