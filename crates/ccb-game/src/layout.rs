//! 2D layouts (menus, HUD): BRLYT pane trees rendered by an orthographic overlay camera,
//! animated by BRLAN files. Layout materials use NW4R's default combiner, expressed as
//! TEV stages for the shared GX shader: `lerp(black, white, texture) * vertex color * alpha`.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use wii_formats::{
    lyt::{AnimKind, Animation, Layout, PaneKind},
    rfnt::Font,
    tpl, u8arc,
};

use crate::{
    data::GameData,
    gx_material::{GxKey, GxMaterial, GxParams},
};

/// Render layer used by layouts and their camera.
pub const UI_LAYER: usize = 1;
/// Layout space height; the overlay camera shows this many units vertically.
pub const LAYOUT_HEIGHT: f32 = 456.0;
/// Frames per second of layout animations.
pub const LYT_FPS: f32 = 60.0;

pub struct LayoutPlugin;

impl Plugin for LayoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ui_camera)
            .add_systems(Update, (animate_layouts, sync_panes, debug_panes).chain());
    }
}

fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: LAYOUT_HEIGHT },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        Transform::from_xyz(0.0, 0.0, 500.0),
        RenderLayers::layer(UI_LAYER),
    ));
}

struct FontAsset {
    font: Font,
    sheets: Vec<Handle<Image>>,
}

/// Every file from the layout archives, plus decoded textures and fonts.
#[derive(Resource, Default)]
pub struct LayoutAssets {
    files: HashMap<String, Vec<u8>>,
    textures: HashMap<String, Handle<Image>>,
    fonts: HashMap<String, Arc<FontAsset>>,
    quad: Option<Handle<Mesh>>,
}

fn rgba_image(w: usize, h: usize, rgba: Vec<u8>) -> Image {
    let mut img = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    img
}

impl LayoutAssets {
    pub fn load(data: &GameData) -> Result<Self> {
        let mut files = HashMap::new();
        for arc in ["commons", "menus", "ingame", "ingame_menus"] {
            let bytes = data.read(&format!("layouts/{arc}.arc.LZ"))?;
            for e in u8arc::parse(&bytes)? {
                if let Some(d) = e.data {
                    let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                    files.insert(name, d.to_vec());
                }
            }
        }
        Ok(Self { files, ..default() })
    }

    pub fn layout(&self, name: &str) -> Result<Layout> {
        Layout::parse(self.files.get(&format!("{name}.brlyt")).with_context(|| format!("no layout {name}"))?)
    }

    pub fn animation(&self, name: &str) -> Result<Arc<Animation>> {
        let d = self.files.get(&format!("{name}.brlan")).with_context(|| format!("no animation {name}"))?;
        Ok(Arc::new(Animation::parse(d)?))
    }

    fn texture(&mut self, name: &str, images: &mut Assets<Image>) -> Option<Handle<Image>> {
        if let Some(h) = self.textures.get(name) {
            return Some(h.clone());
        }
        let img = tpl::parse(self.files.get(name)?).ok()?.into_iter().next()?;
        let h = images.add(rgba_image(img.width, img.height, img.rgba));
        self.textures.insert(name.to_string(), h.clone());
        Some(h)
    }

    fn font(&mut self, name: &str, images: &mut Assets<Image>) -> Option<Arc<FontAsset>> {
        if let Some(f) = self.fonts.get(name) {
            return Some(f.clone());
        }
        let font = Font::parse(self.files.get(name)?).ok()?;
        // NW4R treats intensity-only (I4/I8) font sheets as alpha masks; the text color
        // supplies the RGB. Our decoder gives (I, I, I, I), so force RGB to white. The
        // "3D" fonts store shading in the intensity, so coverage is boosted to keep glyph
        // bodies solid (the original layers shadow/outline font variants on top).
        let sheets = font
            .sheets
            .iter()
            .map(|s| {
                let mut px = s.clone();
                if px.chunks_exact(4).all(|c| c[0] == c[3]) {
                    px.chunks_exact_mut(4).for_each(|c| {
                        c[3] = (c[3] as u32 * 2).min(255) as u8;
                        c[..3].fill(255);
                    });
                }
                images.add(rgba_image(font.sheet_width as usize, font.sheet_height as usize, px))
            })
            .collect();
        let f = Arc::new(FontAsset { font, sheets });
        self.fonts.insert(name.to_string(), f.clone());
        Some(f)
    }

