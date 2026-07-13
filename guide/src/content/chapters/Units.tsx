import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Units() {
  return (
    <article>
      <H1>Units of measure</H1>
      <Lede>
        A number rarely means anything on its own — <C>5.0</C> is five <em>what</em>? Metres, seconds,
        dollars? Cadenza lets you carry the unit along with the value, checks that you never add a length
        to a time, and then erases the whole apparatus before the program runs. It costs nothing.
      </Lede>

      <P>
        A <strong>quantity</strong> is a number paired with a unit. You build one with <C>Qty.of</C> —
        give it a value and a unit — and you recover the plain number with <C>Qty.value</C>:
      </P>
      <Runnable source={`(Qty.value (Qty.of 5.0 (Unit.of #"metre")))`} />
      <P>
        The result is just <C>5.0</C>: <C>Qty.value</C> strips the unit off. Toggle this snippet to the
        ML surface and you'll see the unit read back as a tidy postfix — <C>5.0 metre</C>. That is the
        whole idea: the <C>metre</C> travels with the <C>5.0</C> through type-checking, then vanishes.
      </P>

      <H2>The same dimension converts on its own</H2>
      <P>
        A kilometre and a metre are two units of the one dimension <em>length</em>. Combining them is
        well-formed even though they differ — each carries an exact scale to its dimension's reference, so
        Cadenza converts and adds them for you. One kilometre plus five hundred metres is fifteen hundred
        metres:
      </P>
      <Runnable
        source={`(Qty.value
  (+ (Qty.of 1.0 (Unit.of #"kilometre"))
     (Qty.of 500.0 (Unit.of #"metre"))))`}
      />
      <P>
        This is the one place Cadenza converts a number for you without being asked — and it earns the
        exception by being <em>exact</em>: kilometre-to-metre is times a thousand, no rounding, no guess.
        A mix of different <em>dimensions</em> gets no such courtesy, as we'll see in a moment.
      </P>

      <H2>Converting on purpose</H2>
      <P>
        When you want a specific unit out, ask for it by name with <C>Unit.in</C> — the target unit
        first, then the quantity. Two kilometres <em>in metres</em>:
      </P>
      <Runnable
        source={`(Qty.value
  (Unit.in (Unit.of #"metre")
           (Qty.of 2.0 (Unit.of #"kilometre"))))`}
      />
      <P>
        It runs both ways across a scale. Two hundred fifty milliseconds <em>in seconds</em> is a quarter
        of a second:
      </P>
      <Runnable
        source={`(Qty.value
  (Unit.in (Unit.of #"second")
           (Qty.of 250.0 (Unit.prefix milli (Unit.of #"second")))))`}
      />

      <H2>Different dimensions do not mix</H2>
      <P>
        Here is where the safety shows up. A length and a time share nothing — there is no exact factor
        between metres and seconds — so adding them is a mistake, and the compiler says so <em>before</em>{" "}
        the program ever runs:
      </P>
      <Note>
        This one is <strong>meant to be rejected</strong>. Run it and read the diagnostic — it names both
        dimensions and refuses. That's the feature working: a units bug is a compile error, never a wrong
        answer at runtime.
      </Note>
      <Runnable
        source={`(+ (Qty.of 1.0 (Unit.of #"metre"))
   (Qty.of 1.0 (Unit.of #"second")))`}
        expect="error"
      />
      <P>
        The same guard covers conversion: asking for a length <C>Unit.in</C> a time is the same category
        error, caught the same way.
      </P>

      <H2>Dimensions compose</H2>
      <P>
        Units aren't just labels you match — they combine. Divide a distance by a time and you get a
        speed; the units divide right along with the numbers. Two hundred forty metres over eight seconds
        is thirty metres per second:
      </P>
      <Runnable
        source={`(Qty.value
  (/ (Qty.of 240.0 (Unit.of #"metre"))
     (Qty.of 8.0 (Unit.of #"second"))))`}
      />
      <P>
        The value is <C>30.0</C>, and its <em>unit</em> is metres-per-second — a compound Cadenza built by
        dividing the two. You can spell such units directly, too: <C>(Unit.* a b)</C> for a product (an
        area is a length times a length), <C>(Unit./ a b)</C> for a quotient, <C>(Unit.^ u n)</C> for a
        power, and <C>Unit.one</C> for a plain dimensionless number.
      </P>

      <H2>Raising a quantity to a power</H2>
      <P>
        <C>Qty.pow</C> raises a whole quantity — value <em>and</em> unit — to a compile-time integer power.
        Square a length and you get an area; the unit becomes metres-squared while the value squares. A
        five-metre side gives twenty-five square metres:
      </P>
      <Runnable source={`(Qty.value (Qty.pow (Qty.of 5.0 (Unit.of #"metre")) 2))`} />
      <P>
        The exponent can be <em>negative</em>, and that's where units earn their keep. A period is a time;
        its <em>reciprocal</em> is a frequency, with the unit inverted to per-second. A four-second period
        is a frequency of <C>0.25</C> per second:
      </P>
      <Runnable source={`(Qty.value (Qty.pow (Qty.of 4.0 (Unit.of #"second")) -1))`} />
      <P>
        That <C>-1</C> didn't just divide the number — it flipped the dimension from <em>time</em> to{" "}
        <em>per-time</em>. Try to add the result to a plain length and you'll get the same CDZ0501 you saw
        above: a frequency and a length are different dimensions, and the compiler knows it.
      </P>

      <H2>Prefixes: SI and binary</H2>
      <P>
        A prefix scales a unit by an exact factor. The SI decimal prefixes — <C>kilo</C> (10³),{" "}
        <C>milli</C> (10⁻³), <C>mega</C>, <C>micro</C>, and the rest — go through <C>Unit.prefix</C>. So do
        the IEC <em>binary</em> prefixes for information, where the steps are powers of two: <C>kibi</C> is
        1024, <C>mebi</C> is 2²⁰. One mebibyte is exactly 1&nbsp;048&nbsp;576 bytes, not a million:
      </P>
      <Runnable
        source={`(Qty.value
  (Unit.in (Unit.of #"byte")
           (Qty.of 1.0 (Unit.prefix mebi (Unit.of #"byte")))))`}
      />
      <P>
        That's the distinction a units layer is <em>for</em> — the kind that quietly turns a 1&nbsp;MiB
        buffer into a 1&nbsp;MB one in someone's head. Here the two prefixes are different names with
        different exact factors, and the arithmetic can't blur them.
      </P>

      <H2>Declaring your own units</H2>
      <P>
        The built-in table doesn't have to be the end of it. <C>Unit.define</C> introduces a new named
        unit as an exact multiple of one you already have — a name, the unit to build on, and a ratio
        (numerator then denominator). A furlong is 660 feet, so once you've defined it, it converts like
        any other unit — one furlong is <C>201.168</C> metres:
      </P>
      <Runnable
        source={`(Unit.define #"furlong" (Unit.of #"foot") 660 1)
(def (main)
  (Qty.value
    (Unit.in (Unit.of #"metre") (Qty.of 1.0 (Unit.of #"furlong")))))`}
      />
      <P>
        The new name joins the same family, so it carries a dimension and obeys every rule you've seen —
        conversion, mixing, mismatch detection. Define a nautical mile as 1852 metres and two of them come
        to <C>3704</C> metres:
      </P>
      <Runnable
        source={`(Unit.define #"nautical-mile" (Unit.of #"metre") 1852 1)
(def (main)
  (Qty.value
    (Unit.in (Unit.of #"metre") (Qty.of 2.0 (Unit.of #"nautical-mile")))))`}
      />
      <P>
        A unit's name has to mean exactly one conversion. Redefining <C>foot</C> as 2 metres — a value it
        already isn't — is a contradiction, and the compiler rejects it rather than let two definitions
        fight:
      </P>
      <Note>
        This one is <strong>meant to be rejected</strong>. A redefinition that <em>agrees</em> with the
        existing conversion is fine; only a <em>conflicting</em> one is the error (CDZ0502) you'll see
        here.
      </Note>
      <Runnable
        source={`(Unit.define #"foot" (Unit.of #"metre") 2 1)
(def (main) 0)`}
        expect="error"
      />

      <Why tenet="Dimensions are checked, then erased">
        Units live entirely at compile time. <C>(Qty.of 5.0 metre)</C> and the bare <C>5.0</C> emit{" "}
        <em>byte-identical</em> code — the unit is a static claim the checker verifies and then throws
        away, so a dimensional mismatch is always a compile error (CDZ0501) and never a runtime trap. You
        get the discipline of dimensional analysis — a length never adds to a time, a velocity is length
        over time — with zero runtime cost. It's the same principle as the rest of the numeric model:
        Cadenza refuses to guess what you meant, and here it refuses at the moment you write it.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="units:1"
        prompt={
          <>
            Convert <C>3.0</C> kilometres into metres with <C>Unit.in</C>. The target unit comes first,
            then the quantity — the answer is <C>3000</C>.
          </>
        }
        starter={`(Qty.value
  (Unit.in (Unit.of #"metre")
           (Qty.of 3.0 (Unit.of #"?"))))`}
        solution={`(Qty.value
  (Unit.in (Unit.of #"metre")
           (Qty.of 3.0 (Unit.of #"kilometre"))))`}
        expected="3000"
        hint={
          <>
            A kilometre is a named unit of length — <C>(Unit.of #"kilometre")</C>. One kilometre is a
            thousand metres, so three become <C>3000</C>.
          </>
        }
      />

      <Exercise
        id="units:2"
        prompt={
          <>
            A distance divided by a time is a speed. Divide <C>100.0</C> metres by <C>8.0</C> seconds and
            recover the number — the answer is <C>12.5</C> (metres per second).
          </>
        }
        starter={`(Qty.value
  (/ (Qty.of 100.0 (Unit.of #"metre"))
     (Qty.of ? (Unit.of #"second"))))`}
        solution={`(Qty.value
  (/ (Qty.of 100.0 (Unit.of #"metre"))
     (Qty.of 8.0 (Unit.of #"second"))))`}
        expected="12.5"
        hint={
          <>
            The unit divides along with the number — you don't spell the <C>metre/second</C> yourself, the
            division builds it. <C>100.0 / 8.0</C> is <C>12.5</C>.
          </>
        }
      />

      <Exercise
        id="units:3"
        prompt={
          <>
            Cube a <C>2.0</C>-metre edge with <C>Qty.pow</C> to get a volume in cubic metres. Raising to
            the power <C>3</C> gives <C>8</C>.
          </>
        }
        starter={`(Qty.value (Qty.pow (Qty.of 2.0 (Unit.of #"metre")) ?))`}
        solution={`(Qty.value (Qty.pow (Qty.of 2.0 (Unit.of #"metre")) 3))`}
        expected="8"
        hint={
          <>
            The second argument to <C>Qty.pow</C> is the exponent. A cube is the third power, so it's{" "}
            <C>3</C> — and <C>2.0</C> cubed is <C>8</C>.
          </>
        }
      />

      <Exercise
        id="units:4"
        prompt={
          <>
            Define a <C>span</C> as <C>3</C> metres with <C>Unit.define</C>, then convert <C>4.0</C> spans
            to metres. Four spans is <C>12</C> metres.
          </>
        }
        starter={`(Unit.define #"span" (Unit.of #"metre") ? 1)
(def (main)
  (Qty.value
    (Unit.in (Unit.of #"metre") (Qty.of 4.0 (Unit.of #"span")))))`}
        solution={`(Unit.define #"span" (Unit.of #"metre") 3 1)
(def (main)
  (Qty.value
    (Unit.in (Unit.of #"metre") (Qty.of 4.0 (Unit.of #"span")))))`}
        expected="12"
        hint={
          <>
            The ratio is numerator then denominator — a span is <C>3 / 1</C> metres. Then <C>4.0</C> spans
            convert to <C>4 × 3 = 12</C> metres.
          </>
        }
      />
    </article>
  );
}
