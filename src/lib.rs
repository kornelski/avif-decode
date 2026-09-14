use aom_decode::avif::Avif;
use aom_decode::Config;
use imgref::ImgVec;
use rgb::*;
use std::io;

use quick_error::quick_error;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: io::Error) {
            display("{}", err)
            from()
        }
        Decode(err: aom_decode::Error) {
            display("{}", err)
            from()
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub enum Image {
    Rgb8(ImgVec<Rgb<u8>>),
    Rgb16(ImgVec<Rgb<u16>>),
    Rgba8(ImgVec<Rgba<u8>>),
    Rgba16(ImgVec<Rgba<u16>>),
    Gray8(ImgVec<Gray<u8>>),
    Gray16(ImgVec<Gray<u16>>),
}


pub struct Decoder {
    avif: Avif,
}

impl Decoder {
    #[inline(always)]
    pub fn from_avif(mut data: &[u8]) -> Result<Self> {
        Self::from_reader(&mut data)
    }

    #[inline]
    pub fn from_reader<R: io::Read>(reader: &mut R) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Ok(Self {
            avif: Avif::decode(&data, &Config {
                threads: std::thread::available_parallelism().map_or(4, |a| a.get()).min(32),
            })?,
        })
    }

    pub fn to_image(mut self) -> Result<Image> {
        Ok(match self.avif.convert()? {
            aom_decode::avif::Image::RGB8(img) => Image::Rgb8(img),
            aom_decode::avif::Image::RGBA8(img) => Image::Rgba8(img),
            aom_decode::avif::Image::RGB16(img) => Image::Rgb16(img),
            aom_decode::avif::Image::RGBA16(img) => Image::Rgba16(img),
            aom_decode::avif::Image::Gray8(img) => {
                let (buf, width, height) = img.into_contiguous_buf();
                Image::Gray8(ImgVec::new(buf.into_iter().map(Gray::new).collect(), width, height))
            },
            aom_decode::avif::Image::Gray16(img) => {
                let (buf, width, height) = img.into_contiguous_buf();
                Image::Gray16(ImgVec::new(buf.into_iter().map(Gray::new).collect(), width, height))
            },
        })
    }
}
