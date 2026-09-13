# Device Pairing vs RFC 8628 for the Pipedream Menu-Bar Daemon

Scope: design the **first-connect** handshake between the local Pipedream
LSUIElement daemon (no Dock icon, no terminal) and the remote orchestrator
(cloud server in this workspace). The menu-bar UI is its only user-facing
surface. The goal is one user gesture: prove the daemon belongs to this user
and this project allowlist, then keep refreshing silently in the background.

## 1. RFC 8628 device authorization grant — end-to-end shape

RFC 8628 defines a four-step flow designed for input-constrained devices that
already have outbound HTTPS but no usable browser
(https://datatracker.ietf.org/doc/html/rfc8628, §1 "Introduction", §3
"Protocol"). It assumes the device can display a short code and a URL the user
visits on a *different* device. The Pipedream daemon is not that — it is on
the same Mac as the menu-bar app, so we adapt RFC 8628 and then replace it
with a tighter pairing-token alternative in §2.

### 1.1 Protocol sequence (verbatim shape from §3.1–§3.5)

```
┌────────────┐                                ┌────────────────────┐
│  Device    │  POST /device_authorization    │   Authorization    │
│  Client    │────────────────────────────────▶│   Server (cloud)   │
│ (daemon)   │  client_id=…&scope=…           │                    │
│            │◀────────────────────────────────│                    │
│            │  device_code, user_code,        │                    │
│            │  verification_uri, expires_in,  │                    │
│            │  interval (default 5s)          │                    │
│            │                                 │                    │
│  display:  │      user_code + URI            │                    │
│            │                                 │                    │
│            │  POST /token (every ≥interval)  │                    │
│            │  grant_type=urn:ietf:params:    │                    │
│            │   oauth:grant-type:device_code  │                    │
│            │  &device_code=…&client_id=…     │                    │
│            │                                 │                    │
│            │◀────── {error: authorization_   │                    │
│            │         pending}                │                    │
│            │◀────── {error: slow_down}       │  interval += 5s    │
│            │◀────── {error: access_denied}   │  STOP polling      │
│            │◀────── {error: expired_token}   │  restart only on   │
│            │                                  │  user gesture      │
│            │◀────── {access_token,            │                    │
│            │          refresh_token,         │                    │
│            │          token_type: Bearer,    │                    │
│            │          scope, expires_in}     │                    │
└────────────┘                                └────────────────────┘

End-user (separate device)
   open verification_uri on phone/laptop browser
   enter user_code (or scan verification_uri_complete QR)
   approve / deny on the consent screen
```

### 1.2 JSON shapes (mapping RFC parameters to our wire format)

The server in this workspace speaks JSON over WebSocket; we keep the field
names RFC-faithful so the same code paths can be exercised by an
RFC-8628-conformant IdP later if we ever federate.

**Request — `pair.begin`:**
```json
{
  "kind": "pair.begin",
  "v": 1,
  "client_id": "mac.pipedream.local",
  "scope": "projects:alice/pipedream",
  "device": {
    "model": "Mac15,3",
    "os": "macOS 14.5",
    "app_version": "0.4.2",
    "binary_sig": "apple-developer-team-id:ABCDE12345"
  }
}
```

**Response — `pair.challenge` (mirrors §3.2):**
```json
{
  "kind": "pair.challenge",
  "v": 1,
  "device_code": "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS",
  "user_code": "WDJB-MJHT",
  "verification_uri": "https://app.example.com/device",
  "verification_uri_complete": "https://app.example.com/device?user_code=WDJB-MJHT",
  "expires_in": 1800,
  "interval": 5
}
```

**Polling — `pair.poll`:** client re-sends `device_code` every `interval`
seconds; server replies with one of:

```json
// §3.5 success — same as RFC 6749 §5.1
{
  "kind": "pair.poll",
  "state": "active",
  "access_token": "eyJraWQiOi…",
  "refresh_token": "GEvxJ_qH1…",
  "token_type": "Bearer",
  "scope": "projects:alice/pipedream",
  "expires_in": 3600,
  "device_token_id": "dt_01HV…",
  "projects": ["alice/pipedream"]
}

// §3.5 error — keep polling but obey interval
{ "kind": "pair.poll", "error": "authorization_pending" }
{ "kind": "pair.poll", "error": "slow_down" }   // interval += 5; honor forever after

// §3.5 error — stop polling, surface to menu-bar
{ "kind": "pair.poll", "error": "access_denied" }
{ "kind": "pair.poll", "error": "expired_token" }
```

