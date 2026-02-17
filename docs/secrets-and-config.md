# Secrets & Configuration

Gatemini's configuration pipeline transforms YAML config files through environment expansion, secret resolution, and validation into a running daemon with hot-reload support.

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

`.env` files are loaded from up to three locations (later overrides earlier):

1. `~/.env` (home directory)
2. `prismgate_home()/.env` (legacy internal helper name; e.g., `~/.config/gatemini/.env` on macOS/Linux)
3. Sibling of the config file (e.g., `/path/to/config.yaml` -> `/path/to/.env`)

Paths are deduplicated via `canonicalize()` to avoid double-loading when the config dir and `prismgate_home()` resolve to the same directory.

The `Once` pattern ensures env files are loaded exactly once, even if multiple threads call `load_dotenv()` concurrently. This prevents UB from `std::env::set_var` in multi-threaded contexts. Hot-reload does **not** re-read `.env` files — they are startup-only.

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
  interval: 30               # seconds between health checks
  timeout: 5                 # seconds per ping
  failure_threshold: 3       # consecutive failures for circuit break
  max_restarts: 5            # per restart_window
  restart_window: 60         # seconds
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
| `health.interval` | 30s |
| `health.timeout` | 5s |
| `health.failure_threshold` | 3 |
| `health.max_restarts` | 5 |
| `health.restart_window` | 1 min |
| `daemon.idle_timeout` | 5 min |
| `allow_runtime_registration` | `true` |
| `max_dynamic_backends` | 10 |

## Secret Resolution

**Source**: [`src/secrets/resolver.rs`](../src/secrets/resolver.rs)

Gatemini supports three modes for providing secrets. No special configuration is required for modes 1 and 2.

### Mode 1: Direct Environment Variables (simplest)

Use `${VAR}` syntax to reference environment variables or `.env` file values directly:

```yaml
backends:
  cerebras:
    command: cerebras-mcp
    env:
      CEREBRAS_API_KEY: "${CEREBRAS_API_KEY}"
```

### Mode 2: secretref with Env Var Fallback (default)

When BWS is disabled (the default), `secretref:bws:...` patterns automatically fall back to environment variable lookup. The **last path segment** is extracted as the env var name:

```yaml
backends:
  github:
    transport: streamable-http
    url: "https://api.githubcopilot.com/mcp/"
    headers:
      Authorization: "Bearer secretref:bws:project/dotenv/key/GITHUB_PAT_TOKEN"
      # BWS disabled → extracts "GITHUB_PAT_TOKEN" → resolves via std::env::var
```

This means configs written for BWS users work transparently for non-BWS users — just set the corresponding env vars.

The `EnvFallbackProvider` (in `resolver.rs`) registers with name `"bws"` so it handles existing patterns without config changes. It is only registered when `secrets.providers.bws.enabled` is `false`.

### Mode 3: Bitwarden Secrets Manager (BWS)

For full secret management, enable BWS explicitly:

```yaml
secrets:
  strict: true
  providers:
    bws:
      enabled: true
      access_token: "${BWS_ACCESS_TOKEN}"
      organization_id: "${BWS_ORG_ID}"
```

### secretref Pattern Syntax

```
secretref:<provider>:<reference>
```

**Full value** — entire string is a secretref:
```yaml
env:
  API_KEY: "secretref:bws:project/dotenv/key/EXA_API_KEY"
```

**Inline** — secretref embedded in a larger string:
```yaml
headers:
  Authorization: "Bearer secretref:bws:project/dotenv/key/MY_TOKEN"
```

### Pattern Matching

```regex
secretref:([^:\s]+):([\w/.\-]+)
```

- Group 1: Provider name (e.g., `bws`)
- Group 2: Reference path (e.g., `project/dotenv/key/EXA_API_KEY`)

### Strict Mode

When `secrets.strict: true`, empty resolved values and unresolved `secretref:` patterns are treated as errors. When `false` (default), unresolved patterns produce warnings with actionable hints.

### Unresolved Pattern Validation

After secret resolution, Gatemini scans all backend config fields for remaining `secretref:` literals. This catches typos, missing env vars, and misconfigured providers:

- **strict mode**: startup fails with a list of all unresolved patterns
- **non-strict mode**: logs warnings like `"secretref:bws:project/dotenv/key/MISSING_KEY — BWS is disabled and env var 'MISSING_KEY' not found"`

