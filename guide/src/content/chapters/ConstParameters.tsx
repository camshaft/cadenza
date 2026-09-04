// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function ConstParameters() {
  return (
    <article>
      <H1>Const parameters</H1>
      <Lede>Some arguments you know before the program ever runs, like a size, a type, or a table of behaviour. What if the compiler could bake those in and charge you nothing at run time? That's a <C>const</C> parameter: an argument the compiler must know at compile time, so it is <em>inlined</em> into a specialized copy of the function and then erased, which means a different const value produces a different specialization and nothing about it survives to run time.</Lede>
      <P>You mark a parameter <C>const</C> by wrapping its annotated binder: <Cadenza ast="Y2R6YXN0AAEECgVjb25zdAoBOgoEbmFtZQoEVHlwZQYAAAABAAIAAwEDAQIDAQIABAU=" kind="expr">(const (: name Type))</Cadenza>. The <C>const</C> sits on the <em>parameter</em>, not the type. At the call site you pass an ordinary argument, and the only rule is that it must be a closed, compile-time-known value such as a constant, a type, or a record of functions, never something computed from runtime data.</P>
      <H2>A constant, inlined and specialized</H2>
      <P>The simplest const parameter is a constant scalar. Here <C>scale</C> takes a <C>const</C> factor <C>k</C> and a runtime <C>x</C>. Each call fixes <C>k</C> to a literal, so the compiler bakes that factor into a specialized <C>scale</C>, giving one copy with <C>k = 3</C> and another with <C>k = 2</C>:</P>
      <Runnable
        source={`(def (scale (const (: k Int64)) (: x Int64)) (* k x))

(def (main) (+ (scale 3 5) (scale 2 3)))`}
      />
      <P>That's <C>15 + 6 = 21</C>. The two calls compile to two distinct functions with the factor already substituted, the same way a generic function is monomorphized per type. A const parameter is that idea for <em>values</em>: specialize per compile-time-known argument, then erase it.</P>
      <H2>A dictionary of behaviour</H2>
      <P>The const value doesn't have to be a scalar, since it can be a whole <em>record of functions</em>, a "dictionary" that tells the function how to behave. Here <C>fold-n</C> applies an operation <C>n</C> times, and the operation lives in a const dictionary <C>d</C>. Calling it with two different dictionaries, one that adds 10 and one that doubles, specializes each call with its operation inlined:</P>
      <Runnable
        source={`(def
  (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))

(def
  (main)
  (+ (fold-n #record((= op (fn (x) (+ x 10)))) 3 0) (fold-n #record((= op (fn (x) (* x 2)))) 3 1)))`}
        id="fold-n"
      />
      <P>The first fold adds 10 three times from <C>0</C> to reach <C>30</C>, the second doubles three times from <C>1</C> to reach <C>8</C>, and together they make <C>38</C>. Because <C>d</C> is const, the <Cadenza ast="Y2R6YXN0AAEDGgoBZAoCb3AEAAAAAQACAQMAAQID" kind="expr">d.op</Cadenza> lookup folds to the concrete function in each specialized copy, so no record is passed at run time and no indirect call is emitted. You've hand-written the mechanism a typeclass or trait system would automate, handing the implementation to the function as a compile-time argument.</P>
      <H2>A const type parameter</H2>
      <P>The const argument can even be a <em>type</em>. A <Cadenza ast="Y2R6YXN0AAEECgVjb25zdAoBOgoBdAoEVHlwZQYAAAABAAIAAwEDAQIDAQIABAU=" kind="expr">(const (: t Type))</Cadenza> parameter takes a type-value at compile time (types are ordinary values, as shown in <em>Types as values</em>), and the body can branch on it with <C>Type.eq</C>, folding to a constant per specialization. Here <C>is-int</C> asks whether the type it was handed is <C>Int64</C>:</P>
      <Runnable
        source={`(def (is-int (const (: t Type)) (: x Int64)) (Type.eq t Int64))

(def (main) (is-int Int64 5))`}
        id="is-int"
      />
      <P>Called with <C>Int64</C> it folds to <C>true</C>, and calling it with <C>Bool</C> folds the same code to <C>false</C>, because each call site is specialized for the type it named so the comparison is settled at compile time rather than run time. The caller hands the type in as an argument and the compiler bakes a dedicated copy for it.</P>
      <Note>A <C>const</C> parameter can carry either a type or a dictionary, and both are the same idea seen from two angles. Passing a <C>const</C> <em>type</em> (a <Cadenza ast="Y2R6YXN0AAEECgVjb25zdAoBOgoBdAoEVHlwZQYAAAABAAIAAwEDAQIDAQIABAU=" kind="expr">(const (: t Type))</Cadenza> parameter) lets the caller name the type the code specializes for, which is <em>types as values</em> made into an argument, while passing a <C>const</C> <em>dictionary</em> lets the caller name the behaviour the code runs. Both ride the same machinery, since a compile-time-known argument is inlined and specialized and then erased before the program runs.</Note>
      <Why tenet="A compile-time argument costs nothing at run time">A const parameter lets a function be <em>parameterized</em> by a size, a strategy, a type, or a table of operations without paying for that flexibility when it runs. The argument is resolved while the program is compiled, folded into a specialized copy, and erased, so the generated code is exactly what you'd have written by hand for that specific value. It's the same bargain as generic monomorphization and first-class types: express the abstraction at compile time, keep the machine code concrete. And the compiler enforces the honesty, because a const argument that isn't actually known at compile time is a clean error rather than a silent fallback to a runtime value.</Why>
      <H2>Your turn</H2>
      <Exercise
        id="const-parameters:1"
        prompt={<><C>scale</C> bakes its <C>const</C> factor <C>k</C> into each specialized copy. Fill the factor so <C>(scale ? 5)</C> gives <C>20</C>.</>}
        starter={`(def (scale (const (: k Int64)) (: x Int64)) (* k x))

(def (main) (scale ? 5))`}
        solution={`(def (scale (const (: k Int64)) (: x Int64)) (* k x))

(def (main) (scale 4 5))`}
        expected="20"
        hint={<><C>k * 5 = 20</C> needs <C>k = 4</C>. That <C>4</C> is inlined into the specialized <C>scale</C> at compile time, so being a const argument it must be a literal rather than a runtime value.</>}
      />
      <Exercise
        id="const-parameters:2"
        prompt={<>The const dictionary decides the operation. Fill the body of <C>op</C> so that folding it three times from <C>0</C>, adding the same amount each step, reaches <C>15</C>.</>}
        starter={`(def
  (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))

(def (main) (fold-n #record((= op (fn (x) (+ x ?)))) 3 0))`}
        solution={`(def
  (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))

(def (main) (fold-n #record((= op (fn (x) (+ x 5)))) 3 0))`}
        expected="15"
        hint={<>Three steps from <C>0</C>, each adding the same amount <C>a</C>, gives <C>3 × a</C>. For <C>15</C> that's <C>a = 5</C>, so <C>op</C> is <Cadenza ast="Y2R6YXN0AAEECgJmbgoBeAoBKwABBQgAAAABAQEBAAIAAQADAQMDBAUBAwACBgc=" kind="expr">(fn (x) (+ x 5))</Cadenza>. The dictionary is const, so this operation is inlined into the specialized <C>fold-n</C>.</>}
      />
    </article>
  );
}
