# Contributing to Cobalto

Thank you for your interest in contributing to Cobalto!

## How to Contribute

1. Fork the repository and create your branch.
2. Follow existing code style and documentation conventions.
3. Add or update unit tests for any new public (and most internal) APIs.

## Running the Tests

Run all tests with:
```
cargo test
```
To check test coverage:
```
cargo tarpaulin
```

**Note:** Some lines (especially in async manual HTTP/WS loop code, server run-loops, and file watchers) are hard or impossible to exercise in unit tests and may remain uncovered. These are considered "exempt." Integrate and test where possible with integration or e2e tests.

## Style

- Document all public APIs with doc-comments (`///`).
- Write clear and descriptive commit messages.
- Favor small, focused pull requests.

## Issues & Feature Requests

Please use GitHub Issues to report bugs or propose new features.

---

Happy hacking!
