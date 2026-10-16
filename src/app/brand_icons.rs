//! Embedded colour artwork shared by windows, the tray and Linux packages.
use std::sync::{Arc, OnceLock};

macro_rules! icon {
    ($size:literal) => {
        include_bytes!(concat!(
            "../../packaging/linux/icons/hicolor/",
            $size,
            "x",
            $size,
            "/apps/dev.rustleblower.Rustleblower.png"
        ))
    };
}

pub(super) fn window_icon() -> Arc<image::RgbaImage> {
    static ICON: OnceLock<Arc<image::RgbaImage>> = OnceLock::new();
    ICON.get_or_init(|| Arc::new(decode(icon!("256")))).clone()
}

pub(super) fn tray_icons() -> Vec<ksni::Icon> {
    [
        icon!("16").as_slice(),
        icon!("22").as_slice(),
        icon!("24").as_slice(),
        icon!("32").as_slice(),
    ]
    .into_iter()
    .map(|png| {
        let image = decode(png);
        ksni::Icon {
            width: image.width() as i32,
            height: image.height() as i32,
            // StatusNotifierItem uses unpremultiplied ARGB, network order.
            data: image
                .pixels()
                .flat_map(|p| [p[3], p[0], p[1], p[2]])
                .collect(),
        }
    })
    .collect()
}

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("checked-in Rustleblower icon is a valid PNG")
        .to_rgba8()
}
