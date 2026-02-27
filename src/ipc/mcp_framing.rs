//! Lightweight MCP handshake framer.
//!
//! Reads newline-delimited JSON-RPC messages (rmcp's `transport-io` format)
//! and classifies them for handshake interception. Only used for the 3-message
//! MCP handshake — zero performance concern.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Classification of an MCP JSON-RPC message for handshake purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpMessage {
    /// `{"method": "initialize", ...}` — client → server
    InitializeRequest,
    /// Response with `"result"` or `"error"`, no `"method"` — server → client
    InitializeResponse,
    /// `{"method": "notifications/initialized", ...}` — client → server
    InitializedNotification,
    /// Anything else (shouldn't appear during handshake)
    Other,
}

/// Read bytes until `\n` (inclusive) from the given reader.
///
/// Returns the complete line including the trailing newline.
/// Returns an empty vec on clean EOF (no partial data).
/// Returns `UnexpectedEof` if the stream closes mid-line.
pub async fn read_line<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(4096);
    loop {
        let byte = match reader.read_u8().await {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                if buf.is_empty() {
                    // Clean EOF before any data — not an error, just "no more messages"
                    return Ok(Vec::new());
                }
                // Partial line — the peer closed mid-message
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream closed mid-line during handshake",
                ));
            }
            Err(e) => return Err(e),
        };
        buf.push(byte);
        if byte == b'\n' {
            return Ok(buf);
        }
    }
}

/// Classify a raw JSON-RPC line for handshake purposes.
///
/// Uses `serde_json::Value` for safe classification — no byte-scan false positives.
pub fn classify(line: &[u8]) -> McpMessage {
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(line) else {
        return McpMessage::Other;
    };

    let obj = match val.as_object() {
        Some(o) => o,
        None => return McpMessage::Other,
    };

    if let Some(method) = obj.get("method").and_then(|v| v.as_str()) {
        match method {
            "initialize" => return McpMessage::InitializeRequest,
            "notifications/initialized" => return McpMessage::InitializedNotification,
            _ => return McpMessage::Other,
        }
    }

    // No "method" field — check for response (has "result" or "error")
    if obj.contains_key("result") || obj.contains_key("error") {
        return McpMessage::InitializeResponse;
    }

    McpMessage::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_initialize_request() {
        let line = br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
        assert_eq!(classify(line), McpMessage::InitializeRequest);
    }

    #[test]
    fn classify_initialize_response() {
        let line = br#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"gatemini","version":"1.0.0"}}}"#;
        assert_eq!(classify(line), McpMessage::InitializeResponse);
    }

    #[test]
    fn classify_initialized_notification() {
        let line = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert_eq!(classify(line), McpMessage::InitializedNotification);
    }

    #[test]
    fn classify_error_response() {
        let line = br#"{"jsonrpc":"2.0","id":0,"error":{"code":-32600,"message":"bad request"}}"#;
        assert_eq!(classify(line), McpMessage::InitializeResponse);
    }

    #[test]
    fn classify_other_method() {
        let line = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        assert_eq!(classify(line), McpMessage::Other);
    }

    #[test]
    fn classify_invalid_json() {
        assert_eq!(classify(b"not json at all"), McpMessage::Other);
    }

    #[test]
    fn classify_empty() {
        assert_eq!(classify(b""), McpMessage::Other);
    }

    #[tokio::test]
    async fn read_line_normal() {
        let data = b"hello world\nsecond line\n";
        let mut cursor = &data[..];
        let line = read_line(&mut cursor).await.unwrap();
        assert_eq!(line, b"hello world\n");
    }

    #[tokio::test]
    async fn read_line_eof_clean() {
        let data: &[u8] = b"";
        let mut cursor = data;
        let line = read_line(&mut cursor).await.unwrap();
        assert!(line.is_empty());
    }

    #[tokio::test]
    async fn read_line_eof_partial() {
        let data = b"partial without newline";
        let mut cursor = &data[..];
        let err = read_line(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
