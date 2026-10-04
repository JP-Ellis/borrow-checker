//! The stored values of a text metadata key, for the palette's suggestions.

use super::escape_like;
use crate::BcResult;
use crate::transaction::Service;

impl Service {
    /// The stored values of the text key `key` whose text contains `needle`,
    /// ignoring ASCII case, each with how many entries hold it; at most
    /// `limit`.
    ///
    /// Entries on transactions and on legs both count. Values group by their
    /// exact text. Values starting with the needle come first, then by count
    /// descending, then by value. An unknown key, or one not registered as
    /// text, has no values.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key, as registered.
    /// * `needle` - The text each value must contain.
    /// * `limit` - The most values to return.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database failure.
    pub async fn metadata_values(
        &self,
        key: &str,
        needle: &str,
        limit: u32,
    ) -> BcResult<Vec<(String, u64)>> {
        let folded = escape_like(&needle.to_ascii_lowercase());
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT m.value_text, COUNT(*) AS uses \
             FROM (SELECT value_text FROM transaction_metadata WHERE key = ?1 \
                   UNION ALL \
                   SELECT value_text FROM posting_metadata WHERE key = ?1) AS m \
             WHERE EXISTS (SELECT 1 FROM metadata_keys k \
                           WHERE k.key = ?1 AND k.value_type = 'text') \
               AND lower(m.value_text) LIKE ?2 ESCAPE '\\' \
             GROUP BY m.value_text \
             ORDER BY lower(m.value_text) LIKE ?3 ESCAPE '\\' DESC, uses DESC, m.value_text \
             LIMIT ?4",
        )
        .bind(key)
        .bind(format!("%{folded}%"))
        .bind(format!("{folded}%"))
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(value, uses)| (value, u64::try_from(uses).unwrap_or(0)))
            .collect())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::MetaEntry;
    use bc_models::MetaKey;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use crate::transaction::Service;

    /// Metadata from `(key, value)` pairs.
    fn meta(entries: &[(&str, MetaValue)]) -> Metadata {
        Metadata::new(
            entries
                .iter()
                .map(|(key, value)| MetaEntry::new(MetaKey::new(*key).expect("key"), value.clone()))
                .collect(),
        )
    }

    /// Text metadata value.
    fn text(value: &str) -> MetaValue {
        MetaValue::Text(value.to_owned())
    }

    /// A two-leg transaction carrying `tx_meta`, with `leg_meta` on its first leg.
    fn tx(
        a: &AccountId,
        b: &AccountId,
        tx_meta: &[(&str, MetaValue)],
        leg_meta: &[(&str, MetaValue)],
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 6, 1))
            .description("Invented")
            .metadata(meta(tx_meta))
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(5), CommodityCode::new("AUD")))
                    .metadata(meta(leg_meta))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-5), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    /// A ledger with payees on transactions and on a leg, and a number key.
    async fn ledger(pool: &sqlx::SqlitePool) -> Service {
        let accounts = crate::account::Service::new(pool.clone());
        let mut ids = Vec::new();
        for (name, ty) in [("A", AccountType::Asset), ("B", AccountType::Expense)] {
            ids.push(
                accounts
                    .create()
                    .name(name)
                    .account_type(ty)
                    .kind(AccountKind::DepositAccount)
                    .call()
                    .await
                    .expect("account"),
            );
        }
        let [a, b] = ids.as_slice() else {
            panic!("two accounts");
        };
        let svc = Service::new(pool.clone());
        let rows: Vec<Transaction> = vec![
            tx(a, b, &[("payee", text("Example Cafe"))], &[]),
            tx(a, b, &[("payee", text("Example Cafe"))], &[]),
            tx(a, b, &[], &[("payee", text("Example Cafe"))]),
            tx(a, b, &[("payee", text("Cafe Uno"))], &[]),
            tx(a, b, &[("payee", text("Corner Cafe"))], &[]),
            tx(a, b, &[("payee", text("example cafe"))], &[]),
            tx(
                a,
                b,
                &[("payee", text("Bakery"))],
                &[("km", MetaValue::Number(dec!(5)))],
            ),
            tx(a, b, &[("note", text("50% off"))], &[]),
            tx(a, b, &[("note", text("50 off"))], &[]),
            tx(a, b, &[("note", text("a_b"))], &[]),
            tx(a, b, &[("note", text("axb"))], &[]),
            tx(a, b, &[("note", text("a..b"))], &[]),
        ];
        for row in rows {
            svc.create(row).await.expect("create");
        }
        svc
    }

    /// `(value, count)` pairs, owned.
    fn pairs(expected: &[(&str, u64)]) -> Vec<(String, u64)> {
        expected
            .iter()
            .map(|(v, n)| ((*v).to_owned(), *n))
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn values_rank_prefix_matches_first_then_by_count(pool: sqlx::SqlitePool) {
        let svc = ledger(&pool).await;
        let got = svc
            .metadata_values("payee", "CAFE", 50)
            .await
            .expect("values");
        assert_eq!(
            got,
            pairs(&[
                ("Cafe Uno", 1),
                ("Example Cafe", 3),
                ("Corner Cafe", 1),
                ("example cafe", 1),
            ])
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_needle_lists_every_value_by_count(pool: sqlx::SqlitePool) {
        let svc = ledger(&pool).await;
        let got = svc.metadata_values("payee", "", 50).await.expect("values");
        assert_eq!(
            got,
            pairs(&[
                ("Example Cafe", 3),
                ("Bakery", 1),
                ("Cafe Uno", 1),
                ("Corner Cafe", 1),
                ("example cafe", 1),
            ])
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_limit_caps_the_list(pool: sqlx::SqlitePool) {
        let svc = ledger(&pool).await;
        let got = svc
            .metadata_values("payee", "cafe", 2)
            .await
            .expect("values");
        assert_eq!(got, pairs(&[("Cafe Uno", 1), ("Example Cafe", 3)]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn like_metacharacters_and_dots_match_literally(pool: sqlx::SqlitePool) {
        let svc = ledger(&pool).await;
        let percent = svc.metadata_values("note", "%", 50).await.expect("percent");
        assert_eq!(percent, pairs(&[("50% off", 1)]));
        let underscore = svc
            .metadata_values("note", "_", 50)
            .await
            .expect("underscore");
        assert_eq!(underscore, pairs(&[("a_b", 1)]));
        let dots = svc.metadata_values("note", "a..b", 50).await.expect("dots");
        assert_eq!(dots, pairs(&[("a..b", 1)]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_and_non_text_keys_have_no_values(pool: sqlx::SqlitePool) {
        let svc = ledger(&pool).await;
        assert_eq!(
            svc.metadata_values("nope", "", 50).await.expect("unknown"),
            Vec::new()
        );
        assert_eq!(
            svc.metadata_values("km", "", 50).await.expect("number"),
            Vec::new()
        );
    }
}
