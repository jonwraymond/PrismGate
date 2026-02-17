use anyhow::{Context, Result, bail};
use regex::Regex;
use std::collections::HashMap;

/// A provider that can resolve secret references for a given scheme.
pub trait SecretProvider: Send + Sync {
    /// Provider name (e.g., "bws").
    fn name(&self) -> &str;

    /// Resolve a reference string (e.g., "project/dotenv/key/CEREBRAS_API_KEY") to its value.
    fn resolve(&self, reference: &str) -> Result<String>;
}

/// Fallback provider for `secretref:bws:...` patterns when BWS is disabled.
///
/// Extracts the last path segment (the key name) from the reference and
/// resolves it via `std::env::var`. This lets configs written for BWS work
/// transparently when users set the same key names as environment variables
/// or in `.env` files.
///
/// Registered with name `"bws"` so it transparently handles existing
/// `secretref:bws:...` patterns. Only registered when BWS is disabled.
pub struct EnvFallbackProvider;

impl SecretProvider for EnvFallbackProvider {
    fn name(&self) -> &str {
        "bws"
    }

    fn resolve(&self, reference: &str) -> Result<String> {
        let key = reference
            .rsplit('/')
            .next()
            .context("cannot extract key name from secretref reference")?;
        std::env::var(key).with_context(|| {
            format!(
                "secretref:bws:{reference} — BWS is disabled and env var '{key}' not found. \
                 Set {key} in your .env file or shell environment."
            )
        })
    }
}

/// Resolves `secretref:<provider>:<reference>` patterns in config values.
pub struct SecretResolver {
    providers: HashMap<String, Box<dyn SecretProvider>>,
    pattern: Regex,
    strict: bool,
}

impl SecretResolver {
    /// Create a new resolver.
    /// When `strict` is true, empty resolved values are treated as errors.
    pub fn new(strict: bool) -> Self {
        Self {
            providers: HashMap::new(),
            // Matches: secretref:<provider>:<reference>
            // Reference consists of path-like characters (alphanumeric, /, _, -, .)
            // Stops at &, =, whitespace, quotes, and other URL/config delimiters
            pattern: Regex::new(r"secretref:([^:\s]+):([\w/.\-]+)").unwrap(),
            strict,
        }
    }

