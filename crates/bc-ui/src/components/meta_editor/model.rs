// Leptos-free working buffer behind the metadata editor.
//
// One [`MetaRow`] per editable row. A row that the user has not touched keeps
// only its `source` and re-emits it verbatim; a row the user has touched grows a
// [`MetaDraft`] holding the raw text of every control the row owns, and is
// rebuilt from that draft on save. Round-tripping is therefore structural: an
// entry the editor has no typed input for still survives a load-save cycle
// unchanged, as does one whose key is missing from the registry snapshot.
//
// This file is Leptos-free so `main.rs`'s `components_tests` shim can `include!`
// it and run its tests on the host.

use bc_ipc::MetaEntryDto;
use bc_ipc::MetaKeyDefDto;
use bc_ipc::MetaTypeDto;
use bc_ipc::MetaValueDto;
use rust_decimal::Decimal;

/// One editable row of an owner's metadata.
///
/// Equality is what the row *emits*, not how it is being edited: two rows are
/// equal when they serialise to the same entry. The dirty-gated save bar diffs
/// whole buffers, and a row edited back to the value it loaded with has changed
/// nothing.
#[derive(Clone, Debug)]
pub struct MetaRow {
    /// Stable per-row identity for keyed rendering and reordering; not persisted.
    pub uid: u64,
    /// The entry exactly as loaded, or `None` for a row the user added.
    pub source: Option<MetaEntryDto>,
    /// Present only once the user has touched this row.
    pub draft: Option<MetaDraft>,
}

/// A row's in-flight edit: the key, the type it is being edited as, and the raw
/// text of every control the row owns.
///
/// One draft carries all six controls rather than an enum per type so switching
/// the type on a create row never discards what the user already typed.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaDraft {
    /// The key this row files its value under.
    pub key: String,
    /// The type the row is being edited as.
    pub ty: MetaTypeDto,
    /// Raw text for the text, number, date and timestamp controls, the numeric
    /// half of an amount, and any value being repaired as text.
    pub text: String,
    /// State of the boolean checkbox.
    pub boolean: bool,
    /// Commodity code paired with `text` for an amount.
    pub commodity: String,
    /// Account id chosen in the picker.
    pub account_id: String,
}

/// How a row should be presented, derived from the registry snapshot and the
/// shape of the loaded value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// The key is typed and its value fits; offer the control for that type.
    Typed(MetaTypeDto),
    /// The store could not fit this value into its key's registered type as of
    /// the last write. Offer a text box and a live-parsed badge.
    Mismatched(MetaTypeDto),
    /// An account entry whose account was deleted: the path is frozen in text
    /// and the foreign key is gone.
    Tombstone,
    /// The key is absent from the registry snapshot, so the editor does not know
    /// what type to write. Render the canonical text read-only.
    Untyped,
}

impl PartialEq for MetaRow {
    fn eq(&self, other: &Self) -> bool {
        self.emitted() == other.emitted()
    }
}

impl MetaRow {
    /// Returns the entry this row serialises to, or `None` when it is pruned.
    ///
    /// A row with no draft re-emits its source verbatim, flag and all — which is
    /// what makes the round-trip guarantee independent of whether the editor has
    /// a control for the value's type. A drafted row is rebuilt and emits
    /// `mismatched: false`; the backend derives the flag afresh either way and
    /// discards what the entry claims.
    ///
    /// A row with a blank value, or a value under a blank key, is pruned.
    #[must_use]
    pub fn emitted(&self) -> Option<MetaEntryDto> {
        let Some(ref draft) = self.draft else {
            return self.source.clone();
        };
        if draft.key.trim().is_empty() || is_blank(draft.ty, draft) {
            return None;
        }
        // A row the store flagged sends its value back as text whatever it now
        // parses as. The write path's rescue case is what repairs it; the editor
        // constructs no typed variant to repair anything.
        let value = if self.was_flagged() {
            MetaValueDto::Text(raw_text(draft.ty, draft))
        } else {
            draft_value(draft.ty, draft)
        };
        Some(MetaEntryDto::new(draft.key.trim(), value))
    }

    /// Returns the key this row currently carries, preferring the draft's.
    #[must_use]
    pub fn key(&self) -> &str {
        match self.draft {
            Some(ref draft) => draft.key.as_str(),
            None => self.source.as_ref().map_or("", |entry| entry.key.as_str()),
        }
    }

