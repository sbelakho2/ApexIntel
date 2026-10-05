//! Inspect a saved r.jina.ai response with the supplier-pricing parsers.
//! Usage: cargo run -p apex-crawl --example supplier_probe -- <file> <alibaba|1688|lcsc|baidu>
use apex_crawl::supplier_pricing::{parse_price_offers, parse_search_page, PricingSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: supplier_probe <file> <source>");
        std::process::exit(2);
    };
    let Some(source) = PricingSource::from_str(&args.next().unwrap_or_else(|| "lcsc".to_string()))
    else {
        eprintln!("source must be alibaba|1688|lcsc|baidu");
        std::process::exit(2);
    };
    let markdown = match std::fs::read_to_string(&path) {
        Ok(markdown) => markdown,
        Err(error) => {
            eprintln!("cannot read {path}: {error}");
            std::process::exit(2);
        }
    };
    let links: Vec<String> = {
        let mut links = Vec::new();
        for line in markdown.lines() {
            links.extend(apex_crawl::supplier_pricing::extract_listing_links(
                line, source,
            ));
        }
        links
    };
    println!("listing links: {}", links.len());
    for link in links.iter().take(5) {
        println!("  {link}");
    }
    let blocks = parse_search_page(&markdown, source);
    println!("search blocks: {}", blocks.len());
    for block in blocks.iter().take(5) {
        println!(
            "  url={} in_stock={} offers={}",
            block.listing_url.as_deref().unwrap_or("none"),
            block.in_stock,
            block.offers.len()
        );
        for offer in block.offers.iter().take(4) {
            println!(
                "    {} {} moq={} max={:?} teaser={} raw={:?}",
                offer.unit_price, offer.currency, offer.moq, offer.max_qty, offer.teaser, offer.raw
            );
        }
    }
    let all = parse_price_offers(&markdown);
    println!("total parsed offers: {}", all.len());
    for offer in all.iter().take(8) {
        println!(
            "  {} {} moq={} max={:?} teaser={} raw={:?}",
            offer.unit_price, offer.currency, offer.moq, offer.max_qty, offer.teaser, offer.raw
        );
    }
}
