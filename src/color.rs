//! Color space metadata of a decoded frame
//!
//! These enums describe the frame's metadata, as defined by the
//! “Colour information” (CICP) section of [ITU-T H.273](https://www.itu.int/rec/T-REC-H.273).
//!
//! They are the frame's own description of its pixel data — pass them to
//! the [yuv](https://lib.rs/yuv) crate to convert pixels to RGB.

/// Chroma subsampling format
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ChromaSampling {
    /// 4:2:0 = 2x2 pixels of luma per 1 pixel of chroma
    Cs420,
    /// 4:2:2 = Horizontally subsampled
    Cs422,
    /// 4:4:4 = Not subsampled (chroma and luma are 1:1)
    Cs444,
    /// 4:0:0 = Only luma (grayscale)
    Monochrome,
}

/// Range of allowed values for pixels
#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Range {
    /// Luma is 16-235, Chroma is 16-240
    Limited,
    /// 0-255
    Full,
}

/// Bit depth (8 = 1 byte, >=10 = 2 bytes)
#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum Depth {
    Depth8 = 8,
    Depth10 = 10,
    Depth12 = 12,
    Depth16 = 16,
}

impl Depth {
    /// Number of bits used by each pixel component
    #[inline(always)]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self as u32
    }
}

/// As defined by the “Matrix coefficients” section of ISO/IEC 23091-4/ITU-T H.273.
#[repr(C)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MatrixCoefficients {
    /// Identity matrix
    Identity = 0,
    /// BT.709 ITU-R Rec. BT.709-5, ITU-R Rec. BT.1361 conventional colour gamut system and extended colour gamut system,
    /// IEC 61966-2-4 xvYCC709,
    /// Society of Motion Picture and Television Engineers RP 177 (1993)
    /// KR = 0.2126; KB = 0.0722
    BT709,
    /// United States Federal Communications Commission Title 47 Code of Federal Regulations (2003) 73.682 (a) (20)
    /// KR = 0.30; KB = 0.11
    Fcc = 4,
    /// ITU-R Rec. BT.470-6 System B,G, IEC 61966-2-4 xvYCC601 (historical)
    /// KR = 0.299; KB = 0.114
    BT470BG,
    /// BT.601-7 525 (SMPTE 170 M)
    /// KR = 0.299; KB = 0.114
    BT601,
    /// SMPTE 240 M
    /// KR = 0.212; KB = 0.087
    SMPTE240,
    /// `YCgCo`
    YCgCo,
    /// BT.2020 non-constant luminance, BT.2100 YCbCr
    BT2020NCL,
    /// SMPTE ST 2085 `YDzDx`
    SMPTE2085,
    /// Chromaticity-derived non-constant luminance
    ChromatNCL,
    /// Chromaticity-derived constant luminance
    ChromatCL,
    /// BT.2020 `ICtCp`
    ICtCp,
}
