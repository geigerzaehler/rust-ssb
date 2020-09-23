//! Application agnostic RPC protocol
//!
//! The protocol is defined in the [Scuttlebutt Protocol Guide][ssb-prot]. The
//! de facto reference implementation is [_ssbc/muxrpc_][ssbc-muxrpc].
//!
//! [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
//! [ssbc-muxrpc]: https://github.com/ssbc/muxrpc
mod client;
mod endpoint;
mod header;
mod packet;
mod packet_stream;
mod server;
mod stream_request;
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

#[doc(inline)]
pub use client::{AsyncRequestError, AsyncResponse, Client};

#[doc(inline)]
pub use packet::Body;

#[doc(inline)]
pub use endpoint::Endpoint;

#[doc(inline)]
pub use server::{Service, SinkError};

mod stream_item;
#[doc(inline)]
pub use stream_item::StreamItem;

mod error;
#[doc(inline)]
pub use error::Error;
