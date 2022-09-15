// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use futures::FutureExt;
use s2n_quic_core::time::{self, Clock as ClockTrait, Timestamp};
use tokio::time::{sleep_until, Instant, Sleep};

#[derive(Clone, Debug)]
pub struct Clock(Instant);

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Self(Instant::now())
    }

    pub fn timer(&self) -> Timer {
        Timer::new(self.clone())
    }
}

impl ClockTrait for Clock {
    fn get_time(&self) -> time::Timestamp {
        let duration = self.0.elapsed();
        unsafe {
            // Safety: time duration is only derived from a single `Instant`
            time::Timestamp::from_duration(duration)
        }
    }
}

#[derive(Debug)]
pub struct Timer {
    /// A reference to the current clock
    clock: Clock,
    /// The `Instant` at which the timer should expire
    target: Option<Instant>,
    /// The handle to the timer entry in the tokio runtime
    sleep: Pin<Box<Sleep>>,
    fast_wakeup: FastWakeup,
}

impl Timer {
    fn new(clock: Clock) -> Self {
        /// We can't create a timer without first arming it to something, so just set it to 1s in
        /// the future.
        const INITIAL_TIMEOUT: Duration = Duration::from_secs(1);

        let target = clock.0 + INITIAL_TIMEOUT;
        let sleep = Box::pin(sleep_until(target));
        Self {
            clock,
            target: Some(target),
            sleep,
            fast_wakeup: FastWakeup::Idle,
        }
    }

    /// Modifies the target expiration timestamp for the timer
    pub fn update(&mut self, timestamp: Timestamp, now: Timestamp) {
        let delay = unsafe {
            // Safety: the same clock epoch is being used
            timestamp.as_duration()
        };
        let now = unsafe {
            // Safety: the same clock epoch is being used
            now.as_duration()
        };

        // floor the durations to milliseconds to reduce timer churn
        let delay = Duration::from_millis(delay.as_millis() as u64);
        let now = Duration::from_millis(now.as_millis() as u64);

        // add the delay to the clock's epoch
        let next_time = self.clock.0 + delay;

        // If the target hasn't changed then don't do anything
        if Some(next_time) == self.target {
            return;
        }

        // remember the target time
        self.target = Some(next_time);

        // check if the target has already passed, if so yield and wakeup on the next loop
        if delay <= now {
            self.fast_wakeup.enable();
        } else {
            self.fast_wakeup.idle();
            // if the clock has changed let the sleep future know
            self.sleep.as_mut().reset(next_time);
        }
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Only poll the inner timer if we have a target set
        if self.target.is_none() {
            return Poll::Pending;
        }

        // check if the fast wakeup is ready
        if self.fast_wakeup.poll_unpin(cx).is_ready() {
            return Poll::Ready(());
        }

        let res = self.sleep.as_mut().poll(cx);

        if res.is_ready() {
            // clear the target after it fires, otherwise we'll endlessly wake up the task
            self.target = None;
        }

        res
    }
}

#[derive(Copy, Clone, Debug)]
enum FastWakeup {
    Idle,
    Yield,
    Ready,
    Cooldown,
}

impl FastWakeup {
    fn enable(&mut self) {
        match self {
            Self::Cooldown => *self = Self::Yield,
            _ => *self = Self::Ready,
        }
    }

    fn idle(&mut self) {
        *self = Self::Idle;
    }
}

impl Future for FastWakeup {
    type Output = ();

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match *self {
            Self::Idle | Self::Cooldown => Poll::Pending,
            Self::Yield => {
                eprint!(".");
                cx.waker().wake_by_ref();
                *self = Self::Ready;
                Poll::Pending
            }
            Self::Ready => {
                *self = Self::Cooldown;
                Poll::Ready(())
            }
        }
    }
}