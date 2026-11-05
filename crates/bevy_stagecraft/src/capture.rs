//! Frames as pixels, and the files made from them: PNG, GIF and filmstrips.
//!
//! Nothing here touches the GPU, so it is tested without one. The filmstrip
//! draws its own marks and numbers from shapes, because a CPU rasteriser with
//! a font would be a heavy dependency for a tick and a cross; the words of a
//! caption are already in the frames, drawn by the cursor overlay.

use alloc::vec::Vec;
use core::fmt;
use std::io::Cursor;

use bevy::{
    image::{Image, TextureFormatPixelInfo},
    render::render_resource::TextureFormat,
};
use image::{
    ColorType, Delay, ImageEncoder, RgbaImage,
    codecs::{
        gif::{GifEncoder, Repeat},
        png::PngEncoder,
    },
};

/// Bytes in one RGBA pixel.
const CHANNELS: usize = 4;
/// The cell background of a filmstrip, behind and between thumbnails.
const STRIP_BACKGROUND: [u8; 4] = [24, 24, 28, 255];
/// The band under a thumbnail whose scenario had no failure so far.
const PASS_BAND: [u8; 4] = [36, 92, 52, 255];
/// The band under a thumbnail whose scenario had failed by then.
const FAIL_BAND: [u8; 4] = [128, 36, 36, 255];
/// The band under a thumbnail with nothing to say.
const NEUTRAL_BAND: [u8; 4] = [52, 52, 60, 255];
/// The colour of marks and numbers on a band.
const INK: [u8; 4] = [245, 245, 245, 255];
/// Height of the band under each thumbnail, in pixels.
const BAND_HEIGHT: u32 = 28;
/// Gap between cells of a filmstrip, in pixels.
const GAP: u32 = 6;
/// Side of the box a mark is drawn in, in pixels.
const MARK_SIZE: u32 = 18;
/// Padding inside a band, in pixels.
const BAND_PADDING: u32 = 5;
/// Size of one pixel of the digit font, in pixels.
const DIGIT_SCALE: u32 = 3;
/// Width of a digit in the digit font, in font pixels.
const DIGIT_WIDTH: u32 = 3;
/// Height of a digit in the digit font, in font pixels.
const DIGIT_HEIGHT: u32 = 5;
/// Thickness of the strokes of a mark, in pixels.
const STROKE: i32 = 2;

/// Why a frame could not be read or written.
#[derive(Debug)]
pub enum CaptureError {
    /// The image has no pixel data, or in a format this module cannot read.
    UnsupportedImage(String),
    /// The pixel data does not match the size.
    BadSize,
    /// Encoding failed.
    Encode(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedImage(why) => write!(f, "unsupported image: {why}"),
            Self::BadSize => write!(f, "pixel data does not match the frame size"),
            Self::Encode(why) => write!(f, "encoding failed: {why}"),
        }
    }
}

impl core::error::Error for CaptureError {}

/// One picture: tightly packed 8-bit RGBA, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// `width * height * 4` bytes.
    rgba: Vec<u8>,
}

/// The index of a pixel's first byte.
const fn offset(x: u32, y: u32, width: u32) -> usize {
    (y as usize * width as usize + x as usize) * CHANNELS
}

