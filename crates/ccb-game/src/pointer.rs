//! The pointer: the mouse stands in for the Wii Remote's IR pointer. Its position is kept
//! in layout units (608x456-high space, origin at screen center, +Y up) and it is drawn
//! with the game's hand cursor.

use bevy::{camera::visibility::RenderLayers, prelude::*, window::{CursorOptions, PrimaryWindow}};

use crate::{
    gx_material::GxMaterial,
    layout::{self, LayoutAssets, UI_LAYER},
};

#[derive(Resource, Default, Debug)]
pub struct Pointer {
    /// Layout-space position, `None` when the pointer is off screen.
    pub pos: Option<Vec2>,
    /// Window position in logical pixels (for picking in the 3D view).
    pub screen: Option<Vec2>,
    pub pressed: bool,
    pub just_pressed: bool,
    pub just_released: bool,
}

#[derive(Component)]
struct PointerSprite;

pub struct PointerPlugin;

impl Plugin for PointerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pointer>()
            .add_systems(Startup, spawn_pointer)
            .add_systems(PreUpdate, read_pointer)
            .add_systems(Update, move_sprite);
    }
}

fn spawn_pointer(
    mut commands: Commands,
    mut assets: ResMut<LayoutAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if let Ok(mut c) = cursor.single_mut() {
        c.visible = false;
    }
    let (mesh, material) = assets.sprite("handcursor01.tpl", &mut meshes, &mut materials, &mut images);
    commands.spawn((
        PointerSprite,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        // Above every layout pane.
        Transform::from_xyz(0.0, 0.0, 400.0).with_scale(Vec3::new(47.0, 48.0, 1.0)),
        Visibility::Hidden,
        RenderLayers::layer(UI_LAYER),
    ));
}

fn read_pointer(
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut pointer: ResMut<Pointer>,
) {
    let Ok(w) = windows.single() else { return };
    let scale = layout::LAYOUT_HEIGHT / w.height().max(1.0);
    pointer.screen = w.cursor_position();
    pointer.pos = w
        .cursor_position()
        .map(|c| Vec2::new((c.x - w.width() / 2.0) * scale, (w.height() / 2.0 - c.y) * scale));
    pointer.pressed = buttons.pressed(MouseButton::Left);
    pointer.just_pressed = buttons.just_pressed(MouseButton::Left);
    pointer.just_released = buttons.just_released(MouseButton::Left);
}

fn move_sprite(pointer: Res<Pointer>, mut sprite: Query<(&mut Transform, &mut Visibility), With<PointerSprite>>) {
    let Ok((mut t, mut v)) = sprite.single_mut() else { return };
    match pointer.pos {
        Some(p) => {
            // The fingertip (the hotspot) is near the image's top-left corner.
            t.translation.x = p.x - 8.0;
            t.translation.y = p.y + 4.0;
            *v = Visibility::Visible;
        }
        None => *v = Visibility::Hidden,
    }
}
