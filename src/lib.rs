//! An unfinished implementation of the [Scuttlebut protocol][protocol] in rust
//!
//! [protocol]: https://ssbc.github.io/scuttlebutt-protocol-guide

#![warn(missing_debug_implementations, clippy::all)]

#[macro_use]
extern crate prettytable;

#[cfg(test)]
#[macro_use]
mod test_utils;

mod box_stream;
pub mod crypto;
pub mod discovery;
pub mod handshake;
pub mod multi_address;
pub mod rpc;
pub mod secret_file;
pub mod ssbc;
pub mod utils;

pub const SCUTTLEBUTT_NETWORK_IDENTIFIER: [u8; 32] = [
    0xd4, 0xa1, 0xcb, 0x88, 0xa6, 0x6f, 0x02, 0xf8, 0xdb, 0x63, 0x5c, 0xe2, 0x64, 0x41, 0xcc, 0x5d,
    0xac, 0x1b, 0x08, 0x42, 0x0c, 0xea, 0xac, 0x23, 0x08, 0x39, 0xb7, 0x55, 0x84, 0x5a, 0x9f, 0xfb,
];
