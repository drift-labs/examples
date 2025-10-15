//! # Oracle Limit Market Maker Bot (Example)
//!
//! **⚠️ FOR EDUCATIONAL PURPOSES ONLY, NOT FOR PRODUCTION USE ⚠️**
//!
//! Example bot demonstrating Drift Protocol's Rust SDK with oracle limit market making.
//!
//! ## Strategy
//! - Places oracle limit order based on L2 best bid/ask from DLOB
//! - Uses inventory skewing to manage position risk:
//!   - Long position: widen bids, tighten asks (encourage selling)
//!   - Short position: tighten bids, widen asks (encourage buying)
//! - Dynamic order sizing: reduces size on position side as inventory grows
//!
//! ## Configuration
//! Set environment variables:
//! - RPC_ENDPOINT: Solana RPC endpoint
//! - PRIVATE_KEY: Base58 encoded private key
//! - GRPC_URL: GRPC endpoint for orderbook streaming
//! - GRPC_X_TOKEN: Authentication token for GRPC
//!
//! ## Usage
//! Press Ctrl+C for graceful shutdown (cancels orders and closes position).

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
        // Market and sizing
        target_market: "BTC-PERP".to_string(),
        order_size: 0.001,
        max_position_size: 0.01,
        spread_multiplier: 1.5,

        // Update thresholds
        debounce_ms: 1000,
        oracle_change_threshold_bps: 0.5,

        // Account
        authority: None,
        subaccount_id: 0,
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
            if let Err(e) = bot.stop().await {
                log::error!("Error during shutdown: {}", e);
            }
            info!("Shutdown completed successfully");
        }
    }

    Ok(())
}
