// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Ordering() {
  return (
    <article>
      <H1>Comparison & ordering</H1>
      <Lede>Asking how two values relate, and the shape a total order takes.</Lede>
      <H2>The comparison operators</H2>
      <P>The six comparisons, <C>=</C>, <C>&lt;</C>, <C>&lt;=</C>, <C>&gt;</C>, <C>&gt;=</C>, and <C>not</C> for negation, each produce a <C>Bool</C>, so evaluating one gives you <C>true</C> or <C>false</C> directly:</P>
      <Runnable
        source={`(<= 5 5)`}
      />
      <P>Combine them with <C>and</C> / <C>or</C> to express a range check, as in "is 5 between 1 and 10?", which is again just a <C>Bool</C>:</P>
      <Runnable
        source={`(and (>= 5 1) (<= 5 10))`}
      />
      <H2>Building on comparison</H2>
      <P>Small decisions built from comparisons are a large part of everyday code. Here is <C>min</C>, and a <C>clamp</C> that keeps a value within a range:</P>
      <Runnable
        source={`(def (min a b) (if (< a b) a b))

(def (main) (min 8 3))`}
        id="min"
      />
      <Runnable
        source={`(def (clamp lo hi x) (if (< x lo) lo (if (> x hi) hi x)))

(def (main) (clamp 0 10 42))`}
        id="clamp"
      />
      <P><Cadenza ast="Y2R6YXN0AAEDCgNtaW4AAQgAAQMEAAAAAQACAQMAAQID" kind="expr">(min 8 3)</Cadenza> is <C>3</C>, the smaller of the two. And <Cadenza ast="Y2R6YXN0AAEECgVjbGFtcAAAAAEKAAEqBQAAAAEAAgADAQQAAQIDBA==" kind="expr">(clamp 0 10 42)</Cadenza> is <C>10</C>: 42 is past the upper bound, so <C>clamp</C> pulls it back to <C>10</C>, and feeding it a value already inside <C>0</C>–<C>10</C> gives that value back unchanged.</P>
      <H2>Three answers, not two</H2>
      <P>A single <C>&lt;</C> only tells you yes-or-no. But comparing two values really has <em>three</em> possible answers: less, equal, or greater. You <em>could</em> encode that as <C>-1</C> / <C>0</C> / <C>1</C> and nest a couple of <C>if</C>s:</P>
      <Runnable
        source={`(def (cmp a b) (if (< a b) -1 (if (= a b) 0 1)))

(def (main) (cmp 3 9))`}
      />
      <P>That works, but those numbers are a convention you have to remember, and nothing stops a caller from forgetting the <C>equal</C> case or inventing a meaningless <C>2</C>. Cadenza gives you the three answers as a <em>value</em> instead.</P>
      <H2>The <C>Ordering</C> value</H2>
      <P><C>Ordering.of</C> takes two values and returns an <C>Ordering</C>, a sum with exactly three variants, <C>Less</C>, <C>Equal</C>, and <C>Greater</C>. The value itself is the answer, a named case instead of a magic number. Here <C>3</C> is less than <C>9</C>, so you get <C>Less</C>:</P>
      <Runnable
        source={`(Ordering.of 3 9)`}
      />
      <P>You read an <C>Ordering</C> apart with <C>match</C> to act on each case, the same way you would any sum. Because it's a closed sum, the compiler holds you to all three variants: drop the <C>Greater</C> arm and Run, and instead of a value you get a compile-time error, <C>non-exhaustive match: pattern `Greater` not covered</C>:</P>
      <Note>This one is <strong>meant to be rejected</strong>. The point is that a forgotten case is caught when you write it, not discovered as a wrong answer in production.</Note>
      <Runnable
        source={`(match (Ordering.of 3 9) ((Less _) "less") ((Equal _) "equal"))`}
        expect="error"
      />
      <P><C>Ordering.of</C> is <em>generic</em>, so it works on any two values of the same type, not just numbers. Text compares in dictionary order, so <C>"apple"</C> comes before <C>"banana"</C>, giving <C>Less</C> again:</P>
      <Runnable
        source={`(Ordering.of "apple" "banana")`}
      />
      <H2>Ordered types can be keys</H2>
      <P><C>Bytes</C> is ordered too, lexicographically over its <em>unsigned</em> byte values, the same way Text compares in dictionary order. So <Cadenza ast="Y2R6YXN0AAEGGgoFQnl0ZXMKAm9mFAABAQABAgkAAAABAAIBAwABAgADAAQABQEDBAUGAQIDBwg=" kind="expr">(Bytes.of #list(1 2))</Cadenza> comes before <Cadenza ast="Y2R6YXN0AAEGGgoFQnl0ZXMKAm9mFAABAQABAwkAAAABAAIBAwABAgADAAQABQEDBAUGAQIDBwg=" kind="expr">(Bytes.of #list(1 3))</Cadenza>, decided at the first byte that differs:</P>
      <Runnable
        source={`(< (Bytes.of #list(1 2)) (Bytes.of #list(1 3)))`}
      />
      <P>Watch the <em>unsigned</em> part: a byte holding <C>128</C> is <em>greater</em> than one holding <C>127</C>, not less, because the bytes are compared as 0–255, never as signed numbers:</P>
      <Runnable
        source={`(> (Bytes.of #list(128)) (Bytes.of #list(127)))`}
      />
      <P>A <C>Map</C> key or a <C>Set</C> element needs two things: the collection compares keys for <em>equality</em> to find an entry, and it uses a <em>total order</em> to enumerate its contents in a stable, canonical sequence (so <C>Map.to-list</C> always yields the same order). <C>Bytes</C> now has both, so a byte string can be a key directly, and iterating the collection reads back in sorted key order. Here a <C>Map</C> keyed by <C>Bytes</C> finds its entry:</P>
      <Runnable
        source={`(do
  (def (main) (Map.lookup (Map.insert (Map.empty) (Bytes.of #list(1 2)) 42) (Bytes.of #list(1 2))))

  (export main))`}
        id="ord-lookup"
      />
      <P>The lookup returns <Cadenza ast="Y2R6YXN0AAECCgRTb21lAAEqAwAAAAEBAgABAg==" kind="expr">(Some 42)</Cadenza>: the second <C>Bytes</C> value compares equal to the key that was inserted, so the <C>Map</C> finds it. A value you can order is a value you can organize.</P>
      <H2>What can't be a key: a function</H2>
      <P>The flip side of that rule draws a sharp line. Keying needs equality and a total order, and a <em>function</em> has neither: two closures can compute the same results yet be different values, and there's no canonical way to compare or order them. So a function can't be a <C>Map</C> key or a <C>Set</C> element, and the compiler says so rather than inventing an answer. A <C>Set</C> of functions is rejected:</P>
      <Runnable
        source={`(do
  (def (main) (Set.len (Set.of #list((fn (x) (+ x 1))))))

  (export main))`}
        expect="error"
      />
      <P>The error is <C>CDZ0216</C>: <em>a value of function type … cannot be a map/set key</em>, since a function <em>has no canonical identity, so it is neither equatable nor orderable</em>. The check walks the whole key, not just its outer shape, so a function buried inside a tuple key or a list element is caught the same way. The fix the message points to is to key by a <em>value</em> instead: a field the closure captures, an id you assign, a tag, anything with the equality and order that a function lacks.</P>
      <Why tenet="A total order is a three-way answer">Returning <C>-1 / 0 / 1</C> works, but it leans on a convention and can't be enforced. Cadenza's <C>Ordering.of</C> yields the <C>Ordering</C> sum of <em>less</em>, <em>equal</em>, and <em>greater</em>, so the three cases have names and the compiler checks a caller handled <em>all</em> of them (you saw the missing-arm error above). There's no fourth nonsense value to guard against: what would <C>2</C>, or <C>true,true</C>, even mean? And it's one order for every type, whether numbers, text, or the rest, so sorting and lookup behave the same way everywhere, observed identically by every consumer.</Why>
      <H2>Your turn</H2>
      <Exercise
        id="ordering:1"
        prompt={<>Finish the predicate <C>outside</C> so it returns <C>true</C> when <C>x</C> is <em>below 0 or above 10</em>, and <C>false</C> in between. With <Cadenza ast="Y2R6YXN0AAECCgdvdXRzaWRlAAEPAwAAAAEBAgABAg==" kind="expr">(outside 15)</Cadenza> the answer is <C>true</C>.</>}
        starter={`(def (outside x) (or (< x 0) ?))

(def (main) (outside 15))`}
        solution={`(def (outside x) (or (< x 0) (> x 10)))

(def (main) (outside 15))`}
        expected="true"
        hint={<>The two ways to be outside are joined with <C>or</C>; the second is "above 10", namely <Cadenza ast="Y2R6YXN0AAEDCgE+CgF4AAEKBAAAAAEAAgEDAAECAw==" kind="expr">{"(> x 10)"}</Cadenza>. <C>15</C> is above 10, so the result is <C>true</C>.</>}
      />
      <Exercise
        id="ordering:2"
        prompt={<>Write <C>max</C> using <C>Ordering.of</C> and <C>match</C>, picking <C>a</C> when it's greater or equal, and <C>b</C> when <C>a</C> is less. <Cadenza ast="Y2R6YXN0AAEDCgNtYXgAAQgAAQMEAAAAAQACAQMAAQID" kind="expr">(max 8 3)</Cadenza> should give <C>8</C>.</>}
        starter={`(def (max a b) (match (Ordering.of a b) ((Less _) ?) ((Equal _) a) ((Greater _) a)))

(def (main) (max 8 3))`}
        solution={`(def (max a b) (match (Ordering.of a b) ((Less _) b) ((Equal _) a) ((Greater _) a)))

(def (main) (max 8 3))`}
        expected="8"
        hint={<>The <C>Less</C> arm is the one case where <C>a</C> is <em>not</em> the maximum, so return <C>b</C> there.</>}
      />
    </article>
  );
}
