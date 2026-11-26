//! Send's media browser on phones, headless (docs/FILE_SELECTION.md): the
//! device's photos and videos listed in the app from its asset provider, a
//! page at a time, chosen several at once and added to the selection as
//! references, nothing read until a transfer starts. The provider here
//! stands in for Android's MediaStore (registered under its scheme): it
//! holds no bytes, and counts what is asked of it.

#[path = "common/shot.rs"]
mod shot;

use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use dexdrop_core::asset::{
    AssetId, AssetProvider, AssetSource, MediaItem, MediaType, Page, Query, Sort, SourceReader,
    register,
};
use nsh_core::{element::Os, geometry::size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

/// What the provider was asked.
#[derive(Default)]
struct Asked {
    /// Each list's offset and limit.
    lists: Mutex<Vec<(usize, usize)>>,
    /// Originals opened (none while browsing and choosing).
    opens: AtomicUsize,
}

/// `photos` photos (`IMG_0001.jpg`…, 3 MB each, newest first), 3 videos and
/// one large video.
struct Library {
    photos: usize,
    asked: Arc<Asked>,
    /// Where thumbnails are drawn (`None`: the provider has none).
    thumbs: Option<PathBuf>,
}

/// A thumbnail for item `n`: a soft two-color gradient, a different hue
/// each, as a photo's colors would be.
fn picture(n: u64, to: &std::path::Path) {
    let hue = (n * 47 % 360) as f32;
    let rgb = |h: f32, l: f32| {
        let h = h / 60.0;
        let c = 0.55;
        let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        [
            ((r + l) * 255.0).min(255.0) as u8,
            ((g + l) * 255.0).min(255.0) as u8,
            ((b + l) * 255.0).min(255.0) as u8,
        ]
    };
    let (a, b) = (rgb(hue, 0.35), rgb((hue + 40.0) % 360.0, 0.15));
    let img = image::RgbImage::from_fn(96, 96, |x, y| {
        let t = (x + y) as f32 / 190.0;
        image::Rgb([0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8))
    });
    img.save(to).unwrap();
}

fn item(token: &str, name: &str, kind: MediaType, size: u64, at: i64) -> MediaItem {
    MediaItem {
        id: AssetId::new(dexdrop_net::assets::ANDROID_MEDIA, token),
        name: name.to_owned(),
        mime_type: String::new(),
        size: Some(size),
        media_type: kind,
        source: AssetSource::AndroidMediaStore,
        created_at: Some(at),
        modified_at: Some(at),
        duration: (kind == MediaType::Video).then_some(65_000),
        width: None,
        height: None,
        album: Some(
            if kind == MediaType::Image {
                "Camera"
            } else {
                "Movies"
            }
            .to_owned(),
        ),
    }
}

impl Library {
    fn all(&self) -> Vec<MediaItem> {
        let mut all: Vec<MediaItem> = (1..=self.photos)
            .map(|i| {
                item(
                    &format!("image:{i}"),
                    &format!("IMG_{i:04}.jpg"),
                    MediaType::Image,
                    3_000_000,
                    1_700_000_000 + i as i64,
                )
            })
            .collect();
        for i in 1..=3 {
            all.push(item(
                &format!("video:{i}"),
                &format!("clip{i}.mp4"),
                MediaType::Video,
                20_000_000,
                1_600_000_000 + i,
            ));
        }
        all.push(item(
            "video:99",
            "Holiday.mov",
            MediaType::Video,
            3 * 1024 * 1024 * 1024,
            1_500_000_000,
        ));
        all
    }
}

impl AssetProvider for Library {
    fn scheme(&self) -> &str {
        dexdrop_net::assets::ANDROID_MEDIA
    }
    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
        self.all()
            .into_iter()
            .find(|m| &m.id == id)
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn open(&self, _id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        self.asked.opens.fetch_add(1, Ordering::Relaxed);
        Err(io::ErrorKind::Unsupported.into())
    }
    fn list(&self, q: &Query) -> io::Result<Page> {
        self.asked.lists.lock().unwrap().push((q.offset, q.limit));
        let mut all: Vec<MediaItem> = self
            .all()
            .into_iter()
            .filter(|m| q.kinds.contains(&m.media_type))
            .filter(|m| m.size.unwrap_or(0) >= q.min_size.unwrap_or(0))
            .filter(|m| {
                q.search
                    .as_ref()
                    .is_none_or(|s| m.name.to_lowercase().contains(&s.to_lowercase()))
            })
            .filter(|m| q.album.is_none() || m.album == q.album)
            .collect();
        match q.sort {
            Sort::Newest => all.sort_by_key(|m| -m.modified_at.unwrap_or(0)),
            Sort::Oldest => all.sort_by_key(|m| m.modified_at.unwrap_or(0)),
            Sort::Largest => all.sort_by_key(|m| std::cmp::Reverse(m.size)),
            Sort::Name => all.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        let more = all.len() > q.offset + q.limit;
        Ok(Page {
            items: all.into_iter().skip(q.offset).take(q.limit).collect(),
            more,
        })
    }
    fn thumbnail(&self, id: &AssetId, _px: u32) -> io::Result<Option<PathBuf>> {
        let Some(dir) = &self.thumbs else {
            return Ok(None);
        };
        let token = id.token();
        let n: u64 = token
            .rsplit(':')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
            + if token.starts_with("video") { 500 } else { 0 };
        let to = dir.join(format!("{}.png", token.replace(':', "-")));
        if !to.exists() {
            std::fs::create_dir_all(dir)?;
            picture(n, &to);
        }
        Ok(Some(to))
    }
    fn albums(&self, kinds: &[MediaType]) -> io::Result<Vec<(String, usize)>> {
        Ok(if kinds.contains(&MediaType::Image) {
            vec![("Camera".into(), self.photos), ("Screenshots".into(), 0)]
        } else {
            vec![("Movies".into(), 4)]
        })
    }
}

/// The tests share a process, and the provider registry with it: one at a
/// time.
static ONE: Mutex<()> = Mutex::new(());

fn start(photos: usize, access: Option<&str>) -> (Headless, tempfile::TempDir, Arc<Asked>) {
    let (app, dir, asked, _camera) = start_with(photos, access, false);
    (app, dir, asked)
}

fn start_with(
    photos: usize,
    access: Option<&str>,
    thumbs: bool,
) -> (Headless, tempfile::TempDir, Arc<Asked>, shot::Camera) {
    let asked = Arc::new(Asked::default());
    let dir = tempfile::tempdir().unwrap();
    register(Arc::new(Library {
        photos,
        asked: asked.clone(),
        thumbs: thumbs.then(|| dir.path().join("thumbs")),
    }));
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test"}"#,
        )
        .unwrap();
    }
    // SAFETY: the tests run one at a time (ONE), before the app starts.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
        match access {
            Some(a) => std::env::set_var("DEXDROP_MEDIA_ACCESS", a),
            None => std::env::remove_var("DEXDROP_MEDIA_ACCESS"),
        }
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    config.viewport = size(px(390.0), px(844.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    let mut app = Headless::start(config).unwrap();
    // Before any step: it records the styles and images mounted from the
    // first batch on.
    let camera = shot::Camera::attach(&mut app);
    app.wait_for("Nearby").unwrap();
    (app, dir, asked, camera)
}

const WAIT: Duration = Duration::from_secs(5);

/// Photos opens the browser; two taps choose two photos, the bar at the
/// foot counts them and their size, Add puts them in the selection: as
/// references, nothing opened.
#[test]
fn photos_are_chosen_in_the_app_and_added_without_reading_them() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir, asked) = start(40, None);
    app.press("id:send").unwrap();
    app.wait_for_within("id:pick-photos", WAIT).unwrap();
    app.press("id:pick-photos").unwrap();
    app.wait_for_within("id:library", WAIT).unwrap();
    // Newest first.
    app.wait_for_within("label:IMG_0040.jpg", WAIT).unwrap();
    app.press("label:IMG_0040.jpg").unwrap();
    app.press("label:IMG_0039.jpg").unwrap();
    app.wait_for_within("text:2 selected · 6.00 MB", WAIT)
        .unwrap();
    // Tapped again: out.
    app.press("label:IMG_0039.jpg").unwrap();
    app.wait_for_within("text:1 selected · 3.00 MB", WAIT)
        .unwrap();
    app.press("label:IMG_0038.jpg").unwrap();
    app.wait_for_within("label:Add 2 items", WAIT).unwrap();
    app.press("id:library-add").unwrap();
    app.wait_for_gone("id:library", WAIT).unwrap();
    app.wait_for_within("label:Remove IMG_0040.jpg", WAIT)
        .unwrap();
    assert!(app.screen().id("label:Remove IMG_0038.jpg").is_ok());
    app.wait_for_within("id:next", WAIT).unwrap();
    // Listed and described, never opened.
    assert_eq!(asked.opens.load(Ordering::Relaxed), 0);
}

