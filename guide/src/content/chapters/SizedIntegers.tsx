import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function SizedIntegers() {
  return (
    <article>
      <H1>Sized integers</H1>
      <Lede>
        <C>Int64</C> is the everyday integer. When a value has a fixed width — a byte, a 16-bit field, a
        protocol number — Cadenza gives it a type that says so, and never converts one width to another
        behind your back.
      </Lede>

      <P>
        A sized integer type is named by its signedness and its bit width: <C>UInt8</C> (0–255),{" "}
        <C>Int8</C> (−128–127), <C>UInt16</C>, <C>UInt32</C>, and so on. You don't get one by writing a
        literal — a bare <C>200</C> is an <C>Int64</C>. You <em>convert into</em> a sized type by name,
        with <C>.of</C>:
      </P>
      <Runnable source={`(UInt8.of 200)`} />
      <P>
        <C>200</C> fits in a byte, so this is the <C>UInt8</C> value 200. Different widths hold different
        ranges — <C>UInt16.of</C> happily takes a number a byte couldn't:
      </P>
      <Runnable source={`(UInt16.of 300)`} />

      <H2>Conversion is checked</H2>
      <P>
        What if the number doesn't fit? <C>.of</C> is the <em>checked</em> conversion: it verifies the
        value is in range, and traps if it isn't — rather than silently keeping the wrong bits. Asking a
        byte to hold 300 is a range error:
      </P>
      <Note>
        This one is <strong>meant to trap</strong>. Run it and read the status bar — the range check
        firing is the point: you find out immediately, not via a corrupted value downstream.
      </Note>
      <Runnable source={`(UInt8.of 300)`} expect="error" />

      <H2>When you want the low bits: <C>wrap</C></H2>
      <P>
        Sometimes truncation is exactly what you mean — a checksum, a rollover counter, packing into a
        byte. That's a different, explicit operation: <C>wrap</C> keeps the low bits and discards the
        rest. <C>300</C> wrapped to 8 bits is <C>300 − 256 = 44</C>:
      </P>
      <Runnable source={`(UInt8.wrap 300)`} />
      <P>
        <C>of</C> and <C>wrap</C> are two honest choices for "this doesn't fit" — halt, or take the low
        bits — and you say which one you mean. Neither happens by accident.
      </P>

      <H2>Widths are distinct types</H2>
      <P>
        A <C>UInt8</C> and a <C>UInt16</C> are <em>different types</em>. Try to add them and the compiler
        refuses — it won't quietly widen one to match the other:
      </P>
      <Runnable source={`(+ (UInt8.of 1) (UInt16.of 300))`} expect="error" />
      <P>
        The fix is to say which width you want and convert there — the widening is a thing you write, not
        a thing that happens to you.
      </P>

      <Why tenet="A width is part of the type, and never crossed silently">
        Bugs love implicit integer conversion: a value that fit in the source width but not the
        destination, a sign that flipped on the way, a truncation nobody wrote. Cadenza makes the width
        part of the type, so two widths simply don't mix — and every crossing is one you asked for by
        name, choosing <C>of</C> (halt if it doesn't fit) or <C>wrap</C> (take the low bits). It's the
        same discipline as <C>Int64</C> refusing to blur with <C>Bool</C>, applied to the sizes: the
        compiler never makes the "did you mean to lose data here?" decision for you.
      </Why>

      <Note>
        Signed widths work the same way — <C>Int8.of</C> takes −128 through 127, traps outside it, and{" "}
        <C>Int8.wrap</C> gives you the low 8 bits. Same two choices, at every width.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="sized-integers:1"
        prompt={
          <>
            <C>258</C> doesn't fit a byte, and you want it truncated to the low 8 bits, not a halt. Pick
            the operation that takes the low bits — the result should be <C>2</C> (that's{" "}
            <C>258 − 256</C>). Which of <C>of</C> / <C>wrap</C> goes in the blank?
          </>
        }
        starter={`(UInt8.? 258)`}
        solution={`(UInt8.wrap 258)`}
        expected="2"
        hint={
          <>
            <C>wrap</C> keeps the low bits; <C>of</C> would <em>refuse</em> <C>258</C> as out of range. You
            asked for truncation, so it's <C>wrap</C> — and <C>258</C> wraps to <C>2</C>.
          </>
        }
      />

      <Exercise
        id="sized-integers:2"
        prompt={
          <>
            This won't compile: <C>(+ (UInt8.of 1) (UInt16.of 300))</C> mixes two widths. Fix it by making
            the first operand a <C>UInt16</C> too, so both sides match and the sum is <C>301</C>.
          </>
        }
        starter={`(+ (UInt?.of 1) (UInt16.of 300))`}
        solution={`(+ (UInt16.of 1) (UInt16.of 300))`}
        expected="301"
        hint={
          <>
            Widths don't mix, so convert the <C>1</C> at the same width as the other operand —{" "}
            <C>UInt16</C>. Then <C>1 + 300 = 301</C>.
          </>
        }
      />
    </article>
  );
}
