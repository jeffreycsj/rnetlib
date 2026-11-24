//! Resource policy for development probes, not a production runtime setting.

/// Minimum available GiB for startup or an ongoing probe memory check.
/// The lower budget requires an explicit ordinary GitHub CI context. Unknown
/// profiles, local runs and release evidence always retain the strict defaults.
pub fn minimum_gib(startup: bool) -> u64 {
    select_minimum_gib(
        startup,
        std::env::var("RNET_RESOURCE_PROFILE").ok().as_deref(),
        std::env::var("GITHUB_ACTIONS").ok().as_deref(),
        std::env::var("GITHUB_WORKFLOW").ok().as_deref(),
        std::env::var_os("RNET_EVIDENCE_RUN_ID").is_some(),
    )
}

/// Pure policy selection keeps environment edge cases testable without mutating
/// process-wide variables while Rust tests run concurrently.
pub fn select_minimum_gib(
    startup: bool,
    profile: Option<&str>,
    github_actions: Option<&str>,
    workflow: Option<&str>,
    release_evidence: bool,
) -> u64 {
    let ordinary_ci = profile == Some("github-ci")
        && github_actions == Some("true")
        && workflow == Some("ci")
        && !release_evidence;
    match (ordinary_ci, startup) {
        (true, true) => 6,
        (true, false) => 2,
        (false, true) => 19,
        (false, false) => 15,
    }
}
