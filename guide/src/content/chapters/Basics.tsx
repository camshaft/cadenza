import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";

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

      <H2>Higher-order functions</H2>
      <P>
        Because functions are values, a function can take another function as an argument.{" "}
        <C>apply-twice</C> applies its argument function twice:
      </P>
      <Runnable source={`(let ((apply-twice (fn (f v) (f (f v)))))
  (apply-twice (fn (x) (+ x 1)) 5))`} />
    </article>
  );
}
