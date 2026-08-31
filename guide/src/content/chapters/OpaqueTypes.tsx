// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function OpaqueTypes() {
  return (
    <article>
      <H1>Opaque types</H1>
      <Lede>How do you guarantee a value is <em>always</em> valid, so a percentage is never above 100 and a list is never empty, no matter what code touches it? Make the type <em>opaque</em> by having a module export the type's <em>name</em> while keeping its <em>constructor</em> private, which is an abstract data type. Code elsewhere can hold and pass its values and call the module's functions on them, but it can't build or take one apart, so an invariant established when the value is made holds <em>everywhere</em>, forever.</Lede>
      <P>Exporting a type and exporting its <em>constructors</em> are two independent decisions. A bare <Cadenza>(export Percent)</Cadenza> publishes only the type's <em>handle</em>, enough to name it but not to construct it. Adding <Cadenza>(export Percent.*)</Cadenza> (or naming a specific variant) publishes the constructors too, making the type <em>concrete</em>. So opacity is the <em>default</em> of exporting a type; concreteness is opt-in. Withholding the constructor is what makes a type opaque.</P>
      <H2>A validated type: Percent</H2>
      <P>Here's where it earns its keep. A <em>percentage</em> should always be between 0 and 100, since a discount of 150% or −20% is nonsense. Model it as an opaque <C>Percent</C> whose only maker, <C>percent</C>, <em>validates</em>: it clamps anything out of range into <C>[0, 100]</C>. Feed it a wild <C>150</C> and what comes back is a legitimate <C>100</C>:</P>
      <Runnable
        source={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (main) (rate (percent 150)))`}
      />
      <P>What this buys you isn't the clamping itself but what every <em>downstream</em> function can now assume. Because a <C>Percent</C> can only come from <C>percent</C>, any code that receives one <em>knows</em> it's in range, with no re-checking. Here <C>apply-discount</C> takes a price and a <C>Percent</C> and subtracts that fraction, so a 25% discount off 200 is 150:</P>
      <Runnable
        source={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (apply-discount (: price Int64) (: p Percent)) (- price (/ (* price (rate p)) 100)))

(def (main) (apply-discount 200 (percent 25)))`}
      />
      <P>And the invariant is what makes <C>apply-discount</C> <em>safe</em>: a discount can never exceed 100%, so a price can never go negative. Try to discount by a nonsensical 150% and the <C>percent</C> maker has already clamped it to 100%, so the worst case is a free item (price <C>0</C>), never a negative one:</P>
      <Runnable
        source={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (apply-discount (: price Int64) (: p Percent)) (- price (/ (* price (rate p)) 100)))

(def (main) (apply-discount 200 (percent 150)))`}
      />
      <P><C>apply-discount</C> never validates its <C>Percent</C> because it doesn't have to. The type is a <em>proof</em> the value was checked once, at the only place it could be made. That's the difference between a bare <C>Int64</C> (which every consumer must defensively re-check) and an opaque <C>Percent</C> (checked once, trusted everywhere).</P>
      <H2>The boundary is what enforces it</H2>
      <P>These run in one file, where the constructor <C>Pct</C> is visible, so you can see the whole mechanism. The <em>enforcement</em> lives at the module boundary. When <C>Percent</C> is its own module exporting only its handle plus <C>percent</C>, <C>rate</C>, and <C>apply-discount</C>, another module may name <C>Percent</C>, hold one, and call those functions, but it may <em>not</em> reach the constructor to forge an out-of-range one. An attempt is a compile error:</P>
      <Note><C>{"// module \"percent\" does (export Percent)  — the handle only, Pct stays private"}</C> <br /> <C>{"// another module tries: (Percent.Pct 150)   — a 150% \"percentage\", skipping the validator"}</C> <br /> <C>cdz</C> reports <C>CDZ0214</C>: the constructor <C>Pct</C> is withheld, so a <C>Percent</C> can be built only through the module's exported functions.</Note>
      <P>The same wall stops an importer from taking a value apart, since it can't match on <C>Pct</C>, strip it, or structurally compare two <C>Percent</C>s to reverse-engineer the representation. Every <C>Percent</C> that exists anywhere in the program came from <C>percent</C> and is therefore in range: the invariant is not a convention the caller must remember, it's a fact the type system guarantees. (The guide runs one module at a time, so the runnables above show the mechanism from <em>inside</em>; the <C>CDZ0214</C> rejection is what the compiler prints when the forge is attempted from <em>outside</em>.)</P>
      <Why tenet="Hide the representation, and the invariant can't be broken">Data hiding here isn't a convention or a naming trick but something checked by the type system. Because a type's handle and its constructors export independently, a module can publish a fully usable type whose <em>representation</em> is genuinely unreachable: the only values that exist are the ones its own functions made. So an invariant established in a smart constructor, whether in range, non-empty, sorted, normalized, or validated, holds for <em>every</em> value of that type, everywhere, with zero trust in callers. Cadenza leans on this at the highest stakes: its machine-checked proof kernel makes a <C>Thm</C> (theorem) an opaque type whose constructor is private, so the <em>only</em> way to obtain a <C>Thm</C> is to call one of the kernel's sound inference rules, so a bug in a tactic literally cannot forge a false theorem, because it cannot construct a <C>Thm</C> at all. Same mechanism as <C>Percent</C>, protecting soundness itself.</Why>
      <H2>Your turn</H2>
      <Exercise
        id="opaque-types:1"
        prompt={<>The validator clamps out-of-range input into <C>[0, 100]</C>. Fill an input that's <em>too large</em> so that reading the rate back gives <C>100</C>, the ceiling the invariant enforces.</>}
        starter={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (main) (rate (percent ?)))`}
        solution={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (main) (rate (percent 250)))`}
        expected="100"
        hint={<>Any value above <C>100</C> clamps to <C>100</C>, so <C>250</C> does too. The maker is the one place the ceiling is enforced, so <C>rate</C> can never read more than <C>100</C>.</>}
      />
      <Exercise
        id="opaque-types:2"
        prompt={<>Because a <C>Percent</C> is always in range, <C>apply-discount</C> is safe. Fill the discount so that 300 becomes 210, a 30% discount off 300.</>}
        starter={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (apply-discount (: price Int64) (: p Percent)) (- price (/ (* price (rate p)) 100)))

(def (main) (apply-discount 300 (percent ?)))`}
        solution={`(type Percent (Pct Int64))

(def
  (percent (: n Int64))
  (if (< n 0) (Percent.Pct 0) (if (> n 100) (Percent.Pct 100) (Percent.Pct n))))

(def (rate (: p Percent)) (let (((Percent.Pct v) p)) v))

(def (apply-discount (: price Int64) (: p Percent)) (- price (/ (* price (rate p)) 100)))

(def (main) (apply-discount 300 (percent 30)))`}
        expected="210"
        hint={<>30% of 300 is 90, and 300 − 90 is 210, so the discount is <C>30</C>. Because the type guarantees the rate is at most 100, the discounted price is never negative.</>}
      />
    </article>
  );
}
