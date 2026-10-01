//! Renders the tray icon: up to three characters of text, colour-coded, as an
//! RGBA bitmap, plus an encoder for the Windows ICO container.

use ab_glyph::{point, Font, FontRef, Glyph, PxScale, ScaleFont};

/// Subset of DejaVu Sans Bold containing just the characters we draw.
static FONT_DATA: &[u8] = include_bytes!("../assets/digits.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const GOOD: Rgb = Rgb(0x3f, 0xc3, 0x5f);
pub const WARN: Rgb = Rgb(0xff, 0xb0, 0x1f);
pub const BAD: Rgb = Rgb(0xf0, 0x40, 0x3c);
pub const NEUTRAL: Rgb = Rgb(0xa0, 0xa8, 0xb0);

/// A square RGBA image, straight (non-premultiplied) alpha, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub size: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    fn new(size: u32) -> Self {
        Image {
            size,
            rgba: vec![0; (size * size * 4) as usize],
        }
    }

    fn blend(&mut self, x: u32, y: u32, color: Rgb, alpha: f32) {
        if x >= self.size || y >= self.size || alpha <= 0.0 {
            return;
        }
        let a = alpha.min(1.0);
        let i = ((y * self.size + x) * 4) as usize;
        let px = &mut self.rgba[i..i + 4];
        let da = px[3] as f32 / 255.0;
        let out_a = a + da * (1.0 - a);
        if out_a <= 0.0 {
            return;
        }
        let mix = |s: u8, d: u8| -> u8 {
            ((s as f32 * a + d as f32 * da * (1.0 - a)) / out_a)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        px[0] = mix(color.0, px[0]);
        px[1] = mix(color.1, px[1]);
        px[2] = mix(color.2, px[2]);
        px[3] = (out_a * 255.0).round() as u8;
    }
}

/// Minimum horizontal squeeze applied to fit wide strings (x scale / y scale).
const MIN_ASPECT: f32 = 0.58;

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_DATA).expect("embedded font is valid")
}

/// Lay out `text` so its ink fits inside a `w` x `h` box, returning positioned glyphs
/// and the ink bounds (min_x, min_y, max_x, max_y) in pixels.
fn layout(font: &FontRef<'static>, text: &str, w: f32, h: f32) -> Vec<Glyph> {
    // Measure at a reference scale to learn the natural ink box.
    let reference = 100.0;
    let (rw, rh) = ink_size(
        font,
        text,
        PxScale {
            x: reference,
            y: reference,
        },
    );
    if rw <= 0.0 || rh <= 0.0 {
        return Vec::new();
    }
    let mut sy = h / rh * reference;
    let mut sx = w / rw * reference;
    if sx > sy {
        sx = sy;
    }
    if sx / sy < MIN_ASPECT {
        sy = sx / MIN_ASPECT;
    }
    let scale = PxScale { x: sx, y: sy };
    position_glyphs(font, text, scale)
}

fn position_glyphs(font: &FontRef<'static>, text: &str, scale: PxScale) -> Vec<Glyph> {
    let scaled = font.as_scaled(scale);
    let mut x = 0.0;
    let mut glyphs = Vec::new();
    let mut last = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = last {
            x += scaled.kern(prev, id);
        }
        glyphs.push(id.with_scale_and_position(scale, point(x, 0.0)));
        x += scaled.h_advance(id);
        last = Some(id);
    }
    glyphs
}

fn ink_bounds(font: &FontRef<'static>, glyphs: &[Glyph]) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    for g in glyphs {
        if let Some(o) = font.outline_glyph(g.clone()) {
            let r = o.px_bounds();
            b = Some(match b {
                None => (r.min.x, r.min.y, r.max.x, r.max.y),
                Some((x0, y0, x1, y1)) => (
                    x0.min(r.min.x),
                    y0.min(r.min.y),
                    x1.max(r.max.x),
                    y1.max(r.max.y),
                ),
            });
        }
    }
    b
}

fn ink_size(font: &FontRef<'static>, text: &str, scale: PxScale) -> (f32, f32) {
    let glyphs = position_glyphs(font, text, scale);
    match ink_bounds(font, &glyphs) {
        Some((x0, y0, x1, y1)) => (x1 - x0, y1 - y0),
        None => (0.0, 0.0),
    }
}

