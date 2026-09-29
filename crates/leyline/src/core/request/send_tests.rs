use super::{RetryPlan, plan_retry};
use crate::core::error::{Error, Kind};
use crate::core::retry::RetryPolicy;

fn io(kind: std::io::ErrorKind) -> crate::core::Result<crate::core::Response> {
    Err(Error::new(Kind::Io).with_source(std::io::Error::new(kind, "x")))
}

#[test]
fn transient_retries_connection_loss_only() {
    let policy = RetryPolicy::transient();
    assert!(matches!(
        plan_retry(
            &io(std::io::ErrorKind::ConnectionReset),
            &policy,
            0,
            true,
            true
        ),
        RetryPlan::Backoff(_)
    ));
    assert!(matches!(
        plan_retry(
            &io(std::io::ErrorKind::PermissionDenied),
            &policy,
            0,
            true,
            true
        ),
        RetryPlan::Stop
    ));
    assert!(matches!(
        plan_retry(&Err(Error::new(Kind::Timeout)), &policy, 0, true, true),
        RetryPlan::Backoff(_)
    ));
}

#[test]
fn no_policy_never_retries() {
    let policy = RetryPolicy::none();
    assert!(matches!(
        plan_retry(
            &io(std::io::ErrorKind::ConnectionReset),
            &policy,
            0,
            true,
            true
        ),
        RetryPlan::Stop
    ));
}
