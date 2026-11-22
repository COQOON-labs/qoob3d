// Crane column (M0 probe part, docs/concept/m0-report.md) the way the
// M0 probe built it: one half profile (x = radius, y = height) revolved
// about Z, then the pocket cut from the top (AG-SC-07).

let r = 13.0 / 2.0;      // column radius
let h = 20.0;            // column height
let c = 0.5;             // chamfer of the column edges
let rz = 4.0 / 2.0;      // pin radius
let lz = 2.45;           // pin length
let cz = 0.3;            // chamfer at the pin end

let half = polygon([
    [0.0, -lz],
    [rz - cz, -lz],
    [rz, -lz + cz],
    [rz, 0.0],
    [r - c, 0.0],
    [r, c],
    [r, h - c],
    [r - c, h],
    [0.0, h],
]);
let body = revolve(half, 360.0);
let cavity = box(4.4, 5.4, 4.2).move(0.0, 0.0, h - 4.2);

show(body - cavity, "crane_column");
