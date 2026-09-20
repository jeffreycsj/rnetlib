#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    rnet_transport::fuzz_support::datagram_preflight(data);
});
