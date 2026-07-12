import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function Data() {
  return (
    <article>
      <H1>Data: tuples &amp; records</H1>
      <Lede>Compound values — and the runtime that carries them across the boundary.</Lede>

      <H2>Tuples</H2>
      <P>
        A tuple is a fixed positional product of values. When you Run this, notice the result carries
        its full structural type, <C>(Tuple Int64 Int64 Int64)</C>.
      </P>
      <Runnable title="A tuple" source={`(tuple 1 2 3)`} />

      <Note>
        This is where the browser does something remarkable: a compound value like a tuple lives in a
        value-heap runtime. Your browser composes that runtime with your compiled program and walks the
        result back into the text you see — the same path the native toolchain uses.
      </Note>

      <H2>Records</H2>
      <P>
        A record has a fixed set of <em>named</em> fields. Access a field with the <C>.</C> accessor.
      </P>
      <Runnable source={`(let ((p (record (x 1) (y 2))))
  (. p x))`} />

      <P>Return the whole record to see its structural type:</P>
      <Runnable source={`(record (x 1) (y 2))`} />

      <Why tenet="Everything is a record">
        Records aren't just one data type among many — they're the mechanism the whole language is
        built from. A module is a record of its exports; a built-in like <C>List</C> is a record of its
        operations; a sum type is a record of its constructors. Because the special things are ordinary
        values reached the ordinary way, the compiler needs just <em>one</em> lookup rule instead of
        dozens of special cases — and a name you define simply shadows a built-in of the same name,
        with no magic.
      </Why>

      <H2>Functions inside data</H2>
      <P>
        Since functions are values, they can live in tuples and records too. Here we pull a function
        out of a tuple and call it:
      </P>
      <Runnable source={`((. (tuple (fn (x) (+ x 1)) 9) 0) 5)`} />
    </article>
  );
}
