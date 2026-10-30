use leyline::{Browser, Session};

const IN_STOCK: &[u8] = b"\"availability\":\"InStock\"";
const OUT_OF_STOCK: &[u8] = b"\"availability\":\"OutOfStock\"";
const PAGE_LIMIT: usize = 512 * 1024;

enum Availability {
    InStock,
    OutOfStock,
    Unknown,
}

fn contains(body: &[u8], from: usize, marker: &[u8]) -> bool {
    let start = from.saturating_sub(marker.len() - 1);
    body[start..].windows(marker.len()).any(|w| w == marker)
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com/".to_string());
    let session = Session::from(Browser::default());

    let (page, matched) = session
        .get(&url)
        .error_for_status()
        .stream()
        .send()
        .await?
        .read_until(PAGE_LIMIT, |body, from| {
            contains(body, from, IN_STOCK) || contains(body, from, OUT_OF_STOCK)
        })
        .await?;

    let availability = match matched {
        true if contains(&page, 0, IN_STOCK) => Availability::InStock,
        true => Availability::OutOfStock,
        false => Availability::Unknown,
    };
    match availability {
        Availability::InStock => println!("{url}: in stock"),
        Availability::OutOfStock => println!("{url}: out of stock"),
        Availability::Unknown => {
            println!("{url}: unknown, no marker in {} decoded bytes", page.len())
        }
    }
    Ok(())
}
