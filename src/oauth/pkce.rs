//! PKCE (Proof Key for Code Exchange) implementation - RFC 7636.

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};

/// PKCE challenge and verifier pair.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// The code verifier (random string)
    pub verifier: String,

    /// The code challenge (SHA256 hash of verifier)
    pub challenge: String,

    /// The challenge method (always "S256")
    pub method: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge.
    pub fn new() -> Result<Self> {
        // Generate random verifier (43-128 characters)
        let verifier = Self::generate_verifier()?;

        // Compute SHA256 challenge
        let challenge = Self::compute_challenge(&verifier)?;

        Ok(Self {
            verifier,
            challenge,
            method: "S256".to_string(),
        })
    }

    /// Generate a random code verifier.
    fn generate_verifier() -> Result<String> {
        let mut rng = rand::thread_rng();
        let random_bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
        Ok(URL_SAFE_NO_PAD.encode(random_bytes))
    }

    /// Compute the code challenge from a verifier.
    fn compute_challenge(verifier: &str) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        Ok(URL_SAFE_NO_PAD.encode(hash))
    }
}

impl Default for PkceChallenge {
    fn default() -> Self {
        Self::new().expect("failed to generate PKCE challenge")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let pkce = PkceChallenge::new().unwrap();

        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.challenge.is_empty());
        assert_eq!(pkce.method, "S256");

        // Verifier and challenge should be different
        assert_ne!(pkce.verifier, pkce.challenge);
    }

    #[test]
    fn test_pkce_deterministic() {
        let verifier = "test_verifier_12345";
        let challenge = PkceChallenge::compute_challenge(verifier).unwrap();

        // Same verifier should produce same challenge
        let challenge2 = PkceChallenge::compute_challenge(verifier).unwrap();
        assert_eq!(challenge, challenge2);
    }
}
