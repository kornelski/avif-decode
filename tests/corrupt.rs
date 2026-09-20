//! Regression tests for decoding valid and corrupt AVIF input

/// A tiny valid AVIF (16×16, 8-bit 4:4:4, encoded with avifenc)
const VALID: &[u8] = include_bytes!("data/tiny.avif");

/// A bit in the AV1 payload's frame data; flipping it makes rav1d reject the frame
const CORRUPT_OFFSET: usize = 280;

fn dims(img: avif_decode::Image) -> (usize, usize) {
    match img {
        avif_decode::Image::Rgb8(i) => (i.width(), i.height()),
        avif_decode::Image::Rgb16(i) => (i.width(), i.height()),
        avif_decode::Image::Rgba8(i) => (i.width(), i.height()),
        avif_decode::Image::Rgba16(i) => (i.width(), i.height()),
        avif_decode::Image::Gray8(i) => (i.width(), i.height()),
        avif_decode::Image::Gray16(i) => (i.width(), i.height()),
    }
}

#[test]
fn valid_avif_decodes() {
    let img = avif_decode::Decoder::from_avif(VALID).unwrap().to_image().unwrap();
    assert_eq!(dims(img), (16, 16));
}

/// A corrupt AV1 payload must be reported as an error.
///
/// Not running yet: rav1d 1.1.0 panics in its own error path
/// (`rav1d-1.1.0/src/decode.rs:4997` unwraps a missing frame header), and since that panic
/// happens inside rav1d's `extern "C"` entry point it can't unwind — the process aborts
/// instead of returning an error. An upstream fix has been submitted; remove the `ignore`
/// once a rav1d release with it is available.
#[test]
#[ignore = "rav1d 1.1.0 aborts (SIGABRT) on this corrupt payload; waiting for the upstream fix"]
fn corrupt_av1_payload_is_an_error() {
    let mut data = VALID.to_vec();
    data[CORRUPT_OFFSET] ^= 0x01;

    let err = match avif_decode::Decoder::from_avif(&data).and_then(avif_decode::Decoder::to_image) {
        Ok(_) => panic!("a corrupt AV1 payload must be an error, not a crash"),
        Err(err) => err,
    };
    assert!(matches!(err, avif_decode::Error::Rav1d(_)), "expected a rav1d error, got: {err}");
}
