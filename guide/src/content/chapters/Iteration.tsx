// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function Iteration() {
  return (
    <article>
      <H1>Iteration without loops</H1>
      <Lede>Coming from most languages, you would reach for a <C>for</C> or a <C>while</C> here. Cadenza has neither. This chapter is how you repeat work instead, and why the language leaves loops out.</Lede>
      <P>There is no loop keyword in Cadenza. The full set of keywords is <C>def</C>, <C>do</C>, <C>effect</C>, <C>else</C>, <C>export</C>, <C>handle</C>, <C>if</C>, <C>import</C>, <C>in</C>, <C>let</C>, <C>match</C>, <C>module</C>, <C>return</C>, <C>then</C>, and <C>type</C>, with no <C>for</C>, no <C>while</C>, and no <C>loop</C>. Repetition is done with <em>functions that call themselves</em>: recursion. That sounds like a limitation. It removes a whole class of bugs.</P>
      <H2>Why no loops</H2>
      <P>A loop is a <em>statement</em>: it runs for its side effects, mutating a counter and an accumulator until a condition flips. That mutable state is where off-by-one errors, uninitialized accumulators, and forgotten updates live. Cadenza is expression-oriented (everything computes a <em>value</em>), so repetition computes a value too. There is no loop variable to misinitialize and no <C>i++</C> to forget, because there is no mutable loop state at all: each step is a fresh function call with its arguments spelled out.</P>
      <H2>The mechanism: a recursive accumulator</H2>
      <P>The workhorse pattern is a function that carries the answer-so-far in an argument, the <em>accumulator</em>, and calls itself with an updated one. It needs two things: a <strong>base case</strong> that stops the recursion and returns the accumulator, and a <strong>recursive case</strong> that does one step and recurses on the rest. Here is a sum from <C>n</C> down to <C>1</C>:</P>
      <Runnable
        source={`(def (main) (sum-to 5 0))

(def (sum-to n acc) (if (= n 0) acc (sum-to (- n 1) (+ acc n))))`}
      />
      <P>Read it as a loop turned inside out: <C>acc</C> is the running total, <C>n</C> counts down, the <Cadenza ast="Y2R6YXN0AAEDCgE9CgFuAAAEAAAAAQACAQMAAQID" kind="expr">(= n 0)</Cadenza> check is the exit condition, and each call adds <C>n</C> to <C>acc</C> and continues. When <C>n</C> reaches <C>0</C> the base case hands back the total, <C>15</C>. Nothing mutates; each call just receives the next pair of values.</P>
      <P>The same shape works over a list. Match the list by its structure, either the empty list <Cadenza ast="Y2R6YXN0AAEBFAIAAAEBAAE=" kind="expr">#list()</Cadenza> or a non-empty <Cadenza ast="Y2R6YXN0AAEEFAoBeAoCLi4KBHJlc3QGAAAAAQACAAMBAgIDAQMAAQQF" kind="expr">#list(x (.. rest))</Cadenza> that binds the first element to <C>x</C> and the remainder to <C>rest</C>, and thread the accumulator through:</P>
      <Runnable
        source={`(def (main) (sum-list #list(10 20 30) 0))

(def (sum-list xs acc) (match xs (#list() acc) (#list(x (.. rest)) (sum-list rest (+ acc x)))))`}
      />
      <P>The empty list is the base case (return the accumulator); the non-empty case adds the head to the accumulator and recurses on the tail. Building a value instead of a number is the identical move. Here it reverses a list by taking each element off the front and putting it on the <em>front</em> of the accumulator, so the first element read ends up deepest and the last read ends up first:</P>
      <Runnable
        source={`(def (main) (rev #list(1 2 3) #list()))

(def (rev xs acc) (match xs (#list() acc) (#list(x (.. rest)) (rev rest (List.prepend acc x)))))`}
      />
      <P>Prepending is what does the reversing: element <C>1</C> is placed first, then <C>2</C> goes in front of it, then <C>3</C> in front of that, so <Cadenza ast="Y2R6YXN0AAEEFAABAQABAgABAwUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(1 2 3)</Cadenza> comes back as <Cadenza ast="Y2R6YXN0AAEEFAABAwABAgABAQUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(3 2 1)</Cadenza>. <Ch to="/lists"> <C>List.prepend</C></Ch> adds an element to the front, which is what flips the order; appending each element to the end with <C>List.push</C> would instead copy the list unchanged. A quick <C>@test</C> pins it, reading the three positions of the result back and checking they spell <C>3</C>, <C>2</C>, <C>1</C> (as the single number <C>321</C>):</P>
      <Runnable
        source={`(def (rev xs acc) (match xs (#list() acc) (#list(x (.. rest)) (rev rest (List.prepend acc x)))))

(def (nth xs i) (match (List.at xs i) ((Some v) v) ((None _) 0)))

(@
  test
  (def
    (rev-reverses)
    (let
      ((r (rev #list(1 2 3) #list())))
      (assert-eq
        (+ (* 100 (nth r 0)) (+ (* 10 (nth r 1)) (nth r 2)))
        321
        "rev of (1 2 3) should read back as 3,2,1"))))`}
        mode="test"
      />
      <Note>Notice the recursive call is the <em>last</em> thing each step does: it sits in <em>tail position</em>. A recursion in tail position compiles to a loop. It reuses one stack frame rather than stacking a new one per element, so an accumulator over a long list runs in constant stack space. Threading the accumulator is what puts the call in tail position; a version that adds <em>after</em> the recursive call (<Cadenza ast="Y2R6YXN0AAEECgErCgF4CgNzdW0KBHJlc3QGAAAAAQACAAMBAgIDAQMAAQQF" kind="expr">(+ x (sum rest))</Cadenza>) does not, and you meet exactly that shape in the next chapter.</Note>
      <Why tenet="Repetition is a value, not a statement">A loop mutates state for effect; a recursive function <em>returns</em> the result of repeating. Making iteration an expression means every repetition has a value and a type, there is no mutable loop counter to get wrong, and the same tool, a function, does the job with no special loop syntax to learn. Uniformity over special cases, applied to the oldest control structure there is.</Why>
      <P>You will rarely write the accumulator by hand for long. The <em>fold</em> family packages exactly this pattern (a base value and a step that combines the running result with each element), so you state the step and let the traversal disappear. The next chapter, <Ch to="/lists">Lists</Ch>, puts recursion to work over sequences, and <Ch to="/iterators">Iterators</Ch> adds a lazy, on-demand layer on top: the fold vocabulary (<C>map</C>, <C>filter</C>, <C>fold</C>) built from the very mechanism you just saw.</P>
    </article>
  );
}
