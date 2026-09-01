// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function SizedIntegers() {
  return (
    <article>
      <H1>Sized integers</H1>
      <Lede><C>Int64</C> is the everyday integer, but when a value has a fixed width, whether a byte, a 16-bit field, or a protocol number, Cadenza gives it a type that says so, and never converts one width to another behind your back.</Lede>
      <P>A sized integer type is named by its signedness and its bit width: <C>UInt8</C> (0–255), <C>Int8</C> (−128–127), <C>UInt16</C>, <C>UInt32</C>, and so on. You don't get one by writing a literal, since a bare <C>200</C> is an <C>Int64</C>. You <em>convert into</em> a sized type by name, with <C>.of</C>:</P>
      <Runnable
        source={`(UInt8.of 200)`}
      />
      <P><C>200</C> fits in a byte, so this is the <C>UInt8</C> value 200. Different widths hold different ranges, so <C>UInt16.of</C> happily takes a number a byte couldn't:</P>
      <Runnable
        source={`(UInt16.of 300)`}
      />
      <H2>Conversion is checked</H2>
      <P>What if the number doesn't fit? <C>.of</C> is the <em>checked</em> conversion, so it verifies the value is in range and traps if it isn't, rather than silently keeping the wrong bits. Asking a byte to hold 300 is a range error:</P>
      <Note>This one is <strong>meant to trap</strong>. Run it and read the status bar, where the range check firing is the point, so you find out immediately, not via a corrupted value downstream.</Note>
      <Runnable
        source={`(UInt8.of 300)`}
        expect="error"
      />
      <H2>When you want the low bits: <C>wrap</C></H2>
      <P>Sometimes truncation is exactly what you mean, whether for a checksum, a rollover counter, or packing into a byte. That's a different, explicit operation, since <C>wrap</C> keeps the low bits and discards the rest. <C>300</C> wrapped to 8 bits is <C>300 − 256 = 44</C>:</P>
      <Runnable
        source={`(UInt8.wrap 300)`}
      />
      <P><C>of</C> and <C>wrap</C> are the two honest choices for "this doesn't fit", either halting or taking the low bits, and you say which one you mean. Neither happens by accident.</P>
      <H2>Widths are distinct types</H2>
      <P>A <C>UInt8</C> and a <C>UInt16</C> are <em>different types</em>. Try to add them and the compiler refuses, because it won't quietly widen one to match the other:</P>
      <Runnable
        source={`(+ (UInt8.of 1) (UInt16.of 300))`}
        expect="error"
      />
      <P>The fix is to say which width you want and convert there, so the widening is a thing you write, not a thing that happens to you.</P>
      <H2>Why isn't <C>Int</C> a type?</H2>
      <P>A newcomer's first reflex is to annotate with a bare <C>Int</C>, the way most languages name their integer type. In Cadenza <C>Int</C>, <C>UInt</C>, and <C>Float</C> are not types at all: they're <em>width constructors</em> that <em>build</em> a sized type from a bit width. <Cadenza ast="Y2R6YXN0AAECCgNJbnQAAUADAAAAAQECAAEC" kind="expr">(Int 64)</Cadenza> is <C>Int64</C>, <Cadenza ast="Y2R6YXN0AAECCgRVSW50AAEIAwAAAAEBAgABAg==" kind="expr">(UInt 8)</Cadenza> is <C>UInt8</C>, <Cadenza ast="Y2R6YXN0AAECCgVGbG9hdAABIAMAAAABAQIAAQI=" kind="expr">(Float 32)</Cadenza> is <C>Float32</C>. So writing a bare <C>Int</C> where a type belongs uses a value where a type is required, and the compiler says so:</P>
      <Note>This one is <strong>meant to be refused</strong>: <C>Int</C> is a width constructor, not a type, so the annotation has nothing to stand on. The diagnostic names the sized default <C>Int64</C> (and offers a one-click fix to it), and points you at the other widths if you meant one of those.</Note>
      <Runnable
        source={`(def (f (: a Int)) a)`}
        expect="error"
      />
      <P>The fix is to name a concrete width, so <C>Int64</C> for the everyday integer, or <C>Int32</C>, <C>UInt8</C>, and the rest for a fixed size. The compound form <Cadenza ast="Y2R6YXN0AAECCgNJbnQAAUADAAAAAQECAAEC" kind="expr">(Int 64)</Cadenza> is itself a perfectly good type, exactly equal to <C>Int64</C>, so this compiles and runs, and <Cadenza ast="Y2R6YXN0AAECCgRhZGQxAAEpAwAAAAEBAgABAg==" kind="expr">(add1 41)</Cadenza> is <C>42</C>:</P>
      <Runnable
        source={`(do
  (def (add1 (: n (Int 64))) (+ n 1))

  (def (main) (add1 41))

  (export main))`}
      />
      <P>Only the <em>bare</em> name is the mistake, since <Cadenza ast="Y2R6YXN0AAECCgNJbnQAAUADAAAAAQECAAEC" kind="expr">(Int 64)</Cadenza> and <C>Int64</C> are the same type written two ways. Reach for the width name and the reflex costs you nothing.</P>
      <H2>Arithmetic stays inside the width</H2>
      <P>The width isn't just checked at conversion, since it's enforced in arithmetic too. Two <C>UInt8</C>s add as a <C>UInt8</C>, and a sum that would exceed <C>255</C> is caught, exactly like <C>Int64</C> overflow. <C>200 + 100</C> can't fit a byte, so it's refused:</P>
      <Note>This one is <strong>meant to be refused</strong>: the <C>UInt8</C> sum overflows its width, so the compiler declines rather than wrapping, the same no-silent-wrap discipline, at 8 bits.</Note>
      <Runnable
        source={`(+ (UInt8.of 200) (UInt8.of 100))`}
        expect="error"
      />
      <P>When you <em>do</em> want a wider result, widen first: <C>Int64.of</C> lifts a sized value back to the everyday integer, where the sum has room. The <C>UInt8</C> <C>200</C> becomes the <C>Int64</C> <C>200</C>, and adding <C>100</C> is fine:</P>
      <Runnable
        source={`(+ (Int64.of (UInt8.of 200)) 100)`}
      />
      <Why tenet="A width is part of the type, and never crossed silently">Bugs love implicit integer conversion: a value that fit in the source width but not the destination, a sign that flipped on the way, a truncation nobody wrote. Cadenza makes the width part of the type, so two widths simply don't mix, and every crossing is one you asked for by name, choosing <C>of</C> (halt if it doesn't fit) or <C>wrap</C> (take the low bits). It's the same discipline as <C>Int64</C> refusing to blur with <C>Bool</C>, applied to the sizes: the compiler never makes the "did you mean to lose data here?" decision for you.</Why>
      <Note>Signed widths work the same way, so <C>Int8.of</C> takes −128 through 127, traps outside it, and <C>Int8.wrap</C> gives you the low 8 bits. Same two choices, at every width.</Note>
      <P>Integers, checked and sized and never blurred, are only half the number line. Next comes the other half, the approximate, real-valued world of <em>floating-point numbers</em>, with its own operators and its own honest tradeoff.</P>
      <H2>Your turn</H2>
      <Exercise
        id="sized-integers:1"
        prompt={<><C>258</C> doesn't fit a byte, and you want it truncated to the low 8 bits, not a halt. Pick the operation that takes the low bits, so the result should be <C>2</C> (that's <C>258 − 256</C>). Which of <C>of</C> / <C>wrap</C> goes in the blank?</>}
        starter={`(UInt8.? 258)`}
        solution={`(UInt8.wrap 258)`}
        expected="2"
        hint={<><C>wrap</C> keeps the low bits, whereas <C>of</C> would <em>refuse</em> <C>258</C> as out of range. You asked for truncation, so it's <C>wrap</C>, and <C>258</C> wraps to <C>2</C>.</>}
      />
      <Exercise
        id="sized-integers:2"
        prompt={<>This won't compile: <Cadenza ast="Y2R6YXN0AAEHCgErGgoFVUludDgKAm9mAAEBCgZVSW50MTYAAgEsDgAAAAEAAgADAQMBAgMABAECBAUAAQAFAAMBAwcICQAGAQIKCwEDAAYMDQ==" kind="expr">(+ (UInt8.of 1) (UInt16.of 300))</Cadenza> mixes two widths. Fix it by making the first operand a <C>UInt16</C> too, so both sides match and the sum is <C>301</C>.</>}
        starter={`(+ (UInt?.of 1) (UInt16.of 300))`}
        solution={`(+ (UInt16.of 1) (UInt16.of 300))`}
        expected="301"
        hint={<>Widths don't mix, so convert the <C>1</C> at the same width as the other operand, namely <C>UInt16</C>. Then <C>1 + 300 = 301</C>.</>}
      />
    </article>
  );
}
