use crate::{Opts, Result};
use core::{fmt::Debug, time::Duration};
use hdrhistogram::{sync::Recorder as Rec, Histogram, SyncHistogram};
use std::{io, time::Instant};

pub struct Stats {
    hist: SyncHistogram<u64>,
    epoch: Instant,
}

impl Default for Stats {
    fn default() -> Self {
        let max = Duration::from_secs(60);
        let epoch = Instant::now();

        let hist = Histogram::new_with_max(max.as_nanos() as _, 3)
            .unwrap()
            .into_sync();
        Self { hist, epoch }
    }
}

impl Stats {
    pub fn recorders(&self, count: usize) -> Vec<Recorder> {
        (0..count).map(|_| self.recorder()).collect()
    }

    pub fn recorder(&self) -> Recorder {
        let hist = self.hist.recorder();
        Recorder {
            hist,
            epoch: self.epoch,
        }
    }

    pub fn spawn(mut self, opts: &Opts) {
        let tsv = opts.tsv;

        let mut print = move || {
            self.hist.refresh_timeout(Duration::from_secs(1));

            let stdout = std::io::stdout();
            let stdout = stdout.lock();

            self.quantiles(stdout, 20, 1, tsv).unwrap();
        };

        if let Some(time) = opts.time {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(time));

                print();

                std::process::exit(0);
            });
        } else {
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                print();
            });
        }
    }

    fn quantiles<W: std::io::Write>(
        &self,
        mut writer: W,
        quantile_precision: usize,
        ticks_per_half: u32,
        tsv: bool,
    ) -> Result<()> {
        if tsv {
            let mean = Duration::from_nanos(self.hist.mean() as _);
            let stdev = Duration::from_nanos(self.hist.stdev() as _);
            let max = Duration::from_nanos(self.hist.max() as _);
            let min = Duration::from_nanos(self.hist.min() as _);
            let len = self.hist.len();
            let time = self.epoch.elapsed();
            let tps = (1.0 / time.as_secs_f64()) * self.hist.len() as f64;

            writeln!(
                writer,
                "{mean:?}\t{stdev:?}\t{min:?}\t{max:?}\t{len:?}\t{time:?}\t{tps:?}"
            )?;

            return Ok(());
        }

        writeln!(
            writer,
            "{:>12} {:>quantile_precision$} {:>quantile_precision$} {:>10} {:>14}\n",
            "Value",
            "QuantileValue",
            "QuantileIteration",
            "TotalCount",
            "1/(1-Quantile)",
            quantile_precision = quantile_precision + 2 // + 2 from leading "0." for numbers
        )?;
        for v in self.hist.iter_quantiles(ticks_per_half) {
            let count = v.count_since_last_iteration();
            let value = Duration::from_nanos(v.value_iterated_to());
            if v.quantile_iterated_to() < 1.0 {
                writeln!(
                    writer,
                    "{:12?} {:1.*} {:1.*} {:10} {:14.2}",
                    value,
                    quantile_precision,
                    v.quantile(),
                    quantile_precision,
                    v.quantile_iterated_to(),
                    count,
                    1_f64 / (1_f64 - v.quantile_iterated_to())
                )?;
            } else {
                writeln!(
                    writer,
                    "{:12?} {:1.*} {:1.*} {:10} {:>14}",
                    value,
                    quantile_precision,
                    v.quantile(),
                    quantile_precision,
                    v.quantile_iterated_to(),
                    count,
                    "∞"
                )?;
            }
        }

        fn write_extra_data<T1: Debug, T2: Debug, W: std::io::Write>(
            writer: &mut W,
            label1: &str,
            data1: T1,
            label2: &str,
            data2: T2,
        ) -> Result<(), io::Error> {
            writeln!(
                writer,
                "#[{:10} = {:12.2?}, {:14} = {:12.2?}]",
                label1, data1, label2, data2
            )
        }

        write_extra_data(
            &mut writer,
            "Mean",
            Duration::from_nanos(self.hist.mean() as _),
            "StdDeviation",
            Duration::from_nanos(self.hist.stdev() as _),
        )?;
        write_extra_data(
            &mut writer,
            "Max",
            Duration::from_nanos(self.hist.max() as _),
            "Total count",
            self.hist.len(),
        )?;

        let time = self.epoch.elapsed();
        let tps = (1.0 / time.as_secs_f64()) * self.hist.len() as f64;

        write_extra_data(&mut writer, "TPS", tps, "Time", time)?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct Recorder {
    hist: Rec<u64>,
    epoch: Instant,
}

impl Recorder {
    pub fn timestamp(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as _
    }

    pub fn record(&mut self, timestamp: u64) {
        let then = self.epoch + Duration::from_nanos(timestamp);
        let elapsed = then.elapsed();
        self.hist.saturating_record(elapsed.as_nanos() as _);
    }
}
