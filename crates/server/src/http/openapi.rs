//! The OpenAPI description, built from the same router that serves the API.

use utoipa::OpenApi as _;
use utoipa::openapi::OpenApi;

use super::error::ErrorBody;

const DESCRIPTION: &str = "The HTTP API of Neon Workspace.\n\n\
Every error response has the same body: `{\"error\": {\"code\": string, \"message\": string}}`. \
`code` is stable and meant for programs, such as `not_found` or `invalid_json`; \
`message` says what went wrong, for people.";

#[derive(utoipa::OpenApi)]
#[openapi(
    info(title = "Neon Workspace API", description = DESCRIPTION),
    components(schemas(ErrorBody)),
    tags((name = "meta", description = "What the server is."))
)]
struct ApiDoc;

/// The parts of the description that no route contributes: its info, tags and shared
/// schemas. The routes add their own entries as they register.
pub(super) fn base() -> OpenApi {
    ApiDoc::openapi()
}

/// The description of every route.
pub fn spec() -> OpenApi {
    super::api().into_openapi()
}

/// The description as committed in `api/openapi.json`: pretty-printed with a trailing
/// newline. Maps in it are sorted, so the output only changes when the API does.
pub fn spec_json() -> String {
    render(&spec())
}

pub(super) fn render(spec: &OpenApi) -> String {
    let mut json = spec
        .to_pretty_json()
        .expect("the description serializes: every key in it is a string");
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_names_the_api_its_version_and_license() {
        let info = spec().info;

        assert_eq!(info.title, "Neon Workspace API");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        let license = info.license.expect("a license");
        assert_eq!(
            license.identifier.as_deref(),
            Some("PolyForm-Noncommercial-1.0.0")
        );
        assert!(
            info.description
                .is_some_and(|d| d.contains(r#"{"error": {"code""#))
        );
    }

    #[test]
    fn error_shape_is_a_shared_schema() {
        let components = spec().components.expect("components");

        assert!(components.schemas.contains_key("ErrorBody"));
        assert!(components.schemas.contains_key("ErrorDetail"));
    }

    #[test]
    fn routes_are_documented_under_the_api_prefix() {
        let paths: Vec<String> = spec().paths.paths.into_keys().collect();

        assert_eq!(paths, ["/api/v1/meta"]);
    }

    #[test]
    fn output_is_stable() {
        let json = spec_json();

        assert_eq!(json, spec_json());
        assert!(json.ends_with("}\n"));
    }
}
