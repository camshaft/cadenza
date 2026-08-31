// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function AdHocPolymorphism() {
  return (
    <article>
      <H1>Ad-hoc polymorphism</H1>
      <Lede>One name, more than one meaning, chosen per type. Most languages bolt on a trait or typeclass system for this, but Cadenza doesn't need one, because a "trait" is just a <em>record of functions you pass in</em> and dispatch is choosing which record to pass. No new machinery: records and functions you already have.</Lede>
      <P>"Polymorphism" means one piece of code working over many types. The <em>ad-hoc</em> kind is where the behaviour genuinely differs per type, so <C>show</C> renders an integer one way and a boolean another. Other languages introduce typeclasses (Haskell), traits (Rust), or interfaces to carry those per-type implementations. The Cadenza insight: that bundle of per-type operations is just a <strong>record whose fields are functions</strong>, and a polymorphic function is one that takes such a record as an argument and calls its fields.</P>
      <H2>A trait is a record of functions</H2>
      <P>Say you want a <C>show</C>-like operation that turns a value into a number, differently per type. The "trait" is a record with a <C>describe</C> field: <C>{"{ describe: T -> Int64 }"}</C>. An <em>instance</em> for a type is a record with that field filled in. A polymorphic function takes the record and calls <C>(. dict describe)</C>. Dispatch is picking which record to hand it:</P>
      <Runnable
        source={`(def (describe-int n) (+ n 100))

(def (describe-bool b) (if b 1 0))

(def (show-with dict x) (dict.describe x))

(def (int-show) #record((= describe describe-int)))

(def (bool-show) #record((= describe describe-bool)))

(def (main) (+ (show-with (int-show) 5) (show-with (bool-show) true)))`}
      />
      <P><C>show-with</C> is the polymorphic function, and it doesn't know or care <em>which</em> describe it's calling. <C>int-show</C> and <C>bool-show</C> are the two instances (the "trait implementations"), each a plain record. <C>main</C> dispatches by passing the right one, so <C>5</C> becomes <C>105</C> and <C>true</C> becomes <C>1</C>, summing to <C>106</C>. That's the whole mechanism, with no trait keyword and no instance resolution, just a record argument. And it's deliberate: the spec says a trait instance is an <em>explicitly-passed dictionary</em>, never resolved from ambient scope, so you can always see exactly which implementation a call uses by reading the argument.</P>
      <H2>Make the dictionary free: a const parameter</H2>
      <P>Passing a record around sounds like it should cost something at run time, but it doesn't have to, because the <C>const</C> parameter you met in the previous chapter applies here: mark the dictionary parameter <C>const</C> and the compiler inlines it into a specialized copy and erases it, so the <C>(. dict describe)</C> lookup folds to the concrete function with no record passed and no field looked up at run time.</P>
      <Runnable
        source={`(def (describe-int n) (+ n 100))

(def (describe-bool b) (if b 999 0))

(def (show-with (const dict) x) (dict.describe x))

(def
  (main)
  (+
    (show-with #record((= describe describe-int)) 5)
    (show-with #record((= describe describe-bool)) true)))`}
      />
      <P>The very same two instances as before, one for <C>Int64</C> and one for <C>Bool</C>, now dispatch through a <C>const</C> dictionary: <C>describe-int</C> turns <C>5</C> into <C>105</C>, <C>describe-bool</C> turns <C>true</C> into <C>999</C>, summing to <C>1104</C>. Because the parameter is <C>const</C> and left <em>unannotated</em>, <C>show-with</C> stays fully generic and works for <em>any</em> type whose dictionary carries a <C>describe</C>, yet the compiler still inlines each call into a specialized copy and erases the record. This is exactly what a typeclass system does internally by threading a dictionary to the use site, except here the dictionary is an ordinary record you can see, and the erasure is the general <C>const</C> mechanism rather than special trait plumbing.</P>
      <Note>Left the dictionary <em>unannotated</em> on purpose: that's what keeps <C>show-with</C> generic over the element type. Cadenza has no <C>∀</C>-binder inside an annotation, so you can't write a generic type like <C>{"(Record (: describe (-> a Int64)))"}</C> with a free <C>a</C>, so an unannotated parameter <em>is</em> the generic form. When you <em>want</em> the signature written out, there's a second way, below.</Note>
      <H2>Spelling out the generic: a type parameter</H2>
      <P>The unannotated version is generic but silent about it. If you'd rather <em>document</em> the constraint that "this works for some type <C>t</C>, given a dictionary of <C>describe : t → Int64</C>", take the type as an explicit <C>(: t Type)</C> parameter (the same first-class <em>types as values</em> idea) and annotate the rest in terms of it. The caller passes the concrete type at the front; both instances still dispatch through the one <C>show-with</C>:</P>
      <Runnable
        source={`(def (describe-int n) (+ n 100))

(def (describe-bool b) (if b 999 0))

(def (show-with (: t Type) (: dict (Record (: describe (-> t Int64)))) (: x t)) (dict.describe x))

(def
  (main)
  (+
    (show-with Int64 #record((= describe describe-int)) 5)
    (show-with Bool #record((= describe describe-bool)) true)))`}
      />
      <P>Same <C>1104</C>, now with a fully written-out signature: <C>show-with</C> declares that its element type is a parameter <C>t</C> and its dictionary must carry <C>describe : t → Int64</C>. Feed it <C>Int64</C> with an integer dictionary, or <C>Bool</C> with a boolean one, and each checks against that same shape. It's the more verbose of the two, but it says out loud what the unannotated form leaves to inference, so pick whichever fits how much you want the reader to see.</P>
      <H2>The built-in instances: typed operators</H2>
      <P>The prelude's operators are this same shape, pre-supplied. <C>+</C> is one surface name backed by a per-type implementation, choosing integer addition for integers, floating-point for floats, and exact for rationals by the operand type. Here the very same <C>+</C> and <C>&lt;</C> run over floats and over integers in one program:</P>
      <Runnable
        source={`(def (main) (if (< (+ 1.5 1.5) 4.0) (+ 1 2) 99))`}
      />
      <P>The <Cadenza>(+ 1.5 1.5)</Cadenza> is float addition and the <Cadenza>(+ 1 2)</Cadenza> integer addition, the same symbol picking the right implementation per type and folding to <C>3</C>. You didn't pass a dictionary because the prelude already provides the instances; it's the record-of-operations idea with the record built in.</P>
      <H2>And the implicit face: generic specialization</H2>
      <P>One more form needs no dictionary at all: a generic function the compiler <em>monomorphizes</em>. Write <C>len</C> with no type annotations and it works for any list; the compiler emits a distinct specialized copy per concrete type it's actually called at:</P>
      <Runnable
        source={`(def (len xs) (match xs (#list() 0) (#list(h (.. t)) (+ 1 (len t)))))

(def (main) (+ (len #list(1 2 3)) (len #list("a" "b"))))`}
      />
      <P>That calls <C>len</C> at <C>List Int64</C> and <C>List String</C> for <C>3 + 2 = 5</C>, with a separate machine function for each and no runtime type tag. (<C>cdz instantiations len some-file.cdz</C> lists the concrete types it was compiled at; the editor shows the same as a CodeLens above the definition.) This is the <em>parametric</em> case, one algorithm reused, whereas the dictionary and the operators are the genuinely per-type-<em>behaviour</em> cases.</P>
      <Note>What Cadenza deliberately does <em>not</em> have: implicit instance resolution. There's no trait keyword and no compiler search for "the <C>Show</C> instance for this type" from ambient scope, so you pass the dictionary yourself or use a prelude operator that already carries its instances. You also can't declare two functions with the same name and have argument types pick between them; that's rejected, not resolved. The upshot is that a call's meaning is always visible in its arguments, never conjured from a global table.</Note>
      <Why tenet="Traits are records you pass in, not new machinery">A typeclass or trait system is a second little language: instance declarations, resolution rules, coherence checks. Cadenza gets the same expressiveness from what it already has, since a record of functions is the dictionary, passing it is dispatch, and marking it <C>const</C> makes it free (inlined and erased, exactly what a typeclass compiler emits). Because the dictionary is explicit, there's no invisible resolution to reason about: you can always read which implementation a call uses. And the prelude's typed operators are just the pre-built instances of this shape, so the everyday <C>+</C> and the hand-rolled <C>show-with</C> are the same idea at two levels.</Why>
      <H2>Your turn</H2>
      <Exercise
        id="ad-hoc-polymorphism:1"
        prompt={<>A dictionary is a record of functions. <C>apply-op</C> calls the record's <C>op</C> field on <C>x</C>. Fill the function stored in the record so <C>(apply-op … 5)</C> triples its input, giving <C>15</C>.</>}
        starter={`(def (triple n) (* 3 n))

(def (apply-op dict x) (dict.op x))

(def (main) (apply-op #record((= op ?)) 5))`}
        solution={`(def (triple n) (* 3 n))

(def (apply-op dict x) (dict.op x))

(def (main) (apply-op #record((= op triple)) 5))`}
        expected="15"
        hint={<>The record's <C>op</C> field is the function to call, so put <C>triple</C> there and <C>apply-op</C> calls it on <C>5</C> for <C>15</C>. Dispatch is just choosing which function the record carries.</>}
      />
      <Exercise
        id="ad-hoc-polymorphism:2"
        prompt={<>The same <C>+</C> means integer or float addition depending on its operands. Fill the float so the comparison <C>(&lt; (+ 2.0 ?) 5.0)</C> is <em>true</em>, which selects the <Cadenza>(+ 10 20)</Cadenza> integer branch and gives <C>30</C>.</>}
        starter={`(def (main) (if (< (+ 2.0 ?) 5.0) (+ 10 20) 0))`}
        solution={`(def (main) (if (< (+ 2.0 1.0) 5.0) (+ 10 20) 0))`}
        expected="30"
        hint={<>You need <C>(+ 2.0 ?)</C> to stay under <C>5.0</C>, so any float below <C>3.0</C> works, and <C>1.0</C> gives <C>3.0 &lt; 5.0</C>, true. The addition on <C>2.0</C> is float addition, while <Cadenza>(+ 10 20)</Cadenza> is integer addition: one operator, its per-type instances.</>}
      />
    </article>
  );
}
