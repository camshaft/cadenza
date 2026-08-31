// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Bytes() {
  return (
    <article>
      <H1>Bytes</H1>
      <Lede>A string is Unicode text, but when you need the raw octets underneath, whether for a file, a protocol, or a hash, that's a <C>Bytes</C> value.</Lede>
      <P>A <C>Bytes</C> is an immutable sequence of 8-bit values. You can write one as a byte-string literal <C>b"…"</C>, where the <C>b</C> prefix distinguishes it from a text string, or build one from a list of numbers with <C>Bytes.of</C>. Return one and you see it printed back as a <C>b"…"</C> literal with non-printable bytes shown as <C>\x</C> escapes, so here the numbers <C>10</C>, <C>20</C>, <C>30</C> become the three octets <C>b"\n\x14\x1e"</C>:</P>
      <Runnable
        source={`(Bytes.of #list(10 20 30))`}
      />
      <P><C>Bytes.len</C> counts those octets, so <C>b"hi!"</C> is the three bytes <C>h</C>, <C>i</C>, <C>!</C>, and its length is <C>3</C>:</P>
      <Runnable
        source={`(Bytes.len b"hi!")`}
      />
      <H2>Two ways to write the same bytes</H2>
      <P>A literal and a built sequence are just two spellings of one value, and Cadenza compares them by value, so they're <em>equal</em> when their bytes match. <C>b"AB"</C> is the two bytes 65 and 66:</P>
      <Runnable
        source={`(= b"AB" (Bytes.of #list(65 66)))`}
      />
      <H2>Reaching in safely</H2>
      <P>Like <C>List.at</C>, <C>Bytes.at</C> can miss, because an out-of-range index has no byte to return, so it hands back an <C>Option</C> you take apart with <C>match</C>. Here index 1 holds <C>20</C>:</P>
      <Runnable
        source={`(def (main) (match (Bytes.at (Bytes.of #list(10 20 30)) 1) ((Some b) b) ((None _) -1)))`}
      />
      <P>Push the index past the end and the <C>None</C> arm fires, with no out-of-bounds crash.</P>
      <H2>Joining and slicing</H2>
      <P><C>Bytes.concat</C> joins two byte sequences, while <C>Bytes.slice</C> takes a <em>start</em> index and a <em>length</em>, and because that window might run off the end it returns an <C>Option</C>. Both return new values, leaving the originals untouched:</P>
      <Runnable
        source={`(Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4 5)))`}
      />
      <P>Joining a 2-byte sequence with a 3-byte one gives the 5-byte <C>b"\x01\x02\x03\x04\x05"</C>, the two originals laid end to end, both untouched.</P>
      <Runnable
        source={`(def
  (main)
  (Bytes.len (Option.expect (Bytes.slice (Bytes.of #list(1 2 3 4 5)) 1 3) "out of range")))`}
      />
      <P><C>(Bytes.slice bs 1 3)</C> takes 3 bytes starting at index 1, so its length is <C>3</C>. Ask for more bytes than remain and you get the <C>None</C> arm instead.</P>
      <Note>A small difference worth noting: <C>Bytes.slice</C> takes a start and a <em>length</em>, while <C>String.slice</C> from the previous chapter takes a start and an <em>end</em> index. So <C>(Bytes.slice bs 1 3)</C> is three bytes, but <C>(String.slice s 1 3)</C> is two characters (indices 1 and 2). Read the argument names and you won't be caught out.</Note>
      <H2>From text to bytes</H2>
      <P><C>String.to-bytes</C>, the crossing the <em>Strings</em> chapter covered, hands you a string's UTF-8 encoding as a <C>Bytes</C>, which lets us see what <C>Bytes.len</C> measures, namely octets rather than characters. Return the bytes for the 4-character <C>"café"</C> and the encoding is right there in <C>b"caf\xc3\xa9"</C>:</P>
      <Runnable
        source={`((. String to-bytes) "café")`}
      />
      <P>You can see it directly: <C>c</C>, <C>a</C>, <C>f</C> are one byte each, and the <C>é</C> is the two bytes <C>\xc3\xa9</C>, which is five octets for four characters. So <C>Bytes.len</C> of this is <C>5</C>, counting the octets it's given whatever the character count of the text that produced them.</P>
      <Why tenet="Text and bytes are different types">Many languages blur strings and byte arrays into one thing, and the encoding bugs follow. Cadenza keeps them apart: a <C>String</C> is a sequence of Unicode characters; a <C>Bytes</C> is a sequence of octets. You cross between them with a named, explicit operation (<C>String.to-bytes</C>) that says which encoding you mean, so "how long is it?" and "what's at index 2?" always have one unambiguous answer, and a byte-level concern can never silently corrupt text.</Why>
      <Note>Everything here is the same value discipline as lists and strings: <C>Bytes</C> is immutable (operations return new values), compared by content, and indexed through an <C>Option</C> so a bad offset is a value you handle, not a crash.</Note>
      <P>Raw octets are a flat sequence, but real formats give them <em>structure</em>, such as a 2-byte length, a tag, and a payload. <em>Binary matching</em>, next, describes that layout as typed segments that build and destructure <C>Bytes</C> both ways.</P>
      <H2>Your turn</H2>
      <Exercise
        id="bytes:1"
        prompt={<>Read the byte at index <C>2</C> of the literal <C>b"ABC"</C>. Indexing is safe, so it comes back as an <C>Option</C>, matched here, and the byte for <C>C</C> is <C>67</C>.</>}
        starter={`(def (main) (match (Bytes.at b"ABC" ?) ((Some x) x) ((None _) -1)))`}
        solution={`(def (main) (match (Bytes.at b"ABC" 2) ((Some x) x) ((None _) -1)))`}
        expected="67"
        hint={<>Bytes are indexed from 0, so the third byte (<C>C</C>) is index <C>2</C>. Its ASCII code is <C>67</C>.</>}
      />
      <Exercise
        id="bytes:2"
        prompt={<><C>Bytes.concat</C> joins two byte sequences into a new one, and because <C>Bytes</C> compares by content, you can check the result against a literal. Fill the hole with the byte-string that completes <C>b"AB"</C> into <C>b"ABC"</C>, so the equality holds and the answer is <C>true</C>.</>}
        starter={`(def (main) (= (Bytes.concat b"AB" ?) b"ABC"))`}
        solution={`(def (main) (= (Bytes.concat b"AB" b"C") b"ABC"))`}
        expected="true"
        hint={<>Joining <C>b"AB"</C> with one more byte should give <C>b"ABC"</C>, so the missing piece is the single-byte literal <C>b"C"</C>. The join builds a fresh value, then <C>=</C> compares it to <C>b"ABC"</C> byte for byte.</>}
      />
    </article>
  );
}
