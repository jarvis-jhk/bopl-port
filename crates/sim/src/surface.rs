//! Walking on platforms: the perimeter parameterisation of a
//! `StickyRoundedRectangle` (player.md §5).

use crate::fix::Fix;
use crate::object::ObjId;
use crate::vec2::Vec2;
use crate::world::{Kind, World};

/// Perimeter zones (clockwise from the top face's left end).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    Up = 0,
    TopRight = 1,
    Right = 2,
    BotRight = 3,
    Down = 4,
    BotLeft = 5,
    Left = 6,
    TopLeft = 7,
}

impl Zone {
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Zone::TopRight | Zone::BotRight | Zone::BotLeft | Zone::TopLeft
        )
    }

    fn from_index(i: usize) -> Zone {
        [
            Zone::Up,
            Zone::TopRight,
            Zone::Right,
            Zone::BotRight,
            Zone::Down,
            Zone::BotLeft,
            Zone::Left,
            Zone::TopLeft,
        ][i]
    }
}

/// A snapshot of a platform's walkable surface.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub obj: ObjId,
    pub position: Vec2,
    pub right: Vec2,
    pub up: Vec2,
    /// Inner half extents (`extents`).
    pub x: Fix,
    pub y: Fix,
    /// `rr.radius` (world radius of the collider).
    pub radius: Fix,
    pub c: [Fix; 8],
    pub total: Fix,
}

impl Surface {
    /// The surface of a platform object (None if it is not a live platform).
    pub fn of(w: &World, obj: ObjId) -> Option<Surface> {
        let o = w.objects.get(obj)?;
        let Kind::Platform(p) = &o.kind else {
            return None;
        };
        let radius = match w.physics.rr(obj) {
            Some(r) => r.radius,
            None => o.collider.map_or(Fix::ZERO, |c| c.start_radius),
        };
        Some(Surface {
            obj,
            position: o.tf.position,
            right: o.tf.right,
            up: o.tf.up,
            x: p.sticky.extents.x,
            y: p.sticky.extents.y,
            radius,
            c: p.sticky.c,
            total: p.sticky.total,
        })
    }

    /// Zone and normal at `p` (§5.1).
    pub fn zone_normal(&self, p: Vec2) -> (Zone, Vec2) {
        let d = p - self.position;
        let a = d.dot(self.right);
        let b = d.dot(self.up);
        let vec2 = self.right * self.x;
        let vec = self.up * self.y;
        if a <= -self.x {
            if b >= self.y {
                (
                    Zone::TopLeft,
                    (p - (self.position - vec2 + vec)).normalized_safe(),
                )
            } else if b <= -self.y {
                (
                    Zone::BotLeft,
                    (p - (self.position - vec2 - vec)).normalized_safe(),
                )
            } else {
                (Zone::Left, -self.right)
            }
        } else if a >= self.x {
            if b >= self.y {
                (
                    Zone::TopRight,
                    (p - (self.position + vec2 + vec)).normalized_safe(),
                )
            } else if b <= -self.y {
                (
                    Zone::BotRight,
                    (p - (self.position + vec2 - vec)).normalized_safe(),
                )
            } else {
                (Zone::Right, self.right)
            }
        } else if b >= Fix::ZERO {
            (Zone::Up, self.up)
        } else {
            (Zone::Down, -self.up)
        }
    }

    /// `currentNormal` at a position.
    pub fn normal(&self, p: Vec2) -> Vec2 {
        self.zone_normal(p).1
    }

    /// `forwardsDirection` at a position (clockwise along the surface).
    pub fn forward(&self, p: Vec2) -> Vec2 {
        self.normal(p).mul_by_neg_i()
    }

    pub fn in_corner(&self, p: Vec2) -> bool {
        self.zone_normal(p).0.is_corner()
    }

    /// Centre-path length for a bopl of radius `r`.
    pub fn path_len(&self, r: Fix) -> Fix {
        self.total + r * Fix::PI_TIMES_2
    }

    /// `PositionFromLocalPlayerPos(t, r)` (§5.2).
    pub fn position_from_param(&self, t: Fix, r: Fix) -> Vec2 {
        let l = t * self.path_len(r);
        let mut acc = Fix::ZERO;
        let rr = self.radius + r;
        let (pos, right, up, x, y) = (self.position, self.right, self.up, self.x, self.y);
        for i in 0..8 {
            let f = l - acc;
            acc += self.c[i];
            let z = Zone::from_index(i);
            if z.is_corner() {
                acc += r * Fix::PI_OVER_2;
            }
            if l < acc {
                let arc = |th: Fix| (right * th.cos() + up * th.sin()) * rr;
                return match z {
                    Zone::Up => pos - right * x + up * (y + rr) + right * f,
                    Zone::TopRight => pos + right * x + up * y + arc(Fix::PI_OVER_2 - f / rr),
                    Zone::Right => pos + right * (x + rr) + up * (y - f),
                    Zone::BotRight => pos + right * x - up * y + arc(-(f / rr)),
                    Zone::Down => pos + right * x - up * (y + rr) - right * f,
                    Zone::BotLeft => {
                        pos - right * x - up * y + arc(Fix::PI_OVER_2 - f / rr + Fix::PI)
                    }
                    Zone::Left => pos - right * (x + rr) + up * (f - y),
                    Zone::TopLeft => pos - right * x + up * y + arc(Fix::PI - f / rr),
                };
            }
        }
        Vec2::ZERO
    }

