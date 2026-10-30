use crate::core::error::{Cause, Error};
use crate::core::retry::Retry;
use crate::core::retry::RetryPlan;

fn io(kind: std::io::ErrorKind) -> crate::core::Result<crate::core::Response> {
    Err(Error::new(Cause::Io).with_source(std::io::Error::new(kind, "x")))
}

#[test]
fn transient_retries_connection_loss_only() {
    let policy = Retry::new(3);
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::ConnectionReset), 0, true, || true),
        RetryPlan::Backoff(_)
    ));
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::PermissionDenied), 0, true, || true),
        RetryPlan::Stop
    ));
    assert!(matches!(
        policy.plan(&Err(Error::new(Cause::Timeout)), 0, true, || true),
        RetryPlan::Backoff(_)
    ));
}

#[test]
fn no_policy_never_retries() {
    let policy = Retry::new(0);
    assert!(matches!(
        policy.plan(&io(std::io::ErrorKind::ConnectionReset), 0, true, || true),
        RetryPlan::Stop
    ));
}
