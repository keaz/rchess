//! Piece images for the Image glyph style (spec 9.3): the Cburnett PNGs from
//! `assets/pieces/` embedded in the binary, [`composite`] to draw one piece onto an
//! opaque background of an exact pixel size, and [`ImageCache`] to keep what is
//! built from those composites between frames.
//!
//! Nothing here touches the terminal: the cache is generic over its values, so the
//! board keeps ratatui-image protocols in it and the tests keep plain numbers.

use std::collections::HashMap;
use std::sync::OnceLock;

use image::imageops::{self, FilterType};
use image::{ImageFormat, Rgba, RgbaImage};

use crate::core::Piece;

/// Width and height in pixels of every embedded piece image.
pub const SOURCE_SIZE: u32 = 256;

/// The embedded PNGs (see `assets/pieces/README.md`), indexed by
/// `Color::index` and then `PieceKind::index`: pawn, knight, bishop, rook,
/// queen, king.
const PNGS: [[&[u8]; 6]; 2] = [
    [
        include_bytes!("../../assets/pieces/wP.png"),
        include_bytes!("../../assets/pieces/wN.png"),
        include_bytes!("../../assets/pieces/wB.png"),
        include_bytes!("../../assets/pieces/wR.png"),
        include_bytes!("../../assets/pieces/wQ.png"),
        include_bytes!("../../assets/pieces/wK.png"),
    ],
    [
        include_bytes!("../../assets/pieces/bP.png"),
        include_bytes!("../../assets/pieces/bN.png"),
        include_bytes!("../../assets/pieces/bB.png"),
        include_bytes!("../../assets/pieces/bR.png"),
        include_bytes!("../../assets/pieces/bQ.png"),
        include_bytes!("../../assets/pieces/bK.png"),
    ],
];

/// The decoded images, laid out like [`PNGS`] and each filled on first use.
static DECODED: [[OnceLock<RgbaImage>; 6]; 2] = [const { [const { OnceLock::new() }; 6] }; 2];

/// The image of `piece`: [`SOURCE_SIZE`] pixels square, RGBA, transparent around
/// the piece. It is decoded on first use and kept for the life of the process.
fn source(piece: Piece) -> &'static RgbaImage {
    let (color, kind) = (piece.color.index(), piece.kind.index());
    DECODED[color][kind].get_or_init(|| {
        image::load_from_memory_with_format(PNGS[color][kind], ImageFormat::Png)
            .expect("the embedded piece images are valid PNGs")
            .into_rgba8()
    })
}

