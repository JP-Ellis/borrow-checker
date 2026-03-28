//! Event entity identifier.

crate::define_id!(EventId, "event");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_id_has_correct_prefix() {
        assert!(EventId::new().to_string().starts_with("event_"));
    }
}
