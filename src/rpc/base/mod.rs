//! Application agnostic RPC protocol
//!
//! The protocol is defined in the [Scuttlebutt Protocol Guide][ssb-prot]. The
//! de facto reference implementation is [_ssbc/muxrpc_][ssbc-muxrpc].
//!
//! [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
//! [ssbc-muxrpc]: https://github.com/ssbc/muxrpc
mod client;
mod header;
mod packet;
mod packet_stream;

#[doc(inline)]
pub use client::{AsyncRequestError, AsyncResponse, Client};
