import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";

export default function Lists() {
  return (
    <article>
      <H1>Lists</H1>
      <Lede>Ordered, immutable sequences — built and measured on the value heap.</Lede>

      <P>
        A list is written with <C>list</C>. Lists are <em>persistent</em>: operations like{" "}
        <C>List.push</C> and <C>List.concat</C> return a new list and leave the original untouched.
        Ask a list its length with <C>List.len</C>.
      </P>
      <Runnable source={`(List.len (list 1 2 3))`} />

      <Note>
        These examples return a <em>number</em> computed from a list (its length), rather than the
        list itself — returning a whole list across the program boundary is a capability the compiler
        is still growing. Everything <em>inside</em> the program can build and transform lists freely.
      </Note>

      <H2>Building lists</H2>
      <P>
        <C>List.push</C> adds an element to the end; <C>List.concat</C> joins two lists. Because they
        return new lists, you can chain them and measure the result:
      </P>
      <Runnable source={`(List.len (List.push (list 1 2) 3))`} />
      <Runnable source={`(List.len (List.concat (list 1 2) (list 3 4 5)))`} />

      <H2>Lists through functions</H2>
      <P>A function can take a list and compute over it. Here <C>count</C> just reports its length:</P>
      <Runnable
        wrap={false}
        source={`(module m
  (def (count xs) (List.len xs))
  (def (main) (count (list 10 20 30 40)))
  (export main))`}
      />

      <H2>Your turn</H2>
      <Exercise
        prompt={<>Concatenate the two lists, then report the total length — it should be <C>5</C>.</>}
        starter={`(List.len (List.concat (list 1 2) ?))`}
        solution={`(List.len (List.concat (list 1 2) (list 3 4 5)))`}
        expected="5"
        hint={<>The second argument to <C>List.concat</C> is another <C>(list …)</C> with three elements.</>}
      />
    </article>
  );
}
