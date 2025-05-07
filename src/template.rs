//! Cobalto Template Engine with Tailwind Support
//!
//! This module implements a Django-inspired template engine for Rust, now with Tailwind integration.
//!
//! Workflow:
//! 1. `render_template` loads the child template.
//! 2. `tokenize_template` splits content into Text, Variable, and Tag tokens.
//! 3. `parse_tokens` and `parse_nodes` build an AST of `Node`.
//! 4. Child `Block` definitions and `Extends` tag are collected.
//! 5. `merge_blocks` merges child blocks into the base template, replacing all matching blocks by name (supports multiple occurrences).
//! 6. `render_nodes` walks the merged AST and outputs HTML, resolving variables, `if` conditions, `for` loops, and Tailwind imports via `{% tailwind %}`.
//!
//! Runtime logging is controlled via `set_display_logs`.

use log::debug;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::router::Response;

/// Global switch for enabling/disabling internal template logs
static DISPLAY_LOGS: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// Enable or disable internal debug logs for the template engine
pub fn set_display_logs(enabled: bool) {
    DISPLAY_LOGS.store(enabled, Ordering::Relaxed);
}

/// Internal debug: logs only if DISPLAY_LOGS is true
macro_rules! tdebug {
    ($($arg:tt)+) => {
        if DISPLAY_LOGS.load(Ordering::Relaxed) {
            debug!($($arg)+);
        }
    }
}

/// Supported value types for template context
#[derive(Clone)]
pub enum TemplateValue {
    String(String),
    Bool(bool),
    Number(f64),
    List(Vec<TemplateValue>),
    Object(HashMap<String, TemplateValue>),
}

impl TemplateValue {
    /// Convert the value to a string for rendering
    pub fn as_string(&self) -> String {
        match self {
            TemplateValue::String(s) => s.clone(),
            TemplateValue::Bool(b) => b.to_string(),
            TemplateValue::Number(n) => n.to_string(),
            TemplateValue::List(_) | TemplateValue::Object(_) => String::new(),
        }
    }
}

impl fmt::Display for TemplateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_string())
    }
}

/// Token types extracted from the template
#[derive(Debug, Clone)]
pub enum Token {
    Text(String),     // Plain text
    Variable(String), // {{ variable }}
    Tag(String),      // {% tag %}
}

/// AST node types for the template engine
#[derive(Debug, Clone)]
pub enum Node {
    Text(String),
    Variable(String),
    If {
        condition: String,
        then_body: Vec<Node>,
        else_body: Vec<Node>,
    },
    For {
        var_name: String,
        list_name: String,
        body: Vec<Node>,
    },
    Block {
        name: String,
        body: Vec<Node>,
    },
    Extends(String), // {% extends "base.html" %}
    Tailwind,        // {% tailwind %}
}

/// Tokenizes the template content into a Vec<Token>
pub fn tokenize_template(content: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let re = Regex::new(r"(?s)(\{\{.*?\}\}|\{%.*?%\})").unwrap();
    let mut last_end = 0;
    for mat in re.find_iter(content) {
        let start = mat.start();
        let end = mat.end();
        if start > last_end {
            tokens.push(Token::Text(content[last_end..start].to_string()));
        }
        let m = mat.as_str().trim();
        if m.starts_with("{{") {
            let inner = m
                .trim_start_matches("{{")
                .trim_end_matches("}}")
                .trim()
                .to_string();
            tdebug!("tokenize: Variable '{{ {{ {} }} }}'", inner);
            tokens.push(Token::Variable(inner));
        } else {
            let inner = m
                .trim_start_matches("{% ")
                .trim_end_matches(" %}")
                .trim()
                .to_string();
            tdebug!("tokenize: Tag '{{% {} %}}'", inner);
            tokens.push(Token::Tag(inner));
        }
        last_end = end;
    }
    if last_end < content.len() {
        tokens.push(Token::Text(content[last_end..].to_string()));
    }
    tokens
}

