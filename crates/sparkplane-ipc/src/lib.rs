//! Bounded, credential-authorized local IPC for the Sparkplane executor.
#![deny(unsafe_code)]
pub mod cancel;
pub mod client;
pub mod codec;
pub mod envelope;
pub mod server;
pub mod stream;
pub use cancel::{CancelGuard, CancelRegistry, Dispatched, dispatch_with_cancel};
pub use client::{CallOpts, Client};
pub use codec::{RequestCodec, ResponseCodec};
pub use envelope::{
    BlobKind, BlobRef, ErrorBody, ParseRequestError, Request, Response, SCHEMA_VERSION, SpanId,
    TraceId, parse_request_strict,
};
pub use server::{Handler, PeerAuthorizer, PeerCredentials, SameEuidAuthorizer, Server};
pub use stream::{Event, EventCodec, KIND_CLOSED};
