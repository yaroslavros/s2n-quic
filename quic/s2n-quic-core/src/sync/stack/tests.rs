// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use alloc::collections::VecDeque;
use bolero::*;
use core::fmt;

struct Model<T> {
    oracle: VecDeque<T>,
    subject: Stack<T>,
}

impl<T: Clone + PartialEq + fmt::Debug> Model<T> {
    fn new(cap: usize) -> Self {
        let subject = Stack::new(cap);
        let oracle = VecDeque::new();
        Self { oracle, subject }
    }

    fn push(&mut self, value: T) {
        //eprintln!("PUSH: {value:?}");
        // if we're at capacity then we need to drop the oldest item
        if self.subject.capacity() == self.oracle.len() {
            let _ = self.oracle.pop_front();
        }
        self.oracle.push_back(value.clone());
        self.subject.push(value);

        assert_eq!(self.oracle.is_empty(), self.subject.is_empty());
        assert_eq!(self.oracle.len(), self.subject.len());
        if self.subject.is_full() {
            assert_eq!(self.subject.len(), self.subject.capacity());
        }
    }

    fn pop(&mut self) {
        //eprintln!("POP: ...");
        let expected = self.oracle.pop_back();
        let actual = self.subject.pop();
        assert_eq!(expected, actual);
        //eprintln!("   : {actual:?}");
    }

    fn retain_bottom<F: FnMut(&T) -> ControlFlow<bool>>(&mut self, mut f: F) {
        //eprintln!("RETAIN BOTTOM");
        self.subject.retain_bottom(|v| {
            let res = f(v);
            match res {
                ControlFlow::Continue(()) | ControlFlow::Break(false) => {
                    let expected = self.oracle.pop_front().unwrap();
                    assert_eq!(v, &expected);
                }
                ControlFlow::Break(true) => {
                    // noop
                }
            }
            res
        })
    }

    fn finish(&mut self) {
        while !self.oracle.is_empty() {
            self.pop();
        }

        self.pop();
    }
}

#[derive(Clone, Copy, Debug, TypeGenerator)]
enum Op {
    Push { count: u8 },
    Pop { count: u8 },
    Retain { count: u8 },
}

#[test]
fn differential_test() {
    check!()
        .with_generator((2..=256, gen::<Vec<Op>>()))
        .for_each(|(cap, ops)| {
            let mut v = 0u64;
            let mut model = Model::new(*cap);
            for op in ops {
                match *op {
                    Op::Push { count } => {
                        for _ in 0..count {
                            model.push(Box::new(v));
                            v += 1;
                        }
                    }
                    Op::Pop { count } => {
                        for _ in 0..count {
                            model.pop();
                        }
                    }
                    Op::Retain { mut count } => model.retain_bottom(|_v| {
                        if count > 0 {
                            count -= 1;
                            ControlFlow::Continue(())
                        } else {
                            ControlFlow::Break(true)
                        }
                    }),
                }
            }
            model.finish();
        })
}

#[test]
fn push_pop_test() {
    for cap in 2..8 {
        for count in 1..=(cap * 2) {
            eprintln!("====== CAP: {cap}, COUNT: {count} ======");
            let mut v = 0;
            let mut model = Model::new(cap);
            for _ in 0..cap {
                model.pop();

                for _ in 0..count {
                    model.push(v);
                    v += 1;
                }

                for _ in 0..count {
                    model.pop();
                }
            }
            model.finish();
        }
    }
}

#[test]
fn cursor_nav() {
    check!()
        .with_generator((gen::<usize>(), 1usize..=32))
        .cloned()
        .for_each(|(value, cap)| {
            // One lap is the smallest power of two greater than `cap`.
            let one_lap = (cap + 1).next_power_of_two();

            let mut current = Cursor {
                value,
                one_lap,
                cap,
            };

            // if `value` ended up being invalid for the given capacity then make it valid
            if let Some(diff) = current.index(one_lap).checked_sub(cap) {
                current = current.with_value(value.wrapping_sub(diff + 1));
                assert!(current.index(one_lap) < cap);
            }

            let next = current.next(one_lap, cap);
            assert_ne!(current, next);

            let prev = next.prev(one_lap, cap);
            assert_eq!(
                current,
                prev,
                "current={:?} prev={:?} next={:?}",
                (current, current.lap(one_lap), current.index(one_lap),),
                (prev, prev.lap(one_lap), prev.index(one_lap),),
                (next, next.lap(one_lap), next.index(one_lap),)
            );
        })
}

#[test]
fn stress_test() {
    use std::{thread, time::Duration};

    let stack = Stack::<Box<u64>>::new(4096);
    let stack = &stack;

    thread::scope(|s| {
        s.spawn(|| loop {
            if let Some(v) = stack.pop() {
                (0u64..16).contains(&v);
            } else {
                thread::yield_now();
            }
        });

        for i in 0u64..2 {
            s.spawn(move || loop {
                for _ in 0..(i + 1) * 10000 {
                    if let Some(v) = stack.push(Box::new(i)) {
                        (0u64..16).contains(&v);
                    }
                }
                thread::yield_now();
            });
        }
    });
}
