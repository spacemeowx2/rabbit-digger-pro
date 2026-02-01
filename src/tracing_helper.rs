use chrono::Utc;
use serde::ser::{SerializeMap, Serializer as _};
use std::io;
use tracing::{Event, Subscriber};
use tracing_serde::fields::AsMap;
use tracing_serde::AsSerde;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

// From https://docs.rs/tracing-subscriber/latest/src/tracing_subscriber/fmt/format/json.rs.html
struct SerializableContext<'a, 'b, Span, N>(
    &'b tracing_subscriber::fmt::FmtContext<'a, Span, N>,
    std::marker::PhantomData<N>,
)
where
    Span: Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static;

impl<'a, 'b, Span, N> serde::ser::Serialize for SerializableContext<'a, 'b, Span, N>
where
    Span: Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn serialize<Ser>(&self, serializer_o: Ser) -> Result<Ser::Ok, Ser::Error>
    where
        Ser: serde::ser::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut serializer = serializer_o.serialize_seq(None)?;

        if let Some(leaf_span) = self.0.lookup_current() {
            for span in leaf_span.scope().from_root() {
                serializer.serialize_element(&SerializableSpan(&span, self.1))?;
            }
        }

        serializer.end()
    }
}

struct SerializableSpan<'a, 'b, Span, N>(
    &'b tracing_subscriber::registry::SpanRef<'a, Span>,
    std::marker::PhantomData<N>,
)
where
    Span: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static;

impl<'a, 'b, Span, N> serde::ser::Serialize for SerializableSpan<'a, 'b, Span, N>
where
    Span: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn serialize<Ser>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error>
    where
        Ser: serde::ser::Serializer,
    {
        let mut serializer = serializer.serialize_map(None)?;

        let ext = self.0.extensions();
        let data = ext
            .get::<tracing_subscriber::fmt::FormattedFields<N>>()
            .expect("Unable to find FormattedFields in extensions; this is a bug");

        match serde_json::from_str::<serde_json::Value>(data) {
            Ok(serde_json::Value::Object(fields)) => {
                for field in fields {
                    serializer.serialize_entry(&field.0, &field.1)?;
                }
            }
            Ok(_) if cfg!(debug_assertions) => panic!(
                "span '{}' had malformed fields! this is a bug.\n  error: invalid JSON object\n  fields: {:?}",
                self.0.metadata().name(),
                data
            ),
            Ok(value) => {
                serializer.serialize_entry("field", &value)?;
                serializer.serialize_entry("field_error", "field was no a valid object")?
            }
            Err(e) if cfg!(debug_assertions) => panic!(
                "span '{}' had malformed fields! this is a bug.\n  error: {}\n  fields: {:?}",
                self.0.metadata().name(),
                e,
                data
            ),
            // If we *aren't* in debug mode, it's probably best not
            // crash the program, but let's at least make sure it's clear
            // that the fields are not supposed to be missing.
            Err(e) => serializer.serialize_entry("field_error", &format!("{}", e))?,
        };
        serializer.serialize_entry("name", self.0.metadata().name())?;
        serializer.end()
    }
}

// From https://github.com/tokio-rs/tracing/issues/1531
pub struct TraceIdFormat;

impl<S, N> FormatEvent<S, N> for TraceIdFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let meta = event.metadata();

        let mut visit = || {
            let format_field_marker: std::marker::PhantomData<N> = std::marker::PhantomData;
            let current_span = event
                .parent()
                .and_then(|id| ctx.span(id))
                .or_else(|| ctx.lookup_current());

            let mut serializer = serde_json::Serializer::new(WriteAdaptor::new(&mut writer));
            let mut serializer = serializer.serialize_map(None)?;
            serializer.serialize_entry("timestamp", &Utc::now().to_rfc3339())?;
            serializer.serialize_entry("level", &meta.level().as_serde())?;
            serializer.serialize_entry("fields", &event.field_map())?;
            serializer.serialize_entry("target", meta.target())?;
            if let Some(ref span) = current_span {
                serializer
                    .serialize_entry("span", &SerializableSpan(span, format_field_marker))
                    .unwrap_or(());
            }

            if current_span.is_some() {
                serializer
                    .serialize_entry("spans", &SerializableContext(&ctx, format_field_marker))?;
            }

            serializer.end()
        };

        visit().map_err(|_| std::fmt::Error)?;
        writeln!(writer)
    }
}

pub struct WriteAdaptor<'a> {
    fmt_write: &'a mut dyn std::fmt::Write,
}

impl<'a> WriteAdaptor<'a> {
    pub fn new(fmt_write: &'a mut dyn std::fmt::Write) -> Self {
        Self { fmt_write }
    }
}

impl<'a> io::Write for WriteAdaptor<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s =
            std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.fmt_write
            .write_str(s)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(s.as_bytes().len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn test_write_adaptor_valid_utf8() {
        let mut out = String::new();
        let mut w = WriteAdaptor::new(&mut out);
        w.write_all(b"abc").unwrap();
        assert_eq!(out, "abc");
    }

    #[test]
    fn test_write_adaptor_invalid_utf8_is_error() {
        let mut out = String::new();
        let mut w = WriteAdaptor::new(&mut out);
        let err = w.write(&[0xff]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    struct SharedBufWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedBufWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut locked = self.0.lock().unwrap();
            locked.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBufWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SharedBufWriter(self.0.clone())
        }
    }

    #[test]
    fn test_trace_id_format_emits_json_with_span_context() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let make = SharedBuf(buf.clone());

        let layer = tracing_subscriber::fmt::layer()
            .json()
            .event_format(TraceIdFormat)
            .with_writer(make);

        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("test_span", answer = 42);
            let _g = span.enter();
            tracing::info!(hello = true, "msg");
        });

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let line = out.lines().find(|l| !l.trim().is_empty()).unwrap();
        let v: serde_json::Value = serde_json::from_str(line).unwrap();

        assert!(v.get("timestamp").is_some());
        assert!(v.get("level").is_some());
        assert!(v.get("fields").is_some());
        assert!(v.get("target").is_some());
        assert!(v.get("span").is_some());
        assert!(v.get("spans").is_some());
    }

    #[test]
    fn test_trace_id_format_without_span_context() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let make = SharedBuf(buf.clone());

        let layer = tracing_subscriber::fmt::layer()
            .json()
            .event_format(TraceIdFormat)
            .with_writer(make);

        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(hello = true, "msg");
        });

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let line = out.lines().find(|l| !l.trim().is_empty()).unwrap();
        let v: serde_json::Value = serde_json::from_str(line).unwrap();

        assert!(v.get("timestamp").is_some());
        assert!(v.get("fields").is_some());
        assert!(v.get("span").is_none());
        assert!(v.get("spans").is_none());
    }
}