    /// A unit quad and material showing one texture (for sprites outside layouts).
    pub fn sprite(
        &mut self,
        texture: &str,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<GxMaterial>,
        images: &mut Assets<Image>,
    ) -> (Handle<Mesh>, Handle<GxMaterial>) {
        let tex = self.texture(texture, images);
        (self.unit_quad(meshes), materials.add(layout_material(None, tex)))
    }

    fn unit_quad(&mut self, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        self.quad.get_or_insert_with(|| meshes.add(quad_mesh([[255; 4]; 4], None))).clone()
    }
}

/// A quad spanning x 0..1, y 0..-1 (top-left origin), vertices TL, TR, BL, BR.
fn quad_mesh(colors: [[u8; 4]; 4], uvs: Option<[[f32; 2]; 4]>) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [1.0, -1.0, 0.0]]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.unwrap_or([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]).to_vec());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.map(|c| c.map(|v| v as f32 / 255.0)).to_vec());
    m.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
    m
}

const fn color_env(a: u32, b: u32, c: u32, d: u32) -> u32 {
    d | c << 4 | b << 8 | a << 12 | 1 << 19
}
const fn alpha_env(a: u32, b: u32, c: u32, d: u32) -> u32 {
    d << 4 | c << 7 | b << 10 | a << 13 | 1 << 19
}

/// NW4R's default layout combiner as TEV stages. konst[3].a carries the pane alpha.
fn layout_material(mat: Option<&wii_formats::lyt::LytMaterial>, texture: Option<Handle<Image>>) -> GxMaterial {
    let mut p = GxParams::default();
    p.info = UVec4::new(3, 7 | 7 << 3, 0, 0);
    // Color inputs: 0 CPREV 2 C0 4 C1 8 TEXC 10 RASC 15 ZERO; alpha: 0 APREV 1 A0 2 A1 4 TEXA 5 RASA 6 KONST 7 ZERO.
    p.color_env[0] = UVec4::new(color_env(2, 4, 8, 15), color_env(15, 0, 10, 15), color_env(15, 15, 15, 0), 0);
    p.alpha_env[0] = UVec4::new(alpha_env(1, 2, 4, 7), alpha_env(7, 0, 5, 7), alpha_env(7, 0, 6, 7), 0);
    let tex_on = texture.is_some() as u32;
    // Stage 2 selects KONST alpha = K3_A (0x1f).
    p.order[0] = UVec4::new(tex_on << 4, 0, 0x1f << 16, 0);
    let (black, white) = mat.map_or(([0i16; 4], [255i16; 4]), |m| (m.tev_colors[0], m.tev_colors[1]));
    let v = |c: [i16; 4]| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32) / 255.0;
    p.regs = [Vec4::ZERO, v(black), v(white), mat.map_or(Vec4::ZERO, |m| v(m.tev_colors[2]))];
    p.konst[3] = Vec4::ONE;
    if let Some(srt) = mat.and_then(|m| m.tex_srt.first()) {
        p.set_tex_srt(*srt);
    }
    GxMaterial {
        params: p,
        tex0: texture,
        tex1: None,
        alpha_mode: AlphaMode::Blend,
        key: GxKey { cull: 0, depth_write: 0, color_write: 1 },
    }
}

/// Animated state of one pane.
#[derive(Clone, Debug)]
pub struct PaneState {
    pub translate: Vec3,
    pub rotate: Vec3,
    pub scale: Vec2,
    pub size: Vec2,
    pub alpha: f32,
    pub visible: bool,
}

#[derive(Component)]
pub struct LayoutPane {
    pub index: usize,
    pub base: PaneState,
    pub cur: PaneState,
    origin: u8,
    /// Entity holding this pane's mesh and material, if it draws anything.
    visual: Option<Entity>,
    material: Option<Handle<GxMaterial>>,
    material_index: Option<usize>,
    /// Alpha after inheriting from parents (computed each frame).
    global_alpha: f32,
    /// Extra scale applied on top of the animated scale (button rollover etc.).
    pub scale_mul: f32,
    parent: Option<usize>,
    influenced_alpha: bool,
}

