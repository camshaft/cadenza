import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Symbols() {
  return (
    <article>
      <H1>Symbols</H1>
      <Lede>
        Sometimes a value is just a <em>name</em> — a status, a mode, one choice from a fixed set. A
        symbol is exactly that: an interned name you compare by identity.
      </Lede>

      <P>
        A symbol is written <C>#"…"</C> — the <C>#</C> tells it apart from a text string. Where a string
        is <em>content</em> you might slice, join, or measure, a symbol is a bare label whose only
        question is "is it this one?". So the operation on symbols is equality:
      </P>
      <Runnable
        source={`(def (main)
  (if (= #"red" #"red") 1 0))`}
      />
      <P>
        Two symbols are equal exactly when they're spelled the same — no matter how long the name, the
        comparison is a single identity check, not a character-by-character scan.
      </P>

      <Note>
        In the conventional surface the quotes are just noise when the name is a plain identifier, so{" "}
        <C>#"red"</C> may be written <C>#red</C> — the two are the same symbol. The quotes are only needed
        when the content isn't an identifier: a name with a space, a leading digit, or a dot (
        <C>#"List.at"</C>) keeps them. Toggle the syntax and a snippet's <C>#red</C> reappears as{" "}
        <C>#"red"</C> in the s-expression surface, where the quoted form is canonical.
      </Note>

      <H2>One choice from a fixed set</H2>
      <P>
        That's what symbols are for: a value drawn from a small, known set of names. A traffic light is{" "}
        <C>#"red"</C>, <C>#"yellow"</C>, or <C>#"green"</C>, and a function can decide on it — here, how
        many seconds to wait:
      </P>
      <Runnable
        source={`(def (wait light)
  (if (= light #"red") 30
    (if (= light #"yellow") 5
      0)))
(def (main) (wait #"red"))`}
      />
      <P>
        <C>#"red"</C> waits 30, <C>#"yellow"</C> 5, and anything else (green) 0. The light is passed
        around as a plain value and matched by name where the decision is made — no numbers to remember,
        no strings to keep in sync.
      </P>

      <Why tenet="A name compared by identity, not by its text">
        Why a distinct type, rather than just a string like <C>"red"</C>? Because the intent differs. A
        string is text you might transform; a symbol is a fixed label whose only meaning is <em>which
        label it is</em>. Making it its own type says so — you can't accidentally slice a status tag or
        take its length — and lets the compiler treat it as an interned identity (one cheap comparison,
        whatever the name's length) instead of a sequence to scan. Same instinct as keeping <C>Bytes</C>{" "}
        apart from <C>String</C>: one type per kind of thing, so the compiler catches a category mistake.
      </Why>

      <H2>From a string, explicitly</H2>
      <P>
        When a name arrives as text — parsed from input, or assembled at run time — <C>Symbol.of</C>{" "}
        interns it into a symbol. The result is the very same value as writing the literal: a symbol
        built from the pieces <C>"ye"</C> and <C>"s"</C> equals <C>#"yes"</C>:
      </P>
      <Runnable
        source={`(def (main)
  (if (= (Symbol.of (String.concat "ye" "s")) #"yes") 1 0))`}
      />

      <Note>
        Text-to-symbol is an explicit step (<C>Symbol.of</C>), just like <C>String.to-bytes</C> — the one
        place you cross between the two types is spelled out, so a symbol and a string never silently
        stand in for each other.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="symbols:1"
        prompt={
          <>
            <C>score</C> dispatches on a medal from the fixed set <C>#"gold"</C> / <C>#"silver"</C> /{" "}
            <C>#"bronze"</C>: gold scores <C>3</C>, silver <C>2</C>, anything else <C>1</C>. The gold and
            fallback arms are done — fill the middle comparison so <C>(score #"silver")</C> gives <C>2</C>.
          </>
        }
        starter={`(def (score m)
  (if (= m #"gold") 3
    (if (= m ?) 2 1)))
(def (main) (score #"silver"))`}
        solution={`(def (score m)
  (if (= m #"gold") 3
    (if (= m #"silver") 2 1)))
(def (main) (score #"silver"))`}
        expected="2"
        hint={
          <>
            The middle arm handles silver, so compare <C>m</C> against <C>#"silver"</C>. Each symbol is
            checked by equality; <C>#"bronze"</C> matches neither and falls through to <C>1</C>.
          </>
        }
      />

      <Exercise
        id="symbols:2"
        prompt={
          <>
            A symbol is an ordinary value, so a function can <em>return</em> one, not just test it.{" "}
            <C>next</C> advances a traffic light around its cycle: red turns to green, green to yellow,
            yellow back to red. The green and yellow cases are written — fill the hole with the symbol{" "}
            red becomes, so <C>(next #"red")</C> is <C>#"green"</C> and the check gives <C>1</C>.
          </>
        }
        starter={`(def (next light)
  (if (= light #"red") ?
    (if (= light #"green") #"yellow"
      #"red")))
(def (main) (if (= (next #"red") #"green") 1 0))`}
        solution={`(def (next light)
  (if (= light #"red") #"green"
    (if (= light #"green") #"yellow"
      #"red")))
(def (main) (if (= (next #"red") #"green") 1 0))`}
        expected="1"
        hint={
          <>
            The hole is the value the function <em>hands back</em> for red — a symbol literal, <C>#"green"</C>.
            The result is a symbol like any other, which the check then compares against <C>#"green"</C>.
          </>
        }
      />
    </article>
  );
}