/// `value * numerator / denominator`, without overflowing `u32` on the way.
fn scale(value: u32, numerator: u32, denominator: u32) -> u32 {
    let scaled = u64::from(value) * u64::from(numerator) / u64::from(denominator.max(1));
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

impl Frame {
    /// A frame from RGBA bytes, or `None` if the length does not fit the size.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(CHANNELS)?;
        (rgba.len() == expected).then_some(Self {
            width,
            height,
            rgba,
        })
    }

    /// A frame of one colour.
    #[must_use]
    pub fn filled(width: u32, height: u32, colour: [u8; 4]) -> Self {
        let rgba = colour
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * CHANNELS)
            .collect();
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Reads a rendered or loaded Bevy image (8-bit RGBA or BGRA).
    ///
    /// # Errors
    ///
    /// Fails if the image has no data on the CPU, or another pixel format.
    pub fn from_image(image: &Image) -> Result<Self, CaptureError> {
        let size = image.texture_descriptor.size;
        let data = image
            .data
            .as_ref()
            .ok_or_else(|| CaptureError::UnsupportedImage("no pixel data".into()))?;
        let format = image.texture_descriptor.format;
        let swap = match format {
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => false,
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => true,
            other => {
                return Err(CaptureError::UnsupportedImage(format!(
                    "format {other:?} ({} bytes a pixel)",
                    other.pixel_size().unwrap_or(0)
                )));
            }
        };
        let mut rgba = data.clone();
        if swap {
            for pixel in rgba.as_chunks_mut::<CHANNELS>().0 {
                pixel.swap(0, 2);
            }
        }
        // The alpha of a screenshot is not meaningful; a recording is opaque.
        for pixel in rgba.as_chunks_mut::<CHANNELS>().0 {
            if let Some(alpha) = pixel.get_mut(3) {
                *alpha = u8::MAX;
            }
        }
        Self::new(size.width, size.height, rgba).ok_or(CaptureError::BadSize)
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, `width * height * 4` bytes, top row first.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The colour at `(x, y)`, or `None` outside the frame.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let start = offset(x, y, self.width);
        let bytes = self.rgba.get(start..start + CHANNELS)?;
        <[u8; 4]>::try_from(bytes).ok()
    }

    /// Sets the pixel at `(x, y)`; outside the frame it does nothing.
    fn put(&mut self, x: i32, y: i32, colour: [u8; 4]) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if x >= self.width || y >= self.height {
            return;
        }
        let start = offset(x, y, self.width);
        if let Some(target) = self.rgba.get_mut(start..start + CHANNELS) {
            target.copy_from_slice(&colour);
        }
    }

    /// Fills a rectangle, clipped to the frame.
    fn fill_rect(&mut self, left: u32, top: u32, width: u32, height: u32, colour: [u8; 4]) {
        for y in top..top.saturating_add(height).min(self.height) {
            for x in left..left.saturating_add(width).min(self.width) {
                let start = offset(x, y, self.width);
                if let Some(target) = self.rgba.get_mut(start..start + CHANNELS) {
                    target.copy_from_slice(&colour);
                }
            }
        }
    }

    /// Copies `source` onto this frame with its top-left corner at `(left, top)`.
    fn blit(&mut self, source: &Self, left: u32, top: u32) {
        for y in 0..source.height {
            let target_y = top + y;
            if target_y >= self.height {
                break;
            }
            let width = source.width.min(self.width.saturating_sub(left));
            let from = offset(0, y, source.width);
            let to = offset(left, target_y, self.width);
            let len = width as usize * CHANNELS;
            if let (Some(row), Some(target)) = (
                source.rgba.get(from..from + len),
                self.rgba.get_mut(to..to + len),
            ) {
                target.copy_from_slice(row);
            }
        }
    }

    /// This frame scaled down to `width` pixels across, keeping the aspect
    /// ratio, by averaging the pixels that fall in each new one. A frame that
    /// is not wider than `width` is returned as it is.
    #[must_use]
    pub fn scaled_to_width(&self, width: u32) -> Self {
        if width == 0 || self.width <= width || self.height == 0 {
            return self.clone();
        }
        let height = scale(self.height, width, self.width).max(1);
        let mut rgba = Vec::with_capacity(width as usize * height as usize * CHANNELS);
        for y in 0..height {
            let y0 = scale(y, self.height, height);
            let y1 = scale(y + 1, self.height, height);
            for x in 0..width {
                let x0 = scale(x, self.width, width);
                let x1 = scale(x + 1, self.width, width);
                let mut sums = [0_u64; CHANNELS];
                let mut count = 0_u64;
                for sy in y0..y1.max(y0 + 1) {
                    for sx in x0..x1.max(x0 + 1) {
                        if let Some(pixel) = self.pixel(sx, sy) {
                            for (sum, value) in sums.iter_mut().zip(pixel) {
                                *sum += u64::from(value);
                            }
                            count += 1;
                        }
                    }
                }
                let count = count.max(1);
                for sum in sums {
                    rgba.push(u8::try_from((sum + count / 2) / count).unwrap_or(u8::MAX));
                }
            }
        }
        Self {
            width,
            height,
            rgba,
        }
    }

    /// The frame as a PNG file.
    ///
    /// # Errors
    ///
    /// Fails if the encoder does.
    pub fn to_png(&self) -> Result<Vec<u8>, CaptureError> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&self.rgba, self.width, self.height, ColorType::Rgba8.into())
            .map_err(|error| CaptureError::Encode(error.to_string()))?;
        Ok(bytes)
    }
}

/// An animated GIF of `frames`, each shown for its delay in milliseconds.
/// Every frame must be the same size. The GIF loops forever.
///
/// # Errors
///
/// Fails if a frame does not fit the first one's size, or the encoder fails.
pub fn encode_gif(frames: &[(Frame, u32)]) -> Result<Vec<u8>, CaptureError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(Cursor::new(&mut bytes), GIF_SPEED);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|error| CaptureError::Encode(error.to_string()))?;
        for (frame, delay_ms) in frames {
            let image = RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
                .ok_or(CaptureError::BadSize)?;
            encoder
                .encode_frame(image::Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(*delay_ms, 1),
                ))
                .map_err(|error| CaptureError::Encode(error.to_string()))?;
        }
    }
    Ok(bytes)
}