/// Parses a sequence of Token into an AST of Node
pub fn parse_tokens(tokens: &[Token]) -> Vec<Node> {
    let mut idx = 0;
    parse_nodes(tokens, &mut idx, &[])
}

/// Recursive parser: consumes tokens until an `end_tag` is found
fn parse_nodes(tokens: &[Token], idx: &mut usize, end_tags: &[&str]) -> Vec<Node> {
    let mut nodes = Vec::new();
    while *idx < tokens.len() {
        match &tokens[*idx] {
            Token::Text(t) => {
                nodes.push(Node::Text(t.clone()));
                *idx += 1;
            }
            Token::Variable(v) => {
                nodes.push(Node::Variable(v.clone()));
                *idx += 1;
            }
            Token::Tag(tag) => {
                let t = tag.trim();
                if end_tags.contains(&t) {
                    break;
                }
                // Handle extends
                if let Some(rest) = t.strip_prefix("extends ") {
                    nodes.push(Node::Extends(rest.trim_matches('"').to_string()));
                    *idx += 1;
                    continue;
                }
                // Handle block
                if let Some(name) = t.strip_prefix("block ") {
                    *idx += 1;
                    let body = parse_nodes(tokens, idx, &["endblock"]);
                    *idx += 1; // skip endblock
                    nodes.push(Node::Block {
                        name: name.to_string(),
                        body,
                    });
                    continue;
                }
                // Handle if/else/endif
                if let Some(cond) = t.strip_prefix("if ") {
                    *idx += 1;
                    let then_body = parse_nodes(tokens, idx, &["else", "endif"]);
                    let mut else_body = Vec::new();
                    if *idx < tokens.len() {
                        if let Token::Tag(tt) = &tokens[*idx] {
                            if tt.trim() == "else" {
                                *idx += 1;
                                else_body = parse_nodes(tokens, idx, &["endif"]);
                            }
                        }
                    }
                    *idx += 1; // skip endif
                    nodes.push(Node::If {
                        condition: cond.to_string(),
                        then_body,
                        else_body,
                    });
                    continue;
                }
                // Handle for/endfor
                if let Some(rest) = t.strip_prefix("for ") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() == 3 && parts[1] == "in" {
                        *idx += 1;
                        let body = parse_nodes(tokens, idx, &["endfor"]);
                        *idx += 1; // skip endfor
                        nodes.push(Node::For {
                            var_name: parts[0].to_string(),
                            list_name: parts[2].to_string(),
                            body,
                        });
                        continue;
                    }
                }
                // Handle tailwind tag
                if t == "tailwind" {
                    nodes.push(Node::Tailwind);
                    *idx += 1;
                    continue;
                }
                // Unknown tag: skip
                *idx += 1;
            }
        }
    }
    nodes
}

/// Resolves a dotted variable path 'a.b.c' within the context
fn resolve_variable<'a>(
    name: &str,
    context: &'a HashMap<String, TemplateValue>,
) -> Option<&'a TemplateValue> {
    let mut current: Option<&TemplateValue> = None;
    for (i, key) in name.split('.').enumerate() {
        if i == 0 {
            current = context.get(key);
        } else if let Some(TemplateValue::Object(map)) = current {
            current = map.get(key);
        } else {
            return None;
        }
    }
    current
}

