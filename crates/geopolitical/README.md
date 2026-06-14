# Geopolitical Intelligence Module

A comprehensive geopolitical intelligence module for the ApexIntel OSINT platform, providing real-time analysis and monitoring of global geopolitical risks.

## Features

### 3.3.1 Sanctions Monitoring
- OFAC SDN (Specially Designated Nationals) list integration
- EU sanction list tracking
- UN sanction list monitoring
- Automated compliance alerts
- Name matching and screening
- False positive management

### 3.3.2 Trade Intelligence
- HS code classification and validation
- Tariff impact assessment
- Trade agreement monitoring
- Supply chain rerouting signals
- Import/export data analysis
- Trade restriction tracking

### 3.3.3 Political Risk Assessment
- Country stability scoring (0-1 scale)
- Policy change detection
- Conflict zone monitoring
- Infrastructure risk analysis
- Comprehensive risk assessment reports

### 3.3.4 Regulatory Intelligence
- Industry regulation tracking
- Environmental compliance monitoring
- Labor law change detection
- Tax policy updates
- Compliance deadline tracking

## Usage

```rust
use geopolitical::{GeopoliticalIntelligence, GeopoliticalConfig};

// Create configuration
let config = GeopoliticalConfig::default();

// Create intelligence client
let geo = GeopoliticalIntelligence::new(config);

// Sanctions screening
let sanctions = geo.sanctions();
let result = sanctions.screen(ScreeningRequest {
    name: Some("Test Entity".to_string()),
    threshold: 0.7,
    ..Default::default()
}).await?;

// Trade intelligence
let trade = geo.trade();
let tariff = trade.lookup_tariff("847130", &CountryCode::new("US"), &CountryCode::new("CN")).await?;

// Political risk assessment
let political = geo.political_risk();
let assessment = political.assess_risk(RiskAssessmentRequest {
    country: CountryCode::new("US"),
    ..Default::default()
}).await?;

// Regulatory intelligence
let regulatory = geo.regulatory();
let report = regulatory.generate_compliance_report("Acme Corp", &CountryCode::new("US"));
```

## Error Handling

All operations return `Result<T, GeopoliticalError>` for proper error handling:

```rust
use geopolitical::GeopoliticalError;

match result {
    Ok(data) => { /* use data */ }
    Err(e) => {
        eprintln!("Error: {} (code: {})", e, e.error_code());
        if e.is_retryable() {
            // Retry logic
        }
    }
}
```

## Testing

Run tests with:

```bash
cargo test -p geopolitical
```

Run with verbose output:

```bash
cargo test -p geopolitical -- --nocapture
```

## License

MIT
