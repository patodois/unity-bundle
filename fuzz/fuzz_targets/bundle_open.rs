#![no_main]
//! `Bundle::new` on raw bytes, then every read a user makes from it (entries, ranges, the
//! serialized files inside).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    unity_bundle_fuzz::bounded(data.len(), || unity_bundle_fuzz::exercise_bundle(data));
});