/// Root of a spawned layout.
#[derive(Component)]
pub struct LayoutRoot {
    pub layout: Arc<Layout>,
    pub panes: Vec<Entity>,
    names: HashMap<String, usize>,
    /// Materials by layout material index (for material animations).
    materials: HashMap<usize, Vec<Handle<GxMaterial>>>,
}

impl LayoutRoot {
    pub fn pane(&self, name: &str) -> Option<Entity> {
        self.names.get(name).map(|&i| self.panes[i])
    }

    /// Materials drawn by the named pane.
    pub fn pane_materials(&self, name: &str) -> Vec<Handle<GxMaterial>> {
        self.names
            .get(name)
            .and_then(|&i| self.layout.pane_material(i))
            .and_then(|m| self.materials.get(&m).cloned())
            .unwrap_or_default()
    }
}

/// Whether a layout-space point lies inside a pane's rectangle (e.g. a bounding pane).
pub fn pane_contains(pane: &LayoutPane, gt: &GlobalTransform, point: Vec2) -> bool {
    let local = gt.affine().inverse().transform_point3(point.extend(0.0));
    let off = origin_offset(pane.origin, pane.cur.size);
    local.x >= off.x && local.x <= off.x + pane.cur.size.x && local.y <= off.y && local.y >= off.y - pane.cur.size.y
}

/// Animations playing on a layout, each with its local frame (0 = the clip's start).
/// Starting a clip replaces any playing clip bound to the same groups.
#[derive(Component, Default)]
pub struct LayoutAnimator {
    pub playing: Vec<(Arc<Animation>, f32)>,
}

impl LayoutAnimator {
    pub fn play(&mut self, anim: Arc<Animation>) {
        self.playing.retain(|(a, _)| a.groups != anim.groups || a.groups.is_empty());
        self.playing.push((anim, 0.0));
    }
}

fn origin_offset(origin: u8, size: Vec2) -> Vec2 {
    let hx = (origin % 3) as f32 * 0.5;
    let vy = (origin / 3) as f32 * 0.5;
    Vec2::new(-size.x * hx, size.y * vy)
}

/// Builds the glyph mesh(es) for a text pane: one mesh per font sheet, in pane-local
/// units with the pane box's top-left at (0,0).
#[allow(clippy::too_many_arguments)]
fn text_meshes(
    font: &FontAsset,
    text: &str,
    pane_size: Vec2,
    position: u8,
    alignment: u8,
    font_size: [f32; 2],
    char_space: f32,
    line_space: f32,
    colors: ([u8; 4], [u8; 4]),
) -> Vec<(usize, Mesh)> {
    let f = &font.font;
    let sx = font_size[0] / f.width.max(1) as f32;
    let sy = font_size[1] / f.height.max(1) as f32;
    let line_h = f.line_feed as f32 * sy + line_space;
    let lines: Vec<&str> = text.split('\n').collect();
    let width_of = |l: &str| -> f32 {
        let w: f32 = l.chars().map(|c| f.glyph(c).char_width as f32 * sx + char_space).sum();
        (w - char_space).max(0.0)
    };
    let block_w = lines.iter().map(|l| width_of(l)).fold(0.0, f32::max);
    let block_h = lines.len() as f32 * line_h;
    let hx = (position % 3) as f32 * 0.5;
    let vy = (position / 3) as f32 * 0.5;
    let block_x = (pane_size.x - block_w) * hx;
    let block_y = -(pane_size.y - block_h) * vy;
    let align = match alignment {
        1 => 0.0,
        2 => 0.5,
        3 => 1.0,
        _ => hx,
    };
    let (sw, sh) = (f.sheet_width as f32, f.sheet_height as f32);
    let mut per_sheet: HashMap<usize, (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>)> = HashMap::new();
    let col = |c: [u8; 4]| c.map(|v| v as f32 / 255.0);
    for (li, line) in lines.iter().enumerate() {
        let mut x = block_x + (block_w - width_of(line)) * align;
        let y = block_y - li as f32 * line_h;
        for ch in line.chars() {
            let g = f.glyph(ch);
            let (sheet, cx, cy) = f.cell(g.index);
            let (x0, x1) = (x + g.left as f32 * sx, x + (g.left as f32 + g.glyph_width as f32) * sx);
            let (y0, y1) = (y, y - f.cell_height as f32 * sy);
            let (u0, u1) = (cx as f32 / sw, (cx as f32 + g.glyph_width as f32) / sw);
            let (v0, v1) = (cy as f32 / sh, (cy as f32 + f.cell_height as f32) / sh);
            let e = per_sheet.entry(sheet).or_default();
            let base = e.0.len() as u32;
            e.0.extend([[x0, y0, 0.0], [x1, y0, 0.0], [x0, y1, 0.0], [x1, y1, 0.0]]);
            e.1.extend([[u0, v0], [u1, v0], [u0, v1], [u1, v1]]);
            e.2.extend([col(colors.0), col(colors.0), col(colors.1), col(colors.1)]);
            e.3.extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
            x += g.char_width as f32 * sx + char_space;
        }
    }
    per_sheet
        .into_iter()
        .map(|(sheet, (pos, uv, colors, idx))| {
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            m.insert_indices(Indices::U32(idx));
            (sheet, m)
        })
        .collect()
}