    /// Returns whether the store flagged this row's loaded value.
    ///
    /// The flag is the store's verdict when the entry loaded, never a judgement
    /// on what the user has typed since.
    #[must_use]
    pub fn was_flagged(&self) -> bool {
        self.source.as_ref().is_some_and(|entry| entry.mismatched)
    }

    /// Returns whether this row carries neither a key nor a value.
    ///
    /// Drives the `Backspace`-deletes-the-row rule.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self.draft {
            Some(ref draft) => draft.key.trim().is_empty() && is_blank(draft.ty, draft),
            None => self.source.is_none(),
        }
    }
}

impl MetaDraft {
    /// Seeds a draft from a loaded entry, or a blank one when `source` is `None`.
    ///
    /// # Arguments
    ///
    /// * `source` - The entry as loaded, if this row has one.
    /// * `keys` - The registry snapshot, which decides the type when it knows the key.
    /// * `default_commodity` - Commodity to seed a fresh amount row with.
    #[must_use]
    pub fn seed(
        source: Option<&MetaEntryDto>,
        keys: &[MetaKeyDefDto],
        default_commodity: &str,
    ) -> Self {
        let key = source.map(|entry| entry.key.clone()).unwrap_or_default();
        let ty = registered_type(keys, &key)
            .or_else(|| source.map(|entry| entry.value.ty()))
            .unwrap_or(MetaTypeDto::Text);
        let mut draft = Self {
            key,
            ty,
            text: String::new(),
            boolean: false,
            commodity: default_commodity.to_owned(),
            account_id: String::new(),
        };
        if let Some(entry) = source {
            match entry.value {
                MetaValueDto::Boolean(flag) => draft.boolean = flag,
                MetaValueDto::Amount(ref amount) => {
                    draft.text = amount.value.to_string();
                    draft.commodity.clone_from(&amount.currency_code);
                }
                MetaValueDto::Account(ref id) => draft.account_id.clone_from(id),
                MetaValueDto::Text(_)
                | MetaValueDto::Number(_)
                | MetaValueDto::Date(_)
                | MetaValueDto::Timestamp(_) => draft.text = canonical_text(&entry.value),
            }
        }
        draft
    }
}

/// Returns the type the registry snapshot holds for `key`.
///
/// The backend normalises a key to ASCII lowercase, so the lookup ignores case:
/// a key typed `Payee` is the registered `payee` and must not read as an
/// unregistered key needing creation.
///
/// # Arguments
///
/// * `keys` - The registry snapshot.
/// * `key` - The key to look up.
#[must_use]
pub fn registered_type(keys: &[MetaKeyDefDto], key: &str) -> Option<MetaTypeDto> {
    keys.iter()
        .find(|def| def.key.eq_ignore_ascii_case(key))
        .map(|def| def.ty)
}

/// Renders a value in the canonical string form the backend parses back.
///
/// Mirrors `bc_models::MetaValue::canonical`, which is unreachable here: the
/// WASM bundle must never pull in `bc-models`. The forms must stay identical
/// because what the editor emits is parsed by `MetaType::parse_value`, that
/// method's inverse.
///
/// # Arguments
///
/// * `value` - The value to render.
#[must_use]
pub fn canonical_text(value: &MetaValueDto) -> String {
    match *value {
        MetaValueDto::Text(ref text) => text.clone(),
        MetaValueDto::Number(number) => number.to_string(),
        MetaValueDto::Boolean(flag) => flag.to_string(),
        MetaValueDto::Date(day) => day.to_string(),
        MetaValueDto::Timestamp(stamp) => stamp.to_string(),
        MetaValueDto::Amount(ref amount) => {
            format!("{} {}", amount.value, amount.currency_code)
        }
        MetaValueDto::Account(ref id) => id.clone(),
    }
}

