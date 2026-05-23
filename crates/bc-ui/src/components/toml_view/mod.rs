//! Prettified TOML-like view components.
//!
//! These are read-only rendering primitives. They produce the IDE-aesthetic
//! data display used in the transaction detail panel and elsewhere.
//! The output is *not* valid TOML — it is a visual language inspired by TOML.
#![allow(
    dead_code,
    reason = "items unused until connected in later tasks; audited at end of milestone"
)]

use bc_ipc::Money;
use bc_ipc::USD;
use bc_ipc::currency_from_code;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::num::format_amount;

import_style!(style, "toml_view.module.scss");

/// Renders a `[section]` header line.
///
/// The section name is passed as children, e.g. `<TomlSection>"transaction"</TomlSection>`.
#[component]
pub fn TomlSection(
    /// Section name rendered between `[` `]`.
    children: Children,
    /// Optional comment shown after `  #` on the same line, right-aligned.
    #[prop(optional, into)]
    comment: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::section>
            "[" {children()} "]"
            {comment.map(|c| view! { <span class=style::row_comment>"  # "{c}</span> })}
        </div>
    }
}

/// Renders a `[[array_section]]` header line with an optional inline comment.
///
/// The section name is passed as children; the trailing `# comment` is a prop.
#[component]
pub fn TomlArraySection(
    /// Section name rendered between `[[` `]]`.
    children: Children,
    /// Optional comment shown after `  #` on the same line.
    #[prop(optional, into)]
    comment: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::section>
            "[[" {children()} "]]"
            {comment.map(|c| view! { <span class=style::row_comment>"  # "{c}</span> })}
        </div>
    }
}

/// Renders a standalone `# comment` line placed above the element it annotates.
///
/// Consecutive `TomlComment` elements stack flush against each other and hug
/// the following row via the sibling CSS rules on `.comment_line`.
#[component]
pub fn TomlComment(
    /// Comment text (the `#` prefix is added automatically).
    children: Children,
) -> impl IntoView {
    view! { <div class=style::comment_line>{children()}</div> }
}

/// Value kind for a [`KvValue`] slot, determining colour and formatting.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum KvKind {
    /// Quoted string — rendered in `--bc-string` (green).
    Str,
    /// ISO-8601 date — rendered in `--bc-type` (blue).
    Date,
    /// Keyword / enum — rendered in `--bc-keyword` (accent/rust).
    Keyword,
    /// Array of tag strings — rendered as `[ "tag1", "tag2" ]`.
    Tags,
}

/// Key slot for [`TomlKv`] — children become the left-hand identifier.
#[slot]
pub struct KvKey {
    /// Key identifier text.
    children: Children,
}

/// Value slot for [`TomlKv`].
///
/// For [`KvKind::Str`], [`KvKind::Date`], and [`KvKind::Keyword`], pass the
/// value as children.  For [`KvKind::Tags`], supply `tags` instead; children
/// are ignored.
#[slot]
pub struct KvValue {
    /// Controls colour and formatting.
    kind: KvKind,
    /// Tag strings; only used when `kind` is [`KvKind::Tags`].
    #[prop(optional)]
    tags: Vec<String>,
    /// Value content for non-Tags kinds.
    #[prop(optional)]
    children: Option<Children>,
}

/// Renders a `key = value` row in the TOML-like style.
///
/// Pass the key name and value as typed child slots:
///
/// ```
/// <TomlKv>
///     <KvKey>"date"</KvKey>
///     <KvValue kind=KvKind::Date>"2026-04-30"</KvValue>
/// </TomlKv>
/// ```
#[component]
pub fn TomlKv(
    /// Key slot — children become the left-hand identifier.
    kv_key: KvKey,
    /// Value slot — carries the kind, optional tags, and optional children.
    kv_value: KvValue,
    /// Optional comment shown after `  #` on the same line, right-aligned.
    #[prop(optional, into)]
    comment: Option<String>,
) -> impl IntoView {
    let KvValue {
        kind,
        tags,
        children,
    } = kv_value;

    let value_view = match kind {
        KvKind::Str => {
            view! { <span class=style::str_val>"\""{children.map(|c| c())}"\""</span> }.into_any()
        }
        KvKind::Date => {
            view! { <span class=style::date_val>{children.map(|c| c())}</span> }.into_any()
        }
        KvKind::Keyword => {
            view! { <span class=style::kw_val>{children.map(|c| c())}</span> }.into_any()
        }
        KvKind::Tags => {
            let inner = tags
                .into_iter()
                .enumerate()
                .map(|(i, t)| {
                    view! {
                        {(i != 0).then_some(", ")}
                        <span class=style::str_val>"\""{t}"\""</span>
                    }
                })
                .collect::<Vec<_>>();
            view! { <span class=style::tag_val>"[ "{inner}" ]"</span> }.into_any()
        }
    };

    view! {
        <div class=style::kv_row>
            <span class=style::key>{(kv_key.children)()}</span>
            <span class=style::eq>"="</span>
            {value_view}
            {comment.map(|c| view! { <span class=style::row_comment>"  # "{c}</span> })}
        </div>
    }
}

/// Renders one posting row: account path (left) and amount (right).
///
/// The account path is passed as children; amount and optional note are props.
#[component]
#[expect(clippy::needless_pass_by_value, reason = "Leptos requires owned props")]
pub fn TomlPosting(
    /// Monetary amount (minor units + currency code).
    amount: Money,
    /// Optional inline comment shown above the posting row.
    #[prop(optional, into)]
    note: Option<String>,
    /// Account path, e.g. `"Assets :: Smart Access"`.
    children: Children,
) -> impl IntoView {
    let currency = currency_from_code(&amount.currency_code).unwrap_or(&USD);
    let amount_str = format_amount(amount.minor_units, currency);
    let amt_class = if amount.minor_units >= 0 {
        style::amt_pos
    } else {
        style::amt_neg
    };

    view! {
        <div class=style::posting>
            {note.map(|n| view! { <div class=style::posting_note>"# "{n}</div> })}
            <div class=style::posting_row>
                <span class=style::posting_acct>{children()}</span>
                <span class=format!("{} {}", style::posting_amt, amt_class)>{amount_str}</span>
            </div>
        </div>
    }
}

/// Renders one audit log entry: timestamp, kind badge, and message.
///
/// The message is passed as children; timestamp and kind are props.
#[component]
pub fn TomlAuditEntry(
    /// Timestamp string, e.g. `"09:04"`.
    #[prop(into)]
    time: String,
    /// Event kind badge, e.g. `"import"`.
    #[prop(into)]
    kind: String,
    /// Optional trailing inline comment, right-aligned.
    #[prop(optional, into)]
    comment: Option<String>,
    /// Human-readable message describing the audit event.
    children: Children,
) -> impl IntoView {
    view! {
        <div class=style::audit_row>
            <span class=style::audit_time>{time}</span>
            <span class=style::audit_kind>"["{kind}"]"</span>
            <span class=style::audit_msg>{children()}</span>
            {comment.map(|c| view! { <span class=style::row_comment>"  # "{c}</span> })}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
