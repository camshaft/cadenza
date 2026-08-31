// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { AppLink } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Units() {
  return (
    <article>
      <H1>Units of measure</H1>
      <Lede>A number rarely means anything on its own. <C>5.0</C> is five <em>what</em>? Meters, seconds, dollars? Cadenza lets you carry the unit along with the value, checks that you never add a length to a time, and then erases the whole apparatus before the program runs. It costs nothing.</Lede>
      <P>A <strong>quantity</strong> is a number paired with a unit. You build one with <C>Qty.of</C>, giving it a value and a unit, and the quantity <em>carries that unit as part of its value</em>. Run this and the result comes back tagged with its unit, <C>5.0 meter</C>. The <C>meter</C> travels with the <C>5.0</C>:</P>
      <Runnable
        source={`(Qty.of 5.0 (Unit.of #"meter"))`}
      />
      <P>When you want the bare number back, to hand it to something that doesn't speak units, <C>Qty.value</C> strips the unit off and gives you just the <C>5.0</C>:</P>
      <Runnable
        source={`(Qty.value (Qty.of 5.0 (Unit.of #"meter")))`}
      />
      <H2>The same dimension converts on its own</H2>
      <P>A kilometer and a meter are two units of the one dimension <em>length</em>. Combining them is well-formed even though they differ: each carries an exact scale to its dimension's reference, so Cadenza converts and adds them for you. One kilometer plus five hundred meters is fifteen hundred meters:</P>
      <Runnable
        source={`(+ (Qty.of 1.0 (Unit.of #"kilometer")) (Qty.of 500.0 (Unit.of #"meter")))`}
      />
      <P>The result is a quantity, <C>1500.0 meter</C>, carried with its unit and reported at the dimension's reference unit (meters), the kilometer scaled in on the way. This is the one place Cadenza converts a number for you without being asked, and it earns the exception by being <em>exact</em>: kilometer-to-meter is times a thousand, no rounding, no guess. A mix of different <em>dimensions</em> gets no such courtesy, as we'll see in a moment.</P>
      <P>Because the quantity surface is meant to read like English, common plurals resolve to their singular unit. <C>#"feet"</C> is just <C>#"foot"</C>, the same member of the length family. So one meter plus four feet is well-formed and converts exactly, to <C>2.2192 meter</C>:</P>
      <Runnable
        source={`(+ (Qty.of 1.0 (Unit.of #"meter")) (Qty.of 4.0 (Unit.of #"feet")))`}
      />
      <Note>Toggle to the conventional surface and the second quantity reads <C>4.0 feet</C>. The plural survives in the surface even though it means the singular unit underneath. Write <C>#"foot"</C> instead and you'd get the identical result.</Note>
      <H2>Converting on purpose</H2>
      <P>When you want a specific unit out, convert to it with <C>Unit.in</C>: name the target unit first, then the quantity. Two kilometers <em>in meters</em>:</P>
      <Runnable
        source={`(Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"kilometer")))`}
      />
      <P>The result is the bare number <C>2000</C>. Converting <em>into</em> a unit is the deliberate exit from the quantity world: you asked "how many meters?" and get the plain number of meters, ready for ordinary arithmetic. There's no <C>Qty.value</C> to strip because <C>Unit.in</C> already hands back the number. Toggle to the ML surface and it reads as a plain postfix, <C>2.0 kilometer as meter</C>. It runs both ways across a scale: two hundred fifty milliseconds in seconds is a quarter of a second (<C>0.25</C>):</P>
      <Runnable
        source={`(Unit.in (Unit.of #"second") (Qty.of 250.0 (Unit.of #"millisecond")))`}
      />
      <H2>Different dimensions do not mix</H2>
      <P>Here is where the safety shows up. A length and a time share nothing. There is no exact factor between meters and seconds, so adding them is a mistake, and the compiler says so <em>before</em> the program ever runs:</P>
      <Note>This one is <strong>meant to be rejected</strong>. Run it and read the diagnostic: it names both dimensions and refuses. That's the feature working: a units bug is a compile error, never a wrong answer at runtime.</Note>
      <Runnable
        source={`(+ (Qty.of 1.0 (Unit.of #"meter")) (Qty.of 1.0 (Unit.of #"second")))`}
        expect="error"
      />
      <P>The same guard covers conversion: asking for a length <C>Unit.in</C> a time is the same category error, caught the same way.</P>
      <H2>Dimensions compose</H2>
      <P>Units aren't just labels you match; they combine. Divide a distance by a time and you get a speed, and the units divide right along with the numbers. Two hundred forty meters over eight seconds is thirty meters per second:</P>
      <Runnable
        source={`(/ (Qty.of 240.0 (Unit.of #"meter")) (Qty.of 8.0 (Unit.of #"second")))`}
      />
      <P>The result carries its derived unit, <C>30.0 meter/second</C>, a compound Cadenza built by dividing the two units right along with the numbers. You can spell such units directly, too: <C>(Unit.* a b)</C> for a product (an area is a length times a length), <C>(Unit./ a b)</C> for a quotient, <C>(Unit.^ u n)</C> for a power, and <C>Unit.one</C> for a plain dimensionless number.</P>
      <H2>Raising a quantity to a power</H2>
      <P><C>Qty.pow</C> raises a whole quantity, value <em>and</em> unit, to a compile-time integer power. Square a length and you get an area: the unit becomes meters-squared while the value squares. A five-meter side gives twenty-five square meters, <C>25.0 meter^2</C>, the unit squared along with the value:</P>
      <Runnable
        source={`(Qty.pow (Qty.of 5.0 (Unit.of #"meter")) 2)`}
      />
      <P>The exponent can be <em>negative</em>, and that's where units earn their keep. A period is a time, and its <em>reciprocal</em> is a frequency, with the unit inverted to per-second. A four-second period is a frequency of <C>0.25 1/second</C>:</P>
      <Runnable
        source={`(Qty.pow (Qty.of 4.0 (Unit.of #"second")) -1)`}
      />
      <P>That <C>-1</C> didn't just divide the number; it flipped the dimension from <em>time</em> to <em>per-time</em>. Try to add the result to a plain length and you'll get the same CDZ0501 you saw above: a frequency and a length are different dimensions, and the compiler knows it.</P>
      <H2>Prefixes: SI and binary</H2>
      <P>Most scaled units already have a name: <C>kilometer</C>, <C>millisecond</C>, <C>mebibyte</C> are units of their families just like <C>meter</C> is, so you write them the same way and everything above (mixing, converting, comparing) just works. The names carry two prefix systems that matter and must not blur together: SI <em>decimal</em> prefixes step by powers of ten (<C>kilobyte</C> is 1000 bytes), while the IEC <em>binary</em> prefixes step by powers of two (<C>mebibyte</C> is 2²⁰). One mebibyte is exactly 1&nbsp;048&nbsp;576 bytes, not a million:</P>
      <Runnable
        source={`(Unit.in (Unit.of #"byte") (Qty.of 1.0 (Unit.of #"mebibyte")))`}
      />
      <P>That's the distinction a units layer is <em>for</em>: the kind that quietly turns a 1&nbsp;MiB buffer into a 1&nbsp;MB one in someone's head. <C>kibibyte</C> and <C>kilobyte</C> are different names with different exact factors, and the arithmetic can't blur them.</P>
      <Note>Need a prefix on a unit that has no ready-made name? <C>Unit.prefix</C> applies one to any unit. <C>(Unit.prefix milli (Unit.of #"meter"))</C> is the millimeter and <C>(Unit.prefix mebi …)</C> the mebi- scale, the general mechanism the named units above are built from.</Note>
      <H2>Declaring your own units</H2>
      <P>The built-in table doesn't have to be the end of it. <C>Unit.define</C> introduces a new named unit as an exact multiple of one you already have: a name, the unit to build on, and a ratio (numerator then denominator). A furlong is 660 feet, so once you've defined it, it converts like any other unit. One furlong is <C>201.168</C> meters:</P>
      <Runnable
        source={`(Unit.define #"furlong" (Unit.of #"foot") 660 1)

(def (main) (Unit.in (Unit.of #"meter") (Qty.of 1.0 (Unit.of #"furlong"))))`}
      />
      <P>The new name joins the same family, so it carries a dimension and obeys every rule you've seen: conversion, mixing, mismatch detection. Define a nautical mile as 1852 meters and two of them come to <C>3704</C> meters:</P>
      <Runnable
        source={`(Unit.define #"nautical-mile" (Unit.of #"meter") 1852 1)

(def (main) (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"nautical-mile"))))`}
      />
      <P>A unit's name has to mean exactly one conversion. Redefining <C>foot</C> as 2 meters, a value it already isn't, is a contradiction, and the compiler rejects it rather than let two definitions fight:</P>
      <Note>This one is <strong>meant to be rejected</strong>. A redefinition that <em>agrees</em> with the existing conversion is fine; only a <em>conflicting</em> one is the error (CDZ0502) you'll see here.</Note>
      <Runnable
        source={`(Unit.define #"foot" (Unit.of #"meter") 2 1)

(def (main) 0)`}
        expect="error"
      />
      <H2>Worked example: exact CAD</H2>
      <P>Here's where units and <em>exact</em> numbers pay off together. Every quantity so far used a <C>Float64</C> value, but a <C>Qty</C> can carry a <em>rational</em> just as well, and then a conversion is <em>exact</em>, no rounding. This is exactly how Cadenza's CAD library models solids: rational coordinates in real units, so a metric body and imperial fasteners coexist with no float drift. A quarter-inch hole in a millimetre plate converts to precisely <C>127/20</C> mm, which is <C>6.35</C> mm, exactly, as a fraction:</P>
      <Runnable
        source={`(def
  (main)
  (=
    (Unit.in (Unit.of #"millimeter") (Qty.of (Rational.of 1 4) (Unit.of #"inch")))
    (Rational.of 127 20)))`}
      />
      <P>That reads <C>true</C>: the converted value is the exact rational <C>127/20</C>, not a float approximation of 6.35. The exactness comes from the <em>value</em> type: the same conversion with a rational never accumulates the drift a float would. Sum a third of a millimetre three times and you land back on exactly <C>1</C>, so the comparison against <C>(Rational.of 1 1)</C> is <C>true</C>, which floating point can't promise:</P>
      <Runnable
        source={`(def (main) (let ((t (Rational.of 1 3))) (= (+ (+ t t) t) (Rational.of 1 1))))`}
      />
      <P>This is the split that makes the CAD model trustworthy: the <em>model</em> (coordinates, dimensions, bolt positions) stays exact in rationals and units, so a bounding box reports true dimensions and a metric-plus-imperial assembly is precise to the fraction. Float only appears at the very end, in the <em>mesh</em> the renderer draws (an arbitrary-angle rotation needs sine and cosine, which aren't rational). Exact where it matters, float only at the geometry kernel, and the <AppLink to="/cad"> CAD page </AppLink> lets you see a model like this rendered. The <AppLink to="/calculator"> calculator </AppLink> works in dimensioned quantities too, if you'd rather just do unit arithmetic at a prompt.</P>
      <Why tenet="Dimensions are checked, then erased">Units live entirely at compile time. <C>(Qty.of 5.0 meter)</C> and the bare <C>5.0</C> emit <em>byte-identical</em> code: the unit is a static claim the checker verifies and then throws away, so a dimensional mismatch is always a compile error (CDZ0501) and never a runtime trap. You get the discipline of dimensional analysis (a length never adds to a time, a velocity is length over time) with zero runtime cost. It's the same principle as the rest of the numeric model: Cadenza refuses to guess what you meant, and here it refuses at the moment you write it.</Why>
      <H2>A quantity is a value you can key by</H2>
      <P>That erasure is also why a quantity works as a value in its own right: the unit is gone at run time, but the magnitude is a perfectly good map key. Build a map under a five-kilometre quantity, then look it up with a separately-constructed but equal key, and it hits, because quantities compare by content like every other value:</P>
      <Runnable
        source={`(def
  (main)
  (let
    ((m (Map.insert (Map.empty) (Qty.of 5 (Unit.prefix kilo (Unit.base #"meter"))) 42)))
    (match
      (Map.lookup m (Qty.of 5 (Unit.prefix kilo (Unit.base #"meter"))))
      ((Some x) x)
      ((None) 0))))`}
      />
      <P>The lookup returns <C>42</C>: a quantity assembled twice is one key, not two. The same by-content rule holds from the set side, so two equal quantities in a <C>Set</C> collapse to one. And because units are checked then erased, <C>5</C> kilometres and <C>5</C> metres are different <em>types</em>, not just different values, so the compiler won't let them share a map at all; convert one with <C>Unit.in</C> first if you mean them to meet.</P>
      <H2>Your turn</H2>
      <Exercise
        id="units:1"
        prompt={<>Convert <C>3.0</C> kilometers into meters with <C>Unit.in</C>. The target unit comes first, then the quantity, and the answer is <C>3000</C>.</>}
        starter={`(Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"?")))`}
        solution={`(Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"kilometer")))`}
        expected="3000.0"
        hint={<>A kilometer is a named unit of length, <C>(Unit.of #"kilometer")</C>. One kilometer is a thousand meters, so three become <C>3000</C>.</>}
      />
      <Exercise
        id="units:2"
        prompt={<>A distance divided by a time is a speed. Divide <C>100.0</C> meters by <C>8.0</C> seconds and recover the number. The answer is <C>12.5</C> (meters per second).</>}
        starter={`(Qty.value (/ (Qty.of 100.0 (Unit.of #"meter")) (Qty.of ? (Unit.of #"second"))))`}
        solution={`(Qty.value (/ (Qty.of 100.0 (Unit.of #"meter")) (Qty.of 8.0 (Unit.of #"second"))))`}
        expected="12.5"
        hint={<>The unit divides along with the number. You don't spell the <C>meter/second</C> yourself, the division builds it. <C>100.0 / 8.0</C> is <C>12.5</C>.</>}
      />
      <Exercise
        id="units:3"
        prompt={<>Cube a <C>2.0</C>-meter edge with <C>Qty.pow</C> to get a volume in cubic meters. Raising to the power <C>3</C> gives <C>8</C>.</>}
        starter={`(Qty.value (Qty.pow (Qty.of 2.0 (Unit.of #"meter")) ?))`}
        solution={`(Qty.value (Qty.pow (Qty.of 2.0 (Unit.of #"meter")) 3))`}
        expected="8.0"
        hint={<>The second argument to <C>Qty.pow</C> is the exponent. A cube is the third power, so it's <C>3</C>, and <C>2.0</C> cubed is <C>8</C>.</>}
      />
      <Exercise
        id="units:4"
        prompt={<>Define a <C>span</C> as <C>3</C> meters with <C>Unit.define</C>, then convert <C>4.0</C> spans to meters. Four spans is <C>12</C> meters.</>}
        starter={`(Unit.define #"span" (Unit.of #"meter") ? 1)

(def (main) (Unit.in (Unit.of #"meter") (Qty.of 4.0 (Unit.of #"span"))))`}
        solution={`(Unit.define #"span" (Unit.of #"meter") 3 1)

(def (main) (Unit.in (Unit.of #"meter") (Qty.of 4.0 (Unit.of #"span"))))`}
        expected="12.0"
        hint={<>The ratio is numerator then denominator, so a span is <C>3 / 1</C> meters. Then <C>4.0</C> spans convert to <C>4 × 3 = 12</C> meters.</>}
      />
      <Exercise
        id="units:5"
        prompt={<>Exact conversion, no rounding. A <C>1/4</C>-inch length is exactly <C>127/20</C> mm. Fill the numerator of the rational quarter-inch so the exactness check gives <C>true</C>.</>}
        starter={`(def
  (main)
  (=
    (Unit.in (Unit.of #"millimeter") (Qty.of (Rational.of ? 4) (Unit.of #"inch")))
    (Rational.of 127 20)))`}
        solution={`(def
  (main)
  (=
    (Unit.in (Unit.of #"millimeter") (Qty.of (Rational.of 1 4) (Unit.of #"inch")))
    (Rational.of 127 20)))`}
        expected="true"
        hint={<>A quarter inch is <C>(Rational.of 1 4)</C>, numerator <C>1</C>. One inch is <C>25.4</C> mm, so a quarter is <C>6.35</C> mm, which as an exact fraction is <C>127/20</C>. The check confirms the conversion landed on that rational exactly.</>}
      />
    </article>
  );
}
