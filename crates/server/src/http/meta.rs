//! `GET /api/v1/meta`: tells a client it reached a Neon server, and which version.

use axum::extract::State;
use neon_app::settings::INSTANCE_NAME;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::AppState;
use super::error::ErrorBody;
use super::extract::Json;

/// What [`Meta::product`] always says: a client that reads anything else did not reach a Neon
/// Teamspace server.
pub const PRODUCT: &str = "neon-teamspace";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Meta {
    /// Always `neon-teamspace`, so a client can tell this server from any other that answers
    /// JSON at the same path, as the apps do before they load a server's pages.
    #[schema(example = "neon-teamspace")]
    pub product: String,
    /// The instance's name, from the `instance.name` setting.
    #[schema(example = "Neon Teamspace")]
    pub name: String,
    /// The server's version.
    #[schema(example = json!(env!("CARGO_PKG_VERSION")))]
    pub version: String,
}

#[utoipa::path(
    get,
    path = "/meta",
    tag = "meta",
    summary = "Instance metadata",
    description = "The product, the instance's name and the server's version. Needs no \
                   authentication, so clients can check they reached a Neon server.",
    responses(
        (status = OK, description = "Instance metadata.", body = Meta),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn get_meta(State(state): State<AppState>) -> Json<Meta> {
    Json(Meta {
        product: PRODUCT.to_owned(),
        name: state.settings.current().get(INSTANCE_NAME).to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}
