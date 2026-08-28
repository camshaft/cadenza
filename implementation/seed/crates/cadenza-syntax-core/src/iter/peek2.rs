//! Two-item lookahead over any iterator.

pub struct Peek2<I: Iterator> {
    iter: I,
    len: u8,
    ended: bool,
    buf: [Option<I::Item>; 2],
}

impl<I: Iterator> Peek2<I> {
    pub fn new(iter: I) -> Self {
        Self {
            iter,
            len: 0,
            ended: false,
            buf: [None, None],
        }
    }

    pub fn peek(&mut self) -> Option<&I::Item> {
        if self.len == 0 {
            self.push();
        }
        self.buf[0].as_ref()
    }

    /// The second lookahead item (one past `peek`).
    pub fn peek2(&mut self) -> Option<&I::Item> {
        while self.len < 2 {
            self.push();
            if self.ended {
                break;
            }
        }
        self.buf[1].as_ref()
    }

    pub fn next_if(&mut self, f: impl FnOnce(&I::Item) -> bool) -> Option<I::Item> {
        let v = self.peek()?;
        if f(v) { self.next() } else { None }
    }

    pub fn next_if_eq<V>(&mut self, v: V) -> Option<I::Item>
    where
        I::Item: PartialEq<V>,
    {
        self.next_if(|x| x.eq(&v))
    }

    fn push(&mut self) {
        debug_assert!(self.len < 2);
        let Some(next) = self.take_next() else {
            return;
        };
        self.buf[self.len as usize] = Some(next);
        self.len += 1;
    }

    fn take_next(&mut self) -> Option<I::Item> {
        if self.ended {
            return None;
        }
        let v = self.iter.next();
        if v.is_none() {
            self.ended = true;
        }
        v
    }
}

impl<I: Iterator> Iterator for Peek2<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 1 {
            self.len = 0;
            return self.buf[0].take();
        } else if self.len == 2 {
            self.len = 1;
            let v = self.buf[0].take();
            self.buf[0] = self.buf[1].take();
            return v;
        }
        self.take_next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_single_never_panic_and_report_absence() {
        // The lexer peeks at end-of-input every token, so the empty/1-item edges must be total.
        let mut e: Peek2<std::vec::IntoIter<u8>> = Peek2::new(Vec::new().into_iter());
        assert_eq!(e.peek(), None);
        assert_eq!(e.peek2(), None);
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None, "still None after end, no panic");

        let mut one = Peek2::new(vec![7u8].into_iter());
        assert_eq!(one.peek(), Some(&7));
        assert_eq!(one.peek2(), None, "no second item"); // must fill-then-stop at the early end
        assert_eq!(
            one.peek(),
            Some(&7),
            "peek is idempotent, peek2 didn't consume peek"
        );
        assert_eq!(one.next(), Some(7));
        assert_eq!(one.peek(), None);
        assert_eq!(one.next(), None);
    }

    #[test]
    fn peek_and_peek2_do_not_consume_and_next_if_is_conditional() {
        let mut p = Peek2::new(vec![1u8, 2, 3].into_iter());
        // Repeated peeks are stable and non-consuming; peek2 looks one further without disturbing peek.
        assert_eq!(p.peek(), Some(&1));
        assert_eq!(p.peek(), Some(&1));
        assert_eq!(p.peek2(), Some(&2));
        assert_eq!(p.peek(), Some(&1), "peek2 left peek untouched");
        // next_if consumes only on a match; the buffered lookahead shifts correctly afterwards.
        assert_eq!(p.next_if(|&x| x == 9), None, "no match, nothing consumed");
        assert_eq!(p.peek(), Some(&1));
        assert_eq!(p.next_if_eq(1), Some(1), "match consumes");
        assert_eq!(p.peek(), Some(&2), "buffer shifted 2 into peek slot");
        assert_eq!(p.peek2(), Some(&3));
        assert_eq!(p.next(), Some(2));
        assert_eq!(p.next(), Some(3));
        assert_eq!(p.next(), None);
    }

    /// A tiny deterministic PRNG (SplitMix64) — mirrors the crate's other unit-test PRNGs.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    #[test]
    fn differential_against_a_plain_index_model_over_random_op_sequences() {
        // Peek2's `next` shuffles a two-slot buffer (the `len == 2` arm shifts buf[1] -> buf[0]); a bug
        // there desyncs every SUBSEQUENT item, which an isolated assertion misses. Drive a random mix of
        // peek/peek2/next/next_if and check each result against a plain-index model of the SAME data, then
        // drain and require the remainder to be exactly the un-consumed tail — so any position/buffer
        // corruption surfaces as a mismatch, and no op ever panics on the empty/near-empty edges.
        let mut rng = Rng(0x9017_5eed_c0de_1234);
        for _ in 0..20_000 {
            let n = (rng.next() % 7) as usize;
            let data: Vec<u16> = (0..n as u16).collect();
            let mut p = Peek2::new(data.clone().into_iter());
            let mut idx = 0usize; // index the plain iterator would next yield
            for _ in 0..n + 4 {
                match rng.next() % 4 {
                    0 => {
                        let want = data.get(idx);
                        assert_eq!(p.peek(), want, "peek at idx {idx} of {data:?}");
                    }
                    1 => {
                        let want = data.get(idx + 1);
                        assert_eq!(p.peek2(), want, "peek2 at idx {idx} of {data:?}");
                    }
                    2 => {
                        let want = data.get(idx).copied();
                        let got = p.next();
                        assert_eq!(got, want, "next at idx {idx} of {data:?}");
                        if got.is_some() {
                            idx += 1;
                        }
                    }
                    _ => {
                        // next_if against a random target: consumes iff the head matches.
                        let tgt = (rng.next() % (n as u64 + 1)) as u16;
                        let head = data.get(idx).copied();
                        let got = p.next_if(|&x| x == tgt);
                        if head == Some(tgt) {
                            assert_eq!(got, head, "next_if match at idx {idx}");
                            idx += 1;
                        } else {
                            assert_eq!(
                                got, None,
                                "next_if non-match at idx {idx} must not consume"
                            );
                        }
                    }
                }
            }
            // Draining must yield exactly the un-consumed tail, in order. `Peek2` is itself an iterator,
            // so consume it with a `for` (the drain path exercises `next` through the buffer just the same).
            let mut rest = Vec::new();
            for v in p {
                rest.push(v);
            }
            assert_eq!(
                rest.as_slice(),
                &data[idx..],
                "drain tail for {data:?} after idx {idx}"
            );
        }
    }
}
