// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function RecordsTuples() {
  return (
    <article>
      <H1>Working with records & tuples</H1>
      <Lede>Passing structured data around, reaching into it, and nesting it.</Lede>
      <P>You met records and tuples in <em>Data</em>, and here we put them to work in the everyday rhythm of building a little bundle of values, handing it to a function, and reaching in for the pieces.</P>
      <H2>Functions over records</H2>
      <P>A record travels as one value, so a function can take a whole record and read its fields with the <C>.</C> accessor. Here <C>area</C> multiplies a rectangle's width and height:</P>
      <Runnable
        source={`(def (area r) (* r.w r.h))

(def (main) (area #record((= w 4) (= h 5))))`}
      />
      <P>Reaching in with <C>.</C> repeatedly gets noisy when a function wants several fields. You can instead name them up front with a <em>record pattern</em> that binds each field in one move, right in the parameter, which <Ch to="/irrefutable-patterns">Irrefutable patterns</Ch> covers in full alongside tuple destructuring.</P>
      <H2>Updating a field</H2>
      <P>Records are immutable, so you never <em>change</em> a field but instead produce a new record that differs in one place. <C>Record.with</C> does exactly that: give it a record, a <C>#field</C> selector naming which field, and the new value, and it hands back a copy with that field replaced. Here a price of <C>2</C> becomes <C>9</C>:</P>
      <Runnable
        source={`(. (Record.with #record((= item 1) (= price 2)) #"price" 9) price)`}
        id="record-with"
      />
      <Note>The field is named with a <C>#</C> selector, written <C>#"price"</C> in the s-expr surface and <C>#price</C> in the ML surface (flip the syntax toggle to see it). It's a symbol picking the field by name, not a value; the record and the new value are the other two operands.</Note>
      <P>"Hands back a copy" is the important part, because the original is untouched. Bind a record, make an updated version, then read the <em>original</em> back and you'll see it never moved:</P>
      <Runnable
        source={`(let ((base #record((= hp 5) (= mp 3)))) (. (Record.with base #"hp" 99) hp))`}
        id="record-with-immut"
      />
      <P>That reads <C>99</C>, but swap <C>Record.with base #"hp" 99</C> for plain <C>base</C> and Run again to get <C>5</C>, the original, still there. Two records, sharing everything but the one field.</P>
      <P><C>Record.with</C> is for a field that already exists; to <em>add</em> a new one, use <C>Record.extend</C>. Keeping them separate is deliberate, because it means a typo'd field name can't silently create a new field where you meant to update an old one:</P>
      <Runnable
        source={`(. (Record.extend #record((= x 1) (= y 2)) #"z" 3) z)`}
      />
      <P>And the compiler holds the line: use <C>Record.with</C> on a field that isn't there and it won't guess, but tells you to reach for <C>Record.extend</C> instead.</P>
      <Runnable
        source={`(. (Record.with #record((= a 1)) #"z" 5) z)`}
        expect="error"
      />
      <H2>Combining two records</H2>
      <P>Where <C>extend</C> adds one field, <C>Record.merge</C> combines two whole records into one whose fields are the <em>union</em> of both. Merge a record of sizes with a record of colours and you can reach any field of either side:</P>
      <Runnable
        source={`(. (Record.merge #record((= w 4) (= h 5)) #record((= r 255))) r)`}
        id="record-merge"
      />
      <P>The result has all three fields <C>w</C>, <C>h</C>, and <C>r</C>, so <C>.r</C> reads <C>255</C>. And the same no-clobber discipline applies: the two records must have <em>disjoint</em> fields. Ask to merge two records that both define <C>x</C> and the compiler refuses rather than silently pick a winner:</P>
      <Runnable
        source={`(. (Record.merge #record((= x 1)) #record((= x 2))) x)`}
        expect="error"
      />
      <H2>Nesting</H2>
      <P>Records and tuples nest freely, so a field can hold another record and a tuple can hold a record, and so on. Chain the accessor to reach inside:</P>
      <Runnable
        source={`(. (. #record((= pt #record((= x 3) (= y 4)))) pt) y)`}
      />
      <H2>Tuples as lightweight pairs</H2>
      <P>When you just need to carry a couple of values together without naming a record type, a tuple is the tool. Access elements by position with <C>.</C> and an index. Here <C>swap</C> flips a pair, and since a tuple is an ordinary value it can hand the whole flipped pair straight back:</P>
      <Runnable
        source={`(def (swap p) #tuple((. p 1) (. p 0)))

(def (main) (swap #tuple(3 7)))`}
        id="swap-tuple"
      />
      <P>The result is <Cadenza ast="Y2R6YXN0AAEDFQABBwABAwQAAAABAAIBAwABAgM=" kind="expr">#tuple(7 3)</Cadenza>, both elements in their new positions, returned as one value you could pass along or reach into further.</P>
      <P>And just as <C>Record.merge</C> combined two records, <C>Tuple.concat</C> joins two tuples end to end into one wider tuple. It's the positional version, so there's no disjointness rule, and the second tuple's elements simply follow the first, their indices shifting up. Cat a pair onto a triple and index <C>3</C> of the result is the first element of the second tuple, <C>4</C>:</P>
      <Runnable
        source={`(. (Tuple.concat #tuple(1 2) #tuple(3 4 5)) 3)`}
        id="tuple-concat"
      />
      <Why tenet="Records are named, tuples are positional">Why have both? A tuple is a fixed <em>positional</em> product you reach into by index, and it reads best when the pieces are obvious from context (a coordinate pair, a swap). A record has fixed <em>named</em> fields you reach into by name, and it reads best when the pieces deserve labels (a rectangle's <C>w</C> and <C>h</C>). Both are just values with a fixed shape the compiler knows, so it can check every field access against the actual structure, with no "field not found" surprises at run time.</Why>
      <H2>Putting it together</H2>
      <P>A small taste of real code: compute the squared distance of a point from the origin, by squaring each coordinate and summing. It composes a helper (<C>sq</C>) with record access:</P>
      <Runnable
        source={`(def (sq x) (* x x))

(def (dist2 p) (+ (sq p.x) (sq p.y)))

(def (main) (dist2 #record((= x 3) (= y 4))))`}
      />
      <H2>Your turn</H2>
      <Exercise
        id="records-tuples:1"
        prompt={<>Finish <C>perimeter</C> so a 4×5 rectangle gives <C>18</C> (two widths plus two heights).</>}
        starter={`(def (perimeter r) (+ (* 2 r.w) ?))

(def (main) (perimeter #record((= w 4) (= h 5))))`}
        solution={`(def (perimeter r) (+ (* 2 r.w) (* 2 r.h)))

(def (main) (perimeter #record((= w 4) (= h 5))))`}
        expected="18"
        hint={<>Mirror the width term for the height: <Cadenza ast="Y2R6YXN0AAEFCgEqAAECGgoBcgoBaAcAAAABAAIAAwAEAQMCAwQBAwABBQY=" kind="expr">(* 2 r.h)</Cadenza>.</>}
      />
      <Exercise
        id="records-tuples:2"
        prompt={<>Concatenating shifts the second tuple's indices up by the first's length. Cat <Cadenza ast="Y2R6YXN0AAEDFQABAQABAgQAAAABAAIBAwABAgM=" kind="expr">#tuple(1 2)</Cadenza> onto <Cadenza ast="Y2R6YXN0AAEEFQABAwABBAABBQUAAAABAAIAAwEEAAECAwQ=" kind="expr">#tuple(3 4 5)</Cadenza> and reach the <em>last</em> element, <C>5</C>, by its index in the joined tuple. Which index is it?</>}
        starter={`(. (Tuple.concat #tuple(1 2) #tuple(3 4 5)) ?)`}
        solution={`(. (Tuple.concat #tuple(1 2) #tuple(3 4 5)) 4)`}
        expected="5"
        hint={<>The joined tuple is <Cadenza ast="Y2R6YXN0AAEGFQABAQABAgABAwABBAABBQcAAAABAAIAAwAEAAUBBgABAgMEBQY=" kind="expr">#tuple(1 2 3 4 5)</Cadenza>, five elements with indices <C>0</C> to <C>4</C>. The <C>5</C> is last, so its index is <C>4</C>, not <C>2</C>, because the first tuple pushed it two slots over.</>}
      />
      <Exercise
        id="records-tuples:3"
        prompt={<><Cadenza ast="Y2R6YXN0AAEGFhkKAXgAAQoKAXkAARQKAAAAAQACAAMBAwECAwABAAQABQEDBQYHAQMABAgJ" kind="expr">#record((= x 10) (= y 20))</Cadenza> has no <C>z</C> field. Add one, <C>z = 30</C>, then read it back for the answer <C>30</C>. Which operation adds a <em>new</em> field, <C>with</C> or <C>extend</C>? Fill in the blank.</>}
        starter={`(. (Record.? #record((= x 10) (= y 20)) #"z" 30) z)`}
        solution={`(. (Record.extend #record((= x 10) (= y 20)) #"z" 30) z)`}
        expected="30"
        hint={<><C>with</C> only updates a field that already exists; adding a brand-new one is <C>Record.extend</C>. (Try <C>with</C> and the compiler declines, since <C>z</C> isn't there to update.)</>}
      />
    </article>
  );
}
