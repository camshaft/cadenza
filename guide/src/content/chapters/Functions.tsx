// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Functions() {
  return (
    <article>
      <H1>Composing functions</H1>
      <Lede>Functions are values, so you can build bigger transformations out of smaller ones by piping, by composing, and by applying one argument at a time.</Lede>
      <P>You've already met higher-order functions in <C>Values &amp; functions</C>: because a function is an ordinary value, you can pass one to another function and return one from a function. This chapter is about the everyday shapes that grow out of that single idea.</P>
      <H2>The pipeline operator</H2>
      <P>When you transform a value through a series of steps, writing it as nested calls reads inside-out: the <em>last</em> thing to happen is written <em>first</em>. The pipeline operator <C>|&gt;</C> flips that around, so <C>x |&gt; f</C> means "take <C>x</C> and feed it to <C>f</C>", i.e. exactly <C>(f x)</C>, but written left-to-right in the order things happen.</P>
      <Runnable
        source={`(def (inc n) (+ n 1))

(def (dbl n) (* n 2))

(def (main) (|> (|> 5 inc) dbl))`}
      />
      <P>Read the <C>main</C> in the conventional syntax (flip the toggle in the header) and it becomes <C>5 |&gt; inc |&gt; dbl</C>: start with 5, increment to 6, double to 12. Same program, but the pipeline reads like a sentence, with data flowing through a sequence of steps.</P>
      <Why tenet="One program, many syntaxes"><C>|&gt;</C> isn't a special construct the compiler treats differently, but an ordinary operator the resolver rewrites into a plain application, so <C>x |&gt; f</C> and <C>(f x)</C> are the very same program underneath. That's the pattern throughout Cadenza: surface conveniences desugar to the one homoiconic core, which is why the syntax toggle can always show you the other view. The pipeline is sugar for readability, never a second way for a program to <em>mean</em> something.</Why>
      <H2>Composing two functions into one</H2>
      <P>Sometimes you don't want to pipe a value right now, but instead want a <em>new function</em> that is two functions glued together. Since functions are values, you can write <C>compose</C> yourself: it takes two functions and returns a function that runs one, then the other.</P>
      <Runnable
        source={`(def (compose f g) (fn (x) (f (g x))))

(def (inc n) (+ n 1))

(def (dbl n) (* n 2))

(def (main) ((compose inc dbl) 10))`}
      />
      <P><C>(compose inc dbl)</C> is a brand-new function that doubles then increments, so applying it to 10 gives 21. Nothing built-in made this possible; <C>compose</C> is a few characters of ordinary Cadenza, because functions are just values you can package up.</P>
      <H2>One argument at a time</H2>
      <P>A two-argument function is really a function that takes the first argument and returns a function waiting for the second. So you can apply arguments one at a time, which is called <em>currying</em>. Give <C>add</C> just its first argument and you get back a function:</P>
      <Runnable
        source={`(def (add a b) (+ a b))

(def (main) ((add 3) 4))`}
      />
      <P><C>(add 3)</C> is a function that adds 3 to whatever comes next; applying it to 4 yields 7. And capturing that partial application in a binding gives you a small, reusable specialization:</P>
      <Runnable
        source={`(def (adder n) (fn (x) (+ x n)))

(def (main) (let ((add3 (adder 3))) (+ (add3 1) (add3 10))))`}
      />
      <P><C>add3</C> remembers its <C>n</C> and can be called as many times as you like, here twice, for <C>4 + 13 = 17</C>.</P>
      <Why tenet="Uniformity over special cases">Currying isn't a separate feature, since it falls out of a single rule: every function takes exactly one argument. Multi-argument functions, partial application, and the pipeline operator are all just consequences of that one uniform shape. Cadenza would rather have one rule with no exceptions than a pile of special cases, because a smaller set of rules is a smaller set of things that can surprise you.</Why>
      <H2>Inline functions, inferred</H2>
      <P>The everyday use of a higher-order function is to pass a small function <em>inline</em>, as in the familiar map-and-combine over a list. Here <C>map-sum</C> applies <C>f</C> to each element and adds the results; the <C>f</C> we hand it is an anonymous <C>(fn (x) …)</C> written right at the call. Neither the lambda's parameter nor <C>map-sum</C>'s <C>f</C> carries a type annotation, because inference works both out from how <C>f</C> is used and in from the lambda's body:</P>
      <Runnable
        source={`(def (map-sum f acc xs) (match xs (#list() acc) (#list(h (.. t)) (map-sum f (+ acc (f h)) t))))

(def (main) (map-sum (fn (x) (+ x 1)) 0 #list(5 7 30)))`}
      />
      <P>Each element is incremented and the results summed: <C>6 + 8 + 31 = 45</C>. You write the lambda with no type ceremony and the compiler figures out the types from how the argument is used.</P>
      <P>The same holds for a <em>multi-argument</em> callback, as in the classic accumulator fold. Here <C>fold-list</C> takes a two-argument <C>f</C> and threads an accumulator through the list, and the lambda <C>(fn (x a) (+ a x))</C> is again fully unannotated on both sides. Folding <C>#list(5 7 30)</C> from <C>0</C> sums them to <C>42</C>:</P>
      <Runnable
        source={`(def (fold-list f acc xs) (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))

(def (main) (fold-list (fn (x a) (+ a x)) 0 #list(5 7 30)))`}
      />
      <P>No annotation on the closure's <C>x</C> or <C>a</C>, none on <C>fold-list</C>'s <C>f</C>, yet inference recovers all of it from how they're used. That's the everyday shape: write the callback inline and let the types follow.</P>
      <H2>Your turn</H2>
      <Exercise
        id="functions:1"
        prompt={<>Use the pipeline to feed <C>4</C> through <C>dbl</C> so the result is <C>8</C>.</>}
        starter={`(def (dbl n) (* n 2))

(def (main) (|> 4 ?))`}
        solution={`(def (dbl n) (* n 2))

(def (main) (|> 4 dbl))`}
        expected="8"
        hint={<>The right-hand side of <C>|&gt;</C> is the function to pipe into, here <C>dbl</C>.</>}
      />
      <Exercise
        id="functions:2"
        prompt={<><C>compose</C> is written for you, and <C>(compose f g)</C> runs <C>g</C> first, then <C>f</C>. Order the two functions so <C>5</C> is <em>doubled, then incremented</em>, giving <C>11</C>. Fill in the first argument.</>}
        starter={`(def (compose f g) (fn (x) (f (g x))))

(def (inc n) (+ n 1))

(def (dbl n) (* n 2))

(def (main) ((compose ? dbl) 5))`}
        solution={`(def (compose f g) (fn (x) (f (g x))))

(def (inc n) (+ n 1))

(def (dbl n) (* n 2))

(def (main) ((compose inc dbl) 5))`}
        expected="11"
        hint={<>The <em>second</em> argument runs first, so <C>dbl</C> doubles <C>5</C> to <C>10</C>; the <em>first</em> then runs on that, so it's <C>inc</C>, giving <C>11</C>. (Swap them to <C>(compose dbl inc)</C> and you'd get <C>12</C> instead.)</>}
      />
    </article>
  );
}
