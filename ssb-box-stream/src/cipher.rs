use std::convert::{TryFrom as _, TryInto as _};

use crate::crypto;

/// Size of the encrypted and authenticated header
pub(crate) const BOXED_HEADER_SIZE: usize = 34;

/// Size of the plain text header.
pub(crate) const HEADER_SIZE: usize = 18;

/// The plain-text packet that indicates the end of the packet stream.
pub(crate) const GOODBYE_PACKET: [u8; HEADER_SIZE] = [0u8; HEADER_SIZE];

/// Maximum size of the payload of a packet
pub(crate) const MAX_PACKET_SIZE_BYTES: u16 = 4 * 1024;

/// Parameters for encrypting or decrypting a sequence of packets
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    key: sodiumoxide::crypto::secretbox::Key,
    nonce: sodiumoxide::crypto::secretbox::Nonce,
}

impl Params {
    pub fn new(key: crypto::secretbox::Key, nonce: crypto::secretbox::Nonce) -> Self {
        Self { key, nonce }
    }
}

#[cfg(test)]
impl Params {
    pub fn arbitrary() -> Params {
        Params {
            key: crypto::secretbox::gen_key(),
            nonce: crypto::secretbox::gen_nonce(),
        }
    }
}

impl Params {
    /// Encrypt a payload and return the encrypted packet.
    ///
    /// Panics if `payload` size exceeds [MAX_PACKET_SIZE_BYTES].
    pub(crate) fn encrypt(&mut self, mut dst: impl bytes::BufMut, data: &[u8]) {
        for payload in data.chunks(MAX_PACKET_SIZE_BYTES as usize) {
            self.encrypt_one(&mut dst, payload);
        }
    }

    fn encrypt_one(&mut self, mut dst: impl bytes::BufMut, payload: &[u8]) {
        assert!(payload.len() <= MAX_PACKET_SIZE_BYTES as usize);
        let body_len_bytes = (payload.len() as u16).to_be_bytes();

        let header_nonce = self.nonce;
        let body_nonce = nonce_increment_be(&header_nonce);

        let mut encrypted_body = Vec::from(payload);
        let body_tag =
            crypto::secretbox::seal_detached(encrypted_body.as_mut_slice(), &body_nonce, &self.key);

        let header = [body_len_bytes.as_ref(), body_tag.as_ref()].concat();
        let boxed_header = crypto::secretbox::seal(&header, &header_nonce, &self.key);

        self.nonce = nonce_increment_be(&body_nonce);

        dst.put(boxed_header.as_ref());
        dst.put(encrypted_body.as_ref());
    }

    pub(crate) fn goodbye(&mut self) -> Vec<u8> {
        let header_nonce = self.nonce;
        crypto::secretbox::seal(&GOODBYE_PACKET, &header_nonce, &self.key)
    }

    /// Decrypt a packet header. If successful, returns the length of the packet body and the
    /// authentication tag.
    ///
    /// Errors if the header cannot be decrypted or authenticated.
    pub(crate) fn decrypt_header(
        &mut self,
        boxed_header: &[u8; BOXED_HEADER_SIZE],
    ) -> Result<Option<(u16, crypto::secretbox::Tag)>, ()> {
        let header_nonce = self.nonce;
        let header = crypto::secretbox::open(boxed_header, &header_nonce, &self.key)?;
        if header[..] == GOODBYE_PACKET[..] {
            return Ok(None);
        }

        let body_len = u16::from_be_bytes(header[0..2].try_into().unwrap());
        let body_tag = crypto::secretbox::Tag::from_slice(&header[2..]).unwrap();
        self.nonce = nonce_increment_be(&header_nonce);
        Ok(Some((body_len, body_tag)))
    }

    /// Decrypt and authenticate a packet body.
    ///
    /// Errors if the header cannot be decrypted or authenticated.
    pub(crate) fn decrypt_body(
        &mut self,
        tag: &crypto::secretbox::Tag,
        cipher_body: &[u8],
    ) -> Result<Vec<u8>, ()> {
        let body_nonce = self.nonce;
        let mut body = Vec::from(cipher_body);
        crypto::secretbox::open_detached(&mut body, &tag, &body_nonce, &self.key)?;
        self.nonce = nonce_increment_be(&body_nonce);
        Ok(body)
    }
}

/// Calls [increment_be] on the nonce data.
fn nonce_increment_be(nonce: &crypto::secretbox::Nonce) -> crypto::secretbox::Nonce {
    let mut bytes = <[u8; crypto::secretbox::NONCEBYTES]>::try_from(nonce.as_ref()).unwrap();
    increment_be(&mut bytes);
    crypto::secretbox::Nonce::from_slice(&bytes).unwrap()
}

/// Interpret the buffer as a big endian unsigned integer and increment it by one. Overflows when
/// all bits are 1.
fn increment_be(bytes: &mut [u8]) {
    for byte in bytes.iter_mut().rev() {
        if *byte == u8::MAX {
            *byte = 0
        } else {
            *byte += 1;
            break;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn increment_be_u64() {
        fn test(value: u64) {
            let mut bytes = value.to_be_bytes();
            increment_be(&mut bytes);
            assert_eq!(value + 1, u64::from_be_bytes(bytes));
        }

        test(0);
        test(1);
        test(u64::MAX - 1);

        let mut max_bytes = u64::MAX.to_be_bytes();
        increment_be(&mut max_bytes);
        assert_eq!(0, u64::from_be_bytes(max_bytes))
    }

    #[test_strategy::proptest]
    fn box_crypt_roundtrip(payloads: Vec<Vec<u8>>) {
        let _ = sodiumoxide::init();
        let mut decrypt = Params::arbitrary();
        let mut encrypt = decrypt.clone();

        for payload in payloads {
            if payload.is_empty() {
                continue;
            }
            let mut cipher_text = bytes::BytesMut::new();
            encrypt.encrypt(&mut cipher_text, &payload);

            let mut boxed_header = [0u8; BOXED_HEADER_SIZE];
            boxed_header.copy_from_slice(&cipher_text[0..BOXED_HEADER_SIZE]);
            let (body_len, body_tag) = decrypt.decrypt_header(&boxed_header).unwrap().unwrap();

            let cipher_body = &cipher_text[BOXED_HEADER_SIZE..];
            prop_assert_eq!(body_len as usize, cipher_body.len());

            let msg_out = decrypt.decrypt_body(&body_tag, cipher_body).unwrap();
            prop_assert_eq!(payload, msg_out);
        }

        let goodbye = encrypt.goodbye();
        let mut boxed_header = [0u8; BOXED_HEADER_SIZE];
        boxed_header.copy_from_slice(&goodbye[0..BOXED_HEADER_SIZE]);
        let result = decrypt.decrypt_header(&boxed_header).unwrap();
        prop_assert!(result.is_none());
    }
}
