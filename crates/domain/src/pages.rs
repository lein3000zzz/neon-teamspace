//! Pages: a workspace's documents, which nest under one another.
//!
//! A page is either in a teamspace or private to one member. Its subpages are in the same
//! place: a teamspace's pages nest under pages of that teamspace, and a member's private
//! pages under their own private pages. Moving a page moves its subpages with it, and so
//! does trashing it.
//!
//! Who may see and change a page is the rules' to say, in
//! [`authorize`](crate::workspaces::authorize): a page takes its access from its teamspace,
//! or from its owner for a private page, and from what it, or a page above it, is shared
//! for.

pub mod files;
mod page;
pub mod position;

pub use page::{
    Cover, Font, InvalidTitle, MAX_DEPTH, MAX_TITLE, Page, PageAccess, PageKind, PageStyle,
    Placement, Share, Space, check_title,
};
pub use position::InvalidPosition;
