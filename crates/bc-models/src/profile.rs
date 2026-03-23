//! Profile entity identifier.

use core::fmt;
use core::str::FromStr;

use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

crate::define_id!(ProfileId, "profile");

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
    fn profile_id_has_correct_prefix() {
        assert!(ProfileId::new().to_string().starts_with("profile_"));
    }

    #[test]
    fn profile_id_roundtrip_display_parse() {
        let id = ProfileId::new();
        let parsed: ProfileId = id.to_string().parse().expect("valid ProfileId");
        assert_eq!(id, parsed);
    }

    #[test]
    fn profile_id_serialize() {
        assert_json_snapshot!(ProfileId::new(), @r#""profile_[id]""#);
    }
}
