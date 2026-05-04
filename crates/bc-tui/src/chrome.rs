//! Chrome layer — permanent components mounted at application startup.
//!
//! Chrome components (`TabBar`, `StatusBar`, `HelpOverlay`) are mounted once in
//! `main()` and remain mounted for the entire application lifetime. They
//! occupy fixed rows at the top and bottom of the terminal.

pub mod help_overlay;
pub mod status_bar;
pub mod tab_bar;

use tuirealm::application::Application;
use tuirealm::event::Key;
use tuirealm::event::NoUserEvent;
use tuirealm::subscription::EventClause;
use tuirealm::subscription::Sub;
use tuirealm::subscription::SubClause;

use crate::id::AccountsId;
use crate::id::BudgetId;
use crate::id::ChromeId;
use crate::id::Id;
use crate::msg::Msg;
use crate::msg::Tab;

/// Mounts all chrome components into `app`.
///
/// Call this once during application startup, before the main loop begins.
///
/// # Errors
///
/// Returns an error if any component fails to mount (e.g., duplicate ID).
pub fn mount(app: &mut Application<Id, Msg, NoUserEvent>, active_tab: Tab) -> anyhow::Result<()> {
    // TabBar receives global navigation keys via subscriptions so that these
    // shortcuts work even when a screen component has keyboard focus.
    app.mount(
        Id::Chrome(ChromeId::TabBar),
        Box::new(tab_bar::TabBar::new(active_tab)),
        vec![
            Sub::new(
                EventClause::Keyboard(Key::Char('q').into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::Char('?').into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::Char('1').into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::Char('2').into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::Char('3').into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::Tab.into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
            Sub::new(
                EventClause::Keyboard(Key::BackTab.into()),
                SubClause::And(
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(
                        Id::Accounts(AccountsId::TransactionForm),
                    )))),
                    Box::new(SubClause::Not(Box::new(SubClause::IsMounted(Id::Budget(
                        BudgetId::AllocationForm,
                    ))))),
                ),
            ),
        ],
    )?;
    app.mount(
        Id::Chrome(ChromeId::StatusBar),
        Box::new(status_bar::StatusBar::new()),
        vec![],
    )?;
    app.mount(
        Id::Chrome(ChromeId::HelpOverlay),
        Box::new(help_overlay::HelpOverlay::new()),
        vec![],
    )?;
    Ok(())
}