/// Converts message text (`\n` escapes, `\_` icon placeholders) to display text.
pub fn display_text(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\_", "")
}

/// Text overrides applied when spawning a layout: strings by pane name, and a color the
/// game code assigns to all text (the file's colors are placeholders).
#[derive(Default)]
pub struct TextSetup<'a> {
    pub strings: HashMap<&'a str, String>,
    pub color: Option<[u8; 4]>,
}

/// Spawns a layout.
#[allow(clippy::too_many_arguments)]
pub fn spawn_layout(
    commands: &mut Commands,
    assets: &mut LayoutAssets,
    name: &str,
    texts: &TextSetup,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) -> Result<Entity> {
    let layout = Arc::new(assets.layout(name)?);
    let layer = RenderLayers::layer(UI_LAYER);
    let root = commands
        .spawn((Name::new(format!("layout {name}")), Transform::default(), Visibility::default(), layer.clone()))
        .id();
    let quad = assets.unit_quad(meshes);
    let mut panes = Vec::with_capacity(layout.panes.len());
    let mut mat_handles: HashMap<usize, Vec<Handle<GxMaterial>>> = HashMap::new();
    for (i, p) in layout.panes.iter().enumerate() {
        let st = PaneState {
            translate: Vec3::from(p.translate),
            rotate: Vec3::from(p.rotate),
            scale: Vec2::from(p.scale),
            size: Vec2::from(p.size),
            alpha: p.alpha as f32 / 255.0,
            visible: p.visible,
        };
        let e = commands.spawn((Name::new(p.name.clone()), Transform::default(), Visibility::default(), layer.clone())).id();
        commands.entity(p.parent.map_or(root, |pi| panes[pi])).add_child(e);
        // Draw order: later panes on top.
        let z = i as f32 * 0.05;
        let mut visual = None;
        let mut material = None;
        let mut material_index = None;
        match &p.kind {
            PaneKind::Picture(q) | PaneKind::Window { content: q, .. } => {
                let m = layout.materials.get(q.material);
                let tex = m
                    .and_then(|m| m.tex_maps.first())
                    .and_then(|t| layout.textures.get(t.texture))
                    .and_then(|t| assets.texture(t, images));
                let mc = m.map_or([255; 4], |m| m.mat_color);
                let colors = q.colors.map(|c| [0, 1, 2, 3].map(|k| (c[k] as u32 * mc[k] as u32 / 255) as u8));
                let mesh = if colors == [[255; 4]; 4] && q.uvs.is_none() { quad.clone() } else { meshes.add(quad_mesh(colors, q.uvs)) };
                let h = materials.add(layout_material(m, tex));
                let v = commands
                    .spawn((Mesh3d(mesh), MeshMaterial3d(h.clone()), Transform::from_xyz(0.0, 0.0, z), layer.clone()))
                    .id();
                commands.entity(e).add_child(v);
                mat_handles.entry(q.material).or_default().push(h.clone());
                (visual, material, material_index) = (Some(v), Some(h), Some(q.material));
            }
            PaneKind::Text {
                material: mi, font, text, position, alignment, color_top, color_bottom, font_size, char_space, line_space,
            } => {
                let font_asset = layout.fonts.get(*font).and_then(|f| assets.font(f, images));
                let text = texts.strings.get(p.name.as_str()).cloned().unwrap_or_else(|| text.clone());
                let (top, bottom) = texts.color.map_or((*color_top, *color_bottom), |c| (c, c));
                if let Some(fa) = font_asset {
                    let holder = commands.spawn((Transform::from_xyz(0.0, 0.0, z), Visibility::default(), layer.clone())).id();
                    commands.entity(e).add_child(holder);
                    let m = layout.materials.get(*mi);
                    for (sheet, mesh) in text_meshes(
                        &fa, &text, st.size, *position, *alignment, *font_size, *char_space, *line_space,
                        (top, bottom),
                    ) {
                        let h = materials.add(layout_material(m, fa.sheets.get(sheet).cloned()));
                        let g = commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(h.clone()), Transform::default(), layer.clone())).id();
                        commands.entity(holder).add_child(g);
                        mat_handles.entry(*mi).or_default().push(h.clone());
                        material = Some(h);
                    }
                    (visual, material_index) = (Some(holder), Some(*mi));
                }
            }
            PaneKind::Null | PaneKind::Bounding => {}
        }
        commands.entity(e).insert(LayoutPane {
            index: i,
            base: st.clone(),
            cur: st,
            origin: p.origin,
            visual,
            material,
            material_index,
            global_alpha: 1.0,
            scale_mul: 1.0,
            parent: p.parent,
            influenced_alpha: p.influenced_alpha,
        });
        panes.push(e);
    }
    let names = layout.panes.iter().enumerate().map(|(i, p)| (p.name.clone(), i)).collect();
    commands.entity(root).insert((LayoutRoot { layout, panes, names, materials: mat_handles }, LayoutAnimator::default()));
    Ok(root)
}

