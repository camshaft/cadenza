// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Symbols() {
  return (
    <article>
      <H1>Symbols</H1>
      <Lede>Sometimes a value is just a <em>name</em>, like a status, a mode, or one choice from a fixed set. A symbol is exactly that: an interned name you compare by identity.</Lede>
      <P>A symbol is written <C>#"…"</C>, where the <C>#</C> tells it apart from a text string. Where a string is <em>content</em> you might slice, join, or measure, a symbol is a bare label whose only question is "is it this one?". So the operation on symbols is equality:</P>
      <Runnable
        source={`(= #"red" #"red")`}
      />
      <P>Two symbols are equal exactly when they're spelled the same, and no matter how long the name, the comparison is a single identity check, not a character-by-character scan.</P>
      <Note>In the conventional surface the quotes are just noise when the name is a plain identifier, so <C>#"red"</C> may be written <C>#red</C>, and the two are the same symbol. The quotes are only needed when the content isn't an identifier: a name with a space, a leading digit, or a dot ( <C>#"List.at"</C>) keeps them. Toggle the syntax and a snippet's <C>#red</C> reappears as <C>#"red"</C> in the s-expression surface, where the quoted form is canonical.</Note>
      <H2>One choice from a fixed set</H2>
      <P>That's what symbols are for: a value drawn from a small, known set of names. A traffic light is <Cadenza>#"red"</Cadenza>, <Cadenza>#"yellow"</Cadenza>, or <Cadenza>#"green"</Cadenza>, and a function can decide on it, here choosing how many seconds to wait:</P>
      <Runnable
        source={`(def (wait light) (if (= light #"red") 30 (if (= light #"yellow") 5 0)))

(def (main) (wait #"red"))`}
      />
      <P><Cadenza>#"red"</Cadenza> waits 30, <Cadenza>#"yellow"</Cadenza> 5, and anything else (green) 0. The light is passed around as a plain value and matched by name where the decision is made, with no numbers to remember and no strings to keep in sync.</P>
      <Why tenet="A name compared by identity, not by its text">Why a distinct type, rather than just a string like <C>"red"</C>? Because the intent differs. A string is text you might transform; a symbol is a fixed label whose only meaning is <em>which label it is</em>. Making it its own type says so, so you can't accidentally slice a status tag or take its length, and lets the compiler treat it as an interned identity (one cheap comparison, whatever the name's length) instead of a sequence to scan. Same instinct as keeping <C>Bytes</C> apart from <C>String</C>: one type per kind of thing, so the compiler catches a category mistake.</Why>
      <H2>From a string, explicitly</H2>
      <P>When a name arrives as text, whether parsed from input or assembled at run time, <C>Symbol.of</C> interns it into a symbol. The result is the very same value as writing the literal: a symbol built from the pieces <C>"ye"</C> and <C>"s"</C> equals <Cadenza>#"yes"</Cadenza>:</P>
      <Runnable
        source={`(= (Symbol.of (String.concat "ye" "s")) #"yes")`}
      />
      <Note>Text-to-symbol is an explicit step (<C>Symbol.of</C>), just like <C>String.to-bytes</C>, so the one place you cross between the two types is spelled out, and a symbol and a string never silently stand in for each other.</Note>
      <P>That's the last of the everyday value shapes: numbers, text, bytes, symbols, and the collections that hold them. Now we look harder at <em>one</em> of them: how Cadenza models numbers, and why it refuses to convert them behind your back. <em>The numeric model</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="symbols:1"
        prompt={<><C>score</C> dispatches on a medal from the fixed set <Cadenza>#"gold"</Cadenza> / <Cadenza>#"silver"</Cadenza> / <Cadenza>#"bronze"</Cadenza>: gold scores <C>3</C>, silver <C>2</C>, anything else <C>1</C>. The gold and fallback arms are done, so fill the middle comparison to make <Cadenza>(score #"silver")</Cadenza> give <C>2</C>.</>}
        starter={`(def (score m) (if (= m #"gold") 3 (if (= m ?) 2 1)))

(def (main) (score #"silver"))`}
        solution={`(def (score m) (if (= m #"gold") 3 (if (= m #"silver") 2 1)))

(def (main) (score #"silver"))`}
        expected="2"
        hint={<>The middle arm handles silver, so compare <C>m</C> against <Cadenza>#"silver"</Cadenza>. Each symbol is checked by equality; <Cadenza>#"bronze"</Cadenza> matches neither and falls through to <C>1</C>.</>}
      />
      <Exercise
        id="symbols:2"
        prompt={<>A symbol is an ordinary value, so a function can <em>return</em> one, not just test it. <C>next</C> advances a traffic light around its cycle: red turns to green, green to yellow, yellow back to red. The green and yellow cases are written, so fill the hole with the symbol red becomes, making <Cadenza>(next #"red")</Cadenza> return <Cadenza>#"green"</Cadenza> and the check give <C>true</C>.</>}
        starter={`(def (next light) (if (= light #"red") ? (if (= light #"green") #"yellow" #"red")))

(def (main) (= (next #"red") #"green"))`}
        solution={`(def (next light) (if (= light #"red") #"green" (if (= light #"green") #"yellow" #"red")))

(def (main) (= (next #"red") #"green"))`}
        expected="true"
        hint={<>The hole is the value the function <em>hands back</em> for red, a symbol literal, <Cadenza>#"green"</Cadenza>. The result is a symbol like any other, which the check then compares against <Cadenza>#"green"</Cadenza>.</>}
      />
    </article>
  );
}
