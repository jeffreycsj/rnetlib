#[test]
fn telemetry_has_one_current_layout_and_export_per_operation() {
    let header = include_str!("../../../include/rnet.h");
    let exports = include_str!("../../../scripts/expected-exports.txt");
    for obsolete in [
        "rnet_logger_v2",
        "rnet_log_v2_fn",
        "rnet_metrics_v2",
        "rnet_metrics_v3",
        "rnet_latency_metric_v2",
        "rnet_metrics_snapshot_v2",
        "rnet_metrics_snapshot_v3",
        "rnet_latency_snapshot_v2",
    ] {
        assert!(
            !header.contains(obsolete),
            "obsolete public layout/function: {obsolete}"
        );
        assert!(
            !exports.lines().any(|name| name == obsolete),
            "obsolete export: {obsolete}"
        );
    }
    for current in ["rnet_metrics_snapshot", "rnet_latency_snapshot"] {
        assert_eq!(exports.lines().filter(|name| *name == current).count(), 1);
    }
    assert!(header.contains("p999_us"));
    assert!(header.contains("admission_rejected_by_reason"));
}

#[test]
fn application_send_and_event_contracts_do_not_expose_message_types_or_streams() {
    let header = include_str!("../../../include/rnet.h");
    for obsolete in [
        "uint32_t msg_type",
        "uint32_t stream_id",
        "uint64_t request_id",
        "int32_t rnet_send(",
    ] {
        assert!(
            !header.contains(obsolete),
            "legacy framing exposed in public ABI: {obsolete}"
        );
    }
}

#[test]
fn endpoint_and_poll_symbols_are_current_only() {
    let header = include_str!("../../../include/rnet.h");
    for obsolete in [
        "rnet_server_config_v2",
        "rnet_client_config_v2",
        "rnet_server_open_v2",
        "rnet_client_connect_v2",
        "rnet_poll_events_ex",
        "size_t rnet_poll_events(",
    ] {
        assert!(
            !header.contains(obsolete),
            "obsolete endpoint/poll API: {obsolete}"
        );
    }
}

#[test]
fn game_runtime_and_poll_have_one_complete_layout() {
    let header = include_str!("../../../include/rnet.h");
    for obsolete in [
        "rnet_game_config_v2",
        "rnet_game_event_v2",
        "rnet_game_runtime_create_logged",
        "rnet_game_runtime_create_v2",
        "rnet_game_poll_events_v2",
    ] {
        assert!(!header.contains(obsolete), "obsolete game API: {obsolete}");
    }
}

#[test]
fn runtime_configuration_has_no_compatibility_layout_or_constructor() {
    let header = include_str!("../../../include/rnet.h");
    for obsolete in [
        "rnet_config_v3",
        "rnet_config_v4",
        "rnet_config_v5",
        "rnet_runtime_create_v2",
        "rnet_runtime_create_v3",
        "rnet_runtime_create_v4",
        "rnet_runtime_create_v5",
        "logger_v2",
    ] {
        assert!(
            !header.contains(obsolete),
            "obsolete runtime API: {obsolete}"
        );
    }
}
