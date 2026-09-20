use crate::{Error, Result};
use crate::color::{ChromaSampling, Depth, MatrixCoefficients, Range};
use imgref::ImgRef;
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::*;
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture};
use rav1d::src::lib::{dav1d_open, dav1d_picture_unref, dav1d_send_data};
use rav1d::Dav1dResult;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::{io, ptr};
use std::ptr::NonNull;

pub(crate) struct Rav1dDecoder {
    ctx: Option<Dav1dContext>,
    /// The last decoded picture. Kept alive so that [`FrameTempRef`] can borrow it.
    picture: Option<Dav1dPicture>,
}

impl Rav1dDecoder {
    #[inline]
    pub(crate) fn new(threads: usize) -> Result<Self> {
        unsafe {
            let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
            dav1d_default_settings(NonNull::from(&mut settings).cast());
            let settings = settings.assume_init_mut();
            // rav1d supports up to 256 threads; 0 means one per logical CPU
            settings.n_threads = threads.min(256) as _;
            // Still images are decoded one frame at a time, so there's nothing to pipeline:
            // keep a single frame context to make each decoded frame available immediately
            // (tile decoding is still multi-threaded via `n_threads`).
            settings.max_frame_delay = 1;
            // Grain changes the output pixels, and the other decoders synthesize it too
            // (libaom and libavif both apply it), so it must not be left to the caller
            settings.apply_grain = 1;

            let mut ctx = None;
            let res = dav1d_open(Some(NonNull::from(&mut ctx)), Some(NonNull::from(settings)));
            err_if_failed(res)?;
            Ok(Self { ctx, picture: None })
        }
    }

    /// Take AV1-compressed data and decode a *single* frame into raw frame data (YUV pixels). This is for AVIF, and can't handle video.
    ///
    /// The returned frame is temporary. You must copy data out of it and drop it before decoding other files.
    ///
    /// See [yuv](https://lib.rs/yuv) crate for conversion to RGB.
    #[inline]
    pub(crate) fn decode_frame<'a>(&'a mut self, av1_data: &[u8]) -> Result<FrameTempRef<'a>> {
        let pic = self.decode_into(av1_data)?;
        Ok(FrameTempRef(pic, PhantomData))
    }

    /// Decodes `av1_data` and replaces the previously decoded picture (if any) with the new one.
    /// Returns a reference to the picture held in `self`.
    fn decode_into(&mut self, av1_data: &[u8]) -> Result<&Dav1dPicture> {
        let ctx = self.ctx.ok_or(Error::InternalError("decoder closed"))?;
        let pic = unsafe { self.decode_av1_data(ctx, av1_data) }?;

        if let Some(old) = &mut self.picture {
            unsafe { dav1d_picture_unref(Some(NonNull::from(old))) };
        }
        Ok(self.picture.insert(pic))
    }

    /// Feeds one AV1 temporal unit (a single AVIF item) to the decoder and returns its picture.
    /// For a layered item this is the final layer, which is the complete image.
    ///
    /// # Safety
    ///
    /// `ctx` must be a context obtained from `dav1d_open`.
    unsafe fn decode_av1_data(&self, ctx: Dav1dContext, av1_data: &[u8]) -> Result<Dav1dPicture> {
        unsafe {
            let mut data = Dav1dData::default();
            // A layered (progressively decodable) item decodes to one picture per layer, and only
            // the last one is the complete image: later layers refine the earlier ones, and film
            // grain is applied on the final layer only
            let mut last_pic = None;
            let send_res = (|| -> Result<()> {
                let buf = dav1d_data_create(Some(NonNull::from(&mut data)), av1_data.len());
                let Some(buf) = NonNull::new(buf) else {
                    return Err(Error::Io(io::ErrorKind::OutOfMemory.into()));
                };
                ptr::copy_nonoverlapping(av1_data.as_ptr(), buf.as_ptr(), av1_data.len());

                // One AVIF item is a single temporal unit, so one `send_data` consumes it all.
                // If the decoder is still holding a frame (the item had more than one frame),
                // take the pending frame out and retry, so that `send_data` can make progress.
                loop {
                    let res = dav1d_send_data(Some(ctx), Some(NonNull::from(&mut data)));
                    if res.0 == 0 {
                        return Ok(());
                    }
                    let mut pending = Dav1dPicture::default();
                    let drain_res = dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut pending)));
                    if drain_res.0 != 0 {
                        // there's no frame to drain, so the decoder can't accept this data
                        dav1d_picture_unref(Some(NonNull::from(&mut pending)));
                        return err_if_failed(res);
                    }
                    // drop the previous layer, which this picture supersedes
                    if let Some(mut prev) = last_pic.replace(pending) {
                        dav1d_picture_unref(Some(NonNull::from(&mut prev)));
                    }
                }
            })();

            // the decoder owns the buffer only when it has taken it
            dav1d_data_unref(Some(NonNull::from(&mut data)));
            send_res?;

            // A layered (progressively decodable) item produces one picture per layer, and only the
            // last one is the complete image: later layers refine the earlier ones, and film grain
            // is synthesized on the final layer only. So take pictures until the decoder stops
            // producing them, and keep the newest one.
            loop {
                let mut pic = Dav1dPicture::default();
                let res = dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut pic)));
                if res.0 != 0 {
                    dav1d_picture_unref(Some(NonNull::from(&mut pic)));
                    if let Some(pic) = last_pic {
                        return Ok(pic);
                    }
                    // nothing decoded: report the decoder's own reason for stopping
                    err_if_failed(res)?;
                    return Err(Error::InternalError("no frame decoded"));
                }
                if let Some(mut prev) = last_pic.replace(pic) {
                    dav1d_picture_unref(Some(NonNull::from(&mut prev)));
                }
            }
        }
    }
}

