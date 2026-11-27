//! The MCP server's handler: what it tells clients it is and can do, its tools, and its
//! resources with their subscriptions. One handler serves every session; each request
//! carries the principal the gate authenticated it as.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use axum::http::request::Parts;
use neon_domain::auth::Principal;
use neon_observability::errors::{mark_expected, mark_unexpected};
use neon_observability::red::Red;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CompleteRequestParams, CompleteResult,
    ErrorCode, ErrorData, GetPromptRequestParams, GetPromptResponse, Implementation,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ServerCapabilities, ServerConfig, SubscribeRequestParams, Tool,
    UnsubscribeRequestParams,
};
use rmcp::service::RequestContext;
use rmcp::transport::common::http_header::HEADER_SESSION_ID;
use rmcp::{RoleServer, ServerHandler};
use tracing::{Instrument as _, Span, info_span};

use super::resources::{self, Resource};
use super::sessions::Sessions;
use super::tools::{self, ToolError};
use super::{RequestSpan, Services, watch};

/// The oldest revision of the protocol the server speaks: the first whose transport has
/// sessions over Streamable HTTP. An `initialize` asking for an older one, or one the server
/// does not know, is answered with [`NEWEST`].
const OLDEST: ProtocolVersion = ProtocolVersion::V_2025_03_26;

/// The newest revision the server speaks, the last with an `initialize` handshake.
const NEWEST: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// The revisions of the protocol the server speaks, oldest first: from [`OLDEST`] to
/// [`NEWEST`].
fn protocol_versions() -> &'static [ProtocolVersion] {
    let known = ProtocolVersion::known_up_to(&NEWEST);
    let first = known
        .iter()
        .position(|version| version.as_str() >= OLDEST.as_str())
        .unwrap_or(known.len());
    &known[first..]
}

/// What the server tells a client as it initializes.
const INSTRUCTIONS: &str = "Neon Teamspace: the pages and databases of one workspace, as the \
token's identity sees them. Start with whoami and list_teamspaces, then get_page_tree. \
read_page gives a page's blocks as markdown, each block's id on a line `<!-- block <id> -->` \
before it; edit_page names those ids. A task is a row of a database: claim_task takes one \
nobody holds, and neon://me/tasks lists those assigned to you. list_comments and add_comment \
read and write a page's comments; get_inbox and neon://me/inbox tell what mentions you, replies \
to you and is assigned to you, and mark_read clears it.";

/// Serves every session's requests. Cheap to clone; clones share everything.
#[derive(Clone)]
pub(crate) struct Server {
    services: Services,
    sessions: Arc<Sessions>,
    red: Red,
}

impl Server {
    pub(super) fn new(services: Services, sessions: Arc<Sessions>, red: Red) -> Self {
        Self {
            services,
            sessions,
            red,
        }
    }

    /// Runs one request in its span, under the HTTP request's, measured as `operation`.
    async fn observe<T>(
        &self,
        operation: &'static str,
        context: &RequestContext<RoleServer>,
        principal: &Principal,
        work: impl Future<Output = Result<T, ToolError>>,
    ) -> Result<T, ToolError> {
        let parent = parts(context)
            .and_then(|parts| parts.extensions.get::<RequestSpan>())
            .map(|RequestSpan(span)| span.clone())
            .unwrap_or_else(Span::current);
        let span = info_span!(
            parent: &parent,
            "mcp",
            otel.name = %format!("mcp {operation}"),
            mcp.operation = operation,
            user_id = %principal.user.id,
        );
        let guard = self.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.expected => {
                mark_expected(&span, error.code);
                guard.ok();
            }
            Err(error) => {
                mark_unexpected(&span, error.code);
                guard.err();
            }
        }
        result
    }
}

/// The principal the gate authenticated the request as.
fn principal(context: &RequestContext<RoleServer>) -> Result<Principal, ErrorData> {
    parts(context)
        .and_then(|parts| parts.extensions.get::<Principal>())
        .cloned()
        .ok_or_else(|| ErrorData::internal_error("the request carries no principal", None))
}

fn parts(context: &RequestContext<RoleServer>) -> Option<&Parts> {
    context.extensions.get::<Parts>()
}

/// The session the request belongs to.
fn session(context: &RequestContext<RoleServer>) -> Result<String, ErrorData> {
    parts(context)
        .and_then(|parts| parts.headers.get(HEADER_SESSION_ID))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| ErrorData::invalid_request("subscriptions belong to a session", None))
}

/// A resource request's refusal as a protocol error: what is not there, or not the
/// caller's to see, is not found. Its data holds the API's code, as `{"code"}`.
fn resource_error(error: ToolError) -> ErrorData {
    let data = Some(serde_json::json!({"code": error.code}));
    if !error.expected {
        return ErrorData::internal_error(error.message, data);
    }
    match error.code {
        "not_found" => ErrorData::resource_not_found(error.message, data),
        _ => ErrorData::invalid_params(error.message, data),
    }
}

/// What a client asking for something the server does not offer gets: method not found,
/// saying so in a sentence.
fn not_offered(what: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::METHOD_NOT_FOUND,
        format!("this server offers no {what}: it has tools and resources"),
        None,
    )
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_resources_subscribe()
            .build();
        ServerConfig::new(capabilities)
            .with_protocol_version(NEWEST)
            .with_server_info(
                Implementation::new("neon", env!("CARGO_PKG_VERSION")).with_title("Neon Teamspace"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(protocol_versions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::list()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools::find(name).map(tools::Spec::tool)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let principal = principal(&context)?;
        let Some(spec) = tools::find(&request.name) else {
            return Err(ErrorData::invalid_params(
                format!("there is no tool {:?}: tools/list names them", request.name),
                None,
            ));
        };
        let arguments = request.arguments.unwrap_or_default();
        let called = self
            .observe(
                spec.name,
                &context,
                &principal,
                tools::call(&self.services, &principal, spec, arguments),
            )
            .await;
        let result = match called {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => error.into_result(),
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let principal = principal(&context)?;
        let listed = self
            .observe(
                "list_resources",
                &context,
                &principal,
                resources::list(&self.services, &principal),
            )
            .await
            .map_err(resource_error)?;
        Ok(ListResourcesResult::with_all_items(listed))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(
            resources::templates(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let principal = principal(&context)?;
        let resource = Resource::parse(&request.uri).map_err(resource_error)?;
        let contents = self
            .observe(
                "read_resource",
                &context,
                &principal,
                resources::read(&self.services, &principal, &resource),
            )
            .await
            .map_err(resource_error)?;
        Ok(ReadResourceResult::new(contents).into())
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let principal = principal(&context)?;
        let session = session(&context)?;
        let resource = Resource::parse(&request.uri).map_err(resource_error)?;
        self.observe("subscribe", &context, &principal, async {
            resources::check(&self.services, &principal, &resource).await?;
            let watch = watch::spawn(
                self.services.clone(),
                principal.clone(),
                resource,
                context.peer.clone(),
            )
            .await;
            self.sessions.subscribe(&session, watch)
        })
        .await
        .map_err(resource_error)
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Err(not_offered("prompts"))
    }

    async fn get_prompt(
        &self,
        _request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        Err(not_offered("prompts"))
    }

    async fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, ErrorData> {
        Err(not_offered("completions"))
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let session = session(&context)?;
        let resource = Resource::parse(&request.uri).map_err(resource_error)?;
        self.sessions.unsubscribe(&session, &resource);
        Ok(())
    }
}
