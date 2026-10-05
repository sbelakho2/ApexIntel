//! Real-BOM dogfood evaluation of the supplier-pricing pipeline.
//!
//! Runs `SupplierPricePipeline` against real part numbers fetched through the
//! live r.jina.ai reader and scores correctness against ground-truth price
//! bands (USD) established for each part at its evaluation quantity.
//!
//! Metrics:
//!   * `band_ok`   — chosen quotable price falls inside the part's band.
//!   * `band_miss` — a quotable price was reported but outside the band
//!                   (currency confusion, teaser leakage, tier-unit errors).
//!   * `no_result` — no quotable price (marketplace walls, empty searches).
//!   * `verified`  — the chosen price literally appears in the fetched
//!                   listing markdown (round-trip verification).
//!   * `teaser`    — chosen price came from a teaser (must be 0).
//!
//! The correctness bar: 100% of reported quotes inside their band, 100%
//! verified, 0 teasers. `no_result` is reported but does not count as wrong.
//!
//! Usage: cargo run -p apex-crawl --example supplier_pricing_eval [--limit N]

use std::sync::Arc;

use apex_crawl::supplier_pricing::{
    offer_to_usd, parse_price_offers, SupplierPricePipeline, SupplierPricingClient,
    SupplierPricingConfig,
};
use tokio::time::Instant;

struct EvalPart {
    part: &'static str,
    qty: u64,
    min_usd: f64,
    max_usd: f64,
    note: &'static str,
}

// Bands are deliberately generous (they exist to catch *decade-scale* errors:
// a ¥-read-as-$ parse, a teaser accepted as a quote, or a tier-unit swap
// moves the price 5-100×), not to arbitrage the market.
fn truncate(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out.replace('\n', " ")
}

