//! Foreign exchange rate abstraction.

use std::sync::Arc;

/// Error type for FX rate operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FxError {
    /// Conversion between these two commodities is not available.
    #[error("FX conversion from {from} to {to} is not available")]
    Unavailable {
        /// Source commodity code.
        from: String,
        /// Target commodity code.
        to: String,
    },
}

/// Converts amounts between commodities.
pub trait FxRateService: Send + Sync {
    /// Convert `amount` to `to_commodity`.
    ///
    /// # Errors
    ///
    /// Returns [`FxError::Unavailable`] if conversion is not possible.
    fn convert(
        &self,
        amount: &bc_models::Amount,
        to_commodity: &bc_models::CommodityCode,
    ) -> Result<bc_models::Amount, FxError>;
}

/// Placeholder FX service — always returns [`FxError::Unavailable`] for cross-currency conversions.
#[non_exhaustive]
pub struct NoopFxRateService;

impl FxRateService for NoopFxRateService {
    #[inline]
    fn convert(
        &self,
        amount: &bc_models::Amount,
        to_commodity: &bc_models::CommodityCode,
    ) -> Result<bc_models::Amount, FxError> {
        if amount.commodity() == to_commodity {
            return Ok(amount.clone());
        }
        Err(FxError::Unavailable {
            from: amount.commodity().to_string(),
            to: to_commodity.to_string(),
        })
    }
}

/// Returns a [`NoopFxRateService`] wrapped in an [`Arc`].
#[must_use]
#[inline]
pub fn noop_fx() -> Arc<dyn FxRateService> {
    Arc::new(NoopFxRateService)
}