    /// Register a secret provider.
    pub fn register(&mut self, provider: Box<dyn SecretProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// Resolve all secretref patterns in a single string value.
    /// If the entire value is a single secretref, returns the resolved value directly.
    /// Otherwise performs inline replacement of all secretref patterns.
    pub fn resolve_value(&self, value: &str) -> Result<String> {
        // Fast path: no secretref at all
        if !value.contains("secretref:") {
            return Ok(value.to_string());
        }

        // Check if the entire value is a single secretref (no surrounding text)
        let captures: Vec<_> = self.pattern.captures_iter(value).collect();

        if captures.len() == 1 {
            let cap = &captures[0];
            let full_match = cap.get(0).unwrap();
            if full_match.start() == 0 && full_match.end() == value.len() {
                // Entire value is a single secretref — resolve directly
                let provider_name = &cap[1];
                let reference = &cap[2];
                return self.resolve_single(provider_name, reference);
            }
        }

        // Inline replacement: replace right-to-left to preserve earlier indices
        let mut result = value.to_string();
        let matches: Vec<_> = self
            .pattern
            .captures_iter(value)
            .map(|cap| {
                let full = cap.get(0).unwrap();
                (
                    full.start(),
                    full.end(),
                    cap[1].to_string(),
                    cap[2].to_string(),
                )
            })
            .collect();

        // Replace right-to-left
        for (start, end, provider_name, reference) in matches.into_iter().rev() {
            let resolved = self.resolve_single(&provider_name, &reference)?;
            result.replace_range(start..end, &resolved);
        }

        Ok(result)
    }

    /// Resolve all values in a HashMap.
    pub fn resolve_map(&self, map: &mut HashMap<String, String>) -> Result<()> {
        for (key, value) in map.iter_mut() {
            let resolved = self
                .resolve_value(value)
                .with_context(|| format!("resolving key '{key}'"))?;
            *value = resolved;
        }
        Ok(())
    }

    /// Resolve all values in a Vec.
    pub fn resolve_slice(&self, slice: &mut [String]) -> Result<()> {
        for (i, value) in slice.iter_mut().enumerate() {
            let resolved = self
                .resolve_value(value)
                .with_context(|| format!("resolving index {i}"))?;
            *value = resolved;
        }
        Ok(())
    }

    /// Resolve an optional string value.
    pub fn resolve_option(&self, opt: &mut Option<String>) -> Result<()> {
        if let Some(value) = opt {
            *value = self.resolve_value(value)?;
        }
        Ok(())
    }

    fn resolve_single(&self, provider_name: &str, reference: &str) -> Result<String> {
        let provider = self
            .providers
            .get(provider_name)
            .with_context(|| format!("unknown secret provider: '{provider_name}'"))?;

        let resolved = provider.resolve(reference).with_context(|| {
            format!("provider '{provider_name}' failed to resolve '{reference}'")
        })?;

        if self.strict && resolved.is_empty() {
            bail!(
                "secret provider '{provider_name}' returned empty value for '{reference}' (strict mode)"
            );
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubProvider {
        secrets: HashMap<String, String>,
    }

    impl StubProvider {
        fn new(secrets: Vec<(&str, &str)>) -> Self {
            Self {
                secrets: secrets
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }
        }
    }

    impl SecretProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn resolve(&self, reference: &str) -> Result<String> {
            self.secrets
                .get(reference)
                .cloned()
                .with_context(|| format!("secret not found: {reference}"))
        }
    }

    fn make_resolver(strict: bool) -> SecretResolver {
        let mut resolver = SecretResolver::new(strict);
        resolver.register(Box::new(StubProvider::new(vec![
            ("project/dotenv/key/API_KEY", "sk-12345"),
            ("project/dotenv/key/TOKEN", "tok-abc"),
            ("project/dotenv/key/EMPTY", ""),
        ])));
        resolver
    }

    #[test]
    fn test_resolve_full_value() {
        let resolver = make_resolver(false);
        let result = resolver
            .resolve_value("secretref:stub:project/dotenv/key/API_KEY")
            .unwrap();
        assert_eq!(result, "sk-12345");
    }

    #[test]
    fn test_resolve_inline() {
        let resolver = make_resolver(false);
        let result = resolver
            .resolve_value("Bearer secretref:stub:project/dotenv/key/TOKEN")
            .unwrap();
        assert_eq!(result, "Bearer tok-abc");
    }

    #[test]
    fn test_resolve_multiple_inline() {
        let resolver = make_resolver(false);
        let result = resolver
            .resolve_value(
                "https://api.example.com?key=secretref:stub:project/dotenv/key/API_KEY&token=secretref:stub:project/dotenv/key/TOKEN",
            )
            .unwrap();
        assert_eq!(result, "https://api.example.com?key=sk-12345&token=tok-abc");
    }

    #[test]
    fn test_no_secretref_passthrough() {
        let resolver = make_resolver(false);
        let result = resolver.resolve_value("plain-value").unwrap();
        assert_eq!(result, "plain-value");
    }

    #[test]
    fn test_unknown_provider_error() {
        let resolver = make_resolver(false);
        let result = resolver.resolve_value("secretref:unknown:some/ref");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown secret provider")
        );
    }

    #[test]
    fn test_strict_mode_rejects_empty() {
        let resolver = make_resolver(true);
        let result = resolver.resolve_value("secretref:stub:project/dotenv/key/EMPTY");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty value"));
    }

    #[test]
    fn test_lenient_mode_allows_empty() {
        let resolver = make_resolver(false);
        let result = resolver
            .resolve_value("secretref:stub:project/dotenv/key/EMPTY")
            .unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_resolve_map() {
        let resolver = make_resolver(false);
        let mut map = HashMap::new();
        map.insert(
            "key1".to_string(),
            "secretref:stub:project/dotenv/key/API_KEY".to_string(),
        );
        map.insert("key2".to_string(), "literal".to_string());
        resolver.resolve_map(&mut map).unwrap();
        assert_eq!(map["key1"], "sk-12345");
        assert_eq!(map["key2"], "literal");
    }

    // --- EnvFallbackProvider tests ---

    #[test]
    fn test_env_fallback_provider_resolve() {
        // SAFETY: test runs single-threaded
        unsafe { std::env::set_var("GATEMINI_TEST_SECRET_1", "my-secret-value") };

        let provider = EnvFallbackProvider;
        let result = provider
            .resolve("project/dotenv/key/GATEMINI_TEST_SECRET_1")
            .unwrap();
        assert_eq!(result, "my-secret-value");

        unsafe { std::env::remove_var("GATEMINI_TEST_SECRET_1") };
    }

    #[test]
    fn test_env_fallback_provider_missing() {
        // Ensure the var doesn't exist
        unsafe { std::env::remove_var("GATEMINI_TEST_NONEXISTENT") };

        let provider = EnvFallbackProvider;
        let result = provider.resolve("project/dotenv/key/GATEMINI_TEST_NONEXISTENT");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("GATEMINI_TEST_NONEXISTENT"), "error should mention the key name: {err}");
        assert!(err.contains("BWS is disabled"), "error should mention BWS is disabled: {err}");
    }

    #[test]
    fn test_env_fallback_inline_resolution() {
        unsafe { std::env::set_var("GATEMINI_TEST_TOKEN_2", "tok-inline-test") };

        let mut resolver = SecretResolver::new(false);
        resolver.register(Box::new(EnvFallbackProvider));

        let result = resolver
            .resolve_value("Bearer secretref:bws:project/dotenv/key/GATEMINI_TEST_TOKEN_2")
            .unwrap();
        assert_eq!(result, "Bearer tok-inline-test");

        unsafe { std::env::remove_var("GATEMINI_TEST_TOKEN_2") };
    }

    #[test]
    fn test_env_fallback_full_value_resolution() {
        unsafe { std::env::set_var("GATEMINI_TEST_FULL_3", "full-value-result") };

        let mut resolver = SecretResolver::new(false);
        resolver.register(Box::new(EnvFallbackProvider));

        let result = resolver
            .resolve_value("secretref:bws:project/dotenv/key/GATEMINI_TEST_FULL_3")
            .unwrap();
        assert_eq!(result, "full-value-result");

        unsafe { std::env::remove_var("GATEMINI_TEST_FULL_3") };
    }
}
