import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Bytes() {
  return (
    <article>
      <H1>Bytes</H1>
      <Lede>
        A string is Unicode text. When you need the raw octets underneath — for a file, a protocol, a
        hash — that's a <C>Bytes</C> value.
      </Lede>

      <P>
        A <C>Bytes</C> is an immutable sequence of 8-bit values. You can write one as a byte-string
        literal — <C>b"…"</C>, the <C>b</C> prefix distinguishing it from a text string — or build one
        from a list of numbers with <C>Bytes.of</C>. Either way, <C>Bytes.len</C> counts the octets:
      </P>
      <Runnable source={`(Bytes.len b"hi!")`} />
      <Runnable source={`(Bytes.len (Bytes.of (list 10 20 30)))`} />
      <P>
        Both are <C>3</C>: <C>b"hi!"</C> is the three octets <C>h</C>, <C>i</C>, <C>!</C>, and the built
        sequence has three numbers in its list.
      </P>

      <H2>Two ways to write the same bytes</H2>
      <P>
        A literal and a built sequence are just two spellings of one value — and Cadenza compares them by
        value, so they're <em>equal</em> when their bytes match. <C>b"AB"</C> is the two bytes 65 and 66:
      </P>
      <Runnable
        source={`(def (main)
  (if (= b"AB" (Bytes.of (list 65 66))) 1 0))`}
      />

      <H2>Reaching in safely</H2>
      <P>
        Like <C>List.at</C>, <C>Bytes.at</C> can miss — an out-of-range index has no byte to return — so
        it hands back an <C>Option</C> you take apart with <C>match</C>. Here index 1 holds <C>20</C>:
      </P>
      <Runnable
        source={`(def (main)
  (match (Bytes.at (Bytes.of (list 10 20 30)) 1)
    ((Some b) b)
    ((None _) (- 0 1))))`}
      />
      <P>Push the index past the end and the <C>None</C> arm fires — no out-of-bounds crash.</P>

      <H2>Joining and slicing</H2>
      <P>
        <C>Bytes.concat</C> joins two byte sequences; <C>Bytes.slice</C> takes a <em>start</em> index and
        a <em>length</em> — and because that window might run off the end, it returns an <C>Option</C>.
        Both return new values; the originals are untouched:
      </P>
      <Runnable source={`(Bytes.len (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4 5))))`} />
      <P>Joining a 2-byte sequence with a 3-byte one gives <C>5</C> bytes.</P>
      <Runnable
        source={`(def (main)
  (Bytes.len
    (Option.expect (Bytes.slice (Bytes.of (list 1 2 3 4 5)) 1 3) "out of range")))`}
      />
      <P>
        <C>(Bytes.slice bs 1 3)</C> takes 3 bytes starting at index 1 — so its length is <C>3</C>. Ask
        for more bytes than remain and you get the <C>None</C> arm instead.
      </P>
      <Note>
        A small difference worth noting: <C>Bytes.slice</C> takes a start and a <em>length</em>, while{" "}
        <C>String.slice</C> from the previous chapter takes a start and an <em>end</em> index. So{" "}
        <C>(Bytes.slice bs 1 3)</C> is three bytes, but <C>(String.slice s 1 3)</C> is two characters
        (indices 1 and 2). Read the argument names and you won't be caught out.
      </Note>

      <H2>From text to bytes</H2>
      <P>
        The <em>Strings</em> chapter noted that <C>"café"</C> is 4 characters but 5 bytes in UTF-8.{" "}
        <C>String.to-bytes</C> gives you exactly those encoded bytes, and their length is the byte count:
      </P>
      <Runnable source={`(Bytes.len (String.to-bytes "café"))`} />
      <P>
        <C>5</C>, not <C>4</C>: the <C>é</C> encodes to two bytes, so the byte length runs one past the
        character count. (Use <C>String.scalar-len</C> if you want the <C>4</C> characters instead.)
      </P>

      <Why tenet="Text and bytes are different types">
        Many languages blur strings and byte arrays into one thing, and the encoding bugs follow. Cadenza
        keeps them apart: a <C>String</C> is a sequence of Unicode characters; a <C>Bytes</C> is a
        sequence of octets. You cross between them with a named, explicit operation
        (<C>String.to-bytes</C>) that says which encoding you mean — so "how long is it?" and "what's at
        index 2?" always have one unambiguous answer, and a byte-level concern can never silently corrupt
        text.
      </Why>

      <Note>
        Everything here is the same value discipline as lists and strings: <C>Bytes</C> is immutable
        (operations return new values), compared by content, and indexed through an <C>Option</C> so a bad
        offset is a value you handle, not a crash.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="bytes:1"
        prompt={
          <>
            Read the byte at index <C>2</C> of the literal <C>b"ABC"</C>. Indexing is safe, so it comes
            back as an <C>Option</C> — matched here — and the byte for <C>C</C> is <C>67</C>.
          </>
        }
        starter={`(def (main)
  (match (Bytes.at b"ABC" ?)
    ((Some x) x)
    ((None _) (- 0 1))))`}
        solution={`(def (main)
  (match (Bytes.at b"ABC" 2)
    ((Some x) x)
    ((None _) (- 0 1))))`}
        expected="67"
        hint={
          <>
            Bytes are indexed from 0, so the third byte (<C>C</C>) is index <C>2</C>. Its ASCII code is{" "}
            <C>67</C>.
          </>
        }
      />

      <Exercise
        id="bytes:2"
        prompt={
          <>
            Slice the three bytes <C>B C D</C> out of <C>b"ABCDE"</C>, starting at index <C>1</C>.
            Remember <C>Bytes.slice</C>'s second argument is a <em>length</em>, not an end index — fill it
            in so <C>Bytes.len</C> of the result is <C>3</C>.
          </>
        }
        starter={`(def (main)
  (Bytes.len
    (Option.expect (Bytes.slice b"ABCDE" 1 ?) "in range")))`}
        solution={`(def (main)
  (Bytes.len
    (Option.expect (Bytes.slice b"ABCDE" 1 3) "in range")))`}
        expected="3"
        hint={
          <>
            You want three bytes, and the argument <em>is</em> the count — so it's <C>3</C>, not an end
            index. (Write <C>4</C>, as if it were <C>String.slice</C>'s end, and you'd get four bytes —{" "}
            <C>B C D E</C> — instead.)
          </>
        }
      />
    </article>
  );
}
