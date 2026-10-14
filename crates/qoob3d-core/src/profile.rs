//! Sketch profiles as computed by the kernel (ADR-0028): a closed region
//! with its key (ADR-0011), area and a triangulation in sketch coordinates
//! for display and hit tests.

use crate::document::EntityId;

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Sorted ids of the curves bounding the region (ADR-0011).
    pub key: Vec<EntityId>,
    pub area_mm2: f64,
    /// Vertices in sketch coordinates (mm).
    pub positions: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl Profile {
    /// Whether `p` (sketch coordinates) lies inside the region.
    pub fn contains(&self, p: [f64; 2]) -> bool {
        self.triangles.iter().any(|t| {
            let [a, b, c] = t.map(|i| self.positions[i as usize]);
            let s1 = side(p, a, b);
            let s2 = side(p, b, c);
            let s3 = side(p, c, a);
            (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
        })
    }
}

fn side(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_triangulated_region() {
        let square = Profile {
            key: vec![EntityId(1)],
            area_mm2: 100.0,
            positions: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        assert!(square.contains([5.0, 5.0]));
        assert!(square.contains([0.0, 5.0]), "on the border");
        assert!(!square.contains([11.0, 5.0]));
    }
}