/// Rasterise `text` centred in a `size` x `size` coverage mask (values 0..=1),
/// keeping `margin` pixels clear on every side.
fn text_mask(text: &str, size: u32, margin: f32) -> Vec<f32> {
    let font = font();
    let mut mask = vec![0.0f32; (size * size) as usize];
    let avail = size as f32 - 2.0 * margin;
    let glyphs = layout(&font, text, avail, avail);
    let Some((x0, y0, x1, y1)) = ink_bounds(&font, &glyphs) else {
        return mask;
    };
    // Offset so that the ink box is centred in the icon, snapped to whole pixels
    // so vertical strokes stay crisp.
    let dx = ((size as f32 - (x1 - x0)) / 2.0 - x0).round();
    let dy = ((size as f32 - (y1 - y0)) / 2.0 - y0).round();
    for g in glyphs {
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|px, py, c| {
                let x = b.min.x + px as f32 + dx;
                let y = b.min.y + py as f32 + dy;
                if x >= 0.0 && y >= 0.0 && (x as u32) < size && (y as u32) < size {
                    let i = (y as u32 * size + x as u32) as usize;
                    mask[i] = (mask[i] + c).min(1.0);
                }
            });
        }
    }
    mask
}

/// Render the icon: `text` on a filled rounded square of `color`.
/// White ink on green/red, dark ink on amber for contrast.
pub fn render(text: &str, color: Rgb, size: u32) -> Image {
    let mut img = Image::new(size);
    let radius = (size as f32 * 0.2).max(1.0);
    for y in 0..size {
        for x in 0..size {
            img.blend(x, y, color, rounded_rect_coverage(x, y, size, radius));
        }
    }
    let mask = text_mask(text, size, (size as f32 * 0.08).max(1.0));
    let ink = if color == WARN {
        Rgb(0x20, 0x20, 0x20)
    } else {
        Rgb(255, 255, 255)
    };
    for y in 0..size {
        for x in 0..size {
            img.blend(x, y, ink, mask[(y * size + x) as usize]);
        }
    }
    img
}

