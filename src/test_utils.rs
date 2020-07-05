use proptest::test_runner::{TestError, TestRunner};
use std::future::Future;

pub use proptest::prelude::*;

pub fn proptest<T: std::fmt::Debug>(strategy: impl Strategy<Value = T>, f: impl Fn(T)) {
    let config = ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let mut runner = TestRunner::new(config);
    match runner.run(&strategy, |value| {
        f(value);
        Ok(())
    }) {
        Ok(_) => (),
        Err(TestError::Fail(reason, value)) => {
            panic!("\nTest failed:\n  {}\n  value: {:?}\n", reason, value)
        }
        Err(TestError::Abort(reason)) => panic!("\nTest aborted: {}", reason),
    }
}

pub fn proptest_async<T: std::fmt::Debug, Fut>(
    strategy: impl Strategy<Value = T>,
    f: impl Fn(T) -> Fut,
) where
    Fut: Future<Output = ()>,
{
    proptest(strategy, |value| async_std::task::block_on(f(value)))
}
