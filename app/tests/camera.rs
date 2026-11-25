//! Scanning another device's QR code with the camera (FR-024), headless:
//! `nsh-extra`'s camera answered by a fake backend that "takes" a picture
//! of a QR invitation, as NSH recommends for system features
//! (`set_backend`). One test: the backend is process-wide.

use std::{sync::Arc, time::Duration};

use nsh_core::{geometry::size, picker::PickedFile, platform::Os, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_extra::camera::{self, CameraBackend, PhotoFuture, PhotoRequest};
use nsh_testing::Headless;

const INVITATION: &str = "DEXDROP1:TESTINVITATION";

/// A camera whose every photo is `png`.
struct Fake(Vec<u8>);

impl CameraBackend for Fake {
    fn available(&self) -> bool {
        true
    }

    fn take_photo(&self, _: PhotoRequest) -> PhotoFuture {
        let file = PickedFile::new("IMG_0001.png", "image/png", self.0.clone().into());
        Box::pin(async move { Ok(Some(file)) })
    }
}

/// `text` as a QR code in a photo: dark modules on a light page.
fn qr_png(text: &str) -> Vec<u8> {
    let (width, modules) = dexdrop_core::invitation::qr_matrix(text).unwrap();
    let scale = 6;
    let side = (width as u32 + 8) * scale;
    let img = image::GrayImage::from_fn(side, side, |x, y| {
        let (mx, my) = ((x / scale) as i64 - 4, (y / scale) as i64 - 4);
        let dark = mx >= 0
            && my >= 0
            && (mx as usize) < width
            && (my as usize) < width
            && modules[my as usize * width + mx as usize];
        image::Luma([if dark { 20 } else { 235 }])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// An Android phone with a file picked, on Send's "Use a code" page.
fn code_page(dir: &tempfile::TempDir) -> Headless {
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().join("data"));
    config.viewport = size(px(390.0), px(844.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    let file = dir.path().join("talk.pdf");
    std::fs::write(&file, vec![7u8; 4096]).unwrap();
    app.on_pick_paths(vec![file]).unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-files").unwrap();
    app.press("id:pick-files").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    app.press("id:next").unwrap();
    app.wait_for_within("text:Use a code", Duration::from_secs(5))
        .unwrap();
    app.press("text:Use a code").unwrap();
    app.wait_for_within("label:Code or invitation", Duration::from_secs(5))
        .unwrap();
    app
}

/// Without a camera there is no Scan button; with one, the photo's QR
/// code fills in the invitation.
#[test]
fn scanning_a_qr_code_fills_in_the_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = code_page(&dir);
    app.wait(Duration::from_millis(200)).unwrap();
    assert!(
        app.screen().id("id:scan-qr").is_err(),
        "Scan QR without a camera"
    );
    drop(app);

    camera::set_backend(Some(Arc::new(Fake(qr_png(INVITATION)))));
    let dir = tempfile::tempdir().unwrap();
    let mut app = code_page(&dir);
    app.wait_for_within("id:scan-qr", Duration::from_secs(5))
        .unwrap();
    app.press("id:scan-qr").unwrap();
    app.wait_for_within("text:Invitation read.", Duration::from_secs(10))
        .unwrap();
    let value = app
        .screen()
        .input_text("label:Code or invitation")
        .unwrap()
        .unwrap_or_default()
        .to_owned();
    camera::set_backend(None);
    assert_eq!(value, INVITATION);
}
