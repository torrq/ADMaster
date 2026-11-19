use std::collections::HashMap;

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use resvg::{tiny_skia, usvg};

pub const APP_ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

pub fn rasterize_icon_svg(svg_bytes: &[u8], target_px: u32) -> Result<egui::IconData, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opt)
        .map_err(|e| format!("Failed to parse icon SVG: {}", e))?;

    let px = target_px.clamp(16, 1024);
    let mut pixmap = tiny_skia::Pixmap::new(px, px)
        .ok_or_else(|| "Failed to allocate icon pixmap".to_string())?;

    let svg_size = tree.size();
    let sx = (px as f32) / svg_size.width().max(1.0);
    let sy = (px as f32) / svg_size.height().max(1.0);
    let scale = sx.min(sy);
    let transform = tiny_skia::Transform::from_scale(scale, scale);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();

    Ok(egui::IconData {
        rgba,
        width: px,
        height: px,
    })
}

#[derive(Default)]
pub struct SvgIconCache {
    textures: HashMap<(&'static str, u32), TextureHandle>,
}

impl SvgIconCache {
    pub fn icon_texture(
        &mut self,
        ctx: &egui::Context,
        id: &'static str,
        svg_bytes: &[u8],
        size_pt: f32,
    ) -> Option<&TextureHandle> {
        let ppp = ctx.pixels_per_point().clamp(1.0, 4.0);
        let px = (size_pt * ppp).round().clamp(12.0, 512.0) as u32;
        let key = (id, px);

        if let std::collections::hash_map::Entry::Vacant(e) = self.textures.entry(key) {
            let opts = usvg::Options::default();
            let tree = usvg::Tree::from_data(svg_bytes, &opts).ok()?;
            let svg_size = tree.size();
            let scale = ((px as f32) / svg_size.width().max(1.0))
                .min((px as f32) / svg_size.height().max(1.0));
            let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            let image =
                ColorImage::from_rgba_premultiplied([px as usize, px as usize], &pixmap.take());
            let handle = ctx.load_texture(
                format!("admaster_svg_{}_{}", id, px),
                image,
                TextureOptions::LINEAR,
            );
            e.insert(handle);
        }

        self.textures.get(&key)
    }

    pub fn paint_app_icon(&mut self, ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
        let size_pt = rect.width().min(rect.height());
        let ppp = ctx.pixels_per_point().max(1.0);
        let snapped_min = Pos2::new(
            ((rect.center().x - size_pt * 0.5) * ppp).round() / ppp,
            ((rect.center().y - size_pt * 0.5) * ppp).round() / ppp,
        );
        let icon_rect = Rect::from_min_size(snapped_min, Vec2::splat(size_pt));
        if let Some(tex) = self.icon_texture(ctx, "app_icon", APP_ICON_SVG, size_pt) {
            painter.image(
                tex.id(),
                icon_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
}
