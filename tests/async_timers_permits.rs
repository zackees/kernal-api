use kernal_api::async_engine::{self, Deadline, PeriodicTimer, Semaphore};
use std::time::Duration;

#[test]
fn immediate_permits_share_capacity_and_release_on_drop() {
    let pool = Semaphore::new(1);
    let clone = pool.clone();
    let permit = pool.try_acquire().unwrap();
    assert!(clone.try_acquire().is_none());
    drop(pool);
    drop(permit);
    assert!(clone.try_acquire().is_some());
    assert!(Semaphore::new(0).try_acquire().is_none());
}

#[tokio::test(start_paused = true)]
async fn deadline_waits_keep_the_original_expiry() {
    let deadline = Deadline::after(Duration::from_secs(10));
    async_engine::sleep(Duration::from_secs(4)).await;
    let start = tokio::time::Instant::now();
    async_engine::sleep_until(deadline).await;
    assert_eq!(start.elapsed(), Duration::from_secs(6));
    assert!(deadline.is_elapsed());
    async_engine::sleep_until(deadline).await;
    assert_eq!(start.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn periodic_ticks_are_immediate_then_fixed_cadence_with_burst_catchup() {
    let mut timer = PeriodicTimer::new(Duration::from_secs(10)).unwrap();
    let start = tokio::time::Instant::now();
    timer.tick().await;
    assert_eq!(start.elapsed(), Duration::ZERO);
    // Dropping a pending tick does not consume its scheduled occurrence.
    assert!(async_engine::timeout(Duration::from_secs(4), timer.tick())
        .await
        .is_err());
    timer.tick().await;
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    async_engine::sleep(Duration::from_secs(25)).await;
    timer.tick().await;
    timer.tick().await;
    assert_eq!(start.elapsed(), Duration::from_secs(35));
    timer.tick().await;
    assert_eq!(start.elapsed(), Duration::from_secs(40));
}

#[tokio::test]
async fn periodic_timer_rejects_zero_and_unbounded_periods() {
    assert!(PeriodicTimer::new(Duration::ZERO).is_err());
    assert!(PeriodicTimer::new(Duration::MAX).is_err());
    let maximum = Duration::from_secs(365 * 24 * 60 * 60);
    assert!(PeriodicTimer::new(maximum).is_ok());
    assert!(PeriodicTimer::new(maximum + Duration::from_nanos(1)).is_err());
}

#[test]
fn periodic_timer_requires_an_entered_runtime() {
    assert!(PeriodicTimer::new(Duration::from_secs(1)).is_err());
}
