import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Basics() {
  return (
    <article>
      <H1>Values &amp; functions</H1>
      <Lede>Literals, bindings, and first-class functions — the vocabulary everything is built from.</Lede>

      <H2>Bindings</H2>
      <P>
        A <C>let</C> binds a name to a value for the rest of an expression. Bindings are lexical, and
        an inner binding may shadow an outer one.
      </P>
      <Runnable source={`(let ((x 10)) (+ x 5))`} />

      <H2>Functions are values</H2>
      <P>
        A function is written with <C>fn</C> and is an ordinary value — you can bind it, pass it, and
        return it. Here we bind a function <C>inc</C> and call it.
      </P>
      <Runnable source={`(let ((inc (fn (x) (+ x 1)))) (inc 4))`} />

      <P>
        Functions close over their environment. <C>adder</C> returns a function that remembers{" "}
        <C>n</C>:
      </P>
      <Runnable source={`(let ((adder (fn (n) (fn (x) (+ x n)))))
  ((adder 3) 10))`} />

      <Why tenet="Uniformity over special cases">
        Underneath, every function takes exactly <em>one</em> argument and returns one value — a
        two-argument function is sugar for a function returning a function (that's why{" "}
        <C>adder</C> above works so naturally). Cadenza leans on this kind of uniformity everywhere:
        fewer special cases means fewer places for the compiler — and your mental model — to disagree
        with itself.
      </Why>

      <H2>Higher-order functions</H2>
      <P>
        Because functions are values, a function can take another function as an argument.{" "}
        <C>apply-twice</C> applies its argument function twice:
      </P>
      <Runnable source={`(let ((apply-twice (fn (f v) (f (f v)))))
  (apply-twice (fn (x) (+ x 1)) 5))`} />

      <H2>Your turn</H2>
      <P>Time to write some Cadenza. Fill in the blank, press Check, and the guide will grade it.</P>

      <Exercise
        id="basics:1"
        prompt={<>Make this produce <C>42</C> by doubling 21.</>}
        starter={`(* 21 ?)`}
        solution={`(* 21 2)`}
        expected="42"
        hint={<>Replace the <C>?</C> with the number that doubles 21.</>}
      />

      <Exercise
        id="basics:2"
        prompt={
          <>
            <C>apply-twice</C> runs a function on a value twice. Pass it a function that <em>doubles</em>{" "}
            its input, applied to <C>3</C> — doubling twice gives <C>12</C>.
          </>
        }
        starter={`(let ((apply-twice (fn (f v) (f (f v)))))
  (apply-twice (fn (x) ?) 3))`}
        solution={`(let ((apply-twice (fn (f v) (f (f v)))))
  (apply-twice (fn (x) (* x 2)) 3))`}
        expected="12"
        hint={
          <>
            The hole is the body of the function you're handing in — it should double its argument:{" "}
            <C>(* x 2)</C>. Then <C>3 → 6 → 12</C>.
          </>
        }
      />
    </article>
  );
}
