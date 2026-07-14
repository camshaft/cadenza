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

      <H2>Reaching in safely</H2>
      <P>
        <C>List.at</C> gets the element at an index. But what if the index is out of range? Rather than
        crash, <C>List.at</C> returns an <C>Option</C> — <C>(Some x)</C> when the element exists,{" "}
        <C>(None unit)</C> when it doesn't — which you take apart with <C>match</C>. Here index 1 exists,
        so you get its value, <C>20</C>:
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

      <H2>Updating a slot — without touching the original</H2>
      <P>
        <C>List.update</C> takes a list, an index, and a new value, and hands back a list with that one
        slot changed. The word "update" is doing something specific here: it does <em>not</em> reach into
        your list and overwrite a slot — nothing in Cadenza does that. It builds a <em>new</em> list. The
        old one is still exactly what it was. This snippet proves it — bind a list, make a bumped version,
        then read the original back:
      </P>
      <Runnable
        source={`(let ((original (list 10 20 30))
      (bumped   (List.update original 1 99)))
  (match (List.at original 1)
    ((Some x) x)
    ((None _) 0)))`}
      />
      <P>
        The answer is <C>20</C>, not <C>99</C>: <C>original</C> never changed. The <C>99</C> lives only in{" "}
        <C>bumped</C>. Swap <C>original</C> for <C>bumped</C> in the <C>List.at</C> line and Run again —
        now you'll see <C>99</C>. Two lists, sharing most of their structure under the hood, each with its
        own value.
      </P>

      <Why tenet="Immutable, persistent values">
        Every value in Cadenza is immutable; an "update" always produces a fresh value and leaves every
        existing reference untouched. That's not just tidiness — it's what makes the whole system tractable.
        Because values can never form a cycle, the runtime reclaims memory by simple reference counting, no
        garbage collector needed. Because a list you handed to a function can't change underneath you,
        there's a whole class of aliasing bug that simply cannot happen. And whether a list is stored as a
        flat array or a balanced tree is the runtime's business, not yours: one type, many representations,
        sharing structure between versions so <C>update</C> doesn't copy the whole thing.
      </Why>

      <H2>Lists through functions</H2>
      <P>
        A function can take a list and compute over it — the element type rides along, so <C>count</C>{" "}
        works on a list of any element type:
      </P>
      <Runnable
        source={`(def (count xs) (List.len xs))
(def (main) (count (list 10 20 30 40)))`}
      />
      <P>
        To visit <em>every</em> element, walk the list by index with recursion (the loop you met in{" "}
        <em>Control flow</em>): <C>List.at</C> hands back <C>(Some v)</C> while there's an element and{" "}
        <C>(None _)</C> once you step past the end — which is exactly the base case. Here <C>sum-from</C>{" "}
        adds up a list of numbers:
      </P>
      <Runnable
        source={`(def (sum-from xs i)
  (match (List.at xs i)
    ((Some v) (+ v (sum-from xs (+ i 1))))
    ((None _) 0)))
(def (main) (sum-from (list 10 20 30) 0))`}
      />
      <P>
        The <C>None</C> arm ends the recursion the moment the index runs off the end — no separate length
        check, no way to read past the end, because the missing element <em>is</em> the stopping signal.
        And you didn't have to tell it the elements are numbers: the type flows from <C>List.at</C>'s
        result through the <C>+</C>, so <C>sum-from</C> is inferred to work on a list of <C>Int64</C>.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="lists:1"
        prompt={
          <>
            Use <C>List.update</C> to change index <C>0</C> of <C>(list 5 6 7)</C> to <C>50</C>, then read
            that slot back with <C>List.at</C>. The answer is <C>50</C>.
          </>
        }
        starter={`(match (List.at (List.update (list 5 6 7) 0 ?) 0)
  ((Some x) x)
  ((None _) 0))`}
        solution={`(match (List.at (List.update (list 5 6 7) 0 50) 0)
  ((Some x) x)
  ((None _) 0))`}
        expected="50"
        hint={
          <>
            <C>List.update</C> takes the list, the index (<C>0</C>), and the new value (<C>50</C>) — in that
            order. Then <C>List.at … 0</C> reads the slot you just set.
          </>
        }
      />

      <Exercise
        id="lists:2"
        prompt={
          <>
            Join <C>(list 1 2)</C> and <C>(list 3 4 5)</C>, then pull out the element at index <C>3</C> with{" "}
            <C>List.at</C>. Counting from zero, that's the <C>4</C>.
          </>
        }
        starter={`(match (List.at (List.concat (list 1 2) (list 3 4 5)) ?)
  ((Some x) x)
  ((None _) 0))`}
        solution={`(match (List.at (List.concat (list 1 2) (list 3 4 5)) 3)
  ((Some x) x)
  ((None _) 0))`}
        expected="4"
        hint={
          <>
            The joined list is <C>1 2 3 4 5</C>. Index <C>0</C> is the <C>1</C>, so index <C>3</C> is the{" "}
            <C>4</C>.
          </>
        }
      />

      <Exercise
        id="lists:3"
        prompt={
          <>
            Complete the recursive step of <C>sum-from</C> so it adds every element. The <C>None</C> base
            case is done; in the <C>Some</C> arm, add this element to the sum of the rest. Over{" "}
            <C>(list 1 2 3 4)</C> the total is <C>10</C>.
          </>
        }
        starter={`(def (sum-from xs i)
  (match (List.at xs i)
    ((Some v) (+ v ?))
    ((None _) 0)))
(def (main) (sum-from (list 1 2 3 4) 0))`}
        solution={`(def (sum-from xs i)
  (match (List.at xs i)
    ((Some v) (+ v (sum-from xs (+ i 1))))
    ((None _) 0)))
(def (main) (sum-from (list 1 2 3 4) 0))`}
        expected="10"
        hint={
          <>
            "The sum of the rest" is the same function on the next index: <C>(sum-from xs (+ i 1))</C>.
          </>
        }
      />
    </article>
  );
}
