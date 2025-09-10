import {
	DriftClient,
	DLOBSubscriber,
	OrderSubscriber,
	EventSubscriber,
	initialize,
	convertToNumber,
	MarketType,
	OrderType,
	PositionDirection,
	BASE_PRECISION,
	PerpMarkets,
	Wallet,
	type OraclePriceData,
	type PerpMarketConfig,
	SlotSubscriber,
	PostOnlyParams,
	QUOTE_PRECISION,
	BN,
	loadKeypair,
} from '@drift-labs/sdk';
import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { makeLogger } from './utils';
import { MM_CONFIG } from './config';

const log = makeLogger('example-maker');

// Bot state tracking
type BotState = {
	oraclePrice: number;
	previousOraclePrice: number;
	lastOracleUpdate: number;
	hasActiveOrders: boolean;
	totalMatches: number;
	realizedPnl: number;
	isRunning: boolean;
	isProcessing: boolean;
};

// FIFO queue entry for P&L tracking
type FillEntry = {
	timestamp: number;
	size: number;
	price: number;
};

/**
 * Simple market maker that provides liquidity by placing bid/ask orders
 * Core loop: listen to oracle → calculate quotes → place orders → track fills
 */
export class ExampleMaker {
	// Drift protocol components
	private driftClient!: DriftClient;
	private slotSubscriber!: SlotSubscriber;
	private dlobSubscriber!: DLOBSubscriber;
	private orderSubscriber!: OrderSubscriber;
	private eventSubscriber!: EventSubscriber;

	// Market configuration
	private marketIndex!: number;
	private marketConfig!: PerpMarketConfig;

	// Bot state
	private state: BotState = {
		oraclePrice: 0,
		previousOraclePrice: 0,
		lastOracleUpdate: 0,
		hasActiveOrders: false,
		totalMatches: 0,
		realizedPnl: 0,
		isRunning: false,
		isProcessing: false,
	};

	// FIFO queues for P&L calculation
	private buyQueue: FillEntry[] = [];
	private sellQueue: FillEntry[] = [];

	constructor() {
		// Find market configuration
		const market = PerpMarkets[MM_CONFIG.ENV].find(
			(m) => m.baseAssetSymbol === MM_CONFIG.TARGET_MARKET
		);
		if (!market) {
			throw new Error(`Market ${MM_CONFIG.TARGET_MARKET} not found`);
		}

		this.marketIndex = market.marketIndex;
		this.marketConfig = market;

		log.info('MARKET', 'Market found', {
			symbol: MM_CONFIG.TARGET_MARKET,
			marketIndex: this.marketIndex,
		});
	}

	async initialize(): Promise<{ success: boolean; error?: string }> {
		try {
			log.info('INIT', 'Initializing Example Maker');

			const initResult = await this.initializeDrift();
			if (!initResult.success) return initResult;

			await this.initializeSubscribers();
			this.setupOracleListener();
			await this.setupFillEventListener();

			log.info('INIT', 'Example Maker initialized successfully');
			return { success: true };
		} catch (error) {
			log.error('INIT', 'Initialization failed', error as Error);
			return {
				success: false,
				error: `Initialization failed: ${(error as Error).message}`,
			};
		}
	}

	/**
	 * Initialize Drift client and authenticate wallet
	 */
	private async initializeDrift(): Promise<{
		success: boolean;
		error?: string;
	}> {
		try {
			const sdk = initialize({ env: MM_CONFIG.ENV });
			const connection = new Connection(MM_CONFIG.RPC_ENDPOINT, {
				commitment: 'confirmed',
			});

			let wallet: Wallet;

			if (MM_CONFIG.KEYPAIR_PATH) {
				// Use keypair file if path is provided
				wallet = new Wallet(loadKeypair(MM_CONFIG.KEYPAIR_PATH));
			} else if (MM_CONFIG.PRIVATE_KEY) {
				// Fall back to private key string
				const parseKey = (key: string): Uint8Array =>
					key.startsWith('[')
						? new Uint8Array(JSON.parse(key))
						: Buffer.from(key, 'base64');

				const secretKey = parseKey(MM_CONFIG.PRIVATE_KEY);
				wallet = new Wallet(Keypair.fromSecretKey(secretKey));
			} else {
				throw new Error(
					'Either KEYPAIR_PATH or PRIVATE_KEY environment variable must be set'
				);
			}

			// Initialize Drift client
			this.driftClient = new DriftClient({
				connection,
				wallet,
				env: MM_CONFIG.ENV,
				programID: new PublicKey(sdk.DRIFT_PROGRAM_ID),
				accountSubscription: {
					type: 'websocket',
				},
			});

			await this.driftClient.subscribe();
			await this.driftClient.getUser().exists();

			log.info('DRIFT', 'Drift client initialized successfully', {
				publicKey: wallet.publicKey.toString(),
				env: MM_CONFIG.ENV,
			});

			return { success: true };
		} catch (error) {
			return {
				success: false,
				error: `Drift initialization failed: ${(error as Error).message}`,
			};
		}
	}

