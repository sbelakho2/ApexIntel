#[test]
fn experimental_only_capabilities_are_unavailable_without_flag() {
    assert!(!apex_llm::EXPERIMENTAL_FEATURES_ENABLED);
}