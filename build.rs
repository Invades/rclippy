use std::{env, error::Error, fs, path::PathBuf};

use tiny_skia::{Pixmap, Transform};

const ICON_SIZES: [u32; 6] = [32, 40, 48, 64, 96, 256];
const TRAY_SIZES: [u32; 5] = [32, 40, 48, 64, 96];
const MONO_WHITE: [u8; 3] = [255, 255, 255];
const FIT_ALPHA_THRESHOLD: u8 = 48;
const FIT_SCALE: f32 = 0.98;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=assets/rclippy.svg");

    let svg = fs::read("assets/rclippy.svg")?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let mono_svg = remove_mono_details(std::str::from_utf8(&svg)?)?;
    let mono_tree =
        resvg::usvg::Tree::from_data(mono_svg.as_bytes(), &resvg::usvg::Options::default())?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR unset")?);

    for size in ICON_SIZES {
        let rgba = render_svg(&tree, size)?;
        fs::write(out_dir.join(format!("rclippy-{size}.rgba")), rgba)?;
    }

    for size in TRAY_SIZES {
        let tray_rgba = render_svg(&mono_tree, size)?;
        fs::write(
            out_dir.join(format!("rclippy-{size}-mono.rgba")),
            monochrome_rgba(&tray_rgba, MONO_WHITE),
        )?;
    }

    Ok(())
}

fn render_svg(tree: &resvg::usvg::Tree, size: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let render_size = (size * 4).max(512);
    let rendered = render_svg_at(tree, render_size)?;
    let fitted = fit_content_to_icon(&rendered, render_size, size);

    Ok(unpremultiply_rgba(&fitted))
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
