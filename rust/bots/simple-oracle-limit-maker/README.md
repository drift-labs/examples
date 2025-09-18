# Simple Oracle Limit Maker

**⚠️ FOR EDUCATIONAL PURPOSES ONLY - NOT FOR PRODUCTION USE ⚠️**

A simplified market maker demonstrating Drift Protocol's Rust SDK with basic spread and inventory management.

## Strategy

- **Spread**: Places bid/ask orders around oracle price
- **Inventory Skewing**: Adjusts quotes based on position (widen away from inventory)
- **Oracle Tracking**: Updates orders when oracle price moves significantly

## Configuration

Edit `main.rs` to customize:

```rust
let config = BotConfig {
    target_market: "BTC-PERP".to_string(),     // Market to trade
    order_size: 0.001,                        // BTC per order
    max_position: 0.01,                       // Position limit
    base_spread_bps: 2,                       // Base spread (2 bps)
    max_skew_bps: 10,                         // Max position skew
    debounce_ms: 500,                         // Oracle update throttle
    oracle_change_threshold_bps: 5,           // Min change to update
    authority: None,                          // For delegation
    subaccount_id: 0,                         // Subaccount ID
};
```

## Setup

Create `.env` file:

```bash
RPC_ENDPOINT="https://api.mainnet-beta.solana.com"
PRIVATE_KEY=your_drift_wallet_private_key
```

```bash
cargo build
./target/debug/simple-oracle-limit-maker
```

## Disclaimers

- Educational code only, not production ready
- No kill switches or advanced risk management
- Non-atomic order replacement (cancel → place)
- May leave partial positions during errors
- No comprehensive monitoring or alerts

## Structure

- `src/maker.rs` - Core market making logic and order management
- `src/main.rs` - Entry point with configuration and graceful shutdown
