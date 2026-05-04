//! Envelope status detail panel component.
//!
//! Displays the full budget status of a selected envelope (read-only),
//! or a placeholder when no envelope is selected.

use bc_core::EnvelopeStatus;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::component::AppComponent;
use tuirealm::component::Component;
use tuirealm::event::Event;
use tuirealm::event::NoUserEvent;
use tuirealm::props::AttrValue;
use tuirealm::props::Attribute;
use tuirealm::props::Props;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::text::Span;
use tuirealm::ratatui::text::Text;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;
use tuirealm::state::State;

use crate::msg::Msg;

// MARK: private component

/// Raw widget that renders the envelope status detail panel.
struct Detail {
    /// Component props storage.
    props: Props,
    /// Envelope status to display, if any.
    status: Option<EnvelopeStatus>,
}

impl Detail {
    /// Returns the ratatui color for a progress ratio.
    #[inline]
    #[must_use]
    fn bar_color(pct: f64) -> Color {
        if pct < 0.85_f64 {
            Color::Green
        } else if pct <= 1.0_f64 {
            Color::Yellow
        } else {
            Color::Red
        }
    }

    /// Build the `▼` marker line indicating the elapsed fraction of the period.
    ///
    /// The `▼` is positioned at `[` + `elapsed × bar_width` characters.
    ///
    /// # Arguments
    ///
    /// * `elapsed` - Fraction of the period elapsed, in `[0.0, 1.0]`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::float_arithmetic,
        reason = "progress bar uses intentional lossy float-to-usize cast on a clamped [0,1] value"
    )]
    fn build_marker_line(elapsed: f64) -> Line<'static> {
        let bar_width: usize = 30;
        let marker_pos = (elapsed.clamp(0.0_f64, 1.0_f64) * bar_width as f64).round() as usize;
        // "[" occupies position 0; bar starts at position 1.
        let spaces = " ".repeat(marker_pos.saturating_add(1));
        Line::from(format!("{spaces}\u{25bc}")) // ▼
    }

    /// Build the colored progress bar [`Line`] for `pct`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::float_arithmetic,
        reason = "progress bar uses intentional lossy float-to-usize cast on a clamped [0,1] value"
    )]
    fn build_bar_line(pct: f64) -> Line<'static> {
        let bar_width: usize = 30;
        let filled = (pct.clamp(0.0_f64, 1.0_f64) * bar_width as f64).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        let fill_color = Self::bar_color(pct);
        let mut spans = vec![Span::raw("[")];
        if filled > 0 {
            spans.push(Span::styled(
                "\u{2588}".repeat(filled), // █
                Style::default().fg(fill_color),
            ));
        }
        if empty > 0 {
            spans.push(Span::raw("\u{2591}".repeat(empty))); // ░
        }
        spans.push(Span::raw(format!("] {:.1}%", pct * 100.0_f64)));
        Line::from(spans)
    }

    /// Format the envelope status as ratatui [`Text`].
    ///
    /// Returns the placeholder text when no status is set.
    #[inline]
    #[must_use]
    fn render_lines(&self) -> Text<'static> {
        let Some(s) = &self.status else {
            return Text::from("Select an envelope to see budget status.".to_owned());
        };

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "division guarded by is_zero() check; string-formatted Decimal parses to f64"
        )]
        let pct_f64: f64 = if s.allocated.is_zero() {
            0.0_f64
        } else {
            format!("{}", s.actuals / s.allocated)
                .parse::<f64>()
                .unwrap_or(0.0_f64)
                .max(0.0_f64)
        };

        let commodity_suffix = s
            .commodity
            .as_ref()
            .map_or_else(String::new, |c| format!(" {c}"));

        // Time-elapsed fraction — only when window contains today.
        let today = jiff::Zoned::now().date();
        #[expect(
            clippy::arithmetic_side_effects,
            clippy::as_conversions,
            clippy::cast_precision_loss,
            clippy::float_arithmetic,
            reason = "elapsed days within bounded window; Date subtraction and as-f64 cast are intentional"
        )]
        let elapsed: Option<f64> = (today >= s.window.start && today < s.window.end)
            .then(|| {
                let window_days = s.window.days();
                (window_days > 0).then(|| {
                    let elapsed_days = i64::from((today - s.window.start).get_days());
                    (elapsed_days as f64) / (window_days as f64)
                })
            })
            .flatten();

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Header: name + period label
        lines.push(Line::from(format!(
            "{}  [\u{2190} {} \u{2192}]", // ← label →
            s.envelope.name(),
            s.window.label
        )));
        lines.push(Line::from("\u{2500}".repeat(35)));
        lines.push(Line::from(format!(
            "Allocated:  {}{}",
            s.allocated, commodity_suffix
        )));
        lines.push(Line::from(format!(
            "Spent:      {}{}",
            s.actuals, commodity_suffix
        )));
        lines.push(Line::from(format!(
            "Available:  {}{}",
            s.available, commodity_suffix
        )));
        lines.push(Line::from(""));

        // Time-elapsed marker (only for current windows)
        if let Some(e) = elapsed {
            lines.push(Self::build_marker_line(e));
        }
        lines.push(Self::build_bar_line(pct_f64));

        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "Rollover:   {:?}",
            s.envelope.rollover_policy()
        )));
        lines.push(Line::from(format!(
            "Period:     {} \u{2013} {}",
            s.period_start, s.period_end
        )));

        Text::from(lines)
    }
}

