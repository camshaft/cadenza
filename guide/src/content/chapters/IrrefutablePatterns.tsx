// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function IrrefutablePatterns() {
  return (
    <article>
      <H1>Irrefutable patterns</H1>
      <Lede>Destructuring that always matches, so it can bind directly in a let or in a function's arguments, no match needed.</Lede>
      <P>In <strong>Pattern matching</strong> a <C>match</C> arm might or might not fire, so the compiler makes you cover every case. But some patterns can never fail: a tuple is always a tuple, and a type with a single constructor has only one shape. A pattern that always matches is <em>irrefutable</em>, and because it can't fail there's nothing to decide, so you can use it to bind a value directly, with no <C>match</C> at all.</P>
      <H2>Destructuring in a let</H2>
      <P>A <C>let</C> binder doesn't have to be a plain name. Give it a tuple pattern and it takes the pair apart in place, binding each part to its own name. Here <C>a</C> is <C>3</C> and <C>b</C> is <C>4</C>, so the sum is <C>7</C>, with no <C>.0</C> or <C>.1</C> indexing:</P>
      <Runnable
        source={`(def (main) (let ((#tuple(a b) #tuple(3 4))) (+ a b)))`}
      />
      <P>Patterns nest, so one binder can reach several layers deep in a single step. Here the inner tuple is destructured at the same time as the outer one, binding <C>a</C>, <C>b</C>, and <C>c</C> at once:</P>
      <Runnable
        source={`(def (main) (let ((#tuple(a #tuple(b c)) #tuple(1 #tuple(2 3)))) (+ a (+ b c))))`}
      />
      <P>A record binder works the same way, naming fields instead of positions. This one binds <C>a</C> to the <C>x</C> field and <C>b</C> to the <C>y</C> field, so the sum is <C>7</C>:</P>
      <Runnable
        source={`(def (main) (let ((#record((= x a) (= y b)) #record((= x 3) (= y 4)))) (+ a b)))`}
      />
      <P>And a record pattern needn't name every field: bind just the one you want and leave the rest. Since a record is keyed by name, the fields you skip simply don't appear, and the order you write them in doesn't matter. Here only <C>x</C> is bound, reading back <C>3</C>:</P>
      <Runnable
        source={`(def (main) (let ((#record((= x a)) #record((= x 3) (= y 4)))) a))`}
      />
      <H2>Destructuring in a function's arguments</H2>
      <P>The same move works right in a parameter position. <C>add-pair</C> takes one tuple argument and names both of its parts in the parameter list, so the body can use <C>a</C> and <C>b</C> directly:</P>
      <Runnable
        source={`(def (add-pair #tuple(a b)) (+ a b))

(def (main) (add-pair #tuple(3 4)))`}
      />
      <P>A single-constructor type is irrefutable in the same way, since there's no other shape it could be, so a pattern like <Cadenza ast="Y2R6YXN0AAECCgFDCgFjAwAAAAEBAgABAg==" kind="expr">(C c)</Cadenza> on a one-constructor type unwraps its payload directly in the parameter, with no <C>match</C>. Here <C>Celsius</C> wraps an <C>Int64</C>, and <C>to-f</C> names the wrapped value <C>c</C> right in its parameter list:</P>
      <Runnable
        source={`(type Celsius (C Int64))

(def (to-f (C c)) (+ (/ (* c 9) 5) 32))

(def (main) (to-f (C 100)))`}
      />
      <P><Cadenza ast="Y2R6YXN0AAEDCgR0by1mCgFDAAFkBQAAAAEAAgECAQIBAgADBA==" kind="expr">(to-f (C 100))</Cadenza> is <C>212</C>, unwrapping the <C>100</C> and converting it. One constructor means one shape, so the compiler knows the pattern can't miss.</P>
      <P>Records destructure in a parameter too. <C>mag</C> takes one point record and names its <C>x</C> and <C>y</C> fields directly in the parameter list, so the body reads <C>a</C> and <C>b</C> with no accessor. The squared magnitude of <C>(3, 4)</C> is <C>3² + 4² = 25</C>:</P>
      <Runnable
        source={`(def (mag #record((= x a) (= y b))) (+ (* a a) (* b b)))

(def (main) (mag #record((= x 3) (= y 4))))`}
      />
      <H2>Where the line is drawn</H2>
      <P>The moment a pattern <em>could</em> fail, it stops being usable as a binder. A sum with more than one variant is <em>refutable</em>: matching <C>Some</C> leaves <C>None</C> with nowhere to go. So the compiler refuses it in a binding position and tells you to use a <C>match</C> instead:</P>
      <Note>This one is <strong>meant to be refused</strong>. Run it and read the status bar: <C>CDZ0210</C>, "a multi-variant constructor pattern is refutable ... only in a <C>match</C> arm". That's the dividing line, the same exhaustiveness guarantee from <strong>Pattern matching</strong>, now deciding where a pattern is allowed to bind.</Note>
      <Runnable
        source={`(type Opt (Some Int64) (None unit))

(def (unwrap (Some x)) x)

(def (main) (unwrap (Some 5)))`}
        expect="error"
      />
      <Why tenet="A pattern that always matches can bind anywhere; one that might fail belongs in a match">Irrefutable destructuring isn't a separate feature, but the same pattern language from <C>match</C>, allowed in the places where the compiler can prove it can't fail: a <C>let</C> binder and a parameter. So there's one consistent way to take a value apart, and the compiler draws the safe line for you. A tuple or single-constructor pattern binds directly, and a refutable one is turned away with <C>CDZ0210</C> rather than silently ignoring a case it didn't handle.</Why>
      <P>Irrefutable destructuring shows up constantly in the recursive code ahead, binding a list's head and tail or unwrapping a state tuple as it threads through a loop. That's <em>Iteration without loops</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="irrefutable-patterns:1"
        prompt={<>Destructure the tuple right in the <C>let</C> binder, then finish the body so it sums the two parts. With <Cadenza ast="Y2R6YXN0AAEDFQABCgABFAQAAAABAAIBAwABAgM=" kind="expr">#tuple(10 20)</Cadenza> the answer is <C>30</C>.</>}
        starter={`(def (main) (let ((#tuple(a b) #tuple(10 20))) ?))`}
        solution={`(def (main) (let ((#tuple(a b) #tuple(10 20))) (+ a b)))`}
        expected="30"
        hint={<>Both names come from the tuple pattern, so the body is <Cadenza ast="Y2R6YXN0AAEDCgErCgFhCgFiBAAAAAEAAgEDAAECAw==" kind="expr">(+ a b)</Cadenza>.</>}
      />
      <Exercise
        id="irrefutable-patterns:2"
        prompt={<><C>fst</C> destructures its tuple argument in the parameter list. Finish the body so it returns the <em>first</em> part. With <Cadenza ast="Y2R6YXN0AAEECgNmc3QVAAEHAAEJBgAAAAEAAgADAQMBAgMBAgAEBQ==" kind="expr">(fst #tuple(7 9))</Cadenza> the answer is <C>7</C>.</>}
        starter={`(def (fst #tuple(a b)) ?)

(def (main) (fst #tuple(7 9)))`}
        solution={`(def (fst #tuple(a b)) a)

(def (main) (fst #tuple(7 9)))`}
        expected="7"
        hint={<>The parameter pattern binds both parts; return the first one, <C>a</C>.</>}
      />
    </article>
  );
}
