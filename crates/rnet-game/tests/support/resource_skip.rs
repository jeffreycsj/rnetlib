//! Local smoke tests may skip under memory pressure; required CI coverage must fail.

pub fn report(reason: &str) {
    report_with_policy(
        std::env::var_os("RNET_REQUIRE_RESOURCE_TESTS").is_some(),
        reason,
    );
}

fn report_with_policy(required: bool, reason: &str) {
    assert!(!required, "required resource test could not run: {reason}");
    eprintln!("{reason}");
}

#[test]
fn local_resource_refusal_remains_skippable() {
    report_with_policy(false, "synthetic local resource refusal");
}

#[test]
#[should_panic(expected = "required resource test could not run")]
fn required_resource_refusal_is_a_test_failure() {
    report_with_policy(true, "synthetic required resource refusal");
}
