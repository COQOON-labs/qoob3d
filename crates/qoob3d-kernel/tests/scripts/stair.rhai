// Stair (M0 probe part, docs/concept/m0-report.md): two inclined stringers
// cut to the height, 11 steps with two through slots each, and six post
// eyes with a bore. Units mm, +Z up.
//
// Booleans are batched (AG-SC-05): each step is one `cut` with both slots,
// the whole part is one `union`.

let n = 11;                  // steps (source)
let rise = 5.91;             // rise per step (source)
let angle = 52.4;            // stair angle in degrees (source)
let width = 20.0;            // outer width (source)
let height = 65.0;           // total height (source)
let stringer_t = 1.6;        // stringer thickness (source)
let stringer_b = 5.0;        // stringer width across the slope (source)
let step_depth = 5.0;        // assumption
let step_thickness = 0.8;    // assumption
let slot_w = 0.6;            // slot width (source)
let slot_l = 12.0;           // slot length (assumption)
let slot_spacing = 2.0;      // centre distance of the two slots (assumption)
let eye_d = 2.2;             // post eye outer diameter (assumption)
let eye_h = 2.0;             // post eye height (assumption)
let eye_bore = 0.9;          // post eye bore (source)
let eye_overlap = 0.5;       // eye reaches 0.5 mm into the stringer (assumption)
let eye_steps = [2, 6, 10];  // eyes at these steps, 3 per side (assumption)

let rad = angle * PI() / 180.0;
let parts = [];

// Stringers: a strip along the slope (centre line z = y * tan(angle)),
// longer than needed and cut to 0 <= z <= height.
let lead = 2.0 * stringer_b;
let strip_len = height / sin(rad) + 2.0 * lead;
let big = 10.0 * height;
let slab = box(big, big, height);
for x0 in [-width / 2.0, width / 2.0 - stringer_t] {
    let strip = box(stringer_t, strip_len, stringer_b)
        .move(stringer_t / 2.0, strip_len / 2.0 - lead, -stringer_b / 2.0)
        .rotate("x", angle)
        .move(x0, 0.0, 0.0);
    parts.push(strip & slab);
}

// Steps: top face at z = i * rise, centred on the stringer centre line.
let inner = width - 2.0 * stringer_t;
for i in range(1, n + 1) {
    let z = i.to_float() * rise;
    let y = z / tan(rad);
    let step = box(inner, step_depth, step_thickness).move(0.0, y, z - step_thickness);
    let slots = [];
    for dy in [-slot_spacing / 2.0, slot_spacing / 2.0] {
        slots.push(box(slot_l, slot_w, 3.0 * step_thickness)
            .move(0.0, y + dy, z - 3.0 * step_thickness));
    }
    parts.push(cut(step, slots));
}

// Post eyes outside the stringers, centred in height on the step level.
let r = eye_d / 2.0;
for side in [-1.0, 1.0] {
    for i in eye_steps {
        let x = side * (width / 2.0 + r - eye_overlap);
        let z = i.to_float() * rise;
        let y = z / tan(rad);
        let eye = cylinder(eye_d, eye_h).move(x, y, z - eye_h / 2.0);
        parts.push(eye - cylinder(eye_bore, 2.0 * eye_h).move(x, y, z - eye_h));
    }
}

show(union(parts), "stair");
