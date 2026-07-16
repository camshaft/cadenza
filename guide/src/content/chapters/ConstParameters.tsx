import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function ConstParameters() {
  return (
    <article>
      <H1>Const parameters</H1>
      <Lede>
        A <C>const</C> parameter is an argument the compiler must know at compile time. It gets{" "}
        <em>inlined</em> into a specialized copy of the function and then erased — so a different const
        value produces a different specialization, and nothing about it survives to run time.
      </Lede>

      <P>
        You mark a parameter <C>const</C> by wrapping its annotated binder: <C>(const (: name Type))</C>.
        The <C>const</C> sits on the <em>parameter</em>, not the type. At the call site you pass an ordinary
        argument — the only rule is that it must be a closed, compile-time-known value (a constant, a type,
        a record of functions), never something computed from runtime data.
      </P>

      <H2>A constant, inlined and specialized</H2>
      <P>
        The simplest const parameter is a constant scalar. Here <C>scale</C> takes a <C>const</C> factor{" "}
        <C>k</C> and a runtime <C>x</C>. Each call fixes <C>k</C> to a literal, so the compiler bakes that
        factor into a specialized <C>scale</C> — one copy with <C>k = 3</C>, another with <C>k = 2</C>:
      </P>
      <Runnable
        source={`(def (scale (const (: k Int64)) (: x Int64))
  (* k x))
(def (main)
  (+ (scale 3 5) (scale 2 3)))`}
      />
      <P>
        That's <C>15 + 6 = 21</C>. The two calls compile to two distinct functions with the factor already
        substituted — the same way a generic function is monomorphized per type. A const parameter is that
        idea for <em>values</em>: specialize per compile-time-known argument, then erase it.
      </P>

      <H2>A dictionary of behaviour</H2>
      <P>
        The const value doesn't have to be a scalar — it can be a whole <em>record of functions</em>, a
        "dictionary" that tells the function how to behave. Here <C>fold-n</C> applies an operation{" "}
        <C>n</C> times, and the operation lives in a const dictionary <C>d</C>. Call it with two different
        dictionaries — one that adds 10, one that doubles — and each call specializes with its operation
        inlined:
      </P>
      <Runnable
        source={`(def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
(def (main)
  (+ (fold-n (record (op (fn (x) (+ x 10)))) 3 0)
     (fold-n (record (op (fn (x) (* x 2)))) 3 1)))`}
      />
      <P>
        The first fold adds 10 three times from <C>0</C> — <C>30</C>; the second doubles three times from{" "}
        <C>1</C> — <C>8</C>; together <C>38</C>. Because <C>d</C> is const, the <C>(. d op)</C> lookup folds
        to the concrete function in each specialized copy: no record is passed at run time, and no indirect
        call is emitted. You've hand-written the mechanism a typeclass or trait system would automate —
        handing the implementation to the function as a compile-time argument.
      </P>

      <Note>
        This is the third face of the compile-time-argument idea, and it ties the last few chapters
        together. Passing a <C>const</C> <em>type</em> (a <C>(const (: t Type))</C> parameter) is the
        explicit form of <em>types as values</em> — the caller names the type. Passing a <C>const</C>{" "}
        <em>dictionary</em>, as above, is the explicit form of <em>ad-hoc polymorphism</em> — the caller
        names the behaviour. Both are the same machinery: a compile-time-known argument, inlined and
        specialized, erased before the program runs.
      </Note>

      <Why tenet="A compile-time argument costs nothing at run time">
        A const parameter lets a function be <em>parameterized</em> — by a size, a strategy, a type, a
        table of operations — without paying for that flexibility when it runs. The argument is resolved
        while the program is compiled, folded into a specialized copy, and erased, so the generated code is
        exactly what you'd have written by hand for that specific value. It's the same bargain as generic
        monomorphization and first-class types: express the abstraction at compile time, keep the machine
        code concrete. And the compiler enforces the honesty — a const argument that isn't actually known
        at compile time is a clean error, not a silent fallback to a runtime value.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="const-parameters:1"
        prompt={
          <>
            <C>scale</C> bakes its <C>const</C> factor <C>k</C> into each specialized copy. Fill the factor
            so <C>(scale ? 5)</C> gives <C>20</C>.
          </>
        }
        starter={`(def (scale (const (: k Int64)) (: x Int64))
  (* k x))
(def (main) (scale ? 5))`}
        solution={`(def (scale (const (: k Int64)) (: x Int64))
  (* k x))
(def (main) (scale 4 5))`}
        expected="20"
        hint={
          <>
            <C>k * 5 = 20</C> needs <C>k = 4</C>. That <C>4</C> is baked into the specialized <C>scale</C> at
            compile time — it's a const argument, so it must be a literal, not a runtime value.
          </>
        }
      />

      <Exercise
        id="const-parameters:2"
        prompt={
          <>
            The const dictionary decides the operation. Fill the body of <C>op</C> so folding it three times
            from <C>0</C> — adding the same amount each step — reaches <C>15</C>.
          </>
        }
        starter={`(def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
(def (main) (fold-n (record (op (fn (x) (+ x ?)))) 3 0))`}
        solution={`(def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
(def (main) (fold-n (record (op (fn (x) (+ x 5)))) 3 0))`}
        expected="15"
        hint={
          <>
            Three steps from <C>0</C>, each adding the same amount <C>a</C>, gives <C>3 × a</C>. For{" "}
            <C>15</C> that's <C>a = 5</C> — so <C>op</C> is <C>(fn (x) (+ x 5))</C>. The dictionary is const,
            so this operation is inlined into the specialized <C>fold-n</C>.
          </>
        }
      />
    </article>
  );
}