/// Classifies a row for presentation.
///
/// Mismatch is tested before tombstone: a flagged account entry keeps no account
/// link, so its frozen text is a failed value rather than a deleted account.
///
/// A tombstone stops being one the moment the user repoints it: the frozen path
/// describes the entry as loaded, and once a replacement account is picked it
/// describes nothing the row will save.
///
/// # Arguments
///
/// * `row` - The row to classify.
/// * `keys` - The registry snapshot.
#[must_use]
pub fn classify(row: &MetaRow, keys: &[MetaKeyDefDto]) -> RowKind {
    let Some(ty) = registered_type(keys, row.key()) else {
        // A key the snapshot does not know is untyped rather than broken. A row
        // the user created carries the type picked on the create row.
        return row
            .draft
            .as_ref()
            .map_or(RowKind::Untyped, |draft| RowKind::Typed(draft.ty));
    };
    if row.was_flagged() {
        return RowKind::Mismatched(ty);
    }
    let frozen_account = ty == MetaTypeDto::Account
        && row
            .draft
            .as_ref()
            .is_none_or(|draft| draft.account_id.trim().is_empty())
        && row
            .source
            .as_ref()
            .is_some_and(|entry| matches!(entry.value, MetaValueDto::Text(_)));
    if frozen_account {
        RowKind::Tombstone
    } else {
        RowKind::Typed(ty)
    }
}

/// Returns whether a draft's value currently parses as `ty`.
///
/// Drives the live badge on a row being repaired. `text` accepts anything and
/// `boolean` is a checkbox, so both always parse.
///
/// # Arguments
///
/// * `ty` - The type to parse against.
/// * `draft` - The draft holding the raw controls.
#[must_use]
pub fn parses_as(ty: MetaTypeDto, draft: &MetaDraft) -> bool {
    match ty {
        MetaTypeDto::Text | MetaTypeDto::Boolean => true,
        MetaTypeDto::Number => draft.text.trim().parse::<Decimal>().is_ok(),
        MetaTypeDto::Date => draft.text.trim().parse::<jiff::civil::Date>().is_ok(),
        MetaTypeDto::Timestamp => draft.text.trim().parse::<jiff::Timestamp>().is_ok(),
        MetaTypeDto::Amount => {
            !draft.commodity.trim().is_empty() && draft.text.trim().parse::<Decimal>().is_ok()
        }
        MetaTypeDto::Account => !draft.account_id.trim().is_empty(),
    }
}

/// Returns whether a draft holds no value at all.
///
/// A blank row is pruned on save, as a blank extra-date row already was. A
/// checkbox always holds a value, so a boolean row is never blank.
fn is_blank(ty: MetaTypeDto, draft: &MetaDraft) -> bool {
    match ty {
        MetaTypeDto::Boolean => false,
        MetaTypeDto::Account => draft.account_id.trim().is_empty() && draft.text.trim().is_empty(),
        MetaTypeDto::Text
        | MetaTypeDto::Number
        | MetaTypeDto::Date
        | MetaTypeDto::Timestamp
        | MetaTypeDto::Amount => draft.text.trim().is_empty(),
    }
}

/// Renders a draft's value as the raw text the write path will re-parse.
fn raw_text(ty: MetaTypeDto, draft: &MetaDraft) -> String {
    match ty {
        MetaTypeDto::Amount => format!("{} {}", draft.text.trim(), draft.commodity.trim())
            .trim()
            .to_owned(),
        MetaTypeDto::Account => {
            if draft.account_id.trim().is_empty() {
                draft.text.clone()
            } else {
                draft.account_id.trim().to_owned()
            }
        }
        MetaTypeDto::Boolean => draft.boolean.to_string(),
        MetaTypeDto::Text | MetaTypeDto::Number | MetaTypeDto::Date | MetaTypeDto::Timestamp => {
            draft.text.clone()
        }
    }
}

/// Builds the value a drafted row emits.
///
/// A value that does not parse as its type goes out as [`MetaValueDto::Text`],
/// for the write path's rescue case to parse and flag. That is the same path a
/// repair takes, so there is one behaviour rather than two.
fn draft_value(ty: MetaTypeDto, draft: &MetaDraft) -> MetaValueDto {
    if !parses_as(ty, draft) {
        return MetaValueDto::Text(raw_text(ty, draft));
    }
    match ty {
        MetaTypeDto::Text => MetaValueDto::Text(draft.text.clone()),
        MetaTypeDto::Boolean => MetaValueDto::Boolean(draft.boolean),
        MetaTypeDto::Number => draft.text.trim().parse::<Decimal>().map_or_else(
            |_err| MetaValueDto::Text(raw_text(ty, draft)),
            MetaValueDto::Number,
        ),
        MetaTypeDto::Date => draft.text.trim().parse::<jiff::civil::Date>().map_or_else(
            |_err| MetaValueDto::Text(raw_text(ty, draft)),
            MetaValueDto::Date,
        ),
        MetaTypeDto::Timestamp => draft.text.trim().parse::<jiff::Timestamp>().map_or_else(
            |_err| MetaValueDto::Text(raw_text(ty, draft)),
            MetaValueDto::Timestamp,
        ),
        MetaTypeDto::Amount => draft.text.trim().parse::<Decimal>().map_or_else(
            |_err| MetaValueDto::Text(raw_text(ty, draft)),
            |number| MetaValueDto::Amount(bc_ipc::Amount::new(number, draft.commodity.trim())),
        ),
        MetaTypeDto::Account => MetaValueDto::Account(draft.account_id.trim().to_owned()),
    }
}

