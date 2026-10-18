//! Cameras with Unity semantics.
//!
//! World space is y-up. A Unity camera looks along its local +z axis; its
//! view matrix is the inverse of its world transform with z negated, and the
//! projection is right-handed (OpenGL style). We build the same matrices
//! (with wgpu's 0..1 depth range) so world-space shader maths written for the
//! original (screen positions of world points, horizon heights) carries over.

use glam::{Mat4, Quat, Vec2, Vec3};

/// Projection of a [`Camera`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Unity `orthographicSize`: half the visible height in world units.
    Orthographic { size: f32 },
    /// Vertical field of view in degrees.
    Perspective { fov_y: f32 },
}

/// A camera: world pose, projection and clip planes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub rotation: Quat,
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
    /// Forced aspect ratio (width / height); `None` uses the target's.
    pub aspect: Option<f32>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, -10.0),
            rotation: Quat::IDENTITY,
            projection: Projection::Orthographic { size: 5.0 },
            near: 0.3,
            far: 1000.0,
            aspect: None,
        }
    }
}

impl Camera {
    /// An orthographic camera at `position` (z = -10) with half-height `size`.
    pub fn ortho(position: Vec2, size: f32) -> Self {
        Self {
            position: position.extend(-10.0),
            projection: Projection::Orthographic { size },
            ..Default::default()
        }
    }

    /// The aspect used for a target of `size` pixels.
    pub fn aspect_for(&self, size: (u32, u32)) -> f32 {
        self.aspect
            .unwrap_or(size.0.max(1) as f32 / size.1.max(1) as f32)
    }

    /// Unity's `worldToCameraMatrix`.
    pub fn view(&self) -> Mat4 {
        let cam_to_world = Mat4::from_rotation_translation(self.rotation, self.position);
        Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0)) * cam_to_world.inverse()
    }

    /// Projection matrix (right-handed, looking down -z, depth 0..1).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Orthographic { size } => {
                let w = size * aspect;
                Mat4::orthographic_rh(-w, w, -size, size, self.near, self.far)
            }
            Projection::Perspective { fov_y } => {
                Mat4::perspective_rh(fov_y.to_radians(), aspect, self.near, self.far)
            }
        }
    }

    /// World to clip space for a target with the given aspect.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Half extents of an orthographic camera's view (perspective: at
    /// distance 1).
    pub fn half_extents(&self, aspect: f32) -> Vec2 {
        let h = match self.projection {
            Projection::Orthographic { size } => size,
            Projection::Perspective { fov_y } => (fov_y.to_radians() * 0.5).tan(),
        };
        Vec2::new(h * aspect, h)
    }

    /// Converts a world position to target pixels (origin top-left, y down).
    pub fn world_to_screen(&self, world: Vec3, target: (u32, u32)) -> Vec2 {
        let (w, h) = (target.0 as f32, target.1 as f32);
        let ndc = self
            .view_proj(self.aspect_for(target))
            .project_point3(world);
        Vec2::new((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h)
    }

    /// Converts a pixel position (origin top-left) to world space on the
    /// plane z = 0 (orthographic cameras).
    pub fn screen_to_world(&self, pixel: Vec2, target: (u32, u32)) -> Vec2 {
        let (w, h) = (target.0 as f32, target.1 as f32);
        let ndc = Vec2::new(pixel.x / w * 2.0 - 1.0, 1.0 - pixel.y / h * 2.0);
        let inv = self.view_proj(self.aspect_for(target)).inverse();
        let a = inv.project_point3(ndc.extend(0.0));
        let b = inv.project_point3(ndc.extend(1.0));
        let t = if (b.z - a.z).abs() > 1e-9 {
            -a.z / (b.z - a.z)
        } else {
            0.0
        };
        (a + (b - a) * t).truncate()
    }
}

/// Clip matrix for screen-space drawing: pixel coordinates with the origin
/// at the top-left and y pointing down.
pub fn screen_proj(width: u32, height: u32) -> Mat4 {
    Mat4::orthographic_rh(0.0, width as f32, height as f32, 0.0, -1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho_matches_unity_framing() {
        // The main camera of every level: (0, 8, -100), size 30.
        let mut cam = Camera::ortho(Vec2::new(0.0, 8.0), 30.0);
        cam.position.z = -100.0;
        let t = (1920, 1080);
        let top = cam.world_to_screen(Vec3::new(0.0, 38.0, 0.0), t);
        assert!(top.y.abs() < 1e-3, "{top}");
        let c = cam.world_to_screen(Vec3::new(0.0, 8.0, 0.0), t);
        assert!((c - Vec2::new(960.0, 540.0)).length() < 1e-3);
        let back = cam.screen_to_world(Vec2::new(100.0, 200.0), t);
        let again = cam.world_to_screen(back.extend(0.0), t);
        assert!((again - Vec2::new(100.0, 200.0)).length() < 1e-2);
        // Inside the clip range: depth between 0 and 1.
        let d = cam.view_proj(16.0 / 9.0).project_point3(Vec3::ZERO).z;
        assert!((0.0..=1.0).contains(&d), "{d}");
    }

    #[test]
    fn perspective_looks_along_z() {
        let cam = Camera {
            position: Vec3::new(0.0, 8.0, -52.0),
            projection: Projection::Perspective { fov_y: 60.0 },
            ..Default::default()
        };
        let vp = cam.view_proj(16.0 / 9.0);
        // A point straight ahead projects to the centre, one above it up.
        let p = vp.project_point3(Vec3::new(0.0, 8.0, 100.0));
        assert!(p.x.abs() < 1e-5 && p.y.abs() < 1e-5);
        let q = vp.project_point3(Vec3::new(0.0, 9.0, 100.0));
        assert!(q.y > 0.0);
        let r = vp.project_point3(Vec3::new(1.0, 8.0, 100.0));
        assert!(r.x > 0.0, "x stays to the right");
    }
}