impl Component for Detail {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .cloned()
            .and_then(|v| {
                if let AttrValue::Flag(b) = v {
                    Some(b)
                } else {
                    None
                }
            })
            .unwrap_or(false);

        let border_color = if focused { Color::Cyan } else { Color::White };
        let content = self.render_lines();
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
    fn query(&self, attr: Attribute) -> Option<tuirealm::props::QueryResult<'_>> {
        self.props.get_for_query(attr)
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
        CmdResult::NoChange
    }
}

// MARK: public wrapper

/// Tui-realm component wrapper for the envelope status detail panel widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as detail::EnvelopeDetail; repetition is intentional"
)]
#[non_exhaustive]
#[derive(Component)]
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

impl AppComponent<Msg, NoUserEvent> for EnvelopeDetail {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; remaining variants all produce None"
    )]
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        use tuirealm::event::Key;
        use tuirealm::event::KeyEvent;
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Left | Key::Char('h') | Key::Esc,
                ..
            }) => Some(Msg::Budget(crate::msg::BudgetMsg::FocusSidebar)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('['),
                ..
            }) => Some(Msg::Budget(crate::msg::BudgetMsg::PeriodPrev)),
            Event::Keyboard(KeyEvent {
                code: Key::Char(']'),
                ..
            }) => Some(Msg::Budget(crate::msg::BudgetMsg::PeriodNext)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('a'),
                ..
            }) => Some(Msg::Budget(crate::msg::BudgetMsg::OpenAllocate)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tuirealm::ratatui::style::Color;

    use super::*;

    #[test]
    fn render_content_no_status_returns_placeholder() {
        let d = Detail {
            props: Props::default(),
            status: None,
        };
        let text = d.render_lines();
        // Single line containing the placeholder text
        assert_eq!(text.lines.len(), 1);
        let first = text.lines.first().expect("one line present");
        let line_str: String = first.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(line_str, "Select an envelope to see budget status.");
    }

    #[test]
    fn bar_color_green_below_85() {
        assert_eq!(Detail::bar_color(0.5_f64), Color::Green);
        assert_eq!(Detail::bar_color(0.84_f64), Color::Green);
    }

    #[test]
    fn bar_color_yellow_at_85_to_100() {
        assert_eq!(Detail::bar_color(0.85_f64), Color::Yellow);
        assert_eq!(Detail::bar_color(1.0_f64), Color::Yellow);
    }

    #[test]
    fn bar_color_red_above_100() {
        assert_eq!(Detail::bar_color(1.01_f64), Color::Red);
        assert_eq!(Detail::bar_color(2.0_f64), Color::Red);
    }

    #[test]
    fn marker_line_position() {
        // elapsed = 0.5, bar_width = 30 → marker at position 15
        // "[" (1 char) + 15 chars = 16 chars before "▼"
        let line = Detail::build_marker_line(0.5_f64);
        let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Find the ▼ and check it's at position 16 (0-indexed)
        let pos = content.find('\u{25bc}').expect("marker present");
        assert_eq!(pos, 16);
    }
}
