import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";

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

      <H2>Functions inside data</H2>
      <P>
        Since functions are values, they can live in tuples and records too. Here we pull a function
        out of a tuple and call it:
      </P>
      <Runnable source={`((. (tuple (fn (x) (+ x 1)) 9) 0) 5)`} />
    </article>
  );
}
