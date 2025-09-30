use anyhow::Result;
use drift_rs::{
    types::{
        Context, MarketId, MarketType, OrderParams, OrderType, PerpPosition, PositionDirection,
        PostOnlyParam,
    },
    DriftClient, GrpcSubscribeOpts, Pubkey, RpcClient, Wallet,
};
use log::{debug, error, info};
use solana_sdk::commitment_config::CommitmentLevel;
use std::{
    env,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::Duration;

// Drift precision constants
const BASE_PRECISION: f64 = 1_000_000_000.0; // 1e9
const QUOTE_PRECISION: f64 = 1_000_000.0; // 1e6

/// Bot configuration parameters
#[derive(Debug, Clone)]
pub struct BotConfig {
    pub target_market: String,            // Market
    pub order_size: f64,                  // Amount per order (side)
    pub max_position: f64,                // Maximum position size before skewing
    pub base_spread_bps: u16,             // Minimum spread around oracle price
    pub max_skew_bps: u16,                // Maximum additional skew when at max position
    pub debounce_ms: u64,                 // Minimum time between oracle updates
    pub oracle_change_threshold_bps: u16, // Minimum price change to trigger update
    pub authority: Option<String>,        // Authority pubkey (for delegation)
    pub subaccount_id: u16,               // Subaccount ID
}

#[derive(Debug)]
struct QuoteParams {
    bid_offset_bps: f64,
    ask_offset_bps: f64,
    size: f64,
}

/// Market Maker bot
pub struct OracleLimitMakerBot {
    client: DriftClient,
    config: BotConfig,
    // Market identification
    market_id: MarketId,
    // Bot state
    oracle_price: i64,
    prev_oracle_price: i64,
    last_oracle_update: u64,
    has_active_orders: bool,
    is_running: bool,
    is_processing: bool,
}

impl OracleLimitMakerBot {
    pub async fn new(config: BotConfig) -> Result<Self> {
        info!("Initializing oracle limit maker bot...");

        let client = Self::init_drift_client().await?;

        // Get market_id directly
        let market_id = client
            .market_lookup(&config.target_market)
            .ok_or_else(|| anyhow::anyhow!("Market symbol '{}' not found", config.target_market))?;

        info!("Found market: {} -> {:?}", config.target_market, market_id);

        // Subscribe to oracle feed
        // Comment out if using gRPC
        // client.subscribe_oracles(&[market_id]).await?;
        // info!(
        //     "Subscribed to oracle feed for market: {}",
        //     config.target_market
        // );

        // Get initial oracle price
        let initial_oracle_price = client.oracle_price(market_id).await.unwrap_or(0);
        info!(
            "Initial oracle price: ${:.2}",
            initial_oracle_price as f64 / QUOTE_PRECISION
        );

        info!(
            "Bot initialized for market: {} (subaccount: {})",
            config.target_market, config.subaccount_id
        );

        Ok(Self {
            client,
            config,
            market_id,
            oracle_price: initial_oracle_price,
            prev_oracle_price: 0,
            last_oracle_update: 0,
            has_active_orders: false,
            is_running: false,
            is_processing: false,
        })
    }

    async fn init_drift_client() -> Result<DriftClient> {
        let rpc_endpoint = env::var("RPC_ENDPOINT").expect("RPC_ENDPOINT not set");
        let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");

        let context = Context::MainNet;
        let wallet = Wallet::try_from_str(&private_key).unwrap();
        let rpc_client = RpcClient::new(rpc_endpoint);

        let client = DriftClient::new(context, rpc_client, wallet)
            .await
            .expect("Failed to initialize client");

        // gRPC version
        let grpc_url = env::var("GRPC_URL").expect("GRPC_URL not set");
        let grpc_token = env::var("GRPC_X_TOKEN").expect("GRPC_X_TOKEN not set");

        client
            .grpc_subscribe(
                grpc_url,
                grpc_token,
                GrpcSubscribeOpts::default().commitment(CommitmentLevel::Processed),
                true,
            )
            .await?;

        info!(
            "Connected to Drift with wallet: {}",
            client.wallet().authority()
        );
        Ok(client)
    }

    /// Start the main trading loop
    pub async fn start(&mut self) -> Result<()> {
        self.is_running = true;
        info!("Starting oracle limit maker...");

        while self.is_running {
            if let Err(e) = self.process_cycle().await {
                error!("Trading cycle failed: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await; // Back off on errors
                continue;
            }

            tokio::time::sleep(Duration::from_millis(self.config.debounce_ms)).await;
        }

        Ok(())
    }

    /// Process single trading cycle
    async fn process_cycle(&mut self) -> Result<()> {
        if self.is_processing {
            return Ok(());
        }

        // Get current oracle price
        let current_price = self.client.oracle_price(self.market_id).await?;

        // Check if price changed significantly
        if self.should_update_quotes(current_price) {
            self.handle_oracle_update(current_price).await?;
        } else {
            debug!(
                "Oracle price unchanged or within threshold: ${:.2}",
                current_price as f64 / QUOTE_PRECISION
            );
        }

        Ok(())
    }

    /// Check if we should update quotes based on price change and debounce
    fn should_update_quotes(&self, new_price: i64) -> bool {
        let now = get_current_timestamp();

        // Debounce rapid updates
        if now - self.last_oracle_update < self.config.debounce_ms {
            return false;
        }

        // Skip if price change is too small
        if self.prev_oracle_price > 0 {
            let change_bps = ((new_price - self.prev_oracle_price).abs() as f64
                / self.prev_oracle_price as f64)
                * 10000.0;

            if change_bps < self.config.oracle_change_threshold_bps as f64 {
                return false;
            }

            debug!("Oracle price change: {:.2} bps", change_bps);
        }

        true
    }

    /// Handle oracle price update
    async fn handle_oracle_update(&mut self, new_oracle_price: i64) -> Result<()> {
        if self.is_processing {
            return Ok(());
        }
        self.is_processing = true;

        info!(
            "Processing oracle update: ${:.2} -> ${:.2}",
            self.oracle_price as f64 / QUOTE_PRECISION,
            new_oracle_price as f64 / QUOTE_PRECISION
        );

        // Update price state
        self.prev_oracle_price = self.oracle_price;
        self.oracle_price = new_oracle_price;
        self.last_oracle_update = get_current_timestamp();

        // Get current position for inventory skewing
        let current_position = self.get_current_position().await?;
        let position_size =
            current_position.map(|p| p.base_asset_amount).unwrap_or(0) as f64 / BASE_PRECISION;

        info!(
            "Current position: {:.6} {}",
            position_size,
            self.config.target_market.replace("-PERP", "")
        );

        // Calculate quotes with inventory skewing
        let quotes = self.calculate_quotes(position_size);

        // Update orders: cancel existing + place new
        self.update_orders(quotes).await?;

        self.is_processing = false;
        Ok(())
    }

    /// Get current perp position
    async fn get_current_position(&self) -> Result<Option<PerpPosition>> {
        let subaccount = self.get_subaccount();
        let user_account = self.client.get_user_account(&subaccount).await?;

        Ok(user_account
            .perp_positions
            .iter()
            .find(|pos| pos.market_index == self.market_id.index())
            .cloned())
    }

    /// Calculate bid/ask quotes with inventory skewing
    fn calculate_quotes(&self, current_position: f64) -> QuoteParams {
        let base_spread_bps = self.config.base_spread_bps as f64;
        let half_spread_bps = base_spread_bps / 2.0;

        // Calculate position ratio and skew
        let position_ratio = current_position / self.config.max_position;
        let skew_bps = position_ratio.abs() * self.config.max_skew_bps as f64;

        let mut bid_offset_bps = half_spread_bps;
        let mut ask_offset_bps = half_spread_bps;

        // Inventory skewing: widen quotes away from position direction
        if current_position > 0.0 {
            // Long position: widen bids, tighten asks to encourage selling
            bid_offset_bps += skew_bps;
            ask_offset_bps = (ask_offset_bps - skew_bps).max(1.0); // Ensure positive
            debug!(
                "Long position detected, widening bids (+{:.1} bps), tightening asks (-{:.1} bps)",
                skew_bps, skew_bps
            );
        } else if current_position < 0.0 {
            // Short position: tighten bids, widen asks to encourage buying
            bid_offset_bps = (bid_offset_bps - skew_bps).max(1.0); // Ensure positive
            ask_offset_bps += skew_bps;
            debug!(
                "Short position detected, tightening bids (-{:.1} bps), widening asks (+{:.1} bps)",
                skew_bps, skew_bps
            );
        } else {
            debug!("No position, using base spread");
        }

        QuoteParams {
            bid_offset_bps,
            ask_offset_bps,
            size: self.config.order_size,
        }
    }

    /// Update orders: cancel existing and place new quotes
    async fn update_orders(&mut self, quotes: QuoteParams) -> Result<()> {
        let subaccount = self.get_subaccount();

        // Calculate oracle price offsets
        let oracle_price_f64 = self.oracle_price as f64;
        let bid_price_offset = -(oracle_price_f64 * quotes.bid_offset_bps / 10000.0) as i32;
        let ask_price_offset = (oracle_price_f64 * quotes.ask_offset_bps / 10000.0) as i32;

        // For logging, calculate display prices
        let bid_display_price = oracle_price_f64 + (bid_price_offset as f64);
        let ask_display_price = oracle_price_f64 + (ask_price_offset as f64);

        // Create bid order (buy) with oracle offset
        let bid_order = OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: PositionDirection::Long,
            base_asset_amount: (quotes.size * BASE_PRECISION) as u64,
            market_index: self.market_id.index(),
            price: 0,                                    // Set to 0 when using oracle offset
            oracle_price_offset: Some(bid_price_offset), // Negative for bid
            post_only: PostOnlyParam::TryPostOnly,
            ..Default::default()
        };

        // Create ask order (sell) with oracle offset
        let ask_order = OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: PositionDirection::Short,
            base_asset_amount: (quotes.size * BASE_PRECISION) as u64,
            market_index: self.market_id.index(),
            price: 0,                                    // Set to 0 when using oracle offset
            oracle_price_offset: Some(ask_price_offset), // Positive for ask
            post_only: PostOnlyParam::TryPostOnly,
            ..Default::default()
        };

        // Single atomic transaction: cancel + place both orders
        let cancel_and_place_tx = self
            .client
            .init_tx(&subaccount, self.is_delegated())
            .await?
            .cancel_orders((self.market_id.index(), MarketType::Perp), None)
            .place_orders(vec![bid_order, ask_order])
            .build();

        let signature = self.client.sign_and_send(cancel_and_place_tx).await?;
        self.has_active_orders = true;

        info!(
            "Updated quotes - Bid: ${:.2} (-{:.1} bps), Ask: ${:.2} (+{:.1} bps), Size: {:.6}, Sig: {}",
            bid_display_price / QUOTE_PRECISION,
            quotes.bid_offset_bps,
            ask_display_price / QUOTE_PRECISION,
            quotes.ask_offset_bps,
            quotes.size,
            signature
        );

        Ok(())
    }
    /// Get subaccount pubkey
    fn get_subaccount(&self) -> Pubkey {
        match &self.config.authority {
            Some(authority_str) => {
                let authority = Pubkey::from_str(authority_str).expect("Invalid authority pubkey");
                Wallet::derive_user_account(&authority, self.config.subaccount_id)
            }
            None => self.client.wallet().sub_account(self.config.subaccount_id),
        }
    }

    /// Check if using delegated signing
    fn is_delegated(&self) -> bool {
        self.config.authority.is_some()
    }

    /// Stop the bot and clean up
    pub async fn stop(&mut self) {
        info!("Stopping oracle limit maker bot...");
        self.is_running = false;

        // Cancel any active orders
        if self.has_active_orders {
            info!("Cancelling active orders before shutdown");
            let subaccount = self.get_subaccount();
            if let Ok(cancel_tx) = self
                .client
                .init_tx(&subaccount, self.is_delegated())
                .await
                .map(|tx| tx.cancel_all_orders().build())
            {
                if let Err(e) = self.client.sign_and_send(cancel_tx).await {
                    error!("Failed to cancel orders during shutdown: {}", e);
                } else {
                    info!("Successfully cancelled orders");
                }
            }
        }

        // Close any open positions
        if let Ok(current_position) = self.get_current_position().await {
            if let Some(pos) = current_position {
                if pos.base_asset_amount != 0 {
                    info!("Closing open position before shutdown");
                    let close_direction = if pos.base_asset_amount > 0 {
                        PositionDirection::Short
                    } else {
                        PositionDirection::Long
                    };

                    let close_order = OrderParams {
                        order_type: OrderType::Market,
                        market_type: MarketType::Perp,
                        direction: close_direction,
                        base_asset_amount: pos.base_asset_amount.unsigned_abs(),
                        market_index: self.market_id.index(),
                        reduce_only: true,
                        ..Default::default()
                    };

                    if let Ok(close_tx) = self
                        .client
                        .init_tx(&self.get_subaccount(), self.is_delegated())
                        .await
                        .map(|tx| tx.place_orders(vec![close_order]).build())
                    {
                        if let Err(e) = self.client.sign_and_send(close_tx).await {
                            error!("Failed to close position during shutdown: {}", e);
                        } else {
                            info!("Successfully closed position");
                        }
                    }
                }
            }
        }

        // Unsubscribe from oracle feed
        if let Err(e) = self.client.unsubscribe().await {
            error!("Failed to unsubscribe: {}", e);
        } else {
            info!("Unsubscribed from oracle feeds");
        }

        info!("Oracle limit maker bot stopped");
    }
}

/// Get current timestamp in milliseconds
fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
