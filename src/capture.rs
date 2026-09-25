//! Capture through the XDG Desktop Portal, the way cosmic-screenshot does.
//!
//! The portal (xdg-desktop-portal-cosmic) draws the selection across every
//! output, so this crate never creates a surface of its own before the editor.

use std::io::Read;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use cosmic::dialog::ashpd::desktop::ResponseError;
use cosmic::dialog::ashpd::desktop::screenshot::Screenshot;
use tiny_skia::Pixmap;
use wl_clipboard_rs::paste;

use crate::render;

/// Where the portal put the picture.
#[derive(Debug, PartialEq)]
pub enum Location {
    File(PathBuf),
    Clipboard,
}

/// Classifies the URI the portal answers with.
pub fn locate(uri: &str) -> Result<Location, String> {
    if let Some(path) = uri.strip_prefix("file://") {
        let path = percent_decode(path);
        if path.is_empty() {
            return Err(format!("portal returned an empty file URI: {uri}"));
        }
        return Ok(Location::File(PathBuf::from(path)));
    }
    if uri.starts_with("clipboard:") {
        return Ok(Location::Clipboard);
    }
    Err(format!("portal returned an unsupported URI: {uri}"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if b'%' == bytes[i] && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Asks the portal for an interactive screenshot. `Ok(None)` means the user
/// pressed Esc in the portal's selection.
pub async fn request() -> Result<Option<Pixmap>, String> {
    let request = Screenshot::request()
        .interactive(true)
        .modal(true)
        .send()
        .await
        .map_err(|e| format!("screenshot portal request failed: {e}"))?;
    let response = match request.response() {
        Ok(r) => r,
        Err(cosmic::dialog::ashpd::Error::Response(ResponseError::Cancelled)) => return Ok(None),
        Err(e) => return Err(format!("screenshot portal failed: {e}")),
    };
    let bytes = match locate(response.uri().as_str())? {
        Location::File(path) => {
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
        Location::Clipboard => read_clipboard_png()?,
    };
    render::decode_png(&bytes).map(Some)
}

/// The portal sets the clipboard before it answers, but give it a moment in
/// case the offer reaches this client after the reply does.
fn read_clipboard_png() -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for _ in 0..20 {
        match paste::get_contents(
            paste::ClipboardType::Regular,
            paste::Seat::Unspecified,
            paste::MimeType::Specific("image/png"),
        ) {
            Ok((mut pipe, _)) => {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes)
                    .map_err(|e| format!("cannot read the clipboard: {e}"))?;
                if !bytes.is_empty() {
                    return Ok(bytes);
                }
                last = "the clipboard image is empty".into();
            }
            Err(e) => last = e.to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "the portal copied the screenshot, but it cannot be read back: {last}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_is_a_path() {
        assert_eq!(
            locate("file:///home/u/Pictures/Screenshot%20one.png").unwrap(),
            Location::File(PathBuf::from("/home/u/Pictures/Screenshot one.png"))
        );
    }

    #[test]
    fn the_clipboard_uri_is_the_clipboard() {
        assert_eq!(locate("clipboard:///").unwrap(), Location::Clipboard);
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(locate("https://example.com/a.png").is_err());
        assert!(locate("file://").is_err());
    }
}
