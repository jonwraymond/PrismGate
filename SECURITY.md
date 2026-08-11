# Security Policy

## Supported Versions

Only the latest release line is actively supported with security patches.

| Version | Supported          |
| ------- | ------------------ |
| 1.14.x  | :white_check_mark: |
| < 1.14  | :x:                |

## Reporting a Vulnerability

The PrismGate team takes security issues seriously. We appreciate your efforts
to responsibly disclose your findings.

### Responsible Disclosure Process

1. **Do not open a public issue.** Security reports must be kept confidential
   until a fix is available.

2. **Send your report** to **[security@prismgate.dev](mailto:security@prismgate.dev)**.
   Include as much detail as possible:
   - A clear description of the vulnerability
   - Steps to reproduce the issue
   - Affected versions
   - Any potential mitigations you've identified
   - Your contact information for follow-up

3. **What to expect:**

   | Milestone | Target |
   |-----------|--------|
   | Acknowledgment of receipt | Within **48 hours** |
   | Initial assessment and confirmation | Within **72 hours** |
   | Fix released | Within **7 days** of confirmation (critical issues) |

4. **After the fix** is released, we will:
   - Publish a security advisory with credit to the reporter (unless anonymity
     is requested)
   - Coordinate a coordinated disclosure timeline if the reporter prefers to
     publish their own advisory

### Scope

The security policy covers:

- The `gatemini` binary and all runtime modes (proxy, direct, serve)
- The Unix socket IPC layer
- Configuration parsing and secret resolution
- The V8 sandbox boundary
- MCP protocol handling
- Backend transport implementations (stdio, streamable-http, cli-adapter)

### Out of Scope

- Issues in third-party backend MCP servers proxied through Gatemini
- Denial-of-service attacks already mitigated by configurable limits
  (max_memory_mb, restart thresholds, idle timeouts)
- Social engineering or phishing attacks

### Recognition

We maintain a [security hall of fame](https://github.com/jonwraymond/prismgate/security/advisories)
acknowledging researchers who have responsibly disclosed vulnerabilities.