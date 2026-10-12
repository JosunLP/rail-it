#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    railit_fuzz::node_wire(data);
});
