// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { AppLink } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Metaprogramming() {
  return (
    <article>
      <H1>Metaprogramming</H1>
      <Lede>What if a program could read and rewrite another program, with no macro language to learn, just the tools you already have? In Cadenza, code is data: <C>quote</C> hands you a program's structure as an ordinary value you can inspect, take apart, build up, and (if you like) run. There's no separate macro system; the AST is a sum type like any other, so you already know how to work with it.</Lede>
      <H2>Quote: a program as a value</H2>
      <P>Normally <Cadenza>(+ 1 2)</Cadenza> evaluates to <C>3</C>. Wrap it in <C>quote</C> and it doesn't run at all. You get back the <em>structure</em> of the expression instead: a list whose head is the name <C>+</C> and whose arguments are the integers <C>1</C> and <C>2</C>.</P>
      <Runnable
        source={`(quote (+ 1 2))`}
      />
      <P>The result reads <C>Ast.List([Ast.Name("+"), Ast.Int(1), Ast.Int(2)])</C>, a value of type <C>Ast</C>. Each syntactic form is a variant: an integer literal is an <C>Ast.Int</C>, a name is an <C>Ast.Name</C>, a compound form is an <C>Ast.List</C> of its parts. Quoting <em>reifies</em> the code into that tree without evaluating a thing inside it.</P>
      <H2>The AST is an ordinary sum</H2>
      <P>Because the AST is a plain sum type, you take it apart with <C>match</C>, exactly like <C>Option</C> or a traffic-light symbol. Match a quoted integer and bind its payload:</P>
      <Runnable
        source={`(match (quote 42) ((Ast.Int n) n) (_ (BigInt.of 0)))`}
      />
      <P>The <C>Ast.Int</C> arm binds <C>n = 42</C>. A quoted <em>compound</em> form is an <C>Ast.List</C>, so you can reach into its elements. Here we hand the element list straight back, so you see the operator name and both arguments as <C>Ast</C> nodes:</P>
      <Runnable
        source={`(match (quote (+ 1 2)) ((Ast.List elems) (Ast.List elems)) (_ (quote nil)))`}
      />
      <P>The result reads <C>Ast.List([Ast.Name("+"), Ast.Int(1), Ast.Int(2)])</C>: the operator name and its two arguments, each still an <C>Ast</C> node. And since <C>Ast</C> is an ordinary sum, its match obeys the same exhaustiveness rule as any other: a match that inspects one form carries a catch-all <C>_</C> for the rest (here it hands back a harmless <C>(quote nil)</C>).</P>
      <Note>You can build an AST directly with its constructors, too, and the two routes agree. A quoted literal produces the same node written by hand, so <C>(quote 42)</C> and <Cadenza>(Ast.Int 42)</Cadenza> both read <C>Ast.Int(42)</C>. Quote is just a convenient way to write down a tree you could also assemble constructor by constructor.</Note>
      <Runnable
        source={`(quote 42)`}
      />
      <Runnable
        source={`(Ast.Int 42)`}
      />
      <H2>Every literal has its variant</H2>
      <P>Each literal kind quotes to its own variant. A boolean quotes to an <C>Ast.Bool</C>, matches like any other variant, and its payload is a real <C>Bool</C>:</P>
      <Runnable
        source={`(match (quote false) ((Ast.Bool b) b) (_ true))`}
      />
      <P>The <C>Ast.Bool</C> arm binds <C>b = false</C>, so the whole match is <C>false</C>. A string works the same way: a string literal is an <C>Ast.Str</C>, distinct from an <C>Ast.Name</C> (which is an identifier):</P>
      <Runnable
        source={`(match (quote "hi") ((Ast.Str s) (Ast.Str s)) (_ (quote nil)))`}
      />
      <P>A float has its own variant too, <C>Ast.Float</C>, distinct from <C>Ast.Int</C>, so <C>(quote 2.5)</C> matches the float arm and hands its node back as <C>Ast.Float(2.5)</C>:</P>
      <Runnable
        source={`(match (quote 2.5) ((Ast.Float f) (Ast.Float f)) (_ (quote nil)))`}
      />
      <P>A byte-string literal has its own variant too. A <C>b"…"</C> blob quotes to an <C>Ast.Bytes</C> whose payload is a real <C>Bytes</C> value, so a binary literal is a single first-class node rather than a list of one node per byte:</P>
      <Runnable
        source={`(match (quote b"hi") ((Ast.Bytes b) (Ast.Bytes b)) (_ (quote nil)))`}
      />
      <P>That completes the literal set: integers, floats, strings, booleans, names, and byte strings each reify to their own variant. And because a constructor is type-checked like any other, <Cadenza>(Ast.Bool 5)</Cadenza> is a compile error: the payload must be a <C>Bool</C>, not an integer.</P>
      <Runnable
        source={`(Ast.Bool 5)`}
        expect="error"
      />
      <H2>Building a tree yourself</H2>
      <P>Since the AST is just a sum, you can assemble a form node by node with the constructors: an <C>Ast.List</C> over the operator name and its arguments. This builds the very same tree that <C>(quote (+ 1 2))</C> gives, so the two are equal:</P>
      <Runnable
        source={`(= (quote (+ 1 2)) (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2))))`}
      />
      <P>Constructing by hand is what you reach for when the pieces come from <em>values</em> rather than being written out: a computed argument, a name chosen at run time.</P>
      <Note>The ML surface has lighter sugar for this: a <em>quasiquote</em> is a backtick-brace template, and an <em>unquote</em> (a comma) drops a value into a hole. <C>{"`{ ,x + 10 }"}</C> with <C>x = 2</C> builds the AST for <C>(+ 2 10)</C>, i.e. <C>Ast.List([Ast.Name("+"), Ast.Int(2), Ast.Int(10)])</C>. It's exactly the constructor call above, written as a template. This is construction, not execution: the <C>,x</C> evaluates <em>x</em> to get a value to embed, not the whole form.</Note>
      <H2>Eval: run a tree</H2>
      <P>An AST is inert data until you <C>eval</C> it, which executes the tree as code. Evaluating the quoted <Cadenza>(+ 1 2)</Cadenza> finally gives <C>3</C>:</P>
      <Runnable
        source={`(def (main) (eval (quote (+ 1 2))))`}
      />
      <P>And a tree you <em>built</em> runs the same way. Assemble a call to <C>double</C> on the argument <C>21</C> and eval it: the reconstructed <Cadenza>(double 21)</Cadenza> folds to <C>42</C>. That's the shape of a macro: build a form, then run it.</P>
      <Runnable
        source={`(def (double x) (* 2 x))

(def (main) (eval (Ast.List #list((Ast.Name "double") (Ast.Int 21)))))`}
      />
      <P>A quoted <em>value</em> literal evals back to itself. Unlike a quoted name (which <C>eval</C> resolves) or a quoted call (which it runs), a bare value literal such as an integer, float, boolean, string, or byte string is already its own value, so evaluating its node just hands it back. A quoted byte string reifies to an <C>Ast.Bytes</C>, and <C>(eval (quote b"hi"))</C> is the original <C>b"hi"</C>, closing the loop from source to node to value:</P>
      <Runnable
        source={`(def (main) (= (eval (quote b"hi")) b"hi"))`}
      />
      <Note>A tree can also be serialized: <C>Ast.encode</C> turns an AST into bytes and <C>Ast.decode</C> reads them back (as a <C>Result</C>, since arbitrary bytes might not be a valid tree). Encoding a node and decoding it returns an equal value: the AST survives the round-trip intact, which is how one generation of the compiler hands a program to the next.</Note>
      <Runnable
        source={`(match (Ast.decode (Ast.encode (Ast.Int 7))) ((Ok a) a) ((Err _) (quote nil)))`}
      />
      <P>A byte-string node survives the same round-trip, and it's where the binary codec earns its keep: a <C>b"…"</C> blob rides the wire as a single length-prefixed <C>Ast.Bytes</C> leaf, not one node per byte, so a binary payload stays compact. Encode a blob and decode it back and you get an equal node, so this is <C>true</C>:</P>
      <Runnable
        source={`(match (Ast.decode (Ast.encode (Ast.Bytes b"hi"))) ((Ok a) (= a (Ast.Bytes b"hi"))) ((Err _) false))`}
      />
      <P>There's a text round-trip too: <C>Ast.print</C> renders a tree as source text and <C>Ast.read</C> parses text back into a tree. This one survives at <em>arbitrary precision</em>, an <C>Ast.Int</C> holds a full <C>BigInt</C>, so a 26-digit literal that no 64-bit integer could carry prints its whole decimal and reads back to the exact same node, not a truncated or misread one. Here the round-tripped value equals the original, so this is <C>true</C>:</P>
      <Runnable
        source={`(match
  (Ast.read (Ast.print (Ast.Int (: 99999999999999999999999999 BigInt))))
  ((Ast.Int n) (= n (: 99999999999999999999999999 BigInt)))
  (_ false))`}
      />
      <P><C>Ast.read</C> is careful about what counts as a number: an all-digits token (with an optional leading <C>-</C>) becomes an <C>Ast.Int</C> at any magnitude, while anything else, a name, a decimal point, stays the variant it should be. So <C>Ast.read</C> and <C>Ast.print</C> compose into an identity on trees, the same guarantee as the binary codec above, along the human-readable path.</P>
      <P>The sign rule is worth pinning down, since it mirrors the lexer: a leading <C>-</C> is part of a number, but a leading <C>+</C> is an ordinary operator name, so <C>Ast.read</C> classifies <C>"+5"</C> as an <C>Ast.Name</C>, not an <C>Ast.Int</C>. This checks both, returning <C>true</C> only if <C>"+5"</C> read as a name and <C>"-5"</C> read as an integer:</P>
      <Runnable
        source={`(and
  (match (Ast.read "+5") ((Ast.Name _n) true) (_ false))
  (match (Ast.read "-5") ((Ast.Int _n) true) (_ false)))`}
      />
      <P>A byte-string node prints and reads back the same way. <C>Ast.print</C> renders an <C>Ast.Bytes</C> as its <C>b"…"</C> literal, and <C>Ast.read</C> parses that back to an equal node, so the text round-trip holds for binary blobs just as it does for numbers, this time carrying a byte no text string could, the non-UTF-8 <C>\xff</C>:</P>
      <Runnable
        source={`(= (Ast.read (Ast.print (Ast.Bytes b"\\x00\\xff"))) (Ast.Bytes b"\\x00\\xff"))`}
      />
      <P>It's the <C>b"…"</C> form that marks a byte literal, and <C>Ast.read</C> only treats it as one when the <C>b</C> is immediately followed by the quote: a bare <C>b</C> on its own is an ordinary identifier, so <C>(Ast.read "b")</C> is an <C>Ast.Name</C>, not a zero-length <C>Ast.Bytes</C>. That keeps a variable named <C>b</C> from being mistaken for an empty blob:</P>
      <Runnable
        source={`(match (Ast.read "b") ((Ast.Name _n) true) (_ false))`}
      />
      <H2>Interpolating a computed subtree</H2>
      <P>The point of a template is a hole you fill at run time. Here the argument isn't written out; it's a <em>computed</em> value (<Cadenza>(* 3 7)</Cadenza> = 21) spliced into the tree, which is then evaluated. The built <Cadenza>(+ 21 4)</Cadenza> runs to <C>25</C>:</P>
      <Runnable
        source={`(def (main) (let ((x (* 3 7))) (eval (Ast.List #list((Ast.Name "+") (Ast.Int x) (Ast.Int 4))))))`}
      />
      <P>The <C>(Ast.Int x)</C> lifts the runtime value <C>x</C> into a leaf of the tree. This is interpolation: the shape is fixed, one piece comes from a computation. The ML surface writes exactly this with a quasiquote and an unquote, <C>{"`{ ,x + 4 }"}</C>, the comma marking the spot the value <C>x</C> drops into. Template with holes, holes filled by values.</P>
      <H2>Splicing a whole list of elements</H2>
      <P>Where an unquote drops in <em>one</em> value, an <em>unquote-splicing</em> drops in a whole list of them, each element becoming its own node in the surrounding form. In the conventional surface it's the <C>`,@</C> marker; here we write it <C>(unquote-splicing xs)</C>. The lift is type-directed: a list of integers splices to <C>Ast.Int</C> leaves, floats to <C>Ast.Float</C>, booleans to <C>Ast.Bool</C>, strings to <C>Ast.Str</C>, so the leaf matches the element. Splice a list of floats into a call and the resulting <C>Ast.List</C> has the head plus one node per element: here <C>f</C> and two floats, so <C>3</C> children:</P>
      <Runnable
        source={`(def
  (main)
  (match (quasiquote (f (unquote-splicing #list(1.5 2.5)))) ((Ast.List es) (List.len es)) (_ 0)))`}
      />
      <P>The two floats became two <C>Ast.Float</C> nodes alongside the <C>Ast.Name</C> for <C>f</C>. One rule to remember: the spliced list must be a <em>compile-time-constant</em> list of scalars, since the splice is resolved when the tree is built, not at run time. A list computed from runtime data, or a list whose elements are themselves lists, isn't liftable this way (that's a later capability); the constant-scalar case is what builds a form from a known set of arguments.</P>
      <H2>Matching a tree by shape</H2>
      <P>Construction has a dual: taking a tree apart by matching its shape. Because a compound form is an <C>Ast.List</C>, you match one and reach into its parts. Here we check that the head of a quoted form is the operator <C>+</C>:</P>
      <Runnable
        source={`(def
  (main)
  (match (quote (+ 1 2)) ((Ast.List #list((Ast.Name op) (.. rest))) (= op "+")) (_ false)))`}
      />
      <P>The head's name is <C>"+"</C>, so the check reads <C>true</C>. The pattern binds <C>op</C> to that name and <C>rest</C> to the arguments, so a macro can dispatch on what a form <em>is</em> before rewriting it. There's also a quasiquote <em>pattern</em> for the common shapes: a <C>quasiquote</C> form with <C>unquote</C> holes as a match arm binds the operands directly, the mirror image of building with a quasiquote template. The next section puts it to work.</P>
      <H2>A quasiquote pattern, and an interpreter</H2>
      <P>That quasiquote pattern is the whole game for an interpreter or a macro. In a <C>match</C> arm, a <C>quasiquote</C> form with <C>unquote</C> holes matches a tree of that shape and <em>binds</em> what's in each hole. <C>(unquote x)</C> in a pattern is a binder, the dual of <C>(unquote x)</C> in a template (which embeds a value). Match a runtime <C>Ast</C> against <C>(quasiquote (+ (unquote x) (unquote y)))</C> and you get its two operands as sub-trees, ready to recurse on. Here's a complete little evaluator over an arithmetic <C>Ast</C>: integers evaluate to themselves, and each operator shape recurses into its operands:</P>
      <Runnable
        source={`(def
  (eval-expr (: a Ast))
  (match
    a
    ((Ast.Int n) n)
    ((quasiquote (+ (unquote x) (unquote y))) (+ (eval-expr x) (eval-expr y)))
    ((quasiquote (* (unquote x) (unquote y))) (* (eval-expr x) (eval-expr y)))
    (_ (BigInt.of 0))))

(def (main) (eval-expr (quote (* (+ 1 2) 4))))`}
      />
      <P>The quoted <Cadenza>(* (+ 1 2) 4)</Cadenza> is a real tree, and <C>eval-expr</C> walks it: the outer <C>*</C> arm binds <C>x</C> to the sub-tree <Cadenza>(+ 1 2)</Cadenza> and <C>y</C> to <C>4</C>, recurses into each, and multiplies to <C>(1 + 2) * 4 = 12</C>. This is exactly how the compiler and a macro take apart the code handed to them: the same <C>match</C> you use on any sum type, with a template-shaped pattern for the syntax you care about. Note the arms dispatch on the operator too (the <C>+</C> arm won't match a <C>*</C> form), so distinguishing one operator from another is just two arms.</P>
      <Why tenet="One representation for code, and it's an ordinary value">Many languages bolt on a separate macro system: a second little language, with its own rules, for programs that write programs. Cadenza doesn't. The AST is a sum type declared like any other, so the tools you already have (<C>match</C>, constructors, <C>=</C>, lists) are the whole metaprogramming toolkit. The compiler itself operates on these AST values natively rather than poking at string-tagged reflection, and <C>eval</C> is an optional extra (for macros and the REPL), not something the core depends on. Code as data, with no new machinery to learn.</Why>
      <P>You can watch a program become an <C>Ast</C> value, and see the WebAssembly and Rust it compiles to, in the <AppLink to="/playground"> playground </AppLink> , where code-as-data stops being an abstraction and becomes something you can poke at.</P>
      <H2>Your turn</H2>
      <Exercise
        id="metaprogramming:1"
        prompt={<><C>quote</C> reifies a form without running it, so a quoted compound is an <C>Ast.List</C> of its parts. Fill the arm so this counts the elements of <C>(quote (f 1 2 3))</C>, its operator plus three arguments, giving <C>4</C>.</>}
        starter={`(match (quote (f 1 2 3)) ((Ast.List elems) ?) (_ 0))`}
        solution={`(match (quote (f 1 2 3)) ((Ast.List elems) (List.len elems)) (_ 0))`}
        expected="4"
        hint={<>The <C>Ast.List</C> arm binds <C>elems</C> to the list of child nodes. Its length is <Cadenza>(List.len elems)</Cadenza>: one for the name <C>f</C> and one for each argument, so <C>4</C>.</>}
      />
      <Exercise
        id="metaprogramming:2"
        prompt={<>Build a call and run it. The tree is a <Cadenza>(triple 14)</Cadenza> form assembled by hand, an <C>Ast.List</C> of the name <C>triple</C> and one argument. Fill the argument node so <C>eval</C> runs <Cadenza>(triple 14)</Cadenza> and gives <C>42</C>.</>}
        starter={`(def (triple x) (* 3 x))

(def (main) (eval (Ast.List #list((Ast.Name "triple") ?))))`}
        solution={`(def (triple x) (* 3 x))

(def (main) (eval (Ast.List #list((Ast.Name "triple") (Ast.Int 14)))))`}
        expected="42"
        hint={<>The argument is the integer <C>14</C> as an AST node, an <C>Ast.Int</C>, the same kind <C>(quote 14)</C> would give. So the hole is <Cadenza>(Ast.Int 14)</Cadenza>, and <C>eval</C> then runs <Cadenza>(triple 14)</Cadenza>.</>}
      />
      <Exercise
        id="metaprogramming:3"
        prompt={<><C>unquote-splicing</C> drops each element of a list in as its own node. Splice a three-element list into <C>(g …)</C> and count the children of the resulting <C>Ast.List</C>: the operator <C>g</C> plus the three spliced nodes. Fill the list so the count is <C>4</C>.</>}
        starter={`(match (quasiquote (g (unquote-splicing #list(10 20 ?)))) ((Ast.List es) (List.len es)) (_ 0))`}
        solution={`(match (quasiquote (g (unquote-splicing #list(10 20 30)))) ((Ast.List es) (List.len es)) (_ 0))`}
        expected="4"
        hint={<>Three spliced elements plus the head <C>g</C> is <C>4</C> children, so the list needs three elements, so add a third integer like <C>30</C>. Each becomes its own <C>Ast.Int</C> node next to the <C>Ast.Name</C> for <C>g</C>.</>}
      />
    </article>
  );
}
