use anyhow::{Result, bail};
use std::collections::VecDeque;

// Constants
const EMA_SMOOTHING_NUMERATOR: f64 = 2.0;
const EMA_PERIOD_OFFSET: f64 = 1.0;

/// Calculates EMA update using exponential moving average formula.
#[inline]
fn update_ema(current_ema: f64, new_price: f64, alpha: f64) -> f64 {
    if !current_ema.is_finite() || !new_price.is_finite() || !alpha.is_finite() {
        return f64::NAN;
    }
    if alpha < 0.0 || alpha > 1.0 {
        return f64::NAN;
    }
    alpha * new_price + (1.0 - alpha) * current_ema
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Long,
    Short,
    Neutral,
}

#[allow(dead_code)]
pub struct EMA {
    pub current_fast: f64,
    pub current_slow: f64,
    fast_history: VecDeque<f64>,
    slow_history: VecDeque<f64>,
    alpha_fast: f64,
    alpha_slow: f64,
    fast_period: u32,
    slow_period: u32,
    history_size: usize,
    signal_buffer: f64,
}

#[allow(dead_code)]
impl EMA {
    pub fn new(
        fast_period: u32,
        slow_period: u32,
        history_size: usize,
        signal_buffer: f64,
    ) -> Self {
        Self {
            current_fast: 0.0,
            current_slow: 0.0,
            fast_history: VecDeque::with_capacity(history_size),
            slow_history: VecDeque::with_capacity(history_size),
            alpha_fast: EMA_SMOOTHING_NUMERATOR / (fast_period as f64 + EMA_PERIOD_OFFSET),
            alpha_slow: EMA_SMOOTHING_NUMERATOR / (slow_period as f64 + EMA_PERIOD_OFFSET),
            fast_period,
            slow_period,
            history_size,
            signal_buffer,
        }
    }

    /// Initializes EMAs with historical price data.
    pub fn initialize(&mut self, prices: &[f64]) -> Result<()> {
        if prices.is_empty() {
            bail!("Price array cannot be empty");
        }

        self.current_fast = prices[0];
        self.current_slow = prices[0];
        self.fast_history.clear();
        self.slow_history.clear();
        self.fast_history.push_back(self.current_fast);
        self.slow_history.push_back(self.current_slow);

        for &price in &prices[1..] {
            self.current_fast = update_ema(self.current_fast, price, self.alpha_fast);
            if !self.current_fast.is_finite() {
                bail!("Fast EMA calculation failed");
            }
            self.fast_history.push_back(self.current_fast);
            if self.fast_history.len() > self.history_size {
                self.fast_history.pop_front();
            }

            self.current_slow = update_ema(self.current_slow, price, self.alpha_slow);
            if !self.current_slow.is_finite() {
                bail!("Slow EMA calculation failed");
            }
            self.slow_history.push_back(self.current_slow);
            if self.slow_history.len() > self.history_size {
                self.slow_history.pop_front();
            }
        }

        Ok(())
    }

    /// Updates EMAs with new price.
    pub fn update(&mut self, price: f64) -> Result<()> {
        self.current_fast = update_ema(self.current_fast, price, self.alpha_fast);
        if !self.current_fast.is_finite() {
            bail!("Fast EMA update failed");
        }

        self.fast_history.push_back(self.current_fast);
        if self.fast_history.len() > self.history_size {
            self.fast_history.pop_front();
        }

        self.current_slow = update_ema(self.current_slow, price, self.alpha_slow);
        if !self.current_slow.is_finite() {
            bail!("Slow EMA update failed");
        }

        self.slow_history.push_back(self.current_slow);
        if self.slow_history.len() > self.history_size {
            self.slow_history.pop_front();
        }

        Ok(())
    }

    /// Generates trading signal based on EMA crossover with buffer.
    pub fn crossover_signal(&self) -> Signal {
        let diff = self.current_fast - self.current_slow;

        if diff > self.signal_buffer {
            Signal::Long
        } else if diff < -self.signal_buffer {
            Signal::Short
        } else {
            Signal::Neutral
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_ema() {
        assert_eq!(update_ema(100.0, 105.0, 0.2), 101.0);
        assert!(update_ema(f64::NAN, 100.0, 0.2).is_nan());
        assert!(update_ema(100.0, 105.0, -0.1).is_nan());
        assert!(update_ema(100.0, 105.0, 1.1).is_nan());
    }

    #[test]
    fn test_ema_creation() {
        let ema = EMA::new(13, 34, 20, 2.0);
        assert_eq!(ema.fast_period, 13);
        assert_eq!(ema.slow_period, 34);
        assert_eq!(ema.history_size, 20);
        assert_eq!(ema.signal_buffer, 2.0);
        assert_eq!(ema.current_fast, 0.0);
        assert_eq!(ema.current_slow, 0.0);
        assert!((ema.alpha_fast - 2.0 / 14.0).abs() < 1e-10);
        assert!((ema.alpha_slow - 2.0 / 35.0).abs() < 1e-10);
    }

    #[test]
    fn test_initialization() {
        let mut ema = EMA::new(13, 34, 20, 2.0);
        let prices = vec![100.0, 102.0, 101.0, 103.0];

        assert!(ema.initialize(&prices).is_ok());
        assert_ne!(ema.current_fast, 0.0);
        assert_eq!(ema.fast_history.len(), 4);

        assert!(ema.initialize(&[]).is_err());
    }

    #[test]
    fn test_updates() {
        let mut ema = EMA::new(13, 34, 20, 2.0);
        ema.initialize(&[100.0, 101.0]).unwrap();

        let old_fast = ema.current_fast;
        assert!(ema.update(102.0).is_ok());
        assert_ne!(ema.current_fast, old_fast);

        let history_size = 5;
        let mut ema_small = EMA::new(13, 34, history_size, 2.0);
        let prices = vec![100.0; history_size + 3];
        ema_small.initialize(&prices).unwrap();
        assert_eq!(ema_small.fast_history.len(), history_size);
    }

    #[test]
    fn test_signals() {
        let mut ema = EMA::new(13, 34, 20, 2.0);

        ema.current_fast = 105.0;
        ema.current_slow = 100.0;
        assert_eq!(ema.crossover_signal(), Signal::Long);

        ema.current_fast = 100.0;
        ema.current_slow = 105.0;
        assert_eq!(ema.crossover_signal(), Signal::Short);

        ema.current_fast = 101.0;
        ema.current_slow = 100.0;
        assert_eq!(ema.crossover_signal(), Signal::Neutral);

        let mut ema_tight = EMA::new(13, 34, 20, 0.5);
        ema_tight.current_fast = 101.0;
        ema_tight.current_slow = 100.0;
        assert_eq!(ema_tight.crossover_signal(), Signal::Long);
    }
}
