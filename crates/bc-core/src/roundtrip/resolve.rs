//! Binding a parsed working file's names to the stored ledger.
//!
//! Apply creates no names. Every account, tag, commodity and metadata key a
//! working file states must already exist, and a [`Resolver`] reports each one
//! that does not.

use std::collections::HashMap;
use std::fmt;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Commodity;
use bc_models::Cost;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaType;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Quote;
use bc_models::Reconciliation;
use bc_models::Tag;
use bc_models::TagForest;
use bc_models::TagId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::AccountPath;
use crate::AccountResolver;
use crate::AccountService;
use crate::BcResult;
use crate::CommodityResolver;
use crate::CommodityService;
use crate::MetadataService;
use crate::RawMetaEntry;
use crate::RawMetaValue;
use crate::RawPosting;
use crate::RawTransaction;
use crate::TagService;
use crate::metadata::coerce::Coerced;
use crate::metadata::coerce::coerce;

/// A name a working file states that the ledger does not hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Unresolved {
    /// An account path, on a leg or as a metadata value.
    Account(String),
    /// A tag path, on a transaction or a leg.
    Tag(String),
    /// A commodity code, on an amount, price or cost.
    Commodity(String),
    /// A metadata key the registry does not hold.
    MetadataKey(String),
}

impl fmt::Display for Unresolved {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Account(ref path) => write!(f, "unknown account `{path}`"),
            Self::Tag(ref path) => write!(f, "unknown tag `{path}`"),
            Self::Commodity(ref code) => write!(f, "unknown commodity `{code}`"),
            Self::MetadataKey(ref key) => write!(f, "unknown metadata key `{key}`"),
        }
    }
}

/// Resolves the names in a parsed working file against one snapshot of the
/// ledger's accounts, tags, commodities and metadata keys.
#[derive(Debug, Clone)]
pub struct Resolver {
    /// Account path to account.
    accounts: HashMap<String, AccountId>,
    /// Account to path, for reports.
    paths: HashMap<AccountId, String>,
    /// Tag path to tag.
    tags: HashMap<String, TagId>,
    /// Registered commodities.
    commodities: CommodityResolver,
    /// Registered metadata keys and their types.
    keys: HashMap<MetaKey, MetaType>,
}

