import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function RecordsTuples() {
  return (
    <article>
      <H1>Working with records &amp; tuples</H1>
      <Lede>Passing structured data around, reaching into it, and nesting it.</Lede>

      <P>
        You met records and tuples in <em>Data</em>. Here we put them to work — the everyday rhythm of
        building a little bundle of values, handing it to a function, and reaching in for the pieces.
      </P>

      <H2>Functions over records</H2>
      <P>
        A record travels as one value, so a function can take a whole record and read its fields with
        the <C>.</C> accessor. Here <C>area</C> multiplies a rectangle's width and height:
      </P>
      <Runnable
        wrap={false}
        source={`(module m
  (def (area r) (* (. r w) (. r h)))
  (def (main) (area (record (w 4) (h 5))))
  (export main))`}
      />

      <H2>Nesting</H2>
      <P>
        Records and tuples nest freely — a field can hold another record, a tuple can hold a record,
        and so on. Chain the accessor to reach inside:
      </P>
      <Runnable source={`(. (. (record (pt (record (x 3) (y 4)))) pt) y)`} />

      <H2>Tuples as lightweight pairs</H2>
      <P>
        When you just need to carry a couple of values together without naming a record type, a tuple
        is the tool. Access elements by position with <C>.</C> and an index. Here <C>swap</C> flips a
        pair, and we read back its new first element:
      </P>
      <Runnable
        wrap={false}
        source={`(module m
  (def (swap p) (tuple (. p 1) (. p 0)))
  (def (main) (. (swap (tuple 3 7)) 0))
  (export main))`}
      />

      <Why tenet="Records are named, tuples are positional">
        Why have both? A tuple is a fixed <em>positional</em> product — reach in by index — and reads
        best when the pieces are obvious from context (a coordinate pair, a swap). A record has fixed{" "}
        <em>named</em> fields — reach in by name — and reads best when the pieces deserve labels (a
        rectangle's <C>w</C> and <C>h</C>). Both are just values with a fixed shape the compiler knows,
        so it can check every field access against the actual structure — no "field not found" surprises
        at run time.
      </Why>

      <H2>Putting it together</H2>
      <P>
        A small taste of real code: compute the squared distance of a point from the origin, by squaring
        each coordinate and summing. It composes a helper (<C>sq</C>) with record access:
      </P>
      <Runnable
        wrap={false}
        source={`(module m
  (def (sq x) (* x x))
  (def (dist2 p) (+ (sq (. p x)) (sq (. p y))))
  (def (main) (dist2 (record (x 3) (y 4))))
  (export main))`}
      />

      <H2>Your turn</H2>
      <Exercise
        id="records-tuples:1"
        prompt={<>Finish <C>perimeter</C> so a 4×5 rectangle gives <C>18</C> (two widths plus two heights).</>}
        starter={`(module m
  (def (perimeter r)
    (+ (* 2 (. r w)) ?))
  (def (main) (perimeter (record (w 4) (h 5))))
  (export main))`}
        solution={`(module m
  (def (perimeter r)
    (+ (* 2 (. r w)) (* 2 (. r h))))
  (def (main) (perimeter (record (w 4) (h 5))))
  (export main))`}
        expected="18"
        wrap={false}
        hint={<>Mirror the width term for the height: <C>(* 2 (. r h))</C>.</>}
      />

      <Exercise
        id="records-tuples:2"
        prompt={<>Write <C>third</C> to pull element 2 out of a 3-tuple, so <C>(third (tuple 5 6 7))</C> gives <C>7</C>.</>}
        starter={`(module m
  (def (third t) (. t ?))
  (def (main) (third (tuple 5 6 7)))
  (export main))`}
        solution={`(module m
  (def (third t) (. t 2))
  (def (main) (third (tuple 5 6 7)))
  (export main))`}
        expected="7"
        wrap={false}
        hint={<>Tuple indices start at 0, so the third element is index <C>2</C>.</>}
      />
    </article>
  );
}
