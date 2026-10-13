#![no_main]
//! Fuzz target: the full `Logger` pipeline with default fields,
//! thread-local context, redaction and key de-duplication.
//!
//! Invariants:
//! * never panics;
//! * every emitted line is valid JSON whose level and message are the
//!   record's own (no field can spoof them);
//! * redacted keys never leak their value.

use std::io::Write;
use std::sync::{Arc, Mutex};

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use log_io::format::JsonFormat;
use log_io::{Field, Level, Logger, context};
use log_io_fuzz::FuzzValue;

#[derive(Arbitrary, Debug)]
struct Input {
    defaults: Vec<(String, String)>,
    context: Vec<(String, String)>,
    calls: Vec<(String, Vec<(String, FuzzValue)>)>,
    secret: String,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|input: Input| {
    let out = Capture::default();
    let mut builder = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .redact_key("password");
    for (k, v) in &input.defaults {
        builder = builder.with_default_field(k.clone(), v.as_str());
    }
    let logger = builder.writer(out.clone(), JsonFormat::new()).build();

    context::clear();
    let guards: Vec<_> = input
        .context
        .iter()
        .map(|(k, v)| context::with_field(k, v.as_str()))
        .collect();
    for (message, fields) in &input.calls {
        let mut fields: Vec<Field<'_>> = fields
            .iter()
            .map(|(k, v)| Field::new(k, v.as_value()))
            .collect();
        fields.push(Field::new("password", input.secret.as_str()));
        logger.log(Level::Warn, message, &fields);
    }
    drop(guards);

    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let mut lines = text.lines();
    for (message, _) in &input.calls {
        let line = lines.next().expect("one line per call");
        let v: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        assert_eq!(v["level"], "warn");
        assert_eq!(v["message"].as_str(), Some(message.as_str()));
        assert_eq!(v["password"], "[REDACTED]");
    }
    assert!(lines.next().is_none());
});