impl Resolver {
    /// Loads every account, tag, commodity and metadata key from the database.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data failure.
    #[inline]
    pub async fn load(pool: &SqlitePool) -> BcResult<Self> {
        let account_service = AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&account_service).await?;
        let accounts = account_service
            .list_all()
            .await?
            .iter()
            .filter_map(|account| {
                resolver
                    .path_of(account.id())
                    .map(|path| (path.to_owned(), account.id().clone()))
            })
            .collect();
        let tags = TagService::new(pool.clone()).list().await?;
        let commodities = CommodityService::new(pool.clone()).list_all().await?;
        let keys = MetadataService::new(pool.clone())
            .list()
            .await?
            .into_iter()
            .map(|def| (def.key().clone(), def.ty()))
            .collect();
        Ok(Self::new(accounts, &tags, &commodities, keys))
    }

    /// Builds a resolver from an already-loaded ledger.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Every account by its colon-separated path.
    /// * `tags` - Every tag.
    /// * `commodities` - Every registered commodity.
    /// * `keys` - Every registered metadata key and its type.
    #[must_use]
    #[inline]
    pub fn new(
        accounts: HashMap<String, AccountId>,
        tags: &[Tag],
        commodities: &[Commodity],
        keys: HashMap<MetaKey, MetaType>,
    ) -> Self {
        let paths = accounts
            .iter()
            .map(|(path, id)| (id.clone(), path.clone()))
            .collect();
        let forest = TagForest::new(tags.to_vec());
        let by_path = tags
            .iter()
            .filter_map(|tag| {
                forest
                    .path_of(tag.id())
                    .map(|path| (path.to_string(), tag.id().clone()))
            })
            .collect();
        Self {
            accounts,
            paths,
            tags: by_path,
            commodities: CommodityResolver::from_commodities(commodities),
            keys,
        }
    }

    /// Returns the path of `id`, for reports.
    #[must_use]
    #[inline]
    pub fn account_path(&self, id: &AccountId) -> Option<&str> {
        self.paths.get(id).map(String::as_str)
    }

    /// Binds `raw` to the ledger as a new transaction.
    ///
    /// The result carries fresh transaction and posting IDs, `Flagged`
    /// reconciliation and the current time as `created_at`; the file's flag
    /// is ignored. Each metadata value is fitted to its key's registered type
    /// the way a write would fit it, so text the writer quoted for want of a
    /// Beancount type, such as a timestamp or a colon-less account path,
    /// compares equal to the stored value.
    ///
    /// # Errors
    ///
    /// Returns every name `raw` states that the ledger does not hold, each
    /// once, in the order first stated.
    #[inline]
    pub fn transaction(&self, raw: &RawTransaction) -> Result<Transaction, Vec<Unresolved>> {
        let mut missing = Vec::new();
        let tag_ids = self.tag_ids(&raw.tags, &mut missing);
        let metadata = self.metadata(&raw.metadata, &mut missing);
        let postings: Vec<Posting> = raw
            .postings
            .iter()
            .map(|leg| self.posting(leg, &mut missing))
            .collect();
        if !missing.is_empty() {
            let mut unique = Vec::with_capacity(missing.len());
            for name in missing {
                if !unique.contains(&name) {
                    unique.push(name);
                }
            }
            return Err(unique);
        }
        Ok(Transaction::builder()
            .id(TransactionId::new())
            .date(raw.date)
            .description(raw.description.clone())
            .postings(postings)
            .reconciliation(Reconciliation::Flagged)
            .tag_ids(tag_ids)
            .metadata(metadata)
            .created_at(Timestamp::now())
            .build())
    }

    /// Binds one leg.
    fn posting(&self, raw: &RawPosting, missing: &mut Vec<Unresolved>) -> Posting {
        let account_id = self.account(&raw.account).unwrap_or_else(|| {
            missing.push(Unresolved::Account(raw.account.clone()));
            AccountId::new()
        });
        let amount = raw.amount.as_ref().map(|a| self.amount(a, missing));
        let price = raw.price.as_ref().map(|q| self.quote(q, missing));
        let cost = raw.cost.as_ref().map(|c| {
            Cost::builder()
                .basis(self.quote(c.basis(), missing))
                .maybe_date(c.date())
                .maybe_label(c.label().map(ToOwned::to_owned))
                .build()
        });
        let native = raw.native.as_ref();
        Posting::builder()
            .id(PostingId::new())
            .account_id(account_id)
            .maybe_amount(amount)
            .maybe_price(price)
            .maybe_cost(cost)
            .metadata(self.metadata(&raw.metadata, missing))
            .tag_ids(self.tag_ids(&raw.tags, missing))
            .maybe_spread_from(native.and_then(|n| n.spread_from))
            .maybe_spread_until(native.and_then(|n| n.spread_until))
            .build()
    }

    /// The account `path` names, if any.
    fn account(&self, path: &str) -> Option<AccountId> {
        let normalised = AccountPath::parse(path).ok()?.to_string();
        self.accounts.get(&normalised).cloned()
    }

    /// `amount` under its registered commodity code.
    fn amount(&self, amount: &Amount, missing: &mut Vec<Unresolved>) -> Amount {
        if let Some(code) = self.commodities.resolve(amount.commodity()) {
            Amount::new(amount.value(), code)
        } else {
            missing.push(Unresolved::Commodity(amount.commodity().to_string()));
            amount.clone()
        }
    }

    /// `quote` with its amount under its registered commodity code.
    fn quote(&self, quote: &Quote, missing: &mut Vec<Unresolved>) -> Quote {
        match *quote {
            Quote::PerUnit(ref amount) => Quote::PerUnit(self.amount(amount, missing)),
            Quote::Total(ref amount) => Quote::Total(self.amount(amount, missing)),
        }
    }

    /// The tags `paths` name, in order.
    fn tag_ids(&self, paths: &[String], missing: &mut Vec<Unresolved>) -> Vec<TagId> {
        paths
            .iter()
            .filter_map(|path| {
                let found = self.tags.get(path).cloned();
                if found.is_none() {
                    missing.push(Unresolved::Tag(path.clone()));
                }
                found
            })
            .collect()
    }

    /// The entries in order, each fitted to its key's registered type.
    fn metadata(&self, entries: &[RawMetaEntry], missing: &mut Vec<Unresolved>) -> Metadata {
        entries
            .iter()
            .filter_map(|entry| {
                let Some(&ty) = self.keys.get(&entry.key) else {
                    missing.push(Unresolved::MetadataKey(entry.key.to_string()));
                    return None;
                };
                self.meta_entry(entry, ty, missing)
            })
            .collect()
    }

    /// Fits one entry to `ty`, resolving an account path where `ty` wants an
    /// account.
    fn meta_entry(
        &self,
        entry: &RawMetaEntry,
        ty: MetaType,
        missing: &mut Vec<Unresolved>,
    ) -> Option<MetaEntry> {
        let key = entry.key.clone();
        let value = match entry.value {
            RawMetaValue::AccountPath(ref path) if ty == MetaType::Text => {
                MetaValue::Text(path.clone())
            }
            RawMetaValue::AccountPath(ref path) => {
                if let Some(id) = self.account(path) {
                    MetaValue::Account(id)
                } else {
                    missing.push(Unresolved::Account(path.clone()));
                    return None;
                }
            }
            RawMetaValue::Resolved(MetaValue::Text(ref text)) if ty == MetaType::Account => {
                if let Some(id) = self.account(text) {
                    MetaValue::Account(id)
                } else if let Coerced::Fits(value) = coerce(&MetaValue::Text(text.clone()), ty) {
                    value
                } else {
                    missing.push(Unresolved::Account(text.clone()));
                    return None;
                }
            }
            RawMetaValue::Resolved(ref value) => value.clone(),
        };
        Some(match coerce(&value, ty) {
            Coerced::Fits(fitted) => MetaEntry::new(key, fitted),
            Coerced::Mismatch => MetaEntry::mismatch(key, value.canonical()),
        })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::PathSpec;

    fn key(name: &str) -> MetaKey {
        MetaKey::new(name).expect("valid key")
    }

    fn fixture() -> (Resolver, AccountId) {
        let bank = AccountId::new();
        let tag = Tag::builder().name("trip").build();
        let resolver = Resolver::new(
            HashMap::from([("Assets:Bank".to_owned(), bank.clone())]),
            &[tag],
            &[Commodity::builder().code("AUD").decimals(2).build()],
            HashMap::from([
                (key("seen"), MetaType::Timestamp),
                (key("via"), MetaType::Account),
                (key("note"), MetaType::Text),
            ]),
        );
        (resolver, bank)
    }

    fn raw(metadata: Vec<RawMetaEntry>) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2026, 1, 2))
            .description("Woolworths")
            .metadata(metadata)
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .amount(Amount::new(dec!(-10.00), CommodityCode::new("aud")))
                    .build(),
            ])
            .build()
    }

    #[test]
    fn quoted_timestamp_and_account_coerce_to_registered_type() {
        let (resolver, bank) = fixture();
        let stamp = Timestamp::from_second(1_700_000_000).expect("valid timestamp");
        let tx = resolver
            .transaction(&raw(vec![
                RawMetaEntry::resolved(key("seen"), MetaValue::Text(stamp.to_string())),
                RawMetaEntry::resolved(key("via"), MetaValue::Text("Assets:Bank".to_owned())),
                RawMetaEntry::account_path(key("note"), "Assets:Bank"),
            ]))
            .expect("resolves");
        assert_eq!(
            tx.metadata(),
            &Metadata::new(vec![
                MetaEntry::new(key("seen"), MetaValue::Timestamp(stamp)),
                MetaEntry::new(key("via"), MetaValue::Account(bank)),
                MetaEntry::new(key("note"), MetaValue::Text("Assets:Bank".to_owned())),
            ])
        );
        let leg = tx.postings().first().expect("one leg");
        assert_eq!(leg.amount().expect("amount").commodity().as_str(), "AUD");
        assert_eq!(tx.reconciliation(), Reconciliation::Flagged);
    }

    #[test]
    fn every_unknown_name_is_reported_once() {
        let (resolver, _) = fixture();
        let mut tx = raw(vec![
            RawMetaEntry::resolved(key("mystery"), MetaValue::Boolean(true)),
            RawMetaEntry::resolved(key("via"), MetaValue::Text("Assets:Nowhere".to_owned())),
        ]);
        tx.tags = vec!["holiday".to_owned()];
        tx.postings.push(
            RawPosting::builder()
                .account("Expenses:Fod")
                .amount(Amount::new(dec!(10.00), CommodityCode::new("XYZ")))
                .build(),
        );
        tx.postings
            .push(RawPosting::builder().account("Expenses:Fod").build());
        assert_eq!(
            resolver.transaction(&tx).expect_err("unresolved"),
            vec![
                Unresolved::Tag("holiday".to_owned()),
                Unresolved::MetadataKey("mystery".to_owned()),
                Unresolved::Account("Assets:Nowhere".to_owned()),
                Unresolved::Account("Expenses:Fod".to_owned()),
                Unresolved::Commodity("XYZ".to_owned()),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_reads_the_ledger(pool: SqlitePool) {
        AccountService::new(pool.clone())
            .create_path(
                &PathSpec::builder()
                    .path(AccountPath::parse("Assets:Checking").expect("path"))
                    .build(),
            )
            .await
            .expect("account");
        let resolver = Resolver::load(&pool).await.expect("load");
        let found = resolver.account("Assets:Checking");
        assert_eq!(
            found.as_ref().and_then(|id| resolver.account_path(id)),
            Some("Assets:Checking")
        );
    }
}
