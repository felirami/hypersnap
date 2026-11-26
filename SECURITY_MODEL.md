# Hypersnap threat model

Repository: `felirami/hypersnap` (fork of `farcasterorg/hypersnap`), whole repository in scope, including `vendor/malachite` and `vendor/libp2p-scatter`. No separate infrastructure repository: the deployment is described by `docker-compose*.yml`, `Dockerfile` and `scripts/hypersnap*.sh` in this repository. Nodes are treated as directly exposed to the internet (not behind the opencaster gateway Worker).

## System overview and trust boundaries

Hypersnap is a Rust Farcaster node: a sharded, BFT-consensus (Malachite/Tendermint-style) chain. Shard 0 produces `Block`s; data shards 1..N produce `ShardChunk`s of user messages. Nodes are **validators** (consensus participants, holding a consensus private key in `config.toml`) or **read nodes** (followers that sync blocks and serve APIs). Both are in scope.

Trust boundaries:
1. Internet -> gRPC `rpc_address` (default `0.0.0.0:3383`) and HTTP `http_address` (`0.0.0.0:3381`), published by the compose files.
2. Internet / p2p peers -> libp2p gossip over QUIC (`3382/udp`): gossipsub topics, request-response (proposal/vote recovery, read sync), identify/kad/mDNS discovery.
3. Validator set -> consensus messages (proposals, votes, commits) decoded by `src/consensus/malachite/snapchain_codec.rs` and handled by `vendor/malachite`.
4. Node -> outbound: L1/Base RPC (`l1_rpc_url`, `base_onchain_events.rpc_url`), fname server (`fnames.url`), snapshot download (`snapshot.snapshot_download_url`) and S3/R2 upload, webhook and mini app notification target URLs supplied by users.
5. Operator -> local config (`config.toml`, `SNAPCHAIN_*` env), RocksDB under `.rocks`: trusted.

## Components

| Component | Code | Notes |
|---|---|---|
| **Consensus** | `src/consensus/`, `src/consensus/malachite/`, `vendor/malachite/`, `src/network/proposal_recovery.rs`, `src/network/vote_recovery.rs` | Proposer/validator, codec, Malachite engine/WAL/sync |
| **P2P Network** | `src/network/gossip.rs`, `src/network/discovery.rs`, `src/network/mesh/`, `src/network/topic_schedule.rs`, `vendor/libp2p-scatter/` | Gossipsub, peer discovery, mesh view/crawl |
| **Mempool & Validation** | `src/mempool/`, `src/core/validations/`, `src/core/message.rs`, `src/version/` | Message admission, signatures, rate limits, L1 gate, engine-version gates |
| **Storage & Engine** | `src/storage/` (engines, stores, trie, db, snapshot), `src/jobs/`, `src/bootstrap/`, `src/hyper/` | State transitions, merkle trie, pruning, snapshot download/upload, replication bootstrap |
| **Public RPC** | `src/network/server.rs`, `src/network/http_server.rs`, `src/network/rpc_extensions.rs`, `src/network/replication/`, `src/network/admin_server.rs` | gRPC HubService, `/v1/*` HTTP gateway, replication server, admin gRPC |
| **v2 API** | `src/api/` (http.rs, indexers, search, feeds, webhooks, notifications, ssrf.rs) | Neynar-compatible `/v2/farcaster/*`, webhooks, mini app notifications |
| **Connectors** | `src/connectors/onchain_events/`, `src/connectors/fname/`, `src/native_registration.rs` | Onchain registry ingestion, ENS, fname transfers |
| **Deployment** | `Dockerfile`, `docker-compose*.yml`, `scripts/`, `.github/workflows/`, `site/` | Container, bootstrap script, CI, docs site |

## Entry points and who can reach them

