//! Flags whose meaning comes from where they sit among a `transaction add`
//! or `transaction edit` command's arguments.
//!
//! An opener (`--posting`, `--add`, `--set`, `--remove`) starts a posting
//! scope, and every modifier after it applies to that posting until the next
//! opener. A modifier before the first opener applies to the transaction.

use core::marker::PhantomData;

use clap::ArgAction;

/// One scoped flag, named by its long form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flag {
    /// `edit`'s positional transaction ID.
    Id,
    /// `--find ACCOUNT DATE [AMOUNT]`.
    Find,
    /// `--posting ACCOUNT [AMOUNT COMMODITY]` on `add`.
    Posting,
    /// `--add ACCOUNT [AMOUNT COMMODITY]` on `edit`.
    Add,
    /// `--set POSTING`.
    Set,
    /// `--remove POSTING`.
    Remove,
    /// `--date DATE`.
    Date,
    /// `--description TEXT`.
    Description,
    /// `--meta KEY=VALUE`.
    Meta,
    /// `--clear-meta KEY`.
    ClearMeta,
    /// `--tag TAG`.
    Tag,
    /// `--untag TAG`.
    Untag,
    /// `--account ACCOUNT`.
    Account,
    /// `--amount AMOUNT COMMODITY`.
    Amount,
    /// `--cost AMOUNT COMMODITY`.
    Cost,
    /// `--total-cost AMOUNT COMMODITY`.
    TotalCost,
    /// `--lot-date DATE`.
    LotDate,
    /// `--lot-label LABEL`.
    LotLabel,
    /// `--no-cost`.
    NoCost,
    /// `--price AMOUNT COMMODITY`.
    Price,
    /// `--total-price AMOUNT COMMODITY`.
    TotalPrice,
    /// `--no-price`.
    NoPrice,
    /// `--spread FROM UNTIL`.
    Spread,
    /// `--no-spread`.
    NoSpread,
}

impl Flag {
    /// The long name, which is also the clap arg ID.
    pub(super) const fn long(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Find => "find",
            Self::Posting => "posting",
            Self::Add => "add",
            Self::Set => "set",
            Self::Remove => "remove",
            Self::Date => "date",
            Self::Description => "description",
            Self::Meta => "meta",
            Self::ClearMeta => "clear-meta",
            Self::Tag => "tag",
            Self::Untag => "untag",
            Self::Account => "account",
            Self::Amount => "amount",
            Self::Cost => "cost",
            Self::TotalCost => "total-cost",
            Self::LotDate => "lot-date",
            Self::LotLabel => "lot-label",
            Self::NoCost => "no-cost",
            Self::Price => "price",
            Self::TotalPrice => "total-price",
            Self::NoPrice => "no-price",
            Self::Spread => "spread",
            Self::NoSpread => "no-spread",
        }
    }

    /// Whether the flag takes no value.
    const fn is_switch(self) -> bool {
        matches!(self, Self::NoCost | Self::NoPrice | Self::NoSpread)
    }

    /// The clap arg for this flag, with the help text both commands share.
    pub(super) fn arg(self) -> clap::Arg {
        let named = clap::Arg::new(self.long()).long(self.long());
        let pair = |arg: clap::Arg, help: &'static str| {
            arg.action(ArgAction::Append)
                .num_args(2)
                .value_names(["AMOUNT", "COMMODITY"])
                .allow_negative_numbers(true)
                .help(help)
        };
        let one = |arg: clap::Arg, name: &'static str, help: &'static str| {
            arg.action(ArgAction::Append)
                .num_args(1)
                .value_name(name)
                .help(help)
        };
        // `ArgAction::Count` records a single index however often the switch
        // repeats, so a switch takes an empty placeholder value per occurrence.
        let switch = |arg: clap::Arg, help: &'static str| {
            arg.action(ArgAction::Append)
                .num_args(0)
                .default_missing_value("")
                .help(help)
        };
        match self {
            Self::Id => clap::Arg::new("id")
                .index(1)
                .value_name("ID")
                .help("Transaction ID. Give this, or --find."),
            Self::Find => named
                .action(ArgAction::Append)
                .num_args(2..=3)
                .value_names(["ACCOUNT", "DATE", "AMOUNT"])
                .allow_negative_numbers(true)
                .help("Find the transaction by a posting on exactly ACCOUNT dated DATE, adding its signed AMOUNT when several match."),
            Self::Posting | Self::Add => named
                .action(ArgAction::Append)
                .num_args(1..=3)
                .value_names(["ACCOUNT", "AMOUNT", "COMMODITY"])
                .allow_negative_numbers(true)
                .help("Open a new posting on ACCOUNT (path or ID). Without AMOUNT and COMMODITY its amount is elided. Posting flags after it apply to it."),
            Self::Set => named
                .action(ArgAction::Append)
                .num_args(1..=3)
                .value_names(["POSTING", "AMOUNT", "COMMODITY"])
                .allow_negative_numbers(true)
                .help("Change a stored posting: a posting ID, an account, or an account with its amount. Posting flags after it say what changes; the rest is kept."),
            Self::Remove => named
                .action(ArgAction::Append)
                .num_args(1..=3)
                .value_names(["POSTING", "AMOUNT", "COMMODITY"])
                .allow_negative_numbers(true)
                .help("Remove a stored posting, named as for --set."),
            Self::Date => one(named, "DATE", "The transaction's date (YYYY-MM-DD). Goes before the first posting."),
            Self::Description => one(named, "TEXT", "The transaction's description. Goes before the first posting.")
                .allow_hyphen_values(true),
            Self::Meta => one(named, "KEY=VALUE", "Metadata entry for the transaction, or for the posting it follows. Repeat for several; a key may repeat.")
                .allow_hyphen_values(true),
            Self::ClearMeta => one(named, "KEY", "Remove every entry under KEY from the transaction, or from the posting it follows."),
            Self::Tag => one(named, "TAG", "Tag the transaction, or the posting it follows. TAG is an ID or a path; a missing path is created."),
            Self::Untag => one(named, "TAG", "Remove a tag from the transaction, or from the posting it follows."),
            Self::Account => one(named, "ACCOUNT", "Move the posting this --set names to ACCOUNT."),
            Self::Amount => pair(named, "Replace the amount of the posting this --set names."),
            Self::Cost => pair(named, "Per-unit cost basis of the posting it follows."),
            Self::TotalCost => pair(named, "Total cost basis of the posting it follows."),
            Self::LotDate => one(named, "DATE", "Lot date of the posting's cost."),
            Self::LotLabel => one(named, "LABEL", "Lot label of the posting's cost.")
                .allow_hyphen_values(true),
            Self::Price => pair(named, "Per-unit price of the posting it follows."),
            Self::TotalPrice => pair(named, "Total price of the posting it follows."),
            Self::Spread => named
                .action(ArgAction::Append)
                .num_args(2)
                .value_names(["FROM", "UNTIL"])
                .help("Accrue the posting it follows over FROM..=UNTIL."),
            Self::NoCost => switch(named, "Clear the cost of the posting this --set names."),
            Self::NoPrice => switch(named, "Clear the price of the posting this --set names."),
            Self::NoSpread => switch(named, "Clear the spread of the posting this --set names."),
        }
    }
}

