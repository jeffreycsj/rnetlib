use rnet::{
    rnet_game_metrics_log_interval_set, rnet_game_runtime_create, rnet_game_runtime_destroy,
    rnet_game_runtime_stop, RnetGameConfig, RNET_E_INVALID_HANDLE, RNET_OK,
};

#[test]
fn game_log_interval_validates_handles_and_supports_disable_without_a_logger() {
    assert_eq!(
        rnet_game_metrics_log_interval_set(0, 1),
        RNET_E_INVALID_HANDLE
    );
    let mut runtime = 0;
    assert_eq!(
        unsafe {
            rnet_game_runtime_create(&RnetGameConfig::default(), std::ptr::null(), &mut runtime)
        },
        RNET_OK
    );
    assert_eq!(rnet_game_metrics_log_interval_set(runtime, 1), RNET_OK);
    assert_eq!(rnet_game_metrics_log_interval_set(runtime, 0), RNET_OK);
    assert_eq!(rnet_game_runtime_stop(runtime, 0), RNET_OK);
    assert_eq!(rnet_game_runtime_destroy(runtime), RNET_OK);
    assert_eq!(
        rnet_game_metrics_log_interval_set(runtime, 1),
        RNET_E_INVALID_HANDLE
    );
}
