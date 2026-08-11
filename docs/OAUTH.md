# OAuth 2.0 Authentication

Gatemini supports OAuth 2.0 authentication for MCP backends that require it. The OAuth flow is automatic, secure, and handles token refresh transparently.

## How It Works

1. **Configure OAuth in your backend** - Add OAuth settings to your backend configuration
2. **First connection** - Gatemini automatically detects missing/expired tokens and launches the OAuth flow
3. **Browser authentication** - Your browser opens to the provider's login page
4. **Token storage** - Tokens are securely stored in `~/.cache/prismgate/oauth_tokens.json` with `0600` permissions
5. **Automatic refresh** - Expired tokens are automatically refreshed using refresh tokens
6. **Transparent usage** - Once authenticated, tokens are automatically injected into requests

## Configuration

### Auto-Discovery (Recommended)

Most OAuth providers support auto-discovery via `.well-known/oauth-authorization-server`:

```yaml
backends:
  my-oauth-backend:
    transport: streamable-http
    url: "https://api.example.com/mcp"
    oauth:
      discover: true                    # Auto-discover OAuth endpoints
      client_id: "your-client-id"
      scopes: ["read", "write"]
      use_pkce: true                    # PKCE for enhanced security (recommended)
      callback_port: 8080               # Local callback server port
```

### Manual Configuration

If auto-discovery isn't supported, specify endpoints manually:

```yaml
backends:
  my-oauth-backend:
    transport: streamable-http
    url: "https://api.example.com/mcp"
    oauth:
      discover: false
      authorization_url: "https://auth.example.com/oauth/authorize"
      token_url: "https://auth.example.com/oauth/token"
      client_id: "your-client-id"
      client_secret: "${OAUTH_CLIENT_SECRET}"  # Optional, for confidential clients
      scopes: ["api.read", "api.write"]
      use_pkce: true
      redirect_uri: "http://localhost:8080/callback"
      callback_port: 8080
```

## OAuth Flow

### Automatic Flow (Recommended)

Just start using the backend - OAuth happens automatically:

```bash
# First time using an OAuth-enabled backend
gatemini
# → Detects missing token
# → Opens browser for authentication
# → Stores token securely
# → Connects to backend
```

### Manual Authentication

You can also authenticate manually before using a backend:

```bash
gatemini auth my-backend \
  --url https://api.example.com \
  --client-id YOUR_CLIENT_ID \
  --scopes read,write
```

## Token Management

### Token Storage

Tokens are stored in `~/.cache/prismgate/oauth_tokens.json` with restrictive permissions (`0600` on Unix).

### Token Refresh

Gatemini automatically refreshes expired tokens using refresh tokens:

1. **Token expires** - Detected before making requests
2. **Refresh attempt** - Uses refresh token to get new access token
3. **Fallback** - If refresh fails, triggers new OAuth flow

### Token Inspection

View stored tokens:

```bash
cat ~/.cache/prismgate/oauth_tokens.json | jq
```

### Token Revocation

Remove a token to force re-authentication:

```bash
# Edit the file and remove the backend's token
vim ~/.cache/prismgate/oauth_tokens.json

# Or delete all tokens
rm ~/.cache/prismgate/oauth_tokens.json
```

## Security Features

### PKCE (Proof Key for Code Exchange)

PKCE is enabled by default (`use_pkce: true`) and provides:
- Protection against authorization code interception
- Safe for public clients (no client secret needed)
- Recommended for all OAuth flows

### Secure Storage

- Tokens stored with `0600` permissions (owner read/write only)
- Stored in user's cache directory
- Never logged or exposed in error messages

### Token Expiration

- Tokens checked for expiration before each use
- 60-second buffer to prevent race conditions
- Automatic refresh when possible

## Example: Deriver Backend

Here's a complete example for an OAuth-enabled backend like Deriver:

```yaml
backends:
  deriver:
    transport: streamable-http
    url: "https://api.deriver.example.com/mcp"
    timeout: 60s
    oauth:
      discover: true
      client_id: "deriver-mcp-client"
      scopes: ["mcp.read", "mcp.write"]
      use_pkce: true
      callback_port: 8080
```

First use:

```bash
# Start gatemini
gatemini

# First request to deriver backend triggers OAuth:
# 1. Browser opens to Deriver's login page
# 2. You authenticate with Deriver
# 3. Browser redirects to http://localhost:8080/callback
# 4. Token is stored securely
# 5. Request proceeds with token

# Subsequent requests use cached token automatically
```

## Troubleshooting

### Browser doesn't open

The OAuth flow prints the authorization URL. Copy and paste it into your browser manually.

### Callback fails

Ensure:
1. `callback_port` is not in use
2. Firewall allows localhost connections
3. OAuth app's redirect URI matches `http://localhost:{callback_port}/callback`

### Token refresh fails

If refresh tokens aren't working:
1. Check if provider supports refresh tokens
2. Verify `offline_access` or similar scope is requested
3. Delete token and re-authenticate: `rm ~/.cache/prismgate/oauth_tokens.json`

### Provider doesn't support auto-discovery

Set `discover: false` and provide `authorization_url` and `token_url` manually.

## OAuth Provider Setup

To use OAuth with your MCP backend, you need to:

1. **Register an OAuth application** with your provider
2. **Set redirect URI** to `http://localhost:8080/callback` (or your configured port)
3. **Get client ID** (and optionally client secret)
4. **Configure scopes** required for MCP access
5. **Add to gatemini config** as shown above

## Advanced Configuration

### Custom Callback Port

```yaml
oauth:
  callback_port: 9090
  redirect_uri: "http://localhost:9090/callback"
```

### Client Secret (Confidential Clients)

```yaml
oauth:
  client_id: "your-client-id"
  client_secret: "${OAUTH_CLIENT_SECRET}"  # From env var
```

### Multiple Scopes

```yaml
oauth:
  scopes:
    - "mcp.read"
    - "mcp.write"
    - "user.profile"
```

## Implementation Details

- **OAuth 2.0 Authorization Code Flow** with PKCE
- **Auto-discovery** via RFC 8414 (OAuth 2.0 Authorization Server Metadata)
- **Token refresh** via RFC 6749 (OAuth 2.0 Refresh Token)
- **PKCE** via RFC 7636 (Proof Key for Code Exchange)
- **Local callback server** on configurable port
- **Secure token storage** with restrictive file permissions
