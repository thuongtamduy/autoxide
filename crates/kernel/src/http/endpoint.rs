//! SPIKE F9 (draft branch, never merged): `#[endpoint]`, `routes!`, `Routes` — a module declares its routes and their
//! OpenAPI description without depending on `utoipa` / `utoipa-axum`.

// Bên trong vẫn là `utoipa_axum::router::OpenApiRouter`: đổi bộ sinh OpenAPI chỉ đổi file này và `autoxide-macros`.

use axum::routing::{MethodFilter, MethodRouter};
use utoipa::openapi::RefOr;
use utoipa::openapi::path::{HttpMethod, Operation, Paths};
use utoipa::openapi::schema::Schema as OpenApiSchema;
use utoipa_axum::router::OpenApiRouter;

pub use autoxide_macros::{Params, Schema, endpoint};

/// The router a module returns from `Module::routes` (transitional: the module manifest replaces it).
pub type ModuleRouter = OpenApiRouter;

/// The OpenAPI description of one endpoint, built by `#[endpoint]`. Not written by hand.
#[doc(hidden)]
pub struct EndpointDoc {
    path: String,
    methods: Vec<HttpMethod>,
    operation: Operation,
    schemas: Vec<(String, RefOr<OpenApiSchema>)>,
}

impl EndpointDoc {
    #[doc(hidden)]
    pub fn __new(
        path: String,
        methods: Vec<HttpMethod>,
        operation: Operation,
        schemas: Vec<(String, RefOr<OpenApiSchema>)>,
    ) -> Self {
        EndpointDoc { path, methods, operation, schemas }
    }

    /// The HTTP methods of the endpoint, as axum method filters.
    #[doc(hidden)]
    pub fn __filters(&self) -> Vec<MethodFilter> {
        self.methods.iter().map(filter).collect()
    }
}

fn filter(method: &HttpMethod) -> MethodFilter {
    match method {
        HttpMethod::Get => MethodFilter::GET,
        HttpMethod::Put => MethodFilter::PUT,
        HttpMethod::Post => MethodFilter::POST,
        HttpMethod::Head => MethodFilter::HEAD,
        HttpMethod::Patch => MethodFilter::PATCH,
        HttpMethod::Trace => MethodFilter::TRACE,
        HttpMethod::Delete => MethodFilter::DELETE,
        HttpMethod::Options => MethodFilter::OPTIONS,
    }
}

/// An endpoint declared with `#[endpoint]` (implemented by the marker the attribute generates).
pub trait Endpoint {
    #[doc(hidden)]
    fn __describe() -> EndpointDoc;
}

/// The routes of a module, with their OpenAPI description; built with [`routes!`](crate::routes).
pub struct Routes<S = ()>(OpenApiRouter<S>);

impl<S: Clone + Send + Sync + 'static> Default for Routes<S> {
    fn default() -> Self {
        Routes(OpenApiRouter::new())
    }
}

impl<S: Clone + Send + Sync + 'static> Routes<S> {
    /// No routes.
    pub fn new() -> Self {
        Self::default()
    }

    #[doc(hidden)]
    pub fn __add(self, doc: EndpointDoc, router: MethodRouter<S>) -> Self {
        let mut paths = Paths::new();
        paths.add_path_operation(&doc.path, doc.methods, doc.operation);
        Routes(self.0.routes((doc.schemas, paths, router)))
    }

    /// Gives the handlers their state (`State<T>`).
    pub fn with_state<S2: Clone + Send + Sync + 'static>(self, state: S) -> Routes<S2> {
        Routes(self.0.with_state(state))
    }
}

impl From<Routes> for OpenApiRouter {
    fn from(routes: Routes) -> Self {
        routes.0
    }
}

/// The routes of the endpoints named (functions with `#[endpoint]`), in order.
#[macro_export]
macro_rules! routes {
    ($($handler:path),+ $(,)?) => {{
        let routes = $crate::http::Routes::new();
        $(
            let routes = {
                let doc = <$handler as $crate::http::Endpoint>::__describe();
                let router = doc.__filters().into_iter().fold(
                    $crate::__private::axum::routing::MethodRouter::new(),
                    |router, filter| router.on(filter, $handler),
                );
                routes.__add(doc, router)
            };
        )+
        routes
    }};
}