/// Builds one row per loaded entry, in load order.
///
/// # Arguments
///
/// * `entries` - The owner's entries as loaded.
#[must_use]
pub fn rows_from_entries(entries: &[MetaEntryDto]) -> Vec<MetaRow> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| MetaRow {
            uid: u64::try_from(index).unwrap_or(u64::MAX),
            source: Some(entry.clone()),
            draft: None,
        })
        .collect()
}

/// Serialises the buffer back into entries, in row order.
///
/// The emitted order is `position`; no position field crosses the wire.
///
/// # Arguments
///
/// * `rows` - The buffer.
#[must_use]
pub fn emit_rows(rows: &[MetaRow]) -> Vec<MetaEntryDto> {
    rows.iter().filter_map(MetaRow::emitted).collect()
}

/// Returns the next free row identity.
///
/// # Arguments
///
/// * `rows` - The buffer.
#[must_use]
pub fn next_uid(rows: &[MetaRow]) -> u64 {
    rows.iter()
        .map(|row| row.uid)
        .max()
        .map_or(0, |max| max.saturating_add(1))
}

/// Appends a blank row and returns its identity.
///
/// # Arguments
///
/// * `rows` - The buffer to append to.
/// * `default_commodity` - Commodity to seed the row's amount control with.
pub fn push_blank_row(rows: &mut Vec<MetaRow>, default_commodity: &str) -> u64 {
    let uid = next_uid(rows);
    rows.push(MetaRow {
        uid,
        source: None,
        draft: Some(MetaDraft::seed(None, &[], default_commodity)),
    });
    uid
}

/// Inserts a blank row directly below `uid` and returns the new row's identity.
///
/// # Arguments
///
/// * `rows` - The buffer to insert into.
/// * `uid` - The row to insert below.
/// * `default_commodity` - Commodity to seed the row's amount control with.
pub fn insert_row_below(rows: &mut Vec<MetaRow>, uid: u64, default_commodity: &str) -> u64 {
    let new_uid = next_uid(rows);
    let row = MetaRow {
        uid: new_uid,
        source: None,
        draft: Some(MetaDraft::seed(None, &[], default_commodity)),
    };
    match rows.iter().position(|r| r.uid == uid) {
        Some(index) => rows.insert(index.saturating_add(1), row),
        None => rows.push(row),
    }
    new_uid
}

/// Removes the row identified by `uid`, returning whether it was there.
///
/// # Arguments
///
/// * `rows` - The buffer.
/// * `uid` - The row to remove.
pub fn remove_row(rows: &mut Vec<MetaRow>, uid: u64) -> bool {
    let before = rows.len();
    rows.retain(|row| row.uid != uid);
    rows.len() != before
}

/// Moves the row identified by `uid` one place, returning whether it moved.
///
/// Row order is `position` on the wire, so this is the UI expression of a
/// cross-key reorder — real user intent that the whole-owner event shape exists
/// to preserve.
///
/// # Arguments
///
/// * `rows` - The buffer.
/// * `uid` - The row to move.
/// * `up` - Move towards the front when `true`, towards the back when `false`.
pub fn move_row(rows: &mut [MetaRow], uid: u64, up: bool) -> bool {
    let Some(index) = rows.iter().position(|row| row.uid == uid) else {
        return false;
    };
    let target = if up {
        if index == 0 {
            return false;
        }
        index.saturating_sub(1)
    } else {
        let next = index.saturating_add(1);
        if next >= rows.len() {
            return false;
        }
        next
    };
    rows.swap(index, target);
    true
}

