//! QA test pages — available only in debug builds via `/__test/*` routes.

mod num;
mod root;
mod status_pill;
mod tag_token;

pub use num::NumTest;
pub use root::Root;
pub use status_pill::StatusPillTest;
pub use tag_token::TagTokenTest;
