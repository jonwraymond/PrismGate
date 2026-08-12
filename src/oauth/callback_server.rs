//! Local HTTP server for OAuth callback handling.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;

/// OAuth callback result.
#[derive(Debug, Clone)]
pub enum CallbackOutcome {
    /// Successful authorization code grant.
    Success(CallbackResult),
    /// Provider returned an OAuth error on the redirect.
    Error {
        error: String,
        description: Option<String>,
    },
}

/// Successful OAuth callback payload.
#[derive(Debug, Clone)]
pub struct CallbackResult {
    /// Authorization code from OAuth provider
    pub code: String,

    /// State parameter (for CSRF protection)
    pub state: Option<String>,
}

/// Local HTTP server for OAuth callbacks.
pub struct CallbackServer {
    port: u16,
    result: Arc<Mutex<Option<CallbackOutcome>>>,
}

impl CallbackServer {
    /// Create a new callback server.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            result: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the server and wait for a callback.
    ///
    /// Returns the authorization code when received, or an error if the
    /// provider redirected with `error=` / `error_description=`.
    pub async fn wait_for_callback(&self) -> Result<CallbackResult> {
        let result = self.result.clone();
        let result_filter = warp::any().map(move || result.clone());
        let result_shutdown = self.result.clone();

        let callback = warp::get()
            .and(warp::path("callback"))
            .and(warp::query::<CallbackQuery>())
            .and(result_filter)
            .and_then(handle_callback);

        let routes = callback;

        // Start server
        let (addr, server) = warp::serve(routes).bind_with_graceful_shutdown(
            ([127, 0, 0, 1], self.port),
            async move {
                // Wait until we have a result
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    let guard = result_shutdown.lock().await;
                    if guard.is_some() {
                        break;
                    }
                }
            },
        );

        eprintln!("OAuth callback server listening on http://{}", addr);

        // Run server in background
        tokio::spawn(server);

        // Wait for result
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let guard = self.result.lock().await;
            match guard.as_ref() {
                Some(CallbackOutcome::Success(result)) => return Ok(result.clone()),
                Some(CallbackOutcome::Error { error, description }) => {
                    if let Some(desc) = description {
                        anyhow::bail!("OAuth provider error: {error} ({desc})");
                    }
                    anyhow::bail!("OAuth provider error: {error}");
                }
                None => {}
            }
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct CallbackQuery {
    /// Present on success.
    code: Option<String>,
    /// Present on provider error redirects (RFC 6749 §4.1.2.1).
    error: Option<String>,
    error_description: Option<String>,
    state: Option<String>,
}

async fn handle_callback(
    query: CallbackQuery,
    result: Arc<Mutex<Option<CallbackOutcome>>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    if let Some(error) = query.error {
        let description = query.error_description;
        let mut guard = result.lock().await;
        *guard = Some(CallbackOutcome::Error {
            error: error.clone(),
            description: description.clone(),
        });

        let detail = description.unwrap_or_else(|| "No additional details provided.".to_string());
        let html = format!(
            r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>OAuth Failed</title>
            <style>
                body {{
                    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
                    display: flex;
                    justify-content: center;
                    align-items: center;
                    height: 100vh;
                    margin: 0;
                    background: #111;
                    color: #eee;
                }}
                .container {{
                    background: #1c1c1c;
                    padding: 2rem;
                    border-radius: 8px;
                    box-shadow: 0 2px 8px rgba(0,0,0,0.4);
                    text-align: center;
                    max-width: 40rem;
                }}
                h1 {{ color: #ef4444; margin: 0 0 1rem 0; }}
                p {{ color: #bbb; margin: 0.5rem 0; }}
                code {{ color: #fbbf24; }}
            </style>
        </head>
        <body>
            <div class="container">
                <h1>Authorization Failed</h1>
                <p><code>{error}</code></p>
                <p>{detail}</p>
                <p>You can close this window and return to the terminal.</p>
            </div>
        </body>
        </html>
        "#,
            error = html_escape(&error),
            detail = html_escape(&detail),
        );
        return Ok(warp::reply::html(html));
    }

    let Some(code) = query.code else {
        return Ok(warp::reply::html(
            r#"
        <!DOCTYPE html>
        <html><body style="font-family:sans-serif;background:#111;color:#eee;padding:2rem">
          <h1>OAuth callback missing code</h1>
          <p>The provider did not return an authorization code or error.</p>
        </body></html>
        "#
            .to_string(),
        ));
    };

    // Store the result
    let mut guard = result.lock().await;
    *guard = Some(CallbackOutcome::Success(CallbackResult {
        code,
        state: query.state,
    }));

    // Return success page
    Ok(warp::reply::html(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>OAuth Success</title>
            <style>
                body {
                    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
                    display: flex;
                    justify-content: center;
                    align-items: center;
                    height: 100vh;
                    margin: 0;
                    background: #f5f5f5;
                }
                .container {
                    background: white;
                    padding: 2rem;
                    border-radius: 8px;
                    box-shadow: 0 2px 8px rgba(0,0,0,0.1);
                    text-align: center;
                }
                h1 { color: #22c55e; margin: 0 0 1rem 0; }
                p { color: #666; margin: 0; }
            </style>
        </head>
        <body>
            <div class="container">
                <h1>✓ Authorization Successful</h1>
                <p>You can close this window and return to the terminal.</p>
            </div>
        </body>
        </html>
        "#
        .to_string(),
    ))
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
