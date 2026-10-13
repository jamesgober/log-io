//! Structured logging pipeline for Rust.
//!
//! `log-io` moves structured log records through three stages: a
//! [`Filter`] decides whether a record is wanted, a
//! [`Format`](format::Format) serializes it (JSON, logfmt, or a
//! human-readable line), and one or more [`Sink`]s deliver the bytes.
//! The steady-state hot path performs no heap allocation, the data
//! model and formatters work under `no_std`, and the default build has
//! zero dependencies. Optional bridges plug the pipeline into the `log`
//! and `tracing` ecosystems.
//!
//! # Quick start
//!
#![cfg_attr(all(feature = "std", feature = "json"), doc = "```")]
#![cfg_attr(not(all(feature = "std", feature = "json")), doc = "```ignore")]
//! use log_io::{Level, Logger};
//!
//! let logger = Logger::builder()
//!     .level(Level::Info)
//!     .env_filter("LOG_IO") // e.g. LOG_IO="info,my_app::db=debug"
//!     .stdout_json()
//!     .build();
//! let _guard = log_io::init(logger)?; // flushes on drop
//!
//! let port = 8080_u16;
//! log_io::info!("listening on port {port}", tls = true);
//! log_io::warn!(target: "my_app::db", "slow query", ms = 412_u64, table = "users");
//! # Ok::<(), log_io::SetGlobalError>(())
//! ```
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `std` | yes | [`Logger`], sinks, [`context`], timestamps, the global logger |
//! | `json` | yes | [`format::JsonFormat`] |
//! | `logfmt` | yes | [`format::LogfmtFormat`] |
//! | `human` | yes | [`format::HumanFormat`] (works under `no_std`) |
//! | `log` | no | `compat::LogBridge`, a `log::Log` implementation (adds the `log` crate) |
//! | `tracing` | no | `compat::TracingLayer`, a `tracing-subscriber` layer (adds `tracing-core` and `tracing-subscriber`) |
//!
//! Without `std` the crate is `no_std` (no `alloc` needed): [`Record`],
//! [`Field`], [`Value`], [`Level`] and the three formatters, which write
//! to any [`core::fmt::Write`].
//!
//! # Threading and async
//!
//! [`Logger`] and every sink are `Send + Sync`. The built-in synchronous
//! sinks write on the calling thread under a mutex, so inside an async
//! runtime they block the executor thread for the duration of the
//! write; [`sink::NonBlocking`] moves the write to a background thread.
//! Context is thread-local; [`context::ContextSnapshot`] carries it into
//! tasks and other threads.
//!
//! # Durability
//!
//! Records at or above the flush level (default [`Level::Error`]) are
//! flushed to every sink before the logging call returns. Everything
//! else may sit in a buffer until a flush, a [`FlushGuard`] drop, or the
//! last [`Logger`] clone being dropped. See [`sink::FileSink`] for the
//! exit paths that lose buffered data and how to avoid them.
//!
//! # Allocation
//!
//! Emitting a record through the built-in synchronous sinks does not
//! allocate in steady state, including timestamps, thread-local
//! context, default fields, and `format!`-style messages, as long as at
//! most 16 ambient (default + context) fields are attached. Creating a
//! context guard, capturing a context snapshot, and queueing a record on
//! [`sink::NonBlocking`] may allocate. `tests/alloc.rs` enforces this
//! with a counting allocator.
//!
//! # Stability
//!
//! Semantic versioning. `Value`, `Error`, `Metadata`, `Record` and the
//! configuration enums are `#[non_exhaustive]`, so they can grow in
//! minor releases. See `docs/MIGRATION.md` for upgrading from 1.x.

#![doc(html_root_url = "https://docs.rs/log-io")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(warnings)]
#![deny(missing_docs)]
#![deny(unused_must_use)]
#![deny(unused_results)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::unreachable)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod level;
mod record;
mod value;

pub use crate::level::{Level, ParseLevelError};
pub use crate::record::{Field, Metadata, Record};
pub use crate::value::Value;

pub mod format;
pub use crate::format::Format;

#[cfg(feature = "std")]
mod builder;
#[cfg(feature = "std")]
mod error;
#[cfg(feature = "std")]
mod filter;
#[cfg(feature = "std")]
mod logger;
#[cfg(feature = "std")]
mod macros;
#[cfg(feature = "std")]
pub(crate) mod time;

#[cfg(feature = "std")]
pub mod context;
#[cfg(feature = "std")]
pub mod global;
#[cfg(feature = "std")]
pub mod sink;

#[cfg(any(feature = "log", feature = "tracing"))]
pub mod compat;

#[cfg(feature = "std")]
pub use crate::builder::{BuildError, LoggerBuilder};
#[cfg(feature = "std")]
pub use crate::error::{Error, Result};
#[cfg(feature = "std")]
pub use crate::filter::{Filter, FilterRule, ParseFilterError, ParseFilterErrorKind};
#[cfg(feature = "std")]
pub use crate::global::{FlushGuard, SetGlobalError, init, set as set_global};
#[cfg(feature = "std")]
pub use crate::logger::{ErrorHandler, Logger, REDACTED};
#[cfg(feature = "std")]
pub use crate::sink::Sink;

#[cfg(feature = "std")]
#[doc(hidden)]
pub use crate::macros::__private;