/// `piece` drawn on a solid `background` (RGB) of exactly `width_px` × `height_px`
/// pixels, every one of them opaque.
///
/// The piece is scaled with [`FilterType::Lanczos3`] to the largest size that fits
/// with its aspect ratio kept, and centred; the rest of the longer side is
/// background. It is blended onto the background before it is scaled, so its
/// anti-aliased edges are scaled against the colour they are shown on, and nothing
/// depends on how a terminal treats transparency. A zero width or height gives an
/// empty image.
#[must_use]
pub fn composite(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> RgbaImage {
    let [r, g, b] = background;
    let mut canvas = RgbaImage::from_pixel(width_px, height_px, Rgba([r, g, b, 255]));
    let source = source(piece);
    let (fit_w, fit_h) = fit(source.dimensions(), (width_px, height_px));
    if fit_w == 0 || fit_h == 0 {
        return canvas;
    }

    let mut flat = source.clone();
    for pixel in flat.pixels_mut() {
        *pixel = alpha_over(background, *pixel);
    }
    let scaled = imageops::resize(&flat, fit_w, fit_h, FilterType::Lanczos3);
    let x = (width_px - fit_w) / 2;
    let y = (height_px - fit_h) / 2;
    imageops::replace(&mut canvas, &scaled, i64::from(x), i64::from(y));
    canvas
}

/// The largest size with the aspect ratio of `source` that fits in `area`, each
/// side rounded to the nearest pixel and at least 1; `(0, 0)` when either has a
/// zero side.
fn fit((src_w, src_h): (u32, u32), (area_w, area_h): (u32, u32)) -> (u32, u32) {
    if src_w == 0 || src_h == 0 || area_w == 0 || area_h == 0 {
        return (0, 0);
    }
    let (src_w, src_h) = (u64::from(src_w), u64::from(src_h));
    // Rounds `a × b / c` and keeps it within 1..=max, so it fits in a u32.
    let scale = |a: u64, b: u32, c: u64, max: u32| {
        let scaled = (a * u64::from(b) + c / 2) / c;
        scaled.clamp(1, u64::from(max)) as u32
    };
    if src_w * u64::from(area_h) <= src_h * u64::from(area_w) {
        // The height limits the size.
        (scale(src_w, area_h, src_h, area_w), area_h)
    } else {
        (area_w, scale(src_h, area_w, src_w, area_h))
    }
}

/// `pixel` blended over the opaque colour `background` with its straight alpha, in
/// sRGB values as terminals and image viewers blend them. The result is opaque.
fn alpha_over(background: [u8; 3], pixel: Rgba<u8>) -> Rgba<u8> {
    let alpha = u16::from(pixel[3]);
    // At most (255 × 255 + 127) / 255 = 255, so the result fits in a u8.
    let mix = |fg: u8, bg: u8| {
        ((u16::from(fg) * alpha + u16::from(bg) * (255 - alpha) + 127) / 255) as u8
    };
    Rgba([
        mix(pixel[0], background[0]),
        mix(pixel[1], background[1]),
        mix(pixel[2], background[2]),
        255,
    ])
}

/// Everything a piece composite depends on, and so the key of [`ImageCache`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    /// The piece drawn.
    pub piece: Piece,
    /// The square's background colour behind the piece, as RGB: light, dark, or a
    /// highlight tint.
    pub background: [u8; 3],
    /// Width of the image in pixels.
    pub width_px: u32,
    /// Height of the image in pixels.
    pub height_px: u32,
}

impl ImageKey {
    /// The key for `piece` on `background` at `width_px` × `height_px` pixels.
    pub const fn new(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> Self {
        ImageKey {
            piece,
            background,
            width_px,
            height_px,
        }
    }

    /// The [`composite`] this key describes.
    #[must_use]
    pub fn composite(&self) -> RgbaImage {
        composite(self.piece, self.background, self.width_px, self.height_px)
    }
}

/// Values built from piece composites, one per [`ImageKey`], so a piece is scaled
/// and encoded for the terminal once rather than every frame. The board keeps
/// ratatui-image protocols here; a piece that moves to a square with the same
/// background reuses its entry.
///
/// The cache never evicts on its own. Its owner calls [`ImageCache::clear`] when
/// the square size or the font size changes, which keeps it to one entry per piece
/// and background in use.
#[derive(Debug)]
pub struct ImageCache<T> {
    entries: HashMap<ImageKey, T>,
}

impl<T> ImageCache<T> {
    /// An empty cache.
    pub fn new() -> Self {
        ImageCache {
            entries: HashMap::new(),
        }
    }

    /// The value stored for `key`, if any.
    pub fn get(&self, key: &ImageKey) -> Option<&T> {
        self.entries.get(key)
    }

    /// The value for `key`, first built with `make` and stored when it is missing.
    pub fn get_or_insert_with(
        &mut self,
        key: ImageKey,
        make: impl FnOnce(&ImageKey) -> T,
    ) -> &mut T {
        self.entries.entry(key).or_insert_with_key(make)
    }