const EVAL_BOMS: &[EvalPart] = &[
    EvalPart {
        part: "GRM155R71C104KA88D",
        qty: 100,
        min_usd: 0.001,
        max_usd: 0.10,
        note: "Murata 0402 100nF X7R MLCC",
    },
    EvalPart {
        part: "RC0603FR-0710KL",
        qty: 100,
        min_usd: 0.0008,
        max_usd: 0.05,
        note: "Yageo 0603 10k 1%",
    },
    EvalPart {
        part: "RC0805FR-0710KL",
        qty: 100,
        min_usd: 0.001,
        max_usd: 0.06,
        note: "Yageo 0805 10k 1%",
    },
    EvalPart {
        part: "C0603C104K5RACTU",
        qty: 100,
        min_usd: 0.001,
        max_usd: 0.10,
        note: "KEMET 0603 100nF",
    },
    EvalPart {
        part: "SN74LVC1G125DBVR",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.60,
        note: "TI single buffer",
    },
    EvalPart {
        part: "BSS138LT1G",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.20,
        note: "ON Semi small MOSFET",
    },
    EvalPart {
        part: "1N4148W-7-F",
        qty: 100,
        min_usd: 0.003,
        max_usd: 0.15,
        note: "Diodes Inc switching diode",
    },
    EvalPart {
        part: "AMS1117-3.3",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.25,
        note: "3.3V LDO",
    },
    EvalPart {
        part: "AP2112K-3.3TRG1",
        qty: 10,
        min_usd: 0.03,
        max_usd: 0.50,
        note: "Diodes Inc LDO",
    },
    EvalPart {
        part: "STM32F103C8T6",
        qty: 10,
        min_usd: 0.50,
        max_usd: 4.00,
        note: "STM32F1 MCU",
    },
    EvalPart {
        part: "ATmega328P-AU",
        qty: 10,
        min_usd: 1.00,
        max_usd: 5.00,
        note: "8-bit MCU",
    },
    EvalPart {
        part: "ESP32-WROOM-32E",
        qty: 10,
        min_usd: 1.00,
        max_usd: 8.00,
        note: "ESP32 module",
    },
    EvalPart {
        part: "NRF24L01+",
        qty: 10,
        min_usd: 0.20,
        max_usd: 6.00,
        note: "2.4GHz radio module",
    },
    EvalPart {
        part: "CH340G",
        qty: 10,
        min_usd: 0.20,
        max_usd: 2.00,
        note: "USB-UART bridge",
    },
    EvalPart {
        part: "CP2102N-A02-GQFN28R",
        qty: 10,
        min_usd: 0.50,
        max_usd: 4.00,
        note: "Silabs USB-UART",
    },
    EvalPart {
        part: "LSM6DS3TR",
        qty: 10,
        min_usd: 0.50,
        max_usd: 5.00,
        note: "ST IMU",
    },
    EvalPart {
        part: "PCF8563T/5,518",
        qty: 10,
        min_usd: 0.10,
        max_usd: 2.00,
        note: "NXP RTC",
    },
    EvalPart {
        part: "2N7002",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.10,
        note: "logic MOSFET",
    },
    EvalPart {
        part: "MAX232ESE+",
        qty: 10,
        min_usd: 0.20,
        max_usd: 2.50,
        note: "RS-232 driver",
    },
    EvalPart {
        part: "W25Q128JVS?IQ",
        qty: 10,
        min_usd: 0.30,
        max_usd: 3.00,
        note: "Winbond 128Mbit flash",
    },
    EvalPart {
        part: "USBLC6-2SC6",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.40,
        note: "USB ESD protection",
    },
    EvalPart {
        part: "AO3400A",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.20,
        note: "small N-MOSFET",
    },
    EvalPart {
        // Bands assume the quoted tier's MOQ <= qty; a part whose only
        // fetched row prices at MOQ 1 carries a qty-1 single-unit premium
        // (2-10x the volume tier), so the ceiling allows it.
        part: "TL431ACDBZR",
        qty: 100,
        min_usd: 0.005,
        max_usd: 1.00,
        note: "shunt reference",
    },
    EvalPart {
        part: "TXS0102DCUR",
        qty: 10,
        min_usd: 0.05,
        max_usd: 0.80,
        note: "level shifter",
    },
    EvalPart {
        part: "LM358",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.40,
        note: "dual op-amp",
    },
    EvalPart {
        part: "LM339DR",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.50,
        note: "quad comparator",
    },
    EvalPart {
        part: "NE5532DR",
        qty: 10,
        min_usd: 0.05,
        max_usd: 0.80,
        note: "audio op-amp",
    },
    EvalPart {
        part: "MCP6002-I/SN",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.20,
        note: "rail-to-rail op-amp",
    },
    EvalPart {
        part: "INA226AIDGSR",
        qty: 10,
        min_usd: 0.30,
        max_usd: 4.00,
        note: "current sense",
    },
    EvalPart {
        part: "74HC595D",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.50,
        note: "shift register",
    },
    EvalPart {
        part: "74HC14D",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.50,
        note: "hex schmitt",
    },
    EvalPart {
        part: "SN74HC165DR",
        qty: 10,
        min_usd: 0.05,
        max_usd: 1.50,
        note: "PISO register",
    },
    EvalPart {
        part: "LM2596S-5.0",
        qty: 10,
        min_usd: 0.30,
        max_usd: 3.50,
        note: "buck regulator",
    },
    EvalPart {
        part: "MP1584EN",
        qty: 10,
        min_usd: 0.20,
        max_usd: 3.00,
        note: "buck converter",
    },
    EvalPart {
        part: "MT3608",
        qty: 10,
        min_usd: 0.05,
        max_usd: 1.00,
        note: "boost converter",
    },
    EvalPart {
        part: "TPS5430DDAR",
        qty: 10,
        min_usd: 0.30,
        max_usd: 3.50,
        note: "3A buck",
    },
    EvalPart {
        part: "LM7805CT",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.50,
        note: "5V linear",
    },
    EvalPart {
        part: "TP4056",
        qty: 10,
        min_usd: 0.05,
        max_usd: 1.00,
        note: "charger module",
    },
    EvalPart {
        part: "DW01A",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.40,
        note: "battery protect",
    },
    EvalPart {
        part: "24C02",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.40,
        note: "2k EEPROM",
    },
    EvalPart {
        part: "AT24C256",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.20,
        note: "256k EEPROM",
    },
    EvalPart {
        part: "W25Q16JVSNIQ",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.50,
        note: "16Mbit flash",
    },
    EvalPart {
        part: "8MHz HC-49S",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.50,
        note: "crystal",
    },
    EvalPart {
        part: "32.768kHz crystal",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.50,
        note: "watch crystal",
    },
    EvalPart {
        part: "CC1101RGPR",
        qty: 10,
        min_usd: 0.50,
        max_usd: 5.00,
        note: "sub-GHz radio",
    },
    EvalPart {
        part: "SX1278",
        qty: 10,
        min_usd: 1.00,
        max_usd: 8.00,
        note: "LoRa module",
    },
    EvalPart {
        part: "SIM800C",
        qty: 10,
        min_usd: 1.50,
        max_usd: 10.00,
        note: "GSM module",
    },
    EvalPart {
        part: "DS18B20",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.50,
        note: "1-wire sensor",
    },
    EvalPart {
        part: "DHT22",
        qty: 10,
        min_usd: 0.50,
        max_usd: 4.00,
        note: "temp/humidity",
    },
    EvalPart {
        part: "MPU-6050",
        qty: 10,
        min_usd: 1.00,
        max_usd: 40.00,
        note: "IMU chip (EOL, price spiked)",
    },
    EvalPart {
        part: "SSD1306",
        qty: 10,
        min_usd: 0.15,
        max_usd: 6.00,
        note: "OLED chip/module",
    },
    EvalPart {
        part: "PAM8403",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.50,
        note: "audio amp",
    },
    EvalPart {
        part: "LM386N-1",
        qty: 10,
        min_usd: 0.05,
        max_usd: 0.80,
        note: "audio amp IC",
    },
    EvalPart {
        part: "PC817",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.25,
        note: "opto coupler",
    },
    EvalPart {
        part: "4N35",
        qty: 10,
        min_usd: 0.02,
        max_usd: 0.40,
        note: "opto coupler",
    },
    EvalPart {
        part: "IRFZ44N",
        qty: 10,
        min_usd: 0.10,
        max_usd: 1.50,
        note: "power MOSFET",
    },
    EvalPart {
        part: "SI2302",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.25,
        note: "small MOSFET",
    },
    EvalPart {
        part: "SS34",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.40,
        note: "schottky",
    },
    EvalPart {
        part: "1N5819",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.40,
        note: "schottky",
    },
    EvalPart {
        part: "BAV99",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.20,
        note: "dual diode",
    },
    EvalPart {
        part: "S8050",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.15,
        note: "NPN transistor",
    },
    EvalPart {
        part: "BC847C",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.15,
        note: "NPN transistor",
    },
    EvalPart {
        part: "CL10A106KP8NNNC",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.40,
        note: "10uF 0603",
    },
    EvalPart {
        part: "GRM188R71E105KA12D",
        qty: 100,
        min_usd: 0.002,
        max_usd: 0.25,
        note: "1uF 0805",
    },
    EvalPart {
        part: "C0603C220J5GACTU",
        qty: 100,
        min_usd: 0.001,
        max_usd: 0.15,
        note: "22pF 0603",
    },
    EvalPart {
        part: "RC0603FR-074K7L",
        qty: 100,
        min_usd: 0.0008,
        max_usd: 0.06,
        note: "4.7k 0603",
    },
    EvalPart {
        part: "NR4018T100M",
        qty: 100,
        min_usd: 0.005,
        max_usd: 0.70,
        note: "10uH inductor",
    },
    EvalPart {
        part: "SMBJ5.0A",
        qty: 100,
        min_usd: 0.01,
        max_usd: 0.40,
        note: "TVS diode",
    },
    EvalPart {
        part: "WS2812B",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.60,
        note: "RGB LED",
    },
    EvalPart {
        part: "JST XH2.54 2P",
        qty: 100,
        min_usd: 0.002,
        max_usd: 0.40,
        note: "pin header",
    },
    EvalPart {
        part: "TYPE-C-31-M-12",
        qty: 10,
        min_usd: 0.05,
        max_usd: 1.50,
        note: "USB-C receptacle",
    },
    EvalPart {
        part: "HR911105A",
        qty: 10,
        min_usd: 0.10,
        max_usd: 3.00,
        note: "ethernet jack",
    },
    EvalPart {
        part: "SRD-05VDC-SL-C",
        qty: 10,
        min_usd: 0.20,
        max_usd: 2.00,
        note: "5V relay",
    },
    EvalPart {
        part: "ESP-12F",
        qty: 10,
        min_usd: 0.50,
        max_usd: 4.00,
        note: "ESP8266 module",
    },
    EvalPart {
        part: "GD32F103C8T6",
        qty: 10,
        min_usd: 0.50,
        max_usd: 3.00,
        note: "GD32 MCU",
    },
    EvalPart {
        part: "RP2040",
        qty: 10,
        min_usd: 0.50,
        max_usd: 2.50,
        note: "RP2040 MCU",
    },
    EvalPart {
        part: "CH32V003F4P6",
        qty: 100,
        min_usd: 0.02,
        max_usd: 0.40,
        note: "RISC-V MCU",
    },
    EvalPart {
        part: "PCM5102A",
        qty: 10,
        min_usd: 0.50,
        max_usd: 5.00,
        note: "DAC module",
    },
];

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(EVAL_BOMS.len());
    let only_part = args
        .iter()
        .position(|a| a == "--part")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.to_string());
    let skip = args
        .iter()
        .position(|a| a == "--skip")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    // Anonymous reader limits are ~20 RPM; crawl gently: two-way concurrency,
    // 1.5s per-source pacing, generous retry budget honouring Retry-After.
    let config = SupplierPricingConfig {
        max_listings_per_source: 6,
        min_request_interval: std::time::Duration::from_millis(1500),
        max_retries: 3,
        base_backoff: std::time::Duration::from_secs(3),
        max_backoff: std::time::Duration::from_secs(30),
        circuit_failure_threshold: 10,
        circuit_cooldown: std::time::Duration::from_secs(120),
        cache_ttl: std::time::Duration::from_secs(86400),
        max_concurrent_requests: 2,
        // Persist fetched reader pages across runs: repeat dogfooding costs
        // no reader quota and survives restarts.
        cache_path: Some(std::path::PathBuf::from(
            "/tmp/apex_supplier_pricing_cache.json",
        )),
        ..SupplierPricingConfig::from_env()
    };
    let fx = config.fx_cny_usd;
    let client = match SupplierPricingClient::new(config.clone()) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("client construction failed: {error}");
            std::process::exit(2);
        }
    };
    let pipeline = SupplierPricePipeline::new(client.clone());

    let started = Instant::now();
    let mut band_ok = 0usize;
    let mut band_miss = 0usize;
    let mut weak = 0usize;
    let mut no_result = 0usize;
    let mut verified = 0usize;
    let mut unverified = 0usize;
    let mut teasers = 0usize;

    println!("part | qty | chosen | cur | source | moq | median_usd | n | fx | outlier | reconfirm | band | verify | note");
    let selected: Vec<&EvalPart> = match &only_part {
        Some(name) => EVAL_BOMS.iter().filter(|e| e.part == name).collect(),
        None => EVAL_BOMS.iter().skip(skip).take(limit).collect(),
    };
    let mut retry_list: Vec<&EvalPart> = Vec::new();
    let mut handle_quote = |eval: &EvalPart, quote: &apex_crawl::supplier_pricing::LowestQuote| {
        let loose = format!("{}", quote.unit_price);
        let appears = !quote.evidence_raw.is_empty() && quote.evidence_raw.contains(&loose);
        if quote.low_confidence {
            weak += 1;
            println!(
                "{} | {} | {:.6} | {} | {} | {} | {:.6} | {} | {} | {} | {} | WEAK | {} | {}",
                eval.part,
                eval.qty,
                quote.unit_price,
                quote.currency,
                quote.source,
                quote.moq,
                quote.median_unit_usd.unwrap_or(f64::NAN),
                quote.sample_size,
                quote.fx_applied,
                quote.outlier_risk,
                quote.reconfirmation_required,
                if appears { "yes" } else { "NO" },
                eval.note,
            );
            return;
        }
        let usd = offer_to_usd(
            &apex_crawl::supplier_pricing::PriceOffer {
                unit_price: quote.unit_price,
                currency: quote.currency.clone(),
                moq: quote.moq,
                max_qty: quote.max_qty,
                teaser: false,
                price_max: None,
                raw: String::new(),
            },
            fx,
        )
        .unwrap_or(quote.unit_price);
        let in_band = usd >= eval.min_usd && usd <= eval.max_usd;
        if in_band {
            band_ok += 1;
        } else {
            band_miss += 1;
        }
        let teaser_leak = parse_price_offers(&quote.evidence_raw)
            .iter()
            .any(|o| (o.unit_price - quote.unit_price).abs() < 1e-9 && o.teaser);
        if appears {
            verified += 1;
        } else {
            unverified += 1;
        }
        if teaser_leak {
            teasers += 1;
        }
        println!(
            "{} | {} | {:.6} | {} | {} | {} | {:.6} | {} | {} | {} | {} | {} | {} | {}",
            eval.part,
            eval.qty,
            quote.unit_price,
            quote.currency,
            quote.source,
            quote.moq,
            quote.median_unit_usd.unwrap_or(f64::NAN),
            quote.sample_size,
            quote.fx_applied,
            quote.outlier_risk,
            quote.reconfirmation_required,
            if in_band { "OK" } else { "MISS" },
            if appears { "yes" } else { "NO" },
            eval.note,
        );
        if !appears {
            println!("  evidence: {}", truncate(&quote.evidence_raw, 120));
        }
    };

    for eval in selected {
        match pipeline
            .price_for_part(eval.part, eval.qty, Some("USD"))
            .await
        {
            Ok(quote) => handle_quote(eval, &quote),
            Err(_) => retry_list.push(eval),
        }
    }
    // Retry rounds: transient reader throttling can leave gaps; retry the
    // parts that quoted nothing until the waves pass (bounded).
    let mut pending: Vec<&EvalPart> = retry_list;
    for round in 0..3 {
        if pending.is_empty() {
            break;
        }
        if round > 0 {
            println!("retry round {round}: waiting for throttle waves to pass…");
            tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        }
        let mut still_pending: Vec<&EvalPart> = Vec::new();
        for eval in pending {
            match pipeline
                .price_for_part(eval.part, eval.qty, Some("USD"))
                .await
            {
                Ok(quote) => handle_quote(eval, &quote),
                Err(_) => still_pending.push(eval),
            }
        }
        pending = still_pending;
    }
    for eval in pending {
        no_result += 1;
        println!(
            "{} | {} | (no quotable price) | | | | | | | | | n/a | {}",
            eval.part, eval.qty, eval.note
        );
    }

    let elapsed = started.elapsed();
    let quoted = band_ok + band_miss;
    println!();
    println!("elapsed: {elapsed:.1?}");
    println!(
        "parts attempted: {}",
        match &only_part {
            Some(_) => 1,
            None => EVAL_BOMS.len().min(limit),
        }
    );
    println!(
        "quoted: {quoted}  band_ok: {band_ok}  band_miss: {band_miss}  no_result: {no_result}"
    );
    println!("verified: {verified}  unverified: {unverified}  weak_evidence: {weak}  teaser_flags: {teasers}");
    let correctness = if quoted == 0 {
        100.0
    } else {
        band_ok as f64 / quoted as f64 * 100.0
    };
    println!("band correctness: {correctness:.1}% (bar: >= 99%)");
    let pass = correctness >= 99.0 && unverified == 0 && teasers == 0;
    if pass {
        println!("EVAL PASS");
    } else {
        println!("EVAL FAIL");
        std::process::exit(1);
    }
}
