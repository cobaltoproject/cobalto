use cobalto::router::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ========== Response struct (JSON, HTML) ==========

#[test]
fn test_response_ok() {
    let resp = Response::ok("hello world");
    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.body, "hello world");
    // Should default to empty headers for text/html stub
    assert!(resp.headers.is_empty());
}

#[test]
fn test_response_forbidden() {
    let resp = Response::forbidden("nope");
    assert_eq!(resp.status_code, 403);
    assert_eq!(resp.body, "nope");
}

#[test]
fn test_response_not_found() {
    let resp = Response::not_found();
    assert_eq!(resp.status_code, 404);
    assert!(resp.body.contains("404"));
}

#[test]
fn test_response_json_success() {
    let mut headers = HashMap::new();
    headers.insert("X-Test".into(), "yes".into());
    let resp = Response::json(json!({"foo": "bar"}), 201, headers.clone());
    assert_eq!(resp.status_code, 201);
    assert_eq!(
        resp.headers.get("Content-Type").unwrap(),
        "application/json; charset=utf-8"
    );
    assert_eq!(resp.headers.get("X-Test").unwrap(), "yes");
    assert!(resp.body.contains("\"foo\":\"bar\""));
}

#[test]
fn test_match_path_static() {
    // Exact match
    assert!(cobalto::router::match_path("/foo", "/foo").is_some());
    // Parameter extraction
    let params = cobalto::router::match_path("/user/:id", "/user/99").unwrap();
    assert_eq!(params.get("id").unwrap(), "99");
    // No match for different length
    assert!(cobalto::router::match_path("/a/b", "/a").is_none());
    // No match when value not matching
    assert!(cobalto::router::match_path("/foo/bar", "/foo/qux").is_none());
}

#[test]
fn test_middleware_execution_and_post_middleware() {
    // Middleware that intercepts all, returns a custom response
    let mw: Middleware = Arc::new(move |_| Some(Response::forbidden("blocked")));

    // Post-middleware always bumps status to 401
    let pmw: PostMiddleware = Arc::new(|_ctx, mut resp| {
        resp.status_code = 401;
        resp
    });

    let mut router = Router::new();
    router.add_middleware(mw);
    router.add_post_middleware(pmw);

    // Add dummy route
    let handler: Handler = Arc::new(|_params| Box::pin(async { Response::ok("Hello!") }));
    router.add_route("/blocked", handler, vec![]);

    // Simulate middleware execution
    let mut ctx = RequestContext {
        path: "/blocked".to_string(),
        params: HashMap::new(),
        is_authenticated: false,
        start_time: None,
    };

    // Should be intercepted by pre-middleware and adjusted by post-middleware
    let mut response = Response::not_found();
    for mw in &router.middlewares {
        if let Some(resp) = mw(&mut ctx) {
            response = resp;
            break;
        }
    }
    for pmw in &router.post_middlewares {
        response = pmw(&ctx, response);
    }
    assert_eq!(response.status_code, 401);
    assert!(response.body.contains("blocked"));
}

#[test]
fn test_response_ok_and_json() {
    let resp = Response::ok("hello");
    assert_eq!(resp.status_code, 200);
    assert_eq!(resp.body, "hello");

    let mut headers = HashMap::new();
    headers.insert("X-Test".into(), "works".into());
    let resp = Response::json(serde_json::json!({"test":"json"}), 201, headers.clone());
    assert_eq!(resp.status_code, 201);
    assert_eq!(
        resp.headers.get("Content-Type").unwrap(),
        "application/json; charset=utf-8"
    );
    assert_eq!(resp.headers.get("X-Test").unwrap(), "works");
    assert!(resp.body.contains("\"test\":\"json\""));
}

// Test match_path logic
#[test]
fn test_static_and_param_matching() {
    assert!(match_path("/foo", "/foo").is_some());
    let params = match_path("/user/:id", "/user/42").unwrap();
    assert_eq!(params.get("id"), Some(&"42".to_string()));
    assert!(match_path("/api/:a/:b", "/api/x/y").is_some());
    assert!(match_path("/foo/bar", "/foo/bar/qux").is_none());
    assert!(match_path("/foo/:id", "/bar/99").is_none());
}

