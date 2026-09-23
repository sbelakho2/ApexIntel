/// Only meaningful when the `experimental` feature is not compiled in; under
/// `--all-features` the experimental capabilities are intentionally enabled.
#[cfg(not(feature = "experimental"))]
#[test]
fn experimental_only_capabilities_are_unavailable_without_flag() {
    assert!(!std::hint::black_box(
        apex_llm::EXPERIMENTAL_FEATURES_ENABLED
    ));
}