	/**
	 * Initialize data subscribers for market data and events
	 */
	private async initializeSubscribers(): Promise<void> {
		try {
			log.info('SUBSCRIBERS', 'Initializing subscribers');

			// Slot subscriber for blockchain state
			this.slotSubscriber = new SlotSubscriber(this.driftClient.connection);

			// Order book subscriber
			this.orderSubscriber = new OrderSubscriber({
				driftClient: this.driftClient,
				subscriptionConfig: {
					type: 'websocket',
					commitment: 'processed',
					resubTimeoutMs: 30000,
				},
			});

			// DLOB (Decentralized Limit Order Book) subscriber
			this.dlobSubscriber = new DLOBSubscriber({
				dlobSource: this.orderSubscriber,
				slotSource: this.slotSubscriber,
				driftClient: this.driftClient,
				updateFrequency: 1000,
			});

			// Event subscriber for fill notifications
			this.eventSubscriber = new EventSubscriber(
				this.driftClient.connection,
				this.driftClient.program,
				{
					eventTypes: ['OrderActionRecord'],
					commitment: 'confirmed',
					logProviderConfig: {
						type: 'websocket',
					},
				}
			);

			// Start all subscriptions
			await this.orderSubscriber.subscribe();
			await this.dlobSubscriber.subscribe();
			await this.eventSubscriber.subscribe();
			await this.slotSubscriber.subscribe();

			log.info('SUBSCRIBERS', 'All subscribers initialized successfully');
		} catch (error) {
			throw new Error(
				`Subscriber initialization failed: ${(error as Error).message}`
			);
		}
	}

	/**
	 * Setup oracle price feed listener
	 * This is the primary trigger for quote updates
	 */
	private setupOracleListener(): void {
		log.info('ORACLE', 'Setting up oracle listener');

		this.driftClient.eventEmitter.on(
			'oraclePriceUpdate',
			(publicKey: PublicKey, oracleSource: any, data: OraclePriceData) => {
				// Only process updates for our target market
				if (!publicKey.equals(this.marketConfig.oracle)) return;

				if (!this.isValidOracleUpdate(data)) return;

				const oraclePrice = convertToNumber(data.price);

				if (this.shouldSkipOracleUpdate(oraclePrice)) return;

				this.handleOracleUpdate(oraclePrice).catch((error) => {
					log.error('ORACLE', 'Oracle update handler failed', error);
				});
			}
		);

		log.info('ORACLE', 'Oracle listener setup complete');
	}

	/**
	 * Validate oracle data to prevent acting on corrupt prices
	 */
	private isValidOracleUpdate(data: OraclePriceData): boolean {
		const oraclePrice = convertToNumber(data.price);

		if (oraclePrice <= 0) {
			log.warn('ORACLE', 'Invalid oracle price received', {
				price: data.price.toString(),
			});
			return false;
		}

		// Skip if price drops >99% (likely corrupt data)
		if (
			oraclePrice <= 0 ||
			(this.state.oraclePrice > 0 &&
				oraclePrice < this.state.oraclePrice * 0.01)
		) {
			log.warn('ORACLE', 'Skipping suspicious oracle price drop', {
				current: this.state.oraclePrice,
				new: oraclePrice,
			});
			return false;
		}

		return true;
	}

	/**
	 * Debounce oracle updates to avoid excessive quote changes
	 */
	private shouldSkipOracleUpdate(newPrice: number): boolean {
		const now = Date.now();

		// Debounce rapid updates
		if (now - this.state.lastOracleUpdate < MM_CONFIG.DEBOUNCE_MS) {
			return true;
		}

		// Skip if price change is too small
		if (this.state.previousOraclePrice > 0) {
			const changeBps =
				Math.abs(
					(newPrice - this.state.previousOraclePrice) /
						this.state.previousOraclePrice
				) * 10000;

			if (changeBps < MM_CONFIG.ORACLE_CHANGE_THRESHOLD_BPS) {
				return true;
			}
		}

		return false;
	}

