import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Errors() {
  return (
    <article>
      <H1>Errors &amp; absence</H1>
      <Lede>What happens when there's no answer — and how Cadenza makes you deal with it.</Lede>

      <P>
        Not every operation has an answer. Divide by zero, look past the end of a list, add two numbers
        that overflow — a language has to do <em>something</em>. Cadenza's answer is consistent: instead
        of crashing or returning a bogus value, these operations hand you an <C>Option</C> — either{" "}
        <C>(Some x)</C> with a value, or <C>(None unit)</C> with nothing — and the type system makes you
        acknowledge both cases.
      </P>

      <H2>Returning an Option</H2>
      <P>
        Here's safe division: it returns <C>None</C> when the divisor is zero, and <C>Some</C> of the
        quotient otherwise. The caller <C>match</C>es on the result and can't forget the empty case.
      </P>
      <Runnable
        source={`(def (safe-div a b)
  (if (= b 0) (None unit) (Some (/ a b))))
(def (main)
  (match (safe-div 10 2)
    ((Some q) q)
    ((None _) (- 0 1))))`}
      />
      <P>Change the <C>2</C> to <C>0</C> and Run: the <C>None</C> arm fires and you get <C>-1</C> — no crash.</P>

      <Why tenet="Partiality is data, not a trap">
        Reading past the end of a list, dividing by zero, a lookup that misses — in Cadenza these are
        ordinary values your program <em>handles</em>, not crashes it suffers. Absence has a type
        (<C>Option</C>), so the compiler can see whether you've dealt with it. The alternative —
        returning a special sentinel like <C>-1</C>, or trapping — either invites a caller to forget the
        edge case or takes the decision out of their hands. An <C>Option</C> puts it back in the type.
      </Why>

      <H2>Safe indexing</H2>
      <P>
        The same pattern shows up in the standard library. <C>List.at</C> returns an <C>Option</C> — you
        never read past the end by accident:
      </P>
      <Runnable
        source={`(def (main)
  (match (List.at (list 10 20 30) 1)
    ((Some x) x)
    ((None _) 0)))`}
      />

      <H2>When you're sure: <C>expect</C></H2>
      <P>
        Sometimes you know an <C>Option</C> holds a value and want to get on with it. <C>Option.expect</C>{" "}
        unwraps a <C>Some</C>, or halts with your message if it's a <C>None</C>. The message is required —
        so the one place you turn absence into a crash is spelled out, right where it happens. Index 1 is
        present, so this just hands back <C>20</C>:
      </P>
      <Runnable source={`(Option.expect (List.at (list 10 20 30) 1) "index out of range")`} />
      <P>
        But ask for index <C>9</C> — off the end — and there's no value to unwrap. <C>expect</C> makes
        good on its name and halts, with the message you supplied:
      </P>
      <Note>
        This one is <strong>meant to halt</strong>. It compiles fine — the trap is a run-time event, not
        a compile error — so Run it and read the status bar: the program stops deliberately, at the exact
        spot you asked it to, rather than limping on with a bogus value.
      </Note>
      <Runnable
        source={`(Option.expect (List.at (list 10 20 30) 9) "index out of range")`}
        expect="error"
      />
      <P>
        That's the trade <C>expect</C> makes explicit: you're promising the <C>Option</C> is a{" "}
        <C>Some</C>, and if you're wrong the program halts <em>here</em>, named, instead of a wrong answer
        leaking downstream. Contrast the <C>match</C> above, which forces you to write the <C>None</C>{" "}
        case — <C>expect</C> is the "I've already checked, let me proceed" shortcut, and the required
        message is the receipt.
      </P>

      <H2>Arithmetic that can't answer</H2>
      <P>
        Overflow is the same story. The ordinary <C>*</C> traps if it overflows; when you'd rather{" "}
        <em>handle</em> an overflow than crash, <C>Int64.checked-mul</C> (and <C>checked-add</C>) returns
        an <C>Option</C> — <C>None</C> exactly when the result wouldn't fit.
      </P>
      <Runnable
        source={`(def (main)
  (match (Int64.checked-mul 9223372036854775807 2)
    ((Some x) x)
    ((None _) (- 0 1))))`}
      />
      <P>That's <C>Int64</C>'s largest value times 2 — it can't fit, so you get the <C>None</C> arm.</P>

      <H2>Matching on the value inside</H2>
      <P>
        A pattern can look <em>inside</em> a variant, not just name it. Here the first arm only fires for
        exactly <C>(Some 0)</C>; a different <C>Some</C> falls through to the binding arm.
      </P>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))
(def (describe o)
  (match o
    ((Some 0) 100)
    ((Some x) x)
    ((None _) (- 0 1))))
(def (main) (describe (Some 0)))`}
      />

      <H2>Failing with a reason: <C>Result</C></H2>
      <P>
        An <C>Option</C> says <em>whether</em> there's an answer; sometimes you also want to say{" "}
        <em>why</em> there isn't. <C>Result</C> is the sum for that — <C>(Ok value)</C> when it worked,{" "}
        <C>(Err e)</C> carrying a reason when it didn't — and you take it apart with <C>match</C> exactly
        like an <C>Option</C>. Here <C>safe-div</C> reports the offending divisor on failure:
      </P>
      <Runnable
        source={`(def (safe-div a b)
  (if (= b 0) (Err b) (Ok (/ a b))))
(def (main)
  (match (safe-div 20 4)
    ((Ok q) q)
    ((Err e) (- 0 1))))`}
      />
      <P>
        This takes the <C>Ok</C> arm for <C>20 / 4 = 5</C>. Change the <C>4</C> to <C>0</C> and Run: the{" "}
        <C>Err</C> arm fires instead, and <C>e</C> is bound to the reason (here the divisor, <C>0</C>) —
        the same exhaustive <C>match</C>, now with a payload that explains the failure.
      </P>

      <H2>Your turn</H2>
      <Exercise
        id="errors:1"
        prompt={
          <>
            <C>check</C> returns <C>(Err 8)</C> for this input. Finish the <C>Err</C> arm to hand back the
            reason it carries, so the result is <C>8</C>.
          </>
        }
        starter={`(def (check n)
  (if (= n 0) (Ok n) (Err n)))
(def (main)
  (match (check 8)
    ((Ok v) v)
    ((Err e) ?)))`}
        solution={`(def (check n)
  (if (= n 0) (Ok n) (Err n)))
(def (main)
  (match (check 8)
    ((Ok v) v)
    ((Err e) e)))`}
        expected="8"
        hint={
          <>
            The <C>Err</C> arm binds its payload to <C>e</C> — return that binding to surface the reason:{" "}
            just <C>e</C>.
          </>
        }
      />

      <Exercise
        id="errors:2"
        prompt={<>Multiply 6 and 7 with a checked op and unwrap it, so the answer is <C>42</C>.</>}
        starter={`(def (main)
  (Option.expect (Int64.checked-mul 6 ?) "overflow"))`}
        solution={`(def (main)
  (Option.expect (Int64.checked-mul 6 7) "overflow"))`}
        expected="42"
        hint={<>Fill in the second factor: <C>7</C>.</>}
      />
    </article>
  );
}
