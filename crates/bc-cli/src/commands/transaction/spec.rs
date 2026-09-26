//! Resolving an account path or ID typed on the command line.

use core::str::FromStr as _;

/// Resolves an account path or ID, as typed, to an account ID.
pub(super) type Lookup<'a> = dyn Fn(&str) -> Option<bc_models::AccountId> + 'a;

/// Builds a [`Lookup`] over `resolver` that accepts an account ID or a path.
///
/// Either must name an existing account. An unknown ID would otherwise fail
/// only at the write, after `add` or `edit` has created any new tag.
pub(super) fn account_lookup(
    resolver: &bc_core::AccountResolver,
) -> impl Fn(&str) -> Option<bc_models::AccountId> + '_ {
    move |text| {
        #[expect(clippy::shadow_reuse, reason = "trim yields the same string")]
        let text = text.trim();
        if let Ok(id) = bc_models::AccountId::from_str(text) {
            return resolver.path_of(&id).is_some().then_some(id);
        }
        let path = bc_core::AccountPath::parse(text).ok()?;
        if let bc_core::Resolution::Resolved { id, .. } = resolver.resolve(&path) {
            Some(id)
        } else {
            None
        }
    }
}
