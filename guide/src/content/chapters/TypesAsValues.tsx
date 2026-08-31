// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function TypesAsValues() {
  return (
    <article>
      <H1>Types as values</H1>
      <Lede>In most languages, types live in a sealed-off world you can't compute with, so you can't pass one to a function or compare two of them. What if a type were just a value? In Cadenza it is: you can bind a type to a name, pass it, return it, compare two of them, and even branch on them. The catch: it's all settled at compile time, and erased before the program runs.</Lede>
      <H2>A type is a value</H2>
      <P>Bind <C>Int64</C> to a name and return it. The program's result <em>is</em> that type, so it crosses the boundary as <C>Int64</C>, and its own type is <C>Type</C>, the type of types:</P>
      <Runnable
        source={`(let ((t Int64)) t)`}
      />
      <P>The result reads <C>Int64 : Type</C>. A type-value is fully known at compile time, so it flows out of a program directly, but only a <em>determined</em> type does; a type still waiting on a parameter has no runtime form and is refused.</P>
      <H2>Reflecting a value's type</H2>
      <P><C>Type.of</C> gives you the type-value of any expression, computed by the same inference that checks the rest of the program. You can feed it straight back as an annotation, here reflecting <C>y</C>'s type (<C>Int64</C>) and using it to annotate <C>100</C>, which agrees and is transparent:</P>
      <Runnable
        source={`(let ((y 42)) (: 100 (Type.of y)))`}
      />
      <P>The reflected type is a real type, checked in full: annotate a value it <em>doesn't</em> match and you get the same <C>CDZ0203</C> a written mismatch would. <C>Type.of</C> reads the static type, not a runtime value, so it's decided and erased before anything runs.</P>
      <H2>Comparing types</H2>
      <P>Two type-values compare with <C>Type.eq</C>, which folds to a constant <C>Bool</C> at compile time. Same type, <C>true</C>:</P>
      <Runnable
        source={`(Type.eq (Type.of 5) (Type.of 6))`}
      />
      <P>Different types, <C>false</C>, because <C>Int64</C> is not <C>Bool</C>:</P>
      <Runnable
        source={`(Type.eq (Type.of 5) (Type.of true))`}
      />
      <P>The comparison is exact and structural, and it carries the <em>whole</em> type. A quantity's unit is part of its type, so a length and a time compare unequal even at the same magnitude:</P>
      <Runnable
        source={`(Type.eq (Type.of (Qty.of 1.0 (Unit.base #"meter"))) (Type.of (Qty.of 1.0 (Unit.base #"second"))))`}
      />
      <H2>Branching on a type</H2>
      <P>Because <C>Type.eq</C> produces a compile-time constant, an <C>if</C> over it selects a branch <em>at compile time</em>, so the program branches on types, with no runtime test emitted. Here the condition is <C>true</C>, so the whole expression is <C>100</C>:</P>
      <Runnable
        source={`(if (Type.eq (Type.of 5) Int64) 100 200)`}
      />
      <P>A written type (<C>Int64</C>) and a reflected one (<Cadenza>(Type.of 5)</Cadenza>) are the same kind of value, so they compare freely. This is the point where inference and first-class types meet: the compiler computes a type, you compare it, and the answer picks the code, all before the program runs.</P>
      <Note>This is the surface the compiler's own generic machinery is built on: a type passed as a value drives <em>ad-hoc polymorphism</em> (one name, a per-type implementation chosen at compile time), and a <C>const</C> parameter lets a caller fix a compile-time-known argument that's then erased. Both compile to specialized, type-free code, and the observable <em>value</em> is identical to the hand-written monomorphic version, so the abstraction costs nothing at runtime.</Note>
      <Why tenet="Types are first-class values, decided then erased">Most languages put types in a separate universe, a grammar you can't compute with, gone by the time anything runs. Cadenza keeps the erasure but drops the separation: a type is a value of type <C>Type</C>, computed and compared with ordinary code, so reflection (<C>Type.of</C>), type equality (<C>Type.eq</C>), and compile-time type branches need no new machinery. And because a type-value never flows from runtime data, all of it settles at compile time and vanishes, so you get the expressiveness of computing with types and the zero cost of erasing them.</Why>
      <H2>Your turn</H2>
      <Exercise
        id="types-as-values:1"
        prompt={<>Compare two types at compile time. Fill the type to compare against so that <Cadenza>(Type.of true)</Cadenza> matches, making the comparison <C>true</C>, not <C>false</C>.</>}
        starter={`(Type.eq (Type.of true) ?)`}
        solution={`(Type.eq (Type.of true) Bool)`}
        expected="true"
        hint={<><Cadenza>(Type.of true)</Cadenza> is the type <C>Bool</C>, so comparing it against <C>Bool</C> is <C>true</C>. (Compare against <C>Int64</C> and you'd get <C>false</C>.)</>}
      />
      <Exercise
        id="types-as-values:2"
        prompt={<>Two type-values are equal only when the types match exactly. Fill the value so its reflected type equals <Cadenza>(Type.of 5)</Cadenza>, an <C>Int64</C>, making the check <C>true</C>.</>}
        starter={`(Type.eq (Type.of 5) (Type.of ?))`}
        solution={`(Type.eq (Type.of 5) (Type.of 99))`}
        expected="true"
        hint={<><Cadenza>(Type.of 5)</Cadenza> is <C>Int64</C>, so you need another value whose type is <C>Int64</C>, any integer literal, e.g. <C>99</C>. A <C>true</C> or a <C>1.0</C> would be a different type and give <C>false</C>.</>}
      />
    </article>
  );
}
