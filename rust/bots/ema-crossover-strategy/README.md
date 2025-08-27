# EMA Crossover Trading Bot

**⚠️ FOR EDUCATIONAL PURPOSES ONLY - NOT FOR PRODUCTION USE ⚠️**

A simple trading bot demonstrating Drift Protocol's Rust SDK with an EMA crossover strategy.

## Strategy

- **Long**: Fast EMA crosses above slow EMA + buffer
- **Short**: Fast EMA crosses below slow EMA - buffer
- **Neutral**: Close all positions when EMAs converge

Uses 13/34 period EMAs with price data from Binance.

## Configuration

Edit `main.rs` to customize:

```rust
let config = BotConfig {
    order_size: 0.001,                       // BTC per trade
    market_index: 1,                         // BTC-PERP
    update_interval: Duration::from_secs(2), // Signal check frequency
    ema_fast_period: 13,                     // Fast EMA
    ema_slow_period: 34,                     // Slow EMA
    ema_signal_buffer: 2.0,                  // $2 threshold
    // ... other settings
};
```

## Disclaimers

- Educational code only, not production ready
- No risk management or position sizing
- Uses market orders (potential poor fills)
- May contain bugs or inefficiencies
- No backtesting or optimization

## Structure

- `main.rs` - Entry point and configuration
- `trading.rs` - Bot logic and Drift SDK integration
- `signal.rs` - EMA calculation and signal generation
- `prices.rs` - Binance price data fetching
