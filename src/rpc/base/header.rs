#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Header {
    pub flags: HeaderFlags,
    pub body_type: BodyType,
    #[cfg_attr(test, proptest(strategy = "1u32..=u32::MAX"))]
    pub body_len: u32,
    pub request_number: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[cfg_attr(test, proptest(no_params))]
pub struct HeaderFlags {
    pub is_stream: bool,
    pub is_end_or_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[repr(u8)]
pub enum BodyType {
    Binary = 0,
    Utf8String = 1,
    Json = 2,
}

/// Error returned from [Header::parse].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeaderParseError {
    #[error("Invalid body type {value}")]
    InvalidBodyType { value: u8 },
}

impl BodyType {
    fn from_flags(value: u8) -> Result<Self, HeaderParseError> {
        const BODY_TYPE_MASK: u8 = 0b0000_0011;
        match value & BODY_TYPE_MASK {
            0 => Ok(BodyType::Binary),
            1 => Ok(BodyType::Utf8String),
            2 => Ok(BodyType::Json),
            value => Err(HeaderParseError::InvalidBodyType { value }),
        }
    }
}

const IS_STREAM_MASK: u8 = 0b1000;
const IS_END_OR_ERROR_MASK: u8 = 0b0100;

impl Header {
    pub const SIZE: usize = 9;

    pub fn parse(data: [u8; Self::SIZE]) -> Result<Option<Self>, HeaderParseError> {
        use bytes::Buf as _;

        if data == [0u8; Self::SIZE] {
            return Ok(None);
        }

        let mut bytes = bytes::Bytes::copy_from_slice(&data);

        let flags = bytes.get_u8();
        let is_stream = flags & IS_STREAM_MASK != 0;
        let is_end_or_error = flags & IS_END_OR_ERROR_MASK != 0;
        let body_type = BodyType::from_flags(flags)?;
        let body_len = bytes.get_u32();
        let request_number = bytes.get_i32();
        debug_assert!(!bytes.has_remaining());

        Ok(Some(Self {
            flags: HeaderFlags {
                is_stream,
                is_end_or_error,
            },
            body_type,
            body_len,
            request_number,
        }))
    }

    pub fn build(&self) -> [u8; Self::SIZE] {
        use bytes::BufMut as _;

        let mut header = [0u8; Self::SIZE];
        let cursor = &mut &mut header[..];
        let mut flags = self.body_type as u8;
        if self.flags.is_stream {
            flags |= IS_STREAM_MASK;
        }
        if self.flags.is_end_or_error {
            flags |= IS_END_OR_ERROR_MASK;
        }
        cursor.put_u8(flags);
        cursor.put_u32(self.body_len);
        cursor.put_i32(self.request_number);
        debug_assert!(!cursor.has_remaining_mut());
        header
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn header_parse_build(header: Header) {
        prop_assert_eq!(Header::parse(header.build()).unwrap().unwrap(), header);
    }

    #[proptest]
    fn header_build_parse(mut header_data: [u8; Header::SIZE]) {
        header_data[0] &= 0b0000_1111;
        let header = match Header::parse(header_data) {
            Ok(Some(header)) => header,
            _ => prop_reject!(),
        };
        prop_assert_eq!(header.build(), header_data);
    }

    #[proptest]
    fn header_invalid_type(mut header_data: [u8; Header::SIZE]) {
        header_data[0] |= 0b0000_0011;
        let result = Header::parse(header_data);
        prop_assert_eq!(result, Err(HeaderParseError::InvalidBodyType { value: 3 }));
    }

    #[test]
    fn end_header() {
        let header_data = [0u8; Header::SIZE];
        let opt_header = Header::parse(header_data).unwrap();
        assert_eq!(opt_header, None);
    }
}
