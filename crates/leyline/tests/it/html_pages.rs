use leyline::html;

#[test]
fn pages_give_meta_tokens_and_links() {
    let page = r#"<html><head>
<meta name="CSRF-Token" content="a&amp;b">
<meta property="og:title" content="Cart">
</head><body>
<script>var s = "<a href='/fake'>x</a>";</script>
<a href="/p/1" rel="next">First
  product</a>
<a name="anchor">no href</a>
</body></html>"#;
    assert_eq!(html::meta(page, "csrf-token").as_deref(), Some("a&b"));
    assert_eq!(html::meta(page, "og:title").as_deref(), Some("Cart"));
    let links = html::links(page);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].href(), "/p/1");
    assert_eq!(links[0].text(), "First product");
    assert_eq!(links[0].rel(), Some("next"));
}

#[test]
fn pages_give_readable_text() {
    let page = r#"<!doctype html><html><head><title>Guide</title>
<style>p { color: red }</style></head><body>
<svg viewBox="0 0 8 8"><text>icon</text></svg>
<h1>Install &amp; run</h1>
<p>Read the <a href="/g">guide</a>, then
   <code>cargo run</code>.<br>Done.</p>
<script>var x = "<p>fake</p>";</script>
<noscript><p>Enable scripts</p></noscript>
<ul><li>fast</li><li>small</li></ul>
<pre>
fn main() {
    run();
}
</pre>
</body></html>"#;
    assert_eq!(
        html::text(page),
        "Install & run\nRead the guide, then cargo run.\nDone.\nfast\nsmall\nfn main() {\n    run();\n}"
    );
    assert_eq!(
        html::text("<html><head><title>T</title><meta charset=utf-8><p>Body text<p>More"),
        "Body text\nMore"
    );
}

#[test]
fn pages_keep_text_around_comments_brackets_and_empty_templates() {
    assert_eq!(
        html::text("Lead text<p>one <!-- note --> two</p><p>a < b and 1<2</p>"),
        "Lead text\none two\na < b and 1<2"
    );
    assert_eq!(
        html::text("<p>shown</p><template/><p>hidden</p></template><p>after</p>"),
        "shown\nafter"
    );
}

#[test]
fn pages_ignore_the_value_of_a_directive_they_do_not_use() {
    let page = r#"<meta name="robots" content="max-image-preview: none">"#;
    let rules = html::Directives::read(page, &http::HeaderMap::new(), "*");
    assert!(rules.index() && rules.follow());
}

#[test]
fn pages_apply_rules_named_for_the_agents_product_token() {
    let page = r#"<meta name="ExampleBot" content="noindex">"#;
    let rules = html::Directives::read(
        page,
        &http::HeaderMap::new(),
        "ExampleBot/2.1 (+https://example.com/bot)",
    );
    assert!(!rules.index());
}

#[test]
fn pages_give_robots_directives_and_canonical() {
    let page = r#"<html><head>
<meta name="robots" content="noindex">
<meta name="ExampleBot" content="nofollow, noai">
<meta name="otherbot" content="none">
<link rel="alternate stylesheet" href="/dark.css">
<link rel="Canonical" href="/guide/">
</head><body><p>x</p></body></html>"#;
    let headers = http::HeaderMap::new();
    let directives = html::Directives::read(page, &headers, "examplebot");
    assert!(!directives.index());
    assert!(!directives.follow());
    assert!(!directives.ai());
    let plain = html::Directives::read(page, &headers, "*");
    assert!(!plain.index());
    assert!(plain.follow());
    assert!(plain.ai());
    assert_eq!(html::canonical(page).as_deref(), Some("/guide/"));

    let mut headers = http::HeaderMap::new();
    headers.append("x-robots-tag", "otherbot: noindex".parse().unwrap());
    headers.append("x-robots-tag", "examplebot: nofollow".parse().unwrap());
    headers.append(
        "x-robots-tag",
        "unavailable_after: 25 Jun 2030 15:00:00 PST, noai"
            .parse()
            .unwrap(),
    );
    let bare = "<p>no meta</p>";
    let directives = html::Directives::read(bare, &headers, "ExampleBot");
    assert!(directives.index());
    assert!(!directives.follow());
    assert!(!directives.ai());
    assert_eq!(html::canonical(bare), None);
}
