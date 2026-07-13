import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Lists() {
  return (
    <article>
      <H1>Lists</H1>
      <Lede>Ordered, immutable sequences — built and measured on the value heap.</Lede>

      <P>
        A list is written with <C>list</C>. Lists are <em>persistent</em>: operations like{" "}
        <C>List.push</C> and <C>List.concat</C> return a new list and leave the original untouched.
        Ask a list its length with <C>List.len</C>.
      </P>
      <Runnable source={`(List.len (list 1 2 3))`} />

      <H2>Building lists</H2>
      <P>
        <C>List.push</C> adds an element to the end; <C>List.concat</C> joins two lists. Because they
        return new lists, you can chain them and measure the result:
      </P>
      <Runnable source={`(List.len (List.push (list 1 2) 3))`} />
      <Runnable source={`(List.len (List.concat (list 1 2) (list 3 4 5)))`} />

      <Why tenet="Immutable, persistent values">
        Notice that <C>List.push</C> doesn't change the list you gave it — it returns a <em>new</em>
        one. Every value in Cadenza is immutable; an "update" always produces a fresh value. This isn't
        only for tidiness: because values can never form a cycle, the runtime can reclaim memory by
        simple reference counting — no garbage collector needed. And whether a list is stored as a
        flat array or a fancy tree is the runtime's business, not yours: it's one type, many possible
        representations.
      </Why>

      <H2>Reaching in safely</H2>
      <P>
        <C>List.at</C> gets the element at an index. But what if the index is out of range? Rather than
        crash, <C>List.at</C> returns an <C>Option</C> — <C>(Some x)</C> when the element exists,{" "}
        <C>(None unit)</C> when it doesn't — which you take apart with <C>match</C>. Here index 1 exists:
      </P>
      <Runnable
        source={`(def (main)
  (match (List.at (list 10 20 30) 1)
    ((Some x) x)
    ((None _) (- 0 1))))`}
      />
      <P>
        Change the index <C>1</C> to <C>9</C> and Run: the lookup misses, the <C>None</C> arm fires,
        and you get <C>-1</C> — no crash, just a value that says "nothing there".
      </P>

      <Why tenet="Partiality is data, not a trap">
        Reading past the end of a list, looking up a missing key, decoding bad text — in Cadenza these
        yield an <C>Option</C> (or a result), never a crash or a garbage value. Absence is an ordinary
        value your program <em>handles</em>. If you ever do want "crash on missing", that's a single,
        explicit operation that demands a message — so the one place a program turns absence into a halt
        is visible right where it happens, not hidden inside every accessor.
      </Why>

      <H2>Lists through functions</H2>
      <P>A function can take a list and compute over it. Here <C>count</C> just reports its length:</P>
      <Runnable
        source={`(def (count xs) (List.len xs))
(def (main) (count (list 10 20 30 40)))`}
      />

      <H2>Your turn</H2>
      <Exercise
        id="lists:1"
        prompt={<>Concatenate the two lists, then report the total length — it should be <C>5</C>.</>}
        starter={`(List.len (List.concat (list 1 2) ?))`}
        solution={`(List.len (List.concat (list 1 2) (list 3 4 5)))`}
        expected="5"
        hint={<>The second argument to <C>List.concat</C> is another <C>(list …)</C> with three elements.</>}
      />
    </article>
  );
}
