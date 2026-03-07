# Architecture

Gatemini runs as a shared daemon with lightweight per-session proxies. The daemon owns backend connections and the live registry; each client process only bridges stdio to the daemon over a Unix socket.

![IPC architecture](diagrams/ipc-architecture.svg){ .diagram-wide }

## Process model

There are three operating modes:

| Mode | Entry point | Purpose |
|------|-------------|---------|
| Proxy | `gatemini` | Default MCP client integration |
| Direct | `gatemini --direct` | Single-session debugging with no daemon |
| Daemon | `gatemini serve` | Foreground daemon process |

Proxy mode is what most clients use. It performs no backend setup and no registry initialization itself. Its job is to connect to the daemon or start it when needed.

## Initialization order

The heavy initialization path is shared between direct mode and daemon mode in `src/main.rs`.

Actual order:

1. load `.env`
2. load config
3. initialize tracing
4. resolve secrets
5. create tracker, registry, and backend manager
6. load cached tools and usage stats
7. register aliases and composite tools
8. spawn background workers

Background workers include:

- backend startup
- health checker
- config watcher
- optional admin API

## Early socket binding

Daemon mode does something important before that initialization completes: it binds the socket early.

![Daemon lifecycle](diagrams/daemon-lifecycle.svg){ .diagram-wide }

That ordering matters because proxies can connect while initialization is still in progress. Bytes sent by the client wait in the kernel socket buffer until the accept loop starts.

## Proxy startup and reconnect behavior

Proxy startup is coordinated with a non-blocking flock on the socket lock file.

![Proxy startup](diagrams/proxy-startup.svg){ .diagram-medium }

The flow is:

1. clean up a stale socket if the PID is dead
2. try a fast-path connect
3. if needed, acquire the lock
4. connect again in case another proxy won the race
5. spawn `gatemini serve`
6. wait for the socket to become connectable
7. bridge stdio to the socket

The proxy also caches the MCP initialize request and initialized notification. If the daemon restarts, the proxy reconnects and replays that handshake so the client session can continue with less disruption.

## Accept loop and shutdown

After initialization, the daemon accepts client connections and creates a fresh `GateminiServer` per client. Those server instances are cheap because they share the real state through `Arc`s.

Shutdown triggers:

- idle timeout with zero active sessions
- `SIGTERM`
- `SIGINT`

Shutdown sequence:

1. stop accepting new clients
2. wait for connected clients to drain, up to `daemon.client_drain_timeout`
3. notify background tasks
4. stop backends and wait for in-flight calls, up to `health.drain_timeout`
5. remove socket, lock, and PID files

## Socket paths

Socket resolution is deterministic so proxies and daemon always look in the same place.

| Platform | Default path |
|----------|--------------|
| Linux with `XDG_RUNTIME_DIR` | `$XDG_RUNTIME_DIR/gatemini.sock` |
| macOS and fallback | `/tmp/gatemini-$UID.sock` |

Sibling paths are also used for:

- lock file
- PID file

## Backend ownership

The daemon owns:

- live backend instances
- the shared tool registry
- recent-call and latency tracking
- config watch state
- optional admin HTTP routes

Clients never talk directly to backend MCP servers. They talk to Gatemini, and Gatemini forwards or orchestrates calls on their behalf.
