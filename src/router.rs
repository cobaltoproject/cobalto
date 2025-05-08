use crate::orm::Db;
/// Cobalto Web Framework Router module
///
/// This module provides the core routing, HTTP, and WebSocket infrastructure
/// for Cobalto applications. It allows for:
///
/// - Path and parameter-based routing of traditional HTTP endpoints
/// - Global and route-specific middleware (pre and post)
/// - Unified, ergonomic registration of user-facing WebSocket endpoints on dedicated ports
/// - Built-in support for hot-reload via special websocket route, with file watcher trigger
///
/// All API surfaces are designed to be "batteries-included" and make it fast and safe
/// to build modern backends and real-time features alike.
///
use crate::settings::Settings;
use axum::Router as AxumRouter;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use notify::event::DataChange;
use notify::event::ModifyKind::Data;
use notify::{EventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub settings: Settings,
}

/// Represents the outcome of an HTTP handler in Cobalto.
/// Supports HTML, JSON, and custom status/headers.
use serde::Serialize;
pub struct Response {
    pub status_code: u16,
    pub body: String,
    pub headers: HashMap<String, String>,
}

impl Response {
    /// Construct a new HTTP 200 response with HTML/text body.
    pub fn ok(body: impl Into<String>) -> Self {
        Response {
            status_code: 200,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    /// Construct a new HTTP 403 response with text body.
    pub fn forbidden(body: impl Into<String>) -> Self {
        Response {
            status_code: 403,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    /// Construct a new HTTP 404 "not found" response.
    pub fn not_found() -> Self {
        Response {
            status_code: 404,
            body: "404 Not Found".to_string(),
            headers: HashMap::new(),
        }
    }

    /// Construct a new HTTP JSON response.
    /// Accepts any serde-serializable payload, status, and custom headers.
    pub fn json<T: Serialize>(
        data: T,
        status_code: u16,
        mut headers: HashMap<String, String>,
    ) -> Self {
        match serde_json::to_string(&data) {
            Ok(body) => {
                headers.insert(
                    "Content-Type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                );
                Response {
                    status_code,
                    body,
                    headers,
                }
            }
            Err(_) => {
                headers.insert(
                    "Content-Type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                );
                Response {
                    status_code: 500,
                    body: "{\"error\": \"Serialization failed\"}".to_string(),
                    headers,
                }
            }
        }
    }
}

/// Holds metadata about the current HTTP request and its extracted path parameters.
/// Middleware and handlers can modify/read this context.
pub struct RequestContext {
    pub path: String,
    pub params: HashMap<String, String>,
    pub is_authenticated: bool,
    pub start_time: Option<Instant>,
}

/// Type alias for async handler functions for HTTP routes.
/// Accepts a map of extracted parameters and returns a Response.
pub type Handler = Arc<
    dyn Fn(HashMap<String, String>, AppState) -> Pin<Box<dyn Future<Output = Response> + Send>>
        + Send
        + Sync,
>;

/// Type alias for synchronous, pre-processing middleware executed before the handler.
/// If a middleware returns Some(Response), request handling stops and this response is sent.
pub type Middleware = Arc<dyn Fn(&mut RequestContext) -> Option<Response> + Send + Sync>;

/// Type alias for post-processing middleware executed after the handler.
/// Post-middleware can inspect/modify the response before it is sent.
pub type PostMiddleware = Arc<dyn Fn(&RequestContext, Response) -> Response + Send + Sync>;

/// Represents a registered HTTP route and its associated handler + middleware.
#[derive(Clone)]
pub struct Route {
    pub path_pattern: String,
    pub handler: Handler,
    pub middlewares: Vec<Middleware>,
}

/// Context for user WebSocket handlers, providing matched path and params.
#[derive(Clone)]
pub struct WsContext {
    pub path: String,
    pub params: HashMap<String, String>,
}

/// Async handler type for WebSocket routes.
/// Accepts WsContext and an Axum WebSocket stream for two-way comms.
pub type WsHandler =
    Arc<dyn Fn(WsContext, WebSocket) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Represents a registered user WebSocket route and its associated handler.
#[derive(Clone)]
pub struct WsRoute {
    pub path_pattern: String,
    pub handler: WsHandler,
}

// Settings struct changes: expects both http_port and ws_port in Settings

/// The main application router for Cobalto.
/// Manages all HTTP routes, WebSocket routes, and global middleware.
#[derive(Clone)]
pub struct Router {
    pub routes: Vec<Route>,
    pub ws_routes: Vec<WsRoute>,
    pub middlewares: Vec<Middleware>,
    pub post_middlewares: Vec<PostMiddleware>,
    pub app_state: Option<AppState>,
}

/// Serializes and sends an HTTP Response over a raw TCP socket connection.
async fn send_response(mut socket: tokio::net::TcpStream, response: Response) {
    let mut headers = String::new();
    for (key, value) in response.headers {
        headers.push_str(&format!("{}: {}\r\n", key, value));
    }

    let response_text = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}\
\r\n{}",
        response.status_code,
        status_text(response.status_code),
        response.body.len(),
        headers,
        response.body
    );

    let _ = socket.write_all(response_text.as_bytes()).await;
}

/// Maps status codes to HTTP status text for responses.
pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Unknown",
    }
}

impl Router {
    /// Create a new, empty application router.
    pub fn new() -> Self {
        Router {
            routes: Vec::new(),
            ws_routes: Vec::new(),
            middlewares: Vec::new(),
            post_middlewares: Vec::new(),
            app_state: None,
        }
    }

    /// Register an HTTP route with path pattern, handler, and any route-specific middleware.
    pub fn add_route(
        &mut self,
        path_pattern: &str,
        handler: Handler,
        middlewares: Vec<Middleware>,
    ) {
        self.routes.push(Route {
            path_pattern: path_pattern.to_string(),
            handler,
            middlewares,
        });
    }

    /// Register a WebSocket route and handler.
    pub fn add_websocket(&mut self, path_pattern: &str, handler: WsHandler) {
        self.ws_routes.push(WsRoute {
            path_pattern: path_pattern.to_string(),
            handler,
        });
    }

    /// Add a global pre-middleware to be run before all HTTP handlers.
    pub fn add_middleware(&mut self, middleware: Middleware) {
        self.middlewares.push(middleware);
    }

    /// Add a post-middleware to be run after each HTTP handler.
    pub fn add_post_middleware(&mut self, middleware: PostMiddleware) {
        self.post_middlewares.push(middleware);
    }

    pub fn set_app_state(&mut self, state: AppState) {
        self.app_state = Some(state);
    }

    /// Watches the template directory for changes; notifies via WS broadcast for live-reload.
    fn setup_ws_reload_watcher(&self, template_path: PathBuf, sender: broadcast::Sender<String>) {
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            let mut watcher = notify::recommended_watcher(move |res| {
                let _ = tx.blocking_send(res);
            })
            .expect("Failed to create watcher");

            watcher
                .watch(&template_path, RecursiveMode::Recursive)
                .unwrap();

            while let Some(res) = rx.recv().await {
                match res {
                    Ok(event) => {
                        if let EventKind::Modify(Data(DataChange::Content)) = event.kind {
                            if let Some(path) = event.paths.first() {
                                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                                    log::info!("📄 Template changed: {}", file_name);
                                    let _ = sender.send("reload".to_string());
                                }
                            }
                        }
                    }
                    Err(e) => log::error!("Watch error: {:?}", e),
                }
            }
        });
    }

    /// Orchestrate the entire application by launching both HTTP and WebSocket servers
    /// on their respective ports, as provided in the Settings.
    ///
    /// This is the typical entry point for production use.
    pub async fn run(
        &mut self,
        settings: Settings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut ws_self = self.clone();
        let mut http_self = self.clone();

        let http_addr = format!("{}:{}", settings.host, settings.port); // or settings.http_port
        let ws_addr = format!("{}:{}", settings.host, settings.ws_port);

        let ws_settings = settings.clone();

        // Start WS on a separate task
        let ws_handle = tokio::spawn(async move { ws_self.run_ws(&ws_addr, ws_settings).await });

        http_self.run_http(&http_addr, settings).await?;

        ws_handle.await??;
        Ok(())
    }

    /// Start the HTTP server for standard GET/POST etc. endpoints.
    /// Uses a classic TcpListener and manual HTTP parsing for fine-grained control.
    pub async fn run_http(
        &mut self,
        addr: &str,
        _settings: Settings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(addr).await?;
        println!("HTTP Server running on http://{}", addr);

        loop {
            let (mut socket, _) = listener.accept().await?;
            let routes = self.routes.clone();
            let middlewares = self.middlewares.clone();
            let post_middlewares = self.post_middlewares.clone();
            let state = self.app_state.clone().expect("App state not set in Router");
            tokio::spawn(async move {
                let mut buffer = [0; 1024];
                if let Ok(_) = socket.read(&mut buffer).await {
                    let request = String::from_utf8_lossy(&buffer[..]);
                    let first_line = request.lines().next().unwrap_or_default();
                    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

                    let parts: Vec<&str> = path.split('?').collect();
                    let real_path = parts[0];
                    let mut ctx = RequestContext {
                        path: real_path.to_string(),
                        params: HashMap::new(),
                        is_authenticated: parts
                            .get(1)
                            .map(|q| q.contains("token=abc123"))
                            .unwrap_or(false),
                        start_time: None,
                    };

                    for middleware in &middlewares {
                        if let Some(response) = (middleware)(&mut ctx) {
                            send_response(socket, response).await;
                            return;
                        }
                    }

                    let mut response = Response::not_found();

                    for route in &routes {
                        if let Some(params) = match_path(&route.path_pattern, &ctx.path) {
                            ctx.params = params;

                            for middleware in &route.middlewares {
                                if let Some(response) = (middleware)(&mut ctx) {
                                    send_response(socket, response).await;
                                    return;
                                }
                            }
                            response = (route.handler)(ctx.params.clone(), state.clone()).await;
                            break;
                        }
                    }

                    for post_middleware in &post_middlewares {
                        response = (post_middleware)(&ctx, response);
                    }

                    send_response(socket, response).await;
                }
            });
        }
    }

    /// Build an Axum router for all registered WebSocket routes and the optional hot-reload route.
    /// Called only from run_ws().
    pub fn build_ws_axum_router(
        &mut self,
        settings: &Settings,
        reload_sender: Option<broadcast::Sender<String>>,
    ) -> AxumRouter {
        let mut app = AxumRouter::new();

        // Add all user-defined websocket routes
        for ws_route in &self.ws_routes {
            let ws_path = ws_route.path_pattern.clone();
            let ws_handler = ws_route.handler.clone();
            let ws_path_for_route = ws_path.clone();
            app = app.route(
                &ws_path,
                get(move |ws: WebSocketUpgrade| {
                    let ws_handler = ws_handler.clone();
                    let ws_path = ws_path_for_route.clone(); // move in for this closure
                    async move {
                        ws.on_upgrade(move |socket| {
                            (ws_handler)(
                                WsContext {
                                    path: ws_path.clone(),
                                    params: HashMap::new(),
                                },
                                socket,
                            )
                        })
                    }
                }),
            );
        }

        // Add hot-reload websocket if enabled
        if settings.debug {
            let tx = reload_sender;
            app = app.route(
                "/ws/reload",
                get(move |ws: WebSocketUpgrade| {
                    let tx = tx.clone();
                    async move {
                        ws.on_upgrade(move |mut socket| async move {
                            if let Some(tx) = tx {
                                let mut rx = tx.subscribe();
                                log::info!("🔌 Hot Reload WebSocket client connected!");
                                while let Ok(msg) = rx.recv().await {
                                    if socket.send(Message::Text(msg.into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        })
                    }
                }),
            );
        }

        app
    }

    /// Start the WebSocket server, registering both user endpoints and hot-reload if enabled.
    /// Uses Axum and Hyper for WebSocket protocol handling.
    pub async fn run_ws(
        &mut self,
        addr: &str,
        settings: Settings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use axum::serve;
        use tokio::net::TcpListener;

        let mut reload_sender = None;
        if settings.debug {
            let template_path = PathBuf::from(&settings.template.dir);
            let (sender, _) = broadcast::channel::<String>(10);
            reload_sender = Some(sender.clone());
            self.setup_ws_reload_watcher(template_path, sender);
        }

        let app = self.build_ws_axum_router(&settings, reload_sender);

        let addr: SocketAddr = addr.parse()?;
        let listener = TcpListener::bind(addr).await?;
        println!("WebSocket Server running at ws://{}", addr);

        serve(listener, app).await?;
        Ok(())
    }
}

#[macro_export]
macro_rules! route {
    ($router:expr, $( $method:ident $path:expr => { $handler:expr $(, $middleware:expr )* } ),* $(,)?) => {
        $(
            $router.add_route(
                $path,
                Arc::new(move |params, state| Box::pin($handler(params, state.clone()))),
                vec![$($middleware),*]
            );
        )*
    };
}

/// Matches a path pattern (e.g. `/foo/:id`) against a real path,
/// extracting parameters into a HashMap if matched, or None if not.
pub fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_parts: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    let path_parts: Vec<&str> = path.trim_matches('/').split('/').collect();

    if pattern_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (p, a) in pattern_parts.iter().zip(path_parts.iter()) {
        if p.starts_with(':') {
            params.insert(p[1..].to_string(), a.to_string());
        } else if p != a {
            return None;
        }
    }

    Some(params)
}