/// Returns the first text-shaped value filed under `key`.
///
/// The collapsed transaction row's avatar initial and display name read `payee`
/// through this. An entry the store flagged is text-shaped, so it is reachable
/// and reads as the raw text the user typed — the honest thing to show in a name
/// cell.
///
/// # Arguments
///
/// * `entries` - The owner's entries.
/// * `key` - The key to look up.
#[must_use]
pub fn first_text_by_key<'a>(entries: &'a [MetaEntryDto], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .filter(|entry| entry.key == key)
        .find_map(|entry| match entry.value {
            MetaValueDto::Text(ref text) => Some(text.as_str()),
            MetaValueDto::Number(_)
            | MetaValueDto::Boolean(_)
            | MetaValueDto::Date(_)
            | MetaValueDto::Timestamp(_)
            | MetaValueDto::Amount(_)
            | MetaValueDto::Account(_) => None,
        })
}

#[cfg(test)]
mod tests {
    use bc_ipc::Amount;
    use bc_ipc::MetaEntryDto;
    use bc_ipc::MetaKeyDefDto;
    use bc_ipc::MetaTypeDto;
    use bc_ipc::MetaValueDto;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::MetaDraft;
    use super::MetaRow;
    use super::RowKind;
    use super::canonical_text;
    use super::classify;
    use super::emit_rows;
    use super::first_text_by_key;
    use super::insert_row_below;
    use super::move_row;
    use super::parses_as;
    use super::push_blank_row;
    use super::remove_row;
    use super::rows_from_entries;

    /// A fake account id in the backend's shape; not a real account.
    const ACCOUNT_ID: &str = "account_00000000000000000000000000";

    /// One entry of each of the seven types, under seven distinct keys.
    fn seven_entries() -> Vec<MetaEntryDto> {
        vec![
            MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            MetaEntryDto::new("invoice", MetaValueDto::Number(Decimal::new(150_250, 2))),
            MetaEntryDto::new("reimbursed", MetaValueDto::Boolean(true)),
            MetaEntryDto::new("cleared", MetaValueDto::Date(jiff::civil::date(2026, 5, 2))),
            MetaEntryDto::new(
                "seen-at",
                MetaValueDto::Timestamp(
                    jiff::Timestamp::from_second(1_700_000_000).expect("valid timestamp"),
                ),
            ),
            MetaEntryDto::new(
                "budgeted",
                MetaValueDto::Amount(Amount::new(Decimal::new(4_200, 2), "AUD")),
            ),
            MetaEntryDto::new("offset", MetaValueDto::Account(ACCOUNT_ID.to_owned())),
        ]
    }

    /// A registry snapshot typing every key `seven_entries` uses.
    fn registry() -> Vec<MetaKeyDefDto> {
        vec![
            MetaKeyDefDto::new("payee", MetaTypeDto::Text),
            MetaKeyDefDto::new("invoice", MetaTypeDto::Number),
            MetaKeyDefDto::new("reimbursed", MetaTypeDto::Boolean),
            MetaKeyDefDto::new("cleared", MetaTypeDto::Date),
            MetaKeyDefDto::new("seen-at", MetaTypeDto::Timestamp),
            MetaKeyDefDto::new("budgeted", MetaTypeDto::Amount),
            MetaKeyDefDto::new("offset", MetaTypeDto::Account),
        ]
    }

    /// A draft holding `text` under `ty`, with the other controls blank.
    fn draft(ty: MetaTypeDto, key: &str, text: &str) -> MetaDraft {
        MetaDraft {
            key: key.to_owned(),
            ty,
            text: text.to_owned(),
            boolean: false,
            commodity: String::new(),
            account_id: String::new(),
        }
    }

    #[test]
    fn an_untouched_buffer_re_emits_every_entry_verbatim() {
        let mut entries = seven_entries();
        entries.push(MetaEntryDto::flagged("cleared", "not-a-date"));
        entries.push(MetaEntryDto::new(
            "shipment",
            MetaValueDto::Text("in transit".to_owned()),
        ));
        let rows = rows_from_entries(&entries);
        assert_eq!(
            emit_rows(&rows),
            entries,
            "an untouched row re-emits its source verbatim, whether or not the editor \
             has a control for its type and whether or not the registry knows its key"
        );
    }

