# Oracle Limit Maker

**⚠️ FOR EDUCATIONAL PURPOSES ONLY, NOT FOR PRODUCTION USE ⚠️**

A market maker example demonstrating Drift Protocol's Rust SDK with orderbook aware quoting and inventory management.

## Strategy

- **L2 Orderbook Integration**: Quotes based on actual best bid/ask from DLOB
- **Market-Aware Spreads**: Places orders at a multiple of current market spread (e.g. 1.5x)
- **Inventory Skewing**: Dynamically adjusts spread based on position
  - Long position: widen bids, tighten asks (encourage selling)
  - Short position: tighten bids, widen asks (encourage buying)
- **Dynamic Sizing**: Reduces order size on position side as inventory grows
- **Oracle Tracking**: Updates orders when oracle price moves significantly

## Configuration

Edit `main.rs` to customize:

```rust
let config = BotConfig {
    target_market: "BTC-PERP".to_string(),     // Market to trade
    order_size: 0.001,                         // BTC per order
    max_position_size: 0.01,                   // Position limit
    spread_multiplier: 1.5,                    // Quote at 1.5x market spread
    debounce_ms: 1000,                         // Oracle update throttle (ms)
    oracle_change_threshold_bps: 0.5,          // Min change to update (bps)
    authority: None,                           // For delegation
    subaccount_id: 0,                          // Subaccount ID
};
```

## Setup

Create `.env` file:

```bash
RPC_ENDPOINT="https://api.mainnet-beta.solana.com"
PRIVATE_KEY=your_drift_wallet_private_key
GRPC_URL="https://dlob.drift.trade"
GRPC_X_TOKEN=your_grpc_token
```

Run:

```bash
cargo build --release
./target/release/oracle-limit-maker
```

## Disclaimers

- Educational code only, not production ready
- No kill switches or advanced risk management
- May leave partial positions during errors or network issues
- No comprehensive monitoring, alerts, or failure recovery
- Requires stable GRPC connection for orderbook data

## Structure

- `src/maker.rs` - Core market making logic with DLOB integration and order management
- `src/main.rs` - Entry point with configuration and graceful shutdown handling
