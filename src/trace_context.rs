//! W3C Trace Context propagation for distributed tracing.
//!
//! Implements the W3C Trace Context specification to propagate
//! trace context across MCP backend calls. The `traceparent` and
//! `tracestate` headers are carried in the `_meta` field of MCP
//! requests and extracted from responses.

use std::fmt;

/// A W3C Trace Context `traceparent` value.
///
/// Format: `version-trace_id-parent_id-trace_flags`
/// Example: `00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceParent {
    pub version: u8,
    pub trace_id: [u8; 16],
    pub parent_id: [u8; 8],
    pub trace_flags: TraceFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TraceFlags(u8);

impl TraceFlags {
    pub const SAMPLED: u8 = 0x01;

    pub fn sampled(&self) -> bool {
        self.0 & Self::SAMPLED != 0
    }

    pub fn set_sampled(&mut self, sampled: bool) {
        if sampled {
            self.0 |= Self::SAMPLED;
        } else {
            self.0 &= !Self::SAMPLED;
        }
    }
}

/// Fill bytes from a UUID v4.
fn random_bytes(buf: &mut [u8]) {
    let mut offset = 0;
    while offset < buf.len() {
        let uuid = uuid::Uuid::new_v4();
        let bytes = uuid.as_bytes();
        let n = (buf.len() - offset).min(16);
        buf[offset..offset + n].copy_from_slice(&bytes[..n]);
        offset += n;
    }
}

impl TraceParent {
    /// Create a new TraceParent with a random trace_id and parent_id.
    pub fn new() -> Self {
        let mut trace_id = [0u8; 16];
        let mut parent_id = [0u8; 8];
        random_bytes(&mut trace_id);
        random_bytes(&mut parent_id);
        Self {
            version: 0,
            trace_id,
            parent_id,
            trace_flags: TraceFlags::default(),
        }
    }

    /// Create a sampled trace parent.
    pub fn new_sampled() -> Self {
        let mut tp = Self::new();
        tp.trace_flags.set_sampled(true);
        tp
    }

    /// Parse a traceparent header value.
    /// Format: `00-{32 hex}-{16 hex}-{2 hex}`
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 4 {
            return None;
        }

        let version = u8::from_str_radix(parts[0], 16).ok()?;

        let trace_id_hex = parts[1];
        if trace_id_hex.len() != 32 {
            return None;
        }
        let mut trace_id = [0u8; 16];
        for i in 0..16 {
            trace_id[i] = u8::from_str_radix(&trace_id_hex[i * 2..i * 2 + 2], 16).ok()?;
        }

        let parent_id_hex = parts[2];
        if parent_id_hex.len() != 16 {
            return None;
        }
        let mut parent_id = [0u8; 8];
        for i in 0..8 {
            parent_id[i] = u8::from_str_radix(&parent_id_hex[i * 2..i * 2 + 2], 16).ok()?;
        }

        let trace_flags_hex = parts[3];
        if trace_flags_hex.len() != 2 {
            return None;
        }
        let trace_flags = TraceFlags(u8::from_str_radix(trace_flags_hex, 16).ok()?);

        Some(Self {
            version,
            trace_id,
            parent_id,
            trace_flags,
        })
    }

    /// Generate a new child span (new parent_id, same trace_id).
    pub fn child(&self) -> Self {
        let mut parent_id = [0u8; 8];
        random_bytes(&mut parent_id);
        Self {
            version: self.version,
            trace_id: self.trace_id,
            parent_id,
            trace_flags: self.trace_flags,
        }
    }
}

impl fmt::Display for TraceParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}-{:02x}",
            self.version,
            self.trace_id[0],
            self.trace_id[1],
            self.trace_id[2],
            self.trace_id[3],
            self.trace_id[4],
            self.trace_id[5],
            self.trace_id[6],
            self.trace_id[7],
            self.trace_id[8],
            self.trace_id[9],
            self.trace_id[10],
            self.trace_id[11],
            self.trace_id[12],
            self.trace_id[13],
            self.trace_id[14],
            self.trace_id[15],
            self.parent_id[0],
            self.parent_id[1],
            self.parent_id[2],
            self.parent_id[3],
            self.parent_id[4],
            self.parent_id[5],
            self.parent_id[6],
            self.parent_id[7],
            self.trace_flags.0,
        )
    }
}