### Where Secrets Are Resolved

Secrets are resolved in all backend config fields:
- `command`
- `args` array
- `env` values
- `url` strings
- `headers` values
- `prerequisite.args` and `prerequisite.env`

## Bitwarden Secrets Manager (BWS)

**Source**: [`src/secrets/bws.rs`](../src/secrets/bws.rs)

The BWS provider integrates with [Bitwarden Secrets Manager](https://bitwarden.com/help/secrets-manager-overview/) for centralized secret storage.

### Configuration

```yaml
secrets:
  strict: true
  providers:
    bws:
      enabled: true
      access_token: "${BWS_ACCESS_TOKEN}"
      organization_id: "${BWS_ORG_ID}"
```

### Reference Format

```
secretref:bws:project/dotenv/key/{VAR_NAME}
```

The path format follows BWS's hierarchy: `project` → `dotenv` (collection) → `key` → `{VAR_NAME}`.

### Authentication

BWS uses machine account access tokens scoped to specific secret sets. The access token is read from config or the `$BWS_ACCESS_TOKEN` environment variable.

## Secret Provider Trait

Gatemini's secret resolution is provider-agnostic through a trait:

```rust
pub trait SecretProvider: Send + Sync {
    fn name(&self) -> &str;
    fn resolve(&self, reference: &str) -> Result<String>;
}
```

Built-in providers:
- **`BwsSdkProvider`** — Bitwarden Secrets Manager (when `bws.enabled: true`)
- **`EnvFallbackProvider`** — Environment variable lookup (when BWS disabled, extracts last path segment as env var name)

Custom providers (e.g., HashiCorp Vault, 1Password, AWS Secrets Manager) can be added by implementing this trait.

### Comparison with Other Tools

| Tool | Pattern | Example |
|------|---------|---------|
| **Gatemini** | `secretref:provider:path` | `secretref:bws:project/dotenv/key/API_KEY` |
| [1Password CLI](https://developer.1password.com/docs/cli/secret-references/) | `op://vault/item/field` | `op://dev/server/api_key` |
| [Vault Agent](https://developer.hashicorp.com/vault/docs/agent-and-proxy/agent/template) | `{{ .Data.data.key }}` | `{{ .Data.data.api_key }}` |
| [K8s External Secrets](https://external-secrets.io/latest/introduction/overview/) | `SecretStore` + `ExternalSecret` | YAML resources |

Gatemini's approach most closely resembles 1Password's URI-based references: config files with secret references can be safely committed to Git, and values are resolved at runtime.

## Hot-Reload

**Source**: [`src/config.rs`](../src/config.rs) -- `watch_config()`

Gatemini watches the config file for changes using the `notify` crate (macOS: kqueue, Linux: inotify):

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

### Minimal (no secrets)

```yaml
backends:
  sequential-thinking:
    command: mcp-server-sequential-thinking
    timeout: 120s

  deepwiki:
    transport: streamable-http
    url: "https://mcp.deepwiki.com/mcp"
    timeout: 90s
```

### With env vars (Mode 1)

```yaml
backends:
  exa:
    command: npx
    args: ["-y", "exa-mcp-server"]
    env:
      EXA_API_KEY: "${EXA_API_KEY}"   # Set in .env or shell
```

### With secretref + env fallback (Mode 2, default)

```yaml
# No secrets section needed — BWS disabled by default
backends:
  exa:
    command: npx
    args: ["-y", "exa-mcp-server"]
    env:
      EXA_API_KEY: "secretref:bws:project/dotenv/key/EXA_API_KEY"
      # Extracts "EXA_API_KEY", resolves via env var when BWS is off

  github:
    transport: streamable-http
    url: "https://api.githubcopilot.com/mcp/"
    headers:
      Authorization: "Bearer secretref:bws:project/dotenv/key/GITHUB_PAT_TOKEN"
    timeout: 60s
```

### With BWS enabled (Mode 3)

```yaml
secrets:
  strict: true
  providers:
    bws:
      enabled: true
      access_token: "${BWS_ACCESS_TOKEN}"
      organization_id: "${BWS_ORG_ID}"

backends:
  exa:
    command: npx
    args: ["-y", "exa-mcp-server"]
    env:
      EXA_API_KEY: "secretref:bws:project/dotenv/key/EXA_API_KEY"
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
