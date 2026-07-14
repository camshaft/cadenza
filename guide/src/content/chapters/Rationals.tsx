import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
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
        You saw in <strong>Floating-point numbers</strong> that <C>(+. 0.1 0.2)</C> isn't quite{" "}
        <C>0.3</C> — floats trade exactness for speed. A <C>Rational</C> makes the other trade: it holds a
        number as an exact ratio of two integers, so arithmetic never rounds. Build one with{" "}
        <C>Rational.of</C>, giving a numerator and a denominator:
      </P>
      <Runnable source={`(Rational.of 1 2)`} />
      <P>
        The result prints as <C>1/2</C>. A whole number is just a denominator of one — <C>Rational.of-int</C>{" "}
        makes that explicit:
      </P>
      <Runnable source={`(Rational.of-int 5)`} />

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
        total for any nonzero divisor: <C>(3/4) / (2/1)</C> is <C>3/8</C>, no remainder, no rounding.
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
