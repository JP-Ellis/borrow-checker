//! Posting specifications: `ACCOUNT:AMOUNT:COMMODITY[{COST}][@PRICE]`, where
//! `ACCOUNT` is an account path or an account ID.

use core::str::FromStr as _;

use super::leg;
use crate::error::CliError;
use crate::error::CliResult;

/// Resolves the account part of a spec or selector to an account ID.
pub(super) type Lookup<'a> = dyn Fn(&str) -> Option<bc_models::AccountId> + 'a;

/// Builds a [`Lookup`] over `resolver` that accepts an account ID or a path.
///
/// An ID is accepted as written, whether or not an account holds it, as
/// `--posting` has always done. A path must resolve to an existing account.
pub(super) fn account_lookup(
    resolver: &bc_core::AccountResolver,
) -> impl Fn(&str) -> Option<bc_models::AccountId> + '_ {
    move |text| {
        if let Ok(id) = bc_models::AccountId::from_str(text.trim()) {
            return Some(id);
        }
        let path = bc_core::AccountPath::parse(text).ok()?;
        if let bc_core::Resolution::Resolved { id, .. } = resolver.resolve(&path) {
            Some(id)
        } else {
            None
        }
    }
}

/// Splits `text` at the one `:` whose left part names an account and whose
/// right part `accept` takes.
///
/// Paths contain `:` as the grammar does, so every split point is tried.
///
/// # Errors
///
/// Returns a message when no split works: the leg error from the longest
/// prefix that named an account, or a note that no prefix did. The leg grammar
/// admits at most one valid split; two are reported rather than guessed.
pub(super) fn split_account<T>(
    text: &str,
    lookup: &Lookup<'_>,
    accept: impl Fn(&str) -> Result<T, String>,
) -> Result<(bc_models::AccountId, T), String> {
    let mut found: Vec<(&str, bc_models::AccountId, T)> = Vec::new();
    let mut last_error: Option<String> = None;
    for (idx, _) in text.match_indices(':') {
        let Some((left, rest)) = text.split_at_checked(idx) else {
            continue;
        };
        let Some(right) = rest.get(1..) else {
            continue;
        };
        let Some(id) = lookup(left) else {
            continue;
        };
        match accept(right) {
            Ok(value) => found.push((left, id, value)),
            Err(e) => last_error = Some(e),
        }
    }
    if !text.contains(':') {
        return Err("expected ACCOUNT:AMOUNT:COMMODITY".into());
    }
    let mut candidates = found.into_iter();
    match (candidates.next(), candidates.next()) {
        (Some((_, id, value)), None) => Ok((id, value)),
        (Some((first, ..)), Some((second, ..))) => Err(format!(
            "both '{first}' and '{second}' are accounts; use the account ID"
        )),
        (None, _) => {
            Err(last_error.unwrap_or_else(|| format!("no account matches the start of '{text}'")))
        }
    }
}

/// Parses a posting spec into a posting with a fresh ID.
///
/// # Errors
///
/// Returns [`CliError::Arg`] naming the spec when no account prefix resolves
/// or the leg is malformed.
pub(super) fn parse_posting(spec: &str, lookup: &Lookup<'_>) -> CliResult<bc_models::Posting> {
    let (account_id, parsed) = split_account(spec, lookup, leg::parse_leg)
        .map_err(|e| CliError::Arg(format!("invalid posting '{spec}': {e}")))?;

    let cost = parsed.cost.map(|block| {
        bc_models::Cost::builder()
            .basis(quote_of(block.kind, block.basis))
            .maybe_date(block.date)
            .maybe_label(block.label)
            .build()
    });
    let price = parsed.price.map(|price| quote_of(price.kind, price.figure));

    Ok(bc_models::Posting::builder()
        .id(bc_models::PostingId::new())
        .account_id(account_id)
        .amount(amount_of(parsed.units))
        .maybe_cost(cost)
        .maybe_price(price)
        .build())
}

/// Builds the model amount for a parsed figure.
pub(super) fn amount_of(figure: leg::Figure) -> bc_models::Amount {
    bc_models::Amount::new(figure.value, bc_models::CommodityCode::new(figure.code))
}

/// Builds the model quote for a parsed figure of the given kind.
fn quote_of(kind: leg::Kind, figure: leg::Figure) -> bc_models::Quote {
    match kind {
        leg::Kind::PerUnit => bc_models::Quote::PerUnit(amount_of(figure)),
        leg::Kind::Total => bc_models::Quote::Total(amount_of(figure)),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::str::FromStr as _;
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::parse_posting;

    /// A lookup over invented paths, accepting any well-formed ID as well.
    fn lookup_in(
        paths: &[&str],
    ) -> (
        HashMap<String, bc_models::AccountId>,
        impl Fn(&str) -> Option<bc_models::AccountId>,
    ) {
        let map: HashMap<String, bc_models::AccountId> = paths
            .iter()
            .map(|p| ((*p).to_owned(), bc_models::AccountId::new()))
            .collect();
        let copy = map.clone();
        let lookup = move |text: &str| {
            bc_models::AccountId::from_str(text)
                .ok()
                .or_else(|| copy.get(text.trim()).cloned())
        };
        (map, lookup)
    }

    #[test]
    fn a_path_spec_resolves_the_account() {
        let (map, lookup) = lookup_in(&["Assets:Checking"]);
        let posting = parse_posting("Assets:Checking:-50.00:AUD", &lookup).expect("parses");
        assert_eq!(
            posting.account_id(),
            map.get("Assets:Checking").expect("mapped")
        );
        let amount = posting.amount().expect("amount set");
        assert_eq!(amount.value(), dec!(-50.00));
        assert_eq!(amount.commodity().as_str(), "AUD");
    }

    #[test]
    fn an_id_spec_still_parses() {
        let (_map, lookup) = lookup_in(&[]);
        let id = bc_models::AccountId::new();
        let posting = parse_posting(&format!("{id}:50.00:AUD"), &lookup).expect("parses");
        assert_eq!(posting.account_id(), &id);
    }

    #[test]
    fn numeric_segment_resolves_to_the_deeper_account() {
        let (map, lookup) = lookup_in(&["Assets:Bank", "Assets:Bank:123456789"]);
        let posting = parse_posting("Assets:Bank:123456789:50:AUD", &lookup).expect("parses");
        assert_eq!(
            posting.account_id(),
            map.get("Assets:Bank:123456789").expect("mapped")
        );
    }

    #[test]
    fn cost_and_price_survive_a_path_account() {
        let (_map, lookup) = lookup_in(&["Assets:Broker"]);
        let posting =
            parse_posting("Assets:Broker:2:AAPL{105:AUD}@150:AUD", &lookup).expect("parses");
        assert!(posting.cost().is_some());
        assert!(posting.price().is_some());
    }

    #[rstest]
    #[case::no_colon("Assets", "expected ACCOUNT:AMOUNT:COMMODITY")]
    #[case::unknown_account("Assets:Nowhere:5:AUD", "no account matches the start of")]
    #[case::bad_leg("Assets:Checking:abc:AUD", "invalid amount 'abc'")]
    #[case::bad_id("notanid:50.00:AUD", "no account matches the start of")]
    fn errors_name_the_spec_and_the_problem(#[case] spec: &str, #[case] expected: &str) {
        let (_map, lookup) = lookup_in(&["Assets:Checking"]);
        let err = parse_posting(spec, &lookup)
            .expect_err("rejects")
            .to_string();
        assert!(err.contains(expected), "got: {err}");
        assert!(err.contains(spec), "got: {err}");
    }
}
