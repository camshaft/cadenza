import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function BinaryMatching() {
  return (
    <article>
      <H1>Binary matching</H1>
      <Lede>
        A <C>Bytes</C> chapter showed raw octets; this one gives them structure. The <C>(bin …)</C> form
        describes a byte layout as a sequence of typed segments — and it works both ways: in expression
        position it <em>builds</em> a <C>Bytes</C>, in a <C>match</C> pattern it <em>takes one apart</em>.
      </Lede>

      <H2>Building bytes from segments</H2>
      <P>
        A <C>(bin …)</C> expression lays out fixed-width fields. <C>(u16 258)</C> is a 16-bit unsigned
        integer — two bytes, most-significant first (big-endian, the network default) — and <C>(u8 …)</C>{" "}
        one byte. Return it and you see the exact octets it lays down:
      </P>
      <Runnable source={`(bin (u16 258) (u8 (UInt8.wrap 5)))`} />
      <P>
        <C>{`b"\\x01\\x02\\x05"`}</C> — three bytes: <C>258</C> is <C>0x0102</C>, written big-endian as{" "}
        <C>\x01</C> then <C>\x02</C>, followed by the <C>u8</C> byte <C>\x05</C>. Each fixed-width segment
        contributes exactly its width — a <C>u16</C> is always two bytes, a <C>u32</C> always four —
        whatever value it carries, so you can read the layout straight off the result.
      </P>

      <H2>Taking bytes apart</H2>
      <P>
        The same grammar, on the left of a <C>match</C> arm, <em>reads</em> a <C>Bytes</C> back. Matching a
        two-byte value against <C>(bin (u16 n))</C> binds <C>n</C> to the integer those bytes encode —
        construction and matching are exact inverses:
      </P>
      <Runnable
        source={`(match (bin (u16 258))
  ((bin (u16 n)) n)
  (_             0))`}
      />
      <P>
        The arm reads <C>258</C> back. Byte order is explicit and honored both ways: add the <C>le</C>{" "}
        modifier and the same integer is written — and read — least-significant byte first.
      </P>
      <Runnable
        source={`(match (bin (u16 258 le))
  ((bin (u16 n le)) n)
  (_                0))`}
      />

      <H2>A literal segment dispatches</H2>
      <P>
        A literal in a segment matches by equality — the binary analogue of a literal value pattern. That's
        how you dispatch on a tag byte, then read the fields behind it. Here a leading <C>1</C> guards the
        arm, and the following <C>u16</C> is read as <C>n</C>:
      </P>
      <Runnable
        source={`(match (Bytes.of (list 1 1 2))
  ((bin (u8 1) (u16 n)) n)
  (_                    0))`}
      />
      <P>
        Written as a hex literal, the same idea reads a magic-number header legibly — a <C>u32</C> equal to{" "}
        <C>0x89504E47</C> is the PNG signature, and a trailing <C>(bytes rest)</C> absorbs the payload after
        it:
      </P>
      <Runnable
        source={`(match (Bytes.of (list 137 80 78 71 1 2))
  ((bin (u32 0x89504E47) (bytes rest)) 1)
  (_                                   0))`}
      />

      <H2>A pattern accounts for the whole value</H2>
      <P>
        A <C>bin</C> pattern must describe the <em>entire</em> byte sequence — leftover bytes are a
        non-match. Three bytes against a pattern that names only two doesn't fire, so this falls to the
        catch-all and gives <C>0</C>:
      </P>
      <Runnable
        source={`(match (Bytes.of (list 1 2 3))
  ((bin (u16 n)) n)
  (_             0))`}
      />
      <P>
        The fix is a trailing unsized <C>(bytes rest)</C>, which absorbs the variable-length remainder — now
        the <C>u16</C> reads the first two bytes and <C>rest</C> takes the third, so the arm matches:
      </P>
      <Runnable
        source={`(match (Bytes.of (list 1 2 3))
  ((bin (u16 n) (bytes rest)) n)
  (_                          0))`}
      />
      <Note>
        Because a <C>bin</C> pattern never covers every possible byte sequence, a <C>match</C> over a{" "}
        <C>Bytes</C> needs a catch-all <C>_</C> arm — the same exhaustiveness rule as a sum match. Without
        one it's a compile error (CDZ0210).
      </Note>

      <H2>A segment's size can be a value</H2>
      <P>
        The payoff: a segment's <em>length</em> can be a name bound earlier in the same pattern — the
        length-prefixed frame that every wire format is built on. Read a count <C>n</C>, then bind exactly{" "}
        <C>n</C> bytes to <C>body</C>, and let a final <C>rest</C> take what's left:
      </P>
      <Runnable
        source={`(match (Bytes.of (list 2 10 20 99))
  ((bin (u8 n) (bytes body n) (bytes rest)) (Bytes.len body))
  (_                                        0))`}
      />
      <P>
        The first byte is <C>2</C>, so <C>body</C> is the next two bytes (length <C>2</C>) and <C>rest</C> is
        the trailing <C>99</C>. Building the same frame is the mirror image — write the length as a prefix,
        then splice the payload:
      </P>
      <Runnable
        source={`(Bytes.len
  (bin (u16 (UInt16.of (Bytes.len (Bytes.of (list 10 20 30)))))
       (bytes (Bytes.of (list 10 20 30)))))`}
      />
      <P>
        A two-byte length prefix plus a three-byte payload is five bytes. The length is computed and
        narrowed to the segment's width with <C>UInt16.of</C> — a checked narrow, so a payload too long to
        frame in 16 bits is a real error, not a silent wrap.
      </P>

      <Why tenet="A binary layout is width-typed, and checked at compile time">
        A fixed-width segment takes a value of <em>exactly</em> its width — <C>(u8 v)</C> wants a{" "}
        <C>UInt8</C>, <C>(bits v k)</C> a <C>(UInt k)</C>. Hand it something wider or negative and it's a
        compile-time type error, not a runtime "does not fit" trap — construction is total, and narrowing is
        the caller's explicit choice (<C>UInt8.wrap</C> truncates, <C>UInt8.of</C> narrows checked). And the
        byte alignment is static too: a layout whose bits don't close a byte is rejected before it runs. The
        result is that a binary format's shape is checked the same way the rest of your types are — the
        layout can't silently corrupt a value, because a value that wouldn't fit never compiles.
      </Why>

      <P>
        Bytes and binary layouts are about raw data. The last of the core value types is the opposite — a
        value that's purely a <em>name</em>, compared by identity: <em>symbols</em>, next.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="binary-matching:1"
        prompt={
          <>
            Construction and matching are inverses over the same segment grammar. A <C>u16</C> holding{" "}
            <C>513</C> is matched back — fill the segment kind so <C>n</C> reads <C>513</C>.
          </>
        }
        starter={`(match (bin (u16 513))
  ((bin (? n)) n)
  (_           0))`}
        solution={`(match (bin (u16 513))
  ((bin (u16 n)) n)
  (_             0))`}
        expected="513"
        hint={
          <>
            The pattern segment must match the constructed one: a <C>u16</C> was written, so read it with a{" "}
            <C>u16</C> segment. A mismatched width would read a different number of bytes and not round-trip.
          </>
        }
      />

      <Exercise
        id="binary-matching:2"
        prompt={
          <>
            A dependent-size frame: the first byte says how many bytes of body follow. Fill the size of the{" "}
            <C>body</C> segment so it binds exactly the count the leading <C>u8</C> named — the answer is the
            body's length, <C>3</C>.
          </>
        }
        starter={`(match (Bytes.of (list 3 10 20 30 99))
  ((bin (u8 n) (bytes body ?) (bytes rest)) (Bytes.len body))
  (_                                        0))`}
        solution={`(match (Bytes.of (list 3 10 20 30 99))
  ((bin (u8 n) (bytes body n) (bytes rest)) (Bytes.len body))
  (_                                        0))`}
        expected="3"
        hint={
          <>
            The size is the name bound by the earlier segment — <C>n</C>, read from the first byte (<C>3</C>).
            So <C>(bytes body n)</C> binds the next three bytes, and <C>rest</C> takes the trailing <C>99</C>.
          </>
        }
      />
    </article>
  );
}
