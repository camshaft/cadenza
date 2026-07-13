import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function PatternMatching() {
  return (
    <article>
      <H1>Pattern matching</H1>
      <Lede>
        Deciding by shape — and why <C>match</C> is a set of patterns the compiler can check, not a
        chain of <C>if</C>s.
      </Lede>

      <H2>Matching literals</H2>
      <P>
        A <C>match</C> chooses an arm by matching the scrutinee against each arm's <em>pattern</em>. The
        last arm here uses <C>_</C>, the wildcard, which matches anything:
      </P>
      <Runnable source={`(match 2
  (1 10)
  (2 20)
  (_ 0))`} />
      <P>Change the <C>2</C> being matched to <C>1</C> or <C>7</C> and Run to see a different arm fire.</P>

      <H2>Sum types</H2>
      <P>
        A sum type is a set of tagged variants. You declare it with <C>type</C>, build a value with one
        of its constructors, and take it apart by matching each variant. Here <C>Opt</C> is either{" "}
        <C>Some</C> carrying an <C>Int64</C>, or <C>None</C>. The <C>(Some x)</C> arm <em>binds</em> the
        payload to <C>x</C>:
      </P>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))
(def (main)
  (match (Some 7)
    ((Some x) x)
    ((None _) 0)))`}
      />
      <P>
        Swap <C>(Some 7)</C> for <C>(None unit)</C> and Run to take the other arm — it returns <C>0</C>.
      </P>

      <H2>The compiler checks you covered every case</H2>
      <P>
        Here's the payoff. Drop the <C>None</C> arm and the compiler <em>refuses</em> to compile — it can
        see, from the type, that a case is unhandled:
      </P>
      <Note>
        This one is <strong>meant to be refused</strong>. Run it and read the status bar:{" "}
        <C>non-exhaustive match</C> — the missing variant named for you, before the program ever runs.
      </Note>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))
(def (main)
  (match (Some 7)
    ((Some x) x)))`}
        expect="error"
      />

      <Why tenet="match is patterns, not predicates">
        Many languages let a branch head be any boolean test. Cadenza deliberately doesn't: a{" "}
        <C>match</C> arm is always a <em>pattern</em> — a constructor that destructures, a literal, a
        binding, or <C>_</C>. Why refuse the more flexible option? Because a head that could be an
        arbitrary predicate quietly demotes the real question — <em>"did you handle every variant?"</em>{" "}
        — down to <em>"is there an else?"</em>. Keeping arms as patterns is what lets the compiler check
        exhaustiveness against the type, and turn a whole class of "forgot a case" bugs into compile
        errors. Value conditions still have a home; it's just <C>if</C>, not <C>match</C>.
      </Why>

      <H2>Guards: a pattern plus a condition</H2>
      <P>
        When you do want to test a value — not just its shape — an arm can carry a <em>guard</em>: a
        pattern with an <C>if</C> condition. The arm fires only when the pattern matches <em>and</em> the
        guard holds. Here a number is classified by sign:
      </P>
      <Runnable
        source={`(def (sign n)
  (match n
    ((guard x (< x 0)) (- 0 1))
    (0 0)
    (_ 1)))
(def (main) (sign (- 0 8)))`}
      />
      <P>
        <C>(guard x (&lt; x 0))</C> binds the value to <C>x</C> and fires only when <C>x &lt; 0</C>, so{" "}
        <C>-8</C> returns <C>-1</C>; <C>0</C> takes the literal arm, and everything else the wildcard. A
        guard is the bridge between "match on shape" and "decide on value" — without turning the whole
        arm back into an arbitrary predicate.
      </P>

      <H2>More than two variants</H2>
      <P>
        Sums aren't limited to <C>Some</C>/<C>None</C>. A traffic light is a three-variant sum, and a{" "}
        <C>match</C> over it must cover all three (or the compiler complains):
      </P>
      <Runnable
        source={`(type Light (Red unit) (Yellow unit) (Green unit))
(def (wait l)
  (match l
    ((Red _) 30)
    ((Yellow _) 5)
    ((Green _) 0)))
(def (main) (wait (Red unit)))`}
      />
      <Note>
        This is the typed cousin of the symbol dispatch from the Symbols chapter. A symbol tag is checked
        with <C>=</C> and any typo compiles; a sum's variants are checked by the compiler, so a forgotten
        or misspelled case is caught. Reach for a sum when the set of cases is fixed and worth enforcing.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="pattern-matching:1"
        prompt={<>Add the missing <C>None</C> arm so this compiles and returns <C>0</C>.</>}
        starter={`(type Opt (Some Int64) (None unit))
(def (main)
  (match (None unit)
    ((Some x) x)
    ?))`}
        solution={`(type Opt (Some Int64) (None unit))
(def (main)
  (match (None unit)
    ((Some x) x)
    ((None _) 0)))`}
        expected="0"
        hint={<>The missing case is <C>None</C>; an arm is a <C>(pattern body)</C> pair: <C>((None _) 0)</C>.</>}
      />

      <Exercise
        id="pattern-matching:2"
        prompt={<>Finish <C>wait</C>'s <C>Green</C> arm so a green light gives <C>0</C>.</>}
        starter={`(type Light (Red unit) (Yellow unit) (Green unit))
(def (wait l)
  (match l
    ((Red _) 30)
    ((Yellow _) 5)
    ((Green _) ?)))
(def (main) (wait (Green unit)))`}
        solution={`(type Light (Red unit) (Yellow unit) (Green unit))
(def (wait l)
  (match l
    ((Red _) 30)
    ((Yellow _) 5)
    ((Green _) 0)))
(def (main) (wait (Green unit)))`}
        expected="0"
        hint={<>A green light means "go" — no wait, so the arm's body is <C>0</C>.</>}
      />
    </article>
  );
}