### 1.3 Polling algorithm (must match §3.5 + §3.5 connection-timeout rules)

Client-side state machine for the polling loop:

```
let interval = response.interval.unwrap_or(5);   // RFC default = 5
loop {
    match poll() {
        active                  => store tokens, exit
        authorization_pending   => sleep(interval); continue
        slow_down               => interval += 5; sleep(interval); continue
        access_denied           => show "denied" in menu-bar; exit
        expired_token           => show "code expired — click to retry" in menu-bar; exit
        transport timeout/EOF   => sleep(max(interval, prev*2)); continue   // RFC §3.5 exponential backoff REQUIRED on transport failure
        other                   => show generic error; exit
    }
}
```

The RFC §3.5 "On encountering a connection timeout, clients MUST unilaterally
reduce their polling frequency … exponential backoff … doubling the polling
interval … is RECOMMENDED" clause is the easy part to forget on a daemon that
never exits. Bake it into a single `PairPoller` struct so there is exactly one
implementation.

### 1.4 Why RFC 8628 is the wrong default here

RFC 8628 is designed for TVs and printers: a device the user has *never
authenticated* before, in a different physical room, with no other way to type
credentials into it. Our situation is the opposite:

- The menu-bar app **already has a UI on the same machine** — there is no
  reason to ask the user to pick up a phone and type `WDJB-MJHT`.
- The orchestrator is a **private deployment**, not an open IdP. The
  user-code-then-browser-redirect dance adds friction without buying any
  phishing defense we don't already have (we control both endpoints).
