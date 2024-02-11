# AVIF to PNG converter

Converts AVIF images (AV1 + HEIF) to standard PNG images. It's meant to decode very accurately (using AOM, precise colorspace conversions). It is not meant to produce small PNG files. Some PNG files will be 100 times larger than the AVIF input.

It can also be used as a Rust library for decoding straight to uncompressed pixels (RGBA, etc).

## Usage

```bash
avif_decode input.avif output.png
```

It always processes one file at a time. If output path is omitted, it'll be the same as the input, except with a `.png` extension. Add `-f` to overwrite output files.

### Install from source

Requires [Rust 1.75 or later](https://rustup.rs) and a C compiler.

```bash
cargo install avif-decode
```

## Features

### Supported AVIF features

 * 8-, 10-, and 12-bit deep images. Images are saved as 16-bit PNG when necessary.
 * RGB and many flavors of YUV color spaces in both full and "studio" range.
 * Alpha channel, in both premultiplied and uncorrelated alpha modes.
 * 4:4:4, 4:2:2, 4:2:0 chroma modes. Chroma subsampling uses box upsampling.
   BTW: The AVIF spec intentionally left chroma upsampling algorithm unspecified, so images using 4:2:0 or 4:2:2 modes will have decoder-dependent distortions. It's best to never use chroma subsampling modes in AVIF.

### Unsupported features

 * Any form of HDR. Maybe later.
 * YCgCo color space.
 * Embedded ICC color profiles. AV1 already supports *so many* color spaces, it'd be rude to support an extra color conversion layer that's complex and unecessary.
 * The kitchen sink of pointless HEIF features. I'm writing an image decoder, not Photoshop.

 ## License

New BSD
