use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use tiny_skia::{IntSize, Pixmap, Transform};

const ICON_SIZES: [u32; 6] = [32, 40, 48, 64, 96, 256];
const TRAY_SIZES: [u32; 5] = [32, 40, 48, 64, 96];
const WINDOWS_ICON_SIZES: [u32; 4] = [16, 32, 48, 256];
const MONO_WHITE: [u8; 3] = [255, 255, 255];
const FIT_ALPHA_THRESHOLD: u8 = 48;
const FIT_SCALE: f32 = 0.98;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=assets/rclippy.svg");
    println!("cargo:rerun-if-env-changed=RCLIPPY_INSTALLER_PAYLOAD");
    println!("cargo:rerun-if-env-changed=RCLIPPY_UNINSTALLER_PAYLOAD");

    let svg = fs::read("assets/rclippy.svg")?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let mono_svg = remove_mono_details(std::str::from_utf8(&svg)?)?;
    let mono_tree =
        resvg::usvg::Tree::from_data(mono_svg.as_bytes(), &resvg::usvg::Options::default())?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR unset")?);
    write_installer_payload(&out_dir)?;
    write_named_payload(
        &out_dir,
        "RCLIPPY_UNINSTALLER_PAYLOAD",
        "rclippy-uninstaller-payload.exe",
    )?;
    write_windows_icon(&out_dir, &tree)?;

    for size in ICON_SIZES {
        let rgba = render_svg(&tree, size)?;
        fs::write(out_dir.join(format!("rclippy-{size}.rgba")), rgba)?;
    }

    let package_icon_dir = PathBuf::from("target/package-icons");
    fs::create_dir_all(&package_icon_dir)?;
    for size in [16, 32, 128, 256, 512] {
        let premultiplied_rgba = render_svg_premultiplied(&tree, size)?;
        let icon = Pixmap::from_vec(
            premultiplied_rgba,
            IntSize::from_wh(size, size).ok_or("create package icon size")?,
        )
        .ok_or("create package icon pixmap")?;
        icon.save_png(package_icon_dir.join(format!("rclippy-{size}.png")))?;
    }

    for size in TRAY_SIZES {
        let tray_rgba = render_svg(&mono_tree, size)?;
        fs::write(
            out_dir.join(format!("rclippy-{size}-mono.rgba")),
            monochrome_rgba(&tray_rgba, MONO_WHITE),
        )?;
    }

    compile_windows_resources(&out_dir)?;

    Ok(())
}

fn write_installer_payload(out_dir: &Path) -> Result<(), Box<dyn Error>> {
    write_named_payload(
        out_dir,
        "RCLIPPY_INSTALLER_PAYLOAD",
        "rclippy-installer-payload.exe",
    )
}

fn write_named_payload(
    out_dir: &Path,
    env_name: &str,
    file_name: &str,
) -> Result<(), Box<dyn Error>> {
    let payload_path = out_dir.join(file_name);
    match env::var_os(env_name) {
        Some(path) => fs::copy(PathBuf::from(path), payload_path).map(|_| ())?,
        None => fs::write(payload_path, [])?,
    }
    Ok(())
}

fn write_windows_icon(out_dir: &Path, tree: &resvg::usvg::Tree) -> Result<(), Box<dyn Error>> {
    let mut icon_images = Vec::new();
    for size in WINDOWS_ICON_SIZES {
        icon_images.push((size, render_svg(tree, size)?));
    }

    let ico = encode_ico(&icon_images)?;
    fs::create_dir_all("target/package-icons")?;
    fs::write(out_dir.join("rclippy.ico"), &ico)?;
    fs::write(
        PathBuf::from("target/package-icons").join("rclippy.ico"),
        ico,
    )?;
    Ok(())
}

fn compile_windows_resources(out_dir: &Path) -> Result<(), Box<dyn Error>> {
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            .set_icon(out_dir.join("rclippy.ico").to_string_lossy().as_ref())
            .compile()?;
    }

    Ok(())
}

