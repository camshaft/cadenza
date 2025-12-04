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

    #[expect(dead_code)]
    pub fn next_if_in<V>(&mut self, v: &[V]) -> Option<I::Item>
    where
        I::Item: PartialEq<V>,
    {
        self.next_if(|x| {
            for y in v {
                if x.eq(y) {
                    return true;
                }
            }
            false
        })
    }

    pub fn peek2(&mut self) -> (Option<&I::Item>, Option<&I::Item>) {
        while self.len < 2 {
            self.push();
        }
        (self.buf[0].as_ref(), self.buf[1].as_ref())
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
    use bolero::{TypeGenerator, check};

    #[derive(TypeGenerator, Clone, Debug)]
    enum Operation {
        Next,
        Peek,
        Peek2,
    }

    #[test]
    fn model_test() {
        check!()
            .with_type::<Vec<Operation>>()
            .for_each(|operations| {
                let iter = 0u8..255;
                let mut oracle = iter.clone();
                let mut subject = Peek2::new(iter);

                for op in operations {
                    match op {
                        Operation::Next => {
                            assert_eq!(oracle.next(), subject.next());
                        }
                        Operation::Peek => {
                            let mut iter = oracle.clone();
                            assert_eq!(iter.next(), subject.peek().cloned());
                        }
                        Operation::Peek2 => {
                            let mut iter = oracle.clone();
                            let (a, b) = subject.peek2();
                            assert_eq!(iter.next(), a.cloned(), "{a:?} {b:?}");
                            assert_eq!(iter.next(), b.cloned(), "{a:?} {b:?}");
                        }
                    }
                }
            });
    }
}
