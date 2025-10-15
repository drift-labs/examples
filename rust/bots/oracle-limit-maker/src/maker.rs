use std::{
    env,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use drift_rs::{
    dlob::{builder::DLOBBuilder, DLOB},
    math::constants::{BASE_PRECISION, QUOTE_PRECISION},
    types::{
        Context, MarketId, MarketType, OrderParams, OrderType, PerpPosition, PositionDirection,
        PostOnlyParam, RpcSendTransactionConfig,
    },
    DriftClient, GrpcSubscribeOpts, Pubkey, RpcClient, Wallet,
};
use log::{error, info, warn};
use solana_sdk::commitment_config::CommitmentLevel;
use std::str::FromStr;

/// Bot configuration parameters
#[derive(Debug, Clone)]
pub struct BotConfig {
    // Market symbol
    pub target_market: String,
    // Amount per order (base units)
    pub order_size: f64,
    // Maximum position size before skewing
    pub max_position_size: f64,
    // Multiplier for market spread (e.g. 1.5 = 150% of market spread)
    pub spread_multiplier: f64,
    // Minimum time between oracle updates
    pub debounce_ms: u64,
    // Minimum price change to trigger update (BPS)
    pub oracle_change_threshold_bps: f32,
    // Authority pubkey (for delegation)
    pub authority: Option<String>,
    // Subaccount ID
    pub subaccount_id: u16,
}
/// Runtime state
#[derive(Default)]
struct State {
    prev_oracle_price: i64,
    last_update_time: u64,
    is_running: bool,
}

/// Oracle-based market maker bot
pub struct OracleLimitMakerBot {
    config: BotConfig,
    client: DriftClient,
    dlob: &'static DLOB,
    market_id: MarketId,
    state: State,
}

// Local precision constants as f64
const QUOTE_PRECISION_F64: f64 = QUOTE_PRECISION as f64;
const BASE_PRECISION_F64: f64 = BASE_PRECISION as f64;

impl OracleLimitMakerBot {
    /// Initialize the bot with client and subscriptions
    pub async fn new(config: BotConfig) -> Result<Self> {
        // Load environment variables
        let rpc_endpoint = env::var("RPC_ENDPOINT").expect("RPC_ENDPOINT not set");
        let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");
        let grpc_url = env::var("GRPC_URL").expect("GRPC_URL not set");
        let grpc_token = env::var("GRPC_X_TOKEN").expect("GRPC_X_TOKEN not set");

        info!("Initializing market maker for '{}'", config.target_market);

        // Create drift client
        let context = Context::MainNet;
        let wallet = Wallet::try_from_str(&private_key)?;
        let rpc_client = RpcClient::new(rpc_endpoint);
        let client = DriftClient::new(context, rpc_client, wallet).await?;

        info!("Drift client initialized");

        // Get market ID
        let market_id = client
            .market_lookup(&config.target_market)
            .ok_or_else(|| anyhow::anyhow!("Market '{}' not found", config.target_market))?;

        info!("Found market: {} -> {:?}", config.target_market, market_id);

        // Setup DLOB builder
        let dlob_builder = DLOBBuilder::new(vec![market_id]);

        // Subscribe via GRPC
        client
            .grpc_subscribe(
                grpc_url,
                grpc_token,
                GrpcSubscribeOpts::default()
                    .commitment(CommitmentLevel::Processed)
                    .usermap_on()
                    .on_user_account(
                        dlob_builder.account_update_handler(client.backend().account_map()),
                    )
                    .on_slot(dlob_builder.slot_update_handler(client.clone())),
                true,
            )
            .await?;

        let dlob = dlob_builder.dlob();

        info!("Subscriptions active, DLOB ready");

        Ok(Self {
            config,
            client,
            dlob,
            market_id,
            state: State::default(),
        })
    }

    /// Main trading loop
    async fn trading_loop(&mut self) -> Result<()> {
        info!("Trading loop started");

        self.state.is_running = true;
        while self.state.is_running {
            // Get current oracle price
            let oracle = self
                .client
                .try_get_oracle_price_data_and_slot(self.market_id)
                .ok_or_else(|| anyhow::anyhow!("Failed to get oracle price"))?;
            let current_oracle_price = oracle.data.price;

            // Check if we should update quotes
            if self.should_update(current_oracle_price) {
                if let Err(e) = self.process_update(current_oracle_price).await {
                    error!("Update failed: {}", e);
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("Trading loop stopped");
        Ok(())
    }

    /// Check if quotes should be updated based on oracle price change and debounce
    fn should_update(&self, new_price: i64) -> bool {
        let now = get_current_timestamp_ms();

        // Debounce check
        if now - self.state.last_update_time < self.config.debounce_ms {
            return false;
        }

        // Allow first cycle
        if self.state.prev_oracle_price == 0 {
            return true;
        }

        // Price change check
        let price_diff = (new_price - self.state.prev_oracle_price).abs() as f32;
        let change_bps = (price_diff * 10_000.0) / self.state.prev_oracle_price.abs() as f32;

        if change_bps >= self.config.oracle_change_threshold_bps {
            info!(
                "Update triggered, oracle moved {:.2} bps in {:.1}s",
                change_bps,
                (now - self.state.last_update_time) as f64 / 1000.0
            );
            return true;
        }

        false
    }

    /// Process quote update based on new oracle price
    async fn process_update(&mut self, new_price: i64) -> Result<()> {
        let update_start = std::time::Instant::now();
        let oracle_price = new_price as f64 / QUOTE_PRECISION_F64;

        // Get L2 orderbook snapshot
        let l2 = self
            .dlob
            .get_l2_snapshot(self.market_id.index(), MarketType::Perp);

        // Get best bid and ask
        let (bid_price, _) = l2
            .bids
            .iter()
            .next_back()
            .map(|(p, s)| (*p as f64 / QUOTE_PRECISION_F64, *s))
            .ok_or_else(|| {
                warn!("Empty bid side");
                anyhow::anyhow!("No bids in orderbook")
            })?;

        let (ask_price, _) = l2
            .asks
            .iter()
            .next()
            .map(|(p, s)| (*p as f64 / QUOTE_PRECISION_F64, *s))
            .ok_or_else(|| {
                warn!("Empty ask side");
                anyhow::anyhow!("No asks in orderbook")
            })?;

        let mid_price = (bid_price + ask_price) / 2.0;
        let current_spread = ask_price - bid_price;

        info!(
            "L2 snapshot: best_bid ${:.2}, best_ask ${:.2}, spread ${:.4}",
            bid_price, ask_price, current_spread
        );

        // Calculate our spread based on market spread
        let our_spread = current_spread * self.config.spread_multiplier;

        // Get current position
        let position = self.get_current_position().await?.unwrap_or_default();
        let base_amount = position.base_asset_amount as f64 / BASE_PRECISION_F64;
        let position_ratio = base_amount / self.config.max_position_size;

        // Calculate dynamic sizing
        let (bid_size, ask_size) =
            Self::calculate_dynamic_sizing(self.config.order_size, position_ratio);

        // Calculate inventory skew
        let (bid_mult, ask_mult) = Self::calculate_inventory_skew(position_ratio);

        // Calculate our quotes
        let our_bid = mid_price - (our_spread / 2.0 * bid_mult);
        let our_ask = mid_price + (our_spread / 2.0 * ask_mult);

        // Convert to oracle offsets
        let bid_offset = ((our_bid - oracle_price) * QUOTE_PRECISION_F64) as i32;
        let ask_offset = ((our_ask - oracle_price) * QUOTE_PRECISION_F64) as i32;

        info!(
            "Position: base={:.4}, ratio={:.3}, bid_mult={:.3}, ask_mult={:.3}",
            base_amount, position_ratio, bid_mult, ask_mult
        );

        info!(
            "Quotes: mid ${:.2}, bid ${:.2} (offset {}), ask ${:.2} (offset {}), spread ${:.4}",
            mid_price, our_bid, bid_offset, our_ask, ask_offset, our_spread
        );

        // Build orders
        let subaccount = self.get_subaccount();

        let bid_order = OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: PositionDirection::Long,
            base_asset_amount: (bid_size * BASE_PRECISION_F64) as u64,
            market_index: self.market_id.index(),
            price: 0,
            oracle_price_offset: Some(bid_offset),
            post_only: PostOnlyParam::TryPostOnly,
            ..Default::default()
        };

        let ask_order = OrderParams {
            order_type: OrderType::Limit,
            market_type: MarketType::Perp,
            direction: PositionDirection::Short,
            base_asset_amount: (ask_size * BASE_PRECISION_F64) as u64,
            market_index: self.market_id.index(),
            price: 0,
            oracle_price_offset: Some(ask_offset),
            post_only: PostOnlyParam::TryPostOnly,
            ..Default::default()
        };

        // Build and send transaction
        let tx_start = std::time::Instant::now();
        let tx = self
            .client
            .init_tx(&subaccount, self.is_delegated())
            .await?
            .cancel_orders((self.market_id.index(), MarketType::Perp), None)
            .place_orders(vec![bid_order, ask_order])
            .build();

        let config = RpcSendTransactionConfig {
            skip_preflight: true,
            ..Default::default()
        };

        let signature = self
            .client
            .sign_and_send_with_config(tx, None, config)
            .await?;

        let tx_time_ms = tx_start.elapsed().as_millis();

        info!("Orders placed successfully. Sig: {}", signature);

        // Update state
        self.state.prev_oracle_price = new_price;
        self.state.last_update_time = get_current_timestamp_ms();

        info!(
            "Update completed in {}ms (tx: {}ms)",
            update_start.elapsed().as_millis(),
            tx_time_ms
        );

        Ok(())
    }

    /// Calculate inventory skew multipliers based on position
    fn calculate_inventory_skew(position_ratio: f64) -> (f64, f64) {
        if position_ratio.abs() <= 0.1 {
            return (1.0, 1.0);
        }

        let abs_ratio = position_ratio.abs();
        let max_skew = 0.8;
        let scale = 0.2;
        let skew = max_skew * (abs_ratio / scale).tanh();

        if position_ratio > 0.0 {
            // Long position: widen bids, tighten asks
            (1.0 + skew, 1.0 - skew)
        } else {
            // Short position: tighten bids, widen asks
            (1.0 - skew, 1.0 + skew)
        }
    }

    /// Calculate dynamic order sizing based on position
    fn calculate_dynamic_sizing(base_size: f64, position_ratio: f64) -> (f64, f64) {
        let abs_ratio = position_ratio.abs();
        let reduction_start_pct = 0.2;

        // At max position, stop adding to that side
        if abs_ratio >= 1.0 {
            return if position_ratio > 0.0 {
                (0.0, base_size)
            } else {
                (base_size, 0.0)
            };
        }

        // Gradually reduce size as position grows
        let size_multiplier = if abs_ratio > reduction_start_pct {
            let slope = -1.0 / (1.0 - reduction_start_pct);
            let intercept = -slope;
            slope * abs_ratio + intercept
        } else {
            1.0
        };

        if position_ratio > 0.0 {
            (base_size * size_multiplier, base_size)
        } else if position_ratio < 0.0 {
            (base_size, base_size * size_multiplier)
        } else {
            (base_size, base_size)
        }
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

    /// Start the bot
    pub async fn start(&mut self) -> Result<()> {
        self.trading_loop().await
    }

    /// Stop the bot and clean up
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping bot");
        self.state.is_running = false;

        let subaccount = self.get_subaccount();

        // Check if position exists
        let should_close = self
            .get_current_position()
            .await
            .ok()
            .and_then(|pos| pos)
            .filter(|pos| pos.base_asset_amount != 0);

        if let Some(pos) = should_close {
            // Cancel orders + close position atomically
            info!("Closing position and cancelling orders");

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

            let tx = self
                .client
                .init_tx(&subaccount, self.is_delegated())
                .await?
                .cancel_all_orders()
                .place_orders(vec![close_order])
                .build();

            match self.client.sign_and_send(tx).await {
                Ok(sig) => info!("Cancelled orders and closed position. Sig: {}", sig),
                Err(e) => error!("Failed to cancel and close: {}", e),
            }
        } else {
            // Just cancel orders
            info!("Cancelling orders");
            let tx = self
                .client
                .init_tx(&subaccount, self.is_delegated())
                .await?
                .cancel_all_orders()
                .build();

            match self.client.sign_and_send(tx).await {
                Ok(sig) => info!("Cancelled orders. Sig: {}", sig),
                Err(e) => error!("Failed to cancel orders: {}", e),
            }
        }

        // Unsubscribe
        self.client.grpc_unsubscribe();
        if let Err(e) = self.client.unsubscribe().await {
            error!("Failed to unsubscribe: {}", e);
        } else {
            info!("Unsubscribed successfully");
        }

        Ok(())
    }
}

fn get_current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
