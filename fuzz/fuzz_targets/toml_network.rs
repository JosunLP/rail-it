#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    railit_fuzz::toml_network(data);
});
