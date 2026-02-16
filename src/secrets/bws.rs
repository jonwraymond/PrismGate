use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use super::resolver::SecretProvider;

/// BWS provider that caches all secrets at initialization.
/// Uses the official bitwarden SDK to authenticate and fetch secrets.
pub struct BwsSdkProvider {
    /// project name → project UUID
    project_by_name: HashMap<String, Uuid>,
    /// (project_id, secret_key) → secret_value
    secret_cache: HashMap<(Uuid, String), String>,
}

impl BwsSdkProvider {
    /// Create a new BWS provider by authenticating and caching all secrets.
    ///
    /// `access_token`: BWS service account access token.
    /// `org_id`: Organization UUID string. If None, falls back to BWS_ORG_ID env var.
    pub async fn new(access_token: String, org_id: Option<String>) -> Result<Self> {
        use bitwarden::auth::login::AccessTokenLoginRequest;
        use bitwarden::secrets_manager::projects::ProjectsListRequest;
        use bitwarden::secrets_manager::secrets::{SecretIdentifiersRequest, SecretsGetRequest};
        use bitwarden::secrets_manager::{ClientProjectsExt, ClientSecretsExt};

        // Resolve org ID: config → env var → error
        let org_id_str = match org_id {
            Some(id) if !id.is_empty() => id,
            _ => std::env::var("BWS_ORG_ID")
                .context("organization_id not set in config and BWS_ORG_ID env var not found")?,
        };
        let organization_id =
            Uuid::parse_str(&org_id_str).context("invalid organization_id UUID")?;

        // Create client and authenticate
        let client = bitwarden::Client::new(None);
        client
            .auth()
            .login_access_token(&AccessTokenLoginRequest {
                access_token,
                state_file: None,
            })
            .await
            .context("BWS authentication failed")?;

        // List all projects
        let projects_resp = client
            .projects()
            .list(&ProjectsListRequest { organization_id })
            .await
            .context("failed to list BWS projects")?;

        let mut project_by_name = HashMap::new();
        for project in &projects_resp.data {
            project_by_name.insert(project.name.clone(), project.id);
        }

        // List all secret identifiers
        let identifiers_resp = client
            .secrets()
            .list(&SecretIdentifiersRequest { organization_id })
            .await
            .context("failed to list BWS secret identifiers")?;

        let ids: Vec<Uuid> = identifiers_resp.data.iter().map(|s| s.id).collect();

        // Batch-fetch all secrets
        let mut secret_cache = HashMap::new();
        if !ids.is_empty() {
            let secrets_resp = client
                .secrets()
                .get_by_ids(SecretsGetRequest { ids })
                .await
                .context("failed to fetch BWS secrets")?;

            for secret in &secrets_resp.data {
                if let Some(project_id) = secret.project_id {
                    secret_cache.insert((project_id, secret.key.clone()), secret.value.clone());
                }
            }
        }

        info!(
            projects = project_by_name.len(),
            secrets = secret_cache.len(),
            "BWS cache populated"
        );

        Ok(Self {
            project_by_name,
            secret_cache,
        })
    }

    /// Create a provider with pre-populated caches (for testing).
    #[cfg(test)]
    pub fn from_caches(
        project_by_name: HashMap<String, Uuid>,
        secret_cache: HashMap<(Uuid, String), String>,
    ) -> Self {
        Self {
            project_by_name,
            secret_cache,
        }
    }
}

impl SecretProvider for BwsSdkProvider {
    fn name(&self) -> &str {
        "bws"
    }

    /// Resolve a reference in the format `project/<PROJECT_NAME>/key/<KEY>`.
    fn resolve(&self, reference: &str) -> Result<String> {
        let parts: Vec<&str> = reference.splitn(4, '/').collect();
        if parts.len() != 4 || parts[0] != "project" || parts[2] != "key" {
            bail!(
                "invalid BWS reference format: '{reference}' (expected 'project/<name>/key/<key>')"
            );
        }

        let project_name = parts[1];
        let key = parts[3];

        let project_id = self
            .project_by_name
            .get(project_name)
            .with_context(|| format!("BWS project not found: '{project_name}'"))?;

        self.secret_cache
            .get(&(*project_id, key.to_string()))
            .cloned()
            .with_context(|| format!("BWS secret not found: project='{project_name}', key='{key}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> BwsSdkProvider {
        let project_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let project2_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();

        let mut project_by_name = HashMap::new();
        project_by_name.insert("dotenv".to_string(), project_id);
        project_by_name.insert("production".to_string(), project2_id);

        let mut secret_cache = HashMap::new();
        secret_cache.insert(
            (project_id, "API_KEY".to_string()),
            "sk-secret-123".to_string(),
        );
        secret_cache.insert((project_id, "TOKEN".to_string()), "tok-abc-456".to_string());
        secret_cache.insert(
            (project2_id, "DB_URL".to_string()),
            "postgres://prod:pass@db:5432".to_string(),
        );

        BwsSdkProvider::from_caches(project_by_name, secret_cache)
    }

    #[test]
    fn test_resolve_valid_reference() {
        let provider = make_provider();
        let result = provider.resolve("project/dotenv/key/API_KEY").unwrap();
        assert_eq!(result, "sk-secret-123");
    }

    #[test]
    fn test_resolve_different_project() {
        let provider = make_provider();
        let result = provider.resolve("project/production/key/DB_URL").unwrap();
        assert_eq!(result, "postgres://prod:pass@db:5432");
    }

    #[test]
    fn test_resolve_unknown_project() {
        let provider = make_provider();
        let result = provider.resolve("project/unknown/key/API_KEY");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("project not found")
        );
    }

    #[test]
    fn test_resolve_unknown_key() {
        let provider = make_provider();
        let result = provider.resolve("project/dotenv/key/MISSING");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("secret not found"));
    }

    #[test]
    fn test_resolve_invalid_format() {
        let provider = make_provider();
        let result = provider.resolve("invalid/format");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid BWS reference format")
        );
    }
}
