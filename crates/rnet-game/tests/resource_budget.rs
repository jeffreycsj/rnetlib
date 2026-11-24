//! The ordinary CI allowance must never change local or release defaults.
#[path = "../src/bin/support/resource_budget.rs"]
mod resource_budget;

#[test]
fn current_budget_has_distinct_startup_headroom() {
    assert_eq!(
        resource_budget::minimum_gib(true) - resource_budget::minimum_gib(false),
        4
    );
}

#[test]
fn ordinary_ci_accepts_the_observed_host_but_keeps_a_running_reserve() {
    let available_gib = 14;
    let minimum = resource_budget::select_minimum_gib;
    assert!(available_gib >= minimum(true, Some("github-ci"), Some("true"), Some("ci"), false));
    assert_eq!(
        minimum(true, Some("github-ci"), Some("true"), Some("ci"), false),
        6
    );
    assert_eq!(
        minimum(false, Some("github-ci"), Some("true"), Some("ci"), false),
        2
    );
    // Missing the opt-in must retain the original refusal on this same machine.
    assert!(available_gib < minimum(true, None, Some("true"), Some("ci"), false));
}

#[test]
fn local_unknown_and_release_contexts_keep_the_strict_budget() {
    for (profile, actions, workflow, evidence) in [
        (None, None, None, false),
        (Some("github-ci"), None, None, false),
        (Some("github-ci"), Some("false"), Some("ci"), false),
        (Some("unknown"), Some("true"), Some("ci"), false),
        (Some("strict"), Some("true"), Some("ci"), false),
        (
            Some("github-ci"),
            Some("true"),
            Some("release-qualification"),
            false,
        ),
        (Some("github-ci"), Some("true"), Some("ci"), true),
    ] {
        assert_eq!(
            resource_budget::select_minimum_gib(true, profile, actions, workflow, evidence),
            19
        );
        assert_eq!(
            resource_budget::select_minimum_gib(false, profile, actions, workflow, evidence),
            15
        );
    }
}
