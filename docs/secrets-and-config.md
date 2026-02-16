# Secrets & Configuration

PrismGate's configuration pipeline transforms YAML config files through environment expansion, secret resolution, and validation into a running daemon with hot-reload support.

## Configuration Pipeline

**Source**: [`src/config.rs`](../src/config.rs)

```
1. load_dotenv()           ~/.env loaded via Once pattern (thread-safe)
         │
2. YAML parse              serde_yaml_ng with shellexpand
         │                  $VAR, ${VAR}, ${VAR:-default}, ~
         │
3. resolve_secrets_async   secretref: patterns resolved via providers
         │
4. validate                Required fields, valid values
         │
5. hot-reload watcher      notify crate watches config file
```

### Stage 1: Environment Loading

```rust
static DOTENV_ONCE: std::sync::Once = std::sync::Once::new();

pub fn load_dotenv() {
    DOTENV_ONCE.call_once(|| {
        dotenvy::dotenv().ok();
    });
}
```

The `Once` pattern ensures `~/.env` is loaded exactly once, even if multiple threads call `load_dotenv()` concurrently. This prevents UB from `std::env::set_var` in multi-threaded contexts.

### Stage 2: YAML Parsing

Config values support shell-like expansion via `shellexpand`:

```yaml
backends:
  my_backend:
    command: "${HOME}/tools/my-server"    # Environment variable
    cwd: "~/projects/my-project"          # Tilde expansion
    env:
      API_KEY: "${MY_API_KEY}"            # Env var in nested values
      FALLBACK: "${MISSING:-default}"     # Default value syntax
```

### Stage 3: Secret Resolution