	/**
	 * Setup fill event listener to track our order executions
	 */
	private async setupFillEventListener(): Promise<void> {
		const userAccountPublicKey =
			await this.driftClient.getUserAccountPublicKey();
		const userAccountString = userAccountPublicKey.toString();

		log.info('FILL_LISTENER', 'Setting up fill event listener', {
			userAccount: userAccountString,
		});

		this.eventSubscriber.eventEmitter.on('newEvent', (event: any) => {
			if (!this.isOurFillEvent(event, userAccountString)) return;

			const fillData = this.extractFillData(event, userAccountString);
			if (!fillData) return;

			this.handleFill(fillData);
		});
	}

	/**
	 * Check if fill event is for our orders
	 */
	private isOurFillEvent(event: any, userAccountString: string): boolean {
		return (
			event.eventType === 'OrderActionRecord' &&
			event.marketIndex === this.marketIndex &&
			event.marketType &&
			'perp' in event.marketType &&
			event.action &&
			'fill' in event.action &&
			(event.maker?.toString() === userAccountString ||
				event.taker?.toString() === userAccountString)
		);
	}

	/**
	 * Extract fill details from event data
	 */
	private extractFillData(
		event: any,
		userAccountString: string
	): {
		price: number;
		size: number;
		side: 'BUY' | 'SELL';
	} | null {
		if (!event.baseAssetAmountFilled || !event.quoteAssetAmountFilled) {
			log.warn('FILL_EXTRACT', 'Missing fill amounts in event');
			return null;
		}

		try {
			const baseAmount =
				event.baseAssetAmountFilled.toNumber() / BASE_PRECISION.toNumber();
			const quoteAmount =
				event.quoteAssetAmountFilled.toNumber() / QUOTE_PRECISION.toNumber();

			const price = quoteAmount / baseAmount;
			const size = Math.abs(baseAmount);

			// Determine if we were maker or taker, and the side
			const isMaker = event.maker?.toString() === userAccountString;
			const side = isMaker
				? event.makerOrderDirection && 'long' in event.makerOrderDirection
					? 'BUY'
					: 'SELL'
				: event.takerOrderDirection && 'long' in event.takerOrderDirection
					? 'BUY'
					: 'SELL';

			return { price, size, side };
		} catch (error) {
			log.error('FILL_EXTRACT', 'Failed to extract fill data', error as Error);
			return null;
		}
	}

	/**
	 * Handle fill execution and update P&L tracking
	 */
	private handleFill(fill: {
		side: 'BUY' | 'SELL';
		size: number;
		price: number;
	}): void {
		// Add to appropriate FIFO queue
		if (fill.side === 'BUY') {
			this.buyQueue.push({
				timestamp: Date.now(),
				size: fill.size,
				price: fill.price,
			});
		} else {
			this.sellQueue.push({
				timestamp: Date.now(),
				size: fill.size,
				price: fill.price,
			});
		}

		log.info('FILL', 'Fill detected', {
			side: fill.side,
			size: fill.size.toFixed(6),
			price: fill.price.toFixed(2),
		});

		this.processFIFOMatching();
	}

	/**
	 * FIFO matching to calculate realized P&L
	 * Matches oldest buys with oldest sells to track true profitability
	 */
	private processFIFOMatching(): void {
		while (this.buyQueue.length > 0 && this.sellQueue.length > 0) {
			const oldestBuy = this.buyQueue[0]!;
			const oldestSell = this.sellQueue[0]!;

			// Match the smaller of the two sizes
			const matchSize = Math.min(oldestBuy.size, oldestSell.size);
			const pnl = (oldestSell.price - oldestBuy.price) * matchSize;

			this.state.realizedPnl += pnl;
			this.state.totalMatches++;

			log.info('MATCH', 'Round trip completed', {
				matchNumber: this.state.totalMatches,
				matchSize: matchSize.toFixed(6),
				buyPrice: oldestBuy.price.toFixed(2),
				sellPrice: oldestSell.price.toFixed(2),
				pnl: pnl.toFixed(2),
				totalPnl: this.state.realizedPnl.toFixed(2),
			});

			// Update remaining sizes
			oldestBuy.size -= matchSize;
			oldestSell.size -= matchSize;

			// Remove fully consumed trades
			if (oldestBuy.size <= 0.000001) {
				this.buyQueue.shift();
			}
			if (oldestSell.size <= 0.000001) {
				this.sellQueue.shift();
			}
		}
	}

