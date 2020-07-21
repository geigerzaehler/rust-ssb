use crate::crypto;
use std::convert::{TryFrom as _, TryInto as _};

pub const BOXED_HEADER_SIZE: usize = 34;
pub const HEADER_SIZE: usize = 18;
pub const GOODBYE_HEADER: [u8; HEADER_SIZE] = [0u8; HEADER_SIZE];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxCrypt {
    pub key: crypto::secretbox::Key,
    pub nonce: crypto::secretbox::Nonce,
}

#[cfg(test)]
impl BoxCrypt {
    pub fn arbitrary() -> BoxCrypt {
        BoxCrypt {
            key: crypto::secretbox::gen_key(),
            nonce: crypto::secretbox::gen_nonce(),
        }
    }
}

impl BoxCrypt {
    pub(super) fn encrypt(&mut self, body: Packet) -> Vec<u8> {
        let body_len_bytes = (body.len() as u16).to_be_bytes();

        let header_nonce = self.nonce;
        let body_nonce = nonce_increment_be(&header_nonce);

        let mut encrypted_body = body.0;
        let body_tag =
            crypto::secretbox::seal_detached(encrypted_body.as_mut_slice(), &body_nonce, &self.key);

        let header = [body_len_bytes.as_ref(), body_tag.as_ref()].concat();
        let boxed_header = crypto::secretbox::seal(&header, &header_nonce, &self.key);

        self.nonce = nonce_increment_be(&body_nonce);

        let mut package = boxed_header;
        package.extend_from_slice(&encrypted_body);
        package
    }

    pub(super) fn goodbye(&mut self) -> Vec<u8> {
        let header_nonce = self.nonce;
        crypto::secretbox::seal(&GOODBYE_HEADER, &header_nonce, &self.key)
    }

    pub(super) fn decrypt_head(
        &mut self,
        boxed_header: &[u8; BOXED_HEADER_SIZE],
    ) -> Result<Option<(u16, crypto::secretbox::Tag)>, ()> {
        let header_nonce = self.nonce;
        let header = crypto::secretbox::open(boxed_header, &header_nonce, &self.key)?;
        if header[..] == GOODBYE_HEADER[..] {
            return Ok(None);
        }

        let body_len = u16::from_be_bytes(header[0..2].try_into().unwrap());
        let body_tag = crypto::secretbox::Tag::from_slice(&header[2..]).unwrap();
        self.nonce = nonce_increment_be(&header_nonce);
        Ok(Some((body_len, body_tag)))
    }

    pub(super) fn decrypt_body(
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

pub struct Packet(Vec<u8>);

impl Packet {
    const MAX_SIZE_BYTES: u16 = 4 * 1024;

    pub(super) fn build(data: &[u8]) -> Vec<Packet> {
        data.chunks(Self::MAX_SIZE_BYTES as usize)
            .filter_map(|data| {
                if data.is_empty() {
                    None
                } else {
                    Some(Packet(Vec::from(data)))
                }
            })
            .collect()
    }

    fn len(&self) -> u16 {
        self.0.len() as u16
    }
}

impl std::ops::Deref for Packet {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
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
    use crate::test_utils::*;

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

    #[proptest]
    fn box_crypt_roundtrip(messages: Vec<Vec<u8>>) {
        let _ = sodiumoxide::init();
        let mut decrypt = BoxCrypt::arbitrary();
        let mut encrypt = decrypt.clone();

        for message in messages {
            if message.is_empty() {
                continue;
            }
            let packet = Packet::build(&message).pop().unwrap();
            let message = Vec::from(&*packet);
            let cipher_text = encrypt.encrypt(packet);

            let mut boxed_header = [0u8; BOXED_HEADER_SIZE];
            boxed_header.copy_from_slice(&cipher_text[0..BOXED_HEADER_SIZE]);
            let (body_len, body_tag) = decrypt.decrypt_head(&boxed_header).unwrap().unwrap();

            let cipher_body = &cipher_text[BOXED_HEADER_SIZE..];
            prop_assert_eq!(body_len as usize, cipher_body.len());

            let msg_out = decrypt.decrypt_body(&body_tag, cipher_body).unwrap();
            prop_assert_eq!(message, msg_out);
        }

        let goodbye = encrypt.goodbye();
        let mut boxed_header = [0u8; BOXED_HEADER_SIZE];
        boxed_header.copy_from_slice(&goodbye[0..BOXED_HEADER_SIZE]);
        let result = decrypt.decrypt_head(&boxed_header).unwrap();
        prop_assert!(result.is_none());
    }
}
