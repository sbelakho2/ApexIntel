/// All statistical computations assume Rust's default IEEE 754 `roundTiesToEven`
/// semantics on `f64` operations. Reproducibility-sensitive tests should compare
/// floating-point outputs with a tolerance rather than exact equality.
pub const IEEE_754_ROUNDING_MODE: &str = "IEEE 754 roundTiesToEven (nearest, ties to even)";

pub mod anomaly;
pub mod bayesian;
pub mod calibration;
pub mod changepoint;
pub mod correlation;
pub mod fdr;
pub mod fisher;
pub mod graph_risk;
pub mod granger;
pub mod hazard;
pub mod mutual_info;
pub mod observation_anomaly;
pub mod pipeline;
pub mod utils;