/// One flag as written, with its values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Written {
    /// The flag.
    pub flag: Flag,
    /// Its values, as typed; empty for a switch.
    pub values: Vec<String>,
}

/// The scoped flags one command accepts.
pub(super) trait FlagSet {
    /// Every flag, in the order `--help` lists them.
    const FLAGS: &'static [Flag];

    /// The clap arg for `flag` on this command.
    fn arg(flag: Flag) -> clap::Arg {
        flag.arg()
    }
}

/// The flags of `transaction add`.
#[derive(Debug, Clone, Copy)]
pub(super) struct AddFlags;

impl FlagSet for AddFlags {
    const FLAGS: &'static [Flag] = &[
        Flag::Date,
        Flag::Description,
        Flag::Posting,
        Flag::Meta,
        Flag::Tag,
        Flag::Cost,
        Flag::TotalCost,
        Flag::LotDate,
        Flag::LotLabel,
        Flag::Price,
        Flag::TotalPrice,
        Flag::Spread,
    ];

    fn arg(flag: Flag) -> clap::Arg {
        let arg = flag.arg();
        if matches!(flag, Flag::Date | Flag::Description) {
            arg.required(true)
        } else {
            arg
        }
    }
}

/// The flags of `transaction edit`.
#[derive(Debug, Clone, Copy)]
pub(super) struct EditFlags;

impl FlagSet for EditFlags {
    const FLAGS: &'static [Flag] = &[
        Flag::Id,
        Flag::Find,
        Flag::Add,
        Flag::Set,
        Flag::Remove,
        Flag::Meta,
        Flag::ClearMeta,
        Flag::Tag,
        Flag::Untag,
        Flag::Account,
        Flag::Amount,
        Flag::Cost,
        Flag::TotalCost,
        Flag::LotDate,
        Flag::LotLabel,
        Flag::NoCost,
        Flag::Price,
        Flag::TotalPrice,
        Flag::NoPrice,
        Flag::Spread,
        Flag::NoSpread,
    ];
}

/// Every scoped flag of one command, in the order it was written.
#[derive(Debug, Clone)]
pub(super) struct Scoped<S> {
    /// The flags, first to last.
    pub written: Vec<Written>,
    /// The command whose flags these are.
    pub set: PhantomData<S>,
}