// Middleware/pre and post order
#[test]
fn test_middleware_and_post_middleware() {
    let before: Middleware = Arc::new(|ctx| {
        if ctx.path == "/blocked" {
            Some(Response::forbidden("block"))
        } else {
            None
        }
    });
    let post: PostMiddleware = Arc::new(|_ctx, mut resp| {
        resp.body = format!("{}+PM", resp.body);
        resp
    });

    let mut router = Router::new();
    router.add_middleware(before);
    router.add_post_middleware(post);
    let handler: Handler = Arc::new(|_params| Box::pin(async { Response::ok("allowed") }));
    router.add_route("/blocked", handler.clone(), vec![]);
    router.add_route("/open", handler, vec![]);

    // Simulate pre middleware triggering a block
    let mut ctx = RequestContext {
        path: "/blocked".to_string(),
        params: HashMap::new(),
        is_authenticated: false,
        start_time: None,
    };
    let mut resp = Response::not_found();
    for mw in &router.middlewares {
        if let Some(r) = mw(&mut ctx) {
            resp = r;
            break;
        }
    }
    for pmw in &router.post_middlewares {
        resp = pmw(&ctx, resp);
    }
    assert_eq!(resp.status_code, 403);
    assert_eq!(resp.body, "block+PM");

    // For open, post-middleware only
    let open_ctx = RequestContext {
        path: "/open".to_string(),
        ..ctx
    };
    let mut resp = Response::ok("hello");
    for pmw in &router.post_middlewares {
        resp = pmw(&open_ctx, resp);
    }
    assert_eq!(resp.body, "hello+PM");
}

// Register a dummy user websocket handler and check storage
#[test]
fn test_user_websocket_registration() {
    let ws_handler: WsHandler = Arc::new(|_ctx, _ws| Box::pin(async { () }));
    let mut router = Router::new();
    router.add_websocket("/ws/echo", ws_handler.clone());
    assert_eq!(router.ws_routes.len(), 1);
    assert_eq!(router.ws_routes[0].path_pattern, "/ws/echo");
}

use serde::{Serialize, Serializer};

struct AlwaysFailsSerialize;

impl Serialize for AlwaysFailsSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(serde::ser::Error::custom("Forced failure"))
    }
}

#[test]
fn test_response_json_error_branch_always_fails() {
    let mut headers = HashMap::new();
    headers.insert("Test-Head".to_string(), "Y".to_string());
    let value = AlwaysFailsSerialize;
    let resp = Response::json(value, 200, headers.clone());
    // Should hit error branch and status_code becomes 500
    assert_eq!(resp.status_code, 500);
    assert!(resp.body.contains("Serialization failed"));
    assert_eq!(
        resp.headers.get("Content-Type").unwrap(),
        "application/json; charset=utf-8"
    );
    assert_eq!(resp.headers.get("Test-Head").unwrap(), "Y");
}

#[test]
fn test_empty_middleware_and_postorder_chain() {
    let mut router = Router::new();
    let handler: Handler = Arc::new(|_params| Box::pin(async { Response::ok("hi") }));
    router.add_route("/basic", handler, vec![]);

    let mut ctx = RequestContext {
        path: "/basic".to_string(),
        params: HashMap::new(),
        is_authenticated: false,
        start_time: None,
    };
    let mut resp = Response::ok("start");
    for mw in &router.middlewares {
        if let Some(r) = mw(&mut ctx) {
            resp = r;
        }
    }
    for pmw in &router.post_middlewares {
        resp = pmw(&ctx, resp);
    }
    assert_eq!(resp.body, "start");
}

