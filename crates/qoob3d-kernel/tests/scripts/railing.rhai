// Railing section from wire (M0 probe part, werft/m0/teile.py `reling`):
// a U-shaped handrail with bent corners, three inner posts and two
// horizontal rails. Every wire is its own body (as in the M0 probe).

let d = 0.8;             // posts and handrail (source)
let d_rail = 0.5;        // horizontal rails (source)
let rails_z = [8.0, 15.0]; // rail heights (source)
let top = 22.0;          // handrail height (source)
let pitch = 24.0;        // post pitch (source)
let bend = 1.5;          // bend radius (source: >= 1.5 mm)
let fields = 4;          // 4 fields = 5 posts (assumption)
let foot = 2.0;          // posts reach 2 mm into the deck (assumption)

let length = fields.to_float() * pitch;

// End posts, handrail and both bends in one wire; the kernel rounds the
// corners with the bend radius (AG-SC-08).
show(pipe([[0.0, 0.0, -foot], [0.0, 0.0, top], [length, 0.0, top], [length, 0.0, -foot]], d, bend),
    "handrail");

for k in range(1, fields) {
    let x = k.to_float() * pitch;
    show(pipe([[x, 0.0, -foot], [x, 0.0, top]], d, bend), "post_" + k);
}

for z in rails_z {
    show(pipe([[0.0, 0.0, z], [length, 0.0, z]], d_rail, bend), "rail_" + z);
}
