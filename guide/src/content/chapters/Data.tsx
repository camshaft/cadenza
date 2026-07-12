import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function Data() {
  return (
    <article>
      <H1>Tuples &amp; records</H1>
      <Lede>Bundling several values into one — positionally, or by name.</Lede>

      <P>
        So far our programs have passed around single values. Real programs group values together: a
        point is an <C>x</C> and a <C>y</C>, a result is a value and a status. Cadenza has two ways to
        bundle values — <strong>tuples</strong> (by position) and <strong>records</strong> (by name).
      </P>

      <H2>Tuples</H2>
      <P>
        A tuple is a fixed positional product of values. When you Run this, notice the result carries
        its full structural type, <C>(Tuple Int64 Int64 Int64)</C> — the compiler tracks exactly how
        many elements it has and the type of each.
      </P>
      <Runnable title="A tuple" source={`(tuple 1 2 3)`} />

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

      <P>
        That's the vocabulary. Next we'll <em>use</em> it — passing records into functions, nesting
        them, and reaching in for the pieces — in <strong>Working with records &amp; tuples</strong>.
      </P>
    </article>
  );
}
