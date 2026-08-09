//! The header row as read from a file, and the column resolution it supports.

use std::borrow::Cow;
use std::collections::BTreeMap;

use bc_sdk::ImportError;

use crate::config::ColumnRef;

/// A file's header row, and the name-to-index resolution it supports.
///
/// Unlike a plain name-to-index map, this records *every* index carrying a
/// given name, so two columns sharing a name cannot silently collapse into one
/// with the earlier becoming unreachable.
#[derive(Debug)]
pub(crate) struct HeaderMap {
    /// Lowercased header name to every index carrying it, ascending. A name is
    /// ambiguous when its entry holds more than one index. Empty for a
    /// headerless file, where every reference must be positional.
    by_name: BTreeMap<String, Vec<usize>>,
}

impl HeaderMap {
    /// Builds the map from a header record and the profile's column references.
    ///
    /// A duplicated name is fatal when some configuration field addresses it,
    /// because resolution would otherwise have to guess which column was meant.
    /// A duplicated name nothing addresses is harmless and only warns — real
    /// headers carry unnamed empty columns, and trailing prose leaves junk
    /// names, neither of which may reject a file.
    ///
    /// # Arguments
    ///
    /// * `header` - The header record, or `None` for a headerless file.
    /// * `refs` - Every configured column reference paired with its field name,
    ///   as returned by [`crate::config::Config::column_refs`].
    ///
    /// # Returns
    ///
    /// The map, and warnings for the caller to emit.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::InvalidConfig`] listing every addressed
    /// duplicate, following [`crate::config::Config::validate`]'s convention of
    /// reporting all violations together.
    pub(crate) fn build(
        header: Option<&csv::StringRecord>,
        refs: &[(Cow<'static, str>, &ColumnRef)],
    ) -> Result<(Self, Vec<String>), ImportError> {
        let Some(record) = header else {
            return Ok((
                Self {
                    by_name: BTreeMap::new(),
                },
                Vec::new(),
            ));
        };

        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, name) in record.iter().enumerate() {
            by_name
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(index);
        }

        let mut problems = Vec::new();
        let mut warnings = Vec::new();
        for (name, indices) in by_name.iter().filter(|&(_, i)| i.len() > 1) {
            // Report the header's own spelling rather than the folded key.
            let display = indices
                .first()
                .and_then(|&i| record.get(i))
                .unwrap_or(name.as_str());
            let columns = indices
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" and ");
            let fields: Vec<&str> = refs
                .iter()
                .filter(|&(_, column)| {
                    column
                        .as_name()
                        .is_some_and(|n| n.to_ascii_lowercase() == *name)
                })
                .map(|(field, _)| field.as_ref())
                .collect();

            if fields.is_empty() {
                warnings.push(format!(
                    "the header names '{display}' at columns {columns}; no field \
                     addresses it, so the duplication is harmless here"
                ));
            } else {
                problems.push(format!(
                    "{} names '{display}', which appears at columns {columns}; \
                     address it by zero-based index instead",
                    fields.join(", ")
                ));
            }
        }

        if problems.is_empty() {
            Ok((Self { by_name }, warnings))
        } else {
            Err(ImportError::InvalidConfig(problems.join("; ")))
        }
    }

