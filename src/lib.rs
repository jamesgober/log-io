//! Structured logging pipeline for Rust. Zero-allocation fast path, JSON / logfmt / human-readable outputs, context propagation (request-id, trace-id), per-module filtering, async-safe sinks. An IO pipeline for log records, not a wrapper around log+tracing.
//!
//! # Status
//!
//! This crate is in early scaffolding. The public API is not yet
//! defined. See [the repository](https://github.com/jamesgober/log-io)
//! and `.dev/ROADMAP.md` for the milestone plan.

#![doc(html_root_url = "https://docs.rs/log-io")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
