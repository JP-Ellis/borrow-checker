//! The parsed query, before any name is resolved.

use crate::span::Span;

/// A parsed query expression.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Expr exhaustively"
)]
pub enum Expr {
    /// Two or more alternatives; matches when any does.
    Or(Vec<Expr>, Span),
    /// Two or more conjuncts; matches when all do.
    And(Vec<Expr>, Span),
    /// Negation, written `-x` or `not x`.
    Not(Box<Expr>, Span),
    /// A `field:criterion` term.
    Term(Term),
    /// Free text, searched in the description.
    Word(Value),
}

/// A `field:criterion` term.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Term {
    /// The field before the colon.
    pub field: Field,
    /// Everything after the colon.
    pub criterion: Criterion,
    /// The whole term.
    pub span: Span,
}

/// A field name as written: a built-in (`account`) or a metadata key (`@payee`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Field {
    /// The name without `@`, as typed (case preserved).
    pub name: String,
    /// Whether the name carried the `@` metadata sigil.
    pub meta: bool,
    /// The name, including any `@`.
    pub span: Span,
}

/// A literal value: bare text or the unescaped contents of a quoted string.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Value {
    /// The text, with quotes removed and escapes applied.
    pub text: String,
    /// The value as written, including any quotes.
    pub span: Span,
}

/// What a term asks of its field.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Criterion exhaustively"
)]
pub enum Criterion {
    /// A bare `*`: the field has any value.
    Any(Span),
    /// An optional operator and one value.
    Compare {
        /// The operator; [`Op::Match`] when none was written.
        op: Op,
        /// The value.
        value: Value,
        /// The operator and value.
        span: Span,
    },
    /// `lo..hi`, either end optional.
    Range {
        /// The lower end.
        lo: Option<Value>,
        /// The upper end.
        hi: Option<Value>,
        /// The whole range.
        span: Span,
    },
    /// A parenthesised sub-expression, as `any:(…)` takes.
    Group(Box<Expr>, Span),
}

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Op exhaustively"
)]
pub enum Op {
    /// No operator: the field's default match.
    Match,
    /// `=`.
    Equal,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
}

impl Op {
    /// The operator as written; empty for [`Op::Match`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Match => "",
            Self::Equal => "=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
        }
    }
}

impl Expr {
    /// The text this expression was parsed from.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Or(_, span) | Self::And(_, span) | Self::Not(_, span) => *span,
            Self::Term(term) => term.span,
            Self::Word(value) => value.span,
        }
    }

    /// A copy with every span zeroed, for comparing structure alone.
    #[must_use]
    pub fn without_spans(&self) -> Self {
        match self {
            Self::Or(items, _) => Self::Or(
                items.iter().map(Self::without_spans).collect(),
                Span::default(),
            ),
            Self::And(items, _) => Self::And(
                items.iter().map(Self::without_spans).collect(),
                Span::default(),
            ),
            Self::Not(inner, _) => Self::Not(Box::new(inner.without_spans()), Span::default()),
            Self::Term(term) => Self::Term(term.without_spans()),
            Self::Word(value) => Self::Word(value.without_spans()),
        }
    }
}

impl Term {
    /// Creates a term.
    ///
    /// # Arguments
    ///
    /// * `field` - The field before the colon.
    /// * `criterion` - Everything after the colon.
    /// * `span` - The whole term.
    #[must_use]
    pub const fn new(field: Field, criterion: Criterion, span: Span) -> Self {
        Self {
            field,
            criterion,
            span,
        }
    }

    /// A copy with every span zeroed.
    fn without_spans(&self) -> Self {
        Self::new(
            Field::new(&self.field.name, self.field.meta, Span::default()),
            self.criterion.without_spans(),
            Span::default(),
        )
    }
}

impl Field {
    /// Creates a field name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name without `@`.
    /// * `meta` - Whether it names a metadata key.
    /// * `span` - The name as written.
    #[must_use]
    pub fn new(name: &str, meta: bool, span: Span) -> Self {
        Self {
            name: name.to_owned(),
            meta,
            span,
        }
    }
}

impl Value {
    /// Creates a value.
    ///
    /// # Arguments
    ///
    /// * `text` - The unescaped text.
    /// * `span` - The value as written.
    #[must_use]
    pub fn new(text: &str, span: Span) -> Self {
        Self {
            text: text.to_owned(),
            span,
        }
    }

    /// A copy with the span zeroed.
    fn without_spans(&self) -> Self {
        Self::new(&self.text, Span::default())
    }
}

impl Criterion {
    /// The criterion as written.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Any(span)
            | Self::Compare { span, .. }
            | Self::Range { span, .. }
            | Self::Group(_, span) => *span,
        }
    }

    /// A copy with every span zeroed.
    fn without_spans(&self) -> Self {
        match self {
            Self::Any(_) => Self::Any(Span::default()),
            Self::Compare { op, value, .. } => Self::Compare {
                op: *op,
                value: value.without_spans(),
                span: Span::default(),
            },
            Self::Range { lo, hi, .. } => Self::Range {
                lo: lo.as_ref().map(Value::without_spans),
                hi: hi.as_ref().map(Value::without_spans),
                span: Span::default(),
            },
            Self::Group(inner, _) => Self::Group(Box::new(inner.without_spans()), Span::default()),
        }
    }
}
