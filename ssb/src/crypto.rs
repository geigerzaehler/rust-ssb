//! Facade for [`sodiumoxide::crypto`].
//!
//! Every submodule re-exports items from the corresponding [sodiumoxide::crypto] module.
use sodiumoxide::crypto::{hash::sha256, scalarmult::curve25519};
use std::convert::TryFrom;

pub use sodiumoxide::crypto::box_;

pub mod auth {
    pub use sodiumoxide::crypto::auth::*;

    pub fn key_from_array(bytes: &[u8; 32]) -> Key {
        Key::from_slice(bytes).unwrap()
    }
}

pub mod secretbox {
    pub use sodiumoxide::crypto::secretbox::*;

    pub fn key_from_array(bytes: &[u8; 32]) -> Key {
        Key::from_slice(bytes).unwrap()
    }
}

pub mod sign {
    pub use sodiumoxide::crypto::sign::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct KeyPair {
        pub public: PublicKey,
        pub secret: SecretKey,
    }

    impl KeyPair {
        pub fn new(public: PublicKey, secret: SecretKey) -> Self {
            Self { public, secret }
        }

        /// See [gen_keypair].
        pub fn gen() -> Self {
            let (public, secret) = gen_keypair();
            Self::new(public, secret)
        }
    }
}

pub fn share_key(
    public_key: &box_::PublicKey,
    secret_key: &box_::SecretKey,
) -> Option<box_::SecretKey> {
    let group_element = curve25519::GroupElement::from_slice(public_key.as_ref()).unwrap();
    let scalar = curve25519::Scalar::from_slice(secret_key.as_ref()).unwrap();
    let shared = curve25519::scalarmult(&scalar, &group_element).ok()?;
    Some(box_::SecretKey::from_slice(shared.as_ref()).unwrap())
}

/// Alternative to [sha256::hash] with a nicer interface.
pub fn hash(data: impl AsRef<[u8]>) -> [u8; 32] {
    <[u8; 32]>::try_from(sha256::hash(data.as_ref()).as_ref()).unwrap()
}

/// Convert a sign key to an exchange key.
pub fn sign_to_box_pk(&public_key: &sign::PublicKey) -> Option<box_::PublicKey> {
    let mut curve25519_pk = [0u8; box_::PUBLICKEYBYTES];
    let result = unsafe {
        libsodium_sys::crypto_sign_ed25519_pk_to_curve25519(
            curve25519_pk.as_mut_ptr(),
            public_key.as_ref().as_ptr(),
        )
    };

    if result == 0 {
        Some(box_::PublicKey::from_slice(&curve25519_pk).unwrap())
    } else {
        None
    }
}

pub fn sign_to_box_sk(secret_key: &sign::SecretKey) -> Option<box_::SecretKey> {
    let mut curve25519_sk = [0u8; box_::SECRETKEYBYTES];
    let result = unsafe {
        libsodium_sys::crypto_sign_ed25519_sk_to_curve25519(
            curve25519_sk.as_mut_ptr(),
            secret_key.as_ref().as_ptr(),
        )
    };

    if result == 0 {
        Some(box_::SecretKey::from_slice(&curve25519_sk).unwrap())
    } else {
        None
    }
}
