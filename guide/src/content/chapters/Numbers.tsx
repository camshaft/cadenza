import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function Numbers() {
  return (
    <article>
      <H1>The numeric model</H1>
      <Lede>Checked integers, and no silent promotion between numeric types.</Lede>

      <H2>Checked Int64</H2>
      <P>
        Cadenza's core integer type is a checked <C>Int64</C>. Arithmetic that would overflow is
        defined, not undefined behavior.
      </P>
      <Runnable source={`(* 1000 1000)`} />

      <H2>No silent promotion</H2>
      <P>
        Numeric types do not silently coerce into one another. Mixing an <C>Int64</C> with a{" "}
        <C>Float64</C> is a compile-time rejection rather than a quietly-inserted conversion — the
        compiler would rather refuse than guess what you meant.
      </P>
      <Note>
        This example is <strong>meant to be refused</strong>. Run it and read the diagnostic — the
        guide's status bar shows the compiler declining, which is the correct outcome here.
      </Note>
      <Runnable source={`(+ 2 2.0)`} expect="error" />

      <P>
        This is a running theme in Cadenza: an operation the compiler cannot carry out correctly{" "}
        <em>declines</em> with a diagnostic instead of miscompiling. You will see the same discipline
        in the type system and pattern matching.
      </P>
    </article>
  );
}
