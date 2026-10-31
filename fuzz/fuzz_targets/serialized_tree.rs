#![no_main]
//! A valid serialized file whose one type tree blob and one object are the fuzzer's bytes: the
//! blob reader and the deserializer, without having to get the file layout right.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(bytes) = unity_bundle_fuzz::wrap_serialized(data) {
        unity_bundle_fuzz::bounded(data.len(), || {
            unity_bundle_fuzz::exercise_serialized(&bytes)
        });
    }
});
