use crate::color::{ChromaSampling, Depth, MatrixCoefficients, Range};
use crate::rav1d::FrameTempRef;
use crate::{Error, Image, Result};
use imgref::{Img, ImgRef, ImgVec};
use rgb::prelude::*;
use rgb::{Gray, Rgb, Rgba};
use std::borrow::Cow;
use std::collections::TryReserveError;
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

/// It's an enum, because frames may have different pixel formats
pub enum Planes<'a> {
    /// 8-bit YUV (YCbCr, YCgCo, etc.). This is a collection of 3 planes. Each may have a different size depending on `chroma_sampling`.
    ///
    /// Consult `matrix_coefficients()` for meaning of these channels
    YuvPlanes8 {
        y: ImgRef<'a, u8>,
        u: ImgRef<'a, u8>,
        v: ImgRef<'a, u8>,
        chroma_sampling: ChromaSampling,
    },
    /// 8-bit grayscale
    Mono8(ImgRef<'a, u8>),
    /// 10/12/16-bit color. Not subsampled. See `depth` field for actual range of pixels used.
    ///
    /// It's not `u16`, because it's not guaranteed to be aligned. Use `u16::from_ne_bytes()` or `ptr::read_unaligned`.
    YuvPlanes16 {
        y: ImgRef<'a, [u8; 2]>,
        u: ImgRef<'a, [u8; 2]>,
        v: ImgRef<'a, [u8; 2]>,
        chroma_sampling: ChromaSampling,
        depth: Depth,
    },
    /// 10/12/16-bit grayscale. It's not `u16`, because it's not guaranteed to be aligned. Use `u16::from_ne_bytes()` or `ptr::read_unaligned`.
    Mono16(ImgRef<'a, [u8; 2]>, Depth),
}

pub(crate) fn write_alpha16(img: &mut Image, y: ImgRef<'_, [u8; 2]>, depth: Depth, range: Range, premultiplied_alpha: bool) -> Result<()> {
    match img {
        Image::Rgba8(img) => {
            for (y_row, img_row) in y.rows().zip(img.rows_mut()) {
                if y_row.len() != img_row.len() {
                    return Err(Error::Unsupported("invalid alpha size"));
                }
                for (y, px) in y_row.iter().copied().zip(img_row.iter_mut()) {
                    px.a = (luma16(u16::from_ne_bytes(y), depth, range) >> 8) as u8;
                }
                if premultiplied_alpha {
                    unpremultiply8(img_row);
                }
            }
        },
        Image::Rgba16(img) => {
            for (y_row, img_row) in y.rows().zip(img.rows_mut()) {
                if y_row.len() != img_row.len() {
                    return Err(Error::Unsupported("invalid alpha size"));
                }
                for (y, px) in y_row.iter().copied().zip(img_row.iter_mut()) {
                    px.a = luma16(u16::from_ne_bytes(y), depth, range);
                }
                if premultiplied_alpha {
                    unpremultiply16(img_row);
                }
            }
        },
        _ => return Err(Error::InternalError("not an RGBA image")),
    }
    Ok(())
}

pub(crate) fn write_alpha8(img: &mut Image, y: ImgRef<'_, u8>, range: Range, premultiplied_alpha: bool) -> Result<()> {
    match img {
        Image::Rgba8(img) => {
            for (y_row, img_row) in y.rows().zip(img.rows_mut()) {
                if y_row.len() != img_row.len() {
                    return Err(Error::Unsupported("invalid alpha size"));
                }
                for (y, px) in y_row.iter().copied().zip(img_row.iter_mut()) {
                    px.a = luma8(y, range);
                }
                if premultiplied_alpha {
                    unpremultiply8(img_row);
                }
            }
        },
        Image::Rgba16(img) => {
            for (y_row, img_row) in y.rows().zip(img.rows_mut()) {
                if y_row.len() != img_row.len() {
                    return Err(Error::Unsupported("invalid alpha size"));
                }
                for (y, px) in y_row.iter().copied().zip(img_row.iter_mut()) {
                    px.a = luma16(u16::from(y), Depth::Depth8, range);
                }
                if premultiplied_alpha {
                    unpremultiply16(img_row);
                }
            }
        },
        _ => return Err(Error::InternalError("not an RGBA image")),
    }
    Ok(())
}

#[cold]
fn unpremultiply8(img_row: &mut [Rgba<u8>]) {
    for px in img_row.iter_mut() {
        if px.a != 255 && px.a != 0 {
            *px.rgb_mut() = px.rgb().map(|c| (u16::from(c) * 255 / u16::from(px.a)).min(255) as u8);
        }
    }
}

#[cold]
fn unpremultiply16(img_row: &mut [Rgba<u16>]) {
    for px in img_row.iter_mut() {
        if px.a != 0xFFFF && px.a != 0 {
            *px.rgb_mut() = px.rgb().map(|c| (u32::from(c) * 0xFFFF / u32::from(px.a)).min(0xFFFF) as u16);
        }
    }
}

fn imgref_align_to_u16(plane: ImgRef<'_, [u8; 2]>) -> Img<Cow<'_, [u16]>> {
    plane.new_buf(match bytemuck::try_cast_slice(plane.buf()) {
        Ok(samples) => Cow::Borrowed(samples),
        Err(_) => aligned_copy(plane),
    })
}

#[cold]
fn aligned_copy(plane: ImgRef<'_, [u8; 2]>) -> Cow<'_, [u16]> {
    Cow::Owned(plane.buf().iter().map(|b| u16::from_ne_bytes(*b)).collect())
}

impl FrameTempRef<'_> {
    /// Access pixel data
    ///
    /// The data can be grayscale (mono) or YUV (YCbCr), so the result is wrapped in an `enum`
    pub(crate) fn planes(&self) -> Result<Planes<'_>> {
        let chroma_sampling = self.chroma_sampling()?;
        let depth = self.depth()?;
        unsafe {
            Ok(match (chroma_sampling, depth) {
                (ChromaSampling::Monochrome, Depth::Depth8) => {
                    Planes::Mono8(self.single_plane_img(0)?)
                },
                (ChromaSampling::Monochrome, depth) => {
                    Planes::Mono16(self.single_plane_img(0)?, depth)
                },
                (chroma_sampling, Depth::Depth8) => {
                    let y = self.single_plane_img::<u8>(0)?;
                    // rav1d's plane order is always Y, U, V
                    let u = self.single_plane_img(1)?;
                    let v = self.single_plane_img(2)?;
                    Planes::YuvPlanes8 { y, u, v, chroma_sampling }
                },
                (chroma_sampling, depth) => {
                    let y = self.single_plane_img::<[u8; 2]>(0)?;
                    let u = self.single_plane_img(1)?;
                    let v = self.single_plane_img(2)?;
                    Planes::YuvPlanes16 { y, u, v, chroma_sampling, depth }
                },
            })
        }
    }

    pub(crate) fn yuv_to_rgb16(&self, y: ImgRef<'_, [u8; 2]>, u: ImgRef<'_, [u8; 2]>, v: ImgRef<'_, [u8; 2]>, depth: Depth, chroma_sampling: ChromaSampling, has_alpha: bool) -> Result<Image, Error> {
        let mc = self.matrix_coefficients().unwrap_or(MatrixCoefficients::BT601);
        let conv = conversion(mc, self.range())?;
        let range = to_yuv_range(self.range());
        let width = y.width();
        let height = y.height();
        // Planes are read where the decoder left them; only unaligned planes are copied
        let (y_plane, u_plane, v_plane) = (imgref_align_to_u16(y), imgref_align_to_u16(u), imgref_align_to_u16(v));
        let planar = YuvPlanarImage {
            y_plane: y_plane.buf(),
            y_stride: y_plane.stride() as u32,
            u_plane: u_plane.buf(),
            u_stride: u_plane.stride() as u32,
            v_plane: v_plane.buf(),
            v_stride: v_plane.stride() as u32,
            width: width as u32,
            height: height as u32,
        };
        if has_alpha {
            let mut out = vec![Rgba::<u16>::new(0, 0, 0, 0); width * height];
            convert16(&planar, bytemuck::cast_slice_mut(out.as_mut_slice()), (width * 4) as u32, chroma_sampling, depth, conv, range, true)?;
            Ok(Image::Rgba16(ImgVec::new(out, width, height)))
        } else {
            let mut out = vec![Rgb::<u16>::new(0, 0, 0); width * height];
            convert16(&planar, bytemuck::cast_slice_mut(out.as_mut_slice()), (width * 3) as u32, chroma_sampling, depth, conv, range, false)?;
            Ok(Image::Rgb16(ImgVec::new(out, width, height)))
        }
    }

    pub(crate) fn yuv_to_gray16(&self, y: ImgRef<'_, [u8; 2]>, depth: Depth, has_alpha: bool) -> Result<Image, TryReserveError> {
        let range = self.range();
        let width = y.width();
        let height = y.height();
        Ok(if has_alpha {
            let mut out = Vec::new();
            out.try_reserve_exact(width * height)?;
            out.extend(y.rows().flat_map(|row| {
                row.iter().copied().map(|y| {
                    let g = luma16(u16::from_ne_bytes(y), depth, range);
                    Rgba::new(g, g, g, 0)
                })
            }));
            Image::Rgba16(ImgVec::new(out, width, height))
        } else {
            let mut out = Vec::new();
            out.try_reserve_exact(width * height)?;
            out.extend(y.rows().flat_map(|row| {
                row.iter()
                    .copied()
                    .map(|y| Gray::new(luma16(u16::from_ne_bytes(y), depth, range)))
            }));
            Image::Gray16(ImgVec::new(out, width, height))
        })
    }

    pub(crate) fn yuv_to_gray8(&self, y: ImgRef<'_, u8>, has_alpha: bool) -> Result<Image, TryReserveError> {
        let range = self.range();
        let width = y.width();
        let height = y.height();
        Ok(if has_alpha {
            let mut out = Vec::new();
            out.try_reserve_exact(width * height)?;
            out.extend(y.rows().flat_map(|row| {
                row.iter().copied().map(|y| {
                    let g = luma8(y, range);
                    Rgba::new(g, g, g, 0)
                })
            }));
            Image::Rgba8(ImgVec::new(out, width, height))
        } else {
            let mut out = Vec::new();
            out.try_reserve_exact(width * height)?;
            out.extend(y.rows().flat_map(|row| {
                row.iter()
                    .copied()
                    .map(|y| Gray::new(luma8(y, range)))
            }));
            Image::Gray8(ImgVec::new(out, width, height))
        })
    }

    pub(crate) fn yuv_to_rgb8(&self, y: ImgRef<'_, u8>, u: ImgRef<'_, u8>, v: ImgRef<'_, u8>, chroma_sampling: ChromaSampling, has_alpha: bool) -> Result<Image, Error> {
        let mc = self.matrix_coefficients().unwrap_or(MatrixCoefficients::BT601);
        let conv = conversion(mc, self.range())?;
        let range = to_yuv_range(self.range());
        let width = y.width();
        let height = y.height();
        let planar = YuvPlanarImage {
            y_plane: y.buf(),
            y_stride: y.stride() as u32,
            u_plane: u.buf(),
            u_stride: u.stride() as u32,
            v_plane: v.buf(),
            v_stride: v.stride() as u32,
            width: width as u32,
            height: height as u32,
        };
        if has_alpha {
            let mut out = vec![Rgba::<u8>::new(0, 0, 0, 0); width * height];
            convert8(&planar, bytemuck::cast_slice_mut(out.as_mut_slice()), (width * 4) as u32, chroma_sampling, conv, range, true)?;
            Ok(Image::Rgba8(ImgVec::new(out, width, height)))
        } else {
            let mut out = vec![Rgb::<u8>::new(0, 0, 0); width * height];
            convert8(&planar, bytemuck::cast_slice_mut(out.as_mut_slice()), (width * 3) as u32, chroma_sampling, conv, range, false)?;
            Ok(Image::Rgb8(ImgVec::new(out, width, height)))
        }
    }
}

/// Meaning of the frame's planes, as far as conversion to RGB is concerned
#[derive(Debug, Copy, Clone, PartialEq)]
enum Conversion {
    /// Planes are G, B, R (identity matrix). Only valid without chroma subsampling.
    Gbr,
    /// `YCgCo`
    YCgCo,
    /// YCbCr with one of the standard matrices
    Matrix(YuvStandardMatrix),
}

fn conversion(mc: MatrixCoefficients, range: Range) -> Result<Conversion, Error> {
    Ok(match mc {
        MatrixCoefficients::Identity => Conversion::Gbr,
        MatrixCoefficients::YCgCo => Conversion::YCgCo,
        MatrixCoefficients::BT709 => Conversion::Matrix(YuvStandardMatrix::Bt709),
        MatrixCoefficients::Fcc => Conversion::Matrix(YuvStandardMatrix::Fcc),
        // BT.470 System B,G uses the same matrix as BT.601 (ITU-T H.273 table 4),
        // which is not what `yuv`'s own `Bt470_6` variant means
        MatrixCoefficients::BT470BG |
        MatrixCoefficients::BT601 => Conversion::Matrix(YuvStandardMatrix::Bt601),
        MatrixCoefficients::SMPTE240 => Conversion::Matrix(YuvStandardMatrix::Smpte240),
        MatrixCoefficients::BT2020NCL => Conversion::Matrix(YuvStandardMatrix::Bt2020),
        _ => {
            log::debug!("Unsupported matrix coefficients: {mc:?} ({range:?})");
            return Err(Error::Unsupported("matrix coefficients"));
        },
    })
}

#[inline]
fn to_yuv_range(range: Range) -> YuvRange {
    match range {
        Range::Full => YuvRange::Full,
        Range::Limited => YuvRange::Limited,
    }
}

/// Convert 8-bit planes into `dst`, which is RGB when `has_alpha` is false and RGBA otherwise
fn convert8(planar: &YuvPlanarImage<u8>, dst: &mut [u8], dst_stride: u32, chroma_sampling: ChromaSampling, conv: Conversion, range: YuvRange, has_alpha: bool) -> Result<(), Error> {
    use ChromaSampling::{Cs420, Cs422, Cs444, Monochrome};
    use Conversion::{Gbr, Matrix, YCgCo};
    let res = match (conv, chroma_sampling) {
        (Gbr, Cs444) => if has_alpha {
            yuv::gbr_to_rgba(planar, dst, dst_stride, range)
        } else {
            yuv::gbr_to_rgb(planar, dst, dst_stride, range)
        },
        (Gbr, _) => return Err(Error::Unsupported("identity matrix with chroma subsampling")),
        (YCgCo, Cs444) => if has_alpha {
            yuv::ycgco444_to_rgba(planar, dst, dst_stride, range)
        } else {
            yuv::ycgco444_to_rgb(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs422) => if has_alpha {
            yuv::ycgco422_to_rgba(planar, dst, dst_stride, range)
        } else {
            yuv::ycgco422_to_rgb(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs420) => if has_alpha {
            yuv::ycgco420_to_rgba(planar, dst, dst_stride, range)
        } else {
            yuv::ycgco420_to_rgb(planar, dst, dst_stride, range)
        },
        (YCgCo, Monochrome) => return Err(Error::InternalError("monochrome in a color conversion")),
        (Matrix(m), Cs444) => if has_alpha {
            yuv::yuv444_to_rgba(planar, dst, dst_stride, range, m)
        } else {
            yuv::yuv444_to_rgb(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs422) => if has_alpha {
            yuv::yuv422_to_rgba(planar, dst, dst_stride, range, m)
        } else {
            yuv::yuv422_to_rgb(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs420) => if has_alpha {
            yuv::yuv420_to_rgba(planar, dst, dst_stride, range, m)
        } else {
            yuv::yuv420_to_rgb(planar, dst, dst_stride, range, m)
        },
        (Matrix(_), Monochrome) => return Err(Error::InternalError("monochrome in a color conversion")),
    };
    res.inspect_err(|e| log::debug!("{e} (conversion={conv:?}, range={range:?})"))?;
    Ok(())
}

/// Convert 16-bit planes into `dst`, which is RGB when `has_alpha` is false and RGBA otherwise
fn convert16(planar: &YuvPlanarImage<u16>, dst: &mut [u16], dst_stride: u32, chroma_sampling: ChromaSampling, depth: Depth, conv: Conversion, range: YuvRange, has_alpha: bool) -> Result<(), Error> {
    use ChromaSampling::{Cs420, Cs422, Cs444, Monochrome};
    use Conversion::{Gbr, Matrix, YCgCo};
    let res = match (conv, chroma_sampling, depth) {
        (Gbr, Cs444, Depth::Depth10) => if has_alpha {
            yuv::gb10_to_rgba10(planar, dst, dst_stride, range)
        } else {
            yuv::gb10_to_rgb10(planar, dst, dst_stride, range)
        },
        (Gbr, Cs444, Depth::Depth12) => if has_alpha {
            yuv::gb12_to_rgba12(planar, dst, dst_stride, range)
        } else {
            yuv::gb12_to_rgb12(planar, dst, dst_stride, range)
        },
        (Gbr, Cs444, _) => if has_alpha {
            yuv::gb16_to_rgba16(planar, dst, dst_stride, range)
        } else {
            yuv::gb16_to_rgb16(planar, dst, dst_stride, range)
        },
        (Gbr, _, _) => return Err(Error::Unsupported("identity matrix with chroma subsampling")),
        (YCgCo, Cs444, Depth::Depth10) => if has_alpha {
            yuv::icgc410_to_rgba10(planar, dst, dst_stride, range)
        } else {
            yuv::icgc410_to_rgb10(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs422, Depth::Depth10) => if has_alpha {
            yuv::icgc210_to_rgba10(planar, dst, dst_stride, range)
        } else {
            yuv::icgc210_to_rgb10(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs420, Depth::Depth10) => if has_alpha {
            yuv::icgc010_to_rgba10(planar, dst, dst_stride, range)
        } else {
            yuv::icgc010_to_rgb10(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs444, Depth::Depth12) => if has_alpha {
            yuv::icgc412_to_rgba12(planar, dst, dst_stride, range)
        } else {
            yuv::icgc412_to_rgb12(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs422, Depth::Depth12) => if has_alpha {
            yuv::icgc212_to_rgba12(planar, dst, dst_stride, range)
        } else {
            yuv::icgc212_to_rgb12(planar, dst, dst_stride, range)
        },
        (YCgCo, Cs420, Depth::Depth12) => if has_alpha {
            yuv::icgc012_to_rgba12(planar, dst, dst_stride, range)
        } else {
            yuv::icgc012_to_rgb12(planar, dst, dst_stride, range)
        },
        (YCgCo, _, _) => return Err(Error::Unsupported("YCgCo at this depth")),
        (Matrix(m), Cs444, Depth::Depth10) => if has_alpha {
            yuv::i410_to_rgba10(planar, dst, dst_stride, range, m)
        } else {
            yuv::i410_to_rgb10(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs444, Depth::Depth12) => if has_alpha {
            yuv::i412_to_rgba12(planar, dst, dst_stride, range, m)
        } else {
            yuv::i412_to_rgb12(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs444, _) => if has_alpha {
            yuv::i416_to_rgba16(planar, dst, dst_stride, range, m)
        } else {
            yuv::i416_to_rgb16(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs422, Depth::Depth10) => if has_alpha {
            yuv::i210_to_rgba10(planar, dst, dst_stride, range, m)
        } else {
            yuv::i210_to_rgb10(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs422, Depth::Depth12) => if has_alpha {
            yuv::i212_to_rgba12(planar, dst, dst_stride, range, m)
        } else {
            yuv::i212_to_rgb12(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs422, _) => if has_alpha {
            yuv::i216_to_rgba16(planar, dst, dst_stride, range, m)
        } else {
            yuv::i216_to_rgb16(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs420, Depth::Depth10) => if has_alpha {
            yuv::i010_to_rgba10(planar, dst, dst_stride, range, m)
        } else {
            yuv::i010_to_rgb10(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs420, Depth::Depth12) => if has_alpha {
            yuv::i012_to_rgba12(planar, dst, dst_stride, range, m)
        } else {
            yuv::i012_to_rgb12(planar, dst, dst_stride, range, m)
        },
        (Matrix(m), Cs420, _) => if has_alpha {
            yuv::i016_to_rgba16(planar, dst, dst_stride, range, m)
        } else {
            yuv::i016_to_rgb16(planar, dst, dst_stride, range, m)
        },
        (Matrix(_), Monochrome, _) => return Err(Error::InternalError("monochrome in a color conversion")),
    };
    res.inspect_err(|e| log::debug!("{e} (conversion={conv:?}, range={range:?})"))?;

    // The conversions work at their own bit depth, but the crate assumes full 16-bit range
    if depth != Depth::Depth16 {
        let shift = 16 - depth.bits();
        let low_shift = depth.bits() - shift;
        for v in dst.iter_mut() {
            *v = (*v << shift) | (*v >> low_shift);
        }
    }
    Ok(())
}

/// Expand an 8-bit luma sample to the full 8-bit range
#[inline]
fn luma8(v: u8, range: Range) -> u8 {
    (luma16(u16::from(v), Depth::Depth8, range) >> 8) as u8
}

/// Expand a luma sample of the given depth to the full 16-bit range
#[inline]
fn luma16(v: u16, depth: Depth, range: Range) -> u16 {
    let bits = depth.bits();
    let (min, span) = match range {
        Range::Full => (0, (1 << bits) - 1),
        Range::Limited => (16 << (bits - 8), 219 << (bits - 8)),
    };
    let v = u32::from(v).saturating_sub(min).min(span);
    ((v * 65535 + span / 2) / span) as u16
}