    /// Resolves a column reference to a zero-based index within a row.
    ///
    /// # Arguments
    ///
    /// * `column` - The reference to resolve.
    ///
    /// # Returns
    ///
    /// The zero-based column index.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::MissingField`] when a name does not appear in the
    /// header, and [`ImportError::InvalidConfig`] when it appears more than
    /// once. [`HeaderMap::build`] already rejects every addressed duplicate, so
    /// the latter guards against a caller resolving a reference outside
    /// [`crate::config::Config::column_refs`] rather than a reachable state.
    pub(crate) fn resolve(&self, column: &ColumnRef) -> Result<usize, ImportError> {
        match *column {
            ColumnRef::Index(index) => Ok(index),
            ColumnRef::Name(ref name) => {
                match self
                    .by_name
                    .get(&name.to_ascii_lowercase())
                    .map(Vec::as_slice)
                {
                    Some(&[index]) => Ok(index),
                    Some(indices) if indices.len() > 1 => Err(ImportError::InvalidConfig(format!(
                        "{} is ambiguous: the header names it at columns {}",
                        column.describe(),
                        indices
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ))),
                    Some(_) | None => Err(ImportError::MissingField(column.describe())),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use pretty_assertions::assert_eq;

    use super::*;

    /// Builds a header record from field names.
    fn header(names: &[&str]) -> csv::StringRecord {
        csv::StringRecord::from(names.to_vec())
    }

    #[test]
    fn build_rejects_a_duplicate_name_the_profile_addresses() {
        let column = ColumnRef::Name("Income".to_owned());
        let refs = vec![(Cow::Borrowed("date_column"), &column)];

        let err = HeaderMap::build(
            Some(&header(&["Date", "Balance", "Income", "Value", "Income"])),
            &refs,
        )
        .expect_err("a referenced duplicate must be fatal");

        let ImportError::InvalidConfig(message) = err else {
            unreachable!("expected InvalidConfig, got {err:?}")
        };
        assert!(message.contains("date_column"), "{message}");
        assert!(message.contains("'Income'"), "{message}");
        assert!(message.contains("columns 2 and 4"), "{message}");
    }

    #[test]
    fn build_reports_every_referenced_duplicate_at_once() {
        // Mirrors Config::validate's convention of listing all violations in
        // one error rather than one per call.
        let income = ColumnRef::Name("Income".to_owned());
        let value = ColumnRef::Name("Value".to_owned());
        let refs = vec![
            (Cow::Borrowed("date_column"), &income),
            (Cow::Borrowed("balance_column"), &value),
        ];

        let err = HeaderMap::build(
            Some(&header(&["Income", "Value", "Income", "Value"])),
            &refs,
        )
        .expect_err("both referenced duplicates must be fatal");

        let ImportError::InvalidConfig(message) = err else {
            unreachable!("expected InvalidConfig, got {err:?}")
        };
        assert!(message.contains("date_column"), "{message}");
        assert!(message.contains("balance_column"), "{message}");
    }

    #[test]
    fn build_only_warns_about_a_duplicate_no_field_addresses() {
        // Bank B's header carries an unnamed empty column, and trailing prose
        // can leave junk names. Neither may reject a file that imports today.
        let column = ColumnRef::Name("Date".to_owned());
        let refs = vec![(Cow::Borrowed("date_column"), &column)];

        let (columns, warnings) =
            HeaderMap::build(Some(&header(&["Date", "", "Amount", ""])), &refs)
                .expect("an unaddressed duplicate must not be fatal");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("columns 1 and 3"), "{}", warnings[0]);
        assert_eq!(columns.resolve(&column).expect("Date resolves"), 0);
    }

    #[test]
    fn resolve_matches_names_case_insensitively() {
        let (columns, warnings) = HeaderMap::build(Some(&header(&["Date", "Amount"])), &[])
            .expect("a clean header builds");

        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(
            columns
                .resolve(&ColumnRef::Name("amount".to_owned()))
                .expect("case-insensitive match"),
            1
        );
    }

    #[test]
    fn resolve_reports_a_name_absent_from_the_header() {
        let (columns, _) = HeaderMap::build(Some(&header(&["Date", "Amount"])), &[])
            .expect("a clean header builds");

        let err = columns
            .resolve(&ColumnRef::Name("Payee".to_owned()))
            .expect_err("an absent name must not resolve");

        assert_eq!(err.to_string(), "missing required field: 'Payee'");
    }

    #[test]
    fn a_headerless_file_resolves_indices_but_no_names() {
        let (columns, warnings) = HeaderMap::build(None, &[]).expect("no header builds");

        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(columns.resolve(&ColumnRef::Index(2)).expect("index"), 2);
        columns
            .resolve(&ColumnRef::Name("Date".to_owned()))
            .expect_err("names cannot resolve without a header");
    }

    #[test]
    fn resolve_refuses_an_ambiguous_name_defensively() {
        // build() makes referenced duplicates fatal, so this is unreachable in
        // production. Pinned so a future caller resolving a ref outside
        // Config::column_refs() cannot reintroduce silent first-wins.
        let (columns, _) = HeaderMap::build(Some(&header(&["Income", "Income"])), &[])
            .expect("an unaddressed duplicate builds");

        columns
            .resolve(&ColumnRef::Name("Income".to_owned()))
            .expect_err("an ambiguous name must not resolve");
    }
}
