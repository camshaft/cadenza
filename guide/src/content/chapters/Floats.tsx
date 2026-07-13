import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Floats() {
  return (
    <article>
      <H1>Floating-point numbers</H1>
      <Lede>Real-valued arithmetic — with its own operators, so a mix with integers can never slip by unnoticed.</Lede>

      <P>
        Alongside the checked integers from <C>The numeric model</C>, Cadenza has floating-point
        numbers. A number written with a decimal point is a <C>Float64</C> — a 64-bit IEEE-754 value,
        the same kind of number most languages call a "double".
      </P>
      <Runnable source={`3.14`} />

      <H2>Floats have their own operators</H2>
      <P>
        Here's the part that surprises people coming from other languages: floating-point arithmetic
        uses <em>different operators</em> than integer arithmetic. You add floats with <C>+.</C>,
        subtract with <C>-.</C>, multiply with <C>*.</C>, and divide with <C>/.</C> — each one a{" "}
        <C>+</C>, <C>-</C>, <C>*</C>, <C>/</C> with a trailing dot.
      </P>
      <Runnable source={`(-. 5.0 1.5)`} />
      <Runnable source={`(/. 7.0 2.0)`} />
      <P>
        Notice <C>(/. 7.0 2.0)</C> is <C>3.5</C> — real division, not the integer division you'd get
        from whole numbers. The operator you choose <em>is</em> the type of arithmetic you get.
      </P>

      <H2>Floating-point is approximate — and honest about it</H2>
      <P>
        IEEE-754 floats can't represent every decimal exactly, and Cadenza doesn't pretend otherwise.
        The classic example: add a tenth and two tenths, and the result isn't quite three tenths.
      </P>
      <Runnable source={`(+. 0.1 0.2)`} />
      <P>
        That <C>0.30000000000000004</C> isn't a bug — it's the true value of the nearest float to the
        sum, the same answer you'd get in any IEEE-754 language. Cadenza shows you the real number
        rather than rounding it away, so what you read is what your program actually computed.
      </P>

      <H2>Ints and floats never mix silently</H2>
      <P>
        Try to add an integer to a float with the ordinary integer <C>+</C> and the compiler refuses —
        the same way it refuses to add a number and a boolean. There is no automatic widening from{" "}
        <C>Int64</C> to <C>Float64</C>.
      </P>
      <Note>
        This example is <strong>meant to be refused</strong>. Run it and read the diagnostic: the
        compiler declines with <C>CDZ0301</C> rather than inventing a conversion.
      </Note>
      <Runnable source={`(+ 2 2.0)`} expect="error" />

      <Why tenet="No silent promotion — refuse the ambiguity">
        Most languages quietly promote the <C>2</C> to <C>2.0</C> here. That convenience hides a real
        decision: converting an exact integer to an approximate float can lose information, and doing it
        automatically means the loss happens somewhere you never wrote. Cadenza makes you say it. Because
        integer and float arithmetic use <em>different operators</em> (<C>+</C> vs <C>+.</C>), the two
        number worlds can't blur together by accident — a mismatched operator is a compile-time error
        pointing right at the spot, not a rounding surprise discovered in production.
      </Why>

      <H2>Converting on purpose</H2>
      <P>
        When you <em>do</em> want to turn an integer into a float, you ask for it by name with{" "}
        <C>Float64.of-int</C>. It's an ordinary function — visible, deliberate, exactly where you meant
        the conversion to happen.
      </P>
      <Runnable source={`(Float64.of-int 7)`} />
      <P>
        Now the number is a float, so it composes with float arithmetic:
      </P>
      <Runnable source={`(*. (Float64.of-int 3) 1.5)`} />

      <H2>A worked example</H2>
      <P>
        Putting it together — the area of a circle, all in floating-point. Edit the radius and Run.
      </P>
      <Runnable
        source={`(def (area r)
  (*. 3.14159 (*. r r)))
(def (main) (area 2.0))`}
      />

      <Note>
        There's a 32-bit float too, <C>Float32</C>, with its own <C>Float32.of-int</C>. The two widths
        mirror the integer story: one family of operators, an explicit conversion between sizes, and
        never a silent one.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="floats:1"
        prompt={<>Finish the expression so it halves <C>9.0</C> — the result should be <C>4.5</C>.</>}
        starter={`(/. 9.0 ?)`}
        solution={`(/. 9.0 2.0)`}
        expected="4.5"
        hint={<>Division uses <C>/.</C>, and both operands must be floats — so the divisor is <C>2.0</C>, not <C>2</C>.</>}
      />

      <Exercise
        id="floats:2"
        prompt={<>Convert the integer <C>10</C> to a float and add <C>0.5</C> to it — the result should be <C>10.5</C>.</>}
        starter={`(+. (Float64.of-int 10) ?)`}
        solution={`(+. (Float64.of-int 10) 0.5)`}
        expected="10.5"
        hint={<>The blank is a float literal. Adding uses <C>+.</C>, and <C>Float64.of-int 10</C> is already the float <C>10.0</C>.</>}
      />
    </article>
  );
}
