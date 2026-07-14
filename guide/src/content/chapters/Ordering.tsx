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
      <P>
        <C>(min 8 3)</C> is <C>3</C>, the smaller of the two. And <C>(clamp 0 10 42)</C> is <C>10</C>: 42 is
        past the upper bound, so <C>clamp</C> pulls it back to <C>10</C> — feed it a value already inside{" "}
        <C>0</C>–<C>10</C> and you get that value back unchanged.
      </P>

      <H2>Three answers, not two</H2>
      <P>
        A single <C>&lt;</C> only tells you yes-or-no. But comparing two values really has{" "}
        <em>three</em> possible answers: less, equal, or greater. You <em>could</em> encode that as{" "}
        <C>-1</C> / <C>0</C> / <C>1</C> and nest a couple of <C>if</C>s:
      </P>
      <Runnable
        source={`(def (cmp a b)
  (if (< a b) (- 0 1)
      (if (= a b) 0 1)))
(def (main) (cmp 3 9))`}
      />
      <P>
        That works, but those numbers are a convention you have to remember, and nothing stops a caller
        from forgetting the <C>equal</C> case or inventing a meaningless <C>2</C>. Cadenza gives you the
        three answers as a <em>value</em> instead.
      </P>

      <H2>The <C>Ordering</C> value</H2>
      <P>
        <C>compare</C> takes two values and returns an <C>Ordering</C> — a sum with exactly three
        variants, <C>Less</C>, <C>Equal</C>, and <C>Greater</C>. You read it apart with <C>match</C>, the
        same way you would any sum. Here <C>3</C> is less than <C>9</C>, so the <C>Less</C> arm fires:
      </P>
      <Runnable
        source={`(def (order-sign a b)
  (match (compare a b)
    ((Less _)    (- 0 1))
    ((Equal _)   0)
    ((Greater _) 1)))
(def (main) (order-sign 3 9))`}
      />
      <P>
        Now the three cases have names, not magic numbers — and because <C>Ordering</C> is a closed sum,
        the compiler holds you to all three. Delete the <C>Greater</C> arm and Run: instead of a value
        you get a compile-time error, <C>non-exhaustive match: pattern `Greater` not covered</C>:
      </P>
      <Note>
        This one is <strong>meant to be rejected</strong>. The point is that a forgotten case is caught
        when you write it, not discovered as a wrong answer in production.
      </Note>
      <Runnable
        source={`(match (compare 3 9)
  ((Less _)  1)
  ((Equal _) 0))`}
        expect="error"
      />

      <P>
        <C>compare</C> is <em>generic</em> — it works on any two values of the same type, not just
        numbers. Text compares in dictionary order, so <C>"apple"</C> comes before <C>"banana"</C> and
        the <C>Less</C> arm fires again:
      </P>
      <Runnable
        source={`(def (order-sign a b)
  (match (compare a b)
    ((Less _)    (- 0 1))
    ((Equal _)   0)
    ((Greater _) 1)))
(def (main) (order-sign "apple" "banana"))`}
      />

      <Why tenet="A total order is a three-way answer">
        Returning <C>-1 / 0 / 1</C> works, but it leans on a convention and can't be enforced. Cadenza's{" "}
        <C>compare</C> yields the <C>Ordering</C> sum — <em>less</em>, <em>equal</em>, <em>greater</em> —
        so the three cases have names and the compiler checks a caller handled <em>all</em> of them
        (you saw the missing-arm error above). There's no fourth nonsense value to guard against: what
        would <C>2</C>, or <C>true,true</C>, even mean? And it's one order for every type — numbers, text,
        and the rest — so sorting and lookup behave the same way everywhere, observed identically by every
        consumer.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="ordering:1"
        prompt={
          <>
            Finish <C>outside</C> so it returns <C>1</C> when <C>x</C> is <em>below 0 or above 10</em>,
            and <C>0</C> in between. With <C>(outside 15)</C> the answer is <C>1</C>.
          </>
        }
        starter={`(def (outside x)
  (if (or (< x 0) ?) 1 0))
(def (main) (outside 15))`}
        solution={`(def (outside x)
  (if (or (< x 0) (> x 10)) 1 0))
(def (main) (outside 15))`}
        expected="1"
        hint={
          <>
            The two ways to be outside are joined with <C>or</C>; the second is "above 10" —{" "}
            <C>(&gt; x 10)</C>. <C>15</C> is above 10, so the result is <C>1</C>.
          </>
        }
      />

      <Exercise
        id="ordering:2"
        prompt={
          <>
            Write <C>max</C> using <C>compare</C> and <C>match</C> — pick <C>a</C> when it's greater or
            equal, and <C>b</C> when <C>a</C> is less. <C>(max 8 3)</C> should give <C>8</C>.
          </>
        }
        starter={`(def (max a b)
  (match (compare a b)
    ((Less _)    ?)
    ((Equal _)   a)
    ((Greater _) a)))
(def (main) (max 8 3))`}
        solution={`(def (max a b)
  (match (compare a b)
    ((Less _)    b)
    ((Equal _)   a)
    ((Greater _) a)))
(def (main) (max 8 3))`}
        expected="8"
        hint={
          <>
            The <C>Less</C> arm is the one case where <C>a</C> is <em>not</em> the maximum — so return{" "}
            <C>b</C> there.
          </>
        }
      />
    </article>
  );
}
