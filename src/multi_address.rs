//! MultiAddress format for representing layered network addresess.
//!
//! A [MultiAddress] is a list of addresses for a connecting to a peer on the
//! network. Each [Address] is a stack of [Protocols]. A connection is
//! established using the first protocol and then in the stack. Subsequent
//! protocols use the previous protocol as the underyling transport
//!
//! Each protocol consists of a [`name`][Protocol::name] that identifies the
//! protocol and a list of [`data`][Protocol::data] segments that specify the
//! parameters of the protocol
//!
//! ```rust
//! # use ssb::multi_address::MultiAddress;
//! use std::str::FromStr;
//! let multi_address_string = "net:172.17.0.2:8008~shs:UkXKGs5VCAcDQTvfOw9aQ903k0oERoSCy/3H2minTWk=";
//! let multi_address = MultiAddress::from_str(multi_address_string).unwrap();
//! let address = &multi_address.addresses[0];
//!
//! let net = &address.protocols[0];
//! assert_eq!(net.name, "net");
//! assert_eq!(net.data[0], "172.17.0.2");
//! assert_eq!(net.data[1], "8008");
//!
//! let shs = &address.protocols[1];
//! assert_eq!(shs.name, "shs");
//! assert_eq!(shs.data[0], "UkXKGs5VCAcDQTvfOw9aQ903k0oERoSCy/3H2minTWk=");
//!
//! assert_eq!(multi_address.to_string(), multi_address_string);
//! ```

#[derive(Debug, PartialEq, Eq)]
pub struct MultiAddress {
    pub addresses: Vec<Address>,
}

impl std::str::FromStr for MultiAddress {
    type Err = peg::error::ParseError<peg::str::LineCol>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parser::multi_address(s)
    }
}

impl std::fmt::Display for MultiAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.addresses
                .iter()
                .map(Address::to_string)
                .collect::<Vec<String>>()
                .join(";")
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Address {
    pub protocols: Vec<Protocol>,
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.protocols
                .iter()
                .map(Protocol::to_string)
                .collect::<Vec<String>>()
                .join("~")
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Protocol {
    pub name: String,
    pub data: Vec<String>,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.data.iter().map(|item| {
            item.chars().fold(String::new(), |mut s, c| {
                match c {
                    '!' | ':' | ';' | '~' => s.push('!'),
                    _ => (),
                }
                s.push(c);
                s
            })
        });
        let result = std::iter::once(self.name.clone())
            .chain(data)
            .collect::<Vec<String>>()
            .join(":");

        write!(f, "{}", result)
    }
}

peg::parser! {
    grammar parser() for str {
        rule name() -> String
            = n:$([ 'a'..='z'] ['a'..='z' | '0'..='9' | '-' ]+) { String::from(n) }

        rule escaped_char() -> String
            = "!" x:$[ '!' | ':' | ';' | '~' ] { String::from(x) }

        rule data_char() -> String
            = x:$[ '"'..='9'  | '<'..='}' ] { String::from(x) }

        rule data() -> String
            = cs:( data_char() / escaped_char())* { cs.concat() }

        rule data_part() -> String
            = ":" data:data() { data }

        rule protocol() -> Protocol
            = name:name() data:data_part()* { Protocol { name, data } }

        rule address() -> Address
            = protocols:protocol() ** "~" { Address { protocols } }

        pub rule multi_address() -> MultiAddress
            = addresses:address() ** ";" { MultiAddress { addresses } }
  }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::*;

    #[proptest]
    fn to_string_then_parse(#[strategy(multi_address_strategy())] multi_address: MultiAddress) {
        let s = multi_address.to_string();
        let result = parser::multi_address(&s)?;

        prop_assert_eq!(result, multi_address);
    }

    fn multi_address_strategy() -> impl Strategy<Value = MultiAddress> {
        use proptest::strategy::Strategy as _;
        let data = proptest::collection::vec("[!-~]+", 0..4);
        let name = "[a-z][a-z0-9-]";
        let protocol = (data, name).prop_map(|(data, name)| Protocol { name, data });
        let address =
            proptest::collection::vec(protocol, 1..4).prop_map(|protocols| Address { protocols });
        proptest::collection::vec(address, 1..4).prop_map(|addresses| MultiAddress { addresses })
    }
}