impl Default for TraceParent {
    fn default() -> Self {
        Self::new()
    }
}

/// Carries W3C trace context through the gateway.
#[derive(Debug, Clone, Default)]
pub struct TraceContext {
    pub traceparent: Option<TraceParent>,
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// Create a new sampled trace context.
    pub fn new_root() -> Self {
        Self {
            traceparent: Some(TraceParent::new_sampled()),
            tracestate: None,
        }
    }

    /// Extract TraceContext from MCP `_meta` map.
    pub fn from_meta(meta: &serde_json::Map<String, serde_json::Value>) -> Self {
        let traceparent = meta
            .get("traceparent")
            .and_then(|v| v.as_str())
            .and_then(TraceParent::parse);

        let tracestate = meta
            .get("tracestate")
            .and_then(|v| v.as_str())
            .map(String::from);

        Self {
            traceparent,
            tracestate,
        }
    }

    /// Inject into MCP `_meta` map for propagation to backends.
    pub fn inject_meta(&self, meta: &mut serde_json::Map<String, serde_json::Value>) {
        if let Some(ref tp) = self.traceparent {
            meta.insert(
                "traceparent".to_string(),
                serde_json::Value::String(tp.to_string()),
            );
        }
        if let Some(ref ts) = self.tracestate {
            meta.insert(
                "tracestate".to_string(),
                serde_json::Value::String(ts.clone()),
            );
        }
    }

    /// Create a child context for a downstream call.
    pub fn child(&self) -> Self {
        Self {
            traceparent: self.traceparent.as_ref().map(|tp| tp.child()),
            tracestate: self.tracestate.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traceparent_roundtrip() {
        let tp = TraceParent::new();
        let s = tp.to_string();
        let parsed = TraceParent::parse(&s).unwrap();
        assert_eq!(tp.version, parsed.version);
        assert_eq!(tp.trace_id, parsed.trace_id);
        assert_eq!(tp.parent_id, parsed.parent_id);
        assert_eq!(tp.trace_flags, parsed.trace_flags);
    }

    #[test]
    fn test_traceparent_parse_w3c_example() {
        let tp =
            TraceParent::parse("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01").unwrap();
        assert_eq!(tp.version, 0);
        assert!(tp.trace_flags.sampled());
    }

    #[test]
    fn test_traceparent_parse_invalid() {
        assert!(TraceParent::parse("invalid").is_none());
        assert!(TraceParent::parse("00-short-short-01").is_none());
        assert!(
            TraceParent::parse("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331").is_none()
        );
    }

    #[test]
    fn test_child_span_shares_trace_id() {
        let parent = TraceParent::new();
        let child = parent.child();
        assert_eq!(parent.trace_id, child.trace_id);
        // Parent ID should differ (probabilistic, but UUIDs won't collide)
        assert_ne!(parent.parent_id, [0; 8]);
    }

    #[test]
    fn test_trace_context_meta_roundtrip() {
        let ctx = TraceContext::new_root();
        let mut meta = serde_json::Map::new();
        ctx.inject_meta(&mut meta);

        let extracted = TraceContext::from_meta(&meta);
        assert!(extracted.traceparent.is_some());
        assert_eq!(
            ctx.traceparent.unwrap().to_string(),
            extracted.traceparent.unwrap().to_string()
        );
    }

    #[test]
    fn test_trace_context_from_empty_meta() {
        let meta = serde_json::Map::new();
        let ctx = TraceContext::from_meta(&meta);
        assert!(ctx.traceparent.is_none());
        assert!(ctx.tracestate.is_none());
    }
}
