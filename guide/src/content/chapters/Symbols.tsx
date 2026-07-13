import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Symbols() {
  return (
    <article>
      <H1>Symbols</H1>
      <Lede>
        Sometimes a value is just a <em>name</em> — a status, a tag, a choice from a fixed set. A symbol
        is exactly that: an interned name you compare by identity.
      </Lede>

      <P>
        A symbol is written <C>#"…"</C> — the <C>#</C> distinguishing it from a text string. Where a{" "}
        string is <em>content</em> you might slice, join, or measure, a symbol is a bare label: the only
        thing you do with it is ask whether two symbols are the <em>same</em>. Equality is the whole point:
      </P>
      <Runnable
        source={`(def (main)
  (if (= #"red" #"red") 1 0))`}
      />
      <P>Two symbols are equal exactly when they're spelled the same — and two different names aren't:</P>
      <Runnable
        source={`(def (main)
  (if (= #"cat" #"dog") 1 0))`}
      />

      <H2>Symbols as tags</H2>
      <P>
        The natural use is a status or mode passed around and checked. Here a function takes a symbol and
        reports whether it's the active one:
      </P>
      <Runnable
        source={`(def (is-active s)
  (if (= s #"active") 1 0))
(def (main) (is-active #"active"))`}
      />
      <P>Pass <C>#"idle"</C> instead and you get <C>0</C> — the same tag, checked by identity.</P>

      <Why tenet="A name compared by identity, not by its text">
        Why a distinct type, rather than just using a string like <C>"red"</C>? Because the intent is
        different. A string is text you might transform; a symbol is a fixed label whose only meaning is
        which label it is. Splitting them says so in the type — you'd never accidentally slice a status
        tag or measure its length — and lets the compiler treat a symbol as an interned identity
        (a fast equality check), not a sequence of characters to scan. Same spirit as keeping{" "}
        <C>Bytes</C> apart from <C>String</C>: one type per kind of thing.
      </Why>

      <H2>From a string, explicitly</H2>
      <P>
        When a name arrives as text — say, parsed from input — <C>Symbol.of</C> turns it into a symbol. A
        symbol built from <C>"on"</C> is the very same value as the literal <C>#"on"</C>:
      </P>
      <Runnable
        source={`(def (main)
  (if (= (Symbol.of "on") #"on") 1 0))`}
      />

      <Note>
        Text-to-symbol is an explicit step (<C>Symbol.of</C>), just like <C>String.to-bytes</C> — the one
        place you cross between the two types is spelled out, so a symbol and a string never silently
        stand in for each other.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="symbols:1"
        prompt={<>Make <C>is-active</C> answer for the <C>#"active"</C> tag, so the result is <C>1</C>.</>}
        starter={`(def (is-active s)
  (if (= s ?) 1 0))
(def (main) (is-active #"active"))`}
        solution={`(def (is-active s)
  (if (= s #"active") 1 0))
(def (main) (is-active #"active"))`}
        expected="1"
        hint={<>Compare against the symbol literal <C>#"active"</C>.</>}
      />

      <Exercise
        id="symbols:2"
        prompt={<>Build a symbol from the text <C>"go"</C> and check it equals <C>#"go"</C> — the answer is <C>1</C>.</>}
        starter={`(def (main)
  (if (= (Symbol.of ?) #"go") 1 0))`}
        solution={`(def (main)
  (if (= (Symbol.of "go") #"go") 1 0))`}
        expected="1"
        hint={<>Pass the string <C>"go"</C> to <C>Symbol.of</C>.</>}
      />
    </article>
  );
}
