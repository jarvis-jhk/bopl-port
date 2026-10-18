//! The seam between the simulation and the presentation.
//!
//! The simulation (`bopl-sim`) knows nothing about rendering. Each frame the
//! game turns the interpolated sim state into a [`WorldView`]: platform
//! poses (by index into the level's platform list), dynamic sprites
//! (players, pickups, projectiles, effects) and camera inputs. The
//! [`LevelView`](crate::level_view::LevelView) combines it with the level's
//! static presentation (backdrop, water, cameras) into a render frame.
//! Sounds go the other way as names (see `crate::sound`).

use bopl_render::{Color, Quad};
use glam::{Vec2, Vec3};

/// Pose of one platform (index into `Level::platforms`), from the sim's
/// fixed-point transform converted to floats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlatformPose {
    pub platform: usize,
    pub position: Vec2,
    /// Radians, counter-clockwise.
    pub rotation: f32,
    /// Uniform scale relative to the authored scale (growth/shrink rays).
    pub scale: f32,
    /// False once the platform is gone (sudden death, destroyed).
    pub visible: bool,
}

/// A dynamic sprite drawn like a SpriteRenderer of the original.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteDraw {
    /// Import sprite id (`sprites.json`).
    pub sprite: String,
    pub position: Vec3,
    /// Radians, counter-clockwise.
    pub rotation: f32,
    pub scale: Vec2,
    pub color: Color,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Sorting layer index (`index.json` order) and order in layer.
    pub sorting_layer: usize,
    pub sorting_order: i32,
    /// Unity layer (decides which cameras see it: 8 Player, 12 effect, ...).
    pub layer: u32,
    /// Material id (`materials.json`); `None` = Sprites/Default.
    pub material: Option<String>,
    /// 0 none, 1 visible inside, 2 visible outside sprite masks.
    pub mask_interaction: i32,
    /// Per-renderer shader values the original's scripts set on the
    /// material (meaning per shader, see `level_view::apply_material`).
    pub params: [f32; 4],
    /// Per-renderer colours (`_MainColor`, `_ShadowColor` of the owner's
    /// player material for the smear shader).
    pub colors: Option<[Color; 2]>,
    /// Draw size of sliced/tiled renderers (`None` = the sprite's size).
    pub size: Option<Vec2>,
}

/// A prepared quad (particles, rope strips, shapes) drawn by the cameras
/// whose culling mask contains `layer`, sorted with the sprites.
#[derive(Clone, Debug)]
pub struct QuadDraw {
    pub quad: Quad,
    /// Material id: its shading is applied to `quad` (`None` keeps the
    /// quad's own shading).
    pub material: Option<String>,
    pub params: [f32; 4],
    pub layer: u32,
    pub sorting_layer: usize,
    pub sorting_order: i32,
    /// World z (sorting along the view axis).
    pub z: f32,
    pub mask_interaction: i32,
}

#[allow(dead_code)] // Filled by the sim presentation (M3+).
impl SpriteDraw {
    pub fn new(sprite: impl Into<String>, position: Vec3) -> Self {
        Self {
            sprite: sprite.into(),
            position,
            rotation: 0.0,
            scale: Vec2::ONE,
            color: Color::WHITE,
            flip_x: false,
            flip_y: false,
            sorting_layer: 0,
            sorting_order: 0,
            layer: 0,
            material: None,
            mask_interaction: 0,
            params: [0.0; 4],
            colors: None,
            size: None,
        }
    }
}

/// What the presentation shows this frame.
#[derive(Clone, Debug, Default)]
pub struct WorldView {
    /// Seconds since the level was loaded (presentation clock: fades,
    /// shader animation).
    pub time: f32,
    /// Main camera x; `None` keeps the level's authored position.
    pub camera_x: Option<f32>,
    /// Main camera orthographic size when zoomed out (`None` = the
    /// level's, 30); the camera then moves up so that its bottom edge stays.
    pub camera_size: Option<f32>,
    /// Camera shake offset (world units).
    pub shake: Vec2,
    /// Time stop active (freezes shader animation like `_Timestop01`).
    pub time_stopped: bool,
    /// Platform poses; platforms without an entry keep their authored pose.
    pub platforms: Vec<PlatformPose>,
    pub sprites: Vec<SpriteDraw>,
    /// Prepared quads (particles, ropes).
    pub quads: Vec<QuadDraw>,
    /// Clear the slime-trail textures (first frame of a level).
    pub clear_trails: bool,
}

/// The main camera's horizontal follow (`PlayerAverageCamera`, levels.md 7):
/// target = midpoint of the alive players' min and max x (both start at 0),
/// smoothed, clamped to the level's camera range, snapped to 1/60 unit.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)] // Driven by player positions from the sim (M3+).
pub struct CameraFollow {
    pub x: f32,
    /// Orthographic size (30; larger while zoomed out, see [`Self::zoom`]).
    pub size: f32,
}

