// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Data() {
  return (
    <article>
      <H1>Tuples & records</H1>
      <Lede>Bundling several values into one, positionally or by name.</Lede>
      <P>So far our programs have passed around single values. Real programs group values together: a point is an <C>x</C> and a <C>y</C>, a result is a value and a status. Cadenza has two ways to bundle values, namely <strong>tuples</strong> (by position) and <strong>records</strong> (by name).</P>
      <H2>Tuples</H2>
      <P>A tuple is a fixed positional product of values. When you Run this, look at the type the result carries, <C>(Tuple Int64 Bool)</C>: the compiler tracks not just <em>how many</em> elements there are but the type of each one, in order. The two slots hold different types, and that's fine, because a tuple's shape is part of its type.</P>
      <Runnable
        source={`#tuple(7 true)`}
        title="A tuple"
      />
      <H2>Records</H2>
      <P>A record has a fixed set of <em>named</em> fields. Reach a field with the <C>.</C> accessor, here the <C>leap</C> flag of a little "year" record:</P>
      <Runnable
        source={`(let ((y #record((= year 2026) (= leap true)))) y.leap)`}
      />
      <P>Return the whole record and, just like the tuple, it carries a structural type, one entry per field, each with its own type:</P>
      <Runnable
        source={`#record((= year 2026) (= leap true))`}
      />
      <Why tenet="Everything is a record">Records aren't just one data type among many, since they're the mechanism the whole language is built from. A module is a record of its exports; a built-in like <C>List</C> is a record of its operations; a sum type is a record of its constructors. Because the special things are ordinary values reached the ordinary way, the compiler needs just <em>one</em> lookup rule instead of dozens of special cases, and a name you define simply shadows a built-in of the same name, with no magic.</Why>
      <H2>Compared by value</H2>
      <P>Two tuples or two records are equal when their contents are, which is structural equality, not identity. A tuple matches position by position, so <Cadenza>#tuple(1 2)</Cadenza> equals another <Cadenza>#tuple(1 2)</Cadenza>:</P>
      <Runnable
        source={`(= #tuple(1 2) #tuple(1 2))`}
      />
      <P>A record matches by <em>field name</em>, not field order, so the same fields written in a different order are still the same record. That's the by-name nature showing through: a record is its set of named fields, however you list them.</P>
      <Runnable
        source={`(= #record((= x 1) (= y 2)) #record((= y 2) (= x 1)))`}
      />
      <H2>Taking one apart with a let</H2>
      <P>Beyond reaching a single field, you can bind <em>all</em> of a tuple's parts at once by destructuring it in a <C>let</C>. The pattern on the left of the binding names each part; here a two-tuple binds <C>a</C> and <C>b</C> in one step, then adds them:</P>
      <Runnable
        source={`(let ((#tuple(a b) #tuple(3 4))) (+ a b))`}
      />
      <P>A two-tuple always has exactly this shape, so the pattern can't fail to match: it's <em>irrefutable</em>, and that's exactly when a <C>let</C> binding is the right tool rather than a one-armed <C>match</C>. Destructuring is the subject of <strong>Pattern matching</strong> and <strong>Irrefutable patterns</strong>; the point here is that a tuple or record is an ordinary value a pattern can name and take apart.</P>
      <H2>They compose</H2>
      <P>A tuple can hold a record, a record field can hold a tuple, and so on, so the shapes nest freely, and the accessor chains to reach inside. Here a record has one field, <C>pair</C>, holding a tuple; we reach the field, then index into the tuple to pull out its second element:</P>
      <Runnable
        source={`(. (. #record((= pair #tuple(10 20))) pair) 1)`}
      />
      <P>Reaching <C>pair</C> gets the tuple, then index <C>1</C> pulls its second element, <C>20</C>. We'll lean on this in the next chapter, but for now the point is just that the two ways of bundling stack together, with no special rule for the combination.</P>
      <H2>Functions inside data</H2>
      <P>Since functions are values, they can live in tuples and records too. Here we pull a function out of a tuple and call it:</P>
      <Runnable
        source={`((. #tuple((fn (x) (+ x 1)) 9) 0) 5)`}
      />
      <P>Element <C>0</C> of the tuple is the increment function; applying it to <C>5</C> gives <C>6</C>. The <C>9</C> in the other slot just rides along, since a tuple can mix a function and a plain value.</P>
      <H2>Your turn</H2>
      <Exercise
        id="data:1"
        prompt={<>The record has three named fields. Reach the <C>y</C> field with the <C>.</C> accessor, so the answer is <C>20</C>.</>}
        starter={`(. #record((= x 10) (= y 20) (= z 30)) ?)`}
        solution={`(. #record((= x 10) (= y 20) (= z 30)) y)`}
        expected="20"
        hint={<>The accessor takes the field's <em>name</em>, not a number, because records are reached by name. Write <C>y</C>.</>}
      />
      <Exercise
        id="data:2"
        prompt={<>Now both ways at once, reaching into a nested shape. This record's <C>point</C> field holds a tuple; the field is already reached by name, so finish the chain with the <em>index</em> that pulls out the <em>last</em> element, <C>9</C>.</>}
        starter={`(. (. #record((= point #tuple(7 8 9))) point) ?)`}
        solution={`(. (. #record((= point #tuple(7 8 9))) point) 2)`}
        expected="9"
        hint={<>The field access <C>point</C> used a name; the tuple step uses a <em>number</em>. Indices start at <C>0</C>, so the last of three elements is index <C>2</C>.</>}
      />
      <P>That's the vocabulary. Next we'll <em>use</em> it by passing records into functions, nesting them, and reaching in for the pieces, in <strong>Working with records &amp; tuples</strong>.</P>
    </article>
  );
}
