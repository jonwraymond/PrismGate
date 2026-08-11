//! Local HTTP server for OAuth callback handling.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;

/// OAuth callback result.
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
    result: Arc<Mutex<Option<CallbackResult>>>,
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
    /// Returns the authorization code when received.
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
        let (addr, server) = warp::serve(routes)
            .bind_with_graceful_shutdown(
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
            if let Some(result) = guard.as_ref() {
                return Ok(result.clone());
            }
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct CallbackQuery {
    code: String,
    state: Option<String>,
}

async fn handle_callback(
    query: CallbackQuery,
    result: Arc<Mutex<Option<CallbackResult>>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    // Store the result
    let mut guard = result.lock().await;
    *guard = Some(CallbackResult {
        code: query.code,
        state: query.state,
    });

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
        "#,
    ))
}
