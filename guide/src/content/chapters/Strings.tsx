import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Strings() {
  return (
    <article>
      <H1>Strings &amp; text</H1>
      <Lede>Text as a sequence of Unicode characters — with two honest ways to measure it.</Lede>

      <P>
        A string literal is written in double quotes. Strings are values like any other — bind them,
        pass them, return them.
      </P>
      <Runnable source={`"hello, world"`} />

      <H2>Joining strings</H2>
      <P>
        <C>String.concat</C> joins two strings into a new one (strings are immutable, so it returns a
        fresh value). Chain it, or wrap it in a function:
      </P>
      <Runnable
        source={`(def (greet name)
  (String.concat "Hello, " name))
(def (main) (greet "Cadenza"))`}
      />

      <H2>How long is a string?</H2>
      <P>
        Here's a question with two right answers. How long is <C>"café"</C>? Counted in{" "}
        <em>characters</em> it's 4; counted in <em>bytes</em> (its UTF-8 encoding) it's 5, because{" "}
        <C>é</C> takes two bytes. Cadenza gives you both, named so you can't confuse them:
      </P>
      <Runnable source={`(String.scalar-len "café")`} />
      <Runnable source={`(String.byte-len "café")`} />

      <Why tenet="A string is Unicode characters, not bytes">
        A Cadenza string is a sequence of Unicode <em>scalar values</em> (characters), not a bag of
        bytes — so what it contains doesn't depend on how it's encoded. But real programs sometimes
        need the byte size (for a buffer, a protocol). Rather than pick one meaning of "length" and
        make the other a footgun, the language offers <em>both</em>, under names that say which you're
        getting: <C>scalar-len</C> counts characters, <C>byte-len</C> counts UTF-8 bytes. No silent
        surprise when a non-ASCII character makes the two disagree.
      </Why>

      <H2>Reaching in safely</H2>
      <P>
        Like <C>List.at</C>, <C>String.at</C> returns an <C>Option</C> — the one-character string at a
        given position, or <C>None</C> if the index is past the end. You never read off the end by
        accident. It indexes by <em>character</em> (scalar), not byte.
      </P>
      <Runnable
        source={`(def (main)
  (match (String.at "abc" 1)
    ((Some ch) ch)
    ((None _) "?")))`}
      />
      <Note>
        The compiler itself builds its diagnostics and export names out of strings this way — string
        handling isn't a library bolted on, it's part of how Cadenza describes itself.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="strings:1"
        prompt={<>Finish <C>banner</C> so its result is <C>6</C> characters — join <C>"hi"</C> with itself, then measure it.</>}
        starter={`(def (main)
  (String.scalar-len (String.concat "hi" ?)))`}
        solution={`(def (main)
  (String.scalar-len (String.concat "hi" "hihi")))`}
        expected="6"
        hint={<>"hi" plus a 4-character string makes 6 characters. What 4-character string?</>}
      />

      <Exercise
        id="strings:2"
        prompt={<>Report the number of <em>characters</em> in <C>"héllo"</C> — it should be <C>5</C>, not the byte count.</>}
        starter={`(def (main) (String.?-len "héllo"))`}
        solution={`(def (main) (String.scalar-len "héllo"))`}
        expected="5"
        hint={<>Characters, not bytes → <C>scalar-len</C>.</>}
      />
    </article>
  );
}
