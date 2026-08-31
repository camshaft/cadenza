// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Errors() {
  return (
    <article>
      <H1>Errors & absence</H1>
      <Lede>What happens when there's no answer, and how Cadenza makes you deal with it.</Lede>
      <P>Not every operation has an answer, and a language has to do something when you look past the end of a list or look up a missing key or ask for a result that doesn't fit. Cadenza represents a value that might be missing with the <C>Option</C> type, which is either <Cadenza>(Some x)</Cadenza> with a value or <Cadenza>(None unit)</Cadenza> with nothing, so the type system makes a caller acknowledge both cases rather than return a bogus default. A genuinely undefined operation like dividing by zero is a different story that halts rather than inventing a value, as you saw in <strong>The numeric model</strong>, so this chapter is about the kind of absence you can handle.</P>
      <H2>Returning an Option</H2>
      <P>Here's safe division: it returns <C>None</C> when the divisor is zero, and <C>Some</C> of the quotient otherwise. The caller <C>match</C>es on the result and can't forget the empty case.</P>
      <Runnable
        source={`(def (safe-div a b) (if (= b 0) (None unit) (Some (/ a b))))

(def (main) (match (safe-div 10 2) ((Some q) q) ((None _) -1)))`}
      />
      <P>Change the <C>2</C> to <C>0</C> and Run: the <C>None</C> arm fires and you get <C>-1</C>, no crash.</P>
      <Why tenet="Partiality is data, not a trap">Reading past the end of a list and looking up a missing key are ordinary values your program handles in Cadenza rather than crashes it suffers, because absence has a type. Since that type is <C>Option</C>, the compiler can see whether you've dealt with it, whereas a special sentinel like <C>-1</C> would invite a caller to forget the edge case and a trap would take the decision out of their hands. An <C>Option</C> keeps the choice in the type where the caller must confront it.</Why>
      <H2>Safe indexing</H2>
      <P>The same pattern shows up in the standard library. <C>List.at</C> returns an <C>Option</C>, so you never read past the end by accident:</P>
      <Runnable
        source={`(def (main) (match (List.at #list(10 20 30) 1) ((Some x) x) ((None _) 0)))`}
      />
      <P>Index <C>1</C> holds <C>20</C>, so the <C>Some</C> arm binds it and you get <C>20</C>. Change the index to <C>9</C> and the <C>None</C> arm's <C>0</C> comes back instead: a miss you handle, not a crash.</P>
      <H2>When you're sure: <C>expect</C></H2>
      <P>Sometimes you know an <C>Option</C> holds a value and want to get on with it. <C>Option.expect</C> unwraps a <C>Some</C>, or halts with your message if it's a <C>None</C>. The message is required, so the one place you turn absence into a crash is spelled out, right where it happens. Index 1 is present, so this just hands back <C>20</C>:</P>
      <Runnable
        source={`(Option.expect (List.at #list(10 20 30) 1) "index out of range")`}
      />
      <P>But ask for index <C>9</C>, off the end, and there's no value to unwrap. <C>expect</C> makes good on its name and halts, with the message you supplied:</P>
      <Note>This one is <strong>meant to halt</strong>. It compiles fine (the trap is a run-time event, not a compile error), so Run it and read the status bar: the program stops deliberately, at the exact spot you asked it to, rather than limping on with a bogus value.</Note>
      <Runnable
        source={`(Option.expect (List.at #list(10 20 30) 9) "index out of range")`}
        expect="error"
      />
      <P>That's the trade <C>expect</C> makes explicit: you're promising the <C>Option</C> is a <C>Some</C>, and if you're wrong the program halts <em>here</em>, named, instead of a wrong answer leaking downstream. Contrast the <C>match</C> above, which forces you to write the <C>None</C> case. <C>expect</C> is the "I've already checked, let me proceed" shortcut, and the required message is the receipt.</P>
      <H2>Arithmetic that can't answer</H2>
      <P>Overflow is the same story. The ordinary <C>*</C> traps if it overflows; when you'd rather <em>handle</em> an overflow than crash, <C>Int64.checked-mul</C> (and <C>checked-add</C>) returns an <C>Option</C>, <C>None</C> exactly when the result wouldn't fit.</P>
      <Runnable
        source={`(def (main) (match (Int64.checked-mul 9223372036854775807 2) ((Some x) x) ((None _) -1)))`}
      />
      <P>That's <C>Int64</C>'s largest value times 2, which can't fit, so you get the <C>None</C> arm.</P>
      <H2>Chaining fallible steps: the <C>?</C> operator</H2>
      <P>Match on one <C>Option</C> and you write two arms. Chain several (add two checked results, each of which might overflow) and the nested matches pile up, burying the happy path. The <C>?</C> operator (written <C>(try …)</C>) collapses that: on a <C>Some</C> it unwraps the value and carries on; on a <C>None</C> it <em>short-circuits</em>, making the whole function's result that <C>None</C>. Here two checked-adds both succeed, so <C>x</C> and <C>y</C> unwrap and the function returns <Cadenza>(Some 84)</Cadenza>:</P>
      <Runnable
        source={`(def
  (main)
  (let
    ((x (try (Int64.checked-add 20 22))))
    (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y)))))`}
      />
      <P>No match arms, no nesting, just the happy path, top to bottom. And when a step <em>does</em> fail, the short-circuit takes over: make the first add overflow and its <C>None</C> becomes the function's answer immediately, so the second <C>try</C> and the body never run:</P>
      <Runnable
        source={`(def
  (main)
  (let
    ((x (try (Int64.checked-add Int64.max 1))))
    (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y)))))`}
      />
      <P>The overflowing add is <C>None</C>, so <C>main</C> is <Cadenza>(None unit)</Cadenza>. The enclosing function must itself return the matching kind (an <C>Option</C> here, or a <C>Result</C>): <C>?</C> needs a fallible boundary to short-circuit <em>to</em>, and it doesn't convert between <C>Option</C> and <C>Result</C>: the kinds have to line up.</P>
      <H2>Matching on the value inside</H2>
      <P>A pattern can look <em>inside</em> a variant, not just name it. Here the first arm only fires for exactly <Cadenza>(Some 0)</Cadenza>; a different <C>Some</C> falls through to the binding arm.</P>
      <Runnable
        source={`(def (describe o) (match o ((Some 0) 100) ((Some x) x) ((None _) -1)))

(def (main) (describe (Some 0)))`}
      />
      <P><Cadenza>(Some 0)</Cadenza> matches the literal-<C>0</C> arm, so this is <C>100</C>. Feed it <Cadenza>(Some 7)</Cadenza> instead and the second arm binds <C>x</C> and returns <C>7</C>; <Cadenza>(None unit)</Cadenza> takes the last.</P>
      <H2>Failing with a reason: <C>Result</C></H2>
      <P>An <C>Option</C> says <em>whether</em> there's an answer; sometimes you also want to say <em>why</em> there isn't. <C>Result</C> is the sum for that: <Cadenza>(Ok value)</Cadenza> when it worked, <Cadenza>(Err e)</Cadenza> carrying a reason when it didn't, and you take it apart with <C>match</C> exactly like an <C>Option</C>. Here <C>safe-div</C> reports the offending divisor on failure:</P>
      <Runnable
        source={`(def (safe-div a b) (if (= b 0) (Err b) (Ok (/ a b))))

(def (main) (match (safe-div 20 4) ((Ok q) q) ((Err e) -1)))`}
      />
      <P>This takes the <C>Ok</C> arm for <C>20 / 4 = 5</C>. Change the <C>4</C> to <C>0</C> and Run: the <C>Err</C> arm fires instead, and <C>e</C> is bound to the reason (here the divisor, <C>0</C>): the same exhaustive <C>match</C>, now with a payload that explains the failure.</P>
      <H2>Your turn</H2>
      <Exercise
        id="errors:1"
        prompt={<><C>check</C> returns <Cadenza>(Err 8)</Cadenza> for this input. Finish the <C>Err</C> arm to hand back the reason it carries, so the result is <C>8</C>.</>}
        starter={`(def (check n) (if (= n 0) (Ok n) (Err n)))

(def (main) (match (check 8) ((Ok v) v) ((Err e) ?)))`}
        solution={`(def (check n) (if (= n 0) (Ok n) (Err n)))

(def (main) (match (check 8) ((Ok v) v) ((Err e) e)))`}
        expected="8"
        hint={<>The <C>Err</C> arm binds its payload to <C>e</C>, so return that binding to surface the reason: just <C>e</C>.</>}
      />
      <Exercise
        id="errors:2"
        prompt={<><C>take-from</C> subtracts <C>n</C> from a stock of <C>stock</C>, but only when there's enough; asking for more than you have has no answer. Fill the hole so the "not enough" branch <em>returns absence</em>. Here the stock is <C>3</C> and the request is <C>10</C>, so that branch fires and the <C>None</C> arm hands back <C>-1</C>.</>}
        starter={`(def (take-from stock n) (if (< stock n) ? (Some (- stock n))))

(def (main) (match (take-from 3 10) ((Some left) left) ((None _) -1)))`}
        solution={`(def (take-from stock n) (if (< stock n) (None unit) (Some (- stock n))))

(def (main) (match (take-from 3 10) ((Some left) left) ((None _) -1)))`}
        expected="-1"
        hint={<>The <C>Some</C> branch already carries a value; the empty branch carries nothing, which you write as <Cadenza>(None unit)</Cadenza>, the same <C>None</C> the <C>match</C> below is waiting for.</>}
      />
      <P><C>Option</C>, <C>Result</C>, and <C>?</C> all answer the same question, what happens when a step might not deliver, by making the outcome a <em>value the caller must handle</em>. The next chapter turns that inside out: with <em>effects &amp; handlers</em>, a function <em>performs</em> an operation and lets whoever runs it decide what it means, so the caller answers instead of just inspecting a returned value. It's the pivot from the fundamentals into what makes Cadenza its own language.</P>
    </article>
  );
}
