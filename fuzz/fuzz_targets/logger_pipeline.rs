#![no_main]
//! Fuzz target: the full `Logger` pipeline with default fields,
//! thread-local context, redaction and key de-duplication.
//!
//! Invariants:
//! * never panics;
//! * every emitted line is valid JSON whose level and message are the
//!   record's own (no field can spoof them);
//! * redacted keys never leak their value;
//! * no key appears twice in a record (call-site duplicates included).

use std::io::Write;
use std::sync::{Arc, Mutex};

use std::collections::HashSet;
use std::fmt;

use arbitrary::Arbitrary;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
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

/// Deserializes a JSON object, failing if a key appears twice
/// (`serde_json::Value` would silently keep the last one).
struct UniqueKeys;

impl<'de> de::Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Keys;
        impl<'de> Visitor<'de> for Keys {
            type Value = UniqueKeys;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<UniqueKeys, A::Error> {
                let mut seen = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    let _: IgnoredAny = map.next_value()?;
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                }
                Ok(UniqueKeys)
            }
        }
        deserializer.deserialize_map(Keys)
    }
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
        let _unique: UniqueKeys = serde_json::from_str(line).expect("unique keys");
        assert_eq!(v["level"], "warn");
        assert_eq!(v["message"].as_str(), Some(message.as_str()));
        assert_eq!(v["password"], "[REDACTED]");
    }
    assert!(lines.next().is_none());
});