	/**
	 * Main oracle update handler - triggers quote recalculation
	 */
	private async handleOracleUpdate(newOraclePrice: number): Promise<void> {
		if (!this.state.isRunning) return;

		// Skip if already processing
		if (this.state.isProcessing) {
			return;
		}

		const now = Date.now();

		log.info('ORACLE', 'Processing oracle update', {
			newPrice: newOraclePrice.toFixed(2),
			oldPrice: this.state.oraclePrice.toFixed(2),
		});

		// Update price state
		this.state.previousOraclePrice = this.state.oraclePrice;
		this.state.oraclePrice = newOraclePrice;
		this.state.lastOracleUpdate = now;

		try {
			await this.updateQuotes();
		} catch (error) {
			log.error('UPDATE', 'Failed to update quotes', error as Error);
		}
	}

	/**
	 * Calculate new quotes and update orders
	 */
	private async updateQuotes(): Promise<void> {
		this.state.isProcessing = true;

		try {
			const currentPosition = this.getCurrentPosition();
			const quotes = this.calculateQuotes(currentPosition);

			if (!this.validateQuotes(quotes)) {
				log.warn('QUOTES', 'Invalid quotes generated, skipping update');
				return;
			}

			await this.updateOrders(quotes);
		} finally {
			this.state.isProcessing = false;
		}
	}
	/**
	 * Get current position from Drift client
	 */
	private getCurrentPosition(): number {
		try {
			const perpPosition = this.driftClient
				.getUser()
				.getPerpPosition(this.marketIndex);
			return perpPosition
				? perpPosition.baseAssetAmount.toNumber() / BASE_PRECISION.toNumber()
				: 0;
		} catch (error) {
			log.warn('POSITION', 'Failed to get position', error as Error);
			return 0;
		}
	}

	/**
	 * Calculate bid/ask quotes with inventory skewing
	 * Core market making logic: base spread + position-based skew
	 */
	private calculateQuotes(currentPosition: number): {
		bidOffsetBps: number;
		askOffsetBps: number;
		bidSize: number;
		askSize: number;
	} {
		const baseSpreadBps = MM_CONFIG.BASE_SPREAD_BPS;
		const halfSpreadBps = baseSpreadBps / 2;

		// Calculate position ratio and skew
		const positionRatio = currentPosition / MM_CONFIG.MAX_POSITION;
		const skewBps = Math.abs(positionRatio) * MM_CONFIG.MAX_SKEW_BPS;

		let bidOffsetBps = halfSpreadBps;
		let askOffsetBps = halfSpreadBps;

		// Inventory skewing: widen quotes away from position direction
		if (currentPosition > 0) {
			// Long position: widen bids, tighten asks to encourage selling
			bidOffsetBps += skewBps;
			askOffsetBps = askOffsetBps - skewBps;
		} else if (currentPosition < 0) {
			// Short position: tighten bids, widen asks to encourage buying
			bidOffsetBps = bidOffsetBps - skewBps;
			askOffsetBps += skewBps;
		}

		return {
			bidOffsetBps,
			askOffsetBps,
			bidSize: MM_CONFIG.ORDER_SIZE,
			askSize: MM_CONFIG.ORDER_SIZE,
		};
	}

	/**
	 * Validate quotes before placing orders
	 */
	private validateQuotes(quotes: {
		bidOffsetBps: number;
		askOffsetBps: number;
		bidSize: number;
		askSize: number;
	}): boolean {
		if (quotes.bidOffsetBps <= 0 || quotes.askOffsetBps <= 0) {
			log.warn('VALIDATION', 'Invalid offsets in quotes');
			return false;
		}

		if (quotes.bidSize <= 0 || quotes.askSize <= 0) {
			log.warn('VALIDATION', 'Invalid sizes in quotes');
			return false;
		}

		return true;
	}

