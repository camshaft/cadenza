import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Link } from "react-router-dom";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Rationals() {
  return (
    <article>
      <H1>Exact fractions</H1>
      <Lede>
        A <C>Float64</C> is fast but approximate. When you need <em>exact</em> fractions — a third that
        really is a third — reach for a <C>Rational</C>.
      </Lede>

      <P>
        You saw in <strong>Floating-point numbers</strong> that <C>(+ 0.1 0.2)</C> isn't quite{" "}
        <C>0.3</C> — floats trade exactness for speed. A <C>Rational</C> makes the other trade: it holds a
        number as an exact ratio of two integers, so arithmetic never rounds. Build one with{" "}
        <C>Rational.of</C>, giving a numerator and a denominator:
      </P>
      <Runnable source={`(Rational.of 1 2)`} />
      <P>
        The value comes back as <C>1/2</C>, tagged with its type — <C>1/2 : Rational</C> in the conventional
        surface, <C>(: 1/2 Rational)</C> in s-expressions. A whole number is just a denominator of one —{" "}
        <C>Rational.of-int</C> makes that explicit:
      </P>
      <Runnable source={`(Rational.of-int 5)`} />

      <H2>Writing one directly: the <C>R</C> suffix</H2>
      <P>
        Spelling out <C>Rational.of</C> every time is wordy when you already know the number. A decimal
        with an <C>R</C> suffix is a rational <em>literal</em> — the compiler reads the decimal exactly and
        converts it to a fraction, so <C>0.5R</C> is <C>1/2</C> and <C>1.25R</C> is <C>5/4</C>:
      </P>
      <Runnable source={`0.5R`} />
      <P>
        It's the very same value as the constructor — <C>0.5R</C> equals <C>(Rational.of 1 2)</C> — just
        terser to write. And it's where the contrast with <strong>Floating-point numbers</strong> stops
        being a claim and becomes something you can watch. Add a tenth and two tenths as <em>floats</em> and
        the answer isn't <C>0.3</C> — it's the nearest float to it, which isn't quite <C>0.3</C>:
      </P>
      <Runnable source={`(+ 0.1 0.2)`} />
      <P>
        <C>0.30000000000000004</C> — the drift is real, not hypothetical. So the natural test{" "}
        <em>fails</em>: the float sum is not equal to <C>0.3</C>.
      </P>
      <Runnable source={`(= (+ 0.1 0.2) 0.3)`} />
      <P>
        <C>false</C>. Now write the very same digits as rational literals. The sum is <em>exactly</em>{" "}
        <C>3/10</C>, so the same equality that failed for floats holds for rationals:
      </P>
      <Runnable source={`(= (+ 0.1R 0.2R) 0.3R)`} />
      <P>
        <C>true</C> — same digits you'd type for a float, one letter's difference, and now <C>0.1 + 0.2</C>{" "}
        is the number you <em>meant</em>. The float wasn't buggy; it was doing exactly what binary
        floating-point must. The <C>Rational</C> just makes the other trade — exactness over speed — so the
        arithmetic never rounds in the first place.
      </P>

      <H2>Whole numbers that outgrow Int64: <C>BigInt</C></H2>
      <P>
        The same instinct — trade speed for exactness when it matters — has a whole-number counterpart. An{" "}
        <C>Int64</C> refuses to hold a value past its range: <C>9223372036854775807 × 1000</C> overflows,
        and the compiler declines rather than wrap.
      </P>
      <Runnable source={`(* 9223372036854775807 1000)`} expect="error" />
      <P>
        When you genuinely need bigger, <C>BigInt</C> is the arbitrary-precision integer — it grows to fit
        any whole number. Build one with <C>BigInt.of</C> (or write the <C>N</C> literal suffix), and the
        product that overflowed an <C>Int64</C> is exact:
      </P>
      <Runnable source={`(* (BigInt.of 9223372036854775807) (BigInt.of 1000))`} />
      <P>
        The result comes back as a <C>BigInt</C> far beyond the 64-bit range — no overflow, no wrap. It's
        the same trade as <C>Rational</C>: reach for it when a value must be exact whatever its size, and
        pay for the arbitrary precision only where you asked for it.
      </P>

      <H2>Always in lowest terms</H2>
      <P>
        A rational normalizes itself on construction: it's stored in lowest terms, with the sign on the
        numerator. Ask for <C>2/4</C> and you get back <C>1/2</C> — the same number, canonically written:
      </P>
      <Runnable source={`(Rational.of 2 4)`} />
      <P>
        Because two rationals that denote the same number normalize identically, <C>=</C> compares them
        by <em>value</em>: <C>2/4</C> and <C>1/2</C> are equal, however you wrote them.
      </P>

      <H2>Arithmetic stays exact</H2>
      <P>
        <C>+</C>, <C>-</C>, <C>*</C>, and <C>/</C> over rationals compute the exact result and renormalize.
        Here's the sum floats can't get right — a third plus a third plus a third — and with rationals it
        is <em>exactly</em> one:
      </P>
      <Runnable
        source={`(+ (+ (Rational.of 1 3) (Rational.of 1 3)) (Rational.of 1 3))`}
      />
      <P>
        <C>1/1</C>, not <C>0.9999999999999999</C>. Division is exact too, and — unlike integer division —
        total for any nonzero divisor: <C>(3/4) / (2/1)</C> is <C>3/8</C>, no remainder, no rounding. You
        can try exact fractions yourself in the{" "}
        <Link to="/calculator" className="text-cadenza-300 underline-offset-2 hover:underline">
          calculator
        </Link>{" "}
        — type <C>1 / 3 + 1 / 3 + 1 / 3</C> and watch it come back <C>1</C>.
      </P>
      <Runnable source={`(/ (Rational.of 3 4) (Rational.of 2 1))`} />

      <Why tenet="Exactness is a choice you can make">
        Cadenza doesn't pick one number type and make its weaknesses your problem. A <C>Float64</C> is the
        right tool when you want speed and can tolerate rounding — measurements, graphics, physics. A{" "}
        <C>Rational</C> is the right tool when a rounding error would be a <em>bug</em> — money, exact
        ratios, anything that must add up. They're different types with different operators, so you say
        which guarantee you want, and the compiler never silently swaps one for the other. Same instinct
        as keeping <C>Int64</C> and <C>Float64</C> apart: one type per kind of number, no surprises.
      </Why>

      <Note>
        A zero denominator has no value to denote, so <C>(Rational.of 1 0)</C> is a compile-time error
        (<C>CDZ0304</C>) — the same "no correct answer, so refuse" rule as dividing an integer by zero.
      </Note>

      <P>
        That refuse-when-there's-no-answer instinct runs through every number type you've now met. The next
        chapter, <em>Errors &amp; absence</em>, makes it a tool you hold: <C>Option</C> and <C>Result</C>{" "}
        turn a might-not-have-an-answer into an ordinary value you handle.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="rationals:1"
        prompt={
          <>
            A rational is compared by value, so equal fractions are <C>=</C> however they're written.
            Write the fraction <C>3/6</C> in lowest terms so it equals <C>(Rational.of 1 2)</C> — the
            comparison then gives <C>1</C>.
          </>
        }
        starter={`(if (= (Rational.of 3 6) (Rational.of 1 ?)) 1 0)`}
        solution={`(if (= (Rational.of 3 6) (Rational.of 1 2)) 1 0)`}
        expected="1"
        hint={
          <>
            <C>3/6</C> reduces to <C>1/2</C>, so the denominator is <C>2</C>. Equal rationals compare
            <C>=</C>, giving <C>1</C>.
          </>
        }
      />

      <Exercise
        id="rationals:2"
        prompt={
          <>
            Division is exact — "how many quarters are in a half?" Divide <C>1/2</C> by a quarter so the
            result is <C>2/1</C>; fill in the divisor's denominator. When it's right the check gives{" "}
            <C>1</C>.
          </>
        }
        starter={`(if (= (/ (Rational.of 1 2) (Rational.of 1 ?)) (Rational.of 2 1)) 1 0)`}
        solution={`(if (= (/ (Rational.of 1 2) (Rational.of 1 4)) (Rational.of 2 1)) 1 0)`}
        expected="1"
        hint={
          <>
            A quarter is <C>(Rational.of 1 4)</C>. Dividing by it multiplies by its reciprocal <C>4/1</C>,
            so <C>1/2</C> becomes <C>4/2 = 2/1</C> — exactly two, no rounding.
          </>
        }
      />
    </article>
  );
}
