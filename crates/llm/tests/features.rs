#[test]
fn experimental_only_capabilities_are_unavailable_without_flag() {
    assert!(!std::hint::black_box(
        apex_llm::EXPERIMENTAL_FEATURES_ENABLED
    ));
}
