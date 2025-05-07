# Cobalto

**Cobalto** is a fast, batteries-included web framework for Rust, inspired by Django and Laravel.

- 🚀 Modern async, real-time, and HTTP API support out of the box.
- 🔌 Batteries-included: template engine, live reload, middleware, and easy routing.
- 🦀 Built for Rustaceans: safe, robust, and professional.

## Features

- Easy, familiar route/handler syntax
- User-friendly middleware API
- WebSocket support with route matching
- Live reload for development
- Django-style template engine with blocks and inheritance

## Quickstart

Add Cobalto to your `Cargo.toml`:

```toml
[dependencies]
cobalto = "0.1"
```

Example entrypoint:
```
use cobalto::router::*;

#[tokio::main]
async fn main() {
    let mut router = Router::new();
    router.add_route("/", Arc::new(|_| Box::pin(async { Response::ok("Hello, Cobalto!") })), vec![]);
    // Register more routes, websockets, and middlewares here.
    let settings = cobalto::settings::Settings {
        debug: true,
        host: "127.0.0.1".to_string(),
        port: 8080,
        ws_port: 9000,
        // ...
    };
    router.run(settings).await.unwrap();
}
```

### Testing

Run all tests:
```
cargo test
```

Check code coverage:
```
cargo tarpaulin
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

MIT


### Happy building with Cobalto!
