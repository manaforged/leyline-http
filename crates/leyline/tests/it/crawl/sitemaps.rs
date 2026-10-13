use super::*;

#[tokio::test]
async fn a_crawl_opens_child_sitemaps_only_while_it_needs_pages() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let robots = format!("User-agent: *\nSitemap: {base}/index.xml\n");
    let children: String = (0..5)
        .map(|i| format!("<sitemap><loc>{base}/s{i}.xml</loc></sitemap>"))
        .collect();
    let index = format!("<sitemapindex>{children}</sitemapindex>");
    let page_base = base.clone();
    let server = TestServer::http_on(listener, move |request| {
        let target = request.target.as_str();
        match target {
            "/robots.txt" => TestResponse::new(200).body(robots.clone()),
            "/index.xml" => TestResponse::new(200).body(index.clone()),
            _ if target.ends_with(".xml") => {
                let urls: String = (0..3)
                    .map(|j| format!("<url><loc>{page_base}{}/{j}</loc></url>", &target[..3]))
                    .collect();
                TestResponse::new(200).body(format!("<urlset>{urls}</urlset>"))
            }
            path => html(&format!("<p>{path}</p>")),
        }
    })
    .unwrap();
    let mut crawl = Session::new().crawl(server.url("/")).limit(2);
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let opened = targets(&server)
        .await
        .into_iter()
        .filter(|target| target.starts_with("/s") && target.ends_with(".xml"))
        .count();
    assert!(opened <= 2, "{opened} child sitemaps opened");
}

#[tokio::test]
async fn a_crawl_reads_every_sitemap_form_and_only_the_ones_it_may() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let robots = format!(
        "User-agent: *\nDisallow: /private\nSitemap: {base}/index.xml\nSitemap: {base}/missing.xml\nSitemap: {base}/list.txt\nSitemap: {base}/private/listed.xml\n"
    );
    let index = format!(
        r#"<sm:sitemapindex xmlns:sm="http://www.sitemaps.org/schemas/sitemap/0.9">
           <sm:sitemap><sm:loc>{base}/ns.xml</sm:loc></sm:sitemap>
           <sm:sitemap><sm:loc>{base}/private/hidden.xml</sm:loc></sm:sitemap>
           <sm:sitemap><sm:loc>http://localhost:{port}/far.xml</sm:loc></sm:sitemap>
           </sm:sitemapindex>"#,
        port = addr.port()
    );
    let child = format!("<urlset><url><loc><![CDATA[{base}/cdata?a=1&b=2]]></loc></url></urlset>");
    let missing = format!("<urlset><url><loc>{base}/ghost</loc></url></urlset>");
    let list = format!("{base}/plain\n");
    let server = TestServer::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => TestResponse::new(200).body(robots.clone()),
        "/index.xml" => TestResponse::new(200).body(index.clone()),
        "/ns.xml" => TestResponse::new(200).body(child.clone()),
        "/missing.xml" => TestResponse::new(404).body(missing.clone()),
        "/list.txt" => TestResponse::new(200)
            .header("content-type", "text/plain")
            .body(list.clone()),
        "/private/hidden.xml" | "/private/listed.xml" | "/far.xml" => {
            TestResponse::new(200).body("<urlset/>")
        }
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .respect(Respect::Robots);
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().to_string());
    }
    assert_eq!(
        kept,
        BTreeSet::from([
            format!("{base}/"),
            format!("{base}/cdata?a=1&b=2"),
            format!("{base}/plain"),
        ])
    );
    let sent = targets(&server).await;
    for never in [
        "/private/hidden.xml",
        "/private/listed.xml",
        "/far.xml",
        "/ghost",
    ] {
        assert!(!sent.contains(never), "{never} was requested");
    }
}
