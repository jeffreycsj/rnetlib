#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    rnet_game::fuzz_support::decode_game_wire(data);
});
