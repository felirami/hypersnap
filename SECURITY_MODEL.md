# Hypersnap threat model (felirami/hypersnap)

## System overview
Hypersnap is a Rust Farcaster data node: a sharded, BFT-consensus (Malachite) p2p network that stores signed Farcaster user messages, plus a Neynar-compatible read API, webhooks and mini-app notifications. Nodes are deployed by operators from the published Docker image `farcasterorg/hypersnap` using the in-repo `docker-compose*.yml` / `scripts/hypersnap.sh` (no separate IaC). The role of a node (validator vs read node) comes from its consensus key and the schedule in `validators.toml`.

## Trust boundaries & entry points
Every port in the supplied Compose files is published on all interfaces:
- **3381/tcp HTTP** (`src/network/http_server.rs`, `src/api/*`): `/v1/*` hub API, `/v2/farcaster/*` Neynar-compatible reads, `_status/*`, mesh diagnostics (`src/network/mesh`), webhook and mini-app management (`src/api/webhooks`, `src/api/notifications`). Write endpoints under `/v2` are 501 stubs; writes go through gRPC `SubmitMessage`.
- **3383/tcp gRPC** (`src/network/server.rs`, `src/network/admin_server.rs`, `src/network/rpc_extensions.rs`): `SubmitMessage`/bulk, reads, `Subscribe`, plus admin RPCs (retry onchain events/fnames, run migrations, upload snapshot). Auth is HTTP Basic from `rpc_auth` / `admin_rpc_auth`; **both are empty by default, and empty means no auth** (`authenticate_request` returns Ok).
- **3382/udp QUIC gossip** (`src/network/gossip.rs`, `discovery.rs`, `replication/`, `proposal_recovery.rs`, `vote_recovery.rs`): untrusted peers and the validators send blocks, proposals, votes, mempool messages and replication data.
- **Outbound**: Ethereum/OP RPC for onchain events (`src/connectors/onchain_events`), fname server (`src/connectors/fname`), webhook and notification POSTs to third-party URLs (SSRF guard `src/api/ssrf.rs`), the LLM provider (`FARCASTER_LLM_API_KEY`), and R2/S3 snapshot download/upload (`src/storage/db/snapshot.rs`).
- **Shuttle** (`src/shuttle`, `src/bin/shuttle.rs`): a separate process that relays signed user messages between Hypersnap and Snapchain.

## Components
| Component | Code |
|---|---|
| Consensus & Validators | `src/consensus`, `src/mempool`, `src/native_registration.rs`, `validators.toml`, `src/core/validations` |
| P2P Gossip & Replication | `src/network/gossip*.rs`, `discovery.rs`, `topic_schedule.rs`, `proposal_recovery.rs`, `vote_recovery.rs`, `src/network/replication` |
| gRPC API | `src/network/server.rs`, `admin_server.rs`, `rpc_extensions.rs`, `proto/` |
| HTTP API | `src/network/http_server.rs`, `src/network/mesh`, `src/api` (except webhooks/notifications) |
| Webhooks & Notifications | `src/api/webhooks`, `src/api/notifications`, `src/api/ssrf.rs` |
| Storage & State | `src/storage`, `src/hyper`, `src/jobs`, `src/bootstrap` |
| Onchain & Fname Connectors | `src/connectors` |
| Shuttle & Ops Tooling | `src/shuttle`, `src/bin`, `src/perf`, `scripts/`, `Dockerfile`, `docker-compose*.yml`, `.github/workflows`, `site/` |

## Assets
All four classes are **equally critical**:
1. **Consensus safety**: no forks or equivocation, and validator consensus private keys (in config.toml) stay secret. The latest commits fix fork and recovery bugs (block deadlines for native FIDs, proposal recovery, authenticated gossip).
2. **Message integrity**: nobody can forge, replay or censor casts, signers (KEY_ADD/KEY_REMOVE, gasless signers), fname proofs or native registrations (PoW + custody EIP-712, FIDs ≥ 2^32).
3. **Node availability**: the public HTTP/gRPC/gossip surfaces resist DoS (expensive scans like cast-hash fallback probes, search, feeds, social-graph rebuilds).
4. **Operator secrets**: `webhooks.admin_api_key` / `notifications.admin_api_key` (root credentials that bypass EIP-712), `rpc_auth` / `admin_rpc_auth`, the LLM API key, snapshot upload credentials, webhook HMAC signing secrets, and the mini-app notification tokens stored per FID.

## Attackers (none ruled out)
- Anonymous internet clients on the HTTP and gRPC ports.
- Malicious gossip peers and non-validator nodes.
- A malicious or compromised validator, below the BFT threshold.
- Webhook and mini-app owners who supply callback URLs (SSRF, delivery abuse) or try cross-FID access.

## Existing controls (from code)
- Webhook/app management uses EIP-712 custody signatures with a signed-at window, an (fid, nonce) replay LRU and keccak of the raw body. Admin keys are compared in constant time (`notifications/app_handler.rs`, `send_handler.rs`).
- SSRF: https only, DNS-resolved IP blocklist, checked at registration and at delivery (`src/api/ssrf.rs`).
- Message validation in `src/core/validations`, mempool rate limits (`src/mempool`), L1 gate/validator.
- Compose writes config via env and unsets it, so `ps` does not expose the consensus key.

## Out of scope / accepted risk
- None. Everything is in scope, including `src/bin` tooling, shuttle, `site/` and Hyper mode.

## Finding tags (security scans)
- `Component`: one of the component names above.
- `CVE`: for vulnerable dependencies.
- `Network role`: `validator`, `read node` or `all`.
- `Fork-activated`: the activation height or fork the issue is tied to.

## Open questions
- Do production operators set `rpc_auth`/`admin_rpc_auth`, or put the gRPC port behind a firewall?
- Which nodes enable webhooks, notifications or the LLM summary feature with admin keys set?
