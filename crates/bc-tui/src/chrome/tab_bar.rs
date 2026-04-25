//! Tab bar chrome component — renders the top navigation tabs.

use tuirealm::AttrValue;
use tuirealm::Attribute;
use tuirealm::Component;
use tuirealm::Frame;
use tuirealm::MockComponent;
use tuirealm::NoUserEvent;
use tuirealm::Props;
use tuirealm::State;
use tuirealm::StateValue;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::event::Event;
use tuirealm::event::Key;
use tuirealm::event::KeyEvent;
use tuirealm::props::Color;
use tuirealm::props::Style;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::widgets::Tabs;

use crate::msg::Msg;
use crate::msg::Tab;

/// Raw widget that renders the tab bar.
struct Widget {
    /// Component props storage.
    props: Props,
    /// Currently highlighted tab.
    active_tab: Tab,
}

impl Widget {
    /// Create a new tab bar with the given initially-active tab.
    #[inline]
    #[must_use]
    fn new(active_tab: Tab) -> Self {
        Self {
            props: Props::default(),
            active_tab,
        }
    }
}

impl MockComponent for Widget {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let selected = match self.active_tab {
            Tab::Accounts => 0,
            Tab::Budget => 1,
            Tab::Reports => 2,
        };
        let tabs = Tabs::new(vec!["Accounts", "Budget", "Reports"])
            .select(selected)
            .highlight_style(Style::default().fg(Color::Yellow))
            .style(Style::default().fg(Color::White));
        frame.render_widget(tabs, area);
    }

    #[inline]
    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    #[inline]
    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        // Update active tab when set externally.
        if attr == Attribute::Value {
            if let AttrValue::Payload(tuirealm::props::PropPayload::One(
                tuirealm::props::PropValue::Usize(idx),
            )) = &value
            {
                self.active_tab = match idx {
                    0 => Tab::Accounts,
                    1 => Tab::Budget,
                    _ => Tab::Reports,
                };
            }
        }
        self.props.set(attr, value);
    }

    #[inline]
    fn state(&self) -> State {
        State::One(StateValue::Usize(match self.active_tab {
            Tab::Accounts => 0,
            Tab::Budget => 1,
            Tab::Reports => 2,
        }))
    }

    #[inline]
    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::None
    }
}

/// Tui-realm component wrapper for the tab bar widget.
#[derive(MockComponent)]
#[non_exhaustive]
pub struct TabBar {
    /// Inner raw widget.
    component: Widget,
}

impl TabBar {
    /// Create a new `TabBar` showing the given tab as active.
    #[inline]
    #[must_use]
    pub fn new(active_tab: Tab) -> Self {
        Self {
            component: Widget::new(active_tab),
        }
    }
}

impl Component<Msg, NoUserEvent> for TabBar {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; remaining variants all produce None"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Char('q'),
                ..
            }) => Some(Msg::AppQuit),
            Event::Keyboard(KeyEvent {
                code: Key::Char('?'),
                ..
            }) => Some(Msg::HelpToggle),
            Event::Keyboard(KeyEvent {
                code: Key::Char('1'),
                ..
            }) => Some(Msg::TabSwitch(Tab::Accounts)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('2'),
                ..
            }) => Some(Msg::TabSwitch(Tab::Budget)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('3'),
                ..
            }) => Some(Msg::TabSwitch(Tab::Reports)),
            Event::Keyboard(KeyEvent { code: Key::Tab, .. }) => {
                Some(Msg::TabSwitch(match self.component.active_tab {
                    Tab::Accounts => Tab::Budget,
                    Tab::Budget => Tab::Reports,
                    Tab::Reports => Tab::Accounts,
                }))
            }
            Event::Keyboard(KeyEvent {
                code: Key::BackTab, ..
            }) => Some(Msg::TabSwitch(match self.component.active_tab {
                Tab::Accounts => Tab::Reports,
                Tab::Budget => Tab::Accounts,
                Tab::Reports => Tab::Budget,
            })),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use tuirealm::event::KeyModifiers;

    use super::*;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn tab_cycles_forward_from_accounts() {
        let mut bar = TabBar::new(Tab::Accounts);
        let msg = bar.on(key(Key::Tab));
        pretty_assertions::assert_eq!(msg, Some(Msg::TabSwitch(Tab::Budget)));
    }

    #[test]
    fn backtab_cycles_backward_from_accounts() {
        let mut bar = TabBar::new(Tab::Accounts);
        let msg = bar.on(key(Key::BackTab));
        pretty_assertions::assert_eq!(msg, Some(Msg::TabSwitch(Tab::Reports)));
    }

    #[test]
    fn numeric_key_switches_to_correct_tab() {
        let mut bar = TabBar::new(Tab::Accounts);
        pretty_assertions::assert_eq!(
            bar.on(key(Key::Char('2'))),
            Some(Msg::TabSwitch(Tab::Budget)),
        );
        pretty_assertions::assert_eq!(
            bar.on(key(Key::Char('3'))),
            Some(Msg::TabSwitch(Tab::Reports)),
        );
    }

    #[test]
    fn tab_cycles_from_budget_after_attr_update() {
        // After a TabSwitch to Budget is processed, attr() updates active_tab.
        // Tab from Budget should go to Reports, not back to Budget.
        let mut bar = TabBar::new(Tab::Accounts);
        bar.attr(
            Attribute::Value,
            AttrValue::Payload(tuirealm::props::PropPayload::One(
                tuirealm::props::PropValue::Usize(1),
            )),
        );
        let msg = bar.on(key(Key::Tab));
        pretty_assertions::assert_eq!(msg, Some(Msg::TabSwitch(Tab::Reports)));
    }
}
