//! [load] SSB identity keys from "secret" file.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::crypto;

#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    /// Failed to read file
    #[error("Cannot read file {path}")]
    ReadIo {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    /// Unknown key scheme used in the secret file
    #[error("Unknown key scheme")]
    UnknownKeyScheme,

    /// Failed to decode base64 string
    #[error("Failed to decode base64 string")]
    Base64(
        #[source]
        #[from]
        base64::DecodeError,
    ),

    /// Invalid length of the secret key
    #[error("Invalid secret key length {0}")]
    InvalidSecretKeyLength(usize),

    /// Failed to decode secret file as JSON
    #[error("Failed to decode JSON")]
    Json(
        #[source]
        #[from]
        serde_json::Error,
    ),

    /// The home dir is not set.
    #[error("Cannot determine home directory")]
    NoHomeDir,
}

/// Load secret key from an SSB "secret" file
pub fn load(path: &Path) -> Result<crypto::sign::SecretKey, LoadError> {
    let data = fs::read_to_string(path).map_err(|error| LoadError::ReadIo {
        path: path.to_owned(),
        error,
    })?;
    parse(&data)
}

/// `load()` secret key default secret file `~/.ssb/secret`.
pub fn load_default() -> Result<crypto::sign::SecretKey, LoadError> {
    let home_dir = dirs::home_dir().ok_or_else(|| LoadError::NoHomeDir)?;
    let path = home_dir.join(".ssb").join("secret");
    load(&path)
}

fn parse(data: &str) -> Result<crypto::sign::SecretKey, LoadError> {
    #[derive(serde::Deserialize)]
    struct Secret {
        private: String,
    }

    let data = strip_comments(data);
    let Secret { private } = serde_json::from_str(&data)?;
    let key_data = match private.split('.').collect::<Vec<&str>>().as_slice() {
        [key_data, scheme] if *scheme == "ed25519" => *key_data,
        _ => return Err(LoadError::UnknownKeyScheme),
    };

    let key_data = base64::decode(key_data)?;
    if key_data.len() != crypto::sign::SECRETKEYBYTES {
        return Err(LoadError::InvalidSecretKeyLength(key_data.len()));
    }
    // `unwrap()` only fails if length of key_data is not `crypto::sign::SECRETKEYBYTES` which we
    // check above.
    Ok(crypto::sign::SecretKey::from_slice(&key_data).unwrap())
}

fn strip_comments(data: &str) -> String {
    data.lines()
        .map(|line| if line.starts_with('#') { "" } else { line })
        .collect()
}

#[test]
fn parse_ok() {
    let expected_key = crypto::sign::SecretKey::from_slice(&[1u8; 64][..]).unwrap();
    dbg!(base64::encode(expected_key.as_ref()));
    let data = r#"
# if any one learns this name, they can use it to destroy your identity
# NEVER show this to anyone!!!
#
{
  "curve": "ed25519",
  "public": "OyxgIfCyQUU4GHEvpfe1iwT3iyfEsaxRoRo4uw3XUMA=.ed25519",
  "private": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ==.ed25519",
  "id": "@OyxgIfCyQUU4GHEvpfe1iwT3iyfEsaxRoRo4uw3XUMA=.ed25519"
}"#;

    let key = parse(data).unwrap();
    assert_eq!(key, expected_key);
}
