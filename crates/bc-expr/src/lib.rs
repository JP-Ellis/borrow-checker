//! Evaluates amount expressions.
//!
//! The grammar is Beancount's: a literal is digits with optional comma
//! separators between them and an optional fraction; unary `+` and `-`;
//! binary `+`, `-`, `*` and `/` with the usual precedence; parentheses;
//! whitespace between tokens. `1,234,567.89`, `(1,000,000 * 3 / 100 / 365)`
//! and `-1,000 + 2.50` all evaluate to one [`Decimal`].
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use rust_decimal::Decimal;

/// The reason given when a value or a result exceeds [`Decimal`]'s range.
const OVERFLOW: &str = "overflow";

/// Why an expression did not evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError(String);

impl ExprError {
    /// The human-readable reason, naming the offending token when there is one.
    #[inline]
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }

    /// Whether a literal or an intermediate result exceeded [`Decimal`]'s
    /// range, as opposed to text that does not parse.
    #[inline]
    #[must_use]
    pub fn is_overflow(&self) -> bool {
        self.0 == OVERFLOW
    }
}

impl core::fmt::Display for ExprError {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExprError {}

/// Evaluates `raw` to one [`Decimal`].
///
/// # Errors
///
/// Returns [`ExprError`] when `raw` does not parse, divides by zero, or
/// overflows.
#[inline]
pub fn evaluate(raw: &str) -> Result<Decimal, ExprError> {
    let mut cursor = Cursor { text: raw, pos: 0 };
    cursor.skip_space();
    let value = cursor.sum().map_err(ExprError)?;
    cursor.skip_space();
    if cursor.pos < raw.len() {
        return Err(ExprError(format!(
            "unexpected '{}' after the number",
            cursor.rest()
        )));
    }
    Ok(value)
}

/// A position in the expression text.
struct Cursor<'a> {
    /// The full expression text.
    text: &'a str,
    /// The byte offset of the next unread character.
    pos: usize,
}

impl Cursor<'_> {
    /// The unread text.
    fn rest(&self) -> &str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    /// The next unread character, if any.
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Advances past any whitespace.
    fn skip_space(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.text.len().saturating_sub(trimmed.len());
    }

    /// Consumes `c` when it is next, after any whitespace.
    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        if self.peek() == Some(c) {
            self.pos = self.pos.saturating_add(c.len_utf8());
            true
        } else {
            false
        }
    }

    /// Describes the unread text for an error message.
    fn here(&self) -> String {
        let rest = self.rest();
        if rest.is_empty() {
            "end of input".to_owned()
        } else {
            format!("'{rest}'")
        }
    }

    /// `term (('+' | '-') term)*`.
    fn sum(&mut self) -> Result<Decimal, String> {
        let mut value = self.product()?;
        loop {
            if self.eat('+') {
                let rhs = self.product()?;
                value = value.checked_add(rhs).ok_or(OVERFLOW)?;
            } else if self.eat('-') {
                let rhs = self.product()?;
                value = value.checked_sub(rhs).ok_or(OVERFLOW)?;
            } else {
                return Ok(value);
            }
        }
    }

    /// `factor (('*' | '/') factor)*`.
    fn product(&mut self) -> Result<Decimal, String> {
        let mut value = self.factor()?;
        loop {
            if self.eat('*') {
                let rhs = self.factor()?;
                value = value.checked_mul(rhs).ok_or(OVERFLOW)?;
            } else if self.eat('/') {
                let rhs = self.factor()?;
                if rhs.is_zero() {
                    return Err("division by zero".to_owned());
                }
                value = value.checked_div(rhs).ok_or(OVERFLOW)?;
            } else {
                return Ok(value);
            }
        }
    }

    /// `('+' | '-') factor | '(' sum ')' | literal`.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "negating a Decimal cannot overflow"
    )]
    fn factor(&mut self) -> Result<Decimal, String> {
        if self.eat('-') {
            return Ok(-self.factor()?);
        }
        if self.eat('+') {
            return self.factor();
        }
        if self.eat('(') {
            let value = self.sum()?;
            if !self.eat(')') {
                return Err(format!("expected ')' at {}", self.here()));
            }
            return Ok(value);
        }
        self.literal()
    }

    /// Digits with optional commas between them, then an optional fraction.
    ///
    /// Beancount's lexer accepts any comma between two digits, so a group
    /// need not be three digits wide; the commas are dropped.
    fn literal(&mut self) -> Result<Decimal, String> {
        self.skip_space();
        let rest = self.rest();
        let mut digits = String::new();
        let mut end = 0;
        let mut chars = rest.char_indices().peekable();
        while let Some(&(index, c)) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else if c == ',' && !digits.is_empty() {
                // A comma is a separator only when a digit follows it.
                let follows_digit = rest
                    .get(index.saturating_add(1)..)
                    .and_then(|s| s.chars().next())
                    .is_some_and(|next| next.is_ascii_digit());
                if !follows_digit {
                    break;
                }
            } else {
                break;
            }
            end = index.saturating_add(c.len_utf8());
            chars.next();
        }
        if digits.is_empty() {
            return Err(format!("expected a number at {}", self.here()));
        }
        if let Some(fraction) = rest.get(end..).and_then(|s| s.strip_prefix('.')) {
            digits.push('.');
            end = end.saturating_add(1);
            for c in fraction.chars().take_while(char::is_ascii_digit) {
                digits.push(c);
                end = end.saturating_add(1);
            }
        }
        self.pos = self.pos.saturating_add(end);
        digits
            .parse::<Decimal>()
            .map_err(|_overflow| OVERFLOW.to_owned())
    }
}

