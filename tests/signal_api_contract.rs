//! Signature checks do not install process-wide handlers in the test runner.

#[test]
fn ambient_interrupt_future_can_be_owned_by_a_detached_task() {
    fn assert_send_static<T: Send + 'static>(_: T) {}
    assert_send_static(kernal_api::async_engine::wait_for_interrupt());
}

#[cfg(unix)]
#[test]
fn termination_subscription_is_owned_and_registration_is_fallible() {
    use kernal_api::async_engine::TerminationSignal;
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<TerminationSignal>();
    let _: fn() -> std::io::Result<TerminationSignal> = TerminationSignal::new;
}
