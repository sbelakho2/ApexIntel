use leptos::*;

fn clamp_probability(probability: f64) -> f64 {
    probability.clamp(0.0, 1.0)
}

fn gauge_color(probability: f64, threshold_high: f64, threshold_medium: f64) -> &'static str {
    if probability >= threshold_high {
        "var(--destructive)"
    } else if probability >= threshold_medium {
        "var(--warning)"
    } else {
        "var(--success)"
    }
}

fn polar_to_cartesian(angle_deg: f64, radius: f64) -> (f64, f64) {
    let radians = angle_deg.to_radians();
    (100.0 + radius * radians.cos(), 100.0 - radius * radians.sin())
}

fn arc_path(probability: f64) -> String {
    let clamped = clamp_probability(probability);
    let angle = 180.0 - (180.0 * clamped);
    let (x, y) = polar_to_cartesian(angle, 80.0);
    format!("M 20 100 A 80 80 0 0 1 {x:.2} {y:.2}")
}

fn arc_segment_path(start_angle: f64, end_angle: f64) -> String {
    let (start_x, start_y) = polar_to_cartesian(start_angle, 80.0);
    let (end_x, end_y) = polar_to_cartesian(end_angle, 80.0);
    format!("M {start_x:.2} {start_y:.2} A 80 80 0 0 1 {end_x:.2} {end_y:.2}")
}

#[component]
pub fn ProbabilityGauge(
    probability: f64,
    #[prop(optional)] threshold_high: Option<f64>,
    #[prop(optional)] threshold_medium: Option<f64>,
) -> impl IntoView {
    let high = threshold_high.unwrap_or(0.7);
    let medium = threshold_medium.unwrap_or(0.3);
    let clamped = clamp_probability(probability);
    let needle_angle = 180.0 - (180.0 * clamped);
    let (needle_x, needle_y) = polar_to_cartesian(needle_angle, 58.0);
    let color = gauge_color(clamped, high, medium);
    let arc = arc_path(clamped);
    let safe_arc = arc_segment_path(180.0, 126.0);
    let watch_arc = arc_segment_path(126.0, 54.0);
    let risk_arc = arc_segment_path(54.0, 0.0);

    view! {
        <svg viewBox="0 0 200 120" class="probability-gauge" aria-label="Calibrated event probability gauge">
            <path d="M 20 100 A 80 80 0 0 1 180 100" fill="none" stroke="var(--chart-grid-soft)" stroke-width="16" />
            <path d=safe_arc fill="none" stroke="rgba(45, 140, 60, 0.34)" stroke-width="16" stroke-linecap="round" />
            <path d=watch_arc fill="none" stroke="rgba(255, 190, 0, 0.34)" stroke-width="16" stroke-linecap="round" />
            <path d=risk_arc fill="none" stroke="rgba(214, 45, 45, 0.34)" stroke-width="16" stroke-linecap="round" />
            <path d=arc fill="none" stroke=color stroke-width="16" stroke-linecap="round" />
            <line x1="100" y1="100" x2=format!("{needle_x:.2}") y2=format!("{needle_y:.2}") stroke="var(--foreground)" stroke-width="3" />
            <circle cx="100" cy="100" r="5" fill="var(--foreground)" />
            <text x="100" y="84" text-anchor="middle" font-size="24" font-weight="800">{format!("{:.0}%", clamped * 100.0)}</text>
            <text x="24" y="112" font-size="10" font-weight="800" fill="var(--chart-label)">"Low"</text>
            <text x="100" y="24" text-anchor="middle" font-size="10" font-weight="800" fill="var(--chart-label)">"Watch"</text>
            <text x="176" y="112" text-anchor="end" font-size="10" font-weight="800" fill="var(--chart-label)">"High"</text>
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_thresholds_map_to_expected_colors() {
        assert_eq!(gauge_color(0.82, 0.7, 0.3), "var(--destructive)");
        assert_eq!(gauge_color(0.45, 0.7, 0.3), "var(--warning)");
        assert_eq!(gauge_color(0.1, 0.7, 0.3), "var(--success)");
    }

    #[test]
    fn probability_is_clamped_before_render_math() {
        assert_eq!(clamp_probability(-0.5), 0.0);
        assert_eq!(clamp_probability(1.8), 1.0);
    }
}