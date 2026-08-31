// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function PatternMatching() {
  return (
    <article>
      <H1>Pattern matching</H1>
      <Lede>Deciding by shape, and why <C>match</C> is a set of patterns the compiler can check, not a chain of <C>if</C>s.</Lede>
      <H2>Matching literals</H2>
      <P>A <C>match</C> chooses an arm by matching the scrutinee against each arm's <em>pattern</em>. The last arm here uses <C>_</C>, the wildcard, which matches anything:</P>
      <Runnable
        source={`(match 2 (1 10) (2 20) (_ 0))`}
      />
      <P>Change the <C>2</C> being matched to <C>1</C> or <C>7</C> and Run to see a different arm fire.</P>
      <P>Literals aren't just numbers, since you can match a <C>String</C> the same way, and this is the everyday shape for dispatching on a keyword or command name. Here <C>known-op</C> reports whether a name is one of the operations it recognises, answering <C>false</C> for everything else:</P>
      <Runnable
        source={`(def (known-op name) (match name ("add" true) ("sub" true) (_ false)))

(def (main) (known-op "sub"))`}
      />
      <P><C>"sub"</C> takes the second arm, <C>true</C>. Change it to <C>"add"</C> or something unknown like <C>"mul"</C> and Run again. Note the answer is an honest <C>Bool</C>, not a stand-in number: a recognised name is <C>true</C>, anything else is <C>false</C>. (When a lookup needs to hand back a <em>result</em> that might not exist, you reach for <C>Option</C> rather than a magic value like <C>-1</C>, which is exactly what the next section builds.) The <C>_</C> arm isn't optional here: <C>String</C> (like <C>Int64</C>) has infinitely many values, so the compiler can't see that you've covered them all: leave the wildcard off and it declines with a non-exhaustive-match error, the same guarantee you'll meet with sums below.</P>
      <P>The same shape works for characters. A <C>Char</C> literal is written <C>#\a</C> (the <strong>Strings &amp; text</strong> chapter covers characters), and a <C>match</C> dispatches on one by its Unicode code point. Here <C>is-vowel</C> answers whether a character is a lowercase vowel:</P>
      <Runnable
        source={`(def (is-vowel c) (match c (#\\a true) (#\\e true) (#\\i true) (#\\o true) (#\\u true) (_ false)))

(def (main) (is-vowel #\\e))`}
      />
      <P><C>#\e</C> takes its arm, so the answer is <C>true</C>. Change it to a consonant like <C>#\z</C> and the wildcard arm answers <C>false</C>. As with numbers and strings, the <C>_</C> arm is required, since <C>Char</C> has far too many values for the compiler to see them all listed.</P>
      <H2>Sum types</H2>
      <P>A sum type is a set of tagged variants. You declare it with <C>type</C>, build a value with one of its constructors, and take it apart by matching each variant. Here <C>Opt</C> is either <C>Some</C> carrying an <C>Int64</C>, or <C>None</C>. The <C>(Some x)</C> arm <em>binds</em> the payload to <C>x</C>:</P>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))