/// A library of thousands is listed a page at a time: opening it asks for
/// one page, and only the tiles near the screen exist.
#[test]
fn a_large_library_is_listed_a_page_at_a_time() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir, asked) = start(5000, None);
    app.press("id:send").unwrap();
    app.press("id:pick-photos").unwrap();
    app.wait_for_within("label:IMG_5000.jpg", WAIT).unwrap();
    app.wait(Duration::from_millis(300)).unwrap();
    let lists = asked.lists.lock().unwrap().clone();
    assert!(!lists.is_empty());
    assert!(
        lists.iter().all(|(offset, limit)| offset + limit <= 400),
        "listed beyond the first pages: {lists:?}"
    );
    let tiles = app.screen().count("label:/^IMG_\\d{4}\\.jpg$/").unwrap();
    assert!(
        tiles > 8 && tiles < 200,
        "{tiles} tiles exist for a library of 5000"
    );
    // Select all takes what is listed so far.
    app.press("id:select-all").unwrap();
    app.wait_for_within("text:/^\\d+ selected · /", WAIT)
        .unwrap();
}

/// Large files: the items of 100 MB or more, largest first; the search
/// narrows by name.
#[test]
fn large_files_and_search() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir, _asked) = start(10, None);
    app.press("id:send").unwrap();
    app.press("id:pick-videos").unwrap();
    app.wait_for_within("label:clip3.mp4", WAIT).unwrap();
    app.wait_for_within("label:Holiday.mov", WAIT).unwrap();
    // Its length on the tile.
    assert!(app.screen().count("text:1:05").unwrap() >= 1);
    app.type_text("id:library-search", "clip2").unwrap();
    app.wait_for_gone("label:Holiday.mov", WAIT).unwrap();
    app.wait_for_within("label:clip2.mp4", WAIT).unwrap();
    assert!(app.screen().id("label:clip1.mp4").is_err());
    app.press("text:Large").unwrap();
    // The search still applies: nothing large is called clip2.
    app.wait_for_within("text:Nothing by that name.", WAIT)
        .unwrap();
}

