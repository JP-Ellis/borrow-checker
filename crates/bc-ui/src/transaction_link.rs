//! Opening a saved transaction in an account register from any page.

use bc_ipc::WriteWarning;
use leptos::prelude::*;

use crate::components::toast::ToastAction;
use crate::components::transaction_row::Created;
use crate::filter_ctx::FilterStore;
use crate::pending_warnings::PendingWarnings;
use crate::pending_warnings::use_pending_warnings;
use crate::url_state::History;
use crate::url_state::transaction_link_state;

/// Opens a saved transaction in an account register, from any page.
///
/// Holds the context it needs, so a toast action can run it outside any
/// component.
#[derive(Clone, Copy)]
pub struct TransactionLink {
    /// Performs the navigation and supplies the query and window to keep.
    filter_store: FilterStore,
    /// Carries the warnings to the opened row.
    pending: PendingWarnings,
}

impl TransactionLink {
    /// Navigates to `account_id`'s register with `tx_id` open.
    ///
    /// The register opens the row once its rows load, showing `warnings` as
    /// its band, then drops `tx` from the URL.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The register to open.
    /// * `tx_id` - The transaction to open.
    /// * `date` - The transaction's date; a period window moves to cover it.
    /// * `warnings` - The warnings the row's band shows; empty for none.
    pub fn open_transaction_link(
        self,
        account_id: &str,
        tx_id: &str,
        date: jiff::civil::Date,
        warnings: Vec<WriteWarning>,
    ) {
        self.pending.set(tx_id.to_owned(), warnings);
        let next = transaction_link_state(&self.filter_store.current(), tx_id, date);
        self.filter_store
            .open(&format!("/accounts/{account_id}"), &next, History::Push);
    }

    /// The "View" action for a created transaction's toast, opening it in its
    /// first leg's account. `None` when it has no legs.
    ///
    /// # Arguments
    ///
    /// * `created` - The saved transaction.
    #[must_use]
    pub fn view_action(self, created: &Created) -> Option<ToastAction> {
        let account = created.accounts.first()?.clone();
        let id = created.id.clone();
        let date = created.date;
        let warnings = created.warnings.clone();
        // Built in the save's callback, under no reactive owner: the toast
        // outlives the register, and a callback owned by it would panic on a
        // late click.
        Some(ToastAction {
            label: "View".to_owned(),
            on_activate: Callback::new(move |()| {
                self.open_transaction_link(&account, &id, date, warnings.clone());
            }),
        })
    }
}

/// Reads the [`TransactionLink`] from context. Call during component setup.
#[must_use]
pub fn use_transaction_link() -> TransactionLink {
    TransactionLink {
        filter_store: crate::filter_ctx::use_filter_store(),
        pending: use_pending_warnings(),
    }
}
