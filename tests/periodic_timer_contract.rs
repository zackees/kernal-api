//! Frozen cadence behavior for maintenance and index-writer timers.

use kernal_api::async_engine::{MissedTickBehavior, PeriodicTimer};
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn legacy_periods_above_one_year_do_not_change_the_bounded_constructor() {
    let period = Duration::from_secs(366 * 24 * 60 * 60);
    assert!(PeriodicTimer::new(period).is_err());
    let mut timer = PeriodicTimer::new_unbounded(period).expect("legacy long period");
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let start = tokio::time::Instant::now();
    timer.tick().await;
    assert_eq!(tokio::time::Instant::now(), start);
    assert!(PeriodicTimer::new_unbounded(Duration::ZERO).is_err());
}

#[tokio::test(start_paused = true)]
async fn unrepresentable_legacy_period_is_rejected_before_first_tick() {
    let error = PeriodicTimer::new_unbounded(Duration::MAX)
        .expect_err("host clock cannot represent Duration::MAX");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test(start_paused = true)]
async fn default_timer_retains_immediate_first_tick_and_burst_catchup() {
    let mut timer = PeriodicTimer::new(Duration::from_secs(1)).expect("timer");
    let start = tokio::time::Instant::now();
    timer.tick().await;
    assert_eq!(tokio::time::Instant::now(), start);
    tokio::time::advance(Duration::from_millis(3500)).await;
    let late = tokio::time::Instant::now();
    for _ in 0..3 {
        timer.tick().await;
        assert_eq!(
            tokio::time::Instant::now(),
            late,
            "missed tick should be immediately available"
        );
    }
    let mut next = pin!(timer.tick());
    assert_eq!(
        std::future::Future::poll(next.as_mut(), &mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_a_pending_tick_does_not_consume_it() {
    let mut timer = PeriodicTimer::new(Duration::from_secs(1)).expect("timer");
    timer.tick().await;
    {
        let mut tick = pin!(timer.tick());
        assert_eq!(
            std::future::Future::poll(tick.as_mut(), &mut Context::from_waker(Waker::noop())),
            Poll::Pending
        );
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    let expected = tokio::time::Instant::now();
    timer.tick().await;
    assert_eq!(tokio::time::Instant::now(), expected);
}

#[tokio::test(start_paused = true)]
async fn delay_policy_reschedules_from_the_late_tick_without_a_burst() {
    let mut timer = PeriodicTimer::new(Duration::from_secs(1)).expect("timer");
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let start = tokio::time::Instant::now();
    timer.tick().await;
    assert_eq!(
        tokio::time::Instant::now(),
        start,
        "initial tick remains immediate"
    );
    tokio::time::advance(Duration::from_millis(3500)).await;
    timer.tick().await;
    let late = tokio::time::Instant::now();
    {
        let mut next = pin!(timer.tick());
        assert_eq!(
            std::future::Future::poll(next.as_mut(), &mut Context::from_waker(Waker::noop())),
            Poll::Pending
        );
    }
    timer.tick().await;
    assert_eq!(tokio::time::Instant::now(), late + Duration::from_secs(1));
}
