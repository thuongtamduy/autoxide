//! `/llms.txt` - a page for AI agents to discover the server on their own (step 40 P2), in the format of
//! [llmstxt.org](https://llmstxt.org): an H1, a one-sentence summary in `>`, `##` sections with lists of links.
//!
//! The content is **generated from code when the router is built** (registered modules, `Module::description`,
//! MCP resources and prompts, OpenAPI / MCP / documentation page paths), so it cannot drift; the tests
//! `llms_txt_co_moi_module_va_link` and `moi_link_trong_llms_txt_khong_404` guarantee that. The same
//! content is the MCP resource `autoxide://kernel/llms.txt`.
//!
//! The page lists the modules of the INSTALLATION, not of a tenant's plan: it is a public page that does not
//! know the caller; endpoints and tools of a module the plan lacks still return `module_not_enabled` when
//! called.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::mcp::{McpResource, Prompt, Tool};

/// Path of the page.
pub const LLMS_PATH: &str = "/llms.txt";

/// What the page talks about.
pub(super) struct LlmsInputs<'a> {
    /// `AUTOXIDE_HTTP_PUBLIC_URL` (without a trailing `/`); when unset, links are relative.
    pub(super) base: &'a str,
    pub(super) api_docs: bool,
    /// There is an `/mcp` endpoint (an authentication module exists).
    pub(super) mcp: bool,
    /// There are health check routes (`health_routes`) at the root.
    pub(super) health: bool,
    /// There is a module catalog route (`GET /api/v1/kernel/modules/{name}`): each module links to its entry.
    pub(super) catalog: bool,
    /// Name and description of every module, in registration order.
    pub(super) modules: &'a [(&'static str, &'static str)],
    /// Modules that have REST paths, i.e. their own OpenAPI documentation.
    pub(super) documented: &'a [&'static str],
    /// MCP tools of every module, full name `<module>.<name>`.
    pub(super) tools: &'a [Tool],
    pub(super) resources: &'a [McpResource],
    pub(super) prompts: &'a [Prompt],
}

/// Builds the page content.
pub(super) fn render(inputs: &LlmsInputs<'_>) -> String {
    let base = inputs.base;
    let api = super::API_PREFIX;
    let mut text = String::new();
    // `write!` vào `String` không bao giờ lỗi; `let _ =` để khỏi `unwrap`.
    let _ = writeln!(text, "# Autoxide {}\n", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        text,
        "> Open-source backend core: a REST API described by OpenAPI, and an MCP endpoint where \
         AI agents call the same services as tools, with the same permissions.\n"
    );
    let has_auth = inputs.modules.iter().any(|(name, _)| *name == "auth");
    if has_auth {
        let _ = writeln!(
            text,
            "Authenticate machines with an API key: send `Authorization: Bearer <API key>`. A \
             signed-in user creates one, with only the permissions the agent needs, at \
             `POST {api}/auth/api-keys`. MCP clients can instead sign in with OAuth 2.1 when the server \
             enables it: start from `/.well-known/oauth-protected-resource/mcp`. Refresh OAuth tokens one \
             at a time: a refresh token sent twice (also by two parallel refreshes) counts as stolen and \
             revokes the whole approval.\n"
        );
    }
    let _ = writeln!(
        text,
        "Conventions, the same on every endpoint: a value comes as `{{\"data\": …}}`, a list as \
         `{{\"data\": […], \"pagination\": {{…}}}}` (page with `limit`, `cursor` and `order`, \
         follow `next_cursor`); errors are RFC 9457 Problem Details with a stable `code` and a \
         `message` that says what to fix; send an `Idempotency-Key` header with writes you may \
         retry. MCP tools return the same shapes.\n"
    );

    let _ = writeln!(text, "## API\n");
    let _ = writeln!(
        text,
        "- [OpenAPI document]({base}{}): the full REST contract, with the permission each \
         endpoint needs",
        super::OPENAPI_PATH
    );
    let _ = writeln!(
        text,
        "- OpenAPI document of one module: `{api}/openapi/<module>.json`, linked under Modules below"
    );
    if inputs.api_docs {
        let _ = writeln!(
            text,
            "- [API reference]({base}{}): the OpenAPI document as a web page",
            super::DOCS_PATH
        );
    }
    if inputs.mcp {
        let _ = writeln!(
            text,
            "- [MCP endpoint]({base}{}): Streamable HTTP, authenticated like REST; lists the \
             tools, resources and prompts the caller may use",
            super::MCP_PATH
        );
    }
    if inputs.health {
        let _ = writeln!(text, "- [Health]({base}/health/live): answers while the process runs");
    }
    let _ = writeln!(text);

    let _ = writeln!(text, "## Modules\n");
    let _ = writeln!(
        text,
        "Load only what the task needs: each module has its own OpenAPI document (its endpoints and \
         the schemas they use){}.\n",
        if inputs.mcp {
            " and its own MCP endpoint (only its tools, resources and prompts; same authentication \
             as `/mcp`)"
        } else {
            ""
        }
    );
    for (name, description) in inputs.modules {
        let description = if description.is_empty() { "No description." } else { description };
        if inputs.catalog {
            let _ = writeln!(text, "- [{name}]({base}{api}/kernel/modules/{name}): {description}");
        } else {
            let _ = writeln!(text, "- {name}: {description}");
        }
        let _ = module_links(&mut text, inputs, name);
    }

    if !inputs.prompts.is_empty() {
        let _ = writeln!(text, "\n## MCP prompts\n");
        for prompt in inputs.prompts {
            let _ = write!(text, "- `{}`: {}", prompt.name(), prompt.description());
            let tools = prompt.tools();
            if !tools.is_empty() {
                let names: Vec<String> = tools.iter().map(|tool| format!("`{tool}`")).collect();
                let _ = write!(text, " Leads to {}.", names.join(", "));
            }
            let _ = writeln!(text);
        }
    }
    if !inputs.resources.is_empty() {
        let _ = writeln!(text, "\n## MCP resources\n");
        for resource in inputs.resources {
            let _ = write!(text, "- `{}`: {}", resource.uri(), resource.description());
            if let Some(permission) = resource.permission() {
                let _ = write!(text, " Requires `{}`.", permission.name());
            }
            let _ = writeln!(text);
        }
    }
    text
}

