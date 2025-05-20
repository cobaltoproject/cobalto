use crate::settings::Settings;
use actix_web::{HttpRequest, HttpResponse, Responder, body::BoxBody};
use serde::Serialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

pub struct Request {
    pub params: HashMap<String, String>,
    pub body: String,
}

impl Request {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.body)
    }
}

pub struct Response {
    pub status: u16,
    pub body: String,
    pub headers: HashMap<String, String>,
}

impl Responder for Response {
    type Body = BoxBody;
    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut res =
            HttpResponse::build(actix_web::http::StatusCode::from_u16(self.status).unwrap());
        for (k, v) in self.headers {
            res.append_header((k, v));
        }
        res.body(self.body)
    }
}

impl Response {
    /// HTML response with status 200 and HTML content type
    pub fn html<B: Into<String>>(body: B) -> Self {
        let mut headers = HashMap::new();
        headers.insert(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        );
        Self {
            status: 200,
            body: body.into(),
            headers,
        }
    }

    /// JSON response with status 200 and JSON content type
    pub fn json<T: Serialize>(body: T) -> Self {
        let body = serde_json::to_string(&body)
            .unwrap_or_else(|_| "{\"error\": \"Failed to serialize body\"}".to_string());
        let mut headers = HashMap::new();
        headers.insert(
            "Content-Type".to_string(),
            "application/json; charset=utf-8".to_string(),
        );
        Self {
            status: 200,
            body,
            headers,
        }
    }

    /// Builder for setting a different status code
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Builder for adding or overwriting a header
    pub fn add_header<S: Into<String>>(mut self, key: S, val: S) -> Self {
        self.headers.insert(key.into(), val.into());
        self
    }
}

/// Handler type—expand as needed for params/state later!
pub type Handler =
    Arc<dyn Fn(Request) -> Pin<Box<dyn Future<Output = Response> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct Route {
    pub method: String,
    pub path: String,
    pub handler: Handler,
    pub handler_name: String,
}

/// The Cobalto router is just a list of registered routes for now.
pub struct Router {
    pub routes: Vec<Route>,
    pub settings: Settings,
}

impl Router {
    pub fn new(settings: Settings) -> Self {
        Router {
            routes: Vec::new(),
            settings,
        }
    }

    /// Register a route.
    pub fn add_route(&mut self, method: &str, path: &str, handler: Handler, handler_name: &str) {
        self.routes.push(Route {
            method: method.to_string(),
            path: path.to_string(),
            handler,
            handler_name: handler_name.to_string(),
        });
    }

    /// List all registered routes as (method, path) strings.
    pub fn list_routes(&self) -> Vec<(String, String)> {
        self.routes
            .iter()
            .map(|r| (r.method.clone(), r.path.clone()))
            .collect()
    }

