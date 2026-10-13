//! Bridges from the `log` and `tracing` ecosystems into a [`Logger`].
//!
//! Each bridge sits behind its own non-default feature:
//!
//! * `log` - [`LogBridge`] implements `log::Log`, so records from any
//!   crate that uses the `log` macros flow through a log-io pipeline,
//!   including their structured key-values.
//! * `tracing` - [`TracingLayer`] implements
//!   `tracing_subscriber::Layer`, so `tracing` events are written by
//!   log-io sinks, with the fields of the enclosing spans attached as
//!   context.
//!
//! Both bridges apply the logger's filter, redaction, default fields,
//! thread-local context, and flush policy exactly as for native
//! records.
//!
//! [`Logger`]: crate::Logger

#[cfg(feature = "log")]
mod log_bridge;
#[cfg(feature = "log")]
pub use self::log_bridge::{LogBridge, to_level_filter};

#[cfg(feature = "tracing")]
mod tracing_layer;
#[cfg(feature = "tracing")]
pub use self::tracing_layer::TracingLayer;
