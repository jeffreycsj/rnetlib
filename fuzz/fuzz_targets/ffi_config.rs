#![no_main]

use libfuzzer_sys::fuzz_target;
use rnet::{
    rnet_config_v4_init, rnet_runtime_create_v4, rnet_runtime_destroy, rnet_runtime_stop,
    RnetConfigV4, RNET_OK,
};

fuzz_target!(|data: &[u8]| {
    let mut config = RnetConfigV4::default();
    let _ = unsafe { rnet_config_v4_init(&mut config) };
    config.worker_threads = 1;
    if let Some(selector) = data.first().filter(|selector| **selector != u8::MAX) {
        match selector % 8 {
            0 => config.struct_size = data.get(1).copied().map_or(0, u32::from),
            1 => config.abi_version = data.get(1).copied().map_or(0, u32::from),
            2 => config.worker_threads = data.get(1).copied().map_or(0, u32::from),
            3 => config.event_queue_capacity = data.get(1).copied().map_or(0, u32::from),
            4 => config.max_body_len = data.get(1).copied().map_or(0, u32::from),
            5 => config.handshake_timeout_ms = data.get(1).copied().map_or(0, u64::from),
            6 => config.tcp_send_buffer_bytes = u64::MAX,
            _ => config.ipv6_admission_prefix_bits = u32::MAX,
        }
    }
    let mut runtime = 0;
    let status = unsafe { rnet_runtime_create_v4(&config, std::ptr::null(), &mut runtime) };
    if status == RNET_OK {
        let _ = rnet_runtime_stop(runtime, 0);
        let _ = rnet_runtime_destroy(runtime);
    }
});
