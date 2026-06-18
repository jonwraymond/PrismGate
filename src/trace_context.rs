/// Minimal W3C Trace Context implementation for MCP request tracing.
///
/// Generates traceparent/tracestate headers per W3C Trace Context Level 2
/// and injects them into rmcp Meta payloads via `_meta` field.
use rand::Rng;
use rmcp::model::Meta;
use serde_json::Value;

/// A W3C Trace Context for propagating distributed tracing information.
#[derive(Debug, Clone)]
pub struct TraceContext {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    trace_flags: u8,
    tracestate: Option<String>,
}

impl TraceContext {
    /// Create a new root trace context with randomly generated IDs.
    /// The `trace_flags` byte has bit 0 set to indicate sampling.
    pub fn new_root() -> Self {
        let mut rng = rand::rng();
        let mut trace_id = [0u8; 16];
        let mut span_id = [0u8; 8];
        rng.fill(&mut trace_id);
        rng.fill(&mut span_id);

        Self {
            trace_id,
            span_id,
            trace_flags: 0x01, // sampled
            tracestate: None,
        }
    }

    /// Format the `traceparent` header value per W3C spec.
    /// Format: `00-{trace_id}-{span_id}-{trace_flags:02x}`
    fn traceparent(&self) -> String {
        format!(
            "00-{}-{}-{:02x}",
            hex_encode(&self.trace_id),
            hex_encode(&self.span_id),
            self.trace_flags
        )
    }

    /// Inject W3C Trace Context fields into the rmcp Meta object.
    /// Inserts `traceparent` and `tracestate` (if present) into `_meta`.
    pub fn inject_meta(&self, meta: &mut Meta) {
        meta.insert("traceparent".to_string(), Value::String(self.traceparent()));
        if let Some(ref ts) = self.tracestate {
            meta.insert("tracestate".to_string(), Value::String(ts.clone()));
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            write!(&mut s, "{b:02x}").unwrap();
            s
        })
}
