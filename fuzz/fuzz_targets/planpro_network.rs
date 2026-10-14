#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    railit_fuzz::planpro_network(data);
});
