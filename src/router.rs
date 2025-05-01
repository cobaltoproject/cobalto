use std::collections::HashMap;
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::time::Instant;
use crate::settings::Settings;

pub struct Response {
    pub status_code: u16,
    pub body: String,
    pub headers: HashMap<String, String>,
}

impl Response {
    pub fn ok(body: impl Into<String>) -> Self {
        Response {
            status_code: 200,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    pub fn forbidden(body: impl Into<String>) -> Self {
        Response {
            status_code: 403,
            body: body.into(),
            headers: HashMap::new(),
        }
    }

    pub fn not_found() -> Self {
        Response {
            status_code: 404,
            body: "404 Not Found".to_string(),
            headers: HashMap::new(),
        }
    }
}

pub struct RequestContext {
    pub path: String,
    pub params: HashMap<String, String>,
    pub is_authenticated: bool,
    pub start_time: Option<Instant>,
}

pub type Handler = Arc<dyn Fn(HashMap<String, String>) -> Pin<Box<dyn Future<Output = Response> + Send>> + Send + Sync>;

pub type Middleware = Arc<dyn Fn(&mut RequestContext) -> Option<Response> + Send + Sync>;

pub type PostMiddleware = Arc<dyn Fn(&RequestContext, Response) -> Response + Send + Sync>;

#[derive(Clone)]
pub struct Route {
    pub path_pattern: String,
    pub handler: Handler,
    pub middlewares: Vec<Middleware>,
}

pub struct Router {
    routes: Vec<Route>,
    middlewares: Vec<Middleware>,
    post_middlewares: Vec<PostMiddleware>,
}

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

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Unknown",
    }
}

impl Router {
    pub fn new() -> Self {
        Router {
            routes: Vec::new(),
            middlewares: Vec::new(),
            post_middlewares: Vec::new(),
        }
    }

    pub fn add_route(&mut self, path_pattern: &str, handler: Handler, middlewares: Vec<Middleware>) {
        self.routes.push(Route {
            path_pattern: path_pattern.to_string(),
            handler,
            middlewares,
        });
    }

    pub fn add_middleware(&mut self, middleware: Middleware) {
        self.middlewares.push(middleware);
    }

    pub fn add_post_middleware(&mut self, middleware: PostMiddleware) {
        self.post_middlewares.push(middleware);
    }

    pub async fn run(&self, settings: Settings) -> Result<(), Box<dyn std::error::Error>> {
        let addr_str = format!("{}:{}", settings.host, settings.port);
        let listener = TcpListener::bind(&addr_str).await?;
        println!("Server running on http://{}", addr_str);
        
        loop {
            let (mut socket, _) = listener.accept().await?;
            let routes = self.routes.clone();
            let middlewares = self.middlewares.clone();
            let post_middlewares = self.post_middlewares.clone();
        
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
                        is_authenticated: parts.get(1).map(|q| q.contains("token=abc123")).unwrap_or(false),
                        start_time: None,
                    };
                    
                    // Esegui i middleware
                    for middleware in &middlewares {
                        if let Some(response) = (middleware)(&mut ctx) {
                            send_response(socket, response).await;
                            return;
                        }
                    }
                    
                    // Matcha le routes
                    let mut response = Response::not_found();
                    
                    for route in &routes {
                        if let Some(params) = match_path(&route.path_pattern, &ctx.path) {
                            ctx.params = params;
                    
                            // Middleware specifici della rotta
                            for middleware in &route.middlewares {
                                if let Some(response) = (middleware)(&mut ctx) {
                                    send_response(socket, response).await;
                                    return;
                                }
                            }
                    
                            // Se tutti i middleware specifici passano, esegui handler
                            response = (route.handler)(ctx.params.clone()).await;
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
}

#[macro_export]
macro_rules! route {
    ($router:expr, $( $method:ident $path:expr => { $handler:expr $(, $middleware:expr )* } ),* $(,)?) => {
        $(
            $router.add_route(
                $path,
                Arc::new(|params| Box::pin($handler(params))),
                vec![$($middleware),*]
            );
        )*
    };
}

// Funzione che fa matching dei path e estrae i parametri
fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
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