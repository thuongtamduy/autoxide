//! Endpoints and their routes: a function with `#[endpoint(…)]` is listed in [`routes!`](crate::routes), which
//! builds the module's [`Routes`] (the HTTP routes and their OpenAPI description) for
//! [`Module::endpoints`](crate::module::Module::endpoints).
//!
//! ```ignore
//! use autoxide_kernel::http::{Path, Routes, State, endpoint};
//! use autoxide_kernel::routes;
//!
//! /// Get a note.
//! #[endpoint(
//!     get,
//!     path = "/{id}",
//!     permission = "notes.read",
//!     params(("id" = Uuid, Path, description = "Id of the note.")),
//!     responses(
//!         (status = 200, description = "The note.", body = Data<NoteResponse>, etag),
//!         (status = 404, description = "No note with this id; code `notes.note_not_found`.", body = Problem),
//!     )
//! )]
//! async fn get_note(State(service): State<Arc<NoteService>>, Path(id): Path<Uuid>) -> Result<…, Error> { … }
//!
//! fn routes(service: Arc<NoteService>) -> Routes {
//!     routes![get_note].with_state(service)
//! }
//! ```

// Bên trong vẫn là `utoipa_axum::router::OpenApiRouter`; đổi bộ sinh OpenAPI chỉ đổi file này và `autoxide-macros`
// (D-20261126-kernel-web-layer). Kiểu của utoipa / axum chỉ xuất hiện trong mục ẩn (`__…`), ngoài hợp đồng.

use axum::routing::{MethodFilter, MethodRouter};
use utoipa::openapi::RefOr;
use utoipa::openapi::path::{HttpMethod, Operation, Paths};
use utoipa::openapi::schema::Schema as OpenApiSchema;
use utoipa_axum::router::OpenApiRouter;

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

/// An endpoint declared with `#[endpoint]`. Implemented only by the marker the attribute generates (same name as
/// the function); list the function in [`routes!`](crate::routes).
///
/// Every endpoint says who may call it — `permission = "…"`, `signed_in` or `public`; there is no default:
///
/// ```compile_fail
/// use autoxide_kernel::http::endpoint;
///
/// /// Ping.
/// #[endpoint(get, path = "/ping", responses((status = 200, description = "Alive.")))]
/// async fn ping() {}
/// ```
///
/// ```compile_fail
/// use autoxide_kernel::http::endpoint;
///
/// /// Ping; `x-permissions` is generated, never written.
/// #[endpoint(get, path = "/ping", public, extensions(("x-permissions" = json!([]))),
///     responses((status = 200, description = "Alive.")))]
/// async fn ping() {}
/// ```
///
/// ```
/// use autoxide_kernel::http::endpoint;
///
/// /// Ping.
/// #[endpoint(get, path = "/ping", public, responses((status = 200, description = "Alive.")))]
/// async fn ping() {}
///
/// /// Who am I; the standard 401 is added.
/// #[endpoint(get, path = "/me", signed_in, responses((status = 200, description = "You.")))]
/// async fn me() {}
///
/// /// Read; the standard 401 and 403 are added.
/// #[endpoint(get, path = "/read", permission = ["notes.read", "notes.audit"],
///     responses((status = 200, description = "Read.")))]
/// async fn read() {}
///
/// let routes: autoxide_kernel::http::Routes = autoxide_kernel::routes![ping, me, read];
/// # let _ = routes;
/// ```
pub trait Endpoint {
    #[doc(hidden)]
    fn __describe() -> EndpointDoc;
}

/// The routes of a module with their OpenAPI description, built by [`routes!`](crate::routes). `S` is the state the
/// handlers take with [`State`](super::State); [`with_state`](Self::with_state) gives it.
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
        // axum gộp các method của cùng một path; utoipa gộp operation của cùng một path: thêm từng endpoint một là đủ.
        let mut paths = Paths::new();
        paths.add_path_operation(&doc.path, doc.methods, doc.operation);
        Routes(self.0.routes((doc.schemas, paths, router)))
    }

    /// Gives the handlers their state (`State<T>`, where `T` is `state`'s type).
    pub fn with_state<S2: Clone + Send + Sync + 'static>(self, state: S) -> Routes<S2> {
        Routes(self.0.with_state(state))
    }
}

impl Routes {
    // Cho kernel ghép vào router của module (chỗ duy nhất kiểu của utoipa-axum lộ ra, trong crate).
    pub(crate) fn into_router(self) -> OpenApiRouter {
        self.0
    }
}

impl<S> std::fmt::Debug for Routes<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Routes").finish_non_exhaustive()
    }
}

/// The [`Routes`] of the endpoints named (functions with `#[endpoint]`), in order:
/// `routes![list_notes, create_note].with_state(service)`.
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
