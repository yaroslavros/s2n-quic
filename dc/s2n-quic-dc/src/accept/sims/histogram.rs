// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::{
    fmt::{self, Write as _},
    marker::PhantomData,
    time::Duration,
};

pub use hdrhistogram::*;

pub trait Value: fmt::Debug + Sized {
    fn from_u64(v: u64) -> Self;
    fn from_f64(v: f64) -> Self;
}

impl Value for f64 {
    fn from_u64(v: u64) -> Self {
        v as _
    }

    fn from_f64(v: f64) -> Self {
        v
    }
}

impl Value for Duration {
    fn from_u64(v: u64) -> Self {
        Duration::from_nanos(v)
    }

    fn from_f64(v: f64) -> Self {
        let nanos_to_secs = (Duration::from_secs(1).as_nanos() as f64).recip();
        Duration::from_secs_f64(v * nanos_to_secs)
    }
}

pub trait PrintExt<T: Counter> {
    fn display(&self, mode: PrintMode) -> PrintFmt<'_, T, f64>;
}

impl<T: Counter> PrintExt<T> for Histogram<T> {
    fn display(&self, mode: PrintMode) -> PrintFmt<'_, T, f64> {
        PrintFmt {
            histogram: self,
            mode,
            _value: PhantomData,
        }
    }
}

#[derive(Clone, Copy)]
pub enum PrintMode {
    Recorded,
    Linear { step: u64 },
    Log { start: u64, exp: f64 },
}

pub struct PrintFmt<'a, T: Counter, V: Value = f64> {
    histogram: &'a Histogram<T>,
    mode: PrintMode,
    _value: PhantomData<V>,
}

impl<'a, T: Counter> PrintFmt<'a, T> {
    pub fn with<V: Value>(self) -> PrintFmt<'a, T, V> {
        PrintFmt {
            histogram: self.histogram,
            mode: self.mode,
            _value: PhantomData,
        }
    }
}

impl<'a, T: Counter + Ord, V: Value> fmt::Display for PrintFmt<'a, T, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        print_with::<_, V>(self.histogram, self.mode, f)
    }
}

pub fn print_with<T: Counter + Ord, V: Value>(
    h: &Histogram<T>,
    mode: PrintMode,
    f: &mut fmt::Formatter,
) -> fmt::Result {
    let num_samples = h.len();
    writeln!(f, "# Number of samples = {num_samples}")?;
    if num_samples == 0 {
        return Ok(());
    }

    macro_rules! buckets {
        (| $iter:ident | $expr:expr) => {
            match mode {
                PrintMode::Recorded => {
                    let $iter = h.iter_recorded();
                    $expr
                }
                PrintMode::Linear { step } => {
                    let $iter = h.iter_linear(step);
                    $expr
                }
                PrintMode::Log { start, exp } => {
                    let $iter = h.iter_log(start, exp);
                    $expr
                }
            }
        };
    }

    let min = V::from_u64(h.min());
    let max = V::from_u64(h.max());

    writeln!(f, "#   Min = {min:?}")?;
    writeln!(f, "#   Max = {max:?}")?;

    let mean = V::from_f64(h.mean());
    let stdev = V::from_f64(h.stdev());
    // TODO variance?

    writeln!(f, "#  Mean = {mean:?}")?;
    writeln!(f, "# Stdev = {stdev:?}")?;
    writeln!(f, "#")?;

    let max_bucket_count = buckets!(|iter| iter
        .map(|b| b.count_since_last_iteration())
        .fold(0, core::cmp::max));

    const WIDTH: u64 = 50;
    let count_per_char = (max_bucket_count / WIDTH).max(1);

    writeln!(f, "# Each ∎ is a count of {count_per_char}")?;
    writeln!(f)?;

    let mut count_str = String::new();
    let widest_count = buckets!(|iter| iter.fold(0, |n, b| {
        count_str.clear();
        let count = b.count_since_last_iteration();
        write!(&mut count_str, "{count}").unwrap();
        n.max(count_str.len())
    }));

    let mut start_str = String::new();
    let (widest_start, _) = buckets!(|iter| iter.fold((0, 0), |(n, start), b| {
        start_str.clear();
        write!(&mut start_str, "{:?}", V::from_u64(start + 1)).unwrap();
        let n = n.max(start_str.chars().count());
        (n, b.value_iterated_to())
    }));

    let mut end_str = String::new();
    let widest_end = buckets!(|iter| iter.fold(0, |n, b| {
        end_str.clear();
        write!(&mut end_str, "{:?}", V::from_u64(b.value_iterated_to())).unwrap();
        n.max(end_str.chars().count())
    }));

    buckets!(|iter| {
        let mut last_value = 0;
        for bucket in iter {
            let start = last_value + 1;
            last_value = bucket.value_iterated_to();

            let count = bucket.count_since_last_iteration();

            if count == 0 {
                continue;
            }

            start_str.clear();
            write!(&mut start_str, "{:?}", V::from_u64(start)).unwrap();
            for _ in 0..widest_start - start_str.chars().count() {
                start_str.insert(0, ' ');
            }

            end_str.clear();
            write!(
                &mut end_str,
                "{:?}",
                V::from_u64(bucket.value_iterated_to())
            )
            .unwrap();
            for _ in 0..widest_end - end_str.chars().count() {
                end_str.insert(0, ' ');
            }

            count_str.clear();
            write!(&mut count_str, "{}", count).unwrap();
            for _ in 0..widest_count - count_str.len() {
                count_str.insert(0, ' ');
            }

            write!(f, "{} .. {} [ {} ]: ", start_str, end_str, count_str)?;
            for _ in 0..count / count_per_char {
                write!(f, "∎")?;
            }
            writeln!(f)?;
        }
    });

    Ok(())
}
