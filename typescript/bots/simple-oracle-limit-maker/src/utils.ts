import winston from 'winston';

/**
 * Custom log format with structured data
 * Format: timestamp level [bot] [component] message data
 */
const format = winston.format.combine(
	winston.format.timestamp({ format: 'YYYY-MM-DD HH:mm:ss.SSS' }),
	winston.format.printf(
		({ timestamp, level, message, component, bot, data }) => {
			const parts = [
				timestamp,
				level.toUpperCase(),
				bot ? `[${bot}]` : '',
				component ? `[${component}]` : '',
				message,
				data ? JSON.stringify(data) : '',
			];
			return parts.filter(Boolean).join(' ');
		}
	)
);

/**
 * Winston logger configuration
 * Logs to both console and rotating files for debugging
 */
const logger = winston.createLogger({
	level: process.env.LOG_LEVEL || 'info',
	format,
	transports: [
		// Console output for development
		new winston.transports.Console({ handleExceptions: true }),

		// File output with rotation for production
		new winston.transports.File({
			filename: 'bot.log',
			maxsize: 5_000_000, // 5MB per file
			maxFiles: 3, // Keep 3 files max
		}),
	],
	exitOnError: false, // Don't exit on logging errors
});

/**
 * Create component-specific logger factory
 * Provides structured logging with bot and component context
 *
 * @param bot - Bot name for log filtering
 * @returns Logger with info/warn/error methods
 */
export const makeLogger = (bot: string) => ({
	info: (component: string, msg: string, data?: any) =>
		logger.info(msg, { component, bot, data }),

	warn: (component: string, msg: string, data?: any) =>
		logger.warn(msg, { component, bot, data }),

	error: (component: string, msg: string, err: Error) =>
		logger.error(msg, {
			component,
			bot,
			data: { message: err.message, stack: err.stack },
		}),
});