/// Returns `true` when `raw` is a plain number with no operator.
///
/// A leading sign and comma digit groups are allowed, so `-1,000.50` is a
/// literal while `(42)` and `100 * 2` are expressions. Text that does not
/// evaluate at all is not a literal.
#[inline]
#[must_use]
pub fn is_literal(raw: &str) -> bool {
    has_literal_shape(raw) && evaluate(raw).is_ok()
}

/// Returns `true` when `raw` holds only an optional sign, digits, commas and
/// dots, without checking that it evaluates.
#[inline]
#[must_use]
pub fn has_literal_shape(raw: &str) -> bool {
    let trimmed = raw.trim();
    let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    !unsigned.is_empty()
        && unsigned
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
}

/// Evaluates `raw` and returns the expression text beside the value.
///
/// The text is `None` for a literal and the trimmed input otherwise, so
/// callers store the same expression text for the same input.
///
/// # Errors
///
/// Returns [`ExprError`] when `raw` does not evaluate.
#[inline]
pub fn split(raw: &str) -> Result<(Decimal, Option<String>), ExprError> {
    let value = evaluate(raw)?;
    let expr = (!has_literal_shape(raw)).then(|| raw.trim().to_owned());
    Ok((value, expr))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest]
    #[case("1234567.89", dec!(1234567.89))]
    #[case("1,234,567.89", dec!(1234567.89))]
    #[case("1,000,000", dec!(1000000))]
    #[case("-1,000.50", dec!(-1000.50))]
    #[case("+25", dec!(25))]
    #[case("10.", dec!(10))]
    #[case("0.005", dec!(0.005))]
    #[case("  42  ", dec!(42))]
    fn reads_a_literal(#[case] raw: &str, #[case] expected: Decimal) {
        assert_eq!(evaluate(raw), Ok(expected));
    }

    #[rstest]
    #[case("(1,000,000 * 3 / 100 / 365)", dec!(82.19178082191780821917808219))]
    #[case("(-1,200.00 + 350.00)", dec!(-850.00))]
    #[case("(-1,200.00 + 1,000.00 - 50.00)", dec!(-250.00))]
    #[case("(120.00 - 45.50)", dec!(74.50))]
    #[case("(90 / 4)", dec!(22.5))]
    #[case("100 * 2", dec!(200))]
    #[case("1 + 2 * 3", dec!(7))]
    #[case("(1 + 2) * 3", dec!(9))]
    #[case("10 - 2 - 3", dec!(5))]
    #[case("100 / 10 / 2", dec!(5))]
    #[case("-(1 + 2)", dec!(-3))]
    #[case("- 5", dec!(-5))]
    #[case("((7))", dec!(7))]
    fn evaluates_an_expression(#[case] raw: &str, #[case] expected: Decimal) {
        assert_eq!(evaluate(raw), Ok(expected));
    }

    #[rstest]
    #[case("", "expected a number at end of input")]
    #[case("abc", "expected a number at 'abc'")]
    #[case("1,000 AUD", "unexpected 'AUD' after the number")]
    #[case("(1 + 2", "expected ')' at end of input")]
    #[case("1 + 2)", "unexpected ')' after the number")]
    #[case("1 +", "expected a number at end of input")]
    #[case("1 / 0", "division by zero")]
    #[case("1,000,", "unexpected ',' after the number")]
    #[case(",1", "expected a number at ',1'")]
    #[case("1 ** 2", "expected a number at '* 2'")]
    #[case("(100,000 * 6.00 / 100 / 365", "expected ')' at end of input")]
    #[case("12 AUD", "unexpected 'AUD' after the number")]
    fn rejects_a_malformed_expression(#[case] raw: &str, #[case] reason: &str) {
        assert_eq!(
            evaluate(raw).map_err(|e| e.message().to_owned()),
            Err(reason.to_owned())
        );
    }

    #[test]
    fn reports_overflow() {
        let big = "9".repeat(20);
        assert_eq!(
            evaluate(&format!("{big} * {big}")).map_err(|e| e.message().to_owned()),
            Err("overflow".to_owned())
        );
    }

    #[rstest]
    #[case("99999999999999999999999999999", true)]
    #[case("99999999999999999999 * 99999999999999999999", true)]
    #[case("1.2.3", false)]
    #[case("abc", false)]
    fn tells_overflow_from_bad_text(#[case] raw: &str, #[case] overflow: bool) {
        assert_eq!(evaluate(raw).map_err(|e| e.is_overflow()), Err(overflow));
    }

    #[rstest]
    #[case("1234.56", true)]
    #[case("1,000,000", true)]
    #[case("-1,000.50", true)]
    #[case("+25", true)]
    #[case("  42  ", true)]
    #[case("10.", true)]
    #[case("(42)", false)]
    #[case("100 * 2", false)]
    #[case("1 + 2", false)]
    #[case("-(1 + 2)", false)]
    #[case("", false)]
    #[case("abc", false)]
    #[case("1..2", false)]
    fn classifies_literals(#[case] raw: &str, #[case] expected: bool) {
        assert_eq!(is_literal(raw), expected);
    }

    #[rstest]
    #[case("250.00", dec!(250.00), None)]
    #[case("1,000", dec!(1000), None)]
    #[case("-500", dec!(-500), None)]
    #[case("(30.00 / 4)", dec!(7.5), Some("(30.00 / 4)"))]
    #[case("  (30.00 / 4)  ", dec!(7.5), Some("(30.00 / 4)"))]
    fn split_separates_literal_from_expression(
        #[case] raw: &str,
        #[case] value: Decimal,
        #[case] expr: Option<&str>,
    ) {
        let (v, e) = split(raw).expect("evaluates");
        assert_eq!(v, value);
        assert_eq!(e.as_deref(), expr);
    }

    #[test]
    fn split_passes_the_evaluator_error_through() {
        let err = split("1 / 0").expect_err("division by zero");
        assert_eq!(err.message(), "division by zero");
    }
}
