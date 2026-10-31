#![no_main]
//! The same, inside a valid header: the fuzzer's bytes are the block info (LZ4-compressed or
//! stored) and the blocks, so the table parser and the block decoders are reached at once.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(bytes) = unity_bundle_fuzz::wrap_bundle(data) {
        unity_bundle_fuzz::bounded(data.len(), || unity_bundle_fuzz::exercise_bundle(&bytes));
    }
});
