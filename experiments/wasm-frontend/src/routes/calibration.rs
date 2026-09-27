use apex_shared::CalibrationCurve;
use leptos::*;

use crate::api;
use crate::components::{
    cards::{PageHeader, SurfaceCard},
    charts::reliability_diagram::ReliabilityDiagram,
};

/// Returns an empty calibration curve when no live data is available.
/// The calibration endpoint (/api/admin/calibration) and WebSocket stream
/// (/ws/calibration) provide live data; this fallback is used only during
/// initial load and shows a clear "waiting for data" state.
fn sample_curve() -> CalibrationCurve {
    CalibrationCurve {
        points: vec![],
        brier_score: 0.0,
        reliability: 0.0,
        resolution: 0.0,
    }
}

#[component]
pub fn CalibrationPage() -> impl IntoView {
    let live_curve = create_rw_signal::<Option<CalibrationCurve>>(None);
    let curve = create_resource(|| (), |_| async { api::fetch_calibration_curve().await });

    create_effect(move |_| {
        if let Some(Ok(curve)) = curve.get() {
            live_curve.set(Some(curve));
        }
    });

    #[cfg(target_arch = "wasm32")]
    create_effect(move |_| {
        use wasm_bindgen::{closure::Closure, JsCast};
        use web_sys::{MessageEvent, WebSocket};

        let Some(window) = web_sys::window() else {
            return;
        };
        let protocol = window
            .location()
            .protocol()
            .ok()
            .unwrap_or_else(|| "http:".to_string());
        let host = window.location().host().ok().unwrap_or_default();
        let ws_scheme = if protocol == "https:" { "wss" } else { "ws" };
        let Ok(socket) = WebSocket::new(&format!("{ws_scheme}://{host}/ws/calibration")) else {
            return;
        };

        let onmessage_curve = live_curve;
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                if let Ok(curve) = serde_json::from_str::<CalibrationCurve>(&text) {
                    onmessage_curve.set(Some(curve));
                }
            }
        });
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
        std::mem::forget(socket);
    });

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Statistical Calibration"
                title="Reliability Diagram"
                subtitle="The calibration route loads resolved alert samples and refreshes from a live WebSocket stream."
            />

            <Suspense fallback=move || view! { <SurfaceCard title="Weekly Reliability" subtitle="Loading calibration curve."><p class="muted-copy">"Loading..."</p></SurfaceCard> }>
                {move || {
                    let rendered_curve = live_curve.get().unwrap_or_else(sample_curve);
                    view! {
                        <SurfaceCard title="Weekly Reliability" subtitle="Perfect calibration lies on the diagonal; point size tracks the bucket population.">
                            <ReliabilityDiagram curve=rendered_curve.clone() />
                            <div class="metric-row">
                                <span class="muted-copy">{format!("Brier {:.3}", rendered_curve.brier_score)}</span>
                                <span class="muted-copy">{format!("Reliability {:.3}", rendered_curve.reliability)}</span>
                                <span class="muted-copy">{format!("Resolution {:.3}", rendered_curve.resolution)}</span>
                            </div>
                        </SurfaceCard>
                    }
                }}
            </Suspense>
        </div>
    }
}