- We want the device token to be **bound to the local Mac keychain identity**
  (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`), not to whatever
  browser session happens to approve it. The §5.4 "Remote Phishing" mitigation
  (ask user to confirm the code matches what's on the device) is solving a
  problem we don't have: there is no email, no remote attacker, no QR.

So we keep §3.5's *error vocabulary* (`authorization_pending`, `slow_down`,
`access_denied`, `expired_token`) as the cleanest possible wire shape, but we
collapse the two-step "display URL + user goes there + types code" into one
in-app paste.

## 2. Alternative: short-lived pairing token pasted into the menu-bar

This is what we ship. Same wire vocabulary, single in-app gesture.

### 2.1 Sequence

```
┌─────────────────┐                ┌────────────────┐                ┌─────────────────────┐
│ Pipedream       │  pair.begin    │ Orchestrator   │  user creates   │ Pipedream web app / │
│ daemon (no UI)  │───────────────▶│ (cloud)        │  token in       │ CLI one-shot:       │
│ + menu-bar app  │◀───────────────│                │  dashboard      │   pair-token gen    │
│                 │  pair.challenge│                │  ──────────────▶│                     │
│                 │  + short_code  │                │                 │  returns:           │
│ user pastes     │                │                │                 │    6-char code      │
│ short_code into │                │                │                 │    10-min TTL       │
│ menu-bar "Pair" │                │                │                 │    one-shot         │
│ text field      │                │                │                 │    bound to allow-  │
│                 │  pair.confirm  │                │                 │    list + device    │
│                 │  {short_code,  │                │                 │    pubkey           │
│                 │   pubkey}      │                │                 │                     │
│                 │                │                │                 │                     │
│                 │◀───────────────│                │                 │                     │
│                 │ pair.poll (1×) │                │                 │                     │
│                 │  active +      │                │                 │                     │
│                 │  tokens        │                │                 │                     │
└─────────────────┘                └────────────────┘                └─────────────────────┘
```

The pairing code is **never** sent to a cloud UI; the cloud only ever sees it
when the daemon posts it back to the orchestrator over the existing TLS
WebSocket. The menu-bar app generates it; the user copies it locally from a
companion "Create pairing code" surface (CLI: `pipedream pair new`; web app:
"Pipedream > Devices > Add device"). The "verification URI" that RFC 8628
requires the daemon to show is **not shown to the user** in this design — see
§3.4.

### 2.2 JSON shapes

**`pair.begin`** — same as §1.2 but `client_id` carries a stable device
identifier derived from the Mac keychain (see §4), so the orchestrator can
de-duplicate re-pairings of the same machine.

**`pair.challenge`:**
```json
{
  "kind": "pair.challenge",
  "v": 1,
  "challenge_id": "ch_01HV…",
  "expected_code_length": 6,
  "ttl_seconds": 600,
  "interval": 1,
  "projects": ["alice/pipedream"]
}
```
The orchestrator does **not** return `user_code` or `verification_uri` to the
daemon. The code lives only on the side that issued it (CLI / web app) and on
the orchestrator's challenge record. The daemon's job is to receive it back
from the user.

**`pair.confirm`:**
```json
{
  "kind": "pair.confirm",
  "v": 1,
  "challenge_id": "ch_01HV…",
  "code": "WDJB-MJHT",
  "device_pubkey": "ed25519:MCowBQYDK2VwAyEAR8z…",
  "device_label": "Alice's MacBook Pro",
  "attestation": {
    "apple_team_id": "ABCDE12345",
    "signing_id": "com.pipedream.client",
    "nonce": "…"
  }
}
```

**`pair.poll`** on the next tick — `state: "active"` with the device token
bundle, identical shape to §1.2. The orchestrator returns `error: "expired_token"`
or `error: "access_denied"` if the user typed the wrong code three times
(server-side rate limit, see §3.2) or the challenge aged out.

### 2.3 Why this is strictly better for a private deployment

- **One user gesture instead of three.** Open menu-bar → paste code → done.
  No browser, no second device, no QR.
- **No cloud-paste URL.** The orchestrator returns a `challenge_id`, not a URL
  the user could mistakenly paste into Twitter.
- **Cryptographically bound.** The `device_pubkey` field lets the orchestrator
  bind the issued device token to a key that lives only in the Mac keychain.
  Even if someone leaks the device token, they cannot use it from a different
  machine because subsequent `pair.refresh` calls must sign a challenge with
  the bound key (see §5).
- **Re-pairs are silent.** When the device token rotates, the menu-bar never
  asks the user anything; refresh happens entirely from the keychain. Only
  *first* install needs the paste.

### 2.4 Hybrid: also expose the RFC 8628 shape for federated IdPs

If/when we federate to a third-party identity provider (Auth0, WorkOS, an
enterprise OIDC), the menu-bar can fall back to the §1 RFC 8628 flow by
showing `verification_uri` and `user_code` returned in `pair.challenge`. The
state machine and polling algorithm are identical; only the response field set
gains `user_code` + `verification_uri_complete`. This is why we kept the field
names RFC-faithful.

## 3. Threat model

### 3.1 Replay resistance

| Attack | Mitigation |
|---|---|
| Attacker captures `pair.confirm` from network and replays it | `challenge_id` is a server-issued random 128-bit nonce (RFC 4122 v4); single-use; invalidated the moment it matches a code. Server deletes the challenge record on first match. |
| Attacker captures device token and replays it | Token is bound to `device_pubkey`; the orchestrator challenges every reconnect with a nonce that must be signed by the bound key (mTLS-equivalent at the app layer). Replays without the key fail. |
| Attacker captures refresh token | Refresh token is bound to `device_token_id`; rotating it invalidates the previous refresh token; `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` prevents extraction from a locked or stolen-at-rest Mac (see §4). |
| Attacker brute-forces a 6-char code | Server-side rate limit: 3 wrong codes per `challenge_id`, then the challenge is killed and the menu-bar shows "expired". Combined with the 10-minute TTL this matches the entropy guidance in RFC 8628 §5.1: a base-32 6-char code is ~30 bits; 3 attempts × 600 s ceiling gives well under 2⁻³² brute-force success probability. |

### 3.2 Clock skew

- **Challenge TTL.** Orchestrator signs `ttl_seconds` and `issued_at`; the
  daemon rejects any `pair.confirm` whose `challenge_id` the server says is
  expired (canonical source = server), not its own local clock. The Mac's
  wall clock is never trusted for auth-window arithmetic.
- **Refresh window.** Refresh tokens are issued with `expires_in` and the
  daemon refreshes at `min(60s, expires_in/4)` before expiry. If the network
  is partitioned past the refresh window, the daemon transitions to
  `revoked` (state machine §6) — it does not silently re-pair, and does not
  silently extend sessions using a stale clock.
- **Pairing code generation** is done locally on the device that issues
  the code (CLI / web), so the orchestrator only needs to be approximately
  right (NTP-synced) — it issues `issued_at` itself and applies the TTL.

### 3.3 Local-only first-use confirmation

On the *first* successful pairing, the menu-bar shows a system notification
that requires the user to click **"Trust this device"** before the daemon
stores the token. This:

1. Forces the user to be physically present at the Mac on first install.
2. Catches the case where a user accidentally pastes a code they generated
   on a public terminal (the notification names the device that generated
   the code, by label).
3. Triggers the macOS Keychain access prompt (`SecItemAdd` with
   `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` shows a UI prompt the
   first time it runs and a different "always allow" prompt on subsequent
   runs).

The "Trust this device" prompt is rendered by the menu-bar app, not by the
daemon — the daemon cannot show UI without waking up a window, which an
LSUIElement is supposed to avoid.

### 3.4 Don't trust any URL shown to the user (anti cloud-paste)

The §1 RFC 8628 flow surfaces `verification_uri` to the user. In our §2
design we **deliberately omit it** from the daemon's response. Rationale:

- A user reading a code from a terminal or web UI, then being told "go to
  https://…" creates a perfect pretext for a phisher to put a fake URL on
  the screen. RFC 8628 §5.4 calls this out explicitly.
- Our orchestrator is **not** an open IdP. The user does not need to be
  educated about which URL is the right one — there is exactly one URL the
  orchestrator serves, the menu-bar app already knows it (it's the same
  WebSocket origin), and the user has no reason to type it.
- If we ever federate to a third-party IdP (§2.4), we *will* show
  `verification_uri`, but only after a UI affordance in the menu-bar
  explicitly says "you are about to authenticate with **Auth0**" with the
  domain rendered as a non-clickable label, not as a hyperlink. The user
  must type it manually if they want to proceed — no `verification_uri_complete`
  QR.

Rule of thumb: if the menu-bar shows a URL that ends in a domain the user
doesn't recognise, that's a bug.

### 3.5 Other threats worth naming

- **Session spying (RFC 8628 §5.5).** Not applicable — there is no screen
  the attacker can shoulder-surf to learn a code from.
- **Device trustworthiness (RFC 8628 §5.3).** Mitigated by binding the
  device token to the Apple Developer Team ID + signing identifier
  (`apple_team_id` + `signing_id` in `pair.confirm.attestation`). A
  repackaged binary with a different team ID gets a different
  `device_token_id` family and cannot reuse the issued token.
- **Non-confidential client (RFC 8628 §5.6).** Acknowledged: the daemon
  binary is reproducible and the `device_pubkey` is sent in the clear.
  That's fine because the pubkey is not secret; what is secret is the
  device token, which never leaves the keychain.
- **Slowloris / polling DoS (RFC 8628 §3.5 implicit).** Server caps the
  number of active `challenge_id` per `(client_id, source_ip)` and per
  team. Daemon's exponential backoff on transport timeout (§1.3) ensures a
  bad network can't be turned into a tight loop.

## 4. Token storage on macOS

We use `kSecClassGenericPassword` with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
The daemon binary is signed with the Apple Developer Team ID; the menu-bar
helper (the UI process that owns the keychain entry) is signed with the
same identity. They share via a **Keychain Access Group** that uses the
team ID as its prefix.

### 4.1 Attribute set

```swift
let attributes: [String: Any] = [
    kSecClass:            kSecClassGenericPassword,
    kSecAttrService:      "com.pipedream.client.device-token",
    kSecAttrAccount:      deviceTokenId,            // e.g. "dt_01HV…"
    kSecAttrAccessGroup:  "ABCDE12345.com.pipedream.shared",
    kSecAttrAccessible:   kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    kSecAttrSynchronizable: false,                  // explicit: not iCloud
    kSecAttrLabel:        "Pipedream device token (\(deviceLabel))",
    kSecAttrDescription:  "Bearer token for Pipedream orchestrator",
    kSecValueData:        deviceTokenData           // JSON {access, refresh, device_pubkey, projects, expires_at}
]
```

Reference:
- `kSecClassGenericPassword` — https://developer.apple.com/documentation/security/ksecclassgenericpassword
- `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` —
  https://developer.apple.com/documentation/security/ksecattraccessibleafterfirstunlockthisdeviceonly
  ("The data in the keychain item cannot be accessed after a restart until
  the device has been unlocked once by the user … This is recommended for
  items that need to be accessed by background applications. Items with this
  attribute do not migrate to a new device.")
- `kSecAttrSynchronizable` —
  https://developer.apple.com/documentation/security/ksecattrsynchronizable
  ("Items stored or obtained using the `kSecAttrSynchronizable` key may not
  also specify a `kSecAttrAccessible` value that is incompatible with
  syncing (namely, those whose names end with `ThisDeviceOnly`).")

The last bullet is the critical gotcha: **`*ThisDeviceOnly` and
`kSecAttrSynchronizable=true` are mutually exclusive**. We want both
"survives reboot without user re-login" *and* "never leaves this Mac" — so
we pick `AfterFirstUnlockThisDeviceOnly` and explicitly set
`kSecAttrSynchronizable=false`. iCloud Keychain will never see this token,
which is what we want.

### 4.2 Why `AfterFirstUnlockThisDeviceOnly` (not `WhenUnlocked`)

| Constant | Survives background daemon? | Locked after reboot? | iCloud sync? |
|---|---|---|---|
| `WhenUnlocked` | ❌ daemon can't read while screen locked | ✅ | optional |
| **`AfterFirstUnlockThisDeviceOnly`** | ✅ daemon can read once unlocked post-boot | ✅ until next reboot | ❌ |
| `AfterFirstUnlock` | ✅ | ✅ | optional |
| `WhenUnlockedThisDeviceOnly` | ❌ | ✅ | ❌ |

The daemon is an `LSUIElement` — it runs in the background, with no UI, and
needs the token to refresh tokens, send heartbeats, and reconnect while the
user's screen is locked. `WhenUnlocked*` would break every screen lock. The
`*ThisDeviceOnly` suffix blocks iCloud Keychain and Migration Assistant
exfiltration, which is the part we actually care about.

### 4.3 Sharing across the signed binary (daemon ⇄ menu-bar helper)

The menu-bar app is the UI surface; the daemon is the always-on process.
Both need read/write access to the same keychain item. Apple's rule
(https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps):

> "When you create a new item with the `SecItemAdd(_:_:)` method, you can
> specify a group in the add attributes using the `kSecAttrAccessGroup` key
> … Two or more apps that are in the same access group can share keychain
> items."

Implementation:

1. Both targets have the **Keychain Sharing** capability enabled in Xcode
   (Signing & Capabilities → + Capability → Keychain Sharing).
2. Both list the same group string:
   `$(AppIdentifierPrefix)com.pipedream.shared` — Xcode expands
   `$(AppIdentifierPrefix)` to the Team ID at signing time, producing
   `ABCDE12345.com.pipedream.shared`.
3. Both list this group first in the `keychain-access-groups` entitlement
   array, so it becomes the **default access group** for each process. From
   the Apple docs:
   > "the first keychain access group, if any, that you specify in the
   > corresponding capability becomes the app's default access group."
4. The `application-identifier` entitlement (always present, team ID +
   bundle ID) is the second entry — the daemon's *private* group, which the
   menu-bar helper cannot see.
5. Code-signing both binaries with the same Developer ID (`Developer ID
   Application: Pipedream, Inc. (ABCDE12345)`) means macOS will accept the
   entitlement on both sides; an attacker who replaces either binary with a
   different signing identity loses access.

Reference:
- Sharing access — https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps
- `kSecAttrAccessGroup` — https://developer.apple.com/documentation/security/ksecattraccessgroup
- Entitlement key reference — https://developer.apple.com/documentation/bundleresources/entitlements/keychain-access-groups

### 4.4 What we deliberately don't use

- **`kSecAttrSynchronizable` / iCloud Keychain.** We want a stolen Mac to
  mean a stolen token, not a token that follows the user to their new
  machine.
- **`kSecUseDataProtectionKeychain`.** We don't need iOS-style sandbox
  semantics; the file-based macOS keychain is fine, and Apple's `TN3137`
  documents the gotchas of mixing the two on macOS.
- **File-based storage** (`~/.pipedream/token.json`). Even with `0600`,
  this leaks via Time Machine, crash dumps, `ls` on a shared host, and any
  process running as the user. Keychain is the right tool.

## 5. Token refresh in the background

The daemon runs forever; the user shouldn't have to think about it.

### 5.1 Scheduler

```
struct RefreshScheduler {
    access: AccessToken,
    refresh: RefreshToken,
    device_token_id: String,
    device_privkey: ed25519::SecretKey,    // from keychain, never persisted
    next_refresh_at: Instant,
}

impl RefreshScheduler {
    fn tick(&mut self) -> Action {
        let now = Instant::now();
        if now + Duration::from_secs(60) >= self.access.expires_at {
            // refresh now, with safety margin for slow networks
            Action::Refresh
        } else if now >= self.next_refresh_at {
            Action::ReconnectOrHeartbeat
        } else {
            Action::SleepUntil(self.next_refresh_at)
        }
    }
}
```

- Refresh 60 s before `access.expires_at` (configurable). The 60 s margin
  absorbs a slow reconnect without the daemon seeing a 401.
- After refresh, server returns a new `refresh_token` (rotation); the old
  one is invalidated server-side. Standard OAuth 2.0 refresh-token rotation
  (RFC 6749 §6).
- If refresh fails with `invalid_grant` (refresh token revoked, account
  suspended, allowlist changed), the daemon transitions to `revoked` and
  surfaces a "Re-pair required" entry in the menu-bar. It **does not
  auto-re-pair** — the user has to click.

### 5.2 Refresh request shape (signed by the bound key)

Every refresh and reconnect carries a signature over a server-issued nonce,
proving possession of the device private key (the key in the keychain). This
is what makes a stolen device token useless from a different machine.

```json
{
  "kind": "refresh",
  "v": 1,
  "device_token_id": "dt_01HV…",
  "refresh_token": "GEvxJ_qH1…",
  "server_nonce": "n_01HV…",
  "signature": "ed25519:MCowBQYDK2VwAyEAGZ6…",
  "client_time_ms": 1715000000000
}
// server_nonce + "|" + client_time_ms + "|" + device_token_id
// is the message that gets signed with the device_privkey.
```

Server checks:
- `server_nonce` was issued by this server within the last 60 s and not
  reused.
- `signature` is valid for the `device_pubkey` bound at pairing time.
- `client_time_ms` is within ±5 minutes of server time (clock skew window).
- `refresh_token` matches the bound device token and has not been rotated.

### 5.3 Heartbeat (so the server knows the daemon is alive)

```
every 30 s while connection is open:
    send { kind: "ping", v: 1, ts: now_ms }
    expect { kind: "pong", v: 1, ts: server_now_ms } within 10 s
    if no pong: reconnect with exponential backoff (1s, 2s, 4s, … cap 60s)
```

Heartbeats don't carry the access token — they're identified by the
underlying WebSocket session, which was authenticated at handshake. This
keeps the wire bytes small (the daemon runs on battery on a laptop).

### 5.4 Background lifecycle

The daemon is an `LSUIElement` (no Dock icon). It is launched by
`launchd` via a `LaunchAgent` plist with `RunAtLoad=true`,
`KeepAlive=true`. Because we picked `AfterFirstUnlockThisDeviceOnly`, the
daemon cannot read the token until the user has unlocked the Mac once
post-boot — which is fine, because:

1. `KeepAlive` will retry the launch until it succeeds.
2. On macOS, the first user login *is* the first unlock, so this is
   effectively "wait for the user to log in".
3. While waiting, the daemon holds an open WebSocket with no credentials
   and queues operations in memory. When the user unlocks, the keychain
   unblocks, the device token loads, and the queued operations replay
   (with a server-side idempotency key per operation so duplicates are
   harmless).

This is the entire reason the §4 attribute choice matters: pick
`WhenUnlocked*` and the daemon breaks every time the screen locks; pick
`Always*` (deprecated) and the token is exposed before the user logs in.

## 6. State machine

```
                ┌─────────────────────────────────────────────┐
                │                                             │
                ▼                                             │
        ┌─────────────┐  pair.begin succeeds  ┌─────────────┐ │
   ────▶│  anonymous  │─────────────────────▶│  pending    │ │
        └─────────────┘                       └─────────────┘ │
                ▲                                     │       │
       pair error│                                     │       │
                │                                     ▼       │
        ┌─────────────┐                       ┌─────────────┐ │
        │  revoked    │◀───── refresh fails ──│   paired    │◀┘
        └─────────────┘                       └─────────────┘
                ▲                                     │
                │                                     │ user pastes code,
                │                                     │ server returns tokens
                │                                     ▼
                │                             ┌─────────────┐
                │       server-side revoke    │   active    │
                └─────────────────────────────│             │
                                              └─────────────┘
                                                     │
                                                     │ user clicks
                                                     │ "Revoke this device"
                                                     ▼
                                              ┌─────────────┐
                                              │  revoked    │
                                              └─────────────┘
```

### 6.1 States

| State | Meaning | Visible in menu-bar |
|---|---|---|
| `anonymous` | Daemon has no token; either never paired or freshly launched after `revoked` data wipe. | "Not paired" |
| `pending` | `pair.begin` succeeded, waiting for the user to paste the code. | "Pairing code requested — paste from Pipedream dashboard" |
| `paired` | Server returned tokens, daemon is about to do first-use confirmation and write to keychain. | "Confirming…" (transient) |
| `active` | Token is in the keychain; WebSocket is authenticated; refresh scheduler is running. | Status icon + project count, no text |
| `revoked` | Token was invalidated (refresh failed, user revoked, allowlist changed). Daemon stops the WebSocket, surfaces a notification, and waits for user to re-pair. | "Re-pair required" with action button |

### 6.2 Transitions

| From | Event | To | Side effects |
|---|---|---|---|
| (boot) | daemon starts | `anonymous` | load keychain; if `device_token_id` exists, jump to `active` and skip pairing |
| `anonymous` | user clicks "Pair" | `pending` | `pair.begin` → `pair.challenge`; show "paste code" UI |
| `pending` | user pastes code | `paired` | `pair.confirm` → `pair.poll` returns `active` |
| `pending` | `pair.poll` returns `access_denied` | `anonymous` | clear challenge; show "denied" notification |
| `pending` | `pair.poll` returns `expired_token` | `anonymous` | show "code expired" notification |
| `paired` | server returns `active` + tokens | `active` | first-use confirmation prompt → `SecItemAdd` → start refresh scheduler |
| `paired` | server returns error | `anonymous` | show error, do not store token |
| `active` | refresh succeeds | `active` | update keychain item with new tokens |
| `active` | refresh returns `invalid_grant` | `revoked` | delete keychain item; show "Re-pair required" notification |
| `active` | user clicks "Sign out" in menu-bar | `revoked` | server-side revoke + local keychain delete |
| `active` | server sends `revoke` push | `revoked` | delete keychain item; show notification |
| `revoked` | user clicks "Pair again" | `pending` | new `pair.begin` |

The state machine is implemented as a single `enum DeviceState` plus an
explicit `transition(event) -> Result<DeviceState, TransitionError>` method;
no `match` blocks scattered around the codebase. Every transition logs a
structured event with `device_token_id` (if any), `from`, `to`, `event`,
`reason` — these logs are what the `prismgate://health` resource surfaces.

## 7. References

### Standards
- RFC 6749 — The OAuth 2.0 Authorization Framework
  https://datatracker.ietf.org/doc/html/rfc6749
- RFC 8628 — OAuth 2.0 Device Authorization Grant (Proposed Standard, Aug 2019)
  https://datatracker.ietf.org/doc/html/rfc8628
- RFC 8252 — OAuth 2.0 for Native Apps
  https://datatracker.ietf.org/doc/html/rfc8252
- RFC 7525 — Recommendations for Secure Use of TLS and DTLS (BCP 195)
- RFC 8414 — OAuth 2.0 Authorization Server Metadata
- RFC 8259 — JSON

### Apple Security / Keychain Services
- Keychain services overview —
  https://developer.apple.com/documentation/security/keychain-services
- `kSecClassGenericPassword` —
  https://developer.apple.com/documentation/security/ksecclassgenericpassword
- `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` —
  https://developer.apple.com/documentation/security/ksecattraccessibleafterfirstunlockthisdeviceonly
- `kSecAttrSynchronizable` —
  https://developer.apple.com/documentation/security/ksecattrsynchronizable
- `kSecAttrAccessGroup` —
  https://developer.apple.com/documentation/security/ksecattraccessgroup
- Sharing access to keychain items among a collection of apps —
  https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps
- Keychain Access Groups Entitlement —
  https://developer.apple.com/documentation/bundleresources/entitlements/keychain-access-groups
- Configuring keychain sharing (Xcode) —
  https://developer.apple.com/documentation/xcode/configuring-keychain-sharing
- TN3137: On Mac keychain APIs and implementations —
  https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains

### Internal
- `docs/architecture.md` — Pipedream runtime model (proxy/daemon/socket)
- `src/ipc/socket.rs` — M1 local socket framing (this document covers the
  separate *remote* orchestrator handshake)
- `docs/OAUTH.md` — backend-side OAuth (different scope: backend HTTP
  auth, not the device-to-orchestrator handshake designed here)
