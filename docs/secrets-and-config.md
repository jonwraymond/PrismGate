# Secrets & Configuration

Configuration behavior is defined primarily by `src/config.rs`. This page sticks to what that code actually does today.

## Load pipeline

Config loading follows this order:

1. read YAML from disk
2. expand environment variables with `shellexpand::env`
3. deserialize YAML into `Config`
4. validate transport-specific requirements
5. resolve `secretref:` values asynchronously
6. re-check for unresolved secret refs

## `.env` load order

`.env` files are loaded exactly once per process.

Locations, in order:

1. `~/.env`
2. the standard Gatemini config directory, for example `~/.config/gatemini/.env`
3. a `.env` next to the chosen config file

Later files override earlier ones.

Hot-reload does not re-run `.env` loading. If you change `.env`, restart the process.

## Top-level defaults

Current defaults from `src/config.rs`:

| Setting | Default |
|---------|---------|
| `log_level` | `info` |
| `allow_runtime_registration` | `true` |
| `max_dynamic_backends` | `10` |
| `daemon.idle_timeout` | `5m` |
| `daemon.client_drain_timeout` | `30s` |
| `sandbox.timeout` | `30s` |
| `sandbox.max_output_size` | `200000` |
| `sandbox.max_concurrent_sandboxes` | `8` |
| `admin.listen` | `127.0.0.1:19999` |

Transport defaults:

- backend transport defaults to `stdio`
- backend timeout defaults to `30s`
- retry defaults to 3 attempts with `500ms` initial delay, `2s` max delay, multiplier `2.0`
- `instance_mode` defaults to `shared`
- `pool.min_idle` defaults to `1`
- `pool.max_instances` defaults to `20`
- `pool.acquire_timeout` defaults to `30s`

## Supported transports

The config enum is kebab-case and accepts:

- `stdio`
- `streamable-http`
- `cli-adapter`

Example:

```yaml
backends:
  github:
    transport: streamable-http
    url: "https://api.githubcopilot.com/mcp/"
```

## Secret resolution modes

### Direct environment variables

```yaml
backends:
  exa:
    env:
      EXA_API_KEY: "${EXA_API_KEY}"
```

### `secretref:bws:...` with environment fallback

When Bitwarden Secrets Manager is disabled, the fallback provider still accepts `secretref:bws:...` and resolves the last path segment as an environment variable name.

```yaml
headers:
  Authorization: "Bearer secretref:bws:project/dotenv/key/GITHUB_PAT_TOKEN"
```

With BWS disabled, that looks for `GITHUB_PAT_TOKEN` in the environment.

### Bitwarden Secrets Manager

```yaml
secrets:
  strict: true
  providers:
    bws:
      enabled: true
      access_token: "${BWS_ACCESS_TOKEN}"
      organization_id: "${BWS_ORG_ID}"
```

## What fields secret resolution touches

Secret resolution walks through:

- backend command
- backend args
- backend env map
- backend URL
- backend headers
- prerequisite command
- prerequisite args
- prerequisite env map

## Validation behavior

Validation checks include:

- required fields by transport
- valid transport values
- required CLI adapter definitions
- unresolved secret refs after resolution

The config loader does not implement a general shell language. It performs environment interpolation and then Rust-side validation.

## Hot reload

The config watcher can apply a useful subset of changes without a daemon restart.

Hot-reloaded today:

- backend additions, removals, and config changes
- aliases
- backend-owned tags and fallback-chain changes through backend reconfiguration

Detected but not applied live:

- composite tool changes

Read once at startup:

- daemon lifecycle settings
- `.env` contents

## Admin config note

`admin.allowed_cidrs` exists in config, but the current admin route implementation does not enforce CIDR checks. The binding address still matters because the admin server listens only on `admin.listen`.
