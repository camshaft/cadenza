import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";

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
        Comparisons produce <C>Bool</C> values you can combine. Try wiring two comparisons together
        after reading the next chapters on data.
      </P>
      <Runnable source={`(if (= (+ 2 2) 4) 1 0)`} />
    </article>
  );
}
