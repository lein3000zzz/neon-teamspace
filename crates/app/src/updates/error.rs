/// Why an updates use case refused.
///
/// Every variant is expected: the caller's to fix, or a GitHub or an updater that does not
/// answer, never a failure of the server. Messages are safe to show and never hold the
/// updater's token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdatesError {
    #[error("{0}")]
    Forbidden(&'static str),
    /// `updates.updater_token` is empty.
    #[error(
        "no updater is set up: run the Compose file's updater profile and set \
         updates.updater_token, as the upgrading docs explain, or update by hand"
    )]
    NoUpdater,
    /// The updater is updating the stack already.
    #[error("an update is under way already")]
    UpdateInProgress,
    /// The version asked for is not a release to update to: not a version, not one the last
    /// check saw, not newer than the server's, or the updater refused it.
    #[error("{0}")]
    NotAllowed(String),
    /// The updater did not answer, refused the server's token, or is at no URL.
    #[error("{0}")]
    UpdaterUnreachable(String),
}

impl UpdatesError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Forbidden(_) => "forbidden",
            Self::NoUpdater => "no_updater",
            Self::UpdateInProgress => "update_in_progress",
            Self::NotAllowed(_) => "update_not_allowed",
            Self::UpdaterUnreachable(_) => "updater_unreachable",
        }
    }
}
