//! Owned trace, notification and IPC vocabulary; no desktop runtime dependencies.
#![deny(unsafe_code)]
pub mod error;
pub mod notify;
pub mod obs;
pub mod priority;
pub mod trace;
pub use error::ErrorCode;
pub use priority::Priority;
pub use trace::{SpanId, TraceId};
