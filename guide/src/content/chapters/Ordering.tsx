import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Ordering() {
  return (
    <article>
      <H1>Comparison &amp; ordering</H1>
      <Lede>Asking how two values relate — and the shape a total order takes.</Lede>

      <H2>The comparison operators</H2>
      <P>
        The six comparisons — <C>=</C>, <C>&lt;</C>, <C>&lt;=</C>, <C>&gt;</C>, <C>&gt;=</C>, and{" "}
        <C>not</C> for negation — each produce a <C>Bool</C>. They're most at home as the condition of
        an <C>if</C>:
      </P>
      <Runnable source={`(if (<= 5 5) 1 0)`} />

      <P>
        Combine them with <C>and</C> / <C>or</C> to express a range check — "is 5 between 1 and 10?":
      </P>
      <Runnable source={`(if (and (>= 5 1) (<= 5 10)) 1 0)`} />

      <H2>Building on comparison</H2>
      <P>
        Small decisions built from comparisons are the bread and butter of everyday code. Here is{" "}
        <C>min</C>, and a <C>clamp</C> that keeps a value within a range:
      </P>
      <Runnable
        source={`(def (min a b) (if (< a b) a b))
(def (main) (min 8 3))`}
      />
      <Runnable
        source={`(def (clamp lo hi x)
  (if (< x lo) lo
      (if (> x hi) hi x)))
(def (main) (clamp 0 10 42))`}
      />

      <H2>Three answers, not two</H2>
      <P>
        A single <C>&lt;</C> only tells you yes-or-no. But comparing two values really has{" "}
        <em>three</em> possible answers: less, equal, or greater. A function that returns all three at
        once — here as <C>-1</C>, <C>0</C>, <C>1</C> — lets a caller decide once and handle every case:
      </P>
      <Runnable
        source={`(def (cmp a b)
  (if (< a b) (- 0 1)
      (if (= a b) 0 1)))
(def (main) (cmp 3 9))`}
      />

      <Why tenet="A total order is a three-way answer">
        Returning <C>-1 / 0 / 1</C> works, but Cadenza's design goes one better: comparison yields a
        small sum value with three named variants — <em>less</em>, <em>equal</em>, <em>greater</em> —
        that you take apart with <C>match</C>. Why a sum instead of a number or a pair of booleans?
        Because then the compiler can check that a caller handled <em>all three</em> cases, and there's
        no fourth nonsense value (what would <C>2</C>, or <C>true,true</C>, even mean?). One comparison,
        three cases, exhaustively handled.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="ordering:1"
        prompt={<>Finish <C>max</C> so <C>(max 8 3)</C> gives <C>8</C> — the mirror of <C>min</C>.</>}
        starter={`(def (max a b) (if ? a b))
(def (main) (max 8 3))`}
        solution={`(def (max a b) (if (> a b) a b))
(def (main) (max 8 3))`}
        expected="8"
        hint={<>Keep <C>a</C> when it's the larger one: <C>(&gt; a b)</C>.</>}
      />

      <Note>
        Comparison is defined for more than just numbers — the language surfaces one total order that
        every consumer observes the same way, so sorting and ordering behave consistently across types.
      </Note>
    </article>
  );
}