    #[test]
    fn a_blank_row_is_pruned_and_its_neighbour_survives() {
        let mut rows = rows_from_entries(&[MetaEntryDto::new(
            "payee",
            MetaValueDto::Text("Generic Grocer".to_owned()),
        )]);
        push_blank_row(&mut rows, "AUD");
        push_blank_row(&mut rows, "AUD");
        assert_eq!(
            emit_rows(&rows),
            vec![MetaEntryDto::new(
                "payee",
                MetaValueDto::Text("Generic Grocer".to_owned())
            )]
        );
    }

    #[test]
    fn a_valued_row_with_no_key_is_pruned() {
        let rows = vec![MetaRow {
            uid: 0,
            source: None,
            draft: Some(draft(MetaTypeDto::Text, "  ", "orphaned")),
        }];
        assert_eq!(emit_rows(&rows), Vec::new());
    }

    #[test]
    fn repeated_keys_keep_their_order() {
        let entries = vec![
            MetaEntryDto::new("note", MetaValueDto::Text("first".to_owned())),
            MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            MetaEntryDto::new("note", MetaValueDto::Text("second".to_owned())),
        ];
        let rows = rows_from_entries(&entries);
        assert_eq!(
            emit_rows(&rows),
            entries,
            "a key is not a slot; two entries under one key are two entries"
        );
    }

    #[test]
    fn a_reorder_moves_a_row_across_keys() {
        let entries = vec![
            MetaEntryDto::new("note", MetaValueDto::Text("first".to_owned())),
            MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            MetaEntryDto::new("note", MetaValueDto::Text("second".to_owned())),
        ];
        let mut rows = rows_from_entries(&entries);
        assert!(move_row(&mut rows, 2, true), "the last row moves up");
        assert_eq!(
            emit_rows(&rows),
            vec![
                MetaEntryDto::new("note", MetaValueDto::Text("first".to_owned())),
                MetaEntryDto::new("note", MetaValueDto::Text("second".to_owned())),
                MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            ],
            "a cross-key reorder is real user intent and survives serialisation"
        );
    }

    #[test]
    fn a_reorder_at_either_end_does_nothing() {
        let mut rows = rows_from_entries(&seven_entries());
        assert!(!move_row(&mut rows, 0, true));
        assert!(!move_row(&mut rows, 6, false));
        assert!(!move_row(&mut rows, 99, true), "an absent row never moves");
    }

    #[rstest]
    #[case(MetaTypeDto::Text, "anything at all", true)]
    #[case(MetaTypeDto::Number, "1502.50", true)]
    #[case(MetaTypeDto::Number, "not-a-number", false)]
    #[case(MetaTypeDto::Date, "2026-05-02", true)]
    #[case(MetaTypeDto::Date, "2026-13-99", false)]
    #[case(MetaTypeDto::Timestamp, "2023-11-14T22:13:20Z", true)]
    #[case(MetaTypeDto::Timestamp, "yesterday", false)]
    fn the_live_parse_predicate_reads_the_text_controls(
        #[case] ty: MetaTypeDto,
        #[case] text: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(parses_as(ty, &draft(ty, "k", text)), expected);
    }

    #[test]
    fn a_boolean_always_parses() {
        assert!(parses_as(
            MetaTypeDto::Boolean,
            &draft(MetaTypeDto::Boolean, "k", "")
        ));
    }

    #[test]
    fn an_amount_needs_a_number_and_a_commodity() {
        let mut d = draft(MetaTypeDto::Amount, "budgeted", "42.00");
        assert!(!parses_as(MetaTypeDto::Amount, &d), "no commodity");
        d.commodity = "AUD".to_owned();
        assert!(parses_as(MetaTypeDto::Amount, &d));
        d.text = "lots".to_owned();
        assert!(!parses_as(MetaTypeDto::Amount, &d), "no number");
    }

    #[test]
    fn an_account_needs_a_picked_id() {
        let mut d = draft(MetaTypeDto::Account, "offset", "Assets:Bank");
        assert!(
            !parses_as(MetaTypeDto::Account, &d),
            "typed text is not a picked account"
        );
        d.account_id = ACCOUNT_ID.to_owned();
        assert!(parses_as(MetaTypeDto::Account, &d));
    }