fn animate_layouts(
    time: Res<Time>,
    mut roots: Query<(&LayoutRoot, &mut LayoutAnimator)>,
    mut panes: Query<&mut LayoutPane>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut assets: ResMut<LayoutAssets>,
    mut images: ResMut<Assets<Image>>,
) {
    let dt = time.delta_secs() * LYT_FPS;
    for (root, mut anim) in &mut roots {
        for (a, frame) in anim.playing.iter_mut() {
            *frame += dt;
            let len = (a.end - a.start).max(1) as f32;
            if *frame >= len {
                *frame = if a.looping { *frame % len } else { len };
            }
            let f = a.start as f32 + *frame;
            let bound = root.layout.bound_panes(a);
            let pane_ok = |i: usize| bound.as_ref().is_none_or(|b| b[i]);
            for target in &a.targets {
                if target.is_material {
                    let Some(mi) = root.layout.materials.iter().position(|m| m.name == target.name) else { continue };
                    let used = (0..root.layout.panes.len()).any(|i| pane_ok(i) && root.layout.pane_material(i) == Some(mi));
                    if !used {
                        continue;
                    }
                    let Some(handles) = root.materials.get(&mi) else { continue };
                    for tag in &target.tags {
                        for h in handles {
                            let Some(mut m) = materials.get_mut(h) else { continue };
                            for c in &tag.curves {
                                let v = c.eval(f);
                                match tag.kind {
                                    AnimKind::MaterialColor => {
                                        let (reg, comp) = (c.target as usize / 4, c.target as usize % 4);
                                        match reg {
                                            1..=3 => m.params.regs[reg][comp] = v / 255.0,
                                            4..=7 => m.params.konst[reg - 4][comp] = v / 255.0,
                                            _ => {}
                                        }
                                    }
                                    AnimKind::TextureSrt if c.index == 0 => {
                                        let mut srt = root.layout.materials[mi].tex_srt.first().copied().unwrap_or([0.0, 0.0, 0.0, 1.0, 1.0]);
                                        if let Some(v0) = srt.get_mut(c.target as usize) {
                                            *v0 = v;
                                        }
                                        m.params.set_tex_srt(srt);
                                    }
                                    AnimKind::TexturePattern => {
                                        if let Some(t) = a.textures.get(v as usize) {
                                            m.tex0 = assets.texture(t, &mut images);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    continue;
                }
                let Some(&pi) = root.names.get(&target.name) else { continue };
                if !pane_ok(pi) {
                    continue;
                }
                let Ok(mut pane) = panes.get_mut(root.panes[pi]) else { continue };
                for tag in &target.tags {
                    for c in &tag.curves {
                        let v = c.eval(f);
                        let s = &mut pane.cur;
                        match (tag.kind, c.target) {
                            (AnimKind::PaneSrt, t @ 0..=2) => s.translate[t as usize] = v,
                            (AnimKind::PaneSrt, t @ 3..=5) => s.rotate[t as usize - 3] = v,
                            (AnimKind::PaneSrt, t @ 6..=7) => s.scale[t as usize - 6] = v,
                            (AnimKind::PaneSrt, t @ 8..=9) => s.size[t as usize - 8] = v,
                            (AnimKind::VertexColor, 16) => s.alpha = v / 255.0,
                            (AnimKind::Visibility, 0) => s.visible = v != 0.0,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn sync_panes(
    roots: Query<&LayoutRoot>,
    mut panes: Query<(&mut LayoutPane, &mut Transform, &mut Visibility)>,
    mut visuals: Query<&mut Transform, Without<LayoutPane>>,
    mut materials: ResMut<Assets<GxMaterial>>,
) {
    for root in &roots {
        let mut alphas = vec![1.0f32; root.panes.len()];
        for (i, &e) in root.panes.iter().enumerate() {
            let Ok((mut pane, mut tr, mut vis)) = panes.get_mut(e) else { continue };
            let s = pane.cur.clone();
            let parent_alpha = pane.parent.map_or(1.0, |p| alphas[p]);
            let alpha = s.alpha * parent_alpha;
            // Only panes with the "influenced alpha" flag pass their alpha to children.
            alphas[i] = if pane.influenced_alpha { alpha } else { parent_alpha };
            *tr = Transform {
                translation: Vec3::new(s.translate.x, s.translate.y, 0.0),
                rotation: Quat::from_rotation_z(s.rotate.z.to_radians()),
                scale: Vec3::new(s.scale.x * pane.scale_mul, s.scale.y * pane.scale_mul, 1.0),
            };
            *vis = if s.visible { Visibility::Inherited } else { Visibility::Hidden };
            if let Some(v) = pane.visual {
                if let Ok(mut vt) = visuals.get_mut(v) {
                    let off = origin_offset(pane.origin, s.size);
                    vt.translation.x = off.x;
                    vt.translation.y = off.y;
                    // Pictures use a unit quad; text meshes are already in pane units.
                    if pane.material_index.is_some() && !matches!(root.layout.panes[pane.index].kind, PaneKind::Text { .. }) {
                        vt.scale = Vec3::new(s.size.x, s.size.y, 1.0);
                    }
                }
            }
            if (pane.global_alpha - alpha).abs() > 1e-4 {
                pane.global_alpha = alpha;
                if let Some(idx) = pane.material_index {
                    let handles = root.materials.get(&idx).cloned().unwrap_or_default();
                    for h in handles.iter().chain(pane.material.iter()) {
                        if let Some(mut m) = materials.get_mut(h) {
                            m.params.konst[3].w = alpha;
                        }
                    }
                }
            }
        }
    }
}

/// `CCB_DEBUG_LAYOUT=<frame>` prints every pane's state once at that frame.
fn debug_panes(
    mut frame: Local<u32>,
    roots: Query<&LayoutRoot>,
    panes: Query<(&LayoutPane, &GlobalTransform, &InheritedVisibility)>,
    visuals: Query<(&GlobalTransform, &InheritedVisibility, Option<&MeshMaterial3d<GxMaterial>>)>,
) {
    *frame += 1;
    let Some(at) = std::env::var("CCB_DEBUG_LAYOUT").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    if *frame != at {
        return;
    }
    for root in &roots {
        for (i, &e) in root.panes.iter().enumerate() {
            let Ok((p, gt, vis)) = panes.get(e) else { continue };
            let vinfo = p.visual.and_then(|v| visuals.get(v).ok()).map(|(t, v, m)| (t.translation(), t.scale(), v.get(), m.is_some()));
            info!(
                "{:>3} {:20} vis {} inh {} alpha {:.2} scale {:?} pos {:?} visual {:?}",
                i, root.layout.panes[i].name, p.cur.visible, vis.get(), p.global_alpha, p.cur.scale, gt.translation(), vinfo
            );
        }
    }
}