- **gRPC HubService** (`src/network/server.rs`): anyone on the internet. `rpc_auth` (comma-separated `user:pass`, HTTP Basic, checked by `authenticate_request` in `rpc_extensions.rs`) is empty by default, so read and submit methods are open. Diagnostic/mesh endpoints are gated by `admin_rpc_auth`, open when empty.
- **Admin gRPC** (`src/network/admin_server.rs`): mounted only when `admin_rpc_auth` is non-empty (`main.rs`, `admin_service.enabled()`). `submit_on_chain_event` / `submit_user_name_proof` additionally require Devnet/None network (`allow_debug`). `upload_snapshot`, `run_onchain_events_migration` call `authenticate_request`.
- **HTTP `/v1/*`** (`src/network/http_server.rs`): anyone; includes `POST /v1/submitMessage`, `/v1/submitBulkMessages`, `/v1/validateMessage`, `/v1/events`, `/v1/mesh`, `/v1/mesh/ui`.
- **HTTP `/v2/farcaster/*`** (`src/api/http.rs`, when `[api] enabled = true`, as in the mainnet compose): anyone; read endpoints, search, feeds, and write-style endpoints (cast, reaction, follow, signer, user/register). Webhooks and notifications management use EIP-712 custody signatures (`src/api/webhooks/auth.rs`, `X-Hypersnap-*` headers, nonce LRU, `signed_at_window_secs`) or `X-Admin-Api-Key` (`webhooks.admin_api_key`, `notifications.admin_api_key`, compared with `constant_time_eq`).
- **Replication server** (`src/network/replication/replication_server.rs`): `get_shard_snapshot_metadata`, `get_shard_transactions`, served to syncing peers.
- **Gossip** (`src/network/gossip.rs`): any peer; `MAX_GOSSIP_MESSAGE_SIZE` = 10 MB, `validate_messages()` with explicit `MessageAcceptance` per message; decoded as `proto::GossipMessage` and Malachite codec.
- **Outbound responses parsed by the node**: L1/Base RPC logs, fname server JSON, snapshot metadata and tar chunks.

## Assets and data sensitivity

- **Consensus safety and liveness** of the network (no forks, no halts) and correctness of the merkle state root.
- **Validator consensus private key** (in `config.toml`; `cfg.rs` avoids echoing TOML lines on parse errors).
- **Integrity of user data**: only messages signed by an active signer of the FID are accepted; storage/rate limits enforced; onchain events (ID/key/storage/tier registry) applied correctly.
- **Node availability**: public read/API nodes and validators must survive hostile traffic.
- **Operator secrets**: S3/R2 `aws_access_key_id`/`aws_secret_access_key`, `admin_rpc_auth`, `rpc_auth`, webhook/notification `admin_api_key`, webhook signing secrets, AI provider key (`api.ai.api_key_env`).
- **Operator network**: internal services reachable from the node host (SSRF target).
- Farcaster user data is public by design; notification tokens and webhook secrets are not.

## Attacker profiles that matter (maintainer-confirmed)

All of these matter:
1. **Internet client**: unauthenticated access to gRPC, `/v1`, `/v2` and replication endpoints; can craft arbitrary protobuf/JSON, large or many requests.
2. **Malicious gossip peer**: any libp2p peer; crafts gossip messages, request-response payloads, discovery records.
3. **Byzantine validator below 1/3** of voting power: crafts proposals, votes, commits, shard chunks, blocks. Attacks needing at least 1/3 colluding validators are out of scope (BFT assumption).
4. **Farcaster user / FID owner**: signs valid messages, registers webhooks and mini app URLs; tries to bypass limits or affect other FIDs.

## Existing controls

- Ed25519 message signature verification (`validate_signature` in `src/core/validations/message.rs`), signer checks against onchain key registry, per-message-type validators (`cast.rs`, `link.rs`, `reaction.rs`, `verification.rs`, `contract_signature`).
- Per-FID rate limits with `governor` + `moka` (`src/mempool/mempool.rs` `RateLimits`), storage-unit limits, L1 gate (`src/mempool/l1_gate.rs`), engine-version gates (`src/version/`).
- Shard chunk and block validation (`ShardEngine::validate_shard_chunk`, `BlockEngine::validate_block`) and Malachite commit-certificate verification.
- Gossipsub message validation and size cap; libp2p connection limits.
- SSRF guard `assert_safe_url` (`src/api/ssrf.rs`): https only, rejects any resolved private/loopback/link-local/CGNAT/ULA address, applied at registration and delivery (webhooks `handler.rs`/`delivery.rs`, notifications `app_handler.rs`/`webhook_handler.rs`/`sender.rs`). `allow_loopback_targets` is test-only.
- EIP-712 custody auth with nonce replay cache and clock-skew window for webhooks/notifications.
- Snapshot chunk names are checked against path traversal (tests in `src/storage/db/snapshot.rs` reject `../../outside`).
- Compose writes config via env then `unset HYPERSNAP_CONFIG` to keep secrets out of `ps`.
- An unmerged branch `security-hardening` ("Harden public API against DoS and SSRF") exists; the scan covers the default branch.

## Security Invariants