/// Merges child blocks into base AST by matching block names
fn merge_blocks(nodes: &[Node], child_blocks: &HashMap<String, Vec<Node>>) -> Vec<Node> {
    nodes
        .iter()
        .map(|node| match node {
            Node::Block { name, body } => {
                if let Some(child) = child_blocks.get(name) {
                    Node::Block {
                        name: name.clone(),
                        body: child.clone(),
                    }
                } else {
                    Node::Block {
                        name: name.clone(),
                        body: merge_blocks(body, child_blocks),
                    }
                }
            }
            Node::If {
                condition,
                then_body,
                else_body,
            } => Node::If {
                condition: condition.clone(),
                then_body: merge_blocks(then_body, child_blocks),
                else_body: merge_blocks(else_body, child_blocks),
            },
            Node::For {
                var_name,
                list_name,
                body,
            } => Node::For {
                var_name: var_name.clone(),
                list_name: list_name.clone(),
                body: merge_blocks(body, child_blocks),
            },
            Node::Text(t) => Node::Text(t.clone()),
            Node::Variable(v) => Node::Variable(v.clone()),
            Node::Extends(e) => Node::Extends(e.clone()),
            Node::Tailwind => Node::Tailwind,
        })
        .collect()
}

/// Renders the AST into HTML string using the context
pub fn render_nodes(nodes: &[Node], context: &HashMap<String, TemplateValue>) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Variable(name) => {
                if let Some(val) = resolve_variable(name, context) {
                    out.push_str(&val.as_string());
                }
            }
            Node::If {
                condition,
                then_body,
                else_body,
            } => {
                if let Some(TemplateValue::Bool(true)) = resolve_variable(condition, context) {
                    out.push_str(&render_nodes(then_body, context));
                } else {
                    out.push_str(&render_nodes(else_body, context));
                }
            }
            Node::For {
                var_name,
                list_name,
                body,
            } => {
                if let Some(TemplateValue::List(items)) =
                    resolve_variable(list_name, context).cloned()
                {
                    for item in items {
                        let mut local = context.clone();
                        local.insert(var_name.clone(), item);
                        out.push_str(&render_nodes(body, &local));
                    }
                }
            }
            Node::Block { body, .. } => {
                out.push_str(&render_nodes(body, context));
            }
            Node::Extends(_) => {}
            Node::Tailwind => {
                tdebug!("Inserting Tailwind CDN link");
                out.push_str(r#"<script src="https://cdn.tailwindcss.com"></script>"#);
            }
        }
    }
    out
}

/// Main entry: loads child template, merges with base, and renders HTML
pub fn render_template(template_name: &str, context: &HashMap<String, TemplateValue>) -> Response {
    // Load child template
    let child_path = format!("templates/{}", template_name);
    let child = match std::fs::read_to_string(&child_path) {
        Ok(c) => c,
        Err(_) => {
            return Response {
                status_code: 404,
                body: format!("Template '{}' not found", template_name),
                headers: [(
                    "Content-Type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                )]
                .iter()
                .cloned()
                .collect(),
            };
        }
    };
    let child_nodes = parse_tokens(&tokenize_template(&child));
    tdebug!("Child AST: {:?}", child_nodes);

    // Collect child blocks and detect base
    let mut child_blocks = HashMap::new();
    let mut base_t: Option<String> = None;
    for node in &child_nodes {
        if let Node::Extends(b) = node {
            base_t = Some(b.clone());
        }
        if let Node::Block { name, body } = node {
            child_blocks.insert(name.clone(), body.clone());
        }
    }

    // If extends, load base, merge and render
    let html: String;
    if let Some(base) = base_t {
        let base_content = std::fs::read_to_string(format!("templates/{}", base))
            .unwrap_or(format!("Template '{}' not found", base));
        let base_nodes = parse_tokens(&tokenize_template(&base_content));
        tdebug!("Base AST: {:?}", base_nodes);
        let merged = merge_blocks(&base_nodes, &child_blocks);
        tdebug!("Merged AST: {:?}", merged);
        html = render_nodes(&merged, context);
    } else {
        // Otherwise, merge child blocks and render directly
        let merged = merge_blocks(&child_nodes, &child_blocks);
        html = render_nodes(&merged, context)
    }

    Response {
        status_code: 200,
        body: html.to_string(),
        headers: [(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )]
        .iter()
        .cloned()
        .collect(),
    }
}
