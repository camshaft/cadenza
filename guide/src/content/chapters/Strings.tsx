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
        accident. And it indexes by <em>character</em>, not byte — so in <C>"café"</C> the character at
        index <C>3</C> is the <C>é</C>, even though that <C>é</C> starts at byte 3 and spans two bytes:
      </P>
      <Runnable
        source={`(def (main)
  (match (String.at "café" 3)
    ((Some ch) ch)
    ((None _) "?")))`}
      />
      <Note>
        The compiler itself builds its diagnostics and export names out of strings this way — string
        handling isn't a library bolted on, it's part of how Cadenza describes itself.
      </Note>

      <H2>Slicing out a substring</H2>
      <P>
        To take a run of characters rather than a single one, <C>String.slice</C> selects a half-open
        range <C>[start, end)</C> — from <C>start</C> up to <em>but not including</em> <C>end</C>. Like{" "}
        <C>at</C>, the range might fall outside the string, so it returns an <C>Option</C>. The first five
        characters of <C>"hello world"</C> are <C>"hello"</C>, which is 5 characters long:
      </P>
      <Runnable
        source={`(def (main)
  (String.scalar-len
    (Option.expect (String.slice "hello world" 0 5) "in range")))`}
      />
      <P>
        The bounds count <em>characters</em>, the same as <C>at</C> — so slicing <C>"café"</C> from{" "}
        <C>0</C> to <C>3</C> gives the three characters <C>"caf"</C>, never splitting the two-byte{" "}
        <C>é</C> down the middle. A range where <C>start</C> equals <C>end</C> is a valid, empty slice
        (<C>Some ""</C>); one that runs off the end is <C>None</C>, not a trap.
      </P>

      <H2>Your turn</H2>
      <P>
        The word <C>"naïve"</C> has an accented <C>ï</C> that takes two bytes in UTF-8 — so its two
        lengths disagree. These two exercises ask for each in turn; the point is choosing the operation
        that answers the question you actually mean.
      </P>
      <Exercise
        id="strings:1"
        prompt={
          <>
            How many <em>characters</em> are in <C>"naïve"</C>? Pick the length that counts characters —
            the answer is <C>5</C>.
          </>
        }
        starter={`(def (main) (String.?-len "naïve"))`}
        solution={`(def (main) (String.scalar-len "naïve"))`}
        expected="5"
        hint={
          <>
            Characters (Unicode scalars), not bytes → <C>scalar-len</C>. The accented <C>ï</C> is still
            one character.
          </>
        }
      />

      <Exercise
        id="strings:2"
        prompt={
          <>
            Now how many <em>bytes</em> does <C>"naïve"</C> take in UTF-8? The two-byte <C>ï</C> pushes it
            one past the character count — the answer is <C>6</C>.
          </>
        }
        starter={`(def (main) (String.?-len "naïve"))`}
        solution={`(def (main) (String.byte-len "naïve"))`}
        expected="6"
        hint={
          <>
            Bytes, not characters → <C>byte-len</C>. Five characters, but <C>ï</C> costs two bytes, so{" "}
            <C>6</C>.
          </>
        }
      />
    </article>
  );
}
