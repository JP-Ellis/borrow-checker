//! Profile entity identifier.

crate::define_id!(ProfileId, "profile");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_id_has_correct_prefix() {
        assert!(ProfileId::new().to_string().starts_with("profile_"));
    }
}