    pub async fn run(&self) -> std::io::Result<()> {
        let bind_addr = format!("{}:{}", self.settings.host, self.settings.port);
        let app_state = self.settings.clone();
        let routes = self.routes.clone();

        // Log all registered routes at startup
        println!("╭──────────────────── Registered Routes ────────────────────╮");
        for route in &self.routes {
            println!(
                "│   {:<6}  {}  (fn: {})",
                route.method, route.path, route.handler_name
            );
        }
        println!("╰───────────────────────────────────────────────────────────╯");
        println!("Cobalto router serving on http://{}", bind_addr);

        actix_web::HttpServer::new(move || {
            // Create App with app_data up front
            let app = actix_web::App::new().app_data(actix_web::web::Data::new(app_state.clone()));

            // Fold over all routes, chaining .route calls
            let route_paths: Vec<(String, Vec<String>)> = routes
                .iter()
                .fold(HashMap::new(), |mut map, route| {
                    map.entry(route.path.clone())
                        .or_insert(vec![])
                        .push(route.method.clone());
                    map
                })
                .into_iter()
                .collect();

            let app = routes
                .iter()
                .fold(app, |app, route| {
                    let path_pattern = route.path.clone();
                    let handler = route.handler.clone();
                    let method = route.method.clone();

                    app.route(
                        "/{tail:.*}",
                        actix_web::web::route()
                            .guard(actix_web::guard::fn_guard({
                                let pattern = path_pattern.clone();
                                let method = method.clone();
                                move |ctx| {
                                    // Check HTTP method and path pattern
                                    let req = ctx.head();
                                    let req_method = req.method.as_str();
                                    let req_path = req.uri.path();
                                    req_method == method
                                        && extract_path_params(&pattern, req_path).is_some()
                                }
                            }))
                            .to({
                                let path_pattern = path_pattern.clone();
                                let handler = handler.clone();
                                move |req: HttpRequest, body: actix_web::web::Bytes| {
                                    let path_pattern = path_pattern.clone();
                                    let handler = handler.clone();
                                    async move {
                                        let params = extract_path_params(&path_pattern, req.path())
                                            .unwrap_or_default();
                                        let body_str =
                                            String::from_utf8(body.to_vec()).unwrap_or_default();
                                        let request = Request {
                                            params: params.clone(),
                                            body: body_str,
                                        };

                                        let t0 = std::time::Instant::now();
                                        let response = (handler)(request).await;
                                        let elapsed = t0.elapsed().as_millis();

                                        let now = chrono::Local::now();
                                        let ip = req
                                            .headers()
                                            .get("x-forwarded-for")
                                            .and_then(|hv| hv.to_str().ok())
                                            .map(|s| {
                                                s.split(',').next().unwrap_or(s).trim().to_string()
                                            })
                                            .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
                                            .unwrap_or_else(|| "<unknown>".to_string());
                                        println!(
                                            "[{}] {} {} {} [{}ms, {}]",
                                            now.format("%Y-%m-%d %H:%M:%S"),
                                            req.method(),
                                            req.path(),
                                            response.status,
                                            elapsed,
                                            ip,
                                        );
                                        response
                                    }
                                }
                            }),
                    )
                })
                .default_service(actix_web::web::to({
                    let route_paths = route_paths.clone();
                    move |req: HttpRequest| {
                        let route_paths = route_paths.clone();
                        async move {
                            let req_path = req.path();
                            let req_method = req.method().as_str().to_string();
                            // Find if path matches any known route (regardless of method)
                            let matched = route_paths
                                .iter()
                                .find(|(path, _)| {
                                    // Use extract_path_params logic for pattern match
                                    extract_path_params(path, req_path).is_some()
                                });

                            let ip = req
                                .headers()
                                .get("x-forwarded-for")
                                .and_then(|hv| hv.to_str().ok())
                                .map(|s| {
                                    s.split(',').next().unwrap_or(s).trim().to_string()
                                })
                                .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
                                .unwrap_or_else(|| "<unknown>".to_string());
                            let now = chrono::Local::now();

                            if let Some((_, allowed_methods)) = matched {
                                // Path matches but method does not
                                println!(
                                    "[{}] {} {} 404 [{}]",
                                    now.format("%Y-%m-%d %H:%M:%S"),
                                    req.method(),
                                    req.path(),
                                    ip,
                                );
                                let accept = req
                                    .headers()
                                    .get("accept")
                                    .and_then(|h| h.to_str().ok())
                                    .unwrap_or("");
                                let allow_methods = allowed_methods.join("\", \"");

                                if accept.contains("application/json") {
                                    HttpResponse::NotFound()
                                        .content_type("application/json; charset=utf-8")
                                        .body(format!(r#"{{"error":"Method '{}' not allowed.", "Allowed": ["{}"],"status":404}}"#, req_method, allow_methods))
                                } else {
                                    HttpResponse::NotFound()
                                        .content_type("text/html; charset=utf-8")
                                        .body(format!(r#"<!DOCTYPE html>
                                            <html lang="en">
                                            <head><meta charset="utf-8"><title>404 Not Found</title></head>
                                            <body style="font-family:sans-serif;text-align:center;margin-top:10vh">
                                            <h1 style="font-size:4rem;margin-bottom:0.5em">404</h1>
                                            <p style="font-size:1.5rem;margin-bottom:2em">Method <b>{}</b> not allowed.<br>Allowed methods: [{}]</p>
                                            </body>
                                            </html>
                                            "#, req_method, allow_methods))
                                }
                            } else {
                                // True 404, fallthrough to next (the actual 404 handler)
                                let accept = req
                                    .headers()
                                    .get("accept")
                                    .and_then(|h| h.to_str().ok())
                                    .unwrap_or("");
                                println!(
                                    "[{}] {} {} 404 [{}]",
                                    now.format("%Y-%m-%d %H:%M:%S"),
                                    req.method(),
                                    req.path(),
                                    ip,
                                );
                                if accept.contains("application/json") {
                                    HttpResponse::NotFound()
                                        .content_type("application/json; charset=utf-8")
                                        .body(r#"{"error":"Not found","status":404}"#)
                                } else {
                                    HttpResponse::NotFound()
                                        .content_type("text/html; charset=utf-8")
                                        .body(
                                            r#"<!DOCTYPE html>
                                            <html lang="en">
                                            <head><meta charset="utf-8"><title>404 Not Found</title></head>
                                            <body style="font-family:sans-serif;text-align:center;margin-top:10vh">
                                            <h1 style="font-size:4rem;margin-bottom:0.5em">404</h1>
                                            <p style="font-size:1.5rem;margin-bottom:2em">Page not found</p>
                                            </body>
                                            </html>
                                            "#
                                        )
                                }
                            }
                        }
                    }
                }));
            app
        })
        .bind(bind_addr)?
        .run()
        .await
    }
}

fn extract_path_params(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_parts: Vec<_> = pattern.trim_matches('/').split('/').collect();
    let path_parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if pattern_parts.len() != path_parts.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (p, actual) in pattern_parts.iter().zip(path_parts.iter()) {
        if p.starts_with(':') {
            params.insert(p[1..].to_string(), actual.to_string());
        } else if *p != *actual {
            return None;
        }
    }
    Some(params)
}

#[macro_export]
macro_rules! route {
    ($router:expr, $( $method:ident $path:expr => $handler:expr ),* $(,)?) => {
        $(
            $router.add_route(
                stringify!($method),
                $path,
                Arc::new(|req| Box::pin($handler(req))),
                stringify!($handler)
            );
        )*
    };
}