impl<S: FlagSet> Scoped<S> {
    /// Reads every flag of `S` from `matches`, ordered by argv position.
    fn read(matches: &clap::ArgMatches) -> Self {
        let mut found: Vec<(usize, Written)> = Vec::new();
        for &flag in S::FLAGS {
            let Some(mut indices) = matches.indices_of(flag.long()) else {
                continue;
            };
            let Some(occurrences) = matches.get_occurrences::<String>(flag.long()) else {
                continue;
            };
            for occurrence in occurrences {
                let mut values: Vec<String> = occurrence.cloned().collect();
                // `indices_of` yields one index per value; an occurrence sits
                // at the index of its first value.
                let Some(at) = indices.next() else { break };
                for _ in 1..values.len() {
                    indices.next();
                }
                if flag.is_switch() {
                    // A switch carries only its placeholder value.
                    values.clear();
                }
                found.push((at, Written { flag, values }));
            }
        }
        found.sort_by_key(|(at, _)| *at);
        Self {
            written: found.into_iter().map(|(_, written)| written).collect(),
            set: PhantomData,
        }
    }
}

impl<S: FlagSet> clap::FromArgMatches for Scoped<S> {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        Ok(Self::read(matches))
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = Self::read(matches);
        Ok(())
    }
}

impl<S: FlagSet> clap::Args for Scoped<S> {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        S::FLAGS
            .iter()
            .fold(cmd, |command, &flag| command.arg(S::arg(flag)))
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use clap::Parser as _;
    use pretty_assertions::assert_eq;

    use super::AddFlags;
    use super::EditFlags;
    use super::Flag;
    use super::Scoped;
    use super::Written;

    #[derive(Debug, clap::Parser)]
    struct Add {
        #[command(flatten)]
        flags: Scoped<AddFlags>,
    }

    #[derive(Debug, clap::Parser)]
    struct Edit {
        #[command(flatten)]
        flags: Scoped<EditFlags>,
    }

    fn written(flag: Flag, values: &[&str]) -> Written {
        Written {
            flag,
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    #[test]
    fn flags_come_back_in_the_order_written() {
        let parsed = Add::try_parse_from([
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Groceries",
            "--meta",
            "payee=Example",
            "--posting",
            "Assets:Checking",
            "-60.00",
            "AUD",
            "--posting",
            "Expenses:Groceries",
            "60.00",
            "AUD",
            "--meta",
            "note=weekly",
            "--tag",
            "person:a",
        ])
        .expect("parses");
        assert_eq!(
            parsed.flags.written,
            vec![
                written(Flag::Date, &["2026-03-01"]),
                written(Flag::Description, &["Groceries"]),
                written(Flag::Meta, &["payee=Example"]),
                written(Flag::Posting, &["Assets:Checking", "-60.00", "AUD"]),
                written(Flag::Posting, &["Expenses:Groceries", "60.00", "AUD"]),
                written(Flag::Meta, &["note=weekly"]),
                written(Flag::Tag, &["person:a"]),
            ]
        );
    }

    #[test]
    fn a_zero_value_flag_is_recorded_at_each_occurrence() {
        let parsed = Edit::try_parse_from([
            "edit",
            "--set",
            "Expenses:A",
            "--no-cost",
            "--set",
            "Expenses:B",
            "--no-price",
            "--no-cost",
        ])
        .expect("parses");
        let flags: Vec<Flag> = parsed.flags.written.iter().map(|w| w.flag).collect();
        assert_eq!(
            flags,
            vec![
                Flag::Set,
                Flag::NoCost,
                Flag::Set,
                Flag::NoPrice,
                Flag::NoCost
            ]
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn values_that_start_with_a_hyphen_are_values() {
        let parsed = Add::try_parse_from([
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "-refund",
            "--posting",
            "Assets:Broker",
            "2",
            "ABC",
            "--cost",
            "105",
            "AUD",
            "--lot-label",
            "-a",
            "--meta",
            "note=-x",
        ])
        .expect("parses");
        let values: Vec<&str> = parsed
            .flags
            .written
            .iter()
            .filter(|w| matches!(w.flag, Flag::Description | Flag::LotLabel | Flag::Meta))
            .map(|w| w.values[0].as_str())
            .collect();
        assert_eq!(values, vec!["-refund", "-a", "note=-x"]);
    }

    #[test]
    fn negative_amounts_are_values() {
        let parsed = Edit::try_parse_from([
            "edit",
            "--find",
            "Assets:Checking",
            "2026-03-01",
            "-60.00",
            "--set",
            "Expenses:Groceries",
            "-60.00",
            "AUD",
            "--amount",
            "-5",
            "AUD",
            "--price",
            "-1",
            "AUD",
        ])
        .expect("parses");
        assert_eq!(
            parsed.flags.written,
            vec![
                written(Flag::Find, &["Assets:Checking", "2026-03-01", "-60.00"]),
                written(Flag::Set, &["Expenses:Groceries", "-60.00", "AUD"]),
                written(Flag::Amount, &["-5", "AUD"]),
                written(Flag::Price, &["-1", "AUD"]),
            ]
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn edit_reads_its_positional_id() {
        let parsed = Edit::try_parse_from([
            "edit",
            "01JTXIDPLACEHOLDERXXXXXXXX",
            "--remove",
            "Expenses:A",
        ])
        .expect("parses");
        assert_eq!(
            parsed.flags.written[0],
            written(Flag::Id, &["01JTXIDPLACEHOLDERXXXXXXXX"])
        );
    }
}
