import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function ControlFlow() {
  return (
    <article>
      <H1>Control flow</H1>
      <Lede>
        Choosing between values with <C>if</C>, combining conditions, and looping by recursion.
      </Lede>

      <H2>Conditionals</H2>
      <P>
        An <C>if</C> is an <em>expression</em>: it evaluates to one branch or the other and yields that
        value. It's a value choice, not a statement you run for its effect.
      </P>
      <Runnable source={`(if (< 3 5) 100 200)`} />
      <P>Change <C>&lt;</C> to <C>&gt;</C> and Run again to take the other branch.</P>

      <P>
        Because an <C>if</C> <em>produces</em> a value, that value needs one type — so both branches must
        agree. Ask for one branch that returns a number and another that returns a boolean, and the
        compiler refuses:
      </P>
      <Note>
        This one is <strong>meant to be refused</strong>. Run it and read the status bar: the branches
        can't have different types, because the <C>if</C> as a whole is a value.
      </Note>
      <Runnable source={`(if (< 1 2) 100 true)`} expect="error" />

      <Why tenet="if is an expression, so both branches share a type">
        That single-type rule isn't fussiness — it's what lets <C>if</C> be used anywhere a value is
        expected: as a function argument, a let binding, the arm of another <C>if</C>. There's no
        separate "statement if" you run for effect and "expression if" that yields a value, and none of
        the "one branch forgot to return" bugs that split brings. One form, always a value, always
        well-typed — so the type checker can vet it wherever it appears.
      </Why>

      <H2>Nesting choices</H2>
      <P>An <C>if</C> selects among more than two outcomes by nesting in the else position:</P>
      <Runnable source={`(let ((n 0))
  (if (< n 0) -1
      (if (= n 0) 0 1)))`} />
      <P>
        This is the sign of <C>n</C>: <C>-1</C> if negative, <C>0</C> if zero, <C>1</C> if positive. With{" "}
        <C>n = 0</C> the first test fails and the inner <C>(= n 0)</C> fires, so the answer is <C>0</C> —
        change <C>n</C> to <C>-4</C> or <C>7</C> and Run to take a different branch.
      </P>

      <H2>Booleans compose — and short-circuit</H2>
      <P>
        Comparisons produce <C>Bool</C> values, and <C>and</C>, <C>or</C>, <C>not</C> combine them. They{" "}
        <em>short-circuit</em>: a false <C>and</C> never evaluates its right side. That's what makes a
        guard-then-use pattern safe — here <C>safe</C> checks <C>n</C> isn't zero <em>before</em> dividing
        by it, so the division never runs when it would trap:
      </P>
      <Runnable
        source={`(def (safe n)
  (and (not (= n 0)) (> (/ 100 n) 5)))
(def (main) (if (safe 0) 1 0))`}
      />
      <P>
        With <C>n = 0</C> the first test is false, so <C>(/ 100 0)</C> is skipped entirely and the whole
        thing is <C>false</C> — <C>0</C>. Were <C>and</C> not short-circuiting, that division would trap.
      </P>

      <H2>Recursion</H2>
      <P>
        A function can call itself — that's how you loop in Cadenza. A base case stops the recursion, and
        each step reduces toward it. Here <C>sm</C> sums the integers from <C>n</C> down to 0:
      </P>
      <Runnable
        source={`(def (sm n)
  (if (= n 0) 0 (+ n (sm (- n 1)))))
(def (main) (sm 5))`}
      />
      <P>
        <C>(sm 5)</C> adds <C>5 + 4 + 3 + 2 + 1</C> down to the base case, giving <C>15</C>. Each call peels
        off <C>n</C> and recurses on <C>n - 1</C> until it reaches <C>0</C>.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="control-flow:1"
        prompt={
          <>
            Write <C>pow2</C>, which computes 2 to the <C>n</C>. Here <C>n</C> is just a <em>counter</em> —
            it says how many times to double, but the doubling itself is always the same. Fill in the step
            so <C>(pow2 5)</C> gives <C>32</C>.
          </>
        }
        starter={`(def (pow2 n)
  (if (= n 0) 1 ?))
(def (main) (pow2 5))`}
        solution={`(def (pow2 n)
  (if (= n 0) 1 (* 2 (pow2 (- n 1)))))
(def (main) (pow2 5))`}
        expected="32"
        hint={
          <>
            Unlike <C>sm</C> above, <C>n</C> doesn't appear in the step — you just double the result of one
            fewer step: <C>(* 2 (pow2 (- n 1)))</C>. (Write <C>(* n …)</C> by habit and you'd get factorial,
            <C>120</C>, not <C>32</C>.)
          </>
        }
      />

      <Exercise
        id="control-flow:2"
        prompt={<>Finish <C>in-range</C> so it returns <C>1</C> only when <C>x</C> is between 0 and 10 — here <C>(in-range 5)</C> should give <C>1</C>.</>}
        starter={`(def (in-range x)
  (if (and (< 0 x) ?) 1 0))
(def (main) (in-range 5))`}
        solution={`(def (in-range x)
  (if (and (< 0 x) (< x 10)) 1 0))
(def (main) (in-range 5))`}
        expected="1"
        hint={<>The second half of the <C>and</C> is the upper bound: <C>(&lt; x 10)</C>.</>}
      />
    </article>
  );
}
