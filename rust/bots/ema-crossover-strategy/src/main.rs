//! # EMA Crossover Trading Bot (Example)
//!
//! **⚠️ FOR EDUCATIONAL PURPOSES ONLY - NOT FOR PRODUCTION USE ⚠️**
//!
//! Example bot demonstrating Drift Protocol's Rust SDK with a simple EMA crossover strategy.
//!
//! ## Strategy
//! - Long when fast EMA > slow EMA + buffer
//! - Short when fast EMA < slow EMA - buffer
//! - Close all positions when EMAs converge
//!
//! ## Requirements
//! Set RPC_ENDPOINT and PRIVATE_KEY environment variables.
//! Press Ctrl+C for graceful shutdown.

mod prices;
mod signal;
mod trading;

use anyhow::Result;
use dotenv::dotenv;
use log::info;
use std::time::Duration;
use trading::{BotConfig, EmaBot};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();

    info!("Starting EMA crossover trading bot");

    let config = BotConfig {
        order_size: 0.001,                        // 0.001 BTC per trade
        market_index: 1,                          // Drift BTC-PERP market
        update_interval: Duration::from_secs(60), // Check signals every 2s
        ema_fast_period: 13,                      // Fast EMA period
        ema_slow_period: 34,                      // Slow EMA period
        ema_history_size: 20,                     // History buffer size
        ema_signal_buffer: 2.0,                   // Signal threshold
        binance_ticker: "BTCUSDT".to_string(),    // Price data source
        binance_interval: "5m".to_string(),       // 5-minute klines
        price_history_limit: 100,                 // Initial history size
        price_update_limit: 1,                    // Single price per update
        authority: None,
        subaccount_id: 0, // Default subaccount
    };

    let mut bot = EmaBot::new(config).await?;

    tokio::select! {
        result = bot.start() => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
            bot.stop().await;
        }
    }

    Ok(())
}
