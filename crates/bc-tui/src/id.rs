//! Component identifier types, namespaced by screen.
//!
//! [`Id`] is the key type for tui-realm's `Application<Id, Msg, UserEvent>`.
//! Adding a new screen requires one new variant here and a new `FooId` enum —
//! no existing screen is touched.

/// Top-level component identifier, namespaced by screen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Id {
    /// Chrome components — always mounted.
    Chrome(ChromeId),
    /// Accounts screen components.
    Accounts(AccountsId),
    /// Budget screen components.
    Budget(BudgetId),
    /// Reports screen components.
    Reports(ReportsId),
}

/// Identifiers for the permanent chrome components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChromeId {
    /// Tab bar across the top.
    TabBar,
    /// Status bar across the bottom.
    StatusBar,
    /// Help overlay (floating popup, hidden until `?`).
    HelpOverlay,
}

/// Identifiers for the accounts screen components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccountsId {
    /// Account tree sidebar.
    Sidebar,
    /// Transaction list in the main panel.
    TransactionList,
    /// Transaction detail panel.
    TransactionDetail,
    /// Transaction add/edit form overlay.
    TransactionForm,
}

/// Identifiers for the budget screen components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BudgetId {
    /// Envelope tree sidebar.
    Sidebar,
    /// Envelope status detail panel.
    Detail,
}

/// Identifiers for the reports screen components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReportsId {
    /// Combined report selector and output view.
    View,
}
