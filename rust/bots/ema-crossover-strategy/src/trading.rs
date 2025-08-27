use crate::prices::fetch_binance_prices;
use crate::signal::{EMA, Signal};

use anyhow::Result;
use drift_rs::types::{MarketType, OrderType, PerpPosition, PositionDirection};
use drift_rs::{
    DriftClient, Pubkey, RpcClient, Wallet,
    types::{Context, OrderParams},
};
use log::{error, info};
use solana_sdk::signature::Signature;
use std::env;
use std::str::FromStr;
use std::time::Duration;

/// Bot configuration parameters.
#[derive(Debug, Clone)]
pub struct BotConfig {
    pub order_size: f64,
    pub market_index: u16,
    pub update_interval: Duration,
    pub ema_fast_period: u32,
    pub ema_slow_period: u32,
    pub ema_history_size: usize,
    pub ema_signal_buffer: f64,
    pub binance_ticker: String,
    pub binance_interval: String,
    pub price_history_limit: u32,
    pub price_update_limit: u32,
    pub authority: Option<String>,
    pub subaccount_id: u16,
}

/// Trading bot that executes EMA crossover strategy.
pub struct EmaBot {
    client: DriftClient,
    ema: EMA,
    config: BotConfig,
    current_signal: Signal,
    is_running: bool,
    is_processing: bool,
}

impl EmaBot {
    /// Creates new bot instance and initializes EMA with historical data.
    pub async fn new(config: BotConfig) -> Result<Self> {
        info!("Initializing bot...");

        let client = Self::init_drift_client().await?;
        let ema = Self::init_ema(&config).await?;
        let initial_signal = Signal::Neutral;

        info!("Bot initialized with initial signal: {:?}", initial_signal);

        Ok(Self {
            client,
            ema,
            config,
            current_signal: initial_signal,
            is_running: false,
            is_processing: false,
        })
    }

    /// Initializes Drift client from environment variables.
    async fn init_drift_client() -> Result<DriftClient> {
        let rpc_endpoint = env::var("RPC_ENDPOINT").expect("RPC_ENDPOINT not set");
        let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");

        let context = Context::MainNet;
        let wallet = Wallet::try_from_str(&private_key).unwrap();
        let rpc_client = RpcClient::new(rpc_endpoint);

        let client = DriftClient::new(context, rpc_client, wallet)
            .await
            .expect("Failed to initialize client");

        info!(
            "Connected to Drift with wallet: {}",
            client.wallet().authority()
        );
        Ok(client)
    }

    /// Initializes EMA with historical price data from Binance.
    async fn init_ema(config: &BotConfig) -> Result<EMA> {
        let prices = fetch_binance_prices(
            &config.binance_ticker,
            &config.binance_interval,
            config.price_history_limit,
        )
        .await?;

        let mut ema = EMA::new(
            config.ema_fast_period,
            config.ema_slow_period,
            config.ema_history_size,
            config.ema_signal_buffer,
        );

        ema.initialize(&prices)?;
        info!("EMA initialized with {} price points", prices.len());
        Ok(ema)
    }

    /// Starts the main trading loop.
    pub async fn start(&mut self) -> Result<()> {
        self.is_running = true;
        info!("Starting trading loop...");

        while self.is_running {
            if let Err(e) = self.process_cycle().await {
                error!("Cycle failed: {}", e);
            }
            tokio::time::sleep(self.config.update_interval).await;
        }

        info!("Trading loop stopped");
        Ok(())
    }

    /// Stops bot and closes all positions.
    pub async fn stop(&mut self) {
        info!("Stopping bot...");

        if let Err(e) = self.close_positions().await {
            error!("Failed to close positions during shutdown: {}", e);
        }

        if let Err(e) = self.client.unsubscribe().await {
            error!("Failed to unsubscribe from Drift client: {}", e);
        }

        self.is_running = false;
        info!("Bot stopped");
    }

    /// Processes single trading cycle: updates signal and executes trades.
    async fn process_cycle(&mut self) -> Result<()> {
        if self.is_processing {
            return Ok(());
        }

        self.is_processing = true;

        let new_signal = self.update_signal().await?;

        if new_signal != self.current_signal {
            info!(
                "Signal changed: {:?} -> {:?}",
                self.current_signal, new_signal
            );
            self.update_position(new_signal).await?;
            self.current_signal = new_signal;
        }

        self.is_processing = false;
        Ok(())
    }

    /// Updates EMA with latest price and returns new signal.
    async fn update_signal(&mut self) -> Result<Signal> {
        let prices = fetch_binance_prices(
            &self.config.binance_ticker,
            &self.config.binance_interval,
            self.config.price_update_limit,
        )
        .await?;

        let current_price = prices[0];
        self.ema.update(current_price)?;
        let signal = self.ema.crossover_signal();

        info!(
            "Price: ${:.2}, Fast EMA: {:.2}, Slow EMA: {:.2}, Signal: {:?}",
            current_price, self.ema.current_fast, self.ema.current_slow, signal
        );

        Ok(signal)
    }

