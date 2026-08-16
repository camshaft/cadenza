; breaker probe W — stress the cmp-walk recursion just pinned in 3c223e37b one level DEEPER:
; a LIST of user SUMS whose payload is itself a LIST — the walk must recurse list→sum→list.
; Also the discriminant-before-payload rule at the deeper level, and prefix tiebreak on the
; INNER list.
; Hand-derived (type W (Leaf Int64) (Node (List Int64))):
;   a1 = [Node [1,2]], a2 = [Node [1,3]]: outer lists len-1, elem: same disc(Node), payload [1,2]<[1,3] → true → 1.
;   b1 = [Leaf 5], b2 = [Node [0]]: disc Leaf(0) < Node(1) → true → 1 (payload never read; a walk
;     that read Node's list against Leaf's scalar would type-confuse/crash).
;   c1 = [Node [1]], c2 = [Node [1,2]]: inner prefix rule → true → 1.
;   main = 100*1 + 10*1 + 1 = 111.

(case "the compare walk recurses list-of-sums-of-lists with discriminant-first at depth"
  (input  (do
            (type W (Leaf Int64) (Node (List Int64)))
            (def (main (: k Int64))
              (+ (* 100 (if (< (list (Node (list 1 k))) (list (Node (list 1 3)))) 1 0))
                 (+ (* 10 (if (< (list (Leaf 5)) (list (Node (list 0)))) 1 0))
                    (if (< (list (Node (list 1))) (list (Node (list 1 k)))) 1 0))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 111 Int64)))
