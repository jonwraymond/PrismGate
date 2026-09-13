# Security allowlist patch for RUSTSEC-2026-0258

## Problem
`cargo audit` on CI fails because `h2 v0.3.27` (transitive via `warp v0.3`) has
advisory **RUSTSEC-2026-0258** (unbounded empty DATA frames).

`deny.toml` already has an allowlist entry, but `cargo audit` uses its own
`--ignore` flags and does not read `deny.toml`.

## Fix (two files)

### 1. deny.toml — already applied locally
Add to the `[advisories] ignore` list:
```toml
"RUSTSEC-2026-0258", # h2 unbounded empty DATA frames (transitive via warp/hyper)
```

### 2. .github/workflows/security.yml — needs to be pushed
Add `--ignore RUSTSEC-2026-0258` to the `cargo audit` command block.

**Diff for `security.yml`:**
```diff
           --ignore RUSTSEC-2026-0044
           --ignore RUSTSEC-2026-0048
           --ignore RUSTSEC-2026-0049
+          --ignore RUSTSEC-2026-0258

   deny:
```

## Apply locally
```bash
cd /home/hermes-exec/workspace/PrismGate
sed -i '30a\          --ignore RUSTSEC-2026-0258' .github/workflows/security.yml
git add .github/workflows/security.yml
git commit -m "ci: ignore RUSTSEC-2026-0258 in cargo audit (transitive via warp/hyper)"
git push origin main
```

## Alternative: grant workflow scope to the gh CLI token
If you want me to push this directly, refresh the token with `workflow` scope:
```bash
gh auth refresh -h github.com -s repo -s workflow
```
Then re-run: `git push origin main`
