//! Arithmetic over Beancount numbers, as Fava allows in a budget amount.

use rust_decimal::Decimal;

/// Evaluates an arithmetic expression over Beancount numbers.
///
/// Grammar: `expr := term (('+' | '-') term)*`, `term := factor (('*' | '/')
/// factor)*`, `factor := '-' factor | '(' expr ')' | number`. Whitespace
/// between tokens is ignored.
///
/// # Arguments
///
/// * `input` - The expression, with or without an outer pair of parentheses.
///
/// # Returns
///
/// The value, at `rust_decimal`'s precision.
///
/// # Errors
///
/// Returns a message naming the fault: an unbalanced parenthesis, a division
/// by zero, a missing operand, or text left over after the expression.
pub(crate) fn eval(input: &str) -> Result<Decimal, String> {
    let mut cursor = Cursor { rest: input.trim() };
    let value = cursor.expr()?;
    if !cursor.rest.trim().is_empty() {
        return Err(format!(
            "trailing input after expression: '{}'",
            cursor.rest.trim()
        ));
    }
    Ok(value)
}

/// Parser state: the unconsumed tail of the input.
struct Cursor<'a> {
    /// The unconsumed tail of the input.
    rest: &'a str,
}

impl Cursor<'_> {
    /// Skips leading whitespace.
    fn skip_ws(&mut self) {
        self.rest = self.rest.trim_start();
    }

    /// Consumes `ch` (skipping leading whitespace first) if present.
    ///
    /// # Arguments
    ///
    /// * `ch` - The character to consume.
    ///
    /// # Returns
    ///
    /// `true` if `ch` was consumed.
    fn eat(&mut self, ch: char) -> bool {
        self.skip_ws();
        match self.rest.strip_prefix(ch) {
            Some(tail) => {
                self.rest = tail;
                true
            }
            None => false,
        }
    }

    /// Parses `term (('+' | '-') term)*`.
    ///
    /// # Errors
    ///
    /// Propagates any error from a `term`, or an overflow in `+`/`-`.
    fn expr(&mut self) -> Result<Decimal, String> {
        let mut acc = self.term()?;
        loop {
            if self.eat('+') {
                let rhs = self.term()?;
                acc = acc.checked_add(rhs).ok_or("overflow in addition")?;
            } else if self.eat('-') {
                let rhs = self.term()?;
                acc = acc.checked_sub(rhs).ok_or("overflow in subtraction")?;
            } else {
                return Ok(acc);
            }
        }
    }

    /// Parses `factor (('*' | '/') factor)*`.
    ///
    /// # Errors
    ///
    /// Propagates any error from a `factor`, a division by zero, or an
    /// overflow in `*`/`/`.
    fn term(&mut self) -> Result<Decimal, String> {
        let mut acc = self.factor()?;
        loop {
            if self.eat('*') {
                let rhs = self.factor()?;
                acc = acc.checked_mul(rhs).ok_or("overflow in multiplication")?;
            } else if self.eat('/') {
                let rhs = self.factor()?;
                if rhs.is_zero() {
                    return Err("division by zero".to_owned());
                }
                acc = acc.checked_div(rhs).ok_or("overflow in division")?;
            } else {
                return Ok(acc);
            }
        }
    }

    /// Parses `'-' factor | '(' expr ')' | number`.
    ///
    /// # Errors
    ///
    /// Returns an error for an unbalanced parenthesis, or propagates a
    /// `number` error.
    fn factor(&mut self) -> Result<Decimal, String> {
        if self.eat('-') {
            let value = self.factor()?;
            return Decimal::ZERO
                .checked_sub(value)
                .ok_or_else(|| "overflow in negation".to_owned());
        }
        if self.eat('(') {
            let inner = self.expr()?;
            if !self.eat(')') {
                return Err("unbalanced parenthesis".to_owned());
            }
            return Ok(inner);
        }
        self.number()
    }

    /// Consumes and parses one Beancount number token.
    ///
    /// # Errors
    ///
    /// Returns an error if no digits, `.` or `,` characters remain at the
    /// cursor, or if the token does not parse as a decimal.
    fn number(&mut self) -> Result<Decimal, String> {
        self.skip_ws();
        let end = self
            .rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
            .unwrap_or(self.rest.len());
        let (raw, tail) = self.rest.split_at(end);
        if raw.is_empty() {
            return Err(format!("expected a number at '{}'", self.rest));
        }
        self.rest = tail;
        number(raw)
    }
}

/// Parses one Beancount number token.
///
/// # Arguments
///
/// * `raw` - The token, digits with optional `,` thousands separators and at
///   most one `.`.
///
/// # Errors
///
/// Returns an error if `raw` (with its separators stripped) does not parse as
/// a decimal.
///
/// `,` is stripped wherever it appears in `raw`, without checking that it
/// falls on a three-digit boundary. Fava is the only source of these
/// strings, so a malformed grouping is not worth rejecting.
fn number(raw: &str) -> Result<Decimal, String> {
    let plain: String = raw.chars().filter(|c| *c != ',').collect();
    plain
        .parse::<Decimal>()
        .map_err(|e| format!("bad number '{raw}': {e}"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest::rstest]
    #[case("500.00", dec!(500.00))]
    #[case("-500.00", dec!(-500.00))]
    #[case("0.00", dec!(0.00))]
    #[case("1,200.50", dec!(1200.50))]
    #[case("(1 + 2) * 3", dec!(9))]
    #[case("(100,000 * 0.05 / 365 / 2)", dec!(6.8493150684931506849315068495))]
    #[case("((300,000.00 - 100,000.00) * 0.06 / 365 / 2)", dec!(16.438356164383561643835616438))]
    #[case("-(2 + 3)", dec!(-5))]
    #[case("2 * -3", dec!(-6))]
    fn evaluates(#[case] input: &str, #[case] expected: Decimal) {
        assert_eq!(eval(input).expect("valid expression"), expected);
    }

    #[rstest::rstest]
    #[case("(1 + 2", "unbalanced")]
    #[case("1 / 0", "division by zero")]
    #[case("1 +", "expected a number")]
    #[case("abc", "expected a number")]
    #[case("1 2", "trailing")]
    fn rejects(#[case] input: &str, #[case] fragment: &str) {
        let err = eval(input).expect_err("invalid expression");
        assert!(err.contains(fragment), "{err}");
    }
}
