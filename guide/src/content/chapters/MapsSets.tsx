import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function MapsSets() {
  return (
    <article>
      <H1>Maps &amp; sets</H1>
      <Lede>
        A list keeps things in order. When you care about <em>membership</em> or a <em>key→value</em>{" "}
        association instead, reach for a set or a map.
      </Lede>

      <P>
        Like lists, both are immutable, persistent values: every "insert" or "remove" returns a new
        collection and leaves the original alone. They just answer a different question — a list is about
        position, a set is about "is this in here?", a map is about "what's stored under this key?".
      </P>

      <H2>Sets: membership, without duplicates</H2>
      <P>
        Build one from a list with <C>Set.of</C>. A set collapses duplicates and forgets order, so its{" "}
        <C>Set.len</C> counts <em>distinct</em> elements — here the two <C>2</C>s become one:
      </P>
      <Runnable source={`(Set.len (Set.of (list 1 2 2 3)))`} />
      <P>
        <C>Set.contains</C> answers membership directly:
      </P>
      <Runnable
        source={`(def (main)
  (if (Set.contains (Set.of (list 1 2 3)) 2) 1 0))`}
      />

      <H2>Set algebra</H2>
      <P>
        Sets combine the way they do in maths: <C>Set.union</C> (in either), <C>Set.intersection</C> (in
        both), <C>Set.difference</C> (in the first but not the second). Each returns a new set; measure it
        with <C>Set.len</C>:
      </P>
      <Runnable source={`(Set.len (Set.union (Set.of (list 1 2)) (Set.of (list 2 3 4))))`} />
      <P>
        <C>{`{1,2}`}</C> ∪ <C>{`{2,3,4}`}</C> = <C>{`{1,2,3,4}`}</C>, so four distinct elements. Try{" "}
        <C>Set.intersection</C> in its place — you'll get <C>1</C> (just the shared <C>2</C>).
      </P>

      <H2>Maps: values under keys</H2>
      <P>
        A map starts empty with <C>Map.empty</C> and grows with <C>Map.insert</C>. <C>Map.size</C> reports
        how many keys it holds:
      </P>
      <Runnable
        source={`(def (main)
  (Map.size (Map.insert (Map.insert (Map.empty) 1 10) 2 20)))`}
      />
      <P>
        <C>Map.lookup</C> is the payoff — and, like reaching into a list, it can miss. So it returns an{" "}
        <C>Option</C>: <C>(Some v)</C> when the key is present, <C>(None unit)</C> when it isn't. You take
        it apart with <C>match</C>:
      </P>
      <Runnable
        source={`(def (main)
  (match (Map.lookup (Map.insert (Map.empty) 7 99) 7)
    ((Some v) v)
    ((None _) 0)))`}
      />
      <P>
        Look up a key that isn't there (change the second <C>7</C> to <C>8</C>) and the <C>None</C> arm
        gives <C>0</C> — no crash, just "nothing under that key".
      </P>

      <Why tenet="One question per collection">
        List, set, map — three shapes for three questions: order, membership, association. Picking the
        right one puts your intent in the type, and lets the compiler pick an efficient representation (a
        hash trie for a map or set; an array or tree for a list) without you managing it. And all three
        share the same discipline as the rest of the language: they're immutable, so an "update" is a new
        value; a lookup that can miss returns an <C>Option</C> rather than a crash or a bogus default.
      </Why>

      <H2>Inserting over a key replaces it</H2>
      <P>
        A map holds one value per key, so inserting the same key again replaces the old value — the size
        doesn't grow:
      </P>
      <Runnable
        source={`(def (main)
  (Option.expect
    (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 1 99) 1)
    "missing"))`}
      />
      <P>Two inserts at key <C>1</C>, and the second one wins: <C>99</C>.</P>

      <Note>
        Keys and elements are compared by value, using the same structural equality as everywhere else —
        two equal keys <em>are</em> the same key, whatever built them. That's why <C>Set.of</C> can
        collapse duplicates and a re-insert can replace: equality is a property of values, not of
        identity.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="maps-sets:1"
        prompt={<>Count the distinct elements of <C>(list 3 1 3 3 2)</C> — the answer is <C>3</C>.</>}
        starter={`(Set.len (Set.of ?))`}
        solution={`(Set.len (Set.of (list 3 1 3 3 2)))`}
        expected="3"
        hint={<>Pass the list to <C>Set.of</C>; the set keeps only distinct values, so the three <C>3</C>s count once.</>}
      />

      <Exercise
        id="maps-sets:2"
        prompt={<>Look up key <C>2</C> in a map that stores <C>2 → 20</C>, so the answer is <C>20</C>.</>}
        starter={`(def (main)
  (match (Map.lookup (Map.insert (Map.empty) 2 20) ?)
    ((Some v) v)
    ((None _) 0)))`}
        solution={`(def (main)
  (match (Map.lookup (Map.insert (Map.empty) 2 20) 2)
    ((Some v) v)
    ((None _) 0)))`}
        expected="20"
        hint={<>Look up the key you inserted — <C>2</C>.</>}
      />
    </article>
  );
}