/// Anti-aliased coverage of a rounded square filling the whole icon.
fn rounded_rect_coverage(x: u32, y: u32, size: u32, radius: f32) -> f32 {
    let s = size as f32;
    let cx = x as f32 + 0.5;
    let cy = y as f32 + 0.5;
    // Distance from the pixel centre to the rounded-rect edge (negative inside).
    let qx = (cx - s / 2.0).abs() - (s / 2.0 - radius);
    let qy = (cy - s / 2.0).abs() - (s / 2.0 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    let d = outside + inside - radius;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Encode one or more square RGBA images as a Windows ICO file (32-bit BMP entries).
/// Only used to generate the exe icon checked in under `assets/`.
#[cfg(test)]
pub fn encode_ico(images: &[Image]) -> Vec<u8> {
    let mut out = Vec::new();
    let count = images.len() as u16;
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&count.to_le_bytes());

    let mut bodies: Vec<Vec<u8>> = Vec::new();
    for img in images {
        bodies.push(encode_dib(img));
    }
    let mut offset = 6 + 16 * images.len() as u32;
    for (img, body) in images.iter().zip(&bodies) {
        let dim = |n: u32| if n >= 256 { 0u8 } else { n as u8 };
        out.push(dim(img.size));
        out.push(dim(img.size));
        out.push(0); // palette entries
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += body.len() as u32;
    }
    for body in bodies {
        out.extend_from_slice(&body);
    }
    out
}

/// BITMAPINFOHEADER + bottom-up BGRA pixels + AND mask, as used inside ICO files.
#[cfg(test)]
fn encode_dib(img: &Image) -> Vec<u8> {
    let s = img.size;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(s as i32).to_le_bytes());
    out.extend_from_slice(&((s * 2) as i32).to_le_bytes()); // height includes the AND mask
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(s * s * 4).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution + colours used/important
    for y in (0..s).rev() {
        for x in 0..s {
            let i = ((y * s + x) * 4) as usize;
            let p = &img.rgba[i..i + 4];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    // 1-bit AND mask, rows padded to 32 bits. All zero: transparency comes from alpha.
    let row_bytes = s.div_ceil(32) * 4;
    out.extend(std::iter::repeat_n(0u8, (row_bytes * s) as usize));
    out
}

/// Premultiplied BGRA bytes, top-down, as needed for a 32-bit DIB section on Windows.
pub fn to_premultiplied_bgra(img: &Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.rgba.len());
    for p in img.rgba.as_chunks::<4>().0 {
        let a = p[3] as u32;
        let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        out.extend_from_slice(&[pm(p[2]), pm(p[1]), pm(p[0]), p[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn out_dir() -> PathBuf {
        let dir = std::env::var("PING_AGENT_ICON_DUMP")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ping-agent-icons"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(img: &Image, scale: u32, path: &std::path::Path) {
        let s = img.size * scale;
        let mut data = Vec::with_capacity((s * s * 4) as usize);
        for y in 0..s {
            for x in 0..s {
                let i = (((y / scale) * img.size + x / scale) * 4) as usize;
                data.extend_from_slice(&img.rgba[i..i + 4]);
            }
        }
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), s, s);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&data).unwrap();
    }

    #[test]
    fn renders_something_visible() {
        for size in [16, 20, 24, 32] {
            for text in ["7", "42", "123", "999", "X", "?", "..."] {
                let img = render(text, GOOD, size);
                assert_eq!(img.rgba.len(), (size * size * 4) as usize);
                assert!(
                    img.rgba.chunks(4).any(|p| p[3] > 0),
                    "{text}@{size} is blank"
                );
            }
        }
    }

    #[test]
    fn ico_has_valid_header() {
        let imgs = vec![render("12", GOOD, 16), render("12", GOOD, 32)];
        let ico = encode_ico(&imgs);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 32);
        let first_offset = u32::from_le_bytes(ico[18..22].try_into().unwrap());
        assert_eq!(first_offset, 6 + 32);
        let first_len = u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize;
        assert_eq!(first_len, 40 + 16 * 16 * 4 + 4 * 16);
        assert_eq!(ico.len(), 38 + first_len + 40 + 32 * 32 * 4 + 4 * 32);
    }

    #[test]
    fn premultiplied_bgra_swaps_channels() {
        let img = Image {
            size: 1,
            rgba: vec![255, 0, 0, 128],
        };
        assert_eq!(to_premultiplied_bgra(&img), vec![0, 0, 128, 128]);
    }

    /// Not an assertion test: dumps a magnified contact sheet of sample icons so a
    /// human can eyeball legibility. Set PING_AGENT_ICON_DUMP to choose the folder.
    #[test]
    fn dump_samples() {
        let dir = out_dir();
        let samples = [
            ("<1", GOOD),
            ("7", GOOD),
            ("42", GOOD),
            ("88", WARN),
            ("123", BAD),
            ("999", BAD),
            ("X", BAD),
            ("?", BAD),
            ("...", NEUTRAL),
        ];
        let sizes = [16u32, 20, 24, 32];
        let cell = 72u32;
        let w = cell * samples.len() as u32;
        let h = cell * sizes.len() as u32;
        let mut sheet = vec![0u8; (w * h * 4) as usize];
        for (r, size) in sizes.iter().enumerate() {
            for (c, (text, color)) in samples.iter().enumerate() {
                let img = render(text, *color, *size);
                let mag = 64 / size;
                for y in 0..cell {
                    for x in 0..cell {
                        let bg = if x < cell / 2 {
                            [0x20, 0x20, 0x20]
                        } else {
                            [0xf3, 0xf3, 0xf3]
                        };
                        let (sx, sy) = (x as i32 - 4, y as i32 - 4);
                        let mut px = [bg[0], bg[1], bg[2], 255u8];
                        if sx >= 0
                            && sy >= 0
                            && (sx as u32) < size * mag
                            && (sy as u32) < size * mag
                        {
                            let i = (((sy as u32 / mag) * size + sx as u32 / mag) * 4) as usize;
                            let p = &img.rgba[i..i + 4];
                            let a = p[3] as f32 / 255.0;
                            for k in 0..3 {
                                px[k] = (p[k] as f32 * a + bg[k] as f32 * (1.0 - a)).round() as u8;
                            }
                        }
                        let o =
                            ((((r as u32) * cell + y) * w) + (c as u32) * cell + x) as usize * 4;
                        sheet[o..o + 4].copy_from_slice(&px);
                    }
                }
            }
        }
        write_png_raw(&sheet, w, h, &dir.join("sheet.png"));
        write_png(&render("123", BAD, 16), 1, &dir.join("123_16.png"));
    }

    /// Regenerates `assets/app.ico` (the exe's own icon). Run with
    /// `cargo test write_app_icon -- --ignored`.
    #[test]
    #[ignore]
    fn write_app_icon() {
        let images: Vec<Image> = [16u32, 20, 24, 32, 48, 64, 128]
            .iter()
            .map(|s| render("ms", GOOD, *s))
            .collect();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::write(root.join("assets/app.ico"), encode_ico(&images)).unwrap();
        write_png(&images[5], 1, &out_dir().join("app_64.png"));
    }

    fn write_png_raw(rgba: &[u8], w: u32, h: u32, path: &std::path::Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(rgba).unwrap();
    }
}
