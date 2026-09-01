// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function MapsSets() {
  return (
    <article>
      <H1>Maps & sets</H1>
      <Lede>A list keeps things in order. When you care about <em>membership</em> or a <em>key→value</em> association instead, reach for a set or a map.</Lede>
      <P>Like lists, both are immutable, persistent values: every "insert" or "remove" returns a new collection and leaves the original alone. They just answer a different question: a list is about position, a set is about "is this in here?", a map is about "what's stored under this key?".</P>
      <H2>Sets: membership, without duplicates</H2>
      <P>Build one from a list with <C>Set.of</C>. A set collapses duplicates and forgets order; return one and you'll see the collapse directly: the two <C>2</C>s in the input become a single <C>2</C>, so the set holds <C>1 2 3</C>:</P>
      <Runnable
        source={`(Set.of #list(1 2 2 3))`}
      />
      <P><C>Set.contains</C> answers membership directly:</P>
      <Runnable
        source={`(Set.contains (Set.of #list(1 2 3)) 2)`}
      />
      <H2>Set algebra</H2>
      <P>Sets combine the way they do in maths: <C>Set.union</C> (in either), <C>Set.intersection</C> (in both), <C>Set.difference</C> (in the first but not the second). Each returns a new set; Run it and you see the set itself:</P>
      <Runnable
        source={`(Set.union (Set.of #list(1 2)) (Set.of #list(2 3 4)))`}
      />
      <P><C>{"{1,2}"}</C> ∪ <C>{"{2,3,4}"}</C> = <C>{"{1,2,3,4}"}</C>: the duplicate <C>2</C> collapses, so four distinct elements. Put <C>Set.intersection</C> in its place and Run again: only the shared <C>2</C> survives, so you get <C>{"{2}"}</C>.</P>
      <P><C>Set.difference</C> is the one where <em>order matters</em>: it keeps what's in the first set and not the second. Return the set itself and you can see it: <C>{"{1,2,3}"}</C> minus <C>{"{2,3,4}"}</C> leaves just <C>{"{1}"}</C>:</P>
      <Runnable
        source={`(Set.difference (Set.of #list(1 2 3)) (Set.of #list(2 3 4)))`}
      />
      <P>Swap the two sets and Run again: <C>{"{2,3,4}"}</C> minus <C>{"{1,2,3}"}</C> is <C>{"{4}"}</C> instead, a different answer, because "in the first but not the second" isn't symmetric. Union and intersection don't care which side is which; difference does.</P>
      <H2>Maps: values under keys</H2>
      <P>A map starts empty with <C>Map.empty</C> and grows with <C>Map.insert</C>. <C>Map.len</C> reports how many keys it holds:</P>
      <Runnable
        source={`(def (main) (Map.len (Map.insert (Map.insert (Map.empty) 1 10) 2 20)))`}
      />
      <P><C>Map.lookup</C> is what you reach for a map to do, and like reaching into a list, it can miss. So it returns an <C>Option</C>: <Cadenza ast="Y2R6YXN0AAECCgRTb21lCgF2AwAAAAEBAgABAg==" kind="expr">(Some v)</Cadenza> when the key is present, <Cadenza ast="Y2R6YXN0AAECCgROb25lCgR1bml0AwAAAAEBAgABAg==" kind="expr">(None unit)</Cadenza> when it isn't. You take it apart with <C>match</C>:</P>
      <Runnable
        source={`(def (main) (match (Map.lookup (Map.insert (Map.empty) 7 99) 7) ((Some v) v) ((None _) 0)))`}
      />
      <P>Look up a key that isn't there (change the second <C>7</C> to <C>8</C>) and the <C>None</C> arm gives <C>0</C>: no crash, just "nothing under that key".</P>
      <H2>Literal shorthand</H2>
      <P>Writing <C>Set.of</C> over a list, or a chain of <C>Map.insert</C>s, gets wordy. The conventional surface has a shorthand for each, rounding out the <C>#</C>-prefixed literal family you've been seeing: <C>[…]</C> is a list, <C>#(…)</C> is a <em>set</em>, and <C>{"#{…}"}</C> is a <em>map</em>. They're pure sugar (the same programs underneath), so this set literal is exactly the <C>Set.of</C> call from the top of the chapter, and it still collapses the duplicate to the set <C>{"{1, 2, 3}"}</C>:</P>
      <Runnable
        source={`(Set.of #list(1 2 2 3))`}
      />
      <P>Toggle to the conventional surface and that reads <C>#(1, 2, 2, 3)</C>, and in fact every <C>Set.of #list(…)</C> earlier in this chapter has been showing as <C>#(…)</C> whenever the toggle was on. A map literal spells each entry <C>key = value</C> inside <C>{"#{…}"}</C>; here two entries, and returning it shows both:</P>
      <Runnable
        source={`#map((= 1 10) (= 2 20))`}
      />
      <P>And it's an ordinary map, so <C>Map.lookup</C> works on it just the same: the value under key <C>2</C> is <C>20</C>:</P>
      <Runnable
        source={`(def (main) (Option.expect (Map.lookup #map((= 1 10) (= 2 20)) 2) "missing"))`}
      />
      <Note>These are the same three collections, not new ones: <C>#(…)</C> desugars to <C>Set.of</C> and <C>{"#{…}"}</C> to a map, exactly the forms above. Reach for the literal when you're writing a collection out by hand; reach for <C>Set.of</C> / <C>Map.insert</C> when you're building one from values you already have.</Note>
      <Why tenet="One question per collection">List, set, map: three shapes for three questions: order, membership, association. Picking the right one puts your intent in the type, and lets the compiler pick an efficient representation (a hash trie for a map or set; an array or tree for a list) without you managing it. And all three share the same discipline as the rest of the language: they're immutable, so an "update" is a new value; a lookup that can miss returns an <C>Option</C> rather than a crash or a bogus default.</Why>
      <H2>Inserting over a key replaces it</H2>
      <P>A map holds one value per key, so inserting the same key again replaces the old value, and the size doesn't grow:</P>
      <Runnable
        source={`(def
  (main)
  (Option.expect (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 1 99) 1) "missing"))`}
      />
      <P>Two inserts at key <C>1</C>, and the second one wins: <C>99</C>.</P>
      <Note>Keys and elements are compared by value, using the same structural equality as everywhere else: two equal keys <em>are</em> the same key, whatever built them. That's why <C>Set.of</C> can collapse duplicates and a re-insert can replace: equality is a property of values, not of identity.</Note>
      <H2>Removing and reporting in one step: <C>Map.take</C></H2>
      <P><C>Map.remove</C> discards whatever was under the key. When you want to <em>see</em> it on the way out, <C>Map.take</C> does both at once: it returns a tuple of the value that was there (as an <C>Option</C>, since the key might be absent) and the new map with the key gone. Reach the dropped value with <C>.0</C> and <C>match</C> it; here taking key <C>1</C> from a two-entry map reports the <C>10</C> it held:</P>
      <Runnable
        source={`(def (main) (match (. (Map.take #map((= 1 10) (= 2 20)) 1) 0) ((Some v) v) ((None _) -1)))`}
      />
      <P>The other half of the tuple, <C>.1</C>, is the smaller map with one entry left. Return it and you can see the removal: <C>{"{2 = 20}"}</C>, with key <C>1</C> gone:</P>
      <Runnable
        source={`(def (main) (. (Map.take #map((= 1 10) (= 2 20)) 1) 1))`}
      />
      <P>Take a key that isn't there and <C>.0</C> is <Cadenza ast="Y2R6YXN0AAECCgROb25lCgR1bml0AwAAAAEBAgABAg==" kind="expr">(None unit)</Cadenza> while <C>.1</C> equals the original: removal stays total, and you learn it held nothing in the same step.</P>
      <P><C>Map.insert</C> has the same value-yielding twin, <C>Map.swap</C>: it inserts (or replaces) and reports what the key held <em>before</em>, again as a <C>(prior-value . new-map)</C> tuple. So swapping key <C>1</C> (already <C>10</C>) for <C>99</C> hands back the old <C>10</C> in <C>.0</C>, no separate lookup needed:</P>
      <Runnable
        source={`(def (main) (match (. (Map.swap #map((= 1 10)) 1 99) 0) ((Some old) old) ((None _) -1)))`}
      />
      <P>Swap a key that's new and <C>.0</C> is <Cadenza ast="Y2R6YXN0AAECCgROb25lCgR1bml0AwAAAAEBAgABAg==" kind="expr">(None unit)</Cadenza>: nothing was replaced. Between them, <C>take</C> reports what a remove <em>dropped</em> and <C>swap</C> what an insert <em>overwrote</em>, each in a single step.</P>
      <P>Numbers, symbols, lists, maps: all collections of values. Text is its own thing, with its own honest questions (how long <em>is</em> a string?). <em>Strings &amp; text</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="maps-sets:1"
        prompt={<>How many elements are in <em>both</em> <C>{"{1,2,3}"}</C> and <C>{"{2,3,4}"}</C>? Use <C>Set.intersection</C> and count the result: the shared elements are <C>2</C> and <C>3</C>, so the answer is <C>2</C>.</>}
        starter={`(Set.len (Set.? (Set.of #list(1 2 3)) (Set.of #list(2 3 4))))`}
        solution={`(Set.len (Set.intersection (Set.of #list(1 2 3)) (Set.of #list(2 3 4))))`}
        expected="2"
        hint={<>"In both" is the intersection, <C>Set.intersection</C>. Only <C>2</C> and <C>3</C> appear in each set.</>}
      />
      <Exercise
        id="maps-sets:2"
        prompt={<>This map literal holds two keys, <C>1</C> and <C>2</C>. Every "update" is a new map, so removing a key builds one without it, which is what <C>Map.remove</C> does. Take one key away, then ask <C>Map.len</C> how many remain: the answer should be <C>1</C>. Fill in the operation.</>}
        starter={`(def (main) (Map.len (Map.? #map((= 1 10) (= 2 20)) 1)))`}
        solution={`(def (main) (Map.len (Map.remove #map((= 1 10) (= 2 20)) 1)))`}
        expected="1"
        hint={<>The op that deletes a key is <C>Map.remove</C>; it takes the map and the key. Two keys minus one leaves <C>1</C>, and the original map, as ever, is untouched.</>}
      />
    </article>
  );
}
