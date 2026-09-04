// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Lists() {
  return (
    <article>
      <H1>Lists</H1>
      <Lede>Ordered, immutable sequences, built and measured on the value heap.</Lede>
      <P>A list is written with <C>#list</C>. Lists are <em>persistent</em>: operations like <C>List.push</C> and <C>List.concat</C> return a new list and leave the original untouched. Ask a list its length with <C>List.len</C>.</P>
      <Runnable
        source={`(List.len #list(1 2 3))`}
      />
      <H2>Building lists</H2>
      <P><C>List.push</C> adds an element to the end; <C>List.concat</C> joins two lists. Each returns a whole new list, so Run this and you'll see the result, <Cadenza ast="Y2R6YXN0AAEEFAABAQABAgABAwUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(1 2 3)</Cadenza>, not just a count:</P>
      <Runnable
        source={`(List.push #list(1 2) 3)`}
      />
      <P><C>List.prepend</C> is the mirror of <C>push</C>: it adds an element to the <em>front</em> rather than the end. It takes the list first and the new element second, the same receiver-first order as <C>push</C>, so prepending <C>1</C> to <Cadenza ast="Y2R6YXN0AAEDFAABAgABAwQAAAABAAIBAwABAgM=" kind="expr">#list(2 3)</Cadenza> gives <Cadenza ast="Y2R6YXN0AAEEFAABAQABAgABAwUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(1 2 3)</Cadenza>, the new element leading:</P>
      <Runnable
        source={`(List.prepend #list(2 3) 1)`}
      />
      <P><C>List.concat</C> joins two lists into a new one, so Run this and you see the whole joined list, <Cadenza ast="Y2R6YXN0AAEGFAABAQABAgABAwABBAABBQcAAAABAAIAAwAEAAUBBgABAgMEBQY=" kind="expr">#list(1 2 3 4 5)</Cadenza>, the two inputs laid end to end:</P>
      <Runnable
        source={`(List.concat #list(1 2) #list(3 4 5))`}
      />
      <H2>Reaching in safely</H2>
      <P><C>List.at</C> gets the element at an index. But what if the index is out of range? Rather than crash, <C>List.at</C> returns an <C>Option</C>, either <Cadenza ast="Y2R6YXN0AAECCgRTb21lCgF4AwAAAAEBAgABAg==" kind="expr">(Some x)</Cadenza> when the element exists or <Cadenza ast="Y2R6YXN0AAECCgROb25lCgR1bml0AwAAAAEBAgABAg==" kind="expr">(None unit)</Cadenza> when it doesn't, which you take apart with <C>match</C>. Here index 1 exists, so you get its value, <C>20</C>:</P>
      <Runnable
        source={`(def (main) (match (List.at #list(10 20 30) 1) ((Some x) x) ((None _) -1)))`}
      />
      <P>Change the index <C>1</C> to <C>9</C> and Run: the lookup misses, the <C>None</C> arm fires, and you get <C>-1</C>, no crash, just a value that says "nothing there".</P>
      <Why tenet="Partiality is data, not a trap">Reading past the end of a list, looking up a missing key, decoding bad text: in Cadenza these yield an <C>Option</C> (or a result), never a crash or a garbage value. Absence is an ordinary value your program <em>handles</em>. If you ever do want "crash on missing", that's a single, explicit operation that demands a message, so the one place a program turns absence into a halt is visible right where it happens, not hidden inside every accessor.</Why>
      <H2>Updating a slot without touching the original</H2>
      <P><C>List.update</C> takes a list, an index, and a new value, and hands back a list with that one slot changed. The word "update" is doing something specific here: it does <em>not</em> reach into your list and overwrite a slot, since nothing in Cadenza does that. It builds a <em>new</em> list. The old one is still exactly what it was. This snippet proves it: bind a list, make a bumped version, then read the original back:</P>
      <Runnable
        source={`(let
  ((original #list(10 20 30)) (bumped (List.update original 1 99)))
  (match (List.at original 1) ((Some x) x) ((None _) 0)))`}
        id="list-update-immut"
      />
      <P>The answer is <C>20</C>, not <C>99</C>: <C>original</C> never changed. The <C>99</C> lives only in <C>bumped</C>. Swap <C>original</C> for <C>bumped</C> in the <C>List.at</C> line and Run again to see <C>99</C>. Two lists, sharing most of their structure internally, each with its own value.</P>
      <P>Return the updated list itself and you can see the change in place, one slot different, <Cadenza ast="Y2R6YXN0AAEEFAABCgABYwABHgUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(10 99 30)</Cadenza>, and the input <Cadenza ast="Y2R6YXN0AAEEFAABCgABFAABHgUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(10 20 30)</Cadenza> still intact wherever else it's held:</P>
      <Runnable
        source={`(List.update #list(10 20 30) 1 99)`}
      />
      <Why tenet="Immutable, persistent values">Every value in Cadenza is immutable; an "update" always produces a fresh value and leaves every existing reference untouched. That's not just tidiness; it's what makes the whole system tractable. Because values can never form a cycle, the runtime reclaims memory by simple reference counting, no garbage collector needed. Because a list you handed to a function can't change underneath you, there's a whole class of aliasing bug that simply cannot happen. And whether a list is stored as a flat array or a balanced tree is the runtime's business, not yours: one type, many representations, sharing structure between versions so <C>update</C> doesn't copy the whole thing.</Why>
      <H2>Lists through functions</H2>
      <P>A function can take a list and compute over it, and the element type rides along, so <C>count</C> works on a list of any element type:</P>
      <Runnable
        source={`(def (count xs) (List.len xs))

(def (main) (count #list(10 20 30 40)))`}
      />
      <P>Four elements in, so <C>4</C> out, and you never wrote the element type: <C>count</C> works on a list of anything, because <C>List.len</C> doesn't care what the elements are.</P>
      <P>To visit <em>every</em> element, match the list by shape. A <C>match</C> on a list has two cases: the empty list <Cadenza ast="Y2R6YXN0AAEBFAIAAAEBAAE=" kind="expr">#list()</Cadenza>, and a non-empty one <Cadenza ast="Y2R6YXN0AAEEFAoBeAoCLi4KBHJlc3QGAAAAAQACAAMBAgIDAQMAAQQF" kind="expr">#list(x (.. rest))</Cadenza>, which binds the first element to <C>x</C> and the <em>rest</em> of the list to <C>rest</C>. Recurse on <C>rest</C> and you fold over the whole list. Here <C>sum</C> adds the elements:</P>
      <Runnable
        source={`(def (sum xs) (match xs (#list() 0) (#list(x (.. rest)) (+ x (sum rest)))))

(def (main) (sum #list(10 20 30)))`}
      />
      <P>The empty case is the base case, <C>0</C>, the sum of nothing, and each step peels off one element and sums the rest, so <Cadenza ast="Y2R6YXN0AAEEFAABCgABFAABHgUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(10 20 30)</Cadenza> is <C>10 + (20 + (30 + 0))</C> = <C>60</C>. You didn't declare the element type either: it flows from the <C>+</C>, so <C>sum</C> is inferred over a list of <C>Int64</C>.</P>
      <P>The <C>rest</C> after <C>..</C> is special: it binds the <em>whole tail</em> as one sublist, so it must be a plain name (or <C>_</C>), not another pattern. You can destructure the leading elements as deeply as you like, but you can't nest a pattern in the rest slot itself. This tries to, and the compiler stops you:</P>
      <Note>This one is <strong>meant to be rejected</strong>: <Cadenza ast="Y2R6YXN0AAEEFAoBYgoCLi4KAXIGAAAAAQACAAMBAgIDAQMAAQQF" kind="expr">#list(b (.. r))</Cadenza> in the rest position asks to match the tail against a shape, but the rest binder only ever names the tail. The fix is to bind it, then match it.</Note>
      <Runnable
        source={`(match #list(1 2 3) (#list(a (.. #list(b (.. r)))) a) (_ 0))`}
        expect="error"
      />
      <P>To reach the second element, bind the tail to a name and match <em>that</em>, a two-step you'll use whenever you need more than the head: peel one layer, then look again. Here <C>rest</C> is <Cadenza ast="Y2R6YXN0AAEDFAABFAABHgQAAAABAAIBAwABAgM=" kind="expr">#list(20 30)</Cadenza>, and matching it pulls out <C>20</C>:</P>
      <Runnable
        source={`(match
  #list(10 20 30)
  (#list(a (.. rest)) (match rest (#list(b (.. r)) b) (#list() 0)))
  (#list() 0))`}
        id="list-rest-match"
      />
      <P>A leading element can still be any pattern, so <Cadenza ast="Y2R6YXN0AAEGFBUKAXgKAXkKAi4uCgRyZXN0CQAAAAEAAgADAQMBAgMABAAFAQIFBgEDAAQHCA==" kind="expr">#list(#tuple(x y) (.. rest))</Cadenza> is fine, only the rest slot is name-only. Reach for the tail by name and match it again when you need to see inside.</P>
      <P>A list holds every element at once. Sometimes you want the elements without ever building the whole sequence, even an endless one. That's an <em>iterator</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="lists:1"
        prompt={<>Use <C>List.update</C> to change index <C>0</C> of <Cadenza ast="Y2R6YXN0AAEEFAABBQABBgABBwUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(5 6 7)</Cadenza> to <C>50</C>, then read that slot back with <C>List.at</C>. The answer is <C>50</C>.</>}
        starter={`(match (List.at (List.update #list(5 6 7) 0 ?) 0) ((Some x) x) ((None _) 0))`}
        solution={`(match (List.at (List.update #list(5 6 7) 0 50) 0) ((Some x) x) ((None _) 0))`}
        expected="50"
        hint={<><C>List.update</C> takes the list, the index (<C>0</C>), and the new value (<C>50</C>), in that order. Then <C>List.at … 0</C> reads the slot you just set.</>}
      />
      <Exercise
        id="lists:2"
        prompt={<>Add the single element <C>99</C> to the end of <Cadenza ast="Y2R6YXN0AAEEFAABCgABFAABHgUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(10 20 30)</Cadenza>, then ask its length, so a three-element list grows to <C>4</C>. Which operation appends <em>one element</em>, <C>push</C> or <C>concat</C>? Fill in the blank.</>}
        starter={`(List.len (List.? #list(10 20 30) 99))`}
        solution={`(List.len (List.push #list(10 20 30) 99))`}
        expected="4"
        hint={<><C>push</C> adds a single element to the end; <C>concat</C> joins two <em>lists</em> (and would reject the bare <C>99</C>). Pushing one element onto three gives length <C>4</C>.</>}
      />
      <Exercise
        id="lists:3"
        prompt={<>Here's the same fold shape as <C>sum</C>, but <em>multiplying</em>, and this time the recursive step is written for you; the <em>empty</em> case is the hole. What should <C>prod</C> of the empty list be, so that folding <Cadenza ast="Y2R6YXN0AAEFFAABAQABAgABAwABBAYAAAABAAIAAwAEAQUAAQIDBAU=" kind="expr">#list(1 2 3 4)</Cadenza> gives <C>24</C>? Fill in the base case.</>}
        starter={`(def (prod xs) (match xs (#list() ?) (#list(x (.. rest)) (* x (prod rest)))))

(def (main) (prod #list(1 2 3 4)))`}
        solution={`(def (prod xs) (match xs (#list() 1) (#list(x (.. rest)) (* x (prod rest)))))

(def (main) (prod #list(1 2 3 4)))`}
        expected="24"
        hint={<>The base case has to be the value that leaves a product unchanged, so multiplying by it does nothing. For <C>+</C> that identity was <C>0</C>; for <C>*</C> it's <C>1</C>. (Try <C>0</C> and watch the whole product collapse to <C>0</C>.)</>}
      />
      <Exercise
        id="lists:4"
        prompt={<>Add <C>0</C> to the <em>front</em> of <Cadenza ast="Y2R6YXN0AAEEFAABAQABAgABAwUAAAABAAIAAwEEAAECAwQ=" kind="expr">#list(1 2 3)</Cadenza>, then read index <C>0</C> back with <C>List.at</C> for the answer <C>0</C>. Which operation adds to the front, <C>push</C> or <C>prepend</C>? Fill in the blank.</>}
        starter={`(match (List.at (List.? #list(1 2 3) 0) 0) ((Some x) x) ((None _) -1))`}
        solution={`(match (List.at (List.prepend #list(1 2 3) 0) 0) ((Some x) x) ((None _) -1))`}
        expected="0"
        hint={<><C>push</C> adds to the end, so index <C>0</C> would stay <C>1</C>; <C>prepend</C> adds to the front, so the new <C>0</C> becomes the element at index <C>0</C>.</>}
      />
    </article>
  );
}
