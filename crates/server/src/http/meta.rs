//! `GET /api/v1/meta`: tells a client it reached a Neon server, and which version.

use axum::extract::State;
use neon_app::settings::INSTANCE_NAME;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::AppState;
use super::error::ErrorBody;
use super::extract::Json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Meta {
    /// The instance's name, from the `instance.name` setting.
    #[schema(example = "Neon Workspace")]
    pub name: String,
    /// The server's version.
    #[schema(example = "0.1.0")]
    pub version: String,
}

#[utoipa::path(
    get,
    path = "/meta",
    tag = "meta",
    summary = "Instance metadata",
    description = "The instance's name and the server's version. Needs no authentication, \
                   so clients can check they reached a Neon server.",
    responses(
        (status = OK, description = "Instance metadata.", body = Meta),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn get_meta(State(state): State<AppState>) -> Json<Meta> {
    Json(Meta {
        name: state.settings.current().get(INSTANCE_NAME).to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}