    /// Executes position changes based on signal.
    async fn update_position(&mut self, signal: Signal) -> Result<()> {
        match signal {
            Signal::Long => self.handle_long_signal().await?,
            Signal::Short => self.handle_short_signal().await?,
            Signal::Neutral => {
                let sig = self.close_positions().await?;
                info!("Flattened all positions (Neutral signal): {}", sig);
            }
        }
        Ok(())
    }

    // Handle Long Signal
    async fn handle_long_signal(&mut self) -> Result<()> {
        let current_position = self.get_current_position().await?;

        match current_position {
            Some(pos) if pos.base_asset_amount < 0 => {
                // Close short and open long
                let sig = self.close_and_update(PositionDirection::Long).await?;
                info!("Closed short and opened long: {}", sig);
            }
            Some(pos) if pos.base_asset_amount > 0 => {
                info!("Already long, no action needed");
            }
            _ => {
                // No position or zero position
                let sig = self.update(PositionDirection::Long).await?;
                info!("Opened long position: {}", sig);
            }
        }
        Ok(())
    }

    // Handle Short Signal
    async fn handle_short_signal(&mut self) -> Result<()> {
        let current_position = self.get_current_position().await?;

        match current_position {
            Some(pos) if pos.base_asset_amount > 0 => {
                // Close long and open short
                let sig = self.close_and_update(PositionDirection::Short).await?;
                info!("Closed long and opened short: {}", sig);
            }
            Some(pos) if pos.base_asset_amount < 0 => {
                info!("Already short, no action needed");
            }
            _ => {
                // No position or zero position
                let sig = self.update(PositionDirection::Short).await?;
                info!("Opened short position: {}", sig);
            }
        }
        Ok(())
    }

    /// Closes positions.
    async fn close_positions(&mut self) -> Result<Signature> {
        let subaccount = self.get_subaccount();
        let user_account = self.client.get_user_account(&subaccount).await?;

        let mut reduce_orders = Vec::new();
        for pos in &user_account.perp_positions {
            if pos.base_asset_amount != 0 {
                let direction = if pos.base_asset_amount > 0 {
                    PositionDirection::Short
                } else {
                    PositionDirection::Long
                };

                reduce_orders.push(OrderParams {
                    order_type: OrderType::Market,
                    market_type: MarketType::Perp,
                    direction,
                    base_asset_amount: pos.base_asset_amount.unsigned_abs(),
                    market_index: pos.market_index,
                    reduce_only: true,
                    ..Default::default()
                });
            }
        }

        let tx = self
            .client
            .init_tx(&subaccount, self.is_delegated())
            .await?
            .place_orders(reduce_orders)
            .build();

        let sig = self.client.sign_and_send(tx).await?;
        info!("Flattened all positions: {}", sig);

        Ok(sig)
    }

    /// Places market order in specified direction.
    async fn update(&mut self, direction: PositionDirection) -> Result<Signature> {
        let subaccount = self.get_subaccount();

        let order_params = OrderParams {
            order_type: OrderType::Market,
            market_type: MarketType::Perp,
            direction,
            base_asset_amount: (self.config.order_size * 1e9) as u64,
            market_index: self.config.market_index,
            ..Default::default()
        };

        let tx = self
            .client
            .init_tx(&subaccount, self.is_delegated())
            .await?
            .place_orders(vec![order_params])
            .build();

        self.client.sign_and_send(tx).await.map_err(Into::into)
    }

    /// Closes existing position and opens new one atomically.
    async fn close_and_update(&mut self, direction: PositionDirection) -> Result<Signature> {
        let subaccount = self.get_subaccount();

        // Close existing position
        let order_params = OrderParams {
            order_type: OrderType::Market,
            market_type: MarketType::Perp,
            direction,
            base_asset_amount: (self.config.order_size * 1e9) as u64,
            market_index: self.config.market_index,
            reduce_only: true,
            ..Default::default()
        };

        // Open new position
        let new_order_params = OrderParams {
            order_type: OrderType::Market,
            market_type: MarketType::Perp,
            direction,
            base_asset_amount: (self.config.order_size * 1e9) as u64,
            market_index: self.config.market_index,
            reduce_only: false,
            ..Default::default()
        };

        let tx = self
            .client
            .init_tx(&subaccount, self.is_delegated())
            .await?
            .place_orders(vec![order_params, new_order_params])
            .build();

        self.client.sign_and_send(tx).await.map_err(Into::into)
    }

    fn get_subaccount(&self) -> Pubkey {
        match &self.config.authority {
            Some(authority_str) => {
                let authority = Pubkey::from_str(authority_str).expect("Invalid authority pubkey");
                Wallet::derive_user_account(&authority, self.config.subaccount_id)
            }
            None => self.client.wallet().sub_account(self.config.subaccount_id),
        }
    }

    async fn get_current_position(&self) -> Result<Option<PerpPosition>> {
        let subaccount = self.get_subaccount();
        let user_account = self.client.get_user_account(&subaccount).await?;

        Ok(user_account
            .perp_positions
            .iter()
            .find(|pos| pos.market_index == self.config.market_index)
            .cloned())
    }

    fn is_delegated(&self) -> bool {
        self.config.authority.is_some()
    }
}