See [Secret Resolution](#secret-resolution) below.

### Stage 4: Validation

- Backend names checked for uniqueness
- Required fields verified (command or url per backend)
- Transport type validated (Stdio or HTTP)
- Timeout values checked for sanity

## Config Structs

```yaml
# Top-level Config
log_level: "info"            # tracing log level
daemon:
  idle_timeout: 300          # seconds, 0 = disabled
health:
  interval: 5                # seconds between health checks
  timeout: 10                # seconds per ping
  failure_threshold: 3       # consecutive failures for circuit break
  max_restarts: 5            # per restart_window
  restart_window: 300        # seconds
allow_runtime_registration: true
max_dynamic_backends: 10
semantic:
  model_path: null           # custom model2vec path (optional)
backends:
  backend_name:
    transport: stdio         # or http
    command: "..."           # stdio only
    args: [...]              # stdio only
    url: "..."               # http only
    env: {}                  # environment variables
    headers: {}              # http headers
    timeout: 30              # seconds per tool call
    required_keys: []        # env var keys needed
    prerequisite: {}         # prerequisite process config
```

### Defaults

| Field | Default |
|-------|---------|
| `log_level` | `"info"` |
| `transport` | `Stdio` |
| `timeout` | 30s |
| `health.interval` | 5s |
| `health.timeout` | 10s |
| `health.failure_threshold` | 3 |
| `health.max_restarts` | 5 |
| `health.restart_window` | 5 min |
| `daemon.idle_timeout` | 5 min |
| `allow_runtime_registration` | `true` |
| `max_dynamic_backends` | 10 |

## Secret Resolution

**Source**: [`src/secrets/resolver.rs`](../src/secrets/resolver.rs)

PrismGate resolves secrets at config load time using a `secretref:` pattern syntax:

```
secretref:<provider>:<reference>
```

### Resolution Modes

**Full value** -- entire string is a secretref:
```yaml
env:
  API_KEY: "secretref:bws:project/dotenv/key/EXA_API_KEY"
```
Resolved directly: the secret value replaces the entire string.

**Inline** -- secretref embedded in a larger string:
```yaml
headers:
  Authorization: "Bearer secretref:bws:project/dotenv/key/MY_TOKEN"
```
Resolved via regex replacement: only the `secretref:...` portion is replaced.

### Pattern Matching

```regex
secretref:([^:\s]+):([\w/.\-]+)
```

- Group 1: Provider name (e.g., `bws`)
- Group 2: Reference path (e.g., `project/dotenv/key/EXA_API_KEY`)

### Strict Mode

Empty resolved values are treated as errors. This prevents backends from starting with blank API keys.

### Where Secrets Are Resolved

Secrets are resolved in all backend config fields:
- `env` values
- `headers` values
- `url` strings

## Bitwarden Secrets Manager (BWS)

**Source**: [`src/secrets/bws.rs`](../src/secrets/bws.rs)

The BWS provider integrates with [Bitwarden Secrets Manager](https://bitwarden.com/help/secrets-manager-overview/) for centralized secret storage:

### Configuration

```yaml
secrets:
  bws:
    access_token: "${BWS_ACCESS_TOKEN}"  # or set env var directly
    org_id: "${BWS_ORG_ID}"              # organization ID
```

### Reference Format

```
secretref:bws:project/dotenv/key/{VAR_NAME}
```

The path format follows BWS's hierarchy: `project` → `dotenv` (collection) → `key` → `{VAR_NAME}`.

### Authentication

BWS uses machine account access tokens scoped to specific secret sets. The access token is read from config or the `$BWS_ACCESS_TOKEN` environment variable.

## Secret Provider Trait

PrismGate's secret resolution is provider-agnostic through a trait:

```rust
#[async_trait]
pub trait SecretProvider: Send + Sync {
    async fn resolve(&self, reference: &str) -> Result<String>;
}
```

This enables adding new providers (e.g., HashiCorp Vault, 1Password, AWS Secrets Manager) without changing the resolution pipeline.

### Comparison with Other Tools

| Tool | Pattern | Example |
|------|---------|---------|
| **PrismGate** | `secretref:provider:path` | `secretref:bws:project/dotenv/key/API_KEY` |
| [1Password CLI](https://developer.1password.com/docs/cli/secret-references/) | `op://vault/item/field` | `op://dev/server/api_key` |
| [Vault Agent](https://developer.hashicorp.com/vault/docs/agent-and-proxy/agent/template) | `{{ .Data.data.key }}` | `{{ .Data.data.api_key }}` |
| [K8s External Secrets](https://external-secrets.io/latest/introduction/overview/) | `SecretStore` + `ExternalSecret` | YAML resources |

PrismGate's approach most closely resembles 1Password's URI-based references: config files with secret references can be safely committed to Git, and values are resolved at runtime.

## Hot-Reload

**Source**: [`src/config.rs`](../src/config.rs) -- `watch_config()`

PrismGate watches the config file for changes using the `notify` crate (macOS: kqueue, Linux: inotify):

```
Config file change detected
    │
    ├── Re-parse YAML (shellexpand + serde)
    ├── Re-resolve secrets (async)
    ├── Diff backends:
    │   ├── New backend → add_backend()
    │   ├── Removed backend → remove_backend()
    │   └── Changed backend → stop + restart
    └── arc_swap for lock-free config update
```

### Lock-Free Config Swaps

The config is wrapped in `arc_swap::ArcSwap`, enabling atomic updates without blocking readers:

```rust
config_holder.store(Arc::new(new_config));
```

Existing references to the old config continue working until dropped. New references see the updated config immediately.

## Example Configuration

```yaml
log_level: info

daemon:
  idle_timeout: 300

health:
  interval: 5
  timeout: 10
  failure_threshold: 3
  max_restarts: 5
  restart_window: 300

secrets:
  bws:
    access_token: "${BWS_ACCESS_TOKEN}"

backends:
  exa:
    command: npx
    args: ["-y", "exa-mcp-server"]
    env:
      EXA_API_KEY: "secretref:bws:project/dotenv/key/EXA_API_KEY"

  tavily:
    command: npx
    args: ["-y", "tavily-mcp-server"]
    env:
      TAVILY_API_KEY: "secretref:bws:project/dotenv/key/TAVILY_API_KEY"

  custom_http:
    transport: http
    url: "https://api.example.com/mcp"
    headers:
      Authorization: "Bearer secretref:bws:project/dotenv/key/CUSTOM_TOKEN"
    timeout: 60
```

## Sources

- [`src/config.rs`](../src/config.rs) -- Config pipeline, hot-reload
- [`src/secrets/resolver.rs`](../src/secrets/resolver.rs) -- Secret resolution engine
- [`src/secrets/bws.rs`](../src/secrets/bws.rs) -- Bitwarden integration
- [Bitwarden Secrets Manager](https://bitwarden.com/help/secrets-manager-overview/) -- BWS overview
- [Bitwarden SDK](https://bitwarden.com/help/secrets-manager-sdk/) -- Rust SDK reference
- [Bitwarden Access Tokens](https://bitwarden.com/help/access-tokens/) -- Machine account auth
- [1Password Secret References](https://developer.1password.com/docs/cli/secret-references/) -- Pattern comparison
- [OWASP Secrets Management](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html) -- Best practices
- [shellexpand](https://docs.rs/shellexpand) -- Environment variable expansion
- [notify crate](https://docs.rs/notify) -- File system watcher
- [arc_swap](https://docs.rs/arc-swap) -- Lock-free atomic swaps
