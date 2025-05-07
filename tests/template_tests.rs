use cobalto::template;
use cobalto::template::*;
use std::collections::HashMap;

#[test]
fn test_tokenize_basic() {
    let input = "Hello, {{ username }}! {% if user.is_admin %}Admin!{% endif %}";
    let tokens = tokenize_template(input);
    println!("Tokens: {:?}", tokens);

    assert_eq!(tokens.len(), 6);
    match &tokens[1] {
        Token::Variable(var) => assert_eq!(var, "username"),
        _ => panic!("Expected variable token"),
    }
    match &tokens[2] {
        Token::Text(text) => assert_eq!(text, "! "),
        _ => panic!("Expected text token"),
    }
}

#[test]
fn test_parse_simple_nodes() {
    let input = "Welcome, {{user.name}}";
    let tokens = tokenize_template(input);
    let nodes = parse_tokens(&tokens);

    assert_eq!(nodes.len(), 2);
    match &nodes[1] {
        Node::Variable(var) => assert_eq!(var, "user.name"),
        _ => panic!("Expected variable node"),
    }
}

#[test]
fn test_render_nodes_text_and_variable() {
    let nodes = vec![
        Node::Text("Hello, ".to_string()),
        Node::Variable("username".to_string()),
        Node::Text("!".to_string()),
    ];
    let mut context = HashMap::new();
    context.insert(
        "username".to_string(),
        TemplateValue::String("Alessandro".to_string()),
    );
    let rendered = template::render_nodes(&nodes, &context);
    assert_eq!(rendered, "Hello, Alessandro!");
}

#[test]
fn test_render_if_block_true() {
    let nodes = vec![Node::If {
        condition: "is_admin".to_string(),
        then_body: vec![Node::Text("Welcome admin!".to_string())],
        else_body: vec![Node::Text("Welcome user!".to_string())],
    }];
    let mut context = HashMap::new();
    context.insert("is_admin".to_string(), TemplateValue::Bool(true));
    let rendered = cobalto::template::render_nodes(&nodes, &context);
    assert_eq!(rendered, "Welcome admin!");
}

#[test]
fn test_render_if_block_false() {
    let nodes = vec![Node::If {
        condition: "is_admin".to_string(),
        then_body: vec![Node::Text("Welcome admin!".to_string())],
        else_body: vec![Node::Text("Welcome user!".to_string())],
    }];
    let mut context = HashMap::new();
    context.insert("is_admin".to_string(), TemplateValue::Bool(false));
    let rendered = cobalto::template::render_nodes(&nodes, &context);
    assert_eq!(rendered, "Welcome user!");
}

#[test]
fn test_render_for_loop() {
    let nodes = vec![Node::For {
        var_name: "item".to_string(),
        list_name: "shopping".to_string(),
        body: vec![
            Node::Variable("item".to_string()),
            Node::Text(",".to_string()),
        ],
    }];
    let mut context = HashMap::new();
    context.insert(
        "shopping".to_string(),
        TemplateValue::List(vec![
            TemplateValue::String("Apple".to_string()),
            TemplateValue::String("Banana".to_string()),
        ]),
    );
    let rendered = cobalto::template::render_nodes(&nodes, &context);
    assert_eq!(rendered, "Apple,Banana,");
}

#[test]
fn test_tailwind_tag_inserts_cdn() {
    let nodes = vec![
        Node::Text("start".into()),
        Node::Tailwind,
        Node::Text("end".into()),
    ];
    let context = HashMap::new();
    let html = cobalto::template::render_nodes(&nodes, &context);
    assert!(html.contains("https://cdn.tailwindcss.com"));
    assert!(html.contains("start"));
    assert!(html.contains("end"));
}

#[test]
fn test_block_and_extends_logic() {
    use cobalto::template::*;
    use std::fs;

    fs::create_dir_all("templates").unwrap();

    fs::write(
        "templates/test_base.html",
        "{% block content %}Base{% endblock %}!",
    )
    .unwrap();
    fs::write(
        "templates/test_child.html",
        "{% extends \"test_base.html\" %}{% block content %}Hello{% endblock %}",
    )
    .unwrap();

    let context = HashMap::new();
    let resp = render_template("test_child.html", &context);

    assert_eq!(resp.body, "Hello!");

    fs::remove_file("templates/test_base.html").unwrap();
    fs::remove_file("templates/test_child.html").unwrap();
}

#[test]
fn test_template_not_found_branch() {
    let ctx = HashMap::new();
    let resp = cobalto::template::render_template("hopefully_does_not_exist_zzz999.html", &ctx);
    assert!(resp.body.contains("not found"));
}

#[test]
fn test_unknown_tag_branch() {
    let tokens = vec![
        Token::Tag("unknown_tag whatisthis".into()),
        Token::Text("after".into()),
    ];
    let nodes = parse_tokens(&tokens);
    assert!(matches!(&nodes[0], Node::Text(_)) || matches!(&nodes[0], Node::Extends(_)));
}

#[test]
fn test_template_logging_coverage() {
    set_display_logs(true);
    // Any rendering or parsing will trigger tdebug! branches.
    let ctx = HashMap::new();
    let _ = render_template("hopefully_does_not_exist_zzz999.html", &ctx);
    set_display_logs(false); // for cleanup
}
