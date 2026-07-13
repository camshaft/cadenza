import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function PatternMatching() {
  return (
    <article>
      <H1>Pattern matching</H1>
      <Lede>Deciding by shape — and why `match` is not a chain of `if`s.</Lede>

      <H2>Matching literals</H2>
      <P>
        A <C>match</C> chooses an arm by matching the scrutinee against each arm's <em>pattern</em>.
        The last arm here uses <C>_</C>, the wildcard, which matches anything.
      </P>
      <Runnable source={`(match 2
  (1 10)
  (2 20)
  (_ 0))`} />
      <P>Change the <C>2</C> being matched to <C>1</C> or <C>7</C> and Run to see a different arm fire.</P>

      <H2>Sum types</H2>
      <P>
        A sum type is a set of tagged variants. You declare it with <C>type</C>, construct a value
        with one of its constructors, and take it apart by matching each variant. Here <C>Opt</C> is
        either <C>Some</C> carrying an <C>Int64</C>, or <C>None</C>. (This example is a whole module,
        since it declares a type alongside <C>main</C>.)
      </P>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))
(def (main)
  (match (Some 7)
    ((Some x) x)
    ((None _) 0)))`}
      />
      <Note>
        A match arm's head is a <strong>pattern</strong> — a constructor that destructures, a literal,
        a binding, or <C>_</C> — never an arbitrary boolean test. Because the arms are patterns over
        the scrutinee's type, the compiler can check that you have covered every variant. That is why{" "}
        <C>match</C> is not a <C>cond</C> in disguise: a value <em>condition</em> is an <C>if</C>;{" "}
        <C>match</C> destructures a value's shape.
      </Note>

      <P>
        The <C>(Some x)</C> arm <em>binds</em> the payload to <C>x</C>. Swap <C>(Some 7)</C> for{" "}
        <C>(None unit)</C> in the code above and Run to take the other arm — it returns <C>0</C>.
      </P>

      <Why tenet="match is patterns, not predicates">
        Many languages let a branch head be any boolean test. Cadenza deliberately doesn't: a{" "}
        <C>match</C> arm is always a <em>pattern</em>. Why refuse the more flexible option? Because a
        head that could be an arbitrary predicate quietly demotes the real question —{" "}
        <em>"did you handle every variant?"</em> — down to <em>"is there an else?"</em>. Keeping arms as
        patterns is what lets the compiler check exhaustiveness against the type. Value conditions still
        have a home; it's just <C>if</C>, not <C>match</C>.
      </Why>
    </article>
  );
}