(def (main) (match (Some 7) ((Some x) x) ((None _) 0)))`}
      />
      <P>Swap <C>(Some 7)</C> for <C>(None unit)</C> and Run to take the other arm, which returns <C>0</C>.</P>
      <H2>The compiler checks you covered every case</H2>
      <P>Exhaustiveness is what this buys you. Drop the <C>None</C> arm and the compiler <em>refuses</em> to compile, because it can see, from the type, that a case is unhandled:</P>
      <Note>This one is <strong>meant to be refused</strong>. Run it and read the status bar: <C>non-exhaustive match</C>, the missing variant named for you, before the program ever runs.</Note>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))

(def (main) (match (Some 7) ((Some x) x)))`}
        expect="error"
      />
      <Why tenet="match is patterns, not predicates">Many languages let a branch head be any boolean test. Cadenza deliberately doesn't: a <C>match</C> arm is always a <em>pattern</em>, whether a constructor that destructures, a literal, a binding, or <C>_</C>. Why refuse the more flexible option? Because a head that could be an arbitrary predicate quietly demotes the real question, <em>"did you handle every variant?"</em>, down to <em>"is there an else?"</em>. Keeping arms as patterns is what lets the compiler check exhaustiveness against the type, and turn a whole class of "forgot a case" bugs into compile errors. Value conditions still have a home; it's just <C>if</C>, not <C>match</C>.</Why>
      <H2>Guards: a pattern plus a condition</H2>
      <P>When you do want to test a value, not just its shape, an arm can carry a <em>guard</em>: a pattern with an <C>if</C> condition. The arm fires only when the pattern matches <em>and</em> the guard holds. Here a number is classified by sign:</P>
      <Runnable
        source={`(def (sign n) (match n ((guard x (< x 0)) -1) (0 0) (_ 1)))

(def (main) (sign -8))`}
      />
      <P><C>(guard x (&lt; x 0))</C> binds the value to <C>x</C> and fires only when <C>x &lt; 0</C>, so <C>-8</C> returns <C>-1</C>; <C>0</C> takes the literal arm, and everything else the wildcard. A guard is the bridge between "match on shape" and "decide on value", without turning the whole arm back into an arbitrary predicate.</P>
      <H2>More than two variants</H2>
      <P>Sums aren't limited to <C>Some</C>/<C>None</C>. A traffic light is a three-variant sum, and a <C>match</C> over it must cover all three (or the compiler complains):</P>
      <Runnable
        source={`(type Light (Red unit) (Yellow unit) (Green unit))

(def (wait l) (match l ((Red _) 30) ((Yellow _) 5) ((Green _) 0)))

(def (main) (wait (Red unit)))`}
      />
      <Note>This is the typed cousin of the symbol dispatch from the Symbols chapter. A symbol tag is checked with <C>=</C> and any typo compiles; a sum's variants are checked by the compiler, so a forgotten or misspelled case is caught. Reach for a sum when the set of cases is fixed and worth enforcing.</Note>
      <H2>Matching a map by key</H2>
      <P>A <C>match</C> can also look <em>inside a collection</em>. A map pattern, <C>#map((= key binder) (.. rest))</C>, fires when the map contains that key, binding the associated value to <C>binder</C> (and the leftover entries to <C>rest</C>). It's the pattern-matching counterpart to a <C>Map.lookup</C>: here <C>setting</C> reads the <C>"width"</C> from a config map, returning <C>(Some v)</C> when the key is present and <C>(None unit)</C> when it's absent, because a missing key is an absence, not a magic number:</P>
      <Runnable
        source={`(def (setting m) (match m (#map((= "width" v) (.. rest)) (Some v)) (_ (None unit))))

(def (main) (setting (Map.insert (Map.insert (Map.empty) "width" 80) "height" 50)))`}
      />
      <P>The map has a <C>"width"</C>, so the arm fires, binds <C>v</C> to <C>80</C>, and returns <C>(Some 80)</C>. Drop that key from the map and the pattern no longer matches, so it falls through to the wildcard and returns <C>(None unit)</C>. Toggle to the conventional surface and the pattern reads as <C>{"#{ \"width\" = v, .. rest }"}</C>, a map-literal shape on the left of a match arm (the <strong>Maps &amp; sets</strong> chapter later builds out maps as values).</P>
      <H2>Matching a tuple's shape</H2>
      <P>A tuple pattern takes a value apart by <em>position</em>, and a trailing rest marker <Cadenza>(.. rest)</Cadenza> gathers the elements you didn't name into a smaller tuple, the positional twin of a list's <C>.. rest</C>. Here <Cadenza>#tuple(a b (.. rest))</Cadenza> binds <C>a</C> and <C>b</C> to the first two elements and <C>rest</C> to a tuple of whatever trails, so reading <C>rest</C> back with <C>.0</C> recovers the third element:</P>
      <Runnable
        source={`(match #tuple(3 4 5) (#tuple(a b (.. rest)) (+ (+ a b) (. rest 0))) (_ 0))`}
      />
      <P>So <C>a</C> is <C>3</C>, <C>b</C> is <C>4</C>, and <C>rest</C> is the one-element tuple <Cadenza>#tuple(5)</Cadenza>, whose <C>.0</C> is <C>5</C>, giving <C>3 + 4 + 5 = 12</C>. Two things to hold onto: <C>rest</C> is the trailing <em>sub-tuple</em>, not a flattened list, so a <Cadenza>#tuple(1 2 3 4)</Cadenza> matched by <Cadenza>#tuple(x (.. rest))</Cadenza> leaves <C>rest</C> as <Cadenza>#tuple(2 3 4)</Cadenza>, indexed <C>.0</C>/<C>.1</C>/<C>.2</C>; and the arity is fixed, so <Cadenza>#tuple(a b (.. rest))</Cadenza> needs at least two elements, and a shorter tuple simply doesn't match that arm.</P>
      <Note>The example above binds a rest over a tuple <em>constructed in place</em>, which is what this pattern supports. A rest binder over a fully opaque runtime tuple is <em>not supported</em> on the backends, so the compiler declines it with a clear message rather than compute a wrong answer, the same honest refusal you've seen elsewhere.</Note>
      <H2>Matching a record's fields</H2>
      <P>Records take a rest the same way, by <em>name</em> instead of position. A record pattern names the fields you care about and a trailing <Cadenza>(.. rest)</Cadenza> gathers the rest into a <em>residual record</em>, the record analogue of the tuple rest above. Here <Cadenza>#record((= a x) (.. rest))</Cadenza> binds <C>x</C> to field <C>a</C> and <C>rest</C> to a record of the remaining fields, whose own fields you read back by name:</P>
      <Runnable
        source={`(match #record((= a 1) (= b 2) (= c 3)) (#record((= a x) (.. rest)) (+ (+ x rest.b) rest.c)) (_ 0))`}
      />
      <P>So <C>x</C> is <C>1</C>, and <C>rest</C> is the residual record <Cadenza>#record((= b 2) (= c 3))</Cadenza>, so <C>rest.b</C> is <C>2</C> and <C>rest.c</C> is <C>3</C>, giving <C>1 + 2 + 3 = 6</C>. The key difference from the tuple case: <C>rest</C> here is a <em>record</em>, so you reach into it by field name (<C>rest.b</C>, <C>rest.c</C>), not by position.</P>
      <Note>As with the tuple rest, this supports a record <em>constructed in place</em>. A rest binder over a fully opaque runtime record is <em>not supported</em> on the backends, so the compiler declines it with a clear message rather than a wrong answer.</Note>
      <H2>Matching a set's members</H2>
      <P>A set has no fields or positions, so a set pattern asks a different question: <em>containment</em>. <Cadenza>#set(1 (.. rest))</Cadenza> names the members that must be <em>present</em> and matches any set that contains them, a subset test rather than an equality, exactly like a map pattern matching on the keys it names. The trailing <Cadenza>(.. rest)</Cadenza> then binds <C>rest</C> to the <em>residual set</em>, the scrutinee's members minus the ones you named. Here the set contains <C>1</C>, so the arm fires and <C>rest</C> is what's left:</P>
      <Runnable
        source={`(match #set(1 2 3) (#set(1 (.. rest)) (Some rest)) (_ (None unit)))`}
      />
      <P>The scrutinee <Cadenza>#set(1 2 3)</Cadenza> contains the named <C>1</C>, so the arm matches and <C>rest</C> binds the residual <Cadenza>#set(2 3)</Cadenza>, making the whole expression <Cadenza>(Some #set(2 3))</Cadenza>, the leftover set, returned as an <C>Option</C> since a set that lacks the named member takes the <C>None</C> arm instead. Three things follow from its being a containment test: it matches a <em>superset</em> too (<Cadenza>#set(1)</Cadenza> matches <Cadenza>#set(1 2 3)</Cadenza>), naming a member the set <em>lacks</em> refutes the arm (it falls to the wildcard), and order and duplicates in the pattern are immaterial because a set is unordered. It's the membership-axis twin of the map and record rest: same <Cadenza>(.. rest)</Cadenza> residual, asking "is this present?" instead of "what's at this field?".</P>
      <H2>Your turn</H2>
      <Exercise
        id="pattern-matching:1"
        prompt={<>Add the missing <C>None</C> arm so this compiles and returns <C>0</C>.</>}
        starter={`(type Opt (Some Int64) (None unit))

(def (main) (match (None unit) ((Some x) x) ?))`}
        solution={`(type Opt (Some Int64) (None unit))

(def (main) (match (None unit) ((Some x) x) ((None _) 0)))`}
        expected="0"
        hint={<>The missing case is <C>None</C>; an arm is a <C>(pattern body)</C> pair: <C>((None _) 0)</C>.</>}
      />
      <Exercise
        id="pattern-matching:2"
        prompt={<>Write the <em>guard condition</em> so <C>grade</C> returns <C>1</C> for a passing score of <C>60</C> or more, and <C>0</C> otherwise. With <C>(grade 75)</C> the answer is <C>1</C>.</>}
        starter={`(def (grade s) (match s ((guard x ?) 1) (_ 0)))

(def (main) (grade 75))`}
        solution={`(def (grade s) (match s ((guard x (>= x 60)) 1) (_ 0)))

(def (main) (grade 75))`}
        expected="1"
        hint={<>The guard binds the score to <C>x</C>; the condition for passing is "60 or more", namely <C>(&gt;= x 60)</C>. <C>75</C> clears it, so the first arm fires.</>}
      />
    </article>
  );
}
