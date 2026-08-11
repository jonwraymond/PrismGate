# Security Audit

Last audit: 2026-05-24 | Cargo.lock: 655 dependencies

## Active Vulnerabilities

### HIGH — aws-lc-sys 0.38.0

| Advisory | Severity | Fix |
|---|---|---|
| [RUSTSEC-2026-0048](https://rustsec.org/advisories/RUSTSEC-2026-0048) — CRL Distribution Point Scope Check Logic Error | 7.4 (high) | Upgrade to >=0.39.0 |
| [RUSTSEC-2026-0044](https://rustsec.org/advisories/RUSTSEC-2026-0044) — X.509 Name Constraints Bypass via Wildcard/Unicode CN | high | Upgrade to >=0.39.0 |

**Dependency chain** (all transitive — cannot fix directly):
```
aws-lc-sys 0.38.0
└── aws-lc-rs 1.16.1
    ├── rustls 0.23.37 (→ gatemini direct dep)
    │   ├── reqwest 0.13.2 (→ gatemini direct + rmcp)
    │   └── bitwarden-core 2.0.0 (→ bitwarden 2.0.0 → gatemini)
    ├── quinn-proto 0.11.14 (→ reqwest)
    └── deno_crypto 0.227.0 (→ rustyscript 0.12.3 → gatemini sandbox)
```
**Note:** `aws-lc-rs` has released 1.17+ (pins `aws-lc-sys >=0.39.0`), but `rustls 0.23.37` pins `aws-lc-rs ~1.12`. Requires a `rustls` upgrade in our dependency tree for resolution.

### MEDIUM — rsa 0.9.10

| Advisory | Severity | Fix |
|---|---|---|
| [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) — Marvin Attack: potential key recovery through timing sidechannels | 5.9 (medium) | **No fix available** |

**Dependency chain** (all transitive):
```
rsa 0.9.10
├── deno_crypto 0.227.0 (→ rustyscript 0.12.3 → gatemini sandbox)
└── bitwarden-crypto 2.0.0 (→ bitwarden 2.0.0 → gatemini)
```
**Status:** No upstream fix exists for `rsa 0.9.x`. Both consumers (deno_crypto, bitwarden-crypto) are transitive deps we cannot patch. Ignored in CI.

### MEDIUM — time 0.3.44

| Advisory | Severity | Fix |
|---|---|---|
| [RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009) — Denial of Service via Stack Exhaustion | 6.8 (medium) | Upgrade to >=0.3.47 |

**Dependency chain** (all transitive):
```
time 0.3.44
├── zxcvbn 3.1.0 (→ bitwarden-core 2.0.0 → bitwarden 2.0.0 → gatemini)
└── serde_with 3.14.1 (→ bitwarden-api-* → bitwarden-core → bitwarden → gatemini)
```
**Status:** Both paths go through `bitwarden`. Upstream `bitwarden` crate needs to bump its deps.

## Unmaintained Crates

All transitive — cannot fix directly from gatemini.

| Crate | Version | Advisory | Via |
|---|---|---|---|
| bincode | 1.3.3 | [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141) | deno_core → rustyscript (sandbox) |
| number_prefix | 0.4.0 | [RUSTSEC-2025-0119](https://rustsec.org/advisories/RUSTSEC-2025-0119) | indicatif → tokenizers → model2vec-rs (semantic) |
| paste | 1.0.15 | [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436) | v8 → deno_core → rustyscript (sandbox) |
| unic-char-property | 0.9.0 | [RUSTSEC-2025-0081](https://rustsec.org/advisories/RUSTSEC-2025-0081) | urlpattern → deno_url → rustyscript (sandbox) |
| unic-char-range | 0.9.0 | [RUSTSEC-2025-0075](https://rustsec.org/advisories/RUSTSEC-2025-0075) | unic-ucd-ident → urlpattern → deno_url → rustyscript |
| unic-common | 0.9.0 | [RUSTSEC-2025-0080](https://rustsec.org/advisories/RUSTSEC-2025-0080) | unic-ucd-version → unic-ucd-ident → ... → rustyscript |
| unic-ucd-ident | 0.9.0 | [RUSTSEC-2025-0100](https://rustsec.org/advisories/RUSTSEC-2025-0100) | urlpattern → deno_url → rustyscript |
| unic-ucd-version | 0.9.0 | [RUSTSEC-2025-0098](https://rustsec.org/advisories/RUSTSEC-2025-0098) | unic-ucd-ident → urlpattern → deno_url → rustyscript |

## New Warning (not yet in ignore list)

| Crate | Version | Advisory | Status |
|---|---|---|---|
| rand | 0.8.5 + 0.9.2 | [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097) — Unsound with custom logger using `rand::rng()` | **Needs ignore** — transitive via tokenizers, quinn, bitwarden, deno_crypto |

## Ignored in CI

All of the above are ignored via `--ignore` flags in `.github/workflows/security.yml` because they are transitive dependencies through `rustyscript` (V8 sandbox) or `bitwarden` (secret resolution) that we cannot upgrade from gatemini.

## Process

- **CI:** `cargo audit` runs on every PR and push to `main` via `.github/workflows/ci.yml`.
- **Scheduled:** Daily audit runs via `.github/workflows/security.yml` at 06:00 UTC.
- **New advisories:** When a new RustSec advisory appears, add it to the ignore list after verification, or fix it if it's in a direct dependency.