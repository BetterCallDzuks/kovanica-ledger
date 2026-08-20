# kovanica-testnet-1

Public BlockDAG testnet. Native token **KVNC** (8 decimals). Engine is the Rust node; explorer and wallet are the same process.

| | |
| --- | --- |
| Explorer | https://explorer.kovanica.online |
| Wallet | https://wallet.kovanica.online |
| Network | `kovanica-testnet-1` |
| Premine | 50 KVNC (founder / actor 1) |
| Subsidy cap | 50 KVNC / block, halves every 1000 blocks |
| Min fee | `max(1, subsidy / 500000)` → 0.0001 KVNC at genesis |
| k | 3 (GHOSTDAG) |
| PoW | on (`KOVANICA_POW=1`); work=1 until difficulty is tightened |
| P2P | TCP `KOVANICA_LISTEN` (default `:9000`) |

Live genesis and tip: `GET /api/head`  
Bootstrap blob: `GET /api/bootstrap`

## Tokenomics

- 1 KVNC = 10^8 atoms.
- New coins only from coinbase (issuance + collected fees to the miner).
- Faucet and empty-block minting are **off** on the public explorer (`KOVANICA_FAUCET=0`, `KOVANICA_OPERATOR=0`).
- Wallet `prepare` / `submit` stays open: you sign in the browser; the node packs mempool into a block when asked.

## Run a node

```bash
cd kovanica-ledger
cargo build --release -p kovanica-node

export KOVANICA_MINE=0
export KOVANICA_FAUCET=0
export KOVANICA_ALLOW_RESET=0
export KOVANICA_OPERATOR=0
export KOVANICA_POW=1
export KOVANICA_LISTEN=0.0.0.0:9000
export KOVANICA_PEERS=BOOTSTRAP_IP:9000

./target/release/kovanica-node explorer 127.0.0.1:8080
```

Put nginx in front of `:8080`. Open **9000/tcp** to peers.

`KOVANICA_PEERS` is a comma-separated list. The node pulls blocks on a timer and serves its DAG to inbound peers.

## Wallet

https://wallet.kovanica.online — BIP39 12-word seed in the browser (WebCrypto Ed25519). The node never sees the seed. Download seed is the backup.

## Git

GitHub Contents: write is not granted to the Grok connector (403). Reconnect GitHub with **Contents: Read and write** so this tree can be pushed and tagged `v0.2-testnet-1`.
