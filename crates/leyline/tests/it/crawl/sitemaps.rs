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
    let server = testing::Server::http_on(listener, move |request| {
        let target = request.target.as_str();
        match target {
            "/robots.txt" => testing::Response::new(200).body(robots.clone()),
            "/index.xml" => testing::Response::new(200).body(index.clone()),
            _ if target.ends_with(".xml") => {
                let urls: String = (0..3)
                    .map(|j| format!("<url><loc>{page_base}{}/{j}</loc></url>", &target[..3]))
                    .collect();
                testing::Response::new(200).body(format!("<urlset>{urls}</urlset>"))
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
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(200).body(robots.clone()),
        "/index.xml" => testing::Response::new(200).body(index.clone()),
        "/ns.xml" => testing::Response::new(200).body(child.clone()),
        "/missing.xml" => testing::Response::new(404).body(missing.clone()),
        "/list.txt" => testing::Response::new(200)
            .header("content-type", "text/plain")
            .body(list.clone()),
        "/private/hidden.xml" | "/private/listed.xml" | "/far.xml" => {
            testing::Response::new(200).body("<urlset/>")
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

#[tokio::test]
async fn a_crawl_ends_at_its_limit_without_waiting_for_sitemaps() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/sitemap.xml" => testing::Response::new(200)
            .body("<urlset></urlset>")
            .delay(Duration::from_secs(10)),
        "/" => html(r#"<p>home</p><a href="/a">A</a>"#),
        _ => html("<p>a</p>"),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .limit(2)
        .host_limits(leyline::HostLimits::new());
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        let mut pages = 0;
        while let Some(page) = crawl.next().await {
            page.unwrap();
            pages += 1;
        }
        pages
    })
    .await;
    assert_eq!(ended, Ok(2));
    assert!(targets(&server).await.contains("/sitemap.xml"));
}

#[tokio::test]
async fn a_crawl_reads_a_sitemap_on_another_host_of_the_same_site_only() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let local = listener.local_addr().unwrap();
    let port = local.port();
    let robots = format!(
        "User-agent: *\nSitemap: http://maps.example.test:{port}/map.xml\nSitemap: http://other.test:{port}/far.xml\n"
    );
    let map =
        format!("<urlset><url><loc>http://www.example.test:{port}/listed</loc></url></urlset>");
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(200).body(robots.clone()),
        "/map.xml" => testing::Response::new(200).body(map.clone()),
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let session = Session::builder()
        .resolve("www.example.test", [local])
        .resolve("maps.example.test", [local])
        .resolve("other.test", [local])
        .build()
        .unwrap();
    let mut crawl = session
        .crawl(format!("http://www.example.test:{port}/"))
        .host_limits(leyline::HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert!(kept.contains("/listed"), "{kept:?}");
    assert!(!targets(&server).await.contains("/far.xml"));
}

#[tokio::test]
async fn a_crawl_skips_image_locations_in_a_sitemap() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let map = format!(
        r#"<urlset xmlns:image="http://www.google.com/schemas/sitemap-image/1.1"><url><loc>{base}/page</loc><image:image><image:loc>{base}/photo.jpg</image:loc></image:image></url></urlset>"#
    );
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/sitemap.xml" => testing::Response::new(200).body(map.clone()),
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    let sent = targets(&server).await;
    assert!(sent.contains("/page"));
    assert!(!sent.contains("/photo.jpg"));
}

#[tokio::test]
async fn a_crawl_does_not_follow_a_sitemap_redirect_past_robots() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(200).body("User-agent: *\nDisallow: /private\n"),
        "/sitemap.xml" => testing::Response::new(301).header("location", "/private/map.xml"),
        path => html(&format!("<p>{path}</p>")),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert!(!targets(&server).await.contains("/private/map.xml"));
}

#[tokio::test]
async fn a_crawl_counts_a_failed_sitemap_and_skips_a_cut_gzip() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let robots = format!("User-agent: *\nSitemap: {base}/broken.xml\nSitemap: {base}/cut.xml.gz\n");
    let full = gzip(&format!(
        "<urlset><url><loc>{base}/listed</loc></url></urlset>"
    ));
    let cut = full[..full.len() - 12].to_vec();
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/robots.txt" => testing::Response::new(200).body(robots.clone()),
        "/broken.xml" => testing::Response::new(500),
        "/cut.xml.gz" => testing::Response::new(200).body(cut.clone()),
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert_eq!(crawl.errors(), 2);
}

#[tokio::test]
async fn a_crawl_tries_a_busy_sitemap_again() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let map = format!("<urlset><url><loc>{base}/listed</loc></url></urlset>");
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/sitemap.xml" if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
            testing::Response::new(503)
        }
        "/sitemap.xml" => testing::Response::new(200).body(map.clone()),
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert!(kept.contains("/listed"), "{kept:?}");
}

#[tokio::test]
async fn a_site_without_a_sitemap_is_not_an_error() {
    let server = testing::Server::http(|request| match request.target.as_str() {
        "/" => html("<p>home</p>"),
        _ => testing::Response::new(404),
    })
    .await
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    while let Some(page) = crawl.next().await {
        page.unwrap();
    }
    assert_eq!(crawl.errors(), 0);
}

#[test]
fn a_sitemap_retry_waiting_for_a_free_sitemap_slot_does_not_spin() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let kept = runtime.block_on(async {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let robots = format!(
                "User-agent: *\nSitemap: {base}/busy.xml\nSitemap: {base}/slow1.xml\nSitemap: {base}/slow2.xml\n"
            );
            let map = format!("<urlset><url><loc>{base}/listed</loc></url></urlset>");
            let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = calls.clone();
            let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
                "/robots.txt" => testing::Response::new(200).body(robots.clone()),
                "/busy.xml" if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
                    testing::Response::new(503)
                }
                "/busy.xml" => testing::Response::new(200).body(map.clone()),
                "/slow1.xml" | "/slow2.xml" => testing::Response::new(200)
                    .body("<urlset></urlset>")
                    .delay(Duration::from_secs(2)),
                path => html(&format!("<p>{path}</p>")),
            })
            .unwrap();
            let mut crawl = Session::new()
                .crawl(server.url("/"))
                .host_limits(leyline::HostLimits::new());
            let mut kept = BTreeSet::new();
            while let Some(page) = crawl.next().await {
                kept.insert(page.unwrap().url().path().to_owned());
            }
            kept
        });
        done.send(kept).unwrap();
    });
    let kept = finished.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(kept.contains("/listed"), "{kept:?}");
}

#[tokio::test]
async fn a_crawl_follows_a_sitemap_to_its_new_address() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let map = format!("<urlset><url><loc>{base}/orphan</loc></url></urlset>");
    let server = testing::Server::http_on(listener, move |request| match request.target.as_str() {
        "/sitemap.xml" => testing::Response::new(301).header("location", "/maps/real.xml"),
        "/maps/real.xml" => testing::Response::new(200).body(map.clone()),
        path => html(&format!("<p>{path}</p>")),
    })
    .unwrap();
    let mut crawl = Session::new()
        .crawl(server.url("/"))
        .host_limits(leyline::HostLimits::new());
    let mut kept = BTreeSet::new();
    while let Some(page) = crawl.next().await {
        kept.insert(page.unwrap().url().path().to_owned());
    }
    assert!(kept.contains("/orphan"), "{kept:?}");
}
