//! Reads a Beancount number.
//!
//! Beancount lets a number carry comma digit-group separators and stand
//! inside an arithmetic expression wherever a plain literal is allowed:
//! `1,234,567.89`, `(1,000,000 * 3 / 100 / 365)`, `-1,000 + 2.50`.
//! [`parse_number`] evaluates that grammar to one [`Decimal`].

use rust_decimal::Decimal;

/// Evaluates a Beancount number expression.
///
/// The grammar is Beancount's own: a literal is digits with optional comma
/// separators between them and an optional fraction; unary `+` and `-`;
/// binary `+`, `-`, `*` and `/` with the usual precedence; parentheses;
/// whitespace between tokens.
///
/// # Arguments
///
/// * `raw` - The expression text.
///
/// # Returns
///
/// The evaluated value.
///
/// # Errors
///
/// Returns a message naming the token that stopped the read, a division by
/// zero, or an overflow.
pub(crate) fn parse_number(raw: &str) -> Result<Decimal, String> {
    let mut cursor = Cursor { text: raw, pos: 0 };
    cursor.skip_space();
    let value = cursor.sum()?;
    cursor.skip_space();
    if cursor.pos < raw.len() {
        return Err(format!("unexpected '{}' after the number", cursor.rest()));
    }
    Ok(value)
}

/// A position in the expression text.
struct Cursor<'a> {
    text: &'a str,
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

    /// `term (('+' | '-') term)*`
    fn sum(&mut self) -> Result<Decimal, String> {
        let mut value = self.product()?;
        loop {
            if self.eat('+') {
                let rhs = self.product()?;
                value = value.checked_add(rhs).ok_or("overflow")?;
            } else if self.eat('-') {
                let rhs = self.product()?;
                value = value.checked_sub(rhs).ok_or("overflow")?;
            } else {
                return Ok(value);
            }
        }
    }

    /// `factor (('*' | '/') factor)*`
    fn product(&mut self) -> Result<Decimal, String> {
        let mut value = self.factor()?;
        loop {
            if self.eat('*') {
                let rhs = self.factor()?;
                value = value.checked_mul(rhs).ok_or("overflow")?;
            } else if self.eat('/') {
                let rhs = self.factor()?;
                if rhs.is_zero() {
                    return Err("division by zero".to_owned());
                }
                value = value.checked_div(rhs).ok_or("overflow")?;
            } else {
                return Ok(value);
            }
        }
    }

    /// `('+' | '-') factor | '(' sum ')' | literal`
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
        digits.parse::<Decimal>().map_err(|_| "overflow".to_owned())
    }
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
        assert_eq!(parse_number(raw), Ok(expected));
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
        assert_eq!(parse_number(raw), Ok(expected));
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
    fn rejects_a_malformed_expression(#[case] raw: &str, #[case] reason: &str) {
        assert_eq!(parse_number(raw), Err(reason.to_owned()));
    }

    #[test]
    fn reports_overflow() {
        let big = "9".repeat(20);
        assert_eq!(
            parse_number(&format!("{big} * {big}")),
            Err("overflow".to_owned())
        );
    }
}