/// Child lines under a module: links to its own OpenAPI documentation and MCP endpoint, and the names of
/// its tools.
fn module_links(text: &mut String, inputs: &LlmsInputs<'_>, name: &str) -> std::fmt::Result {
    let base = inputs.base;
    let prefix = format!("{name}.");
    let tools: Vec<String> = inputs
        .tools
        .iter()
        .filter(|tool| tool.name().starts_with(&prefix))
        .map(|tool| format!("`{}`", tool.name()))
        .collect();
    let has_mcp = inputs.mcp
        && (!tools.is_empty()
            || inputs.resources.iter().any(|resource| resource.module() == name)
            || inputs.prompts.iter().any(|prompt| prompt.name().starts_with(&prefix)));
    let mut links = Vec::new();
    if inputs.documented.contains(&name) {
        links.push(format!(
            "[OpenAPI]({base}{})",
            super::openapi_modules::module_openapi_path(name)
        ));
    }
    if has_mcp {
        links.push(format!("[MCP]({base}{})", super::mcp::module_path(name)));
    }
    if links.is_empty() && tools.is_empty() {
        return Ok(());
    }
    write!(text, "  - {}", links.join(" · "))?;
    if !tools.is_empty() {
        let separator = if links.is_empty() { "" } else { " · " };
        write!(text, "{separator}tools: {}", tools.join(", "))?;
    }
    writeln!(text)
}

/// The built content; cloning `Bytes` only bumps the reference count.
#[derive(Clone)]
struct LlmsText(axum::body::Bytes);

/// The `/llms.txt` route with the built content.
pub(super) fn routes(text: String) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(llms_txt)).with_state(LlmsText(text.into()))
}

/// Overview of this server for AI agents.
///
/// Plain text in the llmstxt.org format: where the OpenAPI document and the MCP endpoint are,
/// how to authenticate, the conventions every endpoint follows, the installed modules, and the
/// MCP prompts and resources. Generated from the code at start-up.
#[utoipa::path(
    get,
    path = "/llms.txt",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "llms_txt",
    responses((status = 200, description = "The overview, as Markdown text.", content_type = "text/plain", body = String))
)]
async fn llms_txt(State(LlmsText(text)): State<LlmsText>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        text,
    )
}
