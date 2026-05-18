//! Structured logging pipeline for Rust.
//!
//! `log-io` is an IO pipeline for structured log records. It is not a
//! wrapper around `log` or `tracing`. A record flows through a small
//! number of well-defined stages: filter, format, sink. Each stage is
//! independently composable.
//!
//! # Quick start
//!
//! ```no_run
//! use log_io::{Field, Level, Logger, Value};
//!
//! let logger = Logger::builder()
//!     .level(Level::Info)
//!     .stdout_json()
//!     .build();
//!
//! logger.log(
//!     Level::Info,
//!     "server started",
//!     &[Field::new("port", Value::U64(8080))],
//! );
//! ```
//!
//! # Features
//!
//! * `std` (default): enables IO sinks, file output, async-safe locks,
//!   thread-local context propagation, and timestamps. When disabled,
//!   the crate compiles in `no_std` mode with only the data model and
//!   [`core::fmt::Write`]-based formatters available.
//! * `json` (default): JSON output format.
//! * `logfmt` (default): `key=value` logfmt output format.
//! * `human` (default, requires `std`): human-readable timestamped
//!   format. Requires `std` because timestamps depend on `SystemTime`.
//!
//! # Design notes
//!
//! The fast path is zero-allocation. [`Record`], [`Field`], and
//! [`Value`] all borrow their data. Formatters serialize directly into
//! a writer; no intermediate `String` is produced unless the caller
//! supplies one.
//!
//! # Stability
//!
//! `0.x.y` releases are not API-stable. Stability begins at `1.0.0`.

#![doc(html_root_url = "https://docs.rs/log-io")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod level;
mod record;
mod value;

pub use crate::level::{Level, ParseLevelError};
pub use crate::record::{Field, Metadata, Record};
pub use crate::value::Value;

#[cfg(feature = "std")]
mod filter;
#[cfg(feature = "std")]
pub use crate::filter::{Filter, FilterRule, ParseFilterError};

pub mod format;
pub use crate::format::Format;

#[cfg(feature = "std")]
mod error;
#[cfg(feature = "std")]
pub use crate::error::{Error, Result};

#[cfg(feature = "std")]
pub mod sink;
#[cfg(feature = "std")]
pub use crate::sink::Sink;

#[cfg(feature = "std")]
mod logger;
#[cfg(feature = "std")]
pub use crate::logger::{Logger, LoggerBuilder};

#[cfg(feature = "std")]
pub mod context;

#[cfg(feature = "std")]
pub(crate) mod time;

#[cfg(feature = "std")]
mod macros;
