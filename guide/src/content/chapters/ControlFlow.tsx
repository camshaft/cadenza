import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";

export default function ControlFlow() {
  return (
    <article>
      <H1>Control flow</H1>
      <Lede>Choosing between values with `if`, and combining conditions.</Lede>

      <H2>Conditionals</H2>
      <P>
        An <C>if</C> is an expression: it evaluates to one branch or the other, and both branches must
        produce the same type. It is a <em>value</em> choice, not a statement.
      </P>
      <Runnable source={`(if (< 3 5) 100 200)`} />

      <P>Change <C>&lt;</C> to <C>&gt;</C> and Run again to take the other branch.</P>

      <H2>Nesting choices</H2>
      <P>An <C>if</C> can select among more than two outcomes by nesting in the else position:</P>
      <Runnable source={`(let ((n 0))
  (if (< n 0) -1
      (if (= n 0) 0 1)))`} />

      <H2>Booleans compose</H2>
      <P>
        Comparisons produce <C>Bool</C> values, and <C>and</C>, <C>or</C>, and <C>not</C> combine
        them (short-circuiting, so a false <C>and</C> never evaluates its right side).
      </P>
      <Runnable source={`(if (and (< 1 2) (< 2 3)) 100 0)`} />

      <H2>Recursion</H2>
      <P>
        A function can call itself. This is how you loop in Cadenza: a base case stops the recursion,
        and each step reduces toward it. Here <C>sm</C> sums the integers from <C>n</C> down to 0.
      </P>
      <Runnable
        wrap={false}
        source={`(module m
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`}
      />

      <H2>Your turn</H2>
      <Exercise
        prompt={<>Fix the base case so <C>sm</C> correctly sums 1..5 to <C>15</C>.</>}
        starter={`(module m
  (def (sm n)
    (if (= n 0) ? (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`}
        solution={`(module m
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`}
        expected="15"
        wrap={false}
        hint={<>When <C>n</C> reaches 0, there is nothing left to add — the sum of no numbers is <C>0</C>.</>}
      />
    </article>
  );
}
