use rgb::ComponentMap;
use std::path::Path;
use std::path::PathBuf;
use avif_decode::*;

fn main() {
    let (input_path, output_path) = match parse_args() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}\nusage: avif_decode input.avif output.png", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = run(&input_path, &output_path) {
        eprintln!("error ({}): {}", input_path.display(), e);
        std::process::exit(1);
    }
}

fn parse_args() -> Result<(PathBuf, PathBuf), &'static str> {
    let mut input_path = None;
    let mut output_path = None;
    let mut force = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg {
            a if a == "-v" || a == "--version" => {
                return Err(env!("CARGO_PKG_VERSION"));
            },
            a if a == "-h" || a == "--help" => {
                return Err("");
            },
            a if a == "-f" => {
                force = true;
            },
            a if a == "-o" => {
                if output_path.is_some() {
                    Err("Output specified more than once")?;
                }
                output_path = Some(args.next().ok_or("-o needs a value")?);
            },
            path if input_path.is_none() => {
                input_path = Some(path);
            },
            path if output_path.is_none() => {
                output_path = Some(path);
            },
            _ => Err("Too many arguments. Only input and output path expected")?,
        }
    }
    let input_path = PathBuf::from(input_path.ok_or("Missing input path")?);
    let output_path = args.next().map(PathBuf::from).unwrap_or_else(|| input_path.with_extension("png"));

    if !force && output_path.exists() {
        return Err(if output_path.extension().unwrap_or("".as_ref()) != "png" {
            "output file must be .png. Multiple AVIF input files are not supported."
        } else {
            "output file already exists"
        });
    }
    Ok((input_path, output_path))
}

fn run(input_path: &Path, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {

    let data = std::fs::read(input_path).map_err(|e| format!("Unable to read '{}', because: {}", input_path.display(), e))?;

    let d = Decoder::from_avif(&data)?;
    let encoded = match d.to_image()? {
        Image::Rgb8(img) => {
            let (buf, width, height) = img.into_contiguous_buf();
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::RGB, 8)
        },
        Image::Rgb16(img) => {
            let (mut buf, width, height) = img.into_contiguous_buf();
            buf.iter_mut().for_each(|px| {
                *px = px.map(|c| u16::from_ne_bytes(c.to_be_bytes()));
            });
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::RGB, 16)
        },
        Image::Rgba8(img) => {
            let (buf, width, height) = img.into_contiguous_buf();
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::RGBA, 8)
        },
        Image::Rgba16(img) => {
            let (mut buf, width, height) = img.into_contiguous_buf();
            buf.iter_mut().for_each(|px| {
                *px = px.map(|c| u16::from_ne_bytes(c.to_be_bytes()));
            });
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::RGBA, 16)
        },
        Image::Gray8(img) => {
            let (buf, width, height) = img.into_contiguous_buf();
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::GREY, 8)
        },
        Image::Gray16(img) => {
            let (mut buf, width, height) = img.into_contiguous_buf();
            buf.iter_mut().for_each(|px| {
                *px = px.map(|c| u16::from_ne_bytes(c.to_be_bytes()));
            });
            lodepng::encode_memory(&buf, width, height, lodepng::ColorType::GREY, 16)
        },
    }?;
    std::fs::write(output_path, encoded).map_err(|e| format!("Unable to write '{}', because: {}", output_path.display(), e))?;
    Ok(())
}
