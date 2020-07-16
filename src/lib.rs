//! An unfinished implementation of the [Scuttlebut protocol][protocol] in rust
//!
//! [protocol]: https://ssbc.github.io/scuttlebutt-protocol-guide

mod box_stream;
mod crypto;
pub mod handshake;
pub mod secret_file;
mod utils;

#[cfg(test)]
mod test_utils;
