#![no_main]
//! `SerializedFile::parse` on raw bytes, then reading every object through its type tree.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    unity_bundle_fuzz::bounded(data.len(), || unity_bundle_fuzz::exercise_serialized(data));
});