    /// Number of stored values.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every stored value.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl<T> Default for ImageCache<T> {
    fn default() -> Self {
        ImageCache::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use image::Rgba;

    use super::*;
    use crate::core::{Color as Side, PieceKind};

    /// The truecolor palette's light and dark squares and its selection tint.
    const LIGHT: [u8; 3] = [0xB5, 0x88, 0x63];
    const DARK: [u8; 3] = [0x7A, 0x56, 0x34];
    const SELECTED: [u8; 3] = [0x5E, 0x9B, 0x4A];

    const WHITE_KING: Piece = Piece::new(Side::White, PieceKind::King);

    fn all_pieces() -> impl Iterator<Item = Piece> {
        Side::ALL
            .into_iter()
            .flat_map(|color| PieceKind::ALL.map(|kind| Piece::new(color, kind)))
    }

    fn opaque([r, g, b]: [u8; 3]) -> Rgba<u8> {
        Rgba([r, g, b, 255])
    }

    /// Mean of the red, green and blue channels over the whole image.
    fn mean_brightness(image: &RgbaImage) -> f64 {
        let sum: u64 = image
            .pixels()
            .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
            .sum();
        sum as f64 / (3.0 * f64::from(image.width() * image.height()))
    }

    #[test]
    fn embedded_images_decode_once_to_square_rgba() {
        for piece in all_pieces() {
            let image = source(piece);
            assert_eq!(image.dimensions(), (SOURCE_SIZE, SOURCE_SIZE), "{piece:?}");
            assert_eq!(
                image.get_pixel(0, 0)[3],
                0,
                "{piece:?} corner is not transparent"
            );
            assert!(
                image.pixels().any(|p| p[3] == 255),
                "{piece:?} has no opaque pixel"
            );
            assert!(
                std::ptr::eq(image, source(piece)),
                "{piece:?} decoded twice"
            );
        }
    }

    #[test]
    fn every_piece_has_its_own_image() {
        let pieces: Vec<Piece> = all_pieces().collect();
        for (i, a) in pieces.iter().enumerate() {
            for b in &pieces[i + 1..] {
                assert!(source(*a) != source(*b), "{a:?} and {b:?} share an image");
            }
        }
    }

    #[test]
    fn composite_has_exactly_the_requested_size() {
        for (width, height) in [
            (48, 48),
            (50, 60),
            (60, 20),
            (20, 60),
            (1, 1),
            (3, 7),
            (256, 256),
            (512, 300),
        ] {
            for piece in [WHITE_KING, Piece::new(Side::Black, PieceKind::Knight)] {
                let image = composite(piece, LIGHT, width, height);
                assert_eq!(image.dimensions(), (width, height), "{piece:?}");
            }
        }
    }

    #[test]
    fn composite_is_opaque_with_the_background_in_the_corners() {
        for piece in all_pieces() {
            for background in [LIGHT, DARK, SELECTED] {
                for (width, height) in [(48, 48), (50, 60), (60, 20), (20, 60), (160, 160)] {
                    let image = composite(piece, background, width, height);
                    assert!(
                        image.pixels().all(|p| p[3] == 255),
                        "{piece:?} {width}x{height} has a see-through pixel"
                    );
                    for (x, y) in [
                        (0, 0),
                        (width - 1, 0),
                        (0, height - 1),
                        (width - 1, height - 1),
                    ] {
                        assert_eq!(
                            *image.get_pixel(x, y),
                            opaque(background),
                            "{piece:?} {width}x{height} corner ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn composite_draws_the_piece_in_the_centre() {
        for piece in all_pieces() {
            for background in [LIGHT, DARK, SELECTED] {
                for (width, height) in [(48, 48), (50, 60), (90, 30)] {
                    let image = composite(piece, background, width, height);
                    assert_ne!(
                        *image.get_pixel(width / 2, height / 2),
                        opaque(background),
                        "{piece:?} {width}x{height} centre shows the background"
                    );
                }
            }
        }
    }

    #[test]
    fn composite_keeps_the_aspect_ratio_and_centres_the_piece() {
        let wide = composite(WHITE_KING, DARK, 90, 30);
        for (x, y, pixel) in wide.enumerate_pixels() {
            if !(30..60).contains(&x) {
                assert_eq!(
                    *pixel,
                    opaque(DARK),
                    "wide: ({x}, {y}) is outside the piece"
                );
            }
        }
        let tall = composite(WHITE_KING, DARK, 30, 90);
        for (x, y, pixel) in tall.enumerate_pixels() {
            if !(30..60).contains(&y) {
                assert_eq!(
                    *pixel,
                    opaque(DARK),
                    "tall: ({x}, {y}) is outside the piece"
                );
            }
        }
        // The fitted piece is the same picture either way.
        let square = composite(WHITE_KING, DARK, 30, 30);
        for (x, y, pixel) in square.enumerate_pixels() {
            assert_eq!(wide.get_pixel(x + 30, y), pixel);
            assert_eq!(tall.get_pixel(x, y + 30), pixel);
        }
    }

    #[test]
    fn white_pieces_are_lighter_than_black_pieces() {
        for kind in PieceKind::ALL {
            let white = composite(Piece::new(Side::White, kind), LIGHT, 64, 64);
            let black = composite(Piece::new(Side::Black, kind), LIGHT, 64, 64);
            assert!(white != black, "{kind:?}");
            assert!(
                mean_brightness(&white) > mean_brightness(&black) + 10.0,
                "{kind:?}: white {} vs black {}",
                mean_brightness(&white),
                mean_brightness(&black)
            );
        }
    }

    #[test]
    fn composite_is_deterministic() {
        for piece in all_pieces() {
            assert!(
                composite(piece, LIGHT, 50, 60) == composite(piece, LIGHT, 50, 60),
                "{piece:?}"
            );
        }
    }

    #[test]
    fn zero_sizes_give_empty_images() {
        for (width, height) in [(0, 0), (0, 40), (40, 0)] {
            let image = composite(WHITE_KING, LIGHT, width, height);
            assert_eq!(image.dimensions(), (width, height));
        }
    }

    #[test]
    fn a_key_composites_its_own_fields() {
        let key = ImageKey::new(WHITE_KING, SELECTED, 50, 60);
        assert_eq!(key.piece, WHITE_KING);
        assert_eq!(key.background, SELECTED);
        assert_eq!((key.width_px, key.height_px), (50, 60));
        assert!(key.composite() == composite(WHITE_KING, SELECTED, 50, 60));
    }

    #[test]
    fn cache_builds_each_key_once() {
        let mut cache = ImageCache::new();
        assert!(cache.is_empty());
        let calls = Cell::new(0);
        let make = |key: &ImageKey| {
            calls.set(calls.get() + 1);
            (key.width_px, key.height_px)
        };

        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);
        assert_eq!(cache.get(&key), None);
        assert_eq!(*cache.get_or_insert_with(key, make), (50, 60));
        assert_eq!(*cache.get_or_insert_with(key, make), (50, 60));
        assert_eq!(calls.get(), 1, "the second lookup was a miss");
        assert_eq!(cache.get(&key), Some(&(50, 60)));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_keys_on_piece_background_and_pixel_size() {
        let mut cache = ImageCache::new();
        let calls = Cell::new(0);
        let make = |_: &ImageKey| calls.set(calls.get() + 1);

        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);
        let variants = [
            key,
            ImageKey::new(Piece::new(Side::Black, PieceKind::King), LIGHT, 50, 60),
            ImageKey::new(Piece::new(Side::White, PieceKind::Queen), LIGHT, 50, 60),
            ImageKey::new(WHITE_KING, DARK, 50, 60),
            ImageKey::new(WHITE_KING, SELECTED, 50, 60),
            ImageKey::new(WHITE_KING, LIGHT, 51, 60),
            ImageKey::new(WHITE_KING, LIGHT, 50, 61),
        ];
        for variant in variants {
            cache.get_or_insert_with(variant, make);
        }
        assert_eq!(calls.get(), variants.len());
        assert_eq!(cache.len(), variants.len());

        // A piece that moves to another square of the same colour reuses its entry.
        cache.get_or_insert_with(ImageKey::new(WHITE_KING, LIGHT, 50, 60), make);
        assert_eq!(calls.get(), variants.len());
    }

    #[test]
    fn clear_empties_the_cache() {
        let mut cache = ImageCache::default();
        let calls = Cell::new(0);
        let make = |_: &ImageKey| calls.set(calls.get() + 1);
        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);

        cache.get_or_insert_with(key, make);
        cache.get_or_insert_with(ImageKey::new(WHITE_KING, DARK, 50, 60), make);
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.get(&key), None);
        cache.get_or_insert_with(key, make);
        assert_eq!(calls.get(), 3, "a cleared entry was still found");
    }
}
