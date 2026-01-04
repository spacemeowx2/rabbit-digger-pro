use chrono::Utc;
use serde::ser::{SerializeMap, Serializer as _};
use std::io;
use tracing::{Event, Subscriber};
use tracing_opentelemetry::OtelData;
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

            if let Some(mut span_ref) = ctx.lookup_current() {
                loop {
                    if let Some(OtelData { builder, .. }) = span_ref.extensions().get::<OtelData>()
                    {
                        if let Some(trace_id) = builder.trace_id {
                            serializer.serialize_entry("trace_id", &format!("{:x?}", trace_id))?;
                            break;
                        }
                    }
                    if let Some(parent) = span_ref.parent() {
                        span_ref = parent;
                    } else {
                        break;
                    }
                }
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

    #[test]
    fn test_write_adaptor_write_valid_utf8() {
        let mut buf = String::new();
        let mut adaptor = WriteAdaptor::new(&mut buf);

        let result = adaptor.write(b"hello world");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 11);
        assert_eq!(buf, "hello world");
    }

    #[test]
    fn test_write_adaptor_write_invalid_utf8() {
        let mut buf = String::new();
        let mut adaptor = WriteAdaptor::new(&mut buf);

        let invalid_utf8: &[u8] = &[0xFF, 0xFE];
        let result = adaptor.write(invalid_utf8);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_adaptor_flush() {
        let mut buf = String::new();
        let mut adaptor = WriteAdaptor::new(&mut buf);

        let result = adaptor.flush();
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_adaptor_new() {
        let mut buf = String::new();
        let adaptor = WriteAdaptor::new(&mut buf);

        let _ = adaptor;
    }
}
