//! Toolbar button icons. `assets/icons/*.png` is embedded into the
//! executable and decoded once at startup. Swapping a file is enough to
//! change its look — the `image` crate auto-detects format from the
//! bytes, so switching to .ico wouldn't need any caller changes.

use crate::ui::rgba_to_packed_alpha;

use super::{EditorBtn, Tool};

pub struct Icon {
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<u32>,
}

fn decode(bytes: &[u8]) -> Icon {
    let img = image::load_from_memory(bytes)
        .expect("埋め込みアイコン画像のデコードに失敗")
        .to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let pixels = rgba_to_packed_alpha(w, h, img.as_raw());
    Icon { w, h, pixels }
}

pub struct IconSet {
    select: Icon,
    arrow: Icon,
    polyline: Icon,
    freehand: Icon,
    rect: Icon,
    ellipse: Icon,
    text: Icon,
    number_marker: Icon,
    guide: Icon,
    mosaic: Icon,
    pin: Icon,
    save: Icon,
    copy: Icon,
}

impl IconSet {
    pub fn load() -> Self {
        Self {
            select: decode(include_bytes!("../../assets/icons/select.png")),
            arrow: decode(include_bytes!("../../assets/icons/arrow.png")),
            polyline: decode(include_bytes!("../../assets/icons/polyline.png")),
            freehand: decode(include_bytes!("../../assets/icons/freehand.png")),
            rect: decode(include_bytes!("../../assets/icons/rect.png")),
            ellipse: decode(include_bytes!("../../assets/icons/ellipse.png")),
            text: decode(include_bytes!("../../assets/icons/text.png")),
            number_marker: decode(include_bytes!("../../assets/icons/number_marker.png")),
            guide: decode(include_bytes!("../../assets/icons/guide.png")),
            mosaic: decode(include_bytes!("../../assets/icons/mosaic.png")),
            pin: decode(include_bytes!("../../assets/icons/pin.png")),
            save: decode(include_bytes!("../../assets/icons/save.png")),
            copy: decode(include_bytes!("../../assets/icons/copy.png")),
        }
    }

    pub fn get(&self, btn: EditorBtn) -> &Icon {
        match btn {
            EditorBtn::Tool(Tool::Select) => &self.select,
            EditorBtn::Tool(Tool::Arrow) => &self.arrow,
            EditorBtn::Tool(Tool::Polyline) => &self.polyline,
            EditorBtn::Tool(Tool::Draw) => &self.freehand,
            EditorBtn::Tool(Tool::Rect) => &self.rect,
            EditorBtn::Tool(Tool::Ellipse) => &self.ellipse,
            EditorBtn::Tool(Tool::Text) => &self.text,
            EditorBtn::Tool(Tool::NumberMarker) => &self.number_marker,
            EditorBtn::Tool(Tool::Guide) => &self.guide,
            EditorBtn::Tool(Tool::Mosaic) => &self.mosaic,
            EditorBtn::Pin => &self.pin,
            EditorBtn::Save => &self.save,
            EditorBtn::Copy => &self.copy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_decodes_all_thirteen_icons_without_panicking() {
        let set = IconSet::load();
        for btn in [
            EditorBtn::Tool(Tool::Select),
            EditorBtn::Tool(Tool::Arrow),
            EditorBtn::Tool(Tool::Polyline),
            EditorBtn::Tool(Tool::Draw),
            EditorBtn::Tool(Tool::Rect),
            EditorBtn::Tool(Tool::Ellipse),
            EditorBtn::Tool(Tool::Text),
            EditorBtn::Tool(Tool::NumberMarker),
            EditorBtn::Tool(Tool::Guide),
            EditorBtn::Tool(Tool::Mosaic),
            EditorBtn::Pin,
            EditorBtn::Save,
            EditorBtn::Copy,
        ] {
            let icon = set.get(btn);
            assert_eq!(icon.pixels.len(), icon.w * icon.h);
            assert!(icon.w > 0 && icon.h > 0);
        }
    }
}
