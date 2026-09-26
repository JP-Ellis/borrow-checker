//! Flags whose meaning comes from where they sit among a `transaction add`
//! or `transaction edit` command's arguments.
//!
//! An opener (`--posting`, `--add`, `--set`, `--remove`) starts a posting
//! scope, and every modifier after it applies to that posting until the next
//! opener. A modifier before the first opener applies to the transaction.

use core::marker::PhantomData;

use clap::ArgAction;

use crate::error::CliError;
use crate::error::CliResult;

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

    /// Why this modifier cannot sit at `place`, or `None` when it can.
    fn refusal(self, place: Place) -> Option<&'static str> {
        use Place::AddTransaction;
        use Place::EditTransaction;
        use Place::NewLeg;
        use Place::RemoveLeg;
        use Place::SetLeg;
        match (self, place) {
            (_, RemoveLeg) => Some("a removed leg takes no posting flags"),
            (Self::Date | Self::Description, AddTransaction | EditTransaction)
            | (Self::Meta | Self::Tag, _)
            | (Self::ClearMeta | Self::Untag, EditTransaction | SetLeg)
            | (
                Self::Account | Self::Amount | Self::NoCost | Self::NoPrice | Self::NoSpread,
                SetLeg,
            )
            | (
                Self::Cost
                | Self::TotalCost
                | Self::LotDate
                | Self::LotLabel
                | Self::Price
                | Self::TotalPrice
                | Self::Spread,
                NewLeg | SetLeg,
            ) => None,
            (Self::Date | Self::Description | Self::Id | Self::Find, _) => {
                Some("transaction flags go before the first posting")
            }
            (Self::ClearMeta | Self::Untag, NewLeg) => Some("a new leg has nothing to remove"),
            (Self::Account | Self::Amount, NewLeg) => {
                Some("the opener already names the account and amount")
            }
            (Self::NoCost | Self::NoPrice | Self::NoSpread, NewLeg) => {
                Some("a new leg has nothing to clear")
            }
            (_, AddTransaction | EditTransaction) => Some("it describes a posting"),
            _ => Some("it does not apply here"),
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

/// The `--help` heading every named scoped flag sits under, apart from the
/// global options.
const HEADING: &str = "Transaction and posting flags";

impl<S: FlagSet> clap::Args for Scoped<S> {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        S::FLAGS.iter().fold(cmd, |command, &flag| {
            let arg = S::arg(flag);
            // The positional ID stays under "Arguments".
            command.arg(if flag == Flag::Id {
                arg
            } else {
                arg.help_heading(HEADING)
            })
        })
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

/// The command a [`Plan`] is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    /// `transaction add`.
    Add,
    /// `transaction edit`.
    Edit,
}

/// What an opener names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Opener {
    /// A new leg: its account, and its amount and commodity unless elided.
    New {
        /// The account path or ID, as typed.
        account: String,
        /// The amount and commodity, as typed.
        amount: Option<[String; 2]>,
    },
    /// A stored leg to change, named by one or three tokens.
    Set(Vec<String>),
    /// A stored leg to drop, named by one or three tokens.
    Remove(Vec<String>),
}

/// One opener and the modifiers after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Scope {
    /// What the opener names.
    pub opener: Opener,
    /// The opener as typed, for messages.
    pub label: String,
    /// The modifiers, in order.
    pub modifiers: Vec<Written>,
}

/// A command's scoped flags, grouped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Plan {
    /// `edit`'s transaction ID.
    pub id: Option<String>,
    /// `edit`'s `--find` values.
    pub find: Option<Vec<String>>,
    /// Modifiers before the first opener.
    pub transaction: Vec<Written>,
    /// Each opener with its modifiers, in order.
    pub scopes: Vec<Scope>,
}

/// Where a modifier sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Before the first opener of `add`.
    AddTransaction,
    /// Before the first opener of `edit`.
    EditTransaction,
    /// After `--posting` or `--add`.
    NewLeg,
    /// After `--set`.
    SetLeg,
    /// After `--remove`.
    RemoveLeg,
}

/// Renders a flag and its values as typed.
fn label(written: &Written) -> String {
    let mut out = format!("--{}", written.flag.long());
    for value in &written.values {
        out.push(' ');
        out.push_str(value);
    }
    out
}

/// Reads a one-or-three-token opener, refusing two.
fn one_or_three(written: &Written) -> CliResult<Vec<String>> {
    if written.values.len() == 2 {
        return Err(CliError::Arg(format!(
            "{}: an amount needs its commodity",
            label(written)
        )));
    }
    Ok(written.values.clone())
}

