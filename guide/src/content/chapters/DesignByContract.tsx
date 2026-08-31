// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";

export default function Contracts() {
  return (
    <article>
      <H1>Design by contract</H1>
      <Lede>A function usually carries assumptions, like "this count is never negative" or "I always return something in range". Cadenza lets you write those assumptions down as <em>contracts</em>, and turns them into checks it enforces every time the function runs.</Lede>
      <P>Two annotations do it. <C>@requires</C> states a <em>precondition</em>, what must hold when the function is called, while <C>@ensures</C> states a <em>postcondition</em>, what must hold about its result. Each becomes a check the compiler injects at the function's boundary: violate it and the program <em>traps</em> right there, instead of quietly computing on bad data.</P>
      <H2>@requires: a precondition</H2>
      <P>Here <C>f</C> promises to work only for non-negative inputs. Called with <C>5</C>, it returns <C>6</C>, so the check passes and doesn't change the value:</P>
      <Runnable
        source={`(def (main) (f 5))

(@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))`}
      />
      <P>Feed it a value that breaks the promise and it stops at the boundary, so <C>f(-5)</C> traps, because <C>-5</C> is not <C>{">= 0"}</C>:</P>
      <Runnable
        source={`(def (main) (f -5))

(@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))`}
        expect="error"
      />
      <Note>The precondition is checked <em>once, at entry</em>, which is the Hoare-logic reading of <C>{"{P} body {Q}"}</C>. Because it lives on the function, not the call, it holds even for higher-order or indirect calls. Preconditions also <em>stack</em>: put several <C>@requires</C> on one function and all of them are enforced.</Note>
      <H2>@ensures: a postcondition</H2>
      <P>A postcondition constrains the <em>result</em>. Inside <C>@ensures</C>, the name <C>ret</C> refers to the value the function returns. This <C>f</C> promises a non-negative result, so with <C>200</C> it returns <C>100</C>, honestly:</P>
      <Runnable
        source={`(def (main) (f 200))

(@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))`}
      />
      <P>But <C>f(5)</C> would compute <C>-95</C>, a broken promise, so it traps on the way out instead of returning a wrong answer:</P>
      <Runnable
        source={`(def (main) (f 5))

(@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))`}
        expect="error"
      />
      <H2>The full contract</H2>
      <P>Combine them and you have design by contract in full: <C>@requires</C> guards the input at entry, <C>@ensures</C> guards the result at exit, and they compose. This <C>f</C> demands a non-negative input <em>and</em> promises a non-negative output:</P>
      <Runnable
        source={`(def (main) (f 200))

(@ (requires (>= x 0)) (@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100))))`}
      />
      <P>Called with <C>200</C> both hold and it returns <C>100</C>. Call it with <C>-5</C> and it traps at <em>entry</em> (the precondition); call it with <C>5</C> and it traps at <em>exit</em> (the postcondition catches the <C>-95</C>). A caller can only ever see a result that satisfies the contract, or a clean trap at the exact boundary that broke it.</P>
      <Note>Two things to know: a contract may reference only the function's parameters (and prelude/global names), so naming something out of scope is a compile error at the annotation. And a function whose parameter is named <C>ret</C> can't carry an <C>@ensures</C>: the result binder <C>ret</C> would shadow the parameter, so rather than silently ignore the postcondition, the compiler rejects it and asks you to rename the parameter.</Note>
      <H2>A contract is an ordinary boolean</H2>
      <P>A contract's predicate is just a boolean expression in the def's scope, so it can say far more than <C>&gt;= 0</C>. It can relate two parameters (<C>{"(requires (< lo hi))"}</C>), compare the result to an input (<C>{"(ensures (> ret x))"}</C>), reach into a tuple or record, or even <em>match</em> a sum result. Here <C>safe-dec</C> returns an <C>Option</C>, and the postcondition matches it, promising that whenever there <em>is</em> a value it's non-negative:</P>
      <Runnable
        source={`(def (main) (safe-dec 5))

(@
  (requires (>= x 0))
  (@
    (ensures (match ret ((Some v) (>= v 0)) ((None _u) true)))
    (def (safe-dec (: x Int64)) (if (> x 0) (Some (- x 1)) (None unit)))))`}
      />
      <P>Called with <C>5</C> the result is <C>(Some 4)</C>, and the postcondition's <C>Some</C> arm checks <C>4 &gt;= 0</C>. Because the predicate is ordinary code, a contract is as expressive as any other boolean you could write, it just runs at the boundary instead of in the body.</P>
      <H2>@invariant: guarding a type</H2>
      <P><C>@requires</C> guards a function's entry and <C>@ensures</C> its exit. The third contract, <C>@invariant</C>, guards a <em>type</em>. Put it on a value type and the compiler enforces the predicate at <em>every</em> point a value of that type is built, so a value that breaks it can never come into existence. The predicate is written over <C>self</C>, the value being constructed. Here a <C>Percent</C> must stay between <C>0</C> and <C>100</C>, and building one in range works:</P>
      <Runnable
        source={`(do
  (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))

  (def (mk (: v Int64)) (let (((Percent.Pct p) (Percent.Pct v))) p))

  (def (main) (mk 50))

  (export main))`}
        wrap={false}
      />
      <P><C>mk(50)</C> constructs a <C>Percent</C> and reads back <C>50</C>. Try to build one out of range and it traps at the moment of construction, before the invalid value can exist, so <C>mk(150)</C> never returns a bad <C>Percent</C>:</P>
      <Runnable
        source={`(do
  (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))

  (def (mk (: v Int64)) (let (((Percent.Pct p) (Percent.Pct v))) p))

  (def (main) (mk 150))

  (export main))`}
        expect="error"
        wrap={false}
      />
      <P>The same guards any type with an invariant, a non-empty list or a normalized vector just as much as a bounded number. Because the check fires at construction, downstream code never re-validates: a <C>Percent</C> in hand is <em>always</em> in range, so the illegal state is unrepresentable rather than merely discouraged.</P>
      <Why tenet="Make the assumption an enforced check, not a comment">Every function, and every type, carries assumptions; usually they live in a comment or someone's head, and a violation surfaces far away as corrupted data. A contract puts the assumption where the compiler can enforce it, checked at the boundary, failing loudly and locally the instant it's broken. It's the same <Ch to="/errors"> Option and Result </Ch> instinct of making the failure a value you must confront, turned on a function's inputs and results and on a type's own values.</Why>
      <P>These contracts are checked at run time today. They're also the first step of a larger story: the same <C>@requires</C> a compiler could one day <em>prove</em> and remove, the assumption verified once, statically, instead of on every call. For now, a contract turns something you already assumed into a checked guarantee, just by writing it down. Next, <Ch to="/modules"> Modules </Ch> gather related definitions, contracts and all, under a name.</P>
    </article>
  );
}
