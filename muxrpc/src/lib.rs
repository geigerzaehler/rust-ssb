//! Application agnostic, bidirectional RPC protocol with stream support used by [Scuttlebutt][sb].
//!
//! The protocol is defined in the [Scuttlebutt Protocol Guide][ssb-prot]. The
//! de facto reference implementation is [_ssbc/muxrpc_][ssbc-muxrpc].
//!
//! [ssb-prot]: https://ssbc.github.io/scuttlebutt-protocol-guide/#rpc-protocol
//! [ssbc-muxrpc]: https://github.com/ssbc/muxrpc
//! [sb]: https://scuttlebutt.nz
mod client;
mod endpoint;
mod error;
mod header;
mod packet;
mod packet_stream;
mod server;
mod stream_message;
mod stream_request;
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

#[doc(inline)]
pub use client::{AsyncRequestError, AsyncResponse, Client};

#[doc(inline)]
pub use endpoint::Endpoint;

mod service;
#[doc(inline)]
pub use service::{Body, Error, Service, SinkError, StreamMessage};
