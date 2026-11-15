//! Cấu hình cho giao diện (đề bài giao diện C, D-271): `GET /api/v1/kernel/client-config` gom
//! phần mỗi module khai qua [`Module::client_config`](crate::module::Module::client_config) — chỉ
//! giá trị không phải secret mà giao diện cần để khỏi đoán mặc định của core (ngôn ngữ, giới hạn
//! tải lên, địa chỉ công khai…).
//!
//! Giá trị đọc một lần lúc khởi động (cấu hình không đổi khi chương trình đang chạy). Lược đồ
//! OpenAPI của câu trả lời ghép từ lược đồ của từng phần (`T: ToSchema` của module), nên tài liệu
//! mô tả đủ từng trường mà kernel không cần biết module nào.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::Error;
use crate::module::Module;
use crate::response::{Data, Problem};

/// Phần cấu hình cho giao diện của một module: giá trị và lược đồ OpenAPI của nó.
#[derive(Clone)]
pub struct ClientSection {
    value: Value,
    schema: RefOr<Schema>,
}

// `RefOr<Schema>` của utoipa không cài `Debug`: chỉ in giá trị.
impl std::fmt::Debug for ClientSection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ClientSection").field("value", &self.value).finish_non_exhaustive()
    }
}

impl ClientSection {
    /// Phần từ `value`. `T` chỉ nên có trường đơn giản (chuỗi, số, cờ, danh sách của chúng):
    /// lược đồ của nó được chép nguyên vào tài liệu OpenAPI. Mỗi trường mang `///` mô tả (tiếng
    /// Anh) và chỉ giữ giá trị ai xem cũng được — endpoint không cần đăng nhập.
    pub fn new<T: Serialize + ToSchema>(value: &T) -> Result<Self, Error> {
        let value = serde_json::to_value(value).map_err(|error| {
            Error::internal(format!("client config does not serialize: {error}"))
        })?;
        Ok(ClientSection { value, schema: T::schema() })
    }
}

/// Phần cấu hình của mọi module, điền một lần sau khi mọi module khởi động.
#[derive(Clone, Default)]
pub(crate) struct ClientConfigs(Arc<OnceLock<Vec<(&'static str, ClientSection)>>>);

impl ClientConfigs {
    /// Điền từ các module đã khởi động. Module trả lỗi thì phần của nó vắng (lỗi được ghi log).
    pub(crate) fn fill(&self, modules: &[Box<dyn Module>]) {
        let sections = modules
            .iter()
            .filter_map(|module| match module.client_config() {
                Ok(section) => section.map(|section| (module.name(), section)),
                Err(error) => {
                    tracing::error!(module = module.name(), %error, "client config left out");
                    None
                }
            })
            .collect();
        let _ = self.0.set(sections);
    }

    fn sections(&self) -> &[(&'static str, ClientSection)] {
        self.0.get().map_or(&[], Vec::as_slice)
    }
}

/// Settings of the server an interface needs, by module (`kernel`, `content`, `media`…): only
/// values safe to show anyone. A module without such settings is absent.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(transparent)]
pub(crate) struct ClientConfigResponse(BTreeMap<&'static str, Value>);

pub(crate) fn routes(configs: ClientConfigs) -> OpenApiRouter {
    let mut router = OpenApiRouter::new()
        .routes(routes!(get_client_config))
        .with_state(Arc::new(configs.clone()));
    // Thay lược đồ chung (object bất kỳ) bằng lược đồ ghép từ phần của từng module.
    let mut object = ObjectBuilder::new().description(Some(
        "Settings of the server an interface needs, by module: only values safe to show anyone. \
         A module without such settings is absent.",
    ));
    for (name, section) in configs.sections() {
        object = object.property(*name, section.schema.clone()).required(*name);
    }
    if let Some(components) = router.get_openapi_mut().components.as_mut() {
        components.schemas.insert("ClientConfigResponse".to_owned(), object.build().into());
    }
    router
}

/// Get the settings an interface needs.
///
/// No sign-in needed. Values the server's operator set (or the defaults), by module: languages,
/// path prefixes, upload limits, the public address… — so an interface does not guess the
/// server's defaults. They change only when the server restarts with new settings.
#[utoipa::path(
    get,
    path = "/client-config",
    extensions(("x-public" = json!(true))),
    operation_id = "get_client_config",
    responses(
        (status = 200, description = "The settings, by module.", body = Data<ClientConfigResponse>),
        (status = 500, description = "Unexpected server error.", body = Problem),
    )
)]
async fn get_client_config(
    axum::extract::State(configs): axum::extract::State<Arc<ClientConfigs>>,
) -> Data<ClientConfigResponse> {
    let sections =
        configs.sections().iter().map(|(name, section)| (*name, section.value.clone())).collect();
    Data { data: ClientConfigResponse(sections) }
}

/// Kernel's own section.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct KernelClientConfig {
    /// The public address of the API (`AUTOXIDE_HTTP_PUBLIC_URL`), used in links of mails and by
    /// MCP clients (`<public_url>/mcp`); `null` when the operator did not set it.
    #[schema(example = "https://api.example.com")]
    pub(crate) public_url: Option<String>,
    /// Version of the server.
    #[schema(example = "0.15.0")]
    pub(crate) version: String,
}
