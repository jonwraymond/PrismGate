use thiserror::Error;

use super::BackendState;

/// Custom error type for backend operations to enable robust error handling
/// without fragile string matching.
#[derive(Error, Debug)]
pub enum BackendError {
    /// Backend is not available (unhealthy or stopped).
    /// The state field allows callers to distinguish between different unavailability reasons.
    #[error(
        "backend '{backend}' is not available (state: {state:?}). Check status: @gatemini://backend/{backend}"
    )]
    Unavailable {
        backend: String,
        state: BackendState,
    },

    /// Backend is still starting after multiple retries.
    #[error(
        "backend '{backend}' is still starting (retried {retries} times over ~3.5s). Tool '{tool}' is cached but the backend hasn't connected yet. Check status: @gatemini://backend/{backend}"
    )]
    StillStarting {
        backend: String,
        tool: String,
        retries: usize,
    },

    /// Backend not found in the manager.
    #[error(
        "backend '{backend}' not found after {retries} retries. It may not be configured or failed to start. See all backends: @gatemini://backends"
    )]
    NotFound { backend: String, retries: usize },

    /// Other backend errors (wraps the underlying error).
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl BackendError {
    /// Returns true if this error represents a stopped backend that can be restarted.
    pub fn is_stopped_backend(&self) -> bool {
        matches!(
            self,
            BackendError::Unavailable {
                state: BackendState::Stopped,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stopped_backend() {
        let stopped = BackendError::Unavailable {
            backend: "test".to_string(),
            state: BackendState::Stopped,
        };
        assert!(stopped.is_stopped_backend());

        let unhealthy = BackendError::Unavailable {
            backend: "test".to_string(),
            state: BackendState::Unhealthy,
        };
        assert!(!unhealthy.is_stopped_backend());

        let not_found = BackendError::NotFound {
            backend: "test".to_string(),
            retries: 3,
        };
        assert!(!not_found.is_stopped_backend());
    }

    #[test]
    fn test_error_messages_contain_backend_name() {
        let err = BackendError::Unavailable {
            backend: "test-backend".to_string(),
            state: BackendState::Stopped,
        };
        let msg = err.to_string();
        assert!(msg.contains("test-backend"));
        assert!(msg.contains("@gatemini://backend/test-backend"));
    }

    #[test]
    fn test_still_starting_error() {
        let err = BackendError::StillStarting {
            backend: "slow-backend".to_string(),
            tool: "my_tool".to_string(),
            retries: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("slow-backend"));
        assert!(msg.contains("my_tool"));
        assert!(msg.contains("5 times"));
    }
}
