// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Numbers() {
  return (
    <article>
      <H1>The numeric model</H1>
      <Lede>Checked integers, and no silent conversions between numeric types.</Lede>
      <H2>Checked Int64</H2>
      <P>Cadenza's core integer type is a checked <C>Int64</C>, a 64-bit signed integer. Ordinary arithmetic works as you'd expect, and the result carries its exact type:</P>
      <Runnable
        source={`(* 1000000 1000000)`}
      />
      <P>That's a trillion, <C>1000000000000</C>, comfortably inside a 64-bit integer's range. Keep pushing, though, and a product eventually won't fit.</P>
      <H2>Overflow is caught, not wrapped</H2>
      <P>What happens when a result is too big to fit? In many languages it silently <em>wraps around</em> to a wrong (often negative) answer. Cadenza refuses instead. <C>Int64</C>'s largest value times 2 can't fit, and the compiler says so rather than producing garbage:</P>
      <Note>This example is <strong>meant to be refused</strong>. Run it and read the status bar: the result can't fit an <C>Int64</C>, so the compiler declines rather than wrapping to a bogus value.</Note>
      <Runnable
        source={`(* 9223372036854775807 2)`}
        expect="error"
      />
      <P>Division by zero is the same story, since there's no correct answer, so it's caught, not left to produce a garbage result or a silent zero:</P>
      <Runnable
        source={`(/ 5 0)`}
        expect="error"
      />
      <H2>Division truncates; remainder picks up the rest</H2>
      <P>When it <em>can</em> divide, integer division keeps the whole part and throws away the fraction, so it truncates toward zero. So <C>17 / 5</C> is <C>3</C>, not <C>3.4</C>:</P>
      <Runnable
        source={`(/ 17 5)`}
      />
      <P>The piece that division discards is exactly what <C>%</C>, the remainder, keeps: <C>17 % 5</C> is <C>2</C>, because <C>17 = 5 × 3 + 2</C>. The two together recover the original.</P>
      <Runnable
        source={`(% 17 5)`}
      />
      <P>"Truncates toward zero" matters once a negative is involved: <C>-17 / 5</C> is <C>-3</C>, not <C>-4</C>, because the fraction is dropped, moving the result <em>toward</em> zero rather than down. The remainder follows so the identity still holds (<C>-17 = 5 × -3 + -2</C>), so <C>-17 % 5</C> is <C>-2</C>, so the remainder takes the sign of the dividend.</P>
      <Runnable
        source={`(/ -17 5)`}
      />
      <H2>Handling an overflow instead of halting</H2>
      <P>A bare <C>*</C> that overflows <em>declines</em>, so the whole program stops. Sometimes you'd rather <em>handle</em> the possibility: the checked operations do the same arithmetic but hand back an <C>Option</C>, namely <Cadenza ast="Y2R6YXN0AAECCgRTb21lCgF2AwAAAAEBAgABAg==" kind="expr">(Some v)</Cadenza> when it fits and <Cadenza ast="Y2R6YXN0AAECCgROb25lCgR1bml0AwAAAAEBAgABAg==" kind="expr">(None unit)</Cadenza> when it would overflow, so you decide what happens. Here <C>Int64.checked-mul</C> of two small numbers succeeds:</P>
      <Runnable
        source={`(match (Int64.checked-mul 6 7) ((Some v) v) ((None _) -1))`}
      />
      <P>And the overflow that made the bare <C>*</C> decline instead returns <C>None</C> here, so the <C>None</C> arm runs and the program keeps going, with <C>-1</C> standing in for "didn't fit":</P>
      <Runnable
        source={`(match (Int64.checked-mul 9223372036854775807 2) ((Some v) v) ((None _) -1))`}
      />
      <P>Same discipline, your choice of response: let it halt (the bare operator) or fold the failure into a value you handle (the checked operator). The <C>Option</C> shape is the subject of <strong>Errors &amp; absence</strong>.</P>
      <H2>Types don't mix by accident</H2>
      <P>Numeric and boolean values don't silently coerce into one another either. Ask the compiler to add a number and a boolean and it refuses, with a diagnostic pointing right at the mismatch, rather than inventing a conversion you didn't ask for:</P>
      <Runnable
        source={`(+ 1 true)`}
        expect="error"
      />
      <Why tenet="Refuse the ambiguity, don't guess">Many languages would quietly bridge these gaps by wrapping an overflow around, coercing a boolean to a number, or widening one numeric type into another. Cadenza refuses, because each convenience hides a real question: what did you actually mean? A conversion you didn't write, or a wrap you didn't ask for, is a decision the compiler made <em>for</em> you, and the classic source of "why is this number negative?" bugs. So an operation that can't produce a correct result <em>declines</em> with a diagnostic instead of guessing, and any conversion is something you write by name.</Why>
      <P>This is a running theme: an operation the compiler can't carry out correctly declines instead of guessing. When you'd rather <em>handle</em> an overflow than have it halt the program, that's what the checked operations in <strong>Errors &amp; absence</strong> are for, since they hand back an <C>Option</C> so you decide. And it's the same discipline that keeps integer widths from blurring, next in <strong>Sized integers</strong>.</P>
      <H2>Your turn</H2>
      <Exercise
        id="numbers:1"
        prompt={<>Integer division truncates toward zero. Divide <C>23</C> by <C>4</C>, filling in the divisor so the answer is <C>5</C> (23 ÷ 4 is 5 with 3 left over).</>}
        starter={`(/ 23 ?)`}
        solution={`(/ 23 4)`}
        expected="5"
        hint={<>23 ÷ 4 is 5 remainder 3; integer division keeps the whole part, <C>5</C>. The divisor is <C>4</C>.</>}
      />
      <Exercise
        id="numbers:2"
        prompt={<>Now the other half: use the remainder operator <C>%</C> to find what <C>23 / 4</C> left over. Division gave <C>5</C>; the remainder is <C>3</C>.</>}
        starter={`(? 23 4)`}
        solution={`(% 23 4)`}
        expected="3"
        hint={<>The remainder operator is <C>%</C>. Since <C>23 = 4 × 5 + 3</C>, <Cadenza ast="Y2R6YXN0AAEDCgElAAEXAAEEBAAAAAEAAgEDAAECAw==" kind="expr">(% 23 4)</Cadenza> is <C>3</C>.</>}
      />
    </article>
  );
}
