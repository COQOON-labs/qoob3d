//! FDM printer profiles: build plate, non-printable zones, per-nozzle
//! reachable areas and build height (ADR-0021). The database in `data.rs` is
//! generated from OrcaSlicer's machine profiles by `cargo xtask printers`.
//!
//! In the viewport the plate is centred on the world origin (slicers arrange
//! parts themselves); [`PrinterProfile::bed`] converts printer coordinates.

mod data;

pub use data::PRINTERS;

/// Polygon in mm. Printer coordinates: X right, Y towards the back.
pub type Polygon = &'static [[f32; 2]];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrinterProfile {
    /// Stable identifier (persisted in settings).
    pub id: &'static str,
    pub vendor: &'static str,
    pub name: &'static str,
    /// Area the nozzle can print on.
    pub printable_area: Polygon,
    /// Zone inside the plate that must stay empty (e.g. Bambu X1/P1 front-left
    /// corner). Even-odd polygon, may be empty.
    pub exclude_area: Polygon,
    /// Reachable area per nozzle on multi-nozzle printers (e.g. Bambu H2D);
    /// empty for single-nozzle printers.
    pub extruder_areas: &'static [Polygon],
    pub max_height_mm: f32,
}

pub const DEFAULT_PRINTER_ID: &str = "bambu-lab-p1s";

/// Plate geometry in world coordinates (centred on the origin).
#[derive(Debug, Clone, PartialEq)]
pub struct BedGeometry {
    pub printable: Vec<[f32; 2]>,
    pub exclude: Vec<[f32; 2]>,
    pub extruder_areas: Vec<Vec<[f32; 2]>>,
    pub max_height: f32,
}

impl PrinterProfile {
    pub fn by_id(id: &str) -> Option<&'static PrinterProfile> {
        PRINTERS.iter().find(|p| p.id == id)
    }

    pub fn default_profile() -> &'static PrinterProfile {
        Self::by_id(DEFAULT_PRINTER_ID).expect("default printer exists")
    }

    /// Bounding box `(min, max)` of the printable area (printer coordinates).
    pub fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for p in self.printable_area {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }

    /// Width and depth of the printable area's bounding box.
    pub fn size(&self) -> [f32; 2] {
        let (lo, hi) = self.bounds();
        [hi[0] - lo[0], hi[1] - lo[1]]
    }

    pub fn has_exclude_area(&self) -> bool {
        !self.exclude_area.is_empty()
    }

    pub fn has_single_nozzle_zones(&self) -> bool {
        self.extruder_areas.len() > 1
    }

    /// Plate geometry centred on the world origin.
    pub fn bed(&self) -> BedGeometry {
        let (lo, hi) = self.bounds();
        let c = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
        let shift = |poly: Polygon| poly.iter().map(|p| [p[0] - c[0], p[1] - c[1]]).collect();
        BedGeometry {
            printable: shift(self.printable_area),
            exclude: shift(self.exclude_area),
            extruder_areas: self.extruder_areas.iter().map(|a| shift(a)).collect(),
            max_height: self.max_height_mm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_is_large_ids_unique_and_default_exists() {
        assert!(PRINTERS.len() > 300, "only {} printers", PRINTERS.len());
        let mut ids: Vec<_> = PRINTERS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRINTERS.len(), "duplicate ids");
        assert_eq!(PrinterProfile::default_profile().size(), [256.0, 256.0]);
    }

    #[test]
    fn every_profile_is_plausible() {
        for p in PRINTERS {
            assert!(p.printable_area.len() >= 3, "{p:?}");
            let [w, d] = p.size();
            assert!(w > 10.0 && d > 10.0 && w < 5000.0 && d < 5000.0, "{} {w}×{d}", p.name);
            assert!(p.max_height_mm > 10.0 && p.max_height_mm < 5000.0, "{}", p.name);
        }
    }

    #[test]
    fn bambu_x1_carbon_has_front_left_exclusion() {
        let x1 = PRINTERS.iter().find(|p| p.name == "Bambu Lab X1 Carbon").unwrap();
        assert!(x1.has_exclude_area());
        let bed = x1.bed();
        // Printer origin corner (front left) maps to (-128, -128) in the world.
        let min_x = bed.exclude.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let min_y = bed.exclude.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert_eq!([min_x, min_y], [-128.0, -128.0]);
    }

    #[test]
    fn bambu_h2d_has_two_nozzle_areas() {
        let h2d = PRINTERS.iter().find(|p| p.name == "Bambu Lab H2D").unwrap();
        assert!(h2d.has_single_nozzle_zones());
        assert_eq!(h2d.extruder_areas.len(), 2);
        assert_eq!(h2d.size(), [350.0, 320.0]);
    }

    #[test]
    fn a1_mini_is_small() {
        let mini = PRINTERS.iter().find(|p| p.name == "Bambu Lab A1 mini").unwrap();
        assert_eq!(mini.size(), [180.0, 180.0]);
        assert!(!mini.has_exclude_area());
    }
}