    #[test]
    fn canonical_text_renders_each_type_in_the_form_the_backend_parses() {
        let rendered: Vec<String> = seven_entries()
            .iter()
            .map(|entry| canonical_text(&entry.value))
            .collect();
        assert_eq!(
            rendered,
            vec![
                "Generic Grocer".to_owned(),
                "1502.50".to_owned(),
                "true".to_owned(),
                "2026-05-02".to_owned(),
                "2023-11-14T22:13:20Z".to_owned(),
                "42.00 AUD".to_owned(),
                ACCOUNT_ID.to_owned(),
            ],
            "these are MetaType::parse_value's accepted forms; drifting from them \
             turns a round trip into a mismatch"
        );
    }

    #[test]
    fn an_account_key_holding_text_is_a_tombstone() {
        let rows = rows_from_entries(&[MetaEntryDto::new(
            "offset",
            MetaValueDto::Text("Assets:Bank:Savings".to_owned()),
        )]);
        let row = rows.first().expect("one row");
        assert_eq!(classify(row, &registry()), RowKind::Tombstone);
    }

    #[test]
    fn a_repointed_tombstone_stops_being_one() {
        let mut rows = rows_from_entries(&[MetaEntryDto::new(
            "offset",
            MetaValueDto::Text("Assets:Bank:Savings".to_owned()),
        )]);
        let row = rows.first_mut().expect("one row");
        let mut repointed = MetaDraft::seed(row.source.as_ref(), &registry(), "AUD");
        repointed.account_id = ACCOUNT_ID.to_owned();
        row.draft = Some(repointed);
        assert_eq!(
            classify(row, &registry()),
            RowKind::Typed(MetaTypeDto::Account),
            "the frozen path describes the entry as loaded, and a picked account \
             replaces it"
        );
        assert_eq!(
            emit_rows(&rows),
            vec![MetaEntryDto::new(
                "offset",
                MetaValueDto::Account(ACCOUNT_ID.to_owned())
            )]
        );
    }

    #[test]
    fn a_flagged_account_entry_is_a_mismatch_not_a_tombstone() {
        let rows = rows_from_entries(&[MetaEntryDto::flagged("offset", "Assets:Bank:Savings")]);
        let row = rows.first().expect("one row");
        assert_eq!(
            classify(row, &registry()),
            RowKind::Mismatched(MetaTypeDto::Account),
            "a flagged account entry keeps no account link, so there is nothing to tombstone"
        );
    }

    #[test]
    fn a_key_the_registry_does_not_know_is_untyped() {
        let rows = rows_from_entries(&[MetaEntryDto::new(
            "shipment",
            MetaValueDto::Text("in transit".to_owned()),
        )]);
        let row = rows.first().expect("one row");
        assert_eq!(classify(row, &registry()), RowKind::Untyped);
    }

    #[test]
    fn a_key_typed_in_another_case_is_the_registered_key() {
        let row = MetaRow {
            uid: 0,
            source: None,
            draft: Some(draft(MetaTypeDto::Text, "Invoice", "7")),
        };
        assert_eq!(
            classify(&row, &registry()),
            RowKind::Typed(MetaTypeDto::Number),
            "the backend lowercases a key, so `Invoice` is the registered `invoice` \
             rather than a new key waiting to be created"
        );
    }

    #[test]
    fn a_created_key_takes_the_type_picked_on_its_row() {
        let row = MetaRow {
            uid: 0,
            source: None,
            draft: Some(draft(MetaTypeDto::Number, "shipment", "7")),
        };
        assert_eq!(
            classify(&row, &registry()),
            RowKind::Typed(MetaTypeDto::Number)
        );
    }

    #[test]
    fn every_registered_key_classifies_as_its_type() {
        let rows = rows_from_entries(&seven_entries());
        let kinds: Vec<RowKind> = rows.iter().map(|row| classify(row, &registry())).collect();
        assert_eq!(
            kinds,
            vec![
                RowKind::Typed(MetaTypeDto::Text),
                RowKind::Typed(MetaTypeDto::Number),
                RowKind::Typed(MetaTypeDto::Boolean),
                RowKind::Typed(MetaTypeDto::Date),
                RowKind::Typed(MetaTypeDto::Timestamp),
                RowKind::Typed(MetaTypeDto::Amount),
                RowKind::Typed(MetaTypeDto::Account),
            ]
        );
    }

