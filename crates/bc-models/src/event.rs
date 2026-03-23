//! Event entity identifier.

use core::fmt;
use core::str::FromStr;

use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

crate::define_id!(EventId, "event");

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    macro_rules! assert_json_snapshot {
        ($value:expr, @$snapshot:literal) => {
            insta::with_settings!({ filters => vec![("_[a-z0-9]{26}", "_[id]")] }, {
                insta::assert_json_snapshot!($value, @$snapshot);
            });
        }
    }

    #[test]
    fn event_id_has_correct_prefix() {
        assert!(EventId::new().to_string().starts_with("event_"));
    }

    #[test]
    fn event_id_roundtrip_display_parse() {
        let id = EventId::new();
        let parsed: EventId = id.to_string().parse().expect("valid EventId");
        assert_eq!(id, parsed);
    }

    #[test]
    fn event_id_serialize() {
        assert_json_snapshot!(EventId::new(), @r#""event_[id]""#);
    }
}
