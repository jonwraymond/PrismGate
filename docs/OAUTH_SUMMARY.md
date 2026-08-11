# OAuth 2.0 Implementation Summary

## What We Built

A complete OAuth 2.0 authentication system for MCP backends that:

✅ **Automatic OAuth Flow** - Detects when backends need authentication and triggers OAuth automatically  
✅ **Secure Token Storage** - Stores tokens with `0600` permissions in `~/.cache/prismgate/oauth_tokens.json`  
✅ **Auto-Discovery** - Fetches OAuth endpoints from `.well-known/oauth-authorization-server`  
✅ **PKCE Support** - Enhanced security with Proof Key for Code Exchange (RFC 7636)  
✅ **Token Refresh** - Automatically refreshes expired tokens using refresh tokens  
✅ **Browser Integration** - Opens browser for user authentication, runs local callback server  
✅ **Manual Auth Command** - `gatemini auth` for pre-authentication  

## How It Works for Users

### 1. Configure Backend with OAuth

```yaml
backends:
  deriver:
    transport: streamable-http
    url: "https://api.deriver.example.com/mcp"
    oauth:
      discover: true
      client_id: "your-client-id"
      scopes: ["mcp.read", "mcp.write"]
      use_pkce: true
```

### 2. Start Gatemini

```bash
gatemini
```

### 3. OAuth Happens Automatically

When the backend is first accessed:
1. ✅ Gatemini detects no token exists
2. ✅ Opens browser to provider's login page
3. ✅ User authenticates with provider
4. ✅ Browser redirects to `http://localhost:8080/callback`
5. ✅ Token is securely stored
6. ✅ Backend connection proceeds with token

### 4. Subsequent Uses Are Seamless

- ✅ Token loaded from cache
- ✅ Automatically refreshed if expired
- ✅ Injected into HTTP requests
- ✅ No user interaction needed

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────┐
│                      HTTP Backend                           │
│  - Checks for OAuth config                                  │
│  - Loads/refreshes tokens automatically                     │
│  - Injects tokens into Authorization header                 │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                     OAuth Module                            │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │   Discovery  │  │  OAuth Flow  │  │ Token Store  │      │
│  │              │  │              │  │              │      │
│  │ - Fetch      │  │ - PKCE       │  │ - Save       │      │
│  │   endpoints  │  │ - Browser    │  │ - Load       │      │
│  │ - Parse      │  │ - Callback   │  │ - Refresh    │      │
│  │   metadata   │  │ - Exchange   │  │ - Expire     │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
```

### Files

- `src/oauth/mod.rs` - Public API and orchestration
- `src/oauth/config.rs` - OAuth configuration types
- `src/oauth/discovery.rs` - Auto-discovery (RFC 8414)
- `src/oauth/flow.rs` - Authorization code flow with PKCE
- `src/oauth/pkce.rs` - PKCE challenge generation
- `src/oauth/callback_server.rs` - Local HTTP callback server
- `src/oauth/token_store.rs` - Secure token persistence
- `src/backend/http.rs` - Integration with HTTP backends
- `src/cli.rs` - `auth` command definition
- `src/main.rs` - `auth` command handler

## Security Features

### PKCE (Proof Key for Code Exchange)
- ✅ Generates random code verifier (43-128 chars)
- ✅ Creates SHA-256 code challenge
- ✅ Protects against authorization code interception
- ✅ Safe for public clients (no client secret needed)

### Secure Token Storage
- ✅ Stored in `~/.cache/prismgate/oauth_tokens.json`
- ✅ File permissions set to `0600` (owner read/write only)
- ✅ Never logged or exposed in error messages
- ✅ Tokens checked for expiration before use (60s buffer)

### Token Refresh
- ✅ Automatic refresh when tokens expire
- ✅ Falls back to new OAuth flow if refresh fails
- ✅ Transparent to users

## Example Usage

### Deriver Backend

```yaml
# ~/.config/gatemini/config.yaml
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

```bash
# Start gatemini - OAuth happens automatically on first use
gatemini

# Or pre-authenticate manually
gatemini auth deriver \
  --url https://api.deriver.example.com \
  --client-id deriver-mcp-client \
  --scopes mcp.read,mcp.write
```

## Testing

To test with a real OAuth provider:

1. **Register OAuth app** with provider
2. **Set redirect URI** to `http://localhost:8080/callback`
3. **Get client ID**
4. **Configure backend** in `config.yaml`
5. **Start gatemini** - OAuth flow triggers automatically

## Future Enhancements

Potential improvements:

- [ ] Device code flow for headless environments
- [ ] Token encryption at rest
- [ ] OAuth token introspection
- [ ] Support for JWT bearer tokens
- [ ] OAuth 2.1 compliance
- [ ] Token revocation on backend removal

## Documentation

- **User Guide**: `docs/OAUTH.md` - Complete OAuth usage documentation
- **Example Config**: `config/example.yaml` - OAuth configuration examples
- **CLI Help**: `gatemini auth --help` - Command-line reference
