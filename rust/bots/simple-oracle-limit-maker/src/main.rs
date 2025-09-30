//! # Oracle Limit Market Maker Bot (Example)
//!
//! **⚠️ FOR EDUCATIONAL PURPOSES ONLY - NOT FOR PRODUCTION USE ⚠️**
//!
//! Example bot demonstrating Drift Protocol's Rust SDK with oracle-based market making.
//!
//! ## Strategy
//! - Places limit orders with oracle-relative prices
//! - Uses inventory skewing to manage position risk
//! - Long position: widen bids, tighten asks
//! - Short position: tighten bids, widen asks
//!
//! ## Configuration
//! Set environment variables:
//! - RPC_ENDPOINT: Solana RPC endpoint
//! - PRIVATE_KEY: Base58 encoded private key
//!
//! ## Usage
//! Press Ctrl+C for graceful shutdown.

mod maker;

use anyhow::Result;
use dotenv::dotenv;
use env_logger::Builder;
use log::info;
use maker::{BotConfig, OracleLimitMakerBot};

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables
    dotenv().ok();

    // Initialize logger
    let mut builder = Builder::from_default_env();
    builder.format_timestamp_millis().init();

    info!("Starting Oracle Limit Market Maker Bot");

    let config = BotConfig {
        // Market configuration
        target_market: "BTC-PERP".to_string(), // Market symbol

        // Order sizing
        order_size: 0.001,  // 0.001 BTC per side
        max_position: 0.01, // Max 0.01 BTC position before skewing

        // Spread configuration
        base_spread_bps: 2, // 2 bps base spread (0.02%)
        max_skew_bps: 10,   // Max 10 bps additional skew when positioned

        // Timing configuration
        debounce_ms: 500,               // 500ms minimum between oracle updates
        oracle_change_threshold_bps: 2, // 5 bps minimum price change to trigger update

        // Account configuration
        authority: None,  // Set to Some("pubkey") for delegation
        subaccount_id: 0, // Default subaccount
    };

    // Initialize bot
    let mut bot = OracleLimitMakerBot::new(config).await?;

    // Start trading with graceful shutdown handling
    tokio::select! {
        result = bot.start() => {
            if let Err(e) = result {
                log::error!("Bot encountered fatal error: {}", e);
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C signal, initiating graceful shutdown...");
            bot.stop().await;
            info!("Shutdown completed successfully");
        }
    }

    Ok(())
}
