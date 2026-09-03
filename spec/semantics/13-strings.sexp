; String operations — witnesses collections-and-text.md. The compiler needs string operations
; for error messages, name encoding (export names → bytes in wasm), and instruction tag dispatch.
; String equality is already witnessed in 01-literals; these cover the remaining operations
; the compiler needs.
(diagnostic-quality)

(case
  "string concatenation"
  (doc "The compiler builds error messages and export names via string concatenation.")
  (input (String.concat "hello" " world"))
  (output (: "hello world" String)))

(case
  "a runtime-built string crosses the boundary as its value form"
  (doc
    "A String built at RUN TIME (a recursion over a live parameter, not a compile-time constant)
           returns to the host as `(: \"…\" String)`. A runtime String is the same UTF-8 byte-rope heap
           value as Bytes (String.concat is a byte-rope concat), so it escapes through the SAME looping
           encoder that a runtime Bytes does — only the value-form frame differs (a String vs a Bytes
           leaf). `rep` appends \"x\" `n` times to \"hi\"; with n=3 → \"hixxx\". Pins that a genuinely
           runtime String (not a folded constant) has a component-boundary representation — it declined
           before as \"String has no component boundary representation\".")
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (rep "hi" 3))
      (export main)))
  (output (: "hixxx" String))
  (live-objects known-leak))

(case
  "a runtime string rope compares equal to its flat twin"
  (doc
    "The equality companion of the rope-escape case above: `(= (rep \"hi\" 3) \"hixxx\")` is TRUE →
           1. `rep` appends \"x\" three times via `String.concat`, building a genuinely RUNTIME String ROPE
           whose content IS \"hixxx\" (it renders as \"hixxx\", byte-len 5). `=` lowers to `value-eq` =
           `champ_eq`, a PHYSICAL-byte compare — and a `String.concat` rope's bytes differ from a flat
           leaf's of identical content, so the rope compared UNEQUAL to the flat literal (returned 0, a
           silent MISCOMPILE) until the compiler CANONICALIZES an owned String operand with `bytes-compact`
           before the compare. A flat runtime string (n=0, no concat) already compared correctly; only a
           rope operand needed compaction. Also fixes a string `match` against a literal (it desugars to
           the same `value-eq` chain). Pins that a runtime rope and its flat twin are `=`-equal.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= (rep "hi" 3) "hixxx") 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a 1000-iteration concat loop builds a 4000-byte rope measured exactly"
  (doc
    "The rope-SCALE face (the rep pins concat ≤3 leaves): 1000 tail-loop concats of \"abcd\" build
           a deep rope whose byte-len is exactly 4000 — no leaf dropped, no double-count at any of the
           999 interior seams, and the build itself doesn't degrade (an O(n) per-concat copy would make
           this quadratic). The stress companion of the small rope-eq pins.")
  (input
    (do
      (def
        (build (: n Int64) (: acc String))
        (if (< n 1) acc (build (- n 1) (String.concat acc "abcd"))))
      (def (main (: n Int64)) (String.byte-len (build n "")))
      (export main)))
  (call main (: 1000 Int64))
  (output (: 4000 Int64)))

(case
  "a String param threaded UNCHANGED to a self-call AND consumed by String.concat each step is retained"
  (doc
    "The simultaneously-live retain for a heap STRING (the String analogue of the threaded-List-arg
           retain): `build(s, n, acc) = build(s, n-1, (String.concat acc s))` passes `s` UNCHANGED to the
           self-call AND consumes it in `String.concat` each step, so `s` must be `dup`'d before the consuming
           concat — else the shared rope is freed while still referenced and the rope walk reads OUT OF BOUNDS
           past a depth threshold (was an OOB memory-access TRAP at n>=4; `cdz check`/`compile` clean, only a
           RUN caught it). ROOT was `is_heap_type` gating the Perceus retain-candidate on `Bytes` but not
           `String`/`Symbol` (a String is a heap rope exactly as Bytes is). `build \"x\" n \"\"` appends one
           byte per step -> byte-len n; n=4 and n=8 both run (the trap threshold and beyond).")
  (input
    (do
      (def
        (build (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (build s (- n 1) (String.concat acc s))))
      (def (main (: n Int64)) (String.byte-len (build "x" n "")))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 8 Int64))
  (output (: 8 Int64))
  (live-objects 0))

(case
  "a runtime String.at result compares by content in a recursive scan"
  (doc
    "The char-by-char lexer idiom: a recursive scan reading each scalar with String.at at a runtime
           index and comparing its content, (= (String.at s i) \"a\"). String.at returns Some of a rope
           slice (an offset into the source), so its content-equality once compared by rope OFFSET and never
           matched a flat twin, making count-a of \"banana\" return 0 (a silent wrong value) and blocking a
           lexer over a runtime string. The fix compacts the fresh slice to an independent flat leaf at the
           producer, so it compares by content everywhere. \"banana\" has three a's, so 3.")
  (input
    (do
      (def (at (: s String) (: i Int64)) (Option.expect (String.at s i) "ok"))
      (def
        (cnt (: s String) (: i Int64) (: acc Int64))
        (if (= i (String.byte-len s)) acc (cnt s (+ i 1) (if (= (at s i) "a") (+ acc 1) acc))))
      (def (main) (cnt "banana" 0 0))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a String.at result then reuse of the source does not double-free"
  (doc
    "The String.at slice-compaction plus borrow-dup fix must not double-free the source: reading a
           char with String.at and then reusing the same string afterwards must run to the correct value.
           The Some-branch dups the borrowed source before the consuming bytes-slice, so the slice owns an
           independent reference and the source survives its later use; a missing dup would free the source
           under the slice (a use-after-free). rep builds an owned runtime rope so String.at reaches its
           real producer path. Char 0 of \"hixxx\" is \"h\" (matches, +1) and byte-len is 5, so 1 + 5 = 6.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (rd (: s String))
        (+ (if (= (Option.expect (String.at s 0) "x") "h") 1 0) (String.byte-len s)))
      (def (main) (rd (rep "hi" 3)))
      (export main)))
  (output (: 6 Int64))
  (live-objects known-leak))

(case
  "a separator JOIN over a runtime parts list handles first-vs-rest and the empty list"
  (doc
    "The join idiom: `join parts sep` prepends the separator to every part EXCEPT the first (a
           first-flag threaded through the fold) — [\"alpha\", rope-\"beta\", \"gamma\"] joined by \",\"
           is 16 bytes (5+1+4+1+5); the EMPTY parts list joins to \"\" (0 bytes, the base case a
           first-flag mishandling breaks). One rope part keeps the fold off the constant path. The
           CSV/path-assembly shape every formatter runs.")
  (input
    (do
      (def
        (join (: parts (List String)) (: sep String) (: acc String) (: first Bool))
        (match
          parts
          (#list() acc)
          (#list(h (.. t))
            (join t sep (if first h (String.concat acc (String.concat sep h))) false))))
      (def
        (main (: n Int64))
        (+
          (*
            100
            (String.byte-len (join #list("alpha" (String.concat "be" "ta") "gamma") "," "" true)))
          (+ (String.byte-len (join #list() "," "" true)) n)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1600 Int64))
  ; tightened back to 0 (#6209 borrowed-env + #6307 fold-reclaim collapsed the #6022 interim re-pin;
  ; breaker census 2026-08-30, value byte-correct).
  (live-objects 0))

(case
  "a balanced-paren scan tracks depth over a runtime string and fails fast on early close"
  (doc
    "The delimiter recognizer every parser front-end runs: a `String.at` scalar walk over a
           RUNTIME rope (nested `String.concat`, nothing folds) with a three-way branch per scalar
           (open/close/other) threading a depth counter and its high-water mark. TWO exits with
           different shapes: end-of-string checks `depth = 0` (balanced → ok=1), while an early close
           drives depth NEGATIVE and must short-circuit IMMEDIATELY (ok=-1, the max-depth watermark
           frozen where the scan died) — a scan that only checks balance at the end accepts \")(\".
           The runtime n swaps a mid-rope scalar between `(` and `)`: n=1 → the first string `(a(b))`
           is balanced at max depth 2 (run → 1·10+2 = 12) and the second `)(` + `()` fails at index 0
           with maxd 0 (run → -10), combined 12·100 + (-10) = 1190; n=0 → the first string becomes
           `(a)b))` — the swapped scalar now CLOSES, so depth dies at the second `)` with maxd 1
           (run → -1·10+1 = -9), giving -9·100 + (-10) = -910. The watermark travelling through the
           failure exit is what distinguishes WHERE each scan died. Encoding per run: ok·10 + maxdepth.")
  (input
    (do
      (def
        (scan (: s String) (: i Int64) (: len Int64) (: depth Int64) (: maxd Int64))
        (if
          (< depth 0)
          #tuple(-1 maxd)
          (if
            (>= i len)
            #tuple((if (= depth 0) 1 0) maxd)
            (match
              (String.at s i)
              ((Some c)
                (if
                  (= c "(")
                  (scan s (+ i 1) len (+ depth 1) (if (> (+ depth 1) maxd) (+ depth 1) maxd))
                  (if
                    (= c ")")
                    (scan s (+ i 1) len (- depth 1) maxd)
                    (scan s (+ i 1) len depth maxd))))
              ((None _u) #tuple(-9 maxd))))))
      (def
        (run (: s String))
        (match (scan s 0 (String.scalar-len s) 0 0) (#tuple(ok maxd) (+ (* ok 10) maxd))))
      (def
        (main (: n Int64))
        (do
          (def open (if (> n 0) "(" ")"))
          (+
            (* (run (String.concat "(a" (String.concat open "b))"))) 100)
            (run (String.concat ")" (String.concat "(" "()"))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1190 Int64))
  (call main (: 0 Int64))
  (output (: -910 Int64))
  ; The recursive `(match (String.at s i) ((Some c) …))` scan's Some shell is now reclaimed per iteration
  ; (v-core-opt owned-single-view MatchSum shell reclaim), so the per-scalar leak (was 15/13) collapses to a
  ; tiny residual (1/1) — the arm only BORROWS `c` (value-eq vs "("/")"), never consumes it, so the back-edge
  ; shell drop is sound. Measured on the debug-counters runtime.
  (live-objects known-leak))

(case
  "MULTI-TYPE bracket matching pushes openers on a list stack and rejects the interleave"
  (doc
    "The depth counter above suffices for ONE bracket type; with three, the counter is provably
           insufficient — the INTERLEAVE `([)]` has balanced counts PER TYPE yet is malformed, and
           only a STACK (a list of one-scalar strings: prepend push, two-arm pop) matching each
           closer against the TOP opener BY TYPE rejects it. Four exits, each a face: the balanced
           mixed nest `([]{})` empties the stack at end-of-string (1); the interleave hits a
           TOP-MISMATCH mid-scan and short-circuits (-1); the unclosed `((` reaches the end with
           LEFTOVERS (0); the EMPTY string is vacuously balanced through the never-grown stack (1).
           Non-bracket scalars would pass through the closer-of miss arm (the lexer discipline
           shared with the classifier pin).")
  (input
    (do
      (def (closer-of (: c String)) (if (= c ")") "(" (if (= c "]") "[" (if (= c "}") "{" ""))))
      (def (is-open (: c String)) (if (= c "(") 1 (if (= c "[") 1 (if (= c "{") 1 0))))
      (def
        (go (: s String) (: i Int64) (: len Int64) (: st (List String)))
        (if
          (>= i len)
          (match st (#list() 1) (_ 0))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= (is-open c) 1)
                (go s (+ i 1) len (List.concat #list(c) st))
                (do
                  (def want (closer-of c))
                  (if
                    (= want "")
                    (go s (+ i 1) len st)
                    (match
                      st
                      (#list() -1)
                      (#list(top (.. rest)) (if (= top want) (go s (+ i 1) len rest) -1)))))))
            ((None _u) 0))))
      (def (bal (: s String)) (go s 0 (String.byte-len s) #list()))
      (def
        (main (: mode Int64))
        (do (def s (if (= mode 1) "([]{})" (if (= mode 2) "([)]" (if (= mode 3) "((" "")))) (bal s)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 4 Int64))
  (output (: 1 Int64))
  ; residual leak SURVIVES #6209+#6307 (breaker census 2026-08-30: per-call 18/8/6/0, values
  ; byte-correct) — NOT the collapsed #6022 fold class; shape = list-stack push/pop of string
  ; openers (named to v-core-opt with the B multi-apply residuals). #5766 tolerate-fewer
  ; auto-passes the eventual collapse.
  (live-objects known-leak))

(case
  "a SPLIT on separator slices fields between hits and round-trips through the pinned JOIN"
  (doc
    "The inverse of the separator-JOIN pin above: scan for `,` hits and slice each FIELD as a
           `String.slice` VIEW between them (fallible — unwrapped via Option.expect on provably
           in-bounds windows), the final field flushed by end-of-string. The fields are then re-JOINED
           with the same separator and compared `=` to the ORIGINAL rope — split/join round-trip law;
           a field boundary off by one (separator absorbed into a field, or a dropped final flush)
           breaks the reassembly bit even when the field count survives. The runtime n swaps a
           mid-rope scalar between `d` and a SECOND separator: n=1 → `ab,cd,ef` splits to 3 fields,
           field[1] = `cd` (2 bytes) → 321; n=0 → `ab,c,,ef` has an EMPTY field between adjacent
           separators — 4 fields, field[1] = `c` (1 byte), and the empty field must survive the
           round-trip (a split that skips empty fields re-joins to `ab,c,ef` ≠ s) → 411.")
  (input
    (do
      (def
        (field (: s String) (: start Int64) (: end Int64))
        (Option.expect (String.slice s start end) "in-bounds field"))
      (def
        (split-go (: s String) (: i Int64) (: len Int64) (: start Int64) (: acc (List String)))
        (if
          (>= i len)
          (List.push acc (field s start i))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c ",")
                (split-go s (+ i 1) len (+ i 1) (List.push acc (field s start i)))
                (split-go s (+ i 1) len start acc)))
            ((None _u) acc))))
      (def (split (: s String)) (split-go s 0 (String.scalar-len s) 0 #list()))
      (def
        (join (: parts (List String)) (: sep String) (: acc String) (: first Bool))
        (match
          parts
          (#list() acc)
          (#list(h (.. t))
            (join t sep (if first h (String.concat acc (String.concat sep h))) false))))
      (def
        (main (: n Int64))
        (do
          (def s (String.concat "ab,c" (String.concat (if (> n 0) "d" ",") ",ef")))
          (def parts (split s))
          (+
            (* (List.len parts) 100)
            (+
              (* (String.byte-len (Option.expect (List.at parts 1) "f1")) 10)
              (if (= (join parts "," "" true) s) 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 321 Int64))
  (call main (: 0 Int64))
  (output (: 411 Int64))
  ; residual leak SURVIVES #6209+#6307 (breaker census 2026-08-30: per-call 6/7, values
  ; byte-correct) — NOT the collapsed #6022 fold class; shape = sliced-field List String
  ; accumulation (named to v-core-opt with the B multi-apply residuals). #5766 tolerate-fewer
  ; auto-passes the eventual collapse.
  (live-objects known-leak))

(case
  "a scalar-indexed split over a MULTIBYTE string bounds its walk by scalar-len, not byte-len"
  (doc
    "The multibyte witness for the scalar-vs-byte loop-bound distinction that the ASCII split case
           above cannot exercise: the SAME scalar-indexed `split-go` (String.at / String.slice at scalar
           offsets) over `\"a,é,b\"` — where `é` is ONE scalar but TWO bytes, so byte-len (6) > scalar-len
           (5). Bounding the walk by `String.scalar-len` visits exactly the 5 scalars and finds all 3
           fields (`a` · `é` · `b`) → 3. A `String.byte-len` bound would drive `i` past the last scalar
           into `String.at`'s `(None _u)` arm on the trailing byte-only iterations, dropping the final
           flush → 2 — precisely the latent bug the scalar-len bound fixes. Directly observable: the
           value flips 3→2 under a byte-len regression.")
  (input
    (do
      (def
        (field (: s String) (: start Int64) (: end Int64))
        (Option.expect (String.slice s start end) "in-bounds field"))
      (def
        (split-go (: s String) (: i Int64) (: len Int64) (: start Int64) (: acc (List String)))
        (if
          (>= i len)
          (List.push acc (field s start i))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c ",")
                (split-go s (+ i 1) len (+ i 1) (List.push acc (field s start i)))
                (split-go s (+ i 1) len start acc)))
            ((None _u) acc))))
      (def (split (: s String)) (split-go s 0 (String.scalar-len s) 0 #list()))
      (def (main) (do (def s (String.concat "a," (String.concat "é" ",b"))) (List.len (split s))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a concat-built rope of 1-, 2-, and 3-byte scalars measures and indexes at every width"
  (doc
    "The divergence pin measures an entry arg once; this rope is built by RUNTIME concat from
           1-/2-/3-byte scalars, both lens read (6/3), and String.at indexes EVERY position — the
           returned scalar's OWN byte-len (1/2/3) proves the walk is scalar-boundary not byte-offset,
           across two seams that don't align with scalar widths. Past-end → 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def r (String.concat (String.concat "a" "é") "日"))
          (def b (String.byte-len r))
          (def s (String.scalar-len r))
          (def w (match (String.at r n) ((Some c) (String.byte-len c)) ((None _u) 0)))
          (+ (* b 100) (+ (* s 10) w))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 631 Int64))
  (call main (: 1 Int64))
  (output (: 632 Int64))
  (call main (: 2 Int64))
  (output (: 633 Int64))
  (call main (: 3 Int64))
  (output (: 630 Int64))
  (live-objects known-leak))

(case
  "a scalar slice spanning a width TRANSITION carries its multibyte content exactly"
  (doc
    "The scalar-slice pins extract ASCII from multibyte strings; this slice's CONTENT is
           itself multibyte spanning a 1→2→3-byte transition (slice(1,3), 2 scalars 5 bytes,
           content-verified by = — a byte-indexed slice or an off-by-one at either boundary yields
           wrong BYTES, not just wrong length). Adjacent windows pin both alignments.")
  (input
    (do
      (def
        (main (: st Int64) (: en Int64))
        (do
          (def r (String.concat (String.concat "a" "é") (String.concat "日" "x")))
          (match
            (String.slice r st en)
            ((Some sl) (+ (* (String.byte-len sl) 10) (if (= sl "é日") 1 0)))
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64) (: 3 Int64))
  (output (: 51 Int64))
  (call main (: 0 Int64) (: 2 Int64))
  (output (: 30 Int64))
  (call main (: 2 Int64) (: 4 Int64))
  (output (: 40 Int64))
  (live-objects known-leak))

(case
  "a runtime multibyte rope matches a MULTIBYTE string-literal arm by content"
  (doc
    "The string-match desugar pins use ASCII literals; this arm is multibyte over a runtime
           concat rope, with a SHARED-PREFIX sibling (both arms start with the 2-byte scalar, so the
           probe chain must compare past the shared multibyte prefix). Fall-through face.")
  (input
    (do
      (def (classify (: s String)) (match s ("é日" 1) ("éx" 2) (_ 0)))
      (def
        (main (: mode Int64))
        (classify (String.concat "é" (if (= mode 1) "日" (if (= mode 2) "x" "y")))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64)))

(case
  "a PARSE-INT walks scalars, decodes digits by comparison, and handles a leading minus"
  (doc
    "The atoi idiom over the scalar walk: each `String.at` scalar is decoded to its digit VALUE
           by a comparison chain (one-scalar STRINGS compare by `=` — there is no Char arithmetic on
           this path), accumulated base-10, with two stop conditions layered — end-of-string AND
           first non-digit (the trailing-garbage stop, which must return the prefix parsed SO FAR,
           not reject) — plus a leading `-` dispatched BEFORE the digit walk and applied by negation
           AFTER it. The runtime n swaps the suffix between pure digits and garbage-embedded: n=1 →
           `142` parses fully (1421 with the minus-check bit); n=0 → `17x9` stops at the `x` and
           yields 17 — the 9 after the garbage must NOT resume (171). Both calls also parse the rope
           `-35` (built from three concat pieces) to −35, pinning sign+rope together. A digit chain
           decoded by scalar-order arithmetic instead of comparison, or a stop that consumes the
           garbage scalar, drifts by a digit.")
  (input
    (do
      (def
        (digit (: c String))
        (if
          (= c "0")
          0
          (if
            (= c "1")
            1
            (if
              (= c "2")
              2
              (if
                (= c "3")
                3
                (if
                  (= c "4")
                  4
                  (if
                    (= c "5")
                    5
                    (if (= c "6") 6 (if (= c "7") 7 (if (= c "8") 8 (if (= c "9") 9 -1)))))))))))
      (def
        (go (: s String) (: i Int64) (: len Int64) (: acc Int64))
        (if
          (>= i len)
          acc
          (match
            (String.at s i)
            ((Some c) (do (def d (digit c)) (if (< d 0) acc (go s (+ i 1) len (+ (* acc 10) d)))))
            ((None _u) acc))))
      (def
        (parse (: s String))
        (match
          (String.at s 0)
          ((Some c)
            (if (= c "-") (- 0 (go s 1 (String.scalar-len s) 0)) (go s 0 (String.scalar-len s) 0)))
          ((None _u) 0)))
      (def
        (main (: n Int64))
        (do
          (def suffix (if (> n 0) "42" "7x9"))
          (+
            (* (parse (String.concat "1" suffix)) 10)
            (if (= (parse (String.concat "-" (String.concat "3" "5"))) -35) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1421 Int64))
  (call main (: 0 Int64))
  (output (: 171 Int64))
  (live-objects known-leak))

(case
  "ONE-EDIT-APART checks strings for exactly one substitution or one insertion by length case"
  (doc
    "The edit-distance-≤1 special case, dispatched by LENGTH DIFFERENCE: equal lengths →
           count substitutions in lockstep and demand EXACTLY one (the identical-strings face is 0
           edits — must answer NO, the ≥-vs-= discipline); off-by-one lengths → walk in lockstep to
           the first mismatch, then `rest-eq` resumes with the LONG side's index shifted one ahead
           (the skip — a resume without the shift, or shifting the short side, both fail); a gap of
           two+ → 0 without any walk. Both insertion sites are faces: `cat`/`cats` appends at the
           END (the mismatch never fires; skip-match runs off the short end to its ≥-slen exit) and
           `cat`/`at` deletes at the FRONT (the mismatch fires at index 0 — the earliest shift).
           `cat`/`dog` = 3 substitutions (0), `ab`/`abcd` = gap 2 (0). Faces: 1/1/1/0/0/0.")
  (input
    (do
      (def
        (count-diffs (: a String) (: b String) (: i Int64) (: len Int64) (: d Int64))
        (if
          (>= i len)
          d
          (match
            (String.at a i)
            ((Some ca)
              (match
                (String.at b i)
                ((Some cb) (count-diffs a b (+ i 1) len (if (= ca cb) d (+ d 1))))
                ((None _u) d)))
            ((None _u) d))))
      (def
        (rest-eq (: short String) (: long String) (: j Int64) (: slen Int64))
        (if
          (>= j slen)
          1
          (if
            (=
              (Option.expect (String.at short j) "s2")
              (Option.expect (String.at long (+ j 1)) "l2"))
            (rest-eq short long (+ j 1) slen)
            0)))
      (def
        (skip-match (: short String) (: long String) (: i Int64) (: slen Int64))
        (if
          (>= i slen)
          1
          (do
            (def cs (Option.expect (String.at short i) "s"))
            (def cl (Option.expect (String.at long i) "l"))
            (if (= cs cl) (skip-match short long (+ i 1) slen) (rest-eq short long i slen)))))
      (def
        (one-apart (: a String) (: b String))
        (do
          (def la (String.scalar-len a))
          (def lb (String.scalar-len b))
          (if
            (= la lb)
            (if (= (count-diffs a b 0 la 0) 1) 1 0)
            (if (= (+ la 1) lb) (skip-match a b 0 la) (if (= la (+ lb 1)) (skip-match b a 0 lb) 0)))))
      (def
        (main (: mode Int64))
        (if
          (= mode 1)
          (one-apart "cat" "cut")
          (if
            (= mode 2)
            (one-apart "cat" "cats")
            (if
              (= mode 3)
              (one-apart "cat" "at")
              (if
                (= mode 4)
                (one-apart "cat" "cat")
                (if (= mode 5) (one-apart "cat" "dog") (one-apart "ab" "abcd")))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "STRING ROTATION check searches the needle inside the doubled haystack at equal length"
  (doc
    "The corpus's first SUBSTRING SEARCH (every other dual-string pin walks in lockstep from
           index 0; this runs the naive two-level walk — an outer OFFSET scan over the haystack, an
           inner match-at comparing needle-length scalars at each offset) — applied to the doubled-
           haystack rotation trick: b is a rotation of a ⟺ b occurs in a++a, at equal length. The
           equal-length gate runs FIRST; the EMPTY pair is guarded BEFORE the concat (a find with an
           empty needle vacuously matches at every offset — the guard makes the answer principled).
           Faces: `abcde`/`cdeab` a true rotation found at offset 2 (1); `abced` — an ANAGRAM that
           is NOT a rotation (same scalars, wrong order — the inner match fails at every offset, 0);
           the identity rotation (1); equal-length different content (0); the empty pair (1).")
  (input
    (do
      (def
        (match-at (: hay String) (: nee String) (: off Int64) (: j Int64) (: ln Int64))
        (if
          (>= j ln)
          1
          (if
            (= (Option.expect (String.at hay (+ off j)) "h") (Option.expect (String.at nee j) "n"))
            (match-at hay nee off (+ j 1) ln)
            0)))
      (def
        (find (: hay String) (: nee String) (: off Int64) (: lh Int64) (: ln Int64))
        (if
          (> (+ off ln) lh)
          0
          (if (= (match-at hay nee off 0 ln) 1) 1 (find hay nee (+ off 1) lh ln))))
      (def
        (is-rot (: a String) (: b String))
        (do
          (def la (String.scalar-len a))
          (def lb (String.scalar-len b))
          (if
            (= la lb)
            (if (= la 0) 1 (do (def aa (String.concat a a)) (find aa b 0 (* la 2) lb)))
            0)))
      (def
        (main (: mode Int64))
        (if
          (= mode 1)
          (is-rot "abcde" "cdeab")
          (if
            (= mode 2)
            (is-rot "abcde" "abced")
            (if (= mode 3) (is-rot "ab" "ab") (if (= mode 4) (is-rot "a" "b") (is-rot "" ""))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "EDIT DISTANCE rolls a two-row Levenshtein table over string scalars"
  (doc
    "The general form of the one-edit check above (that one special-cases distance ≤ 1; this
           computes the FULL Levenshtein): a TWO-ROW rolling table — `prev` is the completed row,
           `cur` grows left-to-right, and each cell takes min3 of DELETE (prev[j]+1), INSERT
           (cur[j−1]+1 — a read from the row UNDER CONSTRUCTION), and SUBSTITUTE (prev[j−1] + cost).
           The cur[j−1] read is the sharp edge: it must see the cell appended MOMENTS ago in the
           same row walk (an off-by-one reads the seed value and inflates every distance). Row 0
           seeds 0..lb (transforming from the empty prefix = j inserts); each row starts with i
           (i deletes). Faces: kitten→sitting = 3 (the textbook mix of sub+sub+insert); identical
           → 0 (the cost-0 diagonal rides the whole table); EMPTY a → lb (the seed row IS the
           answer, no row loop runs); flaw→lawn = 2 (delete f, append n — edits at BOTH ends).")
  (input
    (do
      (def (at0 (: xs (List Int64)) (: i Int64)) (Option.expect (List.at xs i) "in-bounds"))
      (def
        (min3 (: a Int64) (: b Int64) (: c Int64))
        (if (< a b) (if (< a c) a c) (if (< b c) b c)))
      (def
        (seed (: j Int64) (: n Int64) (: acc (List Int64)))
        (if (> j n) acc (seed (+ j 1) n (List.push acc j))))
      (def
        (row-go
          (: b String)
          (: j Int64)
          (: lb Int64)
          (: ca String)
          (: prev (List Int64))
          (: cur (List Int64)))
        (if
          (> j lb)
          cur
          (do
            (def cb (Option.expect (String.at b (- j 1)) "cb"))
            (def cost (if (= ca cb) 0 1))
            (def best (min3 (+ (at0 prev j) 1) (+ (at0 cur (- j 1)) 1) (+ (at0 prev (- j 1)) cost)))
            (row-go b (+ j 1) lb ca prev (List.push cur best)))))
      (def
        (rows (: a String) (: b String) (: i Int64) (: la Int64) (: lb Int64) (: prev (List Int64)))
        (if
          (> i la)
          prev
          (do
            (def ca (Option.expect (String.at a (- i 1)) "ca"))
            (rows a b (+ i 1) la lb (row-go b 1 lb ca prev #list(i))))))
      (def
        (lev (: a String) (: b String))
        (do
          (def la (String.scalar-len a))
          (def lb (String.scalar-len b))
          (def final (rows a b 1 la lb (seed 0 lb #list())))
          (at0 final lb)))
      (def
        (main (: mode Int64))
        (if
          (= mode 1)
          (lev "kitten" "sitting")
          (if (= mode 2) (lev "abc" "abc") (if (= mode 3) (lev "" "abc") (lev "flaw" "lawn")))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 4 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a LONGEST-COMMON-PREFIX walks two strings in scalar lockstep to the first mismatch"
  (doc
    "The dual-string lockstep walk (the parse/scan pins above walk ONE string; this reads the
           SAME index of TWO strings per step): advance while `String.at a i` equals `String.at b i`,
           stop at the first mismatch or either end — three exit conditions, each pinned. One operand
           is a runtime ROPE (concat-built), the other flat, so per-step the walk crosses a seam on
           one side but not the other (an index cursor shared between the two reads that advances
           by seam-local offsets drifts on exactly one operand). n=1 → `overlap` vs `overlord` share
           `overl` (5); self-LCP is the full length (7 — the all-equal exit via length, never a
           mismatch); LCP vs the empty string is 0 (the immediate-boundary exit). 570 combined.
           n=0 → `overt` vs `overlord` share `over` (4), self 5, empty 0 → 450.")
  (input
    (do
      (def
        (lcp-go (: a String) (: b String) (: i Int64) (: la Int64) (: lb Int64))
        (if
          (>= i la)
          i
          (if
            (>= i lb)
            i
            (match
              (String.at a i)
              ((Some ca)
                (match
                  (String.at b i)
                  ((Some cb) (if (= ca cb) (lcp-go a b (+ i 1) la lb) i))
                  ((None _u) i)))
              ((None _u) i)))))
      (def
        (lcp (: a String) (: b String))
        (lcp-go a b 0 (String.scalar-len a) (String.scalar-len b)))
      (def
        (main (: n Int64))
        (do
          (def a (String.concat "over" (if (> n 0) "lap" "t")))
          (def b "overlord")
          (+ (* (lcp a b) 100) (+ (* (lcp a a) 10) (lcp a "")))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 570 Int64))
  (call main (: 0 Int64))
  (output (: 450 Int64))
  (live-objects known-leak))

(case
  "a STRING run-length grouping counts adjacent equal scalars across a rope seam"
  (doc
    "The adjacent-grouping walk: thread (current scalar, count) through a String.at scan,
           closing a run — appending its count as a digit — exactly when the scalar CHANGES, with the
           FINAL run flushed by end-of-string (a grouping that only closes on change drops the last
           run). The scalar equality is one-scalar-STRING `=`. The rope seam sits MID-RUN: the
           string is `aab` + swap + `bcc`, so at n=1 (`aabbbcc`) the bbb run SPANS the seam — its
           three scalars come from three different concat pieces, and a scan whose current-scalar
           state resets at a seam splits the run (232 becomes 2112-ish). Runs 2,3,2 → 232. At n=0
           (`aababcc`) the same seam position ALTERNATES instead — runs 2,1,1,1,2 → 21112 — so the
           two calls distinguish seam-state-reset from alternation-miscounting.")
  (input
    (do
      (def
        (go (: s String) (: i Int64) (: len Int64) (: cur String) (: cnt Int64) (: acc Int64))
        (if
          (>= i len)
          (+ (* acc 10) cnt)
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c cur)
                (go s (+ i 1) len cur (+ cnt 1) acc)
                (go s (+ i 1) len c 1 (+ (* acc 10) cnt))))
            ((None _u) acc))))
      (def
        (runs (: s String))
        (match (String.at s 0) ((Some c0) (go s 1 (String.scalar-len s) c0 1 0)) ((None _u) 0)))
      (def
        (main (: n Int64))
        (do (def s (String.concat "aab" (String.concat (if (> n 0) "b" "a") "bcc"))) (runs s)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 232 Int64))
  (call main (: 0 Int64))
  (output (: 21112 Int64))
  (live-objects known-leak))

(case
  "LOOK-AND-SAY iterates run-length description over its own prior output"
  (doc
    "The self-referential upgrade of the run-length grouping above: each iteration DESCRIBES
           the previous string's runs as `<count><digit>` pairs — the OUTPUT of one round is the
           INPUT of the next, so any per-round error (a dropped final flush, a count rendered as the
           wrong scalar via the digit-table read) COMPOUNDS through the iterations rather than
           surfacing once. From the seed `1`: 1 → 11 → 21 → 1211 → 111221 → 312211 (the classic
           sequence; k=5's `111221` input has runs 3,2,2 — three ones, two twos, two ones — the
           first round where a MULTI-run count appears). Verified by byte-length + full-string
           equality at k = 0 (the untouched seed), 1, 3, 5. Encoding: len·10 + string-match bit
           (11 / 21 / 41 / 61).")
  (input
    (do
      (def (digit-str (: v Int64)) (Option.expect (String.at "0123456789" v) "single digit"))
      (def
        (las-go (: s String) (: i Int64) (: len Int64) (: cur String) (: cnt Int64) (: acc String))
        (if
          (>= i len)
          (String.concat acc (String.concat (digit-str cnt) cur))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c cur)
                (las-go s (+ i 1) len cur (+ cnt 1) acc)
                (las-go s (+ i 1) len c 1 (String.concat acc (String.concat (digit-str cnt) cur)))))
            ((None _u) acc))))
      (def
        (las (: s String))
        (match (String.at s 0) ((Some c0) (las-go s 1 (String.byte-len s) c0 1 "")) ((None _u) s)))
      (def (iter (: s String) (: k Int64)) (if (= k 0) s (iter (las s) (- k 1))))
      (def
        (main (: k Int64))
        (do
          (def s (iter "1" k))
          (+
            (* (String.byte-len s) 10)
            (if (= s (if (= k 5) "312211" (if (= k 3) "1211" (if (= k 1) "11" "1")))) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 3 Int64))
  (output (: 41 Int64))
  (call main (: 5 Int64))
  (output (: 61 Int64))
  ; per-call live-objects (B2 re-baseline, coord v-corpus-harness); pre-existing (verified pre/post recent emit)
  (live-objects known-leak))

(case
  "a PALINDROME two-pointer walk closes from both ends of a runtime rope"
  (doc
    "Every pinned scalar walk moves ONE cursor forward; this closes TWO from opposite ends,
           reading `String.at s lo` and `String.at s hi` each step over a runtime rope whose seams
           sit at ASYMMETRIC positions relative to the two cursors (lo crosses a seam on a different
           step than hi). Three termination faces in one probe: the EVEN-length crossover (`abba` —
           lo passes hi without ever equaling it, so a `= hi` stop test spins), the ODD-length center
           self-skip (`abcba` — lo = hi lands on the middle scalar, which needs NO check), and the
           single-scalar immediate-true (`\"a\"` — hi starts AT lo (both 0), so the >= stop fires
           before any read). The runtime n flips the second scalar: n=1 →
           all three palindromic (111); n=0 → `axba` fails at step two, the mismatch exit (0·100 →
           11).")
  (input
    (do
      (def
        (pal (: s String) (: lo Int64) (: hi Int64))
        (if
          (>= lo hi)
          1
          (match
            (String.at s lo)
            ((Some a)
              (match
                (String.at s hi)
                ((Some b) (if (= a b) (pal s (+ lo 1) (- hi 1)) 0))
                ((None _u) -1)))
            ((None _u) -1))))
      (def (check (: s String)) (pal s 0 (- (String.byte-len s) 1)))
      (def
        (main (: n Int64))
        (do
          (def second (if (> n 0) "b" "x"))
          (def w (String.concat "a" (String.concat second "ba")))
          (+
            (* (check w) 100)
            (+ (* (check (String.concat "ab" (String.concat "c" "ba"))) 10) (check "a")))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 111 Int64))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (live-objects known-leak))

(case
  "a THREE-WAY scalar classifier splits vowels, consonants, and other by range and membership"
  (doc
    "The lexer's character-class dispatch: each scalar routes through a MEMBERSHIP chain
           (vowel — five one-scalar equality tests) then a RANGE test (`\"a\" <= c <= \"z\"` — ORDER
           comparison on one-scalar strings, the byte-lexicographic `<=` the sort pins established,
           here driving control flow per scalar), falling through to `other`. Three counters thread
           the walk; exactly one increments per scalar (the classification is a partition — counts
           must sum to the length). The RANGE test runs only on the vowel-chain MISS, so the vowel
           set shadows the range (a classifier testing range first counts vowels as consonants).
           Faces: `hello world` → 3 vowels, 7 consonants, 1 other — the space falls through BOTH
           tests (371); `aeiou` → all vowels, range never fires (500); `xyz 123` → 0 vowels, 3
           consonants at the range TOP edge (x,y,z), 4 other — digits are BELOW `\"a\"` in byte
           order (34). Encoding: v·100 + k·10 + o.")
  (input
    (do
      (def
        (is-vowel (: c String))
        (if (= c "a") 1 (if (= c "e") 1 (if (= c "i") 1 (if (= c "o") 1 (if (= c "u") 1 0))))))
      (def (is-lower (: c String)) (if (>= c "a") (if (<= c "z") 1 0) 0))
      (def
        (go (: s String) (: i Int64) (: len Int64) (: v Int64) (: k Int64) (: o Int64))
        (if
          (>= i len)
          #tuple(v k o)
          (match
            (String.at s i)
            ((Some c)
              (if
                (= (is-vowel c) 1)
                (go s (+ i 1) len (+ v 1) k o)
                (if
                  (= (is-lower c) 1)
                  (go s (+ i 1) len v (+ k 1) o)
                  (go s (+ i 1) len v k (+ o 1)))))
            ((None _u) #tuple(v k o)))))
      (def (classify (: s String)) (go s 0 (String.byte-len s) 0 0 0))
      (def
        (main (: mode Int64))
        (do
          (def s (if (= mode 1) "hello world" (if (= mode 2) "aeiou" "xyz 123")))
          (match (classify s) (#tuple(v k o) (+ (* v 100) (+ (* k 10) o))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 371 Int64))
  (call main (: 2 Int64))
  (output (: 500 Int64))
  (call main (: 3 Int64))
  (output (: 34 Int64))
  (live-objects known-leak))

(case
  "string REVERSE walks scalars back-to-front and anti-commutes with concatenation"
  (doc
    "The DESCENDING index walk (every other pinned scan ascends; this starts at byte-len − 1
           and steps toward 0, terminating on `< 0` — the off-by-one lives at BOTH ends: starting at
           byte-len reads one past, stopping at 0 skips the first scalar). Certified by two algebra
           laws: the ANTI-HOMOMORPHISM `rev(a ++ b) = rev(b) ++ rev(a)` — the operand order SWAPS
           across the concat, which a law-blind implementation (e.g. one reversing chunks but not
           their order) breaks — and the INVOLUTION `rev(rev s) = s`. n=1 → `abcde` reversed is 5
           scalars with both laws holding (511); n=0 makes `a` EMPTY — rev of the empty string is
           the left identity in the swapped composition, len 2 (211). Encoding: len·100 +
           anti-hom·10 + involution.")
  (input
    (do
      (def
        (srev-go (: s String) (: i Int64) (: acc String))
        (if
          (< i 0)
          acc
          (match
            (String.at s i)
            ((Some c) (srev-go s (- i 1) (String.concat acc c)))
            ((None _u) acc))))
      (def (srev (: s String)) (srev-go s (- (String.byte-len s) 1) ""))
      (def
        (main (: n Int64))
        (do
          (def a (if (> n 0) "abc" ""))
          (def b "de")
          (def whole (srev (String.concat a b)))
          (+
            (* (String.byte-len whole) 100)
            (+
              (* (if (= whole (String.concat (srev b) (srev a))) 1 0) 10)
              (if (= (srev (srev (String.concat a b))) (String.concat a b)) 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 511 Int64))
  (call main (: 0 Int64))
  (output (: 211 Int64))
  (live-objects known-leak))

(case
  "a WORD COUNT threads an in-word flag through a scalar scan, counting entries not characters"
  (doc
    "The wc/lexer-mode idiom — the first Bool MODE FLAG threaded through a string walk (every
           other pinned scan threads numeric state): a space clears `inw`, a non-space sets it, and
           the count increments only on the ENTRY transition (non-space seen while `inw` is false) —
           counting words, not characters. The runtime scalar swaps a mid-string position between
           space and letter, flipping word-split against word-merge (`ab cd  ef ` = 3 words vs
           `abxcd  ef ` = 2 — the double space and trailing space must not inflate either). Three
           degenerate faces ride along: the EMPTY string (0), ALL-spaces (the flag never trips — 0),
           and NO-spaces (`solo` — exactly one entry, 1). Encoding: main·1000 + empty·100 +
           spaces·10 + solo. n=1 → 3001; n=0 → 2001.")
  (input
    (do
      (def
        (wc-go (: s String) (: i Int64) (: len Int64) (: inw Bool) (: n Int64))
        (if
          (>= i len)
          n
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c " ")
                (wc-go s (+ i 1) len false n)
                (wc-go s (+ i 1) len true (if inw n (+ n 1)))))
            ((None _u) n))))
      (def (wc (: s String)) (wc-go s 0 (String.scalar-len s) false 0))
      (def
        (main (: n Int64))
        (do
          (def mid (if (> n 0) " " "x"))
          (def s (String.concat "ab" (String.concat mid "cd  ef ")))
          (+ (* (wc s) 1000) (+ (* (wc "") 100) (+ (* (wc "   ") 10) (wc "solo"))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3001 Int64))
  (call main (: 0 Int64))
  (output (: 2001 Int64))
  ; The recursive `(match (String.at s i) ((Some c) …))` scan's Some shell is now reclaimed per iteration
  ; (v-core-opt owned-single-view MatchSum shell reclaim; the arm only borrows `c` via `(= c " ")`, never
  ; consumes it) → was 35, now 1. Measured on the debug-counters runtime.
  (live-objects known-leak))

(case
  "a ROMAN NUMERAL renderer walks a value-symbol table greedily with subtractive pairs"
  (doc
    "The table-driven greedy renderer: a (value, symbol) assoc list — INCLUDING the subtractive
           pairs 900/CM, 400/CD, 90/XC, 40/XL, 9/IX, 4/IV interleaved in strictly descending value
           order — is walked once, and `emit` REPEATS each symbol while the remainder covers its
           value (an inner loop threading (remainder, acc) as a tuple through the outer table walk;
           two nested loop levels over different structures — the table spine and the numeric
           remainder). Skipping a subtractive entry renders the LONG form (VIIII for IX); reordering
           two entries misfires the greed. Faces: 1994 → MCMXCIV (three subtractives in one render,
           7 scalars → 71); 9 → IX (a PURE subtractive, the single-entry render → 21); 40 → XL (21);
           3888 → MMMDCCCLXXXVIII — the LONGEST standard numeral (every additive run at its 3-repeat
           maximum, 15 scalars, no subtractive fires → 151). Verified by byte-length + full-string
           equality.")
  (input
    (do
      (def
        (emit (: n Int64) (: v Int64) (: s String) (: acc String))
        (if (< n v) #tuple(n acc) (emit (- n v) v s (String.concat acc s))))
      (def
        (walk (: n Int64) (: vs (List (Tuple Int64 String))) (: acc String))
        (match
          vs
          (#list() acc)
          (#list(#tuple(v s) (.. t)) (match (emit n v s acc) (#tuple(n2 acc2) (walk n2 t acc2))))))
      (def
        (roman (: n Int64))
        (walk
          n
          #list(#tuple(1000 "M")
            #tuple(900 "CM")
            #tuple(500 "D")
            #tuple(400 "CD")
            #tuple(100 "C")
            #tuple(90 "XC")
            #tuple(50 "L")
            #tuple(40 "XL")
            #tuple(10 "X")
            #tuple(9 "IX")
            #tuple(5 "V")
            #tuple(4 "IV")
            #tuple(1 "I"))
          ""))
      (def
        (main (: n Int64))
        (do
          (def s (roman n))
          (+
            (* (String.byte-len s) 10)
            (if
              (= s (if (= n 1994) "MCMXCIV" (if (= n 9) "IX" (if (= n 40) "XL" "MMMDCCCLXXXVIII"))))
              1
              0))))
      (export main)))
  (call main (: 1994 Int64))
  (output (: 71 Int64))
  (call main (: 9 Int64))
  (output (: 21 Int64))
  (call main (: 40 Int64))
  (output (: 21 Int64))
  (call main (: 3888 Int64))
  (output (: 151 Int64))
  (live-objects known-leak))

(case
  "a ROMAN DECODER subtracts on lookahead and round-trips through the pinned renderer"
  (doc
    "The decoder's mechanism is entirely different from the renderer above: a per-scalar value
           chain plus ONE-scalar LOOKAHEAD — peek at i+1, and a smaller value BEFORE a larger one
           NEGATES instead of adds (the IV/IX rule read from the string side; the lookahead at the
           final scalar must see 'nothing follows' and add). The strongest face composes both:
           `fromroman(roman n) = n` runs encode then decode in ONE program, the two implementations
           certifying each other with the subtractive subtlety live on both sides. Faces: 1994
           (three subtractive pairs to DETECT via lookahead → 19941); 9 (pure subtractive, the
           2-scalar string → 91); 3888 (fifteen scalars, ZERO subtractives — the lookahead must
           never fire → 38881); 3 (the trivial additive run III → 31). Encoding: decoded·10 +
           round-trip bit.")
  (input
    (do
      (def
        (val (: c String))
        (if
          (= c "I")
          1
          (if
            (= c "V")
            5
            (if
              (= c "X")
              10
              (if (= c "L") 50 (if (= c "C") 100 (if (= c "D") 500 (if (= c "M") 1000 0))))))))
      (def
        (dec-go (: s String) (: i Int64) (: len Int64) (: acc Int64))
        (if
          (>= i len)
          acc
          (match
            (String.at s i)
            ((Some c)
              (do
                (def v (val c))
                (def
                  nxt
                  (if
                    (< (+ i 1) len)
                    (match (String.at s (+ i 1)) ((Some c2) (val c2)) ((None _u) 0))
                    0))
                (dec-go s (+ i 1) len (if (> nxt v) (- acc v) (+ acc v)))))
            ((None _u) acc))))
      (def (fromroman (: s String)) (dec-go s 0 (String.scalar-len s) 0))
      (def
        (emit (: n Int64) (: v Int64) (: s String) (: acc String))
        (if (< n v) #tuple(n acc) (emit (- n v) v s (String.concat acc s))))
      (def
        (walk (: n Int64) (: vs (List (Tuple Int64 String))) (: acc String))
        (match
          vs
          (#list() acc)
          (#list(#tuple(v s) (.. t)) (match (emit n v s acc) (#tuple(n2 acc2) (walk n2 t acc2))))))
      (def
        (roman (: n Int64))
        (walk
          n
          #list(#tuple(1000 "M")
            #tuple(900 "CM")
            #tuple(500 "D")
            #tuple(400 "CD")
            #tuple(100 "C")
            #tuple(90 "XC")
            #tuple(50 "L")
            #tuple(40 "XL")
            #tuple(10 "X")
            #tuple(9 "IX")
            #tuple(5 "V")
            #tuple(4 "IV")
            #tuple(1 "I"))
          ""))
      (def (main (: n Int64)) (+ (* (fromroman (roman n)) 10) (if (= (fromroman (roman n)) n) 1 0)))
      (export main)))
  (call main (: 1994 Int64))
  (output (: 19941 Int64))
  (call main (: 9 Int64))
  (output (: 91 Int64))
  (call main (: 3888 Int64))
  (output (: 38881 Int64))
  (call main (: 3 Int64))
  (output (: 31 Int64))
  (live-objects known-leak))

(case
  "a CAESAR cipher shifts alphabet positions modulo 26, involutes at ROT13, passes non-letters"
  (doc
    "The substitution cipher over the alphabet-table pair: each scalar's position is found by
           the index-of SCAN (the decoder direction), shifted `(p + k) mod 26`, and read back by the
           POSITIONAL String.at (the encoder direction) — one walk drives both table directions per
           scalar. Non-letters PASS THROUGH untouched (the space in `hello world` — a cipher that
           shifts the miss sentinel -1 corrupts it). Three certificate layers per call: the encoded
           string against its expected literal, the DECODE round-trip `rot (rot msg k) (26−k) = msg`,
           and the same round-trip over `abz` (whose `z` wraps past the alphabet end at any positive
           k — the mod-26 face). k=13 → ROT13 `uryyb jbeyq` (the involution shift); k=1 → `ifmmp
           xpsme` (minimal shift, `z`→`a` wrap); k=26 → the IDENTITY (`(p+26) mod 26 = p` — encoded
           equals the original, and the round-trip shift 26−26 = 0 must also be identity). All → 111.")
  (input
    (do
      (def
        (find-at (: alpha String) (: c String) (: i Int64) (: len Int64))
        (if
          (>= i len)
          -1
          (match
            (String.at alpha i)
            ((Some d) (if (= d c) i (find-at alpha c (+ i 1) len)))
            ((None _u) -1))))
      (def
        (rot-go (: s String) (: k Int64) (: i Int64) (: len Int64) (: acc String))
        (if
          (>= i len)
          acc
          (match
            (String.at s i)
            ((Some c)
              (do
                (def p (find-at "abcdefghijklmnopqrstuvwxyz" c 0 26))
                (def
                  out
                  (if
                    (< p 0)
                    c
                    (Option.expect
                      (String.at "abcdefghijklmnopqrstuvwxyz" (% (+ p k) 26))
                      "in range")))
                (rot-go s k (+ i 1) len (String.concat acc out))))
            ((None _u) acc))))
      (def (rot (: s String) (: k Int64)) (rot-go s k 0 (String.byte-len s) ""))
      (def
        (main (: k Int64))
        (do
          (def msg "hello world")
          (def enc (rot msg k))
          (+
            (*
              (if (= enc (if (= k 13) "uryyb jbeyq" (if (= k 1) "ifmmp xpsme" "hello world"))) 1 0)
              100)
            (+
              (* (if (= (rot enc (- 26 k)) msg) 1 0) 10)
              (if (= (rot (rot "abz" k) (- 26 k)) "abz") 1 0)))))
      (export main)))
  (call main (: 13 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 111 Int64))
  (call main (: 26 Int64))
  (output (: 111 Int64))
  (live-objects known-leak))

(case
  "a deep rope indexes by scalar at both extremes"
  (doc
    "Scalar addressing through ~500 concat seams: `String.at` reads index 0 (the single \"A\" head
           leaf) and index n·2 (the LAST scalar, deep in the right spine) of a 1001-scalar rope — 10·1+1
           = 11. A walk that lost count crossing seams, or that flattened per-read (correct but masking
           the seam walk), answers the same; the wrong-index failure is a different character at either
           extreme.")
  (input
    (do
      (def
        (build (: n Int64) (: acc String))
        (if (< n 1) acc (build (- n 1) (String.concat acc "xy"))))
      (def
        (main (: n Int64))
        (let
          ((s (String.concat "A" (build n ""))))
          (+
            (* 10 (match (String.at s 0) ((Some c) (if (= c "A") 1 0)) ((None u) -1)))
            (match (String.at s (* n 2)) ((Some c) (if (= c "y") 1 0)) ((None u) -1)))))
      (export main)))
  (call main (: 500 Int64))
  (output (: 11 Int64))
  (live-objects known-leak))

(case
  "two deep ropes built in OPPOSITE orders compare equal by content"
  (doc
    "The tree-shape-independence face: `build-l` appends (`concat acc leaf` — a left-leaning spine)
           and `build-r` prepends (`concat leaf acc` — right-leaning); at n=500 both denote the same 1000
           bytes in maximally different tree shapes. `=` must compare CONTENT across the shapes (a
           structural tree compare, or a flatten applied to only one side, reports unequal). The
           build-order companion of the rope-vs-flat-twin pin.")
  (input
    (do
      (def
        (build-l (: n Int64) (: acc String))
        (if (< n 1) acc (build-l (- n 1) (String.concat acc "ab"))))
      (def
        (build-r (: n Int64) (: acc String))
        (if (< n 1) acc (build-r (- n 1) (String.concat "ab" acc))))
      (def (main (: n Int64)) (if (= (build-l n "") (build-r n "")) 1 0))
      (export main)))
  (call main (: 500 Int64))
  (output (: 1 Int64)))

(case
  "dropping a rope built OVER a survivor must not free the shared child node"
  (doc
    "The ROPE member of the generation-sharing reclaim family (the CHAMP/RRB members live in
           05-compound): r2 = (String.concat r1 suffix) holds r1 as its concat node's LEFT CHILD.
           mode 1 keeps the CHILD r1 and drops the parent — freeing the concat node must not cascade
           into the shared child, and the survivor is verified by FULL content equality (a stale
           length can pass while freed leaf bytes corrupt the content). mode 2 keeps the parent past
           the child's last direct use. Encodes byte-len·10 + content-ok: mode 1 → \"ababab\"(6) →
           61, mode 2 → \"abababzzzz\"(10) → 101.")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def r1 (rep "ab" 3 ""))
          (def r2 (String.concat r1 (rep "z" 4 "")))
          (def keep (if (= mode 1) r1 r2))
          (def ok (if (= keep (if (= mode 1) "ababab" "abababzzzz")) 1 0))
          (+ (* (String.byte-len keep) 10) ok)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 61 Int64))
  (call main (: 2 Int64))
  (output (: 101 Int64))
  ; mode 1 (keep=r1, the shared CHILD) reclaims fully clean (0); mode 2 (keep=r2, the parent rope
  ; that outlives the child's last direct use) leaks ONE cell — the pre-existing keep-not-dropped
  ; reclaim gap (the if-joined dual-borrowed rope is never dropped after its byte-len/value-eq uses),
  ; a SEPARATE tracked follow-up under v-memory-safety. The cross-arm retain fix (mark_binder_dups
  ; If-arm predicate (a)) ELIMINATED the mode-2 double-free UAF (was a wasm `unreachable` trap on the
  ; debug-counters runtime); value-correct + no trap, with this residual pinned per-call.
  (live-objects known-leak))

(case
  "a runtime string rope matches a string-literal arm"
  (doc
    "The `match` sibling: `(match (rep \"hi\" 3) (\"hixxx\" 1) (_ 0))` takes the \"hixxx\" arm → 1. A
           string `match` desugars to a chain of `(= scrutinee <literal>)` value-eq tests, so it hit the
           same rope-vs-flat physical-byte miscompile (took the `_` arm, returning 0) until the rope operand
           is compacted before the compare. Confirms the fix covers the match-desugar path, not only `=`.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (match (rep "hi" 3) ("hixxx" 1) (_ 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime string orders by content-lexicographic byte order"
  (doc
    "Runtime String ORDERING (`<`): `(rep \"app\" …)` builds genuinely-runtime strings whose content is
           compared content-lexicographically. `\"apple\" < \"applf\"` — same first four bytes, then `e`(0x65)
           < `f`(0x66) → the first differing byte decides → true → 1. A runtime String is a UTF-8 byte leaf
           and its blessed total order is the content-lexicographic byte order (core-semantics.md #Compound
           Ordering Is Lexicographic); the seed walks both leaves' bytes (`bytes-get`/`bytes-len`) rather than
           declining. The concat forces a runtime value (a bare literal would const-fold the compare).")
  (input
    (do
      (def (mk (: s String)) (String.concat s ""))
      (def (main) (if (< (mk "apple") (mk "applf")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "runtime string ordering makes a proper prefix less than its extension"
  (doc
    "The prefix rule: `\"app\" < \"apple\"` → true → 1. The two share every byte of the shorter string,
           so no byte differs within the common length; a list/string that is a PROPER PREFIX of another
           compares LESS than the longer one (core-semantics.md #Compound Ordering Is Lexicographic —
           shorter-is-less on a common prefix). Pins the length tiebreak of the runtime byte-lexicographic
           walk, distinct from the first-differing-byte case above.")
  (input
    (do
      (def (mk (: s String)) (String.concat s ""))
      (def (main) (if (< (mk "app") (mk "apple")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "runtime string ordering compares bytes UNSIGNED (a multi-byte scalar exceeds ASCII)"
  (doc
    "Content-lexicographic order compares the UTF-8 bytes as UNSIGNED values: `\"café\" > \"cafz\"`
           because at byte 3 the `é` encoding's lead byte `0xC3` (195) is GREATER than `z` = `0x7A` (122) —
           so `(< \"café\" \"cafz\")` is FALSE → 0. A SIGNED byte compare would read `0xC3` as −61 < 122 and
           wrongly answer true; pins that the walk is unsigned (which for well-formed UTF-8 makes the byte
           order agree with the Unicode scalar order). The multi-byte companion of the ASCII cases above.")
  (input
    (do
      (def (mk (: s String)) (String.concat s ""))
      (def (main) (if (< (mk "café") (mk "cafz")) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "runtime string ordering surfaces all four relational operators"
  (doc
    "`<=`, `>`, `>=` over runtime strings agree with `<` and with each other (one total order surfaced
           through every boolean operator, core-semantics.md #A Total Order Is Observed Through A Three-Way
           Comparison). Packs four checks: `\"app\" <= \"apple\"` (1), `\"apple\" <= \"apple\"` (1, reflexive),
           `\"banana\" > \"apple\"` (1), `\"apple\" >= \"apple\"` (1) → 1000+100+10+1 = 1111. Pins that the
           three-way order derives `<=`/`>`/`>=` consistently, including the equal case on each.")
  (input
    (do
      (def (le (: a String) (: b String)) (if (<= (String.concat a "") (String.concat b "")) 1 0))
      (def (gt (: a String) (: b String)) (if (> (String.concat a "") (String.concat b "")) 1 0))
      (def (ge (: a String) (: b String)) (if (>= (String.concat a "") (String.concat b "")) 1 0))
      (def
        (main)
        (+
          (* 1000 (le "app" "apple"))
          (+ (* 100 (le "apple" "apple")) (+ (* 10 (gt "banana" "apple")) (ge "apple" "apple")))))
      (export main)))
  (output (: 1111 Int64)))

(case
  "a runtime string rope compared through a borrowed operand"
  (doc
    "The BORROWED-operand remainder of the rope-eq fix. The earlier fix compacted only an OWNED
           String operand (a fresh `String.concat` result); a genuine rope reaching `=` through a
           BORROWED operand — a `Map.lookup`-stored value, a `SumPayload`-extracted payload, or a
           runtime-rope param — was compared by its UNFLATTENED header bytes and silently returned the
           WRONG answer. Here `rep \"hi\" 3` = \"hixxx\" (a runtime rope) is stored as a map value, looked
           up, and compared as the BORROWED `Some` payload `s` INSIDE the arm (`(= s \"hixxx\")`). Because
           `bytes-compact` is refcount-NEUTRAL (it flattens the node IN PLACE and returns the same handle,
           unobservable even when shared), the compiler now compacts EVERY String operand — owned OR
           borrowed — and drops only the owned ones, so the borrowed rope payload compares by content.
           Expected: the arm fires and returns 1 (was 0 — a champ_eq physical-byte miss).")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (f (: mp (Map String String)) (: k String))
        (match (Map.lookup mp k) ((Some s) (if (= s "hixxx") 1 0)) ((None) -1)))
      (def (main) (f (Map.insert (Map.empty) "y" (rep "hi" 3)) "y"))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime-rope do-def that escapes an if-select in a match arm survives (multi-use heap keep, not a UAF)"
  (doc
    "breaker FINDING #20 (wasm UAF, rust/rust-async always correct) — v-inference's lower_let
           keep-analysis fix. Inside a `(match (String.at s 0) ((Some c0) …))` arm, a value do-def
           `out = comp-go …` builds a RUNTIME rope; `out` is then used MULTIPLE times (three
           `String.scalar-len`/select reads) and one use ESCAPES the arm via an if-SELECT `(if (< …) out s)`
           feeding `String.byte-len`. The wasm emit freed the escaping heap do-def after the conditional
           borrow, so `byte-len` read a DANGLING/doubled handle → 20 (neither out=8 nor s=11), while
           rust/rust-async returned the correct 8 — a differential wasm wrong-value miscompile. The fix
           routes a multi-use + escaping heap value do-def through lower_let's keep-analysis (kept as a
           Core::Let binder → LocalRef → Borrowed, one retain/drop), so the escaping arm returns a LIVE
           handle. mode 1 (\"aabcccccaaa\" → run-length \"a2b1c5a3\", 8 bytes, shorter than 11) → 8; mode 2
           (\"abc\" → \"a1b1c1\" longer, so s wins) → 3. Both backends. (The comp-go walk is bound by
           scalar-len per the authoring convention — ASCII inputs, behavior-neutral.)")
  (input
    (do
      (def (digit-str (: v Int64)) (Option.expect (String.at "0123456789" v) "d"))
      (def
        (comp-go (: s String) (: i Int64) (: len Int64) (: cur String) (: cnt Int64) (: acc String))
        (if
          (>= i len)
          (String.concat acc (String.concat cur (digit-str cnt)))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c cur)
                (comp-go s (+ i 1) len cur (+ cnt 1) acc)
                (comp-go s (+ i 1) len c 1 (String.concat acc (String.concat cur (digit-str cnt))))))
            ((None _u) acc))))
      (def
        (main (: mode Int64))
        (do
          (def s (if (= mode 1) "aabcccccaaa" "abc"))
          (match
            (String.at s 0)
            ((Some c0)
              (do
                (def out (comp-go s 1 (String.scalar-len s) c0 1 ""))
                (String.byte-len (if (< (String.byte-len out) (String.byte-len s)) out s))))
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 8 Int64))
  (call main (: 2 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "STRING COMPRESSION emits char-count pairs but keeps the original unless strictly shorter"
  (doc
    "The full-algorithm sibling of the FINDING #20 minimal pin above: the same comp-go run-length
           walk, but wrapped in the complete keep-original-unless-STRICTLY-shorter policy and checked
           for CONTENT, not just length (result = byte-len·10 + an equality bit, so a right-length
           wrong-content compression cannot pass). Adds the face the minimal pin lacks: the TIE —
           mode 3 \"aa\" compresses to \"a2\", EQUAL length 2, not strictly shorter, so the original is
           kept (21 = 2·10 + content-match 1). mode 1 \"aabcccccaaa\" → \"a2b1c5a3\" (8 < 11, compression
           wins) → 81; mode 2 \"abc\" → \"a1b1c1\" (6 > 3, original kept) → 31. The escaping do-def `r`
           is again multi-use (measured AND compared), re-crossing the #20 keep-analysis path with the
           select hoisted OUT of the match arm into main's spine. Walk bound by scalar-len per the
           authoring convention; the shortness comparison is byte-len, which IS the policy's semantics.")
  (input
    (do
      (def (digit-str (: v Int64)) (Option.expect (String.at "0123456789" v) "single digit"))
      (def
        (comp-go (: s String) (: i Int64) (: len Int64) (: cur String) (: cnt Int64) (: acc String))
        (if
          (>= i len)
          (String.concat acc (String.concat cur (digit-str cnt)))
          (match
            (String.at s i)
            ((Some c)
              (if
                (= c cur)
                (comp-go s (+ i 1) len cur (+ cnt 1) acc)
                (comp-go s (+ i 1) len c 1 (String.concat acc (String.concat cur (digit-str cnt))))))
            ((None _u) acc))))
      (def
        (compress (: s String))
        (match
          (String.at s 0)
          ((Some c0)
            (do
              (def out (comp-go s 1 (String.scalar-len s) c0 1 ""))
              (if (< (String.byte-len out) (String.byte-len s)) out s)))
          ((None _u) s)))
      (def
        (main (: mode Int64))
        (do
          (def s (if (= mode 1) "aabcccccaaa" (if (= mode 2) "abc" "aa")))
          (def r (compress s))
          (+ (* (String.byte-len r) 10) (if (= r (if (= mode 1) "a2b1c5a3" s)) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 81 Int64))
  (call main (: 2 Int64))
  (output (: 31 Int64))
  (call main (: 3 Int64))
  (output (: 21 Int64))
  (live-objects known-leak))

(case
  "a rope threads TWO chained if-selects, each operand multi-use, and every length stays live"
  (doc
    "The composition face of the FINDING #20 family above: not one escaping select but a CHAIN —
           pick1 selects between two runtime ropes r1/r2 (each already multi-use: measured in the
           condition AND selectable), then pick1 is ITSELF multi-use (measured in pick2's condition
           AND selectable against base). The keep-analysis must keep every operand live across BOTH
           select joins; a placement bug on either link frees a handle the next link still reads.
           The result folds ALL the lengths (pick2·100 + r1·10 + r2), so a stale handle at any depth
           shifts a distinct digit position. mode 1: base \"ab\", r1=\"ababab\"(6), r2=\"aaaaa\"(5) →
           pick1=r2(5), 5<2 false → pick2=r2 → 565. mode 2: base \"xyz\", r1=\"xyz\"(3), r2=\"xxxxx\"(5) →
           pick1=r1(3), 3<3 false → pick2=r1 → 335. mode 3: r1=\"\"(0) — an EMPTY rope through the
           chain — pick1=r1(0), 0<3 true → pick2=base(3) → 305.")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def base (if (= mode 1) "ab" "xyz"))
          (def n1 (if (= mode 1) 3 (if (= mode 2) 1 0)))
          (def c0 (Option.expect (String.at base 0) "c"))
          (def r1 (rep base n1 ""))
          (def r2 (rep c0 5 ""))
          (def pick1 (if (< (String.byte-len r1) (String.byte-len r2)) r1 r2))
          (def pick2 (if (< (String.byte-len pick1) (String.byte-len base)) base pick1))
          (+ (* (String.byte-len pick2) 100) (+ (* (String.byte-len r1) 10) (String.byte-len r2)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 565 Int64))
  (call main (: 2 Int64))
  (output (: 335 Int64))
  (call main (: 3 Int64))
  (output (: 305 Int64))
  (live-objects known-leak))

(case
  "a multi-use rope escapes a select through a Some shell and its twin survives the unwrap"
  (doc
    "The sum-shell face of the #20 keep-analysis family: the if-select's escaping result is
           immediately WRAPPED in a Some constructor, matched back out as `pick`, and only THEN
           measured — the escape crosses a heap-compound boundary, not just a select join. Meanwhile
           the NON-picked twin `r` is re-measured AFTER the shell round-trip (the +r term), so both
           the winner (through the shell) and the loser (past its last select use) must stay live.
           mode 1: r=\"ababab\"(6) vs s=\"q\"(1), 6<1 false → pick=r → 66. mode 2: r=\"xyz\"(3) vs
           s(8), 3<8 true → pick=s → 83 (the picked-OTHER case: r's liveness now rests solely on
           the trailing re-measure). mode 3: r EMPTY(0) vs s(2) → pick=s → 20 (empty rope through
           the shell's sibling read).")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def r (if (= mode 1) (rep "ab" 3 "") (if (= mode 2) (rep "xyz" 1 "") (rep "a" 0 ""))))
          (def s (if (= mode 1) "q" (if (= mode 2) "qqqqqqqq" "qq")))
          (def shell (Some (if (< (String.byte-len r) (String.byte-len s)) s r)))
          (match
            shell
            ((Some pick) (+ (* (String.byte-len pick) 10) (String.byte-len r)))
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 66 Int64))
  (call main (: 2 Int64))
  (output (: 83 Int64))
  (call main (: 3 Int64))
  (output (: 20 Int64))
  (live-objects known-leak))

(case
  "a helper returns the shorter of two borrowed ropes and both caller handles stay live"
  (doc
    "The CALL-BOUNDARY face of the #20 escape family: the select moves into a helper —
           `shorter` returns one of its two borrowed String params, i.e. the callee returns an
           ALIAS of a parameter and ownership transfer at the return edge must not free the
           unreturned twin nor double-count the returned one. The caller then re-measures BOTH
           source ropes after the call, so each is read past the point the callee last touched it.
           mode 1: r1=\"ababab\"(6) vs r2=\"zz\"(2), 6<2 false → p=r2 → 2·100+6·10+2 = 262. mode 2:
           r1=\"x\"(1) vs r2=\"zzzz\"(4), 1<4 true → p=r1 → 1·100+1·10+4 = 114. mode 3: r1 EMPTY
           wins the select (0<1) — an empty rope crosses the return edge → 0+0+1 = 1.")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def (shorter (: a String) (: b String)) (if (< (String.byte-len a) (String.byte-len b)) a b))
      (def
        (main (: mode Int64))
        (do
          (def r1 (if (= mode 1) (rep "ab" 3 "") (if (= mode 2) (rep "x" 1 "") (rep "a" 0 ""))))
          (def r2 (rep "z" (if (= mode 1) 2 (if (= mode 2) 4 1)) ""))
          (def p (shorter r1 r2))
          (+ (* (String.byte-len p) 100) (+ (* (String.byte-len r1) 10) (String.byte-len r2)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 262 Int64))
  (call main (: 2 Int64))
  (output (: 114 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "two ropes pack CROSSWISE into a tuple by a runtime select and both unpack live"
  (doc
    "The BOTH-ESCAPE face of the #20 family: previous pins escape ONE rope past a select and
           keep the loser live via a trailing read; here BOTH ropes escape TOGETHER, packed into a
           tuple whose FIELD ORDER is chosen by a runtime select — (tuple r1 r2) if r1 is shorter,
           else (tuple r2 r1). Neither branch drops either rope, but each branch consumes them in a
           DIFFERENT slot order, so per-branch retain/transfer bookkeeping must agree at the join
           before the tuple is unpacked and both fields plus the original r1 binding are measured.
           mode 1: r1(6)/r2(2), 6<2 false → (r2 r1) → a=2,b=6 → 266. mode 2: r1(1)/r2(4) →
           (r1 r2) → a=1,b=4 → 141. mode 3: r1 EMPTY, 0<1 → (r1 r2) → a=0,b=1,r1=0 → 10.")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def r1 (if (= mode 1) (rep "ab" 3 "") (if (= mode 2) (rep "x" 1 "") (rep "a" 0 ""))))
          (def r2 (rep "z" (if (= mode 1) 2 (if (= mode 2) 4 1)) ""))
          (def
            packed
            (if (< (String.byte-len r1) (String.byte-len r2)) #tuple(r1 r2) #tuple(r2 r1)))
          (match
            packed
            (#tuple(a b)
              (+ (* (String.byte-len a) 100) (+ (* (String.byte-len b) 10) (String.byte-len r1)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 266 Int64))
  (call main (: 2 Int64))
  (output (: 141 Int64))
  (call main (: 3 Int64))
  (output (: 10 Int64))
  (live-objects 0))

(case
  "a string equality on an inlined match operand"
  (doc
    "A `String ==` whose operand is an INLINED function returning a `match` — `(= (f …) \"z\")` where
           `f` returns `(match (Map.lookup m k) ((Some s) s) ((None) \"?\"))`, β-reduced into the `=`
           operand (the default for a non-recursive call). The two arms DISAGREE on ownership: the `Some`
           arm returns a BORROWED payload (`s`, a `sum-payload` read off the owned Option), the `None` arm
           returns an OWNED constant (\"?\"). The value-eq operand-ownership analysis had no `match` arm and
           fell through to a DECLINE (`borrowing op operand has an ownership this backend cannot yet
           prove`), blocking any program that compares a returned map/variant payload once the wrapper
           inlines — the shape a compiler-in-Cadenza substitution pass hits. It now classifies a match
           operand by the JOIN of its arm bodies (owned iff every arm is owned, else borrowed — the
           leak-safe value), so the mixed join here is borrowed: no drop follows, and the leak matches the
           standalone-function path. The lookup finds \"z\", so `= \"z\"` is true → 1.")
  (input
    (do
      (def
        (f (: m (Map String String)) (: k String))
        (match (Map.lookup m k) ((Some s) s) ((None) "?")))
      (def (main) (if (= (f (Map.insert (Map.empty) "y" "z") "y") "z") 1 0))
      (export main)))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a mixed-ownership inlined-match operand leaves the source map intact across repeated compares"
  (doc
    "The DROP-CORRUPTION guard for the match-operand ownership JOIN above: the join classifies the
           mixed Some/None operand as BORROWED, so NO post-compare drop is emitted. Had it mis-joined to
           OWNED, the drop after the first compare would free the "
    z"
    payload
    the
    map
    still
    owns
    —
    and
    this
    case
    would
    see
    it:
    the
    SAME
    let-bound
    map
    is
    consulted
    TWICE
    through
    the
    inlined
    match
    (each compare true → 1 + 1)
    and
    then
    read
    structurally
    ((quasiquote Map.len`) → 1)
    (unquote so)
    3.0
    A
    use-after-free
    on
    the
    payload
    flips
    the
    second
    compare
    (→ 2)
    or
    corrupts
    the
    size
    read.
    Pins
    the
    leak-safe
    side
    of
    the
    join
    with
    the
    source
    value
    still
    live.")
  (input
    (do
      (def
        (f (: m (Map String String)) (: k String))
        (match (Map.lookup m k) ((Some s) s) ((None) "?")))
      (def
        (main)
        (let
          ((m (Map.insert (Map.empty) "y" "z")))
          (+ (+ (if (= (f m "y") "z") 1 0) (if (= (f m "y") "z") 1 0)) (Map.len m))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "an all-owned-arms match operand compares correctly through the ownership join"
  (doc
    "The OWNED side of the match-operand join: BOTH arms build a fresh `String.concat` result, so
           the join is Owned (every arm owned) and the post-compare drop is correct — each arm's rope is
           a temporary nothing else holds. k = 0 → \"zx\" = \"zx\" → 1; k = 1 → \"zy\" ≠ \"zx\" → 0 (a
           genuine value test, both directions). With the mixed-arm case above this pins both join
           outcomes: all-owned → owned (dropped), mixed → borrowed (left to the owner).")
  (input
    (do
      (def (pick (: k Int64)) (match k (0 (String.concat "z" "x")) (_ (String.concat "z" "y"))))
      (def (main (: k Int64)) (if (= (pick k) "zx") 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a THREE-arm match joins a borrowed alias, an owned fresh rope, and a const literal"
  (doc
    "The two ownership-join pins above cover two arms (mixed → borrowed, all-owned → owned);
           this joins THREE distinct ownership classes at one match: arm 1 returns src ITSELF (a
           borrowed alias of a live binding), arm 2 an OWNED fresh rope (concat borrows src into a
           new temporary), arm 3 a CONST literal (baked handle, no refcount at all). After the join
           BOTH r and src are measured — src's last-use analysis differs per arm (aliased out /
           borrowed into the concat / never touched), so the join's reconciliation can't be a
           single static class without a retain. mode 1 → r=src(6) → 66; mode 2 → r=\"ababab\"+
           \"zz\"(8) → 86; mode 3 → r=\"kk\"(2) → 26 (src(6) still live in every mode).")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def src (rep "ab" 3 ""))
          (def r (match mode (1 src) (2 (String.concat src "zz")) (_ "kk")))
          (+ (* (String.byte-len r) 10) (String.byte-len src))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 66 Int64))
  (call main (: 2 Int64))
  (output (: 86 Int64))
  (call main (: 3 Int64))
  (output (: 26 Int64))
  (live-objects known-leak))

(case
  "a runtime string rope map key is found by its flat twin"
  (doc
    "The MAP-KEY companion of the rope-eq cases above: a map keyed by a runtime String ROPE
           `(rep \"hi\" 3)` = \"hixxx\" looked up with the flat literal \"hixxx\" MUST find 42. The map-key
           path hashes+compares with `champ_hash`/`champ_eq` — a PHYSICAL-byte compare, the SAME contract
           `value-eq` uses — so a rope key (different bytes than its flat twin) landed in a different slot
           and `Map.lookup` returned `None` (→ -1), a silent MISCOMPILE. The value-eq rope fix compacted
           only the `=`/match operand; this pins the twin KEY path: the compiler now `bytes-compact`s an
           owned String key at every Map/Set champ site (insert/lookup/remove, set-of/insert/contains/
           remove), so a rope key and its flat twin hash+compare equal. Expected: 42.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup (Map.insert (Map.empty) (rep "hi" 3) 42) "hixxx")
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a flat string map key is found by a runtime rope twin"
  (doc
    "The symmetric direction: insert under the FLAT literal key \"hixxx\", look up with the runtime
           ROPE `(rep \"hi\" 3)` = \"hixxx\" → 42. Compaction canonicalizes the LOOKUP key too (not only the
           inserted key), so the rope lookup key hashes into the flat key's slot. Confirms the fix covers
           both champ sites — the inserted key AND the lookup/borrow key — not just one direction.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup (Map.insert (Map.empty) "hixxx" 42) (rep "hi" 3))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a runtime String.slice probing a flat-keyed map hits by content"
  (doc
    "The SLICE-view member of the champ-key family (the rope cases above canonicalize CONCAT
           ropes): the lookup key is a runtime-START `String.slice` of a larger string — a VIEW whose
           content equals the stored flat key \"bc\". a=1 windows \"bc\" → hit (42); a=0 windows \"ab\" →
           clean miss (-1). The slice view must content-canonicalize at the champ site exactly as a
           rope does (a hash over the view node or the parent's bytes would miss the hit case). The
           String twin of the Bytes slice-view champ-key contract — pinned here because the STRING side
           already works; the Bytes side is finding #16.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty "bc" 42)))
          (match
            (String.slice "abcd" a (+ a 2))
            ((Some s) (match (Map.lookup m s) ((Some v) v) ((None u) -1)))
            ((None u) -2))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a runtime String.slice STORED as a map key is found by a flat probe"
  (doc
    "The stored-key direction: the slice view goes INTO the map as the key and the flat literal
           probes it — insert-site canonicalization, the champ-site twin of the probe-side case above.
           a=1 stores the \"bc\" window → the flat \"bc\" probe finds 42.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (String.slice "abcd" a (+ a 2))
          ((Some s)
            (match (Map.lookup (Map.insert Map.empty s 42) "bc") ((Some v) v) ((None u) -1)))
          ((None u) -2)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (live-objects 0))

; The `String.slice s start end` BOUNDARY contract (third arg is the END, half-open [start,end)): it is
; TOTAL and FALLIBLE — it returns `(Option String)`, `Some` for an in-range window and `None` for ANY
; out-of-range request, never a trap and never a silently CLAMPED view (which would let a read run past the
; string). In range: `end` may equal the length (the full-suffix boundary) and `start == end` is the empty
; window (Some ""). Out of range → None: an `end` past the length, an inverted `start > end`, or a NEGATIVE
; start/end. The None arm is the memory-safety pin — a clamp-instead-of-None reimplementation would return a
; truncated Some and mask an out-of-bounds request. (Source is a LITERAL here so the value semantics are
; measured without the param-string slice-retention husk fenced separately just below.) Verified breaker
; probe ss: fold == runtime == hop.
(case
  "String.slice returns the substring for an in-range half-open window"
  (doc
    "`String.slice \"hello\" a b` over runtime bounds returns Some of the [a,b) window; byte-len of the
           result witnesses it. `[1,3)` = \"el\" (2); `[0,5)` = the full \"hello\" (5, end==len is in range);
           `[2,2)` = \"\" the empty window (0). Pins the half-open in-range semantics, end-inclusive-of-length.")
  (input
    (do
      (def (sl (: a Int64) (: b Int64))
        (match (String.slice "hello" a b) ((Some x) (String.byte-len x)) ((None) -1)))
      (export sl)))
  (call sl (: 1 Int64) (: 3 Int64))
  (output (: 2 Int64))
  (call sl (: 0 Int64) (: 5 Int64))
  (output (: 5 Int64))
  (call sl (: 2 Int64) (: 2 Int64))
  (output (: 0 Int64)))

(case
  "String.slice returns None for any out-of-range window (total, no trap, no clamp)"
  (doc
    "The totality/safety pin: an out-of-range `String.slice` returns None (rendered -1 here), NOT a
           trap and NOT a clamped Some. `[0,99)` end past the length → None; `[3,1)` inverted start>end →
           None; `[-1,3)` negative start → None; `[0,-1)` negative end → None. A clamp-to-bounds reimpl
           would return a truncated Some and hide the out-of-bounds request — this pins the None instead.")
  (input
    (do
      (def (sl (: a Int64) (: b Int64))
        (match (String.slice "hello" a b) ((Some x) (String.byte-len x)) ((None) -1)))
      (export sl)))
  (call sl (: 0 Int64) (: 99 Int64))
  (output (: -1 Int64))
  (call sl (: 3 Int64) (: 1 Int64))
  (output (: -1 Int64))
  (call sl (: -1 Int64) (: 3 Int64))
  (output (: -1 Int64))
  (call sl (: 0 Int64) (: -1 Int64))
  (output (: -1 Int64)))

; ssx3 (breaker): slicing a boundary PARAMETER String leaks ONE husk — the slice pins/DUPs its borrowed
; source string (+1) without a matching drop, so a live-cell remains after the result is consumed. It is
; exactly 1 regardless of window (interior [1,3) or full [0,5)) or slice COUNT (two slices → still 1), and
; the SOURCE, not the view, is the husk. Controls prove the trigger is (param-string AND slice) together:
; a param string consumed by byte-len/at with NO slice reclaims 0 (SCRATCH param bytelen); slicing a
; LITERAL source (the value cases above) reclaims 0; slicing a runtime CONCAT-built source reclaims 0.
; Filed to v-memory-safety. Fenced known-leak so a reclaim fix auto-flips it. (An escaping param slice is a
; separate CDZ0900 decline — not covered here.) NIX-CONFIRMED REAL: flipping this pin to (live-objects 0)
; reds nix corpus-13-strings (got 1) — so unlike the md2/md3 Map-husk case (in-process over-count, nix=0),
; the slice-retention husk is counted faithfully by BOTH the in-process gate and nix. The known-leak pin
; stands; the reclaim is genuinely needed.
(case
  "slicing a boundary parameter String retains one husk (param-source slice-retention leak)"
  (doc
    "`String.slice s a b` where `s` is a boundary PARAMETER String leaves 1 live cell after the sliced
           result is scalar-consumed by byte-len — the slice dups the borrowed source without a drop. The
           value is correct (`\"hello\"[1,3)` byte-len = 2); only the heap balance leaks. Fenced known-leak
           pending the v-memory-safety reclaim; the literal-source value cases above stay at 0.")
  (input
    (do
      (def (sl (: s String) (: a Int64) (: b Int64))
        (match (String.slice s a b) ((Some x) (String.byte-len x)) ((None) -1)))
      (export sl)))
  (call sl (: "hello" String) (: 1 Int64) (: 3 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a trie of 40 rope-built String keys resolves content descent at depth"
  (doc
    "The String-key rows above run on 1-2 keys (slice views, flat probes); this pins a POPULATED
           trie of ROPE-BUILT keys: 40 keys `\"key\" + i dashes` — shared prefix, distinct lengths,
           every key a runtime concat rope — fill a multi-level trie, and a rope-built probe resolves
           entry 25 (10·40 + 25 = 425). The key hash canonicalizes each rope's content at insert AND
           probe among 40 neighbors; a hash over rope structure (leaf boundaries) rather than content
           would miss its own entries.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "-") (- n 1))))
      (def
        (fill (: i Int64) (: m (Map String Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m (String.concat "key" (rep "" i)) i))))
      (def
        (main (: n Int64))
        (do
          (def m (fill n Map.empty))
          (+
            (* 10 (Map.len m))
            (match (Map.lookup m (String.concat "key" (rep "" 25))) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 40 Int64))
  (output (: 425 Int64)))

(case
  "a rope-built String key probes the trie entry stored under its FLAT twin at depth"
  (doc
    "The cross-construction face AT DEPTH (the 1-entry stored-slice-key case above pins the
           2-entry version): among 30 other rope-built entries, a probe assembled as
           `(String.concat \"flat-\" (rep \"\" 3))` — a rope ending in three y's — hits the entry
           stored under the FLAT literal `\"flat-yyy\"` (777).
           Rope-vs-flat canonical equivalence holds through a populated descent — insert-site and
           probe-site canonicalization agree regardless of which construction each side used.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "y") (- n 1))))
      (def
        (fill (: i Int64) (: m (Map String Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m (rep "p" i) i))))
      (def
        (main (: n Int64))
        (do
          (def m (Map.insert (fill n Map.empty) "flat-yyy" 777))
          (match (Map.lookup m (String.concat "flat-" (rep "" 3))) ((Some v) v) ((None _u) -1))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 777 Int64)))

(case
  "a String slice OF a slice over a MULTIBYTE concat rope composes scalar offsets"
  (doc
    "The view-of-a-view face for STRING with multibyte scalars: the parent is a concat rope
           `\"aé∀\" + \"bçd\"` (é = 2 bytes, ∀ = 3 bytes, so scalar index ≠ byte offset), the outer
           slice (1,5) crosses the seam, and the inner slice (1,3) re-offsets WITHIN that view —
           `\"∀b\"`, scalar-len 2 → 201. Composing two view layers must translate scalar offsets
           through both, landing on byte boundaries that don't align with scalar indices; a composition
           that re-based against bytes (or only one layer) would split a scalar or read the wrong
           window. The String twin of the Bytes slice-of-slice rope case (10-bytes); the chunks here
           are constants because a STRING slice's scalar-offset walk cannot pre-resolve — the multibyte
           re-indexing is the pinned machinery, not the seam deferral.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def rope (String.concat "aé∀" "bçd"))
          (match
            (String.slice rope 1 5)
            ((Some outer)
              (match
                (String.slice outer 1 3)
                ((Some inner) (+ (* 100 (String.scalar-len inner)) (if (= inner "∀b") 1 0)))
                ((None _u) -2)))
            ((None _u) -3))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 201 Int64))
  (live-objects known-leak))

(case
  "the composed multibyte slice view equals its literal twin and keys a Map"
  (doc
    "The identity witness of the multibyte view-of-view case above, completing the family the
           single-slice champ-key rows begin: the doubly-sliced view `\"∀b\"` must EQUAL the literal
           by canonical `=` (10) AND find a value stored under the literal as a Map KEY (+7 → 17).
           The canonical form must be the content bytes, independent of the two view layers' offsets
           into the multibyte rope — residue from either layer would hash differently and miss.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def rope (String.concat "aé∀" "bçd"))
          (def
            inner
            (match
              (String.slice rope 1 5)
              ((Some outer) (match (String.slice outer 1 3) ((Some i) i) ((None _u) "")))
              ((None _u) "")))
          (+
            (* 10 (if (= inner "∀b") 1 0))
            (match (Map.lookup (Map.insert Map.empty "∀b" 7) inner) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 17 Int64))
  (live-objects known-leak))

(case
  "a String.slice view returned from a helper OUTLIVES the helper's local parent"
  (doc
    "The String twin of the Bytes slice-escape liveness pin (10-bytes): `mk` builds its rope
           parent as a LOCAL (`String.concat`, runtime storage) and returns a slice view of it — the
           parent binding dies at the helper's return while the view escapes. The caller compares the
           escaped view by CONTENT against a flat literal: a=1 windows \"bc\" → equal (1); a=0 windows
           \"ab\" → unequal (0). A reclaim at scope exit (rather than last-reference) would hand the
           caller a dangling view; a stale window would flip an answer.")
  (input
    (do
      (def
        (mk (: a Int64))
        (let
          ((parent (String.concat "ab" "cd")))
          (match (String.slice parent a (+ a 2)) ((Some s) s) ((None u) ""))))
      (def (main (: a Int64)) (if (= (mk a) "bc") 1 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "Symbol.of over a slice of a dying rope interns the WINDOW content"
  (doc
    "The intern composition of the escape: inside the helper, `Symbol.of` interns a slice view of
           the local rope — the symbol's identity must be the window's CONTENT (\"bc\" → equals the
           interned constant `#\"bc\"`), captured before or independent of the parent's death. An intern
           reading the parent's full bytes, or a window gone stale at the helper return, would produce a
           different symbol. Composes three pinned facts (slice re-basing, rope flatten at intern,
           view escape) into the tokenizer idiom: intern a window of a transient buffer.")
  (input
    (do
      (def
        (mk (: a Int64))
        (let
          ((parent (String.concat "ab" "cd")))
          (match
            (String.slice parent a (+ a 2))
            ((Some s) (Symbol.of s))
            ((None u) (Symbol.of "?")))))
      (def (main (: a Int64)) (if (= (mk a) #"bc") 1 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a runtime string rope inserted into a set is a member"
  (doc
    "The SET element-insert companion of the map-key cases: inserting a runtime String ROPE
           `(rep \"hi\" 3)`=\"hixxx\" into an empty set yields a 1-element set (`Set.len` = 1). Two
           earlier faults: the empty set `(Set.of (list))` leaves its element type an unresolved VAR
           (no construction pins it), so the backend defaulted the element box to `box-int` — which
           mis-boxed the i32 String HANDLE as an integer, emitting an INVALID component; and the rope
           element was not compacted before champ_insert. The fix boxes the element by its OWN concrete
           type when the set's declared type is unresolved, and compacts a rope element. A flat string /
           an Int element already inserted (box-int is correct for an Int); only a heap-handle element
           into an empty set hit the box bug. Expected: len 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (Set.len (Set.insert #set() (rep "hi" 3))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime string rope set element is found by its flat twin"
  (doc
    "The membership companion: after inserting the runtime ROPE `(rep \"hi\" 3)`=\"hixxx\" into a
           set, `Set.contains` with the flat literal \"hixxx\" is TRUE (→ 1). The element-insert now
           compacts the rope to its canonical flat leaf before champ_insert (mirroring the Map key and
           the Set QUERY key), so the flat query's champ_hash lands in the same slot. Pins that the Set
           ELEMENT-insert path canonicalizes a rope element, not only the query key.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (Set.contains (Set.insert #set() (rep "hi" 3)) "hixxx") 1 0))
      (export main)))
  (output (: 1 Int64)))

(case "string length" (input (String.scalar-len "hello")) (output (: 5 Int64)))

(case
  "scalar length counts Unicode scalar values, not bytes"
  (doc
    "Witnesses collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length.
           \"café\" is four scalar values (c, a, f, é) but FIVE UTF-8 bytes — é encodes as
           two bytes. String.scalar-len is the scalar count (4), NOT the byte count (5). The byte count is
           what String.byte-len yields (the byte-len case below), and the two differ here
           precisely because the string is multi-byte. `String.scalar-len \"hello\"` above cannot witness
           this — ASCII makes the two counts coincide.")
  (input (String.scalar-len "café"))
  (output (: 4 Int64)))

(case
  "scalar length of a supplementary-plane character is one scalar value"
  (doc
    "Witnesses collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length
           at the boundary that most tempts a byte- or UTF-16-based miscount: \"😀\" (U+1F600)
           is a single Unicode scalar value — scalar length 1 — even though it is four UTF-8 bytes (and
           two UTF-16 code units). A length implementation counting bytes would report 4, UTF-16 units 2;
           the scalar count is 1.")
  (input (String.scalar-len "😀"))
  (output (: 1 Int64)))

; --- Byte length is the UTF-8 byte count, obtained directly ------------------------------
; collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length: alongside the scalar
; length, a string offers its length in the bytes of its UTF-8 encoding as a SEPARATELY-NAMED op —
; String.byte-len — obtainable WITHOUT first materializing the bytes (it need not go through
; `String.to-bytes → Bytes.len`, though it MUST agree with that composition). There is no unqualified
; `String.len`: every length query names whether it counts scalars or bytes, so the café case that
; tempts the "which length?" confusion is a compile-time-explicit choice, not a silent default.
(case
  "byte length is the UTF-8 byte count"
  (doc
    "`(String.byte-len \"café\")` is 5 — the number of bytes in the UTF-8 encoding (é is two
           bytes), NOT the scalar count 4 (String.scalar-len \"café\" = 4, above). Pins the byte length
           as a first-class, directly-obtained op distinct from the scalar length
           (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).")
  (input (String.byte-len "café"))
  (output (: 5 Int64)))

(case
  "byte length agrees with the length of the encoded bytes"
  (doc
    "`(String.byte-len s)` MUST equal `(Bytes.len (String.to-bytes s))` — the direct byte length
           agrees with materializing the UTF-8 bytes and counting them; only the cost differs. Pins the
           two paths as the same number, so byte-len is a cheap shortcut, not a second answer.")
  (input (= (String.byte-len "café") (Bytes.len (String.to-bytes "café"))))
  (output (: true Bool)))

(case
  "byte length counts the normalized form, not the source spelling"
  (doc
    "The decomposed \"café\" (c, a, f, e + U+0301 combining acute — SIX UTF-8 bytes as written)
           normalizes to NFC (c, a, f, é — FIVE bytes) before it is a String value, so its byte length
           is 5, the byte count of the NORMALIZED contents (collections-and-text.md #A String Offers
           Both A Scalar Length And A Byte Length, 2nd sentence). Pins that byte-len is a function of the
           string's value, not of the incidental byte spelling normalization removes — the byte-length
           companion of the scalar-length-after-normalization case below.")
  (input (String.byte-len "café"))
  (output (: 5 Int64)))

(case
  "string to bytes (UTF-8)"
  (doc
    "The compiler encodes export names as UTF-8 bytes for wasm sections. String.to-bytes
           produces the UTF-8 byte sequence of the string.")
  (input (Bytes.len (String.to-bytes "run")))
  (output (: 3 Int64)))

(case
  "string to bytes encodes multi-byte characters"
  (doc "UTF-8 encodes non-ASCII characters as multiple bytes.")
  (input (Bytes.len (String.to-bytes "café")))
  (output (: 5 Int64)))

(case
  "string to bytes produces the exact UTF-8 byte values, not just the right count"
  (doc
    "String.to-bytes must produce the exact UTF-8 BYTES, not merely the right byte count — a
           length-only check would pass a Latin-1 encoding or a wrong continuation byte. `é` (U+00E9)
           encodes as the two bytes 0xC3 0xA9 = 195 169, and the 4-byte astral `😀` (U+1F600) as 0xF0 0x9F
           0x98 0x80 = 240 159 152 128. `(String.to-bytes \"é😀\")` therefore equals `(Bytes.of (list 195
           169 240 159 152 128))` — the 2-byte then 4-byte sequences concatenated. Pins the byte-level
           correctness of the UTF-8 encoder across the 2-byte and 4-byte forms (the boundaries a naive
           encoder gets wrong), the value companion of the byte-count cases above.")
  (input (= (String.to-bytes "é😀") (Bytes.of #list(195 169 240 159 152 128))))
  (output (: true Bool)))

(case "string equality" (input (= "hello" "hello")) (output (: true Bool)))

(case "string inequality" (input (= "hello" "world")) (output (: false Bool)))

(case
  "a string-literal pattern in a match selects by string equality, distinguishing Unicode"
  (doc
    "A `match` may test a String scrutinee against string-LITERAL patterns, selecting the arm whose
           literal equals the scrutinee (by normalized contents, collections-and-text.md #String Equality
           Follows Normalized Contents) — the same equality the `=` operator uses. `(match \"café\" (\"cafe\"
           1) (\"café\" 2) (_ 9))` selects the `\"café\"` arm, not `\"cafe\"`: the é distinguishes them, so
           the result is 2. Pins that string-literal pattern matching is by full Unicode-scalar equality
           (not a byte-prefix or ASCII-fold), and that a non-matching literal falls through to the wildcard
           — the string companion of the integer-literal-pattern match.")
  (input (match "café" ("cafe" 1) ("café" 2) (_ 9)))
  (output (: 2 Int64)))

(case
  "a RUNTIME string scrutinee matches string literals by equality"
  (doc
    "The RUNTIME companion of the constant string match above — THE compiler head-dispatch idiom: a
           `match` over a String chosen at run time, against string-LITERAL patterns. `op`'s scrutinee `s`
           is a runtime String (selected by a Bool, so it does not fold to a constant), matched `(match s
           (\"add\" 1) (\"sub\" 2) (_ 0))`. A String is a heap value, so this is not a scalar probe chain;
           it lowers to a chain of `(= s literal)` runtime string-equality tests (`value-eq`, the same
           equality `=` uses), the wildcard the tail. `op(if true \"add\" \"sub\")` = `op(\"add\")` selects
           the `\"add\"` arm → 1. Pins that a compiler can dispatch on a runtime keyword/opcode name by
           pattern, not only on a compile-time-constant string.")
  (input
    (do
      (def (op (: s String)) (match s ("add" 1) ("sub" 2) (_ 0)))
      (def (main) (op (if true "add" "sub")))
      (export main)))
  (output (: 1 Int64)))

; --- A runtime String `match` is OBSERVABLY EQUIVALENT to its desugared `(= s literal)` if-chain --------
; The runtime String match above "lowers to a chain of `(= s literal)` value-eq tests, the wildcard the
; tail" (its own doc). That desugaring is a backend-independent lowering choice — so a String match and the
; hand-written `if (= s "a") … else if (= s "b") … else DEFAULT` chain over the SAME scrutinee MUST compute
; the SAME value for every input, including the FALL-THROUGH (no literal matches → the wildcard / else).
; These pin the equivalence by computing BOTH forms in one program and checking they agree — a matched arm
; AND the default arm — so a lowering that reordered the arm tests, dropped the wildcard, or diverged the
; match from its `=`-chain desugaring would be caught, both backends. (A String is a heap value, so its
; runtime scrutinee is constructed inside `main` via `(if …)`, not passed as a call arg.)
(case
  "a runtime String match on a hit arm agrees with its desugared (= s literal) if-chain"
  (doc
    "`viamatch s = (match s (\"add\" 1) (\"sub\" 2) (_ 9))` and `viachain s = (if (= s \"add\") 1 (if
           (= s \"sub\") 2 9))` are the match and its `=`-chain desugaring. On a hit (`\"add\"`, built at
           runtime via `(if true \"add\" \"sub\")` so it is not a constant): match → 1, chain → 1, combined
           `10*match + chain` = 11. Pins the two forms agree on a matched arm, both backends.")
  (input
    (do
      (def (viamatch (: s String)) (match s ("add" 1) ("sub" 2) (_ 9)))
      (def (viachain (: s String)) (if (= s "add") 1 (if (= s "sub") 2 9)))
      (def (main) (+ (* 10 (viamatch (if true "add" "sub"))) (viachain (if true "add" "sub"))))
      (export main)))
  (output (: 11 Int64)))

(case
  "a runtime String match and its (= s literal) if-chain agree on the DEFAULT fall-through arm"
  (doc
    "The fall-through case: a scrutinee matching NO literal (`\"xyz\"`, built at runtime via `(if false
           \"add\" \"xyz\")`) must take the wildcard `(_ 9)` in the match AND the trailing `else 9` in the
           chain — the arm a dropped-wildcard or diverged desugaring would get wrong. `10*viamatch + viachain`
           = 10*9 + 9 = 99. Pins match ≡ `=`-chain on the DEFAULT arm, both backends.")
  (input
    (do
      (def (viamatch (: s String)) (match s ("add" 1) ("sub" 2) (_ 9)))
      (def (viachain (: s String)) (if (= s "add") 1 (if (= s "sub") 2 9)))
      (def (main) (+ (* 10 (viamatch (if false "add" "xyz"))) (viachain (if false "add" "xyz"))))
      (export main)))
  (output (: 99 Int64)))

(case
  "a String match with disjoint literal arms is order-independent (reordering the arms preserves the result)"
  (doc
    "The runtime String match dispatches by a chain of `(= s literal)` tests; when the literal patterns
           are MUTUALLY DISJOINT, at most one can match, so the arm ORDER is immaterial — a lowering that
           reordered the probe chain (e.g. for efficiency) must give the same result. `fwd s = (match s
           (\"add\" 1) (\"sub\" 2) (\"mul\" 3) (_ 9))` and `rev s = (match s (\"mul\" 3) (\"sub\" 2) (\"add\"
           1) (_ 9))` have the SAME arms in reverse order. On `\"sub\"` (built at runtime): both select 2, so
           `10*fwd + rev` = 22. Pins arm order-independence for disjoint string literals, both backends.")
  (input
    (do
      (def (fwd (: s String)) (match s ("add" 1) ("sub" 2) ("mul" 3) (_ 9)))
      (def (rev (: s String)) (match s ("mul" 3) ("sub" 2) ("add" 1) (_ 9)))
      (def (main) (+ (* 10 (fwd (if true "sub" "x"))) (rev (if true "sub" "x"))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a String match with a BOUND default arm equals its (= s literal) if-chain with a bound else"
  (doc
    "The default arm need not be a wildcard `_` — it may BIND the scrutinee and use it. `viamatch s =
           (match s (\"add\" 1) (other (String.byte-len other)))` binds `other` = the whole String in the
           default arm; its `=`-chain desugaring is `viachain s = (if (= s \"add\") 1 (String.byte-len s))`
           (the else reuses the scrutinee, not a fresh name). On `\"wxyz\"` (no hit, byte-len 4): match → 4,
           chain → 4, `100*viamatch + viachain` = 404. Pins match ≡ `=`-chain when the default BINDS and
           consumes the scrutinee (not only a constant wildcard body), both backends.")
  (input
    (do
      (def (viamatch (: s String)) (match s ("add" 1) (other (String.byte-len other))))
      (def (viachain (: s String)) (if (= s "add") 1 (String.byte-len s)))
      (def (main) (+ (* 100 (viamatch (if false "add" "wxyz"))) (viachain (if false "add" "wxyz"))))
      (export main)))
  (output (: 404 Int64)))

; --- The empty string is an ordinary String value ---------------------------------------
; `""` is the zero-length string — a first-class String the compiler needs (an empty error message, an
; empty name). Its length is 0 (counted in Unicode scalar values, of which it has none), it is equal
; only to another empty string, and it is the identity element of concatenation on both sides. The
; non-empty string cases above cannot witness the degenerate boundary where a length computation
; underflows or a concat assumes a non-empty operand; these pin it, the String companion of the
; empty-byte-sequence cluster (10-bytes.sexp) and the empty-map/empty-list cases.
(case
  "the empty string has length zero"
  (doc
    "`(String.scalar-len \"\")` is 0 — the empty string has no Unicode scalar values
           (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length). Pins
           that length handles the zero-length string, not underflowing or reading a phantom scalar.")
  (input (String.scalar-len ""))
  (output (: 0 Int64)))

(case
  "two empty strings are equal"
  (doc
    "`(= \"\" \"\")` is true: two empty strings have identical (empty) normalized contents, so
           they are equal (collections-and-text.md #String Equality Follows Normalized Contents). Pins
           that string equality treats the empty string as a genuine value equal to itself.")
  (input (= "" ""))
  (output (: true Bool)))

(case
  "an empty string is unequal to a non-empty string"
  (doc
    "`(= \"\" \"x\")` is false — the empty string and a one-character string have different
           contents. Pins that emptiness on one side is an ordinary inequality, not a special case.")
  (input (= "" "x"))
  (output (: false Bool)))

(case
  "concatenating an empty string on the right is the identity"
  (doc
    "`(String.concat \"hi\" \"\")` = \"hi\": appending the empty string changes nothing. Pins the
           right identity of String.concat — a concat that mishandles a zero-length operand would break
           the compiler's error-message and name assembly.")
  (input (= (String.concat "hi" "") "hi"))
  (output (: true Bool)))

(case
  "concatenating an empty string on the left is the identity"
  (doc
    "The left-identity companion: `(String.concat \"\" \"hi\")` = \"hi\". Pins that concatenation
           handles a zero-length LEFT operand too, mirroring the empty-byte-sequence concat cases.")
  (input (= (String.concat "" "hi") "hi"))
  (output (: true Bool)))

(case
  "String.concat with an empty operand is the identity on a RUNTIME string (the emit path, not the fold)"
  (doc
    "The two identity cases above use CONSTANT operands, so they fold at compile time. This pins the
           SAME left/right identity on a RUNTIME string — `s` built via `(if true \"hi\" \"x\")` so it is
           not a constant — exercising the runtime `String.concat` EMIT, which must handle a zero-length
           operand (an empty rope leaf / empty UTF-8 span) rather than assuming a non-empty side. `(String.
           concat \"\" s)` and `(String.concat s \"\")` both equal `s` = \"hi\"; `(and …)` of the two → 1.
           A runtime concat that mishandled the empty operand (read a phantom byte, or dropped the non-empty
           side) would break here where the folded cases could not catch it, both backends.")
  (input
    (do
      (def (lid (: s String)) (String.concat "" s))
      (def (rid (: s String)) (String.concat s ""))
      (def
        (main)
        (if (and (= (lid (if true "hi" "x")) "hi") (= (rid (if true "hi" "x")) "hi")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "runtime String.concat is associative — (a·b)·c equals a·(b·c), both equal the flat concatenation"
  (doc
    "Concatenation is associative: `(String.concat (String.concat a b) c)` and `(String.concat a
           (String.concat b c))` build the SAME string. Over runtime operands (`a` via `(if true \"a\"
           \"z\")` so nothing folds), with a multi-byte scalar in the middle: a=\"a\", b=\"é\" (1 scalar,
           2 UTF-8 bytes), c=\"bc\" — both groupings yield \"aébc\". Pins that the runtime concat EMIT (a
           rope build/append) is associative and does not depend on grouping — a rope-rebalance or a
           left-vs-right append that split the multi-byte scalar or reordered bytes at the join would make
           the two groupings differ. `(and (= left \"aébc\") (= right \"aébc\"))` → 1, both backends.")
  (input
    (do
      (def (lft (: a String) (: b String) (: c String)) (String.concat (String.concat a b) c))
      (def (rgt (: a String) (: b String) (: c String)) (String.concat a (String.concat b c)))
      (def
        (main)
        (if
          (and
            (= (lft (if true "a" "z") "é" "bc") "aébc")
            (= (rgt (if true "a" "z") "é" "bc") "aébc"))
          1
          0))
      (export main)))
  (output (: 1 Int64)))

(case
  "an empty-range slice of a non-empty string is Some of the empty string"
  (doc
    "`(String.slice \"hello\" 2 2)` has start = end, so it selects no scalar values — Some of the
           empty string (the in-bounds degenerate companion of the empty-range slice at index 0 already
           witnessed, here at an interior index). A slice whose start equals its end is present and
           empty, not None: the range [2,2) is valid and empty. The unwrapped slice MUST equal \"\".")
  (input (= (Option.expect (String.slice "hello" 2 2) "slice is in bounds") ""))
  (output (: true Bool)))

; --- String equality and length follow Unicode normalization -------------------------------
; collections-and-text.md #String Equality Follows Normalized Contents: "Two strings MUST be equal
; exactly when their NORMALIZED contents are identical, under the text normalization the hashing-
; and-encoding choice pins." And a string literal is stored in that canonical normal form
; (01-literals "a string literal is normalized to the canonical text form"). So two strings that
; differ ONLY in Unicode normalization — "café" with a composed é (U+00E9) vs "café" with a
; decomposed e + combining acute (U+0301) — denote ONE value: they are equal, and both have length 4
; (four scalar values after normalization). The seed stores the raw scalars without normalizing, so
; it compares them unequal and counts the decomposed form as 5 scalar values.
(case
  "strings differing only in Unicode normalization are equal"
  (doc
    "The composed \"café\" (…, U+00E9) and the decomposed \"café\" (…, e + U+0301
           combining acute) are the same text under the pinned normalization, so they MUST be equal
           (collections-and-text.md #String Equality Follows Normalized Contents). The seed compares
           the un-normalized scalar sequences and wrongly answers false.")
  (input (= "café" "café"))
  (output (: true Bool)))

(case
  "string length counts scalar values after normalization"
  (doc
    "The length of the decomposed \"café\" MUST be 4 — after normalization it is the four
           scalar values c, a, f, é, the same as the composed form (String.scalar-len \"café\" = 4,
           witnessed above). The seed counts the un-normalized e + combining acute as 5 scalar values.")
  (input (String.scalar-len "café"))
  (output (: 4 Int64)))

(case
  "string indexing returns Some of the character"
  (doc
    "Witnesses fallible String indexing (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping): an in-bounds scalar index yields the one-scalar string wrapped in Some.")
  (input (String.at "hello" 1))
  (output (: (Some "e") (Option String))))

; --- String indexing is by Unicode scalar value, not by byte -----------------------------
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values ("its contents are
; independent of any byte encoding") + #A String Offers Both A Scalar Length And A Byte Length:
; String.at addresses the string by SCALAR position, not byte offset. The ASCII `(String.at "hello"
; 1)` above cannot witness this — for ASCII the scalar index and byte offset coincide. A multi-byte
; string distinguishes them: in "café" (scalars c,a,f,é; é is 2 UTF-8 bytes) scalar index 3 is "é",
; whereas byte offset 3 lands in the MIDDLE of é's two-byte encoding (an invalid scalar boundary). A
; byte-indexing lowering would slice a partial code unit or trap; the scalar semantics must return "é".
(case
  "string indexing addresses Unicode scalar values, not bytes"
  (doc
    "`(String.at \"café\" 3)` = \"é\": the string is four scalar values (c, a, f, é) and
           index 3 is the last, é — even though é occupies bytes 3–4 of the five-byte UTF-8 encoding,
           so byte offset 3 would be a partial code unit. Pins that String.at indexes by scalar value
           (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), the companion
           of String.scalar-len counting scalars; the ASCII `(String.at \"hello\" 1)` cannot distinguish
           scalar index from byte offset.")
  (input (String.at "café" 3))
  (output (: (Some "é") (Option String))))

; Normalization is a WHOLE-VALUE property, so it must hold on the ADDRESSING axis too, not only on `=`
; (:510) and scalar-len (:518). A DECOMPOSED literal "cafe\u0301" (c, a, f, e, U+0301 combining acute — five
; raw scalars) is normalized by the reader to the four-scalar NFC form (c, a, f, é), so `String.at` and
; `String.scalar-len` operate on the NORMALIZED sequence: index 3 is the composed "é" (U+00E9), not the bare
; "e", and the string ends at scalar 4 (index 4 is None), not at the raw fifth scalar. A lowering that
; normalized for `=`/scalar-len but byte-walked the RAW scalars for `String.at` would return "e" at index 3
; (or index into the combining accent) and report length 5 — this pins the indexing axis against that.
; (This is the reader-normalized LITERAL path; `String.from-bytes` is a faithful byte decode that does NOT
; NFC-normalize — the deliberate known gap pinned at :1395 — so the two paths are distinct.)
(case
  "String.at on a decomposed literal returns the composed scalar at the normalized index"
  (doc
    "The decomposed \"café\" (c, a, f, e + U+0301 combining acute — five raw scalars) is normalized by
           the reader to the four-scalar NFC form, so `(String.at \"café\" 3)` = \"é\" (the COMPOSED U+00E9),
           not the bare \"e\" of the un-normalized fifth-from-last scalar. Indexing addresses the NORMALIZED
           sequence (collections-and-text.md #String Equality Follows Normalized Contents applied to the
           addressing axis), the indexing companion of the `=` (:510) and scalar-len (:518) normalization
           cases. A raw-scalar byte walk would return \"e\".")
  (input (String.at "café" 3))
  (output (: (Some "é") (Option String))))

(case
  "a decomposed literal indexes and measures by its normalized length, not the raw scalar count"
  (doc
    "The combined end-boundary face: the decomposed \"café\" has scalar-len 4 (the NFC form c, a, f, é),
           NOT the raw 5 (e + combining acute), AND `(String.at \"café\" 4)` is None — index 4 is past the
           normalized four-scalar end, not the raw fifth scalar. Sentinel `10·scalar-len + (at 4 ? 1 : 0)` =
           40. Pins that BOTH the length measure and the addressing bound use the normalized sequence; a
           lowering counting the raw 5 scalars would give 51 (len 5, and .at 4 = Some of the combining acute).")
  (input
    (do
      (def
        (main)
        (+
          (* 10 (String.scalar-len "café"))
          (match (String.at "café" 4) ((Some _c) 1) ((None _u) 0))))
      (export main)))
  (call main)
  (output (: 40 Int64)))

(case
  "string indexing past a supplementary-plane scalar lands on the next scalar"
  (doc
    "`(String.at \"😀b\" 1)` = \"b\": 😀 (U+1F600) is ONE scalar value occupying four UTF-8
           bytes (and two UTF-16 code units), so scalar index 1 is the character AFTER it, \"b\". A
           byte- or UTF-16-based index would land inside 😀's encoding. Pins scalar-value addressing
           at the boundary that most tempts a byte/UTF-16 miscount (the indexing companion of the
           supplementary-plane length case).")
  (input (String.at "😀b" 1))
  (output (: (Some "b") (Option String))))

(case
  "a constant-index String.at result compares equal to a literal by content"
  (doc
    "`(= (String.at \"banana\" 1) \"a\")` is true — the scalar at index 1 of \"banana\" is \"a\", and
           the one-scalar String it yields compares equal by CONTENT to the literal \"a\". A constant-index
           `String.at` folds to a `ConstStr`, so its equality is a content compare. Pins that a `String.at`
           result is content-comparable (the character-classifying `(= (String.at s i) …)` a lexer uses).
           MUST be true.")
  (input (= (Option.expect (String.at "banana" 1) "c") "a"))
  (output (: true Bool)))

(case
  "a recursive scalar-walk over a guest-built rope classifies characters on every backend"
  (doc
    "The full walk-and-classify loop a lexer runs (the single-index cases above pin one read): a
           recursive `String.at s i` walk over `String.scalar-len` counting the 'a' scalars of a
           concat-built \"banana\" → 3. Each iteration maps scalar index → byte offset over a ROPE; the
           loop bound comes from scalar-len on the same value, so the two scalar-addressing paths must
           agree at every index, not just index 0.")
  (input
    (do
      (def
        (walk (: s String) (: i Int64) (: n Int64) (: acc Int64))
        (if
          (>= i n)
          acc
          (walk
            s
            (+ i 1)
            n
            (match (String.at s i) ((Some c) (if (= c "a") (+ acc 1) acc)) ((None u) acc)))))
      (def
        (main (: k Int64))
        (let
          ((s (String.concat "ban" (String.concat "an" "a"))))
          (+ (walk s 0 (String.scalar-len s) 0) k)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a constant-index String.at result compares unequal to a different literal"
  (doc
    "The negative companion: index 0 of \"banana\" is \"b\", so `(= (String.at \"banana\" 0) \"a\")`
           is FALSE — the content compare distinguishes \"b\" from \"a\". Together with the case above this
           pins that a folded `String.at` result equals a literal exactly when their content matches. (At a
           CONSTANT index this equality folds and is correct; the same at a RUNTIME index is the
           failures-queue miscompile — these are the working boundary the bug sits against.) MUST be false.")
  (input (= (Option.expect (String.at "banana" 0) "c") "a"))
  (output (: false Bool)))

; --- String.at and String.slice are fallible: an out-of-range index yields None -------------
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values gives a string a defined
; scalar length, and #Indexing And Lookup Are Fallible, Not Trapping requires an out-of-range read to
; yield None rather than trap or produce an unspecified value. So String.at at a scalar index at or
; beyond the length, or at a NEGATIVE index, has no character to return and MUST yield None — exactly
; as List.at / Bytes.at do out of bounds (05-compound-types, 10-bytes). A negative index is the classic
; miscompile: a lowering that casts the index to an unsigned width turns -1 into a huge in-range-looking
; offset; the scalar-index bounds check must catch it as out of range and yield None.
(case
  "string indexing at or beyond the length yields None"
  (doc
    "`(String.at \"hi\" 5)` indexes scalar position 5 of a two-scalar string — out of range, no
           character to return — so it MUST yield None (collections-and-text.md #Indexing And Lookup Are
           Fallible, Not Trapping), the String companion of the List.at / Bytes.at out-of-bounds Nones.")
  (input (String.at "hi" 5))
  (output (: (None unit) (Option String))))

(case
  "a negative string index yields None rather than wrapping to a large offset"
  (doc
    "`(String.at \"hi\" -1)` uses a negative scalar index — no defined character — so it MUST
           yield None, NOT wrap. A lowering that casts the index to an unsigned integer would turn -1
           into a huge positive offset (either reading out of bounds or, worse, an unspecified in-range
           byte); fallible indexing requires None (collections-and-text.md #Indexing And Lookup Are
           Fallible, Not Trapping). The negative-index companion of the out-of-range case above.")
  (input (String.at "hi" -1))
  (output (: (None unit) (Option String))))

(case
  "string slicing yields Some of the substring"
  (doc
    "Witnesses fallible String slicing: an in-bounds range yields the substring wrapped in Some
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping). This case reads
           the Option directly, without unwrapping, to pin the Some.")
  (input (String.slice "hello world" 0 5))
  (output (: (Some "hello") (Option String))))

; --- String.slice bounds are checked: reversed, out-of-range, and negative bounds yield None; ----
; --- an empty in-range slice is Some of the empty string ------------------------------------
; String.slice takes a start and an end scalar index. A well-defined slice needs 0 ≤ start ≤ end ≤
; length; any bounds outside that have no defined substring and MUST yield None (collections-and-text.md
; #Indexing And Lookup Are Fallible, Not Trapping), while a degenerate but in-range slice where start =
; end is Some of the empty string — present, not None. These pin the boundary the encoder relies on when
; it slices instruction/name substrings: a reversed or over-long range is None, an empty in-range slice
; is Some "".
(case
  "a slice whose end is beyond the string length yields None"
  (doc
    "`(String.slice \"hi\" 0 5)` asks for scalars 0..5 of a two-scalar string — the end 5 is
           beyond the length — so the slice has no defined substring and MUST yield None
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping).")
  (input (String.slice "hi" 0 5))
  (output (: (None unit) (Option String))))

(case
  "a slice whose end precedes its start yields None"
  (doc
    "`(String.slice \"hello\" 3 1)` has end 1 before start 3 — a reversed range with no defined
           substring — so it MUST yield None rather than return an empty or reversed string. Pins that
           the start ≤ end constraint is checked, not silently normalized.")
  (input (String.slice "hello" 3 1))
  (output (: (None unit) (Option String))))

(case
  "a slice with a negative start yields None"
  (doc
    "`(String.slice \"hello\" -1 3)` has a negative start index — outside 0..length — so it MUST
           yield None, not wrap a negative bound to a large unsigned offset (the same negative-index
           miscompile the String.at case guards). Pins that both slice bounds are range-checked as
           signed values.")
  (input (String.slice "hello" -1 3))
  (output (: (None unit) (Option String))))

(case
  "a slice whose start equals its end is Some of the empty string"
  (doc
    "`(String.slice \"hello\" 2 2)` is a degenerate but in-range slice (0 ≤ 2 ≤ 2 ≤ 5): it
           selects zero scalars, so it is Some of the empty string \"\" — present, NOT None. Pins that
           the bounds check admits start = end (an empty result) rather than rejecting it, the boundary
           just inside the reversed-range None above.")
  (input (String.slice "hello" 2 2))
  (output (: (Some "") (Option String))))

; --- String.slice over a RUNTIME string (a parameter, not a literal) ------------------------
; The slice cases above feed a string LITERAL, so they const-fold before the runtime emitter is
; reached. A string operation's argument may be a runtime value — a parameter, a match/if-selected
; string — which does NOT fold: the seed must emit a runtime slice that walks the Bytes-backed UTF-8
; leaf to map SCALAR offsets to byte offsets (String offsets are scalar positions, not bytes, unlike
; Bytes.slice's byte length). These pin that the runtime slice agrees with the folded one on value,
; boundary handling, AND scalar-vs-byte indexing — the reader idiom a self-hosting compiler needs.
(case
  "a runtime string is sliced by scalar offsets"
  (doc
    "`(String.slice \"hello\" a b)` with RUNTIME bounds a=1, b=4 yields Some \"ell\" — scalars 1..4.
           Passing the slice BOUNDS as `main` parameters defeats const-folding (the folder cannot evaluate a
           slice whose indices are unknown at compile time), so this exercises the runtime UTF-8 slice walk —
           not the const-fold — and it must agree with the folded literal cases above. (Runtime BOUNDS, not a
           runtime string: a `String.concat s \"\"` over a literal `s` β-reduces + folds back to a literal.)")
  (input
    (do
      (def (main (: a Int64) (: b Int64)) (Option.expect (String.slice "hello" a b) "in range"))
      (export main)))
  (call main (: 1 Int64) (: 4 Int64))
  (output (: "ell" String))
  (live-objects known-leak))

(case
  "a runtime string slice addresses scalar values, not bytes"
  (doc
    "`(String.slice \"aébc\" a b)` with RUNTIME bounds a=1, b=3 yields Some \"éb\" — scalars 1 and 2
           (é is one scalar, TWO UTF-8 bytes). Runtime bounds force the runtime slice walk (the folder can't
           fold an unknown-index slice). A slice that indexed by BYTE offset would split é or read the wrong
           range; pins that the runtime walk maps scalar offsets to byte offsets, exactly as String.at does
           (13-strings §reading a string's scalar addresses scalar values, not bytes).")
  (input
    (do
      (def (main (: a Int64) (: b Int64)) (Option.expect (String.slice "aébc" a b) "in range"))
      (export main)))
  (call main (: 1 Int64) (: 3 Int64))
  (output (: "éb" String))
  (live-objects known-leak))

(case
  "a runtime string slice out of range yields None"
  (doc
    "`(String.slice s 0 5)` on the two-scalar `s = \"hi\"` has end 5 past the length, so it yields
           None — the runtime bounds check agrees with the folded out-of-range case. The match takes the
           None arm (-1), witnessing the absent result rather than a trap or a short string.")
  (input
    (do
      (def (f s) (match (String.slice s 0 5) ((Some x) (String.byte-len x)) ((None _) -1)))
      (def (main) (f "hi"))
      (export main)))
  (output (: -1 Int64)))

(case
  "a runtime string slice with an empty in-range span is Some of the empty string"
  (doc
    "`(String.slice s 2 2)` on a runtime `s = \"hello\"` selects zero scalars — Some \"\", present
           not None (the empty-span boundary, on the runtime path). `String.byte-len` of the result is
           0, distinguishing Some \"\" (0) from None (which the match would send elsewhere).")
  (input
    (do
      (def (f s) (match (String.slice s 2 2) ((Some x) (String.byte-len x)) ((None _) -1)))
      (def (main) (f "hello"))
      (export main)))
  (output (: 0 Int64)))

; --- String.slice with RUNTIME scalar-index arguments over a MULTI-BYTE string --------------------
; The runtime-slice cases above supply CONSTANT slice indices (the seed emits the runtime UTF-8 byte-walk
; for the string, but the offsets fold). When the slice INDICES are runtime values (fn parameters at the
; call boundary), the byte-walk must map each SCALAR offset to a byte offset at run time — the multi-byte
; correctness the scalar `String.at` cases pin, now on the runtime-index slice path. Over "café" (scalars
; c,a,f,é; é is 2 UTF-8 bytes, byte-len 5) a byte-indexing slice would split é or read the wrong span; the
; scalar semantics must isolate whole scalars. These pin runtime-index slice on both a 2-byte scalar (é)
; and a 4-byte supplementary-plane scalar (😀), by byte-length and by content.
(case
  "a runtime-index string slice isolates a multi-byte scalar by its scalar offset"
  (doc
    "`(String.slice \"café\" a b)` with RUNTIME indices a=3, b=4 selects scalar 3 — é, which occupies
           2 UTF-8 bytes — so the slice's `String.byte-len` is 2. A byte-indexing lowering would take byte
           offset 3 (the middle of é) and split the code unit; the scalar semantics map scalar offset → byte
           offset at run time and isolate the whole é. Pins the runtime-index slice's scalar-vs-byte mapping
           on a 2-byte scalar (the runtime-index companion of the constant-index `String.at \"café\" 3` = é).")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
      (export main)))
  (call main (: 3 Int64) (: 4 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a runtime-index string slice of the ASCII prefix before a multi-byte scalar"
  (doc
    "`(String.slice \"café\" 0 3)` with runtime indices selects scalars 0..2 (c,a,f — all ASCII), byte-len
           3. The prefix companion: the walk stops exactly at the scalar-3 boundary (byte 3), not inside é.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
      (export main)))
  (call main (: 0 Int64) (: 3 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a runtime-index string slice spanning ASCII and a multi-byte scalar"
  (doc
    "`(String.slice \"café\" 1 4)` with runtime indices selects scalars 1..3 — a,f (1 byte each) and é
           (2 bytes) — byte-len 4. Pins that the runtime walk accumulates the correct byte span across a
           mix of single- and multi-byte scalars, not a fixed bytes-per-scalar assumption.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
      (export main)))
  (call main (: 1 Int64) (: 4 Int64))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a runtime-index string slice compares equal to the expected multi-byte scalar by content"
  (doc
    "The content companion (not only byte-length): `(String.slice \"café\" a b)` with runtime a=3, b=4
           equals the string \"é\" by content. Confirms the isolated slice is the RIGHT scalar, not merely a
           2-byte span that happens to have the right length.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (= (Option.expect (String.slice "café" a b) "in range") "é"))
      (export main)))
  (call main (: 3 Int64) (: 4 Int64))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "a runtime-index string slice isolates a supplementary-plane scalar (four UTF-8 bytes)"
  (doc
    "`(String.slice \"a😀b\" a b)` with runtime a=1, b=2 selects scalar 1 — 😀 (U+1F600), ONE scalar
           occupying FOUR UTF-8 bytes — so byte-len is 4. A byte- or UTF-16-index walk would land inside the
           emoji's encoding; the scalar walk maps scalar offset 1..2 to the whole four-byte span. Pins the
           runtime-index slice at the supplementary-plane boundary that most tempts a code-unit miscount.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (String.byte-len (Option.expect (String.slice "a😀b" a b) "in range")))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 4 Int64))
  (live-objects known-leak))

; --- String.slice across the SEAM of a genuinely-runtime string ROPE ------------------------------
; The runtime-index cases above slice a FLAT literal ("hello", "café") — the string is a single leaf and
; only the bounds are runtime. A genuine multi-chunk ROPE — a `String.concat` whose left chunk is chosen
; by a run-time `if` (a folded literal concat β-reduces back to a literal, 13-strings §... at ~:661, so a
; runtime-selected chunk is required to defeat the fold) — reaches the slice as a deferred concatenation,
; so the byte-walk must cross the leaf boundary between chunks. These pin the seam-crossing slice: a span
; that begins in the left chunk and ends in the right must read the logical scalars in order across the
; physical seam, including the scalar→byte mapping when a chunk carries a multi-byte scalar.
(case
  "a runtime string slice spans the seam of a runtime-assembled rope"
  (doc
    "`(String.slice (String.concat (pick b \"abc\" \"xyz\") \"def\") lo hi)` over a rope built at run time
           (the left chunk chosen by a run-time `if`, so the concat cannot fold to a literal), with runtime
           bounds spanning the seam: b=true lo=2 hi=4 selects scalars 2..3 = \"cd\" — 'c' the last scalar of
           the left chunk, 'd' the first of the right — reading across the physical leaf boundary of a
           deferred concatenation (#Sharing Is Not Observable). The b=false branch slices the other rope
           (\"xyzdef\") at the same span → \"zd\", pinning that either runtime chunk assembles a real rope the
           slice crosses, not a pre-folded flat leaf. Both backends.")
  (input
    (do
      (def (pick (: b Bool) (: t String) (: f String)) (if b t f))
      (def
        (main (: b Bool) (: lo Int64) (: hi Int64))
        (Option.expect (String.slice (String.concat (pick b "abc" "xyz") "def") lo hi) "in range"))
      (export main)))
  (call main (: true Bool) (: 2 Int64) (: 4 Int64))
  (output (: "cd" String))
  (call main (: false Bool) (: 2 Int64) (: 4 Int64))
  (output (: "zd" String))
  (live-objects known-leak))

(case
  "a slice OF a slice composes the offsets at runtime bounds"
  (doc
    "Slicing a SLICED string re-bases the indices against the inner slice, not the original: the outer
           `(String.slice \"abcdefgh\" a 6)` at `a = 2` is `\"cdef\"`, and the inner `slice … 1 b` at `b = 3`
           reads ITS scalars 1..2 = `\"de\"` — offset `a + 1` into the original. At `(a,b) = (0,2)` the
           outer is `\"abcdef\"` and the inner scalar 1 is `\"b\"`. A slice that leaked the ORIGINAL string's
           indexing (reading scalars 1..b of \"abcdefgh\" regardless of a) would return \"b\" for both calls.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (Option.expect
          (String.slice (Option.expect (String.slice "abcdefgh" a 6) "outer") 1 b)
          "inner"))
      (export main)))
  (call main (: 2 Int64) (: 3 Int64))
  (output (: "de" String))
  (call main (: 0 Int64) (: 2 Int64))
  (output (: "b" String))
  (live-objects known-leak))

(case
  "a concat of two runtime SLICES joins the sliced views, not the originals"
  (doc
    "The build-from-parts idiom: both concat operands are SLICES (of different strings, split at the
           same runtime `a`) — `hello[0..a] ++ world[a..5]`. At `a = 2`: `\"he\" ++ \"rld\"` = `\"herld\"`;
           at `a = 5`: the left slice is the whole `\"hello\"` and the right is EMPTY → `\"hello\"`. Pins
           that concat consumes the sliced VIEWS (a concat reading the operands' original strings would give
           `\"helloworld\"` in both regimes) and that an empty right slice is the concat identity.")
  (input
    (do
      (def
        (main (: a Int64))
        (String.concat
          (Option.expect (String.slice "hello" 0 a) "l")
          (Option.expect (String.slice "world" a 5) "r")))
      (export main)))
  (call main (: 2 Int64))
  (output (: "herld" String))
  (call main (: 5 Int64))
  (output (: "hello" String))
  (live-objects known-leak))

(case
  "a runtime rope slice maps scalar offsets to bytes across the seam for a multi-byte scalar"
  (doc
    "The multi-byte companion of the rope-seam slice: over a runtime rope `(String.concat (pick b \"aé\"
           \"aX\") \"bc\")`, b=true lo=1 hi=3 selects scalars 1..2 = \"éb\" — é (scalar 1, TWO UTF-8 bytes) is the
           last scalar of the left chunk and b (scalar 2) the first of the right, so the slice crosses the
           seam AND maps a multi-byte scalar's offset to its byte span. A byte-indexing walk would split é or
           miscount across the seam; the scalar walk isolates \"éb\" (byte-len 3). Pins the across-seam
           scalar→byte mapping on a genuine rope, both backends.")
  (input
    (do
      (def (pick (: b Bool) (: t String) (: f String)) (if b t f))
      (def
        (main (: b Bool) (: lo Int64) (: hi Int64))
        (String.byte-len
          (Option.expect (String.slice (String.concat (pick b "aé" "aX") "bc") lo hi) "in range")))
      (export main)))
  (call main (: true Bool) (: 1 Int64) (: 3 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

; --- `String.at i` and `String.slice i (i+1)` are the SAME single-scalar addressing — they must agree ---
; `String.at` and `String.slice` are the two runtime scalar-addressing String ops (both byte-walk the UTF-8
; leaf, mapping scalar offsets to byte offsets). A single-scalar slice `[i, i+1)` selects exactly the one
; scalar `String.at i` returns — so `String.slice s i (i+1)` and `String.at s i` MUST produce the equal
; one-scalar substring for the same runtime `s`/`i`. They lower through separate emit paths (String.at's
; one-scalar read vs String.slice's start..end byte-walk), so this pins the two paths AGREE — a lowering
; that computed a different byte span for one (an off-by-one scalar→byte map, or slicing bytes not scalars)
; would diverge them. Checked over a MULTI-BYTE scalar (é = 1 scalar, 2 bytes) so a byte/scalar confusion
; in either path is observable, on both backends.
(case
  "a single-scalar String.slice equals the String.at of the same index (the two scalar-addressing paths agree)"
  (doc
    "`(String.slice s i (+ i 1))` and `(String.at s i)` both address the single scalar at index `i`,
           so their one-scalar substrings are equal. Over a runtime `s = \"aébc\"` (é is one scalar, TWO
           UTF-8 bytes) at `i = 1`: both yield \"é\" — `(= (slice…) (at…))` → true (1). Pins that the
           String.slice byte-walk and the String.at read map scalar→byte identically for a single scalar
           (a byte/scalar off-by-one in either path would make them unequal), both backends. `s` is built
           via `(if true …)` so it is a runtime value, not a folded constant.")
  (input
    (do
      (def
        (viaslice (: s String) (: i Int64))
        (Option.expect (String.slice s i (+ i 1)) "in bounds"))
      (def (viaat (: s String) (: i Int64)) (Option.expect (String.at s i) "in bounds"))
      (def (main) (if (= (viaslice (if true "aébc" "x") 1) (viaat (if true "aébc" "x") 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "the scalar-len of an in-bounds String.slice equals its span (end - start), independent of byte width"
  (doc
    "`String.slice` addresses SCALARS, so an in-bounds `(String.slice s i j)` returns exactly `j - i`
           scalar values regardless of how many BYTES each occupies — `(String.scalar-len (slice s i j))`
           == `j - i`. Over a runtime `s = \"aébcd\"` (é is one scalar, TWO UTF-8 bytes) with the span
           `1..4`: the slice is \"ébc\" — 3 scalars (though 4 bytes), so scalar-len = 3 = 4 - 1. Pins the
           slice's scalar count is its scalar SPAN, not a byte count — a slice that returned a byte-measured
           length, or that mis-mapped the multi-byte scalar, would give the wrong count. The scalar-count
           companion of the slice≡at case above, both backends. `s` is built via `(if true …)` so it is a
           runtime value, not a folded constant.")
  (input
    (do
      (def
        (spanlen (: s String) (: i Int64) (: j Int64))
        (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
      (def (main) (spanlen (if true "aébcd" "x") 1 4))
      (export main)))
  (output (: 3 Int64)))

; slice LENGTH-ALGEBRA neighbors (breaker): the case above pins scalar-len(slice i j) == j - i. These pin
; the companions: on the SAME multi-byte slice the BYTE-len differs from the scalar-len (proving the two
; measures genuinely diverge), a full-string slice (0..scalar-len) recovers the whole string's length by
; BOTH measures, and an empty span (i=j) is scalar-len 0 even at a multi-byte position. Over "aébcd" (a,é,
; b,c,d — 5 scalars; é is 2 UTF-8 bytes, so byte-len 6). `s` is a runtime value via `(if true …)`.
(case
  "the byte-len of a multi-byte String.slice exceeds its scalar-len (the two measures diverge)"
  (doc
    "The SAME slice the span case measures at scalar-len 3, measured by BYTE-len, is 4: `(String.slice
           \"aébcd\" 1 4)` = \"ébc\" — 3 scalars but 4 bytes (é=2, b=1, c=1). Pins that byte-len and scalar-len
           of one slice genuinely DIVERGE for multi-byte content — the slice carries the real UTF-8 bytes,
           and byte-len counts them while scalar-len counts the span. A slice that stored a scalar-count as
           its byte length (or vice versa) would make these equal.")
  (input
    (do
      (def
        (bytelen (: s String) (: i Int64) (: j Int64))
        (String.byte-len (Option.expect (String.slice s i j) "in bounds")))
      (def (main) (bytelen (if true "aébcd" "x") 1 4))
      (export main)))
  (output (: 4 Int64)))

(case
  "a full-string String.slice recovers the whole string's length by both measures"
  (doc
    "Slicing the FULL span `0..scalar-len` returns the whole string: scalar-len of `(String.slice
           \"aébcd\" 0 5)` is 5 (= the string's scalar-len), and its byte-len is 6 (= the string's byte-len,
           é contributing 2). Pins the identity slice at both measures — the span 0..n is the whole string,
           not off-by-one at either end.")
  (input
    (do
      (def
        (spanlen (: s String) (: i Int64) (: j Int64))
        (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
      (def (main) (spanlen (if true "aébcd" "x") 0 5))
      (export main)))
  (output (: 5 Int64)))

(case
  "a full-string String.slice byte-len equals the whole string's byte-len"
  (doc
    "The byte-len companion of the full-string slice: `(String.byte-len (String.slice \"aébcd\" 0 5))`
           is 6 — a=1, é=2, b=1, c=1, d=1. Pins that the identity slice preserves the exact UTF-8 byte count,
           not a scalar-count (which would be 5).")
  (input
    (do
      (def
        (bytelen (: s String) (: i Int64) (: j Int64))
        (String.byte-len (Option.expect (String.slice s i j) "in bounds")))
      (def (main) (bytelen (if true "aébcd" "x") 0 5))
      (export main)))
  (output (: 6 Int64)))

(case
  "an empty-span String.slice at a multi-byte position has scalar-len zero"
  (doc
    "An empty span `i=j` returns the empty string — scalar-len 0 — even when `i` sits at a multi-byte
           scalar boundary. `(String.scalar-len (String.slice \"aébcd\" 1 1))` = 0 (index 1 is é's position).
           Pins that a zero-width span is genuinely empty regardless of the byte width at that position — the
           j - i = 0 span case, the interior-multibyte companion of the empty-span boundary.")
  (input
    (do
      (def
        (spanlen (: s String) (: i Int64) (: j Int64))
        (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
      (def (main) (spanlen (if true "aébcd" "x") 1 1))
      (export main)))
  (output (: 0 Int64)))

; --- A string operation consumes a string SELECTED by runtime control flow -----------------
; A string operation (String.scalar-len/at/slice/concat) takes a string ARGUMENT; that argument may be a
; string SELECTED at run time by an `if` or a `match`, not only a literal. The seed resolves a string
; operation's argument through a runtime `if` — `(String.scalar-len (if b "hello" "hi"))` computes the
; selected string's length — but NOT through a runtime `match`: `(String.scalar-len (match n (0 "zero") (_
; "other")))` declines. Both `if` and `match` select a string value the same way, so a string
; operation must consume either. (This is about CONSUMING a runtime-selected string in an operation,
; distinct from RETURNING one across the run boundary — the latter needs the compound-value output ABI.)
(case
  "String.scalar-len of a string selected by a runtime match"
  (doc
    "`(match n (0 \"zero\") (_ \"other\"))` selects a string by the runtime value n; `String.scalar-len`
           of that selected string is its scalar length — with n=5 the wildcard picks \"other\", length
           5. A string operation must consume a match-selected string exactly as it consumes an
           if-selected one (the control below, which the seed runs). The seed declines the match case
           (\"unsupported dotted-application\") — its string-op argument resolution follows a runtime
           `if` but not a runtime `match`.")
  (input
    (do
      (def (f n) (String.scalar-len (match n (0 "zero") (_ "other"))))
      (def (main) (f 5))
      (export main)))
  (output (: 5 Int64)))

(case
  "String.scalar-len of a string selected by a runtime if"
  (doc
    "The control the case above must match: `String.scalar-len` of a string chosen by a runtime `if`
           computes the selected string's length — `(if b \"hello\" \"hi\")` with b=true is \"hello\",
           length 5. The seed runs this; the match companion must behave identically.")
  (input
    (do (def (f b) (String.scalar-len (if b "hello" "hi"))) (def (main) (f true)) (export main)))
  (output (: 5 Int64)))

; --- A string flows as a genuine RUNTIME value: a fn parameter, a return, a sum payload -----------
; The cases above CONSUME a string in an operation whose result is a scalar (a length), so the string
; itself never crosses a function boundary as a value. These pin the string flowing as a first-class
; runtime value — passed to a function, returned from one, carried in a sum variant, compared for
; equality, concatenated — the operations a program that dispatches on names and threads a symbol
; table requires (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values; the string
; analogue of the "Bytes as a RUNTIME value" cases in 10-bytes.sexp). A string literal reaching one of
; these positions is a runtime value (a UTF-8 leaf), not only a compile-time constant; the compiler
; materializes it on the value heap. `main` here yields a scalar so the observable stays a plain value.
(case
  "a string passed to a function as a runtime argument is measured by its byte length"
  (doc
    "`(len2 \"hello\")` passes the string literal as a genuine runtime argument to `len2`, whose
           body takes its byte length. `String.byte-len` of a runtime string parameter is 5 — the UTF-8
           byte count. Pins that a string flows across a function boundary as a first-class value, not
           only as a folded constant (the front end passes a form's head string to a classifier this way).")
  (input (do (def (len2 s) (String.byte-len s)) (def (main) (len2 "hello")) (export main)))
  (output (: 5 Int64)))

(case
  "a runtime string equality selects a branch by comparing a parameter to a literal"
  (doc
    "`(if (= s \"def\") 1 0)` compares a runtime string parameter `s` against a literal — the
           name-dispatch primitive a compiler uses to recognize a form's head. Equality is structural
           over the UTF-8 bytes: `(pick \"def\")` is 1 (bytes match), `(pick \"x\")` is 0 (length
           differs). Their sum is 1. Pins runtime string `=` as a byte comparison, not a handle identity.")
  (input
    (do (def (pick s) (if (= s "def") 1 0)) (def (main) (+ (pick "def") (pick "x"))) (export main)))
  (output (: 1 Int64)))

(case
  "a multi-way string-head dispatch resolves an operator name to its operation"
  (doc
    "The compiler's front-rung idiom end-to-end: a form's head is a STRING, and the resolver maps
           it to an operation by a chain of runtime string comparisons — `(= h \"+\")`, `(= h \"-\")`,
           `(= h \"*\")` — selecting the arithmetic to perform on the operands. This is the concrete
           `head-prim`/`resolve` shape a name-based front end takes: the reader hands the compiler a head
           NAME, and resolution turns that name into a code (here, directly into the operation) before
           anything downstream runs — the 'resolve names to codes before selecting instructions' step
           (compiler-pipeline.md §Representation) made concrete over runtime strings. `(eval-head \"+\"
           20 22)` resolves `\"+\"` and computes 42. Unlike the single-comparison case above, this pins a
           MULTI-way dispatch — several head names, each selecting a distinct operation — which is what a
           real head resolver is; the falls-through default (an unrecognized head) yields 0 here, the
           value-level stand-in for the front end's decline on an unknown head.")
  (input
    (do
      (def (eval-head h a b) (if (= h "+") (+ a b) (if (= h "-") (- a b) (if (= h "*") (* a b) 0))))
      (def (main) (eval-head "+" 20 22))
      (export main)))
  (output (: 42 Int64)))

(case
  "a string carried as a sum-variant payload is bound and measured at run time"
  (doc
    "A `Node` variant carries a String payload built at run time; a `match` binds it and takes its
           byte length. `(weigh (Node.NSym \"hello\"))` is 5 (the bound string's byte length) and
           `(weigh (Node.NInt 3))` is 3, summing to 8. Pins a string as a runtime sum payload — the
           shape of a symbol-carrying AST node the compiler walks — bound by a match arm and consumed.")
  (input
    (do
      (type Node (NInt Int64) (NSym String))
      (def (weigh n) (match n ((Node.NInt i) i) ((Node.NSym s) (String.byte-len s))))
      (def (main) (+ (weigh (Node.NSym "hello")) (weigh (Node.NInt 3))))
      (export main)))
  (output (: 8 Int64)))

(case
  "concatenating two runtime strings and measuring the result"
  (doc
    "`(String.concat a b)` of two runtime string parameters yields a runtime string whose byte
           length is the sum of the operands' — `(join \"foo\" \"bar\")` is \"foobar\", byte length 6.
           Pins runtime string concatenation (how a compiler assembles a name or a diagnostic from
           fragments), agreeing with `(+ (byte-len a) (byte-len b))` when neither operand is empty.")
  (input
    (do
      (def (join a b) (String.byte-len (String.concat a b)))
      (def (main) (join "foo" "bar"))
      (export main)))
  (output (: 6 Int64)))

(case
  "a tail-recursive string accumulator builds a runtime string and its length is measured"
  (doc
    "A self-recursive function threads a runtime STRING accumulator — `(rep s n)` returns the
           accumulator `s` in its base arm and recurses with `(String.concat s \"x\")` — the string
           analogue of a threaded compound accumulator (a compiler builds a name or a rendered form
           this way, appending fragment by fragment). `(String.byte-len (rep \"\" 3))` is 3: three
           appends of a one-byte \"x\". Pins that the accumulator PARAMETER and the function's RETURN
           both converge to the runtime-string (heap) kind even though the base arm returns the
           parameter bare and the recursive arm is a bare self-call — neither branch of the `if`
           independently reports a heap kind on the first inference pass. Without unifying the two
           branches to the heap kind, the recursive `if` is rejected `if branches differ in kind`
           (the then-branch `s` is heap, the else self-call defaulted to Int64); the same
           accumulator-return-kind convergence a heap-list or Bytes accumulator needs.")
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (String.byte-len (rep "" 3)))
      (export main)))
  (output (: 3 Int64)))

(case
  "the byte length of a runtime string equals the length of its encoded bytes"
  (doc
    "The runtime companion of the const `byte-len`/`to-bytes` agreement: for a runtime string `s`,
           `(String.byte-len s)` MUST equal `(Bytes.len (String.to-bytes s))` — the direct byte count and
           the encode-then-measure path are one number. `\"café\"` has byte length 5 (é is two bytes), so
           this is true. Pins that a runtime String IS its UTF-8 bytes (String.to-bytes is the identity on
           the underlying representation), the invariant the Bytes-backed String realization rests on.")
  (input
    (do
      (def (agree s) (= (String.byte-len s) (Bytes.len (String.to-bytes s))))
      (def (main) (agree "café"))
      (export main)))
  (output (: true Bool)))

(case
  "String.to-bytes of a genuinely-runtime string yields its UTF-8 byte length"
  (doc
    "`String.to-bytes` on a value that is NOT a compile-time-visible constant string — here forced
           runtime by `(String.concat s \"\")`, the same shape `String.at`'s runtime cases use — must emit
           the runtime encoding, not decline. A String IS a UTF-8 Bytes leaf, so the encoding is total: it
           materializes the string's byte-rope into a canonical flat Bytes leaf (the runtime `bytes-compact`
           op — the exact inverse of the runtime `str-from-bytes` decode). `\"café\"` is 5 UTF-8 bytes (é is
           two). Pins the runtime `String.to-bytes` path the compiler-in-Cadenza codec ENCODER rests on —
           previously declined 'not yet computed (constant strings only)'.")
  (input
    (do
      (def (enc s) (Bytes.len (String.to-bytes (String.concat s ""))))
      (def (main) (enc "café"))
      (export main)))
  (output (: 5 Int64)))

(case
  "String.to-bytes of a runtime string threaded through recursion round-trips its bytes"
  (doc
    "The serializer shape: a String payload threaded through a recursive encoder is genuinely runtime,
           so `String.to-bytes(n)` takes the runtime path. Encodes `Name(\"foo\")` as a tag byte (2) then the
           byte-length prefix then the UTF-8 bytes — `1 + 1 + 3 = 5` bytes total. This is the exact shape the
           compiler-ml codec's encode.cdz takes (a Name's UTF-8 written via String.to-bytes), the round-trip
           blocker this op removes.")
  (input
    (do
      (def (b1 x) (Bytes.of #list((UInt8.wrap x))))
      (def (str-payload s) (Bytes.concat (b1 (String.byte-len s)) (String.to-bytes s)))
      (def (main) (Bytes.len (Bytes.concat (b1 2) (str-payload (String.concat "foo" "")))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a byte of a runtime string's encoding is its exact UTF-8 value, not merely the right count"
  (doc
    "Content, not just length: index the runtime `String.to-bytes` result to read a specific UTF-8
           byte. `\"café\"` encodes to `99 97 102 195 169` (é = C3 A9); byte 4 is `169` (0xA9, the trailing
           byte of é). Forced runtime by `(String.concat s \"\")`. Pins that the runtime `bytes-compact`
           flatten PRESERVES the byte content — a rope's node `raw` holds header bytes, not content, so a
           flatten that read the header would give the wrong byte. Reads via `Bytes.at` (→ `Option Int64`),
           avoiding runtime-Bytes value-equality (a separate unimplemented compound heap-walk).")
  (input
    (do
      (def (enc s) (String.to-bytes (String.concat s "")))
      (def (main) (Option.expect (Bytes.at (enc "café") 4) "in range"))
      (export main)))
  (output (: 169 Int64)))

(case
  "two runtime Bytes values of equal content compare equal (rope vs flat)"
  (doc
    "Runtime `Bytes` value-equality (`=`): a `Bytes.concat` builds a ROPE whose physical node bytes
           differ from a flat leaf of IDENTICAL content, so the tagless `champ_eq` walk would compare them
           UNEQUAL unless the rope is flattened first. The `value-eq` emit `bytes-compact`s every direct
           Bytes operand before the compare (the byte twin of the String-operand compaction), so a
           `Bytes.concat` rope `[104,105]` compares EQUAL to the flat literal `Bytes.of [104,105]` → true.
           Pins the DIRECT-operand runtime Bytes `=` — was declined 'comparison of a compound value needs a
           heap walk'.")
  (input
    (do
      (def (rope) (Bytes.concat (Bytes.of #list(104)) (Bytes.of #list(105))))
      (def (main) (= (rope) (Bytes.of #list(104 105))))
      (export main)))
  (output (: true Bool)))

(case
  "runtime Bytes value-equality distinguishes different content"
  (doc
    "The negative companion: two runtime Bytes of DIFFERENT content compare `false` (not merely a
           physical-identity check). A `Bytes.concat` rope `[104,105]` is NOT equal to `Bytes.of [104,106]`.
           Confirms the `value-eq` compare is genuinely structural over the flattened bytes, not a trivial
           always-true / handle-identity compare.")
  (input
    (do
      (def (rope) (Bytes.concat (Bytes.of #list(104)) (Bytes.of #list(105))))
      (def (main) (= (rope) (Bytes.of #list(104 106))))
      (export main)))
  (output (: false Bool)))

(case
  "the runtime UTF-8 encoding of a string equals its exact byte literal"
  (doc
    "The round-trip the compiler-in-Cadenza codec rests on: `String.to-bytes` of a genuinely-runtime
           string (forced by `String.concat s \"\"`) compares EQUAL to the exact UTF-8 `Bytes.of` literal.
           `\"é😀\"` = `195 169 240 159 152 128` (é = C3 A9, 😀 = F0 9F 98 80). The to-bytes result is itself
           a `bytes-compact`ed flat leaf and the `value-eq` emit compacts the literal operand too, so the
           structural compare is exact → true. This is the case that DECLINED before direct-Bytes `=`.")
  (input
    (do
      (def (enc s) (String.to-bytes (String.concat s "")))
      (def (main) (= (enc "é😀") (Bytes.of #list(195 169 240 159 152 128))))
      (export main)))
  (output (: true Bool)))

(case
  "the scalar length of a runtime multi-byte string counts scalars, not bytes"
  (doc
    "`String.scalar-len` of a runtime string counts Unicode scalar values, not UTF-8 bytes:
           `(slen \"café\")` is 4 (c, a, f, é) even though the byte length is 5. Pins that scalar length
           on a runtime value agrees with the const `chars().count()` — the runtime counts the UTF-8
           leading bytes (those not of the form 10xxxxxx), which for well-formed UTF-8 is the scalar
           count (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).")
  (input (do (def (slen s) (String.scalar-len s)) (def (main) (slen "café")) (export main)))
  (output (: 4 Int64)))

(case
  "the scalar length of a GENUINELY-runtime multi-byte string walks the UTF-8 leaf"
  (doc
    "The case above passes a bare literal, which const-FOLDS (`chars().count()`) before emit — it
           never exercises the runtime `String.scalar-len` path. Forcing a genuine runtime String with
           `(String.concat s \"\")` (the same runtime-forcing the runtime `String.at` cases use) makes the
           backend WALK the UTF-8 byte leaf, counting LEAD bytes (`(byte & 0xC0) != 0x80`, not the
           `10xxxxxx` continuation bytes) — the scalar count for well-formed UTF-8. `\"café\"` is 4 scalars
           (c,a,f,é) though 5 bytes (é is a 2-byte encoding). Pins the emit-side scalar-len walk (reusing
           `String.at`'s scalar-scan machinery over the already-exported `bytes-len`/`bytes-get` ops — no
           new runtime op, frozen hash unchanged); the const case above pins the fold. Was a decline before
           this (\"a runtime string's scalar length needs a UTF-8 decoding walk\").")
  (input
    (do
      (def (slen s) (String.scalar-len (String.concat s "")))
      (def (main) (slen "café"))
      (export main)))
  (output (: 4 Int64)))

(case
  "the scalar length of a runtime string spanning every UTF-8 width"
  (doc
    "The full-width witness: a genuinely-runtime `\"café—😀\"` mixes 1-byte (c,a,f), 2-byte (é),
           3-byte (—, U+2014) and 4-byte (😀, U+1F600) encodings — 6 scalars, 12 bytes. `String.scalar-len`
           counts 6 (every lead byte), confirming the walk's `(byte & 0xC0) != 0x80` lead-byte test handles
           all four UTF-8 encoding widths, not just the 2-byte case. Contrast `String.byte-len` = 12. The
           multi-width companion of the café case.")
  (input
    (do
      (def (slen s) (String.scalar-len (String.concat s "😀")))
      (def (main) (slen "café—"))
      (export main)))
  (output (: 6 Int64)))

(case
  "a runtime string is indexed by scalar and the extracted scalar is returned"
  (doc
    "`String.at` on a RUNTIME string — the reader's scalar cursor — reads the i-th Unicode scalar
           as a one-scalar string, fallibly. `(at (concat s \"\") 3)` on \"café\" reads scalar 3 (é,
           which occupies UTF-8 bytes 3–4), returning `(Some \"é\")` — indexed by SCALAR, not byte, so
           the multi-byte é comes back whole. Pins runtime `String.at`: the seed walks the UTF-8 buffer
           to the scalar's byte span and slices it (a String is a Bytes-backed leaf), matching the const
           `chars().nth`. The concat forces a runtime value (a bare literal would const-fold).")
  (input
    (do (def (at s i) (String.at (String.concat s "") i)) (def (main) (at "café" 3)) (export main)))
  (output (: (Some "é") (Option String)))
  (live-objects known-leak))

(case
  "indexing a runtime string past its last scalar yields None"
  (doc
    "The fallible companion: `String.at` at a scalar index at or beyond the string's scalar length
           yields `(None unit)`, never a trap (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping). `(at \"hi\" 5)` on the two-scalar \"hi\" is out of range → None. Pins that a
           runtime `String.at` out-of-bounds is a handled absence — the branch a reader takes at
           end-of-input.")
  (input
    (do
      (def (at s i) (String.at (String.concat s "") i))
      (def (main) (match (at "hi" 5) ((Some c) (String.byte-len c)) ((None _) -1)))
      (export main)))
  (output (: -1 Int64)))

; The runtime String.at cases above force ropes via `(String.concat s "")` — an EMPTY right chunk, so
; the rope has no real interior seam and every scalar lives in one leaf. These use a GENUINE two-chunk
; rope (if-selected left chunk ++ non-empty right chunk) with MULTIBYTE content in the left chunk: the
; scalar→byte mapping must carry ACROSS the seam (scalar 2 begins at byte 3 when the left chunk is
; "aé", at byte 2 when it is "xy" — same scalar index, different byte offset per branch), and a scalar
; read AT the seam-adjacent position must return the multibyte scalar whole.
(case
  "String.at addresses scalars across the seam of a runtime multibyte rope"
  (doc
    "`(String.at (String.concat (pick b) \"bc\") 2)` — the rope is `\"aé\" ++ \"bc\"` (b>0) or
           `\"xy\" ++ \"bc\"` (b≤0). Scalar 2 is \"b\" in BOTH branches, but its BYTE offset differs: 3
           after the two-byte é, 2 after ascii — so a byte-indexed (or per-leaf-only) walk gives the
           wrong answer in exactly one branch. Both calls must see \"b\" → 1. Pins the scalar→byte map
           spans the concat seam with multibyte content upstream of it. Expected: 1, 1.")
  (input
    (do
      (def (pick (: b Int64)) (if (> b 0) "aé" "xy"))
      (def
        (main (: b Int64))
        (match
          (String.at (String.concat (pick b) "bc") 2)
          ((Some c) (if (= c "b") 1 0))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: -1 Int64))
  (output (: 1 Int64))
  ; The non-recursive `(match (String.at …) ((Some c) …))` Some shell is now reclaimed (v-core-opt
  ; owned-single-view MatchSum shell reclaim; the arm only borrows `c` via value-eq) → was 3, now 1.
  (live-objects known-leak))

(case
  "String.at reads a multibyte scalar whole at its position in a runtime two-chunk rope"
  (doc
    "The same two-chunk shape read AT the multibyte scalar: index 1 of `\"aé\" ++ \"cd\"` is \"é\"
           (→ 1); the control branch `\"ab\" ++ \"cd\"` has \"b\" there (→ 2). The é spans two UTF-8
           bytes ending at the leaf boundary — a reader that split the scalar at the leaf edge or
           returned a one-BYTE slice would fail the content compare. Together with the across-the-seam
           case this pins both halves of multibyte addressing in a genuine rope: landing ON the wide
           scalar, and landing PAST it. Expected: 1 (b=1), 2 (b=-1).")
  (input
    (do
      (def (pick (: b Int64)) (if (> b 0) "aé" "ab"))
      (def
        (main (: b Int64))
        (match
          (String.at (String.concat (pick b) "cd") 1)
          ((Some c) (if (= c "é") 1 (if (= c "b") 2 0)))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: -1 Int64))
  (output (: 2 Int64))
  ; The non-recursive `(match (String.at …) ((Some c) …))` Some shell is now reclaimed (v-core-opt
  ; owned-single-view MatchSum shell reclaim; the arm only borrows `c` via value-eq) → was 3, now 1.
  (live-objects known-leak))

(case
  "a runtime string returned across the run boundary renders as its quoted text"
  (doc
    "A string BUILT at run time and returned as `main`'s value crosses the boundary as its proper
           String type and renders as the quoted canonical text — `(join \"hel\" \"lo\")` is
           \"hello\". This exercises the compiler-emitted, type-directed string renderer (the analogue
           of the `b\"…\"` Bytes renderer): a runtime String is walked byte-by-byte and quoted/escaped,
           byte-identical to the const `\"…\"` form. Pins RETURNING a runtime string (distinct from the
           cases above, which consume one to a scalar) — the compound-value output ABI for strings.")
  (input (do (def (join a b) (String.concat a b)) (def (main) (join "hel" "lo")) (export main)))
  (output (: "hello" String)))

(case
  "a returned runtime string with a multi-byte scalar renders the scalar verbatim"
  (doc
    "The rendered form of a runtime string passes a printable Unicode scalar through verbatim, not
           as an escape — `(id \"café\")` renders \"café\" (the é is its raw two-byte UTF-8, matching the
           const path's `{:?}`, which prints printable Unicode literally). Pins that the emitted string
           renderer's escaping agrees with the const renderer on multi-byte scalars, so a rendered string
           reads back to the same value.")
  (input (do (def (id s) (if true s s)) (def (main) (id "café")) (export main)))
  (output (: "café" String)))

(case
  "a returned runtime string with a non-printable scalar renders it verbatim, not a u-escape"
  (doc
    "A non-printable Unicode scalar (U+0007 BEL) renders VERBATIM as its raw byte, NOT as a
           u-escape: the reader recognizes exactly the closed escape set \\n \\t \\r \\\\ \\\" 
           (collections-and-text.md #A String Literal's Escapes Are A Closed Set), which has NO numeric
           escape, so a u-escape would read back as its literal characters rather than the BEL — the
           rendered string would NOT read back to the same value. (String.from-bytes (Bytes.of (list 97
           7 98))) is the 3-scalar string a-BEL-b; it renders with the BEL byte raw. Pins the round-trip
           the value-oracle gate now checks independently (re-reading the rendered text): a rendered
           string MUST read back to the same value, so the renderer emits ONLY the closed escapes.")
  (input
    (do
      (def (main) (Option.expect (String.from-bytes (Bytes.of #list(97 7 98))) "well-formed"))
      (export main)))
  (output (: "ab" String)))

; --- Decoding bytes to a string is TOTAL, never trapping ---------------------------------------
; collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping. Cadenza's String is
; guaranteed-well-formed (a sequence of Unicode scalar values, never invalid UTF-8), so turning a Bytes
; into a String is a CHECKED, PARTIAL operation — some byte sequences are not well-formed UTF-8. The
; language's rule is that this partiality is HANDLED, not trapped: `String.from-bytes` yields an
; `Option<String>` (`(Some s)` for well-formed input, `None` for ill-formed), and the `bin`-pattern
; `(utf8 s)` segment (options/binary-syntax/) is a NON-MATCH on ill-formed bytes so a match's
; exhaustiveness obligation (CDZ0210) forces a branch that handles the bad case. This is the principled
; alternative to a trapping decode: where a type can make the partiality explicit and force it to be
; handled, the language prefers that over a trap (it REFINES total-or-trap — a trap is what remains for
; partialities a type cannot surface, not the first resort). The
; `from-bytes`/`utf8`-segment decode is realized with the `bin` form (options/realized-capability-set/),
; so a generation without binary matching declines it.
(case
  "decoding well-formed UTF-8 bytes yields the string"
  (doc
    "`(String.from-bytes (Bytes.of (list 99 97 102 195 169)))` decodes the UTF-8 bytes of \"café\"
           (c a f, then é as the two bytes 0xC3 0xA9 = 195 169) to `(Some \"café\")`. Pins that a
           well-formed byte sequence decodes to `(Some s)` — the success arm of the total decode
           (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping).")
  (input (= (String.from-bytes (Bytes.of #list(99 97 102 195 169))) (Some "café")))
  (output (: true Bool)))

(case
  "decoding an EMPTY byte sequence yields Some of the empty string"
  (doc
    "The degenerate VALID boundary of the decode (the invalid cases below and the well-formed case
           above all feed NON-empty input): zero bytes is trivially well-formed UTF-8 — the empty string
           encodes to zero bytes — so `(String.from-bytes (Bytes.of (list)))` is `(Some "
    ")`, NOT None. A
           state-machine decoder with an off-by-one on the input length, or one that treated "
    consumed
    no
    bytes
    /
    produced
    no
    scalar"
    as
    a
    decode
    failure,
    would
    wrongly
    return
    None
    for
    empty
    input.
    Pins
    that
    empty
    is
    a
    valid
    decode
    (collections-and-text.md #Decoding Bytes To A String Is Total)
    .")
  (input (= (String.from-bytes (Bytes.of #list())) (Some "")))
  (output (: true Bool)))

(case
  "a runtime String.from-bytes result is read twice and both reads see the decoded string"
  (doc
    "The decode cases above feed a CONSTANT byte sequence (const-folds, no runtime path) and read the
           result once. This pins the RUNTIME + read-twice face: `String.from-bytes` is a source-CONSUMING
           runtime op (it takes ownership of the Bytes and yields a fresh owned String leaf), so its
           `let`/match-bound result read MORE THAN ONCE must be NAMED (decoded once, the handle read by each
           use), not copy-propagated + recomputed — recomputing a consuming op re-consumes its (already-moved)
           source. The bytes are built by recursion (`rep` appends 65='A' three times) so nothing folds; the
           decoded `s` is read by BOTH `String.byte-len` and `String.scalar-len` → 3 + 3 = 6. This is the
           `String.from-bytes` member of the same consuming-op-double-read family as adv-54 (String.slice/
           to-bytes): a copy-propagation that recomputed the decode would re-consume the bytes and the second
           read would see a freed/garbled leaf. Expected: 6.")
  (input
    (do
      (def
        (rep (: acc Bytes) (: n Int64))
        (if (< n 1) acc (rep (Bytes.concat acc (Bytes.of #list(65))) (- n 1))))
      (def
        (main (: k Int64))
        (match
          (String.from-bytes (rep (Bytes.of #list()) 3))
          ((Some s) (+ (String.byte-len s) (String.scalar-len s)))
          ((None u) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "to-bytes of a sliced multibyte string is read twice and both reads see the right bytes"
  (doc
    "The `String.slice`/`to-bytes` member of the consuming-op-double-read family (the sibling the
           from-bytes case above names): the to-bytes buffer of a String.slice VIEW must be independently
           readable N times. `tail` = slice(concat 'ab' 'cdé', 3, 5) = 'dé' (bytes [100, 0xC3, 0xA9]);
           `b = to-bytes tail`; `(Bytes.at b 0) + (Bytes.at b 1)` = 100 + 195 = 295. A slice is a rope VIEW
           (not a fresh owned string), so its to-bytes must materialize an independently-readable buffer,
           NOT a borrow the first read consumes — before the fix (adv-54, runtime StrSlice/StrToBytes
           binding kept, trunk c7a7861b4) wasm returned 100 (b[1] read 0 — the buffer was consumed by the
           first read); rust/rust-async were correct. MULTIBYTE + slice-VIEW + read-more-than-once is the
           trigger (an ASCII slice and a concat-owned source were always fine). Expected 295 on all three
           backends now.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((s (String.concat "ab" "cdé")))
          (match
            (String.slice s 3 5)
            ((Some tail)
              (let
                ((b (String.to-bytes tail)))
                (+
                  (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                  (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 295 Int64))
  (live-objects known-leak))

(case
  "Bytes.concat of two sliced-view to-bytes is read twice and both reads see the concatenated bytes"
  (doc
    "The `Bytes.concat` member of the consuming-op-double-read family (adv-54b, the next op after the
           to-bytes case above): a let-bound `Bytes.concat` whose OPERANDS are `String.to-bytes(String.slice
           …)` VIEWS, read more than once, must see the joined bytes on every read. `tail` = slice(concat
           'ab' 'cdé', 3, 5) = 'dé' = [100, 0xC3, 0xA9]; `b` = concat(to-bytes tail, to-bytes tail) =
           [100,0xC3,0xA9,100,0xC3,0xA9]; `b[0]+b[3]` = 100+100 = 200. Before the fix, `Core::BytesConcat`
           was NOT in `is_runtime_computation`, so the let-bound `b` was copy-propagated + RECOMPUTED at each
           `Bytes.at`, and the recompute CONSUMED the borrowed slice-view sources → the 2nd read walked a
           freed buffer → wasm OOB (rust computed 200). Fixed once adv-66 made BytesConcat CONSUMING in the
           dup pass, unblocking its keep-list entry (trunk 998329abf) — closing the adv-54/54b/66
           aliasing-consume arc. Now 200 on all three backends.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((s (String.concat "ab" "cdé")))
          (match
            (String.slice s 3 5)
            ((Some tail)
              (let
                ((b (Bytes.concat (String.to-bytes tail) (String.to-bytes tail))))
                (+
                  (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                  (Int64.of (Option.expect (Bytes.at b 3) "b3")))))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 200 Int64))
  (live-objects known-leak))

(case
  "a RUNTIME byte-slice torn mid-scalar is rejected by from-bytes at both tear points"
  (doc
    "The RUNTIME-torn face of the ill-formed-decode family: the constant malformed pins below feed
           hand-written bad byte lists; here the invalid sequence arises from SLICING a genuinely valid
           string's encoding mid-scalar. `\"aé🎵z\"` encodes as a|é(2 bytes)|🎵(4 bytes)|z; a 3-byte
           slice at start = 2 captures é's continuation byte + 🎵's first two (a bare continuation
           lead), and at start = 3 captures 🎵's first three bytes (a truncated 4-byte sequence) — both
           decode to None (−1), never a trap or a replacement-character string. Pins that the total
           decode's rejection reaches slices of REAL encodings cut at runtime offsets, not only
           synthetic constant byte lists.")
  (input
    (do
      (def
        (main (: start Int64))
        (let
          ((b (String.to-bytes (String.concat "aé🎵" "z"))))
          (match
            (Bytes.slice b start 3)
            ((Some cut)
              (match (String.from-bytes cut) ((Some t) (String.byte-len t)) ((None _u) -1)))
            ((None _u) -2))))
      (export main)))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "decoding ill-formed UTF-8 bytes yields none, not a trap"
  (doc
    "`(String.from-bytes (Bytes.of (list 255)))` is given 0xFF, which is not a well-formed UTF-8
           sequence (0xFF never appears in valid UTF-8), so the decode yields `None` — NOT a trap and NOT
           an unspecified string with a replacement character. Pins the failure arm as an ordinary value
           the program handles (collections-and-text.md #Decoding Bytes To A String Is Total, Not
           Trapping). This is the whole point of the total decode: ill-formed input is data, not a halt.")
  (input (= (String.from-bytes (Bytes.of #list(255))) None))
  (output (: true Bool)))

(case
  "decoding an overlong UTF-8 encoding yields none"
  (doc
    "`(Bytes.of (list 192 128))` is `C0 80` — the OVERLONG two-byte encoding of U+0000, which
           well-formed UTF-8 forbids (a code point must use its shortest encoding; NUL is the one-byte
           `00`). A decoder that only checked the leading/continuation byte STRUCTURE (a lead byte
           `110xxxxx` then a `10xxxxxx` continuation) would wrongly accept `C0 80`; strict UTF-8 (the
           Unicode definition, matching `str::from_utf8`) rejects it, so `String.from-bytes` yields
           `None`. Pins that the decode enforces shortest-form, not just byte shape — a security-relevant
           distinction (overlong encodings have been used to smuggle forbidden bytes past naive
           validators). This is a requirement on the runtime's UTF-8 validator the reader relies on:
           the byte sequence, not the code point, must be canonical.")
  (input (= (String.from-bytes (Bytes.of #list(192 128))) None))
  (output (: true Bool)))

(case
  "decoding a lone continuation byte yields none"
  (doc
    "`(Bytes.of (list 128))` is `0x80` — a CONTINUATION byte (`10xxxxxx`) with no preceding lead byte.
           A well-formed UTF-8 sequence never starts with a continuation, so `String.from-bytes` yields
           `None`. Distinct from the lone-`0xFF` case (0xFF is never a valid byte at all) and the overlong
           case (structurally-paired but non-canonical): here the byte is a valid CONTINUATION shape but
           appears with no lead to continue — the STATE-MACHINE failure mode a decoder that only rejected
           `0xFF`/overlong could miss. Pins that a stray continuation is rejected.")
  (input (= (String.from-bytes (Bytes.of #list(128))) None))
  (output (: true Bool)))

(case
  "decoding a truncated multi-byte sequence yields none"
  (doc
    "`(Bytes.of (list 195))` is `0xC3` — a 2-byte LEAD (`110xxxxx`) with NO following continuation byte
           (the sequence ends mid-codepoint). `é` needs `C3 A9`; `C3` alone is truncated, so
           `String.from-bytes` yields `None`. The dual of the lone-continuation case: a lead expecting a
           continuation that never arrives (a decode that ran off the end of the input). Together they pin
           BOTH state-machine failure faces — a continuation with no lead, and a lead with no continuation —
           beyond the byte-value (`0xFF`) and shortest-form (overlong) rejections above.")
  (input (= (String.from-bytes (Bytes.of #list(195))) None))
  (output (: true Bool)))

(case
  "decoding a surrogate code point encoded as UTF-8 yields none"
  (doc
    "`(Bytes.of (list 237 160 128))` is `ED A0 80` — the UTF-8-shaped encoding of U+D800, a HIGH
           SURROGATE. Surrogates are not Unicode scalar values (they exist only for UTF-16 pairing), so
           well-formed UTF-8 excludes them even though the three-byte structure `1110xxxx 10xxxxxx
           10xxxxxx` is superficially valid. Strict UTF-8 rejects the surrogate range U+D800..=U+DFFF, so
           `String.from-bytes` yields `None`. The decode companion of the `Char.from-int` surrogate case
           (which rejects U+D800 as data): a String is a sequence of scalar values, so a byte sequence
           encoding a surrogate is not a well-formed String. Pins that the runtime validator rejects
           surrogate encodings, not only structurally-broken bytes — the same Unicode-scalar boundary
           the char surface enforces, now on the byte-decode path the reader uses.")
  (input (= (String.from-bytes (Bytes.of #list(237 160 128))) None))
  (output (: true Bool)))

(case
  "decoding a four-byte sequence for a code point above U+10FFFF yields none"
  (doc
    "`(Bytes.of (list 244 144 128 128))` is `F4 90 80 80` — a structurally-valid 4-byte UTF-8 shape
           `11110xxx 10xxxxxx 10xxxxxx 10xxxxxx` whose decoded code point is U+110000, ONE PAST the maximum
           Unicode scalar U+10FFFF. Strict UTF-8 rejects it: the highest well-formed 4-byte lead is `F4`
           with a second byte at most `8F` (U+10FFFF), so `90` overflows the range. `String.from-bytes`
           yields `None`. The fourth failure mode of the total decode alongside invalid bytes, overlong
           encodings, and surrogates — a byte sequence whose STRUCTURE is valid but whose CODE POINT is out
           of range (the decode companion of the `Char.from-int 1114112` = U+110000 rejection). Pins that
           the validator checks the decoded scalar's range, not only the byte structure.")
  (input (= (String.from-bytes (Bytes.of #list(244 144 128 128))) None))
  (output (: true Bool)))

; The invalid-UTF8 cases above decode CONSTANT `(Bytes.of (list …))` literals, which the fold can validate at
; compile time. A GENUINELY-runtime byte ROPE — a `Bytes.concat` of chunks chosen by a run-time `if` (a
; folded-literal concat would collapse back to a literal) — reaches the emitted `str-from-bytes` decode as a
; multi-chunk deferred concatenation, so the UTF-8 validator must walk the logical bytes ACROSS the leaf seam.
; The sharpest face: a multi-byte scalar STRADDLING the seam — its lead byte the last of the left chunk, its
; continuation the first of the right — must decode as one scalar (a validator that assumed a scalar lies
; within a single leaf would wrongly reject it). Pins the runtime decode over a rope, including the seam-straddle.
(case
  "String.from-bytes validates a multi-byte scalar straddling a runtime byte-rope's seam"
  (doc
    "Over a rope `(Bytes.concat left right)` assembled at run time (the left chunk chosen by a run-time
           `if`, so the concat cannot fold): `sel`=0 builds `[99, 195] ++ [169]` — the é lead `C3`(195) ends
           the left chunk, its continuation `A9`(169) starts the right — a VALID `cé` across the seam →
           Some (→ 1). `sel`=1 builds `[99, 195] ++ [99]` — the lead `C3` then a NON-continuation `c`(99)
           across the seam — INVALID (a lead with no continuation) → None (→ 0). Pins that the runtime
           `str-from-bytes` decode walks the logical bytes across the leaf boundary and validates a scalar
           that spans the seam, matching the const decode of the same byte sequence. Both backends via the
           `(call)` form (a nullary rope const-folds; a runtime-selected chunk forces the genuine rope).")
  (input
    (do
      (def (pickb (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
      (def (validq (: b Bytes)) (match (String.from-bytes b) ((Some _s) 1) ((None) 0)))
      (def
        (main (: sel Int64))
        (validq
          (Bytes.concat
            (Bytes.of #list(99 195))
            (pickb sel (Bytes.of #list(169)) (Bytes.of #list(99))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "encoding a decoded string round-trips to the same bytes"
  (doc
    "For well-formed bytes `b`, decoding then re-encoding yields `b`: matching the `(Some s)` arm of
           `(String.from-bytes b)` and taking `(String.to-bytes s)` gives back the original UTF-8 bytes.
           Pins encode as the inverse of decode-of-well-formed (collections-and-text.md #Decoding Bytes To
           A String Is Total, Not Trapping, 3rd sentence).")
  (input
    (match
      (String.from-bytes (Bytes.of #list(99 97 102 195 169)))
      ((Some s) (= (String.to-bytes s) (Bytes.of #list(99 97 102 195 169))))
      ((None _) false)))
  (output (: true Bool)))

(case
  "decoding an encoded string round-trips to the same bytes (the inverse direction)"
  (doc
    "The INVERSE round-trip of the case above: for a String `s`, ENCODING then DECODING then
           re-encoding yields `s`'s bytes. `(String.to-bytes \"café\")` is the 5-byte UTF-8 `[99 97 102 195
           169]` (é is the 2-byte `C3 A9`); `String.from-bytes` decodes those well-formed bytes back to a
           `Some s'`, and `(String.to-bytes s')` re-encodes to the SAME 5 bytes. Pins that decode is the
           inverse of encode-of-a-string (collections-and-text.md #Decoding Bytes To A String Is Total — the
           bijection in the other direction from the decode-then-encode case above), the shape the compiler
           takes encoding an export name to UTF-8 and reading it back.")
  (input
    (match
      (String.from-bytes (String.to-bytes "café"))
      ((Some s) (= (String.to-bytes s) (Bytes.of #list(99 97 102 195 169))))
      ((None _) false)))
  (output (: true Bool)))

; --- KNOWN GAP (operator ruling 2026-07-18): String.from-bytes does NOT NFC-normalize --------------
; A string LITERAL is NFC-normalized at parse time (the reader; 01-literals + the normalization cases
; above), so two literals differing only in normalization denote ONE value. `String.from-bytes` is
; DIFFERENT: it is a FAITHFUL byte decode — it validates UTF-8 well-formedness (the None cases above) but
; PRESERVES the exact scalar sequence, WITHOUT re-normalizing to NFC. This is a DELIBERATE known gap, not a
; bug: NFC normalization needs the Unicode canonical-composition DATA TABLES, and the operator ruled that
; carrying those in the dependency-free compiler core (which must port cleanly to the Cadenza self-host) is
; not worth the bloat right now. CONSEQUENCE: `from-bytes` obeys "equality follows normalization" only if
; the input bytes were ALREADY NFC — normalization is the CALLER'S responsibility on the from-bytes path
; (a literal on the reader path is already normalized; bytes from an external source may not be). So a
; DECOMPOSED "café" (e + U+0301, bytes `63 61 66 65 CC 81`) decodes to a String that is NOT `=` to the
; composed literal "café" (U+00E9) — because the seed does not normalize either side of `from-bytes`, the
; decoded decomposed form keeps its 6 bytes / 5 scalars and differs from the 5-byte / 4-scalar NFC literal.
; (If the "equal strings differing only in normalization" cases above are ever satisfied by adding NFC at
; the reader, this from-bytes case STILL declines-to-normalize until NFC is carried into the core — track
; both as the same Unicode-normalization gap.) A future `from-bytes-normalizing` (or carrying the tables)
; would flip this; a `from-bytes-raw` escape hatch is MOOT while the default already preserves bytes.
(case
  "String.from-bytes does not NFC-normalize — a decomposed form stays distinct (known gap)"
  (doc
    "`from-bytes` is a faithful byte decode: it does NOT re-normalize to NFC (a deliberate known gap —
           the Unicode composition tables would bloat the dependency-free core; operator ruling 2026-07-18).
           So decoding the DECOMPOSED \"café\" bytes (`e` + U+0301 combining acute = `… 101 204 129`) yields
           a String whose scalars are the decomposed sequence, which is NOT `=` to the COMPOSED literal
           \"café\" (U+00E9) — the seed normalizes NEITHER side of from-bytes. NFC is the caller's
           responsibility on the from-bytes path (a literal is normalized at parse; external bytes are not).
           Pins the non-normalization as INTENDED, documented behavior: `(= (from-bytes decomposed) \"café\")`
           is FALSE. Contrast the well-formed-decode case above (composed bytes → Some \"café\", which DOES
           equal the literal because those bytes are already NFC). Flips only when NFC is carried into the
           core; a from-bytes-raw hatch is moot while the default preserves bytes.")
  (input (= (String.from-bytes (Bytes.of #list(99 97 102 101 204 129))) (Some "café")))
  (output (: false Bool)))

(case
  "an encode-decode round-trip preserves an ASCII string's byte length"
  (doc
    "The length face of the inverse round-trip: `(String.from-bytes (String.to-bytes \"hi\"))` decodes
           the encoded bytes back to a `Some s'` whose `String.byte-len` is 2 — the original length. Pins
           that a to-bytes→from-bytes round-trip yields a usable String of the right size (measured, not
           `=`-compared), the length companion of the byte-preservation case above.")
  (input
    (do
      (def
        (main)
        (match (String.from-bytes (String.to-bytes "hi")) ((Some s) (String.byte-len s)) (None -1)))
      (export main)))
  (output (: 2 Int64)))

(case
  "a Bytes.slice into the to-bytes output decodes at a scalar boundary and rejects mid-scalar"
  (doc
    "The window-alignment discriminator: `(String.to-bytes \"caféx\")` is 6 bytes (é = 0xC3 0xA9 at
           offsets 3-4). A 2-byte `Bytes.slice` at a=3 captures the COMPLETE é sequence — from-bytes
           decodes it (byte-len 2); at a=2 the window is `('f' = 0x66, then é's lead byte 0xC3)` — the é's lead byte with no
           continuation — so from-bytes answers None (-9), the total-decode contract on an ill-formed
           window. One compiled slice+decode witnesses both the aligned and the mid-scalar cut per call;
           an offset drift of one byte flips both answers.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (String.to-bytes "caféx") a 2)
          ((Some w) (match (String.from-bytes w) ((Some s) (String.byte-len s)) ((None u) -9)))
          ((None u) -1)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2 Int64))
  (call main (: 2 Int64))
  (output (: -9 Int64))
  (live-objects known-leak))

(case
  "String.from-bytes decodes a RUNTIME byte sequence built by a recursive appender"
  (doc
    "The runtime-Bytes decode path: `String.from-bytes` of a byte buffer the compiler CANNOT fold to
           a constant — here `(rep b\"\\x68\" 3)` recursively appends the byte `0x69` ('i') three times to a
           leading `0x68` ('h'), building the rope `\"hiii\"` at run time. A constant `(Bytes.of …)` folds via
           `std::str::from_utf8` in the compiler; a genuinely runtime buffer instead lowers to the runtime
           `str-from-bytes` op (strict UTF-8 validation + a zero-copy re-tag of the validated buffer as a
           String — a String IS a UTF-8 Bytes leaf). Pins that a runtime-computed Bytes decodes to the same
           `Some s` the constant fold would (collections-and-text.md #Decoding Bytes To A String Is Total,
           Not Trapping), the shape a self-hosted reader materializes an interned name with.")
  (input
    (do
      (def (rep (: acc Bytes) (: n Int64)) (if (= n 0) acc (rep (Bytes.concat acc b"i") (- n 1))))
      (def (main) (Option.expect (String.from-bytes (rep b"h" 3)) "well-formed"))
      (export main)))
  (output (: "hiii" String))
  (live-objects known-leak))

(case
  "String.from-bytes of an ill-formed RUNTIME byte sequence yields None (never traps)"
  (doc
    "The ill-formed runtime companion: a byte buffer built at run time (a recursive appender of the
           invalid lead byte `0xFF`, which the compiler cannot fold) is NOT well-formed UTF-8, so the runtime
           `str-from-bytes` op returns the NULL sentinel and the compiler builds `(None unit)` — the helper's
           `None` arm returns -1. A TOTAL decode, never a trap, on the RUNTIME path exactly as on the constant
           path (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). Pins that the
           runtime UTF-8 validator (matching `std::str::from_utf8`) drives the fallible decode's `None` for a
           value the compiler could not classify at compile time.")
  (input
    (do
      (def
        (rep (: acc Bytes) (: n Int64))
        (if (= n 0) acc (rep (Bytes.concat acc b"\xff") (- n 1))))
      (def
        (main)
        (match (String.from-bytes (rep b"" 2)) ((Some s) (String.byte-len s)) ((None _) -1)))
      (export main)))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "String.from-bytes decodes a multibyte RUNTIME Bytes ROPE, flattening before validation"
  (doc
    "Exercises `str-from-bytes` on a runtime Bytes ROPE (a `Bytes.concat` tree) whose logical bytes
           span multiple leaves — the op FLATTENS the rope before strict UTF-8 validation, because a rope
           node's raw storage holds header bytes, not content. `(Bytes.concat b\"caf\" b\"\\xc3\\xa9\")` built
           through a recursive appender is the 5-byte UTF-8 of \"café\" (é is the 2-byte `C3 A9`) split across
           rope leaves; decoding it yields `Some s` whose `String.byte-len` is 5. Pins that the runtime
           decode sees the actual content of a shared/spliced buffer, not header bytes (the shape the reader
           hits decoding a name sliced out of a larger input buffer).")
  (input
    (do
      (def
        (build (: acc Bytes) (: n Int64))
        (if (= n 0) acc (build (Bytes.concat acc b"\xc3\xa9") (- n 1))))
      (def
        (main)
        (match (String.from-bytes (build b"caf" 1)) ((Some s) (String.byte-len s)) ((None _) -1)))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a helper decodes bytes to a string and consumes the fallible result"
  (doc
    "The reader's symbol-table idiom: a helper takes raw `Bytes` (a slice of the input), decodes
           them with `String.from-bytes`, and `match`es the fallible result — binding the string in the
           `Some` arm to measure/intern it, and handling malformed bytes in the `None` arm. `(dec (Bytes.of
           (list 104 105)))` decodes \"hi\" and returns its byte length 2. Pins `String.from-bytes`
           consumed THROUGH A FUNCTION BOUNDARY (the shape a self-hosted reader materializes its symbol
           table with), not only at the entrypoint: decoding at `main` directly and matching there works,
           but the same decode-and-match inside a called helper does not yet — the fallible decode's result
           must survive the boundary the way `Bytes.at`/`List.at` results now do. Companion of the
           round-trip case above, which matches `from-bytes` at `main`; this one crosses a call.")
  (input
    (do
      (def (dec b) (match (String.from-bytes b) ((Some s) (String.byte-len s)) ((None _) -1)))
      (def (main) (dec (Bytes.of #list(104 105))))
      (export main)))
  (output (: 2 Int64)))

(case
  "decoding ill-formed bytes through a helper takes the None arm"
  (doc
    "The ill-formed companion: `String.from-bytes` of a RUNTIME Bytes that is not well-formed
           UTF-8 yields `(None unit)`, so the helper's `None` arm returns -1 — a TOTAL decode, never a
           trap (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). `(list
           255)` is a lone `0xFF`, an invalid lead byte. Pins that the runtime UTF-8 validator (emitted
           inline, matching `std::str::from_utf8` — rejecting invalid leads, overlong forms, surrogates,
           and code points > U+10FFFF) drives the fallible decode's `None`, so a reader handles
           malformed input rather than trapping on it. Companion of the well-formed case above.")
  (input
    (do
      (def (dec b) (match (String.from-bytes b) ((Some s) (String.byte-len s)) ((None _) -1)))
      (def (main) (dec (Bytes.of #list(255))))
      (export main)))
  (output (: -1 Int64)))

; The helper cases above decode a FLAT byte leaf (`Bytes.of (list …)`). A `String.from-bytes` over a
; ROPE input — a `Bytes.concat` tree whose `raw` holds header bytes, NOT content — exercises a distinct
; runtime path: `op_str_from_bytes` must `bytes_flatten` the rope BEFORE the strict UTF-8 validate (else
; it would validate the header bytes as UTF-8, garbage). These pin the rope-input decode (well-formed and
; ill-formed), with a runtime `UInt8.wrap`'d element so the Bytes is genuinely runtime-built (not folded).
(case
  "decoding a runtime ROPE Bytes as UTF-8 flattens before validating (well-formed)"
  (doc
    "`String.from-bytes` of a `Bytes.concat` ROPE ([104,105] ++ [n]) with a runtime UInt8 element n:
           the op flattens the rope to its content bytes THEN validates. n=33 (`!`) → the 3 bytes \"hi!\"
           are well-formed UTF-8 → `(Some s)`, byte-len 3. Pins that a rope input decodes by CONTENT, not by
           its concat-node header bytes (which `bytes_flatten` materializes first).")
  (input
    (do
      (def
        (mk (: n Int64))
        (Bytes.concat
          (Bytes.of #list((UInt8.wrap 104) (UInt8.wrap 105)))
          (Bytes.of #list((UInt8.wrap n)))))
      (def
        (main (: n Int64))
        (match (String.from-bytes (mk n)) ((Some s) (String.byte-len s)) ((None _) -1)))
      (export main)))
  (call main (: 33 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "decoding a runtime ROPE Bytes that is ill-formed yields none"
  (doc
    "The ill-formed rope companion: a runtime `Bytes.of (list (UInt8.wrap n) 255)` whose second byte
           is `0xFF` (an invalid UTF-8 lead) → `None` → -1. The rope/runtime-element form of the total
           decode's failure arm; the flatten-then-validate path rejects malformed content, never traps.")
  (input
    (do
      (def (mk (: n Int64)) (Bytes.of #list((UInt8.wrap n) (UInt8.wrap 255))))
      (def
        (main (: n Int64))
        (match (String.from-bytes (mk n)) ((Some s) (String.byte-len s)) ((None _) -1)))
      (export main)))
  (call main (: 104 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a utf8 bin segment binds a decoded string when the bytes are well-formed"
  (doc
    "The `bin` pattern `(bin (u8 n) (utf8 name n))` reads a length byte n, then decodes exactly the
           next n bytes as UTF-8 into `name : String`. Against `(list 3 102 111 111)` — n=3, then the
           ASCII bytes of \"foo\" — the `utf8` segment matches and binds name = \"foo\". Pins the
           string-typed binary segment (options/binary-syntax/), the decode built into pattern matching.")
  (input (match (Bytes.of #list(3 102 111 111)) ((bin (u8 n) (utf8 name n)) name) (_ "invalid")))
  (output (: "foo" String)))

(case
  "a utf8 bin segment is a non-match on ill-formed bytes, forcing the catch-all"
  (doc
    "The same pattern `(bin (u8 n) (utf8 name n))` against `(list 1 255)` — n=1, then the byte 0xFF,
           which is not well-formed UTF-8 — does NOT match the `utf8` segment, so control falls to the
           catch-all and yields \"invalid\". The decode failure is a NON-MATCH, never a trap; because the
           match must be exhaustive (CDZ0210), a catch-all is required, so the ill-formed case is
           necessarily handled (collections-and-text.md #Decoding Bytes To A String Is Total, Not
           Trapping — the exhaustiveness clause). This is how binary matching absorbs invalid UTF-8.")
  (input (match (Bytes.of #list(1 255)) ((bin (u8 n) (utf8 name n)) name) (_ "invalid")))
  (output (: "invalid" String)))

(case
  "a utf8-decoding bin match with no catch-all is non-exhaustive"
  (doc
    "A `bin` pattern with a `(utf8 …)` segment can fail to match on ill-formed bytes, so a match
           whose only arm is such a pattern does not cover every byte sequence and is rejected CDZ0210 —
           the same exhaustiveness rule every `bin` match obeys, made pointed by the fact that the decode
           itself is a source of non-match. Pins that the ill-formed-UTF-8 case cannot be silently
           dropped: the compiler forces a branch for it (collections-and-text.md #Decoding Bytes To A
           String Is Total, Not Trapping).")
  (input (match (Bytes.of #list(3 102 111 111)) ((bin (u8 n) (utf8 name n)) name)))
  (error CDZ0210))

(case
  "two dependent utf8 bin segments each sized by its own preceding length byte"
  (doc
    "The dependent-cursor face: a `bin` pattern reads a length byte, decodes that many bytes, then
           reads a SECOND length byte and decodes that many more — `(bin (u8 a) (utf8 s1 a) (u8 b) (utf8 s2
           b))`. The second segment's length `b` is a value read AFTER the first segment consumed `a` bytes,
           so the match cursor must advance past `s1` before reading `b`. Against `(list 2 65 66 1 67)` —
           a=2 → \"AB\", then b=1 → \"C\" — binds s1=\"AB\", s2=\"C\", and `(String.concat s1 s2)` = \"ABC\".
           Extends the single-dependent `(bin (u8 n) (utf8 name n))` case to a MULTI-segment pattern where a
           later segment's length depends on a value the pattern read earlier, pinning that the decode cursor
           threads correctly across segments (options/binary-syntax/).")
  (input
    (match
      (Bytes.of #list(2 65 66 1 67))
      ((bin (u8 a) (utf8 s1 a) (u8 b) (utf8 s2 b)) (String.concat s1 s2))
      (_ "x")))
  (output (: "ABC" String)))

(case
  "a dependent utf8 bin segment whose length exceeds the remaining bytes is a non-match"
  (doc
    "The same `(bin (u8 n) (utf8 name n))` pattern against `(list 5 102 111)` — n=5 but only 2 bytes
           follow — cannot read the 5 bytes the length demands, so the `utf8` segment does NOT match and
           control falls to the catch-all, yielding \"invalid\". The over-length read is a NON-MATCH, never
           a trap or an out-of-bounds read past the buffer: a dependent length that outruns the input is
           handled by the same exhaustiveness-required catch-all every `bin` match obeys (the length-overrun
           companion of the ill-formed-UTF-8 non-match; collections-and-text.md #Decoding Bytes To A String
           Is Total, Not Trapping).")
  (input (match (Bytes.of #list(5 102 111)) ((bin (u8 n) (utf8 name n)) name) (_ "invalid")))
  (output (: "invalid" String)))

; --- Char — a single validated Unicode scalar, the element type of a string's scalar sequence ----
; collections-and-text.md #A Char Is A Single Unicode Scalar Value: a `Char` is one Unicode scalar
; (a code point in U+0000..=U+10FFFF EXCLUDING the surrogate range U+D800..=U+DFFF), the element type
; of the sequence a String already is (#A String Is A Sequence Of Unicode Scalar Values). The
; type-mapping table already carried a `char` boundary row with no surface producer; `Char` is that
; producer (spec/learnings/2026-07-05-char-is-a-validated-unicode-scalar-the-boundary-already-
; promises.md). A char literal is written `#\<scalar>` (options/char-literal-syntax/hash-scalar-
; literal.md); `Char` is a prelude record, so `Char.to-int` is `(. Char to-int)` and `Char.from-int`
; is `(. Char from-int)`, and `String.scalar-at` reads one scalar of a string.
;
; `chars` is a FRESH capability the seed does not realize (distinct from the realized
; `collections`). A later generation realizes scalar access and the `Char` value form; until then the
; seed DECLINES these — they pin the contract the realization must meet.
(case
  "reading a string's scalar in bounds yields Some of the char"
  (doc
    "Witnesses collections-and-text.md #A String's Scalars Are Addressable: `(String.scalar-at
           \"hello\" 1)` reads the scalar at scalar-position 1 — the char `#\\e` — wrapped in Some (an
           Option<Char>, the fallible read analogous to List.at and String.at). This is the operation
           that was missing: String.scalar-len counted scalars but nothing returned one.")
  (input (String.scalar-at "hello" 1))
  (output (: (Some #\e) (Option Char))))

(case
  "reading a string's scalar out of bounds yields None"
  (doc
    "The out-of-bounds companion: `(String.scalar-at \"hi\" 5)` reads past the end of a two-scalar
           string, so it yields None rather than trapping (collections-and-text.md #A String's Scalars
           Are Addressable — reading is total, and #Indexing And Lookup Are Fallible, Not Trapping). The
           Char analogue of the out-of-range String.at / List.at Nones.")
  (input (String.scalar-at "hi" 5))
  (output (: (None unit) (Option Char))))

(case
  "reading a string's scalar addresses scalar values, not bytes"
  (doc
    "`(String.scalar-at \"café\" 3)` is `(Some #\\é)`: the string is four scalar values (c, a, f,
           é) and scalar-position 3 is é — even though é occupies bytes 3–4 of the five-byte UTF-8
           encoding, so a byte offset would land mid-scalar. Pins that scalar access addresses by scalar
           value (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), returning a
           Char — the Char companion of the scalar-indexed String.at case above.")
  (input (String.scalar-at "café" 3))
  (output (: (Some #\é) (Option Char))))

; The three scalar-at cases above use a CONSTANT string AND a CONSTANT index, so they fold to a
; `Leaf::Char` at compile time (`lower_str_scalar_at`) and never exercise a runtime read. `String.scalar-at`
; with a RUNTIME index EXECUTES: it emits `Core::StrScalarAt`, which calls the runtime `bytes-scalar-at` op
; (wasm) / `chars().nth` (rust) to read the code-point and boxes it into a `Char`, mapping the out-of-range
; sentinel to `None` — building the `(Option Char)`. A runtime `Char` renders as its `#\c` literal (char-as-bool
; render tag-19; #5852 producer + #5932 emit): char is an int at runtime with a distinct RENDER tag, exactly as
; `Bool` is an i32 that renders `true`/`false`. (Earlier this DECLINED under the char-rep WONTFIX; the operator
; retracted that — char-as-bool — so it now computes + renders end-to-end.)
(case
  "String.scalar-at over a runtime index reads the scalar-th Unicode scalar as a Char, fallibly"
  (doc
    "`(String.scalar-at \"café\" i)` with `i` a runtime Int64 PARAMETER cannot fold, so it EXECUTES the
           runtime read: `Core::StrScalarAt` calls `bytes-scalar-at` (wasm) / `chars().nth` (rust), boxes the
           code-point into a `Char`, and maps out-of-range to `None` — a `(Option Char)`. `\"café\"` is four
           scalars (c, a, f, é), so index 3 is `é` → `(Some #\\é)`, index 0 is `c` → `(Some #\\c)`, and an
           out-of-range index (9) → `(None unit)`. A runtime `Char` renders as its `#\\c` literal (char-as-bool
           render tag; #5852 + #5932). Companion to the constant-index `(String.scalar-at \"café\" 3)` fold
           above (same `(Some #\\é)` value, folded at compile time) and to the runtime-char MATCH case below.
           INTERIM: each call currently over-retains 1 live object (the `Some`/`None` result cell is not
           reclaimed on result-teardown), pinned `known-leak 1 1 1` — to be TIGHTENED to 0 when v-rust-backend's
           `Core::StrScalarAt` Some/box reclaim fix lands. A LEAK, not a UAF (values correct on both backends;
           operator seq-278 tolerates interim over-retention).")
  (input (do (def (main (: i Int64)) (String.scalar-at "café" i)) (export main)))
  (call main (: 3 Int64))
  (output (: (Some #\é) (Option Char)))
  (call main (: 0 Int64))
  (output (: (Some #\c) (Option Char)))
  (call main (: 9 Int64))
  (output (: (None unit) (Option Char)))
  (live-objects known-leak))

(case
  "converting a char to its integer scalar value is total"
  (doc
    "Witnesses collections-and-text.md #A Char Converts To And From An Integer Totally:
           `(Char.to-int #\\a)` is 97 — the Unicode scalar value (code point) of the char `a`. Total:
           every char is a scalar value that has an integer code point, so to-int never fails.")
  (input (Char.to-int #\a))
  (output (: 97 Int64)))

; A Char compared/equated to a NUMBER (`(< c 1)`, `(= c 5)`, `(> 0 c)`) is CDZ0203 — a char and a number
; are not comparable — and, at parity with the arithmetic Char-vs-number coercion, carries the same
; `Char.to-int` WRAP fix (`…` marks where the char operand goes). Comparing two Chars is VALID (a Char has a
; defined order). (Migrated from rcdzc a_char_compared_to_a_number_offers_the_char_to_int_conversion_like_arithmetic_does;
; the "not the raw same-type-here unify wording" facet — a message-absence — is subsumed by the positive
; "a character and a number are not comparable" lead asserted here.)
(case
  "a char compared to a number is not comparable and offers the Char.to-int wrap fix"
  (input (do (def (f (: c Char)) (< c 1)) (def (main) 0) (export main)))
  (error
    CDZ0203
    (message "a character and a number are not comparable")
    (fix (kind wrap) (replacement "(Char.to-int …)"))))

(case
  "a char equated to a number offers the Char.to-int wrap fix"
  (input (do (def (f (: c Char)) (= c 5)) (def (main) 0) (export main)))
  (error
    CDZ0203
    (message "a character and a number are not comparable")
    (fix (kind wrap) (replacement "(Char.to-int …)"))))

(case
  "a number-first char comparison offers the Char.to-int wrap fix (either operand order)"
  (input (do (def (f (: c Char)) (> 0 c)) (def (main) 0) (export main)))
  (error
    CDZ0203
    (message "a character and a number are not comparable")
    (fix (kind wrap) (replacement "(Char.to-int …)"))))

(case
  "comparing two Chars is valid (a Char has a defined order)"
  (input (do (def (f (: a Char) (: b Char)) (< a b)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; `Char.to-int : Char -> Int64` and `Symbol.to-string : Symbol -> String` are TOTAL prelude conversions, so
; a Char where Int64 is wanted (`(+ #\a 1)`) / a Symbol where String is wanted gets the corresponding WRAP
; fix (the char/symbol twins of the String->Bytes coercion). The arg lead names the readable "this argument
; is a Char, but a value of type Int64 is expected here". When the expected int is NARROWER than Int64 (the
; wrap yields Int64), the int-width `.of` coercion takes over instead. (Migrated from rcdzc
; a_char_or_symbol_where_a_scalar_string_is_expected_offers_its_total_conversion.)
(case
  "a char where an integer is expected offers the Char.to-int wrap fix"
  (input (do (def (main) (+ #\a 1)) (export main)))
  (error
    CDZ0203
    (message "this argument is a Char, but a value of type Int64 is expected here")
    (fix (kind wrap) (replacement "(Char.to-int …)"))))

(case
  "a symbol where a string is expected offers the Symbol.to-string wrap fix"
  (input (do (def (f (: s String)) s) (def (g (: sym Symbol)) (f sym)) (export g)))
  (error CDZ0203 (fix (kind wrap) (replacement "(Symbol.to-string …)"))))

(case
  "a char summed into a NARROW int takes the int-width coercion, not Char.to-int"
  (input (do (def (g (: n Int8)) (+ n (Char.to-int #\a))) (export g)))
  (error CDZ0301 (fix (kind wrap) (replacement-contains "Int8.of"))))

(case
  "Char.to-int reads a genuinely-runtime char (if-selected) by scalar value"
  (doc
    "The runtime Char rep (Char-rep 1/N): a char chosen at RUN TIME — `(if b #\\a #\\z)` unifies two
           char literals into ONE `Char` value whose identity is known only at run time, so it is NOT
           constant-folded — occupies an i32 machine slot holding its Unicode code point. `Char.to-int` on
           it zero-extends that slot to the `Int64` result. `b`=true -> #\\a -> 97; `b`=false -> #\\z -> 122.
           Pins that a runtime char has a real scalar slot and `Char.to-int` reads it on every backend
           (upgrading the long-standing `runtime char declines pending the Char rep` boundary for this
           read path — the `if`-join char source; the runtime `Char.from-int` path now computes too and
           its boundary sweep is fenced in the from-int family below).")
  (input (do (def (main (: b Bool)) (Char.to-int (if b #\a #\z))) (export main)))
  (call main (: true Bool))
  (output (: 97 Int64))
  (call main (: false Bool))
  (output (: 122 Int64)))

(case
  "runtime Char equality and ordering compare by Unicode code point (scalar i32)"
  (doc
    "Char-rep 2/N: a runtime char — the `(if b …)`-join is not constant-folded — occupies a scalar i32
           code-point slot, so `=` compares by `i32.eq` and `<`/`>`/`<=`/`>=` by the signed-i32 order, which
           matches Unicode-scalar (code-point) order since code points are 0..=0x10FFFF (never negative).
           `b`=true selects #\\a (97): `#\\a = #\\a` is true (1) and `#\\a < #\\m` (97<109) is true (1) ->
           10*1+1 = 11. `b`=false selects #\\z (122): `#\\z = #\\a` is false (0) and `#\\z < #\\m` (122>109)
           is false (0) -> 0. Two distinct outputs pin BOTH equality and ordering on a RUNTIME char, upgrading
           the compound-path decline (`comparison of a compound value needs a heap walk`); routes through the
           same scalar path an Int/Bool/enum-disc takes (is_scalar now includes Ty::Char). Distinct from a
           constant char-literal compare, which folds at compile time.")
  (input
    (do
      (def
        (main (: b Bool))
        (+ (* 10 (if (= (if b #\a #\z) #\a) 1 0)) (if (< (if b #\a #\z) #\m) 1 0)))
      (export main)))
  (call main (: true Bool))
  (output (: 11 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

(case
  "runtime char-literal match dispatches by Unicode code point"
  (doc
    "Char-rep 3/N: a runtime char scrutinee — the `(if b …)`-join is not constant-folded — drives a
           char-literal `match`. Each arm tests the i32 code-point scrutinee against its char literal
           (`const code-point ; i32.eq` on wasm; a native `char` literal pattern on rust). `b`=true selects
           #\\a → the #\\a arm → 1; `b`=false selects #\\z → the #\\z arm → 2; the `_` wildcard covers the
           rest. Pins runtime char-literal MATCH — reachable now that is_scalar routes a Char scrutinee to
           the scalar-match path (2/N) and the scrutinee slot / probe constants ground to i32 (3/N). A
           CONSTANT char match folds at compile time; this is the runtime dispatch.")
  (input (do (def (main (: b Bool)) (match (if b #\a #\z) (#\a 1) (#\z 2) (_ 0))) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

(case
  "converting a scalar-valued integer to a char yields Some"
  (doc
    "`(Char.from-int 97)` is `(Some #\\a)` — 97 is the scalar value of `a`, a valid Unicode scalar,
           so the conversion succeeds (collections-and-text.md #A Char Converts To And From An Integer
           Totally). from-int is FALLIBLE (returns an Option) because not every integer is a scalar; this
           is the success arm.")
  (input (Char.from-int 97))
  (output (: (Some #\a) (Option Char))))

(case
  "converting a surrogate code point to a char yields None"
  (doc
    "`(Char.from-int 55296)` — 55296 is U+D800, a HIGH SURROGATE, which is NOT a Unicode scalar
           value — so from-int yields None rather than producing an invalid char (collections-and-text.md
           #A Char Converts To And From An Integer Totally, and #A Char Is A Single Unicode Scalar Value:
           surrogates are excluded). Pins that the surrogate range is rejected as data (None), never a
           trap and never an ill-formed Char. This is why from-int must be fallible.")
  (input (Char.from-int 55296))
  (output (: (None unit) (Option Char))))

(case
  "converting an out-of-range integer to a char yields None"
  (doc
    "`(Char.from-int 1114112)` — 1114112 is U+110000, one past the maximum scalar U+10FFFF — so it
           is not a scalar value and from-int yields None (collections-and-text.md #A Char Converts To
           And From An Integer Totally). The high-end companion of the surrogate case; both are handled
           as data, not traps.")
  (input (Char.from-int 1114112))
  (output (: (None unit) (Option Char))))

(case
  "converting a NEGATIVE integer to a char yields None"
  (doc
    "The LOW-end companion of the out-of-range cases above (which pin the high end U+10FFFF/U+110000
           and the surrogate block): no negative integer is a Unicode scalar value, so `(Char.from-int -1)`
           yields None — handled as data, not a trap, and NOT wrapped to a huge unsigned value that might
           alias a valid scalar. Pins the lower bound of the valid-scalar check.")
  (input (match (Char.from-int -1) ((Some c) (Char.to-int c)) ((None u) -1)))
  (output (: -1 Int64)))

(case
  "converting zero to a char yields Some — U+0000 (NUL) is a valid scalar"
  (doc
    "The load-bearing low-boundary pin: U+0000 (NUL) IS a valid Unicode scalar and MUST convert, so
           `(Char.from-int 0)` is `Some` and its `Char.to-int` round-trips to 0. A lower-bound check written
           `> 0` instead of `>= 0`, or one that excluded NUL as a control character, would wrongly reject it.
           The accept-side companion of the negative-rejection case (collections-and-text.md #A Char Converts
           To And From An Integer Totally).")
  (input (match (Char.from-int 0) ((Some c) (Char.to-int c)) ((None u) -1)))
  (output (: 0 Int64)))

; The cases above use U+D800 (first surrogate) and U+110000 (one PAST the max). These pin the EXACT
; boundaries where an off-by-one in the range/surrogate check surfaces: U+10FFFF (the MAXIMUM valid scalar,
; one below the U+110000 rejection) is Some; U+DFFF (the LAST surrogate, the block's upper endpoint) is
; None; and U+E000 (the first scalar AFTER the surrogate block) is Some. A check written `< 0x110000`
; instead of `<= 0x10FFFF`, or a surrogate block off by one at either end, flips one of these.
(case
  "Char.from-int at the maximum valid scalar U+10FFFF is Some"
  (doc
    "`(Char.from-int 1114111)` — 1114111 is U+10FFFF, the MAXIMUM Unicode scalar value — is a valid
           scalar, so from-int yields Some. The just-below-the-ceiling companion of the U+110000 (1114112)
           rejection above: 10FFFF is IN range, 110000 is one PAST. Pins the exact upper boundary of the
           valid-scalar check (`<= 0x10FFFF`, not `< 0x110000` off-by-one — both reject 110000 but only the
           correct bound accepts 10FFFF).")
  (input (match (Char.from-int 1114111) ((Some _) 1) ((None _) 0)))
  (output (: 1 Int64)))

(case
  "Char.from-int at the last surrogate U+DFFF is None"
  (doc
    "`(Char.from-int 57343)` — 57343 is U+DFFF, the LAST (highest) surrogate code point — is not a
           scalar, so from-int yields None. The upper-endpoint companion of the U+D800 (55296, the FIRST
           surrogate) case: the surrogate block is [U+D800, U+DFFF] inclusive, so both endpoints reject.
           Pins the block's upper edge (a block ending at 0xDFFE would wrongly accept 0xDFFF).")
  (input (match (Char.from-int 57343) ((Some _) 1) ((None _) 0)))
  (output (: 0 Int64)))

(case
  "Char.from-int at U+E000 (first scalar after the surrogate block) is Some"
  (doc
    "`(Char.from-int 57344)` — 57344 is U+E000, the FIRST scalar value immediately after the surrogate
           block (which ends at U+DFFF) — is valid, so from-int yields Some. Pins that the surrogate
           exclusion ends exactly at U+DFFF: U+E000 is accepted, so the block is [D800, DFFF] and not one
           wider. The lower-boundary complement of the last-surrogate case.")
  (input (match (Char.from-int 57344) ((Some _) 1) ((None _) 0)))
  (output (: 1 Int64)))

; The boundary cases above feed LITERAL integers, so from-int const-FOLDS the scalar-validity check before
; the runtime emitter is reached. This case makes the very same sweep RUNTIME: the code point arrives as a
; parameter `n` (which cannot fold), so the seed must EMIT the surrogate/range check, and it must agree with
; the fold at every off-by-one edge — the runtime `Char.from-int` sweep the from-int family doc promises.
; Some → Char.to-int round-trips the scalar; None → -1 (no valid scalar is negative, so the sentinel never
; aliases a real code point). Verified breaker probe ch: fold == runtime == cadenza-hop across all edges.
(case
  "a runtime (parameter) Char.from-int applies the scalar-validity sweep, agreeing with the fold"
  (doc
    "The runtime companion of the whole from-int boundary family: `n` is a parameter so the check is
           emitted, not folded. U+10FFFF (1114111, max) → Some → 1114111; U+110000 (1114112, one past) → None
           → -1; U+D800 (55296, first surrogate) and U+DFFF (57343, last surrogate) → None → -1; U+E000
           (57344, first scalar after the block) → Some → 57344; U+0000 (NUL) → Some → 0; -1 (negative) →
           None → -1. Pins the emitted surrogate/range check matches the const fold at every edge.")
  (input
    (do
      (def (f (: n Int64)) (match (Char.from-int n) ((Some c) (Char.to-int c)) ((None) -1)))
      (export f)))
  (call f (: 1114111 Int64))
  (output (: 1114111 Int64))
  (call f (: 1114112 Int64))
  (output (: -1 Int64))
  (call f (: 55296 Int64))
  (output (: -1 Int64))
  (call f (: 57343 Int64))
  (output (: -1 Int64))
  (call f (: 57344 Int64))
  (output (: 57344 Int64))
  (call f (: 0 Int64))
  (output (: 0 Int64))
  (call f (: -1 Int64))
  (output (: -1 Int64)))

(case
  "the maximum valid scalar U+10FFFF round-trips through to-int"
  (doc
    "`(Char.to-int (Char.from-int 1114111))` recovers 1114111 — the max scalar survives the char
           round-trip intact. The extreme companion of the mid-range round-trip below: a conversion that
           truncated or mis-handled the 21-bit-wide maximum scalar would lose it.")
  (input (= (Char.to-int (Option.expect (Char.from-int 1114111) "max scalar")) 1114111))
  (output (: true Bool)))

(case
  "char to-int and from-int round-trip through the scalar value"
  (doc
    "For a scalar value v, `(Char.from-int v)` is `(Some c)` and `(Char.to-int c)` is v again:
           `(Char.to-int #\\a)` = 97 and `(Char.from-int 97)` = `(Some #\\a)`, so matching the Some arm
           and taking to-int returns 97. Pins from-int as the inverse of to-int on a valid scalar
           (collections-and-text.md #A Char Converts To And From An Integer Totally). MUST be true.")
  (input (match (Char.from-int 97) ((Some c) (= (Char.to-int c) 97)) ((None _) false)))
  (output (: true Bool)))

(case
  "Char.from-int on a genuinely-runtime integer computes Some/None at run time (Char-rep 4/N follow-on)"
  (doc
    "`(Char.from-int n)` with a RUNTIME `n` (a def PARAMETER supplied by `(call …)`, not a constant
           fold) runs the exact scalar-validity check (`0..=0x10FFFF`, excluding surrogates `0xD800..=0xDFFF`)
           and Option-wrap AT RUN TIME — the runtime companion of the constant from-int cases above, unblocked
           once a runtime `Char` became a representable value (Char-rep 4/N: a boxable `Some` payload). Each
           `(call main v)` binds `c` = `#\\<v>` and returns its code point on a valid scalar, else -1: 97 → a
           valid `#\\a` → 97; 0 → U+0000 valid → 0; 55296 → U+D800 surrogate → None → -1; 1114112 → U+110000
           out-of-range → None → -1; -1 → negative → None → -1. This is the SAME domain the fold + the rust
           `char::from_u32` path enforce, now at run time.")
  (input
    (do
      (def (main (: n Int64)) (match (Char.from-int n) ((Some c) (Char.to-int c)) ((None u) -1)))
      (export main)))
  (call main (: 97 Int64))
  (output (: 97 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 55296 Int64))
  (output (: -1 Int64))
  (call main (: 1114112 Int64))
  (output (: -1 Int64))
  (call main (: -1 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a char literal naming a surrogate is a reader error"
  (doc
    "`#\\u+D800` names U+D800, a high surrogate — NOT a Unicode scalar value — so the char literal
           denotes no valid scalar and the reader rejects it (CDZ0002, collections-and-text.md #A Char Is
           A Single Unicode Scalar Value; options/char-literal-syntax/). The static companion of the
           dynamic `(Char.from-int 55296)` → None: a literal cannot spell a non-scalar, so the surrogate
           case is caught at read time rather than producing an invalid Char.")
  (input #\u+D800)
  (error CDZ0002))

(case
  "a char order agrees with its scalar value"
  (doc
    "Witnesses collections-and-text.md #A Char Is A Single Unicode Scalar Value (2nd sentence: a
           char's ordering is the numeric order of its scalar value): `(< #\\a #\\b)` is true because
           the scalar value of `a` (97) is less than that of `b` (98). Pins that a Char order and the
           string order defined on scalar values agree by construction — a Char is comparable and its
           order is its scalar order.")
  (input (< #\a #\b))
  (output (: true Bool)))

(case
  "a multibyte-scalar char orders by scalar value, not UTF-8 byte length"
  (doc
    "The >127 companion of the ASCII char-order case: ordering is the NUMERIC order of the
           SCALAR VALUE (collections-and-text.md #A Char Is A Single Unicode Scalar Value), even when
           the UTF-8 encodings differ in LENGTH. Built via `Char.from-int` (97 = a, 1 byte; 233 = e-acute,
           2 bytes; 128512 = an emoji, 4 bytes): a < e-acute < emoji by scalar (97 < 233 < 128512) —
           encoded `10*(a<e) + (e<r)` = 11. The ASCII pins never leave the 1-byte range where scalar
           and byte order coincide; this is the first multibyte witness, distinguishing scalar order
           from a byte-length or byte-sequence comparison.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (Char.from-int 97)
          ((Some a)
            (match
              (Char.from-int 233)
              ((Some e)
                (match
                  (Char.from-int 128512)
                  ((Some r) (+ (* 10 (if (< a e) 1 0)) (if (< e r) 1 0)))
                  (None -3)))
              (None -2)))
          (None -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "two equal chars compare equal"
  (doc
    "`(= #\\a #\\a)` is true — a char's value is its scalar, and two chars are equal exactly when
           their scalar values are equal. Pins Char equality as scalar equality, the equality companion
           of the Char order case.")
  (input (= #\a #\a))
  (output (: true Bool)))

(case
  "two chars with different scalar values are unequal"
  (doc
    "`(= #\\a #\\b)` is false — the discriminator for the equal-chars case above: Char equality is a
           genuine scalar comparison, not a blanket true. `a` (97) and `b` (98) have distinct scalar values,
           so they are unequal. Pins that Char `=` distinguishes chars (the false companion of `(= #\\a
           #\\a)`).")
  (input (= #\a #\b))
  (output (: false Bool)))

(case
  "the greater-than operator on chars follows scalar order"
  (doc
    "`(> #\\b #\\a)` is true — `b` (scalar 98) is greater than `a` (97), the `>` companion of the
           `(< #\\a #\\b)` order case. Pins that the strict-greater operator over Char also follows the
           scalar order (both directions of the Char order are reachable).")
  (input (> #\b #\a))
  (output (: true Bool)))

(case
  "the three-way comparison orders chars by scalar value — Less"
  (doc
    "`(Ordering.of #\\a #\\b)` is `(Ordering.Less unit)` — Char offers a total order (its scalar order,
           collections-and-text.md #A Char Is A Single Unicode Scalar Value), so `compare` reports it as
           the Less variant exactly as over Int64/Float64 (core-semantics.md #A Total Order Is Observed
           Through A Three-Way Comparison). Pins that the three-way comparison spans Char, the compare
           companion of the `(< #\\a #\\b)` operator case.")
  (input (Ordering.of #\a #\b))
  (output (: (Less unit) Ordering)))

(case
  "the three-way comparison orders chars by scalar value — Greater"
  (doc
    "`(Ordering.of #\\b #\\a)` is `(Ordering.Greater unit)` — the Greater variant over Char, so `b`
           orders after `a` by scalar value. The Greater companion of the Less case, pinning that compare's
           direction agrees with `>` on Char.")
  (input (Ordering.of #\b #\a))
  (output (: (Greater unit) Ordering)))

(case
  "the three-way comparison orders chars by scalar value — Equal"
  (doc
    "`(Ordering.of #\\a #\\a)` is `(Ordering.Equal unit)` — two chars of the same scalar report the middle
           variant. With the Less and Greater cases this pins all three Ordering variants are reachable over
           Char and discriminated by the scalar relation, exactly as the Int64/Float64 triples are.")
  (input (Ordering.of #\a #\a))
  (output (: (Equal unit) Ordering)))

(case
  "arithmetic between a char and an Int64 is rejected with the plain Char.to-int fix"
  (doc
    "The Int64 BASE CASE of the char-with-number family the float/BigInt/Rational cases below refer to
           ('the integer sibling `(+ #\\a 1)` keeps the plain `Char.to-int` wrap'). `(+ #\\a 1)` mixes a
           `Char` with an Int64 — a Char is not a number (collections-and-text.md #A Char Is A Single Unicode
           Scalar Value), so it is CDZ0203. Unlike the wider-numeric siblings, the repair is the PLAIN
           `(Char.to-int #\\a)` — `Char.to-int` yields Int64, which is exactly the sibling operand's type, so
           NO second conversion step is needed (the float/BigInt/Rational cases wrap it further because Int64
           does not implicitly promote to those). Pins the one-step fix for the Int64 mix, the base the
           two-step-fix cases are measured against. The program's outcome is the rejection.")
  (input (do (def (main) (+ #\a 1)) (export main)))
  (error CDZ0203))

(case
  "comparing a char to a FLOAT is rejected with a working two-step conversion fix"
  (doc
    "A `Char` is not a number, so `(< #\\a 1.0)` is CDZ0203 (collections-and-text.md #A Char Is A
           Single Unicode Scalar Value — a char compares to a char, not to a raw number). The DIAGNOSTIC's
           fix must actually type-check: `Char.to-int` yields Int64, but Cadenza never implicitly promotes
           Int64 → Float, so a bare `(Char.to-int #\\a)` still fails against a `Float64` (CDZ0301). The fix
           therefore wraps BOTH steps — `(Float64.of-int (Char.to-int #\\a))` — matching the sibling float's
           width (a `Float32` sibling gets `Float32.of-int`). Pins that the char-to-number reject offers a
           REPAIR that resolves the error in one shot for the float case (the integer sibling keeps the plain
           `Char.to-int` wrap). The program's outcome is the rejection; there is no value.")
  (input (do (def (main) (< #\a 1.0)) (export main)))
  (error CDZ0203))

(case
  "arithmetic between a char and a FLOAT is rejected with a working two-step conversion fix"
  (doc
    "The ARITHMETIC twin of the char-vs-float comparison case: `(+ #\\a 1.0)` mixes a `Char` with a
           `Float64`, which is CDZ0301 (numeric-model.md #An Arithmetic Operator Requires Both Operands To
           Be One Numeric Type — a Char is not a number, and Cadenza never silently promotes). As with the
           comparison, the fix must type-check: `Char.to-int` yields Int64, and Int64 + Float64 re-fails, so
           the repair is the two-step `(Float64.of-int (Char.to-int #\\a))` matching the sibling float's
           width. Pins fix PARITY between arithmetic and comparison for a char-with-float mix (the integer
           sibling `(+ #\\a 1)` keeps the plain `Char.to-int` wrap). The program's outcome is the rejection.")
  (input (do (def (main) (+ #\a 1.0)) (export main)))
  (error CDZ0301))

(case
  "arithmetic between a char and a BigInt is rejected with a working two-step conversion fix"
  (doc
    "The BigInt sibling of the char-with-float arithmetic case: `(+ #\\a (BigInt.of 5))` mixes a `Char`
           with a `BigInt` — CDZ0301 (a Char is not a number). `Char.to-int` yields Int64, and Int64 + BigInt
           re-fails (no implicit promotion), so the working repair is the two-step `(BigInt.of (Char.to-int
           #\\a))`. Pins fix parity across the numeric tower: Char + {Int, Float, BigInt, Rational} each offer
           a repair that type-checks in one shot (Int keeps the plain `Char.to-int`; the wider types wrap it
           in the target's `of`/`of-int`).")
  (input (do (def (main) (+ #\a (BigInt.of 5))) (export main)))
  (error CDZ0301))

(case
  "arithmetic between a char and a Rational is rejected with a working two-step conversion fix"
  (doc
    "The Rational sibling: `(+ #\\a (Rational.of-int 5))` mixes a `Char` with a `Rational` — CDZ0301.
           The working repair is `(Rational.of-int (Char.to-int #\\a))` (Int64 scalar then lifted to the whole
           rational), completing char-with-numeric fix parity alongside the Int/Float/BigInt cases.")
  (input (do (def (main) (+ #\a (Rational.of-int 5))) (export main)))
  (error CDZ0301))

; --- Char-LITERAL patterns: a `match` dispatches by scalar value ----------------------------------
; A char is a scalar whose identity IS its Unicode scalar value (collections-and-text.md #A Char Is A
; Single Unicode Scalar Value), so a char-literal pattern `(#\a …)` matches by that value — the Char
; analogue of an Int/Bool/String/Symbol-literal match arm (core-semantics.md #Matching Selects The
; First Arm Whose Pattern Matches). A char match dispatches exactly as the char `=` above compares:
; `(match c (#\a 1) (#\b 2) (_ 0))` selects the arm whose char equals `c`. Char is an OPEN type (any
; scalar value), so a char match — like an Int match — needs a wildcard tail to be exhaustive; without
; one it is CDZ0210, and a char pattern over a non-Char scrutinee is a CDZ0201 shape error. (These
; witness the CONSTANT-scrutinee dispatch; a Char has no run-time value form in the seed, exactly as
; the scalar-access cases above note, so every char match folds at compile time.)
(case
  "a char-literal pattern selects the arm whose char matches"
  (doc
    "`(match #\\b (#\\a 1) (#\\b 2) (_ 0))` is 2 — the `#\\b` scrutinee equals the second arm's char
           literal, so that arm is selected (core-semantics.md #Matching Selects The First Arm Whose
           Pattern Matches). The Char analogue of an Int/Bool/String-literal match: dispatch is by scalar
           value, exactly as `(= #\\b #\\b)` holds. Pins that a char literal is a valid match pattern.")
  (input (do (def (main) (match #\b (#\a 1) (#\b 2) (_ 0))) (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a char not among the literal arms falls through to the wildcard"
  (doc
    "`(match #\\z (#\\a 1) (#\\b 2) (_ 0))` is 0 — `#\\z` matches neither char-literal arm, so the
           wildcard `_` tail covers it (core-semantics.md #Matching Selects The First Arm Whose Pattern
           Matches — the wildcard is the last, always-matching arm). The miss companion of the char-match
           hit; pins that char dispatch is genuine (a non-listed char is NOT silently mapped to an arm).")
  (input (do (def (main) (match #\z (#\a 1) (#\b 2) (_ 0))) (export main)))
  (call main)
  (output (: 0 Int64)))

; The WELL-FORMEDNESS rejects the section intro promises (migrated from rcdzc
; a_char_literal_pattern_type_mismatch_and_non_exhaustion_reject): a char pattern over a non-Char scrutinee
; is a CDZ0201 shape error (the char twin of a bool-over-int probe), and — because Char is an OPEN type — a
; char match with no wildcard tail is non-exhaustive, CDZ0210 (exactly like an open Int match).
(case
  "a char-literal pattern over an Int scrutinee is a shape error"
  (input (do (def (main) (match 5 (#\a 1) (_ 0))) (export main)))
  (error CDZ0201))

(case
  "a wildcard-less char match is non-exhaustive (Char is an open type)"
  (input (do (def (main) (match #\a (#\a 1) (#\b 2))) (export main)))
  (error CDZ0210))

(case
  "a char-literal pattern nested in a variant payload matches by scalar value"
  (doc
    "`(match (Tok.Ch #\\a) ((Tok.Ch #\\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0))` is 97 — the variant
           carries a `Char` payload and the arm `(Tok.Ch #\\a)` matches a `Tok.Ch` whose payload equals
           `#\\a`, exactly as the String/Symbol-payload literal arms do. Pins a char literal as a valid
           NESTED sub-pattern (the payload twin of the top-level char match; the `#\\a` payload variant of
           the `(Ch Char)` case in 05-compound-types).")
  (input
    (do
      (type Tok (Ch Char) (End))
      (def (main) (match (Tok.Ch #\a) ((Tok.Ch #\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0)))
      (export main)))
  (call main)
  (output (: 97 Int64)))

(case
  "a nested char-literal payload falls through on a non-matching char"
  (doc
    "`(match (Tok.Ch #\\z) ((Tok.Ch #\\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0))` is 1 — the payload
           `#\\z` does not equal the `(Tok.Ch #\\a)` arm's literal, so the match falls to the `(Tok.Ch _)`
           arm binding any char. The miss companion of the nested-payload hit; pins that a nested char
           literal genuinely discriminates within a variant, not a blanket match on the constructor.")
  (input
    (do
      (type Tok (Ch Char) (End))
      (def (main) (match (Tok.Ch #\z) ((Tok.Ch #\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a char-literal payload matches over a RUNTIME sum scrutinee (boxed char payload, Char-rep 4/N)"
  (doc
    "A `Tok` built at RUN TIME from a Bool param — `(if b (Tok.Ch #\\a) (Tok.End))` — is a heap sum
           whose `Char` payload is BOXED into the i64 heap cell (box-int + i32->i64 extend), NOT a constant
           fold. Matching it with the char-literal arm `(Tok.Ch #\\a)` reads the payload back (get-int +
           i64->i32 narrow) and compares the i32 code point at run time (Char-rep 4/N — a Char is now a
           boxable compound element / sum payload). `(call main true)` builds `(Tok.Ch #\\a)` → the `#\\a`
           arm → 97; `(call main false)` builds `(Tok.End)` → the End arm → 0. Distinct from the nested
           CONSTANT-payload cases above (those fold the `Char` test); this exercises the runtime box/get path.")
  (input
    (do
      (type Tok (Ch Char) (End))
      (def
        (main (: b Bool))
        (match (if b (Tok.Ch #\a) (Tok.End)) ((Tok.Ch #\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0)))
      (export main)))
  (call main (: true Bool))
  (output (: 97 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

(case
  "a runtime char payload binds and reads back its code point (Char-rep 4/N)"
  (doc
    "Binding the `Char` payload of a RUNTIME `Tok.Ch` — the arm `((Tok.Ch c) (Char.to-int c))` — reads
           the boxed char out of the heap cell (get-int + i64->i32 narrow to the i32 code-point slot) and
           `Char.to-int` yields its Int64 code point. `(call main true)` builds `(Tok.Ch #\\a)` → binds `c` =
           `#\\a` → 97; `(call main false)` builds `(Tok.End)` → -1. The binding-read twin of the literal-test
           case above; pins that a char extracted from a runtime sum payload is a sound runtime Char value.")
  (input
    (do
      (type Tok (Ch Char) (End))
      (def
        (main (: b Bool))
        (match (if b (Tok.Ch #\a) (Tok.End)) ((Tok.Ch c) (Char.to-int c)) ((Tok.End) -1)))
      (export main)))
  (call main (: true Bool))
  (output (: 97 Int64))
  (call main (: false Bool))
  (output (: -1 Int64)))

; The landed char-pattern cases use CONSTANT char scrutinees. These pin the neighbors: a RUNTIME char (from
; Char.from-int, not a const fold) reaching a later arm; a char pattern over a NON-char scrutinee is a type
; error (the char pattern enforces its type from the PATTERN — unlike the String/Symbol pattern leak, the
; char path is sound); and a supplementary-plane char literal matches by scalar value.
(case
  "a runtime char reaches a later char-literal arm by scalar value"
  (doc
    "`(match c (#\\a 1) (#\\b 2) (_ 0))` with a RUNTIME `c` = `(Char.from-int 98)` = `#\\b` (not a
           constant fold) takes the SECOND arm → 2. Pins that char-literal dispatch works on a runtime char
           value across the arm list (the runtime companion of the landed constant `#\\b` case), so the
           scalar-value compare runs at run time, not only in the constant fold.")
  (input
    (do
      (def (classify (: c Char)) (match c (#\a 1) (#\b 2) (_ 0)))
      (def (main) (classify (Option.expect (Char.from-int 98) "b")))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a char-literal pattern over a non-char scrutinee is a type error"
  (doc
    "`(match n (#\\a 1) (_ 0))` with `n : Int64` — a Char pattern over an Int64 scrutinee — is rejected
           CDZ0201 ('match pattern type Char does not match scrutinee type Int64'). Pins that the char
           pattern enforces its type FROM THE PATTERN: a Char pattern requires a Char scrutinee, so a
           non-char scrutinee (Int64 here, and equally a String) is caught — the char path does NOT have the
           String/Symbol pattern's cross-nominal leak, it derives the pattern type soundly.")
  (input (do (def (main (: n Int64)) (match n (#\a 1) (_ 0))) (export main)))
  (call main (: 97 Int64))
  (error CDZ0201))

(case
  "a String match with no wildcard is non-exhaustive"
  (doc
    "String is an OPEN type (like Int64/Char) — no finite literal set exhausts it — so a `match` whose
           arms are string literals with no wildcard `_` tail does not cover every string and is rejected
           CDZ0210 (core-semantics.md #Matching Must Be Exhaustive), EVEN with a constant scrutinee: unlike a
           constant Int scrutinee that hits a present arm and folds, a String match still owes a total cover.
           The String analogue of the char/int no-wildcard exhaustiveness rule.")
  (input (match "hi" ("hi" 1) ("yo" 2)))
  (error CDZ0210))

(case
  "a RUNTIME String match with no wildcard is non-exhaustive — the if-chain desugar does not relax the check"
  (doc
    "The runtime-scrutinee companion of the constant case above (migrated from rcdzc
           a_runtime_string_match_without_a_wildcard_is_non_exhaustive): a `match` on a runtime String
           PARAMETER whose arms are string literals with no wildcard `_` tail is still CDZ0210. A runtime
           String match lowers to a chain of `(= s literal)` value-eq tests, and that desugar must NOT relax
           exhaustiveness — the total-cover obligation is checked on the arms before/independent of the
           lowering, exactly as for the constant scrutinee. The with-wildcard form compiles and runs (the
           `(_ …)` cases elsewhere in this file).")
  (input (do (def (op (: s String)) (match s ("add" 1) ("sub" 2))) (export op)))
  (error CDZ0210))

(case
  "an Int-literal pattern over a String scrutinee is a type error"
  (doc
    "`(match \"hi\" (5 1) (_ 0))` — an Int64 pattern over a String scrutinee — is a shape/type error
           rejected CDZ0201, checked structurally before the fold: the pattern's type (Int64) must agree with
           the scrutinee's (String). The String twin of the char-pattern-over-non-Char case above; the String
           path derives the pattern type soundly (no cross-nominal leak). The wildcard `_` makes the match
           exhaustive, so the ONLY fault is the pattern/scrutinee type disagreement.")
  (input (match "hi" (5 1) (_ 0)))
  (error CDZ0201))

(case
  "a supplementary-plane char literal matches by scalar value"
  (doc
    "`(match #\\😀 (#\\a 1) (#\\😀 2) (_ 0))` is 2 — the supplementary-plane scalar U+1F600 (😀, above
           the BMP) equals the second arm's char literal, dispatched by scalar value. Pins that char-literal
           dispatch compares the full 21-bit scalar, not a truncated code unit, so a supplementary-plane
           char discriminates correctly — the char-pattern companion of the supplementary-plane String.at.")
  (input (do (def (main) (match #\😀 (#\a 1) (#\😀 2) (_ 0))) (export main)))
  (call main)
  (output (: 2 Int64)))

; Every char-pattern case above dispatches a CONSTANT char scrutinee — even the "runtime char" case folds,
; because its `(Char.from-int 98)` argument is a compile-time constant that reduces to `#\b` before the
; match. A GENUINELY runtime char — `(Char.from-int n)` where `n` is a def PARAMETER supplied by `(call …)`
; — EXECUTES on the i32 code-point COMPUTE slot (Char-rep 1-4/N: a Char is an i32 code-point value, dispatched
; by scalar value, boxable as a compound element / sum payload; runtime `Char.from-int` computes the Option at
; run time). So a runtime char match dispatches by scalar value — the executing witness the old decline-pin
; promised (case below). This is the COMPUTE slot; the boundary RENDER is the char-as-bool render tag (tag-19),
; so a runtime char displays as its `#\c` literal, exactly as a runtime `Bool` renders `true`/`false` (see the
; runtime `String.scalar-at` executing case above, which reads + renders a runtime `Char`). A future change that
; made the match silently MISCOMPILE (a truncated-code-unit compare, a wrong scalar box) instead of dispatching
; correctly would flip this case's outputs.
(case
  "a match on a genuinely-runtime char (from a runtime Char.from-int) dispatches by scalar value"
  (doc
    "`classify` matches a Char against char-literal arms; called with `(Char.from-int n)` where `n` is
           a runtime Int64 PARAMETER (so the char is NOT constant-folded to a literal like the cases above).
           This EXECUTES now that the runtime char COMPUTE slot landed (Char-rep 1-4/N + runtime `Char.from-int`): the
           runtime char is an i32 code-point slot dispatched by scalar value (3/N), and `Char.from-int` on a
           runtime int computes the Option at run time. `(call main 97)` → `#\\a` → arm 1; `98` → `#\\b` → arm
           2; `99` → `#\\c` → wildcard 0. (Was a decline-pin pending the rep; now the executing witness the
           old pin promised.)")
  (input
    (do
      (def (classify (: c Char)) (match c (#\a 1) (#\b 2) (_ 0)))
      (def (main (: n Int64)) (classify (Option.expect (Char.from-int n) "in range")))
      (export main)))
  (call main (: 97 Int64))
  (output (: 1 Int64))
  (call main (: 98 Int64))
  (output (: 2 Int64))
  (call main (: 99 Int64))
  (output (: 0 Int64))
  (live-objects 0))

; --- String operations at RUN TIME: a string not fixed at compile time ---------------------------------
; The string cases above operate on CONSTANT string literals, so their lengths / slices / concatenations
; fold at compile time. A string chosen at run time — an `(if …)` selecting between two literals produces
; ONE `String` handle unified from both branches, whose identity is known only at run time — exercises the
; runtime query path: the length op reads the actual handle, not a folded constant. These pin that path,
; the string analogue of the runtime-index `List.at` and runtime-key `Map.lookup` cases. (`String.byte-len`
; and `String.at`/`String.concat` accept a runtime string; `String.scalar-len` on a runtime string and
; `String.slice` with a runtime bound are later increments the seed declines — not witnessed here.)
(case
  "the byte length of a run-time-selected string reads the actual handle"
  (doc
    "`(String.byte-len (if b \"hello\" \"hi\"))` — the `if` selects one of two literals at run time,
           yielding one `String` handle whose length is not a compile-time constant. `b`=true → \"hello\"
           (5 bytes), `b`=false → \"hi\" (2 bytes). Pins that `String.byte-len` reads the runtime handle's
           length rather than folding a constant, the string companion of runtime `List.len`.")
  (input (do (def (main (: b Bool)) (String.byte-len (if b "hello" "hi"))) (export main)))
  (call main (: true Bool))
  (output (: 5 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

(case
  "the byte length of a run-time multibyte string exceeds its scalar length"
  (doc
    "`(String.byte-len (if b \"café\" \"ab\"))` = 5 for \"café\" — the é is a 2-byte UTF-8 scalar, so
           the BYTE length (5) exceeds the 4 SCALARS (collections-and-text.md — byte length counts the
           UTF-8 encoding, not the scalars). Pins that the runtime byte-length op counts encoded bytes on a
           string whose content is decided at run time, not the scalar count.")
  (input (do (def (main (: b Bool)) (String.byte-len (if b "café" "ab"))) (export main)))
  (call main (: true Bool))
  (output (: 5 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

(case
  "a run-time-index scalar read is present in bounds and absent out of bounds"
  (doc
    "`(String.at \"abc\" i)` at a run-time index reads the one-scalar substring at scalar position
           `i` — total (collections-and-text.md #A String's Scalars Are Addressable): in bounds → `(Some
           <one-scalar string>)` (i=0 → \"a\", byte-len 1), out of bounds → `None` (i=5 → -1), never a trap.
           The string companion of the runtime-index `List.at`; the index is a parameter, so the read runs
           on the value heap rather than folding.")
  (input
    (do
      (def (main (: i Int64)) (match (String.at "abc" i) ((Some s) (String.byte-len s)) (None -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "concatenation with a run-time-selected operand joins the actual strings"
  (doc
    "`(String.concat (if b \"ab\" \"abcd\") \"z\")` joins a run-time-selected left operand with a
           constant right one; the result's byte length is 3 for \"ab\"+\"z\" and 5 for \"abcd\"+\"z\". Pins
           that `String.concat` joins the ACTUAL runtime string (not a folded constant) — the total binary
           join the compiler itself uses to build messages, exercised with a non-constant operand.")
  (input
    (do
      (def (main (: b Bool)) (String.byte-len (String.concat (if b "ab" "abcd") "z")))
      (export main)))
  (call main (: true Bool))
  (output (: 3 Int64))
  (call main (: false Bool))
  (output (: 5 Int64)))

; --- A run-time-index String.at result compares by CONTENT ----------------------------------------
; `String.at` at a run-time index returns the one-scalar substring as `Some(<slice>)`, where the slice is
; a ROPE — a byte offset INTO the source string, not a flat leaf. String content-equality (`=`) and
; map/set-key hashing compare PHYSICAL bytes, so a rope slice would compare by its OFFSET and never match
; a flat twin of identical content: `(= (String.at s i) "a")` was SILENTLY false even when the char at `i`
; IS 'a' (a wrong value, valid wasm, no diagnostic), so a hand-written lexer scanning a runtime string
; char-by-char could not classify a character. The fix compacts the fresh slice to an independent flat
; leaf at the producer, so the result compares by content everywhere it is used. These pin: a runtime
; `String.at` result equals the same character obtained as a literal (and is unequal to a different one),
; and a recursive char-scan counts matching characters correctly.
(case
  "a run-time-index String.at result compares equal to the same character as a literal"
  (doc
    "`(= (Option.expect (String.at \"abc\" i) \"c\") \"a\")` at a RUNTIME index `i`: index 0 is \"a\"
           (→ 1), index 1 is \"b\" (→ 0). The `String.at` result is a one-scalar rope slice, so its content
           equality against the literal \"a\" must compare by CONTENT, not the slice's rope offset. Before
           the producer-side slice compaction this returned 0 at index 0 — a runtime `String.at` result
           never compared equal to the same character obtained any other way (a silent wrong value; the
           wasm validates). The constant-index form folds and already compared correctly, hiding the bug.")
  (input
    (do
      (def (main (: i Int64)) (if (= (Option.expect (String.at "abc" i) "c") "a") 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a recursive scan counts a runtime string's matching characters"
  (doc
    "`count-a \"banana\"` — a recursive char-scan reading each scalar with `String.at` at a runtime
           index and counting `(= (String.at s i) \"a\")`. \"banana\" has three 'a's, so the result must be
           3. Before the fix it returned 0: each `String.at` result is a `bytes-slice` rope reached through
           `Option.expect`, and its content-equality compared by rope offset, never matching the flat
           literal \"a\". The char-by-char lexer idiom over a runtime string — a compiler-in-Cadenza
           tokenizing its input. The fix compacts the fresh slice at the producer (and `dup`s the borrowed
           source so the slice's reference is independent — the same string threads on into the recursion).")
  (input
    (do
      (def (at (: s String) (: i Int64)) (Option.expect (String.at s i) "ok"))
      (def
        (cnt (: s String) (: i Int64) (: acc Int64))
        (if (= i (String.byte-len s)) acc (cnt s (+ i 1) (if (= (at s i) "a") (+ acc 1) acc))))
      (def (main) (cnt "banana" 0 0))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

; --- Two matched String KEYS live at once across a recursion: a borrowed lookup key is not freed --------
; `Map.lookup`/`Set.contains` BORROW their key — the runtime reads it without consuming it (`champ_hash`/
; `champ_eq` over its bytes). A String key read out of a live sum-payload (a `Node`'s `String` field, bound
; by a match arm) is a BORROW the enclosing tree still owns — so the lookup emit must NOT drop it. It used
; to drop the key UNCONDITIONALLY (correct only for the OWNED-temporary key a constant/rope produces),
; freeing the borrowed key under its owner. With TWO such keys live at one node — a tree-walker consulting a
; node's OWN key AND its child's key, both String sum-payload projections looked up in a `Map String Int64`
; — the second lookup freed a key the first still needed, corrupting a comparison and dropping a per-node
; decision: a SILENT WRONG COUNT (valid wasm, `cdz check` clean, no diagnostic). The trigger needs the two
; keys live SIMULTANEOUSLY across a recursion ≥3 deep; the IDENTICAL tree with Int64 keys (no heap key, no
; borrow) computes correctly, pinning the tree/recursion logic. Same ownership root as the runtime-String
; content-equality and String.at families above; the fix gates the key drop on OWNED-vs-BORROWED (a boxed
; scalar / compacted rope / fresh owned compound is dropped, a borrowed param/local/projection is not).
(case
  "a recursive walk consulting a node's own and its child's String key computes correctly"
  (doc
    "A binary tree whose `Node`s carry a `String` operator key; `pc` counts nodes whose left child
           binds LOOSER — `(< (top l) (pv op))`, comparing the LEFT CHILD's key precedence `(top l)` to the
           node's OWN key precedence `(pv op)` (both looked up in a `Map String Int64`). On the nested tree
           `c{ b{ a{L,L}, L }, L }` the count is 2 (c: top(b)=2 < 3 → 1; b: top(a)=1 < 2 → 1; a: 99 < 1 →
           0). It used to return 1 — with TWO matched `String` sum-payloads (the node's own key and its
           child's) live at once across a recursion ≥3 deep, the second borrowed lookup key was freed under
           its owner and its comparison flipped (a silent wrong value; the wasm validates). The IDENTICAL
           tree with Int64 keys returns 2 (pinning the oracle and that the tree/recursion logic is right).
           `Map.lookup` BORROWS its key, so a key read out of a still-live node is left to its owner, not
           dropped — the pretty-printer precedence-parenthesization idiom over a runtime expression tree.")
  (input
    (do
      (type T (Leaf Int64) (Node String T T))
      (def
        (pv (: op String))
        (match
          (Map.lookup (Map.insert (Map.insert (Map.insert #map() "a" 1) "b" 2) "c" 3) op)
          ((Option.Some p) p)
          ((Option.None _) 0)))
      (def (top (: t T)) (match t ((T.Leaf _) 99) ((T.Node op _ _) (pv op))))
      (def
        (pc (: t T))
        (match
          t
          ((T.Leaf _) 0)
          ((T.Node op l r) (+ (if (< (top l) (pv op)) 1 0) (+ (pc l) (pc r))))))
      (def
        (main (: d Int64))
        (pc (T.Node "c" (T.Node "b" (T.Node "a" (T.Leaf 0) (T.Leaf 0)) (T.Leaf 0)) (T.Leaf 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects 0))

; --- A rope String NESTED IN A COMPOUND compares/keys by CONTENT (v-runtime) ----------------------
; The top-level `=`/map-key compaction (above) canonicalizes a rope String when the string IS the
; operand/key. But the value heap is TAGLESS: `champ_eq`/`champ_hash`'s structural walk compares a
; nested leaf by its PHYSICAL raw bytes and cannot tell a rope-of-bytes child from a compound child, so
; a rope String NESTED in a tuple/record/sum-payload/map-key compared UNEQUAL to its flat twin of equal
; content (a silent wrong value; valid wasm, no diagnostic) — and a compound map key containing an
; assembled string could not be looked up by its literal twin (silent None). The fix canonicalizes a
; String/Bytes leaf with `bytes-compact` AT EACH COMPOUND CONSTRUCTION SITE — the nested-leaf twin of
; `box-float`'s NaN normalize-on-construct — so no compound ever holds a rope and the tagless walk's
; physical compare is exact. `rep s n` builds a rope by `n` concats (rope, content "hi"+n×"x"); one
; concat suffices. Controls pin that a FLAT nested runtime string, identically-built ropes both sides,
; and folded constants were never affected — it is the ROPE-in-a-compound face exactly.
(case
  "a one-concat rope nested in a tuple equals its flat twin"
  (doc
    "`(= (tuple (rep \"hi\" 1) 1) (tuple \"hix\" 1))` — the left tuple's string element is a runtime
           ROPE (one `String.concat`, content \"hix\"), the right's is the flat literal \"hix\". Structural
           equality compares component-wise and string equality is by content, so the tuples are equal → 1.
           Before the construction-site compaction the walk compared the nested rope leaf physically (rope
           header bytes ≠ flat bytes) → 0. MINIMAL: one concat. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #tuple((rep "hi" 1) 1) #tuple("hix" 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a rope in an Option payload equals its flat-twin payload"
  (doc
    "`(= (Option.Some (rep \"hi\" 3)) (Option.Some \"hixxx\"))` — tags match (both Some), payloads are
           content-equal strings (rope vs flat \"hixxx\") → true → 1. The sum-payload face of the nested-rope
           miss; the float twin (`(= (Some Float64.nan) (Some Float64.nan))`) already passed because a NaN is
           canonicalized when its leaf is boxed — a String leaf needs the same treatment. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= (Option.Some (rep "hi" 3)) (Option.Some "hixxx")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a rope in a record field equals its flat-twin field"
  (doc
    "`(= (record (f (rep \"hi\" 3)) (g 1)) (record (f \"hixxx\") (g 1)))` — same field set, field `g`
           equal, field `f` content-equal strings (rope vs flat) → true → 1. A record IS a tuple at run
           time, so the same nested-rope face as the tuple case, keyed by field. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #record((= f (rep "hi" 3)) (= g 1)) #record((= f "hixxx") (= g 1))) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a compound map key containing a rope is found by its flat-twin key"
  (doc
    "`(Map.insert Map.empty (tuple (rep \"hi\" 3) 1) 42)` keys the map by a TUPLE whose string element
           is a runtime rope (content \"hixxx\"); `(Map.lookup … (tuple \"hixxx\" 1))` looks up with the
           flat-twin tuple. Equal keys → must find 42. Before the construction-site compaction the tuple key
           was CHAMP-hashed with its nested rope leaf uncompacted, landing in a different slot than the
           flat-twin query key → None (→ -1). The idiomatic \"key a map by (name, arity) where name was
           assembled by concat\". Expected: 42.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup (Map.insert Map.empty #tuple((rep "hi" 3) 1) 42) #tuple("hixxx" 1))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a compound map key whose LIST element is a rope is found by its flat-twin key"
  (doc
    "`(Map.insert Map.empty (list (rep \"hi\" 3)) 42)` keys the map by a single-element LIST whose
           element is a runtime rope (content \"hixxx\"); `(Map.lookup … (list \"hixxx\"))` looks up with the
           flat-twin list. A list element is stored on the value heap exactly like a tuple element, so the
           nested-rope face reaches a list too; construction-site compaction canonicalizes the element leaf,
           so the key hashes into the same CHAMP slot as its flat twin → 42 (before the fix: None → -1). (A
           direct `=` on two lists is a separate, not-yet-built compare; the map-KEY path exercises the
           list-element compaction here.) Expected: 42.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup (Map.insert Map.empty #list((rep "hi" 3)) 42) #list("hixxx"))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a FLAT runtime string nested in a tuple is unaffected (control)"
  (doc
    "`(= (tuple (rep \"hi\" 0) 1) (tuple \"hi\" 1))` — `rep \"hi\" 0` returns the source string with NO
           concat, so the nested element is a FLAT runtime leaf, not a rope. It compared equal to its flat
           twin before AND after the fix — isolating the bug to the ROPE case, not runtime-ness. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #tuple((rep "hi" 0) 1) #tuple("hi" 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "identically-built ropes on both sides of a nested compare are equal (control)"
  (doc
    "`(= (tuple (rep \"hi\" 3) 1) (tuple (rep \"hi\" 3) 1))` — both nested elements are ropes built the
           SAME way, so their physical shapes matched and this compared equal even BEFORE the fix (the
           physical compare happened to agree). Pins that the fix does not disturb the already-equal case.
           Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #tuple((rep "hi" 3) 1) #tuple((rep "hi" 3) 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

; The nested-rope construction-site compaction (above) RECURSES: a rope nested TWO levels deep is
; canonicalized because each construction site compacts its String children AND the inner compound is
; built before the outer, so no compound at any depth ever holds a rope. These pin depth-2 (tuple-in-
; tuple, sum-in-record, doubly-nested map key) — the shapes a real value tree (a compiler AST node's
; fields) reaches; they held once the leaf-level fix landed, so they guard that the recursion isn't
; later narrowed to depth 1.
(case
  "a rope two levels deep (tuple in a tuple) equals its flat twin"
  (doc
    "`(= (tuple (tuple (rep \"hi\" 3) 1) 2) (tuple (tuple \"hixxx\" 1) 2))` — the rope is the string
           element of the INNER tuple, itself an element of the outer tuple. Both tuples' string leaves are
           compacted at their construction sites (inner built first), so the structural walk compares by
           content at depth 2 → 1. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #tuple(#tuple((rep "hi" 3) 1) 2) #tuple(#tuple("hixxx" 1) 2)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a rope in a sum payload inside a record field equals its flat twin"
  (doc
    "`(= (record (f (Option.Some (rep \"hi\" 3))) (g 1)) (record (f (Option.Some \"hixxx\")) (g 1)))` —
           the rope sits in an `Option.Some` payload that is itself a record field. The sum payload compacts
           at its construction, the record stores the (already-canonical) sum handle → content-equal → 1.
           Mixes the sum-payload and record-field faces at depth 2. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (if
          (=
            #record((= f (Option.Some (rep "hi" 3))) (= g 1))
            #record((= f (Option.Some "hixxx")) (= g 1)))
          1
          0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a doubly-nested tuple map key containing a rope is found by its flat twin"
  (doc
    "The depth-2 compound-KEY face: a map keyed by `(tuple (tuple (rep \"hi\" 3) 1) 2)` (a rope nested
           two levels deep in the key) is looked up with the flat-twin key → 42. Every construction site on
           the key path compacts its string leaf, so the whole key hashes canonically → the flat-twin query
           lands in the same CHAMP slot. Expected: 42.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup
            (Map.insert Map.empty #tuple(#tuple((rep "hi" 3) 1) 2) 42)
            #tuple(#tuple("hixxx" 1) 2))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

; --- Runtime String.from-bytes: the UTF-8 validation-boundary edges --------------------------------
; The runtime decode cases above pin a valid appender rope, an all-invalid buffer (0xFF), and a
; flattened multibyte rope. These pin the VALIDATION BOUNDARY precisely — the malformed classes the
; strict validator must reject even though every byte is individually plausible, and the well-formed
; two-byte sequence it must accept, over Bytes whose elements arrive as runtime UInt8 wraps.
(case
  "String.from-bytes rejects a lone continuation byte"
  (doc
    "A single byte 0x80 — a CONTINUATION byte with no lead — is not well-formed UTF-8, so the
           total decode yields None → -1. Distinct from the 0xFF invalid-LEAD case above: 0x80 is a
           byte that IS valid inside a multibyte sequence, just not at the start — a validator that
           only rejected never-valid bytes (0xFE/0xFF) accepts it.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (String.from-bytes (Bytes.of #list((UInt8.wrap a))))
          ((Some s) (String.byte-len s))
          ((None _) -1)))
      (export main)))
  (call main (: 128 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "String.from-bytes rejects an overlong encoding"
  (doc
    "`[0xC0 0x80]` — the OVERLONG encoding of NUL (a 2-byte form of a value that must be 1
           byte). Each byte is structurally plausible (a 2-byte lead followed by a continuation), so
           a validator that only checked lead/continuation SHAPE accepts it; strict UTF-8
           (`std::str::from_utf8` semantics, which the runtime op pins itself to) rejects overlongs →
           None → -1. The classic smuggling vector for security filters — worth its own pin.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (String.from-bytes (Bytes.of #list((UInt8.wrap a) (UInt8.wrap 128))))
          ((Some s) (String.byte-len s))
          ((None _) -1)))
      (export main)))
  (call main (: 192 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "String.from-bytes accepts a two-byte sequence split across a concat seam"
  (doc
    "The lead byte 0xC3 and its continuation 0xA9 (é) arrive in SEPARATE ropes joined by
           `Bytes.concat` — the multibyte sequence exists only in the FLATTENED buffer. The decode
           accepts it (byte-len 2), pinning that validation runs over the flattened bytes, not
           per-leaf (a per-leaf validator sees a dangling lead in one leaf and a lone continuation in
           the other and wrongly rejects a well-formed string).")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (String.from-bytes
            (Bytes.concat (Bytes.of #list((UInt8.wrap a))) (Bytes.of #list((UInt8.wrap 169)))))
          ((Some s) (String.byte-len s))
          ((None _) -1)))
      (export main)))
  (call main (: 195 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a String param threaded through a self-recursive loop and concatenated each step is retained"
  (doc
    "The idiomatic pretty-printer shape: `build(s, n, acc) = build(s, n-1, String.concat(acc, s))`
           — a String PARAM `s` threaded UNCHANGED through the recursion AND consumed by `String.concat`
           each step. Regression: `s` is consumed by `String.concat` (rc--) yet re-passed to the self-call,
           so the shared ROPE was freed while still referenced and the rope walk read OUT OF BOUNDS past a
           depth threshold — `cdz check` clean, `cdz compile` ok, RAN → wasm trap at n≥4. ROOT: `is_heap_
           type` (the Perceus retain-candidate gate) included `Bytes` but NOT `String` — though a String is
           a heap rope exactly as Bytes is — so `s` was never a retain candidate and no `dup` was emitted
           (the List ops worked because `List` WAS in `is_heap_type`). The fix adds `String`/`Symbol`. Now
           `s` is duped before the consuming concat and the threaded copy stays live. `build(\"x\", 8, \"\")`
           → an 8-char string → byte-len 8.")
  (input
    (do
      (def
        (build (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (build s (- n 1) (String.concat acc s))))
      (def (run (: n Int64)) (String.byte-len (build "x" n "")))
      (export run)))
  (call run (: 8 Int64))
  (output (: 8 Int64))
  (live-objects 0))

; The retention case above checks a deep rope's byte-LEN (=8) but not that its CONTENT reads back correctly
; through the depth. A many-chunk rope (built by repeated String.concat) is a deep byte-rope; String.at /
; String.scalar-len / `=` must traverse it correctly at every position, not just measure its length. This
; pins that: a 20-chunk "ab" rope (40 scalars) indexes right at the start, deep interior, and last position,
; equals its flat twin, and is None past the end — the content-through-depth companion of the byte-len case.
(case
  "a deep many-chunk runtime string rope indexes and measures correctly through its depth"
  (doc
    "A 20-chunk rope built by repeated `String.concat` of \"ab\" (a deep runtime byte-rope, 40 scalars).
           `String.scalar-len` = 40; `String.at` reads the right scalar at index 0 (\"a\"), 1 (\"b\"), the
           deep interior 38 (\"a\") and last 39 (\"b\"); the rope `=` its 40-char flat twin (rope operands are
           compacted before the byte-compare, so a rope equals its flat form); and `String.at 40` is None
           (past the end). Result `(40, \"a\", \"b\", \"a\", \"b\", 1, 0)`. Pins that a MANY-chunk rope's
           addressing/length/equality traverse the full depth correctly, not just the 2-chunk ropes and the
           byte-len-only retention case — the content-through-depth companion of the deep-rope retention case
           above.")
  (input
    (do
      (def
        (build (: n Int64) (: acc String))
        (if (= n 0) acc (build (- n 1) (String.concat acc "ab"))))
      (def (at (: s String) (: i Int64)) (match (String.at s i) ((Some c) c) ((None _u) "X")))
      (def
        (main (: n Int64))
        (let
          ((r (build 20 "")))
          #tuple((String.scalar-len r)
            (at r 0)
            (at r 1)
            (at r 38)
            (at r 39)
            (if (= r "abababababababababababababababababababab") 1 0)
            (match (String.at r 40) ((Some _c) 1) ((None _u) 0)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: #tuple(40 "a" "b" "a" "b" 1 0) (Tuple Int64 String String String String Int64 Int64)))
  (live-objects known-leak))

; The deep-rope case above folds a FIXED chunk ("ab") a fixed number of times; this folds String.concat over
; a RUNTIME LIST of DISTINCT chunks — the list-driven build idiom (assembling a string from computed pieces,
; e.g. a compiler joining rendered fragments), the String twin of the Map.insert-fold and Set.insert-fold
; builds. The chunks arrive through a `(List String)` the fold walks by index, so the concatenation order and
; the per-step consume/drop discipline both run live.
(case
  "a String built by folding String.concat over a runtime list of chunks has the right content and length"
  (doc
    "Fold `String.concat` over a runtime `(List String)` `[\"ab\", \"c\", \"de\"]`, threading the
           accumulator: → \"abcde\". `String.scalar-len` = 5 (the joined length); `String.slice r 3 4` = \"d\"
           by content (the 4th scalar came from the THIRD chunk \"de\", so the fold placed the chunks in list
           order); and the whole rope `=` its flat twin \"abcde\". Result `(5, 1, 1)` (len, slice-is-d,
           equals-flat). Pins the list-driven `String.concat` fold build — the String companion of the
           Map.insert / Set.insert fold-build cases; a fold that reordered or dropped a chunk would flip the
           slice or the flat-equality.")
  (input
    (do
      (def
        (sfold (: xs (List String)) (: i Int64) (: acc String))
        (if
          (< i (List.len xs))
          (match (List.at xs i) ((Some s) (sfold xs (+ i 1) (String.concat acc s))) ((None _u) acc))
          acc))
      (def
        (main (: n Int64))
        (let
          ((r (sfold #list("ab" "c" "de") 0 "")))
          #tuple((String.scalar-len r)
            (match (String.slice r 3 4) ((Some sub) (if (= sub "d") 1 0)) ((None _u) 0))
            (if (= r "abcde") 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (tuple 5 1 1) (Tuple Int64 Int64 Int64)))
  ; The (tuple 5 1 1) folds to a constant, so its embedded constant cells now hoist build-once (WIT static
  ; encoding) — census-excluded immortals — dropping the leak 12→7 (the residual is the runtime String rope).
  (live-objects known-leak))

; --- The threaded-String-param retain: the consumption-shape faces ----------------------------------
; e38228f35 added String/Symbol to the Perceus retain-candidate gate (a param threaded through a
; self-recursive loop AND consumed by concat each step was freed while referenced — an OOB trap past
; depth 4; its pin covers the accumulate-into-acc shape). These pin the sibling consumption shapes,
; promoted from passing breaker probes.
(case
  "a threaded String param consumed twice per step survives the loop"
  (doc
    "`go(s, n, acc) = go(s, n-1, acc + byte-len(concat s s))` — the threaded `s` is consumed
           TWICE per iteration (both concat operands) and re-passed: each step adds 4 (\"abab\"),
           n = 5 → 20. The double-consume face needs TWO retains per step; an off-by-one frees the
           rope on the second consume and re-trips the OOB walk the fix closed.")
  (input
    (do
      (def
        (go (: s String) (: n Int64) (: acc Int64))
        (if (= n 0) acc (go s (- n 1) (+ acc (String.byte-len (String.concat s s))))))
      (def (main (: n Int64)) (go "ab" n 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64))
  (live-objects 0))

(case
  "a threaded String param consumed as the right concat operand survives the loop"
  (doc
    "`(String.concat \"k\" s)` — the threaded param is the RIGHT operand (the fix's shape
           consumes it as acc's appendee on the left path): each step adds byte-len(\"k\"+\"abc\") = 4,
           n = 6 → 24. The operand-position face of the retain (a consume-site scan keyed to one
           operand slot misses the other).")
  (input
    (do
      (def
        (go (: s String) (: n Int64) (: acc Int64))
        (if (= n 0) acc (go s (- n 1) (+ acc (String.byte-len (String.concat "k" s))))))
      (def (main (: n Int64)) (go "abc" n 0))
      (export main)))
  (call main (: 6 Int64))
  (output (: 24 Int64))
  (live-objects 0))

(case
  "a runtime string pattern dispatches by content (value-eq compare)"
  (doc
    "A match with STRING-LITERAL arms over a RUNTIME String value dispatches by CONTENT — the
           `Str`-probe LitTest emits a `value-eq` (`champ_eq`) compare against the literal, `bytes-compact`ing
           the leaf to canonical flat form first so a rope and its flat twin compare equal. Before, a runtime
           string pattern DECLINED ('a string pattern over a runtime payload is not yet supported (only a
           constant folds)'). The scrutinee is a `String.concat` result — a genuine runtime ROPE, not a
           constant that folds — so this exercises the runtime path. `classify (\"a\"+\"b\") = \"ab\"` selects the
           first arm → 1. A generation that skipped this would decline every runtime string match (an
           interpreter dispatching on a keyword, a lexer classifying a token).")
  (input
    (do
      (def (classify (: s String)) (match s ("ab" 1) ("cd" 2) (_ 0)))
      (def (main) (classify (String.concat "a" "b")))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime string pattern falls through to the wildcard when no literal matches"
  (doc
    "The non-match dual of the runtime string dispatch: a rope `\"xy\"` matching neither `\"ab\"` nor
           `\"cd\"` falls through to the wildcard → 0. Confirms each `value-eq` arm genuinely runs (a real
           content compare that can FAIL), not a blind first-arm fold. Paired with the positive case, pins
           that a runtime string match is a correct content-dispatch, not a decline and not a mis-match.")
  (input
    (do
      (def (classify (: s String)) (match s ("ab" 1) ("cd" 2) (_ 0)))
      (def (main) (classify (String.concat "x" "y")))
      (export main)))
  (output (: 0 Int64)))

; --- Runtime string patterns: the order, boundary, and composition faces ----------------------------
; 155cfa329 emits string-literal match arms by content (value-eq); its pins cover dispatch +
; wildcard fall-through. These pin the semantics a probe-chain desugar could scramble, promoted
; from passing breaker probes.
(case
  "first-match-wins holds with a duplicate string-literal arm"
  (doc
    "`(match s (\"e\" 40) (\"k5\" 50) (\"e\" 99) (_ -1))` on a runtime \"e\" → 40 — the FIRST
           duplicate wins and the later one is dead (the string analogue of the scalar
           duplicate-literal order pins; a chain built from a keyed set instead of source order
           answers 99 or nondeterministically).")
  (input
    (do
      (def (classify (: s String)) (match s ("e" 40) ("k5" 50) ("e" 99) (_ -1)))
      (def (main) (classify (String.concat "e" "")))
      (export main)))
  (output (: 40 Int64)))

(case
  "the empty string is a matchable literal"
  (doc
    "`(\"\" 10)` matches a runtime-built empty string (concat of two empties) → 10. The
           zero-length boundary of the content compare (a probe that tests a first byte before the
           length check reads out of bounds or falls through).")
  (input
    (do
      (def (classify (: s String)) (match s ("" 10) ("x" 20) (_ -1)))
      (def (main) (classify (String.concat "" "")))
      (export main)))
  (output (: 10 Int64)))

(case
  "a prefix literal does not shadow a longer string arm"
  (doc
    "Arms `\"ab\"` then `\"abc\"` on a runtime \"abc\": content equality is whole-string (length
           + bytes), so the prefix arm does NOT match and the exact arm fires → 2. A prefix-compare
           probe (memcmp without the length gate) answers 1.")
  (input
    (do
      (def (classify (: s String)) (match s ("ab" 1) ("abc" 2) (_ -1)))
      (def (main) (classify (String.concat "ab" "c")))
      (export main)))
  (output (: 2 Int64)))

(case
  "a guard composes with a string-literal arm"
  (doc
    "`((guard \"ab\" (> k 3)) 1) (\"ab\" 2)` — the SAME literal guarded then bare: k = 5 passes
           the guard → 1; k = 1 fails and falls to the bare twin → 2. Pins guard fall-through
           through the string content-probe chain (the desugar must AND the guard onto the first
           probe without consuming the literal for the second).")
  (input
    (do
      (def (classify (: s String) (: k Int64)) (match s ((guard "ab" (> k 3)) 1) ("ab" 2) (_ -1)))
      (def (main (: k Int64)) (classify (String.concat "a" "b") k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "a named-wildcard guard binder over a string binds in both the guard and the body"
  (doc
    "`(guard t (< t \"m\"))` — a bare-NAME wildcard binder `t` (not `_`, not a string literal) over a
           String scrutinee: `t` binds the WHOLE matched string (a named wildcard matches every string; the
           guard alone gates the arm), and BOTH the guard cond `(< t \"m\")` AND the body read `t`. This is
           the heap/STRING face of the named-guard-binder rule (Finding #46's scalar `let`-wrap, extended to
           the runtime-STRING match desugar by adv-53 / PR#1414): the guarded-wildcard branch must `(let ((t
           scrutinee)) (if guard body else))`-wrap so `t` re-resolves to the `let` binder rather than being
           severed from its `(guard …)` ancestor (which false-rejected CDZ0101 `unbound t`) — and must NOT
           emit a spurious `(= scrutinee t)` (a named wildcard is not a string literal to compare). Prior
           string-guard corpus cases all used a string-LITERAL arm; this pins the NAMED-BINDER shape. `band
           \"apple\"`: `\"apple\" < \"m\"` holds → body reads `t` via `String.byte-len` → 5; the guard-false
           path (`\"zebra\"`) falls to the tail → 3.")
  (input
    (do
      (def (band (: s String)) (match s ((guard t (< t "m")) (String.byte-len t)) (_ 3)))
      (def (main (: pick Int64)) (if (> pick 0) (band "apple") (band "zebra")))
      (export main)))
  (call main (: 1 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

; ============================================================================================
; An exported entry with a STRING parameter, called with a string argument — the exported-entry String-arg
; boundary. The emitted rust LIBRARY (`fn main(s: String) -> i64`) is valid, but the rust-target test DRIVER
; that marshals the `"abc"` argument used to pass it as a `&str` literal against the owned-`String` param →
; E0308 (a differential: wasm cleanly declines, rust FAILED to build). Fixed in the rust gate harness
; (`rust_call_arg` now wraps a string-literal arg `.to_string()` so it crosses as an owned String). A String
; param on a HELPER already built on both backends; this pins the exported-entry surface (breaker-found,
; corpus-bugfix). On wasm this DECLINES (a String across the component entry boundary is unrealized) — a
; sound todo; on rust it now runs → 3, matching the recorded value.
(case
  "an exported entry with a String parameter is called with a string argument"
  (doc
    "`(def (main (: s String)) (String.byte-len s))` exported and called with `\"abc\"` → the UTF-8
           byte length 3. The rust DRIVER now marshals the string arg as an owned `String` (`\"abc\".to_string()`),
           matching the emitted `fn main(s: String)` signature — no more E0308. (wasm declines the String
           entry arg — a sound todo; rust computes it.)")
  (input (do (def (main (: s String)) (String.byte-len s)) (export main)))
  (call main (: "abc" String))
  (output (: 3 Int64)))

(case
  "a MULTIBYTE and an EMPTY runtime String entry arg measure their UTF-8 byte lengths"
  (doc
    "The multibyte + empty edges of the String entry arg (the ascii case above measures 3): `aéz` is
           4 BYTES (é is 2 in UTF-8) though 3 characters — byte-len reads the encoding, not the char count —
           and the empty string is 0. Three calls through ONE export exercise the driver's owned-String
           marshaling at ascii/multibyte/empty. (wasm declines the String entry arg — the same sound todo as
           the ascii case; rust computes.)")
  (input (do (def (main (: s String)) (String.byte-len s)) (export main)))
  (call main (: "abc" String))
  (output (: 3 Int64))
  (call main (: "aéz" String))
  (output (: 4 Int64))
  (call main (: "" String))
  (output (: 0 Int64)))

(case
  "two runtime String entry args compare by content including multibyte"
  (doc
    "TWO String entry parameters compared with `=`: `café` = `café` → 1 (equal content, including the
           2-byte é), `café` ≠ `cafe` → 0 (the accented vs bare e differ). Pins that two independently-
           marshaled owned Strings compare by CONTENT at the boundary (not by handle/pointer) and that the
           compare reads the full byte sequence. (wasm declines the String entry args — sound todo; rust
           computes.)")
  (input (do (def (main (: a String) (: b String)) (if (= a b) 1 0)) (export main)))
  (call main (: "café" String) (: "café" String))
  (output (: 1 Int64))
  (call main (: "café" String) (: "cafe" String))
  (output (: 0 Int64)))

(case
  "a recursive scalar-walk classifies characters across a multibyte String entry arg"
  (doc
    "The LEXER idiom over an entry arg: a recursive `String.at` walk over `String.scalar-len`
           counts 'a' scalars — \"banana\" → 3, \"bén-ané\" → 1 (the accented scalars must not desync the
           index). The guest-rope walk twin passes on wasm; here the CONTENT arrives at the boundary, so
           the walk runs over marshaled entry bytes. (wasm declines the String entry arg — the family's
           sound todo; rust and rust-async compute.)")
  (input
    (do
      (def
        (walk (: s String) (: i Int64) (: n Int64) (: acc Int64))
        (if
          (>= i n)
          acc
          (walk
            s
            (+ i 1)
            n
            (match (String.at s i) ((Some c) (if (= c "a") (+ acc 1) acc)) ((None u) acc)))))
      (def (main (: s String)) (walk s 0 (String.scalar-len s) 0))
      (export main)))
  (call main (: "banana" String))
  (output (: 3 Int64))
  (call main (: "bén-ané" String))
  (output (: 1 Int64)))

(case
  "byte-len and scalar-len DIVERGE on a multibyte String entry arg by the encoding width"
  (doc
    "The two length notions side by side on one entry arg: `byte-len - scalar-len` = the total
           extra encoding bytes — \"café\" → 1 (one 2-byte scalar), \"日本語\" → 6 (three 3-byte
           scalars). A marshal that re-measured in UTF-16 units, or a scalar-len that counted bytes,
           breaks one input. (wasm declines the entry arg — sound todo; rust computes.)")
  (input (do (def (main (: s String)) (- (String.byte-len s) (String.scalar-len s))) (export main)))
  (call main (: "café" String))
  (output (: 1 Int64))
  (call main (: "日本語" String))
  (output (: 6 Int64)))

(case
  "String.slice across a multibyte scalar carries byte-len 2 with scalar-len 1, and degenerate ranges answer Some-empty / None"
  (doc
    "Three boundary faces of the fallible scalar-indexed slice over \"aéb\" (é = ONE scalar, TWO
           bytes): [1,2) extracts é as a VIEW whose byte-len is 2 while scalar-len is 1 (21) — a slice
           machinery that computed the view's byte extent from SCALAR indices 1:1 would report 1;
           [0,0) is a VALID empty slice (Some, byte-len 0 -> 100), not a failure; and the INVERTED
           range [2,1) is None (7), not a trap and not an empty view.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def s "aéb")
          (if
            (= mode 1)
            (match
              (String.slice s 1 2)
              ((Some v) (+ (* 10 (String.byte-len v)) (String.scalar-len v)))
              ((None _u) -1))
            (if
              (= mode 2)
              (match (String.slice s 0 0) ((Some v) (+ 100 (String.byte-len v))) ((None _u) -1))
              (match (String.slice s 2 1) ((Some _v) -2) ((None _u) 7))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 2 Int64))
  (output (: 100 Int64))
  (call main (: 3 Int64))
  (output (: 7 Int64)))

(case
  "String.slice spanning a rope seam maps a scalar range to the exact byte extent"
  (doc
    "The RANGE-VIEW companion of the every-width String.at rope case: the rope
           `(String.concat \"aé\" \"b😀c\")` has scalars a,é,b,😀,c with the seam between é and b.
           Slice [1,4) = \"éb😀\" CROSSES the seam with a multibyte scalar on BOTH sides — its view
           must span byte extent 2+1+4 = 7 over 3 scalars (73); a slice that resolved the scalar
           range against only the leaf containing the START (or mapped scalars to bytes 1:1 past the
           seam) mis-extends. [2,3) sits entirely in the second leaf just past the seam (11); [0,5)
           is the whole rope, byte-len 9 / scalar-len 5 (95).")
  (input
    (do
      (def (mk (: a String) (: b String)) (String.concat a b))
      (def
        (main (: mode Int64))
        (do
          (def s (mk "aé" "b😀c"))
          (def lo (if (= mode 1) 1 (if (= mode 2) 2 0)))
          (def hi (if (= mode 1) 4 (if (= mode 2) 3 5)))
          (match
            (String.slice s lo hi)
            ((Some v) (+ (* 10 (String.byte-len v)) (String.scalar-len v)))
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 73 Int64))
  (call main (: 2 Int64))
  (output (: 11 Int64))
  (call main (: 3 Int64))
  (output (: 95 Int64))
  (live-objects known-leak))

(case
  "a slice of a slice composes scalar offsets against the VIEW, not the base string"
  (doc
    "Re-slicing pins offset COMPOSITION: over `(String.concat \"xé\" \"y😀z\")` (scalars x,é,y,😀,z)
           the outer view [1,4) is \"éy😀\"; the inner slice indexes THAT view. [1,3) of the outer is
           \"y😀\" — byte-len 5, scalar-len 2 (52); resolving inner indices against the BASE instead
           gives \"éy\" (32). [2,3) is the emoji alone (41; base-resolved would be \"y\" -> 11). And
           [1,4) EXCEEDS the outer's 3-scalar extent so it is None (-1) even though the BASE has a
           scalar at index 4 — a base-resolved bounds check would answer Some.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def s (String.concat "xé" "y😀z"))
          (def outer (Option.expect (String.slice s 1 4) "outer in bounds"))
          (def lo (if (= mode 1) 1 (if (= mode 2) 2 1)))
          (def hi (if (= mode 1) 3 (if (= mode 2) 3 4)))
          (match
            (String.slice outer lo hi)
            ((Some v) (+ (* 10 (String.byte-len v)) (String.scalar-len v)))
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 52 Int64))
  (call main (: 2 Int64))
  (output (: 41 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "String.at through a slice VIEW of a rope reads view-relative scalars at every byte width"
  (doc
    "Three view layers compose: rope -> scalar slice -> String.at. Over
           `(String.concat \"xé\" \"y😀z\")` the view [1,4) is \"éy😀\"; String.at indexes the VIEW, and
           each returned one-scalar string's OWN byte-len proves which scalar was read: 0 -> é (2),
           1 -> y (1, the scalar just past the rope seam), 2 -> 😀 (4). Index 3 is None (-1) even
           though the BASE string has z there — a String.at that resolved the index against the
           base (or clamped to the leaf holding the view's start) reads z (1) or misses the seam.")
  (input
    (do
      (def
        (main (: i Int64))
        (do
          (def s (String.concat "xé" "y😀z"))
          (def v (Option.expect (String.slice s 1 4) "in bounds"))
          (match (String.at v i) ((Some c) (String.byte-len c)) ((None _u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 4 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a slice VIEW and a rope compare by content across the two non-flat reps"
  (doc
    "The eq/order pins compare rope-vs-FLAT; this crosses the two NON-FLAT reps directly — a
           borrowed [off,len] VIEW (`slice(\"xkeyz\",1,4)` = \"key\") against a concat ROPE, with all
           three relations read in one pass (100·eq + 10·(view<rope) + (rope<view)). mode 2: rope is
           \"key\" — equal (100). mode 1: \"kex\" sorts BEFORE the view (1). mode 0: \"kez\" sorts
           AFTER (10). A compare that walked the view's PARENT bytes (x-prefixed) or the rope's node
           structure gives eq=0 at mode 2 or flips an order digit.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def view (Option.expect (String.slice "xkeyz" 1 4) "in"))
          (def rope (String.concat "ke" (if (> mode 1) "y" (if (> mode 0) "x" "z"))))
          (+
            (* 100 (if (= view rope) 1 0))
            (+ (* 10 (if (< view rope) 1 0)) (if (< rope view) 1 0)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 100 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64)))

; A recursive fn rebinding its String param to a helper concat-of-two-slices MUST compile to a VALID
; module. Regression: at the recursion whose exit reads a String-length runtime call of the REBOUND
; param AND the helper slices by its own Int64 index, Core::StrSlice emitted its start/end bound
; operands at a FIXED base+7 scratch floor → the i64 checked-arith start slot and the i32
; scalar-len-reclaim end tee collided (one wasm local, two widths → invalid module, func validate
; 'expected i32 found i64'). Fixed by v-wasm-opt 597e0ff7d (float each operand floor to
; (base+7).max(*high), the disjoint-slot discipline). Sibling of the to-bytes br_table fix 4f9658803,
; different seam. String-shrinker shape (what a property-testing user writes). breaker-routed.
(case
  "a recursive drop-scalar walk over a rope converges (string-shrinker shape)"
  (input
    (do
      (def
        (d (: s String) (: i Int64))
        (String.concat
          (Option.expect (String.slice s 0 i) "lo")
          (Option.expect (String.slice s (+ i 1) (String.scalar-len s)) "hi")))
      (def
        (walk (: s String) (: i Int64))
        (if (>= i (String.scalar-len s)) s (walk (d s i) (+ i 1))))
      (def (main (: mode Int64)) (String.byte-len (walk "aébcd" 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a NON-TAIL recursion rebinding its rope arg to a slice-concat accumulates after each return"
  (doc
    "The non-tail frame of the fixed recursive-rebind seam (the shrinker capstone recurses in
           TAIL position; here `(+ (byte-len s) (recurse (d s i) ...))` uses the result AFTER the
           call, so each frame holds its rope ACROSS the recursive call while the callee rebinds a
           derived one): byte-lens 6+5+4 of the successively-dropped \"aébcd\"→\"ébcd\"→\"écd\"
           accumulate to 15 (the é keeps every intermediate multibyte). A frame layout that reused
           the floated bound slot across the non-tail call (or freed the held rope early) corrupts an
           addend. Perimeter guard for 597e0ff7d's high-water fix on the OTHER recursion shape.")
  (input
    (do
      (def
        (d (: s String) (: i Int64))
        (String.concat
          (Option.expect (String.slice s 0 i) "lo")
          (Option.expect (String.slice s (+ i 1) (String.scalar-len s)) "hi")))
      (def
        (sum-lens (: s String) (: i Int64))
        (if (>= i (String.scalar-len s)) 0 (+ (String.byte-len s) (sum-lens (d s i) (+ i 1)))))
      (def (main (: mode Int64)) (sum-lens "aébcd" 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 15 Int64))
  (live-objects known-leak))

(case
  "TWO loop-carried ropes each rebind to derived strings every recursion step"
  (doc
    "Doubles the floated-bound pressure of the fixed recursive-rebind seam: BOTH params are
           ropes rebound per step — `a` to its scalar tail (slice view), `b` to a concat of ITS tail
           — and the accumulator multiplies their byte-lens before the tail call (4·2 + 3·2 + 1·2 =
           16, hand-traced; the é keeps a's byte-len ≠ scalar-len). Two derived-rope slots plus two
           slice bounds live in one recursive frame — a high-water float that tracked only ONE
           pending bound (or crossed the slots) miscomputes an addend or emits the old invalid
           module. Second perimeter guard for 597e0ff7d.")
  (input
    (do
      (def (tail1 (: s String)) (Option.expect (String.slice s 1 (String.scalar-len s)) "t"))
      (def
        (zip-lens (: a String) (: b String) (: acc Int64))
        (if
          (= (String.scalar-len a) 0)
          acc
          (zip-lens
            (tail1 a)
            (String.concat (tail1 b) "x")
            (+ acc (* (String.byte-len a) (String.byte-len b))))))
      (def (main (: mode Int64)) (zip-lens "aéb" "cd" 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 16 Int64))
  (live-objects known-leak))

; --- The overflow-safe fallible-index guard, String face (companion of the 10-bytes family):
; huge and i32-wrap indices must decline to None on the FULL-width check, never wrap into range.
(case
  "String.at and String.slice with near-i64::MAX indices decline to None, not wrap"
  (doc
    "The String siblings of the 10-bytes overflow-safe bounds family (Bytes.at/Bytes.slice pin the wrapping-add and i32-truncation hazards; 13-strings had NO huge-index coverage): String.at at i64::MAX and String.slice at (2^62, 2^62+1) over a runtime 11-scalar rope both decline to None (0/0 → 0). A wrapping-i64 start+len or a signed-<= check takes the in-range path and returns a wrong Some.")
  (input
    (do
      (def
        (main (: i Int64) (: st Int64) (: en Int64))
        (do
          (def s (String.concat "héllo" " wörld"))
          (+
            (* 100 (match (String.at s i) ((Option.Some _c) 1) ((Option.None _u) 0)))
            (match (String.slice s st en) ((Option.Some _v) 1) ((Option.None _u) 0)))))
      (export main)))
  (call
    main
    (: 9223372036854775807 Int64)
    (: 4611686018427387904 Int64)
    (: 4611686018427387905 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "String.at with an index at 2^32+2 declines to None, not an i32-wrapped read"
  (doc
    "The i32-TRUNCATION face: 2^32+2 wraps to index 2 — IN-BOUNDS for the 11-scalar string — so a lowering that narrowed the index to i32 before the bounds check returns a wrong Some scalar; the full-width check declines all three reads to None (0).")
  (input
    (do
      (def
        (main (: i Int64) (: st Int64) (: en Int64))
        (do
          (def s (String.concat "héllo" " wörld"))
          (+
            (* 100 (match (String.at s i) ((Option.Some _c) 1) ((Option.None _u) 0)))
            (match (String.slice s st en) ((Option.Some _v) 1) ((Option.None _u) 0)))))
      (export main)))
  (call main (: 4294967298 Int64) (: 4294967297 Int64) (: 4294967299 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

; --- String batch: the 4-width scalar walk, scalar-wise reversal, the runtime to-bytes round
; trip (with the mid-scalar-cut decline), and a String-to-String effect op whose result feeds
; the next perform. ---
(case
  "a scalar walk over a rope spanning ALL FOUR UTF-8 widths counts the wide scalars"
  (doc
    "The per-scalar walk crossing a 4-BYTE astral scalar mid-rope (the 1/2/3-byte spectrum walk exists; 4-byte was touched only via to-bytes/slice): the byte-advance must step exactly 4 at U+1D11E — a 3-byte-max stride table or surrogate-style split makes scalar-len/at disagree past it. scalar-len 5 / byte-len 11 / wide-count 3 via each returned scalar's own byte-len.")
  (input
    (do
      (def
        (count-wide (: s String) (: i Int64) (: n Int64) (: acc Int64))
        (if
          (>= i n)
          acc
          (match
            (String.at s i)
            ((Option.Some c) (count-wide s (+ i 1) n (+ acc (if (> (String.byte-len c) 1) 1 0))))
            ((Option.None _u) acc))))
      (def
        (main (: k Int64))
        (do
          (def s (String.concat "aé日" (if (= k 1) "𝄞b" "cc")))
          (+
            (* 100 (String.scalar-len s))
            (+ (* 10 (String.byte-len s)) (count-wide s 0 (String.scalar-len s) 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 613 Int64))
  (live-objects known-leak))

(case
  "a scalar-wise string reversal is an involution over a multibyte rope and reverses scalars, not bytes"
  (doc
    "REVERSAL is the canonical scalar-vs-byte discriminator (a byte-wise reversal of héllo splits é into an invalid sequence): rev∘rev=id over the accumulator rebuild, exact content vs the literal olléh (the multibyte scalar rides intact), scalar-len preserved. String.at returning 1-scalar STRINGS is what makes concat-accumulate expressible.")
  (input
    (do
      (def
        (rev-go (: s String) (: i Int64) (: acc String))
        (if
          (< i 0)
          acc
          (match
            (String.at s i)
            ((Option.Some c) (rev-go s (- i 1) (String.concat acc c)))
            ((Option.None _u) acc))))
      (def (rev (: s String)) (rev-go s (- (String.scalar-len s) 1) ""))
      (def
        (main (: k Int64))
        (do
          (def s (String.concat "hél" (if (= k 1) "lo" "la")))
          (+
            (* 100 (if (= (rev (rev s)) s) 1 0))
            (+ (* 10 (if (= (rev s) "olléh") 1 0)) (String.scalar-len (rev s))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 115 Int64))
  (live-objects known-leak))

(case
  "String.to-bytes of a rope round-trips through from-bytes equal, and a mid-scalar cut declines"
  (doc
    "The runtime-rope round-trip (the const byte-len==Bytes.len pin never crosses back): to-bytes → from-bytes → = original, AND slicing the image's first 2 bytes cuts é's scalar in half so from-bytes correctly declines to None — an invalid UTF-8 prefix must not produce a truncated string.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def s (String.concat "hé" (if (= k 1) "llo" "y")))
          (def b (String.to-bytes s))
          (def cut (Option.expect (Bytes.slice b 0 2) "lo"))
          (+
            (*
              100
              (match
                (String.from-bytes b)
                ((Option.Some s2) (if (= s s2) 1 0))
                ((Option.None _u) -1)))
            (match (String.from-bytes cut) ((Option.Some _s) 1) ((Option.None _u) 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100 Int64))
  (live-objects 0))

(case
  "String.byte-len of a runtime rope agrees with Bytes.len of its to-bytes image"
  (doc
    "The runtime-rope face of the const agreement pin: byte-len and Bytes.len∘to-bytes must agree on a concat-built value (6 for café+s).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def s (String.concat "café" (if (= k 1) "s" "")))
          (if (= (String.byte-len s) (Bytes.len (String.to-bytes s))) (String.byte-len s) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6 Int64)))

(case
  "a String-to-String op transforms a rope arg in the arm and its RESULT feeds the next perform"
  (doc
    "The string-builder-through-effect idiom ((-> String String) op): the arm concats brackets around the rope ARG and the body RE-PERFORMS on the first result (wrap∘wrap = [[abc]], byte-len 7). Two heap values cross the arm boundary each way; the second perform consumes a rope the FIRST arm built.")
  (input
    (do
      (effect Fmt (op wrap (-> String String)))
      (def
        (main (: k Int64))
        (handle
          Fmt
          0
          ((wrap (s) c (resume (String.concat "[" (String.concat s "]")) (+ c 1))))
          (do
            (def a (Fmt.wrap (String.concat "ab" (if (= k 1) "c" "d"))))
            (def b (Fmt.wrap a))
            (String.byte-len b))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 7 Int64)))

; --- Ordering through a long multi-chunk common prefix. ---
(case
  "string ordering walks a 400-byte MULTI-CHUNK common prefix to the deciding final byte"
  (doc
    "The :940 ordering pin compares 5-byte single-chunk strings; the deep-rope pins cover EQ only. This orders through a 400-byte 200-chunk common prefix with the deciding byte LAST — the walk crosses ~200 seams in lockstep on BOTH operands before the difference (a seam-skipping or chunk-count-comparing walk decides early/wrong). < and compare agree; eq control.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "ab") (- n 1))))
      (def
        (main (: k Int64))
        (do
          (def base (rep "" 200))
          (def s1 (String.concat base (if (= k 1) "x" "q")))
          (def s2 (String.concat base "y"))
          (+
            (* 100 (if (< s1 s2) 1 0))
            (+
              (*
                10
                (match
                  (Ordering.of s1 s2)
                  ((Ordering.Less _u) 1)
                  ((Ordering.Equal _u) 2)
                  ((Ordering.Greater _u) 3)))
              (if (= s1 s2) 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 110 Int64)))

; --- Literal-arm prefix discrimination across seam positions. ---
(case
  "string-literal arms discriminate a PROPER PREFIX and hit across seam positions"
  (doc
    "The literal-arm pins never put a PROPER PREFIX of another arm's literal as its own arm (café/cafe/caf — a length-ignoring or byte-prefix compare conflates the short arm) nor a scrutinee whose rope SEAM falls mid-literal at different positions (caf+é and ca+fé both select the café arm).")
  (input
    (do
      (def (classify (: s String)) (match s ("café" 1) ("cafe" 2) ("caf" 3) (_ 0)))
      (def
        (main (: k Int64))
        (+
          (* 100 (classify (String.concat "caf" (if (= k 1) "é" "e"))))
          (+ (* 10 (classify (String.concat "ca" "fé"))) (classify (String.concat "caf" "")))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 113 Int64)))

; --- The classify-and-build fold. ---
(case
  "a CLASSIFY-and-BUILD fold maps ints to letter pieces and the built rope equals the literal"
  (doc
    "The render-report idiom: each element classifies through a branch chain and the CLASS piece appends (15 divides by both 3 and 5 and must take the FIRST branch — the branch-order face; the runtime k*7 lands the default); the accumulated rope compares = to the literal.")
  (input
    (do
      (def
        (join-ints (: xs (List Int64)) (: i Int64) (: acc String))
        (match
          (List.at xs i)
          ((Option.Some v)
            (do
              (def piece (if (= (% v 3) 0) "F" (if (= (% v 5) 0) "B" "n")))
              (join-ints xs (+ i 1) (String.concat acc piece))))
          ((Option.None _u) acc)))
      (def
        (main (: k Int64))
        (do
          (def s (join-ints #list(3 5 (* k 7) 15 1) 0 ""))
          (+ (* 100 (String.byte-len s)) (if (= s "FBnFn") 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 501 Int64))
  (live-objects known-leak))

; --- Construction-path equality for strings (the collection companions live in 05/19): a
; string reached via one construction path must compare equal to the same content reached
; via another — the rope-vs-flat and opposite-order pins above cover concat shapes; these
; add SLICE windows (seam-spanning) and the concat-vs-slice cross. ---
(case
  "a slice of a runtime rope equals the flat literal of its window"
  (doc
    "The slice-of-rope face of string canonicalization: `mk` builds the rope \"ab\"+\"cdef\" at run
           time; a slice window SPANNING the seam (indices 1..4 → \"bcd\", crossing the leaf boundary at
           index 2) and a window INSIDE one leaf (0..2 → \"ab\") must both equal their flat literals → 11.
           A slice that re-based against a single leaf (ignoring the rope's seam) or compared the rope
           window without compaction breaks the seam-spanning leg first.")
  (input
    (do
      (def (mk (: a String)) (String.concat a "cdef"))
      (def
        (main (: k Int64))
        (+
          (* 10 (if (= (Option.expect (String.slice (mk "ab") 1 4) "in bounds") "bcd") 1 0))
          (if (= (Option.expect (String.slice (mk "ab") 0 2) "in bounds") "ab") 1 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "a concat-reached string equals a slice-reached string of the same content and not a decoy"
  (doc
    "Cross-construction-path equality: \"abcd\" reached via CONCAT (\"ab\"+\"cd\", a rope) and via
           SLICE (the 1..5 window of \"xabcdy\", a re-based view) — two maximally different physical
           representations of the same 4 bytes must compare equal (tens digit), and the concat rope must
           NOT equal the decoy \"abce\" (ones digit) → 10. The string companion of the collection
           construction-path equality family (via-remove/algebra/swap-take pins in 05/19).")
  (input
    (do
      (def (via-concat (: a String)) (String.concat a "cd"))
      (def (via-slice) (Option.expect (String.slice "xabcdy" 1 5) "in bounds"))
      (def
        (main (: k Int64))
        (+ (* 10 (if (= (via-concat "ab") (via-slice)) 1 0)) (if (= (via-concat "ab") "abce") 1 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64)))

; ── UTF-8 validation ACROSS rope leaves, deepened: the single-seam straddle is pinned above
; (a multi-byte scalar across ONE runtime rope seam); these pin what that case cannot reach — a
; 3-byte scalar split ONE BYTE PER LEAF across BOTH seams of a 3-leaf rope, and a TORN sequence
; whose invalid continuation sits in the NEXT leaf. A per-leaf validator (or a seam-state-resetting
; decoder) passes the single-seam case yet fails these. ---
(case
  "from-bytes decodes a 3-byte scalar split across BOTH seams of a 3-leaf rope"
  (doc
    "One byte per leaf: [0xE1] ++ [0x98] ++ [0x8F] — the 3-byte scalar ᘏ (U+160F) with each byte
           in its own rope leaf. `String.from-bytes` must walk the sequence ACROSS both seams to
           validate + decode: 1 scalar, 3 bytes → 13. A validator that checked leaves independently
           sees a lone lead byte and two orphan continuations — three invalid fragments — and wrongly
           yields None.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def a (Bytes.of #list((UInt8.wrap (+ 225 k)))))
          (def b (Bytes.of #list(152)))
          (def c (Bytes.of #list(143)))
          (match
            (String.from-bytes (Bytes.concat (Bytes.concat a b) c))
            ((Some s) (+ (* 10 (String.scalar-len s)) (String.byte-len s)))
            ((None _u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 13 Int64))
  (live-objects 0))

(case
  "from-bytes rejects a torn sequence whose invalid continuation sits in the NEXT leaf"
  (doc
    "[0xE2 0x98] (a 3-byte lead + one valid continuation) ++ [0x41] ('A', NOT a continuation
           byte): the sequence is torn exactly AT the rope seam — the leftmost leaf alone is a prefix
           of a valid sequence, and the next leaf's first byte breaks it. Strict validation must look
           ACROSS the seam to see the tear and yield None (0). A per-leaf validator either falsely
           accepts the fragments or mis-places the error; a decoder that reset its state at the seam
           would accept 'A' as a fresh scalar and silently drop the dangling lead.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def left (Bytes.of #list(226 152)))
          (def right (Bytes.of #list((UInt8.wrap (+ 65 k)))))
          (match (String.from-bytes (Bytes.concat left right)) ((Some _s) 1) ((None _u) 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects 0))

; ── Reclaim (known-leak): Option.expect over a dead-after-borrowed String.slice leaks the Some shell (migrated from rcdzc) ──
(case
  "Option.expect over a dead-after-borrowed String.slice leaks the Some shell each iteration"
  (doc
    "The compound-Some-shell reclaim gap (node-keyed payload-escape backlog). `sl` slices [1,3) out of
           its String param (a `Some` of a 2-scalar rope slice), `Option.expect`s it, then `String.scalar-len`
           BORROWS the extracted slice (no consume). The `SumExpect` emit drops the owned Some shell only for
           a scalar or dup'd-compound payload — here the payload is a non-dup'd COMPOUND (the slice) dead after
           the borrow, so the shell + its slice are left un-dropped: ~2 cells per iteration. Looping `sl` over
           an owned base rope 5× (base a param, so the slice is a genuine runtime owned temporary, not a const
           fold) leaks 2·5 + 1 (the once-built base) = 11 cells, value-correct throughout: scalar-len of
           slice[1,3) of \"hixxx\" = \"ix\" = 2, summed 5× = 10 (a UAF would trap/corrupt; a wrong reclaim
           would garble the count). Flips to 0 when the node-keyed payload-escape fix lands (drop the shell
           after the last borrow when the payload does not flow out).")
  (input
    (do
      (def (sl (: s String)) (String.scalar-len (Option.expect (String.slice s 1 3) "e")))
      (def
        (loop (: j Int64) (: n Int64) (: base String) (: tot Int64))
        (if (< j n) (loop (+ j 1) n base (+ tot (sl base))) tot))
      (def (f (: h Int64)) (loop 0 5 "hixxx" 0))
      (export f)))
  (call f (: 0 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

; ── Reclaim (known-leak): String.from-bytes over a dead-after-borrowed compound Some shell (migrated from rcdzc) ──
(case
  "String.from-bytes over a dead-after-borrowed compound Some shell leaks it each iteration"
  (doc
    "The String.from-bytes op-face of the compound-Some-shell reclaim gap (same node-keyed
           payload-escape root as the Option.expect/String.slice sibling, distinct leak magnitude — op
           specific). Each iteration builds a fresh runtime rope \"hiii\" (0x68 + 3x 0x69), `String.from-bytes`
           it to a `Some(String)` compound shell, then BORROWS the payload via `String.byte-len` (4). The
           non-dup'd compound Some shell is dead after the borrow and left un-dropped: 2 cells per iteration
           (no once-built base — the rope is rebuilt each iter), so 5 iters leak 10 cells, value-correct
           throughout (byte-len \"hiii\" = 4, summed 5x = 20; a UAF would trap, a wrong reclaim would garble).
           Flips to 0 when the node-keyed payload-escape fix lands (select.rs).")
  (input
    (do
      (def (rep (: acc Bytes) (: n Int64)) (if (= n 0) acc (rep (Bytes.concat acc b"i") (- n 1))))
      (def
        (loop (: k Int64) (: sum Int64))
        (if
          (= k 0)
          sum
          (loop
            (- k 1)
            (+
              sum
              (match (String.from-bytes (rep b"h" 3)) ((Some s) (String.byte-len s)) ((None _) 0))))))
      (def (main (: n Int64)) (loop n 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64))
  (live-objects 0))

; -- breaker batch 446 (2026-08-27): PRE-DELIVERED acceptance fence for static-data increment 5
; (constant-String build-once hoist; #3842 landed the byte-neutral payload-extractor groundwork).
; The Bytes twin of this fence is sbd1/sbd2 in 10-bytes (pinning #3837). Green TODAY on the
; per-eval allocation; the hoist must keep them green — a drop that freed the deduplicated shared
; static would trap or misread the second occurrence, and a per-eval leak reads >=50 in the
; amplification frame loop.
(case
  "ssd1 two occurrences of one constant String literal — branch-selected use, byte-length reads, and runtime equality across the pair"
  (doc
    "`a` branch-selects (on the runtime arg) between the shared literal (byte-len 21) and a different
           one; `b` is a second occurrence of the same literal. n=1: 100*21 + 21 + 1000 (a=b) = 3121. The
           runtime `=` compares the literal against its own second occurrence with drops between the uses.
           MUST be 3121, live-objects 0 — under a build-once hoist the shared static must survive both
           drops.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (if (= n 1) "const-shared-payload!" "other"))
            (x (String.byte-len a))
            (b "const-shared-payload!"))
          (+ (* 100 x) (+ (String.byte-len b) (if (= a b) 1000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3121 Int64))
  (live-objects 0))

(case
  "ssd2 a fifty-frame recursion re-evaluating a constant String literal each frame reclaims to zero"
  (doc
    "Per-frame amplification: each frame branch-selects (parity of k) between the shared literal
           (byte-len 21) and a second literal (byte-len 16), reads the byte-length, and drops. n=50 ->
           25*21 + 25*16 = 925. A leak of even one object per evaluation reads >=50 here; a hoist whose
           drop freed the build-once static would corrupt later frames. MUST be 925, live-objects 0.")
  (input
    (do
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (let
            ((a (if (= (% k 2) 0) "const-shared-payload!" "odd-frame-string")))
            (+ (String.byte-len a) (frames (- k 1))))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 925 Int64))
  (live-objects 0))

; -- runtime String rope conversions (behavioral migration from rcdzc bytes/string cdz-run tests, 2026-08-27):
; concat+byte-len, from-bytes decode/ill-formed, to-bytes encode, all built through a tail-recursive appender
; so the string stays genuinely runtime (opaque to the fold, imports the value-heap runtime).
(case
  "a runtime String.concat then byte-len measures the built rope"
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (String.byte-len (rep "" 3)))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a runtime String.from-bytes decodes a recursively built buffer"
  (input
    (do
      (def (rep (: acc Bytes) (: n Int64)) (if (= n 0) acc (rep (Bytes.concat acc b"i") (- n 1))))
      (def (main) (Option.expect (String.from-bytes (rep b"h" 3)) "utf8"))
      (export main)))
  (call main)
  (output (: "hiii" String))
  (live-objects known-leak))

(case
  "a runtime String.from-bytes of ill-formed bytes takes the None arm"
  (input
    (do
      (def
        (rep (: acc Bytes) (: n Int64))
        (if (= n 0) acc (rep (Bytes.concat acc b"\xff") (- n 1))))
      (def
        (main)
        (match (String.from-bytes (rep b"" 2)) ((Some s) (String.byte-len s)) ((None _) -1)))
      (export main)))
  (call main)
  (output (: -1 Int64)))

(case
  "a runtime String.to-bytes encodes a recursively built string"
  (input
    (do
      (def (rep (: acc String) (: n Int64)) (if (= n 0) acc (rep (String.concat acc "a") (- n 1))))
      (def (main) (Bytes.len (String.to-bytes (rep "" 3))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a bare constant string escapes across the boundary and renders"
  (input (do (def (main) "hello") (export main)))
  (call main)
  (output (: "hello" String)))

; -- char-literal spellings resolve to the same scalar (reader feature; migration from rcdzc
; a_char_literal_is_a_scalar_that_compares_by_scalar_value, 2026-08-27; = / < / surrogate-reject already
; covered by the Char comparison + CDZ0002 cases here).
(case
  "a hex code-point char spelling names the same scalar as the plain char"
  (input (do (def (main) (if (= #\a #\a) 1 0)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a named control char reads to its scalar code point"
  (input (do (def (main) (if (= #\newline #\newline) 1 0)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a char literal past the maximum scalar is a reader defect"
  (input (do (def (main) #\u+110000) (export main)))
  (error CDZ0002))

; -- char/symbol literal list-element pattern refinement (migration from rcdzc
; a_char_or_symbol_literal_list_element_refines_by_value, 2026-08-27): a #\c / #"sym" literal is a refutable
; scalar list element, matching only a list whose element at that position equals the literal.
(case
  "a char-literal list-element pattern matches a runtime list whose head is that char"
  (input
    (do
      (def (classify (: xs (List Char))) (match xs (#list(#\a (.. r)) 1) (_ 0)))
      (def
        (main)
        (classify
          #list((Option.expect (Char.from-int 97) "a") (Option.expect (Char.from-int 98) "b"))))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a char-literal list-element pattern misses when the head differs"
  (input
    (do
      (def (classify (: xs (List Char))) (match xs (#list(#\a (.. r)) 1) (_ 0)))
      (def (main) (classify #list((Option.expect (Char.from-int 122) "z"))))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a symbol-literal list-element pattern matches a list whose head is that symbol"
  (input
    (do
      (def (f (: xs (List Symbol))) (match xs (#list(#"go" (.. r)) 1) (_ 0)))
      (def (main) (f #list((Symbol.of "go"))))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a char literal at a non-head fixed-arity list position matches by value"
  (input
    (do
      (def (f (: xs (List Char))) (match xs (#list(a #\x) 1) (_ 0)))
      (def (main) (f #list(#\p #\x)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

; ── breaker batch 537: slice-dup residue calibration (post-#4425, operator-accepted
; leaks-over-UAF). The Owned classification stopped the Class-A double-free (values verified
; correct in every cell here) at the cost of an unreleased dup per consume. These three cells sit
; OUTSIDE the 12 markers #4425 reconciled: the MINIMAL single-consume cell, the slice-of-slice
; composition, and the dqe-INTERSECTION (a slice inside a dual-used tuple — drops only when BOTH
; the slice-dup residue AND dqe leg-1 are balanced; a partial §4 fix shows an intermediate
; reading here). All flip DOWN under v-core-opt's unified consuming analysis.
(case
  "slc1 a SINGLE-consumed String.slice result leaks its dup (the minimal post-#4425 residue cell)"
  (input
    (do
      (def
        (main (: n Int64))
        (String.byte-len
          (Option.expect
            (String.slice (String.concat "abcdef" (if (> n 0) "XY" "Z")) 1 4)
            "in-range")))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "slc2 a slice-of-slice (two view layers) multi-consumed leaks both layers' dups"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((u
              (Option.expect
                (String.slice
                  (Option.expect
                    (String.slice (String.concat "abcdefgh" (if (> n 0) "XY" "Z")) 1 6)
                    "in-range")
                  1
                  3)
                "in-range")))
          (+ (* 100 (String.byte-len u)) (if (= u "cd") 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 201 Int64))
  (live-objects known-leak))

(case
  "slc3 a slice inside a DUAL-USED tuple (projection + walker) stacks the dqe leg-1 leak on the slice residue"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a
              #tuple(n
                (Option.expect
                  (String.slice (String.concat "abcdef" (if (> n 0) "XY" "Z")) 1 4)
                  "in-range")))
            (b
              #tuple(n
                (Option.expect
                  (String.slice (String.concat "abcdef" (if (> n 0) "XY" "Z")) 1 4)
                  "in-range"))))
          (+ (* 100 (String.byte-len (. a 1))) (+ (. b 0) (if (= a b) 10000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10301 Int64))
  (live-objects known-leak))

; ── breaker batch 545: DEEP-ROPE structural cells (50-concat trees — unprobed depth). rp1 = the
; rope itself reclaims clean after a byte-len tree walk; rp2 = the char-scan idiom across every
; seam (values exact; carries the KNOWN String.at per-read leak at scale, ~2/read — same family
; as the banana lexer pin, flips with it); rp3 = a slice SPANNING ~50 seams reads exact content
; (slc-class residue rider at rope depth).
(case
  "rp1 a 50-concat deep rope's byte-len walks the tree and the rope reclaims clean"
  (input
    (do
      (def (grow (: s String) (: k Int64)) (if (= k 0) s (grow (String.concat s "x") (- k 1))))
      (def (main (: n Int64)) (String.byte-len (grow (if (> n 0) "abc" "z") 50)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 53 Int64))
  (live-objects 0))

(case
  "rp2 a char-scan with String.at across every seam of a 50-concat rope counts exactly (per-read leak family at scale)"
  (input
    (do
      (def (grow (: s String) (: k Int64)) (if (= k 0) s (grow (String.concat s "x") (- k 1))))
      (def (at (: s String) (: i Int64)) (Option.expect (String.at s i) "ok"))
      (def
        (cnt (: s String) (: i Int64) (: acc Int64))
        (if (= i (String.byte-len s)) acc (cnt s (+ i 1) (if (= (at s i) "x") (+ acc 1) acc))))
      (def (main (: n Int64)) (cnt (grow (if (> n 0) "abc" "z") 50) 0 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 50 Int64))
  (live-objects known-leak))

(case
  "rp3 a slice SPANNING the seams of a deep rope reads exact length and content"
  (input
    (do
      (def (grow (: s String) (: k Int64)) (if (= k 0) s (grow (String.concat s "x") (- k 1))))
      (def
        (main (: n Int64))
        (let
          ((r (grow (if (> n 0) "abc" "z") 50)))
          (let
            ((sl (Option.expect (String.slice r 1 (- (String.byte-len r) 1)) "in")))
            (+ (* 100 (String.byte-len sl)) (if (= (Option.expect (String.at sl 10) "ok") "x") 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 5101 Int64))
  (live-objects known-leak))

(case
  "ssc1 String.slice indices are SCALAR (codepoint) positions, not bytes — mid-codepoint cuts are impossible by construction"
  (doc
    "Over `aé😀` (a=1B, é=2B, 😀=4B; 7 bytes, 3 scalars), `(String.slice s 0 k)` takes the first k
           SCALARS: k=1 -> 'a' (1 byte), k=2 -> 'aé' (3 bytes), k=3 -> 'aé😀' (7 bytes). The byte-lengths
           1/3/7 prove the index is a scalar count, NOT a byte offset — so a slice can never land mid-
           codepoint and corrupt a scalar (contrast Bytes.slice which IS byte start+length, tick-344). A
           refactor to byte-offset slicing would change these byte-lengths. `(* 100 byte-len s)` = 700
           carries the total; k=1/2/3 add 1/3/7.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s (String.concat "a" (if (> n 0) "é😀" "x"))))
          (+
            (* 100 (String.byte-len s))
            (match (String.slice s 0 n) ((Some sl) (String.byte-len sl)) ((None) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 701 Int64))
  (call main (: 2 Int64))
  (output (: 703 Int64))
  (call main (: 3 Int64))
  (output (: 707 Int64))
  (live-objects known-leak))

(case
  "sms1 scalar-indexed String.slice isolates a multi-byte codepoint and round-trips on a RUNTIME string"
  (doc
    "Fences that `String.slice` is SCALAR (codepoint) indexed, not byte-indexed, over a runtime-built
           multi-byte string — a byte-indexed regression would cut mid-codepoint and corrupt. `s = rep \"aé🦀\" n`
           is built by runtime recursion (scalar-len 3n, byte-len 7n; the recursion defeats const-fold —
           verified input-dependent, main 1→3401, 2→6401). `(String.slice s 2 3)` isolates the 4-byte 🦀 by its
           SCALAR index 2 (a is 1 byte, é is 2, so a byte-2 slice would land inside é), so its `byte-len` is 4;
           and the scalar-split round-trip `slice[0:2] ++ slice[2:scalar-len] == s` holds. `String.slice`
           returns `(Option String)` (None out-of-bounds), unwrapped here. `1000*scalar-len + 100*byte-len(🦀
           slice) + roundtrip` = 1000*3 + 100*4 + 1 = 3401 at n=1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (> n 0) (String.concat s (rep s (- n 1))) ""))
      (def
        (main (: n Int64))
        (let
          ((s (rep "aé🦀" n)))
          (+
            (* 1000 (String.scalar-len s))
            (+
              (* 100 (String.byte-len (Option.expect (String.slice s 2 3) "")))
              (if
                (=
                  (String.concat
                    (Option.expect (String.slice s 0 2) "")
                    (Option.expect (String.slice s 2 (String.scalar-len s)) ""))
                  s)
                1
                0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3401 Int64))
  (live-objects known-leak))

(case
  "D1 an if-joined dual-used rope sharing a pre-if concat child in BOTH arms reclaims cleanly (was a 980-residual double-free, FIXED #5424)"
  (doc
    "The 980-family sibling the narrowed cross-arm (a) gate does NOT reach. `keep = (if mode r2a r2b)`
           where r2a=(String.concat r1 x) and r2b=(String.concat r1 y) BOTH consume the SAME pre-if rope r1,
           and `keep` is DUAL-used (byte-len + a value-eq compare). All three conditions are required (each
           alone is clean: single-use keep / two-concat-without-if / independent-ropes-dual-use). r1 WAS freed
           TWICE — via the kept arm's post-join drop AND the unkept arm's drop — a double-free SILENT and
           value-correct on the shipped runtime (output 80) but TRAPPING on the debug-counters runtime. FIXED by
           #5424 (let-drop elides a binding CONSUMED into a later sibling init as a fresh-alloc child — the D1
           flat-let over-drop), now clean-0 both modes ([0,0], `(live-objects 0)` genuine; v-memory-safety +
           v-core-opt co-verified on the nix/seated check). RETAINED as a REGRESSION-GUARD (v-memory-safety +
           v-corpus-harness, the #4547 mechanism): the `(live-objects …)` clause FORCES the debug run, so an emit
           change re-introducing this shape's double-free re-fails here rather than shipping it silently. Hand-built
           witness (not yet cdz-smith-reachable); r1 is consumed PRE-`if` so the narrowed cross-arm (a)
           [consume-in-an-if-arm + borrow-as-result-in-the-other] does not reach it.")
  (input
    (do
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: mode Int64))
        (do
          (def r1 (rep "ab" 3 ""))
          (def r2a (String.concat r1 (rep "x" 2 "")))
          (def r2b (String.concat r1 (rep "y" 2 "")))
          (def keep (if (= mode 1) r2a r2b))
          (+
            (* (String.byte-len keep) 10)
            (if (= keep (if (= mode 1) "ababababxx" "ababababyy")) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 80 Int64))
  (call main (: 2 Int64))
  (output (: 80 Int64))
  (live-objects 0))

(case
  "nfc1 runtime-constructed strings are NFC-normalized on every backend"
  (doc
    "The #5707 acceptance face (breaker adv-rust-runtime-skips-nfc lineage, was 112 wasm vs 23 rust):
        a runtime (String.concat \"e\" U+0301) equals the precomposed literal, set-dedups with it, and
        byte-lens 2 (n=1 -> 112); the NON-composing q+U+0301 stays 3 bytes + self-equal + dedups
        (n=0 -> 113, the over-normalization guard); from-bytes re-normalizes identically (n=-5 -> 2).
        NFC-before-value is a String-type INVARIANT every runtime constructor must maintain
        (collections-and-text.md); the rust cdz-rt Core::NfcNormalize arm was a no-op until #5707.")
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (> n 0)
          (do
            (def pre "é")
            (def dec (String.concat "e" "́"))
            (+
              (* 100 (if (= pre dec) 1 0))
              (+ (* 10 (Set.len #set(pre dec))) (String.byte-len dec))))
          (if
            (> n -3)
            (do
              (def qc (String.concat "q" "́"))
              (+
                (* 100 (if (= qc qc) 1 0))
                (+ (* 10 (Set.len #set(qc (String.concat "q" "́")))) (String.byte-len qc))))
            (match
              (String.from-bytes (String.to-bytes (String.concat "e" "́")))
              ((Some s) (String.byte-len s))
              ((None _u) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 112 Int64))
  (call main (: 0 Int64))
  (output (: 113 Int64))
  (call main (: -5 Int64))
  (output (: 2 Int64)))

(case
  "sc1 runtime String.scalar-at round-trips the cadenza hop — multi-byte scalar indexing + out-of-range"
  (doc
    "The #6092 hop face (the sa1 arc's last leg: #5852 front → #5932 both-backend emit → #6082
        corpus flip → #6092 hop): a runtime-index scalar-at re-emits as ((. String scalar-at) op idx)
        and recompiles. Multi-byte discrimination through the hop: 'café' scalar 3 is é (U+00E9=233,
        a SCALAR index, not a byte), scalar 1 is 'a' (97); out-of-range (\"ab\" at 5) → None → -1.
        n=7 → 1000·233-1 = 232999; n=0 → 1000·97-1 = 96999. Byte-idempotent; hop/direct live parity
        (the interim scalar-at result-cell leak-pin, 1/call).")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (*
            1000
            (match
              (String.scalar-at "café" (if (> n 0) 3 1))
              ((Some c) (Char.to-int c))
              ((None _u) -1)))
          (match (String.scalar-at "ab" 5) ((Some c) (Char.to-int c)) ((None _u) -1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 232999 Int64))
  (call main (: 0 Int64))
  (output (: 96999 Int64))
  (live-objects known-leak))

; `String` in type position is transparent over a string value, but a MISMATCH is rejected: `(: "hi" Int64)`
; conflicts the String value with the Int64 annotation (CDZ0203), the String counterpart of `(: 5 Bool)`.
; (Migrated from rcdzc a_string_annotation_checks_against_a_string_value.)
(case
  "a string value annotated as a non-String type is a mismatch"
  (input (do (def (main) (String.byte-len (: "hi" Int64))) (export main)))
  (error CDZ0203))

; A CONSTANT non-String operand to a String op reaches the const-fold path BEFORE the operand type-check.
; It must still surface the SAME CDZ0203 "expects an argument of type String" the RUNTIME operand gets — NOT
; a misleading const-fold decline about UTF-8 decoding / const-ASCII folding, and never an UNCODED reject.
; (Was a bug: `(String.byte-len 5)` → misleading CDZ0900; `(String.at 5 0)` / `(String.concat "a" 5)` → uncoded.
; Fixed by routing the const-fold decline through `runtime_string_op_decline`, whose neutral decline `dedup_faults`
; drops when the coded type error is present. Repro from v-rcdzc-ts-2; verdict = the runtime CDZ0203 above.)
(case
  "a constant non-String operand to String.byte-len is a type mismatch, not a const-fold decline"
  (input (do (def (main) (String.byte-len 5)) (export main)))
  (error CDZ0203))

(case
  "a constant non-String operand to String.at is a type mismatch, not an uncoded decline"
  (input (do (def (main) (String.at 5 0)) (export main)))
  (error CDZ0203))

(case
  "a constant non-String operand to String.concat is a type mismatch, not an uncoded decline"
  (input (do (def (main) (String.concat "a" 5)) (export main)))
  (error CDZ0203))

(case
  "a constant non-String operand to String.slice is a type mismatch, not an uncoded decline"
  (input (do (def (main) (String.slice 5 0 1)) (export main)))
  (error CDZ0203))

; PARITY twins (v-spec-oracle #6861): the RUNTIME-operand form of each op gives the SAME CDZ0203 as its
; constant twin above — const-ness is type-irrelevant, so an optimization (const-fold) must not change
; whether/how the program is rejected. These witness const == runtime diagnostic parity.
(case
  "a runtime non-String operand to String.byte-len is the same CDZ0203 as its constant twin"
  (input (do (def (f (: n Int64)) (String.byte-len n)) (export f)))
  (error CDZ0203))

(case
  "a runtime non-String operand to String.at is the same CDZ0203 as its constant twin"
  (input (do (def (f (: n Int64)) (String.at n 0)) (export f)))
  (error CDZ0203))

(case
  "a runtime non-String operand to String.concat is the same CDZ0203 as its constant twin"
  (input (do (def (f (: n Int64)) (String.concat "a" n)) (export f)))
  (error CDZ0203))

(case
  "a runtime non-String operand to String.slice is the same CDZ0203 as its constant twin"
  (input (do (def (f (: n Int64)) (String.slice n 0 1)) (export f)))
  (error CDZ0203))

; ssx1: scalar addressing over an ASTRAL scalar at a RUNTIME index. The scalar-at cases above use
; constant string+index, so they fold at compile time and never exercise the runtime decode path.
; Here the index is a runtime parameter over "a𝄞é!" (1-byte a, 4-byte astral 𝄞 U+1D11E, 2-byte NFC
; é, 1-byte !): `String.at` returns the whole 1-SCALAR substring (byte-len 4 at the astral — scalar-
; indexed, never a byte or a surrogate half) and `String.scalar-at` the scalar itself
; (Char.to-int 119070 = U+1D11E). Both are None past the 4-scalar end. Encodes bat(n) + 1000*sat(n).
; (breaker probe su2, verified tri-target exact + byte-idempotent, live-objects clean.)
(case
  "String.at and String.scalar-at at a runtime index are scalar-indexed across an astral char"
  (input
    (do
      (def
        (sat (: n Int64))
        (match (String.scalar-at "a𝄞é!" n) ((Some c) (Char.to-int c)) ((None) -1)))
      (def
        (bat (: n Int64))
        (match (String.at "a𝄞é!" n) ((Some t) (String.byte-len t)) ((None) -1)))
      (def (main (: n Int64)) (+ (bat n) (* 1000 (sat n))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 119070004 Int64))
  (call main (: 2 Int64))
  (output (: 233002 Int64))
  (call main (: 4 Int64))
  (output (: -1001 Int64)))

; ssx2: String.slice with a RUNTIME start over the same astral string — scalar-window [i, i+2) as an
; Option. In-range windows land on scalar boundaries regardless of byte widths ([1,3) spans the
; 4-byte 𝄞 + 2-byte é = byte-len 6, scalar-len 2); a window whose END passes the 4-scalar length is
; None (not a clamp, not a trap). Encodes byte-len + 100*scalar-len of the slice, -1 for None.
; (breaker probe su1, verified tri-target exact + byte-idempotent, const twin = runtime value.)
(case
  "String.slice with a runtime start is a scalar window and None past the end"
  (input
    (do
      (def
        (probe (: s String) (: i Int64))
        (match
          (String.slice s i (+ i 2))
          ((Some t) (+ (String.byte-len t) (* 100 (String.scalar-len t))))
          ((None) -1)))
      (def (main (: n Int64)) (probe "a𝄞é!" n))
      (export main)))
  (call main (: 0 Int64))
  (output (: 205 Int64))
  (call main (: 1 Int64))
  (output (: 206 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64)))

; cfx1: the Char.from-int VALID-SCALAR DOMAIN at a RUNTIME argument. The from-int cases above pin the
; boundary points as CONSTANTS (compile-time fold); this fences the same domain when the integer is a
; runtime parameter — the check must execute: Some on [0, 0xD7FF] and [0xE000, 0x10FFFF] including
; both inner edges (55295 = U+D7FF, 57344 = U+E000) and the max (1114111 = U+10FFFF); None (as data,
; never a trap, never an ill-formed Char) on the surrogate block entry (55296 = U+D800) and one past
; the max (1114112 = U+110000). Negative-at-runtime rides the same check (const twin above). Encodes
; to-int of the Some payload, -1 for None. (breaker probe cf1, verified tri-target exact at 9
; boundary args + byte-idempotent hop; rust leg mixes const and fold-opaque runtime facets = 307
; validity bitmap, const twin == runtime.)
(case
  "the Char.from-int scalar-domain check executes on a runtime integer"
  (input
    (do
      (def (main (: n Int64)) (match (Char.from-int n) ((Some c) (Char.to-int c)) ((None u) -1)))
      (export main)))
  (call main (: 55295 Int64))
  (output (: 55295 Int64))
  (call main (: 55296 Int64))
  (output (: -1 Int64))
  (call main (: 57344 Int64))
  (output (: 57344 Int64))
  (call main (: 1114111 Int64))
  (output (: 1114111 Int64))
  (call main (: 1114112 Int64))
  (output (: -1 Int64)))

; bxx1: Bytes.slice takes a LENGTH, not an end — the sibling CONTRAST with String.slice's scalar
; [start, END) window (ssx2 above). Over #list(10 20 30 40): slice(1,3) is THREE bytes from index 1
; (an end-reading would give two), slice(2,2) is the two-byte tail (an end-reading would give the
; empty window), slice(2,3) overruns -> None (strict bounds, no clamp), slice(0,4) is the last-fits
; boundary (length == remaining is a match), and slice(0,0) is Some EMPTY (encoded -10: len 0, at(0)
; on it -> None -> -1). This is the documented divergence the range-as-a-first-class-value proposal
; would migrate (its corpus reframe flips exactly these pins), so the current meaning is locked
; deliberately. Encodes 10*first-byte + len for Some, -1 for None. (breaker probe bxx, verified
; tri-target exact + byte-idempotent, both faces of the at/len machinery exercised on the slice
; result.)
(case
  "Bytes.slice third argument is a length and bounds are strict"
  (input
    (do
      (def
        (main (: a Int64) (: k Int64))
        (let
          ((b (Bytes.of #list(10 20 30 40))))
          (match
            (Bytes.slice b a k)
            ((Some t)
              (+ (* 10 (match (Bytes.at t 0) ((Some v) (Int64.of v)) ((None) -1))) (Bytes.len t)))
            ((None) -1))))
      (export main)))
  (call main (: 1 Int64) (: 3 Int64))
  (output (: 203 Int64))
  (call main (: 2 Int64) (: 2 Int64))
  (output (: 302 Int64))
  (call main (: 2 Int64) (: 3 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64) (: 4 Int64))
  (output (: 104 Int64))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: -10 Int64)))

; sgx1: the STRING (rope) instance of the base-case-consume leak class (lgx1 is the List instance) —
; a recursive worker that CONSUMES its threaded String accumulator via String.byte-len at the
; recursion base case leaks (value-correct). `worker acc = if (> (String.byte-len acc) 2)
; (String.byte-len acc) else worker (String.concat acc "x")` — builds "xxx", returns 3. Leaks 1 rope
; husk. CORRECTED (tick 1546): the trigger is the base-case scalar-consume of the threaded
; accumulator (not the loop guard — see lgx1's corrected note), and it is TYPE-GENERAL (List/Map/
; String all leak; escape-then-inspect reclaims 0). The rope reclaim path is distinct from lgx1's
; list path, so the fix must cover it too. Pinned known-leak + filed. (breaker probe slg.)
(case
  "a string accumulator inspected by byte-len in the recursion guard reclaims (leaks pending loop-guard-borrow reclaim)"
  (input
    (do
      (def
        (worker (: acc String))
        (if (> (String.byte-len acc) 2) (String.byte-len acc) (worker (String.concat acc "x"))))
      (def (main (: n Int64)) (worker ""))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))
