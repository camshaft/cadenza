// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Floats() {
  return (
    <article>
      <H1>Floating-point numbers</H1>
      <Lede>Real-valued arithmetic with one set of operators dispatched on the operand type, so a mix with integers can never slip by unnoticed.</Lede>
      <P>Alongside the checked integers from <C>The numeric model</C>, Cadenza has floating-point numbers. A number written with a decimal point is a <C>Float64</C>, a 64-bit IEEE-754 value that most languages call a "double".</P>
      <Runnable
        source={`3.14`}
      />
      <H2>The same operators, dispatched on the type</H2>
      <P>You add, subtract, multiply, and divide floats with the <em>same</em> <C>+</C>, <C>-</C>, <C>*</C>, <C>/</C> operators as integers. There's no separate float operator to remember, because when both operands are floats <C>+</C> <em>is</em> floating-point addition, and when both are integers it's integer addition. The operand type decides which arithmetic you get.</P>
      <Runnable
        source={`(- 5.0 1.5)`}
      />
      <Runnable
        source={`(/ 7.0 2.0)`}
      />
      <P>Notice <Cadenza>(/ 7.0 2.0)</Cadenza> is <C>3.5</C>, which is real division because the operands are floats. Give <C>/</C> two whole numbers and the very same operator does the integer division you saw earlier. Same symbol; the values in your hands choose the meaning.</P>
      <H2>Floating-point is approximate, and honest about it</H2>
      <P>IEEE-754 floats can't represent every decimal exactly, and Cadenza doesn't pretend otherwise. The classic example: add a tenth and two tenths, and the result isn't quite three tenths.</P>
      <Runnable
        source={`(+ 0.1 0.2)`}
      />
      <P>That <C>0.30000000000000004</C> isn't a bug, since it's the true value of the nearest float to the sum, the same answer you'd get in any IEEE-754 language. Cadenza shows you the real number rather than rounding it away, so what you read is what your program actually computed.</P>
      <H2>Ints and floats never mix silently</H2>
      <P>Try to add an integer to a float and the compiler refuses, the same way it refuses to add a number and a boolean. There is no automatic widening from <C>Int64</C> to <C>Float64</C>. The rejection doesn't come from the operator naming a type; it comes from the two operands disagreeing.</P>
      <Note>This example is <strong>meant to be refused</strong>. Run it and read the diagnostic: the compiler declines with <C>CDZ0301</C> rather than inventing a conversion, and it suggests a one-token fix to make the two operands agree (here, dropping the <C>.0</C> so both are integers).</Note>
      <Runnable
        source={`(+ 2 2.0)`}
        expect="error"
      />
      <Why tenet="No silent promotion; refuse the ambiguity">Most languages quietly promote the <C>2</C> to <C>2.0</C> here. That convenience hides a real decision: converting an exact integer to an approximate float can lose information, and doing it automatically means the loss happens somewhere you never wrote. Cadenza makes you say it. There's just one <C>+</C>, and it requires both operands to be the <em>same</em> numeric type, so an integer and a float can't blur together by accident. A mismatch is a compile-time error pointing right at the spot, not a rounding surprise discovered in production.</Why>
      <H2>Converting on purpose</H2>
      <P>When you <em>do</em> want to turn an integer into a float, you ask for it by name with <C>Float64.of-int</C>. It's an ordinary function, visible and deliberate and exactly where you meant the conversion to happen.</P>
      <Runnable
        source={`(Float64.of-int 7)`}
      />
      <P>Now the number is a float, so it composes with float arithmetic using the same <C>*</C>, now multiplying two floats:</P>
      <Runnable
        source={`(* (Float64.of-int 3) 1.5)`}
      />
      <H2>A worked example</H2>
      <P>Putting it together, here's the area of a circle, all in floating-point. Edit the radius and Run.</P>
      <Runnable
        source={`(def (area (: r Float64)) (* 3.14159 (* r r)))

(def (main) (area 2.0))`}
      />
      <H2>Two widths, never mixed silently</H2>
      <P>There's a 32-bit float too, <C>Float32</C>, with its own <C>Float32.of-int</C>. It's a real, runnable value, and dividing two of them gives a <C>Float32</C> back, <C>7 ÷ 2 = 3.5</C>:</P>
      <Runnable
        source={`(/ (Float32.of-int 7) (Float32.of-int 2))`}
      />
      <P>And the two widths follow the same rule as everything else: they don't blend on their own. Add a <C>Float32</C> to a <C>Float64</C> and the compiler stops you, because a <C>Float32</C> and a <C>Float64</C> have different precision, so combining them is a conversion you must write, not one the language guesses:</P>
      <Note>This one is <strong>meant to be refused</strong>. Run it and the diagnostic is <C>CDZ0301</C>, "floating-point precisions differ", the same no-silent-widening rule that keeps <C>Int64</C> and <C>Float64</C> apart, now between the two float sizes.</Note>
      <Runnable
        source={`(+ (Float32.of-int 1) (Float64.of-int 2))`}
        expect="error"
      />
      <P>A float trades exactness for speed, and is honest about it. But when a rounding error would be a <em>bug</em>, as with money or exact ratios, you want the opposite trade. That's <em>exact fractions</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="floats:1"
        prompt={<>Finish the expression so it halves <C>9.0</C> to give <C>4.5</C>.</>}
        starter={`(/ 9.0 ?)`}
        solution={`(/ 9.0 2.0)`}
        expected="4.5"
        hint={<>Both operands must be floats for <C>/</C> to do real division, so the divisor is <C>2.0</C>, not <C>2</C> (a whole <C>2</C> would make it an int/float mix, which is refused).</>}
      />
      <Exercise
        id="floats:2"
        prompt={<>The two floats here must be the <em>same width</em> to divide. One operand is already a <C>Float32</C>, so convert the <C>5</C> at the matching width for <C>/</C> to work and <C>5 ÷ 2</C> to give <C>2.5</C>. Which of <C>Float32</C> / <C>Float64</C> goes in the blank?</>}
        starter={`(/ (Float?.of-int 5) (Float32.of-int 2))`}
        solution={`(/ (Float32.of-int 5) (Float32.of-int 2))`}
        expected="2.5"
        hint={<>The other operand is a <C>Float32</C>, and widths don't mix, so convert at <C>Float32</C> too. Pick <C>Float64</C> instead and the compiler declines (<C>CDZ0301</C>, precisions differ).</>}
      />
    </article>
  );
}
