//! Small 2D geometry helpers for pattern outlines. Units are millimetres.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Vec2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Axis-aligned bounds as `(min, max)`, or `None` for an empty outline.
pub fn bounds(points: &[Vec2]) -> Option<(Vec2, Vec2)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(min, max), p| {
        (
            Vec2::new(min.x.min(p.x), min.y.min(p.y)),
            Vec2::new(max.x.max(p.x), max.y.max(p.y)),
        )
    }))
}

/// Unsigned polygon area (shoelace formula).
pub fn area(points: &[Vec2]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        twice += a.x * b.y - b.x * a.y;
    }
    (twice * 0.5).abs()
}

/// Total outline length, including the closing edge.
pub fn perimeter(points: &[Vec2]) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    points
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let d = points[(i + 1) % points.len()] - *a;
            d.x.hypot(d.y)
        })
        .sum()
}

/// Even-odd point-in-polygon test.
pub fn contains(points: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let n = points.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (points[i], points[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Outline of an axis-aligned rectangle from two opposite corners.
pub fn rectangle(a: Vec2, b: Vec2) -> Vec<Vec2> {
    let (min, max) = (
        Vec2::new(a.x.min(b.x), a.y.min(b.y)),
        Vec2::new(a.x.max(b.x), a.y.max(b.y)),
    );
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        rectangle(Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0))
    }

    #[test]
    fn rectangle_measurements() {
        let sq = square();
        assert_eq!(area(&sq), 5000.0);
        assert_eq!(perimeter(&sq), 300.0);
        assert_eq!(
            bounds(&sq),
            Some((Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)))
        );
    }

    #[test]
    fn rectangle_normalises_corners() {
        assert_eq!(
            rectangle(Vec2::new(10.0, 10.0), Vec2::new(0.0, 0.0)),
            rectangle(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0))
        );
    }

    #[test]
    fn point_in_polygon() {
        let sq = square();
        assert!(contains(&sq, Vec2::new(50.0, 25.0)));
        assert!(!contains(&sq, Vec2::new(150.0, 25.0)));
        assert!(!contains(&[], Vec2::ZERO));
    }
}
