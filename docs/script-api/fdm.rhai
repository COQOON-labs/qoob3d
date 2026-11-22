// Example module of docs/script-api.md: `import "script-api/fdm" as fdm;`
// Functions of a module are visible as `fdm::name`, constants with `export`.

export const CLEARANCE = 0.2;   // mm, typical FDM fit clearance

// Square plate `w` × `w` × 3 mm with a through hole for a pin of diameter `d`.
fn plate_with_hole(w, d, clearance) {
    box(w, w, 3) - hole(d, 3, clearance).move(0, 0, 3)
}
