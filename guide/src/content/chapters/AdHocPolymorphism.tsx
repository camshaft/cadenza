import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function AdHocPolymorphism() {
  return (
    <article>
      <H1>Ad-hoc polymorphism</H1>
      <Lede>
        One name, more than one meaning, chosen per type. Most languages bolt on a trait or typeclass
        system for this. Cadenza doesn't need one — because a "trait" is just a <em>record of functions you
        pass in</em>, and dispatch is choosing which record to pass. No new machinery: records and
        functions you already have.
      </Lede>

      <P>
        "Polymorphism" means one piece of code working over many types. The <em>ad-hoc</em> kind is where
        the behaviour genuinely differs per type — <C>show</C> renders an integer one way and a boolean
        another. Other languages introduce typeclasses (Haskell), traits (Rust), or interfaces to carry
        those per-type implementations. The Cadenza insight: that bundle of per-type operations is just a{" "}
        <strong>record whose fields are functions</strong>, and a polymorphic function is one that takes
        such a record as an argument and calls its fields.
      </P>

      <H2>A trait is a record of functions</H2>
      <P>
        Say you want a <C>show</C>-like operation — turn a value into a number, differently per type. The
        "trait" is a record with a <C>describe</C> field: <C>{`{ describe: T -> Int64 }`}</C>. An{" "}
        <em>instance</em> for a type is a record with that field filled in. A polymorphic function takes the
        record and calls <C>(. dict describe)</C>. Dispatch is picking which record to hand it:
      </P>
      <Runnable
        source={`(def (describe-int n) (+ n 100))
(def (describe-bool b) (if b 1 0))
(def (show-with dict x) ((. dict describe) x))
(def (int-show) (record (describe describe-int)))
(def (bool-show) (record (describe describe-bool)))
(def (main)
  (+ (show-with (int-show) 5)
     (show-with (bool-show) true)))`}
      />
      <P>
        <C>show-with</C> is the polymorphic function — it doesn't know or care <em>which</em> describe it's
        calling. <C>int-show</C> and <C>bool-show</C> are the two instances (the "trait implementations"),
        each a plain record. <C>main</C> dispatches by passing the right one: <C>5</C> becomes{" "}
        <C>105</C> and <C>true</C> becomes <C>1</C>, summing to <C>106</C>. That's the whole mechanism — no
        trait keyword, no instance resolution, just a record argument. And it's deliberate: the spec says a
        trait instance is an <em>explicitly-passed dictionary</em>, never resolved from ambient scope, so
        you can always see exactly which implementation a call uses by reading the argument.
      </P>

      <H2>Make the dictionary free: a const parameter</H2>
      <P>
        Passing a record around sounds like it should cost something at run time. It doesn't have to — mark
        the dictionary parameter <C>const</C> (see <em>Const parameters</em>) and the compiler inlines it
        into a specialized copy and erases it. The <C>(. dict describe)</C> lookup folds to the concrete
        function; no record is passed, no field is looked up at run time:
      </P>
      <Runnable
        source={`(def (describe-plus n) (+ n 100))
(def (describe-double n) (* n 2))
(def (show-with (const (: dict (Record (describe (-> Int64 Int64))))) (: x Int64))
  ((. dict describe) x))
(def (main)
  (+ (show-with (record (describe describe-plus)) 5)
     (show-with (record (describe describe-double)) 5)))`}
      />
      <P>
        Two different instances through the same specialized <C>show-with</C> — <C>describe-plus</C> gives{" "}
        <C>105</C>, <C>describe-double</C> gives <C>10</C>, summing to <C>115</C> — so dispatch genuinely
        varies even with the dictionary erased. The annotation <C>{`(Record (describe (-> Int64 Int64)))`}</C> is
        a real constraint: it names the field the dictionary must carry and its type, so a record missing{" "}
        <C>describe</C> (or with the wrong signature) is a compile error, not a runtime surprise. Zero runtime
        cost, because the dictionary is a compile-time argument folded away: this is exactly what a typeclass
        system does under the hood — thread a dictionary to the use site — except here the dictionary is an
        ordinary record you can see, and the erasure is the general <C>const</C> mechanism, not special trait
        plumbing.
      </P>

      <H2>The built-in instances: typed operators</H2>
      <P>
        The prelude's operators are this same shape, pre-supplied. <C>+</C> is one surface name backed by a
        per-type implementation — integer addition for integers, floating-point for floats, exact for
        rationals — chosen by the operand type. Here the very same <C>+</C> and <C>&lt;</C> run over floats
        and over integers in one program:
      </P>
      <Runnable
        source={`(def (main)
  (if (< (+ 1.5 1.5) 4.0)
    (+ 1 2)
    99))`}
      />
      <P>
        The <C>(+ 1.5 1.5)</C> is float addition, the <C>(+ 1 2)</C> integer addition — same symbol, the
        right implementation per type, folding to <C>3</C>. You didn't pass a dictionary because the prelude
        already provides the instances; it's the record-of-operations idea with the record built in.
      </P>

      <H2>And the implicit face: generic specialization</H2>
      <P>
        One more form needs no dictionary at all: a generic function the compiler <em>monomorphizes</em>.
        Write <C>len</C> with no type annotations and it works for any list; the compiler emits a distinct
        specialized copy per concrete type it's actually called at:
      </P>
      <Runnable
        source={`(def (len xs)
  (match xs
    ((list) 0)
    ((list h .. t) (+ 1 (len t)))))
(def (main)
  (+ (len (list 1 2 3)) (len (list "a" "b"))))`}
      />
      <P>
        That calls <C>len</C> at <C>List Int64</C> and <C>List String</C> — <C>3 + 2 = 5</C> — with a
        separate machine function for each, no runtime type tag. (<C>cdz instantiations len some-file.cdz</C>{" "}
        lists the concrete types it was compiled at; the editor shows the same as a CodeLens above the
        definition.) This is the <em>parametric</em> case — one algorithm reused — while the dictionary and
        the operators are the genuinely per-type-<em>behaviour</em> cases.
      </P>

      <Note>
        What Cadenza deliberately does <em>not</em> have: implicit instance resolution. There's no trait
        keyword and no compiler search for "the <C>Show</C> instance for this type" from ambient scope — you
        pass the dictionary yourself, or use a prelude operator that already carries its instances. You also
        can't declare two functions with the same name and have argument types pick between them; that's
        rejected, not resolved. The upshot is that a call's meaning is always visible in its arguments,
        never conjured from a global table.
      </Note>

      <Why tenet="Traits are records you pass in — no new machinery">
        A typeclass or trait system is a second little language: instance declarations, resolution rules,
        coherence checks. Cadenza gets the same expressiveness from what it already has — a record of
        functions is the dictionary, passing it is dispatch, and marking it <C>const</C> makes it free
        (inlined and erased, exactly what a typeclass compiler emits). Because the dictionary is explicit,
        there's no invisible resolution to reason about: you can always read which implementation a call
        uses. And the prelude's typed operators are just the pre-built instances of this shape, so the
        everyday <C>+</C> and the hand-rolled <C>show-with</C> are the same idea at two levels.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="ad-hoc-polymorphism:1"
        prompt={
          <>
            A dictionary is a record of functions. <C>apply-op</C> calls the record's <C>op</C> field on{" "}
            <C>x</C>. Fill the function stored in the record so <C>(apply-op … 5)</C> triples its input,
            giving <C>15</C>.
          </>
        }
        starter={`(def (triple n) (* 3 n))
(def (apply-op dict x) ((. dict op) x))
(def (main) (apply-op (record (op ?)) 5))`}
        solution={`(def (triple n) (* 3 n))
(def (apply-op dict x) ((. dict op) x))
(def (main) (apply-op (record (op triple)) 5))`}
        expected="15"
        hint={
          <>
            The record's <C>op</C> field is the function to call — put <C>triple</C> there, and{" "}
            <C>apply-op</C> calls it on <C>5</C> for <C>15</C>. Dispatch is just choosing which function the
            record carries.
          </>
        }
      />

      <Exercise
        id="ad-hoc-polymorphism:2"
        prompt={
          <>
            The same <C>+</C> means integer or float addition depending on its operands. Fill the float so
            the comparison <C>(&lt; (+ 2.0 ?) 5.0)</C> is <em>true</em> — selecting the{" "}
            <C>(+ 10 20)</C> integer branch, giving <C>30</C>.
          </>
        }
        starter={`(def (main)
  (if (< (+ 2.0 ?) 5.0) (+ 10 20) 0))`}
        solution={`(def (main)
  (if (< (+ 2.0 1.0) 5.0) (+ 10 20) 0))`}
        expected="30"
        hint={
          <>
            You need <C>(+ 2.0 ?)</C> to stay under <C>5.0</C>, so any float below <C>3.0</C> works —{" "}
            <C>1.0</C> gives <C>3.0 &lt; 5.0</C>, true. The addition on <C>2.0</C> is float addition, while{" "}
            <C>(+ 10 20)</C> is integer addition: one operator, its per-type instances.
          </>
        }
      />
    </article>
  );
}