/// Groups `written` into a transaction scope and one scope per opener.
///
/// # Errors
///
/// Returns [`CliError::Arg`] naming the flag and the opener it follows when
/// a flag sits where it does not apply, when an opener names an amount
/// without its commodity, when `--find` is given twice, or when `edit` names
/// its transaction by neither or both of `ID` and `--find`.
pub(super) fn fold(written: &[Written], command: Command) -> CliResult<Plan> {
    let mut plan = Plan::default();
    for item in written {
        let open = plan.scopes.last();
        match item.flag {
            Flag::Id | Flag::Find if open.is_some() => {
                return Err(misplaced(
                    item,
                    open,
                    "transaction flags go before the first posting",
                ));
            }
            Flag::Id => plan.id = item.values.first().cloned(),
            Flag::Find => {
                if plan.find.is_some() {
                    return Err(CliError::Arg("--find is given twice".into()));
                }
                plan.find = Some(item.values.clone());
            }
            Flag::Posting | Flag::Add => {
                let tokens = one_or_three(item)?;
                let mut values = tokens.into_iter();
                let account = values.next().unwrap_or_default();
                let amount = match (values.next(), values.next()) {
                    (Some(value), Some(code)) => Some([value, code]),
                    _ => None,
                };
                plan.scopes.push(Scope {
                    opener: Opener::New { account, amount },
                    label: label(item),
                    modifiers: Vec::new(),
                });
            }
            Flag::Set | Flag::Remove => {
                let tokens = one_or_three(item)?;
                let opener = if item.flag == Flag::Set {
                    Opener::Set(tokens)
                } else {
                    Opener::Remove(tokens)
                };
                plan.scopes.push(Scope {
                    opener,
                    label: label(item),
                    modifiers: Vec::new(),
                });
            }
            Flag::Date
            | Flag::Description
            | Flag::Meta
            | Flag::ClearMeta
            | Flag::Tag
            | Flag::Untag
            | Flag::Account
            | Flag::Amount
            | Flag::Cost
            | Flag::TotalCost
            | Flag::LotDate
            | Flag::LotLabel
            | Flag::NoCost
            | Flag::Price
            | Flag::TotalPrice
            | Flag::NoPrice
            | Flag::Spread
            | Flag::NoSpread => {
                let place = match (open.map(|scope| &scope.opener), command) {
                    (None, Command::Add) => Place::AddTransaction,
                    (None, Command::Edit) => Place::EditTransaction,
                    (Some(Opener::New { .. }), _) => Place::NewLeg,
                    (Some(Opener::Set(_)), _) => Place::SetLeg,
                    (Some(Opener::Remove(_)), _) => Place::RemoveLeg,
                };
                if let Some(reason) = item.flag.refusal(place) {
                    return Err(misplaced(item, open, reason));
                }
                match plan.scopes.last_mut() {
                    Some(scope) => scope.modifiers.push(item.clone()),
                    None => plan.transaction.push(item.clone()),
                }
            }
        }
    }
    if command == Command::Edit {
        match (&plan.id, &plan.find) {
            (Some(_), Some(_)) => {
                return Err(CliError::Arg(
                    "give a transaction ID or --find, not both".into(),
                ));
            }
            (None, None) => {
                return Err(CliError::Arg(
                    "give a transaction ID, or --find ACCOUNT DATE [AMOUNT]".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(plan)
}

/// How messages name `flag`: `--long`, or "the transaction ID" for the
/// positional ID, which has no flag form.
fn named(flag: Flag) -> String {
    if flag == Flag::Id {
        "the transaction ID".to_owned()
    } else {
        format!("--{}", flag.long())
    }
}

/// The error for `item` sitting where it does not apply.
fn misplaced(item: &Written, open: Option<&Scope>, reason: &str) -> CliError {
    match open {
        Some(scope) => CliError::Arg(format!(
            "{} follows {}: {reason}",
            named(item.flag),
            scope.label
        )),
        None => CliError::Arg(format!(
            "{} describes a posting; put it after the posting it belongs to",
            named(item.flag)
        )),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use clap::Parser as _;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

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

    #[expect(
        clippy::unwrap_in_result,
        reason = "a parse failure here is a test bug"
    )]
    fn fold_add(extra: &[&str]) -> crate::error::CliResult<super::Plan> {
        let mut argv = vec!["add", "--date", "2026-03-01", "--description", "Groceries"];
        argv.extend_from_slice(extra);
        let parsed = Add::try_parse_from(argv).expect("parses");
        super::fold(&parsed.flags.written, super::Command::Add)
    }

    #[expect(
        clippy::unwrap_in_result,
        reason = "a parse failure here is a test bug"
    )]
    fn fold_edit(extra: &[&str]) -> crate::error::CliResult<super::Plan> {
        let mut argv = vec!["edit"];
        argv.extend_from_slice(extra);
        let parsed = Edit::try_parse_from(argv).expect("parses");
        super::fold(&parsed.flags.written, super::Command::Edit)
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn modifiers_bind_to_the_opener_before_them() {
        let plan = fold_add(&[
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
            "--tag",
            "person:a",
        ])
        .expect("folds");
        let tx: Vec<Flag> = plan.transaction.iter().map(|w| w.flag).collect();
        assert_eq!(tx, vec![Flag::Date, Flag::Description, Flag::Meta]);
        assert_eq!(plan.scopes.len(), 2);
        assert!(plan.scopes[0].modifiers.is_empty());
        assert_eq!(
            plan.scopes[1].modifiers,
            vec![written(Flag::Tag, &["person:a"])]
        );
        assert_eq!(
            plan.scopes[1].opener,
            super::Opener::New {
                account: "Expenses:Groceries".to_owned(),
                amount: Some(["60.00".to_owned(), "AUD".to_owned()]),
            }
        );
        assert_eq!(
            plan.scopes[1].label,
            "--posting Expenses:Groceries 60.00 AUD"
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn an_account_alone_opens_an_elided_leg() {
        let plan = fold_add(&[
            "--posting",
            "Assets:Checking",
            "-5",
            "AUD",
            "--posting",
            "Equity:Opening",
        ])
        .expect("folds");
        assert_eq!(
            plan.scopes[1].opener,
            super::Opener::New {
                account: "Equity:Opening".to_owned(),
                amount: None
            }
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn edit_folds_a_valid_command() {
        let plan =
            fold_edit(&["ID0", "--set", "Expenses:Office", "--tag", "person:a"]).expect("folds");
        assert_eq!(plan.id, Some("ID0".to_owned()));
        assert_eq!(plan.scopes.len(), 1);
        assert_eq!(
            plan.scopes[0].modifiers,
            vec![written(Flag::Tag, &["person:a"])]
        );
    }

    #[rstest]
    #[case::amount_without_commodity(&["--posting", "Assets:Checking", "-5"], "an amount needs its commodity")]
    #[case::posting_flag_on_transaction(&["--cost", "105", "AUD"], "--cost describes a posting")]
    #[case::date_after_opener(&["--posting", "Assets:Checking", "-5", "AUD", "--date", "2026-03-02"], "--date follows --posting Assets:Checking -5 AUD")]
    fn add_rejects(#[case] args: &[&str], #[case] expected: &str) {
        let err = fold_add(args).expect_err("rejects").to_string();
        assert!(err.contains(expected), "got: {err}");
    }

    #[rstest]
    #[case::untag_on_new_leg(&["ID0", "--add", "Expenses:Office", "5", "AUD", "--untag", "person:a"], "--untag follows --add Expenses:Office 5 AUD: a new leg has nothing to remove")]
    #[case::account_on_new_leg(&["ID0", "--add", "Expenses:Office", "5", "AUD", "--account", "Expenses:Other"], "the opener already names the account and amount")]
    #[case::no_cost_on_new_leg(&["ID0", "--add", "Expenses:Office", "5", "AUD", "--no-cost"], "a new leg has nothing to clear")]
    #[case::modifier_on_remove(&["ID0", "--remove", "Expenses:Office", "--tag", "person:a"], "a removed leg takes no posting flags")]
    #[case::account_on_transaction(&["ID0", "--account", "Expenses:Other"], "--account describes a posting")]
    #[case::find_after_opener(&["--remove", "Expenses:Office", "--find", "Assets:Checking", "2026-03-01"], "--find follows --remove Expenses:Office")]
    #[case::find_one_value(&["--find", "Assets:Checking"], "")]
    #[case::find_twice(&["--find", "Assets:Checking", "2026-03-01", "--find", "Assets:Checking", "2026-03-02"], "--find is given twice")]
    #[case::id_after_opener(&["--remove", "Expenses:Office", "5", "AUD", "ID0"], "the transaction ID follows --remove Expenses:Office 5 AUD")]
    #[case::id_and_find(&["ID0", "--find", "Assets:Checking", "2026-03-01"], "give a transaction ID or --find, not both")]
    #[case::neither(&["--remove", "Expenses:Office"], "give a transaction ID, or --find")]
    fn edit_rejects(#[case] args: &[&str], #[case] expected: &str) {
        let Ok(parsed) = Edit::try_parse_from(std::iter::once("edit").chain(args.iter().copied()))
        else {
            // `--find` with one value is clap's to reject.
            assert_eq!(expected, "");
            return;
        };
        let err = super::fold(&parsed.flags.written, super::Command::Edit)
            .expect_err("rejects")
            .to_string();
        assert!(err.contains(expected), "got: {err}");
    }
}
