/// Protocol or application error that is send between peers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Error {
    pub name: String,
    pub message: String,
}

impl Error {
    pub fn new(name: impl ToString, message: impl ToString) -> Self {
        Self {
            name: name.to_string(),
            message: message.to_string(),
        }
    }
}
