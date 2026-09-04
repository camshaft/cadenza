// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Strings() {
  return (
    <article>
      <H1>Strings & text</H1>
      <Lede>Text as a sequence of Unicode characters, with two honest ways to measure it.</Lede>
      <P>A string literal is written in double quotes. Strings are values like any other, so you can bind them, pass them, and return them.</P>
      <Runnable
        source={`"hello, world"`}
      />
      <H2>Joining strings</H2>
      <P><C>String.concat</C> joins two strings into a new one (strings are immutable, so it returns a fresh value). Chain it, or wrap it in a function:</P>
      <Runnable
        source={`(def (greet name) (String.concat "Hello, " name))

(def (main) (greet "Cadenza"))`}
      />
      <P><C>(greet "Cadenza")</C> joins the two pieces into <C>"Hello, Cadenza"</C>, a brand-new string, with the two inputs left untouched.</P>
      <H2>How long is a string?</H2>
      <P>Here's a question with two right answers. How long is <C>"café"</C>? Counted in <em>characters</em> it's 4; counted in <em>bytes</em> (its UTF-8 encoding) it's 5, because <C>é</C> takes two bytes. Cadenza gives you both, named so you can't confuse them:</P>
      <Runnable
        source={`(String.scalar-len "café")`}
      />
      <Runnable
        source={`(String.byte-len "café")`}
      />
      <Why tenet="A string is Unicode characters, not bytes">A Cadenza string is a sequence of Unicode <em>scalar values</em> (characters), not a bag of bytes, so what it contains doesn't depend on how it's encoded. But real programs sometimes need the byte size (for a buffer, a protocol). Rather than pick one meaning of "length" and make the other a footgun, the language offers <em>both</em>, under names that say which you're getting: <C>scalar-len</C> counts characters, <C>byte-len</C> counts UTF-8 bytes. No silent surprise when a non-ASCII character makes the two disagree.</Why>
      <H2>Reaching in safely</H2>
      <P>Like <C>List.at</C>, <C>String.at</C> returns an <C>Option</C>, either the one-character string at a given position or <C>None</C> if the index is past the end. You never read off the end by accident. And it indexes by <em>character</em>, not byte, so in <C>"café"</C> the character at index <C>3</C> is the <C>é</C>, even though that <C>é</C> starts at byte 3 and spans two bytes:</P>
      <Runnable
        source={`(def (main) (match (String.at "café" 3) ((Some ch) ch) ((None _) "?")))`}
      />
      <Note>The compiler itself builds its diagnostics and export names out of strings this way, so string handling isn't a separate library but part of how Cadenza describes itself.</Note>
      <H2>Compared by value</H2>
      <P>Two strings are equal when they hold the same characters, which is structural equality, not identity. So a string you <em>built</em> equals a literal with the same content: <C>(String.concat "ab" "c")</C> equals <C>"abc"</C>, however each was made.</P>
      <Runnable
        source={`(= (String.concat "ab" "c") "abc")`}
      />
      <H2>Crossing to bytes and back</H2>
      <P>Text and raw bytes are different types (the next chapter, <strong>Bytes</strong>, is all about the raw side), and the crossing is explicit. <C>String.to-bytes</C> gives a string's UTF-8 encoding, so <C>"café"</C> is five bytes, the two-byte <C>é</C> included. Going back is <C>String.from-bytes</C>, which returns an <C>Option</C>, because not every byte sequence is valid UTF-8, and a round-trip of well-formed text succeeds:</P>
      <Runnable
        source={`(def
  (main)
  (match
    (String.from-bytes (String.to-bytes "café"))
    ((Some s) (String.scalar-len s))
    ((None _) -1)))`}
      />
      <P>The bytes decode back to <C>"café"</C>, four characters, the same value we started with. The <C>Option</C> is the honest part: decoding <em>arbitrary</em> bytes can fail, so <C>from-bytes</C> hands you an <C>Option</C> to handle rather than assuming the bytes are text.</P>
      <H2>Slicing out a substring</H2>
      <P>To take a run of characters rather than a single one, <C>String.slice</C> selects a half-open range <C>[start, end)</C>, from <C>start</C> up to <em>but not including</em> <C>end</C>. Like <C>at</C>, the range might fall outside the string, so it returns an <C>Option</C>. The first five characters of <C>"hello world"</C> are <C>"hello"</C>, which is 5 characters long:</P>
      <Runnable
        source={`(def (main) (String.scalar-len (Option.expect (String.slice "hello world" 0 5) "in range")))`}
      />
      <P>The bounds count <em>characters</em>, the same as <C>at</C>, so slicing <C>"café"</C> from <C>0</C> to <C>3</C> gives the three characters <C>"caf"</C>, never splitting the two-byte <C>é</C> down the middle. A range where <C>start</C> equals <C>end</C> is a valid, empty slice (<C>Some ""</C>); one that runs off the end is <C>None</C>, not a trap.</P>
      <H2>Characters</H2>
      <P>A string is a sequence of <em>characters</em> (Unicode scalar values), and <C>Char</C> is the type of a single one. A character literal is written <C>#\a</C>: a <C>#\</C> followed by the scalar. Its Unicode scalar value (its code point) is read with <C>Char.to-int</C>, so <C>#\a</C> is <C>97</C>:</P>
      <Runnable
        source={`(Char.to-int #\\a)`}
        id="string-char-code"
      />
      <P><C>String.scalar-at</C> reads the character at a scalar position, the single-character companion of <C>slice</C>. Like <C>at</C> and <C>slice</C> it's fallible, returning an <C>Option Char</C>, so an out-of-range position is <C>None</C> rather than a trap. The character at position <C>1</C> of <C>"hello"</C> is <C>#\e</C>, whose scalar value is <C>101</C>:</P>
      <Runnable
        source={`(def (main) (Char.to-int (Option.expect (String.scalar-at "hello" 1) "in range")))`}
        id="string-scalar-at"
      />
      <P><C>Char.to-int</C> reads a character's Unicode scalar value (its code point) as an <C>Int64</C>. It's <em>total</em>: every character has a code point, so it never fails. Going the other way, <C>Char.from-int</C> is <em>fallible</em>, since not every integer is a valid scalar, so it returns an <C>Option Char</C>. Code point <C>97</C> is <C>#\a</C>:</P>
      <Runnable
        source={`(def (main) (Char.to-int (Option.expect (Char.from-int 97) "valid scalar")))`}
      />
      <P>Because <C>from-int</C> is fallible, the invalid cases are data, not crashes. Code point <C>55296</C> is <C>U+D800</C>, a surrogate that is never a standalone scalar, so <C>from-int</C> gives <C>None</C>, and this match takes the <C>None</C> arm to return <C>0</C>:</P>
      <Runnable
        source={`(def (main) (match (Char.from-int 55296) ((Some c) (Char.to-int c)) ((None) 0)))`}
        id="string-surrogate"
      />
      <Why tenet="A character converts to and from an integer, honestly">Every character has an integer code point, so <C>Char.to-int</C> is total. But the reverse isn't, because the surrogate range and everything past <C>U+10FFFF</C> aren't scalar values, so <C>Char.from-int</C> returns an <C>Option</C> instead of inventing an ill-formed character. The type tells you which direction can fail.</Why>
      <P>Characters compare by their code point. <C>=</C> tests two characters for equality, and <C>&lt;</C>, <C>&gt;</C>, <C>&lt;=</C>, and <C>&gt;=</C> order them by scalar value, so <C>#\a</C> (code point 97) sorts before <C>#\z</C> (122) and this comparison is <C>true</C>:</P>
      <Runnable
        source={`(< #\\a #\\z)`}
        id="string-char-cmp"
      />
      <P>Characters are one view of text. Underneath sits the raw encoding, the octets a file or a protocol actually carries. That's <em>bytes</em>, next.</P>
      <H2>Your turn</H2>
      <P>First a length question, then a slice. The word <C>"naïve"</C> has an accented <C>ï</C> that takes two bytes in UTF-8, so its character count and its byte count disagree, and the point is to pick the operation that answers the question you actually mean.</P>
      <Exercise
        id="strings:1"
        prompt={<>How many <em>characters</em> are in <C>"naïve"</C>? Pick the length that counts characters, so the answer is <C>5</C>.</>}
        starter={`(def (main) (String.?-len "naïve"))`}
        solution={`(def (main) (String.scalar-len "naïve"))`}
        expected="5"
        hint={<>Characters (Unicode scalars), not bytes → <C>scalar-len</C>. The accented <C>ï</C> is still one character.</>}
      />
      <Exercise
        id="strings:2"
        prompt={<>Now a slice, and the half-open range is the whole trick. Pull the first three characters, <C>"cad"</C>, out of <C>"cadenza"</C> by filling in the <em>end</em> index. The check compares the slice against <C>"cad"</C>, so getting the boundary right gives <C>true</C>.</>}
        starter={`(def (main) (= (Option.expect (String.slice "cadenza" 0 ?) "in range") "cad"))`}
        solution={`(def (main) (= (Option.expect (String.slice "cadenza" 0 3) "in range") "cad"))`}
        expected="true"
        hint={<>The range is <C>[start, end)</C>, so <C>end</C> is <em>excluded</em>. To keep characters at indices <C>0</C>, <C>1</C>, <C>2</C> (the <C>"cad"</C>) and stop before index <C>3</C>, the end is <C>3</C>, not <C>2</C>. Write <C>2</C> and you'd get only <C>"ca"</C>.</>}
      />
      <Exercise
        id="strings:3"
        prompt={<>Now a character. Read the first character of <C>"Zebra"</C> and give its Unicode code point. The letter <C>Z</C> is code point <C>90</C>, so pick the operation that turns a character into its integer and the answer is <C>90</C>.</>}
        starter={`(def (main) (Char.?-int (Option.expect (String.scalar-at "Zebra" 0) "in range")))`}
        solution={`(def (main) (Char.to-int (Option.expect (String.scalar-at "Zebra" 0) "in range")))`}
        expected="90"
        hint={<><C>String.scalar-at</C> hands you the character (an <C>Option Char</C>, unwrapped here by <C>Option.expect</C>), and <C>Char.to-int</C> reads its code point. The total direction is <C>to-int</C>; <C>from-int</C> is the fallible reverse.</>}
      />
    </article>
  );
}
