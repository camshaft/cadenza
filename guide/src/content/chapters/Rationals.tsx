// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { AppLink } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Rationals() {
  return (
    <article>
      <H1>Exact fractions</H1>
      <Lede>A <C>Float64</C> is fast but approximate, so when you need an exact fraction like a third that really is a third, reach for a <C>Rational</C>.</Lede>
      <P>You saw in <strong>Floating-point numbers</strong> that <Cadenza ast="Y2R6YXN0AAEDCgErBgD//////////wEBBgD//////////wECBAAAAAEAAgEDAAECAw==" kind="expr">(+ 0.1 0.2)</Cadenza> isn't quite <C>0.3</C>, because floats trade exactness for speed. A <C>Rational</C> makes the other trade by holding a number as an exact ratio of two integers, so arithmetic never rounds. Build one with <C>Rational.of</C>, giving a numerator and a denominator:</P>
      <Runnable
        source={`(Rational.of 1 2)`}
        id="rational-half"
      />
      <P>The value comes back as <C>1/2</C> tagged with its type, which reads <C>1/2 : Rational</C> in the conventional surface and <C>(: 1/2 Rational)</C> in s-expressions. Since a whole number is just a denominator of one, <C>Rational.of-int</C> makes that explicit:</P>
      <Runnable
        source={`(Rational.of-int 5)`}
      />
      <H2>Writing one directly: the <C>R</C> suffix</H2>
      <P>Spelling out <C>Rational.of</C> every time is wordy when you already know the number. A decimal with an <C>R</C> suffix is a rational <em>literal</em> that the compiler reads exactly and converts to a fraction, so <C>0.5R</C> is <C>1/2</C> and <C>1.25R</C> is <C>5/4</C>:</P>
      <Runnable
        source={`0.5R`}
      />
      <P>It's the very same value as the constructor, since <C>0.5R</C> equals <Cadenza ast="Y2R6YXN0AAEFGgoIUmF0aW9uYWwKAm9mAAEBAAECBwAAAAEAAgEDAAECAAMABAEDAwQFBg==" kind="expr">(Rational.of 1 2)</Cadenza>, just terser to write. And it's where the contrast with <strong>Floating-point numbers</strong> stops being a claim and becomes something you can watch. Add a tenth and two tenths as <em>floats</em> and the answer isn't <C>0.3</C> but the nearest float to it, which isn't quite <C>0.3</C>:</P>
      <Runnable
        source={`(+ 0.1 0.2)`}
      />
      <P><C>0.30000000000000004</C>, so the drift is real rather than hypothetical, and the natural test <em>fails</em> because the float sum is not equal to <C>0.3</C>.</P>
      <Runnable
        source={`(= (+ 0.1 0.2) 0.3)`}
      />
      <P><C>false</C>. Now write the very same digits as rational literals. The sum is <em>exactly</em> <C>3/10</C>, so the same equality that failed for floats holds for rationals:</P>
      <Runnable
        source={`(= (+ 0.1R 0.2R) 0.3R)`}
      />
      <P><C>true</C>, the same digits you'd type for a float with one letter's difference, and now <C>0.1 + 0.2</C> is the number you <em>meant</em>. The float wasn't buggy; it was doing exactly what binary floating-point must. The <C>Rational</C> simply makes the other trade of exactness over speed, so the arithmetic never rounds in the first place.</P>
      <H2>Whole numbers that outgrow Int64: <C>BigInt</C></H2>
      <P>The same instinct of trading speed for exactness when it matters has a whole-number counterpart. An <C>Int64</C> refuses to hold a value past its range, so <C>9223372036854775807 × 1000</C> overflows and the compiler declines rather than wrap.</P>
      <Runnable
        source={`(* 9223372036854775807 1000)`}
        expect="error"
      />
      <P>When you genuinely need bigger, <C>BigInt</C> is the arbitrary-precision integer that grows to fit any whole number. Build one with <C>BigInt.of</C> (or write the <C>N</C> literal suffix), and the product that overflowed an <C>Int64</C> is exact:</P>
      <Runnable
        source={`(* (BigInt.of 9223372036854775807) (BigInt.of 1000))`}
      />
      <P>The result comes back as a <C>BigInt</C> far beyond the 64-bit range with no overflow and no wrap. It's the same trade as <C>Rational</C>: reach for it when a value must be exact whatever its size, and pay for the arbitrary precision only where you asked for it.</P>
      <H2>Always in lowest terms</H2>
      <P>A rational normalizes itself on construction, stored in lowest terms with the sign on the numerator. Ask for <C>2/4</C> and you get back <C>1/2</C>, the same number canonically written:</P>
      <Runnable
        source={`(Rational.of 2 4)`}
      />
      <P>Because two rationals that denote the same number normalize identically, <C>=</C> compares them by <em>value</em>: <C>2/4</C> and <C>1/2</C> are equal, however you wrote them.</P>
      <H2>Taking a rational apart</H2>
      <P>Sometimes you want the two integers back out, to display a fraction or to feed its parts on somewhere. <C>Rational.numerator</C> and <C>Rational.denominator</C> hand them over, and because a rational is always stored in lowest terms, they give you the <em>reduced</em> pair, not whatever you happened to type. Ask <C>2/4</C> for its numerator and it's <C>1</C>, since the value is really <C>1/2</C>:</P>
      <Runnable
        source={`(Rational.numerator (Rational.of 2 4))`}
        id="rat-num"
      />
      <P>The denominator of that same <C>2/4</C> is <C>2</C>, completing the reduced <C>1/2</C>. Both come back as a <C>BigInt</C>, so a numerator or denominator that outgrows 64 bits is carried exactly like any other exact integer:</P>
      <Runnable
        source={`(Rational.denominator (Rational.of 2 4))`}
        id="rat-den"
      />
      <P>This is a clean way to <em>see</em> that arithmetic really did stay exact. Add a third three times and ask the result for its denominator: it's <C>1</C>, because the sum is exactly <C>1/1</C>, not a fraction a hair away from one.</P>
      <Runnable
        source={`(Rational.denominator (+ (+ (Rational.of 1 3) (Rational.of 1 3)) (Rational.of 1 3)))`}
        id="rat-den-sum"
      />
      <H2>Rational to a whole number</H2>
      <P>The numerator and denominator hand back the exact integer <em>parts</em>, each an unbounded <C>BigInt</C>. Sometimes you instead want the whole value <em>as</em> one integer at a boundary, a MIDI tick, an array index, a pixel, and that's a projection to a fixed <C>Int64</C>. There are four, differing only in how they handle a fraction: <C>truncate</C> drops toward zero, <C>floor</C> rounds toward negative infinity, <C>ceil</C> toward positive infinity, and <C>round</C> to the nearest (ties going away from zero). They agree on positive whole-ish values and diverge on negatives:</P>
      <Note><C>{"value    truncate  floor  ceil  round"}</C> <br /> <C>{"  7/2        3        3     4     4"}</C> <br /> <C>{" -7/2       -3       -4    -3    -4"}</C> <br /> <C>{"  7/3        2        2     3     2"}</C></Note>
      <P>The split to watch is on negatives: <C>truncate</C> of <C>-7/2</C> is <C>-3</C> (toward zero) while <C>floor</C> is <C>-4</C> (toward negative infinity). They only look the same on positives, so a sign change is where a wrong choice bites:</P>
      <Runnable
        source={`(Rational.truncate (Rational.of -7 2))`}
        id="rational-trunc"
      />
      <P>And <C>round</C> breaks a tie by going <em>away</em> from zero, so <C>5/2</C> rounds to <C>3</C>, not the <C>2</C> that banker's (nearest-even) rounding would give. Cadenza names the rule rather than letting you assume it, the same refusal to guess that runs through the numeric model. All four narrow to <C>Int64</C> and trap on overflow, never silently wrapping:</P>
      <Runnable
        source={`(Rational.round (Rational.of 5 2))`}
        id="rational-round"
      />
      <H2>Arithmetic stays exact</H2>
      <P><C>+</C>, <C>-</C>, <C>*</C>, and <C>/</C> over rationals compute the exact result and renormalize. Here's the sum floats can't get right, a third plus a third plus a third, and with rationals it is <em>exactly</em> one:</P>
      <Runnable
        source={`(+ (+ (Rational.of 1 3) (Rational.of 1 3)) (Rational.of 1 3))`}
        id="rat-sum"
      />
      <P><C>1/1</C>, not <C>0.9999999999999999</C>. Division is exact too, and unlike integer division it stays total for any nonzero divisor, so <C>(3/4) / (2/1)</C> is <C>3/8</C> with no remainder and no rounding. You can try exact fractions yourself in the <AppLink to="/calculator"> calculator </AppLink> by typing <C>1 / 3 + 1 / 3 + 1 / 3</C> and watching it come back <C>1</C>.</P>
      <Runnable
        source={`(/ (Rational.of 3 4) (Rational.of 2 1))`}
        id="rat-div"
      />
      <Why tenet="Exactness is a choice you can make">Cadenza doesn't pick one number type and make its weaknesses your problem. A <C>Float64</C> is the right tool when you want speed and can tolerate rounding, as in measurements, graphics, and physics. A <C>Rational</C> is the right tool when a rounding error would be a <em>bug</em>, as in money, exact ratios, and anything that must add up. They're different types with different operators, so you say which guarantee you want, and the compiler never silently swaps one for the other. Same instinct as keeping <C>Int64</C> and <C>Float64</C> apart: one type per kind of number, no surprises.</Why>
      <Note>A zero denominator has no value to denote, so <Cadenza ast="Y2R6YXN0AAEFGgoIUmF0aW9uYWwKAm9mAAEBAAAHAAAAAQACAQMAAQIAAwAEAQMDBAUG" kind="expr">(Rational.of 1 0)</Cadenza> is a compile-time error (<C>CDZ0304</C>), the same "no correct answer, so refuse" rule as dividing an integer by zero.</Note>
      <P>That refuse-when-there's-no-answer instinct runs through every number type you've now met. The next chapter, <em>Errors &amp; absence</em>, makes it a tool you hold: <C>Option</C> and <C>Result</C> turn a might-not-have-an-answer into an ordinary value you handle.</P>
      <H2>Your turn</H2>
      <Exercise
        id="rationals:1"
        prompt={<>A rational is compared by value, so equal fractions are <C>=</C> however they're written. Write the fraction <C>3/6</C> in lowest terms so it equals <Cadenza ast="Y2R6YXN0AAEFGgoIUmF0aW9uYWwKAm9mAAEBAAECBwAAAAEAAgEDAAECAAMABAEDAwQFBg==" kind="expr">(Rational.of 1 2)</Cadenza> and the comparison gives <C>true</C>.</>}
        starter={`(= (Rational.of 3 6) (Rational.of 1 ?))`}
        solution={`(= (Rational.of 3 6) (Rational.of 1 2))`}
        expected="true"
        hint={<><C>3/6</C> reduces to <C>1/2</C>, so the denominator is <C>2</C>. Equal rationals compare <C>=</C>, giving <C>true</C>.</>}
      />
      <Exercise
        id="rationals:2"
        prompt={<>Division is exact, so ask how many quarters are in a half. Divide <C>1/2</C> by a quarter so the result is <C>2/1</C>, filling in the divisor's denominator, and when it's right the check gives <C>true</C>.</>}
        starter={`(= (/ (Rational.of 1 2) (Rational.of 1 ?)) (Rational.of 2 1))`}
        solution={`(= (/ (Rational.of 1 2) (Rational.of 1 4)) (Rational.of 2 1))`}
        expected="true"
        hint={<>A quarter is <Cadenza ast="Y2R6YXN0AAEFGgoIUmF0aW9uYWwKAm9mAAEBAAEEBwAAAAEAAgEDAAECAAMABAEDAwQFBg==" kind="expr">(Rational.of 1 4)</Cadenza>. Dividing by it multiplies by its reciprocal <C>4/1</C>, so <C>1/2</C> becomes <C>4/2 = 2/1</C>, exactly two with no rounding.</>}
      />
      <Exercise
        id="rationals:3"
        prompt={<>A rational is stored in lowest terms, so its parts come back <em>reduced</em>: <C>6/8</C> is really <C>3/4</C>, so its numerator is <C>3</C>. Which accessor reads the top of the fraction, <C>numerator</C> or <C>denominator</C>? Fill in the blank so the check confirms the numerator is <C>3</C> and gives <C>true</C>.</>}
        starter={`(= (Rational.? (Rational.of 6 8)) (BigInt.of 3))`}
        solution={`(= (Rational.numerator (Rational.of 6 8)) (BigInt.of 3))`}
        expected="true"
        hint={<><C>6/8</C> reduces to <C>3/4</C>, whose numerator is <C>3</C>. <C>Rational.numerator</C> reads the top; <C>Rational.denominator</C> would give <C>4</C>. The accessor returns a <C>BigInt</C>, so it's compared against <Cadenza ast="Y2R6YXN0AAEEGgoGQmlnSW50CgJvZgABAwYAAAABAAIBAwABAgADAQIDBAU=" kind="expr">(BigInt.of 3)</Cadenza>.</>}
      />
    </article>
  );
}
