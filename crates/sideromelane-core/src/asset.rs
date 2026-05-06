//! Asset filename sanitization and image header validation.
//!
//! Both helpers exist to validate untrusted input before it reaches the
//! filesystem or the wiki-link parser.

use std::fmt;

/// Maximum byte length for an asset filename.
const MAX_ASSET_NAME_LEN: usize = 255;
/// Number of bytes inspected by [`validate_image_magic_bytes`].
pub const IMAGE_MAGIC_BYTES_LEN: usize = 16;

/// Errors returned by [`sanitize_asset_filename`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetNameError {
    /// The name is empty.
    Empty,
    /// The name exceeds [`MAX_ASSET_NAME_LEN`] bytes.
    TooLong,
    /// The name is `.` or `..`.
    Reserved,
    /// The name starts with `.`.
    Dotfile,
    /// The name contains a character that is forbidden in wiki embeds or
    /// filesystem paths.
    InvalidCharacter,
}

impl fmt::Display for AssetNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "asset filename is empty",
            Self::TooLong => "asset filename exceeds 255 bytes",
            Self::Reserved => "asset filename `.` or `..` is reserved",
            Self::Dotfile => "asset filename must not start with `.`",
            Self::InvalidCharacter => "asset filename contains an unsupported character",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for AssetNameError {}

/// Validates and normalises a dropped asset filename.
///
/// The returned string is byte-for-byte equal to the input on success.
/// The function rejects names that would break the `![[name]]` embed syntax
/// or that map to reserved or hidden paths.
pub fn sanitize_asset_filename(name: &str) -> Result<String, AssetNameError> {
    if name.is_empty() {
        return Err(AssetNameError::Empty);
    }

    if name.len() > MAX_ASSET_NAME_LEN {
        return Err(AssetNameError::TooLong);
    }

    if name == "." || name == ".." {
        return Err(AssetNameError::Reserved);
    }

    if name.starts_with('.') {
        return Err(AssetNameError::Dotfile);
    }

    for character in name.chars() {
        if matches!(character, '[' | ']' | '\n' | '\r' | '\0') {
            return Err(AssetNameError::InvalidCharacter);
        }
        if character.is_control() {
            return Err(AssetNameError::InvalidCharacter);
        }
    }

    Ok(name.to_owned())
}

/// Errors returned by [`validate_image_magic_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageMagicError {
    /// Fewer than the required number of bytes were available.
    TooShort,
    /// The header did not match a known image format.
    UnknownFormat,
}

impl fmt::Display for ImageMagicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooShort => "image header is too short",
            Self::UnknownFormat => "image header does not match a supported format",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for ImageMagicError {}

/// Validates that `bytes` starts with a magic-byte sequence for a supported
/// image format (PNG, JPEG, GIF87a/89a, WEBP, BMP).
pub fn validate_image_magic_bytes(bytes: &[u8]) -> Result<(), ImageMagicError> {
    if bytes.len() < 3 {
        return Err(ImageMagicError::TooShort);
    }

    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok(());
    }

    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(());
    }

    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok(());
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Ok(());
    }

    if bytes.starts_with(b"BM") {
        return Ok(());
    }

    Err(ImageMagicError::UnknownFormat)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::{
        AssetNameError, ImageMagicError, sanitize_asset_filename, validate_image_magic_bytes,
    };

    #[test]
    fn accepts_simple_name() {
        assert_eq!(sanitize_asset_filename("photo.png").unwrap(), "photo.png");
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(sanitize_asset_filename(""), Err(AssetNameError::Empty));
    }

    #[test]
    fn rejects_too_long() {
        let name = format!("{}.png", "a".repeat(260));
        assert_eq!(sanitize_asset_filename(&name), Err(AssetNameError::TooLong));
    }

    #[test]
    fn rejects_reserved() {
        assert_eq!(sanitize_asset_filename("."), Err(AssetNameError::Reserved));
        assert_eq!(sanitize_asset_filename(".."), Err(AssetNameError::Reserved));
    }

    #[test]
    fn rejects_dotfile() {
        assert_eq!(
            sanitize_asset_filename(".hidden.png"),
            Err(AssetNameError::Dotfile),
        );
    }

    #[test]
    fn rejects_brackets_and_newlines() {
        assert_eq!(
            sanitize_asset_filename("foo[bar].png"),
            Err(AssetNameError::InvalidCharacter),
        );
        assert_eq!(
            sanitize_asset_filename("foo]bar.png"),
            Err(AssetNameError::InvalidCharacter),
        );
        assert_eq!(
            sanitize_asset_filename("foo\nbar.png"),
            Err(AssetNameError::InvalidCharacter),
        );
        assert_eq!(
            sanitize_asset_filename("foo\rbar.png"),
            Err(AssetNameError::InvalidCharacter),
        );
        assert_eq!(
            sanitize_asset_filename("foo\0bar.png"),
            Err(AssetNameError::InvalidCharacter),
        );
    }

    #[test]
    fn rejects_control_chars() {
        assert_eq!(
            sanitize_asset_filename("foo\u{0007}.png"),
            Err(AssetNameError::InvalidCharacter),
        );
    }

    #[test]
    fn validates_png_header() {
        let bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert!(validate_image_magic_bytes(&bytes).is_ok());
    }

    #[test]
    fn validates_jpeg_header() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0];
        assert!(validate_image_magic_bytes(&bytes).is_ok());
    }

    #[test]
    fn validates_gif_headers() {
        assert!(validate_image_magic_bytes(b"GIF87a---").is_ok());
        assert!(validate_image_magic_bytes(b"GIF89a---").is_ok());
    }

    #[test]
    fn validates_webp_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert!(validate_image_magic_bytes(&bytes).is_ok());
    }

    #[test]
    fn validates_bmp_header() {
        assert!(validate_image_magic_bytes(b"BM--").is_ok());
    }

    #[test]
    fn rejects_unknown_format() {
        assert_eq!(
            validate_image_magic_bytes(b"NOTANIMAGE"),
            Err(ImageMagicError::UnknownFormat),
        );
    }

    #[test]
    fn rejects_too_short() {
        assert_eq!(
            validate_image_magic_bytes(&[0x89, 0x50]),
            Err(ImageMagicError::TooShort),
        );
    }
}