/// Before the user allows it, the browser says what it is for and asks
/// with the system's prompt; allowed, it lists.
#[test]
fn the_browser_asks_before_listing() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir, asked) = start(5, Some("none"));
    app.press("id:send").unwrap();
    app.press("id:pick-photos").unwrap();
    app.wait_for_within("id:allow-media", WAIT).unwrap();
    assert!(
        asked.lists.lock().unwrap().is_empty(),
        "listed before allowed"
    );
    app.press("id:allow-media").unwrap();
    app.wait_for_within("label:IMG_0005.jpg", WAIT).unwrap();
}

/// Pictures of the browser, drawn from the headless layout
/// (`common/shot.rs`), into `DEXDROP_SHOTS` (default
/// `target/shots/library`): Send's choices, the permission card, the
/// photo grid with two chosen, Videos, Large files, and Send with the
/// photos added.
#[test]
fn screenshots_of_the_browser() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let out = std::env::var_os("DEXDROP_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/shots/library")
        });
    let (mut app, _dir, _asked, camera) = start_with(60, Some("none"), true);
    app.press("id:send").unwrap();
    app.wait_for_within("id:pick-photos", WAIT).unwrap();
    camera.shoot(&mut app, &out, "1-send");
    app.press("id:pick-photos").unwrap();
    app.wait_for_within("id:allow-media", WAIT).unwrap();
    camera.shoot(&mut app, &out, "2-allow");
    app.press("id:allow-media").unwrap();
    app.wait_for_within("label:IMG_0060.jpg", WAIT).unwrap();
    app.press("label:IMG_0059.jpg").unwrap();
    app.press("label:IMG_0057.jpg").unwrap();
    app.wait_for_within("text:2 selected · 6.00 MB", WAIT)
        .unwrap();
    camera.shoot(&mut app, &out, "3-photos");
    app.press("text:Videos").unwrap();
    app.wait_for_within("label:Holiday.mov", WAIT).unwrap();
    camera.shoot(&mut app, &out, "4-videos");
    app.press("text:Large").unwrap();
    app.wait_for_gone("label:clip1.mp4", WAIT).unwrap();
    app.wait_for_within("label:Holiday.mov", WAIT).unwrap();
    camera.shoot(&mut app, &out, "5-large");
    app.press("id:library-add").unwrap();
    app.wait_for_within("label:Remove IMG_0059.jpg", WAIT)
        .unwrap();
    camera.shoot(&mut app, &out, "6-added");
    for name in [
        "1-send", "2-allow", "3-photos", "4-videos", "5-large", "6-added",
    ] {
        assert!(out.join(format!("{name}.png")).is_file());
    }
}