/// The GIF encoder's colour quantiser speed, `1` (best) to `30` (fastest).
const GIF_SPEED: i32 = 20;

/// What a filmstrip says about the moment a frame was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Nothing had failed yet: a tick.
    Pass,
    /// A step or check had failed: a cross.
    Fail,
    /// No statement: a dot.
    Neutral,
}

/// One cell of a filmstrip.
#[derive(Debug, Clone)]
pub struct FilmstripCell {
    /// The picture, already scaled.
    pub frame: Frame,
    /// The mark under it.
    pub mark: Mark,
    /// The frame number written next to the mark.
    pub number: u32,
}

/// Lays `cells` out in rows of up to `columns` under each other, each
/// picture with a band carrying its mark and its frame number. All pictures
/// should be the same size. `None` when there are no cells.
#[must_use]
pub fn filmstrip(cells: &[FilmstripCell], columns: usize) -> Option<Frame> {
    let first = cells.first()?;
    let columns = columns.max(1).min(cells.len());
    let rows = cells.len().div_ceil(columns);
    let cell_width = first.frame.width;
    let cell_height = first.frame.height + BAND_HEIGHT;
    let columns_u32 = u32::try_from(columns).ok()?;
    let rows_u32 = u32::try_from(rows).ok()?;
    let width = columns_u32 * (cell_width + GAP) + GAP;
    let height = rows_u32 * (cell_height + GAP) + GAP;
    let mut strip = Frame::filled(width, height, STRIP_BACKGROUND);
    for (index, cell) in cells.iter().enumerate() {
        let column = u32::try_from(index % columns).ok()?;
        let row = u32::try_from(index / columns).ok()?;
        let left = GAP + column * (cell_width + GAP);
        let top = GAP + row * (cell_height + GAP);
        strip.blit(&cell.frame, left, top);
        let band_top = top + cell.frame.height;
        let band = match cell.mark {
            Mark::Pass => PASS_BAND,
            Mark::Fail => FAIL_BAND,
            Mark::Neutral => NEUTRAL_BAND,
        };
        strip.fill_rect(left, band_top, cell_width, BAND_HEIGHT, band);
        let mark_left = left + BAND_PADDING;
        let mark_top = band_top + (BAND_HEIGHT - MARK_SIZE) / 2;
        draw_mark(&mut strip, cell.mark, mark_left, mark_top);
        let number_left = mark_left + MARK_SIZE + BAND_PADDING * 2;
        let number_top = band_top + (BAND_HEIGHT - DIGIT_HEIGHT * DIGIT_SCALE) / 2;
        draw_number(&mut strip, cell.number, number_left, number_top);
    }
    Some(strip)
}

/// Draws a thick line from `(x0, y0)` to `(x1, y1)`.
fn draw_line(frame: &mut Frame, from: (i32, i32), to: (i32, i32), colour: [u8; 4]) {
    let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs()).max(1);
    for step in 0..=steps {
        let x = from.0 + (to.0 - from.0) * step / steps;
        let y = from.1 + (to.1 - from.1) * step / steps;
        for dy in 0..STROKE {
            for dx in 0..STROKE {
                frame.put(x + dx, y + dy, colour);
            }
        }
    }
}

/// Draws a tick, a cross or a dot in the box at `(left, top)`.
fn draw_mark(frame: &mut Frame, mark: Mark, left: u32, top: u32) {
    let (Ok(left), Ok(top), Ok(size)) = (
        i32::try_from(left),
        i32::try_from(top),
        i32::try_from(MARK_SIZE),
    ) else {
        return;
    };
    let at = |x: i32, y: i32| (left + x, top + y);
    match mark {
        Mark::Pass => {
            draw_line(frame, at(1, size / 2), at(size / 3 + 1, size - 3), INK);
            draw_line(frame, at(size / 3 + 1, size - 3), at(size - 3, 2), INK);
        }
        Mark::Fail => {
            draw_line(frame, at(2, 2), at(size - 3, size - 3), INK);
            draw_line(frame, at(size - 3, 2), at(2, size - 3), INK);
        }
        Mark::Neutral => {
            let middle = size / 2;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    frame.put(left + middle + dx, top + middle + dy, INK);
                }
            }
        }
    }
}

