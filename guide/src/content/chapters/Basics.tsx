// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Basics() {
  return (
    <article>
      <H1>Values & functions</H1>
      <Lede>Literals, bindings, and first-class functions, the vocabulary everything is built from.</Lede>
      <H2>Bindings</H2>
      <P>A <C>let</C> binds a name to a value for the rest of an expression. Bindings are lexical, and an inner binding may shadow an outer one.</P>
      <Runnable
        source={`(let ((x 10)) (+ x 5))`}
      />
      <P><C>x</C> is <C>10</C> for the body <Cadenza ast="Y2R6YXN0AAEDCgErCgF4AAEFBAAAAAEAAgEDAAECAw==" kind="expr">(+ x 5)</Cadenza>, so this is <C>15</C>.</P>
      <P>Shadowing is just a second binding of the same name. The inner <C>x</C> is computed <em>from</em> the outer one, since the right-hand side still sees the old value, and only then takes over for the rest of the body:</P>
      <Runnable
        source={`(let ((x 10)) (let ((x (* x 2))) (+ x 1)))`}
      />
      <P>The inner binding's <Cadenza ast="Y2R6YXN0AAEDCgEqCgF4AAECBAAAAAEAAgEDAAECAw==" kind="expr">(* x 2)</Cadenza> reads the outer <C>x = 10</C> to get <C>20</C>; the body then sees that inner <C>x</C>, so <Cadenza ast="Y2R6YXN0AAEDCgErCgF4AAEBBAAAAAEAAgEDAAECAw==" kind="expr">(+ x 1)</Cadenza> is <C>21</C>. Nothing was mutated, since the outer <C>x</C> is untouched, just out of view.</P>
      <H2>Functions are values</H2>
      <P>A function is written with <C>fn</C> and is an ordinary value, so you can bind it, pass it, and return it. Here we bind a function <C>inc</C> and call it.</P>
      <Runnable
        source={`(let ((inc (fn (x) (+ x 1)))) (inc 4))`}
      />
      <P>Binding it to <C>inc</C> and calling <Cadenza ast="Y2R6YXN0AAECCgNpbmMAAQQDAAAAAQECAAEC" kind="expr">(inc 4)</Cadenza> gives <C>5</C>.</P>
      <P>Functions close over their environment. <C>adder</C> returns a function that remembers <C>n</C>:</P>
      <Runnable
        source={`(let ((adder (fn (n) (fn (x) (+ x n))))) ((adder 3) 10))`}
      />
      <P><Cadenza ast="Y2R6YXN0AAECCgVhZGRlcgABAwMAAAABAQIAAQI=" kind="expr">(adder 3)</Cadenza> captures <C>n = 3</C> and returns a function that adds 3; applying it to <C>10</C> gives <C>13</C>.</P>
      <Why tenet="Uniformity over special cases">Underneath, every function takes exactly <em>one</em> argument and returns one value, so a two-argument function is sugar for a function returning a function (that's why <C>adder</C> above works so naturally). Cadenza leans on this kind of uniformity everywhere, because fewer special cases means fewer places for the compiler, and your mental model, to disagree with itself.</Why>
      <H2>Higher-order functions</H2>
      <P>Because functions are values, a function can take another function as an argument. <C>apply-twice</C> applies its argument function twice:</P>
      <Runnable
        source={`(let ((apply-twice (fn (f v) (f (f v))))) (apply-twice (fn (x) (+ x 1)) 5))`}
      />
      <P>The passed-in function adds 1, applied twice to <C>5</C>, so <C>5 → 6 → 7</C>.</P>
      <H2>Types are inferred, and can be written</H2>
      <P>Every value has a type, and so far the compiler has worked them out for you, since you never wrote <C>Int64</C> anywhere, yet the results came back typed. When you <em>want</em> to state a type, whether as documentation or to pin down something inference would otherwise leave open, you annotate a binding with its type. Toggle to the ML surface and this <C>dbl</C> reads <C>def dbl(x: Int64)</C>:</P>
      <Runnable
        source={`(def (dbl (: x Int64)) (* x 2))

(def (main) (dbl 21))`}
      />
      <P>An annotation isn't just a comment, because the compiler <em>checks</em> it against the type it inferred and refuses if they disagree. Claiming a plain number is a <C>Bool</C> is a contradiction it won't accept:</P>
      <Note>This one is <strong>meant to be refused</strong>. Run it and read the diagnostic, <C>CDZ0203</C>, "annotation type Bool does not match value type Int64": the annotation is held to the truth, not the other way around.</Note>
      <Runnable
        source={`(: 5 Bool)`}
        expect="error"
      />
      <P>Because inference already knows the types, annotations are optional almost everywhere, so write them where they clarify, and leave them off where they'd just be noise.</P>
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
        prompt={<><C>make-scaler</C> returns a function that multiplies by whatever <C>factor</C> it captured. Fill the hole so <C>triple</C> is a scaler that captures <C>3</C>, then <Cadenza ast="Y2R6YXN0AAECCgZ0cmlwbGUAAQUDAAAAAQECAAEC" kind="expr">(triple 5)</Cadenza> is <C>15</C>.</>}
        starter={`(let
  ((make-scaler (fn (factor) (fn (x) (* x factor)))))
  (let ((triple (make-scaler ?))) (triple 5)))`}
        solution={`(let
  ((make-scaler (fn (factor) (fn (x) (* x factor)))))
  (let ((triple (make-scaler 3))) (triple 5)))`}
        expected="15"
        hint={<>The hole is the <C>factor</C> that <C>triple</C> should capture. You want it to triple, so pass <C>3</C>; the returned function then remembers it, and <Cadenza ast="Y2R6YXN0AAECCgZ0cmlwbGUAAQUDAAAAAQECAAEC" kind="expr">(triple 5)</Cadenza> is <C>5 × 3</C>.</>}
      />
    </article>
  );
}
