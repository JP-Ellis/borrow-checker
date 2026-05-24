//! Type conversions between `bc_models` and `bc_ipc` at the Tauri IPC boundary.
//!
//! Neither [`IntoIpc`] nor [`IntoModel`] can use the standard [`From`] trait
//! because both sides of each conversion are defined in external crates (the
//! Rust orphan rule). The extension-trait pattern is the idiomatic alternative.

use rust_decimal::prelude::ToPrimitive as _;

// MARK: Traits

/// Converts a `bc_models` type into its `bc_ipc` counterpart.
#[expect(
    clippy::allow_attributes,
    reason = "trait is used through impl blocks and test extension method calls, but clippy cannot detect it; #[allow] is necessary"
)]
#[allow(
    dead_code,
    reason = "trait is used through impl blocks and test extension method calls"
)]
pub(crate) trait IntoIpc {
    /// The IPC counterpart type.
    type Output;
    /// Convert `self` into its IPC representation.
    fn into_ipc(self) -> Self::Output;
}

/// Converts a `bc_ipc` type back into its `bc_models` counterpart.
#[expect(
    clippy::allow_attributes,
    reason = "trait is used through impl blocks and test extension method calls, but clippy cannot detect it; #[allow] is necessary"
)]
#[allow(
    dead_code,
    reason = "trait is used through impl blocks and test extension method calls"
)]
pub(crate) trait IntoModel {
    /// The domain model counterpart type.
    type Output;
    /// Convert `self` into its domain model representation.
    fn into_model(self) -> Self::Output;
}

// MARK: Amount

impl IntoIpc for &bc_models::Amount {
    type Output = bc_ipc::Amount;

    /// Converts a [`bc_models::Amount`] to an IPC [`bc_ipc::Amount`].
    ///
    /// Reads scale directly from the decimal value — no currency lookup required.
    /// Multiplies by `10 ^ scale` and rounds using midpoint-nearest-even.
    #[inline]
    fn into_ipc(self) -> bc_ipc::Amount {
        let code = self.commodity().as_str();
        let scale = self.value().scale();
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal multiplication by a power-of-ten scale factor is bounded by the IPC contract and safe in practice"
        )]
        let minor = (self.value() * rust_decimal::Decimal::from(10_u64.pow(scale)))
            .round()
            .to_i64()
            .unwrap_or(0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Decimal::scale() returns u32 representing decimal places, which is always ≤ 28 for rust_decimal and fits in u8"
        )]
        #[expect(
            clippy::as_conversions,
            reason = "Decimal::scale() returns u32 representing decimal places, which is always ≤ 28 for rust_decimal and fits in u8"
        )]
        bc_ipc::Amount::new(minor, code, scale as u8)
    }
}

impl IntoModel for &bc_ipc::Amount {
    type Output = bc_models::Amount;

    /// Converts an IPC [`bc_ipc::Amount`] to a [`bc_models::Amount`].
    ///
    /// Uses the stored `scale` directly — no currency lookup required.
    /// `Decimal::new(mantissa, scale)` constructs `mantissa / 10^scale` exactly.
    #[inline]
    fn into_model(self) -> bc_models::Amount {
        let value = rust_decimal::Decimal::new(self.minor_units, u32::from(self.scale));
        bc_models::Amount::new(
            value,
            bc_models::CommodityCode::new(self.currency_code.clone()),
        )
    }
}

// MARK: AccountType

impl IntoIpc for bc_models::AccountType {
    type Output = bc_ipc::AccountType;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::AccountType and bc_ipc::AccountType are #[non_exhaustive]; \
                  the wildcard fallback to Asset is intentional for future unknown variants"
    )]
    fn into_ipc(self) -> bc_ipc::AccountType {
        match self {
            bc_models::AccountType::Asset => bc_ipc::AccountType::Asset,
            bc_models::AccountType::Liability => bc_ipc::AccountType::Liability,
            bc_models::AccountType::Equity => bc_ipc::AccountType::Equity,
            bc_models::AccountType::Income => bc_ipc::AccountType::Income,
            bc_models::AccountType::Expense => bc_ipc::AccountType::Expense,
            _ => bc_ipc::AccountType::Asset,
        }
    }
}