	/**
	 * Update orders: cancel existing + place new quotes
	 */
	private async updateOrders(quotes: {
		bidOffsetBps: number;
		askOffsetBps: number;
		bidSize: number;
		askSize: number;
	}): Promise<void> {
		try {
			if (this.state.hasActiveOrders) {
				await this.driftClient.cancelOrders(MarketType.PERP, this.marketIndex);
			}

			// Convert BPS to price offsets
			const bidPriceOffset =
				-this.state.oraclePrice * (quotes.bidOffsetBps / 10000);
			const askPriceOffset =
				this.state.oraclePrice * (quotes.askOffsetBps / 10000);

			const bidOrderParams = {
				orderType: OrderType.LIMIT,
				marketIndex: this.marketIndex,
				direction: PositionDirection.LONG,
				baseAssetAmount: this.driftClient.convertToPerpPrecision(
					quotes.bidSize
				),
				price: new BN(0),
				oraclePriceOffset: Math.round(
					bidPriceOffset * QUOTE_PRECISION.toNumber()
				),
				postOnly: PostOnlyParams.TRY_POST_ONLY,
				marketType: MarketType.PERP,
			};

			const askOrderParams = {
				orderType: OrderType.LIMIT,
				marketIndex: this.marketIndex,
				direction: PositionDirection.SHORT,
				baseAssetAmount: this.driftClient.convertToPerpPrecision(
					quotes.askSize
				),
				price: new BN(0),
				oraclePriceOffset: Math.round(
					askPriceOffset * QUOTE_PRECISION.toNumber()
				),
				postOnly: PostOnlyParams.TRY_POST_ONLY,
				marketType: MarketType.PERP,
			};

			await this.driftClient.placePerpOrder(bidOrderParams);
			await this.driftClient.placePerpOrder(askOrderParams);
			this.state.hasActiveOrders = true;

			log.info('ORDERS', 'Oracle orders updated successfully', {
				bidOffset: bidPriceOffset.toFixed(2),
				askOffset: askPriceOffset.toFixed(2),
			});
		} catch (error) {
			log.error('ORDERS', 'Failed to update oracle orders', error as Error);
			this.state.hasActiveOrders = false;
		}
	}

	/**
	 * Close current position with market order
	 */
	private async closeCurrentPosition(): Promise<void> {
		const currentPosition = this.getCurrentPosition();

		if (Math.abs(currentPosition) < 0.0001) {
			return; // No position to close
		}

		try {
			const closeDirection =
				currentPosition > 0 ? PositionDirection.SHORT : PositionDirection.LONG;

			const closeOrderParams = {
				orderType: OrderType.MARKET,
				marketIndex: this.marketIndex,
				direction: closeDirection,
				baseAssetAmount: this.driftClient.convertToPerpPrecision(
					Math.abs(currentPosition)
				),
				reduceOnly: true,
				marketType: MarketType.PERP,
			};

			await this.driftClient.placePerpOrder(closeOrderParams);

			log.info('CLOSE', 'Position closed', {
				size: currentPosition.toFixed(6),
				direction: closeDirection === PositionDirection.LONG ? 'BUY' : 'SELL',
			});
		} catch (error) {
			log.error('CLOSE', 'Failed to close position', error as Error);
		}
	}

	/**
	 * Start the market maker
	 */
	start(): void {
		this.state.isRunning = true;
		log.info('START', 'Example Maker started');
	}

	/**
	 * Stop the market maker and clean up
	 */
	async stop(): Promise<void> {
		log.info('STOP', 'Stopping Example Maker');
		this.state.isRunning = false;

		// Cancel any active orders
		try {
			if (this.state.hasActiveOrders) {
				await this.driftClient.cancelOrders(MarketType.PERP, this.marketIndex);
			}

			// Close existing positions
			await this.closeCurrentPosition();
		} catch (error) {
			log.error(
				'STOP',
				'Failed to cancel orders during shutdown',
				error as Error
			);
		}

		// Unsubscribe from all data feeds
		try {
			await this.dlobSubscriber?.unsubscribe();
			await this.slotSubscriber?.unsubscribe();
			await this.orderSubscriber?.unsubscribe();
			await this.eventSubscriber?.unsubscribe();
			await this.driftClient?.unsubscribe();
		} catch (error) {
			log.error('STOP', 'Failed to unsubscribe components', error as Error);
		}

		log.info('STOP', 'Example Maker stopped');
	}
}
