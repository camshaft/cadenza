import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Numbers() {
  return (
    <article>
      <H1>The numeric model</H1>
      <Lede>Checked integers, and no silent conversions between numeric types.</Lede>

      <H2>Checked Int64</H2>
      <P>
        Cadenza's core integer type is a checked <C>Int64</C> — a 64-bit signed integer. Ordinary
        arithmetic works as you'd expect, and the result carries its exact type:
      </P>
      <Runnable source={`(* 1000000 1000000)`} />

      <H2>Overflow is caught, not wrapped</H2>
      <P>
        What happens when a result is too big to fit? In many languages it silently <em>wraps around</em>{" "}
        to a wrong (often negative) answer. Cadenza refuses instead. <C>Int64</C>'s largest value times 2
        can't fit — and the compiler says so rather than producing garbage:
      </P>
      <Note>
        This example is <strong>meant to be refused</strong>. Run it and read the status bar: the result
        can't fit an <C>Int64</C>, so the compiler declines rather than wrapping to a bogus value.
      </Note>
      <Runnable source={`(* 9223372036854775807 2)`} expect="error" />
      <P>
        Division by zero is the same story — there's no correct answer, so it's caught, not left to
        produce a garbage result or a silent zero:
      </P>
      <Runnable source={`(/ 5 0)`} expect="error" />

      <H2>Division truncates; remainder picks up the rest</H2>
      <P>
        When it <em>can</em> divide, integer division keeps the whole part and throws away the fraction —
        it truncates toward zero. So <C>17 / 5</C> is <C>3</C>, not <C>3.4</C>:
      </P>
      <Runnable source={`(/ 17 5)`} />
      <P>
        The piece that division discards is exactly what <C>%</C>, the remainder, keeps: <C>17 % 5</C> is{" "}
        <C>2</C>, because <C>17 = 5 × 3 + 2</C>. The two together recover the original.
      </P>
      <Runnable source={`(% 17 5)`} />

      <H2>Types don't mix by accident</H2>
      <P>
        Numeric and boolean values don't silently coerce into one another either. Ask the compiler to
        add a number and a boolean and it refuses — with a diagnostic pointing right at the mismatch,
        rather than inventing a conversion you didn't ask for:
      </P>
      <Runnable source={`(+ 1 true)`} expect="error" />

      <Why tenet="Refuse the ambiguity, don't guess">
        Many languages would quietly bridge these gaps — wrapping an overflow around, coercing a boolean
        to a number, widening one numeric type into another. Cadenza refuses, because each convenience
        hides a real question: what did you actually mean? A conversion you didn't write, or a wrap you
        didn't ask for, is a decision the compiler made <em>for</em> you — and the classic source of
        "why is this number negative?" bugs. So an operation that can't produce a correct result{" "}
        <em>declines</em> with a diagnostic instead of guessing, and any conversion is something you
        write by name.
      </Why>

      <P>
        This is a running theme: an operation the compiler can't carry out correctly declines instead of
        guessing. When you'd rather <em>handle</em> an overflow than have it halt the program, that's
        what the checked operations in <strong>Errors &amp; absence</strong> are for — they hand back an{" "}
        <C>Option</C> so you decide. And it's the same discipline that keeps integer widths from blurring,
        next in <strong>Sized integers</strong>.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="numbers:1"
        prompt={<>Integer division truncates toward zero. What is <C>17 / 5</C>? Fill in the divisor so the answer is <C>3</C>.</>}
        starter={`(/ 17 ?)`}
        solution={`(/ 17 5)`}
        expected="3"
        hint={<>17 ÷ 5 is 3 remainder 2; integer division keeps the whole part, <C>3</C>. The divisor is <C>5</C>.</>}
      />

      <Exercise
        id="numbers:2"
        prompt={
          <>
            Use the remainder operator <C>%</C> to find what's left when <C>17</C> is divided by <C>5</C>.
            Division gave <C>3</C>; the remainder is <C>2</C>.
          </>
        }
        starter={`(? 17 5)`}
        solution={`(% 17 5)`}
        expected="2"
        hint={
          <>
            The remainder operator is <C>%</C>. Since <C>17 = 5 × 3 + 2</C>, <C>(% 17 5)</C> is <C>2</C>.
          </>
        }
      />
    </article>
  );
}
