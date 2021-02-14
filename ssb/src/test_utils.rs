pub use proptest::prelude::*;
pub use proptest::test_runner::TestCaseResult;
pub use test_strategy::proptest;

#[macro_export]
macro_rules! prop_reject {
    () => {
        return ::core::result::Result::Err(::proptest::test_runner::TestCaseError::reject(
            "Rejected value",
        ));
    };
    ($msg:expr) => {
        return ::core::result::Result::Err(::proptest::test_runner::TestCaseError::reject($msg));
    };
}