    #[test]
    fn a_touched_row_emits_unflagged_text_even_when_it_still_does_not_parse() {
        let mut rows = rows_from_entries(&[MetaEntryDto::flagged("cleared", "not-a-date")]);
        let row = rows.first_mut().expect("one row");
        row.draft = Some(draft(MetaTypeDto::Date, "cleared", "still not a date"));
        assert_eq!(
            emit_rows(&rows),
            vec![MetaEntryDto::new(
                "cleared",
                MetaValueDto::Text("still not a date".to_owned())
            )],
            "the flag never crosses the wire inwards; the write path derives it afresh"
        );
    }

    #[test]
    fn repairing_a_flagged_row_still_sends_text() {
        let mut rows = rows_from_entries(&[MetaEntryDto::flagged("cleared", "not-a-date")]);
        let row = rows.first_mut().expect("one row");
        row.draft = Some(draft(MetaTypeDto::Date, "cleared", "2026-05-02"));
        assert_eq!(
            emit_rows(&rows),
            vec![MetaEntryDto::new(
                "cleared",
                MetaValueDto::Text("2026-05-02".to_owned())
            )],
            "the editor constructs no typed variant to repair anything; the write \
             path's rescue case parses the text and clears the flag"
        );
    }

    #[test]
    fn a_drafted_row_of_each_type_emits_its_typed_variant() {
        let mut rows: Vec<MetaRow> = Vec::new();
        for entry in seven_entries() {
            let uid = push_blank_row(&mut rows, "AUD");
            let row = rows.iter_mut().find(|r| r.uid == uid).expect("pushed row");
            row.draft = Some(MetaDraft::seed(Some(&entry), &registry(), "AUD"));
        }
        assert_eq!(
            emit_rows(&rows),
            seven_entries(),
            "seeding a draft from an entry and emitting it back is the identity"
        );
    }

    #[test]
    fn a_row_inserted_below_lands_next_to_its_origin() {
        let entries = vec![
            MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            MetaEntryDto::new("note", MetaValueDto::Text("weekly".to_owned())),
        ];
        let mut rows = rows_from_entries(&entries);
        let uid = insert_row_below(&mut rows, 0, "AUD");
        assert_eq!(rows.get(1).map(|row| row.uid), Some(uid));
        assert_eq!(
            emit_rows(&rows),
            entries,
            "the inserted blank row is pruned and the order is otherwise untouched"
        );
    }

    #[test]
    fn removing_a_row_drops_exactly_that_row() {
        let entries = vec![
            MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
            MetaEntryDto::new("note", MetaValueDto::Text("weekly".to_owned())),
        ];
        let mut rows = rows_from_entries(&entries);
        assert!(remove_row(&mut rows, 0));
        assert!(!remove_row(&mut rows, 0), "the row is gone");
        assert_eq!(
            emit_rows(&rows),
            vec![MetaEntryDto::new(
                "note",
                MetaValueDto::Text("weekly".to_owned())
            )]
        );
    }

    #[test]
    fn an_empty_row_reports_itself_empty() {
        let mut rows = rows_from_entries(&seven_entries());
        let uid = push_blank_row(&mut rows, "AUD");
        let blank = rows.iter().find(|row| row.uid == uid).expect("pushed row");
        assert!(blank.is_empty());
        let loaded = rows.first().expect("a loaded row");
        assert!(!loaded.is_empty());
    }

    #[test]
    fn first_text_by_key_reaches_a_flagged_entry() {
        let entries = vec![
            MetaEntryDto::new("invoice", MetaValueDto::Number(Decimal::new(7, 0))),
            MetaEntryDto::flagged("payee", "Generic Grocer"),
            MetaEntryDto::new("payee", MetaValueDto::Text("Other Merchant".to_owned())),
        ];
        assert_eq!(
            first_text_by_key(&entries, "payee"),
            Some("Generic Grocer"),
            "a flagged entry is text-shaped, so it reads as what the user typed"
        );
    }

    #[test]
    fn first_text_by_key_skips_non_text_values() {
        let entries = vec![MetaEntryDto::new(
            "payee",
            MetaValueDto::Number(Decimal::new(7, 0)),
        )];
        assert_eq!(first_text_by_key(&entries, "payee"), None);
        assert_eq!(first_text_by_key(&entries, "absent"), None);
    }
}
