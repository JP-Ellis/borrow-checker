//! QA page for the [`super`] `TomlView` primitives.

use leptos::prelude::*;
use rust_decimal::Decimal;

use super::KvKey;
use super::KvKind;
use super::KvValue;
use super::TomlArraySection;
use super::TomlAuditEntry;
use super::TomlComment;
use super::TomlKv;
use super::TomlKvEdit;
use super::TomlPosting;
use super::TomlSection;

/// Renders all `TomlView` primitives for visual inspection.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "QA showcase component with many examples"
)]
pub fn TomlViewQa() -> impl IntoView {
    let payee = RwSignal::new("Atlassian Pty Ltd".to_owned());
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px;max-width:600px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "transaction with all value types"
                </p>
                <div style="padding:16px;background:var(--bc-surface-accent);border-radius:6px">
                    <TomlSection comment="v2 schema">"transaction"</TomlSection>
                    <TomlComment>"stable identifier for this transaction"</TomlComment>
                    <TomlKv>
                        <KvKey slot>"id"</KvKey>
                        <KvValue slot kind=KvKind::Str>
                            "tx-salary-2026-04-30"
                        </KvValue>
                    </TomlKv>
                    <TomlKv>
                        <KvKey slot>"date"</KvKey>
                        <KvValue slot kind=KvKind::Date>
                            "2026-04-30"
                        </KvValue>
                    </TomlKv>
                    <TomlComment>
                        "normalised from raw import; may differ from"
                        "the description on your bank statement"
                    </TomlComment>
                    <TomlKv>
                        <KvKey slot>"payee"</KvKey>
                        <KvValue slot kind=KvKind::Str>
                            "Atlassian Pty Ltd"
                        </KvValue>
                    </TomlKv>
                    <TomlKv comment="set by autocat rule">
                        <KvKey slot>"status"</KvKey>
                        <KvValue slot kind=KvKind::Keyword>
                            "cleared"
                        </KvValue>
                    </TomlKv>
                    <TomlComment>"user-defined labels"</TomlComment>
                    <TomlKv>
                        <KvKey slot>"tags"</KvKey>
                        <KvValue slot kind=KvKind::Tags tags=vec!["work".into(), "income".into()] />
                    </TomlKv>
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "postings array"
                </p>
                <div style="padding:16px;background:var(--bc-surface-accent);border-radius:6px">
                    <TomlArraySection>"postings"</TomlArraySection>
                    <TomlPosting
                        amount=bc_ipc::Amount::new(Decimal::new(-846_154, 2), "AUD")
                        note="gross pay"
                    >
                        "Income :: Salary"
                    </TomlPosting>
                    <TomlPosting
                        amount=bc_ipc::Amount::new(Decimal::new(327_692, 2), "AUD")
                        note="PAYG"
                    >
                        "Liabilities :: Tax Withheld"
                    </TomlPosting>
                    <TomlPosting
                        amount=bc_ipc::Amount::new(Decimal::new(90_407, 2), "AUD")
                        note="11.5% SGC"
                    >
                        "Assets :: Super :: Employer"
                    </TomlPosting>
                    <TomlPosting
                        amount=bc_ipc::Amount::new(Decimal::new(428_055, 2), "AUD")
                        note="take-home"
                    >
                        "Assets :: Smart Access"
                    </TomlPosting>
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "audit log array — with inline comment"
                </p>
                <div style="padding:16px;background:var(--bc-surface-accent);border-radius:6px">
                    <TomlArraySection comment="tx-salary-2026-04-30">"audit_log"</TomlArraySection>
                    <TomlAuditEntry time="09:04" kind="import">
                        "from commbank-au.wasm@1.4.2"
                    </TomlAuditEntry>
                    <TomlAuditEntry time="09:04" kind="autocat">
                        "rule \"payee=~/atlassian/i → Income::Salary\""
                    </TomlAuditEntry>
                    <TomlAuditEntry time="09:04" kind="split" comment="4 postings created">
                        "applied rule \"salary-split-au\""
                    </TomlAuditEntry>
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "editable kv row (TomlKvEdit)"
                </p>
                <div style="padding:16px;background:var(--bc-surface-accent);border-radius:6px">
                    <TomlKvEdit key="payee" value=payee kind=KvKind::Str />
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "posting without note (optional prop absent)"
                </p>
                <div style="padding:16px;background:var(--bc-surface-accent);border-radius:6px">
                    <TomlArraySection>"postings"</TomlArraySection>
                    <TomlPosting amount=bc_ipc::Amount::new(
                        Decimal::new(-8_420, 2),
                        "AUD",
                    )>"Assets :: Smart Access"</TomlPosting>
                    <TomlPosting amount=bc_ipc::Amount::new(
                        Decimal::new(8_420, 2),
                        "AUD",
                    )>"Expenses :: Groceries"</TomlPosting>
                </div>
            </section>

        </div>
    }
}
