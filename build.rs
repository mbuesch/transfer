use anyhow::{self as ah, format_err as err};
use std::{env, fs, io::Cursor, path::PathBuf};

fn convert_icon_to_rgba() -> ah::Result<()> {
    println!("cargo:rerun-if-changed=assets/icon.png");

    let bytes = fs::read("assets/icon.png")?;
    let decoder = png::Decoder::new(Cursor::new(bytes.as_slice()));
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| err!("icon.png: Unknown output buffer size"))?;
    let mut buf = vec![0_u8; size];
    let info = reader.next_frame(&mut buf)?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => return Err(err!("icon.png: Unsupported color type: {other:?}")),
    };

    // Header: width (u32 LE), height (u32 LE), then raw RGBA bytes.
    let width: u32 = info.width;
    let height: u32 = info.height;
    let mut out = Vec::with_capacity(4 + 4 + rgba.len());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&rgba);

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("icon.rgba"), out)?;
    Ok(())
}

fn main() -> ah::Result<()> {
    convert_icon_to_rgba()?;

    Ok(())
}
