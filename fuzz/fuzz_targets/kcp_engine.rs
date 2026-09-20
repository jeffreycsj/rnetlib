#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use rnet_transport::{KcpEngine, RustKcpEngine};

fuzz_target!(|data: &[u8]| {
    let mtu = data.first().map_or(1200, |value| 576 + usize::from(*value));
    let mut engine = match RustKcpEngine::new_with_mtu(1, mtu) {
        Ok(engine) => engine,
        Err(_) => return,
    };
    let now = data.get(1).copied().map_or(0, u64::from);
    let packet = data.get(2..).unwrap_or_default();
    let _ = engine.input(packet, now);
    let _ = engine.send(packet);
    engine.update(now, &mut |_| {});
    let mut received = BytesMut::new();
    let _ = engine.recv(&mut received);
    let _ = engine.next_update_ms();
    let _ = engine.observed_rtt();
});