    /// `GetClosestPoint(p)` on the platform outline (§5.4).
    pub fn closest_point(&self, p: Vec2) -> Vec2 {
        let d = p - self.position;
        let a = d.dot(self.right);
        let b = d.dot(self.up);
        let (pos, right, up, x, y, r) = (
            self.position,
            self.right,
            self.up,
            self.x,
            self.y,
            self.radius,
        );
        let vec2 = right * x;
        let vec = up * y;
        let corner = |k: Vec2| k + (p - k).normalized() * r;
        if a <= -x {
            if b >= y {
                return corner(pos - vec2 + vec);
            }
            if b <= -y {
                return corner(pos - vec2 - vec);
            }
            pos + (-right) * (x + r) + up * b
        } else if a >= x {
            if b >= y {
                return corner(pos + vec2 + vec);
            }
            if b <= -y {
                return corner(pos + vec2 - vec);
            }
            pos + right * (x + r) + up * b
        } else if b >= Fix::ZERO {
            pos + right * a + up * (y + r)
        } else {
            pos + right * a + (-up) * (y + r)
        }
    }

    /// `LocalPlayerPosFromPos(p, r)` (§5.3), with the original's Atan2.
    pub fn param_from_position(&self, p: Vec2, r: Fix) -> Fix {
        let (zone, _) = self.zone_normal(p);
        let mut base = Fix::ZERO;
        for i in 0..zone as usize {
            base += self.c[i];
            if Zone::from_index(i).is_corner() {
                base += r * Fix::PI_OVER_2;
            }
        }
        let rr = self.radius + r;
        let q = self.closest_point(p);
        let (pos, right, up, x, y, rad) = (
            self.position,
            self.right,
            self.up,
            self.x,
            self.y,
            self.radius,
        );
        let add = match zone {
            Zone::Up => (pos + up * (y + rad) - right * x).distance(q),
            Zone::TopRight => (Vec2::angle2(up, q - (pos + up * y + right * x)) * rr).abs(),
            Zone::Right => (pos + up * y + right * (x + rad)).distance(q),
            Zone::BotRight => (Vec2::angle2(right, q - (pos - up * y + right * x)) * rr).abs(),
            Zone::Down => (pos - up * (y + rad) + right * x).distance(q),
            Zone::BotLeft => (Vec2::angle2(-up, q - (pos - up * y - right * x)) * rr).abs(),
            Zone::Left => (pos - up * y - right * (x + rad)).distance(q),
            Zone::TopLeft => (Vec2::angle2(-right, q - (pos + up * y - right * x)) * rr).abs(),
        };
        (base + add) / self.path_len(r)
    }
}

/// `maxMovementAllowed` (§5.5): the platform's own collider is disabled
/// during the probes and re-enabled afterwards (always, as the original).
pub fn max_movement_allowed(
    w: &mut World,
    plat: ObjId,
    r: Fix,
    pos: Vec2,
    v: Vec2,
    dt: Fix,
) -> Fix {
    let mut res = Fix::from_long(10_000_000_000);
    w.set_collider_enabled(plat, false);
    let d = v.normalized_safe();
    let len = v.magnitude() * dt;
    let o = r * d;
    let h1 = w.physics.raycast_closest_rr(&w.objects, pos + o, d, len);
    let h2 = w
        .physics
        .raycast_closest_rr(&w.objects, pos + o + o.mul_by_i(), d, len);
    let h3 = w
        .physics
        .raycast_closest_rr(&w.objects, pos + o + o.mul_by_neg_i(), d, len);
    if h1.hit() {
        res = h1.near_dist;
    }
    if h2.hit() && h2.near_dist < res {
        res = h2.near_dist;
    }
    if h3.hit() && h3.near_dist < res {
        res = h3.near_dist;
    }
    w.set_collider_enabled(plat, true);
    res
}

/// `moveAlongSurface` (blocking variant).
pub fn move_along_surface(
    w: &mut World,
    plat: ObjId,
    body_pos: Vec2,
    r: Fix,
    speed: Fix,
    dt: Fix,
    t: Fix,
) -> Fix {
    let Some(s) = Surface::of(w, plat) else {
        return t;
    };
    let fwd = s.forward(body_pos);
    let m = max_movement_allowed(w, plat, r, body_pos, speed * fwd, dt);
    let step = m.min(speed.abs()) * speed.sign_fix();
    let step = step * dt;
    // The platform's surface cannot change during the probes.
    (Fix::ONE + t + step / s.path_len(r)).slow_mod(Fix::ONE)
}
