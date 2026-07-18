import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Link } from "react-router-dom";
import { Why } from "../../components/Why.tsx";

export default function DesignByContract() {
  return (
    <article>
      <H1>Design by contract</H1>
      <Lede>
        A function usually carries assumptions — "this count is never negative", "I always return
        something in range". Cadenza lets you write those assumptions down as <em>contracts</em>, and
        turns them into checks it enforces every time the function runs.
      </Lede>

      <P>
        Two annotations do it. <C>@requires</C> states a <em>precondition</em> — what must hold when the
        function is called. <C>@ensures</C> states a <em>postcondition</em> — what must hold about its
        result. Each becomes a check the compiler injects at the function's boundary: violate it and the
        program <em>traps</em> right there, instead of quietly computing on bad data.
      </P>

      <H2>@requires — a precondition</H2>
      <P>
        Here <C>f</C> promises to work only for non-negative inputs. Called with <C>5</C>, it returns{" "}
        <C>6</C> — the check passes and doesn't change the value:
      </P>
      <Runnable
        source={`(def (main) (f 5))
(@ (requires (>= x 0))
  (def (f (: x Int64)) (+ x 1)))`}
      />
      <P>
        Feed it a value that breaks the promise and it stops at the boundary — <C>f(-5)</C> traps,
        because <C>-5</C> is not <C>{`>= 0`}</C>:
      </P>
      <Runnable
        expect="error"
        source={`(def (main) (f -5))
(@ (requires (>= x 0))
  (def (f (: x Int64)) (+ x 1)))`}
      />
      <Note>
        The precondition is checked <em>once, at entry</em> — the Hoare-logic reading of{" "}
        <C>{`{P} body {Q}`}</C>. Because it lives on the function, not the call, it holds even for
        higher-order or indirect calls. Preconditions also <em>stack</em>: put several <C>@requires</C> on
        one function and all of them are enforced.
      </Note>

      <H2>@ensures — a postcondition</H2>
      <P>
        A postcondition constrains the <em>result</em>. Inside <C>@ensures</C>, the name <C>it</C> refers
        to the value the function returns. This <C>f</C> promises a non-negative result — with <C>200</C>{" "}
        it returns <C>100</C>, honestly:
      </P>
      <Runnable
        source={`(def (main) (f 200))
(@ (ensures (>= it 0))
  (def (f (: x Int64)) (- x 100)))`}
      />
      <P>
        But <C>f(5)</C> would compute <C>-95</C> — a broken promise — so it traps on the way out instead
        of returning a wrong answer:
      </P>
      <Runnable
        expect="error"
        source={`(def (main) (f 5))
(@ (ensures (>= it 0))
  (def (f (: x Int64)) (- x 100)))`}
      />

      <H2>The full contract</H2>
      <P>
        Combine them and you have design by contract in full: <C>@requires</C> guards the input at entry,{" "}
        <C>@ensures</C> guards the result at exit, and they compose. This <C>f</C> demands a non-negative
        input <em>and</em> promises a non-negative output:
      </P>
      <Runnable
        source={`(def (main) (f 200))
(@ (requires (>= x 0))
  (@ (ensures (>= it 0))
    (def (f (: x Int64)) (- x 100))))`}
      />
      <P>
        Called with <C>200</C> both hold and it returns <C>100</C>. Call it with <C>-5</C> and it traps at{" "}
        <em>entry</em> (the precondition); call it with <C>5</C> and it traps at <em>exit</em> (the
        postcondition catches the <C>-95</C>). A caller can only ever see a result that satisfies the
        contract — or a clean trap at the exact boundary that broke it.
      </P>
      <Note>
        Two things to know: a contract may reference only the function's parameters (and prelude/global
        names) — naming something out of scope is a compile error at the annotation. And avoid naming a
        parameter <C>it</C> when you use <C>@ensures</C>: the result binder would shadow it, and the
        postcondition is skipped.
      </Note>

      <Why tenet="Make the assumption an enforced check, not a comment">
        Every function has assumptions; usually they live in a comment or someone's head, and a violation
        surfaces far away as corrupted data. A contract puts the assumption where the compiler can enforce
        it — checked at the boundary, failing loudly and locally the instant it's broken. It's the same{" "}
        <Link to="/errors" className="text-cadenza-300 underline-offset-2 hover:underline">
          Option and Result
        </Link>{" "}
        instinct — make the failure a value you must confront — turned on a function's own promises.
      </Why>

      <P>
        These contracts are checked at run time today. They're also the first step of a larger story: the
        same <C>@requires</C> a compiler could one day <em>prove</em> and remove — the assumption verified
        once, statically, instead of on every call. For now, a contract is a guarantee you get for free
        just by writing down what you already assumed. Next,{" "}
        <Link to="/modules" className="text-cadenza-300 underline-offset-2 hover:underline">
          Modules
        </Link>{" "}
        gather related definitions — contracts and all — under a name.
      </P>
    </article>
  );
}