/// `rav1d` reports success as 0, and errors as a negated `errno` value.
fn err_if_failed(res: Dav1dResult) -> Result<()> {
    if res.0 == 0 {
        Ok(())
    } else {
        Err(Error::Rav1d(res.0.unsigned_abs()))
    }
}

impl Drop for Rav1dDecoder {
    #[inline]
    fn drop(&mut self) {
        if let Some(pic) = &mut self.picture {
            unsafe { dav1d_picture_unref(Some(NonNull::from(pic))) };
        }
        unsafe { dav1d_close(Some(NonNull::from(&mut self.ctx))) };
    }
}

/// Frame held in decoder's internal state. Must be dropped before the next call.
pub struct FrameTempRef<'a>(&'a Dav1dPicture, PhantomData<&'a mut Rav1dDecoder>);

impl FrameTempRef<'_> {
    /// # Safety
    ///
    /// The picture must still be alive, and its plane must hold at least `stride * h` pixels.
    #[inline]
    pub(crate) unsafe fn single_plane_img<T: Copy>(&self, plane_n: u8) -> Result<ImgRef<'_, T>> {
        assert!(plane_n < 3);

        let pixel_bytes = if self.0.p.bpc <= 8 { 1 } else { 2 };
        if pixel_bytes != std::mem::size_of::<T>() {
            return Err(Error::InternalError("pixel size"));
        }

        let Some(plane) = self.0.data[plane_n as usize] else {
            return Err(Error::InternalError("missing plane"));
        };

        // Chroma planes are subsampled depending on the pixel layout
        let (ss_hor, ss_ver) = match (plane_n != 0, self.0.p.layout) {
            (false, _) | (true, DAV1D_PIXEL_LAYOUT_I444) => (0u8, 0u8),
            (true, DAV1D_PIXEL_LAYOUT_I420) => (1, 1),
            (true, DAV1D_PIXEL_LAYOUT_I422) => (1, 0),
            (true, _) => return Err(Error::InternalError("unknown pixel layout")),
        };
        let w = (self.0.p.w as usize + usize::from(ss_hor)) >> ss_hor;
        let h = (self.0.p.h as usize + usize::from(ss_ver)) >> ss_ver;

        // rav1d uses one stride for luma, and one shared by both chroma planes
        let stride_bytes = self.0.stride[usize::from(plane_n != 0)];
        if stride_bytes <= 0 || !(stride_bytes as usize).is_multiple_of(pixel_bytes) {
            return Err(Error::InternalError("stride"));
        }
        let stride = stride_bytes as usize / pixel_bytes;
        if stride < w || w == 0 {
            return Err(Error::InternalError("stride"));
        }

        // rav1d allocates whole rows of pixels, so `stride * h` stays within the plane's buffer
        let buf = unsafe { std::slice::from_raw_parts(plane.as_ptr().cast::<T>(), stride * h) };
        Ok(ImgRef::new_stride(buf, w, h, stride))
    }

    /// Whether image uses chroma subsampling or not
    #[inline]
    pub const fn chroma_sampling(&self) -> Result<ChromaSampling> {
        Ok(match self.0.p.layout {
            DAV1D_PIXEL_LAYOUT_I400 => ChromaSampling::Monochrome,
            DAV1D_PIXEL_LAYOUT_I420 => ChromaSampling::Cs420,
            DAV1D_PIXEL_LAYOUT_I422 => ChromaSampling::Cs422,
            DAV1D_PIXEL_LAYOUT_I444 => ChromaSampling::Cs444,
            _ => return Err(Error::InternalError("unknown pixel layout")),
        })
    }

    /// How many bits per pixel that is
    #[inline]
    pub const fn depth(&self) -> Result<Depth> {
        Ok(match self.0.p.bpc {
            8 => Depth::Depth8,
            10 => Depth::Depth10,
            12 => Depth::Depth12,
            16 => Depth::Depth16,
            _ => return Err(Error::InternalError("bit depth")),
        })
    }

    /// Color info from the AV1 sequence header
    #[inline]
    fn seq_hdr(&self) -> Option<&Dav1dSequenceHeader> {
        Some(unsafe { self.0.seq_hdr?.as_ref() })
    }

    /// Flavor of YUV used for the pixels
    ///
    /// See [yuv](https://lib.rs/yuv) crate for conversion to RGB.
    #[inline]
    #[must_use]
    pub fn matrix_coefficients(&self) -> Option<MatrixCoefficients> {
        Some(match self.seq_hdr()?.mtrx {
            DAV1D_MC_IDENTITY => MatrixCoefficients::Identity,
            DAV1D_MC_BT709 => MatrixCoefficients::BT709,
            DAV1D_MC_FCC => MatrixCoefficients::Fcc,
            DAV1D_MC_BT470BG => MatrixCoefficients::BT470BG,
            DAV1D_MC_BT601 => MatrixCoefficients::BT601,
            DAV1D_MC_SMPTE240 => MatrixCoefficients::SMPTE240,
            DAV1D_MC_SMPTE_YCGCO => MatrixCoefficients::YCgCo,
            DAV1D_MC_BT2020_NCL | DAV1D_MC_BT2020_CL => MatrixCoefficients::BT2020NCL,
            DAV1D_MC_SMPTE2085 => MatrixCoefficients::SMPTE2085,
            DAV1D_MC_CHROMAT_NCL => MatrixCoefficients::ChromatNCL,
            DAV1D_MC_CHROMAT_CL => MatrixCoefficients::ChromatCL,
            DAV1D_MC_ICTCP => MatrixCoefficients::ICtCp,
            _ => return None,
        })
    }

    /// Whether pixels are in 0-255 or 16-235/240 range.
    #[inline(always)]
    #[must_use]
    pub fn range(&self) -> Range {
        let Some(hdr) = self.seq_hdr() else {
            return Range::Limited
        };
        // 1 means full range in AV1
        if hdr.color_range != 0 {
            Range::Full
        } else {
            Range::Limited
        }
    }
}
