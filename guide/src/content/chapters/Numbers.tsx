import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function Numbers() {
  return (
    <article>
      <H1>The numeric model</H1>
      <Lede>Checked integers, and no silent conversions between numeric types.</Lede>

      <H2>Checked Int64</H2>
      <P>
        Cadenza's core integer type is a checked <C>Int64</C>. Arithmetic that would overflow is
        defined behavior, not a silent wrap-around — the ordinary operators trap rather than quietly
        producing a wrong answer.
      </P>
      <Runnable source={`(* 1000 1000)`} />

      <H2>Types don't mix by accident</H2>
      <P>
        Numeric and boolean values don't silently coerce into one another. Ask the compiler to add a
        number and a boolean and it refuses — with a diagnostic pointing right at the mismatch, rather
        than inventing a conversion you didn't ask for.
      </P>
      <Note>
        This example is <strong>meant to be refused</strong>. Run it and read the diagnostic: the
        status bar shows the compiler declining, which is exactly the right outcome here.
      </Note>
      <Runnable source={`(+ 1 true)`} expect="error" />

      <Why tenet="No silent promotion — refuse the ambiguity">
        Many languages would quietly bridge a mismatch like this — coercing the boolean to a number, or
        widening one numeric type into another. Cadenza refuses, because that convenience hides a real
        question: what did you actually mean? A conversion you didn't write is a decision the compiler
        made <em>for</em> you. So mixing types is a compile-time error, and a conversion has to be
        something you asked for by name. The same instinct governs overflow: the ordinary <C>+</C> traps
        rather than wrapping around, and wrapping arithmetic is a separate operation you opt into — you
        never get modular arithmetic by accident.
      </Why>

      <P>
        This is a running theme in Cadenza: an operation the compiler can't carry out correctly{" "}
        <em>declines</em> with a diagnostic instead of guessing. You'll see the same discipline in the
        type system and in pattern matching — and, next, in how integers and floating-point numbers
        refuse to blur together.
      </P>
    </article>
  );
}
