use sodiumoxide::crypto::secretbox;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxStreamParams {
    pub encrypt: BoxCryptParams,
    pub decrypt: BoxCryptParams,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxCryptParams {
    pub key: secretbox::Key,
    pub nonce: secretbox::Nonce,
}
