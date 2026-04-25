//! Envelope status detail panel component.
//!
//! Displays the full budget status of a selected envelope (read-only),
//! or a placeholder when no envelope is selected.

use bc_core::EnvelopeStatus;
use tuirealm::AttrValue;
use tuirealm::Attribute;
use tuirealm::Component;
use tuirealm::Frame;
use tuirealm::MockComponent;
use tuirealm::NoUserEvent;
use tuirealm::Props;
use tuirealm::State;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::event::Event;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;

use crate::msg::Msg;

// ─── private component ───────────────────────────────────────────────────────

/// Raw widget that renders the envelope status detail panel.
struct Detail {
    /// Component props storage.
    props: Props,
    /// Envelope status to display, if any.
    status: Option<EnvelopeStatus>,
}

impl Detail {
    /// Format the envelope status as a multi-line string.
    ///
    /// If no status is set, returns the placeholder text.
    /// Otherwise formats: envelope name, period, allocated/spent/available
    /// amounts, a progress bar, rollover policy, and period dates.
    #[inline]
    fn render_content(&self) -> String {
        let Some(s) = &self.status else {
            return "Select an envelope to see budget status.".to_owned();
        };

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "division guarded by is_zero() check; string-formatted Decimal always parses as f64"
        )]
        let pct_f64: f64 = if s.allocated.is_zero() {
            0.0_f64
        } else {
            format!("{}", s.actuals / s.allocated)
                .parse::<f64>()
                .unwrap_or(0.0_f64)
                .clamp(0.0_f64, 1.0_f64)
        };

        let bar = Self::build_progress_bar(pct_f64);

        let commodity_suffix = s
            .commodity
            .as_ref()
            .map_or_else(String::new, |c| format!(" {c}"));

        let mut lines = Vec::new();
        lines.push(format!(
            "{}  ({:?})",
            s.envelope.name(),
            s.envelope.period()
        ));
        lines.push("─".repeat(35));
        lines.push(format!("Allocated:  {}{}", s.allocated, commodity_suffix));
        lines.push(format!("Spent:      {}{}", s.actuals, commodity_suffix));
        lines.push(format!("Available:  {}{}", s.available, commodity_suffix));
        lines.push(String::new());
        lines.push(bar);
        lines.push(String::new());
        lines.push(format!("Rollover:   {:?}", s.envelope.rollover_policy()));
        lines.push(format!(
            "Period:     {} \u{2013} {}",
            s.period_start, s.period_end
        ));

        lines.join("\n")
    }

    /// Build a 20-character ASCII progress bar string for the given ratio.
    ///
    /// # Arguments
    ///
    /// * `pct_f64` - Ratio in `[0.0, 1.0]`.
    ///
    /// # Returns
    ///
    /// A string of the form `[████░░░░░░░░░░░░░░░░] 40.0%`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::float_arithmetic,
        reason = "progress bar uses intentional lossy float-to-usize cast on a clamped [0,1] ratio; bar_width is always 20"
    )]
    #[inline]
    #[must_use]
    fn build_progress_bar(pct_f64: f64) -> String {
        let bar_width: usize = 20;
        let filled = (pct_f64 * bar_width as f64).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        format!(
            "[{}{}] {:.1}%",
            "█".repeat(filled),
            "░".repeat(empty),
            pct_f64 * 100.0_f64
        )
    }
}

impl MockComponent for Detail {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .and_then(|v| {
                if let AttrValue::Flag(b) = v {
                    Some(b)
                } else {
                    None
                }
            })
            .unwrap_or(false);

        let border_color = if focused { Color::Cyan } else { Color::White };
        let content = self.render_content();
        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Envelope Status ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color)),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    #[inline]
    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    #[inline]
    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    #[inline]
    fn state(&self) -> State {
        State::None
    }

    #[inline]
    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::None
    }
}

// ─── public wrapper ──────────────────────────────────────────────────────────

/// Tui-realm component wrapper for the envelope status detail panel widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as detail::EnvelopeDetail; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct EnvelopeDetail {
    /// Inner raw widget.
    component: Detail,
}

impl EnvelopeDetail {
    /// Create a new `EnvelopeDetail` showing the given envelope status, or empty if `None`.
    ///
    /// # Arguments
    ///
    /// * `status` - Optional envelope status to display, or `None` if no envelope is selected.
    ///
    /// # Returns
    ///
    /// A new `EnvelopeDetail` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(status: Option<EnvelopeStatus>) -> Self {
        Self {
            component: Detail {
                props: Props::default(),
                status,
            },
        }
    }

    /// Update the displayed envelope status.
    ///
    /// # Arguments
    ///
    /// * `status` - New envelope status to display, or `None` to show the placeholder.
    #[inline]
    pub fn set_status(&mut self, status: Option<EnvelopeStatus>) {
        self.component.status = status;
    }
}

impl Component<Msg, NoUserEvent> for EnvelopeDetail {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn render_content_no_status_returns_placeholder() {
        let d = Detail {
            props: Props::default(),
            status: None,
        };
        assert_eq!(
            d.render_content(),
            "Select an envelope to see budget status."
        );
    }

    #[test]
    fn progress_bar_at_zero_percent() {
        let bar = Detail::build_progress_bar(0.0);
        assert_eq!(bar, "[░░░░░░░░░░░░░░░░░░░░] 0.0%");
    }

    #[test]
    fn progress_bar_at_half() {
        let bar = Detail::build_progress_bar(0.5);
        assert_eq!(bar, "[██████████░░░░░░░░░░] 50.0%");
    }

    #[test]
    fn progress_bar_at_full() {
        let bar = Detail::build_progress_bar(1.0);
        assert_eq!(bar, "[████████████████████] 100.0%");
    }
}
