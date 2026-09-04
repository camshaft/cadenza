// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";
import { TryChange } from "../../components/TryChange.tsx";

export default function ControlFlow() {
  return (
    <article>
      <H1>Control flow</H1>
      <Lede>Choosing between values with <C>if</C>, combining conditions, and looping by recursion.</Lede>
      <H2>Conditionals</H2>
      <P>An <C>if</C> is an <em>expression</em>: it evaluates to one branch or the other and yields that value. It's a value choice, not a statement you run for its effect.</P>
      <Runnable
        source={`(if (< 3 5) 100 200)`}
        id="if-branch"
      />
      <P><TryChange example="if-branch" find="<" replace=">">Change <C>&lt;</C> to <C>&gt;</C></TryChange> and it runs again to take the other branch, giving <C>200</C>.</P>
      <P>Because an <C>if</C> produces a value and that value needs one type, both branches must agree. Ask for one branch that returns a number and another that returns a boolean, and the compiler refuses:</P>
      <Note>This one is <strong>meant to be refused</strong>. Run it and read the status bar: the branches can't have different types, because the <C>if</C> as a whole is a value.</Note>
      <Runnable
        source={`(if (< 1 2) 100 true)`}
        expect="error"
      />
      <Why tenet="if is an expression, so both branches share a type">That single-type rule isn't fussiness, because it's what lets <C>if</C> be used anywhere a value is expected, whether as a function argument, a let binding, or the arm of another <C>if</C>. There's no separate "statement if" you run for effect and "expression if" that yields a value, so none of the "one branch forgot to return" bugs that split brings can happen here. One form is always a value and always well-typed, so the type checker can vet it wherever it appears.</Why>
      <H2>Nesting choices</H2>
      <P>An <C>if</C> selects among more than two outcomes by nesting in the else position:</P>
      <Runnable
        source={`(let ((n 0)) (if (< n 0) -1 (if (= n 0) 0 1)))`}
        id="sign-of-n"
      />
      <P>This is the sign of <C>n</C>, giving <C>-1</C> if negative, <C>0</C> if zero, and <C>1</C> if positive. With <C>n = 0</C> the first test fails and the inner <Cadenza ast="Y2R6YXN0AAEDCgE9CgFuAAAEAAAAAQACAQMAAQID" kind="expr">(= n 0)</Cadenza> fires, so the answer is <C>0</C>. Change <C>n</C> to <C>-4</C> or <C>7</C> and Run to take a different branch.</P>
      <H2>Booleans compose and short-circuit</H2>
      <P>Comparisons produce <C>Bool</C> values, and <C>and</C>, <C>or</C>, <C>not</C> combine them. They <em>short-circuit</em>, so a false <C>and</C> never evaluates its right side. That's what makes a guard-then-use pattern safe, because here <C>safe</C> checks <C>n</C> isn't zero <em>before</em> dividing by it, so the division never runs when it would trap:</P>
      <Runnable
        source={`(def (safe n) (and (not (= n 0)) (> (/ 100 n) 5)))

(def (main) (safe 0))`}
        id="short-circuit"
      />
      <P>With <C>n = 0</C> the first test is false, so <Cadenza ast="Y2R6YXN0AAEDCgEvAAFkAAAEAAAAAQACAQMAAQID" kind="expr">(/ 100 0)</Cadenza> is skipped entirely and the whole thing renders <C>false</C>. Were <C>and</C> not short-circuiting, that division would trap.</P>
      <H2>Recursion</H2>
      <P>A function can call itself, and that's how you loop in Cadenza. A base case stops the recursion, and each step reduces toward it. Here <C>sm</C> sums the integers from <C>n</C> down to 0:</P>
      <Runnable
        source={`(def (sm n) (if (= n 0) 0 (+ n (sm (- n 1)))))

(def (main) (sm 5))`}
        id="recursion-sum"
      />
      <P><Cadenza ast="Y2R6YXN0AAECCgJzbQABBQMAAAABAQIAAQI=" kind="expr">(sm 5)</Cadenza> adds <C>5 + 4 + 3 + 2 + 1</C> down to the base case, giving <C>15</C>. Each call peels off <C>n</C> and recurses on <C>n - 1</C> until it reaches <C>0</C>.</P>
      <H2>Your turn</H2>
      <Exercise
        id="control-flow:1"
        prompt={<>Write <C>pow2</C>, which computes 2 to the <C>n</C>. Here <C>n</C> is just a <em>counter</em> that says how many times to double, while the doubling itself is always the same. Fill in the step so <Cadenza ast="Y2R6YXN0AAECCgRwb3cyAAEFAwAAAAEBAgABAg==" kind="expr">(pow2 5)</Cadenza> gives <C>32</C>.</>}
        starter={`(def (pow2 n) (if (= n 0) 1 ?))

(def (main) (pow2 5))`}
        solution={`(def (pow2 n) (if (= n 0) 1 (* 2 (pow2 (- n 1)))))

(def (main) (pow2 5))`}
        expected="32"
        hint={<>Unlike <C>sm</C> above, <C>n</C> doesn't appear in the step, since you just double the result of one fewer step with <Cadenza ast="Y2R6YXN0AAEGCgEqAAECCgRwb3cyCgEtCgFuAAEBCQAAAAEAAgADAAQABQEDAwQFAQICBgEDAAEHCA==" kind="expr">(* 2 (pow2 (- n 1)))</Cadenza>. Writing <C>(* n …)</C> by habit would give factorial, <C>120</C>, not <C>32</C>.</>}
      />
      <Exercise
        id="control-flow:2"
        prompt={<><C>fare</C> picks a ticket price by age tier: under 5 rides free (<C>0</C>), 65 and over pays <C>5</C>, everyone in between pays <C>10</C>. The free case is done; fill the hole with the <em>nested</em> <C>if</C> that decides between the adult and senior fares, so that <Cadenza ast="Y2R6YXN0AAECCgRmYXJlAAFGAwAAAAEBAgABAg==" kind="expr">(fare 70)</Cadenza> gives <C>5</C>.</>}
        starter={`(def (fare age) (if (< age 5) 0 ?))

(def (main) (fare 70))`}
        solution={`(def (fare age) (if (< age 5) 0 (if (< age 65) 10 5)))

(def (main) (fare 70))`}
        expected="5"
        hint={<>The hole is a second <C>if</C> in the else position, like the sign-of-<C>n</C> example above. In <Cadenza ast="Y2R6YXN0AAEGCgJpZgoBPAoDYWdlAAFBAAEKAAEFCAAAAAEAAgADAQMBAgMABAAFAQQABAUGBw==" kind="expr">{"(if (< age 65) 10 5)"}</Cadenza>, under 65 is the adult fare <C>10</C> and otherwise the senior <C>5</C>.</>}
      />
    </article>
  );
}
