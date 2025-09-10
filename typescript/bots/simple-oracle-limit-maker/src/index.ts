import { ExampleMaker } from './maker';
import { makeLogger } from './utils';

const log = makeLogger('main');

// Global state for graceful shutdown
let bot: ExampleMaker | null = null;
let isShuttingDown = false;

/**
 * Graceful shutdown handler
 * Ensures orders are cancelled and positions are closed before exit
 */
async function shutdown(signal: string): Promise<void> {
	if (isShuttingDown) return;
	isShuttingDown = true;

	log.info('SHUTDOWN', 'Shutdown initiated', { signal });

	try {
		if (bot) {
			await bot.stop();
		}
		log.info('SHUTDOWN', 'Shutdown completed');
		process.exit(0);
	} catch (error) {
		log.error('SHUTDOWN', 'Shutdown failed', error as Error);
		process.exit(1);
	}
}

/**
 * Main application entry point
 * Sets up error handlers and starts the market maker
 */
async function main(): Promise<void> {
	try {
		log.info('STARTUP', 'Example Maker starting');

		// Setup graceful shutdown handlers
		process.on('SIGINT', () => shutdown('SIGINT')); // Ctrl+C
		process.on('SIGTERM', () => shutdown('SIGTERM')); // Process termination

		// Setup error handlers to prevent crashes
		process.on('uncaughtException', (error) => {
			log.error('FATAL', 'Uncaught exception', error);
			shutdown('UNCAUGHT_EXCEPTION');
		});

		process.on('unhandledRejection', (reason) => {
			log.error('FATAL', 'Unhandled rejection', new Error(String(reason)));
			shutdown('UNHANDLED_REJECTION');
		});

		// Initialize and start the market maker
		bot = new ExampleMaker();

		const initResult = await bot.initialize();
		if (!initResult.success) {
			log.error(
				'INIT',
				'Example Maker initialization failed',
				new Error(initResult.error)
			);
			process.exit(1);
		}

		// Start market making operations
		bot.start();
		log.info('STARTUP', 'Example Maker running');
	} catch (error) {
		log.error('MAIN', 'Fatal error', error as Error);
		process.exit(1);
	}
}

// Start the application with final error handler
main().catch((error) => {
	log.error('MAIN', 'Unhandled main error', error as Error);
	process.exit(1);
});