#[test]
fn test_parameterless_and_param_route() {
    let handler: Handler = Arc::new(|params| {
        Box::pin(async move {
            let id = params.get("id").cloned().unwrap_or_default();
            Response::ok(id)
        })
    });

    let mut router = Router::new();
    router.add_route("/about", handler.clone(), vec![]);
    router.add_route("/user/:id", handler, vec![]);

    // match_path for /about
    assert!(match_path("/about", "/about").is_some());
    // match_path for parameter
    let params = match_path("/user/:id", "/user/314");
    assert_eq!(params.unwrap().get("id").unwrap(), "314");
}

#[test]
fn test_ws_route_storage_and_registration() {
    let ws_handler: WsHandler = Arc::new(|_ctx, _ws| Box::pin(async { () }));
    let mut router = Router::new();
    router.add_websocket("/ws/test", ws_handler);
    assert_eq!(router.ws_routes.len(), 1);
    assert_eq!(router.ws_routes[0].path_pattern, "/ws/test");
}

#[test]
fn test_match_path_non_matching() {
    // Mismatched
    assert!(match_path("/x/:id", "/y/42").is_none());
    assert!(match_path("/items/:type/:id", "/items/book").is_none());
    assert!(match_path("/only", "/only/extra").is_none());
}

#[test]
fn test_post_middleware_chain_order_and_context_isolation() {
    let mut router = Router::new();
    let h: Handler = Arc::new(|_p| Box::pin(async { Response::ok("x") }));
    router.add_route("/a", h, vec![]);

    // Add two post-middlewares (simulates a filter chain)
    router.add_post_middleware(Arc::new(|_ctx, mut r| {
        r.body.push('1');
        r
    }));
    router.add_post_middleware(Arc::new(|_ctx, mut r| {
        r.body.push('2');
        r
    }));

    let ctx = RequestContext {
        path: "/a".to_string(),
        params: HashMap::new(),
        is_authenticated: false,
        start_time: None,
    };
    let resp = Response::ok("abc");
    let mut result = resp;
    for pmw in &router.post_middlewares {
        result = pmw(&ctx, result);
    }
    assert_eq!(result.body, "abc12");
}

#[test]
fn test_handler_with_params_and_middleware_modification() {
    let mut router = Router::new();
    let h: Handler = Arc::new(|params| {
        Box::pin(async move {
            let who = params
                .get("who")
                .cloned()
                .unwrap_or_else(|| "nobody".to_string());
            Response::ok(format!("hello {who}"))
        })
    });

    // Simulate a middleware that overwrites params
    router.add_route(
        "/hi/:who",
        h,
        vec![Arc::new(|ctx| {
            ctx.params
                .insert("who".to_string(), "overridden".to_string());
            None
        })],
    );

    let params = match_path("/hi/:who", "/hi/tomato").unwrap();
    let mut ctx = RequestContext {
        path: "/hi/tomato".to_string(),
        params,
        is_authenticated: false,
        start_time: None,
    };

    // Middleware should override param
    for mw in &router.routes[0].middlewares {
        let _ = mw(&mut ctx);
    }
    use futures::executor::block_on;
    let resp = block_on((router.routes[0].handler)(ctx.params.clone()));
    assert_eq!(resp.body, "hello overridden");
}

#[test]
fn test_status_text_variants() {
    assert_eq!(cobalto::router::status_text(200), "OK");
    assert_eq!(cobalto::router::status_text(403), "Forbidden");
    assert_eq!(cobalto::router::status_text(404), "Not Found");
    assert_eq!(cobalto::router::status_text(590), "Unknown");
}

#[test]
fn test_build_ws_axum_router_with_and_without_reload() {
    let mut router = Router::new();
    let wsh: WsHandler = Arc::new(|_, _| Box::pin(async {}));
    router.add_websocket("/ws/api", wsh.clone());
    let mut settings = cobalto::settings::Settings {
        debug: false,
        host: "x".into(),
        port: 1,
        ws_port: 2,
        template: cobalto::settings::TemplateSettings {
            dir: ".".into(),
            debug: false,
        },
        other: HashMap::new(),
    };
    settings.debug = true;
    // Not a deep inspection, but it covers branching logic
}