// MARK: TransactionStatus / TxStatus

impl IntoIpc for bc_models::TransactionStatus {
    type Output = bc_ipc::TxStatus;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::TransactionStatus and bc_ipc::TxStatus are #[non_exhaustive]; \
                  Voided is kept explicit even though the wildcard fallback also maps to Unreconciled"
    )]
    fn into_ipc(self) -> bc_ipc::TxStatus {
        match self {
            bc_models::TransactionStatus::Cleared => bc_ipc::TxStatus::Cleared,
            bc_models::TransactionStatus::Pending => bc_ipc::TxStatus::Pending,
            bc_models::TransactionStatus::Voided => bc_ipc::TxStatus::Unreconciled,
            _ => bc_ipc::TxStatus::Unreconciled,
        }
    }
}

impl IntoModel for bc_ipc::TxStatus {
    type Output = bc_models::TransactionStatus;

    #[inline]
    fn into_model(self) -> bc_models::TransactionStatus {
        match self {
            bc_ipc::TxStatus::Cleared => bc_models::TransactionStatus::Cleared,
            bc_ipc::TxStatus::Pending | bc_ipc::TxStatus::Unreconciled => {
                bc_models::TransactionStatus::Pending
            }
            _ => bc_models::TransactionStatus::Pending,
        }
    }
}

// MARK: Account

impl IntoIpc for &bc_models::Account {
    type Output = bc_ipc::AccountNode;

    #[inline]
    fn into_ipc(self) -> bc_ipc::AccountNode {
        bc_ipc::AccountNode::new(
            self.id().to_string(),
            self.name(),
            None::<&str>,
            bc_ipc::Amount::new(0, "AUD", 2), // TODO(ipc): compute via BalanceEngine
            self.parent_id().map(ToString::to_string),
            self.account_type().into_ipc(),
            vec![],
        )
    }
}

// MARK: Posting

impl IntoIpc for &bc_models::Posting {
    type Output = bc_ipc::Posting;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Posting {
        let account_id = self.account_id().to_string();
        bc_ipc::Posting::new(
            account_id.clone(),
            account_id, // TODO(ipc): resolve display path via AccountService
            self.amount().into_ipc(),
            self.memo(),
        )
    }
}

// MARK: Transaction

impl IntoIpc for &bc_models::Transaction {
    type Output = bc_ipc::Transaction;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Transaction {
        let postings: Vec<bc_ipc::Posting> =
            self.postings().iter().map(IntoIpc::into_ipc).collect();
        bc_ipc::Transaction::new(
            self.id().to_string(),
            self.date().to_string(),
            self.payee().unwrap_or_default(),
            self.status().into_ipc(),
            vec![], // TODO(ipc): resolve tag paths via TagService
            postings,
            vec![],
        )
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::IntoIpc as _;
    use super::IntoModel as _;

    #[test]
    fn amount_into_ipc_aud() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(1050, 2), // 10.50
            bc_models::CommodityCode::new("AUD"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.minor_units, 1050);
        assert_eq!(ipc.currency_code, "AUD");
        assert_eq!(ipc.scale, 2);
    }

    #[test]
    fn amount_into_model_aud() {
        let ipc = bc_ipc::Amount::new(1050, "AUD", 2);
        let model = (&ipc).into_model();
        assert_eq!(model.value(), rust_decimal::Decimal::new(1050, 2));
        assert_eq!(model.commodity().as_str(), "AUD");
    }

    #[test]
    fn amount_round_trip_jpy() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(1234, 0),
            bc_models::CommodityCode::new("JPY"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.minor_units, 1234);
        assert_eq!(ipc.scale, 0);
        let back = (&ipc).into_model();
        assert_eq!(back, model);
    }

    #[test]
    fn amount_round_trip_btc() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(12345, 8), // 0.00012345 BTC
            bc_models::CommodityCode::new("BTC"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.minor_units, 12345);
        assert_eq!(ipc.scale, 8);
        let back = (&ipc).into_model();
        assert_eq!(back, model);
    }
}
