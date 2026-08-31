// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Iterators() {
  return (
    <article>
      <H1>Iterators & ranges</H1>
      <Lede>A list holds all its elements at once. An <em>iterator</em> produces them one at a time, on demand, so you can describe an enormous (even endless) sequence and pull only the few values you actually need.</Lede>
      <P>The shape is a <em>lazy pull</em>: an iterator answers one question, <C>next</C>, "give me the next element, and the iterator for the rest." When there's nothing left it says so. We model the answer as an <C>Option</C> of a <C>(element, rest)</C> pair: <C>(Some #tuple(v rest))</C> yields <C>v</C> and hands back the iterator <C>rest</C> for what follows, or <C>(None unit)</C> when the sequence is exhausted.</P>
      <H2>An iterator is a value you step</H2>
      <P>Rather than a function that hides its state, we make the iterator an ordinary <em>value</em>, a small sum type naming each kind of iterator, and <C>next</C> interprets one step of it. A <C>Range</C> yields <C>lo</C>, then the range starting at <C>lo + 1</C>, until it reaches <C>hi</C>. Summing a range by stepping it to exhaustion, 1 through 4, gives <C>10</C>:</P>
      <Runnable
        source={`(type Iter (Range (Tuple Int64 Int64)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Range #tuple(1 5))))`}
      />
      <P><C>next</C> is an ordinary recursive function over a plain sum, with no hidden mutable cursor. Exhaustion isn't a special error; it's just the <C>None</C> case, so stepping is <em>total</em> and never traps, whatever the range.</P>
      <H2>Lazy: only what you pull</H2>
      <P>Laziness is the point. Add a <C>Take</C> iterator that wraps another and yields at most <C>n</C> of it, and you can put a bound in front of an <em>enormous</em> range of a million elements, yet only the first three are ever produced. Summing them is <C>0 + 1 + 2 = 3</C>, computed without walking the other 999,997:</P>
      <Runnable
        source={`(type Iter (Range (Tuple Int64 Int64)) (Take (Tuple Int64 Iter)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))
    ((Iter.Take nf)
      (let
        ((#tuple(n src) nf))
        (if
          (<= n 0)
          (None unit)
          (match
            (next src)
            ((None _) (None unit))
            ((Some p) (let ((#tuple(v rest) p)) (Some #tuple(v (Iter.Take #tuple((- n 1) rest))))))))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Take #tuple(3 (Iter.Range #tuple(0 1000000))))))`}
      />
      <P>The <C>Take</C> stops asking after three, so <C>Range</C> is only ever stepped three times. Nothing forces the whole sequence into existence, since each element is computed exactly when the consumer pulls it.</P>
      <H2>Transformers compose</H2>
      <P>Because each iterator kind wraps another, they stack. Add a <C>Double</C> that doubles whatever its inner iterator yields, and you can layer <C>Double</C> over <C>Take</C> over <C>Range</C>, a pipeline that doubles the first three of a huge range: <C>0, 2, 4</C>, summing to <C>6</C>:</P>
      <Runnable
        source={`(type Iter (Range (Tuple Int64 Int64)) (Take (Tuple Int64 Iter)) (Double Iter))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))
    ((Iter.Take nf)
      (let
        ((#tuple(n src) nf))
        (if
          (<= n 0)
          (None unit)
          (match
            (next src)
            ((None _) (None unit))
            ((Some p) (let ((#tuple(v rest) p)) (Some #tuple(v (Iter.Take #tuple((- n 1) rest))))))))))
    ((Iter.Double src)
      (match
        (next src)
        ((None _) (None unit))
        ((Some p) (let ((#tuple(v rest) p)) (Some #tuple((* 2 v) (Iter.Double rest)))))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Double (Iter.Take #tuple(3 (Iter.Range #tuple(0 1000000)))))))`}
      />
      <P>Each layer only asks its inner iterator for the next element and transforms it, so the whole pipeline stays lazy, and you assemble complex sequences from small, independent pieces.</P>
      <Note>This <em>reified</em> encoding, an iterator as a sum of step-shapes with <C>next</C> a plain recursive function, is what the standard iterator library uses today, and it's why: the more obvious "an iterator is a function returning the next element and a new function" needs a recursive function <em>type</em> the inference won't tie without a nominal constructor to break the cycle. The sum form <em>is</em> that constructor, so it sidesteps the problem and reads just as clearly. A real library adds <C>map</C>, <C>filter</C>, <C>zip</C>, and friends the same way, one more variant each.</Note>
      <Why tenet="Describe the sequence, produce only what's used">A list commits to every element up front; an iterator commits to none until asked. That's what lets a range be effectively infinite, a transformer be free (it does nothing until pulled), and a pipeline cost only what the consumer actually reads. And because the iterator is a plain value that <C>next</C> steps, not a hidden one-shot cursor, the same iterator is re-steppable and there's no "already consumed" trap: laziness without the usual footguns.</Why>
      <P>Lists and iterators both keep things in <em>order</em>. When the question is instead membership, or a key-to-value association, you reach for a different collection: <em>maps &amp; sets</em>, next.</P>
      <H2>Your turn</H2>
      <Exercise
        id="iterators:1"
        prompt={<>Step a range to exhaustion. <C>next</C> and <C>sum-it</C> are written; fill in the range's <em>upper</em> bound so summing <C>Range(2, ?)</C>, the half-open <C>2, 3, 4</C>, gives <C>9</C>.</>}
        starter={`(type Iter (Range (Tuple Int64 Int64)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Range #tuple(2 ?))))`}
        solution={`(type Iter (Range (Tuple Int64 Int64)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Range #tuple(2 5))))`}
        expected="9"
        hint={<>The range is half-open <C>[lo, hi)</C>, so <C>hi</C> is excluded. To yield <C>2, 3, 4</C> (which sum to <C>9</C>), stop before <C>5</C>, so the upper bound is <C>5</C>.</>}
      />
      <Exercise
        id="iterators:2"
        prompt={<>Bound an endless range with <C>Take</C>. Fill in how many elements to take from <C>Range(0, 1000000)</C> so the sum of what's pulled, <C>0 + 1 + 2 + 3</C>, is <C>6</C>.</>}
        starter={`(type Iter (Range (Tuple Int64 Int64)) (Take (Tuple Int64 Iter)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))
    ((Iter.Take nf)
      (let
        ((#tuple(n src) nf))
        (if
          (<= n 0)
          (None unit)
          (match
            (next src)
            ((None _) (None unit))
            ((Some p) (let ((#tuple(v rest) p)) (Some #tuple(v (Iter.Take #tuple((- n 1) rest))))))))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Take #tuple(? (Iter.Range #tuple(0 1000000))))))`}
        solution={`(type Iter (Range (Tuple Int64 Int64)) (Take (Tuple Int64 Iter)))

(def
  (next it)
  (match
    it
    ((Iter.Range r)
      (let
        ((#tuple(lo hi) r))
        (if (< lo hi) (Some #tuple(lo (Iter.Range #tuple((+ lo 1) hi)))) (None unit))))
    ((Iter.Take nf)
      (let
        ((#tuple(n src) nf))
        (if
          (<= n 0)
          (None unit)
          (match
            (next src)
            ((None _) (None unit))
            ((Some p) (let ((#tuple(v rest) p)) (Some #tuple(v (Iter.Take #tuple((- n 1) rest))))))))))))

(def
  (sum-it it)
  (match (next it) ((None _) 0) ((Some p) (let ((#tuple(v rest) p)) (+ v (sum-it rest))))))

(def (main) (sum-it (Iter.Take #tuple(4 (Iter.Range #tuple(0 1000000))))))`}
        expected="6"
        hint={<><C>0 + 1 + 2 + 3 = 6</C>, so you pull the first <C>4</C> elements. The range is effectively infinite; <C>Take</C> is what makes summing it terminate.</>}
      />
    </article>
  );
}