fn encode_ico(images: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, Box<dyn Error>> {
    let count = u16::try_from(images.len())?;
    let mut entries = Vec::new();
    let mut payloads = Vec::new();
    let mut offset = 6 + images.len() as u32 * 16;

    for (size, rgba) in images {
        let dib = encode_icon_dib(*size, rgba)?;
        entries.push((*size, dib.len() as u32, offset));
        offset += dib.len() as u32;
        payloads.push(dib);
    }

    let mut ico = Vec::new();
    push_u16(&mut ico, 0);
    push_u16(&mut ico, 1);
    push_u16(&mut ico, count);

    for (size, len, offset) in entries {
        ico.push(if size == 256 { 0 } else { size as u8 });
        ico.push(if size == 256 { 0 } else { size as u8 });
        ico.push(0);
        ico.push(0);
        push_u16(&mut ico, 1);
        push_u16(&mut ico, 32);
        push_u32(&mut ico, len);
        push_u32(&mut ico, offset);
    }

    for payload in payloads {
        ico.extend_from_slice(&payload);
    }

    Ok(ico)
}

fn encode_icon_dib(size: u32, rgba: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels_len = (size * size * 4) as usize;
    if rgba.len() != pixels_len {
        return Err("invalid icon rgba length".into());
    }

    let mask_stride = size.div_ceil(32) * 4;
    let mut dib = Vec::with_capacity(40 + pixels_len + (mask_stride * size) as usize);
    push_u32(&mut dib, 40);
    push_i32(&mut dib, size as i32);
    push_i32(&mut dib, (size * 2) as i32);
    push_u16(&mut dib, 1);
    push_u16(&mut dib, 32);
    push_u32(&mut dib, 0);
    push_u32(&mut dib, size * size * 4);
    push_i32(&mut dib, 0);
    push_i32(&mut dib, 0);
    push_u32(&mut dib, 0);
    push_u32(&mut dib, 0);

    for y in (0..size).rev() {
        for x in 0..size {
            let index = ((y * size + x) * 4) as usize;
            dib.extend_from_slice(&[
                rgba[index + 2],
                rgba[index + 1],
                rgba[index],
                rgba[index + 3],
            ]);
        }
    }

    dib.extend(std::iter::repeat_n(0, (mask_stride * size) as usize));
    Ok(dib)
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn render_svg(tree: &resvg::usvg::Tree, size: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(unpremultiply_rgba(&render_svg_premultiplied(tree, size)?))
}

fn render_svg_premultiplied(
    tree: &resvg::usvg::Tree,
    size: u32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let render_size = (size * 4).max(512);
    let rendered = render_svg_at(tree, render_size)?;
    let fitted = fit_content_to_icon(&rendered, render_size, size);

    Ok(fitted)
}

fn render_svg_at(tree: &resvg::usvg::Tree, size: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut pixmap = Pixmap::new(size, size).ok_or("create icon pixmap")?;
    let svg_size = tree.size();
    let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
    let x = (size as f32 - svg_size.width() * scale) / 2.0;
    let y = (size as f32 - svg_size.height() * scale) / 2.0;
    let transform = Transform::from_translate(x, y).pre_scale(scale, scale);

    resvg::render(tree, transform, &mut pixmap.as_mut());

    Ok(pixmap.data().to_vec())
}

fn fit_content_to_icon(data: &[u8], source_size: u32, target_size: u32) -> Vec<u8> {
    let Some((min_x, min_y, max_x, max_y)) = alpha_bounds(data, source_size) else {
        return vec![0; (target_size * target_size * 4) as usize];
    };

    let source_width = (max_x - min_x + 1) as f32;
    let source_height = (max_y - min_y + 1) as f32;
    let target_area = target_size as f32 * FIT_SCALE;
    let scale = (target_area / source_width).min(target_area / source_height);
    let fitted_width = source_width * scale;
    let fitted_height = source_height * scale;
    let offset_x = (target_size as f32 - fitted_width) / 2.0;
    let offset_y = (target_size as f32 - fitted_height) / 2.0;

    let mut fitted = vec![0; (target_size * target_size * 4) as usize];
    for y in 0..target_size {
        for x in 0..target_size {
            let source_x = (x as f32 + 0.5 - offset_x) / scale + min_x as f32 - 0.5;
            let source_y = (y as f32 + 0.5 - offset_y) / scale + min_y as f32 - 0.5;
            let pixel = sample_bilinear(data, source_size, source_x, source_y);
            let index = ((y * target_size + x) * 4) as usize;
            fitted[index..index + 4].copy_from_slice(&pixel);
        }
    }

    fitted
}

fn alpha_bounds(data: &[u8], size: u32) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = size;
    let mut min_y = size;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for y in 0..size {
        for x in 0..size {
            let alpha = data[((y * size + x) * 4 + 3) as usize];
            if alpha > FIT_ALPHA_THRESHOLD {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }

    found.then_some((min_x, min_y, max_x, max_y))
}

fn sample_bilinear(data: &[u8], size: u32, x: f32, y: f32) -> [u8; 4] {
    let x0 = x.floor();
    let y0 = y.floor();
    let x1 = x0 + 1.0;
    let y1 = y0 + 1.0;
    let tx = x - x0;
    let ty = y - y0;

    let p00 = pixel_at(data, size, x0 as i32, y0 as i32);
    let p10 = pixel_at(data, size, x1 as i32, y0 as i32);
    let p01 = pixel_at(data, size, x0 as i32, y1 as i32);
    let p11 = pixel_at(data, size, x1 as i32, y1 as i32);

    let mut out = [0; 4];
    for channel in 0..4 {
        let top = p00[channel] * (1.0 - tx) + p10[channel] * tx;
        let bottom = p01[channel] * (1.0 - tx) + p11[channel] * tx;
        out[channel] = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn pixel_at(data: &[u8], size: u32, x: i32, y: i32) -> [f32; 4] {
    if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
        return [0.0; 4];
    }

    let index = (((y as u32 * size + x as u32) * 4) as usize).min(data.len() - 4);
    [
        f32::from(data[index]),
        f32::from(data[index + 1]),
        f32::from(data[index + 2]),
        f32::from(data[index + 3]),
    ]
}

fn unpremultiply_rgba(data: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for pixel in data.chunks_exact(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if alpha == 255 {
            rgba.extend_from_slice(pixel);
        } else {
            rgba.push(unpremultiply_channel(pixel[0], alpha));
            rgba.push(unpremultiply_channel(pixel[1], alpha));
            rgba.push(unpremultiply_channel(pixel[2], alpha));
            rgba.push(alpha);
        }
    }
    rgba
}

fn unpremultiply_channel(value: u8, alpha: u8) -> u8 {
    ((u16::from(value) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
}

fn monochrome_rgba(data: &[u8], rgb: [u8; 3]) -> Vec<u8> {
    let mut mono = Vec::with_capacity(data.len());
    for pixel in data.chunks_exact(4) {
        mono.extend_from_slice(&[rgb[0], rgb[1], rgb[2], pixel[3]]);
    }
    mono
}

fn remove_mono_details(svg: &str) -> Result<String, Box<dyn Error>> {
    let without_sync_loop = remove_group(
        svg,
        r##"<g fill="none" stroke="#aaffaa" stroke-width="22" stroke-linecap="round" stroke-linejoin="round">"##,
    )?;
    let without_first_arrow = remove_path(&without_sync_loop, r#"<path d="M 410 250"#)?;
    let without_second_arrow = remove_path(&without_first_arrow, r#"<path d="M 100 260"#)?;
    let without_eyes = remove_group(&without_second_arrow, r#"<g filter="url(#eyeShadow)">"#)?;
    remove_group(&without_eyes, r#"<g fill="none" stroke-linecap="round">"#)
}

fn remove_group(svg: &str, start_marker: &str) -> Result<String, Box<dyn Error>> {
    let start = svg.find(start_marker).ok_or("mono detail group missing")?;
    let after_start = start + start_marker.len();
    let end = svg[after_start..]
        .find("</g>")
        .map(|offset| after_start + offset + "</g>".len())
        .ok_or("mono detail group end missing")?;

    let mut output = String::with_capacity(svg.len() - (end - start));
    output.push_str(&svg[..start]);
    output.push_str(&svg[end..]);
    Ok(output)
}

fn remove_path(svg: &str, start_marker: &str) -> Result<String, Box<dyn Error>> {
    let start = svg.find(start_marker).ok_or("mono detail path missing")?;
    let end = svg[start..]
        .find("/>")
        .map(|offset| start + offset + "/>".len())
        .ok_or("mono detail path end missing")?;

    let mut output = String::with_capacity(svg.len() - (end - start));
    output.push_str(&svg[..start]);
    output.push_str(&svg[end..]);
    Ok(output)
}
