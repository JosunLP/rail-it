#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = railit_fuzz::plan_image(data);
});