/// The digits 0 to 9, three columns by five rows, one row per entry, the
/// leftmost column in the highest of the three bits.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Draws `number` in decimal with its top-left corner at `(left, top)`.
fn draw_number(frame: &mut Frame, number: u32, left: u32, top: u32) {
    let text = number.to_string();
    let mut pen = left;
    for digit in text.chars().filter_map(|c| c.to_digit(10)) {
        let Some(rows) = DIGITS.get(digit as usize) else {
            continue;
        };
        for (row, bits) in (0_u32..).zip(rows) {
            for column in 0..DIGIT_WIDTH {
                if bits >> (DIGIT_WIDTH - 1 - column) & 1 == 1 {
                    let x = pen + column * DIGIT_SCALE;
                    let y = top + row * DIGIT_SCALE;
                    frame.fill_rect(x, y, DIGIT_SCALE, DIGIT_SCALE, INK);
                }
            }
        }
        pen += (DIGIT_WIDTH + 1) * DIGIT_SCALE;
    }
}

#[cfg(test)]
mod tests {
    use image::{AnimationDecoder, codecs::gif::GifDecoder};

    use super::*;

    /// A frame whose pixels are `colour`.
    fn flat(width: u32, height: u32, colour: [u8; 4]) -> Frame {
        Frame::filled(width, height, colour)
    }

    #[test]
    fn a_frame_rejects_pixel_data_of_the_wrong_length() {
        assert!(
            Frame::new(2, 2, vec![0; 15]).is_none(),
            "15 bytes is not 2x2 RGBA"
        );
        assert!(
            Frame::new(2, 2, vec![0; 16]).is_some(),
            "16 bytes is 2x2 RGBA"
        );
    }

    #[test]
    fn scaling_down_averages_the_pixels_it_covers() {
        let mut frame = flat(4, 2, [0, 0, 0, 255]);
        frame.fill_rect(0, 0, 2, 2, [200, 100, 50, 255]);
        let small = frame.scaled_to_width(2);
        assert_eq!((small.width(), small.height()), (2, 1), "aspect is kept");
        assert_eq!(small.pixel(0, 0), Some([200, 100, 50, 255]), "left half");
        assert_eq!(small.pixel(1, 0), Some([0, 0, 0, 255]), "right half");
    }

    #[test]
    fn scaling_never_enlarges_a_frame() {
        let frame = flat(4, 4, [1, 2, 3, 255]);
        assert_eq!(frame.scaled_to_width(10), frame, "wider target is a no-op");
    }

    #[test]
    fn a_png_starts_with_the_png_signature() {
        let png = flat(3, 3, [9, 9, 9, 255]).to_png().unwrap_or_default();
        assert_eq!(png.get(..8), Some(&[137, 80, 78, 71, 13, 10, 26, 10][..]));
    }

    #[test]
    fn a_gif_starts_with_the_gif_signature_and_has_every_frame() {
        let frames = [
            (flat(8, 8, [255, 0, 0, 255]), 100),
            (flat(8, 8, [0, 255, 0, 255]), 100),
        ];
        let gif = encode_gif(&frames).unwrap_or_default();
        assert_eq!(gif.get(..6), Some(&b"GIF89a"[..]));
        let decoded =
            GifDecoder::new(Cursor::new(gif)).map_or(0, |decoder| decoder.into_frames().count());
        assert_eq!(decoded, 2, "both frames decode");
    }

    #[test]
    fn a_filmstrip_lays_cells_in_a_grid_with_coloured_bands() {
        let cell = |mark, number| FilmstripCell {
            frame: flat(10, 10, [255, 255, 255, 255]),
            mark,
            number,
        };
        let cells = [
            cell(Mark::Pass, 1),
            cell(Mark::Fail, 22),
            cell(Mark::Neutral, 3),
        ];
        let strip = filmstrip(&cells, 2).unwrap_or_else(|| flat(1, 1, [0; 4]));
        assert_eq!(strip.width(), 2 * (10 + GAP) + GAP, "two columns");
        assert_eq!(
            strip.height(),
            2 * (10 + BAND_HEIGHT + GAP) + GAP,
            "two rows"
        );
        assert_eq!(
            strip.pixel(GAP + 1, GAP + 1),
            Some([255, 255, 255, 255]),
            "picture"
        );
        let band_row = GAP + 10 + BAND_HEIGHT - 2;
        assert_eq!(strip.pixel(GAP + 9, band_row), Some(PASS_BAND), "pass band");
        assert_eq!(
            strip.pixel(2 * GAP + 10 + 9, band_row),
            Some(FAIL_BAND),
            "fail band"
        );
    }

    #[test]
    fn a_filmstrip_of_nothing_is_nothing() {
        assert!(filmstrip(&[], 4).is_none(), "no cells, no strip");
    }
}
