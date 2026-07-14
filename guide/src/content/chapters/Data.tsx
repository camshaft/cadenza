import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
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
        A tuple is a fixed positional product of values. When you Run this, look at the type the result
        carries — <C>(Tuple Int64 Bool)</C>: the compiler tracks not just <em>how many</em> elements
        there are but the type of each one, in order. The two slots hold different types, and that's
        fine — a tuple's shape is part of its type.
      </P>
      <Runnable title="A tuple" source={`(tuple 7 true)`} />

      <H2>Records</H2>
      <P>
        A record has a fixed set of <em>named</em> fields. Reach a field with the <C>.</C> accessor —
        here, the <C>leap</C> flag of a little "year" record:
      </P>
      <Runnable source={`(let ((y (record (year 2026) (leap true))))
  (. y leap))`} />

      <P>
        Return the whole record and, just like the tuple, it carries a structural type — one entry per
        field, each with its own type:
      </P>
      <Runnable source={`(record (year 2026) (leap true))`} />

      <Why tenet="Everything is a record">
        Records aren't just one data type among many — they're the mechanism the whole language is
        built from. A module is a record of its exports; a built-in like <C>List</C> is a record of its
        operations; a sum type is a record of its constructors. Because the special things are ordinary
        values reached the ordinary way, the compiler needs just <em>one</em> lookup rule instead of
        dozens of special cases — and a name you define simply shadows a built-in of the same name,
        with no magic.
      </Why>

      <H2>They compose</H2>
      <P>
        A tuple can hold a record, a record field can hold a tuple, and so on — the shapes nest freely,
        and the accessor chains to reach inside. Here a record has one field, <C>pair</C>, holding a
        tuple; we reach the field, then index into the tuple to pull out its second element:
      </P>
      <Runnable source={`(. (. (record (pair (tuple 10 20))) pair) 1)`} />
      <P>
        We'll lean on this in the next chapter — for now the point is just that the two ways of bundling
        stack together, with no special rule for the combination.
      </P>

      <H2>Functions inside data</H2>
      <P>
        Since functions are values, they can live in tuples and records too. Here we pull a function
        out of a tuple and call it:
      </P>
      <Runnable source={`((. (tuple (fn (x) (+ x 1)) 9) 0) 5)`} />

      <H2>Your turn</H2>
      <Exercise
        id="data:1"
        prompt={
          <>
            The record has three named fields. Reach the <C>y</C> field with the <C>.</C> accessor — the
            answer is <C>20</C>.
          </>
        }
        starter={`(. (record (x 10) (y 20) (z 30)) ?)`}
        solution={`(. (record (x 10) (y 20) (z 30)) y)`}
        expected="20"
        hint={
          <>
            The accessor takes the field's <em>name</em>, not a number — records are reached by name.
            Write <C>y</C>.
          </>
        }
      />

      <Exercise
        id="data:2"
        prompt={
          <>
            A tuple is reached by <em>position</em>, not name. Use the <C>.</C> accessor with an index to
            pull the middle element out of <C>(tuple 7 8 9)</C> — counting from zero, that's <C>8</C>.
          </>
        }
        starter={`(. (tuple 7 8 9) ?)`}
        solution={`(. (tuple 7 8 9) 1)`}
        expected="8"
        hint={
          <>
            Tuple indices start at <C>0</C>, so the middle of three elements is index <C>1</C> — a{" "}
            <em>number</em>, where the record above used a name.
          </>
        }
      />

      <P>
        That's the vocabulary. Next we'll <em>use</em> it — passing records into functions, nesting
        them, and reaching in for the pieces — in <strong>Working with records &amp; tuples</strong>.
      </P>
    </article>
  );
}