- **Message authenticity (CWE-347)**: every user message entering mempool or a shard chunk passes `validate_message` (Ed25519 over the BLAKE3 hash, signer active for the FID at the message's timestamp per onchain key events). Engine replay of shard chunks re-validates; no path (gRPC, `/v1/submitMessage`, `/v1/submitBulkMessages`, `/v2` write endpoints, gossip mempool messages) may skip it. Validator-originated messages (onchain events, fname proofs) are accepted only from connectors/consensus, never from user RPC on mainnet (`allow_debug` gate).
- **Consensus input validation (CWE-20, CWE-345)**: proposals, votes and commits are accepted only with valid signatures from the current validator set for that height/shard; `WellFormedProposal`/`FullProposal`/`Commits` decoding must reject malformed data without panicking; recovered proposals/votes (`proposal_recovery.rs`, `vote_recovery.rs`, read sync) must be verified exactly like live ones; state roots are recomputed, not trusted.
- **Remote DoS resistance (CWE-400, CWE-770, CWE-248)**: network-reachable decoding and handlers never `unwrap`/`expect`/index on attacker data, never allocate proportional to an attacker-declared length, and bound page sizes (`limit`), batch sizes (`submitBulkMessages`, `/v2 batch/*`), request bodies, scan ranges and concurrency. Gossip relies on the 10 MB cap plus per-message validation; HTTP body reads in `src/api/` (`read_body`, `collect().to_bytes()`) need explicit size limits — gap to verify. Expensive queries (search, feeds, cast-hash fallback probe) are bounded (the probe is single-flight with a 2 s budget).
- **Rate limiting (CWE-770)**: mempool admission enforces per-FID limits and storage limits before expensive work; limits cannot be bypassed by bulk submit or by routing to another shard.
- **Authentication (CWE-287, CWE-306)**: admin gRPC is not mounted unless `admin_rpc_auth` is set; when `rpc_auth`/`admin_rpc_auth` are set, every protected method calls `authenticate_request`. Credentials are compared in constant time (`constant_time_eq` for `X-Admin-Api-Key`; Basic auth comparison in `rpc_extensions.rs` should be too — verify). Empty auth = open is the documented default; whether that is acceptable for public nodes is an open question (below).
- **Authorization / ownership (CWE-639, CWE-862)**: webhook and notification management for an FID requires an EIP-712 signature recovering to that FID's current custody address (`webhooks/auth.rs`), a fresh nonce and `signed_at` in window; an owner can only read/modify its own webhooks/apps.
- **SSRF and egress (CWE-918)**: every outbound request to a user-supplied URL goes through `assert_safe_url(.., config.ssrf_policy())` both at registration and at delivery; redirects must not be followed to unchecked hosts; `SsrfPolicy::AllowLoopback` is never used in production paths. Operator-configured URLs (L1 RPC, fname, snapshot) are trusted but their responses are not.
- **Path traversal and archive handling (CWE-22)**: snapshot metadata chunk names and tar entries are confined to the snapshot directory; search index paths come only from config.
- **Secrets (CWE-532, CWE-200)**: the consensus key, S3 keys, admin keys and webhook secrets never appear in logs, error messages, metrics, `/v1/info`, mesh view or API responses.
- **Webhook payload signing (CWE-345)**: outbound webhook deliveries are HMAC-signed with the webhook secret (`signature_header_name`), with rotation grace (`secret_grace_period_secs`).
- **Injection / XSS (CWE-79)**: `/v1/mesh/ui` (`src/network/mesh/ui.html`) must escape peer-supplied strings (peer IDs, agent versions, addresses) it renders. No SQL; RocksDB keys are built from typed fields.
- **Memory safety**: `unsafe` code (RocksDB, jemalloc, vendored crates) must not be reachable with attacker-controlled sizes.
- **Supply chain**: `rocksdb` pinned to a git rev, vendored Malachite and libp2p-scatter with `PROVENANCE.md`; CI checks out `eth-signature-verifier` at a pinned ref; bootstrap script is `curl | bash` from GitHub `main`.

## Out of scope / accepted risk (maintainer-confirmed)

- Attacks requiring at least 1/3 of validator voting power.
- Local operator config and the node's own database are trusted.
- Nothing else ruled out: all attackers above matter for both validators and public read/API nodes.

## Open questions

- Is open (unauthenticated) gRPC/HTTP read and submit access on public nodes by design? Undecided; findings that depend on it keep the scanner severity.
- Do production operators set `admin_rpc_auth`, `rpc_auth`, and the webhook/notification `admin_api_key`?
- Is `/v1/mesh` / `/v1/mesh/ui` meant to be public on mainnet nodes when `admin_rpc_auth` is empty?
