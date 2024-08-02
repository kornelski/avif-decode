use rgb::ComponentMap;
use rgb::RGBA16;
use aom_decode::chroma::{yuv_420, yuv_422, yuv_444};
use aom_decode::color;
use aom_decode::Config;
use aom_decode::FrameTempRef;
use aom_decode::RowsIters;
use imgref::ImgVec;
use rgb::alt::GRAY16;
use rgb::alt::GRAY8;
use rgb::RGB16;
use rgb::RGB8;
use rgb::RGBA8;
use std::io;
use yuv::YUV;

use quick_error::quick_error;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: io::Error) {
            display("{}", err)
            from()
        }
        Parse(err: avif_parse::Error) {
            display("{}", err)
            from()
        }
        Decode(err: aom_decode::Error) {
            display("{}", err)
            from()
        }
        Meta(err: yuv::Error) {
            display("{}", err)
            from()
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub enum Image {
    Rgb8(ImgVec<RGB8>),
    Rgb16(ImgVec<RGB16>),
    Rgba8(ImgVec<RGBA8>),
    Rgba16(ImgVec<RGBA16>),
    Gray8(ImgVec<GRAY8>),
    Gray16(ImgVec<GRAY16>),
}

enum AlphaImage {
    Gray8(ImgVec<GRAY8>),
    Gray16(ImgVec<GRAY16>),
}

pub struct Decoder {
    _decoder: Box<aom_decode::Decoder>,
    color: FrameTempRef<'static>,
    alpha: Option<AlphaImage>,
    premultiplied_alpha: bool,
}

impl Decoder {
    #[inline(always)]
    pub fn from_avif(mut data: &[u8]) -> Result<Self> {
        Self::from_reader(&mut data)
    }

    #[inline]
    pub fn from_reader<R: io::Read>(reader: &mut R) -> Result<Self> {
        let avif = avif_parse::read_avif(reader)?;
        Self::from_parsed(avif)
    }

    fn from_parsed(avif: avif_parse::AvifData) -> Result<Self> {
        let mut decoder = Box::new(aom_decode::Decoder::new(&Config {
            threads: std::thread::available_parallelism().map(|a| a.get()).unwrap_or(4).min(32),
        })?);

        let alpha = avif.alpha_item.as_ref().map(|a| Self::to_alpha(decoder.decode_frame(a)?)).transpose()?;
        let premultiplied_alpha = avif.premultiplied_alpha;
        let color = decoder.decode_frame(&avif.primary_item)?;
        // This lifetime only exist to prevent further calls on the Decoder.
        // This is guaranteed here by decoding the alpha first.
        let color = unsafe {
            std::mem::transmute::<FrameTempRef<'_>, FrameTempRef<'static>>(color)
        };
        Ok(Self {
            _decoder: decoder,
            color,
            alpha,
            premultiplied_alpha,
        })
    }

    pub fn to_image(self) -> Result<Image> {
        let color = Self::color_convert(self.color)?;
        Ok(if let Some(alpha) = self.alpha {
            let mut image = match (color, alpha) {
                (Image::Rgb8(img), AlphaImage::Gray8(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| c.with_alpha(*a)).collect();
                    Image::Rgba8(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Rgb8(img), AlphaImage::Gray16(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| c.map(|c| (u16::from(c) << 8) | u16::from(c)).with_alpha(*a)).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Rgb16(img), AlphaImage::Gray8(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| c.with_alpha(u16::from(*a) << 8 | u16::from(*a))).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Rgb16(img), AlphaImage::Gray16(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| c.with_alpha(*a)).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Rgba8(img), AlphaImage::Gray8(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| c.with_alpha(*a)).collect();
                    Image::Rgba8(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Gray8(img), AlphaImage::Gray8(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| RGBA8::new(*c,*c,*c,*a)).collect();
                    Image::Rgba8(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Gray8(img), AlphaImage::Gray16(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| {
                        let c = u16::from(*c) << 8 | u16::from(*c);
                        RGBA16::new(c,c,c,*a)
                    }).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Gray16(img), AlphaImage::Gray8(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| RGBA16::new(*c,*c,*c,u16::from(*a) << 8 | u16::from(*a))).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Gray16(img), AlphaImage::Gray16(alpha)) => {
                    let buf = img.pixels().zip(alpha.pixels()).map(|(c, a)| RGBA16::new(*c,*c,*c,*a)).collect();
                    Image::Rgba16(ImgVec::new(buf, img.width(), img.height()))
                },
                (Image::Rgba8(_) | Image::Rgba16(_), _) => unreachable!(),
            };
            if self.premultiplied_alpha {
                match &mut image {
                    Image::Rgba8(img) => {
                        img.pixels_mut().filter(|px| px.a > 0).for_each(|px| {
                            #[inline(always)]
                            fn unprem(val: u8, alpha: u8) -> u8 {
                                ((u16::from(val) * 256) / (u16::from(alpha) * 256) / 256).min(255) as u8
                            }
                            px.r = unprem(px.r, px.a);
                            px.g = unprem(px.g, px.a);
                            px.b = unprem(px.b, px.a);
                        });
                    },
                    Image::Rgba16(img) => {
                        img.pixels_mut().filter(|px| px.a > 0).for_each(|px| {
                            #[inline(always)]
                            fn unprem(val: u16, alpha: u16) -> u16 {
                                ((u32::from(val) * 0xFFFF) / (u32::from(alpha) * 0xFFFF) / 0xFFFF).min(65535) as u16
                            }
                            px.r = unprem(px.r, px.a);
                            px.g = unprem(px.g, px.a);
                            px.b = unprem(px.b, px.a);
                        });
                    },
                    _ => {},
                }
            }
            image
        } else {
            color
        })
    }

    fn color_convert(img: FrameTempRef<'_>) -> Result<Image> {
        let range = img.range();
        Ok(match img.rows_iter()? {
            RowsIters::YuvPlanes8 {y,u,v,chroma_sampling} => {
                let mc = img.matrix_coefficients().unwrap_or(color::MatrixCoefficients::BT709);
                let conv = yuv::convert::RGBConvert::<u8>::new(range, mc)?;
                let width = y.width();
                let height = y.height();
                let mut out = Vec::with_capacity(width * height);
                let mut tmp1;
                let mut tmp2;
                let mut tmp3;
                let px_iter: &mut dyn Iterator<Item=YUV<u8>> = match chroma_sampling {
                    color::ChromaSampling::Cs444 => {
                        tmp1 = yuv_444(y, u, v);
                        &mut tmp1
                    },
                    color::ChromaSampling::Cs420 => {
                        tmp2 = yuv_420(y, u, v);
                        &mut tmp2
                    },
                    color::ChromaSampling::Cs422 => {
                        tmp3 = yuv_422(y, u, v);
                        &mut tmp3
                    },
                    color::ChromaSampling::Monochrome => return Err(Error::Meta(yuv::Error::InvalidDepthRequested)),
                };
                out.extend(px_iter.map(|px| conv.to_rgb(px)));
                Image::Rgb8(ImgVec::new(out, width, height))
            },
            RowsIters::YuvPlanes16 {y,u,v,chroma_sampling, depth} => {
                let mc = img.matrix_coefficients().unwrap_or(color::MatrixCoefficients::BT709);
                let conv = yuv::convert::RGBConvert::<u16>::new(range, mc, depth)?;
                let width = y.width();
                let height = y.height();
                let mut out = Vec::with_capacity(width * height);
                let mut tmp1;
                let mut tmp2;
                let mut tmp3;
                let px_iter: &mut dyn Iterator<Item=YUV<[u8; 2]>> = match chroma_sampling {
                    color::ChromaSampling::Cs444 => {
                        tmp1 = yuv_444(y, u, v);
                        &mut tmp1
                    },
                    color::ChromaSampling::Cs420 => {
                        tmp2 = yuv_420(y, u, v);
                        &mut tmp2
                    },
                    color::ChromaSampling::Cs422 => {
                        tmp3 = yuv_422(y, u, v);
                        &mut tmp3
                    },
                    color::ChromaSampling::Monochrome => unreachable!(),
                };
                out.extend(px_iter.map(|px| conv.to_rgb(YUV{
                    y: u16::from_ne_bytes(px.y),
                    u: u16::from_ne_bytes(px.u),
                    v: u16::from_ne_bytes(px.v),
                })));
                Image::Rgb16(ImgVec::new(out, width, height))
            },
            gray_iters => {
                let mc = img.matrix_coefficients().unwrap_or(color::MatrixCoefficients::Identity);
                match Self::to_gray(range, mc, gray_iters)? {
                    AlphaImage::Gray8(img) => Image::Gray8(img),
                    AlphaImage::Gray16(img) => Image::Gray16(img),
                }
            },
        })
    }

    fn to_alpha(img: FrameTempRef<'_>) -> Result<AlphaImage> {
        let range = img.range();
        let mc = img.matrix_coefficients().unwrap_or(color::MatrixCoefficients::Identity);
        Ok(Self::to_gray(range, mc, img.rows_iter()?)?)
    }

    fn to_gray(range: aom_decode::color::Range, mc: color::MatrixCoefficients, iters: RowsIters) -> Result<AlphaImage, yuv::Error> {
        Ok(match iters {
            RowsIters::YuvPlanes8 {y,..} | RowsIters::Mono8(y) => {
                let conv = yuv::convert::RGBConvert::<u8>::new(range, mc)?;
                let width = y.width();
                let height = y.height();
                let mut out = Vec::with_capacity(width * height);
                out.extend(y.flat_map(|row| {
                    row.iter().copied().map(|y| {
                        GRAY8::new(conv.to_rgb(YUV{y,u:128,v:128}).g)
                    })
                }));
                AlphaImage::Gray8(ImgVec::new(out, width, height))
            },
            RowsIters::YuvPlanes16 {y, depth, ..} | RowsIters::Mono16(y, depth) => {
                let conv = yuv::convert::RGBConvert::<u16>::new(range, mc, depth)?;
                let width = y.width();
                let height = y.height();
                let mut out = Vec::with_capacity(width * height);
                out.extend(y.flat_map(|row| {
                    row.iter().copied().map(|y| {
                        let y = u16::from_ne_bytes(y);
                        GRAY16::new(conv.to_rgb(YUV{y,u:128*256+128,v:128*256+128}).g)
                    })
                }));
                AlphaImage::Gray16(ImgVec::new(out, width, height))
            },
        })
    }
}