#[allow(dead_code)]
impl CameraFollow {
    pub const MIN_X: f32 = -97.27;
    pub const MAX_X: f32 = 97.6;
    pub const SIZE: f32 = 30.0;

    /// Largest zoom-out (1.5 × the original's size).
    pub const MAX_SIZE: f32 = 45.0;

    pub fn new(x: f32) -> Self {
        Self {
            x,
            size: Self::SIZE,
        }
    }

    /// Ours (setting "camera zoom", more than 4 players): the size that
    /// keeps the alive players (`points`: x, y) in view with a margin,
    /// between the original's 30 and [`Self::MAX_SIZE`], smoothed; the
    /// camera's bottom edge stays where the original's is (so the water
    /// looks the same and more sky shows). Disabled: back to 30.
    pub fn zoom(&mut self, dt: f32, points: &[(f32, f32)], aspect: f32, enabled: bool) {
        let mut target = Self::SIZE;
        if enabled && !points.is_empty() {
            let (mut lo, mut hi, mut top) = (0.0f32, 0.0f32, f32::MIN);
            for &(x, y) in points {
                lo = lo.min(x);
                hi = hi.max(x);
                top = top.max(y);
            }
            let margin = 10.0;
            let horizontal = ((hi - lo) * 0.5 + margin) / aspect.max(0.1);
            // Bottom edge at 8 - 30 = -22: the top is at -22 + 2 size.
            let vertical = (top + 6.0 + 22.0) * 0.5;
            target = horizontal.max(vertical).clamp(Self::SIZE, Self::MAX_SIZE);
        }
        let d = (dt.min(0.0667) * 1.5).min(1.0);
        self.size += (target - self.size) * d;
        if (self.size - target).abs() < 0.01 {
            self.size = target;
        }
    }

    /// The camera centre's y for the current size (8 at the original's).
    pub fn y(&self) -> f32 {
        8.0 + (self.size - Self::SIZE)
    }

    /// Advances by a real-time `dt` towards the players at `xs`.
    pub fn update(&mut self, dt: f32, xs: impl IntoIterator<Item = f32>, aspect: f32) -> f32 {
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for x in xs {
            lo = lo.min(x);
            hi = hi.max(x);
        }
        let target = (lo + hi) * 0.5;
        let d = dt.min(0.0667) * 1.5;
        let mut x = (1.0 - d) * self.x + d * target;
        let w = self.size * aspect;
        let (min, max) = (Self::MIN_X + w + 1.0, Self::MAX_X - w - 1.0);
        if min <= max {
            x = x.clamp(min, max);
        } else {
            x = (min + max) * 0.5;
        }
        self.x = (x * 60.0).round() / 60.0;
        self.x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_follow_moves_towards_players_and_clamps() {
        let mut c = CameraFollow::new(0.0);
        // A single player far right: the target is the midpoint with 0.
        // Snapping each step to 1/60 stalls the smoothing once a step is
        // below half a snap (within 1/120 / 0.025 = 1/3 unit), as in the
        // original.
        for _ in 0..600 {
            c.update(1.0 / 60.0, [40.0], 16.0 / 9.0);
        }
        assert!((c.x - 20.0).abs() < 0.34, "{}", c.x);
        // Beyond the clamp: max x = 97.6 - 53.33 - 1.
        for _ in 0..600 {
            c.update(1.0 / 60.0, [190.0], 16.0 / 9.0);
        }
        assert!(
            (c.x - (97.6 - 30.0 * 16.0 / 9.0 - 1.0)).abs() < 1.0 / 60.0,
            "{}",
            c.x
        );
        // Snapped to 1/60.
        assert!(((c.x * 60.0).round() - c.x * 60.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_out_only_when_enabled_and_needed() {
        let wide = [(-70.0, 0.0), (70.0, 0.0), (0.0, 5.0)];
        let mut c = CameraFollow::new(0.0);
        for _ in 0..600 {
            c.zoom(1.0 / 60.0, &wide, 16.0 / 9.0, false);
        }
        assert_eq!(c.size, CameraFollow::SIZE);
        for _ in 0..600 {
            c.zoom(1.0 / 60.0, &wide, 16.0 / 9.0, true);
        }
        // (140 / 2 + 10) / (16 / 9) = 45: the cap.
        assert!((c.size - CameraFollow::MAX_SIZE).abs() < 0.05, "{}", c.size);
        assert!((c.y() - (8.0 + 15.0)).abs() < 0.05);
        // Players close together: back to the original's size.
        for _ in 0..900 {
            c.zoom(1.0 / 60.0, &[(0.0, 0.0), (10.0, 0.0)], 16.0 / 9.0, true);
        }
        assert_eq!(c.size, CameraFollow::SIZE);
    }
}
